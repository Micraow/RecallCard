//! 稳定背景选择的实际资料库验证；全部内容为合成资料。
use chrono::{Duration, Utc};
use recallcard::{
    context::{BootstrapArgs, Context},
    desktop::{BackgroundReview, DesktopSession, VaultInfo},
    policy::Access,
    transport, Event, Memory, MemoryState, Origin, Vault,
};
use serde_json::{json, Value};
use std::path::Path;
use tempfile::{tempdir, TempDir};

struct Fixture {
    _dir: TempDir,
    ui: DesktopSession,
    info: VaultInfo,
    vault: Vault,
    event: Event,
    memory: Memory,
}
fn setup() -> Fixture {
    let dir = tempdir().unwrap();
    let mut ui = DesktopSession::default();
    let info = ui.select_vault(&dir.path().join("资料库"), true).unwrap();
    let vault = Vault::open(Path::new(&info.root)).unwrap();
    let event = capture(&vault, "personal", "one", "合成：我偏好简洁说明");
    let memory = add(
        &vault,
        &event,
        "合成：偏好简洁",
        json!({"protected":false,"labels":["表达偏好","synthetic"],"observed_at":"2020-01-01T00:00:00Z","time_note":"合成时间说明","valid_from":"2020-01-01T00:00:00Z","entities":["合成用户"]}),
    );
    Fixture {
        _dir: dir,
        ui,
        info,
        vault,
        event,
        memory,
    }
}
fn capture(vault: &Vault, scope: &str, id: &str, content: &str) -> Event {
    vault.capture(serde_json::from_value(json!({"role":"user","origin":"user_input","scope":scope,"content":content,"source":{"platform":"synthetic","conversation_id":"one","message_id":id}})).unwrap()).unwrap()
}
fn add(vault: &Vault, event: &Event, content: &str, extras: Value) -> Memory {
    let mut input = json!({"content":content,"source_refs":[event.id],"evidence":"user_explicit","scope":event.data.scope});
    input
        .as_object_mut()
        .unwrap()
        .extend(extras.as_object().unwrap().clone());
    vault
        .add_memory(serde_json::from_value(input).unwrap())
        .unwrap()
}
fn review(f: &mut Fixture, include: bool) -> BackgroundReview {
    let memory = f.vault.memory(&f.memory.id).unwrap();
    f.ui.review_background_change(
        &f.info.session_id,
        "personal",
        &memory.id,
        memory.revision,
        include,
    )
    .unwrap()
}
fn confirm(f: &mut Fixture, review: &BackgroundReview, approval: bool) -> Result<Value, String> {
    f.ui.confirm_background_change(&f.info.session_id, "personal", &review.preview_id, approval)
}
fn bootstrap(vault: &Vault) -> Value {
    transport::invoke(
        &Context::new(vault, Access::new(vec!["personal".into()]).unwrap()),
        "bootstrap",
        json!({}),
    )
    .unwrap()
}

