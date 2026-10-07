//! 工作区列表、来源与导入成果的真实服务合同；仅使用合成资料。
use recallcard::{
    desktop::{DesktopSession, VaultInfo},
    Event, Vault,
};
use serde_json::{json, Value};
use std::{fs, path::Path};
use tempfile::{tempdir, TempDir};

fn setup() -> (TempDir, DesktopSession, VaultInfo, Vault) {
    let dir = tempdir().unwrap();
    let mut session = DesktopSession::default();
    let info = session
        .select_vault(&dir.path().join("vault"), true)
        .unwrap();
    let vault = Vault::open(Path::new(&info.root)).unwrap();
    (dir, session, info, vault)
}

fn input(conversation: &str, message: &str, scope: &str, text: &str) -> Value {
    json!({"role":"user","origin":"user_input","scope":scope,"content":text,
        "metadata":{"conversation_title":format!("合成会话 {conversation}")},
        "source":{"platform":"synthetic-platform","conversation_id":conversation,"message_id":message}})
}

fn capture(vault: &Vault, value: Value) -> Event {
    vault
        .capture(serde_json::from_value(value).unwrap())
        .unwrap()
}

fn reference(event: &Event) -> String {
    format!("event:{}", event.id)
}

#[test]
fn list_read_and_sources_share_readable_metadata_without_inventing_a_memory_conversation() {
    let (_dir, session, info, vault) = setup();
    let user = capture(
        &vault,
        input("用户选择", "one", "personal", "合成共同检索词：选择 Rust"),
    );
    let mut assistant_input = input("助手建议", "two", "personal", "合成共同检索词：建议 Python");
    assistant_input["role"] = json!("assistant");
    assistant_input["origin"] = json!("assistant_output");
    let assistant = capture(&vault, assistant_input);
    let memory = vault.add_memory(serde_json::from_value(json!({
        "content":"合成共同检索词：需要区分用户选择和助手建议", "source_refs":[user.id,assistant.id],
        "evidence":"observed","scope":"personal"
    })).unwrap()).unwrap();
    let memory_ref = format!("memory:{}@{}", memory.id, memory.revision);
    for response in [
        session
            .search(&info.session_id, "personal", "合成共同检索词", "all")
            .unwrap(),
        session.browse(&info.session_id, "personal", "all").unwrap(),
    ] {
        let rows = response["results"].as_array().unwrap();
        let row = rows
            .iter()
            .find(|row| row["ref"] == reference(&user))
            .unwrap();
        assert_eq!(row["conversation_title"], "合成会话 用户选择");
        assert_eq!(row["conversation_ref"], user.data.session_key());
        assert_eq!(row["conversation_ref"], row["session_ref"]);
        assert_eq!(row["platform"], "synthetic-platform");
        assert_eq!(row["role"], "user");
        assert!(row["occurred_at"].is_null());
        let row = rows.iter().find(|row| row["ref"] == memory_ref).unwrap();
        assert!(row["session_ref"].is_null());
        assert!(row.get("conversation_ref").is_none());
        assert!(row.get("conversation_title").is_none());
        assert_eq!(row["source_summary"]["count"], 2);
        assert_eq!(row["source_summary"]["sources"][1]["role"], "assistant");
        assert_eq!(
            row["source_summary"]["sources"][1]["conversation_title"],
            "合成会话 助手建议"
        );
    }
    let read = session
        .read(&info.session_id, "personal", &reference(&user))
        .unwrap();
    assert_eq!(
        read["results"][0]["conversation_ref"],
        user.data.session_key()
    );
    let sources = session
        .sources(&info.session_id, "personal", &memory_ref)
        .unwrap();
    let events = sources["results"][0]["events"].as_array().unwrap();
    assert_eq!(events[0]["conversation_title"], "合成会话 用户选择");
    assert_eq!(events[1]["role"], "assistant");
    assert_eq!(events[1]["conversation_ref"], assistant.data.session_key());
}

