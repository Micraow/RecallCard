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
