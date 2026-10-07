use recallcard::{
    import::import_text,
    import_bundle::{
        inspect_archive, parse_archive, parse_archive_selected, parse_import_bytes,
        read_import_file, MAX_ARCHIVE_BYTES, MAX_ARCHIVE_ENTRIES, MAX_BATCH_EVENTS, MAX_JSON_BYTES,
    },
    Origin, Role, Vault,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    io::{Cursor, Write},
};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

fn conversation(id: &str) -> Value {
    json!({"conversation_id":id,"title":"合成测试会话","current_node":"last","mapping":{
        "root":{"parent":null,"message":null},
        "first":{"parent":"root","message":{"id":"m-user","create_time":1_700_000_000.25,"author":{"role":"user"},"content":{"content_type":"text","parts":["合成问题"]}}},
        "last":{"parent":"first","message":{"id":"m-assistant","create_time":1_699_999_999.5,"author":{"role":"assistant"},"content":{"content_type":"text","parts":["合成回答"]}}},
        "unused":{"parent":"first","message":{"id":"unused","author":{"role":"assistant"},"content":{"content_type":"text","parts":["不在当前分支"]}}}
    }})
}

fn archive(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, contents) in entries {
        writer
            .start_file(
                *name,
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(contents).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn single(conv: Value) -> Vec<u8> {
    archive(&[("chat.json", conv.to_string().into_bytes())])
}

fn signature_offsets(bytes: &[u8], signature: &[u8]) -> Vec<usize> {
    bytes
        .windows(signature.len())
        .enumerate()
        .filter_map(|(index, value)| (value == signature).then_some(index))
        .collect()
}

fn set_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn set_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn lie_about_sizes(bytes: &mut [u8], size: u32) {
    for offset in signature_offsets(bytes, b"PK\x01\x02") {
        set_u32(bytes, offset + 24, size);
    }
    for offset in signature_offsets(bytes, b"PK\x03\x04") {
        set_u32(bytes, offset + 22, size);
    }
}

#[test]
fn object_official_array_and_zip_are_equivalent_with_source_title_time_and_order() {
    let value = conversation("synthetic-one");
    let object =
        parse_import_bytes("chatgpt-export", value.to_string().as_bytes(), "personal").unwrap();
    let array = parse_import_bytes(
        "chatgpt-export",
        json!([value]).to_string().as_bytes(),
        "personal",
    )
    .unwrap();
    let zipped = parse_archive(&single(value), "personal").unwrap();
    assert_eq!(object.events, array.events);
    assert_eq!(object.events, zipped.events);
    let events = object.events;
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].source.message_id, "m-user");
    assert_eq!(events[1].source.message_id, "m-assistant");
    assert_eq!(events[0].source.conversation_id, "synthetic-one");
    assert_eq!(events[0].metadata["conversation_title"], "合成测试会话");
    assert_eq!(
        events[0].occurred_at.unwrap().timestamp_subsec_millis(),
        250
    );
    assert!(events[0].occurred_at > events[1].occurred_at);
    assert_eq!(zipped.coverage.messages.other_branch_messages_skipped, 1);
}

