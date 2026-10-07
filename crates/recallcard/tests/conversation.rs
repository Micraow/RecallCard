use recallcard::{
    context::{Context, SearchArgs},
    conversation::{self, Conversation},
    native,
    policy::Access,
    transport, Vault,
};
use serde_json::{json, Value};
fn fixture() -> Value {
    json!({"schema":"recallcard.conversation/1","capture_id":"synthetic-capture-1","captured_at":"2026-10-07T00:00:00Z","title":"项目选择","source":{"platform":"deepseek","conversation_id":"session-demo","url":"https://chat.deepseek.com/a/chat/s/session-demo"},"coverage":{"extent":"visible_only","complete":false,"reason":"only_rendered_messages","warnings":[]},"messages":[{"id":"user-1","role":"user","text":"这个项目决定使用 Rust，理由是内存安全。","occurred_at":null},{"id":"assistant-1","role":"assistant","text":"建议以后再加入 Python，请你确认。","occurred_at":null}]})
}
fn conversation(v: &Value) -> Conversation {
    Conversation::parse(&v.to_string()).unwrap()
}
fn setup() -> (tempfile::TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    (d, v)
}
#[test]
fn roles_unknown_time_and_partial_coverage_survive_save_and_immediate_recall() {
    let (_d, v) = setup();
    let c = conversation(&fixture());
    let p = conversation::preview(&v, &c, "personal").unwrap();
    let saved =
        conversation::save(&v, &c, "personal", p["approval_hash"].as_str().unwrap()).unwrap();
    assert_eq!(saved["events_added"], 2);
    let events = v.events().unwrap();
    assert!(events.iter().all(|e| e.data.occurred_at.is_none()));
    assert!(events
        .iter()
        .all(|e| e.data.capture.completeness.as_deref() == Some("partial")));
    assert!(events
        .iter()
        .any(|e| e.data.role == recallcard::Role::Assistant));
    let context = Context::new(&v, Access::new(vec!["personal".into()]).unwrap());
    let found = context
        .search(
            serde_json::from_value::<SearchArgs>(json!({"query":"Rust","budget_tokens":8000}))
                .unwrap(),
        )
        .unwrap();
    assert!(!found["results"].as_array().unwrap().is_empty());
    assert_eq!(v.memories().unwrap().len(), 0);
}
#[test]
fn same_export_and_recapture_with_stable_ids_do_not_duplicate_messages() {
    let (_d, v) = setup();
    let mut data = fixture();
    for step in 0..2 {
        data["capture_id"] = json!(format!("capture-{step}"));
        data["captured_at"] = json!(format!("2026-10-07T00:00:0{step}Z"));
        let c = conversation(&data);
        let p = conversation::preview(&v, &c, "personal").unwrap();
        let r =
            conversation::save(&v, &c, "personal", p["approval_hash"].as_str().unwrap()).unwrap();
        assert_eq!(r["events_added"], if step == 0 { 2 } else { 0 });
    }
}
#[test]
fn approval_binds_redacted_content_and_fixed_scope() {
    let (_d, v) = setup();
    let mut data = fixture();
    let c = conversation(&data);
    let p = conversation::preview(&v, &c, "personal").unwrap();
    let hash = p["approval_hash"].as_str().unwrap();
    assert!(conversation::save(&v, &c, "work", hash).is_err());
    data["messages"][0]["text"] = json!("不同的决定");
    assert!(conversation::save(&v, &conversation(&data), "personal", hash).is_err());
    assert!(v.events().unwrap().is_empty());
}
#[test]
fn hostile_or_falsely_complete_exports_are_rejected() {
    for path in ["role", "coverage", "host", "unknown", "duplicate"] {
        let mut data = fixture();
        match path {
            "role" => data["messages"][0]["role"] = json!("system"),
            "coverage" => data["coverage"]["complete"] = json!(true),
            "host" => data["source"]["url"] = json!("https://chat.deepseek.com.evil.test/"),
            "unknown" => data["scope"] = json!("secret"),
            _ => data["messages"][1]["id"] = json!("user-1"),
        }
        assert!(Conversation::parse(&data.to_string()).is_err(), "{path}");
    }
}
fn native_request(action: &str, args: Value) -> Vec<u8> {
    let body=serde_json::to_vec(&json!({"protocol":"recallcard.action/1","request_id":"capture-demo","nonce":"synthetic-nonce-123456","session_ref":"deepseek:demo","action":action,"arguments":args})).unwrap();
    let mut frame = (body.len() as u32).to_ne_bytes().to_vec();
    frame.extend(body);
    frame
}
fn native_call(v: &Vault, scope: Option<&str>, action: &str, args: Value) -> Value {
    let ext = "abcdefghijklmnopabcdefghijklmnop";
    let mut output = Vec::new();
    native::serve_native_capture_io(
        v,
        Access::new(vec!["personal".into()]).unwrap(),
        scope,
        ext,
        &native::extension_origin(ext).unwrap(),
        std::io::Cursor::new(native_request(action, args)),
        &mut output,
    )
    .unwrap();
    serde_json::from_slice(&output[4..]).unwrap()
}
#[test]
fn browser_capture_requires_separate_local_permission_and_mcp_stays_readonly() {
    let (_d, v) = setup();
    let args = json!({"conversation":fixture()});
    assert_eq!(
        native_call(&v, None, "connection", json!({}))["result"]["capture_enabled"],
        false
    );
    assert_eq!(
        native_call(&v, None, "capture_preview", args.clone())["ok"],
        false
    );
    let p = native_call(&v, Some("personal"), "capture_preview", args.clone());
    assert_eq!(p["ok"], true);
    assert!(v.events().unwrap().is_empty());
    let r = native_call(
        &v,
        Some("personal"),
        "capture_save",
        json!({"conversation":fixture(),"approval_hash":p["result"]["approval_hash"]}),
    );
    assert_eq!(r["result"]["events_added"], 2);
    let bad = native_call(
        &v,
        Some("personal"),
        "capture_preview",
        json!({"conversation":fixture(),"scope":"secret"}),
    );
    assert_eq!(bad["ok"], false);
    let c = Context::new(&v, Access::new(vec!["personal".into()]).unwrap());
    assert!(transport::invoke(&c, "capture_save", args).is_err());
}

