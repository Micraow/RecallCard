//! Independent, synthetic compact candidate review contracts. No private fixtures,
//! model-quality assertions, network access, or relevance judgments are used here.
use recallcard::{context::Context, model::hash, policy::Access, transport, Vault};
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn event(v: &Vault, id: &str, session: &str, text: &str, extra: Value) -> String {
    let mut value = json!({
        "role":"user", "origin":"native", "scope":"personal", "content":text,
        "session_id":session, "occurred_at":"2026-01-02T00:00:00Z",
        "source":{"platform":"compact-toy","account_namespace":"owner",
                  "conversation_id":session,"message_id":id},
        "metadata":{"qwen":{"node_id":id,"parent_id":null,"on_current_path":true}}
    });
    for (key, field) in extra.as_object().unwrap() {
        value[key] = field.clone();
    }
    format!(
        "event:{}",
        v.capture(serde_json::from_value(value).unwrap())
            .unwrap()
            .id
    )
}
fn context(v: &Vault) -> Context<'_> {
    Context::new(v, Access::new(vec!["personal".into()]).unwrap())
}
fn search_args() -> Value {
    json!({"query":"beacon", "target":"events", "detail":"session_discovery_compact",
           "limit":3,"budget_bytes":32768})
}
fn discover(v: &Vault, args: Value) -> Value {
    transport::invoke(&context(v), "search", args).unwrap()
}
fn review_args(page: &Value, item: usize, stage: &str) -> Value {
    let candidate = &page["results"][item]["review_candidate"];
    json!({
        "refs":[candidate["anchor_ref"]], "include_adjacent":true,
        "context_anchor_ref":candidate["anchor_ref"], "detail":"compact",
        "event_filter":page["review_template"]["event_filter"],
        "as_of":page["review_template"]["as_of"], "budget_bytes":8192,
        "context_review":{
            "stage":stage,"template":page["review_template"],"candidate":candidate,
            "assessment":{"decision":"uncertain","selected_anchor_ref":null,"evidence_indices":[]}
        }
    })
}
fn assessed(mut args: Value) -> Value {
    args["context_review"]["assessment"] = json!({
        "decision":"relevant", "selected_anchor_ref":args["context_anchor_ref"],
        "evidence_indices":[0]
    });
    args
}
fn rehash(args: &mut Value) {
    let review = &mut args["context_review"];
    review["candidate"]["packet_hash"] = json!("");
    review["candidate"]["packet_hash"] = json!(hash(
        json!([review["template"], review["candidate"]])
            .to_string()
            .as_bytes()
    ));
}
fn fails_closed(ctx: &Context<'_>, args: Value) {
    if let Ok(result) = transport::invoke(ctx, "read", args) {
        assert_eq!(result["deep_read_performed"], false, "{result}");
        assert!(result["results"].as_array().unwrap().is_empty(), "{result}");
        assert!(result["updated_candidate"].is_null(), "{result}");
        assert!(result["next_cursor"].is_null(), "{result}");
    }
}
fn span_text<'a>(v: &Vault, span: &'a Value) -> &'a str {
    let original = v
        .event(
            span["reference"]
                .as_str()
                .unwrap()
                .trim_start_matches("event:"),
        )
        .unwrap()
        .data
        .text();
    let start = span["start_byte"].as_u64().unwrap() as usize;
    let end = span["end_byte"].as_u64().unwrap() as usize;
    let text = span["text"].as_str().unwrap();
    assert_eq!(span["total_bytes"], original.len());
    assert!(start < end && end <= original.len());
    assert_eq!(original.get(start..end), Some(text));
    assert_eq!(text.len(), end - start);
    assert!(text.len() <= 512);
    assert!(span["snapshot"].is_string());
    assert!(span["role"].is_string());
    assert!(span["occurred_at"].is_string());
    text
}
fn inspected(v: &Vault, args: Value) -> Value {
    let result = transport::invoke(&context(v), "read", args).unwrap();
    assert_eq!(result["deep_read_performed"], false, "{result}");
    assert_eq!(result["relevance"], "unresolved", "{result}");
    // Inspection has exactly one evidence copy, never a second full-text result.
    assert!(result["results"].as_array().unwrap().is_empty(), "{result}");
    let evidence = result["updated_candidate"]["evidence"].as_array().unwrap();
    assert_eq!(evidence.len(), 1);
    span_text(v, &evidence[0]);
    assert!(result["navigation"].is_object());
    assert!(result["navigation"].get("next_user_ref").is_none());
    result
}

