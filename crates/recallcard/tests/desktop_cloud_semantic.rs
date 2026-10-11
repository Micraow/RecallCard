//! 合成 OS-store adapter + 真实 Python 子进程/注入传输；没有真实 keyring/云请求。
use keyring::credential::{Credential, CredentialApi, CredentialBuilderApi, CredentialPersistence};
use recallcard::{
    application::{
        background_memory::*, budget::BudgetMode, credentials, embedding_cloud::*, runtime,
        AppResult,
    },
    desktop::{CloudSemanticSetup, DesktopSession},
    semantic::SemanticSearch,
    Vault,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
type Store = Arc<Mutex<BTreeMap<String, Vec<u8>>>>;
struct Builder(Store);
struct Key {
    id: String,
    store: Store,
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
struct NoLlm;
impl MemoryProvider for NoLlm {
    fn execute(
        &mut self,
        _: &MemoryConfig,
        _: &recallcard::dream::DreamJob,
    ) -> AppResult<ProviderOutput> {
        panic!("云向量不应启动整理 LLM")
    }
}
fn calls(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}
const EXT: &str = "abcdefghijklmnopabcdefghijklmnop";
const INSTALL: &str = "11111111-1111-4111-8111-111111111111";
struct Native(std::process::Child);
impl Native {
    fn read(&mut self, id: &str, action: &str, args: Value) -> Value {
        use std::io::{Read, Write};
        let request = json!({"protocol":"recallcard.action/1","request_id":id,"nonce":"cloud-semantic-native-synthetic-nonce","session_ref":"chatgpt:cloud-synthetic","installation_id":INSTALL,"action":"authorized_read","arguments":{"action":action,"arguments":args,"permission_revision":1}});
        let bytes = serde_json::to_vec(&request).unwrap();
        let input = self.0.stdin.as_mut().unwrap();
        input
            .write_all(&(bytes.len() as u32).to_ne_bytes())
            .unwrap();
        input.write_all(&bytes).unwrap();
        input.flush().unwrap();
        let out = self.0.stdout.as_mut().unwrap();
        let mut prefix = [0; 4];
        out.read_exact(&mut prefix).unwrap();
        let length = u32::from_ne_bytes(prefix) as usize;
        assert!(length < 1024 * 1024);
        let mut bytes = vec![0; length];
        out.read_exact(&mut bytes).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn settings() -> CloudEmbeddingSettings {
    CloudEmbeddingSettings {
        endpoint: "https://synthetic-vector.example.invalid/v1/embeddings".into(),
        model: "synthetic-vector".into(),
        dimensions: 2,
        revision: "v1".into(),
        query_instruction: "查找".into(),
        document_instruction: "记忆".into(),
        query_approved: true,
        memory_approved: true,
        budget_mode: BudgetMode::Unlimited,
        max_calls_per_month: 1,
        max_bytes_per_month: 1,
    }
}
#[test]
fn ordinary_setup_indexes_and_queries_with_persistent_budget_and_live_revoke_through_real_worker() {
    let store = Arc::new(Mutex::new(BTreeMap::new()));
    keyring::set_default_credential_builder(Box::new(Builder(store.clone())));
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let source=vault.capture(serde_json::from_value(json!({"role":"user","scope":"personal","origin":"native","content":"合成这次徒步半天","source":{"platform":"synthetic","conversation_id":"cloud","message_id":"one"}})).unwrap()).unwrap();
    let memory=vault.add_memory(serde_json::from_value(json!({"content":"合成本次半日徒步约束","scope":"personal","source_refs":[source.id],"evidence":"user_explicit","navigation":[{"path":"topics/hiking"}]})).unwrap()).unwrap();
    let runtime_state = MemoryRuntime::new(&vault);
    let mut cfg = runtime_state.configuration().unwrap();
    cfg.paused = true;
    runtime_state.configure(cfg).unwrap();
    let resources = dir.path().join("resources");
    let component = resources.join("说明与许可证");
    let worker = component.join("python/recallcard_worker");
    fs::create_dir_all(&worker).unwrap();
    let binary = component.join(if cfg!(windows) {
        "recallcard.exe"
    } else {
        "recallcard"
    });
    fs::hard_link(env!("CARGO_BIN_EXE_recallcard"), &binary).unwrap();
    for entry in
        fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../python/recallcard_worker"))
            .unwrap()
    {
        let entry = entry.unwrap();
        if entry.path().extension().is_some_and(|e| e == "py") {
            fs::copy(entry.path(), worker.join(entry.file_name())).unwrap();
        }
    }
    let log = dir.path().join("mock-requests.jsonl");
    let hold = dir.path().join("hold");
    let release = dir.path().join("release");
    let inject = format!(
        r#"
# 此段仅由合成测试写入隔离安装副本，不进入生产 worker。
import json as _j, time as _t
from pathlib import Path as _P
from .embedding import TransportResponse, canonical
_log=_P({log:?});_hold=_P({hold:?});_release=_P({release:?})
def _fixture(endpoint,payload,key,timeout,limit):
    assert endpoint=='https://synthetic-vector.example.invalid/v1/embeddings'
    assert key=='SYNTHETIC_VECTOR_ONLY'
    value=_j.loads(payload)
    with _log.open('a') as stream:stream.write(_j.dumps({{'request':value,'body_bytes':len(payload)}},ensure_ascii=False)+'\n')
    if _hold.exists():
        until=_t.monotonic()+10
        while not _release.exists() and _t.monotonic()<until:_t.sleep(.01)
    return TransportResponse(200,canonical({{'model':value['model'],'data':[{{'index':i,'embedding':[1.,0.]}} for i,_ in enumerate(value['input'])]}}),{{}})
_original=ManagedCloudClient.__init__
def _init(self,approval,**kw):
    kw['transport']=_fixture
    _original(self,approval,**kw)
ManagedCloudClient.__init__=_init
"#,
        log = log.to_string_lossy(),
        hold = hold.to_string_lossy(),
        release = release.to_string_lossy()
    );
    use std::io::Write;
    fs::OpenOptions::new()
        .append(true)
        .open(worker.join("managed_cloud.py"))
        .unwrap()
        .write_all(inject.as_bytes())
        .unwrap();
    let binary = runtime::register_packaged_cli(&binary, &resources).unwrap();
    let mut desktop = DesktopSession::default();
    let session = desktop.select_vault(vault.root(), false).unwrap();
    let operation = desktop
        .model_operation(&session.session_id, "personal")
        .unwrap();
    let request = CloudSemanticSetup {
        settings: settings(),
        save_in_os: true,
        resume_background: false,
    };
    let mut not_saved = request.clone();
    not_saved.save_in_os = false;
    assert!(operation
        .configure_cloud_semantic(
            not_saved,
            Some("SYNTHETIC_VECTOR_ONLY".into()),
            Some(&binary)
        )
        .is_err());
    assert!(store.lock().unwrap().is_empty());
    let configured = operation
        .configure_cloud_semantic(
            request.clone(),
            Some("SYNTHETIC_VECTOR_ONLY".into()),
            Some(&binary),
        )
        .unwrap();
    assert_eq!(configured.state, "configured");
    assert!(configured.paused);
    assert_eq!(calls(&log).len(), 0);
    let cfg = runtime_state.configuration().unwrap();
    assert!(!cfg.enabled);
    assert!(cfg.provider.is_none());
    let path = cfg.candidate_semantic_config.clone().unwrap();
    assert!(!fs::read_to_string(&path)
        .unwrap()
        .contains("SYNTHETIC_VECTOR_ONLY"));
    assert!(SemanticSearch::from_offline_config_file(&path).is_err());
    assert!(SemanticSearch::from_application_config_file(&vault, &path, &["work".into()]).is_err());
    let mut cfg = cfg;
    cfg.paused = false;
    runtime_state.configure(cfg).unwrap();
    runtime_state.tick(&mut NoLlm).unwrap();
    let state = runtime_state.status().unwrap().index_refresh.unwrap();
    assert_eq!(state.state, "ready", "{state:?}");
    assert_eq!(calls(&log).len(), 1);
    assert_eq!(
        calls(&log)[0]["request"]["input"],
        json!(["记忆\n合成本次半日徒步约束"])
    );
    let result = operation.search("walking constraints", "memories").unwrap();
    assert_eq!(
        result["coverage"]["semantic_query_encoding"], "cloud_encoded",
        "{result}"
    );
    assert!(result.to_string().contains(&memory.id));
    assert_eq!(calls(&log).len(), 2);
    assert_eq!(
        calls(&log)[1]["request"]["input"],
        json!(["查找\nwalking constraints"])
    );
    let fresh = desktop
        .model_operation(&session.session_id, "personal")
        .unwrap();
    let cached = fresh.search("walking constraints", "memories").unwrap();
    assert_eq!(cached["coverage"]["semantic_query_encoding"], "cached");
    assert_eq!(calls(&log).len(), 2);
    // 真实 Native 子进程走 managed 配置和已缓存向量；不触碰真实 OS keyring。
    let installed = desktop
        .prepare_browser_connection(
            &session.session_id,
            "personal",
            EXT,
            &dir.path().join("native"),
            Path::new(env!("CARGO_BIN_EXE_recallcard")),
            false,
        )
        .unwrap();
    recallcard::application::connections::configure(
        &vault,
        recallcard::application::connections::ConnectionGrant {
            client_kind: "browser".into(),
            host_identity: EXT.into(),
            installation_id: Some(INSTALL.into()),
            platform: "chatgpt".into(),
            recall_scopes: vec!["personal".into()],
            capture_scopes: vec![],
            provider_disclosure: true,
            auto_capture: false,
            auto_recall: true,
        },
        None,
    )
    .unwrap();
    let mut native = Native(
        std::process::Command::new(installed["launcher"].as_str().unwrap())
            .arg(format!("chrome-extension://{EXT}/"))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let found = native.read(
        "find",
        "search",
        json!({"query":"walking constraints","target":"views","budget_bytes":8192}),
    );
    assert_eq!(found["ok"], true, "{found}");
    assert_eq!(
        found["result"]["coverage"]["semantic_query_encoding"],
        "cached"
    );
    let view = found["result"]["results"][0]["ref"].clone();
    let page = native.read(
        "directory",
        "read",
        json!({"refs":[view],"budget_bytes":8192}),
    );
    let reference = page["result"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "memory")
        .unwrap()["ref"]
        .clone();
    let original = native.read(
        "source",
        "sources",
        json!({"refs":[reference],"budget_bytes":8192}),
    );
    assert_eq!(
        original["result"]["results"][0]["events"][0]["id"],
        source.id
    );
    assert_eq!(calls(&log).len(), 2);
    let usage = operation.cloud_semantic_status().unwrap().cloud.usage;
    assert_eq!(usage.reserved_calls, 2);
    assert_eq!(
        usage.reserved_bytes,
        calls(&log)
            .iter()
            .map(|v| v["body_bytes"].as_u64().unwrap())
            .sum::<u64>()
    );
    let mut finite = request.clone();
    finite.settings.budget_mode = BudgetMode::Limited;
    finite.settings.max_calls_per_month = 2;
    finite.settings.max_bytes_per_month = 1_000_000;
    // 保持暂停，普通配置动作不会为此测试创建真实 OS-keyring 后台子进程。
    let mut cfg = runtime_state.configuration().unwrap();
    cfg.paused = true;
    runtime_state.configure(cfg).unwrap();
    let target = MemoryProviderConfig {
        endpoint: settings().endpoint,
        model: settings().model,
    };
    credentials::forget_embedding_os(&vault, &target).unwrap();
    assert_eq!(
        operation
            .search("uncached without key", "memories")
            .unwrap()["coverage"]["semantic_error"],
        "key_unavailable"
    );
    assert_eq!(
        operation.search("walking constraints", "memories").unwrap()["coverage"]
            ["semantic_query_encoding"],
        "cached"
    );
    assert_eq!(calls(&log).len(), 2);
    operation
        .configure_cloud_semantic(
            request.clone(),
            Some("SYNTHETIC_VECTOR_ONLY".into()),
            Some(&binary),
        )
        .unwrap();
    operation
        .configure_cloud_semantic(finite, None, Some(&binary))
        .unwrap();
    let exhausted = operation.search("another wording", "memories").unwrap();
    assert_eq!(
        exhausted["coverage"]["semantic_error"],
        "network_budget_exhausted"
    );
    assert_eq!(calls(&log).len(), 2);
    operation
        .configure_cloud_semantic(request.clone(), None, Some(&binary))
        .unwrap();
    fs::write(&hold, b"hold").unwrap();
    let reader = desktop
        .model_operation(&session.session_id, "personal")
        .unwrap();
    let task =
        std::thread::spawn(move || reader.search("new query waiting for revocation", "memories"));
    let until = Instant::now() + Duration::from_secs(10);
    while calls(&log).len() < 3 {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    CloudEmbeddingRuntime::new(&vault)
        .revoke("personal")
        .unwrap();
    fs::write(&release, b"released").unwrap();
    let rejected = task.join().unwrap().unwrap();
    assert_eq!(
        rejected["coverage"]["semantic_error"],
        "cloud_approval_changed"
    );
    let cfg: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let cache: Value = serde_json::from_slice(
        &fs::read(cfg["config"]["query_cache_path"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(cache["queries"].as_array().unwrap().len(), 1);
    assert_eq!(
        CloudEmbeddingRuntime::new(&vault)
            .status("personal")
            .unwrap()
            .usage
            .reserved_calls,
        3
    );
    // LLM 同目标凭据不会被读取：向量身份/命名空间独立。
    let target = MemoryProviderConfig {
        endpoint: settings().endpoint,
        model: settings().model,
    };
    assert!(credentials::load_os(&vault, &target).unwrap().is_none());
    operation
        .configure_cloud_semantic(request.clone(), None, Some(&binary))
        .unwrap();
    fs::remove_file(&hold).unwrap();
    fs::remove_file(&release).unwrap();
    vault.suppress(&source.id, "合成来源撤回".into()).unwrap();
    let suppressed = operation
        .search("new query after source suppression", "memories")
        .unwrap();
    assert!(!suppressed.to_string().contains("合成本次半日徒步约束"));
    assert_eq!(calls(&log).len(), 3);
    assert_eq!(
        native.read(
            "source",
            "sources",
            json!({"refs":[reference],"budget_bytes":8192})
        )["ok"],
        false
    );
    let mut cfg = runtime_state.configuration().unwrap();
    cfg.paused = false;
    runtime_state.configure(cfg).unwrap();
    runtime_state.tick(&mut NoLlm).unwrap();
    let empty = runtime_state.status().unwrap().index_refresh.unwrap();
    assert_eq!(empty.state, "ready");
    assert_eq!(empty.total_documents, 0);
    assert_eq!(calls(&log).len(), 3);
    operation.disable_cloud_semantic().unwrap();
    assert!(runtime_state
        .configuration()
        .unwrap()
        .candidate_semantic_config
        .is_none());
    assert_eq!(vault.events().unwrap().len(), 1);
    assert_eq!(vault.memories().unwrap().len(), 1);
}
