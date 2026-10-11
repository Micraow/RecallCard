//! Independent synthetic source graphs only; not a model-quality benchmark.
use recallcard::{context::Context, policy::Access, transport, Vault};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::Write,
    process::{Command, Stdio},
};

fn event(
    v: &Vault,
    id: &str,
    parent: Option<&str>,
    role: &str,
    text: &str,
    extra: Value,
) -> String {
    let mut input = json!({"role":role,"origin":"native","scope":"personal","content":text,
        "occurred_at":"2026-01-02T10:00:00Z",
        "source":{"platform":"chatgpt","account_namespace":"toy","conversation_id":"one","message_id":id},
        "metadata":{"chatgpt":{"nearest_visible_parent_id":parent,"on_current_path":true}}});
    for (key, value) in extra.as_object().unwrap() {
        input[key] = value.clone();
    }
    format!(
        "event:{}",
        v.capture(serde_json::from_value(input).unwrap())
            .unwrap()
            .id
    )
}
fn args(budget: usize) -> Value {
    json!({"query":"beacon","target":"events","detail":"session_context","budget_bytes":budget})
}
fn call(v: &Vault, args: Value) -> Result<Value, String> {
    transport::invoke(
        &Context::new(v, Access::new(vec!["personal".into()]).unwrap()),
        "search",
        args,
    )
}
fn refs(page: &Value) -> Vec<&str> {
    page["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["ref"].as_str().unwrap())
        .collect()
}
fn reasons(page: &Value) -> String {
    page["sessions"].to_string()
}

#[test]
fn retrieves_both_prior_requirement_and_later_correction_with_roles_and_exact_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let prior = event(&v, "prior", None, "user", "必须离线运行", json!({}));
    let anchor = event(
        &v,
        "anchor",
        Some("prior"),
        "assistant",
        "beacon could use a hosted service",
        json!({}),
    );
    let later = event(
        &v,
        "later",
        Some("anchor"),
        "user",
        "不，仍然只允许本地",
        json!({}),
    );
    let page = call(&v, args(8192)).unwrap();
    assert_eq!(
        refs(&page),
        vec![anchor.as_str(), prior.as_str(), later.as_str()]
    );
    assert_eq!(page["results"][0]["role"], "assistant");
    assert_eq!(page["results"][1]["relation"], "previous");
    assert_eq!(page["results"][2]["relation"], "next");
    assert_eq!(page["results"][2]["role"], "user");
    assert_eq!(page["results"][2]["text"], "不，仍然只允许本地");
    assert_eq!(
        page["results"][2]["text_range"]["end_byte"],
        "不，仍然只允许本地".len()
    );
    assert_eq!(page["status"], "complete");
    assert_eq!(page["session_count"], 1);
    let read = transport::invoke(
        &Context::new(&v, Access::new(vec!["personal".into()]).unwrap()),
        "read",
        json!({"refs":[later],"detail":"compact","budget_bytes":8192}),
    )
    .unwrap();
    assert_eq!(
        page["results"][2]["snapshot"],
        read["results"][0]["snapshot"]
    );
    assert_eq!(
        page["coverage"]["completeness"],
        "bounded_visible_source_graph_only"
    );
}

#[test]
fn huge_anchor_does_not_starve_short_followup_and_all_utf8_pages_reconstruct_originals() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let text = format!(
        "{} beacon {}",
        "中文😀\"\\\n".repeat(1300),
        "尾部🧭".repeat(1000)
    );
    let anchor = event(&v, "anchor", None, "assistant", &text, json!({}));
    let later = event(
        &v,
        "later",
        Some("anchor"),
        "user",
        "短更正：不要这样做",
        json!({}),
    );
    let mut request = args(8192);
    let first = call(&v, request.clone()).unwrap();
    assert!(refs(&first).contains(&later.as_str()));
    assert!(first["results"][0]["text"]
        .as_str()
        .unwrap()
        .contains("beacon"));
    let mut parts: BTreeMap<String, BTreeMap<usize, String>> = BTreeMap::new();
    let mut page = first;
    let mut pages = 0;
    loop {
        pages += 1;
        assert!(pages < 100);
        assert!(serde_json::to_vec(&page).unwrap().len() <= 8192);
        for item in page["results"].as_array().unwrap() {
            let start = item["text_range"]["start_byte"].as_u64().unwrap() as usize;
            let end = item["text_range"]["end_byte"].as_u64().unwrap() as usize;
            let body = item["text"].as_str().unwrap();
            assert_eq!(end - start, body.len());
            assert!(parts
                .entry(item["ref"].as_str().unwrap().into())
                .or_default()
                .insert(start, body.into())
                .is_none());
        }
        if page["next_cursor"].is_null() {
            break;
        }
        request["cursor"] = page["next_cursor"].clone();
        page = call(&v, request.clone()).unwrap();
    }
    for (reference, expected) in [(anchor, text), (later, "短更正：不要这样做".into())] {
        let mut reconstructed = String::new();
        for (offset, piece) in &parts[&reference] {
            assert_eq!(*offset, reconstructed.len());
            reconstructed.push_str(piece);
        }
        assert_eq!(reconstructed, expected);
    }
}

