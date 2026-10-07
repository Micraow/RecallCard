//! 合成数据验证原生多文件入口、确认边界与崩溃恢复；不使用真实账号或网络。
use recallcard::{
    desktop::{DesktopSession, ImportJobState, ImportJobStatus, VaultInfo},
    Vault,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use tempfile::{tempdir, TempDir};
use zip::{write::SimpleFileOptions, ZipWriter};

fn setup() -> (TempDir, DesktopSession, VaultInfo) {
    let directory = tempdir().unwrap();
    let mut session = DesktopSession::default();
    let info = session
        .select_vault(&directory.path().join("vault"), true)
        .unwrap();
    (directory, session, info)
}

fn deepseek(id: &str) -> Value {
    json!({"id":id,"title":"合成 DeepSeek 会话","inserted_at":"2026-01-02T00:00:00Z","updated_at":"2026-01-02T00:01:00Z",
    "mapping": {
        "q":{"id":"q","parent":null,"children":["a"],"message":{"files":[],"model":null,"fragments":[{"type":"REQUEST","content":"password=synthetic-password-for-import"}]}},
        "a":{"id":"a","parent":"q","children":[],"message":{"files":[],"model":"deepseek-chat","fragments":[{"type":"THINK","content":"private-hidden-synthetic-fragment"},{"type":"RESPONSE","content":"合成可见回答"}]}}
    }})
}

fn chatgpt(id: &str, count: usize) -> Value {
    let mut mapping = serde_json::Map::new();
    for n in 0..count {
        mapping.insert(format!("n-{n}"), json!({"parent":if n == 0 { None } else { Some(format!("n-{}",n-1)) },
            "message":{"id":format!("m-{n}"),"author":{"role":if n%2 == 0 {"user"}else{"assistant"}},"content":{"content_type":"text","parts":[format!("合成消息 {n}")]}}}));
    }
    json!({"id":id,"title":"合成 ChatGPT 会话","current_node":format!("n-{}",count-1),"mapping":mapping})
}

fn json_file(directory: &Path, name: &str, value: &Value) -> PathBuf {
    let path = directory.join(name);
    fs::write(&path, value.to_string()).unwrap();
    path
}

fn zip_file(directory: &Path, value: &Value) -> PathBuf {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file("conversations.json", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(value.to_string().as_bytes()).unwrap();
    writer
        .start_file("copy.md", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"not an extra conversation").unwrap();
    let path = directory.join("synthetic.zip");
    fs::write(&path, writer.finish().unwrap().into_inner()).unwrap();
    path
}

fn wait(session: &DesktopSession, info: &VaultInfo, id: &str) -> ImportJobStatus {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let status = session
            .import_job_status(&info.session_id, id, "personal")
            .unwrap();
        if !matches!(
            status.state,
            ImportJobState::Running | ImportJobState::Cancelling
        ) {
            return status;
        }
        assert!(Instant::now() < deadline, "导入任务未在测试期限内结束");
        thread::sleep(Duration::from_millis(5));
    }
}

fn job_dir(info: &VaultInfo, id: &str) -> PathBuf {
    Vault::open(Path::new(&info.root))
        .unwrap()
        .state_dir()
        .unwrap()
        .join("import-jobs")
        .join(id)
}

#[test]
fn mixed_files_one_confirmation_imports_all_and_stages_only_sanitized_input() {
    let (directory, mut session, info) = setup();
    let paths = vec![
        json_file(directory.path(), "deepseek.json", &deepseek("same-id")),
        zip_file(directory.path(), &json!([chatgpt("same-id", 2)])),
    ];
    let preview = session
        .prepare_import_job(&info.session_id, "auto", &paths, "personal")
        .unwrap();
    assert_eq!(preview.files.len(), 2);
    assert_eq!(preview.event_count, 4);
    assert_eq!(preview.conversations.len(), 2);
    assert_eq!(preview.coverage.deepseek.hidden_fragments_skipped, 1);
    assert_eq!(preview.redacted_event_count, 1);
    assert!(preview
        .samples
        .iter()
        .any(|s| s.content.contains("[REDACTED]")));
    assert_eq!(session.status(&info.session_id).unwrap().event_count, 0);
    let job = session
        .start_import_job(&info.session_id, &preview.preview_id, "personal")
        .unwrap();
    assert!(session
        .start_import_job(&info.session_id, &preview.preview_id, "personal")
        .is_err());
    let finished = wait(&session, &info, &job.job_id);
    assert_eq!(finished.state, ImportJobState::Completed);
    assert_eq!(finished.events_processed, 4);
    assert_eq!(finished.events_added, 4);
    assert_eq!(finished.events_duplicates, 0);
    assert!(!finished.can_resume);
    let vault = Vault::open(Path::new(&info.root)).unwrap();
    assert_eq!(vault.events().unwrap().len(), 4);
    assert!(vault.memories().unwrap().is_empty());
    let staging = job_dir(&info, &job.job_id);
    assert!(!staging.starts_with(&info.root));
    let snapshot = fs::read_to_string(staging.join("snapshot.json")).unwrap();
    assert!(!snapshot.contains("synthetic-password-for-import"));
    assert!(!snapshot.contains("private-hidden-synthetic-fragment"));
    assert!(!serde_json::to_string(&finished)
        .unwrap()
        .contains("合成可见回答"));
    let preview = session
        .prepare_import_job(&info.session_id, "auto", &paths, "personal")
        .unwrap();
    let repeated = session
        .start_import_job(&info.session_id, &preview.preview_id, "personal")
        .unwrap();
    let repeated = wait(&session, &info, &repeated.job_id);
    assert_eq!(repeated.events_added, 0);
    assert_eq!(repeated.events_duplicates, 4);
    assert!(session
        .resume_import_job(&info.session_id, &repeated.job_id, "personal")
        .is_err());
}

#[test]
fn any_changed_source_invalidates_entire_confirmation_without_writes() {
    let (directory, mut session, info) = setup();
    let one = json_file(directory.path(), "one.json", &deepseek("one"));
    let two = json_file(directory.path(), "two.json", &deepseek("two"));
    let original = fs::read(&two).unwrap();
    let preview = session
        .prepare_import_job(&info.session_id, "auto", &[one, two.clone()], "personal")
        .unwrap();
    fs::write(&two, b"changed").unwrap();
    assert!(session
        .start_import_job(&info.session_id, &preview.preview_id, "personal")
        .is_err());
    fs::write(&two, original).unwrap();
    assert!(session
        .start_import_job(&info.session_id, &preview.preview_id, "personal")
        .is_err());
    assert_eq!(session.status(&info.session_id).unwrap().event_count, 0);
    assert!(session
        .list_import_jobs(&info.session_id, "personal")
        .unwrap()
        .is_empty());
}

#[test]
fn malformed_later_file_does_not_save_earlier_valid_events() {
    let (directory, mut session, info) = setup();
    let valid = json_file(directory.path(), "valid.json", &deepseek("one"));
    let mut malformed = deepseek("bad");
    malformed["mapping"]["q"]["children"] = json!(["missing-node"]);
    let invalid = json_file(directory.path(), "invalid.json", &malformed);
    assert!(session
        .prepare_import_job(&info.session_id, "auto", &[valid, invalid], "personal")
        .is_err());
    assert_eq!(session.status(&info.session_id).unwrap().event_count, 0);
}

#[test]
fn stale_session_scope_and_replaced_vault_cannot_authorize_a_job() {
    let (directory, mut session, info) = setup();
    let path = json_file(directory.path(), "one.json", &deepseek("one"));
    let preview = session
        .prepare_import_job(&info.session_id, "auto", &[path], "personal")
        .unwrap();
    assert!(session
        .start_import_job(&info.session_id, &preview.preview_id, "work")
        .is_err());
    let job = session
        .start_import_job(&info.session_id, &preview.preview_id, "personal")
        .unwrap();
    wait(&session, &info, &job.job_id);
    assert!(session
        .import_job_status(&info.session_id, &job.job_id, "work")
        .is_err());
    assert!(session
        .cancel_import_job(&info.session_id, &job.job_id, "work")
        .is_err());
    assert!(session
        .list_import_jobs(&info.session_id, "work")
        .unwrap()
        .is_empty());
    let new_info = session.select_vault(Path::new(&info.root), false).unwrap();
    assert!(session
        .import_job_status(&info.session_id, &job.job_id, "personal")
        .is_err());
    assert_eq!(
        session
            .import_job_status(&new_info.session_id, &job.job_id, "personal")
            .unwrap()
            .state,
        ImportJobState::Completed
    );
    session.close_vault();
    fs::rename(&info.root, directory.path().join("original-vault")).unwrap();
    let replacement = session.select_vault(Path::new(&info.root), true).unwrap();
    assert!(session
        .import_job_status(&replacement.session_id, &job.job_id, "personal")
        .is_err());
    assert!(session
        .resume_import_job(&replacement.session_id, &job.job_id, "personal")
        .is_err());
}

#[test]
fn interrupted_checkpoint_replays_frozen_snapshot_without_duplicates_or_original_files() {
    let (directory, mut session, info) = setup();
    let path = json_file(directory.path(), "one.json", &chatgpt("one", 8));
    let preview = session
        .prepare_import_job(
            &info.session_id,
            "auto",
            std::slice::from_ref(&path),
            "personal",
        )
        .unwrap();
    let job = session
        .start_import_job(&info.session_id, &preview.preview_id, "personal")
        .unwrap();
    assert_eq!(wait(&session, &info, &job.job_id).events_added, 8);
    fs::remove_file(path).unwrap();
    // 模拟 Event 都已 durable，但进度回到较早的崩溃检查点且进程 lease 已释放。
    let path = job_dir(&info, &job.job_id).join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["status"]["state"] = json!("running");
    manifest["status"]["events_processed"] = json!(2);
    manifest["status"]["events_added"] = json!(2);
    manifest["status"]["events_duplicates"] = json!(0);
    fs::write(&path, manifest.to_string()).unwrap();
    let interrupted = session
        .import_job_status(&info.session_id, &job.job_id, "personal")
        .unwrap();
    assert_eq!(interrupted.state, ImportJobState::Interrupted);
    assert!(interrupted.can_resume);
    session
        .resume_import_job(&info.session_id, &job.job_id, "personal")
        .unwrap();
    let completed = wait(&session, &info, &job.job_id);
    assert_eq!(completed.state, ImportJobState::Completed);
    assert_eq!(completed.events_added, 8);
    assert_eq!(completed.events_duplicates, 0);
    assert_eq!(
        Vault::open(Path::new(&info.root))
            .unwrap()
            .events()
            .unwrap()
            .len(),
        8
    );
}

#[test]
fn pause_is_responsive_preserves_prefix_and_resume_completes() {
    let (directory, mut session, info) = setup();
    let path = json_file(directory.path(), "many.json", &chatgpt("many", 1500));
    let preview = session
        .prepare_import_job(&info.session_id, "auto", &[path], "personal")
        .unwrap();
    let job = session
        .start_import_job(&info.session_id, &preview.preview_id, "personal")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let status = session
            .import_job_status(&info.session_id, &job.job_id, "personal")
            .unwrap();
        if status.events_processed > 0 {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    session
        .cancel_import_job(&info.session_id, &job.job_id, "personal")
        .unwrap();
    let paused = wait(&session, &info, &job.job_id);
    assert_eq!(paused.state, ImportJobState::Cancelled);
    assert!(paused.events_processed > 0 && paused.events_processed < 1500);
    assert_eq!(
        Vault::open(Path::new(&info.root))
            .unwrap()
            .events()
            .unwrap()
            .len(),
        paused.events_added
    );
    session
        .resume_import_job(&info.session_id, &job.job_id, "personal")
        .unwrap();
    let complete = wait(&session, &info, &job.job_id);
    assert_eq!(complete.state, ImportJobState::Completed);
    assert_eq!(complete.events_added, 1500);
    assert_eq!(complete.events_duplicates, 0);
}

#[test]
fn second_instance_cannot_start_while_vault_import_lease_is_held() {
    let (directory, mut session, info) = setup();
    let mut second = DesktopSession::default();
    let second_info = second.select_vault(Path::new(&info.root), false).unwrap();
    let path = json_file(directory.path(), "one.json", &deepseek("one"));
    let first_preview = session
        .prepare_import_job(
            &info.session_id,
            "auto",
            std::slice::from_ref(&path),
            "personal",
        )
        .unwrap();
    let second_preview = second
        .prepare_import_job(&second_info.session_id, "auto", &[path], "personal")
        .unwrap();
    let state = Vault::open(Path::new(&info.root))
        .unwrap()
        .state_dir()
        .unwrap()
        .join("import-jobs");
    fs::create_dir_all(&state).unwrap();
    let lease = fs::File::create(state.join("active.lock")).unwrap();
    lease.lock().unwrap();
    assert!(session
        .start_import_job(&info.session_id, &first_preview.preview_id, "personal")
        .is_err());
    assert!(second
        .start_import_job(
            &second_info.session_id,
            &second_preview.preview_id,
            "personal"
        )
        .is_err());
    assert_eq!(session.status(&info.session_id).unwrap().event_count, 0);
    lease.unlock().unwrap();
}

#[test]
fn another_open_instance_can_pause_the_running_job_but_cannot_start_another() {
    let (directory, mut first, first_info) = setup();
    let mut second = DesktopSession::default();
    let second_info = second
        .select_vault(Path::new(&first_info.root), false)
        .unwrap();
    let path = json_file(directory.path(), "many.json", &chatgpt("many", 1500));
    let first_preview = first
        .prepare_import_job(
            &first_info.session_id,
            "auto",
            std::slice::from_ref(&path),
            "personal",
        )
        .unwrap();
    let second_preview = second
        .prepare_import_job(&second_info.session_id, "auto", &[path], "personal")
        .unwrap();
    let job = first
        .start_import_job(
            &first_info.session_id,
            &first_preview.preview_id,
            "personal",
        )
        .unwrap();
    assert!(second
        .start_import_job(
            &second_info.session_id,
            &second_preview.preview_id,
            "personal"
        )
        .is_err());
    second
        .cancel_import_job(&second_info.session_id, &job.job_id, "personal")
        .unwrap();
    let stopped = wait(&first, &first_info, &job.job_id);
    assert_eq!(stopped.state, ImportJobState::Cancelled);
    assert!(stopped.can_resume);
    assert!(stopped.events_processed < 1500);
}

#[test]
fn tampered_snapshot_and_symlink_inputs_are_rejected() {
    let (directory, mut session, info) = setup();
    let path = json_file(directory.path(), "one.json", &deepseek("one"));
    #[cfg(unix)]
    {
        let link = directory.path().join("link.json");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(session
            .prepare_import_job(&info.session_id, "auto", &[link], "personal")
            .is_err());
    }
    let preview = session
        .prepare_import_job(&info.session_id, "auto", &[path], "personal")
        .unwrap();
    let job = session
        .start_import_job(&info.session_id, &preview.preview_id, "personal")
        .unwrap();
    wait(&session, &info, &job.job_id);
    let directory = job_dir(&info, &job.job_id);
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    manifest["status"]["state"] = json!("running");
    fs::write(directory.join("manifest.json"), manifest.to_string()).unwrap();
    fs::write(directory.join("snapshot.json"), b"[]").unwrap();
    assert!(session
        .resume_import_job(&info.session_id, &job.job_id, "personal")
        .is_err());
    assert_eq!(
        Vault::open(Path::new(&info.root))
            .unwrap()
            .events()
            .unwrap()
            .len(),
        2
    );
}

fn completed_job(session: &mut DesktopSession, info: &VaultInfo, path: PathBuf) -> ImportJobStatus {
    let preview = session
        .prepare_import_job(&info.session_id, "auto", &[path], "personal")
        .unwrap();
    let started = session
        .start_import_job(&info.session_id, &preview.preview_id, "personal")
        .unwrap();
    let finished = wait(session, info, &started.job_id);
    assert_eq!(finished.state, ImportJobState::Completed);
    finished
}

#[test]
fn batch_results_use_current_records_and_keep_duplicate_only_imports_separate() {
    let (directory, mut session, info) = setup();
    let first_file = json_file(directory.path(), "first.json", &deepseek("first"));
    let first = completed_job(&mut session, &info, first_file.clone());
    let repeated = completed_job(&mut session, &info, first_file);
    assert_eq!(repeated.events_added, 0);
    assert_eq!(repeated.events_duplicates, 2);
    let unrelated = json_file(directory.path(), "unrelated.json", &chatgpt("unrelated", 2));
    completed_job(&mut session, &info, unrelated);
    let initial = session
        .import_job_conversations(&info.session_id, &first.job_id, "personal", 0)
        .unwrap();
    let result = session
        .import_job_conversations(&info.session_id, &repeated.job_id, "personal", 0)
        .unwrap();
    assert_eq!(result["total"], 1);
    assert_eq!(result["conversations"], initial["conversations"]);
    assert_eq!(result["conversations"][0]["platform"], "deepseek");
    assert_eq!(result["conversations"][0]["message_count"], 2);
    assert_eq!(result["status"]["events_added"], 0);
    assert!(!result.to_string().contains("合成 ChatGPT 会话"));
    assert!(session
        .import_job_conversations(&info.session_id, &first.job_id, "work", 0)
        .is_err());
    assert!(session
        .import_job_conversations("stale-session", &first.job_id, "personal", 0)
        .is_err());
    assert!(session
        .import_job_conversations(&info.session_id, "../../other", "personal", 0)
        .is_err());
}

#[test]
fn batch_results_survive_restart_without_original_files_and_remain_read_only() {
    let (directory, mut session, info) = setup();
    let path = json_file(directory.path(), "first.json", &deepseek("first"));
    let job = completed_job(&mut session, &info, path.clone());
    let result = session
        .import_job_conversations(&info.session_id, &job.job_id, "personal", 0)
        .unwrap();
    let manifest_before = fs::read(job_dir(&info, &job.job_id).join("manifest.json")).unwrap();
    let vault = Vault::open(Path::new(&info.root)).unwrap();
    let events_before = vault.events().unwrap();
    fs::remove_file(path).unwrap();
    drop(session);
    let mut restarted = DesktopSession::default();
    let next = restarted
        .select_vault(Path::new(&info.root), false)
        .unwrap();
    for _ in 0..3 {
        let restored = restarted
            .import_job_conversations(&next.session_id, &job.job_id, "personal", 0)
            .unwrap();
        assert_eq!(restored, result);
    }
    assert_eq!(vault.events().unwrap(), events_before);
    assert_eq!(
        fs::read(job_dir(&info, &job.job_id).join("manifest.json")).unwrap(),
        manifest_before
    );
}

#[test]
fn batch_results_exclude_forgotten_and_revised_away_versions() {
    let (directory, mut session, info) = setup();
    let path = json_file(directory.path(), "first.json", &chatgpt("first", 2));
    let job = completed_job(&mut session, &info, path);
    let vault = Vault::open(Path::new(&info.root)).unwrap();
    let events = vault.events().unwrap();
    vault
        .suppress(&events[0].id, "合成测试遗忘".into())
        .unwrap();
    let visible = session
        .import_job_conversations(&info.session_id, &job.job_id, "personal", 0)
        .unwrap();
    assert_eq!(visible["conversations"][0]["message_count"], 1);
    let mut revised = events[1].data.clone();
    revised.content = "本批以后的替代原文，不属于本批".into();
    revised.parts.clear();
    vault.capture(revised).unwrap();
    let hidden = session
        .import_job_conversations(&info.session_id, &job.job_id, "personal", 0)
        .unwrap();
    assert_eq!(hidden["total"], 0);
    assert_eq!(hidden["conversations"], json!([]));
    assert!(!hidden.to_string().contains("本批以后的替代原文"));
}

#[test]
fn partial_failed_results_include_only_verified_processed_prefix() {
    let (directory, mut session, info) = setup();
    let path = json_file(
        directory.path(),
        "many.json",
        &json!([chatgpt("first", 2), chatgpt("second", 2)]),
    );
    let job = completed_job(&mut session, &info, path);
    let manifest_path = job_dir(&info, &job.job_id).join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    // 合成崩溃检查点：实际文件有后续消息，界面仍只能声明持久检查点已核实的前缀。
    for state in ["failed", "cancelled", "running"] {
        manifest["status"]["state"] = json!(state);
        manifest["status"]["events_processed"] = json!(2);
        manifest["status"]["events_added"] = json!(2);
        fs::write(&manifest_path, manifest.to_string()).unwrap();
        let result = session
            .import_job_conversations(&info.session_id, &job.job_id, "personal", 0)
            .unwrap();
        assert_eq!(result["total"], 1);
        assert_eq!(result["conversations"][0]["message_count"], 2);
        assert_eq!(result["status"]["events_processed"], 2);
    }
    manifest["status"]["state"] = json!("failed");
    manifest["status"]["events_processed"] = json!(0);
    manifest["status"]["events_added"] = json!(0);
    fs::write(&manifest_path, manifest.to_string()).unwrap();
    let empty = session
        .import_job_conversations(&info.session_id, &job.job_id, "personal", 0)
        .unwrap();
    assert_eq!(empty["total"], 0);
}

#[test]
fn batch_results_page_without_unrelated_or_duplicate_rows_and_reject_tampered_snapshot() {
    let (directory, mut session, info) = setup();
    let data = Value::Array(
        (0..53)
            .map(|n| chatgpt(&format!("conversation-{n}"), 1))
            .collect(),
    );
    let path = json_file(directory.path(), "many.json", &data);
    let job = completed_job(&mut session, &info, path);
    let first = session
        .import_job_conversations(&info.session_id, &job.job_id, "personal", 0)
        .unwrap();
    let second = session
        .import_job_conversations(&info.session_id, &job.job_id, "personal", 50)
        .unwrap();
    assert_eq!(first["total"], 53);
    assert_eq!(first["next_offset"], 50);
    assert_eq!(first["conversations"].as_array().unwrap().len(), 50);
    assert_eq!(second["conversations"].as_array().unwrap().len(), 3);
    assert!(second["next_offset"].is_null());
    let refs: std::collections::BTreeSet<_> = first["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["conversations"].as_array().unwrap())
        .map(|row| row["session_ref"].as_str().unwrap())
        .collect();
    assert_eq!(refs.len(), 53);
    assert!(serde_json::to_vec(&first).unwrap().len() <= 32768);
    assert!(session
        .import_job_conversations(&info.session_id, &job.job_id, "personal", 100001)
        .is_err());
    fs::write(job_dir(&info, &job.job_id).join("snapshot.json"), "[]").unwrap();
    assert!(session
        .import_job_conversations(&info.session_id, &job.job_id, "personal", 0)
        .is_err());
}

#[test]
fn rapid_status_polling_does_not_misreport_completed_checkpoint_as_interrupted() {
    let (directory, mut session, info) = setup();
    let path = json_file(directory.path(), "polling.json", &chatgpt("polling", 128));
    let mut observer = DesktopSession::default();
    let observed_info = observer.select_vault(Path::new(&info.root), false).unwrap();
    for _ in 0..12 {
        let preview = session
            .prepare_import_job(
                &info.session_id,
                "auto",
                std::slice::from_ref(&path),
                "personal",
            )
            .unwrap();
        let started = session
            .start_import_job(&info.session_id, &preview.preview_id, "personal")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let current = session
                .import_job_status(&info.session_id, &started.job_id, "personal")
                .unwrap();
            let other = observer
                .import_job_status(&observed_info.session_id, &started.job_id, "personal")
                .unwrap();
            for status in [&current, &other] {
                assert!(
                    matches!(
                        status.state,
                        ImportJobState::Running | ImportJobState::Completed
                    ),
                    "健康导入不能因查询竞态变成 {:?}",
                    status.state
                );
            }
            if current.state == ImportJobState::Completed
                && other.state == ImportJobState::Completed
            {
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_micros(100));
        }
    }
}

#[test]
fn old_local_control_does_not_hide_a_different_instances_active_progress() {
    let (directory, mut session, info) = setup();
    let path = json_file(directory.path(), "other-run.json", &chatgpt("other-run", 4));
    let job = completed_job(&mut session, &info, path);
    let directory = job_dir(&info, &job.job_id);
    // 模拟另一实例已取得写入 lease 并建立新的恢复检查点。本实例仍保留旧 completed control。
    let lease = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.join("run.lock"))
        .unwrap();
    lease.lock().unwrap();
    let path = directory.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["status"]["state"] = json!("running");
    manifest["status"]["events_processed"] = json!(1);
    manifest["status"]["events_added"] = json!(1);
    manifest["status"]["message"] = json!("另一实例正在核对本批记录");
    fs::write(path, manifest.to_string()).unwrap();
    let status = session
        .import_job_status(&info.session_id, &job.job_id, "personal")
        .unwrap();
    assert_eq!(status.state, ImportJobState::Running);
    assert_eq!(status.events_processed, 1);
    assert_eq!(status.message, "另一实例正在核对本批记录");
    assert!(!status.can_resume);
    lease.unlock().unwrap();
}
