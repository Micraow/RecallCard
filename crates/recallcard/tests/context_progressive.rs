//! Original toy graphs for bounded, constraint-preserving explicit continuation.
use recallcard::{context::Context, policy::Access, transport, Vault};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn event(v: &Vault, id: &str, parent: Option<&str>, role: &str, extra: Value) -> String {
    let mut input = json!({"role":role,"origin":"native","scope":"personal","session_id":"toy-session",
        "content":format!("original toy text {id}"),"occurred_at":"2026-01-02T00:00:00Z",
        "source":{"platform":"qwen","account_namespace":"toy","conversation_id":"one","message_id":id},
        "metadata":{"qwen":{"node_id":id,"parent_id":parent,"on_current_path":true}}});
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
fn request(reference: &str, anchor: &str) -> Value {
    json!({"refs":[reference],"context_anchor_ref":anchor,"include_adjacent":true,"detail":"compact","budget_bytes":4096})
}
fn call(v: &Vault, args: Value) -> Result<Value, String> {
    transport::invoke(
        &Context::new(v, Access::new(vec!["personal".into()]).unwrap()),
        "read",
        args,
    )
}
fn links(page: &Value) -> &Value {
    &page["results"][0]["metadata"]["adjacent"]
}

#[test]
fn role_filter_stops_at_intermediate_without_skipping_to_next_user() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let root = event(&v, "a", None, "user", json!({}));
    let mid = event(&v, "b", Some("a"), "assistant", json!({}));
    let end = event(&v, "c", Some("b"), "user", json!({}));
    let mut args = request(&root, &root);
    args["event_filter"] = json!({"role":"user"});
    let page = call(&v, args.clone()).unwrap();
    assert!(links(&page)["next_refs"].as_array().unwrap().is_empty());
    assert!(links(&page)["next_user_ref"].is_null());
    assert!(!page.to_string().contains(&mid) && !page.to_string().contains(&end));
    assert!(links(&page)["constraints"]["stop_reasons"]
        .to_string()
        .contains("request_filter_boundary"));
    args["refs"] = json!([end]);
    assert!(call(&v, args).is_err());
}

#[test]
fn filtered_fork_is_not_reinterpreted_as_a_unique_branch() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let root = event(
        &v,
        "a",
        None,
        "user",
        json!({"metadata":{"qwen":{"node_id":"a","parent_id":null}}}),
    );
    let kept = event(
        &v,
        "b",
        Some("a"),
        "user",
        json!({"metadata":{"qwen":{"node_id":"b","parent_id":"a"}}}),
    );
    let removed = event(
        &v,
        "c",
        Some("a"),
        "assistant",
        json!({"metadata":{"qwen":{"node_id":"c","parent_id":"a"}}}),
    );
    let mut args = request(&root, &root);
    args["event_filter"] = json!({"role":"user"});
    let page = call(&v, args.clone()).unwrap();
    assert_eq!(links(&page)["branch_choice_required"], true);
    assert!(links(&page)["next_refs"].as_array().unwrap().is_empty());
    assert!(!page.to_string().contains(&kept) && !page.to_string().contains(&removed));
    args["refs"] = json!([kept]);
    assert!(call(&v, args).unwrap_err().contains("分支"));
}

#[test]
fn original_anchor_prevents_backward_then_sibling_navigation() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let root = event(&v, "a", None, "user", json!({}));
    let main = event(&v, "b", Some("a"), "assistant", json!({}));
    let side = event(
        &v,
        "c",
        Some("a"),
        "assistant",
        json!({"metadata":{"qwen":{"node_id":"c","parent_id":"a","on_current_path":false}}}),
    );
    let page = call(&v, request(&root, &side)).unwrap();
    assert_eq!(
        links(&page)["constraints"]["direction_from_anchor"],
        "previous"
    );
    assert!(links(&page)["next_refs"].as_array().unwrap().is_empty());
    assert!(links(&page)["next_user_ref"].is_null());
    assert!(!page.to_string().contains(&main));
    assert!(call(&v, request(&main, &side)).is_err());
}

