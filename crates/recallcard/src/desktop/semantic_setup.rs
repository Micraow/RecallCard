//! 普通桌面语义设置、读取与连接共用可信配置，不借用 LLM 云批准。
use super::{records::WorkspaceRecords, ModelOperation};
use crate::{
    application::{
        background_memory::{IndexRefreshStatus, MemoryRuntime},
        runtime, AppError, AppResult, ErrorCode,
    },
    context::{Context, SearchArgs},
    policy::Access,
    semantic::{LocalSemanticSetup, SemanticSearch},
};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
#[derive(Debug, Serialize)]
pub struct SemanticSetupStatus {
    pub state: String,
    pub message: String,
    pub settings: Option<LocalSemanticSetup>,
    pub paused: bool,
    pub index_refresh: Option<IndexRefreshStatus>,
    pub service: Option<runtime::ServiceStatus>,
    pub service_error: Option<AppError>,
}
fn invalid(message: impl Into<String>) -> AppError {
    AppError::new(
        ErrorCode::InvalidRequest,
        message,
        "检查本机提供器地址、模型与明确选择的范围后重试",
    )
}
impl ModelOperation {
    pub fn semantic_status(&self) -> AppResult<SemanticSetupStatus> {
        let vault = self.vault()?;
        let runtime = MemoryRuntime::new(&vault);
        let cfg = runtime.configuration()?;
        let mut result = SemanticSetupStatus {
            state: "unconfigured".into(),
            message: "尚未启用本机语义检索，目录与词法读取仍可用".into(),
            settings: None,
            paused: cfg.paused,
            index_refresh: None,
            service: None,
            service_error: None,
        };
        if cfg.scope != self.scope {
            if cfg.enabled
                || cfg.index_maintenance_enabled
                || cfg.candidate_semantic_config.is_some()
            {
                result.state = "scope_unavailable".into();
                result.message = "此资料库已有另一个活动后台范围，请先处理原范围设置".into();
            }
            return Ok(result);
        }
        if let Some(path) = cfg.candidate_semantic_config {
            match crate::semantic::setup::local_setup_snapshot(&path, &self.scope) {
                Ok(settings) => {
                    result.state = "configured".into();
                    result.message="本机配置已保存；索引/查询状态以实际运行结果为准。修改后请重新生成浏览器连接文件".into();
                    result.settings = Some(settings);
                }
                Err(_)
                    if SemanticSearch::from_application_config_file(
                        &vault,
                        &path,
                        std::slice::from_ref(&self.scope),
                    )
                    .is_ok() =>
                {
                    result.state = "cloud_configured".into();
                    result.message =
                        "当前启用独立云向量配置，可在云向量设置中核对用量或停用；本机选项仍保留"
                            .into();
                }
                Err(_) => {
                    result.state = "invalid".into();
                    result.message =
                        "已有语义配置不可核验；原索引保留，可重新选择本机模型设置".into();
                }
            }
        }
        result.index_refresh = runtime.status_for_scope(&self.scope)?.index_refresh;
        match runtime::service_status(&vault) {
            Ok(s) => result.service = Some(s),
            Err(e) => result.service_error = Some(e),
        };
        Ok(result)
    }
    pub fn configure_semantic(
        &self,
        request: LocalSemanticSetup,
    ) -> AppResult<SemanticSetupStatus> {
        let binary = std::env::current_exe().ok().and_then(|p| {
            p.parent().map(|d| {
                d.join(if cfg!(windows) {
                    "recallcard.exe"
                } else {
                    "recallcard"
                })
            })
        });
        self.configure_semantic_inner(request, binary.as_deref())
    }
    pub fn configure_semantic_with_binary(
        &self,
        request: LocalSemanticSetup,
        binary: &Path,
    ) -> AppResult<SemanticSetupStatus> {
        self.configure_semantic_inner(request, Some(binary))
    }
    fn configure_semantic_inner(
        &self,
        request: LocalSemanticSetup,
        binary: Option<&Path>,
    ) -> AppResult<SemanticSetupStatus> {
        request.validate().map_err(invalid)?;
        let vault = self.vault()?;
        let runtime = MemoryRuntime::new(&vault);
        let cfg = runtime.configuration()?;
        if cfg.scope != self.scope
            && (cfg.enabled
                || cfg.index_maintenance_enabled
                || cfg.candidate_semantic_config.is_some())
        {
            return Err(invalid("已有另一个活动后台范围，未覆盖原配置"));
        }
        let python = match binary.filter(|p| p.is_file()) {
            Some(binary) => runtime::python_provider_for_cli(binary)?,
            None => runtime::default_python_provider(),
        };
        crate::application::embedding_cloud::CloudEmbeddingRuntime::new(&vault)
            .revoke(&self.scope)?;
        let path = crate::semantic::setup::managed_local_setup(
            &vault,
            &self.scope,
            &request,
            python.python,
            python.python_path,
        )
        .map_err(invalid)?;
        runtime.configure_semantic(
            &self.scope,
            Some(path),
            request.memory_approved,
            request.resume_background,
        )?;
        let mut result = self.semantic_status()?;
        if !result.paused && request.memory_approved {
            match binary {
                Some(p) if p.is_file() => match runtime::ensure_service(&vault, p) {
                    Ok(service) => result.service = Some(service),
                    Err(e) => result.service_error = Some(e),
                },
                _ => {
                    result.service_error = Some(AppError::new(
                        ErrorCode::ModelUnavailable,
                        "配置已保存，但未找到随桌面安装的后台程序",
                        "补齐 recallcard 后台程序后重试；不会改用网页或外部服务",
                    ))
                }
            }
        }
        Ok(result)
    }
    pub fn disable_semantic(&self) -> AppResult<SemanticSetupStatus> {
        let vault = self.vault()?;
        MemoryRuntime::new(&vault).configure_semantic(&self.scope, None, false, false)?;
        crate::application::embedding_cloud::CloudEmbeddingRuntime::new(&vault)
            .revoke(&self.scope)?;
        self.semantic_status()
    }
    pub fn search(&self, query: &str, target: &str) -> AppResult<Value> {
        super::validate_target(target).map_err(invalid)?;
        if query.trim().is_empty() || query.len() > 4096 {
            return Err(invalid("请输入1–4096字节的检索词"));
        }
        let vault = self.vault()?;
        let cfg = MemoryRuntime::new(&vault).configuration()?;
        let mut error = None;
        let semantic = if cfg.scope == self.scope {
            cfg.candidate_semantic_config.as_ref().and_then(|p| {
                match SemanticSearch::from_application_config_file(
                    &vault,
                    p,
                    std::slice::from_ref(&self.scope),
                ) {
                    Ok(s) => Some(s),
                    Err(_) => {
                        error = Some("offline_config_invalid");
                        None
                    }
                }
            })
        } else {
            None
        };
        let access = Access::new(vec![self.scope.clone()]).map_err(invalid)?;
        let context = match semantic.as_ref() {
            Some(s) => Context::with_semantic(&vault, access, s),
            None => Context::new(&vault, access),
        };
        let mut response = context
            .search(SearchArgs {
                preview_min_bytes: None,
                lexical_match: "exact".into(),
                include_navigation: false,
                query: query.into(),
                target: target.into(),
                session_ref: None,
                as_of: None,
                event_filter: None,
                limit: 30,
                detail: "context".into(),
                budget_tokens: super::RESPONSE_LIMIT,
                cursor: None,
            })
            .map_err(invalid)?;
        if let Some(code) = error {
            response["coverage"]["semantic_error"] = serde_json::json!(code);
        }
        // 只在最终显示补全阶段取短读锁。若查询之后资料失效，拒绝整个旧响应。
        self.vault()?;
        let _guard = vault.read_guard().map_err(invalid)?;
        WorkspaceRecords::load(&vault, &context)
            .and_then(|r| r.enrich_current_search(response))
            .map_err(invalid)
    }
}
