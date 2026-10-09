//! 原创合成回归；没有真实私聊或模型向量。
use recallcard::{context::Context, policy::Access, Vault};
use serde_json::{json, Value};

fn add(
    v: &Vault,
    id: &str,
    role: &str,
    time: Option<&str>,
    path: Option<bool>,
    scope: &str,
) -> String {
    v.capture(serde_json::from_value(json!({
        "role":role,"origin":"native","scope":scope,"occurred_at":time,
        "content":"合成条件 最终方案","source":{"platform":"synthetic","conversation_id":"filter","message_id":id},
        "metadata":{"qwen":{"node_id":id,"on_current_path":path}}
    })).unwrap()).unwrap().id
}
fn query(v: &Vault, filter: Value) -> Result<Value, String> {
    Context::new(v, Access::new(vec!["personal".into()]).unwrap()).search(
        serde_json::from_value(
            json!({"query":"合成条件","event_filter":filter,"budget_bytes":16000,"limit":30}),
        )
        .unwrap(),
    )
}
fn refs(value: &Value) -> Vec<String> {
    value["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["ref"].as_str().unwrap().to_owned())
        .collect()
}
#[test]
fn explicit_role_keeps_original_speaker_and_does_not_invent_memory_role() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let u = add(&v, "u", "user", None, None, "personal");
    let a = add(&v, "a", "assistant", None, None, "personal");
    let m=v.add_memory(serde_json::from_value(json!({"content":"合成条件 最终方案","scope":"personal","source_refs":[u,a],"evidence":"assistant_suggestion"})).unwrap()).unwrap();
    let all = query(&v, Value::Null).unwrap();
    assert!(all.to_string().contains(&m.id));
    let only = query(&v, json!({"role":"user"})).unwrap();
    assert_eq!(refs(&only), vec![format!("event:{u}")]);
    assert_eq!(only["results"][0]["role"], "user");
    let assistant = query(&v, json!({"role":"assistant"})).unwrap();
    assert_eq!(refs(&assistant), vec![format!("event:{a}")]);
    assert!(!only.to_string().contains(&m.id));
    let source = Context::new(&v, Access::new(vec!["personal".into()]).unwrap())
        .sources_page(
            serde_json::from_value(
                json!({"refs":[format!("memory:{}",m.id)],"budget_bytes":16000}),
            )
            .unwrap(),
        )
        .unwrap();
    let roles = source["results"][0]["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["role"].clone())
        .collect::<Vec<_>>();
    assert_eq!(roles, vec![json!("user"), json!("assistant")]);
}
#[test]
fn explicit_inclusive_time_bounds_exclude_unknown_timestamps() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    add(&v, "unknown", "user", None, None, "personal");
    add(
        &v,
        "early",
        "user",
        Some("2026-01-01T00:00:00Z"),
        None,
        "personal",
    );
    let exact = add(
        &v,
        "exact",
        "user",
        Some("2026-02-01T00:00:00Z"),
        None,
        "personal",
    );
    add(
        &v,
        "late",
        "user",
        Some("2026-03-01T00:00:00Z"),
        None,
        "personal",
    );
    let r = query(
        &v,
        json!({"occurred_from":"2026-02-01T00:00:00Z","occurred_until":"2026-02-01T00:00:00Z"}),
    )
    .unwrap();
    assert_eq!(refs(&r), vec![format!("event:{exact}")]);
}
#[test]
fn known_current_path_never_infers_unknown_or_merges_scope() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let yes = add(&v, "yes", "user", None, Some(true), "personal");
    let no = add(&v, "no", "user", None, Some(false), "personal");
    add(&v, "unknown", "user", None, None, "personal");
    add(&v, "foreign", "user", None, Some(true), "project:private");
    assert_eq!(
        refs(&query(&v, json!({"on_current_path":true})).unwrap()),
        vec![format!("event:{yes}")]
    );
    assert_eq!(
        refs(&query(&v, json!({"on_current_path":false})).unwrap()),
        vec![format!("event:{no}")]
    );
    v.suppress(&yes, "合成隐藏".into()).unwrap();
    assert!(refs(&query(&v, json!({"on_current_path":true})).unwrap()).is_empty());
}
#[test]
fn same_context_observes_new_captures_and_hidden_sources() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let ctx = Context::new(&v, Access::new(vec!["personal".into()]).unwrap());
    let make = || {
        serde_json::from_value(json!({"query":"合成条件","target":"events","event_filter":{"role":"user"},"budget_bytes":16000})).unwrap()
    };
    assert_eq!(ctx.search(make()).unwrap()["match_count"], 0);
    let writer = Vault::open(v.root()).unwrap();
    let id = add(&writer, "new", "user", None, None, "personal");
    assert_eq!(
        refs(&ctx.search(make()).unwrap()),
        vec![format!("event:{id}")]
    );
    writer.suppress(&id, "合成撤销".into()).unwrap();
    assert_eq!(ctx.search(make()).unwrap()["match_count"], 0);
}
#[test]
fn pagination_binds_filters_even_if_the_document_set_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    add(&v, "one", "user", None, None, "personal");
    add(&v, "two", "user", None, None, "personal");
    let ctx = Context::new(&v, Access::new(vec!["personal".into()]).unwrap());
    let mut arg = json!({"query":"合成条件","target":"events","limit":1,"budget_bytes":16000});
    let page = ctx
        .search(serde_json::from_value(arg.clone()).unwrap())
        .unwrap();
    assert!(page["next_cursor"].is_string());
    arg["cursor"] = page["next_cursor"].clone();
    arg["event_filter"] = json!({"role":"user"});
    assert!(ctx
        .search(serde_json::from_value(arg).unwrap())
        .unwrap_err()
        .contains("游标已失效"));
}
#[test]
fn invalid_filters_fail_without_silent_broadening() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    assert!(query(
        &v,
        json!({"occurred_from":"2026-03-01T00:00:00Z","occurred_until":"2026-02-01T00:00:00Z"})
    )
    .is_err());
    let ctx = Context::new(&v, Access::new(vec!["personal".into()]).unwrap());
    assert!(ctx
        .search(
            serde_json::from_value(
                json!({"query":"合成条件","target":"memories","event_filter":{"role":"user"}})
            )
            .unwrap()
        )
        .unwrap_err()
        .contains("只筛选原始"));
    assert!(serde_json::from_value::<recallcard::context::SearchArgs>(
        json!({"query":"合成条件","event_filter":{"role":"owner"}})
    )
    .is_err());
    assert!(serde_json::from_value::<recallcard::context::SearchArgs>(
        json!({"query":"合成条件","event_filter":{"guess_role":true}})
    )
    .is_err());
}
#[test]
fn query_wording_does_not_automatically_switch_speaker() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let u = add(&v, "user", "user", None, None, "personal");
    let a = add(&v, "assistant", "assistant", None, None, "personal");
    let ctx = Context::new(&v, Access::new(vec!["personal".into()]).unwrap());
    let r = ctx
        .search(
            serde_json::from_value(
                json!({"query":"最终方案","target":"events","budget_bytes":16000}),
            )
            .unwrap(),
        )
        .unwrap();
    let selected = refs(&r);
    assert!(selected.contains(&format!("event:{u}")) && selected.contains(&format!("event:{a}")));
}
