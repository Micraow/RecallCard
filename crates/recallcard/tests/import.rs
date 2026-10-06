use recallcard::{import::import_text, Vault};
use serde_json::json;
#[test]
fn manual_and_agent_fixtures_import_idempotently() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let web = include_str!("../../../fixtures/manual-web.jsonl");
    let agent = include_str!("../../../fixtures/claude-code.jsonl");
    assert_eq!(
        import_text(&v, "manual-jsonl", web, "project:demo").unwrap()["events_added"],
        2
    );
    assert_eq!(
        import_text(&v, "claude-code", agent, "project:demo").unwrap()["events_added"],
        4
    );
    assert_eq!(
        import_text(&v, "claude-code", agent, "project:demo").unwrap()["events_added"],
        0
    );
    assert_eq!(v.events().unwrap().len(), 6);
}
#[test]
fn chatgpt_import_chooses_current_branch_and_ignores_hidden_reasoning() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let data = json!([{"id":"demo","current_node":"b","mapping":{"root":{"parent":null,"message":null},"a":{"parent":"root","message":{"id":"a","author":{"role":"user"},"create_time":null,"content":{"content_type":"text","parts":["合成问题"]}}},"b":{"parent":"a","message":{"id":"b","author":{"role":"assistant"},"create_time":null,"content":{"content_type":"text","parts":["当前分支"]}}},"other":{"parent":"a","message":{"id":"other","author":{"role":"assistant"},"content":{"content_type":"text","parts":["未选择分支"]}}}}}]);
    let r = import_text(&v, "chatgpt-export", &data.to_string(), "personal").unwrap();
    assert_eq!(r["events_added"], 2);
    assert!(v
        .events()
        .unwrap()
        .iter()
        .all(|e| e.data.occurred_at.is_none()));
    assert!(!v
        .events()
        .unwrap()
        .iter()
        .any(|e| e.data.content.contains("未选择")));
}
#[test]
fn unknown_formats_and_scope_mismatch_do_not_write() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    assert!(import_text(&v, "arbitrary", "{}", "personal").is_err());
    assert!(import_text(
        &v,
        "manual-jsonl",
        include_str!("../../../fixtures/manual-web.jsonl"),
        "personal"
    )
    .is_err());
    assert!(v.events().unwrap().is_empty());
}
