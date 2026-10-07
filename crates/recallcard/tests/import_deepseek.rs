//! 完全人工构造的官方形状样本，不含真实用户历史或附件。
use recallcard::{
    import::import_text,
    import_bundle::*,
    model::{EventInput, Role},
    Vault,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    io::{Cursor, Write},
};
use zip::{write::SimpleFileOptions, ZipWriter};

fn conversation(id: &str) -> Value {
    json!({"id":id,"title":"合成分支演示","inserted_at":"2026-01-02T00:00:00Z","updated_at":"2026-01-02T00:01:00Z",
    "mapping":{
        "root":{"id":"root","parent":null,"children":["q"],"message":null},
        "q":{"id":"q","parent":"root","children":["a","b"],"message":{"files":[],"model":null,"inserted_at":"2026-01-02T08:00:00+08:00","fragments":[{"type":"REQUEST","content":"人工构造的问题"}]}},
        "a":{"id":"a","parent":"q","children":[],"message":{"files":[],"model":"deepseek-chat","inserted_at":"2026-01-02T00:00:02.125Z","fragments":[{"type":"THINK","content":"不得保存的隐藏样本文字"},{"type":"RESPONSE","content":"合成回答甲"}]}},
        "b":{"id":"b","parent":"q","children":[],"message":{"files":[],"model":"deepseek-chat","inserted_at":"2026-01-02T00:00:01Z","fragments":[{"type":"TEMPLATE_RESPONSE","content":"合成回答乙"}]}}
    }})
}
fn archive(value: &Value) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("conversations.json", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(value.to_string().as_bytes()).unwrap();
    zip.finish().unwrap().into_inner()
}
fn parsed(value: &Value) -> ParsedImport {
    parse_import_bytes("deepseek-export", value.to_string().as_bytes(), "personal").unwrap()
}

#[test]
fn official_fragments_preserve_all_siblings_roles_original_time_and_graph() {
    let p = parsed(&json!([conversation("one")]));
    assert_eq!(p.events.len(), 3);
    assert_eq!(
        p.events
            .iter()
            .map(|e| e.source.message_id.as_str())
            .collect::<Vec<_>>(),
        ["q", "a", "b"]
    );
    assert_eq!(p.events[0].role, Role::User);
    assert!(p.events[1..].iter().all(|e| e.role == Role::Assistant));
    assert!(p
        .events
        .iter()
        .all(|e| e.source.platform == "deepseek"
            && e.capture.completeness.as_deref() == Some("partial")));
    assert!(p
        .events
        .iter()
        .all(|e| e.metadata["import_adapter"] == "deepseek-export"));
    assert_eq!(
        p.events[0].occurred_at.unwrap().to_rfc3339(),
        "2026-01-02T00:00:00+00:00"
    );
    assert_eq!(
        p.events[1].occurred_at.unwrap().timestamp_subsec_millis(),
        125
    );
    assert!(p.events[1].occurred_at > p.events[2].occurred_at);
    assert_eq!(p.events[1].metadata["previous_message_id"], "q");
    assert_eq!(p.events[2].metadata["previous_message_id"], "q");
    assert_eq!(
        p.events[0].metadata["deepseek"]["parent_is_structural_root"],
        true
    );
    assert!(p
        .events
        .iter()
        .all(|e| e.metadata["deepseek"]["conversation_has_branches"] == true));
    assert_eq!(
        p.events[0].metadata["deepseek"]["children_ids"],
        json!(["a", "b"])
    );
    assert_eq!(p.coverage.deepseek.nodes_seen, 4);
    assert_eq!(p.coverage.deepseek.message_nodes_seen, 3);
    assert_eq!(p.coverage.deepseek.visible_messages_imported, 3);
    assert_eq!(p.coverage.deepseek.hidden_fragments_skipped, 1);
    assert_eq!(p.coverage.deepseek.branch_points, 1);
    assert_eq!(p.conversation_summaries[0].selection_key, "deepseek:one");
}

