//! 桌面模型操作句柄：短暂核验会话后可移出 UI 锁，在 blocking worker 执行。
use super::{check_scope, DesktopSession};
use crate::{
    application::{
        background_memory::{
            MemoryConfig, MemoryJobControl, MemoryJobReview, MemoryProgress, MemoryRuntime,
            MemoryRuntimeStatus,
        },
        credentials::{self, CredentialStatus, CredentialStorage},
        runtime::{self, ProviderSetupResult, ServiceStatus},
        AppError, AppResult, ErrorCode, JobStatus,
    },
    filesystem::{file_identity, open_local_file, FileIdentity},
    Vault,
};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Clone)]
pub struct ModelOperation {
    root: PathBuf,
    root_identity: FileIdentity,
    marker: FileIdentity,
    scope: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct ModelSetupStatus {
    pub runtime: MemoryRuntimeStatus,
    pub service: ServiceStatus,
    pub credential: CredentialStatus,
}
fn denied() -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        "当前资料库会话、身份或范围已失效",
        "重新打开当前资料库后重试；不会操作另一个资料库",
    )
}
impl DesktopSession {
    /// 调用者只需在此方法期间持有 DesktopSession mutex；勿把 keyring/停机等待放在该锁内。
    pub fn model_operation(&self, session_id: &str, scope: &str) -> AppResult<ModelOperation> {
        check_scope(scope).map_err(|_| denied())?;
        let vault = self.vault(session_id).map_err(|_| denied())?;
        Ok(ModelOperation {
            root: vault.root().into(),
            root_identity: file_identity(&open_local_file(vault.root()).map_err(|_| denied())?)
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
impl ModelOperation {
    fn vault(&self) -> AppResult<Vault> {
        let vault = Vault::open_existing(&self.root).map_err(|_| denied())?;
        if file_identity(&open_local_file(vault.root()).map_err(|_| denied())?)
            .map_err(|_| denied())?
            != self.root_identity
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
    pub fn status(&self) -> AppResult<ModelSetupStatus> {
        let vault = self.vault()?;
        let runtime = MemoryRuntime::new(&vault).status_for_scope(&self.scope)?;
        let mut service = runtime::service_status(&vault)?;
        if service
            .last_memory_job
            .as_ref()
            .is_some_and(|job| job.scope != self.scope)
        {
            service.last_memory_job = None;
        }
        let expected = runtime
            .config
            .provider
            .as_ref()
            .map(credentials::provider_ref);
        let credential = if service.running
            && service.credential.present
            && service.credential.provider_ref == expected
            && runtime
                .config
                .credential_storage
                .is_none_or(|storage| service.credential.storage == storage)
        {
            service.credential.clone()
        } else if runtime.config.credential_storage == Some(CredentialStorage::OsProtected) {
            credentials::status(&vault, runtime.config.provider.as_ref())
        } else {
            Default::default()
        };
        if service.credential.provider_ref != expected
            || runtime
                .config
                .credential_storage
                .is_some_and(|storage| service.credential.storage != storage)
        {
            service.credential = Default::default();
        }
        Ok(ModelSetupStatus {
            runtime,
            service,
            credential,
        })
    }
    pub fn inspect_credential(
        &self,
        target: super::super::application::background_memory::MemoryProviderConfig,
    ) -> AppResult<CredentialStatus> {
        let vault = self.vault()?;
        MemoryConfig {
            scope: self.scope.clone(),
            provider: Some(target.clone()),
            ..Default::default()
        }
        .validate()?;
        Ok(credentials::status(&vault, Some(&target)))
    }
    pub fn configure(
        &self,
        config: MemoryConfig,
        api_key: Option<String>,
        storage: Option<CredentialStorage>,
    ) -> AppResult<ProviderSetupResult> {
        if config.scope != self.scope {
            return Err(denied());
        }
        config.validate()?;
        let vault = self.vault()?;
        let binary = std::env::current_exe().ok().and_then(|path| {
            path.parent().map(|parent| {
                parent.join(if cfg!(windows) {
                    "recallcard.exe"
                } else {
                    "recallcard"
                })
            })
        });
        runtime::configure_provider(&vault, config, api_key, storage, binary.as_deref())
    }
    /// 路径只能由 Tauri 内部 packaged_cli/资源目录核验取得，不能作为 JS 参数。
    pub fn configure_with_binary(
        &self,
        config: MemoryConfig,
        api_key: Option<String>,
        storage: Option<CredentialStorage>,
        trusted_cli: &std::path::Path,
    ) -> AppResult<ProviderSetupResult> {
        if config.scope != self.scope {
            return Err(denied());
        }
        config.validate()?;
        let vault = self.vault()?;
        runtime::configure_provider(&vault, config, api_key, storage, Some(trusted_cli))
    }
    pub fn control(
        &self,
        id: &str,
        action: MemoryJobControl,
    ) -> AppResult<JobStatus<MemoryProgress>> {
        let vault = self.vault()?;
        let runtime = MemoryRuntime::new(&vault);
        if !runtime
            .status_for_scope(&self.scope)?
            .jobs
            .iter()
            .any(|job| job.job_id == id)
        {
            return Err(denied());
        }
        runtime.control(id, action)
    }
    pub fn review(&self, id: &str) -> AppResult<MemoryJobReview> {
        let vault = self.vault()?;
        MemoryRuntime::new(&vault).review(id, &self.scope)
    }
    pub fn stop_service(&self) -> AppResult<ServiceStatus> {
        let vault = self.vault()?;
        runtime::request_service_stop(&vault)?;
        runtime::service_status(&vault)
    }
}