#[test]
fn json_and_markdown_are_not_imported_twice_and_unknown_content_is_reported() {
    let bytes = archive(&[
        ("chat.json", conversation("one").to_string().into_bytes()),
        ("chat.md", b"human-readable duplicate".to_vec()),
        ("notes.md", b"unpaired notes".to_vec()),
        ("settings.json", br#"{"configuration":true}"#.to_vec()),
        ("damaged.json", b"not json".to_vec()),
        ("image.png", vec![0, 1, 2]),
    ]);
    let parsed = parse_archive(&bytes, "personal").unwrap();
    assert_eq!(parsed.events.len(), 2);
    assert_eq!(parsed.coverage.archive_entries, 6);
    assert_eq!(parsed.coverage.json_files, 3);
    assert_eq!(parsed.coverage.recognized_json_files, 1);
    assert_eq!(parsed.coverage.markdown_files_skipped, 2);
    assert_eq!(parsed.coverage.markdown_copies_skipped, 1);
    assert_eq!(parsed.coverage.other_files_skipped, 1);
    assert_eq!(parsed.coverage.invalid_json_files_skipped, 1);
    assert_eq!(parsed.coverage.unrecognized_json_values_skipped, 1);
    assert!(!parsed.coverage.notes.is_empty());
}

#[test]
fn filename_never_supplies_missing_source_identity() {
    let bytes = archive(&[(
        "conversations.json",
        br#"{"messages":["not a ChatGPT export"]}"#.to_vec(),
    )]);
    let parsed = parse_archive(&bytes, "personal").unwrap();
    assert!(parsed.events.is_empty());
    assert_eq!(parsed.coverage.recognized_json_files, 0);
    assert!(parsed
        .coverage
        .notes
        .iter()
        .any(|n| n.contains("没有可识别")));
    let mut missing_id = conversation("one");
    missing_id
        .as_object_mut()
        .unwrap()
        .remove("conversation_id");
    assert!(parse_archive(&single(missing_id), "personal")
        .unwrap_err()
        .contains("缺少编号"));
}

#[test]
fn no_json_has_explicit_zero_coverage() {
    let parsed = parse_archive(
        &archive(&[("chat.md", b"only a readable copy".to_vec())]),
        "personal",
    )
    .unwrap();
    assert_eq!(parsed.coverage.json_files, 0);
    assert!(parsed.events.is_empty());
    assert!(parsed
        .coverage
        .notes
        .iter()
        .any(|n| n.contains("没有可识别")));
}

#[test]
fn source_versions_deduplicate_but_identical_text_in_distinct_conversations_does_not() {
    let first = conversation("one");
    let mut revised = first.clone();
    revised["mapping"]["last"]["message"]["content"]["parts"] = json!(["修订的回答"]);
    let bytes = archive(&[
        ("one.json", first.to_string().into_bytes()),
        (
            "official.json",
            json!([first, conversation("two"), revised])
                .to_string()
                .into_bytes(),
        ),
    ]);
    let parsed = parse_archive(&bytes, "personal").unwrap();
    assert_eq!(parsed.events.len(), 5);
    assert_eq!(parsed.coverage.duplicate_events_skipped, 3);
    assert_eq!(parsed.conversation_summaries.len(), 2);
    assert_eq!(parsed.conversation_summaries[0].event_count, 3);
    assert_eq!(parsed.conversation_summaries[1].event_count, 2);
    let d = tempfile::tempdir().unwrap();
    let vault = Vault::init(&d.path().join("vault")).unwrap();
    for event in &parsed.events {
        vault.capture(event.clone()).unwrap();
    }
    for event in &parsed.events {
        vault.capture(event.clone()).unwrap();
    }
    assert_eq!(vault.events().unwrap().len(), 5);
}

#[test]
fn original_text_import_supports_object_and_remains_idempotent() {
    let d = tempfile::tempdir().unwrap();
    let vault = Vault::init(&d.path().join("vault")).unwrap();
    let value = conversation("one");
    assert_eq!(
        import_text(&vault, "chatgpt-export", &value.to_string(), "personal").unwrap()
            ["events_added"],
        2
    );
    assert_eq!(
        import_text(
            &vault,
            "chatgpt-export",
            &json!([value.clone(), value]).to_string(),
            "personal"
        )
        .unwrap()["events_added"],
        0
    );
}

#[test]
fn hidden_reasoning_is_filtered_before_roles_including_tool_messages() {
    let mut value = conversation("one");
    let mut parent = "last".to_owned();
    let hidden_fields = [
        json!({"channel":"analysis"}),
        json!({"metadata":{"channel":"analysis"}}),
        json!({"metadata":{"is_visually_hidden_from_conversation":true}}),
        json!({"metadata":{"reasoning_status":"is_reasoning"}}),
        json!({"metadata":{"is_thinking_preamble_message":true}}),
        json!({"content":{"content_type":"thoughts","parts":["合成隐藏内容"]}}),
        json!({"content":{"content_type":"reasoning_recap","text":"合成隐藏内容"}}),
    ];
    for (index, fields) in hidden_fields.into_iter().enumerate() {
        let name = format!("hidden-{index}");
        let mut message = json!({"id":name,"author":{"role":if index%2==0 {"tool"} else {"assistant"}},"content":{"content_type":"text","parts":["合成隐藏内容"]}});
        message
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        value["mapping"][&name] = json!({"parent":parent,"message":message});
        parent = name;
    }
    value["mapping"]["tool"] = json!({"parent":parent,"message":{"id":"tool-visible","author":{"role":"tool"},"content":{"content_type":"execution_output","text":"合成可见工具结果"}}});
    value["current_node"] = json!("tool");
    let parsed = parse_archive(&single(value), "personal").unwrap();
    assert_eq!(parsed.events.len(), 3);
    assert_eq!(
        parsed.coverage.messages.hidden_reasoning_messages_skipped,
        7
    );
    assert_eq!(parsed.events[2].role, Role::Tool);
    assert_eq!(parsed.events[2].origin, Origin::ToolOutput);
    assert!(parsed.events.iter().all(|e| !e.content.contains("隐藏")));
}

#[test]
fn missing_time_stays_missing_and_invalid_time_is_not_silently_replaced() {
    let mut value = conversation("one");
    value["mapping"]["first"]["message"]["create_time"] = Value::Null;
    value["mapping"]["last"]["message"]
        .as_object_mut()
        .unwrap()
        .remove("create_time");
    let parsed = parse_archive(&single(value.clone()), "personal").unwrap();
    assert!(parsed
        .events
        .iter()
        .all(|event| event.occurred_at.is_none()));
    value["mapping"]["last"]["message"]["create_time"] = json!("invalid");
    assert!(parse_archive(&single(value), "personal")
        .unwrap_err()
        .contains("create_time"));
}

#[test]
fn malformed_selected_branch_is_rejected_without_fallback() {
    for (path, replacement) in [
        ("current", json!("missing")),
        ("current", Value::Null),
        ("parent", json!("last")),
        ("parent", json!(32)),
        ("parent", json!("missing")),
    ] {
        let mut value = conversation("one");
        if path == "current" {
            value["current_node"] = replacement;
        } else {
            value["mapping"]["first"]["parent"] = replacement;
        }
        assert!(parse_archive(&single(value), "personal").is_err());
    }
}

#[test]
fn selection_uses_original_ids_and_unknown_ids_are_not_ignored() {
    let bytes = archive(&[(
        "all.json",
        json!([conversation("one"), conversation("two")])
            .to_string()
            .into_bytes(),
    )]);
    let summary = inspect_archive(&bytes, "personal").unwrap();
    assert_eq!(summary.conversation_summaries.len(), 2);
    assert_eq!(summary.conversation_summaries[0].source_id, "one");
    assert_eq!(summary.conversation_summaries[0].user_messages, 1);
    let selected = BTreeSet::from(["two".to_owned()]);
    let parsed = parse_archive_selected(&bytes, "personal", &selected).unwrap();
    assert_eq!(parsed.events.len(), 2);
    assert!(parsed
        .events
        .iter()
        .all(|event| event.source.conversation_id == "two"));
    assert_eq!(parsed.coverage.events_available, 4);
    assert_eq!(parsed.coverage.conversations_selected, 1);
    assert!(parse_archive_selected(&bytes, "personal", &BTreeSet::new())
        .unwrap()
        .events
        .is_empty());
    assert!(
        parse_archive_selected(&bytes, "personal", &BTreeSet::from(["missing".into()])).is_err()
    );
}

fn many_messages(id: &str, count: usize) -> Value {
    let mut mapping = serde_json::Map::new();
    for index in 0..count {
        let node = format!("n{index}");
        let parent = if index == 0 {
            Value::Null
        } else {
            json!(format!("n{}", index - 1))
        };
        mapping.insert(node.clone(), json!({"parent":parent,"message":{"id":node,"author":{"role":"user"},"content":{"content_type":"text","parts":["合成消息"]}}}));
    }
    json!({"id":id,"mapping":mapping,"current_node":format!("n{}",count-1)})
}

#[test]
fn large_backups_can_be_inspected_then_selected_in_batches_without_truncation() {
    let bytes = archive(&[(
        "all.json",
        json!([many_messages("one", MAX_BATCH_EVENTS), conversation("two")])
            .to_string()
            .into_bytes(),
    )]);
    let summary = inspect_archive(&bytes, "personal").unwrap();
    assert_eq!(summary.coverage.events_available, MAX_BATCH_EVENTS + 2);
    assert_eq!(
        summary.conversation_summaries[0].event_count,
        MAX_BATCH_EVENTS
    );
    let err = parse_archive(&bytes, "personal").unwrap_err();
    assert!(err.contains("5002") && err.contains("没有写入"));
    let selected =
        parse_archive_selected(&bytes, "personal", &BTreeSet::from(["one".into()])).unwrap();
    assert_eq!(selected.events.len(), MAX_BATCH_EVENTS);
}

#[test]
fn zip_path_traversal_absolute_windows_and_ambiguous_paths_are_rejected() {
    for name in [
        "../chat.json",
        "/chat.json",
        "a/../chat.json",
        "a/./chat.json",
        "C:/chat.json",
        "a\\chat.json",
        "a//chat.json",
    ] {
        let bytes = archive(&[(name, conversation("one").to_string().into_bytes())]);
        assert!(
            parse_archive(&bytes, "personal").is_err(),
            "unexpected path acceptance: {name}"
        );
    }
}

#[test]
fn duplicate_and_case_conflicting_entries_and_file_directory_conflicts_are_rejected() {
    let mut duplicate = archive(&[("a.json", b"{}".to_vec()), ("b.json", b"{}".to_vec())]);
    for index in signature_offsets(&duplicate, b"b.json") {
        duplicate[index] = b'a';
    }
    assert!(parse_archive(&duplicate, "personal")
        .unwrap_err()
        .contains("重复"));
    for entries in [
        vec![("A.json", b"{}".to_vec()), ("a.json", b"{}".to_vec())],
        vec![("dir", vec![]), ("dir/chat.json", b"{}".to_vec())],
        vec![("dir/chat.json", b"{}".to_vec()), ("dir", vec![])],
    ] {
        assert!(parse_archive(&archive(&entries), "personal").is_err());
    }
}

#[test]
fn symbolic_links_encryption_and_unsupported_compression_are_rejected() {
    let original = single(conversation("one"));
    let central = signature_offsets(&original, b"PK\x01\x02")[0];
    let local = signature_offsets(&original, b"PK\x03\x04")[0];
    let mut symlink = original.clone();
    set_u32(&mut symlink, central + 38, 0o120777 << 16);
    assert!(parse_archive(&symlink, "personal")
        .unwrap_err()
        .contains("符号链接"));
    let mut encrypted = original.clone();
    set_u16(&mut encrypted, central + 8, 1);
    set_u16(&mut encrypted, local + 6, 1);
    assert!(parse_archive(&encrypted, "personal")
        .unwrap_err()
        .contains("加密"));
    let mut unsupported = original;
    set_u16(&mut unsupported, central + 10, 99);
    set_u16(&mut unsupported, local + 8, 99);
    assert!(parse_archive(&unsupported, "personal")
        .unwrap_err()
        .contains("压缩"));
}

#[test]
fn archive_size_entry_count_and_declared_json_size_are_bounded() {
    assert!(parse_archive(&vec![0; MAX_ARCHIVE_BYTES + 1], "personal")
        .unwrap_err()
        .contains("64 MiB"));
    let mut original = single(conversation("one"));
    lie_about_sizes(&mut original, MAX_JSON_BYTES as u32 + 1);
    assert!(parse_archive(&original, "personal")
        .unwrap_err()
        .contains("16 MiB"));
    let names: Vec<String> = (0..=MAX_ARCHIVE_ENTRIES)
        .map(|index| format!("file-{index}"))
        .collect();
    let entries: Vec<(&str, Vec<u8>)> = names.iter().map(|name| (name.as_str(), vec![])).collect();
    assert!(parse_archive(&archive(&entries), "personal")
        .unwrap_err()
        .contains("2048"));
}

#[test]
fn json_bomb_is_counted_during_decompression_even_when_headers_understate_size() {
    let mut bytes = archive(&[("chat.json", vec![b' '; MAX_JSON_BYTES + 1])]);
    lie_about_sizes(&mut bytes, 1);
    assert!(parse_archive(&bytes, "personal")
        .unwrap_err()
        .contains("实际展开超过 16 MiB"));
}

#[test]
fn total_size_is_counted_even_for_skipped_non_json_files() {
    let mut declared = archive(&[("a", vec![]), ("b", vec![])]);
    lie_about_sizes(&mut declared, 65 * 1024 * 1024);
    assert!(parse_archive(&declared, "personal")
        .unwrap_err()
        .contains("总展开大小超过 128 MiB"));
    // 以小块生成高压缩率文件，测试本身不分配整个炸弹的展开内容。
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "ignored.bin",
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .unwrap();
    let block = [0u8; 8192];
    for _ in 0..(128 * 1024 * 1024 / block.len()) {
        writer.write_all(&block).unwrap();
    }
    writer.write_all(&[0]).unwrap();
    let mut dishonest = writer.finish().unwrap().into_inner();
    lie_about_sizes(&mut dishonest, 1);
    assert!(parse_archive(&dishonest, "personal")
        .unwrap_err()
        .contains("实际总展开大小超过 128 MiB"));
}

#[test]
fn conflicting_local_headers_corruption_and_truncation_are_rejected() {
    let original = single(conversation("one"));
    let mut local_name = original.clone();
    local_name[30] = b'x';
    assert!(parse_archive(&local_name, "personal")
        .unwrap_err()
        .contains("文件名"));
    let central = signature_offsets(&original, b"PK\x01\x02")[0];
    let mut bad_crc = original.clone();
    set_u32(&mut bad_crc, central + 16, 1);
    assert!(parse_archive(&bad_crc, "personal").is_err());
    for length in [0, 1, 21, original.len() - 1] {
        assert!(parse_archive(&original[..length], "personal").is_err());
    }
}

#[test]
fn bounded_file_entrypoint_sniffs_content_and_does_not_write_a_vault() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("user-selected.backup");
    std::fs::write(&path, single(conversation("one"))).unwrap();
    let parsed = read_import_file(&path, "chatgpt-export", "personal").unwrap();
    assert_eq!(parsed.events.len(), 2);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn stored_archives_and_safe_directories_are_supported() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_directory("backup/", SimpleFileOptions::default())
        .unwrap();
    writer
        .start_file(
            "backup/chat.json",
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )
        .unwrap();
    writer
        .write_all(conversation("one").to_string().as_bytes())
        .unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    let parsed = parse_archive(&bytes, "personal").unwrap();
    assert_eq!(parsed.events.len(), 2);
    assert_eq!(parsed.coverage.directories_skipped, 1);
}

#[test]
fn multimodal_text_keeps_visible_strings_and_reports_skipped_media() {
    let mut value = conversation("one");
    value["mapping"]["first"]["message"]["content"] = json!({"content_type":"multimodal_text", "parts":["可见问题",{"content_type":"image_asset_pointer","asset_pointer":"synthetic-image"}]});
    let parsed = parse_archive(&single(value), "personal").unwrap();
    assert_eq!(parsed.events[0].content, "可见问题");
    assert_eq!(parsed.events[0].role, Role::User);
    assert_eq!(
        parsed.coverage.messages.unsupported_content_parts_skipped,
        1
    );
}

#[cfg(unix)]
#[test]
fn file_entrypoint_rejects_symlinks() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("original.zip");
    let link = directory.path().join("link.zip");
    std::fs::write(&target, single(conversation("one"))).unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(read_import_file(&link, "chatgpt-export", "personal").is_err());
}