#[test]
fn time_path_and_as_of_boundaries_apply_before_text_and_adjacency() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let root = event(&v, "a", None, "user", json!({}));
    let future = event(
        &v,
        "b",
        Some("a"),
        "user",
        json!({"occurred_at":"2026-01-03T00:00:00Z"}),
    );
    let mut args = request(&root, &root);
    args["as_of"] = json!("2026-01-02T00:00:00Z");
    let page = call(&v, args.clone()).unwrap();
    assert!(!page.to_string().contains(&future));
    args["refs"] = json!([future]);
    assert!(call(&v, args).is_err());
    let mut args = request(&root, &root);
    args["event_filter"] = json!({"occurred_from":"2026-01-03T00:00:00Z"});
    assert!(call(&v, args).is_err());
    let mut args = request(&root, &root);
    args["event_filter"] = json!({"on_current_path":false});
    assert!(call(&v, args).is_err());
    let mut args = request(&root, &root);
    args["event_filter"] =
        json!({"occurred_from":"2026-01-03T00:00:00Z","occurred_until":"2026-01-01T00:00:00Z"});
    assert!(call(&v, args).is_err());
}

#[test]
fn source_identity_scope_suppression_and_revision_cannot_be_crossed() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let root = event(&v, "a", None, "user", json!({}));
    let mid = event(&v, "b", Some("a"), "assistant", json!({}));
    let tail = event(&v, "c", Some("b"), "user", json!({}));
    for extra in [
        json!({"scope":"project:private"}),
        json!({"source":{"platform":"qwen","account_namespace":"other","conversation_id":"one","message_id":"z"}}),
        json!({"source":{"platform":"qwen","account_namespace":"toy","conversation_id":"other","message_id":"z"}}),
        json!({"source":{"platform":"other","account_namespace":"toy","conversation_id":"one","message_id":"z"}}),
    ] {
        let foreign = event(&v, "z", Some("a"), "user", extra);
        assert!(call(&v, request(&foreign, &root)).is_err());
    }
    v.suppress(mid.trim_start_matches("event:"), "toy boundary".into())
        .unwrap();
    assert!(call(&v, request(&tail, &root)).is_err());
    let replacement = event(
        &v,
        "a",
        None,
        "user",
        json!({"content":"updated original toy record"}),
    );
    assert_ne!(root, replacement);
    assert!(call(&v, request(&root, &root)).is_err());
}

#[test]
fn constrained_cursor_binds_filters_anchor_budget_and_exact_utf8() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let text = "界🦀é".repeat(2200);
    let root = event(&v, "a", None, "user", json!({"content":text}));
    let base = request(&root, &root);
    let first = call(&v, base.clone()).unwrap();
    let cursor = first["next_cursor"].clone();
    assert!(cursor.is_string());
    for (field, value) in [
        ("event_filter", json!({"role":"user"})),
        ("as_of", json!("2026-01-04T00:00:00Z")),
        ("budget_bytes", json!(8192)),
        ("context_anchor_ref", Value::Null),
    ] {
        let mut changed = base.clone();
        changed["cursor"] = cursor.clone();
        changed[field] = value;
        assert!(call(&v, changed).is_err());
    }
    let mut got = first["results"][0]["text"].as_str().unwrap().to_owned();
    let mut page = first;
    while !page["next_cursor"].is_null() {
        let mut next = base.clone();
        next["cursor"] = page["next_cursor"].clone();
        page = call(&v, next).unwrap();
        let item = &page["results"][0];
        assert_eq!(item["text_range"]["start_byte"], got.len());
        got.push_str(item["text"].as_str().unwrap());
        assert!(serde_json::to_vec(&page).unwrap().len() <= 4096);
    }
    assert_eq!(got, text);
    let mut full = base;
    full["detail"] = json!("full");
    full["budget_bytes"] = json!(32768);
    let page = call(&v, full).unwrap();
    assert!(page["results"][0].get("record").is_none());
}

#[test]
fn restricted_options_require_single_event_and_filters_alone_are_enforced() {
    for tool in transport::tool_definitions().as_array().unwrap() {
        if tool["name"] == "read" || tool["name"] == "sources" {
            for field in ["event_filter", "as_of", "context_anchor_ref"] {
                assert!(tool["inputSchema"]["properties"].get(field).is_some());
            }
        }
    }
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let root = event(&v, "a", None, "user", json!({}));
    assert!(call(
        &v,
        json!({"refs":[root,root],"event_filter":{},"budget_bytes":4096})
    )
    .is_err());
    assert!(call(
        &v,
        json!({"refs":["view:nav/root"],"as_of":"2026-01-01T00:00:00Z"})
    )
    .is_err());
    assert!(call(&v, json!({"refs":[root],"context_anchor_ref":root})).is_err());
    assert!(call(
        &v,
        json!({"refs":[root],"event_filter":{"role":"assistant"}})
    )
    .is_err());
    let page = call(
        &v,
        json!({"refs":[root],"event_filter":{"role":"user"},"budget_bytes":4096}),
    )
    .unwrap();
    assert!(page["results"][0].get("record").is_none());
}

