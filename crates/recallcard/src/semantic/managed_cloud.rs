//! 受审阅的普通云配置；旧 CLI cloud_query 从不自动升级成跨进程授权。
use super::*;
use crate::application::{
    background_memory::{MemoryProviderConfig, MemoryRuntime},
    credentials,
    embedding_cloud::{CloudEmbeddingGrant, CloudEmbeddingRuntime, EmbeddingData},
};
use std::sync::Arc;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManagedConfig {
    schema: String,
    grant: CloudEmbeddingGrant,
    config: SemanticConfig,
}
pub(crate) struct ManagedCloud {
    root: PathBuf,
    path: PathBuf,
    fingerprint: String,
    grant: CloudEmbeddingGrant,
}
impl ManagedCloud {
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    pub fn memory_approved(&self) -> bool {
        self.grant.settings.memory_approved
    }
    pub fn endpoint(&self) -> &str {
        &self.grant.settings.endpoint
    }
    pub fn current(&self) -> bool {
        let Ok(vault) = crate::Vault::open_existing(&self.root) else {
            return false;
        };
        let file_matches = read_local::<ManagedConfig>(&self.path)
            .ok()
            .and_then(|record| serde_json::to_vec(&record).ok())
            .is_some_and(|bytes| hash(&bytes) == self.fingerprint);
        file_matches
            && CloudEmbeddingRuntime::new(&vault)
                .configuration_current(&self.grant, &self.fingerprint)
                .unwrap_or(false)
            && MemoryRuntime::new(&vault)
                .semantic_configuration_matches(&self.grant.scope, &self.path)
                .unwrap_or(false)
    }
    fn ready(&self) -> SResult<crate::Vault> {
        if !self.current() {
            return failure("cloud_approval_changed");
        }
        crate::Vault::open_existing(&self.root)
            .map_err(|_| SemanticError("cloud_approval_unavailable"))
    }
    pub fn exchange(&self, config: &SemanticConfig, request: &Value) -> SResult<Vec<u8>> {
        let vault = self.ready()?;
        let index_only = request["op"] == "embed_documents";
        let data = if index_only {
            EmbeddingData::CurrentMemory
        } else {
            EmbeddingData::Query
        };
        let space: Space = serde_json::from_value(if index_only {
            request["space"].clone()
        } else {
            request["index"]["space"].clone()
        })
        .map_err(|_| SemanticError("invalid_input"))?;
        if space.endpoint != self.grant.settings.endpoint
            || space.model != self.grant.settings.model
            || request["allowed_scopes"] != json!([self.grant.scope])
        {
            return failure("query_not_approved");
        }
        let input = if index_only {
            request["texts"].clone()
        } else {
            let query = request["query"]
                .as_str()
                .ok_or(SemanticError("invalid_input"))?;
            json!([if space.query_instruction.is_empty() {
                query.to_string()
            } else {
                format!("{}\n{}", space.query_instruction, query)
            }])
        };
        let body=serde_json::to_vec(&json!({"model":space.model,"input":input,"dimensions":space.dimensions,"encoding_format":"float"})).map_err(|_|SemanticError("input_limit"))?;
        if index_only {
            // 外发前重新取当前可读 Memory，不能用旧计划或旧向量代替实时来源核验。
            let context = crate::context::Context::new(
                &vault,
                crate::policy::Access::new(vec![self.grant.scope.clone()])
                    .map_err(|_| SemanticError("query_not_approved"))?,
            );
            let guard = vault
                .read_guard()
                .map_err(|_| SemanticError("source_unavailable"))?;
            let documents = context
                .memory_documents_locked(&guard)
                .map_err(|_| SemanticError("source_unavailable"))?;
            let current = corpus_documents(
                &documents,
                std::slice::from_ref(&self.grant.scope),
                Utc::now(),
            );
            let allowed: BTreeSet<String> = current
                .iter()
                .filter_map(|d| d["text"].as_str())
                .map(|text| {
                    if space.document_instruction.is_empty() {
                        text.into()
                    } else {
                        format!("{}\n{}", space.document_instruction, text)
                    }
                })
                .collect();
            if !request["texts"].as_array().is_some_and(|texts| {
                texts
                    .iter()
                    .all(|text| text.as_str().is_some_and(|text| allowed.contains(text)))
            }) {
                return failure("source_changed");
            }
        }
        let target = MemoryProviderConfig {
            endpoint: space.endpoint,
            model: space.model,
        };
        let credential = credentials::load_embedding_os(&vault, &target)
            .map_err(|_| SemanticError("key_unavailable"))?
            .ok_or(SemanticError("key_unavailable"))?;
        CloudEmbeddingRuntime::new(&vault)
            .reserve(
                &self.grant,
                data,
                &self.grant.scope,
                &target.endpoint,
                &target.model,
                body.len() as u64,
            )
            .map_err(|e| {
                SemanticError(match e.code {
                    crate::application::ErrorCode::BudgetExceeded => "network_budget_exhausted",
                    crate::application::ErrorCode::PermissionDenied => "query_not_approved",
                    crate::application::ErrorCode::Conflict => "cloud_budget_busy",
                    _ => "cloud_approval_unavailable",
                })
            })?;
        let mut bytes = serde_json::to_vec(request).map_err(|_| SemanticError("input_limit"))?;
        bytes.push(b'\n');
        if bytes.len() > MAX_REQUEST_BYTES {
            return failure("input_limit");
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let config = config.clone();
        let scope = self.grant.scope.clone();
        let timeout = config.timeout_ms;
        std::thread::Builder::new()
            .name("recallcard-cloud-once".into())
            .spawn(move || {
                let result = (|| {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_io()
                        .enable_time()
                        .build()
                        .map_err(|_| SemanticError("worker_unavailable"))?;
                    runtime.block_on(async {
                        let key = credential
                            .for_target(&target)
                            .ok_or(SemanticError("key_unavailable"))?;
                        let mut process = Process::start_managed(
                            &config,
                            &target.endpoint,
                            &scope,
                            index_only,
                            key,
                        )?;
                        let result = tokio::time::timeout(
                            Duration::from_millis(timeout),
                            process.exchange_limit(&bytes, 512 * 1024),
                        )
                        .await
                        .unwrap_or_else(|_| failure("worker_timeout"));
                        let _ =
                            tokio::time::timeout(Duration::from_millis(250), process.child.kill())
                                .await;
                        result
                    })
                })();
                let _ = sender.send(result);
            })
            .map_err(|_| SemanticError("worker_unavailable"))?;
        let response = receiver
            .recv_timeout(Duration::from_millis(timeout + 500))
            .unwrap_or_else(|_| failure("worker_timeout"))?;
        self.ready()?; // 提供器等待期间可以撤销，旧响应不能返回或入缓存。
        Ok(response)
    }
}
impl SemanticSearch {
    pub fn from_application_config_file(
        vault: &crate::Vault,
        path: &Path,
        scopes: &[String],
    ) -> Result<Self> {
        let file = crate::filesystem::open_local_file(path)?;
        if !file.metadata().map_err(|_| "语义设置不可读取")?.is_file() {
            return Err("语义设置不是普通文件".into());
        }
        let mut bytes = Vec::new();
        file.take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| "语义设置不可读取")?;
        Self::from_application_config_bytes(vault, path, scopes, &bytes)
    }
    pub(crate) fn from_application_config_bytes(
        vault: &crate::Vault,
        path: &Path,
        scopes: &[String],
        bytes: &[u8],
    ) -> Result<Self> {
        if bytes.len() > 65536 {
            return Err("语义设置超过上限".into());
        }
        if let Ok(config) = serde_json::from_slice::<SemanticConfig>(bytes) {
            return Self::from_offline_config(config);
        }
        let record: ManagedConfig =
            serde_json::from_slice(bytes).map_err(|_| "语义设置格式无效")?;
        if record.schema != "recallcard.managed-cloud-semantic/1"
            || !scopes.contains(&record.grant.scope)
            || !path.starts_with(vault.state_dir()?.join("semantic"))
        {
            return Err("独立云语义批准与当前范围不匹配".into());
        }
        let fingerprint = hash(&serde_json::to_vec(&record).map_err(|_| "云语义配置不可核验")?);
        let managed = ManagedCloud {
            root: vault.root().to_path_buf(),
            path: path.to_path_buf(),
            fingerprint,
            grant: record.grant,
        };
        if !managed.current() {
            return Err("云语义配置尚未发布、已改变或已撤回".into());
        }
        let mut backend = Self::new(record.config)?;
        backend.managed = Some(Arc::new(managed));
        Ok(backend)
    }
}
pub(crate) fn write_config(
    vault: &crate::Vault,
    path: &Path,
    grant: CloudEmbeddingGrant,
    config: SemanticConfig,
) -> Result<()> {
    let record = ManagedConfig {
        schema: "recallcard.managed-cloud-semantic/1".into(),
        grant: grant.clone(),
        config,
    };
    let fingerprint = hash(&serde_json::to_vec(&record).map_err(|_| "云语义配置不可核验")?);
    vault.write_replace(path, &record)?;
    CloudEmbeddingRuntime::new(vault)
        .bind_configuration(&grant, &fingerprint)
        .map_err(|e| e.to_string())
}

pub(crate) fn is_managed_file(path: &Path) -> bool {
    read_local::<ManagedConfig>(path).is_ok()
}