#[test]
fn object_array_zip_and_root_null_variants_have_stable_identity() {
    let value = conversation("one");
    let expected = parsed(&value);
    assert_eq!(expected.events, parsed(&json!([value.clone()])).events);
    assert_eq!(
        expected.events,
        parse_import_bytes("auto", &archive(&json!([value.clone()])), "personal")
            .unwrap()
            .events
    );
    let mut equivalent = value;
    equivalent["mapping"]["root"]
        .as_object_mut()
        .unwrap()
        .remove("parent");
    equivalent["updated_at"] = json!("2026-02-02T00:00:00Z");
    equivalent["mapping"]["q"]["children"] = json!(["b", "a"]);
    assert_eq!(expected.events, parsed(&equivalent).events);
}

#[test]
fn hidden_tools_attachment_payloads_never_enter_events_or_metadata() {
    let mut value = conversation("one");
    value["mapping"]["q"]["message"]["files"] = json!([{"id":"synthetic-file","url":"https://example.invalid/private-download","content":"附件内容不保存"}]);
    value["mapping"]["a"]["message"]["fragments"].as_array_mut().unwrap().extend([
        json!({"type":"SEARCH","results":[{"url":"https://example.invalid/search-result","snippet":"搜索结果不保存"}]}),
        json!({"type":"READ_LINK","url":"https://example.invalid/read-link"}),
        json!({"type":"UNKNOWN_FUTURE","content":"未知正文不保存"})]);
    let p = parsed(&value);
    let output = serde_json::to_string(&p.events).unwrap();
    for forbidden in [
        "隐藏样本",
        "private-download",
        "附件内容",
        "search-result",
        "搜索结果",
        "read-link",
        "未知正文",
    ] {
        assert!(!output.contains(forbidden), "{forbidden}");
    }
    assert_eq!(p.coverage.deepseek.attachments_skipped, 1);
    assert_eq!(p.coverage.deepseek.unsupported_fragments_skipped, 3);
    assert_eq!(p.events[0].metadata["deepseek"]["attachments_omitted"], 1);
}

#[test]
fn mixed_roles_and_unsupported_messages_are_counted_without_role_guessing() {
    for (fragments, field) in [
        (
            json!([{"type":"REQUEST","content":"问题"},{"type":"RESPONSE","content":"回答"}]),
            "ambiguous_role_messages_skipped",
        ),
        (
            json!([{"type":"THINK","content":"隐藏内容"}]),
            "hidden_only_messages_skipped",
        ),
        (
            json!([{"type":"UNKNOWN","content":"未知内容"}]),
            "unsupported_messages_skipped",
        ),
        (
            json!([{"type":"RESPONSE","content":{"text":"不转换对象"}}]),
            "unsupported_messages_skipped",
        ),
        (
            json!([{"type":"RESPONSE","content":"  "}]),
            "empty_messages_skipped",
        ),
    ] {
        let mut value = conversation("one");
        value["mapping"]["q"]["message"]["fragments"] = fragments;
        let p = parsed(&value);
        assert_eq!(p.events.len(), 2);
        assert_eq!(
            serde_json::to_value(&p.coverage.deepseek).unwrap()[field],
            1
        );
        assert!(p
            .events
            .iter()
            .all(|e| e.metadata["deepseek"]["parent_message_omitted"] == true));
    }
}

#[test]
fn missing_time_is_not_import_time_and_invalid_provided_time_is_rejected() {
    let mut value = conversation("one");
    value["mapping"]["q"]["message"]
        .as_object_mut()
        .unwrap()
        .remove("inserted_at");
    value["mapping"]["a"]["message"]["inserted_at"] = Value::Null;
    let p = parsed(&value);
    assert!(p.events[0].occurred_at.is_none());
    assert!(p.events[1].occurred_at.is_none());
    assert_eq!(p.coverage.deepseek.missing_message_timestamps, 2);
    value["mapping"]["a"]["message"]["inserted_at"] = json!("yesterday");
    assert!(
        parse_import_bytes("deepseek-export", value.to_string().as_bytes(), "personal")
            .unwrap_err()
            .contains("inserted_at")
    );
}

