//! 跨会话选文交接使用真实服务和合成正本，不联网、不生成持久化摘要。
use recallcard::{
    context::{BootstrapArgs, Context},
    desktop::{DesktopSession, VaultInfo},
    policy::Access,
    Event, Memory, MemoryState, Vault,
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path};
use tempfile::{tempdir, TempDir};

struct Fixture {
    _dir: TempDir,
    ui: DesktopSession,
    info: VaultInfo,
    vault: Vault,
}

fn setup() -> Fixture {
    let dir = tempdir().unwrap();
    let mut ui = DesktopSession::default();
    let info = ui.select_vault(&dir.path().join("vault"), true).unwrap();
    let vault = Vault::open(Path::new(&info.root)).unwrap();
    Fixture {
        _dir: dir,
        ui,
        info,
        vault,
    }
}

fn input(conversation: &str, message: &str, text: &str) -> Value {
    json!({"role":"user","origin":"user_input","scope":"personal","content":text,
        "metadata":{"conversation_title":format!("合成会话 {conversation}")},
        "source":{"platform":"synthetic","conversation_id":conversation,"message_id":message}})
}

fn capture(f: &Fixture, input: Value) -> Event {
    f.vault
        .capture(serde_json::from_value(input).unwrap())
        .unwrap()
}
fn event_ref(event: &Event) -> String {
    format!("event:{}", event.id)
}
fn memory_ref(memory: &Memory) -> String {
    format!("memory:{}@{}", memory.id, memory.revision)
}

fn add(f: &Fixture, event: &Event, text: &str, extras: Value) -> Memory {
    let mut input = json!({"content":text,"source_refs":[event.id],"evidence":"user_explicit","scope":event.data.scope});
    input
        .as_object_mut()
        .unwrap()
        .extend(extras.as_object().unwrap().clone());
    f.vault
        .add_memory(serde_json::from_value(input).unwrap())
        .unwrap()
}
fn prepare(f: &Fixture, refs: &[String], goal: &str, budget: usize) -> Result<Value, String> {
    f.ui.prepare_selected_context(&f.info.session_id, "personal", refs, goal, budget)
}
fn bootstrap(f: &Fixture) -> Value {
    Context::new(&f.vault, Access::new(vec!["personal".into()]).unwrap())
        .bootstrap(BootstrapArgs::default())
        .unwrap()
}

#[test]
fn mixed_sessions_keep_exact_words_roles_times_and_unconfirmed_evidence() {
    let f = setup();
    let user = capture(
        &f,
        input("项目决定", "1", "合成：我选择 Rust，而不是 Python。"),
    );
    let mut assistant_input = input(
        "可选建议",
        "2",
        "合成：可以考虑 Python；用户尚未确认。\n保留第二行。",
    );
    assistant_input["role"] = json!("assistant");
    assistant_input["origin"] = json!("assistant_output");
    assistant_input["occurred_at"] = json!("2024-03-04T05:06:07Z");
    let assistant = capture(&f, assistant_input);
    let memory = add(
        &f,
        &assistant,
        "合成：可考虑 Python",
        json!({"evidence":"assistant_suggestion"}),
    );
    let refs = [event_ref(&user), memory_ref(&memory), event_ref(&assistant)];
    let result = prepare(&f, &refs, "核对我的选择", 32768).unwrap();
    assert_eq!(result["selected_count"], 3);
    assert_eq!(result["selected_refs"], json!(refs));
    assert_eq!(result["included_count"], 3);
    assert_eq!(result["partial"], false);
    assert_eq!(result["truncated"], false);
    assert_eq!(result["pending_refs"], json!([]));
    let rows = result["records"].as_array().unwrap();
    assert_eq!(rows[0]["ref"], refs[0]);
    assert_eq!(rows[0]["role"], "user");
    assert_eq!(rows[0]["text"], user.data.text());
    assert_eq!(rows[0]["conversation_title"], "合成会话 项目决定");
    assert_eq!(rows[0]["conversation_ref"], user.data.session_key());
    assert!(rows[0]["occurred_at"].is_null());
    assert!(rows[1]["role"].is_null());
    assert!(rows[1]["conversation_title"].is_null());
    assert!(rows[1]["conversation_ref"].is_null());
    assert_eq!(rows[1]["evidence"], "AssistantSuggestion");
    assert_eq!(rows[1]["status"], "tentative");
    assert_eq!(rows[1]["text"], memory.data.content);
    assert_eq!(rows[1]["source_refs"], json!([event_ref(&assistant)]));
    assert_eq!(rows[1]["sources"][0]["role"], "assistant");
    assert_eq!(
        rows[1]["sources"][0]["conversation_ref"],
        assistant.data.session_key()
    );
    assert_eq!(rows[2]["text"], assistant.data.text());
    assert_eq!(rows[2]["role"], "assistant");
    assert_eq!(rows[2]["occurred_at"], "2024-03-04T05:06:07Z");
    let text = result["text"].as_str().unwrap();
    assert!(text.contains("AI 建议，非用户事实"));
    assert!(text.contains("非原始逐字引文"));
    assert!(text.contains("不代表事件先后、会话分支或因果关系"));
    assert!(text.contains("发生/观察时间：未知"));
    assert!(!text.contains(&user.captured_at.to_rfc3339()));
    assert!(text.contains(&user.data.text()));
    assert!(text.contains(&assistant.data.text()));
}

