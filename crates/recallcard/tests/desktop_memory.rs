//! 用户维护记忆的实际资料库测试；使用合成内容。
use recallcard::{
    desktop::{DesktopSession, MemoryEdit, VaultInfo},
    Event, Memory, Vault,
};
use serde_json::json;
use std::path::Path;
use tempfile::{tempdir, TempDir};

fn setup() -> (TempDir, DesktopSession, VaultInfo, Vault, Event, Memory) {
    let dir = tempdir().unwrap();
    let mut ui = DesktopSession::default();
    let info = ui.select_vault(&dir.path().join("资料库"), true).unwrap();
    let vault = Vault::open(Path::new(&info.root)).unwrap();
    let event=vault.capture(serde_json::from_value(json!({"role":"user","origin":"user_input","scope":"personal","content":"合成：我偏好简洁说明","source":{"platform":"synthetic","conversation_id":"one","message_id":"one"}})).unwrap()).unwrap();
    let memory = add(&vault, &event, "合成：偏好简洁", true);
    (dir, ui, info, vault, event, memory)
}
fn add(vault: &Vault, event: &Event, content: &str, protected: bool) -> Memory {
    vault.add_memory(serde_json::from_value(json!({"content":content,"source_refs":[event.id],"evidence":"user_explicit","scope":event.data.scope,"protected":protected})).unwrap()).unwrap()
}
fn edit(content: &str, protected: bool) -> MemoryEdit {
    MemoryEdit {
        content: content.into(),
        protected,
        labels: vec!["合成".into()],
    }
}

#[test]
fn edit_preserves_sources_and_evidence_and_requires_protected_approval() {
    let (_dir, mut ui, info, vault, event, memory) = setup();
    let review = ui
        .review_memory_edit(
            &info.session_id,
            "personal",
            &memory.id,
            1,
            edit("合成：技术说明先简洁后细节", false),
        )
        .unwrap();
    assert!(review.requires_protected_approval);
    assert_eq!(vault.memory(&memory.id).unwrap(), memory);
    assert!(ui
        .confirm_memory_change(&info.session_id, &review.preview_id, false)
        .is_err());
    ui.confirm_memory_change(&info.session_id, &review.preview_id, true)
        .unwrap();
    let updated = vault.memory(&memory.id).unwrap();
    assert_eq!(updated.revision, 2);
    assert!(!updated.data.protected);
    assert_eq!(updated.data.evidence, memory.data.evidence);
    assert_eq!(updated.data.source_refs, memory.data.source_refs);
    assert_eq!(vault.event(&event.id).unwrap(), event);
    assert!(ui
        .confirm_memory_change(&info.session_id, &review.preview_id, true)
        .is_err());
}

#[test]
fn forget_shows_dependent_impact_and_restore_rechecks_remaining_rules() {
    let (_dir, mut ui, info, vault, event, memory) = setup();
    let other = add(&vault, &event, "合成：同一证据的另一记忆", false);
    let review = ui
        .review_memory_visibility(
            &info.session_id,
            "personal",
            &memory.id,
            false,
            "合成遗忘原因",
        )
        .unwrap();
    assert_eq!(review.affected_events, 1);
    assert_eq!(review.affected_memories, 2);
    assert!(!vault.is_suppressed(&event.id).unwrap());
    let saved = ui
        .confirm_memory_change(&info.session_id, &review.preview_id, true)
        .unwrap();
    assert_eq!(saved["hidden"], true);
    assert_eq!(
        ui.manage_memories(&info.session_id, "personal", false, 0)
            .unwrap()["total"],
        0
    );
    assert_eq!(
        ui.manage_memories(&info.session_id, "personal", true, 0)
            .unwrap()["total"],
        2
    );
    vault.suppress(&other.id, "合成另一规则".into()).unwrap();
    let restore = ui
        .review_memory_visibility(&info.session_id, "personal", &memory.id, true, "")
        .unwrap();
    let result = ui
        .confirm_memory_change(&info.session_id, &restore.preview_id, true)
        .unwrap();
    assert_eq!(result["hidden"], true);
    assert_eq!(vault.events().unwrap().len(), 1);
    assert!(ui
        .review_memory_visibility(&info.session_id, "personal", &memory.id, true, "")
        .is_err());
    let restore_other = ui
        .review_memory_visibility(&info.session_id, "personal", &other.id, true, "")
        .unwrap();
    assert!(restore_other.requires_protected_approval);
    assert!(ui
        .confirm_memory_change(&info.session_id, &restore_other.preview_id, false)
        .is_err());
    assert_eq!(
        ui.confirm_memory_change(&info.session_id, &restore_other.preview_id, true)
            .unwrap()["hidden"],
        false
    );
    assert_eq!(
        ui.manage_memories(&info.session_id, "personal", false, 0)
            .unwrap()["total"],
        2
    );
}