#[test]
fn choosing_existing_memory_previews_real_default_and_preserves_all_evidence() {
    let mut f = setup();
    let empty =
        f.ui.read_background(&f.info.session_id, "personal")
            .unwrap();
    assert_eq!(empty.selected_count, 0);
    assert!(!empty.candidates[0].selected);
    assert!(empty.candidates[0].can_include);
    assert_eq!(empty.candidates[0].source_refs, f.memory.data.source_refs);
    assert_eq!(
        f.ui.managed_memory_source(&f.info.session_id, "personal", &f.memory.id, &f.event.id)
            .unwrap(),
        f.event
    );
    let preview = review(&mut f, true);
    assert!(!preview.requires_protected_approval);
    assert_eq!(preview.background_before, bootstrap(&f.vault));
    assert_eq!(f.vault.memory(&f.memory.id).unwrap(), f.memory);
    assert_eq!(f.vault.memories().unwrap().len(), 1);
    assert_eq!(f.vault.events().unwrap(), vec![f.event.clone()]);
    let prospective_ref = format!("memory:{}@2", f.memory.id);
    assert!(preview.background_after["refs"]
        .as_array()
        .unwrap()
        .contains(&json!(prospective_ref)));
    assert!(preview.background_after["stable_text"]
        .as_str()
        .unwrap()
        .contains("证据：用户明确表达"));
    assert!(preview
        .copy_text_after
        .contains(preview.background_after["stable_text"].as_str().unwrap()));
    let saved = confirm(&mut f, &preview, false).unwrap();
    let memory = f.vault.memory(&f.memory.id).unwrap();
    let mut expected = f.memory.data.clone();
    expected.protected = true;
    expected.tags.push("bootstrap".into());
    assert_eq!(memory.data, expected);
    assert_eq!(memory.state, f.memory.state);
    assert_eq!(memory.created_at, f.memory.created_at);
    assert_eq!(memory.revision, 2);
    assert_eq!(saved["background"], preview.background_after);
    assert_eq!(saved["copy_text"], preview.copy_text_after);
    assert_eq!(saved["background"], bootstrap(&f.vault));
    let current =
        f.ui.read_background(&f.info.session_id, "personal")
            .unwrap();
    assert_eq!(current.background, saved["background"]);
    assert_eq!(current.selected_count, 1);
    assert!(current.candidates[0].selected && current.candidates[0].included);
    assert!(!current.candidates[0].can_include);
    assert!(current.candidates[0].can_remove);
    let handoff =
        f.ui.continuation(
            &f.info.session_id,
            "personal",
            &f.event.data.session_key(),
            "合成继续",
        )
        .unwrap();
    assert_eq!(handoff["background"], saved["background"]);
    assert!(handoff["text"]
        .as_str()
        .unwrap()
        .contains(saved["background"]["stable_text"].as_str().unwrap()));
    assert!(confirm(&mut f, &preview, false).is_err());
    assert_eq!(f.vault.event(&f.event.id).unwrap(), f.event);
}

#[test]
fn removing_membership_keeps_protection_sources_labels_and_searchability() {
    let mut f = setup();
    let preview = review(&mut f, true);
    confirm(&mut f, &preview, false).unwrap();
    let included = f.vault.memory(&f.memory.id).unwrap();
    let remove = review(&mut f, false);
    assert!(remove.requires_protected_approval);
    assert!(confirm(&mut f, &remove, false).is_err());
    assert_eq!(f.vault.memory(&f.memory.id).unwrap(), included);
    let saved = confirm(&mut f, &remove, true).unwrap();
    let actual = f.vault.memory(&f.memory.id).unwrap();
    let mut expected = included.data;
    expected.tags.retain(|tag| tag != "bootstrap");
    assert_eq!(actual.data, expected);
    assert!(actual.data.protected);
    assert_eq!(actual.revision, 3);
    assert_eq!(saved["background"], remove.background_after);
    assert!(!saved["background"]["stable_text"]
        .as_str()
        .unwrap()
        .contains(&actual.data.content));
    assert!(f
        .ui
        .search(&f.info.session_id, "personal", "合成：偏好简洁", "memories")
        .unwrap()
        .to_string()
        .contains(&f.memory.id));
    assert_eq!(f.vault.event(&f.event.id).unwrap(), f.event);
    let again = review(&mut f, true);
    assert!(again.requires_protected_approval);
    assert!(confirm(&mut f, &again, false).is_err());
    confirm(&mut f, &again, true).unwrap();
}

#[test]
fn ordinary_capture_changes_only_coverage_even_across_decimal_widths() {
    let mut f = setup();
    let preview = review(&mut f, true);
    confirm(&mut f, &preview, false).unwrap();
    let initial = bootstrap(&f.vault);
    for number in 2..=100 {
        capture(
            &f.vault,
            "personal",
            &format!("extra-{number}"),
            "合成普通捕获不改变稳定背景",
        );
        if [9, 10, 99, 100].contains(&number) {
            let actual = bootstrap(&f.vault);
            assert_eq!(actual["coverage"]["captured_events"], number);
            for field in ["stable_text", "bootstrap_version", "refs", "truncated"] {
                assert_eq!(actual[field], initial[field], "{field} at {number}");
            }
        }
    }
}