#[test]
fn metadata_and_location_never_reintroduce_other_scopes_revisions_or_generated_context() {
    let (_dir, session, info, vault) = setup();
    let old = capture(
        &vault,
        input("旧标题不可见", "same", "personal", "合成旧正文"),
    );
    let mut revised = input("旧标题不可见", "same", "personal", "合成新正文");
    revised["metadata"]["conversation_title"] = json!("最新可见标题");
    let new = capture(&vault, revised);
    let private = capture(
        &vault,
        input("工作秘密标题", "private", "project:secret", "合成私人正文"),
    );
    let generated = capture(
        &vault,
        input(
            "注入秘密标题",
            "generated",
            "personal",
            "recallcard.context/1 合成重复副本",
        ),
    );
    for event in [&old, &private, &generated] {
        assert!(session
            .event_location(&info.session_id, "personal", &reference(event))
            .is_err());
    }
    let browse = session.browse(&info.session_id, "personal", "all").unwrap();
    assert_eq!(browse["results"].as_array().unwrap().len(), 1);
    assert_eq!(browse["results"][0]["conversation_title"], "最新可见标题");
    for hidden in ["旧标题不可见", "工作秘密标题", "注入秘密标题"] {
        assert!(!browse.to_string().contains(hidden));
    }
    assert_eq!(
        session
            .event_location(&info.session_id, "personal", &reference(&new))
            .unwrap()["total"],
        1
    );
    vault.suppress(&new.id, "合成遗忘测试".into()).unwrap();
    assert!(session
        .event_location(&info.session_id, "personal", &reference(&new))
        .is_err());
    assert_eq!(
        session.browse(&info.session_id, "personal", "all").unwrap()["total"],
        0
    );
    assert!(session
        .event_location("stale-session", "personal", &reference(&new))
        .is_err());
    assert!(session
        .event_location(&info.session_id, "*", &reference(&new))
        .is_err());
}

#[test]
fn location_opens_the_exact_message_after_variable_size_pages_and_for_disconnected_fragments() {
    let (_dir, session, info, vault) = setup();
    let mut expected = Vec::new();
    for index in 0_usize..27 {
        let mut value = input(
            "长会话",
            &format!("message-{index}"),
            "personal",
            &format!("合成第 {index} 条 {}", "长正文".repeat(1800)),
        );
        value["metadata"]["previous_message_id"] =
            json!(index.checked_sub(1).map(|i| format!("message-{i}")));
        expected.push(capture(&vault, value));
    }
    let first = session
        .conversation_messages(
            &info.session_id,
            "personal",
            &expected[0].data.session_key(),
            0,
        )
        .unwrap();
    assert!(first["next_offset"].as_u64().unwrap() < 20);
    for index in [0, 5, 19, 26] {
        let location = session
            .event_location(&info.session_id, "personal", &reference(&expected[index]))
            .unwrap();
        assert_eq!(location["message_index"], index);
        let page = session
            .conversation_messages(
                &info.session_id,
                "personal",
                location["conversation_ref"].as_str().unwrap(),
                location["offset"].as_u64().unwrap() as usize,
            )
            .unwrap();
        assert_eq!(page["messages"][0]["ref"], reference(&expected[index]));
    }
    let fragments: Vec<_> = (0..8)
        .map(|index| {
            capture(
                &vault,
                input(
                    "断开的片段",
                    &format!("fragment-{index}"),
                    "personal",
                    "合成未记录相对次序的片段",
                ),
            )
        })
        .collect();
    for event in fragments {
        let location = session
            .event_location(&info.session_id, "personal", &reference(&event))
            .unwrap();
        let page = session
            .conversation_messages(
                &info.session_id,
                "personal",
                location["conversation_ref"].as_str().unwrap(),
                location["offset"].as_u64().unwrap() as usize,
            )
            .unwrap();
        assert_eq!(page["messages"][0]["ref"], reference(&event));
        assert_eq!(page["order_known"], false);
    }
}

