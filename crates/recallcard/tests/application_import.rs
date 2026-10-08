//! 完整官方导入应用合同：只用结构等价的合成资料，不读取私人归档。
use recallcard::{
    application::{
        import_reader::{read_source, ImportedConversation},
        *,
    },
    EventInput, EventStreamWriter, Vault,
};
use serde_json::{json, Value};
use std::{
    io::{Cursor, Write},
    path::PathBuf,
};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};
fn deepseek(id: &str, body: &str) -> Value {
    json!({"id":id,"title":format!("合成 {id}"),"inserted_at":"2026-10-08T00:00:00Z","updated_at":"2026-10-08T00:00:01Z","mapping":{
     "root":{"id":"root","parent":null,"children":["u"],"message":null},
     "u":{"id":"u","parent":"root","children":["a","b"],"message":{"inserted_at":"2026-10-08T00:00:00Z","fragments":[{"type":"REQUEST","content":body}]}},
     "a":{"id":"a","parent":"u","children":[],"message":{"inserted_at":"2026-10-08T00:00:01Z","fragments":[{"type":"THINK","content":"不保存的合成内部内容"},{"type":"RESPONSE","content":"回答 A"}]}},
     "b":{"id":"b","parent":"u","children":[],"message":{"inserted_at":"2026-10-08T00:00:01Z","fragments":[{"type":"RESPONSE","content":"回答 B"}]}}
    }})
}
fn archive(name: &str, bytes: &[u8]) -> Vec<u8> {
    let mut z = ZipWriter::new(Cursor::new(Vec::new()));
    z.start_file(
        name,
        SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
    )
    .unwrap();
    z.write_all(bytes).unwrap();
    z.finish().unwrap().into_inner()
}
fn parse(
    bytes: Vec<u8>,
    limits: ImportLimits,
) -> (
    AppResult<import_reader::ReadReport>,
    Vec<ImportedConversation>,
) {
    let mut out = Vec::new();
    let result = read_source(
        &mut Cursor::new(bytes),
        "auto",
        "project:test",
        limits,
        |c| {
            out.push(c);
            Ok(())
        },
        |_| Ok(()),
    );
    (result, out)
}
fn setup() -> (tempfile::TempDir, Vault, ImportService) {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::init(&dir.path().join("vault")).unwrap();
    let s = ImportService::new(&v).unwrap();
    (dir, v, s)
}
fn request(id: &str, path: PathBuf) -> ImportRequest {
    ImportRequest {
        request_id: id.into(),
        paths: vec![path],
        scope: "project:test".into(),
        format: "auto".into(),
    }
}
#[test]
fn whole_json_above_thirty_mib_streams_as_individual_conversations() {
    let body = "大包中的合成正文".repeat(2400);
    let mut json = Vec::new();
    json.push(b'[');
    for n in 0..540 {
        if n > 0 {
            json.push(b',');
        }
        serde_json::to_writer(&mut json, &deepseek(&format!("c{n}"), &body)).unwrap();
    }
    json.push(b']');
    assert!(json.len() > 30 * 1024 * 1024);
    let expected = json.len();
    let zipped = archive("conversations.json", &json);
    drop(json);
    let mut conversations = 0;
    let mut seen = 0;
    let mut progress = 0;
    let report = read_source(
        &mut Cursor::new(zipped),
        "auto",
        "project:test",
        ImportLimits::default(),
        |c| {
            assert_eq!(c.events.len(), 3);
            assert_eq!(c.deepseek_coverage.branch_points, 1);
            assert_eq!(
                c.events[0].source.conversation_id,
                format!("c{conversations}")
            );
            assert_eq!(c.title, Some(format!("合成 c{conversations}")));
            assert_eq!(
                c.events[1].metadata["deepseek"]["nearest_visible_parent_id"],
                "u"
            );
            assert_eq!(
                c.events[2].metadata["deepseek"]["nearest_visible_parent_id"],
                "u"
            );
            assert!(c.events.iter().all(|e| e.scope == "project:test"));
            conversations += 1;
            seen += c.events.len();
            Ok(())
        },
        |_| {
            progress += 1;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(report.expanded_bytes, expected as u64);
    assert_eq!(report.conversations, 540);
    assert_eq!(seen, 1620);
    assert_eq!(report.hidden_fragments, 540);
    assert!(progress > 50);
}
#[test]
fn file_search_and_empty_tool_payloads_have_truthful_structured_provenance() {
    let mut c = deepseek("assets", "正文");
    c["mapping"]["u"]["message"]["fragments"].as_array_mut().unwrap().extend([
      json!({"type":"FILE","files":[{"file_id":"file-x","file_name":"合成.pdf","file_size":125}]}),
      json!({"type":"TOOL_SEARCH","results":[{"url":"https://example.com/source","title":""}]}),json!({"type":"TOOL_OPEN"})]);
    c["mapping"]["a"]["children"] = json!(["f"]);
    c["mapping"]["f"] = json!({"id":"f","parent":"a","children":[],"message":{"inserted_at":null,"fragments":[{"type":"FILE","files":[{"file_id":"file-y","file_name":"only.txt","file_size":0}]}]}});
    let (result, rows) = parse(serde_json::to_vec(&c).unwrap(), ImportLimits::default());
    let result = result.unwrap();
    assert_eq!(result.events, 4);
    assert_eq!(result.file_references, 2);
    assert_eq!(result.citations, 1);
    assert_eq!(result.trace_placeholders, 1);
    assert_eq!(
        rows[0].events[0].metadata["source_assets"]["citations"][0]["title"],
        ""
    );
    assert_eq!(result.unsupported_fragments, 0);
    let u = &rows[0].events[0];
    assert_eq!(u.content, "正文");
    assert_eq!(
        u.metadata["source_assets"]["files"][0]["payload_status"],
        "not_in_export"
    );
    let file = rows[0]
        .events
        .iter()
        .find(|e| e.source.message_id == "f")
        .unwrap();
    assert_eq!(file.kind, "file");
    assert!(file.content.is_empty());
    assert_eq!(file.scope, "project:test");
    assert_eq!(file.metadata["deepseek"]["nearest_visible_parent_id"], "a");
}
#[test]
fn json_framing_rejects_duplicates_trailing_values_and_bad_delimiters() {
    for bytes in [
        b"[{\"id\":1,\"id\":2}]".to_vec(),
        b"[{},]".to_vec(),
        b"[{} {}]".to_vec(),
        b"{}{}".to_vec(),
        b"[\"unterminated]".to_vec(),
        b"[{}]\x0b".to_vec(),
    ] {
        assert_eq!(
            parse(bytes, ImportLimits::default()).0.unwrap_err().code,
            ErrorCode::InvalidJson
        );
    }
    for bytes in [
        b"[]".to_vec(),
        b"[{},true,false,null,-1,\"escaped \\\" text\"]".to_vec(),
    ] {
        assert!(parse(bytes, ImportLimits::default()).0.is_ok());
    }
}
#[test]
fn per_conversation_limit_does_not_limit_the_whole_array() {
    let one = deepseek("bounded", "正文");
    let n = serde_json::to_vec(&one).unwrap().len();
    let limits = ImportLimits {
        conversation_bytes: n + 10,
        ..Default::default()
    };
    assert_eq!(
        parse(serde_json::to_vec(&vec![one.clone(); 20]).unwrap(), limits)
            .0
            .unwrap()
            .conversations,
        20
    );
    let limits = ImportLimits {
        conversation_bytes: n - 1,
        ..limits
    };
    assert_eq!(
        parse(serde_json::to_vec(&one).unwrap(), limits)
            .0
            .unwrap_err()
            .code,
        ErrorCode::ResourceLimit
    );
}
#[test]
fn archive_path_limits_and_crc_fail_before_fact_commit() {
    let (dir, vault, service) = setup();
    let raw = serde_json::to_vec(&deepseek("zip", "正文")).unwrap();
    for (i, data) in [
        archive("../evil.json", &raw),
        archive("conversations.json", &raw)[..40].to_vec(),
    ]
    .into_iter()
    .enumerate()
    {
        let path = dir.path().join(format!("bad-{i}.zip"));
        std::fs::write(&path, data).unwrap();
        let job = service.submit(request(&format!("bad-{i}"), path)).unwrap();
        let result = service.run(&job.job_id, "project:test").unwrap();
        assert_eq!(result.state, JobState::Failed);
        assert_eq!(result.error.unwrap().code, ErrorCode::InvalidArchive);
        assert!(vault.events().unwrap().is_empty());
    }
    let limits = ImportLimits {
        expanded_bytes: raw.len() as u64 - 1,
        ..Default::default()
    };
    assert_eq!(
        parse(archive("conversations.json", &raw), limits)
            .0
            .unwrap_err()
            .code,
        ErrorCode::ResourceLimit
    );
}
#[test]
fn both_entries_use_same_job_and_repeat_import_reports_duplicates() {
    let (dir, vault, service) = setup();
    let path = dir.path().join("official.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&vec![deepseek("one", "正文"), deepseek("two", "第二段")]).unwrap(),
    )
    .unwrap();
    let job = service.submit(request("first", path.clone())).unwrap();
    assert_eq!(
        service
            .submit(request("first", path.clone()))
            .unwrap()
            .job_id,
        job.job_id
    );
    let result = service.run(&job.job_id, "project:test").unwrap();
    assert_eq!(result.state, JobState::Completed);
    assert_eq!(result.progress.events_added, 6);
    assert_eq!(result.progress.conversations, 2);
    let reopened = ImportService::new(&Vault::open(vault.root()).unwrap()).unwrap();
    assert_eq!(
        reopened.status(&job.job_id, "project:test").unwrap().state,
        JobState::Completed
    );
    assert!(reopened.list("personal").unwrap().is_empty());
    assert_eq!(
        reopened.status(&job.job_id, "personal").unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    let second = reopened.submit(request("second", path)).unwrap();
    let result = reopened.run(&second.job_id, "project:test").unwrap();
    assert_eq!(result.progress.events_added, 0);
    assert_eq!(result.progress.events_duplicates, 6);
    assert_eq!(vault.events().unwrap().len(), 6);
}
#[test]
fn source_change_after_queue_requires_new_input_and_never_writes() {
    let (dir, vault, service) = setup();
    let path = dir.path().join("source.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&deepseek("first", "原文")).unwrap(),
    )
    .unwrap();
    let job = service.submit(request("changed", path.clone())).unwrap();
    std::fs::write(
        path,
        serde_json::to_vec(&deepseek("second", "换过的文档")).unwrap(),
    )
    .unwrap();
    let status = service.run(&job.job_id, "project:test").unwrap();
    assert_eq!(status.state, JobState::NeedsInput);
    assert_eq!(status.error.unwrap().code, ErrorCode::SourceChanged);
    assert!(vault.events().unwrap().is_empty());
}
#[test]
fn queued_pause_resume_and_invalid_job_path_are_safe() {
    let (dir, vault, service) = setup();
    let path = dir.path().join("source.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&deepseek("resume", "正文")).unwrap(),
    )
    .unwrap();
    let job = service.submit(request("resume", path)).unwrap();
    assert_eq!(
        service.pause(&job.job_id, "project:test").unwrap().state,
        JobState::Paused
    );
    assert_eq!(
        service.run(&job.job_id, "project:test").unwrap().state,
        JobState::Paused
    );
    assert!(vault.events().unwrap().is_empty());
    service.resume(&job.job_id, "project:test").unwrap();
    assert_eq!(
        service
            .run(&job.job_id, "project:test")
            .unwrap()
            .progress
            .events_added,
        3
    );
    assert_eq!(
        service
            .resume("../../outside", "project:test")
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(!vault.state_dir().unwrap().join("outside").exists());
}
fn input(id: &str, content: &str) -> EventInput {
    serde_json::from_value(json!({"scope":"personal","role":"user","origin":"user_input","content":content,"source":{"platform":"synthetic","conversation_id":"writer","message_id":id}})).unwrap()
}
#[test]
fn writer_indexes_once_and_receipts_survive_restart_without_false_duplicates() {
    let (_dir, vault, _) = setup();
    vault
        .capture_batch((0..30).map(|n| input(&n.to_string(), "旧版本")).collect())
        .unwrap();
    let mut writer = EventStreamWriter::open(&vault).unwrap();
    assert_eq!(writer.indexed_events(), 30);
    let first = writer
        .commit_chunk(
            "batch-0",
            vec![
                input("0", "旧版本"),
                input("0", "新版本"),
                input("new", "新增"),
            ],
        )
        .unwrap();
    assert_eq!((first.events_added, first.events_duplicates), (2, 1));
    for n in 0..25 {
        writer
            .commit_chunk(
                &format!("chunk-{n}"),
                vec![input(&format!("fresh-{n}"), "内容")],
            )
            .unwrap();
    }
    assert_eq!(writer.indexed_events(), 30);
    drop(writer);
    let mut writer = EventStreamWriter::open(&vault).unwrap();
    assert_eq!(
        writer
            .commit_chunk(
                "batch-0",
                vec![
                    input("0", "旧版本"),
                    input("0", "新版本"),
                    input("new", "新增")
                ]
            )
            .unwrap(),
        first
    );
    assert!(writer
        .commit_chunk("batch-0", vec![input("changed", "不同内容")])
        .is_err());
    drop(writer);
    let events = vault.events().unwrap();
    assert_eq!(events.len(), 57);
    assert!(events
        .iter()
        .any(|e| e.data.content == "新版本" && e.data.revision_of.is_some()));
}
#[test]
fn parsing_checkpoint_and_commit_pause_resume_keep_counts_and_sources() {
    use std::time::{Duration, Instant};
    let (dir, vault, service) = setup();
    let path = dir.path().join("resumable.json");
    let payload = vec![b'x'; 100_000];
    let body = std::str::from_utf8(&payload).unwrap();
    let source: Vec<_> = (0..160)
        .map(|n| deepseek(&format!("resume-{n}"), body))
        .collect();
    std::fs::write(&path, serde_json::to_vec(&source).unwrap()).unwrap();
    drop(source);
    let job = service.submit(request("durable-resume", path)).unwrap();
    let runner = service.clone();
    let id = job.job_id.clone();
    let thread = std::thread::spawn(move || runner.run(&id, "project:test").unwrap());
    let start = Instant::now();
    loop {
        let status = service.status(&job.job_id, "project:test").unwrap();
        if status.phase == JobPhase::Parsing && status.progress.events_staged > 0 {
            service.pause(&job.job_id, "project:test").unwrap();
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "未观察到解析检查点"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let paused = thread.join().unwrap();
    assert_eq!(paused.state, JobState::Paused);
    assert!(paused.progress.events_staged > 0);
    assert!(vault.events().unwrap().is_empty());
    let reopened = ImportService::new(&Vault::open(vault.root()).unwrap()).unwrap();
    reopened.resume(&job.job_id, "project:test").unwrap();
    let runner = reopened.clone();
    let id = job.job_id.clone();
    let thread = std::thread::spawn(move || runner.run(&id, "project:test").unwrap());
    let start = Instant::now();
    loop {
        let status = reopened.status(&job.job_id, "project:test").unwrap();
        if status.phase == JobPhase::Committing && status.progress.events_processed > 0 {
            reopened.pause(&job.job_id, "project:test").unwrap();
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "未观察到提交检查点"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let paused = thread.join().unwrap();
    assert_eq!(paused.state, JobState::Paused);
    assert!(paused.progress.events_added > 0);
    assert_eq!(
        vault.events().unwrap().len() as u64,
        paused.progress.events_added
    );
    // 恢复时暂时锁忙不得丢掉已确认提交数量。
    let writer = EventStreamWriter::open(&vault).unwrap();
    reopened.resume(&job.job_id, "project:test").unwrap();
    let blocked = reopened.run(&job.job_id, "project:test").unwrap();
    assert_eq!(blocked.error.as_ref().unwrap().code, ErrorCode::Conflict);
    assert_eq!(blocked.progress.events_added, paused.progress.events_added);
    assert_eq!(
        blocked.error.unwrap().committed_events,
        paused.progress.events_added
    );
    drop(writer);
    reopened.resume(&job.job_id, "project:test").unwrap();
    let complete = reopened.run(&job.job_id, "project:test").unwrap();
    assert_eq!(complete.state, JobState::Completed);
    assert_eq!(complete.progress.events_added, 480);
    assert_eq!(complete.progress.events_duplicates, 0);
    assert_eq!(vault.events().unwrap().len(), 480);
}

#[test]
fn same_export_in_two_scopes_never_revises_or_hides_the_other_copy() {
    use recallcard::{context::Context, policy::Access};
    let (dir, vault, service) = setup();
    let path = dir.path().join("scoped.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&deepseek("scope", "UniqueScopeEvidence")).unwrap(),
    )
    .unwrap();
    for scope in ["personal", "project:test"] {
        let mut req = request(&format!("scope-{}", scope.replace(':', "-")), path.clone());
        req.scope = scope.into();
        let job = service.submit(req).unwrap();
        assert_eq!(
            service.run(&job.job_id, scope).unwrap().state,
            JobState::Completed
        );
    }
    assert!(vault
        .events()
        .unwrap()
        .iter()
        .all(|e| e.data.revision_of.is_none()));
    for scope in ["personal", "project:test"] {
        let ctx = Context::new(&vault, Access::new(vec![scope.into()]).unwrap());
        let result = ctx
            .search(serde_json::from_value(json!({"query":"UniqueScopeEvidence"})).unwrap())
            .unwrap();
        assert_eq!(result["results"].as_array().unwrap().len(), 1);
    }
}

#[test]
fn chatgpt_other_branch_is_preserved_with_selected_path_identity() {
    let message = |id: &str, body: &str| json!({"id":id,"author":{"role":"assistant"},"create_time":null,"content":{"content_type":"text","parts":[body]}});
    let c = json!({"id":"branch","current_node":"a","mapping":{
        "u":{"id":"u","parent":null,"children":["a","b"],"message":message("u", "root")},
        "a":{"id":"a","parent":"u","children":[],"message":message("a", "ChosenAnswer")},
        "b":{"id":"b","parent":"u","children":[],"message":message("b", "AlternativeAnswer")}
    }});
    let (report, rows) = parse(serde_json::to_vec(&c).unwrap(), ImportLimits::default());
    assert_eq!(report.unwrap().events, 3);
    assert_eq!(
        rows[0]
            .events
            .iter()
            .find(|e| e.content == "AlternativeAnswer")
            .unwrap()
            .metadata["chatgpt"]["on_current_path"],
        false
    );
    assert_eq!(
        rows[0]
            .events
            .iter()
            .find(|e| e.content == "ChosenAnswer")
            .unwrap()
            .metadata["chatgpt"]["on_current_path"],
        true
    );
    assert_eq!(rows[0].chatgpt_coverage.other_branch_messages_imported, 1);
}

#[test]
fn no_payload_search_is_a_trace_and_type_errors_are_not_resource_limits() {
    let mut c = deepseek("trace", "visible");
    c["mapping"]["u"]["message"]["fragments"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"TOOL_SEARCH"}));
    let (report, rows) = parse(serde_json::to_vec(&c).unwrap(), ImportLimits::default());
    assert_eq!(report.unwrap().trace_placeholders, 1);
    assert_eq!(
        rows[0].events[0].metadata["source_assets"]["tool_trace"][0]["payload_status"],
        "not_in_export"
    );
    c["title"] = json!(1);
    assert_eq!(
        parse(serde_json::to_vec(&c).unwrap(), ImportLimits::default())
            .0
            .unwrap_err()
            .code,
        ErrorCode::InvalidConversation
    );
    c["title"] = json!("valid");
    c["mapping"]["u"]["message"]["fragments"][0]["content"] =
        json!("x".repeat(2 * 1024 * 1024 + 1));
    let error = parse(serde_json::to_vec(&c).unwrap(), ImportLimits::default())
        .0
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ResourceLimit);
    assert!(error.message.contains("2 MiB"));
}

#[test]
fn repeated_long_titles_hit_normalized_budget_before_staging_or_commit() {
    let mut mapping = serde_json::Map::new();
    for i in 0..13215 {
        let id = format!("m{i:05}");
        mapping.insert(id.clone(), json!({"id":id,"parent":null,"children":[],"message":{"fragments":[{"type":"REQUEST","content":"x"}]}}));
    }
    let raw = serde_json::to_vec(&json!({"id":"bounded","title":"A".repeat(4096),"inserted_at":null,"updated_at":null,"mapping":mapping})).unwrap();
    assert!(raw.len() < 3 * 1024 * 1024);
    let (result, rows) = parse(raw, ImportLimits::default());
    assert_eq!(result.unwrap_err().code, ErrorCode::ResourceLimit);
    assert!(rows.is_empty());
}

#[test]
fn local_zip_crc_mismatch_is_rejected_before_staging() {
    let raw = serde_json::to_vec(&deepseek("crc", "content")).unwrap();
    let mut bytes = archive("conversations.json", &raw);
    assert_eq!(bytes[6] & 8, 0);
    bytes[14] ^= 1;
    let (result, rows) = parse(bytes, ImportLimits::default());
    assert_eq!(result.unwrap_err().code, ErrorCode::InvalidArchive);
    assert!(rows.is_empty());
}