#[test]
fn changed_evidence_rules_or_revision_reject_old_approval() {
    let (_dir, mut ui, info, vault, _event, memory) = setup();
    let review = ui
        .review_memory_edit(
            &info.session_id,
            "personal",
            &memory.id,
            1,
            edit("合成修改", true),
        )
        .unwrap();
    vault.suppress(&memory.id, "合成撤回".into()).unwrap();
    assert!(ui
        .confirm_memory_change(&info.session_id, &review.preview_id, true)
        .is_err());
    assert_eq!(vault.memory(&memory.id).unwrap().revision, 1);
    assert!(ui
        .review_memory_edit(
            &info.session_id,
            "personal",
            &memory.id,
            1,
            edit("合成修改", true)
        )
        .is_err());
    vault.restore(&memory.id).unwrap();
    let review = ui
        .review_memory_edit(
            &info.session_id,
            "personal",
            &memory.id,
            1,
            edit("合成修改", true),
        )
        .unwrap();
    let mut input = memory.data.clone();
    input.content = "合成外部更新".into();
    vault.update_memory(&memory.id, 1, input).unwrap();
    assert!(ui
        .confirm_memory_change(&info.session_id, &review.preview_id, true)
        .is_err());
    assert_eq!(
        vault.memory(&memory.id).unwrap().data.content,
        "合成外部更新"
    );
}

#[test]
fn wrong_scope_cancel_and_vault_switch_do_not_write() {
    let (dir, mut ui, info, vault, _event, memory) = setup();
    assert!(ui
        .managed_memory(&info.session_id, "work", &memory.id)
        .is_err());
    assert!(ui
        .review_memory_edit(
            &info.session_id,
            "work",
            &memory.id,
            1,
            edit("跨范围", false)
        )
        .is_err());
    let review = ui
        .review_memory_visibility(&info.session_id, "personal", &memory.id, false, "取消")
        .unwrap();
    ui.cancel_previews(&info.session_id).unwrap();
    assert!(ui
        .confirm_memory_change(&info.session_id, &review.preview_id, true)
        .is_err());
    let review = ui
        .review_memory_visibility(&info.session_id, "personal", &memory.id, false, "切换")
        .unwrap();
    let next = ui
        .select_vault(&dir.path().join("另一个资料库"), true)
        .unwrap();
    assert!(ui
        .confirm_memory_change(&info.session_id, &review.preview_id, true)
        .is_err());
    assert!(ui
        .confirm_memory_change(&next.session_id, &review.preview_id, true)
        .is_err());
    assert!(!vault.is_suppressed(&memory.id).unwrap());
}

#[test]
fn hidden_memory_cannot_be_edited_and_nonexistent_restore_is_not_created() {
    let (_dir, mut ui, info, vault, event, memory) = setup();
    assert!(ui
        .review_memory_visibility(&info.session_id, "personal", &memory.id, true, "")
        .is_err());
    vault.suppress(&event.id, "来源遗忘".into()).unwrap();
    assert!(ui
        .review_memory_edit(
            &info.session_id,
            "personal",
            &memory.id,
            1,
            edit("合成修改", true)
        )
        .is_err());
    let listing = ui
        .manage_memories(&info.session_id, "personal", true, 0)
        .unwrap();
    assert_eq!(listing["memories"][0]["can_restore"], false);
    assert_eq!(listing["memories"][0]["hidden"], true);
}

#[test]
fn invalid_new_review_revokes_prior_confirmation() {
    let (_dir, mut ui, info, vault, _event, memory) = setup();
    let review = ui
        .review_memory_edit(
            &info.session_id,
            "personal",
            &memory.id,
            1,
            edit("有效修改", true),
        )
        .unwrap();
    assert!(ui
        .review_memory_edit(&info.session_id, "personal", &memory.id, 1, edit("", true))
        .is_err());
    assert!(ui
        .confirm_memory_change(&info.session_id, &review.preview_id, true)
        .is_err());
    assert_eq!(vault.memory(&memory.id).unwrap().revision, 1);
}

#[test]
fn management_reads_only_selected_memories_own_scoped_sources() {
    let (_dir, ui, info, vault, event, memory) = setup();
    vault.suppress(&memory.id, "合成隐藏".into()).unwrap();
    assert_eq!(
        ui.managed_memory_source(&info.session_id, "personal", &memory.id, &event.id)
            .unwrap(),
        event
    );
    let mut input = event.data.clone();
    input.source.message_id = "other".into();
    let unrelated = vault.capture(input).unwrap();
    assert!(ui
        .managed_memory_source(&info.session_id, "personal", &memory.id, &unrelated.id)
        .is_err());
    assert!(ui
        .managed_memory_source(&info.session_id, "work", &memory.id, &event.id)
        .is_err());
    assert!(ui
        .managed_memory_source(
            &info.session_id,
            "personal",
            &memory.id,
            "../control/schema-version.json"
        )
        .is_err());
}

#[test]
fn editing_assistant_derived_memory_does_not_promote_it_to_user_fact() {
    let (_dir, mut ui, info, vault, event, _memory) = setup();
    let memory=vault.add_memory(serde_json::from_value(json!({"content":"合成建议","source_refs":[event.id],"evidence":"assistant_suggestion","scope":"personal","protected":false})).unwrap()).unwrap();
    let review = ui
        .review_memory_edit(
            &info.session_id,
            "personal",
            &memory.id,
            1,
            edit("合成修订建议", false),
        )
        .unwrap();
    ui.confirm_memory_change(&info.session_id, &review.preview_id, false)
        .unwrap();
    let current = vault.memory(&memory.id).unwrap();
    assert_eq!(
        current.data.evidence,
        recallcard::Evidence::AssistantSuggestion
    );
    assert_eq!(current.state, recallcard::MemoryState::Tentative);
}
