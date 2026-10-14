//! 真实 EventIndex + 一次性来源编码子进程；仅隔离副本注入合成传输与内存凭据。
use keyring::credential::{Credential, CredentialApi, CredentialBuilderApi, CredentialPersistence};
use recallcard::{
    application::{
        background_memory::MemoryProviderConfig,
        budget::BudgetMode,
        credentials::{self, CredentialStorage},
        embedding_cloud::*,
    },
    local_events::{CloudSetup, LocalEvents, Setup},
    Vault,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
type Store = Arc<Mutex<BTreeMap<String, Vec<u8>>>>;
struct Builder(Store, Arc<std::sync::atomic::AtomicUsize>);
struct Key {
    id: String,
    store: Store,
    reads: Arc<std::sync::atomic::AtomicUsize>,
}
impl CredentialBuilderApi for Builder {
    fn build(
        &self,
        target: Option<&str>,
        service: &str,
        user: &str,
    ) -> keyring::Result<Box<Credential>> {
        Ok(Box::new(Key {
            id: format!("{target:?}|{service}|{user}"),
            store: self.0.clone(),
            reads: self.1.clone(),
        }))
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn persistence(&self) -> CredentialPersistence {
        CredentialPersistence::ProcessOnly
    }
}
impl CredentialApi for Key {
    fn set_secret(&self, value: &[u8]) -> keyring::Result<()> {
        self.store
            .lock()
            .unwrap()
            .insert(self.id.clone(), value.to_vec());
        Ok(())
    }
    fn get_secret(&self) -> keyring::Result<Vec<u8>> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.store
            .lock()
            .unwrap()
            .get(&self.id)
            .cloned()
            .ok_or(keyring::Error::NoEntry)
    }
    fn delete_credential(&self) -> keyring::Result<()> {
        self.store.lock().unwrap().remove(&self.id);
        Ok(())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
fn calls(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn settings() -> SourceCloudSettings {
    SourceCloudSettings {
        endpoint: "https://synthetic-source.example.invalid/v1/embeddings".into(),
        model: "synthetic-source".into(),
        dimensions: 2,
        revision: "v1".into(),
        query_instruction: "查询".into(),
        document_instruction: "文档".into(),
        query_approved: true,
        events_approved: true,
        attachments_approved: true,
        budget_mode: BudgetMode::Limited,
        max_calls_per_month: 100,
        max_bytes_per_month: 1_048_576,
    }
}
fn setup() -> CloudSetup {
    CloudSetup {
        settings: settings(),
        tokenizer: "synthetic".into(),
        chunk_bytes: 256,
        disk_cap_mib: 16,
    }
}
fn wait_calls(log: &Path, count: usize) {
    let until = Instant::now() + Duration::from_secs(12);
    while calls(log).len() < count && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(calls(log).len(), count);
}
fn bounded_join<T>(pending: std::thread::JoinHandle<T>) -> T {
    let until = Instant::now() + Duration::from_secs(20);
    while !pending.is_finished() {
        assert!(Instant::now() < until, "合成异步来源编码没有在20秒内结束");
        std::thread::sleep(Duration::from_millis(10));
    }
    pending.join().unwrap()
}
fn refresh(index: &LocalEvents<'_>) {
    for _ in 0..10 {
        if index.refresh_step().unwrap()["complete"] == true {
            return;
        }
    }
    panic!("合成来源索引没有完成");
}
#[test]
fn source_host_encodes_real_events_attachments_and_cache_with_revocation_and_exact_body_budget() {
    let store = Arc::new(Mutex::new(BTreeMap::new()));
    let reads = Arc::new(AtomicUsize::new(0));
    keyring::set_default_credential_builder(Box::new(Builder(store.clone(), reads.clone())));
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let attachment = "合成附件正文 retained-source";
    let object = recallcard::model::hash(attachment.as_bytes());
    fs::write(vault.root().join("objects").join(&object), attachment).unwrap();
    let event = vault.capture(serde_json::from_value(json!({"scope":"personal","role":"user","origin":"user_input","content":"合成事件正文 original-source",
        "source":{"platform":"synthetic","conversation_id":"source-cloud","message_id":"one"},
        "metadata":{"source_assets":{"files":[{"payload_status":"retained","detected_media_type":"application/octet-stream","declared_media_type":"text/plain","object_sha256":object,"byte_count":attachment.len()}]}}
    })).unwrap()).unwrap();
    let python_path = dir.path().join("python");
    let worker = python_path.join("recallcard_worker");
    fs::create_dir_all(&worker).unwrap();
    for entry in
        fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../python/recallcard_worker"))
            .unwrap()
    {
        let entry = entry.unwrap();
        if entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "py")
        {
            fs::copy(entry.path(), worker.join(entry.file_name())).unwrap();
        }
    }
    let log = dir.path().join("mock-requests.jsonl");
    let hold = dir.path().join("hold");
    let release = dir.path().join("release");
    let failure = dir.path().join("failure");
    let injection = format!(
        r#"
# 合成测试的隔离安装副本；生产文件没有此注入点。
import json as _j, time as _t
from pathlib import Path as _P
from .embedding import TransportResponse
_log=_P({log:?});_hold=_P({hold:?});_release=_P({release:?});_failure=_P({failure:?})
def _fixture(endpoint,payload,key,timeout,limit):
    assert endpoint=='https://synthetic-source.example.invalid/v1/embeddings'
    assert key=='SYNTHETIC_SOURCE_ONLY'
    value=_j.loads(payload)
    with _log.open('a') as stream:stream.write(_j.dumps({{'request':value,'body_bytes':len(payload)}},ensure_ascii=False)+'\n')
    if _hold.exists():
        until=_t.monotonic()+10
        while not _release.exists() and _t.monotonic()<until:_t.sleep(.01)
    if _failure.exists():return TransportResponse(503,b'',{{}})
    return TransportResponse(200,canonical({{'model':value['model'],'data':[{{'index':i,'embedding':[1.,0.]}} for i,_ in enumerate(value['input'])]}}),{{}})
"#,
        log = log.to_string_lossy(),
        hold = hold.to_string_lossy(),
        release = release.to_string_lossy(),
        failure = failure.to_string_lossy()
    );
    let once = worker.join("source_cloud_once.py");
    let code = fs::read_to_string(&once)
        .unwrap()
        .replace("def run_once(", &(injection + "\ndef run_once("))
        .replace("transport=https_transport):", "transport=_fixture):");
    fs::write(once, code).unwrap();
    let event_index = worker.join("event_index.py");
    let code = fs::read_to_string(&event_index).unwrap().replace("SCHEMA = \"recallcard.local-event-index/1\"", "assert 'RECALLCARD_SOURCE_EMBEDDING_API_KEY' not in os.environ\nSCHEMA = \"recallcard.local-event-index/1\"");
    fs::write(event_index, code).unwrap();
    credentials::prepare_embedding(
        &vault,
        MemoryProviderConfig {
            endpoint: settings().endpoint,
            model: settings().model,
        },
        "SYNTHETIC_SOURCE_ONLY".into(),
        CredentialStorage::OsProtected,
    )
    .unwrap();
    let index = LocalEvents::configure_cloud(
        &vault,
        "personal",
        setup(),
        PathBuf::from("python3"),
        python_path.clone(),
    )
    .unwrap();
    assert!(index.is_cloud());
    let before = reads.load(Ordering::SeqCst);
    assert_eq!(index.status().unwrap()["provider"], "cloud");
    assert_eq!(reads.load(Ordering::SeqCst), before);
    assert!(calls(&log).is_empty());
    refresh(&index);
    assert_eq!(calls(&log).len(), 2);
    let sent = calls(&log)
        .into_iter()
        .flat_map(|row| row["request"]["input"].as_array().unwrap().clone())
        .collect::<Vec<_>>();
    assert!(sent.contains(&json!("文档\n合成事件正文 original-source")));
    assert!(sent.contains(&json!(format!("文档\n{attachment}"))));
    let page = index.search("合成", 10).unwrap();
    assert_eq!(page["coverage"]["cloud_event_semantic"], true);
    assert_eq!(page["coverage"]["local_event_semantic"], false);
    assert_eq!(page["coverage"]["semantic_query_encoding"], "cloud");
    assert!(page.to_string().contains(&event.id));
    let used = CloudEmbeddingRuntime::new(&vault)
        .source_status("personal")
        .unwrap()
        .usage;
    let before = reads.load(Ordering::SeqCst);
    assert_eq!(
        index.search("合成", 10).unwrap()["coverage"]["semantic_query_encoding"],
        "cache"
    );
    assert_eq!(reads.load(Ordering::SeqCst), before);
    assert_eq!(
        CloudEmbeddingRuntime::new(&vault)
            .source_status("personal")
            .unwrap()
            .usage,
        used
    );
    assert_eq!(used.reserved_calls, 3);
    assert_eq!(
        used.reserved_bytes,
        calls(&log)
            .iter()
            .map(|row| row["body_bytes"].as_u64().unwrap())
            .sum::<u64>()
    );
    assert_eq!(
        (
            used.event_calls,
            used.attachment_calls,
            used.source_query_calls
        ),
        (1, 1, 1)
    );
    // HTTP失败只调用一次，仍保留实际HTTP body字节预留，不发布成功结果。
    fs::write(&failure, b"1").unwrap();
    let failure_query = "失败\n中文\"引号\"\\路径\u{2028}尾";
    let context = recallcard::context::Context::new(
        &vault,
        recallcard::policy::Access::new(vec!["personal".into()]).unwrap(),
    );
    let failed = context
        .recall_search(serde_json::from_value(json!({"query":failure_query})).unwrap())
        .unwrap();
    assert_eq!(failed["coverage"]["event_search"], "explicit_cloud");
    assert_eq!(failed["coverage"]["cloud_event_semantic"], false);
    assert_eq!(failed["coverage"]["scopes"][0]["provider"], "cloud");
    assert_eq!(failed["coverage"]["scopes"][0]["state"], "unavailable");
    assert!(failed["coverage"]["scopes"][0]["code"]
        .as_str()
        .unwrap()
        .contains("source_cloud_provider_error"));
    assert_eq!(calls(&log).len(), 4);
    let usage = CloudEmbeddingRuntime::new(&vault)
        .source_status("personal")
        .unwrap()
        .usage;
    assert_eq!(usage.reserved_calls, 4);
    assert_eq!(
        usage.reserved_bytes,
        calls(&log)
            .iter()
            .map(|row| row["body_bytes"].as_u64().unwrap())
            .sum::<u64>()
    );
    fs::remove_file(&failure).unwrap();
    // 提供器等待期间撤销；旧结果不能返回或填入长期查询缓存。
    fs::write(&hold, b"1").unwrap();
    let root = vault.root().to_owned();
    let pending = std::thread::spawn(move || {
        let vault = Vault::open_existing(&root).unwrap();
        LocalEvents::open(&vault, "personal")
            .unwrap()
            .unwrap()
            .search("等待中撤销查询", 10)
    });
    wait_calls(&log, 5);
    LocalEvents::disable_cloud(&vault, "personal").unwrap();
    fs::write(&release, b"1").unwrap();
    assert!(bounded_join(pending).is_err());
    assert!(LocalEvents::open(&vault, "personal").unwrap().is_none());
    assert!(CloudEmbeddingRuntime::new(&vault)
        .source_status("personal")
        .unwrap()
        .grant
        .is_none());
    fs::remove_file(&hold).unwrap();
    let index = LocalEvents::configure_cloud(
        &vault,
        "personal",
        setup(),
        PathBuf::from("python3"),
        python_path.clone(),
    )
    .unwrap();
    // 新grant revision改变附件来源epoch，重建快照仅复用已批准的相同空间向量。
    refresh(&index);
    assert_eq!(
        index.search("等待中撤销查询", 10).unwrap()["coverage"]["semantic_query_encoding"],
        "cloud"
    );
    assert_eq!(calls(&log).len(), 6);
    // 等待期间附件原件改变也使整个来源快照失效，不能返回先前读取的候选。
    fs::remove_file(&release).unwrap();
    fs::write(&hold, b"1").unwrap();
    let root = vault.root().to_owned();
    let pending = std::thread::spawn(move || {
        let vault = Vault::open_existing(&root).unwrap();
        LocalEvents::open(&vault, "personal")
            .unwrap()
            .unwrap()
            .search("等待中附件变化", 10)
    });
    wait_calls(&log, 7);
    fs::write(
        vault.root().join("objects").join(&object),
        vec![b'x'; attachment.len()],
    )
    .unwrap();
    fs::write(&release, b"1").unwrap();
    assert!(bounded_join(pending).is_err());
    assert_eq!(index.status().unwrap()["index_state"], "stale");
    // 来源云停用不能关闭之后选择的numeric-loopback本机索引。
    let local = Setup {
        endpoint: "http://127.0.0.1:9999/v1/embeddings".into(),
        model: "local".into(),
        revision: "v1".into(),
        tokenizer: "synthetic".into(),
        dimensions: 2,
        query_instruction: String::new(),
        document_instruction: String::new(),
        events_approved: true,
        attachments_approved: false,
        query_approved: true,
        chunk_bytes: 256,
        disk_cap_mib: 16,
    };
    LocalEvents::configure(
        &vault,
        "personal",
        local,
        PathBuf::from("python3"),
        python_path,
    )
    .unwrap();
    LocalEvents::disable_cloud(&vault, "personal").unwrap();
    assert!(!LocalEvents::open(&vault, "personal")
        .unwrap()
        .unwrap()
        .is_cloud());
    for entry in fs::read_dir(vault.state_dir().unwrap().join("local-event-semantic")).unwrap() {
        let current = entry.unwrap().path().join("current.json");
        assert!(!fs::read_to_string(current)
            .unwrap()
            .contains("SYNTHETIC_SOURCE_ONLY"));
    }
}
