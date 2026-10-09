//! 浏览器 Native Messaging：读取和用户确认的会话捕获分别授权；MCP 始终只读。
use crate::{context::Context, model::Result, policy::Access, transport::invoke, Vault};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
use tempfile::NamedTempFile;

const HOST_NAME: &str = "com.recallcard.host";
const CONFIG_NAME: &str = "com.recallcard.host.config.json";
const CONFIG_LIMIT: usize = 16 * 1024;
#[cfg(windows)]
const LAUNCHER_NAME: &str = "recallcard-native-launcher.exe";
#[cfg(not(windows))]
const LAUNCHER_NAME: &str = "recallcard-native-launcher";

/// 只由本机安装者生成；浏览器参数与消息不能选择或覆盖配置。
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LauncherConfig {
    schema_version: u32,
    host: String,
    vault: PathBuf,
    scopes: Vec<String>,
    extension_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ipc_endpoint: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    capture_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    semantic_config: Option<PathBuf>,
}

/// 纯配置校验，不读取 Vault，不注册浏览器，也不修改文件。
pub fn parse_launcher_config(bytes: &[u8]) -> Result<LauncherConfig> {
    if bytes.len() > CONFIG_LIMIT {
        return Err("Native 启动配置超过 16 KiB".into());
    }
    let config: LauncherConfig =
        serde_json::from_slice(bytes).map_err(|_| "Native 启动配置 JSON/schema 无效")?;
    if config.schema_version != 1 || config.host != HOST_NAME {
        return Err("Native 启动配置版本或 host 不匹配".into());
    }
    if !config.vault.is_absolute()
        || config.vault.as_os_str().to_string_lossy().contains('\0')
        || config
            .vault
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err("Native 配置 Vault 必须为固定的绝对路径，不能包含相对跳转".into());
    }
    extension_origin(&config.extension_id)?;
    if config.scopes.len() > 32 {
        return Err("Native 配置最多允许 32 个 scope".into());
    }
    let access = Access::new(config.scopes.clone())?;
    if access.scopes().len() != config.scopes.len() {
        return Err("Native 配置不能包含重复 scope".into());
    }
    if let Some(scope) = &config.capture_scope {
        if !access.permits(scope) {
            return Err("浏览器保存范围必须属于已配置的资料范围".into());
        }
    }
    if let Some(path) = &config.semantic_config {
        validate_semantic_path(path)?;
        if config.ipc_endpoint.is_some() {
            return Err("Native 离线语义配置与 IPC 不能同时指定".into());
        }
    }
    Ok(config)
}

/// 只有专用副本文件名启用此入口，普通 recallcard CLI 保持原行为。
/// 配置位置仅取自当前可执行文件目录，绝不取自当前工作目录或参数。
pub fn dispatch_launcher() -> Option<Result<()>> {
    let executable = std::env::current_exe().ok()?;
    if executable.file_name()? != LAUNCHER_NAME {
        return None;
    }
    Some(run_launcher(
        &executable,
        std::env::args_os().skip(1).collect(),
    ))
}

