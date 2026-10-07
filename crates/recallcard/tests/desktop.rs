//! 原生桌面边界测试；所有资料、对话、标识和密钥形状均为合成数据。
use recallcard::{
    desktop::{DesktopSession, VaultInfo},
    dream::DreamJob,
    Event, Memory, Vault,
};
use serde_json::{json, Value};
use std::{fs, path::Path};
use tempfile::{tempdir, TempDir};

fn session() -> (TempDir, DesktopSession, VaultInfo) {
    let dir = tempdir().unwrap();
    let mut session = DesktopSession::default();
    let info = session
        .select_vault(&dir.path().join("合成桌面资料库"), true)
        .unwrap();
    (dir, session, info)
}

fn event_input(id: &str, content: &str, scope: &str) -> Value {
    json!({
        "role":"user", "origin":"user_input", "scope":scope, "content":content,
        "source":{"platform":"desktop-test","conversation_id":"synthetic-session","message_id":id}
    })
}

fn capture(vault: &Vault, id: &str, scope: &str) -> Event {
    vault
        .capture(serde_json::from_value(event_input(id, "合成偏好：使用中文", scope)).unwrap())
        .unwrap()
}

fn memory(vault: &Vault, event: &Event) -> Memory {
    vault
        .add_memory(
            serde_json::from_value(json!({
                "content":"合成旧偏好", "source_refs":[event.id],
                "evidence":"user_explicit", "scope":event.data.scope
            }))
            .unwrap(),
        )
        .unwrap()
}

fn write_result(path: &Path, job: &DreamJob, proposals: Vec<Value>) {
    fs::write(
        path,
        serde_json::to_vec(&json!({
            "schema":"recallcard.dream-result/1", "job_id":job.job_id,
            "input_hash":job.input_hash, "proposals":proposals
        }))
        .unwrap(),
    )
    .unwrap();
}

fn add(event: &Event) -> Value {
    json!({
        "operation":"add", "scope":event.data.scope, "content":"合成新偏好：简洁中文",
        "source_refs":[format!("event:{}",event.id)], "evidence":"user_explicit"
    })
}

#[test]
fn import_preview_is_read_only_redacted_and_confirmed_exactly_once() {
    let (dir, mut session, info) = session();
    assert_eq!(info.scopes, vec!["personal"]);
    assert_eq!(info.health["ok"], true);
    let vault = Vault::open(Path::new(&info.root)).unwrap();
    let path = dir.path().join("合成导入.jsonl");
    let input = event_input(
        "preview",
        "合成偏好：中文。password=synthetic-not-a-real-secret",
        "personal",
    );
    fs::write(&path, input.to_string()).unwrap();
    let preview = session
        .preview_import(&info.session_id, "manual-jsonl", &path, "personal")
        .unwrap();
    assert_eq!(preview.event_count, 1);
    assert_eq!(preview.redacted_event_count, 1);
    assert!(preview.samples[0].content.contains("[REDACTED]"));
    assert!(!serde_json::to_string(&preview)
        .unwrap()
        .contains("synthetic-not-a-real-secret"));
    assert!(vault.events().unwrap().is_empty());
    assert!(vault.memories().unwrap().is_empty());
    let result = session
        .confirm_import(&info.session_id, &preview.preview_id)
        .unwrap();
    assert_eq!(result["events_added"], 1);
    assert!(session
        .confirm_import(&info.session_id, &preview.preview_id)
        .is_err());
    let reference = result["refs"][0].as_str().unwrap();
    let record = session
        .read(&info.session_id, "personal", reference)
        .unwrap();
    assert!(record["results"][0]["record"]["content"]
        .as_str()
        .unwrap()
        .contains("[REDACTED]"));
    let again = session
        .preview_import(&info.session_id, "manual-jsonl", &path, "personal")
        .unwrap();
    assert_eq!(
        session
            .confirm_import(&info.session_id, &again.preview_id)
            .unwrap()["events_added"],
        0
    );
    assert_eq!(session.status(&info.session_id).unwrap().event_count, 1);
}

