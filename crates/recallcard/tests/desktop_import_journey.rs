//! 来源阅读与补充背景使用同一真实本地服务；资料均为合成。
use recallcard::{context::Context, desktop::DesktopSession, policy::Access, Vault};
use serde_json::{json, Value};
#[test]
fn original_activity_order_assets_and_user_update_are_immediately_readable() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let make = |id: &str, time: Value| {
        serde_json::from_value(json!({"role":"user","origin":"user_input","scope":"personal","content":"合成旧聊天","occurred_at":time,"source":{"platform":"synthetic","conversation_id":id,"message_id":"u"},"metadata":{"source_assets":{"files":[{"source_file_id":"f","name":"合成.pdf","byte_count":12,"payload_status":"not_in_export"}],"citations":[],"tool_trace":[],"unsupported_fragments":[]}}})).unwrap()
    };
    vault
        .capture(make("recent", json!("2026-10-01T10:00:00Z")))
        .unwrap();
    vault
        .capture(make("old", json!("2020-01-01T10:00:00Z")))
        .unwrap();
    vault.capture(make("unknown", Value::Null)).unwrap();
    let mut desktop = DesktopSession::default();
    let session = desktop.select_vault(vault.root(), false).unwrap();
    let list = desktop
        .conversations(&session.session_id, "personal")
        .unwrap();
    assert_eq!(
        list["conversations"][0]["last_occurred_at"],
        "2026-10-01T10:00:00Z"
    );
    assert!(list["conversations"][2]["last_occurred_at"].is_null());
    let key = list["conversations"][0]["session_ref"].as_str().unwrap();
    let page = desktop
        .conversation_messages(&session.session_id, "personal", key, 0)
        .unwrap();
    assert_eq!(
        page["messages"][0]["assets"]["files"][0]["name"],
        "合成.pdf"
    );
    assert_eq!(
        page["messages"][0]["assets"]["files"][0]["payload_status"],
        "not_in_export"
    );
    let note = desktop
        .preview_note(
            &session.session_id,
            "personal",
            "SyntheticCurrentUpdate：改为下周交付",
        )
        .unwrap();
    let saved = desktop
        .confirm_note(&session.session_id, &note.preview_id)
        .unwrap();
    let ctx = Context::new(&vault, Access::new(vec!["personal".into()]).unwrap());
    let search = ctx
        .search(serde_json::from_value(json!({"query":"SyntheticCurrentUpdate"})).unwrap())
        .unwrap();
    assert_eq!(search["results"][0]["ref"], saved["ref"]);
    assert!(vault.memories().unwrap().is_empty());
}
