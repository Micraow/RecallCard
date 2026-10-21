//! 本机设置：编码与语言模型的服务地址、模型名与密钥。
//!
//! 密钥只存在本机状态目录（0600），不进 Vault、不进仓库、不出现在界面回显里。
//! 这里保存的是“用哪个服务”，不是“要不要联网”；任何联网仍由用户触发的动作发起。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::model::Result;
use crate::vault::Vault;

const FILE: &str = "settings.json";

/// 全局状态目录：`<base>/recallcard`，与各库的 hashed 目录同级。
pub fn global_dir() -> Result<PathBuf> {
    let base = std::env::var_os("RECALLCARD_STATE_DIR")
        .map(PathBuf::from)
        .or_else(dirs::state_dir)
        .or_else(dirs::data_local_dir)
        .ok_or("无法确定本机状态目录，请设置 RECALLCARD_STATE_DIR")?;
    let directory = base.join("recallcard");
    fs::create_dir_all(&directory).map_err(|_| "无法创建本机状态目录".to_owned())?;
    Ok(directory)
}

/// 编码模型的缓存目录。fastembed 默认用当前目录下的 `.fastembed_cache`，
/// 换个目录启动就"模型不可用"；这里固定到状态目录，并顺手迁移旧缓存。
pub fn model_cache_dir() -> Result<PathBuf> {
    let cache = global_dir()?.join("models");
    fs::create_dir_all(&cache).map_err(|_| "无法创建模型缓存目录".to_owned())?;
    let empty = fs::read_dir(&cache)
        .map(|entries| entries.count() == 0)
        .unwrap_or(false);
    if empty {
        if let Ok(current) = std::env::current_dir() {
            let legacy = current.join(".fastembed_cache");
            if legacy.is_dir() {
                copy_tree(&legacy, &cache);
            }
        }
    }
    Ok(cache)
}

fn copy_tree(from: &Path, to: &Path) {
    let Ok(entries) = fs::read_dir(from) else {
        return;
    };
    for entry in entries.flatten() {
        let source = entry.path();
        let target = to.join(entry.file_name());
        // 模型缓存里 blobs 与 snapshots 之间是软链；照抄实体文件会让缓存翻倍，
        // 所以这里保留链接本身。
        if let Ok(link) = fs::read_link(&source) {
            let _ = std::os::unix::fs::symlink(link, &target);
            continue;
        }
        if source.is_dir() {
            if fs::create_dir_all(&target).is_ok() {
                copy_tree(&source, &target);
            }
        } else if fs::copy(&source, &target).is_err() {
            // 迁移失败不算致命：下一次仍会尝试从网络下载。
        }
    }
}

/// 密钥栏最常见的填错是把服务地址又贴一遍，这在保存时就能看出来。
fn check_key_shape(key: &str, label: &str) -> Result<()> {
    if key.starts_with("http://") || key.starts_with("https://") {
        return Err(format!(
            "{label}的密钥填的是网址；访问凭据填在密钥栏，地址填在服务地址栏"
        ));
    }
    if key.chars().any(char::is_whitespace) {
        return Err(format!("{label}的密钥含空白字符，粘贴时可能带入了换行"));
    }
    Ok(())
}

