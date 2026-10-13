//! 普通回答导入只使用本地合成正文；不执行网站、模型或剪贴板。
use recallcard::{
    desktop::DesktopSession,
    model::{Origin, Role},
    Vault,
};
use serde_json::json;
use std::path::Path;
use tempfile::tempdir;

#[test]
fn ordinary_answer_is_previewed_then_saved_once_as_searchable_assistant_evidence() {
    let dir = tempdir().unwrap();
    let mut ui = DesktopSession::default();
    let info = ui.select_vault(&dir.path().join("vault"), true).unwrap();
    let vault = Vault::open(Path::new(&info.root)).unwrap();
    let preview = ui
        .preview_ai_reply(
            &info.session_id,
            "personal",
            "合成回答：建议周四检查图表。🙂",
        )
        .unwrap();
    assert!(vault.events().unwrap().is_empty());
    assert_eq!(preview.role, "assistant");
    assert!(!preview.automatic_memory);
    assert!(ui
        .confirm_ai_reply(&info.session_id, "project:other", &preview.preview_id)
        .is_err());
    assert!(ui
        .confirm_note(&info.session_id, &preview.preview_id)
        .is_err());
    let receipt = ui
        .confirm_ai_reply(&info.session_id, "personal", &preview.preview_id)
        .unwrap();
    assert_eq!(
        receipt,
        ui.confirm_ai_reply(&info.session_id, "personal", &preview.preview_id)
            .unwrap()
    );
    let events = vault.events().unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.data.role, Role::Assistant);
    assert_eq!(event.data.origin, Origin::AssistantOutput);
    assert_eq!(event.data.content, preview.content);
    assert_eq!(event.data.occurred_at, None);
    assert_eq!(event.data.reply_to, None);
    assert_eq!(event.data.metadata["exclude_from_automatic_memory"], true);
    assert_eq!(event.data.metadata["source_identity_verified"], false);
    assert!(vault.memories().unwrap().is_empty());
    let handoff = ui
        .prepare_selected_context(
            &info.session_id,
            "personal",
            &[receipt["ref"].as_str().unwrap().into()],
            "合成后续问题",
            32768,
        )
        .unwrap();
    assert_eq!(handoff["records"][0]["role"], "assistant");
    assert!(handoff["text"]
        .as_str()
        .unwrap()
        .contains("AI 回答，非用户事实"));
    for evidence in ["user_explicit", "observed"] {
        assert!(vault.add_memory(serde_json::from_value(json!({"content":"不可提升为用户事实", "source_refs":[event.id], "scope":"personal", "evidence":evidence})).unwrap()).is_err());
    }
}

#[test]
fn reply_redaction_cancellation_new_preview_and_session_changes_fail_closed() {
    let dir = tempdir().unwrap();
    let mut ui = DesktopSession::default();
    let info = ui.select_vault(&dir.path().join("vault"), true).unwrap();
    let first = ui
        .preview_ai_reply(
            &info.session_id,
            "personal",
            "合成回答 password=synthetic-reply-secret\n普通文字",
        )
        .unwrap();
    assert!(first.redacted);
    assert!(!first.content.contains("synthetic-reply-secret"));
    let next = ui
        .preview_ai_reply(&info.session_id, "personal", "下一条合成回答")
        .unwrap();
    assert!(ui
        .confirm_ai_reply(&info.session_id, "personal", &first.preview_id)
        .is_err());
    ui.cancel_ai_reply(&info.session_id, "personal", &first.preview_id)
        .unwrap();
    // 已过期的取消不能抹掉新预览。
    let receipt = ui
        .confirm_ai_reply(&info.session_id, "personal", &next.preview_id)
        .unwrap();
    assert_eq!(receipt["role"], "assistant");
    let next = ui
        .preview_ai_reply(&info.session_id, "personal", "需要定向取消的回答")
        .unwrap();
    ui.cancel_ai_reply(&info.session_id, "personal", &next.preview_id)
        .unwrap();
    assert!(ui
        .confirm_ai_reply(&info.session_id, "personal", &next.preview_id)
        .is_err());
    for invalid in [
        "".into(),
        "  \n".into(),
        "界".repeat(22000),
        "引用 recallcard.context/1 的合成文字".into(),
        "recallcard.dream-job/1".into(),
        "recallcard.dream-result/1".into(),
    ] {
        assert!(ui
            .preview_ai_reply(&info.session_id, "personal", &invalid)
            .is_err());
    }
    let next = ui
        .preview_ai_reply(&info.session_id, "personal", "新空间前的合成回答")
        .unwrap();
    let new = ui.select_vault(&dir.path().join("other"), true).unwrap();
    assert!(ui
        .confirm_ai_reply(&info.session_id, "personal", &next.preview_id)
        .is_err());
    assert!(ui
        .confirm_ai_reply(&new.session_id, "personal", &next.preview_id)
        .is_err());
    assert_eq!(
        Vault::open(Path::new(&info.root))
            .unwrap()
            .events()
            .unwrap()
            .len(),
        1
    );
}
