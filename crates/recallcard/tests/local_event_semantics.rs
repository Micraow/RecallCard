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
        attachments_approved: false,
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
    for _ in 0..128 {
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
    let found = transport::invoke(
        &context,
        "recall_search",
        json!({"query":"synthetic-alpha"}),
    )
    .unwrap();
    assert_eq!(found["results"][0]["ref"], alpha);
    assert_eq!(
        found["coverage"]["scopes"][0]["lexical"]["status"],
        "available"
    );
    assert_eq!(
        found["coverage"]["scopes"][0]["fusion"],
        "rrf/2:k=60:lexical-competition"
    );
    assert_eq!(found["results"][0]["retrieval"]["lexical_rank"], 1);
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
        .invoke("recall_search", json!({"query":"synthetic-alpha"}))
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
    let native = call("recall_search", json!({"query":"synthetic-alpha"}));
    assert_eq!(native["ok"], true, "{native}");
    assert_eq!(native["result"]["results"], found["results"]);
    assert_eq!(call("recall_fetch", json!({"id":id}))["result"], read);
    connections::revoke(&vault, &browser.id, browser.permission_revision).unwrap();
    assert_eq!(
        call("recall_search", json!({"query":"synthetic-alpha"}))["ok"],
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

#[test]
fn independent_sqlite_channel_recovers_identifier_outside_semantic_top_hundred() {
    let (_server, endpoint) = server();
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    for n in 0..100 {
        event(
            &vault,
            "personal",
            &format!("synthetic unrelated record {n}"),
            "user_input",
        );
    }
    let exact = event(
        &vault,
        "personal",
        "synthetic-alpha E_NET_731 原始编号",
        "user_input",
    );
    let hidden = event(&vault, "project:hidden", "E_NET_731 无权范围", "user_input");
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
    let found = index.search("E_NET_731", 2).unwrap();
    let row = &found["results"][0];
    assert_eq!(row["ref"], exact);
    assert!(row["semantic_rank"].is_null());
    assert!(row["retrieval"]["semantic_evidence"].is_null());
    assert_eq!(row["retrieval"]["lexical_rank"], 1);
    assert_eq!(
        row["retrieval"]["lexical_evidence_verification"],
        "one_complete_query_term"
    );
    assert_eq!(row["score"], 0.0);
    assert_eq!(row["score_kind"], "cosine");
    assert_eq!(row["retrieval_support"]["protected"], true);
    assert_eq!(row["relevance"], "unassessed");
    assert!(!found.to_string().contains(&hidden));
    assert_eq!(found["coverage"]["lexical"]["status"], "available");
    let neighbor = index.search("E_NET_7310", 2).unwrap();
    assert!(neighbor["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["retrieval"]["lexical_rank"].is_null()));
    vault
        .suppress(
            exact.trim_start_matches("event:"),
            "synthetic correction withdrawal".into(),
        )
        .unwrap();
    assert!(index.search("E_NET_731", 2).is_err());
    refresh(&index);
    let after = index.search("E_NET_731", 2).unwrap();
    assert!(!after.to_string().contains(&exact));
    assert!(after["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["retrieval"]["lexical_rank"].is_null()));
}

#[test]
fn lexical_cross_chunk_preview_and_shared_fetch_keep_complete_original_identifier() {
    use recallcard::{context::Context, policy::Access, transport};
    let (_server, endpoint) = server();
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let original = format!("{}E_PIPE_710 后续原文", " ".repeat(247));
    let exact = event(&vault, "personal", &original, "user_input");
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
    let context = Context::new(&vault, Access::new(vec!["personal".into()]).unwrap());
    let found =
        transport::invoke(&context, "recall_search", json!({"query":"E_PIPE_710"})).unwrap();
    let row = &found["results"][0];
    assert_eq!(row["ref"], exact);
    assert_eq!(row["retrieval_support"]["protected"], true);
    assert_eq!(row["matched_chunk"]["end_byte"], 256);
    assert_eq!(row["retrieval"]["lexical_evidence"]["start_byte"], 247);
    assert_eq!(row["retrieval"]["lexical_evidence"]["end_byte"], 257);
    let start = row["text_range"]["start_byte"].as_u64().unwrap() as usize;
    let end = row["text_range"]["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(row["text"], original[start..end]);
    assert!(row["text"].as_str().unwrap().contains("E_PIPE_710"));
    assert_eq!(
        row["text_hash"],
        recallcard::model::hash(&original.as_bytes()[start..end])
    );
    let page = transport::invoke(&context, "recall_fetch", json!({"id":row["id"]})).unwrap();
    assert_eq!(page["results"][0]["ref"], exact);
    assert_eq!(page["results"][0]["text_range"]["start_byte"], start);
    assert_eq!(page["results"][0]["text"], original[start..]);
    let long = "查一下去年秋天安装的办公室门锁型号还有当时讨论过的备用钥匙存放位置和更换电池日期";
    let limited = transport::invoke(&context, "recall_search", json!({"query":long})).unwrap();
    assert!(!limited["results"].as_array().unwrap().is_empty());
    assert_eq!(
        limited["coverage"]["scopes"][0]["lexical"]["status"],
        "limited_query"
    );
    assert_eq!(
        limited["coverage"]["scopes"][0]["lexical"]["reason"],
        "lexical_query_term_limit"
    );
}

#[test]
fn stopped_encoder_keeps_verified_lexical_source_and_current_authorization() {
    use recallcard::{context::Context, policy::Access, transport};
    let (server, endpoint) = server();
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let body = format!("{}E_PIPE_710 tail", " ".repeat(247));
    let target = event(&vault, "personal", &body, "user_input");
    let hidden = event(&vault, "project:hidden", "E_PIPE_710 hidden", "user_input");
    let index = LocalEvents::configure(
        &vault,
        "personal",
        setup(endpoint),
        "python3".into(),
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../python")
            .canonicalize()
            .unwrap(),
    )
    .unwrap();
    refresh(&index);
    drop(server);
    let direct = index.search("E_PIPE_710", 10).unwrap();
    assert_eq!(direct["coverage"]["event_search"], "local_lexical_fallback");
    assert_eq!(direct["status"], "partial");
    assert_eq!(
        direct["coverage"]["semantic"]["reason"],
        "local_provider_unavailable"
    );
    assert_eq!(direct["coverage"]["local_event_semantic"], false);
    assert_eq!(direct["coverage"]["partial"], true);
    let row = &direct["results"][0];
    assert_eq!(row["ref"], target);
    assert_eq!(row["score_kind"], "unavailable");
    assert!(row["score"].is_null());
    assert!(row["semantic_rank"].is_null());
    assert!(row["retrieval"]["semantic_evidence"].is_null());
    assert_eq!(
        row["retrieval"]["lexical_evidence_verification"],
        "one_complete_query_term"
    );
    assert!(row["text"].as_str().unwrap().contains("E_PIPE_710"));
    let context = Context::new(&vault, Access::new(vec!["personal".into()]).unwrap());
    let page = transport::invoke(&context, "recall_search", json!({"query":"E_PIPE_710"})).unwrap();
    assert_eq!(page["status"], "partial");
    assert_eq!(page["coverage"]["local_event_semantic"], false);
    assert_eq!(page["coverage"]["scopes"][0]["state"], "degraded");
    assert_eq!(page["coverage"]["scopes"][0]["fusion"], "lexical_only");
    assert!(!page.to_string().contains(&hidden));
    let id = page["results"][0]["id"].clone();
    let fetched = transport::invoke(&context, "recall_fetch", json!({"id":id})).unwrap();
    assert!(fetched["results"][0]["text"]
        .as_str()
        .unwrap()
        .contains("E_PIPE_710"));
    let absent =
        transport::invoke(&context, "recall_search", json!({"query":"missing-code"})).unwrap();
    assert_eq!(absent["status"], "unavailable");
    assert_eq!(absent["results"], json!([]));
    assert_eq!(
        absent["coverage"]["scopes"][0]["semantic"]["status"],
        "unavailable"
    );
    LocalEvents::disable(&vault, "personal").unwrap();
    assert!(index.search("E_PIPE_710", 10).is_err());
    assert!(transport::invoke(&context, "recall_fetch", json!({"id":id})).is_err());
}

fn semantic_attachment(
    vault: &Vault,
    scope: &str,
    message: &str,
    body: &str,
    parent: &str,
) -> recallcard::Event {
    let digest = recallcard::hash(body.as_bytes());
    std::fs::write(vault.root().join("objects").join(&digest), body).unwrap();
    vault.capture(serde_json::from_value(json!({"scope":scope,"role":"user","origin":"user_input","content":parent,
        "source":{"platform":"synthetic","conversation_id":"attachment-semantic-contract","message_id":message},
        "metadata":{"source_assets":{"files":[{"name":"synthetic.txt","payload_status":"retained","detected_media_type":"application/octet-stream","declared_media_type":"text/plain","object_sha256":digest,"byte_count":body.len()}]}}})).unwrap()).unwrap()
}
fn attachment_ref(event: &recallcard::Event) -> String {
    format!(
        "attachment:{}/{}",
        event.id,
        event.data.metadata["source_assets"]["files"][0]["object_sha256"]
            .as_str()
            .unwrap()
    )
}
fn semantic_python() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../python")
        .canonicalize()
        .unwrap()
}
#[test]
fn independent_attachment_consent_enables_zero_overlap_candidates_with_exact_fetch() {
    use recallcard::{context::Context, policy::Access, transport};
    let (_server, endpoint) = server();
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let body = "synthetic-alpha 附件原件\r\n中文🙂";
    let source = semantic_attachment(&vault, "personal", "one", body, "上传说明保持原样");
    let hidden = semantic_attachment(
        &vault,
        "project:hidden",
        "hidden",
        "synthetic-alpha 私域合成负例",
        "其他范围",
    );
    let mut settings = setup(endpoint);
    // 负对照：同一个固定向量服务、同一问题，仅普通 Event 批准不能编码附件。
    let legacy = LocalEvents::configure(
        &vault,
        "personal",
        settings.clone(),
        "python3".into(),
        semantic_python(),
    )
    .unwrap();
    refresh(&legacy);
    let without = legacy.search("test-query-a", 10).unwrap();
    assert!(without["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["kind"] == "event"));
    settings.attachments_approved = true;
    let index = LocalEvents::configure(
        &vault,
        "personal",
        settings,
        "python3".into(),
        semantic_python(),
    )
    .unwrap();
    refresh(&index);
    assert!(!body.contains("test-query-a"));
    let context = Context::new(&vault, Access::new(vec!["personal".into()]).unwrap());
    let found =
        transport::invoke(&context, "recall_search", json!({"query":"test-query-a"})).unwrap();
    let row = &found["results"][0];
    assert_eq!(row["ref"], attachment_ref(&source), "{found}");
    assert_eq!(row["kind"], "attachment");
    assert_eq!(row.get("role"), Some(&serde_json::Value::Null));
    assert_eq!(row["evidence"], "Attachment/Content");
    assert_eq!(
        row["evidence_refs"],
        json!([format!("event:{}", source.id)])
    );
    assert_eq!(row["text_range"]["projection"], "attachment.utf8/1");
    assert_eq!(found["coverage"]["attachments"]["semantic"], true);
    assert!(!found.to_string().contains(&attachment_ref(&hidden)));
    let fetched = transport::invoke(&context, "recall_fetch", json!({"id":row["id"]})).unwrap();
    assert_eq!(fetched["results"][0]["text"], body);
    assert_eq!(fetched["results"][0]["snapshot"], row["snapshot"]);
    assert_eq!(
        vault.event(&source.id).unwrap().data.text(),
        "上传说明保持原样"
    );
    let other = Context::new(&vault, Access::new(vec!["project:hidden".into()]).unwrap());
    assert!(transport::invoke(&other, "recall_fetch", json!({"id":row["id"]})).is_err());
    // 词法和语义都命中同一附件时，共同入口只返回一次独立来源。
    let duplicate = transport::invoke(
        &context,
        "recall_search",
        json!({"query":"synthetic-alpha"}),
    )
    .unwrap();
    assert_eq!(
        duplicate["results"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["ref"] == attachment_ref(&source))
            .count(),
        1
    );
    assert!(duplicate["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["ref"] == attachment_ref(&source))
        .unwrap()["role"]
        .is_null());
    assert!(serde_json::to_vec(&found).unwrap().len() <= 8192);
}
#[test]
fn attachment_only_bodyless_parent_and_withdrawn_approval_remain_separate() {
    use recallcard::{context::Context, policy::Access, transport};
    let (_server, endpoint) = server();
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let source = semantic_attachment(
        &vault,
        "personal",
        "bodyless",
        "synthetic-alpha 合成原件",
        "",
    );
    let ordinary = event(&vault, "personal", "synthetic-alpha 普通消息", "user_input");
    let mut settings = setup(endpoint);
    settings.events_approved = false;
    settings.attachments_approved = true;
    let index = LocalEvents::configure(
        &vault,
        "personal",
        settings.clone(),
        "python3".into(),
        semantic_python(),
    )
    .unwrap();
    refresh(&index);
    let rows = index.search("test-query-a", 10).unwrap();
    assert_eq!(rows["results"].as_array().unwrap().len(), 1);
    assert_eq!(rows["results"][0]["ref"], attachment_ref(&source));
    assert!(!rows.to_string().contains(&ordinary));
    assert_eq!(
        index.search("test-query-a", 10).unwrap()["coverage"]["semantic_query_encoding"],
        "cache"
    );
    let context = Context::new(&vault, Access::new(vec!["personal".into()]).unwrap());
    let result =
        transport::invoke(&context, "recall_search", json!({"query":"test-query-a"})).unwrap();
    let id = result["results"][0]["id"].clone();
    settings.events_approved = true;
    settings.attachments_approved = false;
    let replacement = LocalEvents::configure(
        &vault,
        "personal",
        settings,
        "python3".into(),
        semantic_python(),
    )
    .unwrap();
    assert!(index.search("test-query-a", 10).is_err());
    assert!(transport::invoke(&context, "recall_fetch", json!({"id":id})).is_err());
    refresh(&replacement);
    let result = replacement.search("test-query-a", 10).unwrap();
    assert_eq!(result["results"].as_array().unwrap().len(), 1);
    assert_eq!(result["results"][0]["ref"], ordinary);
}
#[test]
fn changed_attachment_object_invalidates_old_semantic_binding_and_refresh_keeps_events() {
    use recallcard::{context::Context, policy::Access, transport};
    for missing in [false, true] {
        let (_server, endpoint) = server();
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::init(&dir.path().join("vault")).unwrap();
        let body = "synthetic-alpha 合成附件";
        let source = semantic_attachment(&vault, "personal", "change", body, "");
        let ordinary = event(&vault, "personal", "synthetic-beta 普通消息", "user_input");
        let mut settings = setup(endpoint);
        settings.attachments_approved = true;
        let index = LocalEvents::configure(
            &vault,
            "personal",
            settings,
            "python3".into(),
            semantic_python(),
        )
        .unwrap();
        refresh(&index);
        let context = Context::new(&vault, Access::new(vec!["personal".into()]).unwrap());
        let found =
            transport::invoke(&context, "recall_search", json!({"query":"test-query-a"})).unwrap();
        let id = found["results"][0]["id"].clone();
        let path = vault
            .root()
            .join("objects")
            .join(recallcard::hash(body.as_bytes()));
        if missing {
            std::fs::remove_file(path).unwrap();
        } else {
            std::fs::write(path, vec![b'x'; body.len()]).unwrap();
        }
        assert!(index.search("test-query-a", 10).is_err());
        assert!(transport::invoke(&context, "recall_fetch", json!({"id":id})).is_err());
        refresh(&index);
        let page = index.search("test-query-a", 10).unwrap();
        assert_eq!(page["results"].as_array().unwrap().len(), 1);
        assert_eq!(page["results"][0]["ref"], ordinary);
        assert_eq!(page["coverage"]["attachments"]["status"], "partial");
        assert!(vault.event(&source.id).unwrap().data.text().is_empty());
    }
}
#[test]
fn semantic_attachment_revision_and_suppression_revoke_old_fetches() {
    use recallcard::{context::Context, policy::Access, transport};
    for revised in [false, true] {
        let (_server, endpoint) = server();
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::init(&dir.path().join("vault")).unwrap();
        let source =
            semantic_attachment(&vault, "personal", "revoke", "synthetic-alpha 合成附件", "");
        let mut settings = setup(endpoint);
        settings.attachments_approved = true;
        let index = LocalEvents::configure(
            &vault,
            "personal",
            settings,
            "python3".into(),
            semantic_python(),
        )
        .unwrap();
        refresh(&index);
        let context = Context::new(&vault, Access::new(vec!["personal".into()]).unwrap());
        let found =
            transport::invoke(&context, "recall_search", json!({"query":"test-query-a"})).unwrap();
        let id = found["results"][0]["id"].clone();
        if revised {
            let mut next = source.data.clone();
            next.content = "附件已移除".into();
            next.metadata = json!({});
            next.revision_of = Some(source.id.clone());
            vault.capture(next).unwrap();
        } else {
            vault.suppress(&source.id, "合成撤销".into()).unwrap();
        }
        assert!(transport::invoke(&context, "recall_fetch", json!({"id":id})).is_err());
        refresh(&index);
        assert!(index.search("test-query-a", 10).unwrap()["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["ref"] != attachment_ref(&source)));
    }
}
#[test]
fn old_event_configuration_without_attachment_field_stays_query_only_compatible() {
    use serde_json::Value;
    let (_server, endpoint) = server();
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let expected = event(&vault, "personal", "synthetic-alpha 普通消息", "user_input");
    let index = LocalEvents::configure(
        &vault,
        "personal",
        setup(endpoint),
        "python3".into(),
        semantic_python(),
    )
    .unwrap();
    refresh(&index);
    let current = vault
        .state_dir()
        .unwrap()
        .join("local-event-semantic")
        .join(recallcard::hash(b"personal"))
        .join("current.json");
    let mut host: Value = serde_json::from_slice(&std::fs::read(&current).unwrap()).unwrap();
    host["settings"]
        .as_object_mut()
        .unwrap()
        .remove("attachments_approved");
    host["settings"]["events_approved"] = json!(false);
    let path = PathBuf::from(host["worker_config"].as_str().unwrap());
    let mut worker: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    worker
        .as_object_mut()
        .unwrap()
        .remove("attachments_approved");
    worker["events_approved"] = json!(false);
    std::fs::write(&current, serde_json::to_vec(&host).unwrap()).unwrap();
    std::fs::write(&path, serde_json::to_vec(&worker).unwrap()).unwrap();
    let legacy = LocalEvents::open(&vault, "personal").unwrap().unwrap();
    assert_eq!(
        legacy.status().unwrap()["settings"]["attachments_approved"],
        false
    );
    assert!(legacy.refresh_step().is_err());
    assert_eq!(
        legacy.search("test-query-a", 10).unwrap()["results"][0]["ref"],
        expected
    );
}

#[test]
fn compact_mixed_facade_preserves_utf8_fetch_and_rejects_withdrawal() {
    use recallcard::{context::Context, policy::Access, transport};
    use std::collections::BTreeMap;
    let (_server, endpoint) = server();
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let mut originals = BTreeMap::new();
    for number in 0..8 {
        let body = format!(
            "{}synthetic-alpha 合成编号{number}，保留引号\"和换行\n终点。",
            "前缀🙂 ".repeat(64)
        );
        let reference = if number % 2 == 0 {
            event(&vault, "personal", &body, "user_input")
        } else {
            let object = recallcard::hash(body.as_bytes());
            std::fs::write(vault.root().join("objects").join(&object), &body).unwrap();
            let parent = vault.capture(serde_json::from_value(json!({
                "scope":"personal","role":"user","origin":"user_input","content":"",
                "source":{"platform":"synthetic","conversation_id":"compact-mixed","message_id":number.to_string()},
                "metadata":{"source_assets":{"files":[{"name":format!("synthetic-{number}.txt"),"payload_status":"retained","declared_media_type":"text/plain","detected_media_type":"text/plain","object_sha256":object,"byte_count":body.len()}]}}
            })).unwrap()).unwrap();
            format!("attachment:{}/{}", parent.id, object)
        };
        originals.insert(reference, body);
    }
    let mut settings = setup(endpoint);
    settings.attachments_approved = true;
    let python_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../python")
        .canonicalize()
        .unwrap();
    let index = LocalEvents::configure(&vault, "personal", settings, "python3".into(), python_path)
        .unwrap();
    refresh(&index);
    let context = Context::new(&vault, Access::new(vec!["personal".into()]).unwrap());
    let page = transport::invoke(
        &context,
        "recall_search",
        json!({"query":"synthetic-alpha"}),
    )
    .unwrap();
    assert!(serde_json::to_vec(&page).unwrap().len() <= 8192);
    assert_eq!(
        page["coverage"]["presentation"],
        "compact_index_metadata_full_source_via_fetch"
    );
    let rows = page["results"].as_array().unwrap();
    assert!(rows.iter().any(|row| row["kind"] == "event"));
    assert!(rows.iter().any(|row| row["kind"] == "attachment"));
    assert!(rows
        .iter()
        .any(|row| row["text_range"]["start_byte"].as_u64().unwrap() > 0));
    for row in rows {
        let original = &originals[row["ref"].as_str().unwrap()];
        let start = row["text_range"]["start_byte"].as_u64().unwrap() as usize;
        let end = row["text_range"]["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(row["text"], original[start..end]);
        assert_eq!(
            row["text_hash"],
            recallcard::hash(&original.as_bytes()[start..end])
        );
        if row["retrieval"].is_object() {
            assert_eq!(row["content_hash"], recallcard::hash(original.as_bytes()));
            assert!(row["matched_chunk"].is_object());
        }
        let fetched = transport::invoke(&context, "recall_fetch", json!({"id":row["id"]})).unwrap();
        let found = &fetched["results"][0];
        let finish = found["text_range"]["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(found["text_range"]["start_byte"], start);
        assert_eq!(found["text"], original[start..finish]);
        assert_eq!(found["snapshot"], row["snapshot"]);
    }
    let selected = rows.iter().find(|row| row["kind"] == "event").unwrap();
    vault
        .suppress(
            selected["ref"]
                .as_str()
                .unwrap()
                .trim_start_matches("event:"),
            "synthetic withdrawal after compact result".into(),
        )
        .unwrap();
    assert!(transport::invoke(&context, "recall_fetch", json!({"id":selected["id"]})).is_err());
}
