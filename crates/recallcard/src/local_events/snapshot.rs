//! 有上限的文件身份清单；初建流式验证正本，不将完整聊天保存在快照。
use super::*;
use sha2::{Digest, Sha256};
const MAX_FILES: usize = 100_000;
const MAX_SOURCES: usize = 100_000;
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Source {
    pub id: String,
    pub file: usize,
    pub chunks: usize,
    #[serde(default)]
    pub object: Option<String>,
}
impl Source {
    pub fn reference(&self) -> String {
        match &self.object {
            Some(object) => format!("attachment:{}/{object}", self.id),
            None => format!("event:{}", self.id),
        }
    }
    /// 调用方持读锁，并先以 verified_sources 检查当前父事件和独立语料批准。
    pub fn text(&self, vault: &Vault, event: &Event) -> Result<String> {
        if event.id != self.id {
            return Err("local_event_source_mismatch".into());
        }
        match &self.object {
            Some(object) => Ok(crate::context::attachments::verified_body(vault, event, object)?.1),
            None => Ok(event.data.text()),
        }
    }
    pub fn snapshot(&self, event: &Event) -> Result<String> {
        if event.id != self.id {
            return Err("local_event_source_mismatch".into());
        }
        match &self.object {
            Some(object) => crate::context::attachments::snapshot(event, object),
            None => crate::model::record_snapshot(event),
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub epoch: String,
    pub files: Vec<PathBuf>,
    pub sources: Vec<Source>,
    #[serde(default)]
    pub attachment_coverage: Value,
}
impl Manifest {
    pub fn chunk_count(&self) -> Result<usize> {
        self.sources.iter().try_fold(0usize, |total, source| {
            if source.chunks == 0 || source.chunks > 65536 {
                return Err("local_event_manifest_invalid".into());
            }
            total
                .checked_add(source.chunks)
                .filter(|v| *v <= 1_000_000)
                .ok_or_else(|| "local_event_chunk_count_limit".into())
        })
    }
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Progress {
    pub epoch: String,
    pub source_pos: usize,
    pub chunk_pos: usize,
    #[serde(default)]
    pub accepted_batches: usize,
}

fn collect(
    root: &Path,
    dir: &Path,
    extension: &str,
    depth: usize,
    files: &mut Vec<PathBuf>,
    path_bytes: &mut usize,
) -> Result<()> {
    if depth > 32 {
        return Err("local_event_inventory_depth_limit".into());
    }
    crate::vault::reject_symlink(dir)?;
    for entry in std::fs::read_dir(dir).map_err(|_| "local_event_inventory_unavailable")? {
        let path = entry
            .map_err(|_| "local_event_inventory_unavailable")?
            .path();
        crate::vault::reject_symlink(&path)?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("local_event_inventory_invalid")?;
        if name.starts_with('.') {
            continue;
        }
        let meta =
            std::fs::symlink_metadata(&path).map_err(|_| "local_event_inventory_unavailable")?;
        if meta.is_dir() {
            collect(root, &path, extension, depth + 1, files, path_bytes)?;
        } else if meta.is_file() && path.extension().and_then(|e| e.to_str()) == Some(extension) {
            if files.len() >= MAX_FILES {
                return Err("local_event_inventory_count_limit".into());
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|_| "local_event_inventory_invalid")?;
            let size = relative.as_os_str().len();
            *path_bytes = path_bytes
                .checked_add(size)
                .filter(|n| *n <= 8 * 1024 * 1024)
                .ok_or("local_event_inventory_path_limit")?;
            if size > 4096 {
                return Err("local_event_inventory_path_limit".into());
            }
            files.push(relative.to_owned());
        } else {
            return Err("local_event_inventory_invalid".into());
        }
    }
    Ok(())
}
pub(super) fn inventory(vault: &Vault) -> Result<(String, Vec<PathBuf>)> {
    let mut files = Vec::new();
    let mut path_bytes = 0;
    collect(
        vault.root(),
        &vault.root().join("events"),
        "jsonl",
        0,
        &mut files,
        &mut path_bytes,
    )?;
    files.sort();
    let events = files.clone();
    collect(
        vault.root(),
        &vault.root().join("control/suppressions"),
        "json",
        0,
        &mut files,
        &mut path_bytes,
    )?;
    files.push(PathBuf::from("control/schema-version.json"));
    files.sort();
    let mut digest = Sha256::new();
    for relative in files {
        let file = open_local_file(&vault.root().join(&relative))?;
        let meta = file
            .metadata()
            .map_err(|_| "local_event_inventory_unavailable")?;
        if !meta.is_file() {
            return Err("local_event_inventory_invalid".into());
        }
        let identity = crate::filesystem::file_identity(&file)?;
        let mut stamp = json!([
            relative,
            identity,
            meta.len(),
            meta.modified()
                .map_err(|_| "local_event_inventory_unavailable")?
        ]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            stamp
                .as_array_mut()
                .unwrap()
                .push(json!([meta.ctime(), meta.ctime_nsec()]));
        }
        let encoded = serde_json::to_vec(&stamp).map_err(|_| "local_event_inventory_invalid")?;
        digest.update((encoded.len() as u64).to_le_bytes());
        digest.update(encoded);
    }
    Ok((format!("{:x}", digest.finalize()), events))
}
pub(super) fn ranges(text: &str, bytes: usize) -> Vec<(usize, usize)> {
    let mut result = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + bytes).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if !text[start..end].trim().is_empty() {
            result.push((start, end));
        }
        start = end;
    }
    result
}
fn eligible(event: &Event, scope: &str, suppressed: &BTreeSet<String>) -> bool {
    event.data.scope == scope
        && !suppressed.contains(&event.id)
        && matches!(event.data.role, Role::User | Role::Assistant)
        && !matches!(
            event.data.origin,
            Origin::ContextInjection | Origin::RecallcardDreamJob
        )
        && !event.data.parts.iter().any(|p| {
            matches!(
                p.origin,
                Origin::ContextInjection | Origin::RecallcardDreamJob
            )
        })
}
/// 正本选择仅缓存进程内的身份、块数和有界附件声明；不保存正文。
struct Selection {
    events: Vec<Source>,
    attachments: super::attachment_sources::Plan,
}
struct Canonical {
    root: PathBuf,
    config: String,
    base_epoch: String,
    selection: std::sync::Arc<Selection>,
}
fn canonical_selection(
    index: &LocalEvents<'_>,
    base_epoch: &str,
    files: &[PathBuf],
) -> Result<std::sync::Arc<Selection>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Option<Canonical>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(None));
    #[cfg(unix)]
    {
        let guard = cache
            .lock()
            .map_err(|_| "local_event_snapshot_unavailable")?;
        if let Some(cached) = guard.as_ref().filter(|cached| {
            cached.root == index.vault.root()
                && cached.config == index.config_hash
                && cached.base_epoch == base_epoch
        }) {
            return Ok(cached.selection.clone());
        }
    }
    let suppressed = index.vault.suppressed_ids()?;
    let mut identities = BTreeMap::new();
    let mut revisions = Vec::new();
    for path in files {
        crate::vault_stream::read_event_segment(&index.vault.root().join(path), |event| {
            if identities.len() >= MAX_SOURCES {
                return Err("local_event_source_count_limit".into());
            }
            let key = event.data.revision_key();
            if identities.insert(event.id.clone(), key.clone()).is_some() {
                return Err("local_event_duplicate_source".into());
            }
            if let Some(old) = event.data.revision_of.as_ref() {
                revisions.push((old.clone(), key));
            }
            Ok(())
        })?;
    }
    let replaced: BTreeSet<_> = revisions
        .into_iter()
        .filter_map(|(id, key)| (identities.get(&id) == Some(&key)).then_some(id))
        .collect();
    let mut selection = Selection {
        events: Vec::new(),
        attachments: super::attachment_sources::Plan::default(),
    };
    // 兼容既有 query-only Event 索引：events=false 禁止新编码，不撤销已建普通对话查询。
    // attachments-only 则明确排除 Event 投影；附件从不继承 events_approved。
    let events_selected =
        index.config.settings.events_approved || !index.config.settings.attachments_approved;
    for (file, path) in files.iter().enumerate() {
        crate::vault_stream::read_event_segment(&index.vault.root().join(path), |event| {
            if !eligible(&event, &index.config.scope, &suppressed) || replaced.contains(&event.id) {
                return Ok(());
            }
            if events_selected {
                let chunks = ranges(&event.data.text(), index.config.settings.chunk_bytes).len();
                if chunks > 0 {
                    selection.events.push(Source {
                        id: event.id.clone(),
                        file,
                        chunks,
                        object: None,
                    });
                }
            }
            if index.config.settings.attachments_approved {
                selection.attachments.observe(&event, file);
            }
            Ok(())
        })?;
    }
    if inventory(index.vault)?.0 != base_epoch {
        return Err("local_event_source_epoch_changed".into());
    }
    let selection = std::sync::Arc::new(selection);
    *cache
        .lock()
        .map_err(|_| "local_event_snapshot_unavailable")? = Some(Canonical {
        root: index.vault.root().to_owned(),
        config: index.config_hash.clone(),
        base_epoch: base_epoch.to_owned(),
        selection: selection.clone(),
    });
    Ok(selection)
}