#[test]
fn low_budget_is_explicit_and_changed_budget_requires_new_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    event(
        &v,
        "anchor",
        None,
        "user",
        &format!("beacon {}", "段落".repeat(3000)),
        json!({}),
    );
    let low = call(&v, args(512)).unwrap();
    assert_eq!(low["status"], "budget_exhausted");
    assert!(low["results"].as_array().unwrap().is_empty());
    assert!(serde_json::to_vec(&low).unwrap().len() <= 512);
    let page = call(&v, args(4096)).unwrap();
    assert!(!page["next_cursor"].is_null());
    for (key, value) in [
        ("budget_bytes", json!(8192)),
        ("limit", json!(1)),
        ("detail", json!("context")),
        ("event_filter", json!({"role":"user"})),
    ] {
        let mut changed = args(4096);
        changed["cursor"] = page["next_cursor"].clone();
        changed[key] = value;
        assert!(call(&v, changed).is_err(), "{key}");
    }
    let mut old = args(8192);
    old["detail"] = json!("context");
    let standard = call(&v, old).unwrap();
    if !standard["next_cursor"].is_null() {
        let mut changed = args(4096);
        changed["cursor"] = standard["next_cursor"].clone();
        assert!(call(&v, changed).is_err());
    }
}

#[test]
fn role_and_time_filters_stop_at_excluded_neighbor_without_leaking_it() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    event(&v, "anchor", None, "user", "beacon requirements", json!({}));
    let assistant = event(
        &v,
        "middle",
        Some("anchor"),
        "assistant",
        "FILTERED ASSISTANT",
        json!({}),
    );
    let later = event(
        &v,
        "later",
        Some("middle"),
        "user",
        "later correction",
        json!({"occurred_at":"2026-01-03T10:00:00Z"}),
    );
    let mut request = args(8192);
    request["event_filter"] = json!({"role":"user"});
    let page = call(&v, request).unwrap();
    assert_eq!(refs(&page).len(), 1);
    assert!(!page.to_string().contains(&assistant));
    assert!(!page.to_string().contains(&later));
    assert!(reasons(&page).contains("request_filter_boundary"));
    let mut request = args(8192);
    request["event_filter"] = json!({"occurred_until":"2026-01-02T23:59:59Z"});
    let page = call(&v, request).unwrap();
    assert_eq!(refs(&page).len(), 2);
    assert!(!refs(&page).contains(&later.as_str()));
    let mut request = args(8192);
    request["as_of"] = json!("2026-01-02T23:59:59Z");
    assert_eq!(refs(&call(&v, request).unwrap()).len(), 2);
}

#[test]
fn hidden_bridge_and_foreign_scope_or_official_source_never_expand() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let anchor = event(
        &v,
        "anchor",
        None,
        "user",
        "beacon",
        json!({"session_id":"shared"}),
    );
    let middle = event(
        &v,
        "middle",
        Some("anchor"),
        "assistant",
        "hidden bridge",
        json!({"session_id":"shared"}),
    );
    let later = event(
        &v,
        "later",
        Some("middle"),
        "user",
        "not reachable",
        json!({"session_id":"shared"}),
    );
    let foreign = event(
        &v,
        "foreign",
        Some("anchor"),
        "user",
        "foreign account",
        json!({"session_id":"shared","source":{"platform":"chatgpt","account_namespace":"elsewhere","conversation_id":"one","message_id":"foreign"}}),
    );
    let secret = event(
        &v,
        "secret",
        Some("anchor"),
        "user",
        "hidden scope",
        json!({"scope":"project:secret","session_id":"shared"}),
    );
    v.suppress(
        middle.trim_start_matches("event:"),
        "toy suppression".into(),
    )
    .unwrap();
    let page = call(&v, args(8192)).unwrap();
    assert_eq!(refs(&page), vec![anchor.as_str()]);
    for reference in [middle, later, foreign, secret] {
        assert!(!page.to_string().contains(&reference));
    }
}