#[test]
fn wrong_scope_cancel_invalid_review_and_vault_switch_revoke_confirmation() {
    let mut f = setup();
    assert!(f
        .ui
        .read_background(&f.info.session_id, "project:other")
        .unwrap()
        .candidates
        .is_empty());
    assert!(f
        .ui
        .review_background_change(&f.info.session_id, "project:other", &f.memory.id, 1, true)
        .is_err());
    let preview = review(&mut f, true);
    assert!(f
        .ui
        .confirm_background_change(
            &f.info.session_id,
            "project:other",
            &preview.preview_id,
            false
        )
        .is_err());
    assert!(confirm(&mut f, &preview, false).is_err());
    let preview = review(&mut f, true);
    f.ui.cancel_previews(&f.info.session_id).unwrap();
    assert!(confirm(&mut f, &preview, false).is_err());
    let preview = review(&mut f, true);
    assert!(f
        .ui
        .review_background_change(&f.info.session_id, "personal", &f.memory.id, 999, true)
        .is_err());
    assert!(confirm(&mut f, &preview, false).is_err());
    let preview = review(&mut f, true);
    let next =
        f.ui.select_vault(&f._dir.path().join("另一个库"), true)
            .unwrap();
    assert!(confirm(&mut f, &preview, false).is_err());
    assert!(f
        .ui
        .confirm_background_change(&next.session_id, "personal", &preview.preview_id, false)
        .is_err());
    assert_eq!(f.vault.memory(&f.memory.id).unwrap(), f.memory);
}

#[test]
fn changed_memory_source_or_other_projection_record_revokes_preview() {
    let mut f = setup();
    let preview = review(&mut f, true);
    let mut changed = f.memory.data.clone();
    changed.time_note = "合成外部时间修正".into();
    f.vault.update_memory(&f.memory.id, 1, changed).unwrap();
    assert!(confirm(&mut f, &preview, false).is_err());
    let preview = review(&mut f, true);
    let revised = capture(&f.vault, "personal", "one", "合成：来源被明确修订");
    assert_eq!(revised.data.revision_of, Some(f.event.id.clone()));
    assert!(confirm(&mut f, &preview, false).is_err());
    let preview = review(&mut f, true);
    add(
        &f.vault,
        &revised,
        "合成其他背景",
        json!({"labels":["bootstrap"]}),
    );
    assert!(confirm(&mut f, &preview, false).is_err());
    assert!(!f.vault.memory(&f.memory.id).unwrap().data.protected);
}

#[test]
fn source_and_memory_suppression_invalidate_and_restoration_cannot_replay() {
    let mut f = setup();
    for target in [f.event.id.clone(), f.memory.id.clone()] {
        let preview = review(&mut f, true);
        f.vault.suppress(&target, "合成遗忘".into()).unwrap();
        assert!(confirm(&mut f, &preview, false).is_err());
        let page =
            f.ui.read_background(&f.info.session_id, "personal")
                .unwrap();
        assert!(page.candidates.is_empty());
        assert_eq!(page.total, 0);
        assert_eq!(page.selected_count, 0);
        let serialized = serde_json::to_string(&page).unwrap();
        assert!(!serialized.contains(&f.memory.data.content));
        assert!(!serialized.contains(&f.event.id));
        assert!(f
            .ui
            .review_background_change(&f.info.session_id, "personal", &f.memory.id, 1, true)
            .is_err());
        f.vault.restore(&target).unwrap();
        assert!(confirm(&mut f, &preview, false).is_err());
    }
    assert_eq!(f.vault.memory(&f.memory.id).unwrap(), f.memory);
}