#[test]
fn malformed_graphs_or_duplicate_identity_fail_before_import() {
    let base = conversation("one");
    let mut cases = Vec::new();
    let mut v = base.clone();
    v["mapping"]["a"]["id"] = json!("q");
    cases.push(v);
    let mut v = base.clone();
    v["mapping"]["a"]["parent"] = json!("missing");
    cases.push(v);
    let mut v = base.clone();
    v["mapping"]["q"]["children"] = json!(["a", "a"]);
    cases.push(v);
    let mut v = base.clone();
    v["mapping"]["q"]["children"] = json!(["a"]);
    cases.push(v);
    let mut v = base.clone();
    v["mapping"]["a"]["parent"] = Value::Null;
    cases.push(v);
    let mut v = base.clone();
    v["mapping"]["root"]["parent"] = json!("a");
    v["mapping"]["a"]["children"] = json!(["root"]);
    cases.push(v);
    let mut v = base.clone();
    v["mapping"]["a"]["message"]["fragments"] = json!("bad");
    cases.push(v);
    for v in cases {
        assert!(
            parse_import_bytes("deepseek-export", v.to_string().as_bytes(), "personal").is_err()
        );
    }
    let raw = base
        .to_string()
        .replacen("\"id\":\"one\"", "\"id\":\"one\",\"id\":\"two\"", 1);
    assert!(
        parse_import_bytes("deepseek-export", raw.as_bytes(), "personal")
            .unwrap_err()
            .contains("重复字段")
    );
}

#[test]
fn routing_is_structural_and_account_files_are_not_conversations() {
    let value = conversation("one");
    assert_eq!(
        detect_import_format(&value.to_string()).unwrap(),
        "deepseek-export"
    );
    assert!(
        parse_import_bytes("chatgpt-export", value.to_string().as_bytes(), "personal")
            .unwrap_err()
            .contains("平台")
    );
    let chatgpt = json!({"id":"one", "current_node":"root", "mapping":{"root":{"parent":null,"message":null}}});
    assert!(parse_import_bytes(
        "deepseek-export",
        chatgpt.to_string().as_bytes(),
        "personal"
    )
    .is_err());
    let account = json!({"id":"account", "email":"synthetic@example.invalid", "settings":{}});
    let p = parsed(&json!([value, account]));
    assert_eq!(p.coverage.unrecognized_json_values_skipped, 1);
    assert!(!serde_json::to_string(&p.events)
        .unwrap()
        .contains("synthetic@example.invalid"));
    assert!(detect_import_format(&json!({"id":"unknown","mapping":{}}).to_string()).is_err());
    let account_bytes = json!({"id":"account","email":"synthetic@example.invalid"})
        .to_string()
        .into_bytes();
    let conv_bytes = conversation("one").to_string().into_bytes();
    let summary = inspect_import_files("auto", &[&conv_bytes, &account_bytes], "personal").unwrap();
    assert_eq!(summary.coverage.json_files, 2);
    assert_eq!(summary.coverage.unrecognized_json_values_skipped, 1);
    assert_eq!(summary.coverage.events_available, 3);
}

#[test]
fn multiarchive_same_names_dedupe_by_provider_conversation_and_preserve_revisions() {
    let one = conversation("one");
    let mut revised = one.clone();
    revised["mapping"]["a"]["message"]["fragments"][1]["content"] = json!("修订回答甲");
    let first = archive(&json!([one.clone()]));
    let second = archive(&json!([one, conversation("two"), revised]));
    let summary = inspect_import_files("auto", &[&first, &second], "personal").unwrap();
    assert_eq!(summary.conversation_summaries.len(), 2);
    assert_eq!(summary.coverage.events_available, 7);
    assert_eq!(summary.coverage.duplicate_events_skipped, 5);
    let ids = summary
        .conversation_summaries
        .iter()
        .map(|c| c.selection_key.clone())
        .collect();
    let p = parse_import_files_selected("auto", &[&first, &second], "personal", &ids).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let events = vault.capture_batch(p.events.clone()).unwrap();
    let revised = events
        .iter()
        .find(|e| e.data.content == "修订回答甲")
        .unwrap();
    assert!(revised.data.revision_of.is_some());
    vault.capture_batch(p.events).unwrap();
    assert_eq!(vault.events().unwrap().len(), 7);
    assert!(
        parse_import_files_selected("auto", &[&first, &second], "personal", &BTreeSet::new())
            .unwrap()
            .events
            .is_empty()
    );
}