#[test]
fn enriched_search_and_browse_keep_the_response_budget_and_redact_metadata() {
    let (_dir, session, info, vault) = setup();
    for index in 0..30 {
        let mut value = input(
            &format!("batch-{index}"),
            "one",
            "personal",
            &format!("合成预算词 {}", "多字节检索正文".repeat(650)),
        );
        value["metadata"]["conversation_title"] = json!(format!(
            "password=synthetic-secret-{index} {}",
            "长标题".repeat(150)
        ));
        capture(&vault, value);
    }
    for response in [
        session
            .search(&info.session_id, "personal", "合成预算词", "events")
            .unwrap(),
        session
            .browse(&info.session_id, "personal", "events")
            .unwrap(),
    ] {
        assert!(serde_json::to_vec(&response).unwrap().len() <= 32768);
        assert!(!response.to_string().contains("synthetic-secret-"));
        let rows = response["results"].as_array().unwrap();
        assert!(!rows.is_empty());
        assert!(rows.iter().any(|row| row["conversation_title"]
            .as_str()
            .is_some_and(|title| title.contains("[REDACTED]"))));
        assert!(rows.iter().all(|row| row["text_truncated"] == true));
    }
}

#[test]
fn import_receipt_opens_only_real_visible_batch_conversations_and_counts_deduplicated_messages() {
    let (dir, mut session, info, vault) = setup();
    capture(
        &vault,
        input("第一段", "outside-batch", "personal", "合成既有额外消息"),
    );
    let events = [
        input("第一段", "one", "personal", "合成导入一"),
        input("第一段", "two", "personal", "合成导入二"),
        input("第二段", "three", "personal", "合成导入三"),
    ];
    let path = dir.path().join("batch.jsonl");
    fs::write(
        &path,
        events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let preview = session
        .preview_import(&info.session_id, "manual-jsonl", &path, "personal")
        .unwrap();
    let first = session
        .confirm_import(&info.session_id, &preview.preview_id)
        .unwrap();
    assert_eq!(first["events_added"], 3);
    assert_eq!(first["events_duplicates"], 0);
    assert_eq!(first["conversation_refs"].as_array().unwrap().len(), 2);
    let groups = first["conversations"].as_array().unwrap();
    assert_eq!(
        groups
            .iter()
            .find(|group| group["title"] == "合成会话 第一段")
            .unwrap()["message_count"],
        2
    );
    for group in groups {
        assert!(session
            .conversation_messages(
                &info.session_id,
                "personal",
                group["session_ref"].as_str().unwrap(),
                0
            )
            .is_ok());
    }
    let preview = session
        .preview_import(&info.session_id, "manual-jsonl", &path, "personal")
        .unwrap();
    let repeated = session
        .confirm_import(&info.session_id, &preview.preview_id)
        .unwrap();
    assert_eq!(repeated["events_added"], 0);
    assert_eq!(repeated["events_duplicates"], 3);
    assert_eq!(repeated["conversation_refs"], first["conversation_refs"]);
    let hidden = first["refs"][2]
        .as_str()
        .unwrap()
        .strip_prefix("event:")
        .unwrap();
    vault.suppress(hidden, "合成遗忘第二段".into()).unwrap();
    let preview = session
        .preview_import(&info.session_id, "manual-jsonl", &path, "personal")
        .unwrap();
    let repeated = session
        .confirm_import(&info.session_id, &preview.preview_id)
        .unwrap();
    assert_eq!(repeated["conversations"].as_array().unwrap().len(), 1);
    assert_eq!(repeated["conversations"][0]["title"], "合成会话 第一段");
    assert!(!repeated["conversations"].to_string().contains("第二段"));
}

#[test]
fn memory_source_summary_does_not_reveal_superseded_source_titles() {
    let (_dir, session, info, vault) = setup();
    let old = capture(
        &vault,
        input("合成隐藏旧来源", "same", "personal", "合成来源原话"),
    );
    let memory = vault.add_memory(serde_json::from_value(json!({"content":"合成记忆内容", "source_refs":[old.id],"evidence":"user_explicit","scope":"personal"})).unwrap()).unwrap();
    let mut updated = old.data.clone();
    updated.content = "合成修订原话".into();
    updated.metadata["conversation_title"] = json!("最新标题");
    vault.capture(updated).unwrap();
    let browse = session
        .browse(&info.session_id, "personal", "memories")
        .unwrap();
    assert_eq!(
        browse["results"][0]["ref"],
        format!("memory:{}@{}", memory.id, memory.revision)
    );
    assert_eq!(browse["results"][0]["source_summary"]["count"], 0);
    assert!(!browse.to_string().contains("合成隐藏旧来源"));
}

#[test]
fn read_and_sources_preserve_complete_evidence_when_optional_metadata_exceeds_budget() {
    let (_dir, session, info, vault) = setup();
    let text = "synthetic evidence ".repeat(1100);
    let mut value = input("极限字段", "one", "personal", &text);
    value["session_id"] = json!("synthetic-session-".repeat(600));
    let event = capture(&vault, value);
    let memory = vault
        .add_memory(
            serde_json::from_value(json!({
                "content":"合成证据必须保持完整", "source_refs":[event.id],
                "evidence":"user_explicit","scope":"personal"
            }))
            .unwrap(),
        )
        .unwrap();
    let read = session
        .read(&info.session_id, "personal", &reference(&event))
        .unwrap();
    assert!(serde_json::to_vec(&read).unwrap().len() <= 32768);
    assert_eq!(read["results"][0]["record"]["content"], text);
    assert!(read["results"][0].get("conversation_ref").is_none());
    let sources = session
        .sources(
            &info.session_id,
            "personal",
            &format!("memory:{}@{}", memory.id, memory.revision),
        )
        .unwrap();
    assert!(serde_json::to_vec(&sources).unwrap().len() <= 32768);
    assert_eq!(sources["results"][0]["events"][0]["content"], text);
    assert!(sources["results"][0]["events"][0]
        .get("conversation_ref")
        .is_none());
    assert!(!sources["truncated"].as_bool().unwrap());
}

#[test]
fn import_result_excludes_selected_conversations_without_any_saved_visible_text() {
    let (dir, mut session, info, _vault) = setup();
    let conversation = |id: &str, content: Value| {
        json!({
            "id":id,"title":format!("合成备份 {id}"),"current_node":"leaf",
            "mapping":{"leaf":{"parent":null,"message":{"id":"one","author":{"role":"user"},"content":content}}}
        })
    };
    let path = dir.path().join("partly-supported.json");
    fs::write(
        &path,
        json!([
            conversation(
                "实际文本",
                json!({"content_type":"text","parts":["合成可保存消息"]})
            ),
            conversation(
                "只有图片",
                json!({"content_type":"image","parts":[{"asset_pointer":"synthetic-image"}]})
            )
        ])
        .to_string(),
    )
    .unwrap();
    let preview = session
        .preview_import(&info.session_id, "chatgpt-export", &path, "personal")
        .unwrap();
    assert_eq!(preview.conversations.len(), 2);
    assert_eq!(preview.event_count, 1);
    let receipt = session
        .confirm_import(&info.session_id, &preview.preview_id)
        .unwrap();
    assert_eq!(receipt["events_seen"], 1);
    assert_eq!(receipt["coverage"]["conversations_selected"], 2);
    assert_eq!(receipt["conversations"].as_array().unwrap().len(), 1);
    assert_eq!(receipt["conversations"][0]["title"], "合成备份 实际文本");
    assert_eq!(receipt["conversation_refs"].as_array().unwrap().len(), 1);
    assert!(!receipt["conversations"].to_string().contains("只有图片"));
}

#[test]
fn invalid_revision_preflight_consumes_approval_without_writes_and_retry_is_idempotent() {
    let (dir, mut session, info, vault) = setup();
    let first = input("预验证后保存", "one", "personal", "合成第一条正文");
    let mut second = input("修复后保存", "two", "personal", "合成第二条正文");
    second["revision_of"] = json!(format!("evt_{}", "0".repeat(64)));
    let path = dir.path().join("interrupted.jsonl");
    fs::write(&path, format!("{first}\n{second}")).unwrap();
    let preview = session
        .preview_import(&info.session_id, "manual-jsonl", &path, "personal")
        .unwrap();
    let error = session
        .confirm_import(&info.session_id, &preview.preview_id)
        .unwrap_err();
    assert!(error.contains("已写入的事件可安全去重"));
    // 缺失修订引用是整批预验证错误，不是落盘中断；首条也不能提前写入。
    // 真正持久化后失败/取消的前缀保留由批捕获和可恢复任务测试独立覆盖。
    assert!(vault.events().unwrap().is_empty());
    assert!(session
        .confirm_import(&info.session_id, &preview.preview_id)
        .is_err());
    second.as_object_mut().unwrap().remove("revision_of");
    fs::write(&path, format!("{first}\n{second}")).unwrap();
    let preview = session
        .preview_import(&info.session_id, "manual-jsonl", &path, "personal")
        .unwrap();
    let receipt = session
        .confirm_import(&info.session_id, &preview.preview_id)
        .unwrap();
    assert_eq!(receipt["events_added"], 2);
    assert_eq!(receipt["events_duplicates"], 0);
    assert_eq!(receipt["conversation_refs"].as_array().unwrap().len(), 2);
    assert_eq!(vault.events().unwrap().len(), 2);

    // 修复后的导入仍需新令牌；再授权复导必须保留原始事件并准确报告重复。
    let before = vault.events().unwrap();
    let preview = session
        .preview_import(&info.session_id, "manual-jsonl", &path, "personal")
        .unwrap();
    let repeated = session
        .confirm_import(&info.session_id, &preview.preview_id)
        .unwrap();
    assert_eq!(repeated["events_added"], 0);
    assert_eq!(repeated["events_duplicates"], 2);
    assert_eq!(repeated["conversation_refs"].as_array().unwrap().len(), 2);
    assert_eq!(vault.events().unwrap(), before);
}

#[test]
fn conversation_header_is_reloaded_from_current_visible_events_after_revision_and_forgetting() {
    let (_dir, session, info, vault) = setup();
    let original = capture(&vault, input("旧标题", "one", "personal", "合成旧原话"));
    let session_ref = original.data.session_key();
    let first = session
        .conversation_messages(&info.session_id, "personal", &session_ref, 0)
        .unwrap();
    assert_eq!(first["title"], "合成会话 旧标题");
    let mut updated = original.data.clone();
    updated.content = "合成修订原话".into();
    updated.metadata["conversation_title"] = json!("修订后的标题");
    let revision = vault.capture(updated).unwrap();
    let refreshed = session
        .conversation_messages(&info.session_id, "personal", &session_ref, 0)
        .unwrap();
    assert_eq!(refreshed["title"], "修订后的标题");
    assert_eq!(refreshed["platform"], "synthetic-platform");
    assert_eq!(refreshed["session_ref"], session_ref);
    assert_eq!(refreshed["total"], 1);
    assert_eq!(refreshed["messages"][0]["ref"], reference(&revision));
    assert!(!refreshed.to_string().contains("合成会话 旧标题"));
    let mut second = input("旧标题", "two", "personal", "合成剩余原话");
    second["metadata"]["conversation_title"] = json!("剩余可见消息的标题");
    second["metadata"]["previous_message_id"] = json!("one");
    let remaining = capture(&vault, second);
    let later_page = session
        .conversation_messages(&info.session_id, "personal", &session_ref, 1)
        .unwrap();
    assert_eq!(later_page["title"], "修订后的标题");
    assert_eq!(later_page["offset"], 1);
    assert_eq!(later_page["total"], 2);
    vault
        .suppress(&revision.id, "合成遗忘原头部来源".into())
        .unwrap();
    let refreshed = session
        .conversation_messages(&info.session_id, "personal", &session_ref, 0)
        .unwrap();
    assert_eq!(refreshed["title"], "剩余可见消息的标题");
    assert_eq!(refreshed["total"], 1);
    assert_eq!(refreshed["messages"][0]["ref"], reference(&remaining));
    assert!(!refreshed.to_string().contains("修订后的标题"));
    assert!(!refreshed.to_string().contains("合成会话 旧标题"));
}
