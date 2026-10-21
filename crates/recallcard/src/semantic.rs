//! 可选语义索引：本机 ONNX 编码或显式批准的云端编码。
//!
//! 索引是派生物，随时可由正本重建；正本文件本身永不外发，外发只发生在批准的云编码
//! 请求体内，且逐批计量。索引与正本通过 generation 与逐文档 content_hash 绑定，
//! 正本变化后旧索引立即失效，不会用过期向量冒充当前事实。
use crate::{
    context::Document,
    model::{hash, Result},
    policy::Access,
    Vault,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

pub const INDEX_SCHEMA: &str = "recallcard.semantic-index/2";
pub const CORPUS_SCHEMA: &str = "recallcard.semantic-corpus/2";

/// 本机默认编码模型；512 维、CLS 池化、L2 归一化，与 model-lock.json 一致。
pub const LOCAL_MODEL: &str = "Qdrant/bge-small-zh-v1.5";
pub const LOCAL_DIMENSIONS: usize = 512;
/// bge-zh 检索查询前缀；只加在查询侧，不加在文档侧。
pub const QUERY_INSTRUCTION: &str = "为这个句子生成表示以用于检索相关文章：";

const MAX_INDEX_BYTES: u64 = 512 * 1024 * 1024;
const MAX_DOCUMENTS: usize = 200_000;

/// 一批编码多少条：太小则请求频繁，太大则单次请求过重且看不到进度。
const ENCODE_CHUNK: usize = 32;

/// 同时发出几批编码。云端一次往返要几秒，串行编码上千条要几十分钟；
/// 并发发几路能接近线性加快，代价是更容易撞上限流（已在请求层做退避重试）。
const ENCODE_WORKERS: usize = 4;
const MAX_RESULTS: usize = 100;
const MAX_TEXT_BYTES: usize = 8 * 1024;
const LOCAL_BATCH: usize = 64;
const CLOUD_BATCH: usize = 16;
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_TRANSMIT_BYTES: usize = 4 * 1024 * 1024;
const DEFAULT_KEY_ENV: &str = "RECALLCARD_EMBEDDING_API_KEY";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SemanticError(pub &'static str);
type SResult<T> = std::result::Result<T, SemanticError>;

fn failure<T>(code: &'static str) -> SResult<T> {
    Err(SemanticError(code))
}

/// model::Result 以字符串承载错误；统一把 IO/JSON 错误映射为用户可读文本。
fn io(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// 编码来源。本机模型不需要批准；云端必须由本机维护者显式配置端点与凭据环境变量。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Provider {
    /// 进程内 ONNX 推理，模型固定；首次使用需要联网下载模型文件，之后走本地缓存。
    Local,
    /// OpenAI 兼容 /v1/embeddings；只接受 https 端点，凭据只从环境变量读取。
    Cloud {
        endpoint: String,
        model: String,
        dimensions: usize,
        #[serde(default = "default_key_env")]
        key_env: String,
        #[serde(default = "default_timeout")]
        timeout_ms: u64,
        #[serde(default = "default_transmit_bytes")]
        max_transmitted_bytes: usize,
        /// 界面填写的密钥只放在内存里，绝不写入索引、配置或日志。
        #[serde(skip)]
        key: Option<String>,
    },
}

fn default_key_env() -> String {
    DEFAULT_KEY_ENV.into()
}
fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT_MS
}
fn default_transmit_bytes() -> usize {
    DEFAULT_TRANSMIT_BYTES
}

/// 只能由用户或可信宿主在启动时提供；检索与 MCP 请求不能选择编码来源或联网权限。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticConfig {
    pub provider: Provider,
    #[serde(default = "default_index_path")]
    pub index_path: PathBuf,
}

fn default_index_path() -> PathBuf {
    PathBuf::from(".index/semantic")
}

/// 编码空间身份。任何一项变化都使旧索引整体失效，不同模型的向量不可混算。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Space {
    provider: String,
    model: String,
    dimensions: usize,
    query_instruction: String,
    document_instruction: String,
    normalization: String,
    preprocessing_version: String,
}

fn space_signature(space: &Space) -> Result<String> {
    Ok(hash(&serde_json::to_vec(space).map_err(io)?))
}

impl Provider {
    fn space(&self) -> Space {
        match self {
            Provider::Local => Space {
                provider: "local-onnx".into(),
                model: LOCAL_MODEL.into(),
                dimensions: LOCAL_DIMENSIONS,
                query_instruction: QUERY_INSTRUCTION.into(),
                document_instruction: String::new(),
                normalization: "l2".into(),
                preprocessing_version: "1".into(),
            },
            Provider::Cloud {
                model, dimensions, ..
            } => Space {
                provider: "cloud-openai-compatible".into(),
                model: model.clone(),
                dimensions: *dimensions,
                query_instruction: String::new(),
                document_instruction: String::new(),
                normalization: "l2".into(),
                preprocessing_version: "1".into(),
            },
        }
    }

    fn validate(&self) -> Result<()> {
        match self {
            Provider::Local => Ok(()),
            Provider::Cloud {
                endpoint,
                model,
                dimensions,
                key_env,
                timeout_ms,
                max_transmitted_bytes,
                key: _,
            } => {
                if !endpoint.starts_with("https://") || endpoint.len() > 2048 {
                    return Err("云端点必须是 https 绝对地址".into());
                }
                if endpoint.contains('@') || endpoint.contains("localhost") {
                    return Err("云端点不能内嵌凭据或指向本机".into());
                }
                if model.trim().is_empty() || model.len() > 256 {
                    return Err("模型标识为空或过长".into());
                }
                if !(1..=16384).contains(dimensions) {
                    return Err("维度必须在 1–16384 之间".into());
                }
                if !key_env
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    || key_env.is_empty()
                    || key_env.len() > 128
                {
                    return Err("凭据环境变量名不合法".into());
                }
                if *timeout_ms < 100 || *timeout_ms > 300_000 {
                    return Err("超时必须在 100–300000 毫秒之间".into());
                }
                if *max_transmitted_bytes < 1024 || *max_transmitted_bytes > 64 * 1024 * 1024 {
                    return Err("外发预算必须在 1 KiB–64 MiB 之间".into());
                }
                Ok(())
            }
        }
    }
}

/// 编码器：本机 ONNX 或批准的云端点。两者产出同一形状（L2 归一化定长向量）。
/// 编码器失败。
///
/// 只有编码器自己声明"已脱敏"的说明才允许出现在用户可见信息里：供应商错误页
/// 可能回显密钥或内部路径，别家编码器也可能随手带上原始正文。未经声明的一律
/// 只留固定文案，宁可少一句诊断，也不把不该外泄的内容写进日志和界面。
pub(crate) enum EncoderFailure {
    /// 编码器已自行抹掉密钥并截断的说明
    Redacted(String),
    /// 未说明原因
    Opaque,
}

impl EncoderFailure {
    fn public(&self) -> &str {
        match self {
            Self::Redacted(detail) => detail,
            Self::Opaque => "编码器没有给出可以公开的原因",
        }
    }
}

