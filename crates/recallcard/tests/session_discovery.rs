//! Independent synthetic retrieval and assessment workflow. No model quality/gold claims.
use recallcard::{context::Context, event_vectors, model::hash, policy::Access, transport, Vault};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Command, Stdio},
};
fn event(v: &Vault, id: &str, session: &str, text: &str, extra: Value) -> String {
    let mut x = json!({"role":"user","origin":"native","scope":"personal","content":text,"session_id":session,
        "occurred_at":"2026-01-02T00:00:00Z","source":{"platform":"toy","account_namespace":"owner","conversation_id":session,"message_id":id},
        "metadata":{"qwen":{"node_id":id,"parent_id":null,"on_current_path":true}}});
    for (k, v) in extra.as_object().unwrap() {
        x[k] = v.clone();
    }
    format!(
        "event:{}",
        v.capture(serde_json::from_value(x).unwrap()).unwrap().id
    )
}
fn context(v: &Vault) -> Context<'_> {
    Context::new(v, Access::new(vec!["personal".into()]).unwrap())
}
fn search() -> Value {
    json!({"query":"beacon","target":"events","detail":"session_discovery","limit":3,"budget_bytes":16384})
}
fn discover(v: &Vault, a: Value) -> Value {
    transport::invoke(&context(v), "search", a).unwrap()
}
fn chosen(item: &Value) -> Value {
    let mut read = item["read_args"].clone();
    read["context_confirmation"]["assessment"] = json!({"decision":"relevant","selected_anchor_ref":item["anchor_ref"],"evidence_indices":[0]});
    read
}
// A deliberately narrow synthetic automatic caller: an explicit task identifier
// must occur in exactly one reviewed packet. No expected source/ref enters this rule.
fn toy_identifier_assessment(page: &Value, identifier: &str) -> Option<Value> {
    let matches = page["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| {
            let evidence = item["context"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .filter(|(_, part)| part["text"].as_str().unwrap().contains(identifier))
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            if evidence.is_empty() || item["confirmation_available"] != true {
                return None;
            }
            let mut args = chosen(item);
            args["context_confirmation"]["assessment"]["evidence_indices"] = json!(evidence);
            Some(args)
        })
        .collect::<Vec<_>>();
    (matches.len() == 1).then(|| matches[0].clone())
}
fn rehash(read: &mut Value) {
    read["context_confirmation"]["binding_hash"] = json!(hash(
        read["context_confirmation"]["binding"]
            .to_string()
            .as_bytes()
    ));
}
fn uncertain(v: &Vault, read: Value) {
    let result = transport::invoke(&context(v), "read", read).unwrap();
    assert_eq!(result["status"], "uncertain");
    assert!(result["results"].as_array().unwrap().is_empty());
    assert_eq!(result["deep_read_performed"], false);
}
fn space() -> Value {
    json!({"schema":"private-vector-cache-v1","endpoint":"https://embedding.invalid/v1/embeddings","model":"discovery-toy","dimensions":3})
}
fn vecfile(root: &Path, text: &str, v: [f64; 3]) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("metadata.json"), space().to_string()).unwrap();
    let binding=hash(b"{\"dimensions\": 3, \"endpoint\": \"https://embedding.invalid/v1/embeddings\", \"model\": \"discovery-toy\", \"schema\": \"private-vector-cache-v1\"}");
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
    fs::write(root.join(format!("{digest}.bin")), raw).unwrap();
}
#[test]
fn independent_channel_session_dedup_precedes_round_robin_and_preserves_raw_scores() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let cache = d.path().join("cache");
    let mut chunks = vec![];
    for i in 0..270 {
        let t = format!("generic unrelated followup {i}");
        let r = event(&v, &format!("g{i}"), "generic", &t, json!({}));
        vecfile(&cache, &t, [1., 0., 0.]);
        chunks.push(
            json!({"ref":r,"start_byte":0,"end_byte":t.len(),"text_sha256":hash(t.as_bytes())}),
        );
    }
    let t = "ZX-9 inspection accepted after the latch correction";
    let target = event(&v, "named", "specific", t, json!({"role":"assistant"}));
    vecfile(&cache, t, [0., 1., 0.]);
    chunks.push(
        json!({"ref":target,"start_byte":0,"end_byte":t.len(),"text_sha256":hash(t.as_bytes())}),
    );
    let pure = "a semantically distant but independently indexed toy source";
    let pure_ref = event(&v, "pure", "pure", pure, json!({}));
    vecfile(&cache, pure, [-1., 0., 0.]);
    chunks.push(json!({"ref":pure_ref,"start_byte":0,"end_byte":pure.len(),"text_sha256":hash(pure.as_bytes())}));
    let q = "ZX-9 inspection";
    vecfile(&cache, q, [1., 0., 0.]);
    fs::write(
        d.path().join("manifest.json"),
        json!({"schema":"recallcard.event-vector-manifest/1","space":space(),"chunks":chunks})
            .to_string(),
    )
    .unwrap();
    event_vectors::import(
        &v,
        Access::new(vec!["personal".into()]).unwrap(),
        &d.path().join("manifest.json"),
        &cache,
        &d.path().join("index.json"),
    )
    .unwrap();
    let index = event_vectors::EventVectorIndex::open(&d.path().join("index.json"))
        .unwrap()
        .with_ranking("semantic_first")
        .unwrap();
    let ctx = context(&v).with_event_vectors(Some(&index));
    let mut a = search();
    a["query"] = json!(q);
    let page = transport::invoke(&ctx, "search", a.clone()).unwrap();
    assert_eq!(page["results"].as_array().unwrap().len(), 3);
    assert_eq!(page["results"][0]["channels"]["semantic"]["raw_score"], 1.0);
    let named = &page["results"][1];
    assert_eq!(named["anchor_ref"], target);
    assert_eq!(named["first_channel"], "lexical");
    assert_eq!(named["channels"]["semantic"]["raw_score"], 0.0);
    assert!(named["channels"]["lexical"]["raw_score"].as_f64().unwrap() > 0.);
    assert_eq!(page["coverage"]["semantic_sessions"], 3);
    assert_eq!(page["results"][2]["anchor_ref"], pure_ref);
    assert_eq!(
        page["results"][2]["channels"]["semantic"]["raw_score"],
        -1.0
    );
    assert_eq!(page["coverage"]["lexical_sessions"], 1);
    assert_eq!(page["relevance"], "unassessed");
    assert!(!page.to_string().contains("\"confidence\""));
    a["detail"] = json!("session_candidates");
    let legacy = transport::invoke(&ctx, "search", a).unwrap();
    assert_eq!(legacy["results"][0]["session_ref"], "generic");
    let assessed = toy_identifier_assessment(&page, "ZX-9").unwrap();
    let read = transport::invoke(&ctx, "read", assessed).unwrap();
    assert_eq!(read["results"][0]["ref"], target);
    assert_eq!(read["relevance"], "caller_assessed");
}
#[test]
fn default_is_uncertain_and_an_automatic_caller_must_supply_selected_evidence() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    event(&v, "one", "topic", "beacon original body", json!({}));
    let p = discover(&v, search());
    let item = &p["results"][0];
    uncertain(&v, item["read_args"].clone());
    for assessment in [
        json!({"decision":"irrelevant","selected_anchor_ref":item["anchor_ref"],"evidence_indices":[0]}),
        json!({"decision":"relevant","selected_anchor_ref":null,"evidence_indices":[0]}),
        json!({"decision":"relevant","selected_anchor_ref":item["anchor_ref"],"evidence_indices":[]}),
    ] {
        let mut a = chosen(item);
        a["context_confirmation"]["assessment"] = assessment;
        uncertain(&v, a);
    }
    let r = transport::invoke(&context(&v), "read", chosen(item)).unwrap();
    assert_eq!(r["relevance"], "caller_assessed");
    assert_eq!(r["evidence_binding"], "verified");
    assert_eq!(r["results"][0]["text"], "beacon original body");
    let mut legacy = item["read_args"].clone();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("context_confirmation");
    assert_eq!(
        transport::invoke(&context(&v), "read", legacy).unwrap()["results"][0]["text"],
        "beacon original body"
    );
}
#[test]
fn preview_is_original_anchor_previous_and_unique_next_with_roles() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let prev = event(&v, "p", "topic", "Earlier explicit requirement", json!({}));
    let anchor = event(
        &v,
        "a",
        "topic",
        "beacon proposed implementation",
        json!({"role":"assistant","reply_to":prev.trim_start_matches("event:")}),
    );
    let next = event(
        &v,
        "n",
        "topic",
        "Later correction: preserve the original",
        json!({"reply_to":anchor.trim_start_matches("event:")}),
    );
    let p = discover(&v, search());
    let parts = p["results"][0]["context"].as_array().unwrap();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0]["ref"], anchor);
    assert_eq!(parts[1]["ref"], prev);
    assert_eq!(parts[2]["ref"], next);
    assert_eq!(parts[0]["role"], "assistant");
    assert_eq!(parts[2]["relation"], "next");
    for part in parts {
        let text = v
            .event(part["ref"].as_str().unwrap().trim_start_matches("event:"))
            .unwrap()
            .data
            .text();
        let r = &part["text_range"];
        assert_eq!(
            &text[r["start_byte"].as_u64().unwrap() as usize
                ..r["end_byte"].as_u64().unwrap() as usize],
            part["text"].as_str().unwrap()
        );
    }
}
#[test]
fn mutated_boundaries_or_evidence_never_return_text_even_with_recomputed_hash() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    event(&v, "a", "topic", "beacon 界🦀 true original", json!({}));
    let p = discover(&v, search());
    let base = chosen(&p["results"][0]);
    for (field, value) in [
        ("event_filter", json!({"role":"assistant"})),
        ("as_of", json!("2026-01-03T00:00:00Z")),
        ("context_anchor_ref", json!("event:wrong")),
    ] {
        let mut a = base.clone();
        a[field] = value;
        uncertain(&v, a);
    }
    for field in [
        "anchor_snapshot",
        "source_group_key",
        "scopes_hash",
        "graph_generation",
    ] {
        let mut a = base.clone();
        a["context_confirmation"]["binding"][field] = json!("0".repeat(64));
        rehash(&mut a);
        uncertain(&v, a);
    }
    for (field, value) in [
        ("snapshot", json!("0".repeat(64))),
        ("text_hash", json!("0".repeat(64))),
        ("projection", json!("other.text")),
        ("start_byte", json!(8)),
        ("end_byte", json!(99999)),
    ] {
        let mut a = base.clone();
        a["context_confirmation"]["binding"]["evidence"][0][field] = value;
        rehash(&mut a);
        uncertain(&v, a);
    }
    for indices in [json!([0, 0]), json!([3])] {
        let mut a = base.clone();
        a["context_confirmation"]["assessment"]["evidence_indices"] = indices;
        uncertain(&v, a);
    }
}
#[test]
fn original_anchor_and_graph_changes_invalidate_assessment() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let a = event(&v, "a", "topic", "beacon original", json!({}));
    let p = discover(&v, search());
    let read = chosen(&p["results"][0]);
    event(
        &v,
        "child",
        "topic",
        "A newly appended graph child",
        json!({"reply_to":a.trim_start_matches("event:")}),
    );
    uncertain(&v, read);
    let p = discover(&v, search());
    let read = chosen(&p["results"][0]);
    v.suppress(
        a.trim_start_matches("event:"),
        "synthetic withdrawal".into(),
    )
    .unwrap();
    uncertain(&v, read);
}
#[test]
fn foreign_same_session_and_sibling_are_not_accepted_as_graph_steps() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let p = event(&v, "parent", "topic", "parent requirement", json!({}));
    let a = event(
        &v,
        "a",
        "topic",
        "beacon offpath anchor",
        json!({"reply_to":p.trim_start_matches("event:"),"metadata":{"qwen":{"node_id":"a","on_current_path":false}}}),
    );
    let sibling = event(
        &v,
        "b",
        "topic",
        "parallel answer",
        json!({"reply_to":p.trim_start_matches("event:"),"metadata":{"qwen":{"node_id":"b","on_current_path":true}}}),
    );
    let foreign = event(
        &v,
        "foreign",
        "topic",
        "foreign account source",
        json!({"source":{"platform":"toy","account_namespace":"different","conversation_id":"topic","message_id":"foreign"}}),
    );
    let page = discover(&v, search());
    assert_eq!(page["results"][0]["anchor_ref"], a);
    assert!(!page["results"][0]["context"].to_string().contains(&sibling));
    for reference in [sibling, foreign] {
        let mut read = chosen(&page["results"][0]);
        read["refs"] = json!([reference]);
        read["offset_bytes"] = json!(0);
        uncertain(&v, read);
    }
}
#[test]
fn role_filter_and_hidden_neighbor_stop_preview_without_scope_leak() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let p = event(
        &v,
        "p",
        "topic",
        "private antecedent",
        json!({"role":"assistant"}),
    );
    event(
        &v,
        "a",
        "topic",
        "beacon visible user",
        json!({"reply_to":p.trim_start_matches("event:")}),
    );
    let hidden = event(
        &v,
        "hidden",
        "elsewhere",
        "beacon secret",
        json!({"scope":"project:private"}),
    );
    let mut args = search();
    args["event_filter"] = json!({"role":"user"});
    let page = discover(&v, args);
    assert!(!page.to_string().contains(&hidden));
    assert_eq!(page["results"][0]["context"].as_array().unwrap().len(), 1);
    assert!(page["results"][0]["context_limits"]["graph_stop_reasons"]
        .as_array()
        .unwrap()
        .contains(&json!("request_filter_boundary")));
}
#[test]
fn empty_filter_canonicalization_does_not_silently_widen_nonempty_filter() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    event(&v, "a", "topic", "beacon body", json!({}));
    let p = discover(&v, search());
    let mut read = chosen(&p["results"][0]);
    read["event_filter"] = json!({});
    assert!(
        transport::invoke(&context(&v), "read", read).unwrap()["results"]
            .as_array()
            .unwrap()
            .len()
            == 1
    );
}
#[test]
fn minimal_budget_retry_and_discovery_cursor_make_progress_without_repeating_groups() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    for i in 0..4 {
        event(
            &v,
            &format!("a{i}"),
            &format!("s{i}"),
            &format!("beacon {}", "界🦀".repeat(3000)),
            json!({}),
        );
    }
    let mut args = search();
    args["budget_bytes"] = json!(512);
    let tiny = discover(&v, args.clone());
    assert_eq!(tiny["status"], "budget_exhausted");
    assert!(serde_json::to_vec(&tiny).unwrap().len() <= 512);
    assert!(tiny["next_cursor"].is_null());
    args["budget_bytes"] = tiny["recommended_min_budget"].clone();
    let retry = discover(&v, args);
    assert!(!retry["results"].as_array().unwrap().is_empty());
    let mut args = search();
    args["limit"] = json!(1);
    let first = discover(&v, args.clone());
    let cursor = first["next_cursor"].clone();
    let mut seen = std::collections::BTreeSet::new();
    let mut page = first;
    loop {
        for row in page["results"].as_array().unwrap() {
            assert!(seen.insert(row["anchor_ref"].as_str().unwrap().to_owned()));
        }
        if page["next_cursor"].is_null() {
            break;
        }
        args["cursor"] = page["next_cursor"].clone();
        page = discover(&v, args.clone());
    }
    assert_eq!(seen.len(), 4);
    for (field, value) in [
        ("detail", json!("session_candidates")),
        ("limit", json!(2)),
        ("budget_bytes", json!(8192)),
        ("event_filter", json!({"role":"user"})),
    ] {
        let mut changed = args.clone();
        changed["cursor"] = cursor.clone();
        changed[field] = value;
        assert!(transport::invoke(&context(&v), "search", changed).is_err());
    }
}
#[test]
fn confirmed_text_cursor_cannot_drop_or_change_the_assessment() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    event(
        &v,
        "a",
        "topic",
        &format!("beacon {}", "long original ".repeat(3000)),
        json!({}),
    );
    let p = discover(&v, search());
    let mut read = chosen(&p["results"][0]);
    read["budget_bytes"] = json!(2048);
    let first = transport::invoke(&context(&v), "read", read.clone()).unwrap();
    assert!(first["next_cursor"].is_string());
    read.as_object_mut().unwrap().remove("offset_bytes");
    read["cursor"] = first["next_cursor"].clone();
    assert!(
        transport::invoke(&context(&v), "read", read.clone()).unwrap()["results"]
            .as_array()
            .unwrap()
            .len()
            == 1
    );
    read.as_object_mut().unwrap().remove("context_confirmation");
    assert!(transport::invoke(&context(&v), "read", read).is_err());
}
#[test]
fn actual_mcp_discovery_automatic_assessment_and_guarded_read_roundtrip() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    event(&v, "a", "topic", "beacon original evidence", json!({}));
    let mut child = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--vault")
        .arg(d.path())
        .args(["mcp", "--scope", "personal"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut rpc = |request: Value| {
        writeln!(input, "{request}").unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        serde_json::from_str::<Value>(&line).unwrap()
    };
    rpc(
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),
    );
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    )
    .unwrap();
    let mut call = |id, name: &str, args: Value| {
        writeln!(input,"{}",json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}})).unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let r: Value = serde_json::from_str(&line).unwrap();
        assert!(r.get("error").is_none(), "{r}");
        serde_json::from_str::<Value>(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let page = call(2, "search", search());
    assert_eq!(
        call(3, "read", page["results"][0]["read_args"].clone())["status"],
        "uncertain"
    );
    let read = call(4, "read", chosen(&page["results"][0]));
    assert_eq!(read["relevance"], "caller_assessed");
    assert_eq!(read["results"][0]["text"], "beacon original evidence");
    drop(input);
    assert!(child.wait().unwrap().success());
}

