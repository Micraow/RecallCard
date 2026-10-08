//! 连接状态与真实本机试读。配置、试读、宿主读取是三种独立证据。
use super::{agent_entry, connections, AppError, AppResult, ErrorCode};
use crate::Vault;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionHealth {
    pub connection_id: String,
    pub permission_revision: u64,
    pub installation: String,
    pub readiness: String,
    pub capture: String,
    pub checked_at: DateTime<Utc>,
    pub last_read_at: Option<DateTime<Utc>>,
    pub last_capture_at: Option<DateTime<Utc>>,
    pub last_local_verified_at: Option<DateTime<Utc>>,
    pub verification_scope: String,
    pub message: String,
    pub read_verification: Option<Value>,
}
#[derive(Debug, Serialize, Deserialize)]
struct CheckReceipt {
    connection_id: String,
    permission_revision: u64,
    scopes: Vec<String>,
    checked_at: DateTime<Utc>,
    success: bool,
    evidence: Value,
}
fn error(code: ErrorCode, message: &str) -> AppError {
    AppError::new(code, message, "刷新连接状态，检查本机程序和资料范围后重试")
}
fn storage(_: impl std::fmt::Display) -> AppError {
    error(ErrorCode::Storage, "本机连接核验记录不可读取")
}
fn receipt_path(vault: &Vault, id: &str) -> AppResult<PathBuf> {
    agent_entry::path(vault, id)?;
    let path = vault
        .state_dir()
        .map_err(storage)?
        .join(format!("connection-check-{id}.json"));
    crate::vault::reject_symlink(&path).map_err(storage)?;
    Ok(path)
}
fn receipt(vault: &Vault, id: &str) -> AppResult<Option<CheckReceipt>> {
    let path = receipt_path(vault, id)?;
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(storage(e)),
    };
    let mut bytes = Vec::new();
    file.take(16 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(storage)?;
    if bytes.len() > 16 * 1024 {
        return Err(storage("size"));
    }
    Ok(Some(serde_json::from_slice(&bytes).map_err(storage)?))
}
/// CLI 与桌面共享确认后的一次安装和本机试读；部分结果不能被误报为未写入。
pub fn apply(vault: &Vault, plan_id: &str) -> AppResult<Value> {
    let plan = super::agent_install::get_plan(vault, plan_id)?;
    let result = super::agent_install::apply(vault, plan_id)?;
    let checked = verify(vault, &result.connection_id, &plan.scope, &plan.binary_path);
    let (health, verification_error) = match checked {
        Ok(health) => (Some(health), None),
        Err(error) => (
            health(vault, &result.connection_id, &plan.scope).ok(),
            Some(error),
        ),
    };
    Ok(
        json!({"connection_id":result.connection_id,"configuration":"written","paths":result.files,"backup_paths":result.backup_paths,"health":health,"verification_error":verification_error,"notices":result.notices,"host_verified":false}),
    )
}
pub fn health(vault: &Vault, id: &str, scope: &str) -> AppResult<ConnectionHealth> {
    let entry = connections::get(vault, id)?
        .ok_or_else(|| error(ErrorCode::ConsentRequired, "此连接尚未配置"))?;
    if !entry
        .grant
        .recall_scopes
        .iter()
        .chain(&entry.grant.capture_scopes)
        .any(|s| s == scope)
    {
        return Err(error(ErrorCode::PermissionDenied, "当前空间不可读取此连接"));
    }
    let local = receipt(vault, id)?.filter(|r| {
        r.connection_id == id
            && r.permission_revision == entry.permission_revision
            && r.scopes == [scope]
    });
    let observed = entry.last_read_at.or(entry.last_bootstrap_at);
    let local_success = local.as_ref().is_some_and(|r| r.success);
    let installation = if entry.grant.client_kind == "browser" {
        if entry.last_handshake_at.is_some() {
            "configured".to_string()
        } else {
            "not_inspected".into()
        }
    } else {
        let inspection = super::agent_install::inspect(vault, id)?;
        if inspection.scope.as_ref().is_some_and(|installed| {
            entry.grant.recall_scopes.as_slice() != std::slice::from_ref(installed)
        }) {
            "configuration_changed".into()
        } else {
            inspection.configuration
        }
    };
    let (readiness, message) = if entry.revoked {
        ("revoked", "连接已撤销；此前被宿主读取的内容无法收回")
    } else if !entry.grant.auto_recall || !entry.grant.provider_disclosure {
        ("paused", "自动读取未启用；不会向此宿主提供背景")
    } else if installation == "configuration_changed" {
        (
            "local_unavailable",
            "项目配置、读取组件或权限版本已变化；请重新预览确认接入",
        )
    } else if entry.state == "failed" || entry.last_error.is_some() {
        (
            "local_unavailable",
            "最近一次客户端操作未完成；历史读取回执不能证明当前连接正常",
        )
    } else if local.as_ref().is_some_and(|r| !r.success) {
        (
            "local_unavailable",
            "本机试读未完成，请检查程序或资料库后重试",
        )
    } else if observed.is_some() {
        (
            "read_verified",
            "已收到带此连接编号的客户端读取；不代表模型已理解或采用",
        )
    } else if local_success {
        (
            "read_verified",
            "本机真实CLI试读通过，仍等待宿主确认并发起首次读取",
        )
    } else {
        ("awaiting_host", "等待宿主确认连接并读取，可先进行本机试读")
    };
    let capture = if !entry.grant.auto_capture || entry.grant.capture_scopes.is_empty() {
        "disabled"
    } else if entry.last_capture_at.is_some() {
        "verified"
    } else {
        "enabled_unverified"
    };
    Ok(ConnectionHealth {
        connection_id: id.into(),
        permission_revision: entry.permission_revision,
        installation,
        readiness: readiness.into(),
        capture: capture.into(),
        checked_at: Utc::now(),
        last_read_at: observed,
        last_capture_at: entry.last_capture_at,
        last_local_verified_at: local.as_ref().filter(|r| r.success).map(|r| r.checked_at),
        verification_scope: if observed.is_some() {
            "client_request"
        } else if local_success {
            "local_cli"
        } else {
            "none"
        }
        .into(),
        message: message.into(),
        read_verification: local.filter(|r| r.success).map(|r| r.evidence),
    })
}
/// binary 由 CLI 的 current_exe 或 GUI 的同安装受信任路径提供，绝不来自网页/模型。
/// 只启动 RecallCard，不执行 Codex、Claude 或任何模型程序。
pub fn verify(vault: &Vault, id: &str, scope: &str, binary: &Path) -> AppResult<ConnectionHealth> {
    let entry = connections::get(vault, id)?
        .ok_or_else(|| error(ErrorCode::ConsentRequired, "此连接尚未授权"))?;
    if entry.revoked
        || !entry.grant.auto_recall
        || !entry.grant.provider_disclosure
        || !entry.grant.recall_scopes.iter().any(|s| s == scope)
    {
        return Err(error(
            ErrorCode::PermissionDenied,
            "此连接尚未允许当前范围的读取",
        ));
    }
    let inspection = super::agent_install::inspect(vault, id)?;
    if inspection.configuration == "configuration_changed"
        || inspection.scope.as_ref().is_some_and(|installed| {
            entry.grant.recall_scopes.as_slice() != std::slice::from_ref(installed)
        })
    {
        return Err(error(
            ErrorCode::Conflict,
            "配置或读取组件已变化，请重新预览确认；不会用另一个程序代替试读",
        ));
    }
    let binary = inspection.binary_path.as_deref().unwrap_or(binary);
    let expected = if cfg!(windows) {
        "recallcard.exe"
    } else {
        "recallcard"
    };
    if !binary.is_absolute()
        || binary.file_name().and_then(|s| s.to_str()) != Some(expected)
        || !binary.is_file()
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "没有找到同安装的RecallCard读取程序",
        ));
    }
    crate::vault::reject_symlink(binary).map_err(storage)?;
    let mut child = Command::new(binary)
        .args(["--vault"])
        .arg(vault.root())
        .args([
            "--json",
            "connection-context",
            id,
            "--scope",
            scope,
            "--budget-bytes",
            "4096",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| error(ErrorCode::Storage, "无法启动本机读取程序"))?;
    let stdout = child.stdout.take().ok_or_else(|| storage("stdout"))?;
    let output = thread::spawn(move || {
        let mut b = Vec::new();
        stdout.take(64 * 1024 + 1).read_to_end(&mut b).map(|_| b)
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if start.elapsed() < Duration::from_secs(15) => {
                thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let parsed = output
        .join()
        .ok()
        .and_then(|r| r.ok())
        .filter(|b| b.len() <= 64 * 1024)
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let value = parsed.as_ref().and_then(|r| r.get("result"));
    let success = status.is_some_and(|s| s.success())
        && parsed.as_ref().is_some_and(|r| r["ok"] == true)
        && value.is_some_and(|v| {
            v["schema"] == agent_entry::SCHEMA
                && v["connection_id"] == id
                && v["permission_revision"] == entry.permission_revision
                && v["scopes"] == json!([scope])
                && v["snapshot"].as_str().is_some_and(|s| s.len() == 64)
        });
    let evidence = if success {
        let v = value.unwrap();
        json!({"snapshot":v["snapshot"],"profile_items":v["profile"].as_array().map_or(0,Vec::len),"source_sessions":v["coverage"]["source_sessions"],"file_path":v["file_path"],"origin":"actual_cli_subprocess","host_verified":false})
    } else {
        json!({"reason":if status.is_none(){"timeout_or_wait_failed"}else{"read_failed"},"origin":"actual_cli_subprocess","host_verified":false})
    };
    let _authorization = connections::authorize(vault, id, entry.permission_revision)?;
    vault
        .write_replace(
            &receipt_path(vault, id)?,
            &CheckReceipt {
                connection_id: id.into(),
                permission_revision: entry.permission_revision,
                scopes: vec![scope.into()],
                checked_at: Utc::now(),
                success,
                evidence,
            },
        )
        .map_err(storage)?;
    drop(_authorization);
    health(vault, id, scope)
}
