//! 浏览器 Native Messaging：只读，范围与扩展 ID 来自本机配置。
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
    serve_native(
        &vault,
        Access::new(config.scopes)?,
        &config.extension_id,
        origin,
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
    mut reader: R,
    mut writer: W,
) -> Result<()> {
    if origin != extension_origin(extension)? {
        return Err("Native Messaging 来源扩展不在允许名单".into());
    }
    let context = Context::new(vault, access);
    let mut cache: BTreeMap<String, (String, Value)> = BTreeMap::new();
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
                    && request.session_ref.len() <= 2048;
                if !valid {
                    json!({"ok":false,"error":"协议、nonce、request_id 或会话绑定无效"})
                } else {
                    let key = format!("{}\0{}", request.session_ref, request.request_id);
                    let hash = crate::hash(&bytes);
                    if let Some((previous, response)) = cache.get(&key) {
                        if *previous == hash {
                            response.clone()
                        } else {
                            json!({"ok":false,"error":"同一 request_id 对应不同请求"})
                        }
                    } else if cache.len() >= 120 {
                        json!({"ok":false,"error":"本连接请求达到上限，请重新连接"})
                    } else {
                        let response = match invoke(
                            &context,
                            &request.action,
                            request.arguments.unwrap_or_else(|| json!({})),
                        ) {
                            Ok(result) => {
                                json!({"ok":true,"result":result,"request_id":request.request_id})
                            }
                            Err(error) => {
                                json!({"ok":false,"error":error,"request_id":request.request_id})
                            }
                        };
                        cache.insert(key, (hash, response.clone()));
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
    let origin = extension_origin(extension)?;
    let access = Access::new(scopes)?;
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
    };
    let config_bytes = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
    parse_launcher_config(&config_bytes)?;
    let source = std::env::current_exe().map_err(|e| e.to_string())?;
    reject_links(&source)?;
    // manifest 最后发布；每个文件原子新建，失败也不覆盖或删除用户的文件。
    // 失败目录可能包含本次已完成的副本，应检查后改用新的输出目录。
    let generated = (|| -> Result<()> {
        write_new(
            &executable,
            &mut File::open(&source).map_err(|e| e.to_string())?,
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