#[test]
fn ambiguous_branches_require_choice_and_backward_walk_never_enters_sibling() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let unknown = json!({"metadata":{"chatgpt":{"nearest_visible_parent_id":"root"}}});
    event(
        &v,
        "root",
        None,
        "user",
        "beacon",
        json!({"metadata":{"chatgpt":{"nearest_visible_parent_id":null}}}),
    );
    let left = event(
        &v,
        "left",
        Some("root"),
        "assistant",
        "left",
        unknown.clone(),
    );
    let right = event(&v, "right", Some("root"), "assistant", "right", unknown);
    let page = call(&v, args(8192)).unwrap();
    assert_eq!(refs(&page).len(), 1);
    assert!(reasons(&page).contains("branch_choice_required"));
    let mut request = args(8192);
    request["query"] = json!("left");
    let page = call(&v, request).unwrap();
    assert!(refs(&page).contains(&left.as_str()));
    assert_eq!(refs(&page).len(), 2);
    assert!(!refs(&page).contains(&right.as_str()));
}

#[test]
fn session_grouping_caps_anchors_and_hops_with_explicit_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    for i in 0..18 {
        event(
            &v,
            &format!("m{i}"),
            (i > 0).then(|| format!("m{}", i - 1)).as_deref(),
            if i % 2 == 0 { "user" } else { "assistant" },
            if [7, 8, 9, 10].contains(&i) {
                "beacon"
            } else {
                "unmatched neighbor"
            },
            json!({}),
        );
    }
    let page = call(&v, args(32768)).unwrap();
    assert_eq!(page["session_count"], 1);
    let session = &page["sessions"][0];
    assert_eq!(session["anchor_refs"].as_array().unwrap().len(), 2);
    assert_eq!(session["matching_anchor_count"], 4);
    assert!(session["selected_message_count"].as_u64().unwrap() <= 12);
    assert!(reasons(&page).contains("anchor_limit"));
    assert!(reasons(&page).contains("hop_limit"));
    for item in page["results"].as_array().unwrap() {
        assert!(item["distance"].as_u64().unwrap() <= 3);
    }
}

#[test]
fn capture_suppression_and_access_changes_invalidate_continuation() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let anchor = event(
        &v,
        "anchor",
        None,
        "user",
        &format!("beacon {}", "long ".repeat(2000)),
        json!({}),
    );
    let first = call(&v, args(4096)).unwrap();
    let mut request = args(4096);
    request["cursor"] = first["next_cursor"].clone();
    assert!(!request["cursor"].is_null());
    let denied = Context::new(&v, Access::new(vec!["project:elsewhere".into()]).unwrap());
    assert!(transport::invoke(&denied, "search", request.clone()).is_err());
    event(
        &v,
        "later",
        Some("anchor"),
        "user",
        "new correction",
        json!({}),
    );
    assert!(call(&v, request.clone()).is_err());
    request["cursor"] = call(&v, args(4096)).unwrap()["next_cursor"].clone();
    v.suppress(anchor.trim_start_matches("event:"), "toy".into())
        .unwrap();
    assert!(call(&v, request).is_err());
}

#[test]
fn missing_graph_is_not_replaced_by_timestamp_order_and_cycles_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    event(&v, "anchor", None, "user", "beacon", json!({"metadata":{}}));
    event(
        &v,
        "later",
        None,
        "assistant",
        "same time is not an edge",
        json!({"metadata":{}}),
    );
    let page = call(&v, args(8192)).unwrap();
    assert_eq!(refs(&page).len(), 1);
    assert!(reasons(&page).contains("source_graph_unavailable"));
    let dir2 = tempfile::tempdir().unwrap();
    let v2 = Vault::init(dir2.path()).unwrap();
    event(&v2, "a", Some("b"), "user", "beacon", json!({}));
    event(&v2, "b", Some("a"), "assistant", "loop", json!({}));
    assert!(call(&v2, args(8192)).unwrap_err().contains("循环"));
}

