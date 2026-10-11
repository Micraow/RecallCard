//! 独立本机语料批准的增量维护；半成品与可检索索引分开，发布前重验当前正本。
use super::*;
use std::io::Write;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalIndexConfig {
    pub endpoint: String,
    pub scopes: Vec<String>,
    #[serde(default = "default_documents")]
    pub max_documents_per_step: usize,
    #[serde(default = "default_bytes")]
    pub max_input_bytes: usize,
}
fn default_documents() -> usize {
    4
}
impl LocalIndexConfig {
    pub(super) fn valid(&self) -> bool {
        LocalQueryConfig {
            endpoint: self.endpoint.clone(),
            scopes: self.scopes.clone(),
            max_local_calls: 1,
            max_input_bytes: self.max_input_bytes,
        }
        .valid()
            && (1..=8).contains(&self.max_documents_per_step)
    }
}
struct Planned {
    reference: String,
    scope: String,
    content_hash: String,
    input_hash: String,
    text: String,
}
pub(crate) struct Work {
    config: SemanticConfig,
    managed: Option<std::sync::Arc<managed_cloud::ManagedCloud>>,
    config_hash: String,
    partial_path: PathBuf,
    index: Index,
    planned: Vec<Planned>,
    ready: BTreeMap<String, Vec<f64>>,
    pub generation: String,
    pub total: usize,
    pub indexed: usize,
    pub encoded: usize,
    pub already_current: bool,
}
impl Work {
    pub fn prepare(
        vault: &crate::Vault,
        path: &Path,
        scope: &str,
        documents: &[Document],
        time: DateTime<Utc>,
        partial: &Path,
    ) -> SResult<Option<Self>> {
        let backend = SemanticSearch::from_application_config_file(vault, path, &[scope.into()])
            .map_err(|_| SemanticError("offline_config_invalid"))?;
        let config = backend.config;
        let managed = backend.managed;
        let endpoint = if let Some(cloud) = &managed {
            if !cloud.memory_approved() {
                return Ok(None);
            }
            cloud.endpoint().to_string()
        } else if let Some(local) = &config.local_index {
            if !local.scopes.contains(&scope.to_owned()) {
                return failure("index_not_approved");
            }
            local.endpoint.clone()
        } else {
            return Ok(None);
        };
        let index: Index = read_local(&config.index_path)?;
        index.validate_shape(&config.expected_space_signature, false)?;
        if index.space.endpoint != endpoint || index.scope != [scope] {
            return failure("scope_mismatch");
        }
        let corpus = corpus_documents(documents, &index.scope, time);
        let generation = corpus_generation(&corpus);
        let planned: Vec<_> = corpus
            .iter()
            .map(|d| {
                let text = d["text"].as_str().unwrap();
                Planned {
                    reference: d["ref"].as_str().unwrap().into(),
                    scope: d["scope"].as_str().unwrap().into(),
                    content_hash: d["content_hash"].as_str().unwrap().into(),
                    input_hash: prepared_hash(&index.space.document_instruction, text),
                    text: if index.space.document_instruction.is_empty() {
                        text.into()
                    } else {
                        format!("{}\n{text}", index.space.document_instruction)
                    },
                }
            })
            .collect();
        if planned.len() > MAX_DOCUMENTS
            || planned.len().saturating_mul(index.space.dimensions) > MAX_VECTOR_VALUES
        {
            return failure("index_size_exceeded");
        }
        let mut reusable = BTreeMap::new();
        for row in &index.documents {
            reusable.insert(row.input_hash.clone(), row.vector.clone());
        }
        let partial_path = partial.with_file_name(format!(
            "index-partial-{}.json",
            hash(format!("{}\n{}", config.expected_space_signature, scope).as_bytes())
        ));
        let checkpoint = if partial_path.exists() {
            Some((partial_path.as_path(), true))
        } else if partial.exists() {
            Some((partial, false))
        } else {
            None
        };
        if let Some((path, strict)) = checkpoint {
            let saved: Index = read_local(path)?;
            // 旧版本只有单个半成品；仅同空间/同范围可迁移，换模型不能误复用或阻塞新任务。
            if strict
                || (saved.space_signature == config.expected_space_signature
                    && saved.scope == index.scope)
            {
                saved.validate_shape(&config.expected_space_signature, false)?;
                if saved.scope != index.scope {
                    return failure("scope_mismatch");
                }
                for row in saved.documents {
                    if reusable
                        .insert(row.input_hash, row.vector.clone())
                        .is_some_and(|old| old != row.vector)
                    {
                        return failure("invalid_cache");
                    }
                }
            }
        }
        let needed: BTreeSet<_> = planned.iter().map(|p| p.input_hash.as_str()).collect();
        reusable.retain(|key, _| needed.contains(key.as_str()));
        let indexed = planned
            .iter()
            .filter(|p| reusable.contains_key(&p.input_hash))
            .count();
        let config_hash = hash(
            &serde_json::to_vec(&config).map_err(|_| SemanticError("offline_config_invalid"))?,
        );
        let already_current = index.complete
            && index
                .check_current(documents, &[scope.into()], time)
                .is_ok();
        Ok(Some(Self {
            config,
            managed,
            config_hash,
            partial_path,
            index,
            total: planned.len(),
            planned,
            ready: reusable,
            generation,
            indexed,
            encoded: 0,
            already_current,
        }))
    }
    pub fn same_generation(&self, documents: &[Document], time: DateTime<Utc>) -> bool {
        corpus_generation(&corpus_documents(documents, &self.index.scope, time)) == self.generation
    }
    pub fn config_current(&self, path: &Path) -> bool {
        if let Some(cloud) = &self.managed {
            return cloud.current();
        }
        SemanticSearch::from_offline_config_file(path)
            .ok()
            .is_some_and(|b| {
                serde_json::to_vec(&b.config)
                    .ok()
                    .is_some_and(|v| hash(&v) == self.config_hash)
            })
    }
    pub fn encode_step(&mut self) -> SResult<()> {
        let (max_documents, max_bytes) = self
            .config
            .local_index
            .as_ref()
            .map(|local| (local.max_documents_per_step, local.max_input_bytes))
            .unwrap_or((4, 65536));
        let mut selected = Vec::new();
        let mut seen = BTreeSet::new();
        let mut bytes = 0;
        let max = max_documents.min(8192 / self.index.space.dimensions);
        for doc in &self.planned {
            if self.ready.contains_key(&doc.input_hash) || !seen.insert(&doc.input_hash) {
                continue;
            }
            if doc.text.len() > max_bytes {
                return failure("embedding_document_too_large");
            }
            if selected.len() == max || bytes + doc.text.len() > max_bytes {
                break;
            }
            bytes += doc.text.len();
            selected.push(doc);
        }
        if selected.is_empty() {
            return Ok(());
        }
        let request = json!({"id":1,"op":"embed_documents","space":self.index.space,"texts":selected.iter().map(|p|&p.text).collect::<Vec<_>>(),"allowed_scopes":self.index.scope});
        let response = if let Some(cloud) = &self.managed {
            let bytes = cloud.exchange(&self.config, &request)?;
            let envelope: Value =
                serde_json::from_slice(&bytes).map_err(|_| SemanticError("malformed_response"))?;
            if envelope["id"] != 1 || envelope["ok"] != true {
                return failure(match envelope["error"]["code"].as_str() {
                    Some("provider_auth") => "provider_auth",
                    Some("rate_limited") => "rate_limited",
                    Some("network_error") => "network_error",
                    _ => "worker_error",
                });
            }
            envelope["result"].clone()
        } else {
            exchange_index(self.config.clone(), request)?
        };
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Encoded {
            space_signature: String,
            encoded: Vec<EncodedQuery>,
            #[serde(default)]
            diagnostics: Option<Value>,
        }
        let response: Encoded =
            serde_json::from_value(response).map_err(|_| SemanticError("malformed_response"))?;
        let _ = response.diagnostics;
        if response.space_signature != self.config.expected_space_signature
            || response.encoded.len() != selected.len()
        {
            return failure("malformed_response");
        }
        for (expected, actual) in selected.iter().zip(&response.encoded) {
            if expected.input_hash != actual.input_hash {
                return failure("malformed_response");
            }
            validate_vector(
                &actual.vector,
                self.index.space.dimensions,
                self.index.space.normalization == "l2",
            )?;
        }
        self.encoded = response.encoded.len();
        for item in response.encoded {
            self.ready.insert(item.input_hash, item.vector);
        }
        self.indexed = self
            .planned
            .iter()
            .filter(|p| self.ready.contains_key(&p.input_hash))
            .count();
        Ok(())
    }
    /// 调用者必须在当前 Vault 读锁及本机配置状态锁内调用，防止校验后发布旧索引。
    pub fn persist(
        &self,
        documents: &[Document],
        time: DateTime<Utc>,
        _partial: &Path,
    ) -> SResult<bool> {
        if corpus_generation(&corpus_documents(documents, &self.index.scope, time))
            != self.generation
        {
            return failure("generation_mismatch");
        }
        let mut candidate = self.index.clone();
        candidate.generation = self.generation.clone();
        candidate.complete = self.indexed == self.total;
        candidate.documents = self
            .planned
            .iter()
            .filter_map(|p| {
                self.ready.get(&p.input_hash).map(|v| IndexDocument {
                    reference: p.reference.clone(),
                    scope: p.scope.clone(),
                    content_hash: p.content_hash.clone(),
                    input_hash: p.input_hash.clone(),
                    vector: v.clone(),
                })
            })
            .collect();
        candidate.validate_shape(&self.config.expected_space_signature, false)?;
        if candidate.complete {
            candidate.check_current(documents, &candidate.scope, time)?;
            atomic_index(&self.config.index_path, &candidate)?;
            // 半成品是可重建缓存，成功发布后保留也不能成为当前事实来源。
            atomic_index(&self.partial_path, &candidate)?;
        } else {
            atomic_index(&self.partial_path, &candidate)?;
        }
        Ok(candidate.complete)
    }
}
fn atomic_index(path: &Path, index: &Index) -> SResult<()> {
    let parent = path.parent().ok_or(SemanticError("index_write_failed"))?;
    if !std::fs::symlink_metadata(parent).is_ok_and(|m| m.is_dir()) {
        return failure("index_write_failed");
    }
    let mut lock_name = path.as_os_str().to_os_string();
    lock_name.push(".index-lock");
    let lock_path = PathBuf::from(lock_name);
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&lock_path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            crate::filesystem::open_local_file(&lock_path)
                .map_err(|_| SemanticError("index_write_failed"))?
        }
        Err(_) => return failure("index_write_failed"),
    };
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return failure("index_write_failed");
    }
    file.try_lock().map_err(|_| SemanticError("index_busy"))?;
    crate::vault::reject_symlink(path).map_err(|_| SemanticError("index_write_failed"))?;
    let bytes = serde_json::to_vec(index).map_err(|_| SemanticError("index_write_failed"))?;
    if bytes.len() as u64 > MAX_CACHE_BYTES {
        return failure("index_size_exceeded");
    }
    let mut pending =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| SemanticError("index_write_failed"))?;
    pending
        .write_all(&bytes)
        .and_then(|_| pending.as_file().sync_all())
        .map_err(|_| SemanticError("index_write_failed"))?;
    crate::vault::reject_symlink(path).map_err(|_| SemanticError("index_write_failed"))?;
    pending
        .persist(path)
        .map_err(|_| SemanticError("index_write_failed"))?;
    #[cfg(unix)]
    File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|_| SemanticError("index_write_failed"))?;
    Ok(())
}
fn exchange_index(config: SemanticConfig, request: Value) -> SResult<Value> {
    let mut input = serde_json::to_vec(&request).map_err(|_| SemanticError("input_limit"))?;
    input.push(b'\n');
    if input.len() > MAX_REQUEST_BYTES {
        return failure("input_limit");
    }
    let timeout = config.timeout_ms;
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("recallcard-local-index".into())
        .spawn(move || {
            let result = (|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_io()
                    .enable_time()
                    .build()
                    .map_err(|_| SemanticError("worker_unavailable"))?;
                runtime.block_on(async {
                    let mut process = Process::start_mode(&config, true)?;
                    let result = tokio::time::timeout(
                        Duration::from_millis(timeout),
                        process.exchange_limit(&input, 512 * 1024),
                    )
                    .await
                    .unwrap_or_else(|_| failure("worker_timeout"));
                    let _ = tokio::time::timeout(Duration::from_millis(250), process.child.kill())
                        .await;
                    result
                })
            })();
            let _ = tx.send(result);
        })
        .map_err(|_| SemanticError("worker_unavailable"))?;
    let raw = rx
        .recv_timeout(Duration::from_millis(timeout + 500))
        .map_err(|_| SemanticError("worker_timeout"))??;
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Envelope {
        id: u64,
        ok: bool,
        #[serde(default)]
        result: Option<Value>,
        #[serde(default)]
        error: Option<WorkerError>,
        #[serde(default)]
        diagnostics: Option<Value>,
    }
    let response: Envelope =
        serde_json::from_slice(&raw).map_err(|_| SemanticError("malformed_response"))?;
    let _ = response.diagnostics;
    if response.id != 1
        || (response.ok && (response.result.is_none() || response.error.is_some()))
        || (!response.ok && (response.result.is_some() || response.error.is_none()))
    {
        return failure("malformed_response");
    }
    if !response.ok {
        return failure(match response.error.unwrap().code.as_str() {
            "local_provider_unavailable" => "local_provider_unavailable",
            "local_provider_error" => "local_provider_error",
            "redirect_blocked" => "redirect_blocked",
            "model_mismatch" => "model_mismatch",
            "index_not_approved" => "index_not_approved",
            "local_budget_exhausted" => "local_budget_exhausted",
            _ => "worker_error",
        });
    }
    Ok(response.result.unwrap())
}
