//! 来源云发送的唯一宿主边界。长期索引无凭据；缓存命中不进入本模块编码。
use super::*;
use crate::application::{
    background_memory::MemoryProviderConfig, credentials, embedding_cloud::SourceEmbeddingData,
};
use std::{process::Stdio, time::Instant};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};

const CALLBACK: &str = "recallcard.host-encode/1";
const ONCE: &str = "recallcard.source-cloud-once/1";
const KEY_ENV: &str = "RECALLCARD_SOURCE_EMBEDDING_API_KEY";
const MAX_LINE: usize = 65536;
const MAX_VECTOR_LINE: usize = 256 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CallbackFrame {
    schema: String,
    id: u64,
    kind: String,
    space_signature: String,
    inputs: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderError {
    code: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderReply {
    schema: String,
    id: u64,
    ok: bool,
    #[serde(default)]
    vectors: Option<Value>,
    #[serde(default)]
    error: Option<ProviderError>,
}
pub(super) fn parse_callback(bytes: &[u8]) -> Result<Value> {
    // 派生结构反序列化拒绝重复字段，不能让JSON最后字段覆盖授权要素。
    let frame: CallbackFrame =
        serde_json::from_slice(bytes).map_err(|_| "source_cloud_callback_invalid")?;
    serde_json::to_value(frame).map_err(|_| "source_cloud_callback_invalid".into())
}

/// 由原始宿主作业确定；工作进程不能选择数据种类、目标、范围或正文。
pub(super) struct Binding {
    epoch: Option<String>,
}
impl Binding {
    pub fn new(config: &Config, request: &Value) -> Result<Option<Self>> {
        if config.cloud.is_none() {
            return Ok(None);
        }
        worker_configuration_current(config)?;
        let op = request["op"]
            .as_str()
            .ok_or("source_cloud_request_invalid")?;
        let epoch = match op {
            "status" => None,
            "begin" | "finish" => Some(
                request["source_epoch"]
                    .as_str()
                    .ok_or("source_cloud_request_invalid")?
                    .to_owned(),
            ),
            "query" | "upsert" => {
                let manifest: Manifest = read_bounded(
                    &config
                        .worker_config
                        .parent()
                        .ok_or("local_event_config_invalid")?
                        .join("manifest.json"),
                    MAX_JSON,
                )?;
                Some(manifest.epoch)
            }
            _ => return Err("source_cloud_request_invalid".into()),
        };
        let binding = Self { epoch };
        binding.verify(config, request)?;
        Ok(Some(binding))
    }
    /// 每次调用只在正本验证期间持读锁，返回后立即释放；网络不持 Vault 锁。
    pub fn verify(
        &self,
        config: &Config,
        request: &Value,
    ) -> Result<Option<(SourceEmbeddingData, Vec<String>)>> {
        let vault =
            Vault::open_existing(&config.root).map_err(|_| "local_event_config_unavailable")?;
        let _guard = vault.read_guard()?;
        let index =
            LocalEvents::open(&vault, &config.scope)?.ok_or("source_cloud_approval_changed")?;
        if index.config_hash != config.identity()? {
            return Err("source_cloud_approval_changed".into());
        }
        index.current()?;
        if self.epoch.as_ref().is_some_and(|epoch| {
            snapshot::source_inventory(&index)
                .map(|inventory| inventory.0 != *epoch)
                .unwrap_or(true)
        }) {
            return Err("local_event_source_epoch_changed".into());
        }
        let grant = config.cloud.as_ref().ok_or("source_cloud_not_approved")?;
        match request["op"].as_str() {
            Some("query") => {
                if !grant.settings.query_approved {
                    return Err("source_cloud_query_not_approved".into());
                }
                let query = request["query"]
                    .as_str()
                    .filter(|query| !query.trim().is_empty() && query.len() <= 4096)
                    .ok_or("source_cloud_request_invalid")?;
                Ok(Some((
                    SourceEmbeddingData::Query,
                    vec![prepare(query, &config.settings.query_instruction)?],
                )))
            }
            Some("upsert") => {
                let epoch = self.epoch.as_ref().ok_or("source_cloud_request_invalid")?;
                let manifest: Manifest =
                    read_bounded(&index.dir()?.join("manifest.json"), MAX_JSON)?;
                if manifest.epoch != *epoch {
                    return Err("local_event_source_epoch_changed".into());
                }
                let reference = request["source"]["ref"]
                    .as_str()
                    .ok_or("source_cloud_request_invalid")?;
                let source = manifest
                    .sources
                    .iter()
                    .find(|source| source.reference() == reference)
                    .ok_or("local_event_source_not_current")?;
                if !snapshot::verified_sources(&index, epoch)?.contains(reference) {
                    return Err("local_event_source_not_current".into());
                }
                let kind = if source.object.is_some() {
                    SourceEmbeddingData::Attachment
                } else {
                    SourceEmbeddingData::Event
                };
                if !match kind {
                    SourceEmbeddingData::Attachment => grant.settings.attachments_approved,
                    SourceEmbeddingData::Event => grant.settings.events_approved,
                    SourceEmbeddingData::Query => false,
                } {
                    return Err("source_cloud_corpus_not_approved".into());
                }
                let event = snapshot::read_source(&vault, &manifest, source)?;
                let text = source.text(&vault, &event)?;
                let ranges = snapshot::ranges(&text, config.settings.chunk_bytes);
                if request["source"]["scope"] != config.scope
                    || request["source"]["content_hash"] != hash(text.as_bytes())
                    || request["source"]["text_bytes"] != json!(text.len())
                    || request["source_chunk_count"] != json!(ranges.len())
                    || source.chunks != ranges.len()
                    || request["source"]["role"]
                        != if source.object.is_some() {
                            Value::Null
                        } else {
                            json!(event.data.role)
                        }
                {
                    return Err("local_event_source_changed".into());
                }
                let chunks = request["chunks"]
                    .as_array()
                    .filter(|chunks| {
                        !chunks.is_empty()
                            && chunks.len() <= 4
                            && chunks.len() * config.settings.dimensions <= 8192
                    })
                    .ok_or("source_cloud_batch_limit")?;
                let mut inputs = Vec::new();
                let mut ordinals = BTreeSet::new();
                for chunk in chunks {
                    let ordinal = chunk["ordinal"]
                        .as_u64()
                        .and_then(|n| usize::try_from(n).ok())
                        .ok_or("source_cloud_request_invalid")?;
                    let &(start, end) =
                        ranges.get(ordinal).ok_or("source_cloud_request_invalid")?;
                    let raw = &text[start..end];
                    if !ordinals.insert(ordinal)
                        || chunk["start_byte"] != json!(start)
                        || chunk["end_byte"] != json!(end)
                        || chunk["text"] != raw
                        || chunk["text_sha256"] != hash(raw.as_bytes())
                    {
                        return Err("local_event_source_changed".into());
                    }
                    inputs.push(prepare(raw, &config.settings.document_instruction)?);
                }
                Ok(Some((kind, inputs)))
            }
            Some("status" | "begin" | "finish") => Ok(None),
            _ => Err("source_cloud_request_invalid".into()),
        }
    }
}
fn prepare(raw: &str, instruction: &str) -> Result<String> {
    let text = if instruction.is_empty() {
        raw.to_owned()
    } else {
        format!("{instruction}\n{raw}")
    };
    if text.len() > 8192 || raw.trim().is_empty() || text.contains('\0') {
        return Err("source_cloud_input_limit".into());
    }
    Ok(text)
}
fn kind_name(kind: SourceEmbeddingData) -> &'static str {
    match kind {
        SourceEmbeddingData::Query => "query",
        SourceEmbeddingData::Event => "event",
        SourceEmbeddingData::Attachment => "attachment",
    }
}
fn deadline_current(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        Err("local_event_worker_timeout".into())
    } else {
        Ok(())
    }
}
/// Tokio 的超时不能中断同步正本/凭据检查，所以两侧都要核对墙钟截止时间。
pub(super) fn checked_before<T>(deadline: Instant, check: impl FnOnce() -> Result<T>) -> Result<T> {
    checked_with_gate(|| deadline_current(deadline), check)
}
fn checked_with_gate<T>(
    mut gate: impl FnMut() -> Result<()>,
    check: impl FnOnce() -> Result<T>,
) -> Result<T> {
    gate()?;
    let result = check()?;
    gate()?;
    Ok(result)
}
fn callback_inputs(
    config: &Config,
    request: &Value,
    binding: &Binding,
    callback: &Value,
) -> Result<(SourceEmbeddingData, Vec<String>)> {
    let (kind, mut allowed) = binding
        .verify(config, request)?
        .ok_or("source_cloud_callback_denied")?;
    let object = callback
        .as_object()
        .ok_or("source_cloud_callback_invalid")?;
    if object.len() != 5
        || callback["schema"] != CALLBACK
        || callback["id"] != 1
        || callback["kind"] != kind_name(kind)
        || callback["space_signature"]
            != hash(
                &serde_json::to_vec(&config.space())
                    .map_err(|_| "source_cloud_callback_invalid")?,
            )
    {
        return Err("source_cloud_callback_invalid".into());
    }
    let values = callback["inputs"]
        .as_array()
        .filter(|values| {
            !values.is_empty()
                && values.len() <= 4
                && values.len() * config.settings.dimensions <= 8192
        })
        .ok_or("source_cloud_callback_invalid")?;
    let mut inputs = Vec::new();
    let mut unique = BTreeSet::new();
    for value in values {
        let input = value
            .as_str()
            .filter(|value| value.len() <= 8192)
            .ok_or("source_cloud_callback_invalid")?;
        if !unique.insert(input) {
            return Err("source_cloud_callback_invalid".into());
        }
        let position = allowed
            .iter()
            .position(|text| text == input)
            .ok_or("source_cloud_callback_denied")?;
        allowed.remove(position);
        inputs.push(input.to_owned());
    }
    if kind == SourceEmbeddingData::Query && inputs.len() != 1 {
        return Err("source_cloud_callback_invalid".into());
    }
    if serde_json::to_vec(callback)
        .map_err(|_| "source_cloud_callback_invalid")?
        .len()
        + 1
        > MAX_LINE
    {
        return Err("source_cloud_input_limit".into());
    }
    Ok((kind, inputs))
}
fn http_body(config: &Config, inputs: &[String]) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(&json!({"model":config.settings.model,"input":inputs,"dimensions":config.settings.dimensions,"encoding_format":"float"})).map_err(|_| "source_cloud_input_limit")?;
    if bytes.len() > MAX_LINE {
        return Err("source_cloud_input_limit".into());
    }
    Ok(bytes)
}
pub(super) async fn encode(
    config: &Config,
    request: &Value,
    binding: &Binding,
    callback: &Value,
    deadline: Instant,
) -> Result<Value> {
    deadline_current(deadline)?;
    let (kind, inputs) = checked_before(deadline, || {
        callback_inputs(config, request, binding, callback)
    })?;
    let body = http_body(config, &inputs)?;
    let grant = config.cloud.as_ref().ok_or("source_cloud_not_approved")?;
    let target = MemoryProviderConfig {
        endpoint: grant.settings.endpoint.clone(),
        model: grant.settings.model.clone(),
    };
    let vault = Vault::open_existing(&config.root).map_err(|_| "local_event_config_unavailable")?;
    // 此处只会由真实 cache miss 回调抵达；批准检查先于任何系统凭据读取。
    deadline_current(deadline)?;
    let credential = credentials::load_embedding_os(&vault, &target)
        .map_err(|_| "source_cloud_key_unavailable")?
        .ok_or("source_cloud_key_unavailable")?;
    deadline_current(deadline)?;
    checked_before(deadline, || binding.verify(config, request))?;
    CloudEmbeddingRuntime::new(&vault)
        .reserve_source(
            grant,
            kind,
            &config.scope,
            &target.endpoint,
            &target.model,
            body.len() as u64,
        )
        .map_err(|error| match error.code {
            crate::application::ErrorCode::BudgetExceeded => "source_cloud_budget_exhausted",
            crate::application::ErrorCode::PermissionDenied => "source_cloud_not_approved",
            crate::application::ErrorCode::Conflict => "source_cloud_budget_busy",
            _ => "source_cloud_approval_unavailable",
        })?;
    deadline_current(deadline)?;
    checked_before(deadline, || binding.verify(config, request))?;
    let packet = json!({"schema":ONCE,"id":1,"scope":config.scope,"kind":kind_name(kind),"space":config.space(),"space_signature":callback["space_signature"],"inputs":inputs});
    let mut bytes = serde_json::to_vec(&packet).map_err(|_| "source_cloud_input_limit")?;
    bytes.push(b'\n');
    if bytes.len() > MAX_LINE {
        return Err("source_cloud_input_limit".into());
    }
    let mut command = Command::new(&config.python);
    command.args(["-I", "-S", "-c", "import sys; sys.path.insert(0, sys.argv[1]); from recallcard_worker.source_cloud_once import main; raise SystemExit(main(sys.argv[2:]))"])
        .arg(&config.python_path).arg("--endpoint").arg(&target.endpoint).arg("--scope").arg(&config.scope)
        .arg("--kind").arg(kind_name(kind)).arg("--key-env").arg(KEY_ENV).env_clear()
        .env(KEY_ENV, credential.for_target(&target).ok_or("source_cloud_key_unavailable")?)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    for key in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    deadline_current(deadline)?;
    let mut child = command
        .spawn()
        .map_err(|_| "source_cloud_worker_unavailable")?;
    drop(command);
    drop(credential);
    let mut stdin = child
        .stdin
        .take()
        .ok_or("source_cloud_worker_unavailable")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("source_cloud_worker_unavailable")?;
    // 启动与凭据存储都可能等待；发送包含正文的 stdin 前最后核验。
    deadline_current(deadline)?;
    checked_before(deadline, || binding.verify(config, request))?;
    stdin
        .write_all(&bytes)
        .await
        .map_err(|_| "source_cloud_worker_io")?;
    stdin
        .shutdown()
        .await
        .map_err(|_| "source_cloud_worker_io")?;
    drop(stdin);
    let mut response = Vec::new();
    stdout
        .take(MAX_VECTOR_LINE as u64 + 1)
        .read_to_end(&mut response)
        .await
        .map_err(|_| "source_cloud_worker_io")?;
    if response.len() > MAX_VECTOR_LINE
        || response.last() != Some(&b'\n')
        || response[..response.len().saturating_sub(1)].contains(&b'\n')
    {
        return Err("source_cloud_response_invalid".into());
    }
    let status = child.wait().await.map_err(|_| "source_cloud_worker_io")?;
    let envelope: ProviderReply =
        serde_json::from_slice(&response).map_err(|_| "source_cloud_response_invalid")?;
    let shape: Value =
        serde_json::from_slice(&response).map_err(|_| "source_cloud_response_invalid")?;
    if shape.as_object().is_none_or(|object| object.len() != 4) {
        return Err("source_cloud_response_invalid".into());
    }
    if envelope.schema != ONCE || envelope.id != 1 {
        return Err("source_cloud_response_invalid".into());
    }
    if !envelope.ok {
        if envelope.vectors.is_some() {
            return Err("source_cloud_response_invalid".into());
        }
        let code = envelope
            .error
            .as_ref()
            .map(|error| error.code.as_str())
            .unwrap_or("");
        return Err(match code {
            "provider_auth" => "source_cloud_provider_auth",
            "rate_limited" => "source_cloud_rate_limited",
            "redirect_blocked" => "source_cloud_redirect_blocked",
            "network_error" => "source_cloud_network_error",
            "provider_error" => "source_cloud_provider_error",
            "model_mismatch" => "source_cloud_model_mismatch",
            "missing_key" => "source_cloud_key_unavailable",
            "network_budget_exhausted" => "source_cloud_budget_exhausted",
            _ => "source_cloud_response_invalid",
        }
        .into());
    }
    if envelope.error.is_some() || !status.success() {
        return Err("source_cloud_response_invalid".into());
    }
    let vectors = validate_vectors(
        envelope
            .vectors
            .as_ref()
            .ok_or("source_cloud_response_invalid")?,
        inputs.len(),
        config.settings.dimensions,
    )?;
    deadline_current(deadline)?;
    checked_before(deadline, || binding.verify(config, request))?;
    Ok(json!({"schema":CALLBACK,"id":1,"ok":true,"vectors":vectors}))
}
fn validate_vectors(value: &Value, count: usize, dimensions: usize) -> Result<Value> {
    let rows = value
        .as_array()
        .filter(|rows| rows.len() == count)
        .ok_or("source_cloud_response_invalid")?;
    if count == 0 || count > 4 || count * dimensions > 8192 {
        return Err("source_cloud_response_invalid".into());
    }
    for row in rows {
        let values = row
            .as_array()
            .filter(|row| row.len() == dimensions)
            .ok_or("source_cloud_response_invalid")?;
        let mut norm = 0.0f64;
        for value in values {
            let number = value
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or("source_cloud_response_invalid")?;
            norm = norm.hypot(number);
        }
        if !norm.is_finite() || norm == 0.0 {
            return Err("source_cloud_response_invalid".into());
        }
    }
    Ok(value.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expiration_during_synchronous_verification_blocks_the_next_side_effect() {
        let expired = std::cell::Cell::new(false);
        let gates = std::cell::Cell::new(0);
        let mut checked = false;
        let mut sent = false;
        // 使用生产 checked_before 的同一前后门禁核心；时间推进由校验器确定，
        // 不依赖真实 sleep、计时精度或运行器是否被 CPU 抢占。
        let result = checked_with_gate(
            || {
                gates.set(gates.get() + 1);
                if expired.get() {
                    Err("local_event_worker_timeout".into())
                } else {
                    Ok(())
                }
            },
            || {
                checked = true;
                expired.set(true);
                Ok(())
            },
        );
        if result.is_ok() {
            sent = true;
        }
        assert!(checked);
        assert_eq!(gates.get(), 2);
        assert!(!sent);
        assert_eq!(result.unwrap_err(), "local_event_worker_timeout");
        let mut checked_after_expiry = false;
        assert!(checked_with_gate(
            || Err("local_event_worker_timeout".into()),
            || {
                checked_after_expiry = true;
                Ok(())
            }
        )
        .is_err());
        assert!(!checked_after_expiry);
    }
    #[test]
    fn callback_duplicate_fields_unknown_fields_and_wrong_id_types_are_rejected() {
        let signature = "0".repeat(64);
        let valid = format!(
            r#"{{"schema":"recallcard.host-encode/1","id":1,"kind":"event","space_signature":"{signature}","inputs":["正文"]}}"#
        );
        assert!(parse_callback(valid.as_bytes()).is_ok());
        for invalid in [
            valid.replace("\"id\":1", "\"id\":0,\"id\":1"),
            valid.replace("\"id\":1", "\"id\":true"),
            valid.replace(
                "\"id\":1",
                "\"id\":1,\"endpoint\":\"https://other.invalid\"",
            ),
            valid.replace(
                "\"kind\":\"event\"",
                "\"kind\":\"query\",\"kind\":\"event\"",
            ),
        ] {
            assert!(parse_callback(invalid.as_bytes()).is_err());
        }
        assert!(serde_json::from_str::<ProviderReply>(r#"{"schema":"recallcard.source-cloud-once/1","id":1,"ok":false,"ok":true,"vectors":[[1,0]]}"#).is_err());
    }
    #[test]
    fn prepared_inputs_preserve_whitespace_and_vectors_have_independent_bounded_shape() {
        assert_eq!(prepare(" 原文\n", " 前缀 ").unwrap(), " 前缀 \n 原文\n");
        assert!(prepare(&"x".repeat(8192), "x").is_err());
        assert!(prepare("\0bad", "").is_err());
        assert!(validate_vectors(&json!([[1, 0], [0, 1]]), 2, 2).is_ok());
        assert!(validate_vectors(&json!([[0, 0]]), 1, 2).is_err());
        assert!(validate_vectors(&json!([[true, 0]]), 1, 2).is_err());
        assert!(validate_vectors(&json!([[1, 0]]), 2, 2).is_err());
        let full = json!([vec![0.1234567890123456f64; 8192]]);
        assert!(serde_json::to_vec(&full).unwrap().len() > MAX_LINE);
        assert!(serde_json::to_vec(&full).unwrap().len() < MAX_VECTOR_LINE);
        assert!(validate_vectors(&full, 1, 8192).is_ok());
        assert!(validate_vectors(&full, 1, 8193).is_err());
    }
}
