//! 独立来源索引。默认仅本机；显式云批准由宿主逐次核验，绝无自动云回退。
use crate::{
    filesystem::open_local_file,
    model::{hash, Event, Origin, Result, Role},
    Vault,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
};
mod attachment_sources;
mod evidence;
mod lexical_context;
pub(crate) mod ranking;
mod snapshot;
mod source_cloud;
mod worker;
use crate::application::embedding_cloud::{
    CloudEmbeddingRuntime, SourceCloudGrant, SourceCloudSettings,
};
use snapshot::Manifest;
const MAX_JSON: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Setup {
    pub endpoint: String,
    pub model: String,
    pub revision: String,
    pub tokenizer: String,
    pub dimensions: usize,
    #[serde(default)]
    pub query_instruction: String,
    #[serde(default)]
    pub document_instruction: String,
    pub events_approved: bool,
    /// 附件原件编码必须单独批准，旧配置绝不继承普通对话批准。
    #[serde(default)]
    pub attachments_approved: bool,
    pub query_approved: bool,
    pub chunk_bytes: usize,
    pub disk_cap_mib: usize,
}
impl Setup {
    pub fn validate(&self) -> Result<()> {
        let target = crate::semantic::LocalSemanticSetup {
            endpoint: self.endpoint.clone(),
            model: self.model.clone(),
            revision: self.revision.clone(),
            dimensions: self.dimensions,
            query_instruction: self.query_instruction.clone(),
            document_instruction: self.document_instruction.clone(),
            query_approved: self.query_approved,
            memory_approved: self.events_approved || self.attachments_approved,
            resume_background: false,
        };
        target
            .validate()
            .map_err(|_| "local_event_settings_invalid")?;
        self.validate_index()
    }
    fn validate_index(&self) -> Result<()> {
        if self.tokenizer.is_empty()
            || self.tokenizer.len() > 256
            || self.tokenizer.chars().any(char::is_control)
            || !(256..=8192).contains(&self.chunk_bytes)
            || !(16..=512).contains(&self.disk_cap_mib)
        {
            return Err("local_event_settings_invalid".into());
        }
        Ok(())
    }
    fn space(&self) -> Value {
        json!({"provider":"local-openai-compatible","endpoint":self.endpoint,"model":self.model,
            "revision":self.revision,"tokenizer":self.tokenizer,"dimensions":self.dimensions,
            "document_instruction":self.document_instruction,"query_instruction":self.query_instruction,
            "normalization":"l2","preprocessing_version":"utf8-v1",
            "chunking_version":format!("utf8-nonoverlap/1:{}",self.chunk_bytes)})
    }
}
/// 云来源的索引参数；与旧 Memory 云批准及 numeric-loopback 设置互不转换。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudSetup {
    pub settings: SourceCloudSettings,
    pub tokenizer: String,
    pub chunk_bytes: usize,
    pub disk_cap_mib: usize,
}
impl CloudSetup {
    fn index_settings(&self) -> Setup {
        Setup {
            endpoint: self.settings.endpoint.clone(),
            model: self.settings.model.clone(),
            revision: self.settings.revision.clone(),
            tokenizer: self.tokenizer.clone(),
            dimensions: self.settings.dimensions,
            query_instruction: self.settings.query_instruction.clone(),
            document_instruction: self.settings.document_instruction.clone(),
            events_approved: self.settings.events_approved,
            attachments_approved: self.settings.attachments_approved,
            query_approved: self.settings.query_approved,
            chunk_bytes: self.chunk_bytes,
            disk_cap_mib: self.disk_cap_mib,
        }
    }
    pub fn validate(&self) -> Result<()> {
        self.settings
            .validate()
            .map_err(|_| "source_cloud_settings_invalid")?;
        self.index_settings().validate_index()?;
        if self.chunk_bytes
            + self.settings.document_instruction.len()
            + usize::from(!self.settings.document_instruction.is_empty())
            > 8192
        {
            return Err("source_cloud_prepared_input_limit".into());
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    schema: String,
    scope: String,
    root: PathBuf,
    settings: Setup,
    python: PathBuf,
    python_path: PathBuf,
    worker_config: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cloud: Option<SourceCloudGrant>,
}
impl Config {
    fn space(&self) -> Value {
        let mut value = self.settings.space();
        if self.cloud.is_some() {
            value["provider"] = json!("host-cloud-openai-compatible");
        }
        value
    }
    fn identity(&self) -> Result<String> {
        Ok(hash(
            &serde_json::to_vec(self).map_err(|_| "local_event_config_invalid")?,
        ))
    }
    fn worker_settings(&self) -> Result<Value> {
        let mut value = json!({"schema":"recallcard.local-event-config/1", "database":self.worker_config.parent().ok_or("local_event_config_invalid")?.join("index.sqlite"),"scope":self.scope,
            "space":self.space(),"events_approved":self.settings.events_approved,"attachments_approved":self.settings.attachments_approved,"query_approved":self.settings.query_approved,
            "disk_cap_bytes":self.settings.disk_cap_mib*1024*1024,"backend":"python"});
        if self.cloud.is_some() {
            value["host_approval_identity"] = json!(self.identity()?);
        }
        Ok(value)
    }
    fn validate(&self, vault: &Vault) -> Result<()> {
        match &self.cloud {
            None => self.settings.validate(),
            Some(grant) => {
                let setup = CloudSetup {
                    settings: grant.settings.clone(),
                    tokenizer: self.settings.tokenizer.clone(),
                    chunk_bytes: self.settings.chunk_bytes,
                    disk_cap_mib: self.settings.disk_cap_mib,
                };
                setup.validate()?;
                if grant.scope != self.scope
                    || serde_json::to_value(setup.index_settings()).ok()
                        != serde_json::to_value(&self.settings).ok()
                    || !CloudEmbeddingRuntime::new(vault)
                        .source_configuration_current(grant, &self.identity()?)
                        .map_err(|_| "source_cloud_approval_unavailable")?
                {
                    return Err("source_cloud_approval_changed".into());
                }
                Ok(())
            }
        }
    }
}
pub struct LocalEvents<'a> {
    vault: &'a Vault,
    config: Config,
    config_hash: String,
    path: PathBuf,
}
fn config_path(vault: &Vault, scope: &str) -> Result<PathBuf> {
    crate::model::validate_scope(scope)?;
    Ok(vault
        .state_dir()?
        .join("local-event-semantic")
        .join(hash(scope.as_bytes()))
        .join("current.json"))
}
fn read_bounded<T: serde::de::DeserializeOwned>(path: &Path, limit: u64) -> Result<T> {
    crate::vault::reject_symlink(path)?;
    let file = open_local_file(path)?;
    if !file
        .metadata()
        .is_ok_and(|m| m.is_file() && m.len() <= limit)
    {
        return Err("local_event_file_limit".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "local_event_file_unavailable")?;
    if bytes.len() as u64 > limit {
        return Err("local_event_file_limit".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "local_event_file_invalid".into())
}
impl<'a> LocalEvents<'a> {
    pub fn configure(
        vault: &'a Vault,
        scope: &str,
        settings: Setup,
        python: PathBuf,
        python_path: PathBuf,
    ) -> Result<Self> {
        settings.validate()?;
        Self::configure_mode(vault, scope, settings, python, python_path, None)
    }
    pub fn configure_cloud(
        vault: &'a Vault,
        scope: &str,
        request: CloudSetup,
        python: PathBuf,
        python_path: PathBuf,
    ) -> Result<Self> {
        request.validate()?;
        let settings = request.index_settings();
        Self::configure_mode(
            vault,
            scope,
            settings,
            python,
            python_path,
            Some(request.settings),
        )
    }
    fn configure_mode(
        vault: &'a Vault,
        scope: &str,
        settings: Setup,
        python: PathBuf,
        python_path: PathBuf,
        cloud: Option<SourceCloudSettings>,
    ) -> Result<Self> {
        if !python_path.is_absolute()
            || !python_path
                .join("recallcard_worker/event_index.py")
                .is_file()
            || python.as_os_str().is_empty()
        {
            return Err("local_event_worker_unavailable".into());
        }
        let path = config_path(vault, scope)?;
        let parent = path.parent().ok_or("local_event_config_invalid")?;
        crate::vault::reject_symlink(parent)?;
        std::fs::create_dir_all(parent).map_err(|_| "local_event_config_unavailable")?;
        let mut space = settings.space();
        if cloud.is_some() {
            space["provider"] = json!("host-cloud-openai-compatible");
        }
        let signature =
            hash(&serde_json::to_vec(&space).map_err(|_| "local_event_config_invalid")?);
        let dir = parent.join(signature);
        crate::vault::reject_symlink(&dir)?;
        std::fs::create_dir_all(&dir).map_err(|_| "local_event_config_unavailable")?;
        let worker_config = dir.join("worker.json");
        let _guard = vault.lock()?;
        if path.exists() {
            let previous: Config = read_bounded(&path, 65536)?;
            if previous.worker_config != worker_config {
                Self::invalidate_worker(vault, &previous)?;
            }
            if previous.cloud.is_some() && cloud.is_none() {
                CloudEmbeddingRuntime::new(vault)
                    .revoke_sources(scope)
                    .map_err(|_| "source_cloud_approval_unavailable")?;
            }
        }
        let cloud = cloud
            .map(|settings| {
                CloudEmbeddingRuntime::new(vault)
                    .configure_sources(scope, settings)
                    .map_err(|_| "source_cloud_approval_unavailable".to_owned())
            })
            .transpose()?;
        let config = Config {
            schema: "recallcard.local-event-host/1".into(),
            scope: scope.into(),
            root: vault.root().to_owned(),
            settings,
            python,
            python_path,
            worker_config,
            cloud,
        };
        let worker = config.worker_settings()?;
        vault.write_replace(&config.worker_config, &worker)?;
        vault.write_replace(&path, &config)?;
        if let Some(grant) = &config.cloud {
            CloudEmbeddingRuntime::new(vault)
                .bind_source_configuration(grant, &config.identity()?)
                .map_err(|_| "source_cloud_approval_unavailable")?;
        }
        drop(_guard);
        Self::open(vault, scope)?.ok_or_else(|| "local_event_config_unavailable".into())
    }
    pub fn open(vault: &'a Vault, scope: &str) -> Result<Option<Self>> {
        let path = config_path(vault, scope)?;
        if !path.exists() {
            return Ok(None);
        }
        let config: Config = read_bounded(&path, 65536)?;
        if !config.settings.events_approved
            && !config.settings.attachments_approved
            && !config.settings.query_approved
        {
            return Ok(None);
        }
        config.validate(vault)?;
        if config.schema != "recallcard.local-event-host/1"
            || config.scope != scope
            || config.root != vault.root()
            || !config.python_path.is_absolute()
            || !config
                .worker_config
                .starts_with(path.parent().ok_or("local_event_config_invalid")?)
        {
            return Err("local_event_config_invalid".into());
        }
        let config_hash = config.identity()?;
        Ok(Some(Self {
            vault,
            config,
            config_hash,
            path,
        }))
    }
    pub fn disable(vault: &Vault, scope: &str) -> Result<()> {
        Self::disable_mode(vault, scope, false)
    }
    fn invalidate_worker(vault: &Vault, config: &Config) -> Result<()> {
        let mut worker: Value = read_bounded(&config.worker_config, 65536)?;
        worker["events_approved"] = json!(false);
        worker["attachments_approved"] = json!(false);
        worker["query_approved"] = json!(false);
        vault.write_replace(&config.worker_config, &worker)
    }
    fn disable_mode(vault: &Vault, scope: &str, cloud_only: bool) -> Result<()> {
        let path = config_path(vault, scope)?;
        let _guard = vault.lock()?;
        if cloud_only {
            CloudEmbeddingRuntime::new(vault)
                .revoke_sources(scope)
                .map_err(|_| "source_cloud_approval_unavailable")?;
        }
        if !path.exists() {
            return Ok(());
        }
        let mut config: Config = read_bounded(&path, 65536)?;
        if cloud_only && config.cloud.is_none() {
            return Ok(());
        }
        if config.cloud.is_some() && !cloud_only {
            CloudEmbeddingRuntime::new(vault)
                .revoke_sources(scope)
                .map_err(|_| "source_cloud_approval_unavailable")?;
        }
        // 判定、撤权及 worker 失效在同一 Vault 锁内；不会误停并发切换后的本机配置。
        Self::invalidate_worker(vault, &config)?;
        config.settings.events_approved = false;
        config.settings.attachments_approved = false;
        config.settings.query_approved = false;
        vault.write_replace(&path, &config)
    }
    fn current(&self) -> Result<()> {
        let current: Config = read_bounded(&self.path, 65536)?;
        if hash(&serde_json::to_vec(&current).map_err(|_| "local_event_config_invalid")?)
            != self.config_hash
        {
            return Err("local_event_configuration_changed".into());
        }
        current.validate(self.vault)?;
        let expected = self.config.worker_settings()?;
        let mut actual: Value = read_bounded(&self.config.worker_config, 65536)?;
        // 兼容旧 worker 配置，但缺失字段只等于 false。
        if actual.get("attachments_approved").is_none() {
            actual["attachments_approved"] = json!(false);
        }
        if actual != expected {
            return Err("local_event_configuration_changed".into());
        }
        Ok(())
    }
    pub fn is_cloud(&self) -> bool {
        self.config.cloud.is_some()
    }
    pub fn disable_cloud(vault: &Vault, scope: &str) -> Result<()> {
        Self::disable_mode(vault, scope, true)
    }
    fn dir(&self) -> Result<&Path> {
        self.config
            .worker_config
            .parent()
            .ok_or_else(|| "local_event_config_invalid".into())
    }
    pub fn status(&self) -> Result<Value> {
        self.current()?;
        let mut status = worker::exchange(&self.config, json!({"op":"status"}))?;
        status["settings"] = serde_json::to_value(&self.config.settings)
            .map_err(|_| "local_event_config_invalid")?;
        status["index_state"] = status["state"].clone();
        let _guard = self.vault.read_guard()?;
        if status["source_epoch"].is_string()
            && status["source_epoch"] != snapshot::source_inventory(self)?.0
        {
            status["index_state"] = json!("stale");
        }
        self.current()?;
        status["state"] = json!("configured");
        status["provider"] = json!(if self.is_cloud() { "cloud" } else { "local" });
        status["message"] = json!(if self.is_cloud() {
            "仅向明确批准的云接收者编码已选范围；配置不代表索引已覆盖或模型质量已核验"
        } else {
            "仅本机明确批准的普通对话或附件索引；配置不代表索引已覆盖或模型质量已核验"
        });
        Ok(status)
    }
    pub fn refresh_step(&self) -> Result<Value> {
        if !self.config.settings.events_approved && !self.config.settings.attachments_approved {
            return Err("local_event_corpus_not_approved".into());
        }
        self.current()?;
        let (manifest, mut progress) = {
            let _guard = self.vault.read_guard()?;
            snapshot::prepare(self)?
        };
        let began = worker::exchange(
            &self.config,
            json!({"op":"begin","source_epoch":manifest.epoch,"lexical_version":lexical_context::VERSION}),
        )?;
        let build_id = began["build_id"]
            .as_str()
            .ok_or("local_event_response_invalid")?;
        let mut receipt = began["receipt"].clone();
        let accepted = began["accepted_batches"]
            .as_u64()
            .ok_or("local_event_response_invalid")? as usize;
        if accepted < progress.accepted_batches {
            progress.source_pos = 0;
            progress.chunk_pos = 0;
            progress.accepted_batches = 0;
        }
        if progress.source_pos == manifest.sources.len() && progress.chunk_pos != 0 {
            return Err("local_event_progress_invalid".into());
        }
        if progress.source_pos < manifest.sources.len() {
            let source = &manifest.sources[progress.source_pos];
            let (event, text) = {
                let _guard = self.vault.read_guard()?;
                self.current()?;
                if snapshot::source_inventory(self)?.0 != manifest.epoch
                    || !snapshot::verified_sources(self, &manifest.epoch)?
                        .contains(&source.reference())
                {
                    return Err("local_event_source_not_current".into());
                }
                let event = snapshot::read_source(self.vault, &manifest, source)?;
                let text = source.text(self.vault, &event)?;
                (event, text)
            };
            let ranges = snapshot::ranges(&text, self.config.settings.chunk_bytes);
            if source.chunks != ranges.len() {
                return Err("local_event_source_changed".into());
            }
            if progress.chunk_pos >= ranges.len() {
                return Err("local_event_progress_invalid".into());
            }
            let end = progress
                .chunk_pos
                .checked_add(
                    (if self.is_cloud() { 4usize } else { 8usize })
                        .min(8192 / self.config.settings.dimensions)
                        .min(
                            65536
                                / (self.config.settings.chunk_bytes
                                    + self.config.settings.document_instruction.len()
                                    + 513),
                        ),
                )
                .ok_or("local_event_progress_invalid")?
                .min(ranges.len());
            if end == progress.chunk_pos {
                return Err("local_event_batch_limit".into());
            }
            let chunks:Vec<_>=ranges[progress.chunk_pos..end].iter().enumerate().map(|(n,(start,end))| {
                let (before, after) = lexical_context::neighbors(&text, *start, *end);
                json!({"ordinal":progress.chunk_pos+n,"start_byte":start,"end_byte":end,"text_sha256":hash(&text.as_bytes()[*start..*end]),"text":&text[*start..*end],"lexical_before":before,"lexical_after":after})
            }).collect();
            let result = worker::exchange(
                &self.config,
                json!({"op":"upsert","build_id":build_id,
                "batch_id":format!("{}:{}",progress.source_pos,progress.chunk_pos),"source":{
                    "ref":source.reference(),"content_hash":hash(text.as_bytes()),"text_bytes":text.len(),
                    "scope":event.data.scope,"role":if source.object.is_some(){Value::Null}else{json!(event.data.role)},"session_ref":event.data.session_key(),"occurred_at":event.data.occurred_at},
                "source_chunk_count":ranges.len(),"chunks":chunks}),
            )?;
            receipt = result["receipt"].clone();
            progress.accepted_batches = result["accepted_batches"]
                .as_u64()
                .ok_or("local_event_response_invalid")?
                as usize;
            progress.chunk_pos = end;
            if end == ranges.len() {
                progress.source_pos += 1;
                progress.chunk_pos = 0;
            }
        }
        let _guard = self.vault.read_guard()?;
        self.current()?;
        if snapshot::source_inventory(self)?.0 != manifest.epoch {
            return Err("local_event_source_epoch_changed".into());
        }
        self.vault
            .write_replace(&self.dir()?.join("progress.json"), &progress)?;
        if progress.source_pos == manifest.sources.len() {
            let published = worker::exchange(
                &self.config,
                json!({"op":"finish","build_id":build_id,"source_epoch":manifest.epoch,
                "source_count":manifest.sources.len(),"chunk_count":manifest.chunk_count()?,"receipt":receipt}),
            )?;
            Ok(
                json!({"state":"ready","complete":true,"processed_sources":progress.source_pos,"total_sources":manifest.sources.len(),"result":published}),
            )
        } else {
            Ok(
                json!({"state":"building","complete":false,"processed_sources":progress.source_pos,"total_sources":manifest.sources.len(),"result":began}),
            )
        }
    }
    pub fn search(&self, query: &str, limit: usize) -> Result<Value> {
        self.search_before(
            query,
            limit,
            std::time::Instant::now() + std::time::Duration::from_secs(45),
        )
    }
    pub(crate) fn verify_binding(&self, epoch: &str, config: &str) -> Result<()> {
        self.current()?;
        if config != self.config_hash || snapshot::source_inventory(self)?.0 != epoch {
            return Err("local_event_source_or_configuration_stale".into());
        }
        Ok(())
    }
    pub(crate) fn verify_source(&self, epoch: &str, config: &str, reference: &str) -> Result<()> {
        self.verify_binding(epoch, config)?;
        if !snapshot::verified_sources(self, epoch)?.contains(reference) {
            return Err("local_event_source_not_current".into());
        }
        Ok(())
    }
    pub(crate) fn search_before(
        &self,
        query: &str,
        limit: usize,
        deadline: std::time::Instant,
    ) -> Result<Value> {
        if !self.config.settings.query_approved {
            return Err("local_event_query_not_approved".into());
        }
        if query.trim().is_empty() || query.len() > 4096 || !(1..=50).contains(&limit) {
            return Err("local_event_query_invalid".into());
        }
        self.current()?;
        let manifest: Manifest = read_bounded(&self.dir()?.join("manifest.json"), MAX_JSON)?;
        {
            let _guard = self.vault.read_guard()?;
            if snapshot::source_inventory(self)?.0 != manifest.epoch {
                return Err("local_event_index_stale_refresh_required".into());
            }
        }
        let status = worker::exchange_before(&self.config, json!({"op":"status"}), deadline)?;
        if status["state"] != "complete" || status["source_epoch"] != manifest.epoch {
            return Err("local_event_index_stale_refresh_required".into());
        }
        let generation = status["published_generation"]
            .as_str()
            .ok_or("local_event_index_not_ready")?;
        // status 可能等待在队列中；真正提交新问题前重新检查批准和配置。
        self.current()?;
        let candidates = worker::exchange_before(
            &self.config,
            json!({"op":"query","query":query,"limit":100,"expected_generation":generation}),
            deadline,
        )?;
        let _guard = self.vault.read_guard()?;
        self.current()?;
        if snapshot::source_inventory(self)?.0 != manifest.epoch {
            return Err("local_event_source_epoch_changed".into());
        }
        if candidates["source_epoch"] != manifest.epoch
            || candidates["generation"] != generation
            || candidates["space_signature"]
                != hash(
                    &serde_json::to_vec(&self.config.space())
                        .map_err(|_| "local_event_config_invalid")?,
                )
        {
            return Err("local_event_response_invalid".into());
        }
        let current_sources = snapshot::verified_sources(self, &manifest.epoch)?;
        let manifest_ids: BTreeSet<_> = manifest.sources.iter().map(|s| s.reference()).collect();
        let source_refs: Vec<_> = current_sources.iter().collect();
        if candidates["source_set_hash"]
            != hash(&serde_json::to_vec(&source_refs).map_err(|_| "local_event_response_invalid")?)
            || status["chunk_count"] != json!(manifest.chunk_count()?)
        {
            return Err("local_event_coverage_mismatch".into());
        }
        if manifest_ids != *current_sources
            || manifest_ids.len() != manifest.sources.len()
            || status["source_count"] != json!(manifest.sources.len())
        {
            return Err("local_event_coverage_mismatch".into());
        }
        let lexical_only = evidence::lexical_only(
            &candidates["coverage"],
            status["lexical_version"].is_string(),
        )?;
        let query_support = ranking::assess(query, "");
        let reorder = !query_support.limited && query_support.protected_anchors > 0;
        let mut verified = Vec::new();
        let mut verified_bytes = 0usize;
        let mut seen = BTreeSet::new();
        let mut rejected = 0usize;
        let mut source_bytes = 0usize;
        let mut verification_limited = false;
        let rows = candidates["results"]
            .as_array()
            .ok_or("local_event_response_invalid")?;
        if rows.len() > 100 {
            return Err("local_event_response_limit".into());
        }
        for (semantic_index, row) in rows.iter().enumerate() {
            if !reorder && (verified.len() >= limit || verified_bytes >= 24576) {
                break;
            }
            if std::time::Instant::now() >= deadline || source_bytes >= 32 * 1024 * 1024 {
                verification_limited = true;
                break;
            }
            let reference = row["ref"].as_str().ok_or("local_event_response_invalid")?;
            if !seen.insert(reference.to_owned()) {
                continue;
            }
            let Some(source) = manifest.sources.iter().find(|s| s.reference() == reference) else {
                rejected += 1;
                continue;
            };
            if !current_sources.contains(&source.reference()) {
                rejected += 1;
                continue;
            }
            let event = snapshot::read_source(self.vault, &manifest, source)?;
            let text = source.text(self.vault, &event)?;
            source_bytes = source_bytes.saturating_add(text.len());
            let start = row["start_byte"]
                .as_u64()
                .ok_or("local_event_response_invalid")? as usize;
            let end = row["end_byte"]
                .as_u64()
                .ok_or("local_event_response_invalid")? as usize;
            let score = evidence::candidate_score(row, lexical_only)?;
            if row["content_hash"] != hash(text.as_bytes())
                || start >= end
                || end > text.len()
                || !text.is_char_boundary(start)
                || !text.is_char_boundary(end)
                || row["text_sha256"] != hash(&text.as_bytes()[start..end])
            {
                return Err("local_event_candidate_source_mismatch".into());
            }
            let retrieval = evidence::verify(
                query,
                &text,
                row,
                semantic_index,
                status["lexical_version"].is_string(),
            )?;
            let (excerpt_start, excerpt_end) =
                lexical_context::excerpt_range(&text, start, end, retrieval.lexical_range);
            let snippet = text[excerpt_start..excerpt_end].to_owned();
            let support = ranking::assess_excerpt(
                query,
                &snippet,
                ranking::separate_boundary(text[..excerpt_start].chars().next_back()),
                ranking::separate_boundary(text[excerpt_end..].chars().next()),
            );
            let value = json!({"ref":reference,"kind":if source.object.is_some(){"attachment"}else{"event"},"scope":event.data.scope,"role":if source.object.is_some(){Value::Null}else{json!(event.data.role)},
                "snapshot":source.snapshot(&event)?,"content_hash":hash(text.as_bytes()),
                "text_hash":hash(snippet.as_bytes()),"matched_chunk":{"start_byte":start,"end_byte":end,"text_hash":row["text_sha256"]},
                "text":snippet,"text_range":{"start_byte":excerpt_start,"end_byte":excerpt_end,"total_bytes":text.len(),"projection":if source.object.is_some(){crate::context::attachments::PROJECTION}else{"event.text"}},"text_truncated":excerpt_start!=0||excerpt_end!=text.len(),
                "score":score,"score_kind":if lexical_only{"unavailable"}else{"cosine"},"semantic_rank":retrieval.semantic_rank,"retrieval":retrieval.metadata,
                "retrieval_support":{"kind":if support.complete{"protected_anchor_match"}else if retrieval.lexical_range.is_some(){"lexical_candidate"}else{"semantic_candidate"},"match_scope":"returned_excerpt","case_policy":"protected_case_sensitive_ascii_support_case_insensitive","protected":support.complete,"protected_anchors":support.protected_anchors,"matched_protected_anchors":support.matched_protected_anchors,"ascii_terms":support.ascii_terms,"matched_ascii_terms":support.matched_ascii_terms,"limited":support.limited},
                "relevance":"unassessed","state":"captured","evidence":if source.object.is_some(){"Attachment/Content".to_owned()}else{format!("{:?}/{:?}",event.data.role,event.data.origin)},"evidence_refs":[if source.object.is_some(){format!("event:{}",event.id)}else{reference.to_owned()}],
                "session_ref":event.data.session_key(),"occurred_at":event.data.occurred_at,"time_note":if event.data.occurred_at.is_none(){"发生时间未知；捕获时间不代表发生时间"}else{""}});
            verified_bytes += serde_json::to_vec(&value)
                .map_err(|_| "local_event_response_invalid")?
                .len();
            verified.push(value);
        }
        // Stable ordering preserves the original fused (or legacy semantic) order within each tier.
        // This can only rearrange the returned, source-verified candidate window.
        verified.sort_by_key(|row| row["retrieval_support"]["protected"] != true);
        let scanned = verified.len();
        let mut results = Vec::new();
        let mut bytes = 0usize;
        for (rank, mut value) in verified.into_iter().enumerate() {
            value["scope_rank"] = json!(rank + 1);
            let size = serde_json::to_vec(&value)
                .map_err(|_| "local_event_response_invalid")?
                .len();
            if results.len() == limit || bytes + size > 24576 {
                break;
            }
            bytes += size;
            results.push(value);
        }
        let mut attachment_coverage = snapshot::attachment_coverage(self, &manifest.epoch)?;
        attachment_coverage["semantic"] =
            json!(self.config.settings.attachments_approved && !lexical_only);
        Ok(
            json!({"query":query,"results":results,"match_count":results.len(),"status":if lexical_only{if results.is_empty(){"unavailable"}else{"partial"}}else if results.is_empty(){"no_verified_candidates"}else{"ok"},
            "truncated":rows.len()>results.len()||rejected>0||verification_limited||candidates["coverage"]["candidates_truncated"]==true||candidates["coverage"]["lexical"]["candidates_truncated"]==true,"coverage":{"event_search":if lexical_only{"local_lexical_fallback"}else if self.is_cloud(){"cloud_semantic"}else{"local_semantic"},"semantic_search":if lexical_only{"unavailable"}else if self.is_cloud(){"cloud_event_index"}else{"local_event_index"},
            "provider":if self.is_cloud(){"cloud"}else{"local"},"cloud_fallback":false,"local_event_semantic":!lexical_only&&!self.is_cloud(),"cloud_event_semantic":!lexical_only&&self.is_cloud(),"attachments":attachment_coverage,"semantic":candidates["coverage"]["semantic"],"semantic_query_encoding":candidates["coverage"]["query_encoding"],"event_index_generation":generation,
            "event_index_snapshot":manifest.epoch,"local_config_identity":self.config_hash,"source_verification":"current_event_hash_and_utf8_range","rejected_candidates":rejected,
            "partial":lexical_only||rejected>0||verification_limited||attachment_coverage["status"]=="partial","candidate_policy":"verified_excerpt_anchors_then_channel_order/v2","lexical":candidates["coverage"]["lexical"],"fusion":candidates["coverage"]["fusion"],"verified_candidates":scanned,"candidate_window_limit":100,"candidate_reordering_enabled":reorder,"candidate_verification_limited":verification_limited,"candidate_text_bytes":source_bytes,"chunk_selection":candidates["coverage"]["chunk_selection"],"note":if lexical_only{"本机编码服务不可用；仅返回有界词法候选，语义路线未执行。无命中不代表没有相关历史。"}else{"向量候选不是答案；角色、时间与后续纠正需回到原话核对。"}}}),
        )
    }
}

/// 调用方持最终 Vault 读锁；跨入口定位符保守绑定所有来源变化。
pub(crate) fn source_epoch(vault: &Vault) -> Result<String> {
    Ok(snapshot::inventory(vault)?.0)
}

/// worker 真正取出排队任务后检查，不能让旧会话沿用已撤销配置开始新编码。
fn worker_configuration_current(config: &Config) -> Result<()> {
    let vault = Vault::open_existing(&config.root).map_err(|_| "local_event_config_unavailable")?;
    let index =
        LocalEvents::open(&vault, &config.scope)?.ok_or("local_event_configuration_changed")?;
    let expected = hash(&serde_json::to_vec(config).map_err(|_| "local_event_config_invalid")?);
    if index.config_hash != expected {
        return Err("local_event_configuration_changed".into());
    }
    index.current()
}