#[test]
fn compact_packet_has_one_template_and_one_copy_of_each_original_span() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let previous = event(
        &v,
        "previous",
        "topic",
        "Earlier explicit constraint.",
        json!({}),
    );
    let anchor = event(
        &v,
        "anchor",
        "topic",
        "beacon proposed alternative.",
        json!({"role":"assistant","reply_to":previous.trim_start_matches("event:")}),
    );
    let next = event(
        &v,
        "next",
        "topic",
        "Later correction: keep the earlier constraint.",
        json!({"reply_to":anchor.trim_start_matches("event:")}),
    );
    let page = discover(&v, search_args());
    assert!(page["review_template"].is_object());
    assert_eq!(page["relevance"], "unassessed");
    let item = &page["results"][0];
    assert!(item["channels"].is_object());
    assert!(item["source_identity"].is_object());
    for absent in ["context", "read_args", "review_template"] {
        assert!(
            item.get(absent).is_none(),
            "duplicate field {absent}: {item}"
        );
    }
    let candidate = &item["review_candidate"];
    assert_eq!(candidate["anchor_ref"], anchor);
    let id = candidate["candidate_id"].as_str().unwrap();
    assert_eq!(id.len(), 32);
    assert!(id.bytes().all(|c| c.is_ascii_hexdigit()));
    let expected = hash(
        json!([
            "recallcard.candidate-id/1",
            candidate["source_group_key"],
            candidate["anchor_ref"],
            candidate["anchor_snapshot"]
        ])
        .to_string()
        .as_bytes(),
    );
    assert_eq!(id, &expected[..32]);
    let evidence = candidate["evidence"].as_array().unwrap();
    assert_eq!(evidence.len(), 3);
    for (span, reference, relation) in [
        (&evidence[0], anchor, "anchor"),
        (&evidence[1], previous, "previous"),
        (&evidence[2], next, "next"),
    ] {
        assert_eq!(span["reference"], reference);
        assert_eq!(span["relation"], relation);
        span_text(&v, span);
        assert_eq!(
            page.to_string()
                .matches(span["text"].as_str().unwrap())
                .count(),
            1
        );
    }
    assert_eq!(evidence[0]["role"], "assistant");
    let mut args = review_args(&page, 0, "inspect");
    let issued_hash = args["context_review"]["candidate"]["packet_hash"].clone();
    rehash(&mut args);
    assert_eq!(
        args["context_review"]["candidate"]["packet_hash"],
        issued_hash
    );
    inspected(&v, args);
}

#[test]
fn preview_floor_is_explicit_utf8_safe_and_not_silently_shrunk() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    event(
        &v,
        "long",
        "topic",
        &format!("{}beacon {}", "界🦀 ".repeat(120), "原文🦀".repeat(800)),
        json!({}),
    );
    for floor in [64, 128, 256, 512] {
        let mut args = search_args();
        args["preview_min_bytes"] = json!(floor);
        args["budget_bytes"] = json!(512);
        let tiny = discover(&v, args.clone());
        assert_eq!(tiny["status"], "budget_exhausted");
        assert!(serde_json::to_vec(&tiny).unwrap().len() <= 512);
        assert!(tiny["results"].as_array().unwrap().is_empty());
        assert!(tiny["next_cursor"].is_null());
        args["budget_bytes"] = tiny["recommended_min_budget"].clone();
        if tiny["retry_cursor"].is_string() {
            args["cursor"] = tiny["retry_cursor"].clone();
        }
        let page = discover(&v, args.clone());
        assert!(
            serde_json::to_vec(&page).unwrap().len()
                <= args["budget_bytes"].as_u64().unwrap() as usize
        );
        let text = span_text(&v, &page["results"][0]["review_candidate"]["evidence"][0]);
        assert!(
            text.len() >= floor - 3,
            "floor {floor}, actual {}",
            text.len()
        );
        assert!(text.contains("beacon"));
    }
    let page = discover(&v, search_args());
    assert!(span_text(&v, &page["results"][0]["review_candidate"]["evidence"][0]).len() >= 509);
    for invalid in [0, 63, 65, 129, 513] {
        let mut args = search_args();
        args["preview_min_bytes"] = json!(invalid);
        assert!(transport::invoke(&context(&v), "search", args).is_err());
    }
}

