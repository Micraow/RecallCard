//! 紧凑投影合成回归；权限、原文与导航语义不因省略详细元数据而放宽。
use recallcard::{context::Context, policy::Access, Vault};
use serde_json::{json, Value};
fn capture(v: &Vault, id: &str, parent: Option<&str>, text: &str, role: &str) -> String {
    let event=v.capture(serde_json::from_value(json!({"role":role,"origin":"native","scope":"personal","occurred_at":"2026-10-01T10:00:00Z","content":text,"source":{"platform":"qwen","account_namespace":"synthetic-account-long-identity","conversation_id":"synthetic-conversation-long-identity","message_id":id},"metadata":{"qwen":{"node_id":id,"nearest_visible_parent_id":parent,"on_current_path":true}}})).unwrap()).unwrap();
    format!("event:{}", event.id)
}
fn read(v: &Vault, args: Value) -> Result<Value, String> {
    Context::new(v, Access::new(vec!["personal".into()]).unwrap())
        .read_page(serde_json::from_value(args).unwrap())
}
#[test]
fn same_budget_compact_carries_more_original_text_and_required_provenance() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let root = capture(
        &v,
        "root",
        None,
        &"合成较长原话内容。".repeat(600),
        "assistant",
    );
    let next = capture(&v, "next", Some("root"), "合成用户更正：仅离线。", "user");
    let full = read(
        &v,
        json!({"refs":[root],"budget_bytes":1500,"include_adjacent":true}),
    )
    .unwrap();
    let compact = read(
        &v,
        json!({"refs":[root],"budget_bytes":1500,"include_adjacent":true,"detail":"compact"}),
    )
    .unwrap();
    let full_text = full["results"][0]["text"].as_str().unwrap_or("");
    let result = &compact["results"][0];
    assert!(result["text"].as_str().unwrap().len() > full_text.len());
    assert_eq!(result["metadata"]["role"], "assistant");
    assert_eq!(result["metadata"]["scope"], "personal");
    assert_eq!(result["metadata"]["occurred_at"], "2026-10-01T10:00:00Z");
    assert_eq!(result["metadata"]["adjacent"]["next_user_ref"], next);
    assert_eq!(result["record_complete"], false);
    assert_eq!(compact["detail"], "compact");
    assert!(result["snapshot"].is_string());
    assert!(serde_json::to_vec(&compact).unwrap().len() <= 1500);
    assert!(result["metadata"].get("source").is_none());
}
#[test]
fn compact_pagination_recovers_exact_utf8_and_mode_cannot_change_mid_page() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let text = "合成中文与 ASCII source。".repeat(120);
    let root = capture(&v, "root", None, &text, "user");
    let mut args = json!({"refs":[root],"budget_bytes":1500,"detail":"compact"});
    let first = read(&v, args.clone()).unwrap();
    let mut wrong = args.clone();
    wrong["detail"] = json!("full");
    wrong["cursor"] = first["next_cursor"].clone();
    assert!(read(&v, wrong).unwrap_err().contains("游标"));
    let mut recovered = String::new();
    let mut pages = 0;
    loop {
        let page = read(&v, args.clone()).unwrap();
        pages += 1;
        assert!(pages < 100);
        recovered.push_str(page["results"][0]["text"].as_str().unwrap());
        if page["next_cursor"].is_null() {
            break;
        }
        args["cursor"] = page["next_cursor"].clone();
    }
    assert_eq!(recovered, text);
}
#[test]
fn compact_source_list_and_read_recheck_hidden_originals() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let root = capture(&v, "root", None, "合成用户来源。", "user");
    let memory=v.add_memory(serde_json::from_value(json!({"content":"合成长期记忆","source_refs":[root.trim_start_matches("event:")],"evidence":"user_explicit"})).unwrap()).unwrap();
    let args =
        json!({"refs":[format!("memory:{}",memory.id)],"detail":"compact","budget_bytes":1500});
    let context = Context::new(&v, Access::new(vec!["personal".into()]).unwrap());
    let before = context
        .sources_page(serde_json::from_value(args.clone()).unwrap())
        .unwrap();
    assert_eq!(before["results"][0]["source_refs"], json!([root]));
    assert!(before["results"][0].get("events").is_none());
    v.suppress(root.trim_start_matches("event:"), "合成隐藏".into())
        .unwrap();
    assert!(context
        .sources_page(serde_json::from_value(args.clone()).unwrap())
        .is_err());
    assert!(read(&v, args).is_err());
}
#[test]
fn default_full_contract_is_unchanged_and_invalid_mode_is_not_silently_ignored() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let root = capture(&v, "root", None, "合成小正文。", "user");
    let mut args = json!({"refs":[root],"budget_bytes":1500});
    let default = read(&v, args.clone()).unwrap();
    args["detail"] = json!("full");
    assert_eq!(default, read(&v, args.clone()).unwrap());
    args["detail"] = json!("summary");
    assert!(read(&v, args).unwrap_err().contains("未知读取"));
}
#[test]
fn compact_navigation_cursors_expire_when_new_user_source_is_written() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let root = capture(&v, "root", None, &"合成原文。".repeat(600), "assistant");
    let mut args =
        json!({"refs":[root],"budget_bytes":1500,"detail":"compact","include_adjacent":true});
    let first = read(&v, args.clone()).unwrap();
    let next = capture(&v, "next", Some("root"), "新合成约束。", "user");
    args["cursor"] = first["next_cursor"].clone();
    assert!(read(&v, args.clone()).unwrap_err().contains("游标"));
    args["cursor"] = Value::Null;
    assert_eq!(
        read(&v, args).unwrap()["results"][0]["metadata"]["adjacent"]["next_user_ref"],
        next
    );
}
