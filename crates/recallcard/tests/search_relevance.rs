//! 公开合成等价例；不包含实际验收资料、身份、项目名或请求原文。
use recallcard::{
    context::{Context, ReadPageArgs, SearchArgs},
    policy::Access,
    Vault,
};
use serde_json::{json, Value};
fn setup() -> (tempfile::TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    (d, v)
}
fn event(v: &Vault, id: &str, role: &str, text: &str) -> String {
    v.capture(serde_json::from_value(json!({"scope":"personal","role":role,"origin":"native","content":text,"occurred_at":"2026-10-01T10:00:00Z","source":{"platform":"synthetic","conversation_id":"relevance","message_id":id}})).unwrap()).unwrap().id
}
fn c(v: &Vault) -> Context<'_> {
    Context::new(v, Access::new(vec!["personal".into()]).unwrap())
}
fn args(query: &str, budget: usize) -> SearchArgs {
    serde_json::from_value(json!({"query":query,"budget_bytes":budget,"limit":10})).unwrap()
}
fn query(v: &Vault, q: &str) -> Value {
    c(v).search(args(q, 12000)).unwrap()
}
#[test]
fn short_han_words_require_the_word_not_an_overlapping_character() {
    let (_d, v) = setup();
    event(&v, "wrong", "user", "图片预览完成，等待最终图案。");
    let id = event(&v, "right", "user", "本期预算为六十元。");
    for term in ["预算", "算为"] {
        let r = query(&v, term);
        assert_eq!(r["match_count"], 1);
        assert_eq!(r["results"][0]["ref"], format!("event:{id}"));
    }
    assert_eq!(
        query(&v, "预")["match_count"],
        2,
        "确实单独查询一个字时允许单字搜索"
    );
}
#[test]
fn every_separate_keyword_must_match_in_chinese_english_and_mixed_queries() {
    let (_d, v) = setup();
    event(&v, "a", "user", "星砂方案的预览已经完成。");
    event(&v, "b", "user", "其他方案的预算待定。");
    let exact = event(&v, "c", "user", "星砂方案预算决定使用 Cache storage。");
    event(&v, "d", "user", "Cached storage 位于其他目录。");
    for term in ["星砂 预算", "Cache STORAGE", "星砂 cache", "CACHE"] {
        let r = query(&v, term);
        assert_eq!(r["match_count"], 1, "{term}: {r}");
        assert_eq!(r["results"][0]["ref"], format!("event:{exact}"));
    }
    for term in ["星砂 开支", "Cache missing"] {
        assert_eq!(query(&v, term)["status"], "no_matches");
    }
    for term in ["", "   ", "！？"] {
        assert!(c(&v).search(args(term, 2000)).is_err());
    }
}
#[test]
fn relevant_user_fact_precedes_tentative_advice_but_advice_query_remains_available() {
    let (_d, v) = setup();
    let user = event(&v, "u", "user", "检索模块最终选择青松方案。");
    let assistant = event(
        &v,
        "a",
        "assistant",
        "检索模块可以考虑赤松方案，我建议先做原型。",
    );
    let mem=v.add_memory(serde_json::from_value(json!({"content":"检索模块建议考虑赤松方案。","source_refs":[assistant],"evidence":"assistant_suggestion"})).unwrap()).unwrap();
    let r = query(&v, "检索模块");
    assert_eq!(r["results"][0]["ref"], format!("event:{user}"));
    let ideas = query(&v, "检索模块 建议");
    assert!(ideas["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["ref"] == format!("memory:{}@1", mem.id)));
    assert!(!ideas["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["ref"] == format!("event:{user}")));
}
#[test]
fn matching_memory_groups_its_source_but_different_memories_remain_discoverable() {
    let (_d, v) = setup();
    let source = event(&v, "u", "user", "存储选择松石格式。存储更新每月运行。");
    let a=v.add_memory(serde_json::from_value(json!({"content":"存储采用松石格式。","source_refs":[source],"evidence":"user_explicit"})).unwrap()).unwrap();
    let b=v.add_memory(serde_json::from_value(json!({"content":"存储每月更新。","source_refs":[source],"evidence":"user_explicit"})).unwrap()).unwrap();
    let r = query(&v, "存储");
    assert_eq!(r["match_count"], 2);
    assert!(r["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|x| x["kind"] == "memory"));
    let refs = r["results"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|x| x["related_refs"].as_array().cloned().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(refs, vec![json!(format!("event:{source}"))]);
    let history = c(&v)
        .sources_page(
            serde_json::from_value(
                json!({"refs":[format!("memory:{}@1",a.id)],"budget_bytes":12000}),
            )
            .unwrap(),
        )
        .unwrap();
    assert!(history.to_string().contains(&source));
    v.set_state(&b.id, 1, recallcard::MemoryState::Superseded)
        .unwrap();
    assert_eq!(query(&v, "每月")["results"][0]["kind"], "event");
    let mut old = args("每月", 12000);
    old.as_of = Some("2026-10-01T12:00:00Z".parse().unwrap());
    assert!(c(&v)
        .search(old)
        .unwrap()
        .to_string()
        .contains("superseded"));
}
#[test]
fn compact_first_page_shares_space_and_cursor_covers_every_distinct_match() {
    let (_d, v) = setup();
    for i in 0..7 {
        event(
            &v,
            &i.to_string(),
            if i % 2 == 0 { "assistant" } else { "user" },
            &format!("灯塔专题 {} 条。{}", i, "更多合成说明。".repeat(500)),
        );
    }
    let mut a = args("灯塔专题", 4096);
    let mut seen = std::collections::BTreeSet::new();
    let mut first = true;
    loop {
        let r = c(&v).search(a.clone()).unwrap();
        assert!(serde_json::to_vec(&r).unwrap().len() <= 4096);
        if first {
            assert!(r["results"].as_array().unwrap().len() >= 3);
            first = false;
        }
        for item in r["results"].as_array().unwrap() {
            assert!(seen.insert(item["ref"].as_str().unwrap().to_owned()));
            assert_eq!(item["text_truncated"], true);
        }
        if r["next_cursor"].is_null() {
            break;
        }
        a.cursor = r["next_cursor"].as_str().map(str::to_owned);
    }
    assert_eq!(seen.len(), 7);
}

#[test]
fn weak_label_summary_preserves_its_strong_source_and_remains_pageable() {
    let (_dir, vault) = setup();
    let source = event(&vault, "beacon-choice", "user", "航标计划选择琥珀格式。");
    for index in 0..8 {
        event(
            &vault,
            &format!("beacon-advice-{index}"),
            "assistant",
            "航标计划可以考虑其他格式，这只是等待讨论的建议。",
        );
    }
    let memory = vault
        .add_memory(
            serde_json::from_value(json!({
                "content":"采用琥珀格式。", "source_refs":[source],
                "labels":["航标计划资料"], "evidence":"user_explicit"
            }))
            .unwrap(),
        )
        .unwrap();
    let grouped = query(&vault, "航标计划");
    let first = &grouped["results"][0];
    assert_eq!(grouped["match_count"], 9);
    assert_eq!(first["ref"], format!("memory:{}@1", memory.id));
    assert_eq!(first["score_source_ref"], format!("event:{source}"));
    assert_eq!(first["related_refs"][0], format!("event:{source}"));
    assert_eq!(first["evidence"], "UserExplicit");
    assert_eq!(first["text"], "采用琥珀格式。");
    let mut raw = args("航标计划", 12000);
    raw.target = "events".into();
    assert_eq!(
        c(&vault).search(raw).unwrap()["results"][0]["ref"],
        format!("event:{source}")
    );
    let mut paged = args("航标计划", 2200);
    paged.limit = 2;
    let mut seen = std::collections::BTreeSet::new();
    loop {
        let page = c(&vault).search(paged.clone()).unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() <= 2200);
        for record in page["results"].as_array().unwrap() {
            assert!(seen.insert(record["ref"].as_str().unwrap().to_owned()));
        }
        if page["next_cursor"].is_null() {
            break;
        }
        paged.cursor = page["next_cursor"].as_str().map(str::to_owned);
    }
    assert_eq!(seen.len(), 9);
    vault
        .suppress(&source, "合成样本来源已隐藏".into())
        .unwrap();
    let hidden = query(&vault, "航标计划");
    assert_eq!(hidden["match_count"], 8);
    assert!(!hidden.to_string().contains(&source));
    assert!(!hidden.to_string().contains(&memory.id));
}
#[test]
fn exhausted_budget_supplies_a_working_retry_size_and_no_matches_stays_distinct() {
    let (_d, v) = setup();
    let source = event(
        &v,
        "u",
        "user",
        &format!("林地样本 {}", "资料".repeat(3000)),
    );
    let r = c(&v).search(args("林地样本", 512)).unwrap();
    assert_eq!(r["status"], "budget_exhausted");
    let n = r["recommended_min_budget"].as_u64().unwrap() as usize;
    assert!(n > 512 && n <= 32768);
    assert_eq!(
        c(&v).search(args("林地样本", n)).unwrap()["results"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        c(&v).search(args("未出现词", 512)).unwrap()["status"],
        "no_matches"
    );
    let reference = format!("event:{source}");
    let r = c(&v)
        .read_page(
            serde_json::from_value::<ReadPageArgs>(json!({"refs":[reference],"budget_bytes":512}))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(r["status"], "budget_exhausted");
    let n = r["recommended_min_budget"].as_u64().unwrap();
    let good = c(&v)
        .read_page(
            serde_json::from_value::<ReadPageArgs>(json!({"refs":[reference],"budget_bytes":n}))
                .unwrap(),
        )
        .unwrap();
    assert!(!good["results"].as_array().unwrap().is_empty());
}
