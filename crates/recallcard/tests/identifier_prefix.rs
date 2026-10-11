//! Artificial identifiers only; no real user corpus or known-answer replay.
use recallcard::{context::Context, policy::Access, transport, Vault};
use serde_json::{json, Value};
fn setup() -> (tempfile::TempDir, Vault) {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    (d, v)
}
fn add(v: &Vault, id: &str, text: &str, scope: &str, role: &str) -> String {
    v.capture(serde_json::from_value(json!({"scope":scope,"role":role,"origin":"native","content":text,"occurred_at":"2026-01-01T00:00:00Z","source":{"platform":"synthetic-prefix","conversation_id":"test","message_id":id}})).unwrap()).unwrap().id
}
fn search(v: &Vault, q: &str, mode: Option<&str>, extra: Value) -> Value {
    let mut a = json!({"query":q,"target":"events","budget_bytes":16000,"limit":50});
    if let Some(m) = mode {
        a["lexical_match"] = json!(m);
    }
    for (k, x) in extra.as_object().unwrap() {
        a[k] = x.clone();
    }
    transport::invoke(
        &Context::new(v, Access::new(vec!["personal".into()]).unwrap()),
        "search",
        a,
    )
    .unwrap()
}
fn refs(v: &Value) -> Vec<String> {
    v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["ref"].as_str().unwrap().into())
        .collect()
}
#[test]
fn default_exact_and_explicit_prefix_preserve_left_boundary() {
    let (_d, v) = setup();
    let exact = add(&v, "a", "Nova", "personal", "user");
    let prefix = add(&v, "b", "NovaEngine", "personal", "user");
    add(&v, "c", "SuperNovaEngine", "personal", "user");
    add(&v, "d", "_NovaEngine", "personal", "user");
    assert_eq!(
        refs(&search(&v, "Nova", None, json!({}))),
        vec![format!("event:{exact}")]
    );
    let r = refs(&search(&v, "nOvA", Some("identifier_prefix"), json!({})));
    assert_eq!(r, vec![format!("event:{exact}"), format!("event:{prefix}")]);
}
#[test]
fn short_numeric_code_and_han_terms_do_not_gain_prefix_expansion() {
    let (_d, v) = setup();
    for (id, t) in [
        ("a", "NovaEngine"),
        ("b", "123456"),
        ("c", "cfg_nameLong"),
        ("d", "数据库模型"),
    ] {
        add(&v, id, t, "personal", "user");
    }
    for q in ["No", "123", "cfg_name", "数据库"] {
        assert_eq!(
            refs(&search(&v, q, None, json!({}))),
            refs(&search(&v, q, Some("identifier_prefix"), json!({})))
        );
    }
}
#[test]
fn exact_priority_survives_repeated_prefix_evidence_and_no_duplicates() {
    let (_d, v) = setup();
    let exact = add(&v, "a", "Nova one", "personal", "assistant");
    add(&v, "b", &"NovaEngine ".repeat(40), "personal", "user");
    let r = search(&v, "Nova", Some("identifier_prefix"), json!({}));
    assert_eq!(r["results"][0]["ref"], format!("event:{exact}"));
    let x = refs(&r);
    assert_eq!(
        x.len(),
        x.iter().collect::<std::collections::BTreeSet<_>>().len()
    );
    assert_eq!(
        x,
        refs(&search(&v, "Nova", Some("identifier_prefix"), json!({})))
    );
}
#[test]
fn permissions_suppression_role_and_time_stay_filtered() {
    let (_d, v) = setup();
    let good = add(&v, "a", "NovaEngine", "personal", "user");
    add(&v, "b", "NovaEngine", "project:hidden", "user");
    add(&v, "c", "NovaEngine", "personal", "assistant");
    let hidden = add(&v, "d", "NovaEngine", "personal", "user");
    v.suppress(&hidden, "test".into()).unwrap();
    let r = search(
        &v,
        "Nova",
        Some("identifier_prefix"),
        json!({"event_filter":{"role":"user","occurred_from":"2025-12-31T00:00:00Z","occurred_until":"2026-01-02T00:00:00Z"}}),
    );
    assert_eq!(refs(&r), vec![format!("event:{good}")]);
    assert!(refs(&search(
        &v,
        "Nova",
        Some("identifier_prefix"),
        json!({"event_filter":{"occurred_from":"2026-02-01T00:00:00Z"}})
    ))
    .is_empty());
    assert!(refs(&search(
        &v,
        "Nova",
        Some("identifier_prefix"),
        json!({"session_ref":"ses_nonexistent"})
    ))
    .is_empty());
}
#[test]
fn pagination_binds_mode_and_has_stable_nonduplicate_pages() {
    let (_d, v) = setup();
    for (i, t) in ["Nova one", "Nova two", "NovaEngine"].iter().enumerate() {
        add(&v, &i.to_string(), t, "personal", "user");
    }
    let first = search(&v, "Nova", Some("identifier_prefix"), json!({"limit":1}));
    let cursor = first["next_cursor"].clone();
    assert!(cursor.is_string());
    let second = search(
        &v,
        "Nova",
        Some("identifier_prefix"),
        json!({"limit":1,"cursor":cursor}),
    );
    assert_ne!(refs(&first), refs(&second));
    let c = Context::new(&v, Access::new(vec!["personal".into()]).unwrap());
    assert!(transport::invoke(&c,"search",json!({"query":"Nova","target":"events","lexical_match":"exact","cursor":cursor,"limit":1,"budget_bytes":16000})).is_err());
}
#[test]
fn schema_rejects_unknown_mode_and_preserves_legacy_budget_alias() {
    let (_d, v) = setup();
    add(&v, "a", "NovaEngine", "personal", "user");
    let c = Context::new(&v, Access::new(vec!["personal".into()]).unwrap());
    assert!(transport::invoke(
        &c,
        "search",
        json!({"query":"Nova","lexical_match":"substring"})
    )
    .is_err());
    let r = transport::invoke(
        &c,
        "search",
        json!({"query":"Nova","lexical_match":"identifier_prefix","budget_tokens":4096}),
    )
    .unwrap();
    assert_eq!(refs(&r).len(), 1);
    let defs = transport::tool_definitions();
    let tool = defs
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "search")
        .unwrap();
    assert_eq!(
        tool["inputSchema"]["properties"]["lexical_match"]["default"],
        "exact"
    );
}
#[test]
fn cli_exposes_opt_in_without_changing_default() {
    let (d, v) = setup();
    add(&v, "a", "NovaEngine", "personal", "user");
    let run = |prefix: bool| {
        let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_recallcard"));
        c.args([
            "--vault",
            d.path().to_str().unwrap(),
            "--json",
            "search",
            "Nova",
            "--budget-bytes",
            "4096",
        ]);
        if prefix {
            c.args(["--lexical-match", "identifier_prefix"]);
        }
        let o = c.output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        serde_json::from_slice::<Value>(&o.stdout).unwrap()
    };
    assert!(refs(&run(false)).is_empty());
    assert_eq!(refs(&run(true)).len(), 1);
}
#[test]
fn expansion_cap_is_explicit_and_exact_results_survive() {
    let (_d, v) = setup();
    let inputs=(0..260).map(|i|serde_json::from_value(json!({"scope":"personal","role":"user","origin":"native","content":format!("NovaEngine{i}"),"source":{"platform":"synthetic-prefix","conversation_id":"cap","message_id":i.to_string()}})).unwrap()).collect();
    v.capture_batch(inputs).unwrap();
    let exact = add(&v, "exact", "Nova", "personal", "user");
    let r = search(&v, "Nova", Some("identifier_prefix"), json!({"limit":1}));
    assert_eq!(r["results"][0]["ref"], format!("event:{exact}"));
    assert_eq!(r["coverage"]["prefix_candidates_truncated"], true);
    assert_eq!(r["match_count"], 257);
}
#[test]
fn natural_chinese_query_is_not_reinterpreted_as_prefix_expansion() {
    let (_d, v) = setup();
    add(
        &v,
        "a",
        "数据库备份在本地保存，旧版本不要覆盖。",
        "personal",
        "user",
    );
    add(
        &v,
        "b",
        "数据库备份覆盖了旧版本，需要恢复。",
        "personal",
        "user",
    );
    let q = "数据库备份应该如何保存旧版本？";
    assert_eq!(
        refs(&search(&v, q, None, json!({}))),
        refs(&search(&v, q, Some("identifier_prefix"), json!({})))
    );
}
#[test]
fn prefix_responses_keep_byte_budget_and_paging_contract() {
    let (_d, v) = setup();
    for i in 0..6 {
        add(
            &v,
            &i.to_string(),
            &format!("NovaEngine{i} {}", "bounded original text ".repeat(60)),
            "personal",
            "user",
        );
    }
    for budget in [512, 1024, 2048, 4096] {
        let r = search(
            &v,
            "Nova",
            Some("identifier_prefix"),
            json!({"budget_bytes":budget}),
        );
        assert!(serde_json::to_vec(&r).unwrap().len() <= budget);
        if r["budget_exhausted"] == true {
            assert!(r["recommended_min_budget"].as_u64().unwrap() > budget as u64);
        }
    }
}