#[test]
fn shrinking_lexical_preview_keeps_the_actual_late_match_and_read_offset() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    event(
        &v,
        "a",
        "one",
        &format!("{}beacon {}", "x ".repeat(200), "after ".repeat(500)),
        json!({}),
    );
    let mut args = search();
    args["limit"] = json!(1);
    args["budget_bytes"] = json!(512);
    let tiny = discover(&v, args.clone());
    args["budget_bytes"] = tiny["recommended_min_budget"].clone();
    args["cursor"] = tiny["retry_cursor"].clone();
    let page = discover(&v, args);
    let item = &page["results"][0];
    assert_eq!(item["context_limits"]["max_each_utf8_bytes"], 64);
    assert!(item["context"][0]["text"]
        .as_str()
        .unwrap()
        .contains("beacon"));
    assert_eq!(
        item["read_args"]["offset_bytes"],
        item["context"][0]["text_range"]["start_byte"]
    );
    assert!(
        transport::invoke(&context(&v), "read", chosen(item)).unwrap()["results"][0]["text"]
            .as_str()
            .unwrap()
            .contains("beacon")
    );
}
#[test]
fn a_larger_later_candidate_can_retry_at_its_offset_with_new_bound_budget() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    let simple = event(&v, "simple", "one", "beacon beacon beacon", json!({}));
    let prior = event(&v, "prior", "two", "unrelated previous words", json!({}));
    let larger = event(
        &v,
        "larger",
        "two",
        &format!(
            "{}beacon {}",
            "unrelated ".repeat(100),
            "after ".repeat(100)
        ),
        json!({"reply_to":prior.trim_start_matches("event:")}),
    );
    event(
        &v,
        "next",
        "two",
        "a subsequent original message",
        json!({"reply_to":larger.trim_start_matches("event:")}),
    );
    let mut args = search();
    args["limit"] = json!(1);
    args["budget_bytes"] = json!(512);
    let tiny = discover(&v, args.clone());
    args["budget_bytes"] = tiny["recommended_min_budget"].clone();
    args["cursor"] = tiny["retry_cursor"].clone();
    let first = discover(&v, args.clone());
    assert_eq!(first["results"][0]["anchor_ref"], simple);
    args["cursor"] = first["next_cursor"].clone();
    let second = discover(&v, args.clone());
    assert_eq!(second["status"], "budget_exhausted");
    let old_cursor = args["cursor"].clone();
    args["budget_bytes"] = second["recommended_min_budget"].clone();
    assert!(transport::invoke(&context(&v), "search", args.clone()).is_err());
    args["cursor"] = second["retry_cursor"].clone();
    assert_ne!(args["cursor"], old_cursor);
    let retried = discover(&v, args);
    assert_eq!(retried["results"][0]["anchor_ref"], larger);
    assert!(retried["next_cursor"].is_null());
}
#[test]
fn one_cyclic_source_graph_does_not_hide_other_candidate_sessions() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    event(
        &v,
        "a",
        "broken",
        "beacon cyclic first",
        json!({"metadata":{"qwen":{"node_id":"a","parent_id":"b","on_current_path":true}}}),
    );
    event(
        &v,
        "b",
        "broken",
        "beacon cyclic second",
        json!({"metadata":{"qwen":{"node_id":"b","parent_id":"a","on_current_path":true}}}),
    );
    let normal = event(&v, "ok", "normal", "beacon normal evidence", json!({}));
    let page = discover(&v, search());
    assert_eq!(page["results"].as_array().unwrap().len(), 2);
    let broken = page["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["confirmation_available"] == false)
        .unwrap();
    uncertain(&v, chosen(broken));
    let valid = page["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["anchor_ref"] == normal)
        .unwrap();
    assert_eq!(
        transport::invoke(&context(&v), "read", chosen(valid)).unwrap()["results"][0]["ref"],
        normal
    );
}

#[test]
fn synthetic_automatic_caller_abstains_on_absent_or_ambiguous_identifiers() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(d.path()).unwrap();
    event(&v, "a", "one", "beacon topic shared", json!({}));
    event(&v, "b", "two", "beacon another shared topic", json!({}));
    let page = discover(&v, search());
    assert!(toy_identifier_assessment(&page, "absent").is_none());
    assert!(toy_identifier_assessment(&page, "shared").is_none());
}