#[test]
fn import_file_edits_and_replacements_invalidate_approval() {
    let (dir, mut session, info) = session();
    let path = dir.path().join("import.jsonl");
    let bytes = event_input("hash", "预览时的原文", "personal").to_string();
    fs::write(&path, &bytes).unwrap();
    let preview = session
        .preview_import(&info.session_id, "manual-jsonl", &path, "personal")
        .unwrap();
    fs::write(
        &path,
        event_input("hash", "确认前被换掉的内容", "personal").to_string(),
    )
    .unwrap();
    assert!(session
        .confirm_import(&info.session_id, &preview.preview_id)
        .unwrap_err()
        .contains("文件已改变"));
    assert_eq!(session.status(&info.session_id).unwrap().event_count, 0);

    fs::write(&path, &bytes).unwrap();
    let preview = session
        .preview_import(&info.session_id, "manual-jsonl", &path, "personal")
        .unwrap();
    // 即使新文件字节相同，也必须重新选择；不能把旧批准转移到替代文件。
    fs::rename(&path, dir.path().join("original.jsonl")).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert!(session
        .confirm_import(&info.session_id, &preview.preview_id)
        .is_err());
    assert_eq!(session.status(&info.session_id).unwrap().event_count, 0);
}

#[test]
fn selecting_or_failing_to_select_another_vault_revokes_old_commands() {
    let (dir, mut session, first) = session();
    let path = dir.path().join("import.jsonl");
    fs::write(
        &path,
        event_input("switch", "合成原文", "personal").to_string(),
    )
    .unwrap();
    let preview = session
        .preview_import(&first.session_id, "manual-jsonl", &path, "personal")
        .unwrap();
    let second = session
        .select_vault(&dir.path().join("second"), true)
        .unwrap();
    assert_ne!(first.session_id, second.session_id);
    assert!(session.status(&first.session_id).is_err());
    assert!(session
        .confirm_import(&first.session_id, &preview.preview_id)
        .is_err());
    assert!(session
        .confirm_import(&second.session_id, &preview.preview_id)
        .is_err());
    assert_eq!(session.status(&second.session_id).unwrap().event_count, 0);
    assert!(session
        .select_vault(&dir.path().join("does-not-exist"), false)
        .is_err());
    assert!(session.status(&second.session_id).is_err());
}

#[test]
fn replacing_vault_directory_invalidates_session_even_at_same_path() {
    let (dir, session, info) = session();
    fs::rename(&info.root, dir.path().join("old-vault")).unwrap();
    Vault::init(Path::new(&info.root)).unwrap();
    assert!(session.status(&info.session_id).is_err());
    assert!(session.browse(&info.session_id, "personal", "all").is_err());
}

#[test]
fn every_read_and_source_path_filters_scope_and_suppression() {
    let (_dir, session, info) = session();
    let vault = Vault::open(Path::new(&info.root)).unwrap();
    let private = capture(&vault, "private", "personal");
    let project = capture(&vault, "project", "project:demo");
    let private_memory = memory(&vault, &private);
    let reference = format!("memory:{}@{}", private_memory.id, private_memory.revision);
    let browse = session
        .browse(&info.session_id, "project:demo", "all")
        .unwrap();
    assert_eq!(browse["results"].as_array().unwrap().len(), 1);
    assert_eq!(browse["results"][0]["ref"], format!("event:{}", project.id));
    let search = session
        .search(&info.session_id, "project:demo", "中文", "all")
        .unwrap();
    assert!(!search.to_string().contains(&private.id));
    assert!(session
        .read(&info.session_id, "project:demo", &reference)
        .is_err());
    assert!(session
        .sources(&info.session_id, "project:demo", &reference)
        .is_err());
    assert!(session.browse(&info.session_id, "", "all").is_err());
    assert!(session.browse(&info.session_id, "*", "all").is_err());
    let sources = session
        .sources(&info.session_id, "personal", &reference)
        .unwrap();
    assert_eq!(sources["results"][0]["events"][0]["id"], private.id);
    vault.suppress(&private.id, "合成抑制".into()).unwrap();
    assert!(session
        .sources(&info.session_id, "personal", &reference)
        .is_err());
    assert_eq!(
        session.browse(&info.session_id, "personal", "all").unwrap()["total"],
        0
    );
}

