//! 本机用户管理的连接授权与实际观察；配置不会冒充握手或模型已接受。
use super::{AppError, AppResult, ErrorCode};
use crate::{model::validate_scope, Vault};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    path::PathBuf,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConnectionGrant {
    pub client_kind: String,
    pub host_identity: String,
    pub installation_id: Option<String>,
    pub platform: String,
    pub recall_scopes: Vec<String>,
    pub capture_scopes: Vec<String>,
    pub provider_disclosure: bool,
    pub auto_capture: bool,
    pub auto_recall: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionEntry {
    pub id: String,
    pub grant: ConnectionGrant,
    pub permission_revision: u64,
    pub state: String,
    pub configured_at: DateTime<Utc>,
    pub last_handshake_at: Option<DateTime<Utc>>,
    pub last_bootstrap_at: Option<DateTime<Utc>>,
    pub last_read_at: Option<DateTime<Utc>>,
    pub last_capture_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub revoked: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingRequest {
    pub request_id: String,
    pub host_identity: String,
    pub installation_id: String,
    pub platform: String,
    pub requested_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub recall_scope_cap: Vec<String>,
    pub capture_scope_cap: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Store {
    vault_identity: String,
    entries: Vec<ConnectionEntry>,
    #[serde(default)]
    pending_pairings: Vec<PairingRequest>,
}

fn error(code: ErrorCode, text: &str) -> AppError {
    AppError::new(
        code,
        text,
        "在连接设置中核对客户端、网站、范围与授权，再重试",
    )
}
fn storage(_: impl std::fmt::Display) -> AppError {
    error(ErrorCode::Storage, "无法读取或更新本机连接状态；授权未确认")
}
struct ConnectionLock(File);
impl Drop for ConnectionLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
fn paths(vault: &Vault) -> AppResult<(PathBuf, ConnectionLock)> {
    paths_with_lock(vault, false)
}
fn paths_with_lock(vault: &Vault, shared: bool) -> AppResult<(PathBuf, ConnectionLock)> {
    let root = vault.state_dir().map_err(storage)?;
    let lock = root.join("connections.lock");
    crate::vault::reject_symlink(&lock).map_err(storage)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock)
        .map_err(storage)?;
    if !file.metadata().map_err(storage)?.is_file() {
        return Err(storage("invalid connection lock"));
    }
    let until =
        std::time::Instant::now() + std::time::Duration::from_secs(if shared { 1 } else { 5 });
    loop {
        let acquired = if shared {
            file.try_lock_shared()
        } else {
            file.try_lock()
        };
        match acquired {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < until => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                let mut busy = error(ErrorCode::Conflict, "连接设置仍在处理中，请稍后重试");
                busy.retryable = true;
                return Err(busy);
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(storage(error)),
        }
    }
    let path = root.join("connections-v1.json");
    crate::vault::reject_symlink(&path).map_err(storage)?;
    Ok((path, ConnectionLock(file)))
}
fn load(vault: &Vault, path: &PathBuf) -> AppResult<Store> {
    let identity = crate::conversation::connection_id(vault, "connections").map_err(storage)?;
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Store {
                vault_identity: identity,
                entries: Vec::new(),
                pending_pairings: Vec::new(),
            })
        }
        Err(e) => return Err(storage(e)),
    };
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(storage)?;
    if bytes.len() > 1024 * 1024 {
        return Err(storage("size"));
    }
    let mut store: Store = serde_json::from_slice(&bytes).map_err(storage)?;
    if store.vault_identity != identity {
        return Err(error(
            ErrorCode::ConsentRequired,
            "资料库身份已变化，旧连接授权失效；请在新位置重新设置连接",
        ));
    }
    if store.entries.len() > 128 || store.pending_pairings.len() > 16 {
        return Err(storage("size"));
    }
    for entry in &mut store.entries {
        if entry.grant.client_kind == "browser" && entry.grant.installation_id.is_none() {
            entry.revoked = true;
            entry.state = "needs_setup".into();
        } else {
            validate(&entry.grant)?;
        }
        if entry.id
            != connection_key(
                &entry.grant.client_kind,
                &entry.grant.host_identity,
                &entry.grant.platform,
                entry.grant.installation_id.as_deref(),
            )
            || entry.permission_revision == 0
        {
            return Err(storage("identity"));
        }
    }
    Ok(store)
}
fn validate(grant: &ConnectionGrant) -> AppResult<()> {
    match (grant.client_kind.as_str(), grant.platform.as_str()) {
        ("browser", "chatgpt" | "deepseek") => {
            if !grant
                .installation_id
                .as_deref()
                .is_some_and(valid_installation_id)
            {
                return Err(error(
                    ErrorCode::ConsentRequired,
                    "请先填写扩展弹窗中的本机安装编号；不能使用其他浏览器配置的授权",
                ));
            }
            crate::native::extension_origin(&grant.host_identity).map_err(|_| {
                error(
                    ErrorCode::InvalidRequest,
                    "浏览器连接必须绑定准确的 Chrome 扩展 ID",
                )
            })?;
        }
        ("claude_code", "claude_code")
            if grant.host_identity == "claude-code"
                && grant
                    .installation_id
                    .as_deref()
                    .is_none_or(valid_installation_id) => {}
        ("codex", "codex")
            if grant.host_identity == "codex"
                && grant
                    .installation_id
                    .as_deref()
                    .is_none_or(valid_installation_id) => {}
        ("chatgpt_mcp", "chatgpt")
            if grant.host_identity == "openai-chatgpt" && grant.installation_id.is_none() => {}
        _ => {
            return Err(error(
                ErrorCode::InvalidRequest,
                "此客户端或网站尚未支持自动接入",
            ))
        }
    }
    for scopes in [&grant.recall_scopes, &grant.capture_scopes] {
        if scopes.len() > 32 {
            return Err(error(
                ErrorCode::ResourceLimit,
                "每种授权最多允许 32 个范围",
            ));
        }
        let mut unique = std::collections::BTreeSet::new();
        for scope in scopes {
            validate_scope(scope).map_err(|_| error(ErrorCode::InvalidRequest, "连接范围无效"))?;
            if !unique.insert(scope) {
                return Err(error(ErrorCode::InvalidRequest, "连接范围不能重复"));
            }
        }
    }
    if grant.auto_capture && (grant.capture_scopes.len() != 1 || grant.client_kind != "browser") {
        return Err(error(
            ErrorCode::InvalidRequest,
            "自动捕获需要一个明确的浏览器保存范围",
        ));
    }
    if grant.auto_recall && (grant.recall_scopes.is_empty() || !grant.provider_disclosure) {
        return Err(error(
            ErrorCode::ConsentRequired,
            "自动准备资料需要读取范围及向所选网站或宿主模型提供资料的授权",
        ));
    }
    if matches!(
        grant.client_kind.as_str(),
        "claude_code" | "codex" | "chatgpt_mcp"
    ) && !grant.capture_scopes.is_empty()
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "原生 MCP 连接仅提供只读资料，不能授予捕获权限",
        ));
    }
    Ok(())
}
pub fn valid_installation_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}
pub fn connection_key(
    kind: &str,
    host: &str,
    platform: &str,
    installation: Option<&str>,
) -> String {
    let binding = match installation {
        Some(id) => format!("{kind}\0{host}\0{platform}\0{id}"),
        None => format!("{kind}\0{host}\0{platform}"),
    };
    format!("conn_{}", crate::hash(binding.as_bytes()))
}
pub fn has_browser_binding(vault: &Vault, extension: &str) -> AppResult<bool> {
    let (path, _lock) = paths(vault)?;
    Ok(load(vault, &path)?.entries.iter().any(|entry| {
        entry.grant.client_kind == "browser" && entry.grant.host_identity == extension
    }))
}

