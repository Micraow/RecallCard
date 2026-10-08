//! 一次建索引的有界事件写入器。每个块先写恢复日志，再公布块收据。
//! 未完成事务使普通读取失败关闭；恢复可重放同一块，不能露出半块成果。
use crate::{
    filesystem::{file_identity, open_local_file, FileIdentity},
    model::*,
    vault::{files_recursive, read_json, reject_symlink, sync_parent, WriteGuard},
    Vault,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufRead, BufReader, Read},
    path::PathBuf,
};

const CHUNK_BYTES: usize = 16 * 1024 * 1024;
const MAX_EVENT_LINE: usize = 4 * 1024 * 1024 + 1024;
const JOURNAL: &str = "event-transaction.json";
const MAX_JOURNAL_BYTES: usize = CHUNK_BYTES + 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChunkReceipt {
    pub key: String,
    pub input_hash: String,
    pub events_processed: u64,
    pub events_added: u64,
    pub events_duplicates: u64,
    pub refs: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EventTransaction {
    version: u32,
    root: PathBuf,
    marker: FileIdentity,
    receipt: ChunkReceipt,
    events: Vec<Event>,
}

pub struct EventStreamWriter<'a> {
    vault: &'a Vault,
    _guard: WriteGuard,
    versions: BTreeMap<String, String>,
    latest: BTreeMap<String, (DateTime<Utc>, String)>,
    known: BTreeSet<String>,
    indexed_events: usize,
    identities: BTreeMap<String, String>,
}
impl Vault {
    /// 不保证时间顺序；回调一次只持有一条正本。调用方跨多次读取需持 read_guard。
    pub fn visit_events<F>(&self, visit: F) -> Result<()>
    where
        F: FnMut(Event) -> Result<()>,
    {
        self.ensure_no_pending_dream()?;
        self.visit_events_unchecked(visit)
    }
    /// 仅供已持有 Vault 写锁的内部事务恢复路径；普通读取必须调用 visit_events。
    pub(crate) fn visit_events_unchecked<F>(&self, mut visit: F) -> Result<()>
    where
        F: FnMut(Event) -> Result<()>,
    {
        let mut ids = BTreeSet::new();
        for path in files_recursive(&self.root().join("events"), "jsonl")? {
            let file = open_local_file(&path)?;
            if !file.metadata().map_err(|e| e.to_string())?.is_file() {
                return Err("事件正本不是普通文件".into());
            }
            let mut reader = BufReader::new(file);
            loop {
                let mut bytes = Vec::new();
                let size = reader
                    .by_ref()
                    .take(MAX_EVENT_LINE as u64 + 1)
                    .read_until(b'\n', &mut bytes)
                    .map_err(|e| e.to_string())?;
                if size == 0 {
                    break;
                }
                if size > MAX_EVENT_LINE || bytes.last() != Some(&b'\n') || size == 1 {
                    return Err("事件段不完整或单条正本超过安全上限".into());
                }
                let event: Event =
                    serde_json::from_slice(&bytes).map_err(|_| "事件正本损坏或存在 Git 冲突")?;
                event.validate()?;
                if !ids.insert(event.id.clone()) {
                    return Err("事件编号重复".into());
                }
                visit(event)?;
            }
        }
        Ok(())
    }
}
fn version_key(input: &EventInput) -> Result<String> {
    let mut normalized = input.clone();
    normalized.revision_of = None;
    normalized.id()
}
impl<'a> EventStreamWriter<'a> {
    pub fn open(vault: &'a Vault) -> Result<Self> {
        let guard = vault.lock_unchecked()?;
        // 同一个 writer 不跨越另一类尚未完成的事务。
        let dream = vault.state_dir()?.join("dream-transaction.json");
        reject_symlink(&dream)?;
        if dream.exists() {
            return Err("存在未完成的 Dream 事务，请先恢复".into());
        }
        recover(vault)?;
        let mut writer = Self {
            vault,
            _guard: guard,
            versions: BTreeMap::new(),
            latest: BTreeMap::new(),
            known: BTreeSet::new(),
            indexed_events: 0,
            identities: BTreeMap::new(),
        };
        vault.visit_events(|event| {
            writer.remember(&event)?;
            writer.indexed_events += 1;
            Ok(())
        })?;
        Ok(writer)
    }
    /// 诊断计数：一个 writer 生命周期只遍历一次已有正本，不因块数增长重扫。
    pub fn indexed_events(&self) -> usize {
        self.indexed_events
    }
    fn remember(&mut self, event: &Event) -> Result<()> {
        self.versions
            .entry(version_key(&event.data)?)
            .or_insert_with(|| event.id.clone());
        let source = event.data.revision_key();
        let version = (event.captured_at, event.id.clone());
        if self.latest.get(&source).is_none_or(|old| old < &version) {
            self.latest.insert(source.clone(), version);
        }
        self.known.insert(event.id.clone());
        self.identities.insert(event.id.clone(), source);
        Ok(())
    }
    pub fn commit_chunk(&mut self, key: &str, mut inputs: Vec<EventInput>) -> Result<ChunkReceipt> {
        if key.is_empty()
            || key.len() > 256
            || key.chars().any(char::is_control)
            || inputs.is_empty()
            || inputs.len() > 64
        {
            return Err("事件提交块的身份或数量无效".into());
        }
        for input in &mut inputs {
            crate::capture::redact_event(input)?;
            input.validate()?;
        }
        let bytes = serde_json::to_vec(&inputs).map_err(|e| e.to_string())?;
        if bytes.len() > CHUNK_BYTES {
            return Err("事件提交块超过 16 MiB".into());
        }
        let input_hash = hash(&bytes);
        drop(bytes);
        let receipt_path = receipt_path(self.vault, key)?;
        if receipt_path.exists() {
            let old: ChunkReceipt = read_json(&receipt_path)?;
            if old.key != key
                || old.input_hash != input_hash
                || old.refs.len() != inputs.len()
                || old.events_processed != inputs.len() as u64
                || old.events_added + old.events_duplicates != old.events_processed
                || old.refs.iter().any(|id| !self.known.contains(id))
            {
                return Err("已提交块的身份、内容或正本已改变".into());
            }
            return Ok(old);
        }
        let mut versions = BTreeMap::new();
        let mut latest = BTreeMap::new();
        let mut identities = BTreeMap::new();
        let mut events = Vec::new();
        let mut refs = Vec::with_capacity(inputs.len());
        for mut input in inputs {
            let version = version_key(&input)?;
            if let Some(id) = versions
                .get(&version)
                .or_else(|| self.versions.get(&version))
            {
                refs.push(id.clone());
                continue;
            }
            let source = input.revision_key();
            if input.revision_of.is_none() {
                input.revision_of = latest
                    .get(&source)
                    .or_else(|| self.latest.get(&source))
                    .map(|(_, id)| id.clone());
            }
            if input.revision_of.as_ref().is_some_and(|id| {
                identities.get(id).or_else(|| self.identities.get(id)) != Some(&source)
            }) {
                return Err("修订必须引用同一资料范围内同一来源的已有事件；本块尚未写入".into());
            }
            let event = Event {
                schema_version: SCHEMA_VERSION,
                id: input.id()?,
                captured_at: Utc::now(),
                data: input,
            };
            versions.insert(version, event.id.clone());
            latest.insert(source.clone(), (event.captured_at, event.id.clone()));
            identities.insert(event.id.clone(), source);
            refs.push(event.id.clone());
            events.push(event);
        }
        let receipt = ChunkReceipt {
            key: key.into(),
            input_hash,
            events_processed: refs.len() as u64,
            events_added: events.len() as u64,
            events_duplicates: (refs.len() - events.len()) as u64,
            refs,
        };
        let transaction = EventTransaction {
            version: 1,
            root: self.vault.root().into(),
            marker: file_identity(&open_local_file(
                &self.vault.root().join("control/schema-version.json"),
            )?)?,
            receipt: receipt.clone(),
            events,
        };
        let journal = self.vault.state_dir()?.join(JOURNAL);
        write_transaction(self.vault, &journal, &transaction)?;
        finish(self.vault, &transaction)?;
        for event in &transaction.events {
            self.remember(event)?;
        }
        Ok(receipt)
    }
}
fn write_transaction(
    vault: &Vault,
    path: &std::path::Path,
    transaction: &EventTransaction,
) -> Result<()> {
    // Pretty 缩进能把合法嵌套元数据放大数百倍；日志必须紧凑且在提交前可恢复。
    let bytes = serde_json::to_vec(transaction).map_err(|_| "事件事务无法编码")?;
    if bytes.len() > MAX_JOURNAL_BYTES {
        return Err("事件事务超过恢复字节上限；本块尚未写入".into());
    }
    let _: EventTransaction =
        serde_json::from_slice(&bytes).map_err(|_| "事件事务嵌套超过恢复上限；本块尚未写入")?;
    vault
        .staged(path, &bytes)?
        .persist_noclobber(path)
        .map_err(|e| e.to_string())?;
    sync_parent(path)
}
fn receipt_path(vault: &Vault, key: &str) -> Result<PathBuf> {
    let directory = vault.state_dir()?.join("event-receipts");
    reject_symlink(&directory)?;
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    Ok(directory.join(format!("{}.json", hash(key.as_bytes()))))
}
fn recover(vault: &Vault) -> Result<()> {
    let journal = vault.state_dir()?.join(JOURNAL);
    reject_symlink(&journal)?;
    if !journal.exists() {
        return Ok(());
    }
    let metadata = fs::metadata(&journal).map_err(|e| e.to_string())?;
    // 兼容此前小型 pretty 日志；新写入严格限制为紧凑、已预先解码的日志。
    if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 {
        return Err("事件事务日志超过上限或不是普通文件".into());
    }
    let file = open_local_file(&journal)?;
    let transaction: EventTransaction =
        serde_json::from_reader(file).map_err(|_| "事件事务日志无效")?;
    finish(vault, &transaction)
}
fn finish(vault: &Vault, transaction: &EventTransaction) -> Result<()> {
    let marker = file_identity(&open_local_file(
        &vault.root().join("control/schema-version.json"),
    )?)?;
    if transaction.version != 1
        || transaction.root != vault.root()
        || transaction.marker != marker
        || transaction.events.len() > 64
        || transaction.receipt.refs.len() > 64
        || transaction.receipt.events_added != transaction.events.len() as u64
        || transaction.receipt.events_processed != transaction.receipt.refs.len() as u64
        || transaction.receipt.events_added + transaction.receipt.events_duplicates
            != transaction.receipt.events_processed
    {
        return Err("事件事务不属于当前资料库或计数无效".into());
    }
    let mut unique = BTreeSet::new();
    for event in &transaction.events {
        event.validate()?;
        if !unique.insert(&event.id) || !transaction.receipt.refs.contains(&event.id) {
            return Err("事件事务含重复或未知正本".into());
        }
    }
    for event in &transaction.events {
        let path = vault.event_path(event);
        reject_symlink(&path)?;
        if path.exists() {
            let existing: Event = read_json(&path)?;
            if &existing != event {
                return Err("恢复目标存在不同内容；不会覆盖正本".into());
            }
            continue;
        }
        fs::create_dir_all(path.parent().ok_or("事件路径无父目录")?).map_err(|e| e.to_string())?;
        let mut bytes = serde_json::to_vec(event).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        vault
            .staged(&path, &bytes)?
            .persist_noclobber(&path)
            .map_err(|e| e.to_string())?;
        sync_parent(&path)?;
    }
    let path = receipt_path(vault, &transaction.receipt.key)?;
    if path.exists() {
        let existing: ChunkReceipt = read_json(&path)?;
        if existing != transaction.receipt {
            return Err("事件块收据冲突".into());
        }
    } else {
        vault.write_new(&path, &transaction.receipt)?;
    }
    let journal = vault.state_dir()?.join(JOURNAL);
    fs::remove_file(&journal).map_err(|e| e.to_string())?;
    sync_parent(&journal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn input(id: &str) -> EventInput {
        serde_json::from_value(json!({"role":"user","origin":"user_input","scope":"personal","content":"合成事务","source":{"platform":"synthetic","conversation_id":"transaction","message_id":id}})).unwrap()
    }
    #[test]
    fn crash_between_event_files_blocks_read_until_exact_chunk_is_recovered() {
        let directory = tempfile::tempdir().unwrap();
        let vault = Vault::init(&directory.path().join("vault")).unwrap();
        let inputs = vec![input("first"), input("second")];
        let events: Vec<_> = inputs
            .iter()
            .cloned()
            .map(|data| Event {
                schema_version: SCHEMA_VERSION,
                id: data.id().unwrap(),
                captured_at: Utc::now(),
                data,
            })
            .collect();
        let receipt = ChunkReceipt {
            key: "crashed-block".into(),
            input_hash: hash(&serde_json::to_vec(&inputs).unwrap()),
            events_processed: 2,
            events_added: 2,
            events_duplicates: 0,
            refs: events.iter().map(|e| e.id.clone()).collect(),
        };
        let transaction = EventTransaction {
            version: 1,
            root: vault.root().into(),
            marker: file_identity(
                &open_local_file(&vault.root().join("control/schema-version.json")).unwrap(),
            )
            .unwrap(),
            receipt: receipt.clone(),
            events: events.clone(),
        };
        vault
            .write_new(&vault.state_dir().unwrap().join(JOURNAL), &transaction)
            .unwrap();
        let path = vault.event_path(&events[0]);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut bytes = serde_json::to_vec(&events[0]).unwrap();
        bytes.push(b'\n');
        vault.write_bytes(&path, &bytes).unwrap();
        assert!(vault.events().is_err());
        assert!(vault.read_guard().is_err());
        let mut writer = EventStreamWriter::open(&vault).unwrap();
        assert_eq!(writer.indexed_events(), 2);
        assert_eq!(
            writer.commit_chunk("crashed-block", inputs).unwrap(),
            receipt
        );
        drop(writer);
        assert_eq!(vault.events().unwrap(), events);
        assert!(!vault.state_dir().unwrap().join(JOURNAL).exists());
    }
    #[test]
    fn deeply_nested_metadata_journal_stays_compact_and_recovers() {
        let directory = tempfile::tempdir().unwrap();
        let vault = Vault::init(&directory.path().join("vault")).unwrap();
        let mut data = input("nested");
        let mut citations = json!(vec![0; 400_000]);
        for _ in 0..80 {
            citations = json!({"n":citations});
        }
        data.metadata = json!({"citations":citations});
        let event = Event {
            schema_version: SCHEMA_VERSION,
            id: data.id().unwrap(),
            captured_at: Utc::now(),
            data: data.clone(),
        };
        let transaction = EventTransaction {
            version: 1,
            root: vault.root().into(),
            marker: file_identity(
                &open_local_file(&vault.root().join("control/schema-version.json")).unwrap(),
            )
            .unwrap(),
            receipt: ChunkReceipt {
                key: "nested".into(),
                input_hash: hash(&serde_json::to_vec(&vec![data]).unwrap()),
                events_processed: 1,
                events_added: 1,
                events_duplicates: 0,
                refs: vec![event.id.clone()],
            },
            events: vec![event],
        };
        let journal = vault.state_dir().unwrap().join(JOURNAL);
        write_transaction(&vault, &journal, &transaction).unwrap();
        assert!(fs::metadata(&journal).unwrap().len() < 1024 * 1024);
        assert!(vault.events().is_err());
        drop(EventStreamWriter::open(&vault).unwrap());
        assert_eq!(vault.events().unwrap().len(), 1);
        assert!(!journal.exists());
    }
    #[test]
    fn chunk_validation_failure_leaves_no_journal_or_partial_event() {
        let directory = tempfile::tempdir().unwrap();
        let vault = Vault::init(&directory.path().join("vault")).unwrap();
        let mut bad = input("bad");
        bad.revision_of = Some(format!("evt_{}", "f".repeat(64)));
        let mut writer = EventStreamWriter::open(&vault).unwrap();
        assert!(writer
            .commit_chunk("bad", vec![input("good"), bad])
            .is_err());
        drop(writer);
        assert!(vault.events().unwrap().is_empty());
        assert!(!vault.state_dir().unwrap().join(JOURNAL).exists());
    }
}