#[test]
fn invalid_import_never_echoes_private_parser_data_or_writes() {
    let (dir, mut session, info) = session();
    let path = dir.path().join("invalid.jsonl");
    let mut invalid = event_input("invalid", "合法内容", "personal");
    invalid["synthetic-private-secret"] = json!("synthetic-secret-value");
    fs::write(&path, invalid.to_string()).unwrap();
    let error = session
        .preview_import(&info.session_id, "manual-jsonl", &path, "personal")
        .unwrap_err();
    assert!(!error.contains("synthetic"));
    assert!(!error.contains(&dir.path().display().to_string()));
    fs::write(
        &path,
        event_input("scope", "原文", "project:other").to_string(),
    )
    .unwrap();
    assert!(session
        .preview_import(&info.session_id, "manual-jsonl", &path, "personal")
        .is_err());
    assert_eq!(session.status(&info.session_id).unwrap().event_count, 0);
}

#[test]
fn import_read_is_bounded_and_all_adapters_use_preview() {
    let (dir, mut session, info) = session();
    let path = dir.path().join("large.jsonl");
    fs::File::create(&path)
        .unwrap()
        .set_len(16 * 1024 * 1024 + 1)
        .unwrap();
    assert!(session
        .preview_import(&info.session_id, "manual-jsonl", &path, "personal")
        .unwrap_err()
        .contains("16 MiB"));
    assert!(session
        .preview_import(&info.session_id, "manual-jsonl", dir.path(), "personal")
        .is_err());
    fs::write(
        &path,
        json!({
            "type":"user", "uuid":"adapter-user", "sessionId":"adapter-session",
            "message":{"content":"合成 Claude Code 内容"}
        })
        .to_string(),
    )
    .unwrap();
    let claude = session
        .preview_import(&info.session_id, "claude-code", &path, "personal")
        .unwrap();
    assert_eq!(claude.event_count, 1);
    session
        .confirm_import(&info.session_id, &claude.preview_id)
        .unwrap();
    fs::write(
        &path,
        json!([{
            "id":"adapter-chatgpt", "current_node":"node", "mapping":{
                "node":{"parent":null,"message":{
                    "id":"chatgpt-user", "author":{"role":"user"},
                    "content":{"content_type":"text","parts":["合成 ChatGPT 内容"]}
                }}
            }
        }])
        .to_string(),
    )
    .unwrap();
    let chatgpt = session
        .preview_import(&info.session_id, "chatgpt-export", &path, "personal")
        .unwrap();
    assert_eq!(chatgpt.event_count, 1);
    session
        .confirm_import(&info.session_id, &chatgpt.preview_id)
        .unwrap();
    assert_eq!(session.status(&info.session_id).unwrap().event_count, 2);
}

#[test]
fn cancelling_or_replacing_a_preview_revokes_its_confirmation() {
    let (dir, mut session, info) = session();
    let path = dir.path().join("preview.jsonl");
    fs::write(
        &path,
        event_input("preview", "合成原文", "personal").to_string(),
    )
    .unwrap();
    let first = session
        .preview_import(&info.session_id, "manual-jsonl", &path, "personal")
        .unwrap();
    let second = session
        .preview_import(&info.session_id, "manual-jsonl", &path, "personal")
        .unwrap();
    assert!(session
        .confirm_import(&info.session_id, &first.preview_id)
        .is_err());
    session.cancel_previews(&info.session_id).unwrap();
    assert!(session
        .confirm_import(&info.session_id, &second.preview_id)
        .is_err());
    assert_eq!(session.status(&info.session_id).unwrap().event_count, 0);
}