#[test]
fn actual_text_begins_with_same_real_default_bootstrap_across_goals_refs_and_budgets() {
    let f = setup();
    let one = capture(&f, input("背景", "1", "合成：请用中文说明"));
    let two = capture(&f, input("新主题", "2", "合成：继续设计"));
    let background = add(
        &f,
        &one,
        "合成：偏好中文",
        json!({"labels":["bootstrap"],"protected":true}),
    );
    let expected = bootstrap(&f);
    let a = prepare(&f, &[event_ref(&one)], "第一个目标", 32768).unwrap();
    let b = prepare(
        &f,
        &[memory_ref(&background), event_ref(&two)],
        "第二个目标",
        16000,
    )
    .unwrap();
    assert_eq!(a["background"], expected);
    assert_eq!(b["background"], expected);
    assert_eq!(a["stable_prefix"], b["stable_prefix"]);
    let prefix = a["stable_prefix"].as_str().unwrap();
    assert!(prefix.ends_with(expected["stable_text"].as_str().unwrap()));
    assert!(a["text"].as_str().unwrap().starts_with(prefix));
    assert!(b["text"].as_str().unwrap().starts_with(prefix));
    assert!(!prefix.contains("第一个目标"));
    assert!(!prefix.contains("第二个目标"));
    capture(&f, input("后续捕获", "3", "合成：动态捕获不改前缀"));
    let c = prepare(&f, &[event_ref(&two)], "第三个目标", 32768).unwrap();
    assert_eq!(c["stable_prefix"], a["stable_prefix"]);
    assert_ne!(c["background"]["coverage"], a["background"]["coverage"]);
    let mut changed = background.data.clone();
    changed.content = "合成：偏好简明中文".into();
    f.vault
        .update_memory(&background.id, background.revision, changed)
        .unwrap();
    let d = prepare(&f, &[event_ref(&two)], "第三个目标", 32768).unwrap();
    assert_ne!(d["stable_prefix"], c["stable_prefix"]);
    assert_eq!(d["background"], bootstrap(&f));
}

#[test]
fn default_bootstrap_truncation_is_preserved_and_disclosed() {
    let f = setup();
    let event = capture(&f, input("背景", "1", "合成来源"));
    for i in 0..5 {
        add(
            &f,
            &event,
            &format!("合成背景{i}{}", "长度测试".repeat(80)),
            json!({"labels":["bootstrap"],"protected":true}),
        );
    }
    let result = prepare(&f, &[event_ref(&event)], "", 32768).unwrap();
    assert_eq!(result["background"], bootstrap(&f));
    assert_eq!(result["background"]["truncated"], true);
    assert_eq!(result["truncated"], true);
    assert_eq!(result["partial"], false);
    assert!(result["text"]
        .as_str()
        .unwrap()
        .contains("默认稳定背景本身存在截短"));
}

#[test]
fn rejects_duplicates_noncanonical_refs_missing_versions_and_invalid_limits() {
    let f = setup();
    let event = capture(&f, input("校验", "1", "合成正文"));
    let memory = add(&f, &event, "合成记忆", json!({}));
    let reference = event_ref(&event);
    for refs in [
        vec![],
        vec![reference.clone(); 9],
        vec![reference.clone(), reference.clone()],
        vec![event.id.clone()],
        vec![format!("{reference}@1")],
        vec![format!("memory:{}", memory.id)],
        vec![format!("memory:{}@01", memory.id)],
        vec![format!("memory:{}@0", memory.id)],
        vec!["view:profile".into()],
        vec!["event:../../private".into()],
        vec![format!("event:evt_{}", "0".repeat(64))],
    ] {
        assert!(prepare(&f, &refs, "", 32768).is_err(), "{refs:?}");
    }
    for budget in [0, 511, 512, 32769, usize::MAX] {
        assert!(prepare(&f, std::slice::from_ref(&reference), "", budget).is_err());
    }
    assert!(prepare(&f, &[reference], &"长".repeat(1366), 32768).is_err());
}

