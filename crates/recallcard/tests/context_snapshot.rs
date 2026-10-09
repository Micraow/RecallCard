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
    let context = Context::new(&v, Access::new(vec!["personal".into()]).unwrap());
    for result in reads(&context, &format!("memory:{memory}")) {
        assert_eq!(result.unwrap_err(), "引用不可访问或已被抑制");
    }
}

fn reads(context: &Context<'_>, reference: &str) -> Vec<Result<Value, String>> {
    let args = json!({"refs":[reference],"budget_bytes":16000});
    vec![
        context.read(serde_json::from_value(args.clone()).unwrap()),
        context.sources(serde_json::from_value(args.clone()).unwrap()),
        context.read_page(serde_json::from_value(args.clone()).unwrap()),
        context.sources_page(serde_json::from_value(args).unwrap()),
    ]
}

#[test]
fn same_context_read_requests_observe_external_capture_and_suppression() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let context = Context::new(&vault, Access::new(vec!["personal".into()]).unwrap());
    let (_, first) = add(&vault, "first");
    assert!(reads(&context, &format!("memory:{first}"))
        .iter()
        .all(Result::is_ok));
    let external = Vault::open(vault.root()).unwrap();
    let (source, second) = add(&external, "second");
    let reference = format!("memory:{second}");
    assert!(reads(&context, &reference).iter().all(Result::is_ok));
    external.suppress(&source, "合成外部隐藏".into()).unwrap();
    for result in reads(&context, &reference) {
        assert_eq!(result.unwrap_err(), "引用不可访问或已被抑制");
    }
    assert!(reads(&context, &format!("memory:{first}"))
        .iter()
        .all(Result::is_ok));
}

#[test]
fn all_read_paths_keep_missing_source_error_and_scope_short_circuit() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let (source, memory) = add(&vault, "missing");
    std::fs::rename(
        vault.root().join("events"),
        vault.root().join("events-backup"),
    )
    .unwrap();
    std::fs::create_dir(vault.root().join("events")).unwrap();
    let context = Context::new(&vault, Access::new(vec!["personal".into()]).unwrap());
    let expected = vault.event(&source).unwrap_err();
    for reference in [format!("event:{source}"), format!("memory:{memory}")] {
        for result in reads(&context, &reference) {
            assert_eq!(result.unwrap_err(), expected);
        }
    }
    let denied = Context::new(
        &vault,
        Access::new(vec!["project:elsewhere".into()]).unwrap(),
    );
    for result in reads(&denied, &format!("memory:{memory}")) {
        assert_eq!(result.unwrap_err(), "引用不可访问或已被抑制");
    }
}

#[test]
fn source_lists_keep_declared_order_in_paged_and_multi_reference_reads() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let (one, _) = add(&vault, "one");
    let (two, _) = add(&vault, "two");
    let memory = vault.add_memory(serde_json::from_value(json!({
        "content":"两处合成依据", "source_refs":[two,one], "scope":"personal", "evidence":"user_explicit"
    })).unwrap()).unwrap();
    let expected = vault.sources(&memory.id).unwrap();
    let context = Context::new(&vault, Access::new(vec!["personal".into()]).unwrap());
    let reference = format!("memory:{}", memory.id);
    for refs in [
        vec![reference.clone()],
        vec![reference.clone(), reference.clone()],
    ] {
        let result = context
            .sources_page(
                serde_json::from_value(json!({"refs":refs,"budget_bytes":16000})).unwrap(),
            )
            .unwrap();
        for item in result["results"].as_array().unwrap() {
            assert_eq!(item["events"], serde_json::to_value(&expected).unwrap());
        }
    }
}

#[test]
fn duplicate_canonical_id_fails_before_any_snapshot_can_collapse_it() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let (source, memory) = add(&vault, "duplicate");
    let event = vault.event(&source).unwrap();
    let job = vault
        .dream_export(std::slice::from_ref(&source), &[], "personal")
        .unwrap();
    let result = serde_json::from_value(json!({
        "schema":"recallcard.dream-result/1", "job_id":job.job_id, "input_hash":job.input_hash,
        "proposals":[{"operation":"add","scope":"personal","content":"合成重复来源检查","source_refs":[source],"evidence":"user_explicit"}]
    })).unwrap();
    let mut duplicate = serde_json::to_vec(&event).unwrap();
    duplicate.push(b'\n');
    std::fs::write(vault.root().join("events/duplicate.jsonl"), duplicate).unwrap();
    assert_eq!(vault.event(&source).unwrap_err(), "事件编号重复");
    assert_eq!(search(&vault).unwrap_err(), "事件编号重复");
    let context = Context::new(&vault, Access::new(vec!["personal".into()]).unwrap());
    for result in reads(&context, &format!("memory:{memory}")) {
        assert_eq!(result.unwrap_err(), "事件编号重复");
    }
    assert_eq!(vault.dream_review(&result).unwrap_err(), "事件编号重复");
}