#[test]
fn inspection_ignores_the_label_and_never_performs_assessed_deep_read() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    event(
        &v,
        "a",
        "topic",
        &format!("beacon {}", "original ".repeat(1000)),
        json!({}),
    );
    let page = discover(&v, search_args());
    for label in ["uncertain", "irrelevant", "relevant"] {
        let mut args = assessed(review_args(&page, 0, "inspect"));
        args["context_review"]["assessment"]["decision"] = json!(label);
        inspected(&v, args);
    }
    for assessment in [
        json!({"decision":"uncertain","selected_anchor_ref":null,"evidence_indices":[]}),
        json!({"decision":"irrelevant","selected_anchor_ref":page["results"][0]["review_candidate"]["anchor_ref"],"evidence_indices":[0]}),
        json!({"decision":"relevant","selected_anchor_ref":null,"evidence_indices":[0]}),
        json!({"decision":"relevant","selected_anchor_ref":page["results"][0]["review_candidate"]["anchor_ref"],"evidence_indices":[]}),
    ] {
        let mut args = review_args(&page, 0, "read");
        args["context_review"]["assessment"] = assessment;
        fails_closed(&context(&v), args);
    }
    let initial = inspected(&v, review_args(&page, 0, "inspect"));
    let mut args = review_args(&page, 0, "read");
    args["context_review"]["candidate"] = initial["updated_candidate"].clone();
    fails_closed(&context(&v), args.clone());
    let result = transport::invoke(&context(&v), "read", assessed(args)).unwrap();
    assert_eq!(result["relevance"], "caller_assessed");
    assert_eq!(result["evidence_binding"], "verified");
    assert!(result["results"][0]["text"].as_str().unwrap().len() > 512);
}

#[test]
fn inspection_is_a_fixed_utf8_window_with_dedicated_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    event(
        &v,
        "a",
        "topic",
        &format!("{}beacon {}", "prefix ".repeat(100), "界🦀".repeat(1800)),
        json!({}),
    );
    let page = discover(&v, search_args());
    let mut args = review_args(&page, 0, "inspect");
    let window_start = args["context_review"]["candidate"]["anchor_start_byte"]
        .as_u64()
        .unwrap() as usize;
    let mut next_start = window_start;
    let mut bytes = 0;
    let mut seen = BTreeSet::new();
    loop {
        let result = inspected(&v, args.clone());
        let span = &result["updated_candidate"]["evidence"][0];
        assert_eq!(span["start_byte"], next_start);
        let start = span["start_byte"].as_u64().unwrap() as usize;
        let end = span["end_byte"].as_u64().unwrap() as usize;
        assert!(seen.insert((start, end)));
        assert!(end <= window_start + 2048);
        bytes += end - start;
        next_start = end;
        if result["next_cursor"].is_null() {
            break;
        }
        assert!(result["next_cursor"].as_str().unwrap().starts_with("ri1:"));
        assert!(
            seen.len() <= 5,
            "inspection cursor failed to stop within its window"
        );
        args["cursor"] = result["next_cursor"].clone();
        args.as_object_mut().unwrap().remove("offset_bytes");
    }
    assert!((2045..=2048).contains(&bytes));
    let mut outside = review_args(&page, 0, "inspect");
    outside["offset_bytes"] = json!(window_start + 2048);
    fails_closed(&context(&v), outside);
    if window_start > 0 {
        let mut before = review_args(&page, 0, "inspect");
        before["offset_bytes"] = json!(0);
        fails_closed(&context(&v), before);
    }
}

#[test]
fn inspection_cursor_cannot_change_stage_budget_target_or_become_a_plain_read() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let a = event(
        &v,
        "a",
        "topic",
        &format!("beacon {}", "original ".repeat(1000)),
        json!({}),
    );
    let next = event(
        &v,
        "next",
        "topic",
        "Related later original.",
        json!({"reply_to":a.trim_start_matches("event:")}),
    );
    let page = discover(&v, search_args());
    let base = review_args(&page, 0, "inspect");
    let first = inspected(&v, base.clone());
    let mut continuation = base.clone();
    continuation["cursor"] = first["next_cursor"].clone();
    assert!(continuation["cursor"].is_string());
    inspected(&v, continuation.clone());
    let mut changed = assessed(continuation.clone());
    changed["context_review"]["stage"] = json!("read");
    fails_closed(&context(&v), changed);
    let mut changed = continuation.clone();
    changed["budget_bytes"] = json!(16384);
    fails_closed(&context(&v), changed);
    let mut changed = continuation.clone();
    changed["refs"] = json!([next]);
    fails_closed(&context(&v), changed);
    let mut changed = continuation.clone();
    changed.as_object_mut().unwrap().remove("context_review");
    assert!(transport::invoke(&context(&v), "read", changed).is_err());
    let mut changed = continuation;
    changed["cursor"] = json!(first["next_cursor"]
        .as_str()
        .unwrap()
        .replacen("ri1:", "r1:", 1));
    fails_closed(&context(&v), changed);
}

