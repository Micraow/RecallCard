//! 已授权 MCP 的离线检索会话；模型请求不能选配置或扩大范围。
use super::{authorize, get, observe};
use crate::{context::Context, model::Result, policy::Access, semantic::SemanticSearch, Vault};
use serde_json::Value;
use std::{io::Read, path::PathBuf, sync::Mutex};

#[derive(Clone, PartialEq, Eq)]
struct ConfigIdentity {
    path: PathBuf,
    digest: String,
    bytes: Vec<u8>,
}
struct Loaded {
    identity: ConfigIdentity,
    backend: std::result::Result<SemanticSearch, String>,
}
/// 一个真实 MCP 进程共用同一个 worker 和编码预算；配置变更后不复用旧权限。
pub struct AgentReadService<'a> {
    vault: &'a Vault,
    id: &'a str,
    access: &'a Access,
    explicit_config: Option<PathBuf>,
    loaded: Mutex<Option<Loaded>>,
    event_vectors: Option<&'a crate::event_vectors::EventVectorIndex>,
}
impl<'a> AgentReadService<'a> {
    pub fn new(
        vault: &'a Vault,
        id: &'a str,
        access: &'a Access,
        explicit_config: Option<PathBuf>,
    ) -> Self {
        Self {
            vault,
            id,
            access,
            explicit_config,
            loaded: Mutex::new(None),
            event_vectors: None,
        }
    }
    pub fn with_event_vectors(
        mut self,
        index: Option<&'a crate::event_vectors::EventVectorIndex>,
    ) -> Self {
        self.event_vectors = index;
        self
    }
    fn selected_config(&self, access: &Access) -> Result<Option<ConfigIdentity>> {
        let path = match &self.explicit_config {
            Some(path) => Some(path.clone()),
            None => {
                let config = crate::application::background_memory::MemoryRuntime::new(self.vault)
                    .configuration()
                    .map_err(|_| "offline_config_invalid".to_string())?;
                if access.permits(&config.scope) {
                    config.candidate_semantic_config
                } else {
                    None
                }
            }
        };
        let Some(path) = path else { return Ok(None) };
        let read = || -> Result<Vec<u8>> {
            crate::vault::reject_symlink(&path)?;
            let file = crate::filesystem::open_local_file(&path)?;
            if !file.metadata().map_err(|e| e.to_string())?.is_file() {
                return Err("not_file".into());
            }
            let mut bytes = Vec::new();
            file.take(65537)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > 65536 {
                return Err("too_large".into());
            }
            Ok(bytes)
        };
        let bytes = read().map_err(|_| "offline_config_invalid".to_string())?;
        Ok(Some(ConfigIdentity {
            path,
            digest: crate::model::hash(&bytes),
            bytes,
        }))
    }
    pub fn invoke(&self, name: &str, args: Value) -> Result<Value> {
        let entry = get(self.vault, self.id)
            .map_err(|e| e.to_string())?
            .ok_or("Agent 连接尚未配置")?;
        let authorization = match authorize(self.vault, self.id, entry.permission_revision) {
            Ok(value) => value,
            Err(error) => {
                self.clear_worker();
                return Err(error.to_string());
            }
        };
        let grant = &authorization.entry.grant;
        if !matches!(
            grant.client_kind.as_str(),
            "claude_code" | "codex" | "chatgpt_mcp"
        ) || !grant.auto_recall
            || !grant.provider_disclosure
        {
            drop(authorization);
            self.clear_worker();
            return Err("此客户端的读取或接收方授权已暂停".into());
        }
        let scopes = grant
            .recall_scopes
            .iter()
            .filter(|scope| self.access.permits(scope))
            .cloned()
            .collect();
        // 提供器等待不持有连接锁。返回前 observe 在同一授权锁内重查 revision/撤权。
        drop(authorization);
        let access = match Access::new(scopes) {
            Ok(access) => access,
            Err(error) => {
                self.clear_worker();
                return Err(error);
            }
        };
        let result = if matches!(name, "search" | "bootstrap") {
            let before = self.selected_config(&access);
            let mut cache = self.loaded.lock().map_err(|_| "离线检索会话不可用")?;
            let mut error = None;
            match &before {
                Ok(Some(identity)) => {
                    if cache.as_ref().is_none_or(|c| c.identity != *identity) {
                        *cache = Some(Loaded {
                            identity: identity.clone(),
                            // 与身份摘要使用同一份已打开字节，避免路径二次读取的配置竞态。
                            backend: SemanticSearch::from_application_config_bytes(
                                self.vault,
                                &identity.path,
                                &access.scopes(),
                                &identity.bytes,
                            )
                            .map_err(|_| "offline_config_invalid".to_string()),
                        });
                    }
                }
                Ok(None) => {
                    *cache = None;
                }
                Err(code) => {
                    *cache = None;
                    error = Some(code.clone());
                }
            }
            let backend = cache.as_ref().and_then(|c| match &c.backend {
                Ok(backend) if backend.authorization_current() => Some(backend),
                Ok(_) => {
                    error = Some("cloud_approval_changed".into());
                    None
                }
                Err(code) => {
                    error = Some(code.clone());
                    None
                }
            });
            let context = match backend {
                Some(backend) => Context::with_semantic(self.vault, access.clone(), backend),
                None => Context::new(self.vault, access.clone()),
            };
            let context = context.with_event_vectors(self.event_vectors);
            let mut result = crate::transport::invoke(&context, name, args);
            if self.selected_config(&access) != before {
                *cache = None;
                result = Err("语义设置在查询期间改变，请使用当前设置重新检索".into());
            } else if let (Ok(value), Some(code)) = (&mut result, error) {
                value["coverage"]["semantic_error"] = serde_json::json!(code);
            }
            result
        } else {
            crate::transport::invoke(&Context::new(self.vault, access), name, args)
        };
        let observed = observe(
            self.vault,
            self.id,
            entry.permission_revision,
            if result.is_ok() {
                if name == "bootstrap" {
                    "bootstrap"
                } else {
                    "read"
                }
            } else {
                "failed"
            },
        );
        if let Err(error) = observed {
            self.clear_worker();
            return Err(error.to_string());
        }
        result
    }
    fn clear_worker(&self) {
        if let Ok(mut cache) = self.loaded.lock() {
            *cache = None;
        }
    }
}
