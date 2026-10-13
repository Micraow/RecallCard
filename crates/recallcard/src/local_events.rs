//! 独立本机原始对话索引。正文只进入有界编码批次和最终命中原文；绝无云回退。
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
mod evidence;
mod lexical_context;
pub(crate) mod ranking;
mod snapshot;
mod worker;
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
            memory_approved: self.events_approved,
            resume_background: false,
        };
        target
            .validate()
            .map_err(|_| "local_event_settings_invalid")?;
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
        let space = settings.space();
        let signature =
            hash(&serde_json::to_vec(&space).map_err(|_| "local_event_config_invalid")?);
        let dir = parent.join(signature);
        crate::vault::reject_symlink(&dir)?;
        std::fs::create_dir_all(&dir).map_err(|_| "local_event_config_unavailable")?;
        let worker_config = dir.join("worker.json");
        let worker = json!({"schema":"recallcard.local-event-config/1","database":dir.join("index.sqlite"),"scope":scope,
            "space":space,"events_approved":settings.events_approved,"query_approved":settings.query_approved,
            "disk_cap_bytes":settings.disk_cap_mib*1024*1024,"backend":"python"});
        let config = Config {
            schema: "recallcard.local-event-host/1".into(),
            scope: scope.into(),
            root: vault.root().to_owned(),
            settings,
            python,
            python_path,
            worker_config,
        };
        let _guard = vault.lock()?;
        vault.write_replace(&config.worker_config, &worker)?;
        vault.write_replace(&path, &config)?;
        drop(_guard);
        Self::open(vault, scope)?.ok_or_else(|| "local_event_config_unavailable".into())
    }
    pub fn open(vault: &'a Vault, scope: &str) -> Result<Option<Self>> {
        let path = config_path(vault, scope)?;
        if !path.exists() {
            return Ok(None);
        }
        let config: Config = read_bounded(&path, 65536)?;
        if !config.settings.events_approved && !config.settings.query_approved {
            return Ok(None);
        }
        config.settings.validate()?;
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
        let config_hash =
            hash(&serde_json::to_vec(&config).map_err(|_| "local_event_config_invalid")?);
        Ok(Some(Self {
            vault,
            config,
            config_hash,
            path,
        }))
    }
    pub fn disable(vault: &Vault, scope: &str) -> Result<()> {
        let path = config_path(vault, scope)?;
        if path.exists() {
            let _guard = vault.lock()?;
            let mut cfg: Config = read_bounded(&path, 65536)?;
            cfg.settings.events_approved = false;
            cfg.settings.query_approved = false;
            // 保存明确停用态，不删除原话或派生索引；open 会明确拒绝无范围配置。
            vault.write_replace(&path, &cfg)?;
            let mut worker: Value = read_bounded(&cfg.worker_config, 65536)?;
            worker["events_approved"] = json!(false);
            worker["query_approved"] = json!(false);
            vault.write_replace(&cfg.worker_config, &worker)?;
        }
        Ok(())
    }
    fn current(&self) -> Result<()> {
        let current: Config = read_bounded(&self.path, 65536)?;
        if hash(&serde_json::to_vec(&current).map_err(|_| "local_event_config_invalid")?)
            != self.config_hash
        {
            return Err("local_event_configuration_changed".into());
        }
        let expected = json!({"schema":"recallcard.local-event-config/1","database":self.dir()?.join("index.sqlite"),"scope":self.config.scope,
            "space":self.config.settings.space(),"events_approved":self.config.settings.events_approved,"query_approved":self.config.settings.query_approved,
            "disk_cap_bytes":self.config.settings.disk_cap_mib*1024*1024,"backend":"python"});
        let actual: Value = read_bounded(&self.config.worker_config, 65536)?;
        if actual != expected {
            return Err("local_event_configuration_changed".into());
        }
        Ok(())
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
            && status["source_epoch"] != snapshot::inventory(self.vault)?.0
        {
            status["index_state"] = json!("stale");
        }
        status["state"] = json!("configured");
        status["message"] =
            json!("仅本机普通对话向量索引；服务配置不代表索引已覆盖或模型质量已核验");
        Ok(status)
    }
    pub fn refresh_step(&self) -> Result<Value> {
        if !self.config.settings.events_approved {
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
            let event = {
                let _guard = self.vault.read_guard()?;
                self.current()?;
                if snapshot::inventory(self.vault)?.0 != manifest.epoch
                    || !snapshot::verified_sources(self, &manifest.epoch)?.contains(&source.id)
                {
                    return Err("local_event_source_not_current".into());
                }
                snapshot::read_source(self.vault, &manifest, source)?
            };
            let text = event.data.text();
            let ranges = snapshot::ranges(&text, self.config.settings.chunk_bytes);
            if source.chunks != ranges.len() {
                return Err("local_event_source_changed".into());
            }
            if progress.chunk_pos >= ranges.len() {
                return Err("local_event_progress_invalid".into());
            }
            let end = progress
                .chunk_pos
                .checked_add(8.min(8192 / self.config.settings.dimensions).min(
                    65536
                        / (self.config.settings.chunk_bytes
                            + self.config.settings.document_instruction.len()
                            + 513),
                ))
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
                    "ref":format!("event:{}",event.id),"content_hash":hash(text.as_bytes()),"text_bytes":text.len(),
                    "scope":event.data.scope,"role":event.data.role,"session_ref":event.data.session_key(),"occurred_at":event.data.occurred_at},
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
        if snapshot::inventory(self.vault)?.0 != manifest.epoch {
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
        if config != self.config_hash || snapshot::inventory(self.vault)?.0 != epoch {
            return Err("local_event_source_or_configuration_stale".into());
        }
        Ok(())
    }
    pub(crate) fn verify_source(&self, epoch: &str, config: &str, reference: &str) -> Result<()> {
        self.verify_binding(epoch, config)?;
        let id = reference
            .strip_prefix("event:")
            .ok_or("local_event_reference_invalid")?;
        if !snapshot::verified_sources(self, epoch)?.contains(id) {
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
            if snapshot::inventory(self.vault)?.0 != manifest.epoch {
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
        if snapshot::inventory(self.vault)?.0 != manifest.epoch {
            return Err("local_event_source_epoch_changed".into());
        }
        if candidates["source_epoch"] != manifest.epoch
            || candidates["generation"] != generation
            || candidates["space_signature"]
                != hash(
                    &serde_json::to_vec(&self.config.settings.space())
                        .map_err(|_| "local_event_config_invalid")?,
                )
        {
            return Err("local_event_response_invalid".into());
        }
        let current_sources = snapshot::verified_sources(self, &manifest.epoch)?;
        let manifest_ids: BTreeSet<_> = manifest.sources.iter().map(|s| s.id.clone()).collect();
        let source_refs: Vec<_> = current_sources
            .iter()
            .map(|id| format!("event:{id}"))
            .collect();
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
            let Some(source) = manifest
                .sources
                .iter()
                .find(|s| format!("event:{}", s.id) == reference)
            else {
                rejected += 1;
                continue;
            };
            if !current_sources.contains(&source.id) {
                rejected += 1;
                continue;
            }
            let event = snapshot::read_source(self.vault, &manifest, source)?;
            let text = event.data.text();
            source_bytes = source_bytes.saturating_add(text.len());
            let start = row["start_byte"]
                .as_u64()
                .ok_or("local_event_response_invalid")? as usize;
            let end = row["end_byte"]
                .as_u64()
                .ok_or("local_event_response_invalid")? as usize;
            let score = row["score"]
                .as_f64()
                .ok_or("local_event_response_invalid")?;
            if row["content_hash"] != hash(text.as_bytes())
                || start >= end
                || end > text.len()
                || !text.is_char_boundary(start)
                || !text.is_char_boundary(end)
                || row["text_sha256"] != hash(&text.as_bytes()[start..end])
                || !score.is_finite()
                || score.abs() > 1.001
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
            let value = json!({"ref":reference,"kind":"event","scope":event.data.scope,"role":event.data.role,
                "snapshot":crate::model::record_snapshot(&event)?,"content_hash":hash(text.as_bytes()),
                "text_hash":hash(snippet.as_bytes()),"matched_chunk":{"start_byte":start,"end_byte":end,"text_hash":row["text_sha256"]},
                "text":snippet,"text_range":{"start_byte":excerpt_start,"end_byte":excerpt_end,"total_bytes":text.len(),"projection":"event.text"},"text_truncated":excerpt_start!=0||excerpt_end!=text.len(),
                "score":score,"score_kind":"cosine","semantic_rank":retrieval.semantic_rank,"retrieval":retrieval.metadata,
                "retrieval_support":{"kind":if support.complete{"protected_anchor_match"}else if retrieval.lexical_range.is_some(){"lexical_candidate"}else{"semantic_candidate"},"match_scope":"returned_excerpt","case_policy":"protected_case_sensitive_ascii_support_case_insensitive","protected":support.complete,"protected_anchors":support.protected_anchors,"matched_protected_anchors":support.matched_protected_anchors,"ascii_terms":support.ascii_terms,"matched_ascii_terms":support.matched_ascii_terms,"limited":support.limited},
                "relevance":"unassessed","state":"captured","evidence":format!("{:?}/{:?}",event.data.role,event.data.origin),"evidence_refs":[reference],
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
        Ok(
            json!({"query":query,"results":results,"match_count":results.len(),"status":if results.is_empty(){"no_verified_candidates"}else{"ok"},
            "truncated":rows.len()>results.len()||rejected>0||verification_limited||candidates["coverage"]["candidates_truncated"]==true||candidates["coverage"]["lexical"]["candidates_truncated"]==true,"coverage":{"event_search":"local_semantic","semantic_search":"local_event_index",
            "local_event_semantic":true,"semantic_query_encoding":candidates["coverage"]["query_encoding"],"event_index_generation":generation,
            "event_index_snapshot":manifest.epoch,"local_config_identity":self.config_hash,"source_verification":"current_event_hash_and_utf8_range","rejected_candidates":rejected,
            "partial":rejected>0||verification_limited,"candidate_policy":"verified_excerpt_anchors_then_channel_order/v2","lexical":candidates["coverage"]["lexical"],"fusion":candidates["coverage"]["fusion"],"verified_candidates":scanned,"candidate_window_limit":100,"candidate_reordering_enabled":reorder,"candidate_verification_limited":verification_limited,"candidate_text_bytes":source_bytes,"chunk_selection":candidates["coverage"]["chunk_selection"],"note":"本机向量候选不是答案；角色、时间与后续纠正需回到原话核对。"}}),
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