#[test]
fn eligibility_explains_time_state_and_scope_without_promoting_suggestions() {
    let mut f = setup();
    let future = add(
        &f.vault,
        &f.event,
        "合成未来",
        json!({"valid_from":"2999-01-01T00:00:00Z"}),
    );
    let expired = add(
        &f.vault,
        &f.event,
        "合成过期",
        json!({"valid_to":"2000-01-01T00:00:00Z"}),
    );
    let tentative = add(
        &f.vault,
        &f.event,
        "合成助手建议",
        json!({"evidence":"assistant_suggestion"}),
    );
    let superseded = add(&f.vault, &f.event, "合成已替代", json!({}));
    f.vault
        .set_state(&superseded.id, 1, MemoryState::Superseded)
        .unwrap();
    let retracted = add(&f.vault, &f.event, "合成已撤回", json!({}));
    f.vault
        .set_state(&retracted.id, 1, MemoryState::Retracted)
        .unwrap();
    let other_source = capture(&f.vault, "project:private", "other", "合成其他分类");
    add(
        &f.vault,
        &other_source,
        "合成另一分类的背景",
        json!({"labels":["bootstrap"]}),
    );
    let page =
        f.ui.read_background(&f.info.session_id, "personal")
            .unwrap();
    assert_eq!(page.total, 5);
    assert!(!serde_json::to_string(&page).unwrap().contains("另一分类"));
    for (memory, reason) in [
        (&future, "生效"),
        (&expired, "过期"),
        (&tentative, "尚待确认"),
        (&superseded, "替代"),
    ] {
        let row = page
            .candidates
            .iter()
            .find(|row| row.id == memory.id)
            .unwrap();
        assert!(!row.can_include && !row.can_remove && !row.included);
        assert!(row.reason.contains(reason));
        let revision = f.vault.memory(&memory.id).unwrap().revision;
        assert!(f
            .ui
            .review_background_change(&f.info.session_id, "personal", &memory.id, revision, true)
            .is_err());
    }
    assert_eq!(f.vault.memory(&tentative.id).unwrap(), tentative);
    assert!(!page.candidates.iter().any(|row| row.id == retracted.id));
    let serialized = serde_json::to_string(&page).unwrap();
    assert!(!serialized.contains(&retracted.data.content));
    assert!(f
        .ui
        .review_background_change(&f.info.session_id, "personal", &retracted.id, 2, true)
        .is_err());
}

#[test]
fn manually_active_suggestion_retains_evidence_and_is_clearly_identified() {
    let mut f = setup();
    let suggestion = add(
        &f.vault,
        &f.event,
        "合成可考虑的建议",
        json!({"evidence":"assistant_suggestion","protected":false}),
    );
    let active = f
        .vault
        .set_state(&suggestion.id, 1, MemoryState::Active)
        .unwrap();
    let preview =
        f.ui.review_background_change(
            &f.info.session_id,
            "personal",
            &active.id,
            active.revision,
            true,
        )
        .unwrap();
    assert!(preview.background_after["stable_text"]
        .as_str()
        .unwrap()
        .contains("AI 建议，非用户事实"));
    f.ui.confirm_background_change(&f.info.session_id, "personal", &preview.preview_id, false)
        .unwrap();
    let saved = f.vault.memory(&active.id).unwrap();
    assert_eq!(saved.data.evidence, active.data.evidence);
    assert_eq!(saved.state, active.state);
    assert_eq!(saved.data.source_refs, active.data.source_refs);
}

#[test]
fn validity_expiring_after_review_prevents_write_without_source_changes() {
    let mut f = setup();
    let deadline = Utc::now() + Duration::seconds(2);
    let mut input = f.memory.data.clone();
    input.valid_to = Some(deadline);
    f.vault.update_memory(&f.memory.id, 1, input).unwrap();
    let before = f.vault.memory(&f.memory.id).unwrap();
    let preview = review(&mut f, true);
    let remaining = (deadline - Utc::now()).to_std().unwrap_or_default();
    std::thread::sleep(remaining + std::time::Duration::from_millis(20));
    assert!(confirm(&mut f, &preview, false)
        .unwrap_err()
        .contains("有效时间"));
    assert_eq!(f.vault.memory(&f.memory.id).unwrap(), before);
    assert!(confirm(&mut f, &preview, false).is_err());
}

