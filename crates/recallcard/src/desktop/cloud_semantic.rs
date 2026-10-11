//! 普通云向量入口，查询与当前 Memory 独立审阅；不复用整理 LLM 密钥或授权。
use super::ModelOperation;
use crate::{
    application::{
        background_memory::{IndexRefreshStatus, MemoryProviderConfig, MemoryRuntime},
        credentials::{self, CredentialStatus, CredentialStorage},
        embedding_cloud::{CloudEmbeddingRuntime, CloudEmbeddingSettings, CloudEmbeddingStatus},
        runtime, AppError, AppResult, ErrorCode,
    },
    semantic::SemanticSearch,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudSemanticSetup {
    pub settings: CloudEmbeddingSettings,
    pub save_in_os: bool,
    #[serde(default)]
    pub resume_background: bool,
}
#[derive(Debug, Serialize)]
pub struct CloudSemanticStatus {
    pub state: String,
    pub message: String,
    pub cloud: CloudEmbeddingStatus,
    pub index_refresh: Option<IndexRefreshStatus>,
    pub paused: bool,
    pub credential: Option<CredentialStatus>,
    pub service_error: Option<AppError>,
}
fn invalid(message: impl Into<String>) -> AppError {
    AppError::new(
        ErrorCode::InvalidRequest,
        message,
        "核对云接收者、独立数据范围、预算与系统凭据保存选择",
    )
}
impl ModelOperation {
    pub fn cloud_semantic_status(&self) -> AppResult<CloudSemanticStatus> {
        let vault = self.vault()?;
        let runtime = MemoryRuntime::new(&vault);
        let config = runtime.configuration()?;
        let cloud = CloudEmbeddingRuntime::new(&vault).status(&self.scope)?;
        let configured = cloud.grant.is_some()
            && config.scope == self.scope
            && config
                .candidate_semantic_config
                .as_ref()
                .is_some_and(|path| {
                    SemanticSearch::from_application_config_file(
                        &vault,
                        path,
                        std::slice::from_ref(&self.scope),
                    )
                    .is_ok()
                });
        Ok(CloudSemanticStatus {
            state: if configured {
                "configured"
            } else if cloud.grant.is_some() {
                "needs_attention"
            } else {
                "unconfigured"
            }
            .into(),
            message: if configured {
                "独立云向量配置已保存；实际编码与索引以读取结果为准，设置不证明供应商已连通"
            } else {
                "云向量未启用或配置未对齐；目录与词法读取保留"
            }
            .into(),
            cloud,
            index_refresh: if config.scope == self.scope {
                runtime.status_for_scope(&self.scope)?.index_refresh
            } else {
                None
            },
            paused: config.paused,
            credential: None,
            service_error: None,
        })
    }
    pub fn configure_cloud_semantic(
        &self,
        request: CloudSemanticSetup,
        key: Option<String>,
        binary: Option<&Path>,
    ) -> AppResult<CloudSemanticStatus> {
        request.settings.validate()?;
        if !request.save_in_os {
            return Err(invalid(
                "当前跨进程云向量需明确选择系统保护存储；不会把会话密钥写入配置或伪装持久可用",
            ));
        }
        let vault = self.vault()?;
        let runtime = MemoryRuntime::new(&vault);
        let cfg = runtime.configuration()?;
        if cfg.scope != self.scope
            && (cfg.enabled
                || cfg.index_maintenance_enabled
                || cfg.candidate_semantic_config.is_some())
        {
            return Err(invalid("已有另一活动范围；请先在原范围停用"));
        }
        let python = match binary {
            Some(binary) => runtime::python_provider_for_cli(binary)?,
            None => runtime::default_python_provider(),
        };
        let target = MemoryProviderConfig {
            endpoint: request.settings.endpoint.clone(),
            model: request.settings.model.clone(),
        };
        let credential = match key {
            Some(key) => credentials::prepare_embedding(
                &vault,
                target.clone(),
                key,
                CredentialStorage::OsProtected,
            )?,
            None => credentials::load_embedding_os(&vault, &target)?
                .ok_or_else(|| invalid("这个云向量目标尚无可用的独立系统凭据"))?,
        };
        if credential.status().storage != CredentialStorage::OsProtected {
            return Err(AppError::new(
                ErrorCode::ModelUnavailable,
                "系统保护存储不可用，未启用云向量；本次输入随调用结束丢弃",
                "解锁系统凭据存储后重试，或使用本机 embedding 选项",
            ));
        }
        let path = crate::semantic::setup::managed_cloud_setup(
            &vault,
            &self.scope,
            &request.settings,
            python.python,
            python.python_path,
        )
        .map_err(|e| invalid(format!("独立系统凭据已保存，但云向量配置未完成：{e}")))?;
        runtime.configure_semantic(
            &self.scope,
            Some(path),
            request.settings.memory_approved,
            request.resume_background,
        )?;
        let mut result = self.cloud_semantic_status()?;
        result.credential = Some(credential.status().clone());
        if request.settings.memory_approved && !result.paused {
            result.service_error = match binary {
                Some(binary) => runtime::ensure_service(&vault, binary).err(),
                None => Some(AppError::new(
                    ErrorCode::ModelUnavailable,
                    "配置已保存，但未确认后台程序",
                    "补齐随应用安装的后台程序后重试；不会撤销已保存记忆",
                )),
            };
        }
        Ok(result)
    }
    pub fn disable_cloud_semantic(&self) -> AppResult<CloudSemanticStatus> {
        let vault = self.vault()?;
        let runtime = MemoryRuntime::new(&vault);
        let cfg = runtime.configuration()?;
        let selected = cfg
            .candidate_semantic_config
            .as_ref()
            .filter(|path| {
                cfg.scope == self.scope && crate::semantic::managed_cloud::is_managed_file(path)
            })
            .cloned();
        CloudEmbeddingRuntime::new(&vault).revoke(&self.scope)?;
        if let Some(path) = selected {
            runtime.clear_semantic_if(&self.scope, &path)?;
        }
        self.cloud_semantic_status()
    }
}