#[test]
fn mcp_binary_publishes_mode_and_serves_actual_bidirectional_context() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let before = event(&v, "before", None, "user", "initial restriction", json!({}));
    event(
        &v,
        "anchor",
        Some("before"),
        "assistant",
        "beacon suggestion",
        json!({}),
    );
    let after = event(
        &v,
        "after",
        Some("anchor"),
        "user",
        "later correction",
        json!({}),
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--vault")
        .arg(dir.path())
        .args(["mcp", "--scope", "personal"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let messages = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search","arguments":args(8192)}}),
    ];
    let mut stdin = child.stdin.take().unwrap();
    for message in messages {
        writeln!(stdin, "{message}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let replies: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(replies[1].to_string().contains("session_context"));
    assert_eq!(replies[2]["result"]["isError"], false);
    let page: Value =
        serde_json::from_str(replies[2]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(refs(&page).contains(&before.as_str()) && refs(&page).contains(&after.as_str()));
    assert_eq!(page["budget_unit"], "utf8_bytes");
    assert_eq!(page["status"], "complete");
}

#[test]
fn minimum_budget_empty_text_and_no_matches_always_make_progress() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    assert_eq!(call(&v, args(512)).unwrap()["status"], "no_matches");
    event(&v, "anchor", None, "user", "beacon 😀中文", json!({}));
    let empty = event(&v, "empty", Some("anchor"), "assistant", "", json!({}));
    let budget = call(&v, args(512)).unwrap()["recommended_min_budget"]
        .as_u64()
        .unwrap() as usize;
    let mut request = args(budget);
    let mut cursors = std::collections::BTreeSet::new();
    let mut empty_count = 0;
    loop {
        let page = call(&v, request.clone()).unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() <= budget);
        assert!(!page["results"].as_array().unwrap().is_empty());
        empty_count += refs(&page)
            .iter()
            .filter(|reference| **reference == empty)
            .count();
        if page["next_cursor"].is_null() {
            break;
        }
        assert!(cursors.insert(page["next_cursor"].to_string()));
        assert!(cursors.len() < 30);
        request["cursor"] = page["next_cursor"].clone();
    }
    assert_eq!(empty_count, 1);
}

#[test]
fn twelve_message_window_exposes_short_users_before_long_anchor_tails() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let long = format!("beacon {}", "合成长回答。".repeat(7000));
    for i in 0..20 {
        event(
            &v,
            &format!("m{i}"),
            (i > 0).then(|| format!("m{}", i - 1)).as_deref(),
            if i == 3 || i == 15 {
                "assistant"
            } else {
                "user"
            },
            if i == 3 || i == 15 {
                &long
            } else {
                "短用户更正"
            },
            json!({}),
        );
    }
    let page = call(&v, args(32768)).unwrap();
    assert_eq!(page["coverage"]["selected_message_count"], 12);
    assert_eq!(page["status"], "partial");
    assert!(!page["next_cursor"].is_null());
    let first_refs: std::collections::BTreeSet<_> = refs(&page).into_iter().collect();
    assert_eq!(first_refs.len(), 12);
    assert_eq!(
        page["results"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["role"] == "user")
            .count(),
        10
    );
    assert!(reasons(&page).contains("message_limit"));
}

#[test]
fn graph_node_ids_are_not_message_ids_and_export_gaps_stay_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(dir.path()).unwrap();
    let before = event(
        &v,
        "message-before",
        None,
        "user",
        "earlier requirement",
        json!({"metadata":{"chatgpt":{"node_id":"node-before","parent_id":null}}}),
    );
    let anchor = event(
        &v,
        "message-anchor",
        None,
        "assistant",
        "beacon suggestion",
        json!({"metadata":{"chatgpt":{"node_id":"node-anchor","parent_id":"node-before","omitted_parent_nodes":1}}}),
    );
    let page = call(&v, args(8192)).unwrap();
    assert_eq!(refs(&page), vec![anchor.as_str(), before.as_str()]);
    assert!(reasons(&page).contains("source_export_gap"));
    let read = transport::invoke(
        &Context::new(&v, Access::new(vec!["personal".into()]).unwrap()),
        "read",
        json!({"refs":[anchor],"include_adjacent":true,"detail":"compact","budget_bytes":8192}),
    )
    .unwrap();
    assert_eq!(
        read["results"][0]["metadata"]["adjacent"]["previous_ref"],
        before
    );
}