#[test]
fn truncation_is_shared_and_never_carries_body_without_evidence_and_reference() {
    let mut f = setup();
    let long = add(
        &f.vault,
        &f.event,
        &"合成长背景必须保留证据引用。".repeat(100),
        json!({"protected":false}),
    );
    let review =
        f.ui.review_background_change(&f.info.session_id, "personal", &long.id, 1, true)
            .unwrap();
    assert_eq!(review.background_after["truncated"], true);
    assert!(review.copy_text_after.contains("部分已选记忆未带上"));
    let saved =
        f.ui.confirm_background_change(&f.info.session_id, "personal", &review.preview_id, false)
            .unwrap();
    assert_eq!(saved["background"], bootstrap(&f.vault));
    assert_eq!(saved["background"]["refs"], json!([]));
    assert!(!saved["background"]["stable_text"]
        .as_str()
        .unwrap()
        .contains("合成长背景"));
    let page =
        f.ui.read_background(&f.info.session_id, "personal")
            .unwrap();
    let row = page
        .candidates
        .iter()
        .find(|row| row.id == long.id)
        .unwrap();
    assert!(row.selected && !row.included && row.text_truncated);
    assert!(row.reason.contains("长度预算"));
    let larger = Context::new(&f.vault, Access::new(vec!["personal".into()]).unwrap())
        .bootstrap(BootstrapArgs {
            budget_tokens: 12000,
        })
        .unwrap();
    assert!(larger["stable_text"]
        .as_str()
        .unwrap()
        .contains(&long.data.content));
    assert_eq!(larger["refs"], json!([format!("memory:{}@2", long.id)]));
    let handoff =
        f.ui.continuation(
            &f.info.session_id,
            "personal",
            &f.event.data.session_key(),
            "",
        )
        .unwrap();
    assert_eq!(handoff["background"], saved["background"]);
    assert_eq!(handoff["truncated"], true);
}

#[test]
fn candidate_pages_count_all_selected_and_legacy_membership_can_be_protected() {
    let mut f = setup();
    let legacy = add(
        &f.vault,
        &f.event,
        "合成旧的未保护背景",
        json!({"labels":["bootstrap","保留标签"],"protected":false}),
    );
    for number in 0..30 {
        add(&f.vault, &f.event, &format!("合成候选 {number}"), json!({}));
    }
    let first =
        f.ui.read_background(&f.info.session_id, "personal")
            .unwrap();
    assert_eq!(first.total, 32);
    assert_eq!(first.candidates.len(), 30);
    assert_eq!(first.selected_count, 1);
    assert_eq!(first.next_offset, Some(30));
    assert!(!first.candidates.iter().any(|row| row.selected));
    let next =
        f.ui.read_background_page(&f.info.session_id, "personal", 30)
            .unwrap();
    assert_eq!(next.candidates.len(), 2);
    assert_eq!(next.next_offset, None);
    assert_eq!(next.background, first.background);
    let row = next
        .candidates
        .iter()
        .find(|row| row.id == legacy.id)
        .unwrap();
    assert!(row.selected && row.can_include && row.can_remove && !row.included);
    let preview =
        f.ui.review_background_change(&f.info.session_id, "personal", &legacy.id, 1, true)
            .unwrap();
    assert_eq!(preview.after.tags, legacy.data.tags);
    assert!(preview.after.protected);
    f.ui.confirm_background_change(&f.info.session_id, "personal", &preview.preview_id, false)
        .unwrap();
    assert!(f.vault.memory(&legacy.id).unwrap().data.protected);
    assert!(f
        .ui
        .read_background_page(&f.info.session_id, "personal", 1_000_001)
        .is_err());
}

