//! 原创合成来源图；不包含真实聊天，也不评估模型理解质量。
use recallcard::{context::Context, policy::Access, Vault};
use serde_json::{json, Value};
fn event(
    v: &Vault,
    node: &str,
    parent: Option<&str>,
    role: &str,
    path: Option<bool>,
    scope_session: (&str, &str),
    long: bool,
) -> String {
    let (scope, session) = scope_session;
    let data = json!({"role":role,"origin":"native","scope":scope,"content":if long{"合成正文片段。".repeat(600)}else{format!("合成条件 {node}")},"source":{"platform":"qwen","account_namespace":"synthetic","conversation_id":session,"message_id":node},"metadata":{"qwen":{"node_id":node,"nearest_visible_parent_id":parent,"parent_id":parent,"on_current_path":path}}});
    format!(
        "event:{}",
        v.capture(serde_json::from_value(data).unwrap()).unwrap().id
    )
}
fn call(v: &Vault, r: &str, adjacent: bool, cursor: Value) -> Result<Value, String> {
    Context::new(v, Access::new(vec!["personal".into()]).unwrap()).read_page(
        serde_json::from_value(
            json!({"refs":[r],"include_adjacent":adjacent,"cursor":cursor,"budget_bytes":2400}),
        )
        .unwrap(),
    )
}
fn links(r: &Value) -> &Value {
    r["results"][0]
        .get("adjacent")
        .unwrap_or(&r["results"][0]["metadata"]["adjacent"])
}
#[test]
fn optional_navigation_follows_source_edges_to_later_user_without_reclassifying_assistant() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let root = event(
        &v,
        "root",
        None,
        "user",
        Some(true),
        ("personal", "one"),
        true,
    );
    let assistant = event(
        &v,
        "answer",
        Some("root"),
        "assistant",
        Some(true),
        ("personal", "one"),
        false,
    );
    let next = event(
        &v,
        "next",
        Some("answer"),
        "user",
        Some(true),
        ("personal", "one"),
        false,
    );
    let plain = call(&v, &root, false, Value::Null).unwrap();
    assert!(links(&plain).is_null());
    let page = call(&v, &root, true, Value::Null).unwrap();
    assert_eq!(links(&page)["next_refs"], json!([assistant]));
    assert_eq!(links(&page)["next_user_ref"], next);
    assert_eq!(page["results"][0]["metadata"]["role"], "user");
    let detail = call(&v, &assistant, true, Value::Null).unwrap();
    assert_eq!(detail["results"][0]["record"]["role"], "assistant");
    assert_eq!(links(&detail)["previous_ref"], root);
    assert_eq!(links(&detail)["next_user_ref"], next);
    assert!(serde_json::to_vec(&page).unwrap().len() <= 2400);
}
#[test]
fn unknown_or_parallel_paths_are_choices_not_temporal_overrides() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let root = event(
        &v,
        "root",
        None,
        "assistant",
        Some(true),
        ("personal", "one"),
        true,
    );
    let old = event(
        &v,
        "other",
        Some("root"),
        "user",
        Some(false),
        ("personal", "one"),
        false,
    );
    let unknown = event(
        &v,
        "unknown",
        Some("root"),
        "user",
        None,
        ("personal", "one"),
        false,
    );
    let page = call(&v, &root, true, Value::Null).unwrap();
    let l = links(&page);
    assert_eq!(l["next_refs"], json!([]));
    assert!(l["next_user_ref"].is_null());
    assert_eq!(l["branch_choice_required"], true);
    let choices = l["branch_choices"].as_array().unwrap();
    assert!(choices.contains(&json!(old)) && choices.contains(&json!(unknown)));
}
#[test]
fn scope_session_and_hidden_intermediate_nodes_never_bridge() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let root = event(&v, "root", None, "user", None, ("personal", "one"), true);
    let mid = event(
        &v,
        "middle",
        Some("root"),
        "assistant",
        None,
        ("personal", "one"),
        false,
    );
    let next = event(
        &v,
        "next",
        Some("middle"),
        "user",
        None,
        ("personal", "one"),
        false,
    );
    let private = event(
        &v,
        "private",
        Some("root"),
        "user",
        None,
        ("project:secret", "one"),
        false,
    );
    let foreign = event(
        &v,
        "foreign",
        Some("root"),
        "user",
        None,
        ("personal", "two"),
        false,
    );
    let first = call(&v, &root, true, Value::Null).unwrap();
    assert_eq!(links(&first)["next_user_ref"], next);
    assert!(!first.to_string().contains(&private));
    assert!(!first.to_string().contains(&foreign));
    v.suppress(mid.trim_start_matches("event:"), "合成隐藏中间消息".into())
        .unwrap();
    assert!(call(&v, &root, true, first["next_cursor"].clone()).is_err());
    let latest = call(&v, &root, true, Value::Null).unwrap();
    assert_eq!(links(&latest)["next_refs"], json!([]));
    assert!(links(&latest)["next_user_ref"].is_null());
    assert!(!latest.to_string().contains(&next));
}
#[test]
fn new_capture_changes_navigation_and_invalidates_only_bound_snapshot() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let root = event(&v, "root", None, "user", None, ("personal", "one"), true);
    let old = call(&v, &root, true, Value::Null).unwrap();
    let writer = Vault::open(v.root()).unwrap();
    let new = event(
        &writer,
        "new",
        Some("root"),
        "user",
        None,
        ("personal", "one"),
        false,
    );
    assert!(call(&v, &root, true, old["next_cursor"].clone())
        .unwrap_err()
        .contains("游标"));
    assert_eq!(
        links(&call(&v, &root, true, Value::Null).unwrap())["next_user_ref"],
        new
    );
    let plain = call(&v, &root, false, Value::Null).unwrap();
    assert!(call(&v, &root, true, plain["next_cursor"].clone()).is_err());
}
#[test]
fn recapture_uses_current_revision_instead_of_treating_history_as_ambiguous() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let first = event(
        &v,
        "duplicate",
        None,
        "user",
        None,
        ("personal", "one"),
        false,
    );
    let other = event(
        &v,
        "duplicate",
        None,
        "assistant",
        None,
        ("personal", "one"),
        false,
    );
    assert_ne!(first, other);
    let child = event(
        &v,
        "child",
        Some("duplicate"),
        "user",
        None,
        ("personal", "one"),
        false,
    );
    let page = call(&v, &child, true, Value::Null).unwrap();
    assert_eq!(links(&page)["previous_ref"], other);
    assert_eq!(
        links(&call(&v, &first, true, Value::Null).unwrap())["historical_or_noncontext_source"],
        true
    );
}
#[test]
fn legacy_duplicate_provider_nodes_without_revision_edges_are_not_guessed() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let first = event(
        &v,
        "duplicate",
        None,
        "user",
        None,
        ("personal", "one"),
        false,
    );
    // 模拟旧导入的两个合法摘要记录，但缺少自动 revision_of 边。只写临时合成 vault。
    let mut second = v.event(first.trim_start_matches("event:")).unwrap();
    second.data.role = recallcard::Role::Assistant;
    second.data.content = "不同合成回答".into();
    second.data.revision_of = None;
    second.id = second.data.id().unwrap();
    second.validate().unwrap();
    let path = v.root().join("events/legacy-duplicate");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(
        path.join(format!("{}.jsonl", second.id)),
        format!("{}\n", serde_json::to_string(&second).unwrap()),
    )
    .unwrap();
    let child = event(
        &v,
        "child",
        Some("duplicate"),
        "user",
        None,
        ("personal", "one"),
        false,
    );
    assert!(links(&call(&v, &child, true, Value::Null).unwrap())["previous_ref"].is_null());
    assert_eq!(
        links(&call(&v, &first, true, Value::Null).unwrap())["ambiguous_source_identity"],
        true
    );
    // 外部编号仍歧义，但 canonical reply_to 自身指向唯一、可读的记录。
    let direct = v.capture(serde_json::from_value(json!({
        "role":"user", "origin":"native", "scope":"personal", "content":"合成明确回复",
        "reply_to":first.trim_start_matches("event:"),
        "source":{"platform":"qwen", "account_namespace":"synthetic", "conversation_id":"one", "message_id":"direct"},
        "metadata":{"qwen":{"nearest_visible_parent_id":"duplicate"}}
    })).unwrap()).unwrap();
    let direct_ref = format!("event:{}", direct.id);
    let direct_page = call(&v, &direct_ref, true, Value::Null).unwrap();
    assert_eq!(links(&direct_page)["previous_ref"], first);
    let first_page = call(&v, &first, true, Value::Null).unwrap();
    assert_eq!(links(&first_page)["ambiguous_source_identity"], true);
    assert_eq!(links(&first_page)["next_user_ref"], direct_ref);
}
#[test]
fn user_named_session_id_cannot_join_different_original_conversations() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let capture = |node: &str, conv: &str, parent: Option<&str>| {
        v.capture(serde_json::from_value(json!({"role":"user","origin":"native","scope":"personal","session_id":"shared-display-name","content":"合成来源身份","source":{"platform":"qwen","conversation_id":conv,"message_id":node},"metadata":{"qwen":{"nearest_visible_parent_id":parent}}})).unwrap()).unwrap()
    };
    let root = capture("root", "one", None);
    let foreign = capture("foreign", "two", Some("root"));
    let r = call(&v, &format!("event:{}", root.id), true, Value::Null).unwrap();
    assert_eq!(links(&r)["next_refs"], json!([]));
    assert!(!r.to_string().contains(&foreign.id));
}
#[test]
fn cycles_fail_closed_instead_of_returning_an_endless_navigation_chain() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let one = event(
        &v,
        "one",
        Some("two"),
        "user",
        None,
        ("personal", "cycle"),
        false,
    );
    event(
        &v,
        "two",
        Some("one"),
        "assistant",
        None,
        ("personal", "cycle"),
        false,
    );
    assert!(call(&v, &one, true, Value::Null)
        .unwrap_err()
        .contains("循环"));
}
#[test]
fn navigation_requires_one_event_and_keeps_memory_source_roles() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let root = event(&v, "root", None, "user", None, ("personal", "one"), false);
    let memory=v.add_memory(serde_json::from_value(json!({"content":"合成记忆","source_refs":[root.trim_start_matches("event:")],"evidence":"user_explicit"})).unwrap()).unwrap();
    assert!(
        call(&v, &format!("memory:{}", memory.id), true, Value::Null)
            .unwrap_err()
            .contains("只用于 Event")
    );
    let c = Context::new(&v, Access::new(vec!["personal".into()]).unwrap());
    assert!(c
        .read_page(
            serde_json::from_value(json!({"refs":[root,root],"include_adjacent":true})).unwrap()
        )
        .unwrap_err()
        .contains("只接受一个"));
}

#[test]
fn unknown_current_path_can_follow_unique_explicit_edges_but_stops_at_a_fork() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let root = event(&v, "root", None, "user", None, ("personal", "one"), true);
    let middle = event(
        &v,
        "middle",
        Some("root"),
        "assistant",
        None,
        ("personal", "one"),
        false,
    );
    let next = event(
        &v,
        "next",
        Some("middle"),
        "user",
        None,
        ("personal", "one"),
        false,
    );
    let page = call(&v, &root, true, Value::Null).unwrap();
    assert_eq!(links(&page)["next_refs"], json!([middle]));
    assert_eq!(links(&page)["next_user_ref"], next);
    assert!(page["results"][0]["metadata"]
        .get("on_current_path")
        .is_none());
    event(
        &v,
        "parallel",
        Some("middle"),
        "user",
        None,
        ("personal", "one"),
        false,
    );
    assert!(links(&call(&v, &root, true, Value::Null).unwrap())["next_user_ref"].is_null());
    let fork = call(&v, &middle, true, Value::Null).unwrap();
    assert_eq!(links(&fork)["next_ref_count"], 2);
    assert_eq!(links(&fork)["branch_choice_required"], true);
}