pub fn inventory(vault: &Vault) -> AppResult<Value> {
    // 轮询读取同一完整状态；共享读者不互斥，撤权提交期间短暂有界等待。
    let (path, _lock) = paths_with_lock(vault, true)?;
    let store = load(vault, &path)?;
    Ok(
        json!({"entries":store.entries,"pending_pairings":store.pending_pairings.into_iter().filter(|request|request.expires_at>Utc::now()).collect::<Vec<_>>(),"supported_protocols":["recallcard.action/1","recallcard.conversation/1","claude-code.SessionStart"],"identity_notice":"浏览器连接绑定本机扩展与网站；不读取账号凭证，无法核验网站登录账号。最终网页发送始终由人完成。"}),
    )
}
pub fn configure(
    vault: &Vault,
    grant: ConnectionGrant,
    expected_revision: Option<u64>,
) -> AppResult<ConnectionEntry> {
    validate(&grant)?;
    let (path, _lock) = paths(vault)?;
    let mut store = load(vault, &path)?;
    let entry = configure_store(&mut store, grant, expected_revision)?;
    if Some(entry.permission_revision) != expected_revision {
        super::agent_entry::invalidate(vault, &entry.id).map_err(storage)?;
    }
    save_store(vault, &path, &store)?;
    Ok(entry)
}
fn configure_store(
    store: &mut Store,
    grant: ConnectionGrant,
    expected_revision: Option<u64>,
) -> AppResult<ConnectionEntry> {
    let id = connection_key(
        &grant.client_kind,
        &grant.host_identity,
        &grant.platform,
        grant.installation_id.as_deref(),
    );
    let previous = store.entries.iter().position(|e| e.id == id);
    if previous.map(|i| store.entries[i].permission_revision) != expected_revision {
        return Err(error(ErrorCode::Conflict, "连接授权已变化，请刷新后再确认"));
    }
    if let Some(i) = previous {
        if !store.entries[i].revoked && store.entries[i].grant == grant {
            return Ok(store.entries[i].clone());
        }
    }
    if previous.is_none() && store.entries.len() >= 128 {
        return Err(error(ErrorCode::ResourceLimit, "本机连接数量已达 128 个"));
    }
    let entry = ConnectionEntry {
        id,
        grant,
        permission_revision: expected_revision.unwrap_or(0) + 1,
        state: "configured_unverified".into(),
        configured_at: Utc::now(),
        last_handshake_at: None,
        last_bootstrap_at: None,
        last_read_at: None,
        last_capture_at: None,
        last_error: None,
        revoked: false,
    };
    if let Some(i) = previous {
        store.entries[i] = entry.clone();
    } else {
        store.entries.push(entry.clone());
    }
    Ok(entry)
}
pub fn revoke(vault: &Vault, id: &str, expected_revision: u64) -> AppResult<ConnectionEntry> {
    let (path, _lock) = paths(vault)?;
    let mut store = load(vault, &path)?;
    let entry = store
        .entries
        .iter_mut()
        .find(|entry| entry.id == id)
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "没有这个连接"))?;
    if entry.permission_revision != expected_revision {
        return Err(error(ErrorCode::Conflict, "连接授权已变化，请刷新后重试"));
    }
    entry.permission_revision += 1;
    entry.revoked = true;
    entry.state = "revoked".into();
    let result = entry.clone();
    super::agent_entry::invalidate(vault, id).map_err(storage)?;
    save_store(vault, &path, &store)?;
    Ok(result)
}
pub fn get(vault: &Vault, id: &str) -> AppResult<Option<ConnectionEntry>> {
    let (path, _lock) = paths(vault)?;
    Ok(load(vault, &path)?
        .entries
        .into_iter()
        .find(|entry| entry.id == id))
}
/// 只接受程序内部固定的观察名，不保存网页文字、查询正文、令牌或错误原文。
pub fn observe(vault: &Vault, id: &str, revision: u64, event: &str) -> AppResult<()> {
    let (path, _lock) = paths(vault)?;
    let mut store = load(vault, &path)?;
    let entry = store
        .entries
        .iter_mut()
        .find(|entry| entry.id == id)
        .ok_or_else(|| error(ErrorCode::ConsentRequired, "连接尚未配置"))?;
    if entry.revoked || entry.permission_revision != revision {
        return Err(error(ErrorCode::PermissionDenied, "连接授权已撤销或变化"));
    }
    let now = Some(Utc::now());
    match event {
        "handshake" => {
            entry.last_handshake_at = now;
            entry.state = "reachable".into();
        }
        "bootstrap" => {
            entry.last_bootstrap_at = now;
            entry.state = "read_succeeded".into();
        }
        "read" => {
            entry.last_read_at = now;
            entry.state = "read_succeeded".into();
        }
        "capture" => {
            entry.last_capture_at = now;
            entry.state = "reachable".into();
        }
        "failed" => {
            entry.last_error = Some("最近一次本机操作未完成，请重试检查连接".into());
            entry.state = "failed".into();
        }
        _ => return Err(error(ErrorCode::InvalidRequest, "不支持的连接观察")),
    }
    if event != "failed" {
        entry.last_error = None;
    }
    save_store(vault, &path, &store)
}