#[test]
fn same_source_ids_across_platforms_require_qualified_selection() {
    let cg = json!({"id":"one", "current_node":"root", "mapping":{"root":{"parent":null,"message":null}}});
    let bytes = archive(&json!([conversation("one"), cg]));
    let summary = inspect_archive(&bytes, "personal").unwrap();
    assert_eq!(summary.conversation_summaries.len(), 2);
    assert!(
        parse_archive_selected(&bytes, "personal", &BTreeSet::from(["one".into()]))
            .unwrap_err()
            .contains("重名")
    );
    assert_eq!(
        parse_archive_selected(&bytes, "personal", &BTreeSet::from(["deepseek:one".into()]))
            .unwrap()
            .events
            .len(),
        3
    );
}

#[test]
fn import_text_supports_deepseek_without_repeated_history() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let text = conversation("one").to_string();
    assert_eq!(
        import_text(&vault, "deepseek-export", &text, "personal").unwrap()["events_added"],
        3
    );
    assert_eq!(
        import_text(&vault, "deepseek-export", &text, "personal").unwrap()["events_added"],
        0
    );
}

#[test]
fn long_official_conversation_is_not_artificially_split_at_five_thousand() {
    let count = 5001;
    let mut mapping = serde_json::Map::new();
    for i in 0..count {
        let id = format!("m{i:05}");
        mapping.insert(id.clone(),json!({"id":id,"parent":if i==0 { Value::Null } else {json!(format!("m{:05}",i-1))},"children":if i+1==count {json!([])} else {json!([format!("m{:05}",i+1)])},"message":{"files":[],"fragments":[{"type":"REQUEST","content":"合成长度样本"}]}}));
    }
    let p = parsed(&json!({"id":"long","inserted_at":null,"updated_at":null,"mapping":mapping}));
    assert_eq!(p.events.len(), count);
}

#[test]
fn multi_file_job_and_json_limits_reject_without_truncation() {
    let bytes = conversation("one").to_string().into_bytes();
    assert!(inspect_import_files(
        "auto",
        &vec![bytes.as_slice(); MAX_IMPORT_FILES + 1],
        "personal"
    )
    .unwrap_err()
    .contains("32"));
    assert!(parse_import_bytes(
        "deepseek-export",
        &vec![b' '; MAX_JSON_BYTES + 1],
        "personal"
    )
    .unwrap_err()
    .contains("16 MiB"));
}

