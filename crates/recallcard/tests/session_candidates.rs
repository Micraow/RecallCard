//! Independent toy candidate selection; no relevance threshold or private gold data.
use recallcard::{context::Context, policy::Access, transport, Vault};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    io::Write,
    process::{Command, Stdio},
};

fn event(v: &Vault, id: &str, session: &str, text: &str, extra: Value) -> String {
    let mut data = json!({"role":"user","origin":"native","scope":"personal","content":text,
        "session_id":session,"occurred_at":"2026-01-02T00:00:00Z",
        "source":{"platform":"qwen","account_namespace":"toy","conversation_id":session,"message_id":id},
        "metadata":{"qwen":{"node_id":id,"parent_id":null,"on_current_path":true}}});
    for (k, x) in extra.as_object().unwrap() {
        data[k] = x.clone();
    }
    format!(
        "event:{}",
        v.capture(serde_json::from_value(data).unwrap()).unwrap().id
    )
}
fn args(budget: usize) -> Value {
    json!({"query":"beacon","target":"events","detail":"session_candidates","limit":3,"budget_bytes":budget})
}
fn call(v: &Vault, args: Value) -> Result<Value, String> {
    transport::invoke(
        &Context::new(v, Access::new(vec!["personal".into()]).unwrap()),
        "search",
        args,
    )
}

#[test]
fn one_bounded_excerpt_per_distinct_session_prevents_long_top_candidate_starvation() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    for i in 0..6 {
        event(
            &v,
            &format!("long{i}"),
            "long",
            &format!("beacon {}", "界🦀 longer original text ".repeat(8000)),
            json!({}),
        );
    }
    event(
        &v,
        "short",
        "short",
        "beacon shorter original text",
        json!({}),
    );
    event(
        &v,
        "third",
        "third",
        "beacon third original source",
        json!({}),
    );
    let page = call(&v, args(8192)).unwrap();
    let entries = page["results"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(page["candidate_count"], 3);
    assert_eq!(page["selection_required"], true);
    let sessions: BTreeSet<_> = entries
        .iter()
        .map(|e| e["session_ref"].as_str().unwrap())
        .collect();
    assert_eq!(sessions.len(), 3);
    for e in entries {
        assert_eq!(e["relevance"], "unassessed");
        assert!(e["text"].as_str().unwrap().len() <= 512);
        assert!(e.get("confidence").is_none());
        let original = v
            .event(
                e["anchor_ref"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("event:"),
            )
            .unwrap()
            .data
            .text();
        let range = &e["text_range"];
        assert_eq!(
            &original[range["start_byte"].as_u64().unwrap() as usize
                ..range["end_byte"].as_u64().unwrap() as usize],
            e["text"].as_str().unwrap()
        );
    }
    assert_eq!(page["coverage"]["scores_are_confidence"], false);
    assert!(serde_json::to_vec(&page).unwrap().len() <= 8192);
}

#[test]
fn user_session_name_cannot_merge_different_official_sources_or_leak_scope() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    event(&v, "a", "shared", "beacon first", json!({}));
    event(
        &v,
        "b",
        "shared",
        "beacon second",
        json!({"source":{"platform":"qwen","account_namespace":"other","conversation_id":"shared","message_id":"b"}}),
    );
    event(
        &v,
        "c",
        "shared",
        "beacon third",
        json!({"source":{"platform":"other","account_namespace":"toy","conversation_id":"shared","message_id":"c"}}),
    );
    let hidden = event(
        &v,
        "private",
        "shared",
        "beacon excluded private source",
        json!({"scope":"project:private"}),
    );
    let page = call(&v, args(8192)).unwrap();
    assert_eq!(page["candidate_count"], 3);
    assert_eq!(page["coverage"]["eligible_event_count"], 3);
    assert!(!page.to_string().contains(&hidden));
    assert_eq!(
        page["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["session_ref"].as_str().unwrap())
            .collect::<BTreeSet<_>>()
            .len(),
        1
    );
}

#[test]
fn filters_and_as_of_apply_before_candidate_counts_and_selection_read() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let allowed = event(&v, "a", "one", "beacon allowed", json!({}));
    event(
        &v,
        "b",
        "two",
        "beacon assistant",
        json!({"role":"assistant"}),
    );
    event(
        &v,
        "c",
        "three",
        "beacon future",
        json!({"occurred_at":"2026-01-03T00:00:00Z"}),
    );
    event(
        &v,
        "d",
        "four",
        "beacon old branch",
        json!({"metadata":{"qwen":{"on_current_path":false}}}),
    );
    let mut request = args(8192);
    request["event_filter"] = json!({"role":"user","on_current_path":true});
    request["as_of"] = json!("2026-01-02T00:00:00Z");
    let page = call(&v, request).unwrap();
    assert_eq!(page["candidate_count"], 1);
    assert_eq!(page["coverage"]["eligible_event_count"], 1);
    let candidate = &page["results"][0];
    assert_eq!(candidate["anchor_ref"], allowed);
    let read = transport::invoke(
        &Context::new(&v, Access::new(vec!["personal".into()]).unwrap()),
        "read",
        candidate["read_args"].clone(),
    )
    .unwrap();
    assert_eq!(read["results"][0]["text"], "beacon allowed");
    assert_eq!(read["results"][0]["snapshot"], candidate["snapshot"]);
    assert_eq!(
        read["results"][0]["metadata"]["adjacent"]["constraints"]["as_of"],
        "2026-01-02T00:00:00Z"
    );
}