/// 词法入口继续使用 inventory；语义附件快照另绑定已批准对象的元数据和配置身份。
/// 不扫描整个对象目录，不传输附件正文；Unix 仅读元数据，其他平台补有界字节摘要。
pub(super) fn source_inventory(index: &LocalEvents<'_>) -> Result<(String, Vec<PathBuf>)> {
    let (base_epoch, files) = inventory(index.vault)?;
    if !index.config.settings.attachments_approved {
        return Ok((base_epoch, files));
    }
    let selection = canonical_selection(index, &base_epoch, &files)?;
    let encoded = serde_json::to_vec(&json!([
        "local-event-attachment-sources/1",
        crate::context::attachments::PROJECTION,
        base_epoch,
        index.config_hash,
        selection.attachments.stamps(index.vault)
    ]))
    .map_err(|_| "local_event_inventory_invalid")?;
    Ok((hash(&encoded), files))
}

struct Verified {
    root: PathBuf,
    config: String,
    epoch: String,
    refs: std::sync::Arc<BTreeSet<String>>,
    sources: Vec<Source>,
    attachment_coverage: Value,
}
fn verified_snapshot(
    index: &LocalEvents<'_>,
    epoch: &str,
    files: &[PathBuf],
) -> Result<std::sync::Arc<Verified>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Option<std::sync::Arc<Verified>>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(None));
    #[cfg(unix)]
    {
        let guard = cache
            .lock()
            .map_err(|_| "local_event_snapshot_unavailable")?;
        if let Some(cached) = guard.as_ref().filter(|cached| {
            cached.root == index.vault.root()
                && cached.config == index.config_hash
                && cached.epoch == epoch
        }) {
            return Ok(cached.clone());
        }
    }
    let (base_epoch, current_files) = inventory(index.vault)?;
    if current_files != files {
        return Err("local_event_source_epoch_changed".into());
    }
    let selection = canonical_selection(index, &base_epoch, files)?;
    let mut sources = selection.events.clone();
    let attachment_coverage = if index.config.settings.attachments_approved {
        selection.attachments.append_sources(
            index.vault,
            &mut sources,
            index.config.settings.chunk_bytes,
        )
    } else {
        json!({"status":"not_approved","projection":crate::context::attachments::PROJECTION,
            "mode":"bounded_local_semantic_index","semantic":false,"documents":0})
    };
    if sources.len() > MAX_SOURCES {
        return Err("local_event_source_count_limit".into());
    }
    if source_inventory(index)?.0 != epoch {
        return Err("local_event_source_epoch_changed".into());
    }
    let verified = std::sync::Arc::new(Verified {
        root: index.vault.root().to_owned(),
        config: index.config_hash.clone(),
        epoch: epoch.to_owned(),
        refs: std::sync::Arc::new(sources.iter().map(Source::reference).collect()),
        sources,
        attachment_coverage,
    });
    *cache
        .lock()
        .map_err(|_| "local_event_snapshot_unavailable")? = Some(verified.clone());
    Ok(verified)
}

