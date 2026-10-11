//! 整个 Vault 的显式同步：确认令牌只绑定一个已预览目标，Git 在窗口锁外执行。
use super::{token, DesktopSession};
use crate::{
    filesystem::{file_identity, open_local_file, FileIdentity},
    model::Result,
    sync::GitSyncPreview,
    Vault,
};
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Serialize)]
pub struct DesktopGitSyncPreview {
    pub preview_id: String,
    pub session_id: String,
    pub whole_vault: bool,
    pub plan: GitSyncPreview,
}
pub(super) struct PendingGitSync {
    id: String,
    plan: GitSyncPreview,
}
pub struct GitSyncOperation {
    root: PathBuf,
    identity: FileIdentity,
    marker: FileIdentity,
    plan: GitSyncPreview,
}
impl DesktopSession {
    pub fn git_sync_status(&self, session_id: &str) -> Result<Value> {
        self.vault(session_id)?.git_status()
    }
    pub fn preview_git_sync(
        &mut self,
        session_id: &str,
        remote: &str,
    ) -> Result<DesktopGitSyncPreview> {
        self.pending_git_sync = None;
        let plan = self.vault(session_id)?.git_sync_preview(remote)?;
        let preview_id = token();
        self.pending_git_sync = Some(PendingGitSync {
            id: preview_id.clone(),
            plan: plan.clone(),
        });
        Ok(DesktopGitSyncPreview {
            preview_id,
            session_id: session_id.into(),
            whole_vault: true,
            plan,
        })
    }
    pub fn begin_git_sync(
        &mut self,
        session_id: &str,
        preview_id: &str,
        approve_whole_vault: bool,
        approve_transfer: bool,
    ) -> Result<GitSyncOperation> {
        self.vault(session_id)?;
        if !approve_whole_vault || !approve_transfer {
            return Err(
                "必须分别确认整个资料库及历史范围，并明确批准本次提交、拉取/整合和推送".into(),
            );
        }
        let pending = self
            .pending_git_sync
            .take()
            .filter(|pending| pending.id == preview_id)
            .ok_or("同步预览已失效，请重新检查目标与整个资料库范围")?;
        let vault = self.vault(session_id)?;
        Ok(GitSyncOperation {
            root: vault.root().into(),
            identity: file_identity(&open_local_file(vault.root())?)?,
            marker: file_identity(&open_local_file(
                &vault.root().join("control/schema-version.json"),
            )?)?,
            plan: pending.plan,
        })
    }
    pub fn cancel_git_sync_preview(&mut self, session_id: &str) -> Result<()> {
        self.vault(session_id)?;
        self.pending_git_sync = None;
        Ok(())
    }
}
impl GitSyncOperation {
    pub fn run(self) -> Result<Value> {
        let vault =
            Vault::open_existing(&self.root).map_err(|_| "同步资料库已不可用，请重新选择")?;
        if file_identity(&open_local_file(vault.root())?)? != self.identity
            || file_identity(&open_local_file(
                &vault.root().join("control/schema-version.json"),
            )?)? != self.marker
        {
            return Err("同步资料库目录或标记已被替换；未提交或联网".into());
        }
        vault.sync_reviewed(&self.plan.remote, &self.plan.fingerprint)
    }
}