#[test]
fn extension_fixture_saves_through_real_installed_launcher_and_is_immediately_readable() {
    use std::io::Write;
    let (_d, v) = setup();
    let ext = "abcdefghijklmnopabcdefghijklmnop";
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../extension/tests/fixtures/conversation.json");
    let data: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    let output = v.root().parent().unwrap().join("native");
    let installed = std::process::Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--vault")
        .arg(v.root())
        .args([
            "native-install",
            "--scope",
            "personal",
            "--capture-scope",
            "personal",
            "--extension-id",
            ext,
            "--output-dir",
        ])
        .arg(output)
        .output()
        .unwrap();
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    let config: Value = serde_json::from_slice(&installed.stdout).unwrap();
    let run = |action: &str, args: Value| {
        let mut child = std::process::Command::new(config["launcher"].as_str().unwrap())
            .arg(native::extension_origin(ext).unwrap())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&native_request(action, args))
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout[4..]).unwrap()
    };
    assert_eq!(
        run("connection", json!({}))["result"]["capture_enabled"],
        true
    );
    let preview = run("capture_preview", json!({"conversation":data}));
    assert_eq!(preview["ok"], true);
    assert!(v.events().unwrap().is_empty());
    let args = json!({"conversation":data,"approval_hash":preview["result"]["approval_hash"]});
    let saved = run("capture_save", args.clone());
    assert_eq!(saved["result"]["events_added"], 2);
    assert_eq!(run("capture_save", args)["result"]["events_added"], 0);
    let response = run("search", json!({"query":"离线会话","budget_tokens":8000}));
    assert_eq!(response["ok"], true);
    assert!(!response["result"]["results"].as_array().unwrap().is_empty());
}

#[test]
fn old_capture_approval_cannot_save_to_another_vault_or_replaced_path() {
    let (d, v) = setup();
    let c = conversation(&fixture());
    let preview = conversation::preview(&v, &c, "personal").unwrap();
    let approval = preview["approval_hash"].as_str().unwrap();
    let other = Vault::init(&d.path().join("other")).unwrap();
    assert!(conversation::save(&other, &c, "personal", approval).is_err());
    assert!(other.events().unwrap().is_empty());
    std::fs::rename(v.root(), d.path().join("old-vault")).unwrap();
    let replacement = Vault::init(v.root()).unwrap();
    assert!(conversation::save(&replacement, &c, "personal", approval).is_err());
    assert!(replacement.events().unwrap().is_empty());
}

#[test]
fn partial_capture_followed_by_full_capture_and_early_revision_preserves_message_order() {
    let (_d, v) = setup();
    let mut full = fixture();
    let mut partial = full.clone();
    partial["messages"] = json!([full["messages"][1].clone()]);
    for data in [&partial, &full] {
        let c = conversation(data);
        let p = conversation::preview(&v, &c, "personal").unwrap();
        conversation::save(&v, &c, "personal", p["approval_hash"].as_str().unwrap()).unwrap();
    }
    full["messages"][0]["text"] = json!("修正早先的决定：使用 Rust 和明确边界。");
    let c = conversation(&full);
    let p = conversation::preview(&v, &c, "personal").unwrap();
    conversation::save(&v, &c, "personal", p["approval_hash"].as_str().unwrap()).unwrap();
    let context = Context::new(&v, Access::new(vec!["personal".into()]).unwrap());
    let visible: std::collections::BTreeSet<_> = context
        .documents()
        .unwrap()
        .into_iter()
        .map(|d| d.reference)
        .collect();
    let current = v
        .events()
        .unwrap()
        .into_iter()
        .filter(|e| visible.contains(&format!("event:{}", e.id)))
        .collect();
    let (ordered, known) = conversation::ordered_events(current);
    assert!(known);
    assert_eq!(ordered.len(), 2);
    assert_eq!(ordered[0].data.source.message_id, "user-1");
    assert_eq!(ordered[1].data.source.message_id, "assistant-1");
}