#[test]
fn dream_review_does_not_publish_and_apply_preserves_evidence() {
    let (dir, mut session, info) = session();
    let vault = Vault::open(Path::new(&info.root)).unwrap();
    let source = capture(&vault, "dream-add", "personal");
    let job = session
        .export_dream(
            &info.session_id,
            "personal",
            std::slice::from_ref(&source.id),
            &[],
        )
        .unwrap();
    let path = dir.path().join("result.json");
    write_result(&path, &job, vec![add(&source)]);
    let preview = session
        .review_dream(&info.session_id, &path, "personal")
        .unwrap();
    assert!(preview.review.can_apply);
    assert!(vault.memories().unwrap().is_empty());
    let receipt = session
        .apply_dream(&info.session_id, &preview.preview_id, false)
        .unwrap();
    let saved = vault.memory(&receipt.changes[0].id).unwrap();
    assert_eq!(saved.data.source_refs, vec![source.id]);
    assert_eq!(saved.data.authority, "dream");
    assert!(!saved.data.protected);
    assert_eq!(vault.memories().unwrap().len(), 1);
    assert!(session
        .apply_dream(&info.session_id, &preview.preview_id, false)
        .is_err());
    let repeated = session
        .review_dream(&info.session_id, &path, "personal")
        .unwrap();
    assert!(repeated.review.already_applied);
    let receipt = session
        .apply_dream(&info.session_id, &repeated.preview_id, false)
        .unwrap();
    assert!(receipt.already_applied);
    assert_eq!(vault.memories().unwrap().len(), 1);
}

#[test]
fn dream_protected_update_requires_separate_approval() {
    let (dir, mut session, info) = session();
    let vault = Vault::open(Path::new(&info.root)).unwrap();
    let source = capture(&vault, "protected", "personal");
    let old = memory(&vault, &source);
    let job = session
        .export_dream(
            &info.session_id,
            "personal",
            std::slice::from_ref(&source.id),
            std::slice::from_ref(&old.id),
        )
        .unwrap();
    let mut proposal = add(&source);
    proposal["operation"] = json!("update");
    proposal["target_ref"] = json!(format!("memory:{}@{}", old.id, old.revision));
    proposal["expected_revision"] = json!(old.revision);
    let path = dir.path().join("protected.json");
    write_result(&path, &job, vec![proposal]);
    let preview = session
        .review_dream(&info.session_id, &path, "personal")
        .unwrap();
    assert!(preview.review.requires_protected_approval);
    assert!(session
        .apply_dream(&info.session_id, &preview.preview_id, false)
        .unwrap_err()
        .contains("受保护"));
    assert_eq!(vault.memory(&old.id).unwrap().revision, 1);
    session
        .apply_dream(&info.session_id, &preview.preview_id, true)
        .unwrap();
    assert_eq!(vault.memory(&old.id).unwrap().revision, 2);
}

#[test]
fn dream_changed_file_and_changed_memory_reject_old_review() {
    let (dir, mut session, info) = session();
    let vault = Vault::open(Path::new(&info.root)).unwrap();
    let source = capture(&vault, "dream-stale", "personal");
    let old = memory(&vault, &source);
    let job = session
        .export_dream(
            &info.session_id,
            "personal",
            std::slice::from_ref(&source.id),
            std::slice::from_ref(&old.id),
        )
        .unwrap();
    let path = dir.path().join("stale.json");
    write_result(&path, &job, vec![add(&source)]);
    let preview = session
        .review_dream(&info.session_id, &path, "personal")
        .unwrap();
    let mut changed = add(&source);
    changed["content"] = json!("用户从未审查的替换内容");
    write_result(&path, &job, vec![changed]);
    assert!(session
        .apply_dream(&info.session_id, &preview.preview_id, false)
        .unwrap_err()
        .contains("文件已改变"));
    write_result(&path, &job, vec![add(&source)]);
    let preview = session
        .review_dream(&info.session_id, &path, "personal")
        .unwrap();
    let mut input = old.data.clone();
    input.content = "并发更新的合成旧记忆".into();
    vault.update_memory(&old.id, old.revision, input).unwrap();
    assert!(session
        .apply_dream(&info.session_id, &preview.preview_id, false)
        .unwrap_err()
        .contains("已变化"));
    assert_eq!(vault.memories().unwrap().len(), 1);
    assert_eq!(vault.memory(&old.id).unwrap().revision, 2);
}