impl From<String> for EncoderFailure {
    fn from(_: String) -> Self {
        Self::Opaque
    }
}

impl From<&str> for EncoderFailure {
    fn from(_: &str) -> Self {
        Self::Opaque
    }
}

pub(crate) trait Encoder: Send + Sync {
    fn space(&self) -> Space;
    fn embed_documents(
        &self,
        texts: &[String],
    ) -> std::result::Result<Vec<Vec<f32>>, EncoderFailure>;
    fn embed_query(&self, text: &str) -> std::result::Result<Vec<f32>, EncoderFailure>;
    fn transmitted_bytes(&self) -> u64;
}

struct LocalEncoder {
    space: Space,
    model: Mutex<fastembed::TextEmbedding>,
}

impl Encoder for LocalEncoder {
    fn space(&self) -> Space {
        self.space.clone()
    }
    fn embed_documents(
        &self,
        texts: &[String],
    ) -> std::result::Result<Vec<Vec<f32>>, EncoderFailure> {
        self.embed(texts, "").map_err(EncoderFailure::Redacted)
    }
    fn embed_query(&self, text: &str) -> std::result::Result<Vec<f32>, EncoderFailure> {
        self.embed(&[text.to_string()], QUERY_INSTRUCTION)
            .map_err(EncoderFailure::Redacted)
            .map(|mut vectors| vectors.remove(0))
    }
    fn transmitted_bytes(&self) -> u64 {
        0
    }
}

impl LocalEncoder {
    fn open(provider: &Provider) -> Result<Self> {
        let Provider::Local = provider else {
            return Err("本机编码器配置类型不符".into());
        };
        // 缓存目录固定到本机状态目录：fastembed 默认相对当前目录，换个目录启动就找不到模型。
        let mut options = fastembed::TextInitOptions::new(fastembed::EmbeddingModel::BGESmallZHV15);
        options.cache_dir = crate::settings::model_cache_dir()?;
        let model = fastembed::TextEmbedding::try_new(options).map_err(|error| {
            // 原因必须留下：是没下载、还是缓存损坏，处理方式完全不同。
            format!("本机编码模型不可用；首次使用需要联网下载模型文件（{error}）")
        })?;
        Ok(Self {
            space: provider.space(),
            model: Mutex::new(model),
        })
    }

    fn embed(&self, texts: &[String], instruction: &str) -> Result<Vec<Vec<f32>>> {
        let prepared: Vec<String> = texts
            .iter()
            .map(|text| {
                if instruction.is_empty() {
                    text.clone()
                } else {
                    format!("{instruction}{text}")
                }
            })
            .collect();
        let mut model = self
            .model
            .lock()
            .map_err(|_| "本机编码器忙或已损坏，请重启进程".to_string())?;
        let mut vectors = Vec::with_capacity(prepared.len());
        for chunk in prepared.chunks(LOCAL_BATCH) {
            let batch = model
                .embed(chunk.iter().map(String::as_str).collect::<Vec<_>>(), None)
                .map_err(|_| "本机编码失败".to_string())?;
            for vector in batch {
                vectors.push(normalize(vector));
            }
        }
        if vectors.len() != texts.len() {
            return Err("本机编码输出数量与输入不符".into());
        }
        Ok(vectors)
    }
}

fn normalize(vector: Vec<f32>) -> Vec<f32> {
    let norm = vector
        .iter()
        .map(|v| (*v as f64).powi(2))
        .sum::<f64>()
        .sqrt();
    if norm == 0.0 || !norm.is_finite() {
        return vector;
    }
    vector.iter().map(|v| (*v as f64 / norm) as f32).collect()
}

struct CloudEncoder {
    space: Space,
    endpoint: String,
    model: String,
    dimensions: usize,
    api_key: String,
    timeout: Duration,
    budget: usize,
    transmitted: Mutex<u64>,
}

impl CloudEncoder {
    fn open(provider: &Provider) -> Result<Self> {
        let Provider::Cloud {
            endpoint,
            model,
            dimensions,
            key_env,
            timeout_ms,
            max_transmitted_bytes,
            key,
        } = provider
        else {
            return Err("云端编码器配置类型不符".into());
        };
        let api_key = match key.as_deref().filter(|value| !value.trim().is_empty()) {
            Some(value) => value.to_owned(),
            None => std::env::var(key_env).map_err(|_| {
                format!("云端编码需要凭据环境变量 {key_env}；不会从命令行或配置文件读取密钥")
            })?,
        };
        if api_key.trim().is_empty() {
            return Err(format!("凭据环境变量 {key_env} 为空"));
        }
        Ok(Self {
            space: provider.space(),
            endpoint: endpoint.clone(),
            model: model.clone(),
            dimensions: *dimensions,
            api_key,
            timeout: Duration::from_millis(*timeout_ms),
            budget: *max_transmitted_bytes,
            transmitted: Mutex::new(0),
        })
    }

    /// 记一次外发额度。返回即释放锁，真正的网络等待不占锁。
    fn reserve(&self, bytes: usize) -> Result<()> {
        let mut sent = self
            .transmitted
            .lock()
            .map_err(|_| "计量锁已损坏".to_string())?;
        if *sent + bytes as u64 > self.budget as u64 {
            return Err(format!(
                "云端外发预算已用尽：本次已发送 {} 字节，上限 {} 字节；分小时间范围多次整理，或改用本机模型",
                *sent, self.budget
            ));
        }
        *sent += bytes as u64;
        Ok(())
    }

