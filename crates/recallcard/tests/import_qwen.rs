//! 原创合成 Qwen 官方形状；不含真实聊天、身份、附件或密钥。
use recallcard::{
    application::{
        import_reader::{read_source, ImportedConversation},
        ErrorCode, ImportLimits, ImportRequest, ImportService, JobState,
    },
    desktop::DesktopSession,
    import_bundle::{detect_import_format, parse_import_bytes},
    Vault,
};
use serde_json::{json, Value};
use std::io::Cursor;

fn sync_projection(c: &mut Value) {
    let mut ids = Vec::new();
    let mut current = c["chat"]["history"]["currentId"]
        .as_str()
        .map(str::to_owned);
    while let Some(id) = current {
        current = c["chat"]["history"]["messages"][&id]["parentId"]
            .as_str()
            .map(str::to_owned);
        ids.push(id);
    }
    ids.reverse();
    c["chat"]["messages"] = json!(ids
        .iter()
        .map(|id| c["chat"]["history"]["messages"][id].clone())
        .collect::<Vec<_>>());
}
fn conversation(id: &str) -> Value {
    let mut c = json!({"id":id,"user_id":"synthetic-account","title":"合成 Qwen 分支","created_at":1767225600,"updated_at":1767225602,"currentId":"a",
      "chat":{"history":{"currentId":"a","messages":{
        "u":{"id":"u","role":"user","content":"只选本地方案，暂不联网。","parentId":null,"childrenIds":["b","a"],"timestamp":1767225600,"files":[{"id":"file-demo","name":"合成.png","size":12,"type":"image","file_type":"image/png","file_class":"image","url":"https://example.invalid/private?token=DO_NOT_COPY_SYNTHETIC","file":{"raw":"NOT_A_REAL_ATTACHMENT"}}]},
        "a":{"id":"a","role":"assistant","content":"","reasoning_content":"PRIVATE_REASONING_CANARY","parentId":"u","childrenIds":[],"timestamp":1767225602,"content_list":[
          {"phase":"thinking_summary","role":"assistant","content":"THINKING_CANARY"},
          {"phase":"answer","role":"assistant","content":"合成答复第一段。"},
          {"phase":"web_search","role":"function","content":"TOOL_CANARY"},
          {"phase":"answer","role":"assistant","content":"第二段保留限制。"},
          {"phase":"answer","role":"function","content":"FUNCTION_ANSWER_CANARY"}]},
        "b":{"id":"b","role":"assistant","content":"","parentId":"u","childrenIds":[],"timestamp":1767225601,"content_list":[{"phase":"answer","role":"assistant","content":"另一个未选分支。"}]}
      }},"messages":[]}});
    sync_projection(&mut c);
    c
}
fn wrapper(values: Vec<Value>) -> Value {
    json!({"success":true,"request_id":"synthetic-request","data":values})
}
fn read(
    value: &Value,
    limits: ImportLimits,
) -> (
    recallcard::application::AppResult<recallcard::application::import_reader::ReadReport>,
    Vec<ImportedConversation>,
) {
    let mut out = Vec::new();
    let r = read_source(
        &mut Cursor::new(value.to_string().into_bytes()),
        "auto",
        "personal",
        limits,
        |c| {
            out.push(c);
            Ok(())
        },
        |_| Ok(()),
    );
    (r, out)
}
#[test]
fn history_once_keeps_siblings_roles_time_answer_order_and_safe_attachment_metadata() {
    let value = wrapper(vec![conversation("one")]);
    assert_eq!(
        detect_import_format(&value.to_string()).unwrap(),
        "qwen-export"
    );
    let (report, out) = read(&value, ImportLimits::default());
    let r = report.unwrap();
    let c = &out[0];
    assert_eq!((r.conversations, r.events, r.file_references), (1, 3, 1));
    assert_eq!(
        (
            r.qwen.nodes_seen,
            r.qwen.linear_duplicates_ignored,
            r.qwen.off_current_path_imported
        ),
        (3, 2, 1)
    );
    assert_eq!(r.qwen.hidden_fragments_skipped, 2);
    assert_eq!(r.qwen.unsupported_fragments_skipped, 2);
    let a = c
        .events
        .iter()
        .find(|e| e.source.message_id == "a")
        .unwrap();
    assert_eq!(a.content, "合成答复第一段。第二段保留限制。");
    assert_eq!(a.occurred_at.unwrap().timestamp(), 1767225602);
    assert_eq!(a.metadata["qwen"]["answer_segment_indices"], json!([1, 3]));
    let u = c
        .events
        .iter()
        .find(|e| e.source.message_id == "u")
        .unwrap();
    assert_eq!(u.metadata["qwen"]["children_ids"], json!(["b", "a"]));
    let all = serde_json::to_string(&c.events).unwrap();
    for secret in [
        "PRIVATE_REASONING_CANARY",
        "THINKING_CANARY",
        "TOOL_CANARY",
        "FUNCTION_ANSWER_CANARY",
        "DO_NOT_COPY_SYNTHETIC",
        "NOT_A_REAL_ATTACHMENT",
    ] {
        assert!(!all.contains(secret));
    }
    let legacy =
        parse_import_bytes("qwen-export", value.to_string().as_bytes(), "personal").unwrap();
    assert_eq!(legacy.events, c.events);
    assert_eq!(legacy.coverage.qwen, c.qwen_coverage);
}
#[test]
fn file_only_nodes_are_external_quotes_and_empty_hidden_nodes_leave_explicit_gaps() {
    let mut c = conversation("empty");
    c["chat"]["history"]["messages"]["u"]["content"] = json!("");
    c["chat"]["history"]["messages"]["a"]["childrenIds"] = json!(["next"]);
    c["chat"]["history"]["messages"]["a"]["content_list"] =
        json!([{"phase":"think","role":"assistant","content":"HIDDEN_ONLY_CANARY"}]);
    c["chat"]["history"]["messages"]["next"] = json!({"id":"next","role":"user","content":"后续可见要求","parentId":"a","childrenIds":[],"timestamp":null});
    c["currentId"] = json!("next");
    c["chat"]["history"]["currentId"] = json!("next");
    sync_projection(&mut c);
    let (r, out) = read(&wrapper(vec![c]), ImportLimits::default());
    let r = r.unwrap();
    assert_eq!(r.qwen.file_only_messages_imported, 1);
    assert_eq!(r.omitted_messages, 1);
    let u = out[0]
        .events
        .iter()
        .find(|e| e.source.message_id == "u")
        .unwrap();
    assert_eq!(u.kind, "file");
    assert_eq!(u.origin, recallcard::Origin::ExternalQuote);
    let next = out[0]
        .events
        .iter()
        .find(|e| e.source.message_id == "next")
        .unwrap();
    assert_eq!(next.metadata["qwen"]["nearest_visible_parent_id"], "u");
    assert_eq!(next.metadata["qwen"]["omitted_parent_nodes"], 1);
    assert!(next.occurred_at.is_none());
}
#[test]
fn graph_projection_and_role_corruption_are_rejected() {
    let original = conversation("invalid");
    for kind in 0..6 {
        let mut c = original.clone();
        match kind {
            0 => c["chat"]["history"]["messages"]["u"]["childrenIds"] = json!(["a", "a"]),
            1 => c["chat"]["history"]["messages"]["a"]["parentId"] = json!("missing"),
            2 => c["chat"]["messages"][0]["content"] = json!("不同投影"),
            3 => {
                c["chat"]["history"]["messages"]["a"]["role"] = json!("unknown");
                sync_projection(&mut c);
            }
            4 => {
                c["chat"]["history"]["messages"]["a"]["timestamp"] = json!("2026-01-01");
                sync_projection(&mut c);
            }
            _ => c["currentId"] = json!("b"),
        }
        assert!(
            read(&wrapper(vec![c]), ImportLimits::default()).0.is_err(),
            "kind {kind}"
        );
    }
}
#[test]
fn wrapper_streams_each_conversation_and_rejects_mixed_platform_data() {
    let values = (0..30)
        .map(|i| conversation(&format!("c-{i}")))
        .collect::<Vec<_>>();
    let bound = values.iter().map(|v| v.to_string().len()).max().unwrap() + 32;
    let limits = ImportLimits {
        conversation_bytes: bound,
        ..ImportLimits::default()
    };
    let value = wrapper(values);
    assert!(value.to_string().len() > bound * 20);
    let (r, _) = read(&value, limits);
    assert_eq!(r.unwrap().conversations, 30);
    assert!(read(
        &wrapper(vec![json!({"account":"not a conversation"})]),
        ImportLimits::default()
    )
    .0
    .is_err());
}
#[test]
fn invalid_wrapper_tail_never_publishes_previously_staged_messages() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::init(&dir.path().join("vault")).unwrap();
    let service = ImportService::new(&vault).unwrap();
    let mut value = wrapper(vec![conversation("stage-only")]);
    value["success"] = json!(false);
    let file = dir.path().join("qwen.json");
    std::fs::write(&file, value.to_string()).unwrap();
    let job = service
        .submit(ImportRequest {
            request_id: "qwen-invalid-tail".into(),
            paths: vec![file],
            scope: "personal".into(),
            format: "qwen-export".into(),
        })
        .unwrap();
    let status = service.run(&job.job_id, "personal").unwrap();
    assert_eq!(status.state, JobState::Failed);
    assert_eq!(status.error.unwrap().code, ErrorCode::InvalidConversation);
    assert!(vault.events().unwrap().is_empty());
}
#[test]
fn duplicate_wrapper_fields_and_trailing_json_are_not_accepted() {
    let valid = wrapper(vec![conversation("syntax")]).to_string();
    for raw in [
        format!("{valid} {{}}"),
        valid.replacen("\"success\":true", "\"success\":true,\"success\":true", 1),
    ] {
        let mut staged = 0;
        let result = read_source(
            &mut Cursor::new(raw.into_bytes()),
            "auto",
            "personal",
            ImportLimits::default(),
            |_| {
                staged += 1;
                Ok(())
            },
            |_| Ok(()),
        );
        assert!(result.is_err());
    }
}
#[test]
fn qwen_branch_handoff_keeps_selection_and_privacy_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let mut ui = DesktopSession::default();
    let info = ui.select_vault(&dir.path().join("vault"), true).unwrap();
    let vault = Vault::open(std::path::Path::new(&info.root)).unwrap();
    let (_, out) = read(
        &wrapper(vec![conversation("tree")]),
        ImportLimits::default(),
    );
    let events = vault
        .capture_batch(out.into_iter().next().unwrap().events)
        .unwrap();
    let session = events[0].data.session_key();
    let page = ui
        .conversation_messages(&info.session_id, "personal", &session, 0)
        .unwrap();
    assert_eq!(page["branch_summary"]["branch_count"], 2);
    let a = events
        .iter()
        .find(|e| e.data.source.message_id == "a")
        .unwrap();
    let b = events
        .iter()
        .find(|e| e.data.source.message_id == "b")
        .unwrap();
    let branch = ui
        .continuation_branch(
            &info.session_id,
            "personal",
            &session,
            "合成追问",
            Some(&format!("event:{}", a.id)),
        )
        .unwrap();
    assert!(branch["text"].as_str().unwrap().contains("第一段"));
    assert!(!branch["text"].as_str().unwrap().contains("未选分支"));
    vault.suppress(&b.id, "合成撤销".into()).unwrap();
    assert!(ui
        .continuation_branch(
            &info.session_id,
            "personal",
            &session,
            "",
            Some(&format!("event:{}", b.id))
        )
        .is_err());
}