#[test]
fn scope_and_generated_context_cannot_enter_selected_records_or_background() {
    let f = setup();
    let personal = capture(&f, input("普通", "1", "合成公开"));
    let mut private_input = input("工作", "2", "合成秘密");
    private_input["scope"] = json!("project:secret");
    let private = capture(&f, private_input);
    let private_memory = add(
        &f,
        &private,
        "合成私有背景",
        json!({"labels":["bootstrap"]}),
    );
    let generated = capture(
        &f,
        input("导入", "3", "recallcard.context/1 合成旧交接副本"),
    );
    for hidden in [
        event_ref(&private),
        memory_ref(&private_memory),
        event_ref(&generated),
    ] {
        assert!(prepare(&f, &[event_ref(&personal), hidden], "", 32768).is_err());
    }
    let result = prepare(&f, &[event_ref(&personal)], "", 32768).unwrap();
    assert!(!result.to_string().contains("合成秘密"));
    assert!(!result.to_string().contains("合成私有背景"));
    assert!(f
        .ui
        .prepare_selected_context(
            &f.info.session_id,
            "../bad",
            &[event_ref(&personal)],
            "",
            32768
        )
        .is_err());
    let private_result =
        f.ui.prepare_selected_context(
            &f.info.session_id,
            "project:secret",
            &[event_ref(&private)],
            "",
            32768,
        )
        .unwrap();
    assert_eq!(private_result["records"][0]["text"], "合成秘密");
}

#[test]
fn forgetting_and_restoring_revalidates_both_selection_and_its_background_sources() {
    let f = setup();
    let event = capture(&f, input("遗忘", "1", "合成原话"));
    let memory = add(&f, &event, "合成固定背景", json!({"labels":["bootstrap"]}));
    let other = capture(&f, input("其他", "2", "合成其他话题"));
    let refs = [event_ref(&event), memory_ref(&memory)];
    let before = prepare(&f, &refs, "继续", 32768).unwrap();
    f.vault.suppress(&event.id, "合成遗忘".into()).unwrap();
    assert!(prepare(&f, &refs, "继续", 32768).is_err());
    assert!(prepare(&f, &[memory_ref(&memory)], "继续", 32768).is_err());
    let safe = prepare(&f, &[event_ref(&other)], "继续", 32768).unwrap();
    assert!(!safe.to_string().contains("合成固定背景"));
    f.vault.restore(&event.id).unwrap();
    let restored = prepare(&f, &refs, "继续", 32768).unwrap();
    assert_eq!(before["text"], restored["text"]);
    f.vault.suppress(&memory.id, "合成遗忘记忆".into()).unwrap();
    assert!(prepare(&f, &refs, "继续", 32768).is_err());
    f.vault.restore(&memory.id).unwrap();
    assert_eq!(
        prepare(&f, &refs, "继续", 32768).unwrap()["text"],
        before["text"]
    );
}

#[test]
fn stale_memory_revisions_retractions_and_expired_memories_are_rejected() {
    let f = setup();
    let event = capture(&f, input("版本", "1", "合成原话"));
    let memory = add(&f, &event, "合成旧版本", json!({}));
    let mut updated = memory.data.clone();
    updated.content = "合成新版本".into();
    let revised = f
        .vault
        .update_memory(&memory.id, memory.revision, updated)
        .unwrap();
    assert!(prepare(&f, &[memory_ref(&memory)], "", 32768).is_err());
    assert_eq!(
        prepare(&f, &[memory_ref(&revised)], "", 32768).unwrap()["records"][0]["text"],
        "合成新版本"
    );
    let retracted = f
        .vault
        .set_state(&revised.id, revised.revision, MemoryState::Retracted)
        .unwrap();
    assert!(prepare(&f, &[memory_ref(&revised)], "", 32768).is_err());
    assert!(prepare(&f, &[memory_ref(&retracted)], "", 32768).is_err());
    let expired = add(
        &f,
        &event,
        "合成已过期",
        json!({"valid_to":"2001-01-01T00:00:00Z"}),
    );
    assert!(prepare(&f, &[memory_ref(&expired)], "", 32768).is_err());
}

#[test]
fn revised_events_cannot_be_selected_but_memory_keeps_its_exact_original_source() {
    let f = setup();
    let old = capture(&f, input("来源", "same", "合成旧原话"));
    let memory = add(&f, &old, "合成旧原话对应记忆", json!({}));
    let mut updated = input("来源", "same", "合成新原话");
    updated["metadata"]["conversation_title"] = json!("合成新标题");
    let current = capture(&f, updated);
    assert!(prepare(&f, &[event_ref(&old)], "", 32768).is_err());
    let result = prepare(&f, &[memory_ref(&memory), event_ref(&current)], "", 32768).unwrap();
    assert_eq!(
        result["records"][0]["source_refs"],
        json!([event_ref(&old)])
    );
    assert_eq!(
        result["records"][0]["sources"][0]["conversation_title"],
        "合成会话 来源"
    );
    assert_eq!(result["records"][1]["text"], "合成新原话");
    let sources =
        f.ui.sources(&f.info.session_id, "personal", &memory_ref(&memory))
            .unwrap();
    assert_eq!(sources["results"][0]["events"][0]["id"], old.id);
    assert_eq!(
        result["records"][0]["sources"][0]["role"],
        sources["results"][0]["events"][0]["role"]
    );
}