    /// 这里的错误文案全部由我们自己写，供应商正文已脱敏，因此可以公开。
    fn request(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let body = json!({
            "model": self.model,
            "input": texts,
            "dimensions": self.dimensions,
            "encoding_format": "float",
        });
        let payload = serde_json::to_vec(&body).map_err(io)?;
        // 先扣额度再发请求：锁不能跨网络调用持有，否则并发就退化成排队。
        self.reserve(payload.len())?;
        let client = reqwest::blocking::Client::builder()
            .timeout(self.timeout)
            .build()
            .map_err(|_| "无法创建云端编码客户端".to_string())?;
        crate::log::note(
            "编码",
            format!(
                "发送 {} 条（{} 字节）到 {}",
                texts.len(),
                payload.len(),
                self.endpoint
            ),
        );
        let started = std::time::Instant::now();
        // 并发编码更容易撞上限流：429 与 5xx 退避重试几次，其余状态直接报错。
        let mut attempt = 0u32;
        let (status, text) = loop {
            attempt += 1;
            let response = client
                .post(&self.endpoint)
                .bearer_auth(&self.api_key)
                .header("content-type", "application/json")
                .body(payload.clone())
                .send()
                .map_err(|_| "云端编码请求失败；不会自动更换供应商".to_string())?;
            let status = response.status();
            crate::log::done("编码", format!("返回 {status}"), started);
            let text = response
                .text()
                .map_err(|_| "云端编码响应无法读取".to_string())?;
            let retryable = status.as_u16() == 429 || status.is_server_error();
            if retryable && attempt < 4 {
                // 重试要再发一次，额度也再记一次。
                self.reserve(payload.len())?;
                let wait = Duration::from_millis(600 * u64::from(attempt));
                crate::log::note("编码", format!("{status}，{} ms 后重试", wait.as_millis()));
                std::thread::sleep(wait);
                continue;
            }
            break (status, text);
        };
        if !status.is_success() {
            // 供应商错误页可能回显密钥，因此先抹掉密钥再截断，只保留可判断原因的部分。
            let detail = redact(&text, &self.api_key);
            let hint = match status.as_u16() {
                401 | 403 => "密钥无效或没有该模型的权限；到设置页重新填写",
                404 => "地址或模型名不对；检查服务地址是否以 /v1/embeddings 结尾",
                429 => "触发了限流；稍后重试或改用本机模型",
                _ => "",
            };
            return Err(match (hint.is_empty(), detail.is_empty()) {
                (false, false) => format!("云端编码返回 {status}：{hint}（{detail}）"),
                (false, true) => format!("云端编码返回 {status}：{hint}"),
                (true, false) => format!("云端编码返回状态 {status}：{detail}"),
                (true, true) => format!("云端编码返回状态 {status}"),
            });
        }
        let parsed: Value =
            serde_json::from_str(&text).map_err(|_| "云端编码响应不是有效 JSON".to_string())?;
        let rows = parsed["data"]
            .as_array()
            .ok_or("云端编码响应缺少 data 数组".to_string())?;
        if rows.len() != texts.len() {
            return Err("云端编码输出数量与输入不符".into());
        }
        let mut sorted: Vec<(usize, Vec<f32>)> = Vec::with_capacity(rows.len());
        for row in rows {
            let index = row["index"].as_u64().ok_or("缺少向量序号".to_string())? as usize;
            let values = row["embedding"]
                .as_array()
                .ok_or("缺少向量数值".to_string())?;
            if values.len() != self.dimensions {
                return Err("云端返回维度与配置不符".into());
            }
            let vector: Vec<f32> = values
                .iter()
                .map(|v| v.as_f64().ok_or_else(|| "向量含非数值".to_string()))
                .collect::<Result<Vec<f64>>>()?
                .into_iter()
                .map(|v| v as f32)
                .collect();
            sorted.push((index, normalize(vector)));
        }
        sorted.sort_by_key(|(index, _)| *index);
        if sorted.windows(2).any(|w| w[0].0 == w[1].0)
            || sorted.first().map(|(i, _)| *i) != Some(0)
            || sorted.last().map(|(i, _)| *i) != Some(texts.len() - 1)
        {
            return Err("云端返回序号不连续或重复".into());
        }
        Ok(sorted.into_iter().map(|(_, v)| v).collect())
    }
}

/// 供应商响应只用来判断原因：抹掉密钥、压缩空白、截断到一小段。
fn redact(text: &str, api_key: &str) -> String {
    let mut cleaned = match api_key.is_empty() {
        true => text.to_owned(),
        false => text.replace(api_key, "***"),
    };
    if let Some(position) = cleaned.find("Bearer ") {
        let rest = &cleaned[position + 7..];
        let end = rest.find(['"', ' ', '\\']).unwrap_or(rest.len());
        cleaned = format!("{}Bearer ***{}", &cleaned[..position], &rest[end..]);
    }
    let flattened = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let clipped: String = flattened.chars().take(240).collect();
    match flattened.chars().count() > 240 {
        true => format!("{clipped}…"),
        false => clipped,
    }
}

impl Encoder for CloudEncoder {
    fn space(&self) -> Space {
        self.space.clone()
    }
    fn embed_documents(
        &self,
        texts: &[String],
    ) -> std::result::Result<Vec<Vec<f32>>, EncoderFailure> {
        let mut vectors = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(CLOUD_BATCH) {
            vectors.extend(self.request(chunk).map_err(EncoderFailure::Redacted)?);
        }
        Ok(vectors)
    }
    fn embed_query(&self, text: &str) -> std::result::Result<Vec<f32>, EncoderFailure> {
        self.request(&[text.to_string()])
            .map_err(EncoderFailure::Redacted)
            .map(|mut vectors| vectors.remove(0))
    }
    fn transmitted_bytes(&self) -> u64 {
        self.transmitted.lock().map(|v| *v).unwrap_or(u64::MAX)
    }
}

fn open_encoder(provider: &Provider) -> Result<Box<dyn Encoder>> {
    provider.validate()?;
    match provider {
        Provider::Local => Ok(Box::new(LocalEncoder::open(provider)?)),
        Provider::Cloud { .. } => Ok(Box::new(CloudEncoder::open(provider)?)),
    }
}

// ---------------------------------------------------------------------------
// 语料：正本投影。Event 与 Memory 都进入索引，未整理的原始对话同样可被语义召回。
// ---------------------------------------------------------------------------

fn corpus_text(document: &Document) -> String {
    // 长正文只取前 MAX_TEXT_BYTES，避免单条吃掉整批预算；截断在 UTF-8 边界。
    crate::context::truncate_utf8(&document.text, MAX_TEXT_BYTES).to_string()
}

pub(crate) fn corpus_documents(
    documents: &[Document],
    scopes: &[String],
    time: DateTime<Utc>,
    instruction: &str,
) -> Vec<Value> {
    documents
        .iter()
        .filter(|document| {
            scopes.contains(&document.scope)
                && (document.kind == "event" || document.current_memory_at(time))
        })
        .filter_map(|document| {
            let text = corpus_text(document);
            // 只有元数据、没有正文的记录不参与语义检索：无可编码内容，
            // 而且部分供应商会直接以「input 不能包含空字符串」拒绝整批。
            if text.trim().is_empty() {
                return None;
            }
            Some(json!({
                "ref": document.reference,
                "kind": document.kind,
                "scope": document.scope,
                "content_hash": hash(document.text.as_bytes()),
                "input_hash": prepared_hash(instruction, &text),
                "text": text,
            }))
        })
        .collect()
}

fn prepared_hash(instruction: &str, text: &str) -> String {
    if instruction.is_empty() {
        hash(text.as_bytes())
    } else {
        hash(format!("{instruction}\n{text}").as_bytes())
    }
}

fn corpus_generation(documents: &[Value]) -> String {
    let identity: Vec<(&Value, &Value)> = documents
        .iter()
        .map(|d| (&d["ref"], &d["input_hash"]))
        .collect();
    hash(&serde_json::to_vec(&identity).expect("语料只包含可序列化 JSON"))
}

