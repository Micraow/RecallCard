//! 纯合成安全回归；回环固定向量只验证传输、恢复与权限合同，不证明语义质量。
use recallcard::{
    hash,
    local_events::{LocalEvents, Setup},
    Vault,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::Duration,
};

static SERIAL: Mutex<()> = Mutex::new(());
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn server(log: &Path) -> (Server, String) {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/local-events/fake_server.py");
    // 重用生产合同fixture，只记录本测试发送的合成input，检查越权正文未到达服务。
    let script = r#"import json,runpy,sys
original=json.loads
def record(*args,**kwargs):
    result=original(*args,**kwargs)
    if isinstance(result,dict) and 'input' in result:
        with open(sys.argv[2],'a',encoding='utf-8') as stream:
            stream.write(json.dumps(result['input'],ensure_ascii=False)+'\n')
    return result
json.loads=record
runpy.run_path(sys.argv[1],run_name='__main__')
"#;
    let mut child = Command::new("python3")
        .args(["-I", "-S", "-c", script])
        .arg(fixture)
        .arg(log)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let port: u16 = line.trim().parse().unwrap();
    (
        Server(child),
        format!("http://127.0.0.1:{port}/v1/embeddings"),
    )
}
fn configure<'a>(vault: &'a Vault, endpoint: String) -> LocalEvents<'a> {
    LocalEvents::configure(
        vault,
        "personal",
        Setup {
            endpoint,
            model: "synthetic-contract".into(),
            revision: "safety-fixture-v1".into(),
            tokenizer: "synthetic-no-tokenizer".into(),
            dimensions: 2,
            query_instruction: String::new(),
            document_instruction: String::new(),
            events_approved: true,
            query_approved: true,
            chunk_bytes: 256,
            disk_cap_mib: 16,
        },
        "python3".into(),
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../python")
            .canonicalize()
            .unwrap(),
    )
    .unwrap()
}
fn event(vault: &Vault, scope: &str, text: &str) -> String {
    vault
        .capture(
            serde_json::from_value(json!({"scope":scope,"role":"user","origin":"user_input",
                "content":text,"source":{"platform":"synthetic","conversation_id":"safety","message_id":text}}))
            .unwrap(),
        )
        .unwrap()
        .id
}
fn refresh(index: &LocalEvents<'_>) {
    for _ in 0..16 {
        if index.refresh_step().unwrap()["complete"] == true {
            return;
        }
    }
    panic!("合成资料应在有界步数内完成");
}
fn read(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn write(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

#[test]
fn poisoned_manifest_config_and_missing_database_never_authorize_sources() {
    let _serial = SERIAL.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("synthetic-provider-inputs.jsonl");
    let (_server, endpoint) = server(&log);
    let vault = Vault::init(&temp.path().join("vault")).unwrap();
    let allowed = event(&vault, "personal", "synthetic-alpha allowed-a");
    let other = event(&vault, "personal", "synthetic-beta allowed-b");
    let forbidden = event(&vault, "project:hidden", "forbidden-cross-scope-marker");
    let index = configure(&vault, endpoint);
    refresh(&index);
    let current = vault
        .state_dir()
        .unwrap()
        .join("local-event-semantic")
        .join(hash(b"personal"))
        .join("current.json");
    let config = read(&current);
    let worker_config = PathBuf::from(config["worker_config"].as_str().unwrap());
    let cache = worker_config.parent().unwrap();
    let manifest_path = cache.join("manifest.json");
    let progress_path = cache.join("progress.json");
    let pristine = read(&manifest_path);
    let forbidden_file = pristine["files"]
        .as_array()
        .unwrap()
        .iter()
        .position(|name| {
            fs::read_to_string(vault.root().join(name.as_str().unwrap()))
                .unwrap()
                .contains(&forbidden)
        })
        .unwrap();
    for append in [true, false] {
        let mut poisoned = pristine.clone();
        let foreign = json!({"id":forbidden,"file":forbidden_file,"chunks":1});
        if append {
            poisoned["sources"].as_array_mut().unwrap().push(foreign);
        } else {
            poisoned["sources"][0] = foreign;
        }
        write(&manifest_path, &poisoned);
        assert!(index.search("test-query-a", 10).is_err());
        refresh(&index);
        let found = index.search("test-query-a", 10).unwrap();
        let refs: Vec<_> = found["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["ref"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(refs.len(), 2);
        assert!(refs.contains(&format!("event:{allowed}")));
        assert!(refs.contains(&format!("event:{other}")));
        assert!(!fs::read_to_string(&log)
            .unwrap()
            .contains("forbidden-cross-scope-marker"));
    }
    let mut poisoned = pristine;
    for source in poisoned["sources"].as_array_mut().unwrap() {
        source["chunks"] = json!(usize::MAX);
    }
    write(&manifest_path, &poisoned);
    let mut progress = read(&progress_path);
    progress["source_pos"] = json!(2);
    progress["chunk_pos"] = json!(0);
    write(&progress_path, &progress);
    let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        index.search("test-query-a", 10)
    }));
    assert!(attempt
        .expect("派生计数溢出必须返回错误，不能panic")
        .is_err());
    refresh(&index);
    let valid_progress = read(&progress_path);
    let mut invalid_progress = valid_progress.clone();
    invalid_progress["source_pos"] = json!(0);
    invalid_progress["chunk_pos"] = json!(usize::MAX);
    write(&progress_path, &invalid_progress);
    let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| index.refresh_step()));
    assert!(attempt
        .expect("越界chunk checkpoint必须返回错误，不能panic")
        .is_err());
    invalid_progress["source_pos"] = json!(usize::MAX);
    write(&progress_path, &invalid_progress);
    refresh(&index);

    let original_worker = read(&worker_config);
    let mut wrong_worker = original_worker.clone();
    wrong_worker["space"]["endpoint"] = json!("https://outside.invalid/v1/embeddings");
    write(&worker_config, &wrong_worker);
    let before = fs::read(&log).unwrap();
    assert!(index
        .status()
        .unwrap_err()
        .contains("configuration_changed"));
    assert!(index
        .search("test-query-a", 10)
        .unwrap_err()
        .contains("configuration_changed"));
    assert!(index
        .refresh_step()
        .unwrap_err()
        .contains("configuration_changed"));
    assert_eq!(fs::read(&log).unwrap(), before);
    write(&worker_config, &original_worker);

    fs::remove_file(cache.join("index.sqlite")).unwrap();
    // 已启动worker必须拒绝消失的数据库；下一显式步骤重启worker并复用旧checkpoint恢复。
    assert!(index.status().is_err());
    refresh(&index);
    assert_eq!(
        index.search("test-query-a", 10).unwrap()["results"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(!fs::read_to_string(&log)
        .unwrap()
        .contains("forbidden-cross-scope-marker"));
}

#[test]
fn idle_worker_restarts_within_the_first_request() {
    let _serial = SERIAL.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let (_server, endpoint) = server(&temp.path().join("unused-synthetic-inputs.jsonl"));
    let vault = Vault::init(&temp.path().join("vault")).unwrap();
    let index = configure(&vault, endpoint);
    assert_eq!(index.status().unwrap()["state"], "configured");
    std::thread::sleep(Duration::from_secs(31));
    assert_eq!(index.status().unwrap()["state"], "configured");
}

#[test]
fn revocation_while_worker_is_blocked_prevents_followup_query_posts() {
    let _serial = SERIAL.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("synthetic-provider-inputs.jsonl");
    let (_server, endpoint) = server(&log);
    let vault = Vault::init(&temp.path().join("vault")).unwrap();
    event(&vault, "personal", "synthetic-alpha queue source");
    let initial = configure(&vault, endpoint.clone());
    refresh(&initial);
    // 使用实际 Python EventIndex；只在测试包装层阻塞 status，编码仍走真实回环HTTP。
    let path = temp.path().join("queued-python");
    let package = path.join("recallcard_worker");
    fs::create_dir_all(&package).unwrap();
    let real = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../python/recallcard_worker")
        .canonicalize()
        .unwrap();
    fs::write(
        package.join("__init__.py"),
        format!("__path__.append({})\n", json!(real.to_string_lossy())),
    )
    .unwrap();
    fs::copy(real.join("event_index.py"), package.join("queue_actual.py")).unwrap();
    let entered = temp.path().join("status-entered");
    let release = temp.path().join("status-release");
    fs::write(
        package.join("event_index.py"),
        format!(
            r#"from . import queue_actual as real
from pathlib import Path
import time
entered=Path({entered})
release=Path({release})
original=real.EventIndex.status
def held(self,request):
    entered.write_text('status reached')
    until=time.monotonic()+12
    while not release.exists() and time.monotonic()<until:
        time.sleep(.01)
    if not release.exists():
        raise RuntimeError('synthetic gate timeout')
    return original(self,request)
real.EventIndex.status=held
main=real.main
"#,
            entered = json!(entered.to_string_lossy()),
            release = json!(release.to_string_lossy())
        ),
    )
    .unwrap();
    let settings: Setup =
        serde_json::from_value(initial.status().unwrap()["settings"].clone()).unwrap();
    let index =
        LocalEvents::configure(&vault, "personal", settings, "python3".into(), path).unwrap();
    let before = fs::read_to_string(&log).unwrap();
    std::thread::scope(|threads| {
        let active = threads.spawn(|| index.search("synthetic pending first query", 10));
        let until = std::time::Instant::now() + Duration::from_secs(5);
        while !entered.exists() && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(entered.exists(), "生产worker的status须实际到达阻塞点");
        let waiting = threads.spawn(|| index.search("synthetic pending second query", 10));
        std::thread::sleep(Duration::from_millis(100));
        LocalEvents::disable(&vault, "personal").unwrap();
        fs::write(&release, "release after revocation").unwrap();
        assert!(active.join().unwrap().is_err());
        assert!(waiting.join().unwrap().is_err());
    });
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        before,
        "撤权后不能开始任何新的embedding POST"
    );
}
