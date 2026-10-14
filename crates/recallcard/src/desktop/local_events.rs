//! 来源索引的独立本机/云批准；未配置、未覆盖或服务错误不自动更换提供器。
use super::ModelOperation;
use crate::{
    application::{
        background_memory::MemoryProviderConfig,
        credentials::{self, CredentialStorage},
        embedding_cloud::{CloudEmbeddingRuntime, SourceCloudSettings},
        runtime, AppError, AppResult, ErrorCode,
    },
    local_events::{CloudSetup, LocalEvents, Setup},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudEventSetup {
    pub settings: SourceCloudSettings,
    pub tokenizer: String,
    pub chunk_bytes: usize,
    pub disk_cap_mib: usize,
    pub save_in_os: bool,
}
impl CloudEventSetup {
    fn index_setup(&self) -> CloudSetup {
        CloudSetup {
            settings: self.settings.clone(),
            tokenizer: self.tokenizer.clone(),
            chunk_bytes: self.chunk_bytes,
            disk_cap_mib: self.disk_cap_mib,
        }
    }
}
fn cloud_error(message: impl Into<String>) -> AppError {
    AppError::new(
        ErrorCode::ModelUnavailable,
        message,
        "核对明确批准的云接收者、数据范围、预算及系统凭据；普通关键词搜索仍可用",
    )
}
fn error(code: String) -> AppError {
    AppError::new(
        ErrorCode::ModelUnavailable,
        format!("原始对话索引未可用（{code}）"),
        "在设置中核对所选服务和索引覆盖；普通关键词搜索仍可用，不会自动更换服务",
    )
}
impl ModelOperation {
    pub fn cloud_event_status(&self) -> AppResult<Value> {
        let vault = self.vault()?;
        let cloud = CloudEmbeddingRuntime::new(&vault).source_status(&self.scope)?;
        let mut result = match LocalEvents::open(&vault, &self.scope) {
            Ok(Some(index)) => index.status().map_err(error)?,
            Ok(None) => {
                json!({"state":"unconfigured","provider":"none","settings":null,"message":"尚未启用来源语义索引"})
            }
            Err(code) => {
                json!({"state":"needs_attention","provider":if cloud.grant.is_some(){"cloud"}else{"none"},"settings":null,"service_error":error(code),"message":"来源语义配置已改变或撤销；关键词读取保留"})
            }
        };
        result["cloud"] =
            serde_json::to_value(cloud).map_err(|_| cloud_error("云来源批准状态不可读取"))?;
        Ok(result)
    }
    pub fn configure_cloud_events(
        &self,
        request: CloudEventSetup,
        key: Option<String>,
        binary: Option<&Path>,
    ) -> AppResult<Value> {
        let setup = request.index_setup();
        setup.validate().map_err(error)?;
        if !request.save_in_os {
            return Err(cloud_error(
                "跨进程云来源编码需要明确选择系统保护存储；未保存或启用",
            ));
        }
        let vault = self.vault()?;
        let python = match binary {
            Some(path) => runtime::python_provider_for_cli(path)?,
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
                .ok_or_else(|| cloud_error("这个云向量目标尚无可用的独立系统凭据"))?,
        };
        if credential.status().storage != CredentialStorage::OsProtected {
            return Err(cloud_error(
                "系统保护存储不可用，未启用云来源编码；本次输入随调用结束丢弃",
            ));
        }
        LocalEvents::configure_cloud(
            &vault,
            &self.scope,
            setup,
            python.python,
            python.python_path,
        )
        .map_err(error)?;
        let mut result = self.cloud_event_status()?;
        result["credential"] = serde_json::to_value(credential.status())
            .map_err(|_| cloud_error("凭据状态不可读取"))?;
        Ok(result)
    }
    pub fn disable_cloud_events(&self) -> AppResult<Value> {
        let vault = self.vault()?;
        LocalEvents::disable_cloud(&vault, &self.scope).map_err(error)?;
        self.cloud_event_status()
    }
    pub fn local_event_status(&self) -> AppResult<Value> {
        let vault = self.vault()?;
        match LocalEvents::open(&vault, &self.scope).map_err(error)? {
            Some(index) => index.status().map_err(error),
            None => Ok(
                json!({"state":"unconfigured","provider":"none","settings":null,"message":"尚未启用来源语义索引"}),
            ),
        }
    }
    pub fn configure_local_events(
        &self,
        request: Setup,
        binary: Option<&Path>,
    ) -> AppResult<Value> {
        let vault = self.vault()?;
        let python = match binary {
            Some(path) => runtime::python_provider_for_cli(path)?,
            None => runtime::default_python_provider(),
        };
        LocalEvents::configure(
            &vault,
            &self.scope,
            request,
            python.python,
            python.python_path,
        )
        .map_err(error)?
        .status()
        .map_err(error)
    }
    pub fn refresh_local_events(&self) -> AppResult<Value> {
        let vault = self.vault()?;
        LocalEvents::open(&vault, &self.scope)
            .map_err(error)?
            .ok_or_else(|| error("unconfigured".into()))?
            .refresh_step()
            .map_err(error)
    }
    pub fn disable_local_events(&self) -> AppResult<Value> {
        let vault = self.vault()?;
        LocalEvents::disable(&vault, &self.scope).map_err(error)?;
        self.local_event_status()
    }
    pub(super) fn local_question_search(
        &self,
        query: &str,
        target: &str,
    ) -> AppResult<Option<Value>> {
        if !matches!(target, "all" | "events" | "memories") {
            return Ok(None);
        }
        let vault = self.vault()?;
        let context = crate::context::Context::new(
            &vault,
            crate::policy::Access::new(vec![self.scope.clone()]).map_err(error)?,
        );
        let mut response = context.recall_question(query, target).map_err(error)?;
        // 保留未配置时既有关键词体验；此分支明确只用本地词法，绝不进入旧云选择器。
        if target != "memories"
            && LocalEvents::open(&vault, &self.scope)
                .map_err(error)?
                .is_none()
        {
            let args=serde_json::from_value(json!({"query":query,"target":target,"lexical_match":"natural_question","limit":10,"budget_bytes":7168})).map_err(|_|error("invalid_request".into()))?;
            let coverage = response["coverage"].clone();
            response = context.search(args).map_err(error)?;
            response["coverage"]["local_event_gateway"] = coverage;
            response["coverage"]["cloud_fallback"] = json!(false);
        }
        self.vault()?;
        Ok(Some(response))
    }
}
