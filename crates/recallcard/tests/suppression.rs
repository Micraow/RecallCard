//! 遗忘按受 scope 约束的来源身份继承，全部使用合成输入。
use recallcard::{
    context::{BootstrapArgs, Context, ReadArgs, SearchArgs},
    policy::Access,
    EventInput, MemoryInput, Vault,
};
use serde_json::json;
fn capture(v: &Vault, scope: &str, message: &str, content: &str) -> recallcard::Event {
    let input: EventInput = serde_json::from_value(json!({
        "role":"user","origin":"native","scope":scope,"content":content,
        "source":{"platform":"manual-web","account_namespace":"synthetic","conversation_id":"same","message_id":message}
    })).unwrap();
    v.capture(input).unwrap()
}
fn setup() -> (tempfile::TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    (d, v)
}
fn context(v: &Vault) -> Context<'_> {
    Context::new(v, Access::new(vec!["personal".into()]).unwrap())
}
fn memory(v: &Vault, source: &str) -> recallcard::Memory {
    v.add_memory(
        serde_json::from_value::<MemoryInput>(json!({
            "scope":"personal", "content":"合成禁止回声文本", "source_refs":[source],
            "evidence":"user_explicit", "protected":true, "labels":["bootstrap"]
        }))
        .unwrap(),
    )
    .unwrap()
}
#[test]
fn forgotten_source_covers_existing_and_future_revisions_but_not_other_sources() {
    let (_d, v) = setup();
    let old = capture(&v, "personal", "one", "合成旧稿");
    let revised = capture(&v, "personal", "one", "合成修订稿");
    let other = capture(&v, "personal", "two", "合成旧稿");
    let other_scope = capture(&v, "project:other", "one", "合成旧稿");
    v.suppress(&old.id, "合成遗忘".into()).unwrap();
    let future = capture(&v, "personal", "one", "合成后续修订");
    let suppressed = v.suppressed_ids().unwrap();
    for event in [&old, &revised, &future] {
        assert!(suppressed.contains(&event.id));
    }
    for event in [&other, &other_scope] {
        assert!(!suppressed.contains(&event.id));
    }
    v.restore(&old.id).unwrap();
    assert!(v.suppressed_ids().unwrap().is_empty());
}
#[test]
fn memory_forget_propagates_to_revision_search_read_bootstrap_embedding_and_dream() {
    let (_d, v) = setup();
    let old = capture(&v, "personal", "one", "合成禁止回声旧原文");
    let first = memory(&v, &old.id);
    v.suppress(&first.id, "合成遗忘".into()).unwrap();
    let future = capture(&v, "personal", "one", "合成禁止回声新原文");
    let second = memory(&v, &future.id);
    let c = context(&v);
    let result = c
        .search(
            serde_json::from_value::<SearchArgs>(json!({"query":"合成","budget_tokens":12000}))
                .unwrap(),
        )
        .unwrap();
    assert!(result["results"].as_array().unwrap().is_empty());
    for reference in [
        format!("event:{}", future.id),
        format!("memory:{}@1", second.id),
    ] {
        assert!(c
            .read(ReadArgs {
                refs: vec![reference],
                budget_tokens: 12000
            })
            .is_err());
    }
    assert!(!c
        .bootstrap(BootstrapArgs {
            budget_tokens: 12000
        })
        .unwrap()
        .to_string()
        .contains("禁止回声"));
    assert!(c.embedding_corpus().unwrap()["documents"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(v
        .dream_export(std::slice::from_ref(&future.id), &[], "personal")
        .is_err());
    v.restore(&first.id).unwrap();
    assert!(v.dream_export(&[future.id], &[], "personal").is_ok());
}
#[test]
fn old_rules_without_source_hashes_still_cover_new_revisions() {
    let (_d, v) = setup();
    let old = capture(&v, "personal", "one", "合成旧版本规则");
    v.suppress(&old.id, "合成遗忘".into()).unwrap();
    let path = v
        .root()
        .join("control/suppressions")
        .join(format!("{}.json", old.id));
    let mut rule: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    rule.as_object_mut().unwrap().remove("source_hashes");
    std::fs::write(path, serde_json::to_vec(&rule).unwrap()).unwrap();
    let future = capture(&v, "personal", "one", "合成新修订");
    assert!(v.is_suppressed(&future.id).unwrap());
}
#[test]
fn persisted_source_hash_survives_missing_original_without_leaking_identity_text() {
    let (_d, v) = setup();
    let old = capture(&v, "personal", "one", "合成缺失原文");
    let rule = v.suppress(&old.id, "合成遗忘".into()).unwrap();
    assert_eq!(rule.source_hashes.len(), 1);
    assert_eq!(rule.source_hashes[0].len(), 64);
    fn remove_event(dir: &std::path::Path, id: &str) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                remove_event(&path, id)
            } else if path.file_stem().and_then(|s| s.to_str()) == Some(id) {
                std::fs::remove_file(path).unwrap();
            }
        }
    }
    remove_event(&v.root().join("events"), &old.id);
    let future = capture(&v, "personal", "one", "合成重新捕获");
    assert!(v.is_suppressed(&future.id).unwrap());
}
#[test]
fn corrupt_suppression_hash_or_filename_fails_closed() {
    let (_d, v) = setup();
    let old = capture(&v, "personal", "one", "合成损坏规则");
    v.suppress(&old.id, "合成遗忘".into()).unwrap();
    let path = v
        .root()
        .join("control/suppressions")
        .join(format!("{}.json", old.id));
    let mut rule: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    rule["source_hashes"] = json!(["not-a-digest"]);
    std::fs::write(&path, serde_json::to_vec(&rule).unwrap()).unwrap();
    assert!(v.suppressed_ids().is_err());
    rule["source_hashes"] = json!([]);
    std::fs::write(&path, serde_json::to_vec(&rule).unwrap()).unwrap();
    std::fs::rename(&path, path.with_file_name("wrong.json")).unwrap();
    assert!(v.suppressed_ids().is_err());
}