#[test]
fn tiny_inspection_budget_returns_no_evidence_and_does_not_advance() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    event(
        &v,
        "a",
        "topic",
        &format!("beacon {}", "界🦀".repeat(1000)),
        json!({}),
    );
    let page = discover(&v, search_args());
    let mut args = review_args(&page, 0, "inspect");
    args["budget_bytes"] = json!(512);
    let result = transport::invoke(&context(&v), "read", args.clone()).unwrap();
    assert_eq!(result["status"], "budget_exhausted");
    assert_eq!(result["deep_read_performed"], false);
    assert!(serde_json::to_vec(&result).unwrap().len() <= 512);
    assert!(result["results"].as_array().unwrap().is_empty());
    assert!(result["updated_candidate"].is_null());
    assert!(result["next_cursor"].is_null());
    args["budget_bytes"] = result["recommended_min_budget"].clone();
    let retry = inspected(&v, args);
    assert_eq!(
        retry["updated_candidate"]["evidence"][0]["start_byte"],
        page["results"][0]["review_candidate"]["anchor_start_byte"]
    );
}

#[test]
fn forged_original_evidence_and_identity_fail_even_after_packet_rehash() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    event(&v, "a", "topic", "beacon 界🦀 source truth", json!({}));
    let page = discover(&v, search_args());
    let base = assessed(review_args(&page, 0, "read"));
    for (field, value) in [
        ("text", json!("beacon invented assertion")),
        ("role", json!("assistant")),
        ("occurred_at", json!("2026-01-01T00:00:00Z")),
        ("total_bytes", json!(999999)),
        ("text_truncated", json!(true)),
        ("snapshot", json!("0".repeat(64))),
        ("start_byte", json!(8)),
        ("end_byte", json!(99999)),
        ("relation", json!("next")),
        ("reference", json!("event:unrelated")),
    ] {
        let mut args = base.clone();
        args["context_review"]["candidate"]["evidence"][0][field] = value;
        rehash(&mut args);
        fails_closed(&context(&v), args.clone());
        args["context_review"]["stage"] = json!("inspect");
        fails_closed(&context(&v), args);
    }
    for (field, value) in [
        ("anchor_snapshot", json!("0".repeat(64))),
        ("source_group_key", json!("0".repeat(64))),
        ("graph_generation", json!("0".repeat(64))),
        ("candidate_id", json!("0".repeat(32))),
        ("anchor_ref", json!("event:unrelated")),
    ] {
        let mut args = base.clone();
        args["context_review"]["candidate"][field] = value;
        rehash(&mut args);
        fails_closed(&context(&v), args);
    }
    for indices in [json!([]), json!([0, 0]), json!([1]), json!([999])] {
        let mut args = base.clone();
        args["context_review"]["assessment"]["evidence_indices"] = indices;
        fails_closed(&context(&v), args);
    }
}

#[test]
fn inspection_keeps_original_anchor_and_exactly_two_monotonic_graph_hops() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let mut refs = Vec::new();
    for i in 0..7 {
        let extra = refs
            .last()
            .map(|p: &String| json!({"reply_to":p.trim_start_matches("event:")}))
            .unwrap_or_else(|| json!({}));
        refs.push(event(
            &v,
            &format!("node-{i}"),
            "topic",
            if i == 3 {
                "beacon anchor"
            } else {
                "Related original message"
            },
            extra,
        ));
    }
    let page = discover(&v, search_args());
    let base = review_args(&page, 0, "inspect");
    assert_eq!(base["context_anchor_ref"], refs[3]);
    for i in [1, 2, 3, 4, 5] {
        let mut args = base.clone();
        args["refs"] = json!([refs[i]]);
        let result = inspected(&v, args);
        let span = &result["updated_candidate"]["evidence"][0];
        assert_eq!(span["reference"], refs[i]);
        assert_eq!(
            span["relation"],
            if i < 3 {
                "previous"
            } else if i > 3 {
                "next"
            } else {
                "anchor"
            }
        );
        assert_eq!(result["updated_candidate"]["anchor_ref"], refs[3]);
        if i == 1 {
            assert!(result["navigation"]["previous_ref"].is_null());
        }
        if i == 5 {
            assert!(result["navigation"]["next_refs"]
                .as_array()
                .unwrap()
                .is_empty());
        }
        let navigation = result["navigation"].to_string();
        assert!(!navigation.contains(&refs[0]));
        assert!(!navigation.contains(&refs[6]));
    }
    for i in [0, 6] {
        let mut args = base.clone();
        args["refs"] = json!([refs[i]]);
        fails_closed(&context(&v), args);
    }
    let mut rebased = base;
    rebased["context_anchor_ref"] = json!(refs[4]);
    rebased["refs"] = json!([refs[6]]);
    fails_closed(&context(&v), rebased);
}

