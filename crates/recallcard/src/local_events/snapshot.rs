//! 有上限的文件身份清单；初建流式验证正本，不将完整聊天保存在快照。
use super::*;
use sha2::{Digest, Sha256};
const MAX_FILES: usize = 100_000;
const MAX_SOURCES: usize = 100_000;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Source {
    pub id: String,
    pub file: usize,
    pub chunks: usize,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub epoch: String,
    pub files: Vec<PathBuf>,
    pub sources: Vec<Source>,
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
pub(super) fn prepare(index: &LocalEvents<'_>) -> Result<(Manifest, Progress)> {
    let (epoch, files) = inventory(index.vault)?;
    let path = index.dir()?.join("manifest.json");
    let verified = verified_sources(index, &epoch)?;
    let old = read_bounded::<Manifest>(&path, MAX_JSON).ok().filter(|m| {
        let ids: BTreeSet<_> = m.sources.iter().map(|s| s.id.clone()).collect();
        m.chunk_count().is_ok()
            && ids == *verified
            && ids.len() == m.sources.len()
            && m.sources
                .iter()
                .all(|s| s.file < files.len() && s.chunks > 0 && s.chunks <= 65536)
    });
    if let Some(manifest) =
        old.filter(|m| m.epoch == epoch && m.files == files && m.sources.len() <= MAX_SOURCES)
    {
        let progress = read_bounded::<Progress>(&index.dir()?.join("progress.json"), 65536)
            .ok()
            .filter(|p| p.epoch == epoch && p.source_pos <= manifest.sources.len())
            .unwrap_or_else(|| Progress {
                epoch,
                ..Default::default()
            });
        return Ok((manifest, progress));
    }
    let suppressed = index.vault.suppressed_ids()?;
    let mut identities = BTreeMap::new();
    let mut revisions = Vec::new();
    let mut sources = Vec::new();
    for (file, path) in files.iter().enumerate() {
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
            if eligible(&event, &index.config.scope, &suppressed) {
                let text = event.data.text();
                if !text.trim().is_empty() {
                    sources.push(Source {
                        id: event.id,
                        file,
                        chunks: ranges(&text, index.config.settings.chunk_bytes).len(),
                    });
                }
            }
            Ok(())
        })?;
    }
    let replaced: BTreeSet<_> = revisions
        .into_iter()
        .filter_map(|(id, key)| (identities.get(&id) == Some(&key)).then_some(id))
        .collect();
    sources.retain(|s| !replaced.contains(&s.id));
    let manifest = Manifest {
        epoch: epoch.clone(),
        files,
        sources,
    };
    let progress = Progress {
        epoch,
        ..Default::default()
    };
    let bytes = serde_json::to_vec(&manifest).map_err(|_| "local_event_manifest_invalid")?;
    if bytes.len() as u64 > MAX_JSON {
        return Err("local_event_manifest_limit".into());
    }
    if inventory(index.vault)?.0 != manifest.epoch {
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

/// 权限快照只在本进程中来自已验证正本；磁盘派生清单不能授予权限。
/// Unix 文件 ctime/identity 和规则清单改变即失效；其他平台先保守重新验证。
pub(super) fn verified_sources(
    index: &LocalEvents<'_>,
    epoch: &str,
) -> Result<std::sync::Arc<BTreeSet<String>>> {
    struct Verified {
        root: PathBuf,
        scope: String,
        epoch: String,
        ids: std::sync::Arc<BTreeSet<String>>,
    }
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Option<Verified>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(None));
    #[cfg(unix)]
    {
        let guard = cache
            .lock()
            .map_err(|_| "local_event_snapshot_unavailable")?;
        if let Some(v) = guard.as_ref().filter(|v| {
            v.root == index.vault.root() && v.scope == index.config.scope && v.epoch == epoch
        }) {
            return Ok(v.ids.clone());
        }
    }
    let suppressed = index.vault.suppressed_ids()?;
    let mut identities = BTreeMap::new();
    let mut revisions = Vec::new();
    let mut ids = BTreeSet::new();
    index.vault.visit_events(|event| {
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
        if eligible(&event, &index.config.scope, &suppressed)
            && !event.data.text().trim().is_empty()
        {
            ids.insert(event.id);
        }
        Ok(())
    })?;
    for (id, key) in revisions {
        if identities.get(&id) == Some(&key) {
            ids.remove(&id);
        }
    }
    if inventory(index.vault)?.0 != epoch {
        return Err("local_event_source_epoch_changed".into());
    }
    let ids = std::sync::Arc::new(ids);
    *cache
        .lock()
        .map_err(|_| "local_event_snapshot_unavailable")? = Some(Verified {
        root: index.vault.root().to_owned(),
        scope: index.config.scope.clone(),
        epoch: epoch.to_owned(),
        ids: ids.clone(),
    });
    Ok(ids)
}