#[test]
fn candidate_cursor_pages_groups_once_and_rejects_changed_contract_or_source() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    for i in 0..4 {
        event(
            &v,
            &format!("n{i}"),
            &format!("s{i}"),
            "beacon identical toy text",
            json!({}),
        );
    }
    let mut request = args(4096);
    request["limit"] = json!(1);
    let first = call(&v, request.clone()).unwrap();
    let cursor = first["next_cursor"].clone();
    assert!(cursor.is_string());
    for (field, value) in [
        ("budget_bytes", json!(8192)),
        ("limit", json!(2)),
        ("detail", json!("session_context")),
        ("event_filter", json!({"role":"user"})),
        ("as_of", json!("2026-01-03T00:00:00Z")),
    ] {
        let mut changed = request.clone();
        changed["cursor"] = cursor.clone();
        changed[field] = value;
        assert!(call(&v, changed).is_err());
    }
    let mut refs = BTreeSet::new();
    let mut page = first;
    loop {
        for e in page["results"].as_array().unwrap() {
            assert!(refs.insert(e["anchor_ref"].as_str().unwrap().to_owned()));
        }
        if page["next_cursor"].is_null() {
            break;
        }
        request["cursor"] = page["next_cursor"].clone();
        page = call(&v, request.clone()).unwrap();
    }
    assert_eq!(refs.len(), 4);
    request["cursor"] = cursor;
    v.suppress(
        refs.first().unwrap().trim_start_matches("event:"),
        "toy hidden".into(),
    )
    .unwrap();
    assert!(call(&v, request).is_err());
}

#[test]
fn small_budget_is_not_no_matches_and_no_matches_does_not_mean_empty_history() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    event(&v, "a", "one", "beacon original", json!({}));
    let tiny = call(&v, args(512)).unwrap();
    assert_eq!(tiny["status"], "budget_exhausted");
    assert!(tiny["next_cursor"].is_null());
    assert!(serde_json::to_vec(&tiny).unwrap().len() <= 512);
    let retry = call(
        &v,
        args(tiny["recommended_min_budget"].as_u64().unwrap() as usize),
    )
    .unwrap();
    assert_eq!(retry["results"].as_array().unwrap().len(), 1);
    let mut empty = args(512);
    empty["query"] = json!("missing-identifier");
    let page = call(&v, empty).unwrap();
    assert_eq!(page["status"], "no_matches");
    assert_eq!(page["candidate_count"], 0);
    assert_eq!(page["coverage"]["eligible_event_count"], 1);
    assert_eq!(page["relevance"], "unassessed");
    let mut filtered = args(8192);
    filtered["event_filter"] = json!({"role":"assistant"});
    let page = call(&v, filtered).unwrap();
    assert_eq!(page["status"], "no_matches");
    assert_eq!(page["coverage"]["eligible_event_count"], 0);
}

#[test]
fn explicit_event_mode_never_silently_changes_memory_search_contract() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    for target in ["all", "memories", "views"] {
        let mut a = args(8192);
        a["target"] = json!(target);
        assert!(call(&v, a).is_err());
    }
    let mut a = args(8192);
    a["include_navigation"] = json!(true);
    assert!(call(&v, a).is_err());
}

#[test]
fn actual_mcp_lists_bounded_choices_then_reads_only_explicitly_chosen_anchor() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    event(&v, "a", "one", "beacon one toy topic", json!({}));
    let chosen = event(&v, "b", "two", "beacon another toy topic", json!({}));
    let mut child = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--vault")
        .arg(d.path())
        .args(["mcp", "--scope", "personal"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let messages = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search","arguments":args(8192)}}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"read","arguments":{"refs":[chosen],"context_anchor_ref":chosen,"include_adjacent":true,"detail":"compact","budget_bytes":4096}}}),
    ];
    for m in messages {
        writeln!(child.stdin.as_mut().unwrap(), "{m}").unwrap();
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let replies: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let pages: Vec<Value> = replies[1..]
        .iter()
        .map(|r| serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap())
        .collect();
    assert_eq!(pages[0]["results"].as_array().unwrap().len(), 2);
    assert_eq!(pages[1]["results"][0]["ref"], chosen);
    assert_eq!(pages[1]["results"][0]["text"], "beacon another toy topic");
}

#[test]
fn oversized_source_labels_are_explicitly_shortened_without_fabricating_session_refs() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let long = "long-source-label-".repeat(5000);
    event(&v, "a", &long, "beacon first", json!({}));
    event(&v, "b", "short", "beacon second", json!({}));
    let page = call(&v, args(8192)).unwrap();
    assert_eq!(page["results"].as_array().unwrap().len(), 2);
    let item = page["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["session_ref_truncated"] == true)
        .unwrap();
    assert!(item["session_ref"].is_null());
    assert!(item["session_label"].as_str().unwrap().len() <= 256);
    assert_eq!(item["source_identity"]["truncated"], true);
    assert!(serde_json::to_vec(&page).unwrap().len() <= 8192);
    assert_eq!(item["read_args"]["refs"][0], item["anchor_ref"]);
}

#[test]
fn choosing_a_late_excerpt_reads_that_evidence_not_the_giant_message_prefix() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let original = format!(
        "{}beacon actual original tail evidence",
        "无关前文。".repeat(5000)
    );
    event(&v, "a", "one", &original, json!({}));
    let page = call(&v, args(8192)).unwrap();
    let item = &page["results"][0];
    let start = item["text_range"]["start_byte"].as_u64().unwrap() as usize;
    assert!(start > 20_000);
    assert_eq!(item["read_args"]["offset_bytes"], start);
    let read = transport::invoke(
        &Context::new(&v, Access::new(vec!["personal".into()]).unwrap()),
        "read",
        item["read_args"].clone(),
    )
    .unwrap();
    assert!(read["results"][0]["text"]
        .as_str()
        .unwrap()
        .contains("beacon actual original tail evidence"));
    assert_eq!(read["results"][0]["text_range"]["start_byte"], start);
}