#[test]
fn sibling_foreign_source_and_ambiguous_fork_are_not_inspection_shortcuts() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let parent = event(&v, "parent", "topic", "Original common parent.", json!({}));
    let anchor = event(
        &v,
        "anchor",
        "topic",
        "beacon branch anchor",
        json!({"reply_to":parent.trim_start_matches("event:"),"metadata":{"qwen":{"node_id":"anchor","on_current_path":false}}}),
    );
    let sibling = event(
        &v,
        "sibling",
        "topic",
        "Sibling body must not leak.",
        json!({"reply_to":parent.trim_start_matches("event:")}),
    );
    let foreign = event(
        &v,
        "foreign",
        "topic",
        "Foreign account must not leak.",
        json!({"source":{"platform":"compact-toy","account_namespace":"different-owner","conversation_id":"topic","message_id":"foreign"}}),
    );
    let first = event(
        &v,
        "fork-one",
        "topic",
        "First ambiguous answer.",
        json!({"reply_to":anchor.trim_start_matches("event:")}),
    );
    let second = event(
        &v,
        "fork-two",
        "topic",
        "Second ambiguous answer.",
        json!({"reply_to":anchor.trim_start_matches("event:")}),
    );
    let page = discover(&v, search_args());
    for reference in [sibling, foreign, first, second] {
        assert!(!page.to_string().contains(&reference));
        let mut args = review_args(&page, 0, "inspect");
        args["refs"] = json!([reference]);
        fails_closed(&context(&v), args);
    }
    let result = inspected(&v, review_args(&page, 0, "inspect"));
    assert!(result["navigation"]["next_refs"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(result["navigation"]
        .to_string()
        .contains("branch_choice_required"));
}

#[test]
fn filters_as_of_and_authorized_scope_are_bound_on_every_inspection() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let previous = event(
        &v,
        "previous",
        "topic",
        "Excluded assistant context.",
        json!({"role":"assistant"}),
    );
    let anchor = event(
        &v,
        "anchor",
        "topic",
        "beacon user context",
        json!({"reply_to":previous.trim_start_matches("event:")}),
    );
    let later = event(
        &v,
        "later",
        "topic",
        "Future context is excluded.",
        json!({"reply_to":anchor.trim_start_matches("event:"),"occurred_at":"2026-01-04T00:00:00Z"}),
    );
    let hidden = event(
        &v,
        "hidden",
        "hidden-topic",
        "beacon hidden project text",
        json!({"scope":"project:private"}),
    );
    let mut search = search_args();
    search["event_filter"] = json!({"role":"user"});
    search["as_of"] = json!("2026-01-03T00:00:00Z");
    let page = discover(&v, search);
    for reference in [&previous, &later, &hidden] {
        assert!(!page.to_string().contains(reference));
    }
    let base = review_args(&page, 0, "inspect");
    inspected(&v, base.clone());
    for reference in [previous, later, hidden] {
        let mut args = base.clone();
        args["refs"] = json!([reference]);
        fails_closed(&context(&v), args);
    }
    for (field, value) in [
        ("event_filter", json!(null)),
        ("event_filter", json!({})),
        ("as_of", json!(null)),
        ("as_of", json!("2026-01-05T00:00:00Z")),
    ] {
        let mut args = base.clone();
        args[field] = value;
        fails_closed(&context(&v), args);
    }
    for scopes in [
        vec!["project:private".into()],
        vec!["personal".into(), "project:private".into()],
    ] {
        let ctx = Context::new(&v, Access::new(scopes).unwrap());
        fails_closed(&ctx, base.clone());
    }
}

#[test]
fn source_revision_suppression_and_graph_changes_invalidate_old_packets() {
    for mutation in ["revision", "suppression", "graph"] {
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::init(dir.path()).unwrap();
        let anchor = event(&v, "anchor", "topic", "beacon original", json!({}));
        let page = discover(&v, search_args());
        let args = review_args(&page, 0, "inspect");
        match mutation {
            "revision" => {
                event(&v, "anchor", "topic", "beacon replacement", json!({}));
            }
            "suppression" => {
                v.suppress(
                    anchor.trim_start_matches("event:"),
                    "synthetic withdrawal".into(),
                )
                .unwrap();
            }
            "graph" => {
                event(
                    &v,
                    "new-child",
                    "topic",
                    "New graph child.",
                    json!({"reply_to":anchor.trim_start_matches("event:")}),
                );
            }
            _ => unreachable!(),
        }
        fails_closed(&context(&v), args.clone());
        let mut read = assessed(args);
        read["context_review"]["stage"] = json!("read");
        fails_closed(&context(&v), read);
    }
}

