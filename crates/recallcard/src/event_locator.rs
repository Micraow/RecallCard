//! 可丢弃的本机路径提示；从不缓存授权、抑制或 Event 正文。
//! Unix ctime/inode/mtime/长度及完整文件清单决定是否重建。目标段每次仍读正本并校验摘要。
use crate::{
    filesystem::open_local_file, model::*, vault::files_recursive,
    vault_stream::read_event_segment, Vault,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    os::unix::fs::MetadataExt,
    path::PathBuf,
};
const MAX_CACHE_BYTES: u64 = 16 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cache {
    version: u8,
    root: PathBuf,
    inventory: String,
    paths: BTreeMap<String, PathBuf>,
}
pub(crate) struct Locator {
    paths: BTreeMap<String, PathBuf>,
    loaded: BTreeMap<String, Event>,
}
impl Locator {
    pub(crate) fn open(vault: &Vault) -> Result<Self> {
        let files = files_recursive(&vault.root().join("events"), "jsonl")?;
        let mut inventory = Vec::new();
        for path in &files {
            let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
            if !meta.is_file() {
                return Err("事件正本不是普通文件".into());
            }
            inventory.push((
                path.clone(),
                meta.dev(),
                meta.ino(),
                meta.len(),
                meta.mtime(),
                meta.mtime_nsec(),
                meta.ctime(),
                meta.ctime_nsec(),
            ));
        }
        let signature = hash(&serde_json::to_vec(&inventory).map_err(|e| e.to_string())?);
        let cache_path = vault.state_dir()?.join("event-locator-v1.json");
        let cached = (|| -> Option<Cache> {
            let file = open_local_file(&cache_path).ok()?;
            let meta = file.metadata().ok()?;
            if !meta.is_file() || meta.len() > MAX_CACHE_BYTES {
                return None;
            }
            let mut bytes = Vec::new();
            file.take(MAX_CACHE_BYTES + 1)
                .read_to_end(&mut bytes)
                .ok()?;
            if bytes.len() as u64 > MAX_CACHE_BYTES {
                return None;
            }
            serde_json::from_slice(&bytes).ok()
        })();
        let file_set = files.iter().collect::<BTreeSet<_>>();
        if let Some(cache) = cached.filter(|cache| {
            cache.version == 1
                && cache.root == vault.root()
                && cache.inventory == signature
                && cache.paths.keys().all(|id| validate_id(id, "evt_").is_ok())
                && cache.paths.values().all(|path| file_set.contains(path))
        }) {
            return Ok(Self {
                paths: cache.paths,
                loaded: BTreeMap::new(),
            });
        }
        let mut paths = BTreeMap::new();
        for path in files {
            read_event_segment(&path, |event| {
                if paths.insert(event.id, path.clone()).is_some() {
                    return Err("事件编号重复".into());
                }
                Ok(())
            })?;
        }
        let cache = Cache {
            version: 1,
            root: vault.root().to_owned(),
            inventory: signature,
            paths,
        };
        if let Ok(bytes) = serde_json::to_vec(&cache) {
            if bytes.len() as u64 <= MAX_CACHE_BYTES {
                // 派生缓存写入失败不影响正本读取；原子替换避免并发读看到半个提示文件。
                let _ = vault.write_bytes(&cache_path, &bytes);
            }
        }
        Ok(Self {
            paths: cache.paths,
            loaded: BTreeMap::new(),
        })
    }
    pub(crate) fn event(&mut self, vault: &Vault, id: &str) -> Result<Event> {
        if let Some(event) = self.loaded.get(id) {
            return Ok(event.clone());
        }
        if let Some(path) = self.paths.get(id) {
            let mut target = None;
            let mut ids = BTreeSet::new();
            read_event_segment(path, |event| {
                if !ids.insert(event.id.clone()) {
                    return Err("事件编号重复".into());
                }
                if event.id == id {
                    target = Some(event);
                }
                Ok(())
            })?;
            if let Some(event) = target {
                self.loaded.insert(id.to_owned(), event.clone());
                return Ok(event);
            }
        }
        // 缺失或错误提示不能给出“无资料”结论；回退到全量正本验证。
        let events = vault.events()?;
        let event = events
            .into_iter()
            .find(|event| event.id == id)
            .ok_or_else(|| format!("找不到原始事件 {id}"))?;
        self.loaded.insert(id.to_owned(), event.clone());
        Ok(event)
    }
}