pub(super) fn prepare(index: &LocalEvents<'_>) -> Result<(Manifest, Progress)> {
    let (epoch, files) = source_inventory(index)?;
    let path = index.dir()?.join("manifest.json");
    let verified = verified_snapshot(index, &epoch, &files)?;
    // 派生清单只可复用精确匹配正本验证结果的身份、位置和块数，不能授予任何权限。
    let old = read_bounded::<Manifest>(&path, MAX_JSON)
        .ok()
        .filter(|manifest| {
            manifest.epoch == epoch
                && manifest.files == files
                && manifest.chunk_count().is_ok()
                && manifest.sources == verified.sources
        });
    if let Some(mut manifest) = old {
        manifest.attachment_coverage = verified.attachment_coverage.clone();
        let progress = read_bounded::<Progress>(&index.dir()?.join("progress.json"), 65536)
            .ok()
            .filter(|progress| {
                progress.epoch == epoch && progress.source_pos <= manifest.sources.len()
            })
            .unwrap_or_else(|| Progress {
                epoch,
                ..Default::default()
            });
        return Ok((manifest, progress));
    }
    let manifest = Manifest {
        epoch: epoch.clone(),
        files,
        sources: verified.sources.clone(),
        attachment_coverage: verified.attachment_coverage.clone(),
    };
    let progress = Progress {
        epoch,
        ..Default::default()
    };
    let bytes = serde_json::to_vec(&manifest).map_err(|_| "local_event_manifest_invalid")?;
    if bytes.len() as u64 > MAX_JSON {
        return Err("local_event_manifest_limit".into());
    }
    manifest.chunk_count()?;
    if source_inventory(index)?.0 != manifest.epoch {
        return Err("local_event_source_epoch_changed".into());
    }
    index.vault.write_replace(&path, &manifest)?;
    index
        .vault
        .write_replace(&index.dir()?.join("progress.json"), &progress)?;
    Ok((manifest, progress))
}
pub(super) fn read_source(vault: &Vault, manifest: &Manifest, source: &Source) -> Result<Event> {
    let path = manifest
        .files
        .get(source.file)
        .ok_or("local_event_manifest_invalid")?;
    if path.is_absolute()
        || !path.starts_with("events")
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("local_event_manifest_invalid".into());
    }
    crate::vault::reject_symlink(&vault.root().join(path))?;
    let mut found = None;
    crate::vault_stream::read_event_segment(&vault.root().join(path), |event| {
        if event.id == source.id {
            if found.is_some() {
                return Err("local_event_duplicate_source".into());
            }
            found = Some(event);
        }
        Ok(())
    })?;
    found.ok_or_else(|| "local_event_source_missing".into())
}