#[test]
fn real_stdio_mcp_reads_beyond_fixed_window_without_a_larger_search() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let mut refs = vec![];
    for i in 0..12 {
        refs.push(event(&v,&format!("n{i}"),(i>0).then(||format!("n{}",i-1)).as_deref(),if i%2==0{"user"}else{"assistant"},json!({"content":if i==0{"beacon original question"}else if i==11{"actual distant toy correction"}else{"intermediate original toy text"}})));
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--vault")
        .arg(d.path())
        .args(["mcp", "--scope", "personal"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut sequence = 0;
    let mut rpc = |method: &str, params: Value| {
        sequence += 1;
        writeln!(
            input,
            "{}",
            json!({"jsonrpc":"2.0","id":sequence,"method":method,"params":params})
        )
        .unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        if method == "initialize" {
            writeln!(
                input,
                "{}",
                json!({"jsonrpc":"2.0","method":"notifications/initialized"})
            )
            .unwrap();
            input.flush().unwrap();
        }
        serde_json::from_str::<Value>(&line).unwrap()
    };
    rpc("initialize", json!({"protocolVersion":"2025-11-25"}));
    let mut invoke = |name: &str, args: Value| {
        let reply = rpc("tools/call", json!({"name":name,"arguments":args}));
        assert!(reply.get("error").is_none(), "{reply}");
        assert_ne!(reply["result"]["isError"], true, "{reply}");
        serde_json::from_str::<Value>(reply["result"]["content"][0]["text"].as_str().unwrap())
            .unwrap()
    };
    let first = invoke(
        "search",
        json!({"query":"beacon","target":"events","detail":"session_context","budget_bytes":8192,"event_filter":{"on_current_path":true}}),
    );
    assert_eq!(first["sessions"][0]["selected_message_count"], 4);
    assert!(!first.to_string().contains(&refs[11]));
    assert_eq!(first["coverage"]["continuation"]["tool"], "read");
    let mut at = refs[3].clone();
    let mut calls = 1;
    let mut total = serde_json::to_vec(&first).unwrap().len();
    loop {
        let mut args = request(&at, &refs[0]);
        args["event_filter"] = json!({"on_current_path":true});
        let page = invoke("read", args);
        calls += 1;
        total += serde_json::to_vec(&page).unwrap().len();
        assert!(serde_json::to_vec(&page).unwrap().len() <= 4096);
        if at == refs[11] {
            assert_eq!(page["results"][0]["text"], "actual distant toy correction");
            break;
        }
        at = links(&page)["next_refs"][0].as_str().unwrap().to_owned();
        assert!(links(&page)["previous_ref"].is_null());
    }
    assert_eq!(calls, 10);
    assert!(total > 8192 && total <= 8192 + 9 * 4096);
    drop(input);
    assert!(child.wait().unwrap().success());
}

#[test]
fn ambiguous_endpoints_are_stops_not_unusable_continuation_links() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let root = event(&v, "a", None, "user", json!({}));
    let child = event(
        &v,
        "b",
        Some("a"),
        "assistant",
        json!({"metadata":{"qwen":{"node_id":"dup","parent_id":"a","on_current_path":true}}}),
    );
    event(
        &v,
        "other",
        None,
        "assistant",
        json!({"metadata":{"qwen":{"node_id":"dup","parent_id":null,"on_current_path":false}}}),
    );
    let page = call(&v, request(&root, &root)).unwrap();
    assert!(links(&page)["next_refs"].as_array().unwrap().is_empty());
    assert_eq!(links(&page)["constraints"]["may_continue"], false);
    assert!(links(&page)["constraints"]["stop_reasons"]
        .to_string()
        .contains("ambiguous_source_identity"));
    assert!(!page.to_string().contains(&child));
    let tail = event(
        &v,
        "tail",
        None,
        "user",
        json!({"reply_to":child.trim_start_matches("event:")}),
    );
    let page = call(&v, request(&tail, &tail)).unwrap();
    assert!(links(&page)["previous_ref"].is_null());
    assert_eq!(links(&page)["constraints"]["may_continue"], false);
    assert!(links(&page)["constraints"]["stop_reasons"]
        .to_string()
        .contains("ambiguous_source_identity"));
}