/// 持有授权锁直到一次读/写结束；撤权返回后不能再有旧授权的在途提交。
pub struct Authorization {
    pub entry: ConnectionEntry,
    _lock: ConnectionLock,
}
pub fn authorize(vault: &Vault, id: &str, revision: u64) -> AppResult<Authorization> {
    let (path, lock) = paths(vault)?;
    let entry = load(vault, &path)?
        .entries
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| error(ErrorCode::ConsentRequired, "此连接尚未授权"))?;
    if entry.revoked || entry.permission_revision != revision {
        return Err(error(ErrorCode::PermissionDenied, "连接授权已撤销或变化"));
    }
    Ok(Authorization { entry, _lock: lock })
}

/// MCP 每次调用重读用户授权；可信启动参数仍是范围硬上限。
pub fn invoke_agent(
    vault: &Vault,
    id: &str,
    access: &crate::policy::Access,
    name: &str,
    args: Value,
) -> crate::model::Result<Value> {
    let entry = get(vault, id)
        .map_err(|e| e.to_string())?
        .ok_or("Agent 连接尚未配置")?;
    let authorization =
        authorize(vault, id, entry.permission_revision).map_err(|e| e.to_string())?;
    let grant = &authorization.entry.grant;
    if !matches!(
        grant.client_kind.as_str(),
        "claude_code" | "codex" | "chatgpt_mcp"
    ) || !grant.auto_recall
        || !grant.provider_disclosure
    {
        return Err("此客户端的读取或接收方授权已暂停".into());
    }
    let scopes = grant
        .recall_scopes
        .iter()
        .filter(|scope| access.permits(scope))
        .cloned()
        .collect();
    let result = crate::transport::invoke(
        &crate::context::Context::new(vault, crate::policy::Access::new(scopes)?),
        name,
        args,
    );
    drop(authorization);
    observe(
        vault,
        id,
        entry.permission_revision,
        if result.is_ok() {
            if name == "bootstrap" {
                "bootstrap"
            } else {
                "read"
            }
        } else {
            "failed"
        },
    )
    .map_err(|e| e.to_string())?;
    result
}

