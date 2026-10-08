//! 连接管理只对本机用户界面开放；当前空间不能审批、修改或撤销其他空间授权。
use super::{check_scope, DesktopSession};
use crate::{
    application::{
        connections::{self, ConnectionEntry, ConnectionGrant, PairingRequest},
        AppError, AppResult, ErrorCode,
    },
    Vault,
};
use serde_json::{json, Value};

fn denied() -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        "当前资料库会话或连接范围已失效",
        "刷新当前空间后重新检查连接",
    )
}
fn invalid_state() -> AppError {
    AppError::new(
        ErrorCode::Internal,
        "本机连接状态格式无效",
        "重新打开连接页；不要继续批准旧请求",
    )
}
fn in_scope(grant: &ConnectionGrant, scope: &str) -> bool {
    let scopes = grant
        .recall_scopes
        .iter()
        .chain(&grant.capture_scopes)
        .collect::<Vec<_>>();
    !scopes.is_empty() && scopes.iter().all(|allowed| allowed.as_str() == scope)
}
impl DesktopSession {
    fn connection_vault(&self, session_id: &str, scope: &str) -> AppResult<&Vault> {
        check_scope(scope).map_err(|_| denied())?;
        self.vault(session_id).map_err(|_| denied())
    }
    fn check_connection_grant(
        &self,
        vault: &Vault,
        scope: &str,
        grant: &ConnectionGrant,
    ) -> AppResult<()> {
        if !in_scope(grant, scope) {
            return Err(denied());
        }
        let id = connections::connection_key(
            &grant.client_kind,
            &grant.host_identity,
            &grant.platform,
            grant.installation_id.as_deref(),
        );
        if connections::get(vault, &id)?.is_some_and(|entry| !in_scope(&entry.grant, scope)) {
            return Err(denied());
        }
        Ok(())
    }
    pub fn connection_inventory(&self, session_id: &str, scope: &str) -> AppResult<Value> {
        let vault = self.connection_vault(session_id, scope)?;
        let inventory = connections::inventory(vault)?;
        let entries = inventory["entries"]
            .as_array()
            .ok_or_else(invalid_state)?
            .iter()
            .map(|row| {
                serde_json::from_value::<ConnectionEntry>(row.clone()).map_err(|_| invalid_state())
            })
            .collect::<AppResult<Vec<_>>>()?
            .into_iter()
            .filter(|entry| in_scope(&entry.grant, scope))
            .collect::<Vec<_>>();
        let pending = inventory["pending_pairings"]
            .as_array()
            .ok_or_else(invalid_state)?
            .iter()
            .map(|row| {
                serde_json::from_value::<PairingRequest>(row.clone()).map_err(|_| invalid_state())
            })
            .collect::<AppResult<Vec<_>>>()?
            .into_iter()
            .filter_map(|mut request| {
                request.recall_scope_cap.retain(|allowed| allowed == scope);
                request.capture_scope_cap.retain(|allowed| allowed == scope);
                (!request.recall_scope_cap.is_empty() || !request.capture_scope_cap.is_empty())
                    .then_some(request)
            })
            .collect::<Vec<_>>();
        let result = json!({"scope":scope,"entries":entries,"pending_pairings":pending,"supported_protocols":inventory["supported_protocols"],"identity_notice":inventory["identity_notice"]});
        if serde_json::to_vec(&result)
            .map_err(|_| invalid_state())?
            .len()
            > 256 * 1024
        {
            return Err(AppError::new(
                ErrorCode::ResourceLimit,
                "当前空间的连接清单超过显示上限",
                "在终端 connections 中检查连接，精简后重试",
            ));
        }
        Ok(result)
    }
    pub fn connection_configure(
        &self,
        session_id: &str,
        scope: &str,
        grant: ConnectionGrant,
        expected_revision: Option<u64>,
    ) -> AppResult<ConnectionEntry> {
        let vault = self.connection_vault(session_id, scope)?;
        self.check_connection_grant(vault, scope, &grant)?;
        connections::configure(vault, grant, expected_revision)
    }
    pub fn connection_revoke(
        &self,
        session_id: &str,
        scope: &str,
        id: &str,
        expected_revision: u64,
    ) -> AppResult<ConnectionEntry> {
        let vault = self.connection_vault(session_id, scope)?;
        let entry = connections::get(vault, id)?.ok_or_else(denied)?;
        if !in_scope(&entry.grant, scope) {
            return Err(denied());
        }
        connections::revoke(vault, id, expected_revision)
    }
    pub fn connection_approve_pairing(
        &self,
        session_id: &str,
        scope: &str,
        request_id: &str,
        grant: ConnectionGrant,
        expected_revision: Option<u64>,
    ) -> AppResult<ConnectionEntry> {
        let vault = self.connection_vault(session_id, scope)?;
        self.check_connection_grant(vault, scope, &grant)?;
        // Core atomically rechecks pending identity, site, caps, expiry and revision.
        connections::approve_pairing(vault, request_id, grant, expected_revision)
    }
    /// 为已批准的 Agent 返回两份独立宿主片段；生成不代表安装或调用成功。
    pub fn connection_agent_configs(
        &self,
        session_id: &str,
        scope: &str,
        id: &str,
        binary_path: &std::path::Path,
    ) -> AppResult<Value> {
        let vault = self.connection_vault(session_id, scope)?;
        let entry = connections::get(vault, id)?.ok_or_else(denied)?;
        if !in_scope(&entry.grant, scope)
            || entry.grant.client_kind != "claude_code"
            || entry.revoked
            || !entry.grant.auto_recall
            || !entry.grant.provider_disclosure
            || !binary_path.is_absolute()
        {
            return Err(denied());
        }
        let binary_path = binary_path.to_str().ok_or_else(denied)?;
        let root = vault.root().to_str().ok_or_else(denied)?;
        let prefix = json!([
            "--vault",
            root,
            "mcp",
            "--scope",
            scope,
            "--connection-id",
            id
        ]);
        Ok(json!({"connection_id":id,"registered":false,
            "mcp":{"mcpServers":{"recallcard":{"command":binary_path,"args":prefix}}},
            "hooks":{"hooks":{"SessionStart":[{"matcher":"startup|resume|compact|clear","hooks":[{"type":"command","command":binary_path,"args":["--vault",root,"agent-hook","--scope",scope,"--connection-id",id,"--budget-tokens","1500"],"timeout":10}]}]}},
            "note":"请分别审阅并合并 MCP 与 SessionStart 配置；未修改宿主设置、未启动 Agent，生成文件不证明实际调用。"}))
    }
    /// 只准备本机 stdio 启动说明和诊断；不建隧道、不配置凭据、不授予访问。
    /// 官方 tunnel-client 可转发 stdio，因此这里不启动 HTTP/SSE 或公网监听。
    pub fn chatgpt_connection_plan(
        &self,
        session_id: &str,
        scope: &str,
        binary_path: &std::path::Path,
    ) -> AppResult<Value> {
        let vault = self.connection_vault(session_id, scope)?;
        if !binary_path.is_absolute() {
            return Err(denied());
        }
        let binary_path = binary_path.to_str().ok_or_else(denied)?;
        let root = vault.root().to_str().ok_or_else(denied)?;
        let id = connections::connection_key("chatgpt_mcp", "openai-chatgpt", "chatgpt", None);
        let entry = connections::get(vault, &id)?;
        if entry
            .as_ref()
            .is_some_and(|entry| !in_scope(&entry.grant, scope))
        {
            return Err(denied());
        }
        let readiness = match &entry {
            None => "permission_required",
            Some(entry) if entry.revoked => "revoked",
            Some(entry) if !entry.grant.provider_disclosure || !entry.grant.auto_recall => "paused",
            Some(_) => "ready",
        };
        Ok(json!({
            "connection_id":id,"scope":scope,"recipient":"ChatGPT / OpenAI",
            "transport":"openai_secure_mcp_tunnel","local_transport":"stdio",
            "local_command":{"command":binary_path,"args":["--vault",root,"mcp","--scope",scope,"--connection-id",id]},
            "local_readiness":readiness,"upstream_verification":"not_checked",
            "last_local_read_at":entry.as_ref().and_then(|entry|entry.last_read_at),
            "last_local_bootstrap_at":entry.as_ref().and_then(|entry|entry.last_bootstrap_at),
            "grant":entry,"capture_enabled":false,"public_listener":false,
            "tunnel_status":"not_inspected","credential_status":"not_inspected",
            "official_connect_url":"https://chatgpt.com/plugins",
            "official_tunnel_url":"https://platform.openai.com/settings/organization/tunnels",
            "official_guide_url":"https://developers.openai.com/api/docs/guides/secure-mcp-tunnels",
            "tools":["bootstrap","search","read","sources"],
            "note":"本机读取接口已准备。仍需在官方流程完成目标账号/空间、隧道与插件授权，再验证一次实际只读调用。本机调用记录不能单独证明 ChatGPT 已连接。"
        }))
    }
}
