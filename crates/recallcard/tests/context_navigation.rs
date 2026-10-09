//! 原创合成导航，不包含真实对话；验证结构闭环，不声称模型整理质量。
use recallcard::{
    context::{Context, ReadPageArgs},
    dream::DreamResult,
    policy::Access,
    Vault,
};
use serde_json::{json, Value};
fn source(v: &Vault, scope: &str, node: &str) -> String {
    v.capture(serde_json::from_value(json!({"role":"user","scope":scope,"origin":"native","content":"合成用户要求：实验必须离线，旧限制仍须核对。","source":{"platform":"synthetic","conversation_id":"nav","message_id":node}})).unwrap()).unwrap().id
}
fn publish(v: &Vault, scope: &str, id: &str, hints: Value) -> String {
    let job = v.dream_export(&[id.into()], &[], scope).unwrap();
    let result:DreamResult=serde_json::from_value(json!({"schema":"recallcard.dream-result/1","job_id":job.job_id,"input_hash":job.input_hash,"proposals":[{"operation":"add","scope":scope,"content":"合成实验必须离线。","source_refs":[id],"evidence":"user_explicit","navigation":hints}]})).unwrap();
    let review = v.dream_review(&result).unwrap();
    assert!(review.can_apply);
    let receipt = v.dream_apply(&result, &review.result_hash, false).unwrap();
    format!("memory:{}@1", receipt.changes[0].id)
}
fn context(v: &Vault) -> Context<'_> {
    Context::new(v, Access::new(vec!["personal".into()]).unwrap())
}
fn read(v: &Vault, r: &str, cursor: Value, budget: usize) -> Result<Value, String> {
    context(v).read_page(
        serde_json::from_value::<ReadPageArgs>(
            json!({"refs":[r],"cursor":cursor,"budget_bytes":budget}),
        )
        .unwrap(),
    )
}
#[test]
fn dream_to_generated_root_to_topic_to_memory_to_sources_is_complete_without_a_model() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let src = source(&v, "personal", "one");
    let memory = publish(
        &v,
        "personal",
        &src,
        json!([{"path":"topics/networking/rdma","title":"RDMA实验","description":"离线限制及实验要求","keywords":["RDMA","offline"],"aliases":["远程直接内存访问"],"related_paths":["projects/lab"]},{"path":"projects/lab","title":"实验项目","related_paths":["topics/networking/rdma"]}]),
    );
    assert!(v.root().join("generated/navigation.json").is_file());
    let bootstrap = context(&v).bootstrap(Default::default()).unwrap();
    assert_eq!(bootstrap["navigation_root"], "view:nav/_root");
    for (r, next) in [
        ("view:nav/_root", "view:nav/topics"),
        ("view:nav/topics", "view:nav/topics/networking"),
        (
            "view:nav/topics/networking",
            "view:nav/topics/networking/rdma",
        ),
    ] {
        let p = read(&v, r, Value::Null, 4096).unwrap();
        assert!(p["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["ref"] == next));
    }
    for r in ["view:nav/topics/networking/rdma", "view:nav/projects/lab"] {
        let p = read(&v, r, Value::Null, 4096).unwrap();
        assert!(p["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["ref"] == memory));
    }
    let item = read(&v, &memory, Value::Null, 4096).unwrap();
    assert!(item.to_string().contains("合成实验必须离线"));
    let evidence = context(&v)
        .sources_page(serde_json::from_value(json!({"refs":[memory],"budget_bytes":4096})).unwrap())
        .unwrap();
    assert!(evidence.to_string().contains(&src));
}
#[test]
fn hidden_sources_remove_all_navigation_metadata_and_old_cursors_cannot_restore_it() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let src = source(&v, "personal", "one");
    publish(
        &v,
        "personal",
        &src,
        json!([{"path":"topics/privateword","title":"合成秘密标题","description":"合成秘密描述","keywords":["合成秘密关键词"],"aliases":["合成秘密别名"]}]),
    );
    let root = read(&v, "view:nav/_root", Value::Null, 4096).unwrap();
    assert!(!root["snapshot"].is_null());
    v.suppress(&src, "合成撤销来源".into()).unwrap();
    let latest = read(&v, "view:nav/_root", Value::Null, 4096).unwrap();
    assert!(!latest.to_string().contains("privateword"));
    assert!(read(&v, "view:nav/topics/privateword", Value::Null, 4096).is_err());
    // 派生文件仍为旧快照，实时读取不能据旧文件恢复已抑制的目录。
    assert!(
        std::fs::read_to_string(v.root().join("generated/navigation.json"))
            .unwrap()
            .contains("privateword")
    );
    v.rebuild_views().unwrap();
    assert!(
        !std::fs::read_to_string(v.root().join("generated/navigation.json"))
            .unwrap()
            .contains("privateword")
    );
}
#[test]
fn unknown_fields_absent_preserve_legacy_memories_and_unfiled_is_discoverable() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let src = source(&v, "personal", "one");
    let mem = publish(&v, "personal", &src, json!([]));
    let p = read(&v, "view:nav/_unfiled", Value::Null, 4096).unwrap();
    assert_eq!(p["orphan_count"], 0);
    assert!(p.to_string().contains(&mem));
    assert!(recallcard::context::parse_ref("view:profile").is_ok());
    assert!(recallcard::context::parse_ref("view:root").is_ok());
    assert!(recallcard::context::parse_ref("view:nav/_root").is_ok());
    let m = v.memories().unwrap().pop().unwrap();
    assert!(serde_json::to_value(&m)
        .unwrap()
        .get("navigation")
        .is_none());
}

fn add(v: &Vault, src: &str, scope: &str, content: &str, hints: Value) -> String {
    v.add_memory(serde_json::from_value(json!({"content":content,"source_refs":[src],"evidence":"user_explicit","scope":scope,"navigation":hints})).unwrap()).unwrap().id
}
fn search(v: &Vault, args: Value) -> Value {
    context(v)
        .search(serde_json::from_value(args).unwrap())
        .unwrap()
}
#[test]
fn paging_is_complete_bounded_and_cursors_bind_scope_detail_and_changed_memories() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let src = source(&v, "personal", "paging");
    for i in 0..8 {
        add(
            &v,
            &src,
            "personal",
            &format!("合成记录{i}"),
            json!([{"path":format!("topic{i}"),"title":format!("主题{i}"),"description":"合成的简短导航简介"}]),
        );
    }
    let first = read(&v, "view:nav/_root", Value::Null, 1500).unwrap();
    let cursor = first["next_cursor"].clone();
    assert!(cursor.is_string());
    let mut all = vec![];
    let mut page = first;
    loop {
        assert!(serde_json::to_vec(&page).unwrap().len() <= 1500);
        all.extend(
            page["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["ref"].as_str().unwrap().to_string()),
        );
        if page["next_cursor"].is_null() {
            break;
        }
        page = read(&v, "view:nav/_root", page["next_cursor"].clone(), 1500).unwrap();
    }
    assert_eq!(all.len(), 8);
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 8);
    let other = Context::new(
        &v,
        Access::new(vec!["personal".into(), "project:extra".into()]).unwrap(),
    );
    assert!(other
        .read_page(
            serde_json::from_value(
                json!({"refs":["view:nav/_root"],"cursor":cursor,"budget_bytes":1500})
            )
            .unwrap()
        )
        .is_err());
    assert!(context(&v).read_page(serde_json::from_value(json!({"refs":["view:nav/_root"],"cursor":cursor,"budget_bytes":1500,"detail":"compact"})).unwrap()).is_err());
    add(
        &v,
        &src,
        "personal",
        "新增合成记录",
        json!([{"path":"newtopic"}]),
    );
    assert!(read(&v, "view:nav/_root", cursor, 1500)
        .unwrap_err()
        .contains("过期"));
    let tiny = read(&v, "view:nav/_root", Value::Null, 512).unwrap();
    assert_eq!(tiny["status"], "budget_exhausted");
    assert!(serde_json::to_vec(&tiny).unwrap().len() <= 512);
    let retry = read(
        &v,
        "view:nav/_root",
        Value::Null,
        tiny["recommended_min_budget"].as_u64().unwrap() as usize,
    )
    .unwrap();
    assert!(!retry["entries"].as_array().unwrap().is_empty());
}
#[test]
fn scope_projection_never_exposes_other_scope_titles_aliases_or_old_cache() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let a = source(&v, "personal", "public");
    let b = source(&v, "project:hidden", "hidden");
    add(
        &v,
        &a,
        "personal",
        "合成公开记忆",
        json!([{"path":"same/path","title":"公开标题","description":"公开简介"}]),
    );
    add(
        &v,
        &b,
        "project:hidden",
        "合成隔离内容",
        json!([{"path":"same/path","title":"不可泄露标题","description":"不可泄露简介","aliases":["不可泄露别名"],"keywords":["不可泄露关键词"]},{"path":"onlyhidden"}]),
    );
    v.rebuild_views().unwrap();
    assert!(
        std::fs::read_to_string(v.root().join("generated/navigation.json"))
            .unwrap()
            .contains("不可泄露")
    );
    for reference in ["view:nav/_root", "view:nav/same", "view:nav/same/path"] {
        let p = read(&v, reference, Value::Null, 12000).unwrap().to_string();
        assert!(!p.contains("不可泄露"));
        assert!(!p.contains("onlyhidden"));
    }
    assert!(read(&v, "view:nav/onlyhidden", Value::Null, 12000).is_err());
    let p = search(
        &v,
        json!({"query":"不可泄露","target":"views","budget_bytes":12000}),
    );
    assert!(p["results"].as_array().unwrap().is_empty());
}
#[test]
fn hints_reject_escape_reserved_names_ambiguity_depth_and_byte_overflow() {
    use recallcard::navigation::{validate, Hint};
    for path in [
        "",
        "A",
        "../secret",
        "a/../b",
        "a//b",
        "/a",
        "a/",
        "_root",
        "_unfiled",
        "labels/abc",
        "a/b/c/d/e/f/g",
        "中文",
        "a%2fb",
        "a\\b",
    ] {
        assert!(
            validate(&[Hint {
                path: path.into(),
                ..Default::default()
            }])
            .is_err(),
            "{path}"
        );
    }
    let h = Hint {
        path: "topics/ok-name_1".into(),
        ..Default::default()
    };
    assert!(validate(std::slice::from_ref(&h)).is_ok());
    assert!(validate(&[h.clone(), h.clone()]).is_err());
    let mut big = h.clone();
    big.description = "字".repeat(171);
    assert!(validate(&[big]).is_err());
    let mut big = h.clone();
    big.keywords = vec!["a".into(); 17];
    assert!(validate(&[big]).is_err());
    let mut big = h.clone();
    big.related_paths = vec!["a".into(); 9];
    assert!(validate(&[big]).is_err());
    let many = (0..8)
        .map(|i| Hint {
            path: format!("p{i}"),
            description: "d".repeat(512),
            keywords: (0..16).map(|j| format!("{j:096}")).collect(),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    assert!(validate(&many).is_err());
}
#[test]
fn related_cycles_are_non_tree_links_and_complete_hint_metadata_is_pageable() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let src = source(&v, "personal", "cycle");
    let description = "完整合成简介".repeat(20);
    add(
        &v,
        &src,
        "personal",
        "合成内容",
        json!([{"path":"a","title":"A","description":description,"aliases":["别名0","别名1","别名2","别名3"],"keywords":["词0","词1","词2","词3","词4","词5"],"related_paths":["b","notavailable"]},{"path":"b","related_paths":["a"]}]),
    );
    let mut p = read(&v, "view:nav/a", Value::Null, 2400).unwrap();
    let mut rows = vec![];
    loop {
        assert!(serde_json::to_vec(&p).unwrap().len() <= 2400);
        rows.extend(p["entries"].as_array().unwrap().clone());
        if p["next_cursor"].is_null() {
            break;
        }
        p = read(&v, "view:nav/a", p["next_cursor"].clone(), 2400).unwrap();
    }
    assert!(rows
        .iter()
        .any(|x| x["kind"] == "description" && x["text"] == description));
    assert!(rows
        .iter()
        .any(|x| x["kind"] == "alias" && x["text"] == "别名3"));
    assert!(rows
        .iter()
        .any(|x| x["kind"] == "keyword" && x["text"] == "词5"));
    assert!(rows
        .iter()
        .any(|x| x["relation"] == "related" && x["ref"] == "view:nav/b"));
    assert!(
        rows.iter()
            .any(|x| x["kind"] == "diagnostic"
                && x["text"].as_str().unwrap().contains("notavailable"))
    );
    let docs = context(&v).documents().unwrap();
    let index =
        recallcard::navigation::Index::build(&docs, &["personal".into()], chrono::Utc::now())
            .unwrap();
    assert!(index.nodes["view:nav/a"].children.is_empty());
    assert!(index.nodes["view:nav/b"].children.is_empty());
    assert!(index.orphan_refs.is_empty());
}
#[test]
fn dream_update_requires_explicit_hints_preserves_id_and_refreshes_entry() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let src = source(&v, "personal", "update");
    publish(
        &v,
        "personal",
        &src,
        json!([{"path":"oldtopic","description":"旧简介"}]),
    );
    let before = v.memories().unwrap().pop().unwrap();
    let new_src = source(&v, "personal", "update2");
    let job = v
        .dream_export(
            std::slice::from_ref(&new_src),
            std::slice::from_ref(&before.id),
            "personal",
        )
        .unwrap();
    let mut result:DreamResult=serde_json::from_value(json!({"schema":"recallcard.dream-result/1","job_id":job.job_id,"input_hash":job.input_hash,"proposals":[{"operation":"update","scope":"personal","target_ref":format!("memory:{}@1",before.id),"expected_revision":1,"content":"合成更正后限制。","source_refs":[new_src],"evidence":"user_explicit"}]})).unwrap();
    assert!(v
        .dream_review(&result)
        .unwrap_err()
        .contains("完整 navigation"));
    result.proposals[0].navigation = Some(
        serde_json::from_value(json!([{"path":"newtopic","description":"更正后简介"}])).unwrap(),
    );
    let review = v.dream_review(&result).unwrap();
    v.dream_apply(&result, &review.result_hash, false).unwrap();
    let after = v.memory(&before.id).unwrap();
    assert_eq!(after.revision, 2);
    assert!(read(&v, "view:nav/oldtopic", Value::Null, 4096).is_err());
    assert!(read(&v, "view:nav/newtopic", Value::Null, 4096)
        .unwrap()
        .to_string()
        .contains(&format!("memory:{}@2", before.id)));
    let again = v.dream_apply(&result, &review.result_hash, false).unwrap();
    assert!(again.already_applied);
    assert_eq!(v.memories().unwrap().len(), 1);
}
#[test]
fn committed_memory_remains_readable_after_derived_failure_and_retry_is_idempotent() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let src = source(&v, "personal", "fault");
    let job = v
        .dream_export(std::slice::from_ref(&src), &[], "personal")
        .unwrap();
    let result:DreamResult=serde_json::from_value(json!({"schema":"recallcard.dream-result/1","job_id":job.job_id,"input_hash":job.input_hash,"proposals":[{"operation":"add","scope":"personal","content":"合成故障恢复记录","source_refs":[src],"evidence":"user_explicit","navigation":[{"path":"recoverable"}]}]})).unwrap();
    let review = v.dream_review(&result).unwrap();
    let output = v.root().join("generated/navigation.json");
    std::fs::create_dir(&output).unwrap();
    let error = v
        .dream_apply(&result, &review.result_hash, false)
        .unwrap_err();
    assert!(error.contains("已安全发布"), "{error}");
    assert_eq!(v.memories().unwrap().len(), 1);
    assert!(read(&v, "view:nav/recoverable", Value::Null, 4096).is_ok());
    std::fs::remove_dir(&output).unwrap();
    let receipt = v.dream_apply(&result, &review.result_hash, false).unwrap();
    assert!(receipt.already_applied);
    assert_eq!(v.memories().unwrap().len(), 1);
    assert!(output.is_file());
}
#[test]
fn view_search_is_explicit_preserves_fact_channel_and_does_not_repeat_full_hints() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let src = source(&v, "personal", "search");
    for i in 0..4 {
        add(
            &v,
            &src,
            "personal",
            &format!("合成检索锚点 正文{i}"),
            json!([{"path":format!("topic{i}"),"title":"合成检索锚点","description":"描述".repeat(60),"aliases":["合成导航别名"]}]),
        );
    }
    let legacy = search(&v, json!({"query":"合成检索锚点","budget_bytes":12000}));
    assert!(legacy["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["kind"] != "view"));
    assert!(legacy["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r.get("navigation").is_none()));
    let aliases = search(
        &v,
        json!({"query":"合成导航别名","target":"views","budget_bytes":12000}),
    );
    assert_eq!(aliases["match_count"], 4);
    assert!(aliases["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["text_range"]["projection"] == "view.navigation"));
    let mixed = search(
        &v,
        json!({"query":"合成检索锚点","include_navigation":true,"budget_bytes":12000,"limit":6}),
    );
    assert_eq!(mixed["results"][0]["kind"], "memory");
    assert_eq!(mixed["results"][1]["kind"], "memory");
    assert_eq!(mixed["results"][2]["kind"], "view");
    assert_eq!(mixed["coverage"]["navigation"]["view_candidates"], 4);
    assert_eq!(mixed["coverage"]["navigation"]["fact_candidates"], 4);
    assert!(context(&v)
        .search(
            serde_json::from_value(
                json!({"query":"x","target":"views","event_filter":{"role":"user"}})
            )
            .unwrap()
        )
        .is_err());
}
#[test]
fn legacy_read_api_returns_navigable_page_and_rejects_mixed_navigation_batch() {
    use recallcard::context::ReadArgs;
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let src = source(&v, "personal", "oldread");
    add(&v, &src, "personal", "合成内容", json!([{"path":"a"}]));
    let p = context(&v)
        .read(ReadArgs {
            refs: vec!["view:nav/_root".into()],
            budget_tokens: 1500,
        })
        .unwrap();
    assert_eq!(p["entries"][0]["ref"], "view:nav/a");
    assert!(context(&v)
        .read(ReadArgs {
            refs: vec!["view:nav/_root".into(), format!("event:{src}")],
            budget_tokens: 4096
        })
        .is_err());
    assert!(context(&v)
        .sources_page(
            serde_json::from_value(json!({"refs":["view:nav/_root"],"budget_bytes":1500})).unwrap()
        )
        .is_err());
}
#[test]
fn actual_stable_text_consumer_receives_root_under_default_budget_without_writing() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let output = recallcard::agent_hook::session_start(
        &v,
        Access::new(vec!["personal".into()]).unwrap(),
        1500,
        r#"{"hook_event_name":"SessionStart","source":"startup"}"#.as_bytes(),
    )
    .unwrap();
    assert!(output["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .contains("view:nav/_root"));
    assert!(serde_json::to_vec(&output).unwrap().len() <= 1500);
    assert!(v.memories().unwrap().is_empty());
    assert!(v.events().unwrap().is_empty());
}

#[test]
fn explicit_scope_markdown_export_links_canonical_memories_and_escapes_data() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let src = source(&v, "personal", "public");
    let memory = publish(
        &v,
        "personal",
        &src,
        json!([
            {"path":"topics/net","title":"<script>alert(1)</script>","description":"[unsafe](https://example.invalid)"},
            {"path":"projects/lab","title":"实验"}
        ]),
    );
    let secret = source(&v, "work", "private");
    publish(
        &v,
        "work",
        &secret,
        json!([{"path":"topics/secret","title":"隐藏项目"}]),
    );
    let exported = v.export_navigation(vec!["personal".into()]).unwrap();
    let index = std::path::Path::new(exported["index"].as_str().unwrap());
    let root = index.parent().unwrap();
    assert!(index.exists());
    assert!(!root.join("topics/secret").exists());
    let topic = std::fs::read_to_string(root.join("topics/net/INDEX.md")).unwrap();
    assert!(!topic.contains("<script>"));
    assert!(topic.contains("&lt;script&gt;"));
    assert!(topic.contains("\\[unsafe\\]"));
    let id = memory
        .strip_prefix("memory:")
        .unwrap()
        .split('@')
        .next()
        .unwrap();
    let canonical = root.join(format!("topics/net/../../../../../memories/{id}.md"));
    assert!(canonical.exists());
    assert!(std::fs::read_to_string(root.join("projects/lab/INDEX.md"))
        .unwrap()
        .contains(id));
    assert!(!topic.contains("合成实验必须离线")); // no duplicated canonical fact
    v.suppress(&src, "撤销".into()).unwrap();
    let fresh = v.export_navigation(vec!["personal".into()]).unwrap();
    assert_ne!(fresh["index"], exported["index"]);
    assert!(!std::path::Path::new(fresh["index"].as_str().unwrap())
        .parent()
        .unwrap()
        .join("topics/net")
        .exists());
    assert!(v.export_navigation(vec![]).is_err());
}

#[test]
fn memory_only_navigation_matches_full_authorized_projection() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let src = source(&v, "personal", "visible");
    publish(&v, "personal", &src, json!([{"path":"topics/equivalence"}]));
    let hidden = source(&v, "work", "hidden");
    publish(&v, "work", &hidden, json!([{"path":"topics/hidden"}]));
    let docs = context(&v).documents().unwrap();
    let full =
        recallcard::navigation::Index::build(&docs, &["personal".into()], chrono::Utc::now())
            .unwrap();
    let page = read(&v, "view:nav/_root", Value::Null, 4096).unwrap();
    assert_eq!(page["snapshot"], full.generation);
    v.suppress(&src, "抑制".into()).unwrap();
    let docs = context(&v).documents().unwrap();
    let full =
        recallcard::navigation::Index::build(&docs, &["personal".into()], chrono::Utc::now())
            .unwrap();
    let page = read(&v, "view:nav/_root", Value::Null, 4096).unwrap();
    assert_eq!(page["snapshot"], full.generation);
    assert_eq!(full.active_memory_count, 0);
}
