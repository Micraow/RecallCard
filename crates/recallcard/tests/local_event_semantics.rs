//! 实际可信Rust→Python→回环合成提供器→SQLite→原文校验；不是模型质量评测。
use recallcard::{
    local_events::{LocalEvents, Setup},
    Vault,
};
use serde_json::json;
use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
};
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn server() -> (Server, String) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/local-events/fake_server.py");
    let mut child = Command::new("python3")
        .arg("-I")
        .arg("-S")
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let port: u16 = line
        .trim()
        .parse()
        .expect("合成回环提供器需要可用的本机端口");
    (
        Server(child),
        format!("http://127.0.0.1:{port}/v1/embeddings"),
    )
}
fn setup(endpoint: String) -> Setup {
    Setup {
        endpoint,
        model: "synthetic-contract".into(),
        revision: "fixture-v1".into(),
        tokenizer: "fixture-no-tokenizer".into(),
        dimensions: 2,
        query_instruction: String::new(),
        document_instruction: String::new(),
        events_approved: true,
        query_approved: true,
        chunk_bytes: 256,
        disk_cap_mib: 16,
    }
}
fn event(vault: &Vault, scope: &str, text: &str, origin: &str) -> String {
    let e=vault.capture(serde_json::from_value(json!({"scope":scope,"role":"user","origin":origin,"content":text,"source":{"platform":"synthetic","conversation_id":"local-contract","message_id":text}})).unwrap()).unwrap();
    format!("event:{}", e.id)
}
fn refresh(index: &LocalEvents<'_>) {
    for _ in 0..64 {
        if index.refresh_step().unwrap()["complete"] == true {
            return;
        }
    }
    panic!("bounded fixture should complete");
}
#[test]
fn ordinary_events_are_indexed_without_memory_and_revalidated_after_changes() {
    let (_server, endpoint) = server();
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let alpha = event(
        &vault,
        "personal",
        &format!("{}synthetic-alpha 原始用户话", " ".repeat(256)),
        "user_input",
    );
    let beta = event(
        &vault,
        "personal",
        "synthetic-beta 另一条原话",
        "user_input",
    );
    event(
        &vault,
        "project:hidden",
        "synthetic-alpha 外部范围不可出现",
        "user_input",
    );
    event(
        &vault,
        "personal",
        "synthetic-alpha 注入内容不能进入索引",
        "context_injection",
    );
    let python_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../python")
        .canonicalize()
        .unwrap();
    let config = setup(endpoint);
    let index = LocalEvents::configure(
        &vault,
        "personal",
        config.clone(),
        "python3".into(),
        python_path.clone(),
    )
    .unwrap();
    refresh(&index);
    assert!(vault.memories().unwrap().is_empty());
    let found = index.search("test-query-a", 10).unwrap();
    assert_eq!(found["results"][0]["ref"], alpha);
    assert_eq!(found["results"][0]["text_range"]["start_byte"], 256);
    assert_eq!(found["results"].as_array().unwrap().len(), 2);
    assert_eq!(
        found["coverage"]["source_verification"],
        "current_event_hash_and_utf8_range"
    );
    let again = index.search("test-query-a", 10).unwrap();
    assert_eq!(again["coverage"]["semantic_query_encoding"], "cache");
    vault
        .suppress(
            alpha.trim_start_matches("event:"),
            "synthetic withdrawal".into(),
        )
        .unwrap();
    assert!(index
        .search("test-query-a", 10)
        .unwrap_err()
        .contains("stale"));
    refresh(&index);
    let found = index.search("test-query-a", 10).unwrap();
    assert_eq!(found["results"].as_array().unwrap().len(), 1);
    assert_eq!(found["results"][0]["ref"], beta);
    let new = event(&vault, "personal", "synthetic-alpha 新增原话", "user_input");
    assert!(index.search("test-query-a", 10).is_err());
    refresh(&index);
    assert_eq!(
        index.search("test-query-a", 10).unwrap()["results"][0]["ref"],
        new
    );
    let mut changed = config;
    changed.revision = "fixture-v2".into();
    let other =
        LocalEvents::configure(&vault, "personal", changed, "python3".into(), python_path).unwrap();
    assert!(index.search("test-query-a", 10).is_err());
    assert!(other.search("test-query-a", 10).is_err());
    refresh(&other);
    LocalEvents::disable(&vault, "personal").unwrap();
    assert!(LocalEvents::open(&vault, "personal").unwrap().is_none());
    assert!(other.search("test-query-a", 10).is_err());
}