fn run_launcher(executable: &Path, arguments: Vec<OsString>) -> Result<()> {
    if arguments.is_empty() || arguments.len() > 2 {
        return Err("Native 启动仅接受来源与可选的 --parent-window 参数".into());
    }
    let origin = arguments[0]
        .to_str()
        .ok_or("Native 来源必须为 UTF-8 字符串")?;
    if let Some(parent) = arguments.get(1) {
        let valid = parent
            .to_str()
            .and_then(|s| s.strip_prefix("--parent-window="))
            .is_some_and(|value| {
                !value.is_empty()
                    && value.bytes().all(|b| b.is_ascii_digit())
                    && value.parse::<u64>().is_ok()
            });
        if !valid {
            return Err("Native 启动参数无效；不能覆盖配置、Vault 或 scope".into());
        }
    }
    reject_links(executable)?;
    let directory = executable.parent().ok_or("Native 启动器没有父目录")?;
    reject_shared_writes(directory)?;
    let path = directory.join(CONFIG_NAME);
    reject_links(&path)?;
    reject_shared_writes(&path)?;
    let metadata = fs::symlink_metadata(&path).map_err(|e| format!("无法读取 Native 配置：{e}"))?;
    if !metadata.is_file() {
        return Err("Native 配置必须为普通文件".into());
    }
    let mut bytes = Vec::new();
    File::open(&path)
        .map_err(|e| e.to_string())?
        .take(CONFIG_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let config = parse_launcher_config(&bytes)?;
    if origin != extension_origin(&config.extension_id)? {
        return Err("Native Messaging 来源扩展不在允许名单".into());
    }
    reject_links(&config.vault)?;
    let vault = Vault::open(&config.vault)?;
    let access = Access::new(config.scopes)?;
    let semantic = config
        .semantic_config
        .as_deref()
        .map(load_native_semantic)
        .transpose()?;
    let services = NativeServices {
        capture_scope: config.capture_scope.as_deref(),
        semantic: semantic.as_ref(),
    };
    if let Some(endpoint) = config.ipc_endpoint {
        let client =
            crate::ipc::Client::new(&vault, &access, endpoint, std::time::Duration::from_secs(5))?;
        return serve_native_bound_service_io(
            |name, args, session, installation| {
                browser_operation(
                    &vault,
                    &access,
                    &services,
                    &config.extension_id,
                    (session, installation),
                    name,
                    &args,
                )
                .unwrap_or_else(|| client.invoke(name, args))
            },
            |session, installation| {
                connection_revision(&vault, &config.extension_id, session, installation)
            },
            &config.extension_id,
            origin,
            std::io::stdin().lock(),
            std::io::stdout().lock(),
        );
    }
    serve_native_configured_io(
        &vault,
        access,
        &services,
        &config.extension_id,
        origin,
        std::io::stdin().lock(),
        std::io::stdout().lock(),
    )
}

fn metadata_is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Windows junction 等 reparse point 也不接受。
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.is_symlink()
    }
}

