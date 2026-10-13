//! 固定原创合成诊断集；不是用户旧 gold，也不代表生产召回率。
use recallcard::{
    context::{Context, SearchArgs},
    policy::Access,
    Vault,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn setup() -> (tempfile::TempDir, Vault, BTreeMap<String, String>) {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let corpus: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/short-question/corpus.json")).unwrap();
    let refs = corpus.into_iter().map(|source| {
        let id = source["source_id"].as_str().unwrap().to_owned();
        let event = vault.capture(serde_json::from_value(json!({"scope":"personal","role":"user","origin":"native","content":source["text"],"occurred_at":"2021-04-01T00:00:00Z","source":{"platform":"synthetic","conversation_id":"short-question","message_id":id}})).unwrap()).unwrap();
        (id, format!("event:{}", event.id))
    }).collect();
    (dir, vault, refs)
}
fn args(query: &str, mode: &str) -> SearchArgs {
    serde_json::from_value(json!({"query":query,"target":"events","lexical_match":mode,"limit":20,"budget_bytes":32768})).unwrap()
}
fn context(vault: &Vault) -> Context<'_> {
    Context::new(vault, Access::new(vec!["personal".into()]).unwrap())
}
fn queries() -> Vec<Value> {
    serde_json::from_str(include_str!("fixtures/short-question/queries.json")).unwrap()
}

#[test]
fn explicit_question_mode_recovers_nine_unpunctuated_sources_without_rewriting_queries() {
    let (_dir, vault, refs) = setup();
    let context = context(&vault);
    let questions = queries()
        .into_iter()
        .filter(|q| q["category"] == "short_unpunctuated_question")
        .collect::<Vec<_>>();
    assert_eq!(questions.len(), 9);
    for query in questions {
        let text = query["query"].as_str().unwrap();
        let expected = &refs[query["expected_sources"][0].as_str().unwrap()];
        let legacy = context.search(args(text, "exact")).unwrap();
        assert_eq!(legacy["status"], "no_matches", "默认模式保持原合同：{text}");
        let question = context.search(args(text, "natural_question")).unwrap();
        assert_eq!(question["coverage"]["lexical_match"], "natural_question");
        assert_eq!(
            question["coverage"]["lexical_matching"],
            "natural_han_bigrams_exact_identifiers"
        );
        assert_eq!(question["results"][0]["ref"], *expected, "{text}");
        assert_eq!(question["match_count"], 1, "{text}");
        let punctuated = context
            .search(args(&format!("{text}？"), "natural_question"))
            .unwrap();
        assert_eq!(punctuated["results"][0]["ref"], *expected);
        assert_eq!(question["coverage"]["semantic_search"], "unavailable");
    }
}

#[test]
fn exact_intent_quotes_short_terms_and_ascii_identifiers_keep_precision_boundaries() {
    let (_dir, vault, refs) = setup();
    let c = context(&vault);
    for query in queries()
        .into_iter()
        .filter(|q| q["category"] == "exact_near_miss")
    {
        let text = query["query"].as_str().unwrap();
        assert_eq!(
            c.search(args(text, "exact")).unwrap()["status"],
            "no_matches"
        );
        assert_eq!(
            c.search(args(&format!("\"{text}\""), "natural_question"))
                .unwrap()["status"],
            "no_matches"
        );
    }
    for text in ["Cache", "CacheGates", "护照更新需要多久", "咖啡豆如何保存"] {
        assert_eq!(
            c.search(args(text, "natural_question")).unwrap()["status"],
            "no_matches",
            "{text}"
        );
    }
    for (text, source) in [
        ("CacheGate", "S10"),
        ("CacheGated", "S11"),
        ("本地备份", "S01"),
    ] {
        let result = c.search(args(text, "natural_question")).unwrap();
        assert_eq!(result["match_count"], 1);
        assert_eq!(result["results"][0]["ref"], refs[source]);
    }
    assert_eq!(
        c.search(args("MissingGate 的本地缓存怎么检查", "natural_question"))
            .unwrap()["status"],
        "no_matches"
    );
}

