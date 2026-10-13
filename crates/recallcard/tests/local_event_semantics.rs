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

#[test]
fn shared_local_gateway_reaches_mcp_native_and_keeps_current_grants() {
    use recallcard::{
        application::connections::{self, AgentReadService, ConnectionGrant},
        context::Context,
        native,
        policy::Access,
        transport,
    };
    let (_server, endpoint) = server();
    let d = tempfile::tempdir().unwrap();
    let vault = Vault::init(&d.path().join("vault")).unwrap();
    let alpha = event(&vault, "personal", "synthetic-alpha 原话", "user_input");
    let hidden = event(
        &vault,
        "project:hidden",
        "synthetic-alpha 范围外原话",
        "user_input",
    );
    let python_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../python")
        .canonicalize()
        .unwrap();
    let index = LocalEvents::configure(
        &vault,
        "personal",
        setup(endpoint),
        "python3".into(),
        python_path,
    )
    .unwrap();
    refresh(&index);
    let access = Access::new(vec!["personal".into()]).unwrap();
    let context = Context::new(&vault, access.clone());
    let found =
        transport::invoke(&context, "recall_search", json!({"query":"test-query-a"})).unwrap();
    assert_eq!(found["results"][0]["ref"], alpha);
    assert!(!found.to_string().contains(&hidden));
    assert!(serde_json::to_vec(&found).unwrap().len() <= 8192);
    let id = found["results"][0]["id"].clone();
    let read = transport::invoke(&context, "recall_fetch", json!({"id":id})).unwrap();
    assert_eq!(
        read["results"][0]["snapshot"],
        found["results"][0]["snapshot"]
    );
    assert_eq!(read["results"][0]["text"], "synthetic-alpha 原话");
    let entry = connections::configure(
        &vault,
        ConnectionGrant {
            client_kind: "chatgpt_mcp".into(),
            host_identity: "openai-chatgpt".into(),
            installation_id: None,
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
    let wide = Access::new(vec!["personal".into(), "project:hidden".into()]).unwrap();
    let service = AgentReadService::new(&vault, &entry.id, &wide, None);
    let mcp = service
        .invoke("recall_search", json!({"query":"test-query-a"}))
        .unwrap();
    assert_eq!(mcp["results"], found["results"]);
    assert_eq!(
        service.invoke("recall_fetch", json!({"id":id})).unwrap(),
        read
    );
    const EXT: &str = "abcdefghijklmnopabcdefghijklmnop";
    const INSTALL: &str = "11111111-1111-4111-8111-111111111111";
    let browser = connections::configure(
        &vault,
        ConnectionGrant {
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
    let call = |action: &str, args: serde_json::Value| {
        let body=serde_json::to_vec(&json!({"protocol":"recallcard.action/1","request_id":"local-read","nonce":"synthetic-local-nonce","session_ref":"chatgpt:local-read","installation_id":INSTALL,"action":"authorized_read","arguments":{"action":action,"arguments":args,"permission_revision":1}})).unwrap();
        let mut frame = (body.len() as u32).to_ne_bytes().to_vec();
        frame.extend(body);
        let mut out = Vec::new();
        native::serve_native_capture_io(
            &vault,
            wide.clone(),
            None,
            EXT,
            &native::extension_origin(EXT).unwrap(),
            frame.as_slice(),
            &mut out,
        )
        .unwrap();
        serde_json::from_slice::<serde_json::Value>(&out[4..]).unwrap()
    };
    let native = call("recall_search", json!({"query":"test-query-a"}));
    assert_eq!(native["ok"], true, "{native}");
    assert_eq!(native["result"]["results"], found["results"]);
    assert_eq!(call("recall_fetch", json!({"id":id}))["result"], read);
    connections::revoke(&vault, &browser.id, browser.permission_revision).unwrap();
    assert_eq!(
        call("recall_search", json!({"query":"test-query-a"}))["ok"],
        false
    );
    connections::revoke(&vault, &entry.id, entry.permission_revision).unwrap();
    assert!(service.invoke("recall_fetch", json!({"id":id})).is_err());
    event(&vault, "personal", "synthetic-beta 新资料", "user_input");
    assert!(transport::invoke(&context, "recall_fetch", json!({"id":id})).is_err());
}

#[test]
fn verified_identifier_beyond_twenty_semantic_candidates_is_reordered_before_budget() {
    let (_server, endpoint) = server();
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    for n in 0..25 {
        event(
            &vault,
            "personal",
            &format!("synthetic distractor {n}, no requested code"),
            "user_input",
        );
    }
    // Deliberately less similar according to the fixed encoder, yet exact text.
    let exact = event(
        &vault,
        "personal",
        "synthetic-alpha E_PIPE_71 原始记录",
        "user_input",
    );
    let python_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../python")
        .canonicalize()
        .unwrap();
    let index = LocalEvents::configure(
        &vault,
        "personal",
        setup(endpoint),
        "python3".into(),
        python_path,
    )
    .unwrap();
    refresh(&index);
    let found = index.search("E_PIPE_71", 2).unwrap();
    assert_eq!(found["results"][0]["ref"], exact);
    assert_eq!(found["results"][0]["semantic_rank"], 26);
    assert_eq!(found["results"][0]["scope_rank"], 1);
    assert_eq!(found["results"][0]["score"], 0.0);
    assert_eq!(found["results"][0]["score_kind"], "cosine");
    assert_eq!(found["results"][0]["retrieval_support"]["protected"], true);
    assert_eq!(
        found["results"][0]["retrieval_support"]["match_scope"],
        "returned_excerpt"
    );
    assert_eq!(found["results"][0]["relevance"], "unassessed");
    assert_eq!(found["coverage"]["verified_candidates"], 26);
    assert_eq!(found["truncated"], true);
    let unknown = index.search("E_PIPE_999", 2).unwrap();
    assert!(unknown["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["retrieval_support"]["protected"] == false && r["relevance"] == "unassessed"));
    assert_eq!(unknown["results"].as_array().unwrap().len(), 2);
}

#[test]
fn original_neighbor_prevents_a_chunk_boundary_from_forging_an_exact_identifier() {
    let (_server, endpoint) = server();
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    // The first 256-byte chunk ends with E_PIPE_71, but the original continues 0.
    let clipped = event(
        &vault,
        "personal",
        &format!("{}E_PIPE_710", " ".repeat(247)),
        "user_input",
    );
    let python_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../python")
        .canonicalize()
        .unwrap();
    let index = LocalEvents::configure(
        &vault,
        "personal",
        setup(endpoint),
        "python3".into(),
        python_path,
    )
    .unwrap();
    refresh(&index);
    let found = index.search("E_PIPE_71", 2).unwrap();
    assert_eq!(found["results"][0]["ref"], clipped);
    assert!(found["results"][0]["text"]
        .as_str()
        .unwrap()
        .ends_with("E_PIPE_71"));
    assert_eq!(found["results"][0]["retrieval_support"]["protected"], false);
    assert_eq!(found["results"][0]["relevance"], "unassessed");
}