#[test]
fn copied_background_is_context_injection_and_cannot_be_new_memory_evidence() {
    let mut f = setup();
    let preview = review(&mut f, true);
    confirm(&mut f, &preview, false).unwrap();
    let page =
        f.ui.read_background(&f.info.session_id, "personal")
            .unwrap();
    assert!(page.copy_text.starts_with("recallcard.context/1\n"));
    let recaptured = capture(&f.vault, "personal", "copied-background", &page.copy_text);
    assert_eq!(recaptured.data.origin, Origin::ContextInjection);
    assert!(f.vault.add_memory(serde_json::from_value(json!({"content":"合成重复提炼","source_refs":[recaptured.id],"evidence":"user_explicit","scope":"personal"})).unwrap()).is_err());
    assert_eq!(bootstrap(&f.vault), page.background);
}

#[test]
fn selected_expired_future_and_tentative_memories_can_be_removed_without_reactivation() {
    let mut f = setup();
    for (name, extra) in [
        ("合成过期已选", json!({"valid_to":"2000-01-01T00:00:00Z"})),
        ("合成未来已选", json!({"valid_from":"2999-01-01T00:00:00Z"})),
        (
            "合成待确认已选",
            json!({"evidence":"assistant_suggestion","valid_to":"2000-01-01T00:00:00Z"}),
        ),
    ] {
        let mut extras = json!({"protected":true,"labels":["bootstrap","保留标签"]});
        extras
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let memory = add(&f.vault, &f.event, name, extras);
        let page =
            f.ui.read_background(&f.info.session_id, "personal")
                .unwrap();
        let row = page
            .candidates
            .iter()
            .find(|row| row.id == memory.id)
            .unwrap();
        assert!(row.selected && row.can_remove && !row.can_include && !row.included);
        assert!(row.reason.contains("仍可取消"));
        assert!(f
            .ui
            .review_background_change(&f.info.session_id, "personal", &memory.id, 1, true)
            .is_err());
        let review =
            f.ui.review_background_change(&f.info.session_id, "personal", &memory.id, 1, false)
                .unwrap();
        assert!(review.requires_protected_approval);
        assert_eq!(f.vault.memory(&memory.id).unwrap(), memory);
        assert!(f
            .ui
            .confirm_background_change(&f.info.session_id, "personal", &review.preview_id, false)
            .is_err());
        let saved = f
            .ui
            .confirm_background_change(&f.info.session_id, "personal", &review.preview_id, true)
            .unwrap();
        let actual = f.vault.memory(&memory.id).unwrap();
        let mut expected = memory.data.clone();
        expected.tags.retain(|tag| tag != "bootstrap");
        assert_eq!(actual.data, expected);
        assert_eq!(actual.state, memory.state);
        assert_eq!(actual.revision, 2);
        assert!(actual.data.protected);
        assert_eq!(saved["background"], bootstrap(&f.vault));
        assert!(!saved["background"]["stable_text"]
            .as_str()
            .unwrap()
            .contains(name));
    }
}

#[test]
fn ordinary_background_omits_suppressed_and_retracted_bodies_sources_and_counts() {
    let f = setup();
    let own_source = capture(
        &f.vault,
        "personal",
        "hidden-source",
        "合成隐藏来源独特原文",
    );
    let own = add(
        &f.vault,
        &own_source,
        "合成被遗忘背景独特正文",
        json!({"labels":["bootstrap"]}),
    );
    f.vault.suppress(&own.id, "合成主动遗忘".into()).unwrap();
    let shared_source = capture(
        &f.vault,
        "personal",
        "shared-hidden-source",
        "合成来源遗忘独特原文",
    );
    let shared = add(
        &f.vault,
        &shared_source,
        "合成因来源遗忘的背景独特正文",
        json!({"labels":["bootstrap"]}),
    );
    f.vault
        .suppress(&shared_source.id, "合成来源主动遗忘".into())
        .unwrap();
    let withdrawn_source = capture(
        &f.vault,
        "personal",
        "withdrawn-source",
        "合成撤回来源独特原文",
    );
    let withdrawn = add(
        &f.vault,
        &withdrawn_source,
        "合成已撤回背景独特正文",
        json!({"labels":["bootstrap"]}),
    );
    f.vault
        .set_state(&withdrawn.id, 1, MemoryState::Retracted)
        .unwrap();
    let page =
        f.ui.read_background(&f.info.session_id, "personal")
            .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.selected_count, 0);
    assert_eq!(page.candidates.len(), 1);
    let serialized = serde_json::to_string(&page).unwrap();
    for memory in [own, shared, withdrawn] {
        assert!(!serialized.contains(&memory.id));
        assert!(!serialized.contains(&memory.data.content));
        for source in memory.data.source_refs {
            assert!(!serialized.contains(&source));
        }
    }
    assert_eq!(
        f.ui.manage_memories(&f.info.session_id, "personal", true, 0)
            .unwrap()["total"],
        4
    );
}