#[test]
fn ambiguous_source_node_identity_cannot_be_inspected() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    for (id, text) in [
        ("first", "beacon intended original"),
        ("second", "An identity collision"),
    ] {
        event(
            &v,
            id,
            "topic",
            text,
            json!({"metadata":{"qwen":{"node_id":"same-source-node","on_current_path":true}}}),
        );
    }
    let page = discover(&v, search_args());
    fails_closed(&context(&v), review_args(&page, 0, "inspect"));
}

#[test]
fn compact_discovery_cursor_binds_preview_floor_and_never_repeats_groups() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    for i in 0..4 {
        event(
            &v,
            &format!("event-{i}"),
            &format!("session-{i}"),
            "beacon independent source",
            json!({}),
        );
    }
    let mut args = search_args();
    args["limit"] = json!(1);
    args["preview_min_bytes"] = json!(256);
    let first = discover(&v, args.clone());
    let cursor = first["next_cursor"].clone();
    assert!(cursor.is_string());
    let mut seen = BTreeSet::new();
    let mut page = first;
    loop {
        for row in page["results"].as_array().unwrap() {
            assert!(seen.insert(
                row["review_candidate"]["source_group_key"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            ));
        }
        if page["next_cursor"].is_null() {
            break;
        }
        args["cursor"] = page["next_cursor"].clone();
        page = discover(&v, args.clone());
    }
    assert_eq!(seen.len(), 4);
    for (field, value) in [
        ("preview_min_bytes", json!(128)),
        ("detail", json!("session_discovery")),
        ("budget_bytes", json!(16384)),
        ("limit", json!(2)),
    ] {
        let mut changed = args.clone();
        changed["cursor"] = cursor.clone();
        changed[field] = value;
        assert!(transport::invoke(&context(&v), "search", changed).is_err());
    }
}

#[test]
fn new_review_shape_cannot_mix_confirmations_or_drop_required_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let anchor = event(&v, "anchor", "topic", "beacon original", json!({}));
    let compact = discover(&v, search_args());
    let base = assessed(review_args(&compact, 0, "read"));
    let mut legacy_search = search_args();
    legacy_search["detail"] = json!("session_discovery");
    let legacy = discover(&v, legacy_search);
    let legacy_args = legacy["results"][0]["read_args"].clone();
    let mut both = base.clone();
    both["context_confirmation"] = legacy_args["context_confirmation"].clone();
    assert!(transport::invoke(&context(&v), "read", both).is_err());
    for (field, value) in [
        ("include_adjacent", json!(false)),
        ("context_anchor_ref", json!(null)),
        ("detail", json!("full")),
        ("refs", json!([anchor, anchor])),
    ] {
        let mut changed = base.clone();
        changed[field] = value;
        fails_closed(&context(&v), changed);
    }
    // Legacy discovery still requires assessment, and ordinary explicit read remains supported.
    fails_closed(&context(&v), legacy_args.clone());
    let mut legacy_assessed = legacy_args;
    legacy_assessed["context_confirmation"]["assessment"] = json!({
        "decision":"relevant","selected_anchor_ref":anchor,"evidence_indices":[0]
    });
    assert_eq!(
        transport::invoke(&context(&v), "read", legacy_assessed).unwrap()["relevance"],
        "caller_assessed"
    );
    assert_eq!(
        transport::invoke(
            &context(&v),
            "read",
            json!({"refs":[anchor],"detail":"compact","budget_bytes":8192})
        )
        .unwrap()["results"][0]["text"],
        "beacon original"
    );
}