#[test]
fn attachment_metadata_may_be_explicitly_unknown_without_fabricating_identity() {
    let mut c = conversation("missing-attachment-info");
    c["chat"]["history"]["messages"]["u"]["content"] = json!("");
    c["chat"]["history"]["messages"]["u"]["files"] = json!([{"id":null,"name":null,"size":null,"file_type":null,"url":"https://example.invalid/DO_NOT_COPY_UNKNOWN_ASSET"}]);
    sync_projection(&mut c);
    let (r, out) = read(&wrapper(vec![c.clone()]), ImportLimits::default());
    assert_eq!(r.unwrap().qwen.attachments_with_missing_metadata, 1);
    let e = out[0]
        .events
        .iter()
        .find(|e| e.source.message_id == "u")
        .unwrap();
    assert_eq!(e.kind, "file");
    assert!(e.metadata["source_assets"]["files"][0]["source_file_id"].is_null());
    assert!(!serde_json::to_string(e)
        .unwrap()
        .contains("DO_NOT_COPY_UNKNOWN_ASSET"));
    c["chat"]["history"]["messages"]["u"]["files"][0]["size"] = json!("invalid");
    sync_projection(&mut c);
    let error = read(&wrapper(vec![c]), ImportLimits::default())
        .0
        .unwrap_err();
    assert_eq!(error.message, "Qwen 附件大小无效");
}