#[test]
fn question_excerpts_locate_deep_hits_and_mode_changes_invalidate_cursors() {
    let (_dir, vault, _) = setup();
    let long = format!("{}本地备份每周执行。", "不相关的合成段落。".repeat(1000));
    let event = vault.capture(serde_json::from_value(json!({"scope":"personal","role":"user","origin":"native","content":long,"source":{"platform":"synthetic","conversation_id":"long","message_id":"tail"}})).unwrap()).unwrap();
    let c = context(&vault);
    let result = c
        .search(args("本地备份何时执行", "natural_question"))
        .unwrap();
    let item = result["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["ref"] == format!("event:{}", event.id))
        .unwrap();
    assert!(item["text"].as_str().unwrap().contains("本地备份每周执行"));
    assert!(item["text_range"]["start_byte"].as_u64().unwrap() > 0);
    let mut first_args = args("备份", "natural_question");
    first_args.limit = 1;
    let first = c.search(first_args).unwrap();
    let cursor = first["next_cursor"].as_str().unwrap();
    for mode in ["exact", "identifier_prefix"] {
        let mut changed = args("备份", mode);
        changed.limit = 1;
        changed.cursor = Some(cursor.into());
        assert!(c.search(changed).is_err(), "不得跨模式借用游标");
    }
}

#[test]
fn questions_do_not_infer_negation_time_or_expand_scope_and_forgotten_sources() {
    let (_dir, vault, refs) = setup();
    vault.capture(serde_json::from_value(json!({"scope":"project:hidden","role":"user","origin":"native","content":"我之前怎么做本地备份：这是其他空间不可读的来源。","source":{"platform":"synthetic","conversation_id":"hidden","message_id":"hidden"}})).unwrap()).unwrap();
    let c = context(&vault);
    for query in ["本地备份不是每天执行", "本地备份现在多久执行"] {
        let result = c.search(args(query, "natural_question")).unwrap();
        let source = result["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["ref"] == refs["S01"])
            .unwrap();
        assert_eq!(source["text"], "本地备份每周执行。");
        assert_eq!(source["occurred_at"], "2021-04-01T00:00:00Z");
        assert_eq!(source["scope"], "personal");
        assert!(result["coverage"]["lexical_note"]
            .as_str()
            .unwrap()
            .contains("不等于答案"));
        assert!(!result.to_string().contains("其他空间不可读"));
    }
    vault
        .suppress(refs["S01"].trim_start_matches("event:"), "合成撤回".into())
        .unwrap();
    assert_eq!(
        c.search(args("我之前怎么做本地备份", "natural_question"))
            .unwrap()["status"],
        "no_matches"
    );
}

#[test]
fn all_search_views_disclose_question_mode_before_budgeting_and_never_call_empty_budget_no_match() {
    let (_dir, vault, _) = setup();
    let c = context(&vault);
    for detail in [
        "brief",
        "context",
        "session_candidates",
        "session_context",
        "session_discovery",
        "session_discovery_compact",
        "session_evidence",
    ] {
        for budget in [512, 1500, 8192, 32768] {
            let mut a = args("我之前怎么做本地备份", "natural_question");
            a.detail = detail.into();
            a.budget_tokens = budget;
            match c.search(a) {
                Ok(result) => {
                    assert!(
                        serde_json::to_vec(&result).unwrap().len() <= budget,
                        "{detail}/{budget}"
                    );
                    assert_eq!(
                        result["coverage"]["lexical_match"], "natural_question",
                        "{detail}/{budget}"
                    );
                    if result["results"]
                        .as_array()
                        .is_some_and(|rows| !rows.is_empty())
                    {
                        assert_eq!(
                            result["coverage"]["lexical_matching"],
                            "natural_han_bigrams_exact_identifiers",
                            "{detail}/{budget}"
                        );
                    }
                    assert_ne!(result["status"], "no_matches", "预算不足不是没有已命中来源");
                }
                Err(error) => assert!(
                    error.contains("预算") || error.contains("budget"),
                    "{detail}/{budget}: {error}"
                ),
            }
        }
    }
}

#[test]
fn standalone_western_punctuation_is_not_an_identifier_but_real_ascii_anchors_remain_required() {
    let (_dir, vault, refs) = setup();
    let c = context(&vault);
    for suffix in ["", "？", "?", ":", "：", ".", "。", " -", " /", " _"] {
        let result = c
            .search(args(
                &format!("本地备份何时执行{suffix}"),
                "natural_question",
            ))
            .unwrap();
        assert_eq!(result["results"][0]["ref"], refs["S01"], "{suffix}");
        assert_eq!(result["match_count"], 1, "{suffix}");
        assert_eq!(
            c.search(args(
                &format!("MissingGate 本地备份何时执行{suffix}"),
                "natural_question"
            ))
            .unwrap()["status"],
            "no_matches"
        );
    }
}
