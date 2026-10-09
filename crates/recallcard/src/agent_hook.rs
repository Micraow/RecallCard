//! Claude Code SessionStart 的有界只读适配器；输入只描述事件，不能授予权限。
use crate::{
    context::{BootstrapArgs, Context},
    model::Result,
    policy::Access,
    Vault,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::Read;

pub const MAX_HOOK_INPUT_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
struct SessionStartInput {
    hook_event_name: String,
    source: SessionSource,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum SessionSource {
    Startup,
    Resume,
    Compact,
    Clear,
    Fork,
}

/// 固定 Vault、授权范围和预算必须来自可信启动配置，不能从 Hook JSON 取值。
///
/// 未使用的宿主字段由 serde 跳过；不打开 transcript_path/cwd，不执行命令，
/// 不安装配置、不写事实源、不捕获会话、不运行 Dream、不调用模型或网络。
/// Bootstrap 的共享读锁可能使用已有的本机状态目录，这不构成事实源写入。
/// 返回值必须作为单个 JSON 对象写到 stdout；调用者应把错误只写到 stderr。
pub fn session_start<R: Read>(
    vault: &Vault,
    access: Access,
    budget_tokens: usize,
    reader: R,
) -> Result<Value> {
    if !(512..=32768).contains(&budget_tokens) {
        return Err("Hook 预算必须在 512–32768 之间，单位为保守 UTF-8 字节上界".into());
    }
    let mut bytes = Vec::new();
    reader
        .take(MAX_HOOK_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取 Hook 输入".to_string())?;
    if bytes.len() > MAX_HOOK_INPUT_BYTES {
        return Err("Hook 输入超过 64 KiB 上限".into());
    }
    let input: SessionStartInput = serde_json::from_slice(&bytes)
        .map_err(|_| "Hook 输入不是受支持的 SessionStart JSON".to_string())?;
    if input.hook_event_name != "SessionStart" {
        return Err("Hook 仅接受 SessionStart 事件".into());
    }
    // source 只选择受支持的生命周期入口；绝不把 session id/时间/路径放进稳定前缀。
    match input.source {
        SessionSource::Startup
        | SessionSource::Resume
        | SessionSource::Compact
        | SessionSource::Clear
        | SessionSource::Fork => {}
    }
    let bootstrap = Context::new(vault, access)
        .bootstrap(BootstrapArgs { budget_tokens })
        .map_err(|_| "无法生成授权范围内的启动资料，请单独运行 doctor 检查 Vault".to_string())?;
    let stable_text = bootstrap["stable_text"]
        .as_str()
        .ok_or_else(|| "启动资料缺少稳定文本".to_string())?;
    let output = json!({
        "hookSpecificOutput": {
            "hookEventName": "SessionStart",
            // 动态覆盖量放在稳定前缀之后；普通捕获不改变 bootstrap_version。
            "additionalContext": format!("{stable_text}\n{}",bootstrap["activity_text"].as_str().unwrap_or_default()),
        }
    });
    let encoded = serde_json::to_vec(&output).map_err(|_| "无法序列化 Hook 输出".to_string())?;
    if encoded.len() > budget_tokens {
        return Err("Hook 输出超过固定预算".into());
    }
    Ok(output)
}

/// 新连接流程：客户端身份从可信启动参数固定，stdin 不能选择或扩大授权。
/// 观察只证明宿主进程取走了启动输出，不证明模型理解或采用。
pub fn session_start_connected<R: Read>(
    vault: &Vault,
    connection_id: &str,
    access: Access,
    budget_tokens: usize,
    reader: R,
) -> Result<Value> {
    use crate::application::connections;
    let entry = connections::get(vault, connection_id)
        .map_err(|e| e.to_string())?
        .ok_or("Agent 连接尚未配置")?;
    let authorization = connections::authorize(vault, connection_id, entry.permission_revision)
        .map_err(|e| e.to_string())?;
    let grant = &authorization.entry.grant;
    if grant.client_kind != "claude_code" || !grant.auto_recall || !grant.provider_disclosure {
        return Err("此 Agent 未授权在生命周期自动接收背景资料".into());
    }
    let result = session_start(
        vault,
        Access::new(
            grant
                .recall_scopes
                .iter()
                .filter(|scope| access.permits(scope))
                .cloned()
                .collect(),
        )?,
        budget_tokens,
        reader,
    );
    drop(authorization);
    connections::observe(
        vault,
        connection_id,
        entry.permission_revision,
        if result.is_ok() {
            "bootstrap"
        } else {
            "failed"
        },
    )
    .map_err(|e| e.to_string())?;
    result
}
