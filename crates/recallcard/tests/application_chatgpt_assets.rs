//! 官方结构等价的合成附件测试；不读取用户归档、不执行附件内容。
use recallcard::{
    application::{import_reader::read_source, *},
    Vault,
};
use serde_json::{json, Value};
use std::io::{Cursor, Write};
use zip::{write::SimpleFileOptions, ZipWriter};
fn conversation() -> Value {
    json!({"id":"synthetic-conversation","title":"合成附件会话","current_node":"answer","is_do_not_remember":false,"memory_scope":"global_enabled","mapping":{
    "root":{"id":"root","parent":null,"message":null},
    "upload":{"id":"upload","parent":"root","message":{"id":"upload","author":{"role":"user"},"create_time":1770000000.125,"content":{"content_type":"multimodal_text","parts":[{"content_type":"image_asset_pointer","asset_pointer":"sediment://file_SYNTHETIC"}]},"metadata":{"parent_id":"untrusted-parent","attachments":[{"id":"file_SYNTHETIC","library_file_id":"libfile_SYNTHETIC","name":"message-name.png","mime_type":"image/jpeg","size":12}]}}},
    "hidden":{"id":"hidden","parent":"upload","message":{"id":"hidden","author":{"role":"assistant"},"content":{"content_type":"thoughts","thoughts":[{"content":"must not import"}]}}},
    "answer":{"id":"answer","parent":"hidden","message":{"id":"answer","author":{"role":"assistant"},"create_time":1769999999.5,"content":{"content_type":"text","parts":["合成可见回答"]}}},
    "alternate":{"id":"alternate","parent":"upload","message":{"id":"alternate","author":{"role":"assistant"},"create_time":null,"content":{"content_type":"text","parts":["合成替代分支"]}}}
    }})
}
fn files(payload: Option<&[u8]>) -> Vec<(String, Vec<u8>)> {
    let mut files=vec![
  ("export/conversations.json".into(),serde_json::to_vec(&vec![conversation()]).unwrap()),
  ("export/library_files.json".into(),serde_json::to_vec(&json!([{"file_id":"file_SYNTHETIC","id":{"id":"libfile_SYNTHETIC"},"file_name":"library-name.png","mime_type":"image/png","file_size_bytes":12,"state":"ready","origination_message_id":"different-origin"}])).unwrap()),
  ("export/conversation_asset_file_names.json".into(),serde_json::to_vec(&json!({"file_SYNTHETIC.dat":"archive-alias.png"})).unwrap()),
  ("export/export_manifest.json".into(),serde_json::to_vec(&json!({"version":1,"manifest_file":"export_manifest.json","export_files":[{"path":"file_SYNTHETIC.dat","size_bytes":12}],"logical_files":{"file_SYNTHETIC.dat":{"files":["file_SYNTHETIC.dat"],"sharded":false}}})).unwrap())];
    if let Some(bytes) = payload {
        files.push(("export/file_SYNTHETIC.dat".into(), bytes.to_vec()));
    }
    files
}
fn zip(files: Vec<(String, Vec<u8>)>) -> Vec<u8> {
    let mut z = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in files {
        z.start_file(
            name,
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
        z.write_all(&bytes).unwrap();
    }
    z.finish().unwrap().into_inner()
}
fn run(bytes: Vec<u8>, id: &str) -> (tempfile::TempDir, Vault, ImportService, JobStatus) {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let source = dir.path().join("official.zip");
    std::fs::write(&source, bytes).unwrap();
    let service = ImportService::new(&vault).unwrap();
    let job = service
        .submit(ImportRequest {
            request_id: id.into(),
            paths: vec![source],
            scope: "personal".into(),
            format: "auto".into(),
        })
        .unwrap();
    let result = service.run(&job.job_id, "personal").unwrap();
    (dir, vault, service, result)
}
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nDATA";

#[test]
fn desktop_worker_retries_transient_vault_contention_without_user_reimport() {
    use std::time::{Duration, Instant};
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let source = dir.path().join("official.zip");
    std::fs::write(&source, zip(files(Some(PNG)))).unwrap();
    let service = ImportService::new(&vault).unwrap();
    let job = service
        .submit(ImportRequest {
            request_id: "writer-contention".into(),
            paths: vec![source],
            scope: "personal".into(),
            format: "auto".into(),
        })
        .unwrap();
    let writer = recallcard::EventStreamWriter::open(&vault).unwrap();
    service.start_worker().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = service.status(&job.job_id, "personal").unwrap();
        assert_ne!(state.state, JobState::Failed);
        if state.state == JobState::Queued && state.phase == JobPhase::Committing {
            assert_eq!(state.progress.events_added, 0);
            assert!(state.error.is_none());
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(vault.events().unwrap().is_empty());
    assert_eq!(
        std::fs::read_dir(vault.root().join("objects"))
            .unwrap()
            .count(),
        0
    );
    drop(writer);
    loop {
        let state = service.status(&job.job_id, "personal").unwrap();
        assert_ne!(state.state, JobState::Failed);
        if state.state == JobState::Completed {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(vault.events().unwrap().len(), 3);
    assert_eq!(
        std::fs::read_dir(vault.root().join("objects"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn queued_contention_respects_pause_before_any_fact_commit() {
    use std::time::{Duration, Instant};
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let source = dir.path().join("official.zip");
    std::fs::write(&source, zip(files(Some(PNG)))).unwrap();
    let service = ImportService::new(&vault).unwrap();
    let job = service
        .submit(ImportRequest {
            request_id: "pause-contention".into(),
            paths: vec![source],
            scope: "personal".into(),
            format: "auto".into(),
        })
        .unwrap();
    let writer = recallcard::EventStreamWriter::open(&vault).unwrap();
    assert_eq!(
        service.run(&job.job_id, "personal").unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(
        service.status(&job.job_id, "personal").unwrap().state,
        JobState::Queued
    );
    service.pause(&job.job_id, "personal").unwrap();
    drop(writer);
    service.start_worker().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        service.status(&job.job_id, "personal").unwrap().state,
        JobState::Paused
    );
    assert!(vault.events().unwrap().is_empty());
    service.resume(&job.job_id, "personal").unwrap();
    service.start_worker().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while service.status(&job.job_id, "personal").unwrap().state != JobState::Completed {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(vault.events().unwrap().len(), 3);
}

#[test]
fn thin_archive_preserves_attachment_only_event_and_real_topology() {
    let (_dir, vault, service, result) = run(zip(files(None)), "thin");
    assert_eq!(result.state, JobState::Completed);
    let events = vault.events().unwrap();
    assert_eq!(events.len(), 3);
    let upload = events
        .iter()
        .find(|e| e.data.source.message_id == "upload")
        .unwrap();
    assert_eq!(upload.data.kind, "file");
    assert!(upload.data.content.is_empty());
    assert_eq!(
        upload.data.occurred_at.unwrap().timestamp_subsec_millis(),
        125
    );
    let f = &upload.data.metadata["source_assets"]["files"][0];
    assert_eq!(f["name"], "message-name.png");
    assert_eq!(f["archive_name_alias"], "archive-alias.png");
    assert_eq!(f["library_name"], "library-name.png");
    assert_eq!(f["payload_status"], "missing");
    assert_eq!(f["declared_media_type_conflict"], true);
    assert!(f.get("object_sha256").is_none());
    let answer = events
        .iter()
        .find(|e| e.data.source.message_id == "answer")
        .unwrap();
    assert!(answer.data.metadata.get("source_assets").is_none());
    assert_eq!(answer.data.metadata["chatgpt"]["parent_id"], "hidden");
    assert_eq!(
        answer.data.metadata["chatgpt"]["nearest_visible_parent_id"],
        "upload"
    );
    assert_eq!(answer.data.metadata["chatgpt"]["omitted_parent_nodes"], 1);
    assert_eq!(answer.data.metadata["chatgpt"]["on_current_path"], true);
    let alt = events
        .iter()
        .find(|e| e.data.source.message_id == "alternate")
        .unwrap();
    assert_eq!(alt.data.metadata["chatgpt"]["on_current_path"], false);
    let coverage = service.result(&result.job_id, "personal").unwrap().coverage;
    assert_eq!(coverage.file_references, 1);
    assert_eq!(coverage.attachment_payloads_missing, 1);
    assert_eq!(coverage.attachment_payloads_retained, 0);
    assert_eq!(
        std::fs::read_dir(vault.root().join("objects"))
            .unwrap()
            .count(),
        0
    );
}
#[test]
fn binary_is_content_addressed_sniffed_and_reimport_is_idempotent() {
    let (_dir, vault, service, result) = run(zip(files(Some(PNG))), "binary");
    assert_eq!(result.state, JobState::Completed);
    let hash = recallcard::hash(PNG);
    assert_eq!(
        std::fs::read(vault.root().join("objects").join(&hash)).unwrap(),
        PNG
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(vault.root().join("objects").join(&hash))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let upload = vault
        .events()
        .unwrap()
        .into_iter()
        .find(|e| e.data.source.message_id == "upload")
        .unwrap();
    let f = &upload.data.metadata["source_assets"]["files"][0];
    assert_eq!(f["object_sha256"], hash);
    assert_eq!(f["detected_media_type"], "image/png");
    assert_eq!(f["payload_status"], "retained");
    assert_eq!(f["content_indexed"], false);
    let again = service.run(&result.job_id, "personal").unwrap();
    assert_eq!(again.state, JobState::Completed);
    assert_eq!(vault.events().unwrap().len(), 3);
    assert_eq!(
        std::fs::read_dir(vault.root().join("objects"))
            .unwrap()
            .count(),
        1
    );
    assert_eq!(
        service
            .result(&result.job_id, "personal")
            .unwrap()
            .coverage
            .attachment_payloads_retained,
        1
    );
}
#[test]
fn readonly_reader_never_claims_payload_was_saved() {
    let mut events = Vec::new();
    let report = read_source(
        &mut Cursor::new(zip(files(Some(PNG)))),
        "auto",
        "personal",
        ImportLimits::default(),
        |c| {
            events.extend(c.events);
            Ok(())
        },
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(report.attachment_payloads_retained, 0);
    assert_eq!(
        events
            .iter()
            .find(|e| e.source.message_id == "upload")
            .unwrap()
            .metadata["source_assets"]["files"][0]["payload_status"],
        "available_not_retained"
    );
}
#[test]
fn size_mismatch_and_corrupt_crc_never_commit_events_or_objects() {
    let invalid_size = zip(files(Some(b"short")));
    let mut corrupt = zip(files(Some(PNG)));
    let offset = corrupt.windows(PNG.len()).position(|p| p == PNG).unwrap();
    corrupt[offset + 9] ^= 1;
    for (i, data) in [invalid_size, corrupt].into_iter().enumerate() {
        let (_d, v, _s, r) = run(data, &format!("invalid{i}"));
        assert_eq!(r.state, JobState::Failed);
        assert!(v.events().unwrap().is_empty());
        assert_eq!(
            std::fs::read_dir(v.root().join("objects")).unwrap().count(),
            0
        );
    }
}
#[test]
fn display_name_never_resolves_wrong_binary() {
    let mut input = files(None);
    input.push(("export/message-name.png".into(), PNG.to_vec()));
    let (_d, v, _s, r) = run(zip(input), "wrong-name");
    assert_eq!(r.state, JobState::Completed);
    assert_eq!(
        std::fs::read_dir(v.root().join("objects")).unwrap().count(),
        0
    );
}
#[test]
fn scripts_are_opaque_non_executable_bytes() {
    let bytes = b"#!/bin/sh\nxx";
    assert_eq!(bytes.len(), 12);
    let (_d, v, _s, r) = run(zip(files(Some(bytes))), "script");
    assert_eq!(r.state, JobState::Completed);
    let event = v
        .events()
        .unwrap()
        .into_iter()
        .find(|e| e.data.source.message_id == "upload")
        .unwrap();
    assert_eq!(
        event.data.metadata["source_assets"]["files"][0]["detected_media_type"],
        "application/octet-stream"
    );
}

#[test]
fn identity_path_works_without_optional_library_catalog() {
    let input = files(Some(PNG))
        .into_iter()
        .filter(|(name, _)| name.ends_with("conversations.json") || name.ends_with(".dat"))
        .map(|(name, data)| (name.strip_prefix("export/").unwrap().into(), data))
        .collect();
    let (_d, v, _s, r) = run(zip(input), "minimal-export");
    assert_eq!(r.state, JobState::Completed);
    assert_eq!(
        std::fs::read_dir(v.root().join("objects")).unwrap().count(),
        1
    );
}
#[test]
fn unsupported_shard_and_duplicate_library_identity_fail_closed() {
    for kind in ["shard", "duplicate"] {
        let input = files(Some(PNG))
            .into_iter()
            .map(|(name, mut data)| {
                if kind == "shard" && name.ends_with("export_manifest.json") {
                    let mut v: Value = serde_json::from_slice(&data).unwrap();
                    v["logical_files"]["file_SYNTHETIC.dat"]["sharded"] = json!(true);
                    data = serde_json::to_vec(&v).unwrap();
                }
                if kind == "duplicate" && name.ends_with("library_files.json") {
                    let mut v: Value = serde_json::from_slice(&data).unwrap();
                    let row = v[0].clone();
                    v.as_array_mut().unwrap().push(row);
                    data = serde_json::to_vec(&v).unwrap();
                }
                (name, data)
            })
            .collect();
        let (_d, v, _s, r) = run(zip(input), kind);
        assert_eq!(r.state, JobState::Failed);
        assert!(v.events().unwrap().is_empty());
        assert_eq!(
            std::fs::read_dir(v.root().join("objects")).unwrap().count(),
            0
        );
    }
}
#[test]
fn repeated_asset_has_one_payload_and_multiple_message_edges() {
    let input = files(Some(PNG))
        .into_iter()
        .map(|(name, mut data)| {
            if name.ends_with("conversations.json") {
                let mut v: Value = serde_json::from_slice(&data).unwrap();
                v[0]["mapping"]["alternate"]["message"]["metadata"] =
                    v[0]["mapping"]["upload"]["message"]["metadata"].clone();
                data = serde_json::to_vec(&v).unwrap();
            }
            (name, data)
        })
        .collect();
    let (_d, v, s, r) = run(zip(input), "reuse");
    assert_eq!(r.state, JobState::Completed);
    assert_eq!(
        std::fs::read_dir(v.root().join("objects")).unwrap().count(),
        1
    );
    assert_eq!(
        s.result(&r.job_id, "personal")
            .unwrap()
            .coverage
            .attachment_payloads_retained,
        2
    );
}

#[test]
fn attachment_names_are_searchable_without_inventing_body_or_crossing_scope() {
    use recallcard::{
        context::{Context, SearchArgs},
        policy::Access,
    };
    let (_d, v, _s, r) = run(zip(files(None)), "name-search");
    assert_eq!(r.state, JobState::Completed);
    let args = || {
        serde_json::from_value::<SearchArgs>(
            json!({"query":"archive-alias.png","target":"events","budget_tokens":12000}),
        )
        .unwrap()
    };
    let allowed = Context::new(&v, Access::new(vec!["personal".into()]).unwrap());
    let found = allowed.search(args()).unwrap();
    assert_eq!(found["results"].as_array().unwrap().len(), 1);
    assert_eq!(found["results"][0]["text"], "");
    assert_eq!(found["results"][0]["text_range"]["total_bytes"], 0);
    let forbidden = Context::new(&v, Access::new(vec!["project:other".into()]).unwrap());
    assert!(forbidden.search(args()).unwrap()["results"]
        .as_array()
        .unwrap()
        .is_empty());
    let id = v
        .events()
        .unwrap()
        .into_iter()
        .find(|e| e.data.source.message_id == "upload")
        .unwrap()
        .id;
    v.suppress(&id, "合成移除".into()).unwrap();
    assert!(allowed.search(args()).unwrap()["results"]
        .as_array()
        .unwrap()
        .is_empty());
}
#[test]
fn omitted_source_kinds_are_reported_without_claiming_message_coverage() {
    let mut input = files(None);
    input.push((
        "export/group_chats.json".into(),
        br#"{"chats":[]}"#.to_vec(),
    ));
    input.push((
        "export/Pages/synthetic/current.json".into(),
        br#"{"page_id":"synthetic"}"#.to_vec(),
    ));
    let (_d, _v, s, r) = run(zip(input), "unsupported-sources");
    assert_eq!(r.state, JobState::Completed);
    let report = s.result(&r.job_id, "personal").unwrap().coverage;
    assert_eq!(
        report.unsupported_archive_sources,
        vec!["group_chats", "pages"]
    );
}

#[test]
fn different_export_roots_never_share_attachment_identity() {
    let input = files(Some(PNG))
        .into_iter()
        .map(|(name, bytes)| {
            let root = if name.ends_with("conversations.json") {
                "B/"
            } else {
                "A/"
            };
            (name.replacen("export/", root, 1), bytes)
        })
        .collect();
    let (_d, v, _s, r) = run(zip(input), "cross-root");
    assert_eq!(r.state, JobState::Completed);
    assert!(v
        .events()
        .unwrap()
        .iter()
        .flat_map(|event| event.data.metadata["source_assets"]["files"]
            .as_array()
            .into_iter()
            .flatten())
        .all(|file| file["payload_status"] == "missing"));
    assert_eq!(
        std::fs::read_dir(v.root().join("objects")).unwrap().count(),
        0
    );
}
#[test]
fn enriched_late_event_size_limit_is_checked_before_any_fact_commit() {
    let mut c = conversation();
    let mut parent = "answer".to_owned();
    for i in 0..66 {
        let id = format!("preceding-{i}");
        c["mapping"][&id] = json!({"id":id,"parent":parent,"message":{"id":id,"author":{"role":"user"},"content":{"content_type":"text","parts":["synthetic preceding message"]}}});
        parent = id;
    }
    let mut attachments = Vec::new();
    let mut library = Vec::new();
    let mut aliases = serde_json::Map::new();
    for i in 0..700 {
        let id = format!("file_LARGE_{i}");
        attachments.push(json!({"id":id,"name":"small"}));
        library.push(json!({"file_id":id,"file_name":"n".repeat(4096)}));
        aliases.insert(format!("{id}.dat"), json!("a".repeat(4096)));
    }
    c["mapping"]["large"] = json!({"id":"large","parent":parent,"message":{"id":"large","author":{"role":"user"},"content":{"content_type":"text","parts":[]},"metadata":{"attachments":attachments}}});
    c["current_node"] = json!("large");
    let data = zip(vec![
        (
            "conversations.json".into(),
            serde_json::to_vec(&vec![c]).unwrap(),
        ),
        (
            "library_files.json".into(),
            serde_json::to_vec(&library).unwrap(),
        ),
        (
            "conversation_asset_file_names.json".into(),
            serde_json::to_vec(&aliases).unwrap(),
        ),
    ]);
    let (_d, v, _s, r) = run(data, "oversized-enrichment");
    assert_eq!(r.state, JobState::Failed);
    assert_eq!(r.error.unwrap().code, ErrorCode::ResourceLimit);
    assert!(v.events().unwrap().is_empty());
    assert_eq!(
        std::fs::read_dir(v.root().join("objects")).unwrap().count(),
        0
    );
}

#[test]
fn missing_catalog_never_allows_cross_root_payload_sharing() {
    let c = serde_json::to_vec(&vec![conversation()]).unwrap();
    for names in [
        ["A/conversations.json", "B/conversations.json"],
        ["A/arbitrary.json", "B/arbitrary.json"],
    ] {
        let input = vec![
            (names[0].into(), c.clone()),
            (names[1].into(), c.clone()),
            ("file_SYNTHETIC.dat".into(), PNG.to_vec()),
        ];
        let (_d, v, _s, r) = run(zip(input), "ambiguous-root");
        assert_eq!(r.state, JobState::Failed);
        assert!(v.events().unwrap().is_empty());
        assert_eq!(
            std::fs::read_dir(v.root().join("objects")).unwrap().count(),
            0
        );
    }
}

#[test]
fn nested_non_target_page_manifest_cannot_replace_chat_catalog() {
    let mut input = files(Some(PNG));
    input.push((
        "export/Pages/synthetic/export_manifest.json".into(),
        br#"{"version":1,"page_id":"synthetic"}"#.to_vec(),
    ));
    let (_d, v, s, r) = run(zip(input), "page-manifest");
    assert_eq!(r.state, JobState::Completed);
    assert_eq!(
        std::fs::read_dir(v.root().join("objects")).unwrap().count(),
        1
    );
    assert_eq!(
        s.result(&r.job_id, "personal")
            .unwrap()
            .coverage
            .unsupported_archive_sources,
        vec!["pages"]
    );
}

#[test]
fn unicode_path_alias_cannot_contradict_audited_asset_name() {
    // 原创合成 ZIP：有效 Unicode Path extra CRC 指向与原始成员名不同的文件。
    // zip writer 会主动拒绝构造这种矛盾，故固定原始编码作为解析器回归输入。
    let hex = concat!(
        "504b0304140000000000f9794d5de5756fd0840100008401000012000000636f6e766572736174696f6e732e6a736f6e5b7b226964223a2022726f6f",
        "742d62222c20227469746c65223a202273796e74686574696320726576696577222c202263757272656e745f6e6f6465223a20226d222c20226d6170",
        "70696e67223a207b22726f6f74223a207b22706172656e74223a206e756c6c2c20226d657373616765223a206e756c6c7d2c20226d223a207b227061",
        "72656e74223a2022726f6f74222c20226d657373616765223a207b226964223a20226d222c2022617574686f72223a207b22726f6c65223a20227573",
        "6572227d2c20226372656174655f74696d65223a20313737303030303030302e3132352c2022636f6e74656e74223a207b22636f6e74656e745f7479",
        "7065223a202274657874222c20227061727473223a205b2273796e74686574696320726576696577206f6e6c79225d7d2c20226d6574616461746122",
        "3a207b226174746163686d656e7473223a205b7b226964223a202266696c655f58222c20226e616d65223a2022422d66696c652e747874222c202273",
        "697a65223a2031327d5d7d7d7d7d7d5d504b03041400000000000000210064f067bd0c0000000c0000000a00170066696c655f582e64617475701300",
        "0112977f4066696c655f4f544845522e646174524f4f545f415f4259544553504b01021403140000000000f9794d5de5756fd0840100008401000012",
        "0000000000000000000000800100000000636f6e766572736174696f6e732e6a736f6e504b010214031400000000000000210064f067bd0c0000000c",
        "0000000a00170000000000000000008001b401000066696c655f582e646174757013000112977f4066696c655f4f544845522e646174504b05060000",
        "0000020002008f000000ff0100000000",
    );
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    let (_d, v, _s, r) = run(bytes, "unicode-conflict");
    assert_eq!(r.state, JobState::Failed);
    assert!(v.events().unwrap().is_empty());
    assert_eq!(
        std::fs::read_dir(v.root().join("objects")).unwrap().count(),
        0
    );
}
