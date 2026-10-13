//! 普通聊天独立本机批准；未配置、未覆盖、服务错误都不改用云编码。
use super::ModelOperation;
use crate::{
    application::{runtime, AppError, AppResult, ErrorCode},
    local_events::{LocalEvents, Setup},
};
use serde_json::{json, Value};
use std::path::Path;
fn error(code: String) -> AppError {
    AppError::new(
        ErrorCode::ModelUnavailable,
        format!("本机原始对话索引未可用（{code}）"),
        "在设置中核对本机服务和索引覆盖；普通关键词搜索仍可用，不会自动改用云服务",
    )
}
impl ModelOperation {
    pub fn local_event_status(&self) -> AppResult<Value> {
        let vault = self.vault()?;
        match LocalEvents::open(&vault, &self.scope).map_err(error)? {
            Some(index) => index.status().map_err(error),
            None => Ok(
                json!({"state":"unconfigured","settings":null,"message":"尚未启用普通对话本机语义索引"}),
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
        if !matches!(target, "all" | "events") {
            return Ok(None);
        }
        let vault = self.vault()?;
        let Some(index) = LocalEvents::open(&vault, &self.scope).map_err(error)? else {
            return Ok(None);
        };
        let mut response = index.search(query, 20).map_err(error)?;
        // 新本机入口不借用另一个Memory/云批准。既有Memory仍可按词法补充且保留来源类型。
        if target == "all" {
            let context = crate::context::Context::new(
                &vault,
                crate::policy::Access::new(vec![self.scope.clone()]).map_err(error)?,
            );
            let args=serde_json::from_value(json!({"query":query,"target":"memories","lexical_match":"natural_question","limit":10,"budget_bytes":8192})).map_err(|_|error("invalid_request".into()))?;
            let memory = context.search(args).map_err(error)?;
            if let (Some(rows), Some(extra)) = (
                response["results"].as_array_mut(),
                memory["results"].as_array(),
            ) {
                rows.extend(extra.iter().cloned());
            }
            response["coverage"]["memory_search"] = json!("lexical_only_no_cloud_fallback");
            response["match_count"] =
                json!(response["results"].as_array().map(Vec::len).unwrap_or(0));
            if memory["truncated"] == true {
                response["truncated"] = json!(true);
            }
        }
        self.vault()?;
        Ok(Some(response))
    }
}
