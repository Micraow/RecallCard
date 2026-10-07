//! 可选 Python 语义检索。配置只来自可信启动者；请求不能选择进程、文件或联网权限。
use crate::{
    context::Document,
    model::{hash, validate_scope, Result},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{mpsc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin, ChildStdout, Command},
};

const MAX_CACHE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_DOCUMENTS: usize = 10_000;
const MAX_VECTOR_VALUES: usize = 500_000;
const MAX_RESULTS: usize = 100;

fn default_timeout() -> u64 {
    5000
}
fn default_key_env() -> String {
    "RECALLCARD_EMBEDDING_API_KEY".into()
}
fn default_calls() -> usize {
    20
}
fn default_bytes() -> usize {
    65_536
}

/// 只能由用户/可信宿主在启动时提供；不接受 search/MCP/native 请求中的此配置。
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticConfig {
    pub python: PathBuf,
    pub python_path: PathBuf,
    pub index_path: PathBuf,
    pub expected_space_signature: String,
    #[serde(default)]
    pub query_cache_path: Option<PathBuf>,
    #[serde(default)]
    pub cloud_query: Option<CloudQueryApproval>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

/// 此对象表示已经获得的独立 query 外发批准；从来不批准语料外发或自动建索引。
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudQueryApproval {
    pub endpoint: String,
    pub scopes: Vec<String>,
    #[serde(default = "default_key_env")]
    pub key_env: String,
    #[serde(default = "default_calls")]
    pub max_network_calls: usize,
    #[serde(default = "default_bytes")]
    pub max_transmitted_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SemanticError(pub &'static str);
type SResult<T> = std::result::Result<T, SemanticError>;
fn failure<T>(code: &'static str) -> SResult<T> {
    Err(SemanticError(code))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Space {
    provider: String,
    endpoint: String,
    model: String,
    dimensions: usize,
    revision: String,
    document_instruction: String,
    query_instruction: String,
    normalization: String,
    preprocessing_version: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexDocument {
    #[serde(rename = "ref")]
    reference: String,
    content_hash: String,
    input_hash: String,
    scope: String,
    vector: Vec<f64>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    schema: String,
    space: Space,
    space_signature: String,
    generation: String,
    scope: Vec<String>,
    documents: Vec<IndexDocument>,
    #[serde(default = "complete")]
    complete: bool,
}
fn complete() -> bool {
    true
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryCache {
    schema: String,
    space_signature: String,
    queries: Vec<CachedQuery>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CachedQuery {
    query_hash: String,
    input_hash: String,
    vector: Vec<f64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    #[serde(rename = "ref")]
    reference: String,
    score: f64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchResult {
    results: Vec<Row>,
    ranking: String,
    space_signature: String,
    generation: String,
    #[serde(default)]
    query_cache_hit: Option<bool>,
    #[serde(default)]
    diagnostics: Option<Value>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerError {
    code: String,
    message: String,
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn bounded_string(value: &str, limit: usize, empty: bool) -> bool {
    value.len() <= limit && !value.contains('\0') && (empty || !value.trim().is_empty())
}
fn valid_scopes(scopes: &[String]) -> bool {
    scopes.len() <= 256
        && scopes.iter().all(|scope| validate_scope(scope).is_ok())
        && scopes.iter().collect::<BTreeSet<_>>().len() == scopes.len()
}
fn validate_vector(vector: &[f64], dimensions: usize, normalized: bool) -> SResult<()> {
    let norm = vector.iter().copied().fold(0.0_f64, f64::hypot);
    if vector.len() != dimensions
        || !vector.iter().all(|v| v.is_finite())
        || !norm.is_finite()
        || norm == 0.0
        || (normalized && (norm - 1.0).abs() > 1e-6)
    {
        return failure("invalid_cache");
    }
    Ok(())
}
fn prepared_hash(instruction: &str, text: &str) -> String {
    if instruction.is_empty() {
        hash(text.as_bytes())
    } else {
        hash(format!("{instruction}\n{text}").as_bytes())
    }
}

/// 拒绝设备/FIFO/目录和直接符号链接，限制解析前读取量；本机父目录仍须由宿主管理。
fn read_local<T: serde::de::DeserializeOwned>(path: &Path) -> SResult<T> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| SemanticError("cache_unavailable"))?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_CACHE_BYTES {
        return failure("invalid_cache");
    }
    let file = File::open(path).map_err(|_| SemanticError("cache_unavailable"))?;
    if !file
        .metadata()
        .map_err(|_| SemanticError("cache_unavailable"))?
        .is_file()
    {
        return failure("invalid_cache");
    }
    let mut bytes = Vec::new();
    file.take(MAX_CACHE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| SemanticError("cache_unavailable"))?;
    if bytes.len() as u64 > MAX_CACHE_BYTES {
        return failure("invalid_cache");
    }
    // 直接反序列化严格结构；不能先转 Value 而吞掉重复字段。
    serde_json::from_slice(&bytes).map_err(|_| SemanticError("invalid_cache"))
}

pub(crate) fn corpus_documents(
    documents: &[Document],
    scopes: &[String],
    time: DateTime<Utc>,
) -> Vec<Value> {
    documents.iter().filter(|document| scopes.contains(&document.scope) && document.current_memory_at(time))
        .map(|document| json!({"ref":document.reference,"content_hash":hash(document.text.as_bytes()),"text":document.text,"scope":document.scope}))
        .collect()
}
fn corpus_generation(documents: &[Value]) -> String {
    hash(&serde_json::to_vec(documents).expect("语料只包含可序列化 JSON"))
}

impl Index {
    fn validate(&self, expected: &str) -> SResult<()> {
        let space = &self.space;
        if self.schema != "recallcard.embedding-index/1" || !self.complete {
            return failure(if self.complete {
                "invalid_cache"
            } else {
                "index_incomplete"
            });
        }
        let signature = hash(
            &serde_json::to_vec(
                &serde_json::to_value(space).map_err(|_| SemanticError("invalid_cache"))?,
            )
            .map_err(|_| SemanticError("invalid_cache"))?,
        );
        if self.space_signature != expected || signature != expected {
            return failure("signature_mismatch");
        }
        if !bounded_string(&space.provider, 128, false)
            || !bounded_string(&space.model, 256, false)
            || !bounded_string(&space.endpoint, 2048, false)
            || !bounded_string(&space.revision, 256, false)
            || !bounded_string(&space.document_instruction, 2048, true)
            || !bounded_string(&space.query_instruction, 2048, true)
            || !(1..=8192).contains(&space.dimensions)
            || !["l2", "none"].contains(&space.normalization.as_str())
            || space.preprocessing_version != "utf8-v1"
            || !digest(&self.generation)
            || !valid_scopes(&self.scope)
            || self.documents.len() > MAX_DOCUMENTS
            || self.documents.len().saturating_mul(space.dimensions) > MAX_VECTOR_VALUES
        {
            return failure("invalid_cache");
        }
        let mut refs = BTreeSet::new();
        let mut inputs: BTreeMap<&str, &[f64]> = BTreeMap::new();
        for document in &self.documents {
            if !self.scope.contains(&document.scope)
                || !refs.insert(&document.reference)
                || !digest(&document.content_hash)
                || !digest(&document.input_hash)
            {
                return failure("invalid_cache");
            }
            validate_vector(
                &document.vector,
                space.dimensions,
                space.normalization == "l2",
            )?;
            if inputs
                .insert(&document.input_hash, &document.vector)
                .is_some_and(|old| old != document.vector)
            {
                return failure("invalid_cache");
            }
        }
        Ok(())
    }

    fn check_current(
        &self,
        documents: &[Document],
        scopes: &[String],
        time: DateTime<Utc>,
    ) -> SResult<()> {
        if self.scope.iter().any(|scope| !scopes.contains(scope)) {
            return failure("scope_mismatch");
        }
        let corpus = corpus_documents(documents, &self.scope, time);
        if corpus_generation(&corpus) != self.generation || corpus.len() != self.documents.len() {
            return failure("generation_mismatch");
        }
        let expected_refs: BTreeSet<&str> = corpus
            .iter()
            .filter_map(|document| document["ref"].as_str())
            .collect();
        let current: BTreeMap<&str, &Document> = documents
            .iter()
            .map(|document| (document.reference.as_str(), document))
            .collect();
        for document in &self.documents {
            if !expected_refs.contains(document.reference.as_str()) {
                return failure("generation_mismatch");
            }
            let Some(live) = current.get(document.reference.as_str()) else {
                return failure("generation_mismatch");
            };
            if document.scope != live.scope
                || document.content_hash != hash(live.text.as_bytes())
                || document.input_hash
                    != prepared_hash(&self.space.document_instruction, &live.text)
            {
                return failure("content_mismatch");
            }
        }
        Ok(())
    }
}

pub(crate) struct Candidates {
    index: Index,
    pub references: Vec<String>,
}
impl Candidates {
    pub(crate) fn revalidate(
        &self,
        documents: &[Document],
        scopes: &[String],
        time: DateTime<Utc>,
    ) -> SResult<()> {
        self.index.check_current(documents, scopes, time)
    }
}

/// 全进程只持有一个 worker；损坏、超时或输出越界后停用，避免重启重置联网预算。
pub struct SemanticSearch {
    config: SemanticConfig,
    supervisor: Mutex<Supervisor>,
}
struct Supervisor {
    sender: Option<mpsc::Sender<Job>>,
    next_id: u64,
    disabled: Option<SemanticError>,
}
struct Job {
    request: Vec<u8>,
    reply: mpsc::SyncSender<SResult<Vec<u8>>>,
}

impl SemanticSearch {
    pub fn from_config_file(path: &Path) -> Result<Self> {
        let config = read_local(path)
            .map_err(|_| "无法读取语义检索启动配置；需使用可信本机普通文件".to_owned())?;
        Self::new(config)
    }
    pub fn new(config: SemanticConfig) -> Result<Self> {
        if config.python.as_os_str().is_empty()
            || !config.python_path.is_absolute()
            || !config.index_path.is_absolute()
            || config
                .query_cache_path
                .as_ref()
                .is_some_and(|path| !path.is_absolute())
            || !digest(&config.expected_space_signature)
            || !(10..=120_000).contains(&config.timeout_ms)
        {
            return Err(
                "语义检索启动配置无效：需可信解释器、绝对路径、空间签名及 10–120000ms 超时".into(),
            );
        }
        if let Some(approval) = &config.cloud_query {
            if approval.scopes.is_empty()
                || !valid_scopes(&approval.scopes)
                || !bounded_string(&approval.endpoint, 2048, false)
                || !approval.endpoint.starts_with("https://")
                || approval.key_env.is_empty()
                || approval.key_env.len() > 128
                || !approval.key_env.bytes().enumerate().all(|(index, byte)| {
                    byte.is_ascii_alphabetic()
                        || byte == b'_'
                        || (index > 0 && byte.is_ascii_digit())
                })
                || !(1..=1000).contains(&approval.max_network_calls)
                || !(1..=4 * 1024 * 1024).contains(&approval.max_transmitted_bytes)
            {
                return Err(
                    "云查询批准配置无效；需明确 HTTPS 接收者、scope、密钥环境变量名和有限用量"
                        .into(),
                );
            }
        }
        Ok(Self {
            config,
            supervisor: Mutex::new(Supervisor {
                sender: None,
                next_id: 0,
                disabled: None,
            }),
        })
    }

    pub(crate) fn candidates(
        &self,
        query: &str,
        documents: &[Document],
        selected: &[Document],
        scopes: &[String],
        time: DateTime<Utc>,
    ) -> SResult<Candidates> {
        let index: Index = read_local(&self.config.index_path)?;
        index.validate(&self.config.expected_space_signature)?;
        index.check_current(documents, scopes, time)?;
        let selected_refs: BTreeSet<&str> = selected
            .iter()
            .map(|document| document.reference.as_str())
            .collect();
        let allowed_refs = index
            .documents
            .iter()
            .filter(|document| selected_refs.contains(document.reference.as_str()))
            .map(|document| document.reference.clone())
            .collect::<Vec<_>>();
        if allowed_refs.is_empty() {
            return failure("no_eligible_memories");
        }
        let allowed_scopes: Vec<String> = selected
            .iter()
            .filter(|document| allowed_refs.contains(&document.reference))
            .map(|document| document.scope.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let query_vector = self.cached_query(&index.space, query)?;
        let mut request = json!({"index":index,"allowed_scopes":allowed_scopes,"allowed_refs":allowed_refs,"limit":MAX_RESULTS,"expected_generation":index.generation});
        if let Some(vector) = query_vector {
            request["op"] = json!("query_vector");
            request["query_vector"] = json!(vector);
            request["space_signature"] = json!(self.config.expected_space_signature);
        } else {
            let Some(approval) = &self.config.cloud_query else {
                return failure("query_not_cached");
            };
            if approval.endpoint != index.space.endpoint
                || allowed_scopes
                    .iter()
                    .any(|scope| !approval.scopes.contains(scope))
            {
                return failure("query_not_approved");
            }
            request["op"] = json!("query");
            request["query"] = json!(query);
        }
        let result = self.exchange(request)?;
        let _ = (result.query_cache_hit, result.diagnostics);
        if result.ranking != "cosine"
            || result.space_signature != index.space_signature
            || result.generation != index.generation
            || result.results.len() > MAX_RESULTS
        {
            self.invalidate();
            return failure("malformed_response");
        }
        let mut seen = BTreeSet::new();
        for row in &result.results {
            if !allowed_refs.contains(&row.reference)
                || !row.score.is_finite()
                || !(-1.0..=1.0).contains(&row.score)
                || !seen.insert(&row.reference)
            {
                self.invalidate();
                return failure("malformed_response");
            }
        }
        // 分数只用于排序，不作为来源可信度；Rust 再融合完整文本候选，保留未 Dream 的 Event。
        Ok(Candidates {
            index,
            references: result
                .results
                .into_iter()
                .map(|row| row.reference)
                .collect(),
        })
    }

    fn cached_query(&self, space: &Space, query: &str) -> SResult<Option<Vec<f64>>> {
        let Some(path) = &self.config.query_cache_path else {
            return Ok(None);
        };
        let cache: QueryCache = read_local(path)?;
        if cache.schema != "recallcard.embedding-query-cache/1"
            || cache.space_signature != self.config.expected_space_signature
            || cache.queries.len() > 32
        {
            return failure("query_cache_incompatible");
        }
        let query_hash = hash(query.as_bytes());
        let input_hash = prepared_hash(&space.query_instruction, query);
        let mut seen = BTreeSet::new();
        let mut found = None;
        for item in cache.queries {
            if !digest(&item.query_hash)
                || !digest(&item.input_hash)
                || !seen.insert(item.query_hash.clone())
            {
                return failure("invalid_cache");
            }
            validate_vector(&item.vector, space.dimensions, false)?;
            if item.query_hash == query_hash {
                if item.input_hash != input_hash {
                    return failure("query_cache_incompatible");
                }
                found = Some(item.vector);
            }
        }
        Ok(found)
    }

    fn invalidate(&self) {
        if let Ok(mut supervisor) = self.supervisor.try_lock() {
            supervisor.disabled = Some(SemanticError("malformed_response"));
            supervisor.sender = None;
        }
    }

    fn exchange(&self, mut request: Value) -> SResult<SearchResult> {
        let mut supervisor = self
            .supervisor
            .try_lock()
            .map_err(|_| SemanticError("worker_busy"))?;
        if let Some(error) = supervisor.disabled {
            return Err(error);
        }
        if supervisor.sender.is_none() {
            let (sender, receiver) = mpsc::channel();
            let config = self.config.clone();
            std::thread::Builder::new()
                .name("recallcard-semantic".into())
                .spawn(move || supervise(config, receiver))
                .map_err(|_| SemanticError("worker_unavailable"))?;
            supervisor.sender = Some(sender);
        }
        supervisor.next_id += 1;
        let id = supervisor.next_id;
        request["id"] = json!(id);
        let mut encoded = serde_json::to_vec(&request).map_err(|_| SemanticError("input_limit"))?;
        encoded.push(b'\n');
        if encoded.len() > MAX_REQUEST_BYTES {
            return failure("input_limit");
        }
        let (reply, receiver) = mpsc::sync_channel(1);
        let received = if supervisor
            .sender
            .as_ref()
            .unwrap()
            .send(Job {
                request: encoded,
                reply,
            })
            .is_err()
        {
            failure("worker_unavailable")
        } else {
            receiver
                .recv_timeout(Duration::from_millis(self.config.timeout_ms + 500))
                .unwrap_or_else(|_| failure("worker_timeout"))
        };
        let bytes = match received {
            Ok(bytes) => bytes,
            Err(error) => {
                supervisor.disabled = Some(error);
                supervisor.sender = None;
                return Err(error);
            }
        };
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Envelope {
            id: u64,
            ok: bool,
            #[serde(default)]
            result: Option<SearchResult>,
            #[serde(default)]
            error: Option<WorkerError>,
            #[serde(default)]
            diagnostics: Option<Value>,
        }
        let envelope: Envelope = match serde_json::from_slice(&bytes) {
            Ok(envelope) => envelope,
            Err(_) => {
                supervisor.disabled = Some(SemanticError("malformed_response"));
                supervisor.sender = None;
                return failure("malformed_response");
            }
        };
        let _ = envelope.diagnostics;
        if envelope.id != id
            || (envelope.ok && (envelope.result.is_none() || envelope.error.is_some()))
            || (!envelope.ok && (envelope.error.is_none() || envelope.result.is_some()))
        {
            supervisor.disabled = Some(SemanticError("malformed_response"));
            supervisor.sender = None;
            return failure("malformed_response");
        }
        if !envelope.ok {
            // 错误正文和任意 worker 字段都不向客户端转发，只使用固定白名单错误码。
            let error = envelope.error.unwrap();
            let _ = error.message;
            let code = match error.code.as_str() {
                "network_disabled" | "approval_required" | "scope_denied" => "query_not_approved",
                "network_budget_exhausted" => "network_budget_exhausted",
                "missing_key" => "key_unavailable",
                "generation_mismatch" => "generation_mismatch",
                "signature_mismatch" => "signature_mismatch",
                "index_incomplete" => "index_incomplete",
                "network_error" => "network_error",
                "provider_error" => "provider_error",
                "provider_auth" => "provider_auth",
                "rate_limited" => "rate_limited",
                "redirect_blocked" => "redirect_blocked",
                "model_mismatch" => "model_mismatch",
                _ => "worker_error",
            };
            return failure(code);
        }
        Ok(envelope.result.unwrap())
    }
}

struct Process {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}
impl Process {
    fn start(config: &SemanticConfig) -> SResult<Self> {
        let mut command = Command::new(&config.python);
        // -I/-S 拒绝当前目录、用户 site 和 PYTHON* 环境注入；只载入启动者指定的模块根目录。
        command.args(["-I", "-S", "-c", "import sys; sys.path.insert(0, sys.argv[1]); from recallcard_worker.__main__ import main; raise SystemExit(main(sys.argv[2:]))"])
            .arg(&config.python_path).arg("--stdio").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).env_clear();
        for name in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        if let Some(approval) = &config.cloud_query {
            command
                .args([
                    "--allow-network",
                    "--approve-data",
                    "query",
                    "--approve-endpoint",
                    &approval.endpoint,
                    "--key-env",
                    &approval.key_env,
                ])
                .arg("--max-network-calls")
                .arg(approval.max_network_calls.to_string())
                .arg("--max-transmitted-bytes")
                .arg(approval.max_transmitted_bytes.to_string())
                .args(["--max-retries", "0"])
                .arg("--timeout")
                .arg((config.timeout_ms as f64 / 1000.0).to_string());
            for scope in &approval.scopes {
                command.arg("--approve-scope").arg(scope);
            }
            if let Some(value) = std::env::var_os(&approval.key_env) {
                command.env(&approval.key_env, value);
            }
        }
        let mut child = command
            .spawn()
            .map_err(|_| SemanticError("worker_unavailable"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or(SemanticError("worker_unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or(SemanticError("worker_unavailable"))?;
        Ok(Self {
            child,
            stdin,
            stdout,
        })
    }
    async fn exchange(&mut self, request: &[u8]) -> SResult<Vec<u8>> {
        self.stdin
            .write_all(request)
            .await
            .map_err(|_| SemanticError("worker_io"))?;
        self.stdin
            .flush()
            .await
            .map_err(|_| SemanticError("worker_io"))?;
        let mut response = Vec::new();
        loop {
            let mut buffer = [0u8; 4096];
            let count = self
                .stdout
                .read(&mut buffer)
                .await
                .map_err(|_| SemanticError("worker_io"))?;
            if count == 0 {
                return failure("worker_exited");
            }
            response.extend_from_slice(&buffer[..count]);
            if response.len() > MAX_RESPONSE_BYTES {
                return failure("output_limit");
            }
            if let Some(newline) = response.iter().position(|byte| *byte == b'\n') {
                if newline + 1 != response.len() {
                    return failure("malformed_response");
                }
                return Ok(response);
            }
        }
    }
}

fn supervise(config: SemanticConfig, receiver: mpsc::Receiver<Job>) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    else {
        return;
    };
    let mut process: Option<Process> = None;
    while let Ok(job) = receiver.recv() {
        let result = runtime.block_on(async {
            tokio::time::timeout(Duration::from_millis(config.timeout_ms), async {
                if process.is_none() {
                    process = Some(Process::start(&config)?);
                }
                process.as_mut().unwrap().exchange(&job.request).await
            })
            .await
            .unwrap_or_else(|_| failure("worker_timeout"))
        });
        let broken = result.is_err();
        let _ = job.reply.send(result);
        if broken {
            break;
        }
    }
    if let Some(mut process) = process {
        // 包括超时、客户端退出和协议失败；取消异步管道无需等待阻塞读线程。
        runtime.block_on(async {
            let _ = tokio::time::timeout(Duration::from_millis(250), process.child.kill()).await;
        });
    }
}