// ---------------------------------------------------------------------------
// 索引文件：元数据 JSON + 向量二进制。两者都在 .index/ 下，可整体删除后重建。
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexMeta {
    schema: String,
    space: Space,
    space_signature: String,
    generation: String,
    scope: Vec<String>,
    dimensions: usize,
    #[serde(default)]
    complete: bool,
    documents: Vec<IndexDocument>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexDocument {
    #[serde(rename = "ref")]
    reference: String,
    content_hash: String,
    input_hash: String,
    scope: String,
    kind: String,
}

/// 只读索引句柄。open() 完成全部校验；编码器在首次查询时才构造，
/// 未配置语义检索的进程不会加载模型，也不会联网。
pub struct SemanticIndex {
    config: SemanticConfig,
    meta: IndexMeta,
    vectors: Vec<Vec<f32>>,
    encoder: Mutex<Option<Box<dyn Encoder>>>,
}

impl SemanticIndex {
    /// 从配置打开索引。索引缺失、损坏、模型空间不符或正本已变化时返回明确错误码，
    /// 调用方据此降级为纯词法检索，不会静默返回错误结果。
    pub fn open(config: &Path) -> Result<Self> {
        let parsed: SemanticConfig = read_json_file(config)?;
        Self::from_config(parsed)
    }

    /// 库内默认索引：无配置时直接用 `<vault>/.index/semantic`。
    /// 只有本地编码能在无配置时自描述；云端端点不入索引，仍需显式配置。
    pub fn open_vault(root: &Path) -> Result<Self> {
        let dir = root.join(default_index_path());
        let meta: IndexMeta = read_json_file(&dir.join("index.json"))?;
        if meta.space.provider != "local-onnx" {
            return Err(
                "索引由云端编码空间生成，需要库里的编码设置；请到设置页确认端点与密钥".into(),
            );
        }
        Self::from_config(SemanticConfig {
            provider: Provider::Local,
            index_path: dir,
        })
    }

    /// 检索与 MCP 的唯一取索引入口：按库里保存的编码设置打开索引。
    ///
    /// 打不开就降级为纯词法检索，但必须把原因说出来——默默不检索比报错更难查。
    pub fn open_for_vault(vault: &crate::vault::Vault) -> Option<Self> {
        let config = match crate::settings::Settings::load(vault)
            .and_then(|settings| settings.semantic_config(vault))
        {
            Ok(config) => config,
            Err(error) => {
                crate::log::note("索引", format!("语义索引不可用：{error}"));
                return None;
            }
        };
        match Self::from_config(config) {
            Ok(index) => Some(index),
            Err(error) => {
                crate::log::note("索引", format!("语义索引不可用：{error}"));
                None
            }
        }
    }

    pub fn from_config(config: SemanticConfig) -> Result<Self> {
        config.provider.validate()?;
        if config.index_path.is_relative() {
            return Err("索引路径必须是绝对路径；配置里的相对路径无法确定库根".into());
        }
        let space = config.provider.space();
        let expected = space_signature(&space)?;
        let dir = config.index_path.clone();
        let meta: IndexMeta = read_json_file(&dir.join("index.json"))?;
        if meta.schema != INDEX_SCHEMA || !meta.complete {
            return Err(if meta.complete {
                "索引 schema 版本不符，请运行 recallcard index build".into()
            } else {
                "索引构建未完成，请重新运行 recallcard index build".into()
            });
        }
        if meta.space_signature != expected {
            return Err("索引由不同编码空间生成；请重建索引".into());
        }
        if meta.dimensions != space.dimensions || meta.documents.len() > MAX_DOCUMENTS {
            return Err("索引维度或规模不合法".into());
        }
        let vectors = read_vectors(
            &dir.join("vectors.bin"),
            meta.dimensions,
            meta.documents.len(),
        )?;
        Ok(Self {
            config,
            meta,
            vectors,
            encoder: Mutex::new(None),
        })
    }

    pub fn index_path(&self) -> &Path {
        &self.config.index_path
    }

    /// 编码查询。不访问正本、不持有 Vault 锁，因此不会阻塞写入。
    /// 编码器在首次调用时构造；构造失败只降级为词法检索，不中断请求。
    pub(crate) fn query_vector(&self, query: &str) -> SResult<Vec<f32>> {
        let mut slot = self
            .encoder
            .lock()
            .map_err(|_| SemanticError("encoder_unavailable"))?;
        if slot.is_none() {
            *slot = Some(
                open_encoder(&self.config.provider)
                    .map_err(|_| SemanticError("encoder_unavailable"))?,
            );
        }
        let encoder = slot.as_ref().expect("编码器已构造");
        let vector = encoder
            .embed_query(query)
            .map_err(|_| SemanticError("query_encode_failed"))?;
        if vector.len() != self.meta.dimensions {
            return failure("dimension_mismatch");
        }
        Ok(vector)
    }

    /// 用已编码的查询向量对当前正本打分。正本变化时返回错误码而非过期结果。
    pub(crate) fn rank(
        &self,
        vector: &[f32],
        documents: &[Document],
        limit: usize,
        scopes: &[String],
        time: DateTime<Utc>,
    ) -> SResult<Vec<String>> {
        self.check_current(documents, scopes, time)?;
        if vector.len() != self.meta.dimensions {
            return failure("dimension_mismatch");
        }
        let mut scored: Vec<(f32, usize)> = self
            .vectors
            .iter()
            .enumerate()
            .map(|(index, stored)| (dot(vector, stored), index))
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let take = limit.clamp(1, MAX_RESULTS).min(scored.len());
        Ok(scored[..take]
            .iter()
            .filter(|(score, _)| *score > 0.0)
            .map(|(_, index)| self.meta.documents[*index].reference.clone())
            .collect())
    }

    pub fn document_count(&self) -> usize {
        self.meta.documents.len()
    }

    pub fn space_summary(&self) -> Value {
        json!({
            "provider": self.meta.space.provider,
            "model": self.meta.space.model,
            "dimensions": self.meta.dimensions,
            "documents": self.meta.documents.len(),
            "generation": self.meta.generation,
            "complete": self.meta.complete,
        })
    }

    fn check_current(
        &self,
        documents: &[Document],
        scopes: &[String],
        time: DateTime<Utc>,
    ) -> SResult<()> {
        if self.meta.scope.iter().collect::<BTreeSet<_>>() != scopes.iter().collect::<BTreeSet<_>>()
        {
            return failure("scope_mismatch");
        }
        let instruction = &self.meta.space.document_instruction;
        let corpus = corpus_documents(documents, scopes, time, instruction);
        if corpus_generation(&corpus) != self.meta.generation
            || corpus.len() != self.meta.documents.len()
        {
            return failure("generation_mismatch");
        }
        let live: BTreeMap<&str, &Document> = documents
            .iter()
            .map(|document| (document.reference.as_str(), document))
            .collect();
        for (index, document) in self.meta.documents.iter().enumerate() {
            let Some(current) = live.get(document.reference.as_str()) else {
                return failure("generation_mismatch");
            };
            let Some(row) = corpus.get(index) else {
                return failure("generation_mismatch");
            };
            if row["ref"].as_str() != Some(document.reference.as_str()) {
                return failure("generation_mismatch");
            }
            if current.scope != document.scope
                || document.content_hash != hash(current.text.as_bytes())
                || document.input_hash != prepared_hash(instruction, &corpus_text(current))
            {
                return failure("content_mismatch");
            }
        }
        Ok(())
    }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn read_vectors(path: &Path, dimensions: usize, rows: usize) -> Result<Vec<Vec<f32>>> {
    crate::vault::reject_symlink(path)?;
    let expected = rows * dimensions * 4;
    let metadata = fs::metadata(path).map_err(|e| format!("向量文件不可读：{e}"))?;
    if metadata.len() as usize != expected || metadata.len() > MAX_INDEX_BYTES {
        return Err("向量文件与索引元数据不匹配；请重建索引".into());
    }
    let mut buffer = Vec::with_capacity(expected);
    File::open(path)
        .map_err(io)?
        .read_to_end(&mut buffer)
        .map_err(io)?;
    let (rows, remainder) = buffer.as_chunks();
    if !remainder.is_empty() {
        return Err("向量文件长度不是 4 字节对齐".into());
    }
    Ok(rows
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect::<Vec<f32>>()
        .chunks_exact(dimensions)
        .map(|row| row.to_vec())
        .collect())
}

fn read_json_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    crate::vault::reject_symlink(path)?;
    let metadata = fs::metadata(path).map_err(|e| format!("配置或索引不可读：{e}"))?;
    if !metadata.is_file() || metadata.len() > MAX_INDEX_BYTES {
        return Err("配置或索引不是有界常规文件".into());
    }
    let mut text = String::new();
    File::open(path)
        .map_err(io)?
        .read_to_string(&mut text)
        .map_err(io)?;
    serde_json::from_str(&text).map_err(|e| format!("配置或索引 JSON 无效：{e}"))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        crate::vault::reject_symlink(parent)?;
        fs::create_dir_all(parent).map_err(io)?;
    }
    let temporary = path.with_extension("tmp");
    {
        let mut file = File::create(&temporary).map_err(io)?;
        file.write_all(bytes).map_err(io)?;
        file.sync_all().map_err(io)?;
    }
    fs::rename(&temporary, path).map_err(io)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 构建与状态
// ---------------------------------------------------------------------------

/// 增量重建索引：input_hash 未变化的文档沿用旧向量，只编码新增与变更部分。
pub fn build_index(vault: &Vault, access: Access, config: Option<&Path>) -> Result<Value> {
    let parsed = resolve_config(vault, config)?;
    let encoder = open_encoder(&parsed.provider)?;
    build_index_with_encoder(vault, access, &parsed, encoder.as_ref())
}

/// 用调用方给出的完整配置重建索引；界面按本机设置选择本地或云端编码。
pub fn build_index_from_config(
    vault: &Vault,
    access: Access,
    config: &SemanticConfig,
) -> Result<Value> {
    build_index_from_config_with_progress(vault, access, config, &|_, _| {})
}

/// 带进度的重建入口：`on_progress(已编码, 总数)`，界面用它显示进度条。
pub fn build_index_from_config_with_progress(
    vault: &Vault,
    access: Access,
    config: &SemanticConfig,
    on_progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Value> {
    config.provider.validate()?;
    let encoder = open_encoder(&config.provider)?;
    build_index_with_progress(vault, access, config, encoder.as_ref(), on_progress)
}

/// 库里已经有索引时，把新增的记录补进去；没有索引就不建。
///
/// 索引是派生物，补建失败只影响语义检索这一路，不能影响写入结果本身，
/// 所以返回 Option 并把错误交给调用方决定是否提示。
pub fn refresh_if_present(vault: &Vault, access: Access) -> Option<Result<Value>> {
    let path = vault_index_path(vault);
    if !path.join("index.json").is_file() {
        return None;
    }
    let config = SemanticConfig {
        provider: Provider::Local,
        index_path: path,
    };
    Some(build_index_from_config(vault, access, &config))
}

/// 库内默认索引位置，供界面与本机设置共同引用。
pub fn vault_index_path(vault: &Vault) -> PathBuf {
    vault.root().join(default_index_path())
}

fn resolve_config(vault: &Vault, config: Option<&Path>) -> Result<SemanticConfig> {
    let parsed: SemanticConfig = match config {
        Some(path) => read_json_file(path)?,
        None => SemanticConfig {
            provider: Provider::Local,
            index_path: vault.root().join(default_index_path()),
        },
    };
    parsed.provider.validate()?;
    if !parsed.index_path.is_absolute() && config.is_some() {
        return Err("配置文件中的 index_path 必须是绝对路径".into());
    }
    Ok(parsed)
}

/// 构建核心。编码器由调用方提供：生产走 open_encoder，测试注入确定性实现。
pub(crate) fn build_index_with_encoder(
    vault: &Vault,
    access: Access,
    parsed: &SemanticConfig,
    encoder: &dyn Encoder,
) -> Result<Value> {
    build_index_with_progress(vault, access, parsed, encoder, &|_, _| {})
}

/// 带进度的重建：`on_progress(已完成, 总数)`，供界面显示进度条。
pub(crate) fn build_index_with_progress(
    vault: &Vault,
    access: Access,
    parsed: &SemanticConfig,
    encoder: &dyn Encoder,
    on_progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Value> {
    let index_path = if parsed.index_path.is_absolute() {
        parsed.index_path.clone()
    } else {
        vault.root().join(&parsed.index_path)
    };
    crate::vault::reject_symlink(&index_path)?;

    let space = encoder.space();
    let signature = space_signature(&space)?;
    let scopes = access.scopes().to_vec();

    let _guard = vault.read_guard()?;
    let context = crate::context::Context::new(vault, access);
    let documents = context.documents()?;
    let now = Utc::now();
    let corpus = corpus_documents(&documents, &scopes, now, &space.document_instruction);
    if corpus.len() > MAX_DOCUMENTS {
        return Err(format!("语料超过 {MAX_DOCUMENTS} 条上限；请先缩小授权范围"));
    }
    let generation = corpus_generation(&corpus);

    // 旧索引只在编码空间一致时可复用向量；不同模型的向量绝不混算。
    let previous: Option<(IndexMeta, Vec<Vec<f32>>)> = (|| {
        let meta: IndexMeta = read_json_file(&index_path.join("index.json")).ok()?;
        if meta.schema != INDEX_SCHEMA || meta.space_signature != signature || !meta.complete {
            return None;
        }
        let vectors = read_vectors(
            &index_path.join("vectors.bin"),
            meta.dimensions,
            meta.documents.len(),
        )
        .ok()?;
        Some((meta, vectors))
    })();

    let reuse: BTreeMap<String, (usize, &IndexDocument)> = previous
        .as_ref()
        .map(|(meta, _)| {
            meta.documents
                .iter()
                .enumerate()
                .map(|(index, document)| (document.reference.clone(), (index, document)))
                .collect()
        })
        .unwrap_or_default();

    let mut output_documents = Vec::with_capacity(corpus.len());
    let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(corpus.len());
    let mut pending: Vec<(usize, String)> = Vec::new();
    let mut reused = 0usize;

    for (position, row) in corpus.iter().enumerate() {
        let reference = row["ref"].as_str().unwrap_or_default().to_string();
        let input_hash = row["input_hash"].as_str().unwrap_or_default().to_string();
        let document = IndexDocument {
            reference: reference.clone(),
            content_hash: row["content_hash"].as_str().unwrap_or_default().to_string(),
            input_hash: input_hash.clone(),
            scope: row["scope"].as_str().unwrap_or_default().to_string(),
            kind: row["kind"].as_str().unwrap_or_default().to_string(),
        };
        let reusable = reuse
            .get(&reference)
            .filter(|(_, old)| old.input_hash == input_hash && old.scope == document.scope)
            .and_then(|(old_index, _)| previous.as_ref()?.1.get(*old_index))
            .filter(|vector| vector.len() == space.dimensions);
        if let Some(vector) = reusable {
            vectors.push(vector.clone());
            reused += 1;
        } else {
            vectors.push(Vec::new());
            pending.push((
                position,
                row["text"].as_str().unwrap_or_default().to_string(),
            ));
        }
        output_documents.push(document);
    }

    let total = corpus.len();
    on_progress(total - pending.len(), total);
    let encoded = if pending.is_empty() {
        0
    } else {
        for (position, vector) in encode_pending(&pending, encoder, &space, total, on_progress)? {
            vectors[position] = vector;
        }
        pending.len()
    };

    if vectors
        .iter()
        .any(|vector| vector.len() != space.dimensions)
    {
        return Err("存在未填充的向量；索引未写入".into());
    }

    // 半成品先写 complete=false，全部落盘后才翻转，避免中断留下可用假象。
    let partial = IndexMeta {
        schema: INDEX_SCHEMA.into(),
        space: space.clone(),
        space_signature: signature.clone(),
        generation: generation.clone(),
        scope: scopes.clone(),
        dimensions: space.dimensions,
        complete: false,
        documents: output_documents.clone(),
    };
    write_atomic(
        &index_path.join("index.json"),
        &serde_json::to_vec_pretty(&partial).map_err(io)?,
    )?;
    write_atomic(&index_path.join("vectors.bin"), &pack_vectors(&vectors))?;
    let final_meta = IndexMeta {
        complete: true,
        ..partial
    };
    write_atomic(
        &index_path.join("index.json"),
        &serde_json::to_vec_pretty(&final_meta).map_err(io)?,
    )?;

    Ok(json!({
        "ok": true,
        "index_path": index_path,
        "documents": output_documents.len(),
        "encoded": encoded,
        "reused": reused,
        "generation": generation,
        "transmitted_bytes": encoder.transmitted_bytes(),
        "space": {
            "provider": space.provider,
            "model": space.model,
            "dimensions": space.dimensions,
        },
        "note": "索引是派生物，可用 recallcard index build 重建；正本文件不受影响"
    }))
}

fn pack_vectors(vectors: &[Vec<f32>]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vectors.len() * vectors.first().map_or(0, |v| v.len()) * 4);
    for vector in vectors {
        for value in vector {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

/// 报告索引覆盖，不触发编码或联网。
pub fn index_status(vault: &Vault) -> Result<Value> {
    let path = vault.root().join(default_index_path());
    let meta: IndexMeta = match read_json_file(&path.join("index.json")) {
        Ok(meta) => meta,
        Err(error) => {
            return Ok(json!({
                "configured": false,
                "index_path": path,
                "reason": error,
                "note": "未配置语义索引时检索走纯词法通道"
            }))
        }
    };
    let vectors = read_vectors(
        &path.join("vectors.bin"),
        meta.dimensions,
        meta.documents.len(),
    );
    let kinds =
        meta.documents
            .iter()
            .fold(BTreeMap::<String, usize>::new(), |mut acc, document| {
                *acc.entry(document.kind.clone()).or_insert(0) += 1;
                acc
            });
    Ok(json!({
        "configured": true,
        "index_path": path,
        "schema": meta.schema,
        "complete": meta.complete,
        "provider": meta.space.provider,
        "model": meta.space.model,
        "dimensions": meta.dimensions,
        "documents": meta.documents.len(),
        "by_kind": kinds,
        "scope": meta.scope,
        "generation": meta.generation,
        "vectors_readable": vectors.is_ok(),
    }))
}

/// 并发编码待处理的文档，按原始位置回填向量。
///
/// 每批仍然走 `Encoder::embed_documents`，所以批大小、脱敏、维度校验都不变；
/// 变的只是同时发出几批。任何一批失败都会让其余线程尽快停下并返回同一个错误。
fn encode_pending(
    pending: &[(usize, String)],
    encoder: &dyn Encoder,
    space: &Space,
    total: usize,
    on_progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Vec<(usize, Vec<f32>)>> {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    let chunks: Vec<&[(usize, String)]> = pending.chunks(ENCODE_CHUNK).collect();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(total - pending.len());
    let failed = AtomicBool::new(false);
    let results: Mutex<Vec<(usize, Vec<f32>)>> = Mutex::new(Vec::new());
    let failure: Mutex<Option<String>> = Mutex::new(None);
    let workers = ENCODE_WORKERS.min(chunks.len()).max(1);

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                if failed.load(Ordering::Relaxed) {
                    break;
                }
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(chunk) = chunks.get(index) else {
                    break;
                };
                let texts: Vec<String> = chunk.iter().map(|(_, text)| text.clone()).collect();
                let started = std::time::Instant::now();
                let outcome = encoder.embed_documents(&texts).and_then(|batch| {
                    if batch.len() != texts.len() {
                        return Err(EncoderFailure::Redacted(
                            "编码输出数量与待编码文档不符".into(),
                        ));
                    }
                    if batch.iter().any(|vector| vector.len() != space.dimensions) {
                        return Err(EncoderFailure::Redacted("编码输出维度与空间不符".into()));
                    }
                    Ok(batch)
                });
                match outcome {
                    Ok(batch) => {
                        if let Ok(mut guard) = results.lock() {
                            guard.extend(
                                chunk
                                    .iter()
                                    .zip(batch)
                                    .map(|((position, _), vector)| (*position, vector)),
                            );
                        }
                        let processed =
                            done.fetch_add(chunk.len(), Ordering::Relaxed) + chunk.len();
                        crate::log::done(
                            "索引",
                            format!("编码 {processed}/{total}（本批 {} 条）", chunk.len()),
                            started,
                        );
                        on_progress(processed, total);
                    }
                    Err(error) => {
                        if let Ok(mut guard) = failure.lock() {
                            if guard.is_none() {
                                *guard = Some(error.public().to_string());
                            }
                        }
                        failed.store(true, Ordering::Relaxed);
                        break;
                    }
                }
            });
        }
    });

    if let Some(reason) = failure.into_inner().unwrap_or(None) {
        return Err(format!("编码失败（索引未写入，正文未外泄）：{reason}"));
    }
    results
        .into_inner()
        .map_err(|_| "编码结果不可用".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::{Context, SearchArgs},
        model::Source,
        policy::Access,
        EventInput, Origin, Role, Vault,
    };

    /// 确定性假编码器：字符二元组频次向量，L2 归一化。
    /// 让"连接池"查询与含"连接池"的文档得分最高，且构建/查询两次结果一致。
    struct FakeEncoder;

    const FAKE_DIM: usize = 64;

    fn fake_vector(text: &str) -> Vec<f32> {
        let chars: Vec<char> = text.chars().collect();
        let mut vector = vec![0f32; FAKE_DIM];
        for window in chars.windows(2) {
            let bigram: String = window.iter().collect();
            let digest = hash(bigram.as_bytes());
            let index = u8::from_str_radix(&digest[..2], 16).unwrap() as usize % FAKE_DIM;
            vector[index] += 1.0;
        }
        normalize(vector)
    }

    impl Encoder for FakeEncoder {
        fn space(&self) -> Space {
            Space {
                provider: "fake-test".into(),
                model: "fake-v1".into(),
                dimensions: FAKE_DIM,
                query_instruction: String::new(),
                document_instruction: String::new(),
                normalization: "l2".into(),
                preprocessing_version: "1".into(),
            }
        }
        fn embed_documents(
            &self,
            texts: &[String],
        ) -> std::result::Result<Vec<Vec<f32>>, EncoderFailure> {
            Ok(texts.iter().map(|t| fake_vector(t)).collect())
        }
        fn embed_query(&self, text: &str) -> std::result::Result<Vec<f32>, EncoderFailure> {
            Ok(fake_vector(text))
        }
        fn transmitted_bytes(&self) -> u64 {
            0
        }
    }

    fn setup() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::init(&dir.path().join("vault")).unwrap();
        (dir, vault)
    }

    fn capture(vault: &Vault, content: &str) -> crate::model::Event {
        let input = EventInput {
            occurred_at: None,
            scope: "personal".into(),
            session_id: Some("conv".into()),
            turn_id: None,
            run_id: None,
            step_id: None,
            revision_of: None,
            reply_to: None,
            caused_by: None,
            kind: "message".into(),
            parts: Vec::new(),
            metadata: Value::Null,
            capture: Default::default(),
            role: Role::User,
            origin: Origin::UserInput,
            content: content.into(),
            source: Source {
                platform: "test".into(),
                account_namespace: String::new(),
                conversation_id: "conv".into(),
                message_id: format!("m_{}", hash(content.as_bytes())),
                url: None,
            },
        };
        vault.capture(input).unwrap()
    }

    fn access() -> Access {
        Access::new(vec!["personal".into()]).unwrap()
    }

    fn build(vault: &Vault) -> Value {
        let config = SemanticConfig {
            provider: Provider::Local,
            index_path: vault.root().join(".index/semantic"),
        };
        build_index_with_encoder(vault, access(), &config, &FakeEncoder).unwrap()
    }

    fn open_index(vault: &Vault) -> SemanticIndex {
        let dir = vault.root().join(".index/semantic");
        let meta: IndexMeta =
            serde_json::from_slice(&fs::read(dir.join("index.json")).unwrap()).unwrap();
        let vectors = read_vectors(
            &dir.join("vectors.bin"),
            meta.dimensions,
            meta.documents.len(),
        )
        .unwrap();
        SemanticIndex {
            config: SemanticConfig {
                provider: Provider::Local,
                index_path: dir,
            },
            meta,
            vectors,
            encoder: Mutex::new(Some(Box::new(FakeEncoder))),
        }
    }

    fn search_args(query: &str) -> SearchArgs {
        SearchArgs {
            query: query.into(),
            target: "all".into(),
            session_ref: None,
            as_of: None,
            limit: 5,
            detail: "brief".into(),
            budget_bytes: 8192,
            cursor: None,
        }
    }

    #[test]
    fn build_writes_complete_index_covering_events() {
        let (_d, vault) = setup();
        capture(&vault, "用户修复了数据库连接池泄漏");
        capture(&vault, "用户喜欢在深夜重构代码");
        let report = build(&vault);
        assert_eq!(report["complete"], Value::Null); // complete 在文件里，不在报告里
        assert!(report["documents"].as_u64().unwrap() >= 2);
        assert_eq!(report["encoded"].as_u64().unwrap(), 2);
        assert_eq!(report["reused"].as_u64().unwrap(), 0);
        let meta: IndexMeta = serde_json::from_slice(
            &fs::read(vault.root().join(".index/semantic/index.json")).unwrap(),
        )
        .unwrap();
        assert!(meta.complete);
        assert!(meta.documents.iter().all(|d| d.kind == "event"));
    }

    #[test]
    fn rebuild_is_incremental_by_input_hash() {
        let (_d, vault) = setup();
        capture(&vault, "第一条事实");
        build(&vault);
        capture(&vault, "第二条事实");
        let report = build(&vault);
        assert_eq!(report["encoded"].as_u64().unwrap(), 1);
        assert_eq!(report["reused"].as_u64().unwrap(), 1);
    }

    #[test]
    fn semantic_ranking_lifts_matching_document() {
        let (_d, vault) = setup();
        capture(&vault, "数据库连接池泄漏已在深夜修复");
        capture(&vault, "周末计划去爬山看日出");
        capture(&vault, "早餐吃了豆浆和油条");
        build(&vault);
        let index = open_index(&vault);
        let context = Context::with_semantic(&vault, access(), &index);
        let response = context.search(search_args("连接池")).unwrap();
        assert_eq!(response["coverage"]["semantic_search"], "available");
        assert_eq!(response["coverage"]["ranking"], "rrf");
        let first = response["results"][0]["text"].as_str().unwrap();
        assert!(first.contains("连接池"));
    }

    #[test]
    fn capture_after_build_invalidates_rank_not_request() {
        let (_d, vault) = setup();
        capture(&vault, "已索引的事实");
        build(&vault);
        let index = open_index(&vault);
        capture(&vault, "构建之后新增的事实");
        // 正本变化后语义通道降级为固定错误码；请求整体仍成功返回词法结果。
        let context = Context::with_semantic(&vault, access(), &index);
        let response = context.search(search_args("事实")).unwrap();
        assert_eq!(response["coverage"]["semantic_search"], "unavailable");
        assert_eq!(
            response["coverage"]["semantic_error"],
            "generation_mismatch"
        );
    }

    #[test]
    fn index_from_other_scope_is_rejected() {
        let (_d, vault) = setup();
        capture(&vault, "个人范围事实");
        build(&vault);
        let index = open_index(&vault);
        let wide = Access::new(vec!["personal".into(), "project".into()]).unwrap();
        let context = Context::with_semantic(&vault, wide, &index);
        let response = context.search(search_args("事实")).unwrap();
        assert_eq!(response["coverage"]["semantic_error"], "scope_mismatch");
    }

    #[test]
    fn incomplete_or_foreign_index_is_refused_at_open() {
        let (_d, vault) = setup();
        capture(&vault, "任意事实");
        build(&vault);
        let dir = vault.root().join(".index/semantic");
        // complete=false 的半成品不可用
        let mut meta: Value =
            serde_json::from_slice(&fs::read(dir.join("index.json")).unwrap()).unwrap();
        meta["complete"] = json!(false);
        fs::write(dir.join("index.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
        let config = SemanticConfig {
            provider: Provider::Local,
            index_path: dir.clone(),
        };
        let error = match SemanticIndex::from_config(config) {
            Err(error) => error,
            Ok(_) => panic!("半成品索引不应可用"),
        };
        assert!(error.contains("未完成"), "{error}");
        // 恢复 complete，改模型空间 → 签名不符
        meta["complete"] = json!(true);
        fs::write(dir.join("index.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
        let config = SemanticConfig {
            provider: Provider::Cloud {
                endpoint: "https://example.com/v1/embeddings".into(),
                model: "text-embedding-v4".into(),
                dimensions: 1024,
                key_env: default_key_env(),
                timeout_ms: default_timeout(),
                max_transmitted_bytes: default_transmit_bytes(),
                key: None,
            },
            index_path: dir,
        };
        let error = match SemanticIndex::from_config(config) {
            Err(error) => error,
            Ok(_) => panic!("不同编码空间的索引不应可用"),
        };
        assert!(error.contains("编码空间"), "{error}");
    }

    #[test]
    fn cloud_provider_validation_rejects_insecure_endpoints() {
        let base = |endpoint: &str, key_env: &str| Provider::Cloud {
            endpoint: endpoint.into(),
            model: "text-embedding-v4".into(),
            dimensions: 1024,
            key_env: key_env.into(),
            timeout_ms: default_timeout(),
            max_transmitted_bytes: default_transmit_bytes(),
            key: None,
        };
        assert!(base("http://api.example.com/v1/embeddings", "K")
            .validate()
            .is_err());
        assert!(base("https://localhost/v1/embeddings", "K")
            .validate()
            .is_err());
        assert!(base("https://user:pass@example.com/v1", "K")
            .validate()
            .is_err());
        assert!(base("https://api.example.com/v1", "BAD-ENV!")
            .validate()
            .is_err());
        assert!(base("https://api.example.com/v1", "RECALLCARD_KEY")
            .validate()
            .is_ok());
    }

    #[test]
    fn encoder_error_surfaces_as_fixed_code_only() {
        struct Broken;
        impl Encoder for Broken {
            fn space(&self) -> Space {
                FakeEncoder.space()
            }
            fn embed_documents(
                &self,
                _: &[String],
            ) -> std::result::Result<Vec<Vec<f32>>, EncoderFailure> {
                Err("供应商内部路径 /opt/secret 与密钥 sk-abc 泄漏样本".into())
            }
            fn embed_query(&self, _: &str) -> std::result::Result<Vec<f32>, EncoderFailure> {
                Err("供应商内部路径 /opt/secret 与密钥 sk-abc 泄漏样本".into())
            }
            fn transmitted_bytes(&self) -> u64 {
                0
            }
        }
        let (_d, vault) = setup();
        capture(&vault, "任意事实");
        let config = SemanticConfig {
            provider: Provider::Local,
            index_path: vault.root().join(".index/semantic"),
        };
        let error = build_index_with_encoder(&vault, access(), &config, &Broken).unwrap_err();
        assert!(error.contains("编码"), "{error}");
        assert!(
            !error.contains("secret") && !error.contains("sk-abc"),
            "{error}"
        );
    }

    #[test]
    fn batches_are_encoded_concurrently() {
        // 每批固定睡 100ms：串行 4 批要 400ms 以上，并发应该在一批的量级。
        struct Slow;
        impl Encoder for Slow {
            fn space(&self) -> Space {
                FakeEncoder.space()
            }
            fn embed_documents(
                &self,
                texts: &[String],
            ) -> std::result::Result<Vec<Vec<f32>>, EncoderFailure> {
                std::thread::sleep(std::time::Duration::from_millis(100));
                Ok(texts.iter().map(|_| vec![0.1; FAKE_DIM]).collect())
            }
            fn embed_query(&self, _: &str) -> std::result::Result<Vec<f32>, EncoderFailure> {
                Ok(vec![0.1; FAKE_DIM])
            }
            fn transmitted_bytes(&self) -> u64 {
                0
            }
        }
        let (_d, vault) = setup();
        for index in 0..(ENCODE_CHUNK * 4) {
            capture(&vault, &format!("第 {index} 条并发测试事实"));
        }
        let config = SemanticConfig {
            provider: Provider::Local,
            index_path: vault.root().join(".index/semantic"),
        };
        let started = std::time::Instant::now();
        build_index_with_encoder(&vault, access(), &config, &Slow).expect("重建成功");
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_millis(150 + 100 * ENCODE_CHUNK as u64 * 4 / 1000),
            "四批应当并发发出，实际用了 {elapsed:?}"
        );
    }

    #[test]
    fn rebuild_reports_progress_from_zero_to_total() {
        let (_d, vault) = setup();
        for text in ["第一条事实", "第二条事实", "第三条事实"] {
            capture(&vault, text);
        }
        let config = SemanticConfig {
            provider: Provider::Local,
            index_path: vault.root().join(".index/semantic"),
        };
        let seen = std::sync::Mutex::new(Vec::<(usize, usize)>::new());
        build_index_with_progress(&vault, access(), &config, &FakeEncoder, &|done, total| {
            seen.lock().unwrap().push((done, total));
        })
        .expect("重建成功");
        let seen = seen.into_inner().unwrap();
        assert_eq!(seen.first(), Some(&(0, 3)), "起步就要报总数：{seen:?}");
        assert_eq!(seen.last(), Some(&(3, 3)), "结束时必须是满进度：{seen:?}");
    }

    #[test]
    fn documents_without_text_never_reach_the_encoder() {
        let document = |reference: &str, text: &str| crate::context::Document {
            reference: reference.into(),
            text: text.into(),
            scope: "personal".into(),
            kind: "event".into(),
            evidence_refs: Vec::new(),
            session_ref: None,
            state: "active".into(),
            occurred_at: None,
            valid_from: None,
            valid_to: None,
            time_note: String::new(),
            evidence: String::new(),
            labels: Vec::new(),
            entities: Vec::new(),
            protected: false,
        };
        let corpus = corpus_documents(
            &[
                document("event:空", ""),
                document("event:只有空白", "   \n\t "),
                document("event:有正文", "周末想去爬山"),
            ],
            &["personal".to_string()],
            Utc::now(),
            "",
        );
        assert_eq!(corpus.len(), 1, "只有带正文的记录该进语料：{corpus:?}");
        assert_eq!(corpus[0]["ref"], "event:有正文");
    }

    #[test]
    fn declared_redacted_reason_reaches_the_message() {
        struct Explained;
        impl Encoder for Explained {
            fn space(&self) -> Space {
                FakeEncoder.space()
            }
            fn embed_documents(
                &self,
                _: &[String],
            ) -> std::result::Result<Vec<Vec<f32>>, EncoderFailure> {
                Err(EncoderFailure::Redacted(
                    "云端编码返回 401：密钥无效".into(),
                ))
            }
            fn embed_query(&self, _: &str) -> std::result::Result<Vec<f32>, EncoderFailure> {
                Err(EncoderFailure::Redacted(
                    "云端编码返回 401：密钥无效".into(),
                ))
            }
            fn transmitted_bytes(&self) -> u64 {
                0
            }
        }
        let (_d, vault) = setup();
        capture(&vault, "任意事实");
        let config = SemanticConfig {
            provider: Provider::Local,
            index_path: vault.root().join(".index/semantic"),
        };
        let error = build_index_with_encoder(&vault, access(), &config, &Explained).unwrap_err();
        assert!(
            error.contains("401"),
            "编码器声明可公开的原因应当保留：{error}"
        );
        assert!(error.contains("密钥无效"), "{error}");
    }
}