fn https_or_empty(value: &str) -> Result<()> {
    if value.is_empty() || value.starts_with("https://") || value.starts_with("http://127.0.0.1") {
        Ok(())
    } else {
        Err("服务地址必须是 https，或本机 http://127.0.0.1".into())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EmbeddingSettings {
    /// `local` 用进程内模型，`cloud` 用 OpenAI 兼容的 /v1/embeddings。
    pub provider: String,
    pub endpoint: String,
    pub model: String,
    pub dimensions: usize,
    pub key: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LlmSettings {
    pub base_url: String,
    pub model: String,
    pub key: String,
    pub max_output_tokens: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub embedding: EmbeddingSettings,
    pub llm: LlmSettings,
}

impl Settings {
    pub fn path(vault: &Vault) -> Result<PathBuf> {
        Ok(vault.state_dir()?.join(FILE))
    }

    pub fn load(vault: &Vault) -> Result<Self> {
        let path = Self::path(vault)?;
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).map_err(|_| SETTINGS_BROKEN.to_owned()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(_) => Err(SETTINGS_BROKEN.to_owned()),
        }
    }

    /// 写入前逐项校验；先写临时文件再改名，避免半份配置被读走。
    pub fn save(vault: &Vault, next: &Settings) -> Result<()> {
        next.validate()?;
        let path = Self::path(vault)?;
        let temporary = path.with_extension("json.tmp");
        let text = serde_json::to_vec_pretty(next).map_err(|e| e.to_string())?;
        write_private(&temporary, &text)?;
        fs::rename(&temporary, &path).map_err(|_| SAVE_FAILED.to_owned())?;
        write_private(&path, &text)?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        self.check_keys()?;
        let embedding = &self.embedding;
        match embedding.provider.as_str() {
            "" | "local" | "cloud" => {}
            _ => return Err("编码服务只能是 local 或 cloud".into()),
        }
        if embedding.provider == "cloud" {
            https_or_empty(&embedding.endpoint)?;
            if embedding.endpoint.is_empty() || embedding.model.is_empty() {
                return Err("云端编码需要服务地址与模型名".into());
            }
            if embedding.dimensions == 0 || embedding.dimensions > 8192 {
                return Err("云端编码维度必须在 1–8192 之间".into());
            }
        }
        https_or_empty(&self.llm.base_url)?;
        if !self.llm.base_url.is_empty() && self.llm.model.is_empty() {
            return Err("填写模型服务地址后必须同时填写模型名".into());
        }
        Ok(())
    }

    /// 给界面的视图：密钥只回报是否已配置，不回报内容。
    pub fn public_view(&self) -> Value {
        json!({
            "embedding": {
                "provider": if self.embedding.provider.is_empty() { "local" } else { &self.embedding.provider },
                "endpoint": self.embedding.endpoint,
                "model": self.embedding.model,
                "dimensions": self.embedding.dimensions,
                "has_key": !self.embedding.key.is_empty(),
            },
            "llm": {
                "base_url": self.llm.base_url,
                "model": self.llm.model,
                "max_output_tokens": self.llm.max_output_tokens,
                "has_key": !self.llm.key.is_empty(),
            },
            "path": FILE,
        })
    }

    /// 界面提交的补丁：密钥字段为空表示“保持原值”，`clear_*` 用于真正删除。
    fn check_keys(&self) -> Result<()> {
        check_key_shape(&self.embedding.key, "编码服务")?;
        check_key_shape(&self.llm.key, "模型服务")?;
        Ok(())
    }

    pub fn apply_patch(&self, patch: &Value) -> Result<Self> {
        let mut next = self.clone();
        if let Some(embedding) = patch.get("embedding").and_then(Value::as_object) {
            if let Some(value) = embedding.get("provider").and_then(Value::as_str) {
                next.embedding.provider = value.into();
            }
            if let Some(value) = embedding.get("endpoint").and_then(Value::as_str) {
                next.embedding.endpoint = crate::providers::normalize_embedding_endpoint(value);
            }
            if let Some(value) = embedding.get("model").and_then(Value::as_str) {
                next.embedding.model = value.trim().into();
            }
            if let Some(value) = embedding.get("dimensions").and_then(Value::as_u64) {
                next.embedding.dimensions = value as usize;
            }
            if let Some(value) = embedding.get("key").and_then(Value::as_str) {
                if !value.trim().is_empty() {
                    next.embedding.key = value.trim().into();
                }
            }
            if embedding.get("clear_key") == Some(&Value::Bool(true)) {
                next.embedding.key.clear();
            }
        }
        if let Some(llm) = patch.get("llm").and_then(Value::as_object) {
            if let Some(value) = llm.get("base_url").and_then(Value::as_str) {
                next.llm.base_url = crate::providers::normalize_chat_base(value);
            }
            if let Some(value) = llm.get("model").and_then(Value::as_str) {
                next.llm.model = value.trim().into();
            }
            if let Some(value) = llm.get("max_output_tokens").and_then(Value::as_u64) {
                next.llm.max_output_tokens = value.clamp(256, 32_000) as u32;
            }
            if let Some(value) = llm.get("key").and_then(Value::as_str) {
                if !value.trim().is_empty() {
                    next.llm.key = value.trim().into();
                }
            }
            if llm.get("clear_key") == Some(&Value::Bool(true)) {
                next.llm.key.clear();
            }
        }
        next.validate()?;
        Ok(next)
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).map_err(|_| SAVE_FAILED.to_owned())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|_| SAVE_FAILED.to_owned())?;
    }
    Ok(())
}

const SETTINGS_BROKEN: &str = "本机设置文件无法解析；请修正或删除后重试";
const SAVE_FAILED: &str = "无法保存本机设置";

/// 一次重建是整库外发：上限给足，超了会报出实际字节数，不会静默只发一半。
pub const CLOUD_INDEX_BUDGET: usize = 16 * 1024 * 1024;

impl Settings {
    /// 设置是唯一事实源：编码怎么走只在这里决定一次，界面与命令行共用。
    pub fn semantic_config(
        &self,
        vault: &crate::vault::Vault,
    ) -> Result<crate::semantic::SemanticConfig> {
        let index_path = crate::semantic::vault_index_path(vault);
        if self.embedding.provider != "cloud" {
            return Ok(crate::semantic::SemanticConfig {
                provider: crate::semantic::Provider::Local,
                index_path,
            });
        }
        if self.embedding.key.is_empty() {
            return Err("云端编码需要密钥；请到设置页填写".into());
        }
        Ok(crate::semantic::SemanticConfig {
            provider: crate::semantic::Provider::Cloud {
                endpoint: self.embedding.endpoint.clone(),
                model: self.embedding.model.clone(),
                dimensions: self.embedding.dimensions,
                key_env: "RECALLCARD_EMBEDDING_KEY".into(),
                timeout_ms: 30_000,
                max_transmitted_bytes: CLOUD_INDEX_BUDGET,
                key: Some(self.embedding.key.clone()),
            },
            index_path,
        })
    }
}