/// 权限仅来自当前已验证正本；缓存同时绑定批准配置、正本 epoch 和有界对象元数据。
/// 返回完整引用，避免把父 Event 的权限误用为附件语料批准。
pub(super) fn verified_sources(
    index: &LocalEvents<'_>,
    epoch: &str,
) -> Result<std::sync::Arc<BTreeSet<String>>> {
    let (current, files) = source_inventory(index)?;
    if current != epoch {
        return Err("local_event_source_epoch_changed".into());
    }
    Ok(verified_snapshot(index, epoch, &files)?.refs.clone())
}
pub(super) fn attachment_coverage(index: &LocalEvents<'_>, epoch: &str) -> Result<Value> {
    let (current, files) = source_inventory(index)?;
    if current != epoch {
        return Err("local_event_source_epoch_changed".into());
    }
    Ok(verified_snapshot(index, epoch, &files)?
        .attachment_coverage
        .clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Vault) {
        let directory = tempfile::tempdir().unwrap();
        let vault = Vault::init(&directory.path().join("vault")).unwrap();
        (directory, vault)
    }
    fn index(vault: &Vault, events: bool, attachments: bool) -> LocalEvents<'_> {
        let config = Config {
            cloud: None,
            schema: "recallcard.local-event-host/1".into(),
            scope: "personal".into(),
            root: vault.root().to_owned(),
            settings: Setup {
                endpoint: "http://127.0.0.1:9999/v1/embeddings".into(),
                model: "synthetic".into(),
                revision: "snapshot-contract".into(),
                tokenizer: "synthetic".into(),
                dimensions: 2,
                query_instruction: String::new(),
                document_instruction: String::new(),
                events_approved: events,
                attachments_approved: attachments,
                query_approved: true,
                chunk_bytes: 256,
                disk_cap_mib: 16,
            },
            python: PathBuf::from("python3"),
            python_path: PathBuf::from("/synthetic/no-worker"),
            worker_config: vault.root().join("snapshot-contract/worker.json"),
        };
        LocalEvents {
            vault,
            config_hash: hash(&serde_json::to_vec(&config).unwrap()),
            path: vault.root().join("snapshot-contract/current.json"),
            config,
        }
    }
    fn declaration(vault: &Vault, bytes: &[u8]) -> Value {
        let object = hash(bytes);
        std::fs::write(vault.root().join("objects").join(&object), bytes).unwrap();
        json!({"payload_status":"retained","detected_media_type":"application/octet-stream",
            "declared_media_type":"text/plain","object_sha256":object,"byte_count":bytes.len()})
    }
    fn event(vault: &Vault, message: &str, text: &str, files: Value) -> Event {
        vault.capture(serde_json::from_value(json!({
            "scope":"personal","role":"user","origin":"user_input","content":text,
            "source":{"platform":"synthetic","conversation_id":"snapshot-contract","message_id":message},
            "metadata":{"source_assets":{"files":files}}
        })).unwrap()).unwrap()
    }
    fn attachment(event: &Event, file: &Value) -> String {
        format!(
            "attachment:{}/{}",
            event.id,
            file["object_sha256"].as_str().unwrap()
        )
    }
    fn refs(index: &LocalEvents<'_>) -> std::sync::Arc<BTreeSet<String>> {
        let epoch = source_inventory(index).unwrap().0;
        verified_sources(index, &epoch).unwrap()
    }
    #[test]
    fn independent_approval_bodyless_parents_and_legacy_query_only() {
        let (_directory, vault) = fixture();
        let file = declaration(&vault, "中文附件🙂\r\n".as_bytes());
        let parent = event(&vault, "ordinary", "ordinary original", json!([file]));
        let empty = event(&vault, "bodyless", "", json!([file]));
        let events = index(&vault, true, false);
        let event_refs = refs(&events);
        assert_eq!(event_refs.len(), 1);
        assert!(event_refs.contains(&format!("event:{}", parent.id)));
        assert_eq!(
            source_inventory(&events).unwrap().0,
            inventory(&vault).unwrap().0
        );
        let both = index(&vault, true, true);
        let both_refs = refs(&both);
        assert_eq!(both_refs.len(), 3);
        assert!(both_refs.contains(&attachment(&parent, &file)));
        assert!(both_refs.contains(&attachment(&empty, &file)));
        let attachments = index(&vault, false, true);
        let only_attachment_refs = refs(&attachments);
        assert_eq!(only_attachment_refs.len(), 2);
        assert!(only_attachment_refs
            .iter()
            .all(|reference| reference.starts_with("attachment:")));
        // 同根目录和 Event epoch 下切换批准不能命中旧附件权限缓存。
        assert_eq!(*refs(&index(&vault, false, false)), *event_refs);
        let source: Source =
            serde_json::from_value(json!({"id":parent.id,"file":0,"chunks":1})).unwrap();
        assert!(source.object.is_none());
        assert_eq!(
            source.snapshot(&parent).unwrap(),
            crate::model::record_snapshot(&parent).unwrap()
        );
        let projected = Source {
            object: file["object_sha256"].as_str().map(str::to_owned),
            ..source
        };
        assert_eq!(projected.text(&vault, &parent).unwrap(), "中文附件🙂\r\n");
        assert_eq!(
            projected.snapshot(&parent).unwrap(),
            crate::context::attachments::snapshot(&parent, file["object_sha256"].as_str().unwrap())
                .unwrap()
        );
    }
    #[test]
    fn derived_manifest_cannot_grant_sources_or_claim_coverage() {
        let (_directory, vault) = fixture();
        let file = declaration(&vault, b"PRIVATE_ATTACHMENT_SENTINEL_8471");
        let parent = event(
            &vault,
            "manifest",
            "PRIVATE_EVENT_SENTINEL_8472",
            json!([file]),
        );
        let index = index(&vault, true, true);
        std::fs::create_dir_all(index.dir().unwrap()).unwrap();
        let (mut manifest, _) = prepare(&index).unwrap();
        let encoded = serde_json::to_string(&manifest).unwrap();
        assert!(!encoded.contains("PRIVATE_ATTACHMENT_SENTINEL_8471"));
        assert!(!encoded.contains("PRIVATE_EVENT_SENTINEL_8472"));
        let forged = Source {
            id: parent.id.clone(),
            file: 0,
            chunks: 1,
            object: Some(hash(b"not-declared")),
        };
        let forged_ref = forged.reference();
        manifest.sources.push(forged);
        vault
            .write_replace(&index.dir().unwrap().join("manifest.json"), &manifest)
            .unwrap();
        assert!(!verified_sources(&index, &manifest.epoch)
            .unwrap()
            .contains(&forged_ref));
        let (mut repaired, _) = prepare(&index).unwrap();
        assert_eq!(repaired.sources.len(), 2);
        assert!(repaired
            .sources
            .iter()
            .all(|source| source.reference() != forged_ref));
        repaired.attachment_coverage = json!({"documents":999_999,"status":"ready"});
        vault
            .write_replace(&index.dir().unwrap().join("manifest.json"), &repaired)
            .unwrap();
        let (checked, _) = prepare(&index).unwrap();
        assert_eq!(checked.attachment_coverage["documents"], 1);
    }
    #[test]
    fn currentness_filters_precede_bounded_attachment_selection() {
        let (_directory, vault) = fixture();
        let old_file = declaration(&vault, b"superseded-body");
        let old = event(
            &vault,
            "old",
            "old original",
            Value::Array(vec![old_file.clone(); 257]),
        );
        let mut replacement = old.data.clone();
        replacement.content = "new original".into();
        replacement.revision_of = Some(old.id.clone());
        replacement.metadata = json!({});
        vault.capture(replacement).unwrap();
        let file = declaration(&vault, b"current-body");
        let current = event(&vault, "current", "", json!([file]));
        let suppressed = event(&vault, "suppressed", "", json!([old_file]));
        vault
            .suppress(&suppressed.id, "synthetic suppression".into())
            .unwrap();
        for (message, scope, origin, part_origin) in [
            ("foreign", "project:other", "user_input", None),
            ("injection", "personal", "context_injection", None),
            ("dream", "personal", "recallcard_dream_job", None),
            (
                "part-injection",
                "personal",
                "user_input",
                Some("context_injection"),
            ),
        ] {
            let mut data = current.data.clone();
            data.source.message_id = message.into();
            data.scope = scope.into();
            data.origin = serde_json::from_value(json!(origin)).unwrap();
            if let Some(origin) = part_origin {
                data.parts =
                    serde_json::from_value(json!([{"text":"injected","origin":origin}])).unwrap();
            }
            vault.capture(data).unwrap();
        }
        let index = index(&vault, false, true);
        let epoch = source_inventory(&index).unwrap().0;
        let current_refs = verified_sources(&index, &epoch).unwrap();
        assert_eq!(current_refs.len(), 1);
        assert!(current_refs.contains(&attachment(&current, &file)));
        let coverage = attachment_coverage(&index, &epoch).unwrap();
        assert_eq!(coverage["files_examined"], 1);
        assert_eq!(coverage["limited"], false);
    }
    #[test]
    fn selection_is_declaration_bounded_and_missing_unsupported_bodies_are_partial() {
        let (_directory, vault) = fixture();
        let file = declaration(&vault, b"selected-body");
        let excluded = declaration(&vault, b"outside-first-256");
        let mut declarations = vec![file.clone(); 256];
        declarations.push(excluded.clone());
        let parent = event(&vault, "bounded", "", json!(declarations));
        let index = index(&vault, false, true);
        let epoch = source_inventory(&index).unwrap().0;
        let selected = verified_sources(&index, &epoch).unwrap();
        assert_eq!(selected.len(), 1);
        assert!(selected.contains(&attachment(&parent, &file)));
        assert!(!selected.contains(&attachment(&parent, &excluded)));
        let coverage = attachment_coverage(&index, &epoch).unwrap();
        assert_eq!(coverage["files_examined"], 256);
        assert_eq!(coverage["limited"], true);
        assert_eq!(coverage["status"], "partial");
        // 不扫描未选中的对象，也不让它们的变化影响当前语义快照。
        std::fs::write(
            vault
                .root()
                .join("objects")
                .join(excluded["object_sha256"].as_str().unwrap()),
            b"unrelated-change",
        )
        .unwrap();
        assert_eq!(source_inventory(&index).unwrap().0, epoch);
        std::fs::remove_file(
            vault
                .root()
                .join("objects")
                .join(file["object_sha256"].as_str().unwrap()),
        )
        .unwrap();
        let missing_epoch = source_inventory(&index).unwrap().0;
        assert_ne!(missing_epoch, epoch);
        assert!(verified_sources(&index, &missing_epoch).unwrap().is_empty());
        assert_eq!(
            attachment_coverage(&index, &missing_epoch).unwrap()["status"],
            "partial"
        );
    }
    #[test]
    fn invalid_bodies_and_total_metadata_budget_do_not_abort_other_sources() {
        let (_directory, vault) = fixture();
        let good = declaration(&vault, b"ordinary-safe-body");
        let pdf = declaration(&vault, b"%PDF-disguised-text");
        let binary = declaration(&vault, b"invalid\0text");
        let parent = event(&vault, "mixed", "", json!([pdf, binary, good]));
        let index = index(&vault, false, true);
        let epoch = source_inventory(&index).unwrap().0;
        let selected = verified_sources(&index, &epoch).unwrap();
        assert_eq!(selected.len(), 1);
        assert!(selected.contains(&attachment(&parent, &good)));
        assert_eq!(attachment_coverage(&index, &epoch).unwrap()["skipped"], 2);
        let one_mib = declaration(&vault, &vec![b'x'; crate::context::attachments::FILE_BYTES]);
        event(&vault, "large", "", Value::Array(vec![one_mib; 17]));
        let new_epoch = source_inventory(&index).unwrap().0;
        let coverage = attachment_coverage(&index, &new_epoch).unwrap();
        assert_eq!(coverage["limited"], true);
        assert!(
            coverage["bytes_examined"].as_u64().unwrap()
                <= crate::context::attachments::TOTAL_BYTES as u64
        );
    }
    #[cfg(unix)]
    #[test]
    fn same_size_tamper_with_restored_mtime_invalidates_epoch_and_source() {
        let (_directory, vault) = fixture();
        let file = declaration(&vault, b"original-body");
        event(&vault, "tamper", "ordinary original", json!([file]));
        let index = index(&vault, true, true);
        let epoch = source_inventory(&index).unwrap().0;
        assert_eq!(verified_sources(&index, &epoch).unwrap().len(), 2);
        let path = vault
            .root()
            .join("objects")
            .join(file["object_sha256"].as_str().unwrap());
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        std::fs::write(&path, b"tampered-body").unwrap();
        std::fs::File::open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        let tampered = source_inventory(&index).unwrap().0;
        assert_ne!(tampered, epoch);
        assert!(verified_sources(&index, &epoch).is_err());
        let current = verified_sources(&index, &tampered).unwrap();
        assert_eq!(current.len(), 1);
        assert!(current
            .iter()
            .all(|reference| reference.starts_with("event:")));
        assert_eq!(
            attachment_coverage(&index, &tampered).unwrap()["skipped"],
            1
        );
    }
}
