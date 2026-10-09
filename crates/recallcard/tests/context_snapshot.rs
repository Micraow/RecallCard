//! 原创合成回归：每次读取复用本次快照，不跨请求缓存授权、隐藏状态或新来源。
use recallcard::{
    context::{Context, SearchArgs},
    policy::Access,
    Vault,
};
use serde_json::{json, Value};
fn search(v: &Vault) -> Result<Value, String> {
    Context::new(v, Access::new(vec!["personal".into()]).unwrap()).search(
        serde_json::from_value::<SearchArgs>(
            json!({"query":"青石","target":"memories","budget_bytes":16000,"limit":20}),
        )
        .unwrap(),
    )
}
fn add(v: &Vault, id: &str) -> (String, String) {
    let e = v.capture(serde_json::from_value(json!({"role":"user","origin":"native","scope":"personal","content":"青石项目采用本地存储。","source":{"platform":"synthetic","conversation_id":"snapshot","message_id":id}})).unwrap()).unwrap();
    let m = v.add_memory(serde_json::from_value(json!({"content":"青石项目采用本地存储。","source_refs":[e.id],"scope":"personal","evidence":"user_explicit"})).unwrap()).unwrap();
    (e.id, m.id)
}
#[test]
fn new_events_and_memories_are_visible_in_the_next_request() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    add(&v, "first");
    assert_eq!(search(&v).unwrap()["match_count"], 1);
    add(&v, "second");
    assert_eq!(search(&v).unwrap()["match_count"], 2);
}
#[test]
fn suppressing_a_source_hides_all_its_memories_in_next_request() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let (source, memory) = add(&v, "first");
    assert!(search(&v).unwrap().to_string().contains(&memory));
    v.suppress(&source, "合成用户隐藏来源".into()).unwrap();
    let result = search(&v).unwrap();
    assert_eq!(result["status"], "no_matches");
    assert!(!result.to_string().contains(&source));
    assert!(!result.to_string().contains(&memory));
}
#[test]
fn missing_original_source_still_fails_closed() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let (source, _) = add(&v, "first");
    assert_eq!(search(&v).unwrap()["match_count"], 1);
    // 仅把临时合成来源移出正本目录；保留目录，区分缺来源与缺目录。
    std::fs::rename(
        d.path().join("vault/events"),
        d.path().join("vault/events-test-backup"),
    )
    .unwrap();
    std::fs::create_dir(d.path().join("vault/events")).unwrap();
    let unchanged_lookup_error = v.event(&source).unwrap_err();
    assert!(unchanged_lookup_error.contains("找不到原始事件"));
    assert_eq!(search(&v).unwrap_err(), unchanged_lookup_error);
}

#[test]
fn inaccessible_scope_does_not_reveal_memory() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let (_source, memory) = add(&v, "first");
    let result = Context::new(&v, Access::new(vec!["project:synthetic".into()]).unwrap())
        .search(serde_json::from_value(json!({"query":"青石","budget_bytes":16000})).unwrap())
        .unwrap();
    assert_eq!(result["status"], "no_matches");
    assert!(!result.to_string().contains(&memory));
}

#[test]
fn legacy_memory_cannot_leak_a_cross_scope_source() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let (old_source, memory) = add(&v, "first");
    let foreign = v.capture(serde_json::from_value(json!({
        "role":"user", "origin":"native", "scope":"project:synthetic", "content":"青石保密方案。",
        "source":{"platform":"synthetic","conversation_id":"foreign","message_id":"foreign"}
    })).unwrap()).unwrap();
    // 模拟已存在的旧版不一致记录；范围检查仍需从正本 Event 读取。
    let path = d.path().join("vault/memories").join(format!("{memory}.md"));
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains(&old_source));
    std::fs::write(path, content.replace(&old_source, &foreign.id)).unwrap();
    assert_eq!(search(&v).unwrap()["status"], "no_matches");
}