#[test]
fn unselected_evidence_and_template_limits_are_not_trusted_after_rehash() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let previous = event(
        &v,
        "previous",
        "topic",
        "Earlier original requirement.",
        json!({}),
    );
    event(
        &v,
        "anchor",
        "topic",
        "beacon original conclusion",
        json!({"reply_to":previous.trim_start_matches("event:")}),
    );
    let page = discover(&v, search_args());
    let base = assessed(review_args(&page, 0, "read"));
    assert_eq!(
        base["context_review"]["candidate"]["evidence"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let mut changed = base.clone();
    changed["context_review"]["candidate"]["evidence"][1]["text"] =
        json!("Invented unselected evidence.");
    rehash(&mut changed);
    fails_closed(&context(&v), changed);
    for (field, value) in [
        ("max_graph_hops", json!(3)),
        ("window_bytes", json!(8192)),
        ("excerpt_bytes", json!(4096)),
        ("scopes_hash", json!("0".repeat(64))),
        ("version", json!("recallcard.candidate-review/forged")),
    ] {
        let mut changed = base.clone();
        changed["context_review"]["template"][field] = value;
        rehash(&mut changed);
        fails_closed(&context(&v), changed);
    }
}

#[test]
fn assessed_read_cursor_preserves_the_review_and_cannot_become_inspection() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    event(
        &v,
        "a",
        "topic",
        &format!("beacon {}", "long original ".repeat(3000)),
        json!({}),
    );
    let page = discover(&v, search_args());
    let mut args = assessed(review_args(&page, 0, "read"));
    args["budget_bytes"] = json!(2048);
    let first = transport::invoke(&context(&v), "read", args.clone()).unwrap();
    assert_eq!(first["relevance"], "caller_assessed");
    assert!(first["next_cursor"].is_string());
    assert!(!first["next_cursor"].as_str().unwrap().starts_with("ri1:"));
    args["cursor"] = first["next_cursor"].clone();
    let next = transport::invoke(&context(&v), "read", args.clone()).unwrap();
    assert_eq!(next["relevance"], "caller_assessed");
    assert_eq!(
        next["results"][0]["text_range"]["start_byte"],
        first["results"][0]["text_range"]["end_byte"]
    );
    let mut changed = args.clone();
    changed["context_review"]["stage"] = json!("inspect");
    fails_closed(&context(&v), changed);
    let mut changed = args.clone();
    changed["context_review"]["assessment"]["decision"] = json!("uncertain");
    fails_closed(&context(&v), changed);
    args.as_object_mut().unwrap().remove("context_review");
    assert!(transport::invoke(&context(&v), "read", args).is_err());
}

#[test]
fn advertised_transport_schema_exposes_compact_discovery_and_both_review_stages() {
    let definitions = transport::tool_definitions();
    let tools = definitions.as_array().unwrap();
    let search = tools.iter().find(|tool| tool["name"] == "search").unwrap();
    assert!(search["inputSchema"]["properties"]["detail"]["enum"]
        .as_array()
        .unwrap()
        .contains(&json!("session_discovery_compact")));
    assert_eq!(
        search["inputSchema"]["properties"]["preview_min_bytes"]["enum"],
        json!([64, 128, 256, 512])
    );
    let read = tools.iter().find(|tool| tool["name"] == "read").unwrap();
    let review = &read["inputSchema"]["properties"]["context_review"];
    assert!(review.is_object());
    assert_eq!(
        review["properties"]["stage"]["enum"],
        json!(["inspect", "read"])
    );
    for required in ["stage", "template", "candidate", "assessment"] {
        assert!(review["required"]
            .as_array()
            .unwrap()
            .contains(&json!(required)));
    }
}

#[test]
fn exact_ascii_window_end_and_old_cursor_after_window_change_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    event(
        &v,
        "a",
        "topic",
        &format!("beacon {}", "original ".repeat(1000)),
        json!({}),
    );
    let page = discover(&v, search_args());
    let base = review_args(&page, 0, "inspect");
    let first = inspected(&v, base.clone());
    let start = base["context_review"]["candidate"]["anchor_start_byte"]
        .as_u64()
        .unwrap();
    let mut at_end = base.clone();
    at_end["offset_bytes"] = json!(start + 2048);
    fails_closed(&context(&v), at_end);
    // This checks cursor consistency, not issuance of unsigned caller-owned packets.
    let mut changed = base;
    changed["context_review"]["candidate"]["anchor_start_byte"] = json!(start + 1);
    rehash(&mut changed);
    changed["cursor"] = first["next_cursor"].clone();
    fails_closed(&context(&v), changed);
}

#[test]
fn empty_neighbor_inspection_does_not_relabel_old_anchor_evidence_as_new_text() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let anchor = event(&v, "anchor", "topic", "beacon original evidence", json!({}));
    let empty = event(
        &v,
        "empty",
        "topic",
        "",
        json!({"reply_to":anchor.trim_start_matches("event:")}),
    );
    let page = discover(&v, search_args());
    let mut args = review_args(&page, 0, "inspect");
    args["refs"] = json!([empty]);
    let result = transport::invoke(&context(&v), "read", args).unwrap();
    assert_eq!(result["deep_read_performed"], false);
    assert_eq!(result["relevance"], "unresolved");
    assert_eq!(result["text_bytes"], 0);
    assert!(result["results"].as_array().unwrap().is_empty());
    assert!(result["updated_candidate"].is_null());
    assert!(result["next_cursor"].is_null());
    assert!(!result.to_string().contains("beacon original evidence"));
}