fn reject_links(path: &Path) -> Result<()> {
    for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata_is_link(&metadata) => {
                return Err(format!(
                    "Native 路径不能含符号链接或重解析点：{}",
                    ancestor.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

fn reject_install_root_link(path: &Path) -> Result<()> {
    // 接受用户明确选定的 /var 等父目录别名，再绑定 canonical 安装路径；
    // 安装根本身（包括 dangling link）仍然不能是符号链接或重解析点。
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata_is_link(&metadata) => {
            Err("安装根目录不能是符号链接或重解析点".into())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn reject_shared_writes(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::symlink_metadata(path)
            .map_err(|e| e.to_string())?
            .permissions()
            .mode()
            & 0o022
            != 0
        {
            return Err("Native 配置及安装目录不能允许组或其他用户写入".into());
        }
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn set_private_permissions(file: &File, executable: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(if executable {
            0o700
        } else {
            0o600
        }))
        .map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    let _ = (file, executable);
    Ok(())
}

fn write_new(path: &Path, reader: &mut impl Read, executable: bool) -> Result<()> {
    reject_links(path)?;
    let mut temporary = NamedTempFile::new_in(path.parent().ok_or("Native 文件没有父目录")?)
        .map_err(|e| e.to_string())?;
    set_private_permissions(temporary.as_file(), executable)?;
    std::io::copy(reader, temporary.as_file_mut()).map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary
        .persist_noclobber(path)
        .map_err(|e| format!("Native 文件已存在或无法安全保存：{e}"))?;
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeRequest {
    protocol: String,
    request_id: String,
    nonce: String,
    action: String,
    #[serde(default)]
    arguments: Option<Value>,
    session_ref: String,
    installation_id: Option<String>,
}
pub fn extension_origin(id: &str) -> Result<String> {
    if id.len() != 32 || !id.bytes().all(|b| (b'a'..=b'p').contains(&b)) {
        return Err("Chrome 扩展 ID 必须为 a–p 的 32 位字符串".into());
    }
    Ok(format!("chrome-extension://{id}/"))
}
pub fn serve_native(vault: &Vault, access: Access, extension: &str, origin: &str) -> Result<()> {
    serve_native_io(
        vault,
        access,
        extension,
        origin,
        std::io::stdin().lock(),
        std::io::stdout().lock(),
    )
}
pub fn serve_native_io<R: Read, W: Write>(
    vault: &Vault,
    access: Access,
    extension: &str,
    origin: &str,
    reader: R,
    writer: W,
) -> Result<()> {
    serve_native_capture_io(vault, access, None, extension, origin, reader, writer)
}

#[derive(Default)]
struct NativeServices<'a> {
    capture_scope: Option<&'a str>,
    semantic: Option<&'a crate::semantic::SemanticSearch>,
}
impl NativeServices<'_> {
    fn read(&self, vault: &Vault, access: Access, name: &str, args: Value) -> Result<Value> {
        let context = match self.semantic {
            Some(semantic) => Context::with_semantic(vault, access, semantic),
            None => Context::new(vault, access),
        };
        invoke(&context, name, args)
    }
}
fn validate_semantic_path(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path.as_os_str().to_string_lossy().contains('\0')
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err("Native 语义配置必须为固定的绝对路径，不能包含相对跳转".into());
    }
    Ok(())
}
fn load_native_semantic(path: &Path) -> Result<crate::semantic::SemanticSearch> {
    validate_semantic_path(path)?;
    reject_links(path)?;
    reject_shared_writes(path)?;
    crate::semantic::SemanticSearch::from_offline_config_file(path)
}

/// 仅用于可信本机启动；每条浏览器消息仍重新核对连接授权与当前资料。
pub fn serve_native_semantic_io<R: Read, W: Write>(
    vault: &Vault,
    access: Access,
    semantic_config: &Path,
    extension: &str,
    origin: &str,
    reader: R,
    writer: W,
) -> Result<()> {
    let semantic = load_native_semantic(semantic_config)?;
    serve_native_configured_io(
        vault,
        access,
        &NativeServices {
            capture_scope: None,
            semantic: Some(&semantic),
        },
        extension,
        origin,
        reader,
        writer,
    )
}

/// capture_scope 只能来自安装者的本机配置，网页和模型不能通过请求启用。
pub fn serve_native_capture_io<R: Read, W: Write>(
    vault: &Vault,
    access: Access,
    capture_scope: Option<&str>,
    extension: &str,
    origin: &str,
    reader: R,
    writer: W,
) -> Result<()> {
    serve_native_configured_io(
        vault,
        access,
        &NativeServices {
            capture_scope,
            semantic: None,
        },
        extension,
        origin,
        reader,
        writer,
    )
}

fn serve_native_configured_io<R: Read, W: Write>(
    vault: &Vault,
    access: Access,
    services: &NativeServices<'_>,
    extension: &str,
    origin: &str,
    reader: R,
    writer: W,
) -> Result<()> {
    if services
        .capture_scope
        .is_some_and(|scope| !access.permits(scope))
    {
        return Err("浏览器保存范围不在本机允许名单中".into());
    }
    serve_native_bound_service_io(
        |name, args, session, installation| {
            browser_operation(
                vault,
                &access,
                services,
                extension,
                (session, installation),
                name,
                &args,
            )
            .unwrap_or_else(|| services.read(vault, access.clone(), name, args))
        },
        |session, installation| connection_revision(vault, extension, session, installation),
        extension,
        origin,
        reader,
        writer,
    )
}

fn connection_revision(
    vault: &Vault,
    extension: &str,
    session: &str,
    installation: Option<&str>,
) -> Result<String> {
    use crate::application::connections;
    let id = connections::connection_key(
        "browser",
        extension,
        session.split(':').next().unwrap_or(""),
        installation,
    );
    Ok(connections::get(vault, &id)
        .map_err(|e| e.to_string())?
        .map(|entry| format!("{}:{}", entry.permission_revision, entry.revoked))
        .unwrap_or_default())
}

fn browser_operation(
    vault: &Vault,
    access: &Access,
    services: &NativeServices<'_>,
    extension: &str,
    binding: (&str, Option<&str>),
    name: &str,
    args: &Value,
) -> Option<Result<Value>> {
    use crate::application::connections;
    let (session, installation) = binding;
    let capture_scope = services.capture_scope;
    let reading = matches!(name, "bootstrap" | "search" | "read" | "sources");
    // Preserve explicitly installed legacy IPC routing only when no managed
    // browser grant exists. Managed installations never fall back to broad scopes.
    if reading && installation.is_none() {
        match connections::has_browser_binding(vault, extension) {
            Ok(false) => return None,
            Ok(true) => {}
            Err(error) => return Some(Err(error.to_string())),
        }
    }

    if !reading
        && !matches!(
            name,
            "connection"
                | "request_pairing"
                | "capture_preview"
                | "capture_save"
                | "automatic_capture"
                | "authorized_read"
        )
    {
        return None;
    }
    let outcome = (|| {
        let platform = session.split(':').next().unwrap_or("");
        let id = connections::connection_key("browser", extension, platform, installation);
        let entry = connections::get(vault, &id).map_err(|e| e.to_string())?;
        if name == "request_pairing" {
            if args.as_object().is_none_or(|args| !args.is_empty()) {
                return Err("连接请求不能自带授权、范围或身份参数".into());
            }
            let installation = installation.ok_or("请先更新扩展，连接请求需要本机安装编号")?;
            let pending = connections::request_pairing(
                vault,
                extension,
                installation,
                platform,
                access.scopes(),
                capture_scope.map(str::to_string).into_iter().collect(),
            )
            .map_err(|e| e.to_string())?;
            return Ok(
                json!({"status":"pending","request_id":pending.request_id,"expires_at":pending.expires_at,"platform":pending.platform,"installation_id":pending.installation_id,"data_access":false}),
            );
        }
        let setup_required = entry.is_none()
            && (installation.is_some()
                || connections::has_browser_binding(vault, extension)
                    .map_err(|e| e.to_string())?);
        if name == "connection" {
            if args.as_object().is_none_or(|a| !a.is_empty()) {
                return Err("连接检查不接受额外参数".into());
            }
            if let Some(entry) = &entry {
                if !entry.revoked {
                    connections::observe(vault, &id, entry.permission_revision, "handshake")
                        .map_err(|e| e.to_string())?;
                }
            }
            let active = entry.as_ref().filter(|entry| !entry.revoked);
            let disclose_configuration =
                !setup_required && entry.as_ref().is_none_or(|entry| !entry.revoked);
            return Ok(
                json!({"connection_id":crate::conversation::connection_id(vault,capture_scope.unwrap_or("read-only"))?,
                "capture_enabled":!setup_required && capture_scope.is_some() && entry.as_ref().is_none_or(|entry|!entry.revoked && capture_scope.is_some_and(|scope|entry.grant.capture_scopes.iter().any(|s|s==scope))),
                "capture_scope":capture_scope.filter(|_|disclose_configuration),"read_scopes":if disclose_configuration {access.scopes()} else {Vec::<String>::new()},"vault_name":if disclose_configuration {vault.root().file_name().and_then(|s|s.to_str()).unwrap_or("RecallCard")} else {"RecallCard"},"protocol":"recallcard.conversation/1",
                "automation":active.map(|entry|json!({"grant_id":entry.id,"permission_revision":entry.permission_revision,"capture":entry.grant.auto_capture && capture_scope.is_some_and(|scope|entry.grant.capture_scopes.iter().any(|s|s==scope)),"recall":entry.grant.auto_recall && entry.grant.provider_disclosure && entry.grant.recall_scopes.iter().any(|scope|access.permits(scope)),"provider_disclosure":entry.grant.provider_disclosure,"platform":entry.grant.platform,"account_identity":"unverified"})),
                "installation_id":installation,"setup_required":setup_required,"state":entry.as_ref().map(|entry|if entry.revoked {"revoked"} else {"reachable"}).unwrap_or("reachable"),"account_identity":"unverified"}),
            );
        }
        if setup_required {
            return Err("此浏览器安装尚未授权，请用扩展显示的连接编号完成本机设置".into());
        }
        if reading {
            if let Some(entry) = entry {
                let authorization = connections::authorize(vault, &id, entry.permission_revision)
                    .map_err(|e| e.to_string())?;
                if !authorization.entry.grant.provider_disclosure {
                    return Err("未授权向此网站准备资料".into());
                }
                let scopes = entry
                    .grant
                    .recall_scopes
                    .into_iter()
                    .filter(|scope| access.permits(scope))
                    .collect();
                let result = services.read(vault, Access::new(scopes)?, name, args.clone())?;
                drop(authorization);
                connections::observe(
                    vault,
                    &id,
                    entry.permission_revision,
                    if name == "bootstrap" {
                        "bootstrap"
                    } else {
                        "read"
                    },
                )
                .map_err(|e| e.to_string())?;
                return Ok(result);
            }
            return services.read(vault, access.clone(), name, args.clone());
        }
        let object = args.as_object().ok_or("会话请求必须为对象")?;
        if name == "automatic_capture" || name == "authorized_read" {
            let fields: &[&str] = if name == "automatic_capture" {
                &["conversation", "permission_revision"]
            } else {
                &["action", "arguments", "permission_revision"]
            };
            if object.len() != fields.len()
                || object.keys().any(|key| !fields.contains(&key.as_str()))
            {
                return Err("自动请求不能附加授权、scope、路径或身份".into());
            }
            let revision = object["permission_revision"]
                .as_u64()
                .ok_or("自动请求缺少授权版本")?;
            let authorization =
                connections::authorize(vault, &id, revision).map_err(|e| e.to_string())?;
            let grant = &authorization.entry.grant;
            let (result, observed) = if name == "authorized_read" {
                if !grant.auto_recall || !grant.provider_disclosure {
                    return Err("此网站未授权自动准备资料".into());
                }
                let action = object["action"].as_str().ok_or("缺少只读动作")?;
                if !matches!(action, "bootstrap" | "search" | "read" | "sources") {
                    return Err("只允许四种只读资料操作".into());
                }
                let scopes = grant
                    .recall_scopes
                    .iter()
                    .filter(|scope| access.permits(scope))
                    .cloned()
                    .collect();
                (
                    services.read(
                        vault,
                        Access::new(scopes)?,
                        action,
                        object["arguments"].clone(),
                    )?,
                    if action == "bootstrap" {
                        "bootstrap"
                    } else {
                        "read"
                    },
                )
            } else {
                let scope = capture_scope.ok_or("本机启动器未授权捕获")?;
                if !grant.auto_capture || !grant.capture_scopes.iter().any(|s| s == scope) {
                    return Err("此网站未授权自动捕获到该范围".into());
                }
                let conversation =
                    crate::conversation::Conversation::parse(&object["conversation"].to_string())?;
                if conversation.source.platform != platform
                    || conversation.metadata["weaker_conversation_identity"] == true
                    || conversation
                        .messages
                        .iter()
                        .any(|m| m.metadata["weaker_identity"] == true)
                {
                    return Err("自动捕获需要明确的平台、会话与稳定消息身份".into());
                }
                if let Some(previous) =
                    conversation.metadata["chunk_previous_message"]["message_id"].as_str()
                {
                    if !vault.events()?.iter().any(|event| {
                        event.data.scope == scope
                            && event.data.source.platform == conversation.source.platform
                            && event.data.source.conversation_id
                                == conversation.source.conversation_id
                            && event.data.source.message_id == previous
                    }) {
                        return Err("分块前驱尚未在当前范围、平台和会话中保存".into());
                    }
                }
                let preview = crate::conversation::preview(vault, &conversation, scope)?;
                (
                    crate::conversation::save(
                        vault,
                        &conversation,
                        scope,
                        preview["approval_hash"].as_str().ok_or("捕获校验失败")?,
                    )?,
                    "capture",
                )
            };
            drop(authorization);
            connections::observe(vault, &id, revision, observed).map_err(|e| e.to_string())?;
            return Ok(result);
        }
        let authorization = entry
            .as_ref()
            .map(|entry| {
                connections::authorize(vault, &id, entry.permission_revision)
                    .map_err(|e| e.to_string())
            })
            .transpose()?;
        let scope =
            capture_scope.ok_or("本机连接尚未允许保存对话，请在桌面连接设置中明确选择保存范围")?;
        if authorization
            .as_ref()
            .is_some_and(|a| !a.entry.grant.capture_scopes.iter().any(|s| s == scope))
        {
            return Err("此连接的捕获范围已撤销".into());
        }
        let fields: &[&str] = if name == "capture_save" {
            &["conversation", "approval_hash"]
        } else {
            &["conversation"]
        };
        if object.len() != fields.len() || object.keys().any(|key| !fields.contains(&key.as_str()))
        {
            return Err("会话请求不能附加 scope、路径或未知参数".into());
        }
        let conversation =
            crate::conversation::Conversation::parse(&object["conversation"].to_string())?;
        if authorization.is_some() && conversation.source.platform != platform {
            return Err("会话来源与当前授权网站不一致".into());
        }
        if name == "capture_preview" {
            return crate::conversation::preview(vault, &conversation, scope);
        }
        crate::conversation::save(
            vault,
            &conversation,
            scope,
            object["approval_hash"]
                .as_str()
                .ok_or("请先预览并明确确认保存")?,
        )
    })();
    if outcome.is_err() {
        let platform = session.split(':').next().unwrap_or("");
        let id = connections::connection_key("browser", extension, platform, installation);
        if let Ok(Some(entry)) = connections::get(vault, &id) {
            if !entry.revoked {
                let _ = connections::observe(vault, &id, entry.permission_revision, "failed");
            }
        }
    }
    Some(outcome)
}

pub fn serve_native_service_io<R: Read, W: Write>(
    service: impl Fn(&str, Value) -> Result<Value>,
    extension: &str,
    origin: &str,
    reader: R,
    writer: W,
) -> Result<()> {
    serve_native_bound_service_io(
        |name, args, _session, _installation| service(name, args),
        |_, _| Ok(String::new()),
        extension,
        origin,
        reader,
        writer,
    )
}
fn serve_native_bound_service_io<R: Read, W: Write>(
    service: impl Fn(&str, Value, &str, Option<&str>) -> Result<Value>,
    freshness: impl Fn(&str, Option<&str>) -> Result<String>,
    extension: &str,
    origin: &str,
    mut reader: R,
    mut writer: W,
) -> Result<()> {
    if origin != extension_origin(extension)? {
        return Err("Native Messaging 来源扩展不在允许名单".into());
    }
    let mut cache: BTreeMap<String, (String, String, Value)> = BTreeMap::new();
    loop {
        let mut prefix = [0u8; 4];
        match reader.read(&mut prefix[..1]) {
            Ok(0) => return Ok(()),
            Ok(_) => {}
            Err(e) => return Err(e.to_string()),
        }
        reader
            .read_exact(&mut prefix[1..])
            .map_err(|_| "Native 消息头不完整")?;
        let length = u32::from_ne_bytes(prefix) as usize;
        if length == 0 || length > 256 * 1024 {
            return Err("Native 输入长度超出 1–262144 字节范围".into());
        }
        let mut bytes = vec![0u8; length];
        reader
            .read_exact(&mut bytes)
            .map_err(|_| "Native 消息正文不完整")?;
        let response = match serde_json::from_slice::<NativeRequest>(&bytes) {
            Err(_) => json!({"ok":false,"error":"Native 请求 JSON/schema 无效"}),
            Ok(request) => {
                let valid = request.protocol == "recallcard.action/1"
                    && request.request_id.len() <= 128
                    && !request.request_id.is_empty()
                    && (16..=256).contains(&request.nonce.len())
                    && !request.session_ref.is_empty()
                    && request.session_ref.len() <= 2048
                    && request
                        .installation_id
                        .as_deref()
                        .is_none_or(crate::application::connections::valid_installation_id);
                if !valid {
                    json!({"ok":false,"error":"协议、nonce、request_id 或会话绑定无效"})
                } else {
                    let key = format!("{}\0{}", request.session_ref, request.request_id);
                    let hash = crate::hash(&bytes);
                    let revision =
                        freshness(&request.session_ref, request.installation_id.as_deref())?;
                    if let Some((previous, saved_revision, response)) = cache.get_mut(&key) {
                        if *saved_revision != revision {
                            // 原结果立即释放，仅保留拒绝重放的占位；恢复授权也不复活旧请求。
                            *response = json!({"ok":false,"error":"授权已变化，不能重放旧结果"});
                            response.clone()
                        } else if *previous == hash {
                            if response["ok"] != true {
                                response.clone()
                            } else if matches!(
                                request.action.as_str(),
                                "bootstrap" | "search" | "read" | "sources" | "authorized_read"
                            ) {
                                // 只读重试仍重新检查事实、抑制与权限；不能借 request_id 重放已撤回正文。
                                match service(
                                    &request.action,
                                    request.arguments.clone().unwrap_or_else(|| json!({})),
                                    &request.session_ref,
                                    request.installation_id.as_deref(),
                                ) {
                                    Ok(current)
                                        if response.get("result") == Some(&current)
                                            && response["ok"] == true =>
                                    {
                                        response.clone()
                                    }
                                    _ => {
                                        *response = json!({"ok":false,"error":"资料或授权已变化，请使用新的 request_id 重新读取"});
                                        response.clone()
                                    }
                                }
                            } else {
                                response.clone()
                            }
                        } else {
                            json!({"ok":false,"error":"同一 request_id 对应不同请求"})
                        }
                    } else if cache.len() >= 120 {
                        json!({"ok":false,"error":"本连接请求达到上限，请重新连接"})
                    } else {
                        let response = match service(
                            &request.action,
                            request.arguments.unwrap_or_else(|| json!({})),
                            &request.session_ref,
                            request.installation_id.as_deref(),
                        ) {
                            Ok(result) => {
                                json!({"ok":true,"result":result,"request_id":request.request_id})
                            }
                            Err(error) => {
                                json!({"ok":false,"error":error,"request_id":request.request_id})
                            }
                        };
                        cache.insert(key, (hash, revision, response.clone()));
                        response
                    }
                }
            }
        };
        let encoded = serde_json::to_vec(&response).map_err(|e| e.to_string())?;
        if encoded.len() > 1024 * 1024 {
            return Err("Native 响应超过浏览器 1 MiB 上限".into());
        }
        writer
            .write_all(&(encoded.len() as u32).to_ne_bytes())
            .map_err(|e| e.to_string())?;
        writer.write_all(&encoded).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;
    }
}
pub fn prepare_install(
    vault: &Vault,
    scopes: Vec<String>,
    extension: &str,
    output: &Path,
) -> Result<Value> {
    prepare_install_with_ipc(vault, scopes, extension, output, None)
}

pub fn prepare_install_with_ipc(
    vault: &Vault,
    scopes: Vec<String>,
    extension: &str,
    output: &Path,
    ipc_endpoint: Option<PathBuf>,
) -> Result<Value> {
    let source = std::env::current_exe().map_err(|e| e.to_string())?;
    prepare_install_from_binary(
        vault,
        scopes,
        extension,
        output,
        ipc_endpoint,
        None,
        &source,
    )
}

pub fn prepare_install_from_binary(
    vault: &Vault,
    scopes: Vec<String>,
    extension: &str,
    output: &Path,
    ipc_endpoint: Option<PathBuf>,
    capture_scope: Option<String>,
    source: &Path,
) -> Result<Value> {
    prepare_install_configured_from_binary(
        vault,
        scopes,
        extension,
        output,
        NativeInstallOptions {
            ipc_endpoint,
            capture_scope,
            semantic_config: None,
        },
        source,
    )
}

/// 可信安装入口的固定配置；不会写入浏览器注册或扩展授权。
#[derive(Default)]
pub struct NativeInstallOptions {
    pub ipc_endpoint: Option<PathBuf>,
    pub capture_scope: Option<String>,
    pub semantic_config: Option<PathBuf>,
}

pub fn prepare_install_configured_from_binary(
    vault: &Vault,
    scopes: Vec<String>,
    extension: &str,
    output: &Path,
    options: NativeInstallOptions,
    source: &Path,
) -> Result<Value> {
    let NativeInstallOptions {
        ipc_endpoint,
        capture_scope,
        semantic_config,
    } = options;
    if let Some(path) = &semantic_config {
        if ipc_endpoint.is_some() {
            return Err("Native 离线语义配置与 IPC 不能同时指定".into());
        }
        load_native_semantic(path)?;
    }
    let origin = extension_origin(extension)?;
    let access = Access::new(scopes)?;
    if let Some(endpoint) = &ipc_endpoint {
        crate::ipc::Client::new(
            vault,
            &access,
            endpoint.clone(),
            std::time::Duration::from_secs(5),
        )?;
    }
    let vault_root = fs::canonicalize(vault.root()).map_err(|e| e.to_string())?;
    reject_links(&vault_root)?;
    reject_install_root_link(output)?;
    #[cfg(unix)]
    let existed = output.exists();
    fs::create_dir_all(output).map_err(|e| e.to_string())?;
    reject_install_root_link(output)?;
    let output = fs::canonicalize(output).map_err(|e| e.to_string())?;
    reject_links(&output)?;
    #[cfg(unix)]
    if !existed {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&output, fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    reject_shared_writes(&output)?;
    let executable = output.join(LAUNCHER_NAME);
    let config_path = output.join(CONFIG_NAME);
    let script = output.join("recallcard-native-host");
    let manifest = output.join("com.recallcard.host.json");
    let mut targets = vec![&executable, &config_path, &manifest];
    if !cfg!(windows) {
        targets.push(&script);
    }
    for target in &targets {
        reject_links(target)?;
        match fs::symlink_metadata(target) {
            Ok(_) => return Err("安装目录已包含生成文件，请选择新的目录以保留原配置".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    let config = LauncherConfig {
        schema_version: 1,
        host: HOST_NAME.into(),
        vault: vault_root,
        scopes: access.scopes(),
        extension_id: extension.into(),
        ipc_endpoint,
        capture_scope,
        semantic_config,
    };
    let config_bytes = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
    parse_launcher_config(&config_bytes)?;
    reject_links(source)?;
    // manifest 最后发布；每个文件原子新建，失败也不覆盖或删除用户的文件。
    // 失败目录可能包含本次已完成的副本，应检查后改用新的输出目录。
    let generated = (|| -> Result<()> {
        write_new(
            &executable,
            &mut File::open(source).map_err(|e| e.to_string())?,
            true,
        )?;
        write_new(&config_path, &mut config_bytes.as_slice(), false)?;
        if !cfg!(windows) {
            let executable_path = executable.to_str().ok_or("Native 启动器路径必须为 UTF-8")?;
            let quoted = format!("'{}'", executable_path.replace('\'', "'\\''"));
            let command = format!("#!/bin/sh\nexec {quoted} \"$@\"\n");
            write_new(&script, &mut command.as_bytes(), true)?;
        }
        let host_path = if cfg!(windows) { &executable } else { &script };
        let value = json!({"name":HOST_NAME,"description":"RecallCard 本机只读上下文桥","path":host_path,"type":"stdio","allowed_origins":[origin]});
        let manifest_bytes = serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?;
        write_new(&manifest, &mut manifest_bytes.as_slice(), false)?;
        #[cfg(unix)]
        File::open(&output)
            .and_then(|file| file.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    generated.map_err(|error| format!("生成 Native 文件失败：{error}；可能保留部分文件，请检查后选择新的输出目录，尚未注册浏览器"))?;
    Ok(json!({
        "ok":true,
        "manifest":manifest,
        "launcher":executable,
        "config":config_path,
        "wrapper":if cfg!(windows) { Value::Null } else { json!(script) },
        "registered":false,
        "note":"只生成待检查文件；请按浏览器与操作系统文档手动注册，不会修改浏览器权限或系统配置"
    }))
}

/// CLI 的只读 IPC 启动方式同样经过安装授权；不能绕过连接撤销。
pub fn serve_native_read_service_io<R: Read, W: Write>(
    vault: &Vault,
    access: Access,
    extension: &str,
    origin: &str,
    service: impl Fn(&str, Value) -> Result<Value>,
    reader: R,
    writer: W,
) -> Result<()> {
    serve_native_bound_service_io(
        |name, args, session, installation| {
            browser_operation(
                vault,
                &access,
                &NativeServices::default(),
                extension,
                (session, installation),
                name,
                &args,
            )
            .unwrap_or_else(|| service(name, args))
        },
        |session, installation| connection_revision(vault, extension, session, installation),
        extension,
        origin,
        reader,
        writer,
    )
}