#[test]
fn dream_scope_conflict_and_vault_switch_are_fail_closed() {
    let (dir, mut session, info) = session();
    let vault = Vault::open(Path::new(&info.root)).unwrap();
    let source = capture(&vault, "dream-scope", "project:demo");
    assert!(session
        .export_dream(
            &info.session_id,
            "personal",
            std::slice::from_ref(&source.id),
            &[]
        )
        .is_err());
    let job = session
        .export_dream(
            &info.session_id,
            "project:demo",
            std::slice::from_ref(&source.id),
            &[],
        )
        .unwrap();
    let path = dir.path().join("scope.json");
    write_result(&path, &job, vec![add(&source)]);
    assert!(session
        .review_dream(&info.session_id, &path, "personal")
        .is_err());
    let mut conflict = add(&source);
    conflict["operation"] = json!("conflict");
    write_result(&path, &job, vec![conflict]);
    let review = session
        .review_dream(&info.session_id, &path, "project:demo")
        .unwrap();
    assert!(!review.review.can_apply);
    assert!(session
        .apply_dream(&info.session_id, &review.preview_id, true)
        .unwrap_err()
        .contains("冲突"));
    write_result(&path, &job, vec![add(&source)]);
    let preview = session
        .review_dream(&info.session_id, &path, "project:demo")
        .unwrap();
    let second = session
        .select_vault(&dir.path().join("second"), true)
        .unwrap();
    assert!(session
        .apply_dream(&info.session_id, &preview.preview_id, true)
        .is_err());
    assert!(session
        .apply_dream(&second.session_id, &preview.preview_id, true)
        .is_err());
    assert!(vault.memories().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn selected_import_file_cannot_be_a_symlink_or_turn_into_one() {
    use std::os::unix::fs::symlink;
    let (dir, mut session, info) = session();
    let target = dir.path().join("target.jsonl");
    let link = dir.path().join("link.jsonl");
    fs::write(
        &target,
        event_input("link", "合成内容", "personal").to_string(),
    )
    .unwrap();
    symlink(&target, &link).unwrap();
    assert!(session
        .preview_import(&info.session_id, "manual-jsonl", &link, "personal")
        .is_err());
    let parent_link = dir.path().join("arbitrary-parent-link");
    symlink(dir.path(), &parent_link).unwrap();
    assert!(session
        .preview_import(
            &info.session_id,
            "manual-jsonl",
            &parent_link.join("target.jsonl"),
            "personal",
        )
        .is_err());
    let preview = session
        .preview_import(&info.session_id, "manual-jsonl", &target, "personal")
        .unwrap();
    let moved = dir.path().join("moved.jsonl");
    fs::rename(&target, &moved).unwrap();
    symlink(&moved, &target).unwrap();
    assert!(session
        .confirm_import(&info.session_id, &preview.preview_id)
        .is_err());
    assert_eq!(session.status(&info.session_id).unwrap().event_count, 0);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_system_tmp_alias_allows_native_file_selection() {
    let directory = tempfile::Builder::new()
        .prefix("recallcard-desktop-system-alias-")
        .tempdir_in("/tmp")
        .unwrap();
    let mut session = DesktopSession::default();
    let info = session
        .select_vault(&directory.path().join("vault"), true)
        .unwrap();
    let path = directory.path().join("selected.jsonl");
    assert!(path.starts_with("/tmp"));
    fs::write(
        &path,
        event_input("macos-native-alias", "合成系统目录内容", "personal").to_string(),
    )
    .unwrap();
    let preview = session
        .preview_import(&info.session_id, "manual-jsonl", &path, "personal")
        .unwrap();
    assert_eq!(preview.event_count, 1);
    assert_eq!(
        session
            .confirm_import(&info.session_id, &preview.preview_id)
            .unwrap()["events_added"],
        1
    );
}
