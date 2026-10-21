//! 工作区列表、来源与导入成果的真实服务合同；仅使用合成资料。
use recallcard::{
    desktop::{DesktopSession, VaultInfo},
    Event, Vault,
};
use serde_json::{json, Value};
use std::path::Path;
use tempfile::{tempdir, TempDir};

mod support;

fn setup() -> (TempDir, DesktopSession, VaultInfo, Vault) {
    support::isolate_state();
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
    let browse = session.browse(&info.session_id, "personal", "all").unwrap();
    assert_eq!(browse["results"].as_array().unwrap().len(), 1);
    assert_eq!(browse["results"][0]["conversation_title"], "最新可见标题");
    for hidden in ["旧标题不可见", "工作秘密标题", "注入秘密标题"] {
        assert!(!browse.to_string().contains(hidden));
    }
    // 列表投影只给当前可见记录：旧 revision 与注入内容不进目录，但按引用读取
    // 原始事件仍然放行——这是"目录是派生视图、事实源是文件"的既定语义。
    assert!(session
        .read(&info.session_id, "personal", &reference(&old))
        .is_ok());
    assert!(session
        .read(&info.session_id, "personal", &reference(&generated))
        .is_ok());
    // 硬边界是 scope：跨范围读取与溯源一律 fail-closed，不能靠 personal 洗白。
    let private_ref = reference(&private);
    assert!(session
        .read(&info.session_id, "personal", &private_ref)
        .is_err());
    assert!(session
        .sources(&info.session_id, "personal", &private_ref)
        .is_err());
    assert!(session
        .read(&info.session_id, "project:secret", &reference(&private))
        .is_ok());
    vault.suppress(&new.id, "合成遗忘测试".into()).unwrap();
    assert_eq!(
        session.browse(&info.session_id, "personal", "all").unwrap()["total"],
        0
    );
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