fn save_store(vault: &Vault, path: &std::path::Path, store: &Store) -> AppResult<()> {
    if serde_json::to_vec_pretty(store).map_err(storage)?.len() > 900 * 1024 {
        return Err(error(
            ErrorCode::ResourceLimit,
            "本机连接设置超过安全大小上限",
        ));
    }
    vault.write_replace(path, store).map_err(storage)
}
/// Native 仅能排队一个零权限请求；范围硬上限来自可信启动器，不来自网页。
pub fn request_pairing(
    vault: &Vault,
    extension: &str,
    installation: &str,
    platform: &str,
    recall_scope_cap: Vec<String>,
    capture_scope_cap: Vec<String>,
) -> AppResult<PairingRequest> {
    let candidate = ConnectionGrant {
        client_kind: "browser".into(),
        host_identity: extension.into(),
        installation_id: Some(installation.into()),
        platform: platform.into(),
        recall_scopes: recall_scope_cap.clone(),
        capture_scopes: capture_scope_cap.clone(),
        provider_disclosure: false,
        auto_capture: false,
        auto_recall: false,
    };
    validate(&candidate)?;
    let (path, _lock) = paths(vault)?;
    let mut store = load(vault, &path)?;
    let now = Utc::now();
    store
        .pending_pairings
        .retain(|request| request.expires_at > now);
    if let Some(existing) = store.pending_pairings.iter().find(|request| {
        request.host_identity == extension
            && request.installation_id == installation
            && request.platform == platform
            && request.recall_scope_cap == recall_scope_cap
            && request.capture_scope_cap == capture_scope_cap
    }) {
        return Ok(existing.clone());
    }
    if store.pending_pairings.len() >= 16 {
        return Err(error(
            ErrorCode::ResourceLimit,
            "待确认连接已达16个，请在桌面处理或等待10分钟过期",
        ));
    }
    let request = PairingRequest {
        request_id: format!("pair_{}", uuid::Uuid::new_v4()),
        host_identity: extension.into(),
        installation_id: installation.into(),
        platform: platform.into(),
        requested_at: now,
        expires_at: now + chrono::Duration::minutes(10),
        recall_scope_cap,
        capture_scope_cap,
    };
    store.pending_pairings.push(request.clone());
    save_store(vault, &path, &store)?;
    Ok(request)
}
/// 只能从本机用户界面/CLI批准；Native、MCP和模型请求均不提供此写入口。
pub fn approve_pairing(
    vault: &Vault,
    request_id: &str,
    grant: ConnectionGrant,
    expected_revision: Option<u64>,
) -> AppResult<ConnectionEntry> {
    validate(&grant)?;
    let (path, _lock) = paths(vault)?;
    let mut store = load(vault, &path)?;
    let request = store
        .pending_pairings
        .iter()
        .find(|request| request.request_id == request_id && request.expires_at > Utc::now())
        .ok_or_else(|| {
            error(
                ErrorCode::ConsentRequired,
                "连接请求已过期或不存在，请从扩展重新发起",
            )
        })?;
    if grant.client_kind != "browser"
        || grant.host_identity != request.host_identity
        || grant.installation_id.as_deref() != Some(request.installation_id.as_str())
        || grant.platform != request.platform
        || grant
            .recall_scopes
            .iter()
            .any(|scope| !request.recall_scope_cap.contains(scope))
        || grant
            .capture_scopes
            .iter()
            .any(|scope| !request.capture_scope_cap.contains(scope))
    {
        return Err(error(
            ErrorCode::PermissionDenied,
            "批准的安装、网站或范围与原始连接请求不一致",
        ));
    }
    let entry = configure_store(&mut store, grant, expected_revision)?;
    store
        .pending_pairings
        .retain(|request| request.request_id != request_id);
    if Some(entry.permission_revision) != expected_revision {
        super::agent_entry::invalidate(vault, &entry.id).map_err(storage)?;
    }
    save_store(vault, &path, &store)?;
    Ok(entry)
}