fn input(content: &str, id: &str) -> EventInput {
    serde_json::from_value(json!({"content":content,"role":"user","origin":"user_input","source":{"platform":"test","conversation_id":"batch","message_id":id}})).unwrap()
}
#[test]
fn batch_capture_preflights_all_inputs_and_replays_old_versions_idempotently() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let first = input("旧版", "m");
    let second = input("新版", "m");
    let mut invalid = input("bad", "bad");
    invalid.scope = "../escape".into();
    assert!(vault.capture_batch(vec![first.clone(), invalid]).is_err());
    assert!(vault.events().unwrap().is_empty());
    let mut missing_revision = input("找不到前版", "bad-reference");
    missing_revision.revision_of = Some(format!("evt_{}", "0".repeat(64)));
    assert!(vault
        .capture_batch(vec![first.clone(), missing_revision])
        .is_err());
    assert!(vault.events().unwrap().is_empty());
    let result = vault
        .capture_batch(vec![
            first.clone(),
            second.clone(),
            first.clone(),
            second.clone(),
        ])
        .unwrap();
    assert_eq!(result[0].id, result[2].id);
    assert_eq!(result[1].id, result[3].id);
    assert_eq!(
        result[1].data.revision_of.as_deref(),
        Some(result[0].id.as_str())
    );
    assert_eq!(vault.capture_batch(vec![first, second]).unwrap().len(), 2);
    assert_eq!(vault.events().unwrap().len(), 2);
}
#[test]
fn batch_progress_is_durable_and_cancellation_or_checkpoint_failure_is_retryable() {
    use std::cell::Cell;
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let inputs = vec![input("甲", "1"), input("乙", "2"), input("丙", "3")];
    let done = Cell::new(0);
    let output = vault
        .capture_batch_with_callbacks(
            inputs.clone(),
            || done.get() < 1,
            |existing| {
                assert!(existing.is_empty());
                Ok(())
            },
            |count, event, added| {
                assert!(added);
                assert!(!event.id.is_empty());
                done.set(count);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(output.len(), 1);
    assert_eq!(vault.events().unwrap().len(), 1);
    let mut flags = Vec::new();
    vault
        .capture_batch_with_progress(
            inputs.clone(),
            || true,
            |_, _, added| {
                flags.push(added);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(flags, [false, true, true]);
    assert_eq!(vault.events().unwrap().len(), 3);
    let extra = vec![input("丁", "4"), input("戊", "5")];
    assert!(vault
        .capture_batch_with_progress(
            extra.clone(),
            || true,
            |_, _, _| Err("模拟进度落盘失败".into())
        )
        .is_err());
    assert_eq!(vault.events().unwrap().len(), 4);
    vault.capture_batch(extra).unwrap();
    assert_eq!(vault.events().unwrap().len(), 5);
    assert!(vault
        .capture_batch_with_progress(inputs, || false, |_, _, _| panic!("取消后不可回调"))
        .unwrap()
        .is_empty());
}

#[test]
fn visible_parent_memo_crosses_omitted_nodes_without_fabricating_a_sibling_chain() {
    let mut value = conversation("with-gap");
    value["mapping"]["q"]["children"] = json!(["hidden", "b"]);
    value["mapping"]["hidden"] = json!({"id":"hidden","parent":"q","children":["connector"],"message":{"files":[],"fragments":[{"type":"THINK","content":"隐藏正文不进入接续"}]}});
    value["mapping"]["connector"] =
        json!({"id":"connector","parent":"hidden","children":["a"],"message":null});
    value["mapping"]["a"]["parent"] = json!("connector");
    let p = parsed(&value);
    let q = p
        .events
        .iter()
        .find(|e| e.source.message_id == "q")
        .unwrap();
    let a = p
        .events
        .iter()
        .find(|e| e.source.message_id == "a")
        .unwrap();
    let b = p
        .events
        .iter()
        .find(|e| e.source.message_id == "b")
        .unwrap();
    assert_eq!(a.metadata["deepseek"]["parent_id"], "connector");
    assert_eq!(a.metadata["deepseek"]["nearest_visible_parent_id"], "q");
    assert_eq!(a.metadata["deepseek"]["omitted_parent_nodes"], 2);
    assert_eq!(a.metadata["deepseek"]["parent_is_structural_root"], false);
    assert_eq!(b.metadata["deepseek"]["nearest_visible_parent_id"], "q");
    assert_eq!(b.metadata["deepseek"]["omitted_parent_nodes"], 0);
    assert!(q.metadata["deepseek"]["nearest_visible_parent_id"].is_null());
    assert_eq!(q.metadata["deepseek"]["omitted_parent_nodes"], 0);
    assert_eq!(p.coverage.deepseek.hidden_only_messages_skipped, 1);
    assert!(!serde_json::to_string(&p.events)
        .unwrap()
        .contains("隐藏正文不进入接续"));
}
