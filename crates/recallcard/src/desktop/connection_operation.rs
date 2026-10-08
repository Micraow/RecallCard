//! 可移出窗口互斥锁的连接操作；异步核验不阻塞来源阅读。
use super::{check_scope, DesktopSession};
use crate::{
    application::{
        agent_install::{self, InstallPlan, InstallRequest},
        connection_setup::{self, ConnectionHealth},
        connections, AppError, AppResult, ErrorCode,
    },
    filesystem::{file_identity, open_local_file, FileIdentity},
    Vault,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
#[derive(Clone)]
pub struct ConnectionOperation {
    root: PathBuf,
    identity: FileIdentity,
    marker: FileIdentity,
    scope: String,
}
fn denied() -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        "当前资料库会话或连接范围已变化",
        "重新打开连接页后重试",
    )
}
impl DesktopSession {
    pub fn connection_operation(
        &self,
        session_id: &str,
        scope: &str,
    ) -> AppResult<ConnectionOperation> {
        check_scope(scope).map_err(|_| denied())?;
        let vault = self.vault(session_id).map_err(|_| denied())?;
        Ok(ConnectionOperation {
            root: vault.root().into(),
            identity: file_identity(&open_local_file(vault.root()).map_err(|_| denied())?)
                .map_err(|_| denied())?,
            marker: file_identity(
                &open_local_file(&vault.root().join("control/schema-version.json"))
                    .map_err(|_| denied())?,
            )
            .map_err(|_| denied())?,
            scope: scope.into(),
        })
    }
}
impl ConnectionOperation {
    fn vault(&self) -> AppResult<Vault> {
        let vault = Vault::open_existing(&self.root).map_err(|_| denied())?;
        if file_identity(&open_local_file(vault.root()).map_err(|_| denied())?)
            .map_err(|_| denied())?
            != self.identity
            || file_identity(
                &open_local_file(&vault.root().join("control/schema-version.json"))
                    .map_err(|_| denied())?,
            )
            .map_err(|_| denied())?
                != self.marker
        {
            return Err(denied());
        }
        Ok(vault)
    }
    fn check_entry(&self, vault: &Vault, id: &str) -> AppResult<()> {
        let entry = connections::get(vault, id)?.ok_or_else(denied)?;
        let mut scopes = entry
            .grant
            .recall_scopes
            .iter()
            .chain(&entry.grant.capture_scopes)
            .peekable();
        if scopes.peek().is_none() || scopes.any(|scope| scope != &self.scope) {
            return Err(denied());
        }
        Ok(())
    }
    pub fn plan(&self, client: &str, project_dir: &Path, binary: &Path) -> AppResult<InstallPlan> {
        agent_install::plan(
            &self.vault()?,
            &InstallRequest {
                client: client.into(),
                connection_id: String::new(),
                scope: self.scope.clone(),
                project_dir: project_dir.into(),
                binary_path: binary.into(),
            },
        )
    }
    pub fn apply(&self, plan_id: &str) -> AppResult<Value> {
        let vault = self.vault()?;
        let plan = agent_install::get_plan(&vault, plan_id)?;
        if plan.scope != self.scope {
            return Err(denied());
        }
        connection_setup::apply(&vault, plan_id)
    }
    pub fn health(&self, id: &str) -> AppResult<ConnectionHealth> {
        let vault = self.vault()?;
        self.check_entry(&vault, id)?;
        connection_setup::health(&vault, id, &self.scope)
    }
    pub fn verify(&self, id: &str, binary: &Path) -> AppResult<ConnectionHealth> {
        let vault = self.vault()?;
        self.check_entry(&vault, id)?;
        connection_setup::verify(&vault, id, &self.scope, binary)
    }
}