#[test]
fn byte_budget_is_bounded_fair_and_accounts_for_every_selected_ref() {
    let f = setup();
    let mut originals = BTreeMap::new();
    for i in 0..8 {
        let text = format!("合成条目{i}：{}", "多字节🦉\"引用\"\n".repeat(1200));
        let event = capture(&f, input(&format!("会话{i}"), "1", &text));
        originals.insert(event_ref(&event), text);
    }
    let refs = originals.keys().cloned().collect::<Vec<_>>();
    for budget in [8000, 16000, 32768] {
        let result = prepare(&f, &refs, "合成预算测试", budget).unwrap();
        let bytes = serde_json::to_vec(&result).unwrap().len();
        assert!(bytes <= budget, "{bytes} > {budget}");
        assert_eq!(result["estimated_tokens"], bytes);
        assert_eq!(result["budget_unit"], "conservative_utf8_bytes");
        assert_eq!(result["partial"], true);
        assert_eq!(result["truncated"], true);
        assert!(result["text"]
            .as_str()
            .unwrap()
            .starts_with(result["stable_prefix"].as_str().unwrap()));
        let mut accounted = Vec::new();
        for row in result["records"].as_array().unwrap() {
            let reference = row["ref"].as_str().unwrap();
            let excerpt = row["text"].as_str().unwrap();
            assert!(originals[reference].starts_with(excerpt));
            assert!(excerpt.len() >= 120);
            assert_eq!(row["truncated"], true);
            assert_eq!(row["source_refs"], json!([reference]));
            accounted.push(reference.to_owned());
        }
        for pending in result["pending_refs"].as_array().unwrap() {
            let reference = pending.as_str().unwrap();
            assert!(result["text"].as_str().unwrap().contains(reference));
            accounted.push(reference.to_owned());
        }
        accounted.sort();
        assert_eq!(accounted, refs);
        if budget == 32768 {
            assert_eq!(result["included_count"], 8);
        }
    }
}

#[test]
fn oversized_source_metadata_is_explicitly_pending_without_truncating_other_evidence() {
    let f = setup();
    let mut huge_input = input("长会话", "1", "合成长出处正文");
    huge_input["session_id"] = json!("huge-session".repeat(2000));
    let huge = capture(&f, huge_input);
    let ordinary = capture(&f, input("普通", "2", "合成可带入原话"));
    let result = prepare(&f, &[event_ref(&huge), event_ref(&ordinary)], "", 32768).unwrap();
    assert_eq!(result["included_count"], 1);
    assert_eq!(result["pending_refs"], json!([event_ref(&huge)]));
    assert_eq!(result["records"][0]["text"], ordinary.data.text());
    assert_eq!(result["records"][0]["truncated"], false);
    assert_eq!(result["partial"], true);
    assert!(serde_json::to_vec(&result).unwrap().len() <= 32768);
}

fn snapshot(root: &Path, dir: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
    for item in fs::read_dir(dir).unwrap() {
        let path = item.unwrap().path();
        if path.is_dir() {
            snapshot(root, &path, files);
        } else {
            files.insert(
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                fs::read(path).unwrap(),
            );
        }
    }
}

#[test]
fn prepare_repeat_cancel_and_invalid_selection_do_not_write_canonical_files() {
    let mut f = setup();
    let event = capture(&f, input("只读", "1", "合成只读原话"));
    let memory = add(&f, &event, "合成只读记忆", json!({}));
    let mut before = BTreeMap::new();
    snapshot(f.vault.root(), f.vault.root(), &mut before);
    let refs = [event_ref(&event), memory_ref(&memory)];
    let a = prepare(&f, &refs, "合成预览", 32768).unwrap();
    let b = prepare(&f, &refs, "合成预览", 32768).unwrap();
    assert_eq!(a, b);
    f.ui.cancel_previews(&f.info.session_id).unwrap();
    assert!(prepare(&f, &["event:invalid".into()], "", 32768).is_err());
    let mut after = BTreeMap::new();
    snapshot(f.vault.root(), f.vault.root(), &mut after);
    assert_eq!(before, after);
    f.ui.close_vault();
    assert!(prepare(&f, &refs, "", 32768).is_err());
}