#[test]
fn ordinary_background_source_rechecks_visibility_membership_and_scope() {
    let f = setup();
    let current =
        f.ui.read_background(&f.info.session_id, "personal")
            .unwrap();
    assert_eq!(current.candidates[0].source_refs, vec![f.event.id.clone()]);
    assert_eq!(
        f.ui.background_memory_source(&f.info.session_id, "personal", &f.memory.id, &f.event.id)
            .unwrap(),
        f.event
    );
    let unrelated = capture(&f.vault, "personal", "unrelated", "合成不属于该记忆的来源");
    assert!(f
        .ui
        .background_memory_source(&f.info.session_id, "personal", &f.memory.id, &unrelated.id)
        .is_err());
    assert!(f
        .ui
        .background_memory_source(
            &f.info.session_id,
            "project:other",
            &f.memory.id,
            &f.event.id
        )
        .is_err());
    for target in [&f.memory.id, &f.event.id] {
        f.vault
            .suppress(target, "合成候选显示后遗忘".into())
            .unwrap();
        assert!(f
            .ui
            .background_memory_source(&f.info.session_id, "personal", &f.memory.id, &f.event.id)
            .is_err());
        assert_eq!(
            f.ui.managed_memory_source(&f.info.session_id, "personal", &f.memory.id, &f.event.id)
                .unwrap(),
            f.event
        );
        f.vault.restore(target).unwrap();
    }
    f.vault
        .set_state(&f.memory.id, 1, MemoryState::Retracted)
        .unwrap();
    assert!(f
        .ui
        .background_memory_source(&f.info.session_id, "personal", &f.memory.id, &f.event.id)
        .is_err());
    assert_eq!(
        f.ui.managed_memory_source(&f.info.session_id, "personal", &f.memory.id, &f.event.id)
            .unwrap(),
        f.event
    );
}

#[test]
fn ordinary_background_full_memory_rechecks_visibility_even_when_revision_is_unchanged() {
    let mut f = setup();
    let page =
        f.ui.read_background(&f.info.session_id, "personal")
            .unwrap();
    let row = &page.candidates[0];
    assert_eq!(
        f.ui.background_memory(&f.info.session_id, "personal", &row.id)
            .unwrap(),
        f.memory
    );
    assert!(f
        .ui
        .background_memory(&f.info.session_id, "project:other", &row.id)
        .is_err());
    for target in [f.memory.id.clone(), f.event.id.clone()] {
        // 已经打开一个未保存的预览，不得给其旧候选开放隐藏全文。
        let preview = review(&mut f, true);
        f.vault
            .suppress(&target, "合成全文打开前遗忘".into())
            .unwrap();
        assert_eq!(f.vault.memory(&row.id).unwrap().revision, row.revision);
        assert!(f
            .ui
            .background_memory(&f.info.session_id, "personal", &row.id)
            .is_err());
        assert!(confirm(&mut f, &preview, false).is_err());
        assert_eq!(
            f.ui.managed_memory(&f.info.session_id, "personal", &row.id)
                .unwrap(),
            f.memory
        );
        f.vault.restore(&target).unwrap();
    }
    f.vault
        .set_state(&row.id, 1, MemoryState::Retracted)
        .unwrap();
    assert!(f
        .ui
        .background_memory(&f.info.session_id, "personal", &row.id)
        .is_err());
    assert_eq!(
        f.ui.managed_memory(&f.info.session_id, "personal", &row.id)
            .unwrap()
            .state,
        MemoryState::Retracted
    );
}
