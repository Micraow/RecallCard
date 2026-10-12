//! Opt-in evidence assembly tests. Invented originals and vectors, no model/gold tuning.
use recallcard::{context::Context, event_vectors, model::hash, policy::Access, transport, Vault};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

fn event(
    v: &Vault,
    id: &str,
    session: &str,
    parent: Option<&str>,
    text: &str,
    extra: Value,
) -> String {
    let mut x = json!({"role":"user","origin":"native","scope":"personal","content":text,"session_id":session,
        "occurred_at":"2026-01-02T00:00:00Z","source":{"platform":"toy","account_namespace":"owner","conversation_id":session,"message_id":id},
        "metadata":{"qwen":{"node_id":id,"parent_id":parent,"on_current_path":true}}});
    for (key, value) in extra.as_object().unwrap() {
        x[key] = value.clone();
    }
    format!(
        "event:{}",
        v.capture(serde_json::from_value(x).unwrap()).unwrap().id
    )
}
fn args(budget: usize) -> Value {
    json!({"query":"beacon","target":"events","detail":"session_evidence","limit":3,"budget_bytes":budget})
}
fn ctx(v: &Vault) -> Context<'_> {
    Context::new(v, Access::new(vec!["personal".into()]).unwrap())
}
fn call(v: &Vault, a: Value) -> Result<Value, String> {
    transport::invoke(&ctx(v), "search", a)
}
fn refs(page: &Value) -> Vec<&str> {
    page["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["ref"].as_str().unwrap())
        .collect()
}
fn space() -> Value {
    json!({"schema":"private-vector-cache-v1","endpoint":"https://embedding.invalid/v1/embeddings","model":"assembly-toy","dimensions":3})
}
fn vector(cache: &Path, text: &str, v: [f64; 3]) {
    fs::create_dir_all(cache).unwrap();
    fs::write(cache.join("metadata.json"), space().to_string()).unwrap();
    let binding=hash(b"{\"dimensions\": 3, \"endpoint\": \"https://embedding.invalid/v1/embeddings\", \"model\": \"assembly-toy\", \"schema\": \"private-vector-cache-v1\"}");
    let digest = hash(text.as_bytes());
    let mut raw = b"RCVEC001".to_vec();
    for h in [&binding, &digest] {
        for i in (0..64).step_by(2) {
            raw.push(u8::from_str_radix(&h[i..i + 2], 16).unwrap());
        }
    }
    for x in v {
        raw.extend(x.to_le_bytes());
    }
    fs::write(cache.join(format!("{digest}.bin")), raw).unwrap();
}
fn index(
    v: &Vault,
    root: &Path,
    texts: &[(String, String, [f64; 3])],
    query: &str,
) -> event_vectors::EventVectorIndex {
    let cache = root.join("cache");
    let mut chunks = vec![];
    for (reference, text, vec) in texts {
        vector(&cache, text, *vec);
        chunks.push(json!({"ref":reference,"start_byte":0,"end_byte":text.len(),"text_sha256":hash(text.as_bytes())}));
    }
    vector(&cache, query, [1., 0., 0.]);
    fs::write(
        root.join("manifest.json"),
        json!({"schema":"recallcard.event-vector-manifest/1","space":space(),"chunks":chunks})
            .to_string(),
    )
    .unwrap();
    event_vectors::import(
        v,
        Access::new(vec!["personal".into()]).unwrap(),
        &root.join("manifest.json"),
        &cache,
        &root.join("index.json"),
    )
    .unwrap();
    event_vectors::EventVectorIndex::open(&root.join("index.json"))
        .unwrap()
        .with_ranking("semantic_first")
        .unwrap()
}

#[test]
fn keeps_distant_semantic_and_lexical_locations_in_one_source_without_choosing_a_winner() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let generic = "continue the earlier idea";
    let named = "beacon latch correction was accepted";
    let a = event(&v, "m0", "one", None, generic, json!({}));
    let mut prior = "m0".to_owned();
    for n in 1..8 {
        let id = format!("m{n}");
        event(
            &v,
            &id,
            "one",
            Some(&prior),
            &format!("bridge turn {n}"),
            json!({}),
        );
        prior = id;
    }
    let b = event(
        &v,
        "m8",
        "one",
        Some(&prior),
        named,
        json!({"role":"assistant"}),
    );
    let correction = event(
        &v,
        "m9",
        "one",
        Some("m8"),
        "Only the small latch, not the large one.",
        json!({}),
    );
    let other = event(
        &v,
        "other",
        "two",
        None,
        "beacon is mentioned in another conversation",
        json!({}),
    );
    let idx = index(
        &v,
        d.path(),
        &[
            (a.clone(), generic.into(), [1., 0., 0.]),
            (b.clone(), named.into(), [0., 1., 0.]),
            (
                other.clone(),
                "beacon is mentioned in another conversation".into(),
                [0., 0., 1.],
            ),
        ],
        "beacon",
    );
    let context = ctx(&v).with_event_vectors(Some(&idx));
    let mut old_args = args(32768);
    old_args["detail"] = json!("session_discovery_compact");
    let old = transport::invoke(&context, "search", old_args).unwrap();
    let old_one = old["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["session_ref"] == "one")
        .unwrap();
    assert_eq!(old_one["review_candidate"]["anchor_ref"], a);
    assert!(!old_one["review_candidate"]["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["reference"] == b));
    let new = transport::invoke(&context, "search", args(32768)).unwrap();
    let one = new["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["session_ref"] == "one")
        .unwrap();
    assert_eq!(one["anchor_refs"], json!([a, b]));
    assert!(refs(&new).contains(&b.as_str()));
    assert!(refs(&new).contains(&correction.as_str()));
    assert!(refs(&new).contains(&other.as_str()));
    assert_eq!(new["coverage"]["relevance"], "unassessed");
    assert_eq!(
        one["source_identity"]["group_key"],
        old_one["source_identity"]["group_key"]
    );
    assert_eq!(one["channels"], old_one["channels"]);
    assert!(one.get("score").is_none());
    assert!(new.to_string().len() <= 32768);
}

#[test]
fn lexical_fallback_keeps_two_distinct_hits_and_preserves_exact_original_ranges() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let mut originals = BTreeMap::new();
    let mut previous = None;
    for n in 0..9 {
        let id = format!("m{n}");
        let text = if n == 0 {
            "beacon 第一处原文".into()
        } else if n == 8 {
            "beacon 第二处原文".into()
        } else {
            format!("连接消息 {n}")
        };
        let r = event(&v, &id, "one", previous.as_deref(), &text, json!({}));
        originals.insert(r, text);
        previous = Some(id);
    }
    let page = call(&v, args(32768)).unwrap();
    assert_eq!(
        page["sessions"][0]["anchor_refs"].as_array().unwrap().len(),
        2
    );
    for item in page["results"].as_array().unwrap() {
        let body = &originals[item["ref"].as_str().unwrap()];
        let lo = item["text_range"]["start_byte"].as_u64().unwrap() as usize;
        let hi = item["text_range"]["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(item["text"], &body[lo..hi]);
    }
}

#[test]
fn long_first_source_does_not_starve_other_sources_or_short_correction() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let giant = event(
        &v,
        "a",
        "one",
        None,
        &format!("beacon {}", "long original ".repeat(1000)),
        json!({"role":"assistant"}),
    );
    let correction = event(
        &v,
        "fix",
        "one",
        Some("a"),
        "No, use the small latch.",
        json!({}),
    );
    let two = event(
        &v,
        "b",
        "two",
        None,
        "beacon second independent source",
        json!({}),
    );
    let three = event(
        &v,
        "c",
        "three",
        None,
        "beacon third independent source",
        json!({}),
    );
    let p = call(&v, args(16384)).unwrap();
    for r in [&giant, &correction, &two, &three] {
        assert!(refs(&p).contains(&r.as_str()));
    }
    assert_eq!(p["sessions"].as_array().unwrap().len(), 3);
    assert_eq!(p["status"], "complete");
    assert_eq!(p["status_scope"], "selected_evidence_previews");
    assert!(p["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["ref"] == giant && r["text_truncated"] == true));
    assert!(p.to_string().len() <= 16384);
}

#[test]
fn cursor_visits_all_record_previews_once_and_binds_mode_budget_filter_and_revision() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let mut originals = BTreeMap::new();
    for n in 0..19 {
        let text = format!("beacon {n} {}", "甲乙丙丁修正🙂。".repeat(200));
        let reference = event(
            &v,
            &format!("m{n}"),
            "one",
            None,
            &text,
            json!({"metadata":{}}),
        );
        originals.insert(reference, text);
    }
    let mut a = args(8192);
    let first = call(&v, a.clone()).unwrap();
    let cursor = first["next_cursor"].clone();
    assert!(cursor.is_string());
    for (key, value) in [
        ("detail", json!("session_context")),
        ("budget_bytes", json!(16384)),
        ("as_of", json!("2026-02-01T00:00:00Z")),
        ("event_filter", json!({"role":"assistant"})),
    ] {
        let mut changed = a.clone();
        changed["cursor"] = cursor.clone();
        changed[key] = value;
        assert!(call(&v, changed).is_err());
    }
    let mut seen = BTreeMap::new();
    let mut page = first;
    for _ in 0..30 {
        assert!(page.to_string().len() <= 8192);
        assert_eq!(page["status_scope"], "selected_evidence_previews");
        assert_eq!(page["coverage"]["preview_only"], true);
        for item in page["results"].as_array().unwrap() {
            let reference = item["ref"].as_str().unwrap();
            let start = item["text_range"]["start_byte"].as_u64().unwrap() as usize;
            let end = item["text_range"]["end_byte"].as_u64().unwrap() as usize;
            assert_eq!(item["text"], originals[reference][start..end]);
            assert!(end - start <= 1024);
            assert_eq!(item["relation"], "anchor");
            assert!(seen.insert(reference.to_owned(), (start, end)).is_none());
        }
        assert!(page["sessions"][0]["limit_reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("source_graph_unavailable")));
        assert_eq!(
            page["sessions"][0]["anchor_refs"].as_array().unwrap().len(),
            page["results"].as_array().unwrap().len()
        );
        if page["next_cursor"].is_null() {
            break;
        }
        a["cursor"] = page["next_cursor"].clone();
        page = call(&v, a.clone()).unwrap();
    }
    assert_eq!(page["status"], "complete");
    assert_eq!(seen.len(), originals.len());
    // A complete preview stream still omits long tails. The existing reader owns them.
    let reference = originals.keys().next().unwrap();
    let read = transport::invoke(
        &ctx(&v),
        "read",
        json!({"refs":[reference],
        "detail":"compact","offset_bytes":1024,"budget_bytes":8192}),
    )
    .unwrap();
    assert!(read.to_string().contains("甲乙丙丁"));
    event(
        &v,
        "new",
        "one",
        None,
        "beacon later revision",
        json!({"metadata":{}}),
    );
    a["cursor"] = cursor;
    assert!(call(&v, a).is_err());
}

#[test]
fn same_session_label_different_source_identity_is_not_merged_and_private_scope_is_absent() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    event(&v, "a", "shared", None, "beacon owner one", json!({}));
    event(
        &v,
        "b",
        "shared",
        None,
        "beacon owner two",
        json!({"source":{"platform":"toy","account_namespace":"second","conversation_id":"shared","message_id":"b"}}),
    );
    let secret = event(
        &v,
        "hidden",
        "shared",
        None,
        "beacon hidden original",
        json!({"scope":"project:secret"}),
    );
    let p = call(&v, args(16384)).unwrap();
    assert_eq!(p["sessions"].as_array().unwrap().len(), 2);
    assert_ne!(
        p["sessions"][0]["source_identity"]["group_key"],
        p["sessions"][1]["source_identity"]["group_key"]
    );
    assert!(!p.to_string().contains(&secret));
    assert!(!p.to_string().contains("hidden original"));
}

#[test]
fn filtered_neighbor_and_unknown_sibling_are_not_brought_in_by_context_assembly() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let a = event(&v, "a", "one", None, "beacon visible anchor", json!({}));
    let assistant = event(
        &v,
        "b",
        "one",
        Some("a"),
        "assistant-only excluded body",
        json!({"role":"assistant"}),
    );
    let mut aargs = args(16384);
    aargs["event_filter"] = json!({"role":"user"});
    let p = call(&v, aargs).unwrap();
    assert!(refs(&p).contains(&a.as_str()));
    assert!(!refs(&p).contains(&assistant.as_str()));
    event(
        &v,
        "c",
        "one",
        Some("a"),
        "unselected sibling body",
        json!({}),
    );
    let p = call(&v, args(16384)).unwrap();
    assert_eq!(refs(&p), vec![a.as_str()]);
    assert!(p["sessions"].to_string().contains("branch_choice_required"));
}

#[test]
fn old_modes_and_transport_contract_remain_available_without_new_proof_fields() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    event(&v, "a", "one", None, "beacon", json!({}));
    for mode in [
        "session_context",
        "session_discovery",
        "session_discovery_compact",
    ] {
        let mut a = args(16384);
        a["detail"] = json!(mode);
        let p = call(&v, a).unwrap();
        assert!(!p
            .to_string()
            .contains("bounded_evidence_without_single_source_commitment"));
    }
    let mut wrong = args(16384);
    wrong["target"] = json!("memories");
    assert!(call(&v, wrong).is_err());
    let mut wrong = args(16384);
    wrong["preview_min_bytes"] = json!(512);
    assert!(call(&v, wrong).is_err());
    let p = call(&v, args(16384)).unwrap();
    assert!(!p.to_string().contains("review_candidate"));
    assert!(!p.to_string().contains("context_confirmation"));
}

#[test]
fn cyclic_source_keeps_only_retrieved_anchors_without_erasing_healthy_sources() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let a = event(
        &v,
        "a",
        "broken",
        Some("b"),
        "beacon cyclic first",
        json!({}),
    );
    let b = event(
        &v,
        "b",
        "broken",
        Some("a"),
        "beacon cyclic second",
        json!({}),
    );
    let tail = event(
        &v,
        "tail",
        "broken",
        Some("b"),
        "not a retrieved anchor",
        json!({}),
    );
    let ok = event(
        &v,
        "ok",
        "normal",
        None,
        "beacon normal evidence",
        json!({}),
    );
    let next = event(
        &v,
        "next",
        "normal",
        Some("ok"),
        "The verified correction.",
        json!({}),
    );
    for mode in [
        "session_discovery",
        "session_discovery_compact",
        "session_evidence",
    ] {
        let mut request = args(32768);
        request["detail"] = json!(mode);
        let page = call(&v, request).unwrap();
        assert!(page.to_string().contains("beacon normal evidence"));
        if mode == "session_evidence" {
            for reference in [&a, &b, &ok, &next] {
                assert!(refs(&page).contains(&reference.as_str()));
            }
            assert!(!refs(&page).contains(&tail.as_str()));
            let broken = page["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["session_ref"] == "broken")
                .unwrap();
            assert_eq!(broken["limit_reasons"], json!(["source_graph_cycle"]));
            assert_eq!(broken["selected_message_count"], 2);
            assert_eq!(broken["window_limited"], true);
            assert!(page["results"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["group"] == broken["group"])
                .all(|r| r["relation"] == "anchor"));
        }
    }
    let mut request = args(32768);
    request["session_ref"] = json!("broken");
    let only_broken = call(&v, request.clone()).unwrap();
    assert_ne!(only_broken["status"], "no_matches");
    assert_eq!(only_broken["results"].as_array().unwrap().len(), 2);
    request["detail"] = json!("session_context");
    assert!(call(&v, request).unwrap_err().contains("循环"));
}

#[test]
fn independent_hits_are_not_capped_by_graph_seeds_and_cross_channel_duplicates_are_removed() {
    for count in [3, 7, 17] {
        let d = tempfile::tempdir().unwrap();
        let v = Vault::init(&d.path().join("vault")).unwrap();
        let mut originals = BTreeMap::new();
        let mut vectors = Vec::new();
        for n in 0..count {
            let text = if n % 2 == 0 {
                format!("beacon record {n} {}", "large body ".repeat(700))
            } else {
                format!("independent short record {n}")
            };
            let reference = event(
                &v,
                &format!("m{n}"),
                "one",
                None,
                &text,
                json!({"metadata":{}}),
            );
            originals.insert(reference.clone(), text.clone());
            vectors.push((reference, text, [1., 0., 0.]));
        }
        let idx = index(&v, d.path(), &vectors, "beacon");
        let context = ctx(&v).with_event_vectors(Some(&idx));
        let mut request = args(8192);
        let mut seen = BTreeSet::new();
        for _ in 0..count + 1 {
            let page = transport::invoke(&context, "search", request.clone()).unwrap();
            assert!(page.to_string().len() <= 8192);
            assert_eq!(page["sessions"][0]["matching_anchor_count"], count);
            assert_eq!(
                page["sessions"][0]["graph_anchor_refs"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            for item in page["results"].as_array().unwrap() {
                let r = item["ref"].as_str().unwrap();
                assert!(seen.insert(r.to_owned()), "cross-channel duplicate {r}");
                let lo = item["text_range"]["start_byte"].as_u64().unwrap() as usize;
                let hi = item["text_range"]["end_byte"].as_u64().unwrap() as usize;
                assert_eq!(item["text"], originals[r][lo..hi]);
                assert!(hi - lo <= 1024);
                assert_eq!(item["relation"], "anchor");
                assert_eq!(item["distance"], 0);
            }
            if page["next_cursor"].is_null() {
                break;
            }
            request["cursor"] = page["next_cursor"].clone();
        }
        assert_eq!(seen.len(), count);
    }
}

#[test]
fn record_previews_share_bytes_across_sources_before_any_long_tail() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let mut expected = BTreeSet::new();
    for source in ["one", "two", "three"] {
        for n in 0..5 {
            let text = if n == 0 {
                format!("beacon {source} {}", "huge ".repeat(8000))
            } else {
                format!("beacon {source} short independent correction {n}")
            };
            expected.insert(event(
                &v,
                &format!("{source}-{n}"),
                source,
                None,
                &text,
                json!({"metadata":{}}),
            ));
        }
    }
    let page = call(&v, args(16384)).unwrap();
    assert_eq!(page["sessions"].as_array().unwrap().len(), 3);
    assert!(page.to_string().len() <= 16384);
    let mut counts = BTreeMap::new();
    let mut refs_seen = BTreeSet::new();
    for item in page["results"].as_array().unwrap() {
        assert!(expected.contains(item["ref"].as_str().unwrap()));
        assert!(refs_seen.insert(item["ref"].as_str().unwrap()));
        assert!(item["text"].as_str().unwrap().len() <= 1024);
        *counts
            .entry(item["group"].as_u64().unwrap())
            .or_insert(0usize) += 1;
    }
    assert_eq!(counts.len(), 3);
    assert!(counts.values().max().unwrap() - counts.values().min().unwrap() <= 1);
}

#[test]
fn asymmetric_sources_finish_all_independent_previews_before_graph_only_neighbors() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let mut independent = BTreeSet::new();
    for n in 0..13 {
        independent.insert(event(
            &v,
            &format!("a{n}"),
            "many",
            None,
            &format!("beacon direct record {n}"),
            json!({"metadata":{}}),
        ));
    }
    independent.insert(event(
        &v,
        "b0",
        "single",
        None,
        "beacon single hit",
        json!({}),
    ));
    let neighbor = event(
        &v,
        "b1",
        "single",
        Some("b0"),
        "The follow-up has no matching search term.",
        json!({}),
    );
    let mut request = args(8192);
    let mut order = Vec::new();
    for _ in 0..20 {
        let page = call(&v, request.clone()).unwrap();
        order.extend(refs(&page).into_iter().map(str::to_owned));
        assert_eq!(
            page["coverage"]["continuation"]["cursor_scope"],
            "selected_evidence_previews"
        );
        if page["next_cursor"].is_null() {
            break;
        }
        request["cursor"] = page["next_cursor"].clone();
    }
    assert_eq!(order.len(), independent.len() + 1);
    assert_eq!(order.last(), Some(&neighbor));
    assert!(order[..independent.len()]
        .iter()
        .all(|r| independent.contains(r)));
}