#[test]
fn constrained_compact_page_keeps_fewer_full_previews_before_shrinking() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    for i in 0..6 {
        event(
            &v,
            &format!("budget-source-{i}"),
            &format!("budget-session-{i}"),
            &format!("beacon {}", "original evidence ".repeat(400)),
            json!({}),
        );
    }
    let mut args = search_args();
    args["limit"] = json!(6);
    args["preview_min_bytes"] = json!(512);
    let all = discover(&v, args.clone());
    assert_eq!(all["results"].as_array().unwrap().len(), 6);

    // Derive a real response budget from two complete candidates instead of
    // depending on incidental metadata lengths in this transport version.
    let mut two_args = args.clone();
    two_args["limit"] = json!(2);
    let two = discover(&v, two_args);
    assert_eq!(two["results"].as_array().unwrap().len(), 2);
    let budget = serde_json::to_vec(&two).unwrap().len() + 256;
    assert!(budget < serde_json::to_vec(&all).unwrap().len());
    args["budget_bytes"] = json!(budget);
    let limited = discover(&v, args);
    let candidates = limited["results"].as_array().unwrap();
    assert!((2..6).contains(&candidates.len()), "{limited}");
    assert!(serde_json::to_vec(&limited).unwrap().len() <= budget);
    assert!(limited["next_cursor"].is_string());
    for item in candidates {
        let evidence = item["review_candidate"]["evidence"].as_array().unwrap();
        assert_eq!(evidence.len(), 1);
        assert_eq!(span_text(&v, &evidence[0]).len(), 512);
    }
}

#[test]
fn candidate_identity_survives_preview_floor_changes_but_not_source_revision() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let original = event(
        &v,
        "stable-message",
        "stable-session",
        &format!("beacon {}", "original evidence ".repeat(400)),
        json!({}),
    );
    let mut args = search_args();
    args["preview_min_bytes"] = json!(128);
    let smaller_floor = discover(&v, args.clone());
    args["preview_min_bytes"] = json!(512);
    let larger_floor = discover(&v, args.clone());
    let first = &smaller_floor["results"][0]["review_candidate"];
    let second = &larger_floor["results"][0]["review_candidate"];
    assert_eq!(first["anchor_ref"], original);
    assert_eq!(second["anchor_ref"], original);
    assert_eq!(first["candidate_id"], second["candidate_id"]);

    let revised = event(
        &v,
        "stable-message",
        "stable-session",
        &format!("beacon revised {}", "replacement evidence ".repeat(400)),
        json!({}),
    );
    assert_ne!(original, revised);
    let updated = discover(&v, args);
    let candidate = &updated["results"][0]["review_candidate"];
    assert_eq!(candidate["anchor_ref"], revised);
    assert_ne!(candidate["anchor_snapshot"], first["anchor_snapshot"]);
    assert_ne!(candidate["candidate_id"], first["candidate_id"]);
}

#[test]
fn compact_packet_keeps_filtered_neighbor_stop_reasons_next_to_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let previous = event(
        &v,
        "hidden-role",
        "topic",
        "Earlier assistant proposal",
        json!({"role":"assistant"}),
    );
    let anchor = event(
        &v,
        "anchor",
        "topic",
        "beacon user correction",
        json!({"reply_to":previous.trim_start_matches("event:")}),
    );
    let mut args = search_args();
    args["event_filter"] = json!({"role":"user"});
    let page = discover(&v, args);
    let item = &page["results"][0];
    assert_eq!(item["context_complete"], false);
    assert!(item["context_stop_reasons"]
        .as_array()
        .unwrap()
        .contains(&json!("request_filter_boundary")));
    let evidence = item["review_candidate"]["evidence"].as_array().unwrap();
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0]["reference"], anchor);
    assert!(!page.to_string().contains("Earlier assistant proposal"));
    assert!(!page.to_string().contains(&previous));
}

#[test]
fn compact_shared_score_units_preserve_legacy_scores_and_empty_page_meaning() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    event(&v, "units", "units-session", "beacon evidence", json!({}));
    let units = json!({"semantic":"cosine","lexical":"existing_lexical_score"});
    for query in ["beacon", "absent-term"] {
        let mut args = search_args();
        args["query"] = json!(query);
        args["budget_bytes"] = json!(8192);
        let compact = discover(&v, args.clone());
        args["detail"] = json!("session_discovery");
        let legacy = discover(&v, args);
        assert_eq!(compact["score_units"], units);
        assert_eq!(compact["relevance"], "unassessed");
        assert!(serde_json::to_vec(&compact).unwrap().len() <= 8192);
        let compact_items = compact["results"].as_array().unwrap();
        let legacy_items = legacy["results"].as_array().unwrap();
        assert_eq!(compact_items.len(), usize::from(query == "beacon"));
        assert_eq!(compact_items.len(), legacy_items.len());
        for (current, previous) in compact_items.iter().zip(legacy_items) {
            assert_eq!(current["channels"], previous["channels"]);
            assert_eq!(previous["score_units"], units);
            assert!(current.get("score_units").is_none());
        }
    }
}
