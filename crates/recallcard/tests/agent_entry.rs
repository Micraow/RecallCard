//! 合成正本与真实 CLI 验证自动文件入口；不执行任何外部 Agent。
use recallcard::{
    application::{
        agent_entry, connection_setup,
        connections::{self, ConnectionGrant},
    },
    EventInput, MemoryInput, Vault,
};
use serde_json::{json, Value};
use std::process::Command;

fn source(v: &Vault, scope: &str, id: &str, text: &str) -> String {
    let input:EventInput=serde_json::from_value(json!({"occurred_at":"2026-10-01T00:00:00Z","role":"user","origin":"native","scope":scope,"content":text,"source":{"platform":"synthetic","conversation_id":format!("source-{id}"),"message_id":id}})).unwrap();
    v.capture(input).unwrap().id
}
fn connect(v: &Vault, scope: &str) -> connections::ConnectionEntry {
    connections::configure(
        v,
        ConnectionGrant {
            client_kind: "codex".into(),
            host_identity: "codex".into(),
            installation_id: None,
            platform: "codex".into(),
            recall_scopes: vec![scope.into()],
            capture_scopes: vec![],
            provider_disclosure: true,
            auto_capture: false,
            auto_recall: true,
        },
        None,
    )
    .unwrap()
}
#[test]
fn live_entry_filters_scope_sources_and_suppression_and_invalidates_before_writes() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let own = source(&v, "personal", "own", "可访问原话");
    let other = source(&v, "project:other", "secret", "OTHER_SCOPE_SECRET");
    let m:MemoryInput=serde_json::from_value(json!({"scope":"personal","content":"通用合成背景","source_refs":[own],"evidence":"user_explicit"})).unwrap();
    v.add_memory(m).unwrap();
    let entry = connect(&v, "personal");
    let a = agent_entry::refresh(&v, &entry.id, vec!["personal".into()], 4096).unwrap();
    let bytes = std::fs::read(&a.file_path).unwrap();
    assert!(bytes.len() <= 4096);
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("通用合成背景"));
    assert!(text.contains(&own));
    assert!(!text.contains(&other));
    assert!(!text.contains("OTHER_SCOPE_SECRET"));
    assert!(agent_entry::refresh(&v, &entry.id, vec!["project:other".into()], 4096).is_err());
    let reopened = Vault::open_existing(v.root()).unwrap();
    let b = agent_entry::refresh(&reopened, &entry.id, vec!["personal".into()], 4096).unwrap();
    assert_eq!(a.snapshot, b.snapshot);
    source(&v, "personal", "new", "刚补充的新来源");
    assert!(!a.file_path.exists(), "正本写入不能留下旧文件入口");
    let c = agent_entry::refresh(&v, &entry.id, vec!["personal".into()], 4096).unwrap();
    assert_ne!(b.snapshot, c.snapshot);
    v.suppress(&own, "合成撤回".into()).unwrap();
    assert!(!a.file_path.exists());
    let suppressed = agent_entry::refresh(&v, &entry.id, vec!["personal".into()], 4096).unwrap();
    let text = serde_json::to_string(&suppressed).unwrap();
    assert!(!text.contains("通用合成背景"));
    assert!(!text.contains(&own));
    connections::revoke(&v, &entry.id, 1).unwrap();
    assert!(!a.file_path.exists());
    assert!(agent_entry::refresh(&v, &entry.id, vec!["personal".into()], 4096).is_err());
}
#[test]
fn file_entry_excludes_inactive_and_cross_scope_evidence_memories() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let own = source(&v, "personal", "own", "可见证据");
    let foreign = source(&v, "project:other", "foreign", "隐私证据");
    let input:MemoryInput=serde_json::from_value(json!({"scope":"personal","content":"旧计划SECRET","source_refs":[own],"evidence":"user_explicit"})).unwrap();
    let m = v.add_memory(input).unwrap();
    v.set_state(&m.id, 1, recallcard::MemoryState::Superseded)
        .unwrap();
    // Cross-scope Memory is forbidden at writer too; filtered sources stay enforced if old data exists.
    let input:MemoryInput=serde_json::from_value(json!({"scope":"personal","content":"跨域SECRET","source_refs":[foreign],"evidence":"user_explicit"})).unwrap();
    let _ = v.add_memory(input);
    let entry = connect(&v, "personal");
    let text = serde_json::to_string(
        &agent_entry::refresh(&v, &entry.id, vec!["personal".into()], 4096).unwrap(),
    )
    .unwrap();
    assert!(!text.contains("旧计划SECRET"));
    assert!(!text.contains("跨域SECRET"));
}
#[test]
fn real_cli_refreshes_file_and_local_probe_does_not_fake_host_observation() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let event = source(&v, "personal", "cli", "CLI实际读取的来源");
    let entry = connect(&v, "personal");
    let output = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .arg("--vault")
        .arg(v.root())
        .args([
            "--json",
            "connection-context",
            &entry.id,
            "--scope",
            "personal",
            "--budget-bytes",
            "4096",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], true);
    assert!(value["result"].to_string().contains(&event));
    let verified = connection_setup::verify(
        &v,
        &entry.id,
        "personal",
        std::path::Path::new(env!("CARGO_BIN_EXE_recallcard")),
    )
    .unwrap();
    assert_eq!(verified.readiness, "read_verified");
    assert_eq!(verified.verification_scope, "local_cli");
    assert!(verified.last_read_at.is_none());
    let stored = connections::get(&v, &entry.id).unwrap().unwrap();
    assert!(stored.last_bootstrap_at.is_none());
    assert!(stored.last_read_at.is_none());
    connections::revoke(&v, &entry.id, 1).unwrap();
    assert_eq!(
        connection_setup::health(&v, &entry.id, "personal")
            .unwrap()
            .readiness,
        "revoked"
    );
    assert!(connection_setup::verify(
        &v,
        &entry.id,
        "personal",
        std::path::Path::new(env!("CARGO_BIN_EXE_recallcard"))
    )
    .is_err());
}
#[cfg(unix)]
#[test]
fn generated_snapshot_symlink_is_never_followed() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let entry = connect(&v, "personal");
    let p = agent_entry::path(&v, &entry.id).unwrap();
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let outside = d.path().join("outside");
    std::fs::write(&outside, b"keep").unwrap();
    std::os::unix::fs::symlink(&outside, &p).unwrap();
    assert!(agent_entry::refresh(&v, &entry.id, vec!["personal".into()], 4096).is_err());
    assert_eq!(std::fs::read(outside).unwrap(), b"keep");
}

#[test]
fn revoke_and_canonical_write_remove_only_owned_crash_snapshots() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let entry = connect(&v, "personal");
    let snapshot = agent_entry::refresh(&v, &entry.id, vec!["personal".into()], 4096).unwrap();
    let root = snapshot.file_path.parent().unwrap();
    let current_temp = root.join(".recallcard-entry-crash123");
    let legacy = root.join(".tmpABC123");
    let unrelated = root.join("other-document.txt");
    for p in [&current_temp, &legacy, &unrelated] {
        std::fs::write(p, b"SYNTHETIC_PRIVATE_BODY").unwrap();
    }
    source(&v, "personal", "new", "新原话");
    assert!(!snapshot.file_path.exists());
    assert!(!current_temp.exists());
    assert!(!legacy.exists());
    assert!(unrelated.exists());
    agent_entry::refresh(&v, &entry.id, vec!["personal".into()], 4096).unwrap();
    std::fs::write(&current_temp, b"SYNTHETIC_PRIVATE_BODY").unwrap();
    connections::revoke(&v, &entry.id, 1).unwrap();
    assert!(!snapshot.file_path.exists());
    assert!(!current_temp.exists());
    assert!(unrelated.exists());
}

#[test]
fn installed_scope_survives_pause_resume_without_reinstall_but_scope_change_does_not() {
    use recallcard::application::agent_install::{self, InstallRequest};
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let project = d.path().join("project");
    std::fs::create_dir(&project).unwrap();
    source(&v, "personal", "initial", "合成背景");
    let binary = std::path::PathBuf::from(env!("CARGO_BIN_EXE_recallcard"));
    let plan = agent_install::plan(
        &v,
        &InstallRequest {
            client: "codex".into(),
            connection_id: String::new(),
            scope: "personal".into(),
            project_dir: project.clone(),
            binary_path: binary.clone(),
        },
    )
    .unwrap();
    let applied = Command::new(&binary)
        .arg("--vault")
        .arg(v.root())
        .args(["--json", "connection-setup", "apply", &plan.plan_id])
        .output()
        .unwrap();
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let applied: Value = serde_json::from_slice(&applied.stdout).unwrap();
    assert_eq!(applied["result"]["configuration"], "written");
    assert_eq!(
        applied["result"]["health"]["verification_scope"],
        "local_cli"
    );
    let entry = connections::get(&v, &plan.connection_id).unwrap().unwrap();
    let initial = connection_setup::verify(&v, &entry.id, "personal", &binary).unwrap();
    assert_eq!(initial.verification_scope, "local_cli");
    let snapshot = agent_entry::path(&v, &entry.id).unwrap();
    let saved = connections::configure(&v, entry.grant.clone(), Some(1)).unwrap();
    assert_eq!(saved.permission_revision, 1);
    assert!(snapshot.exists());
    let original_config = std::fs::read(project.join(".codex/config.toml")).unwrap();
    let mut paused = entry.grant.clone();
    paused.auto_recall = false;
    let paused = connections::configure(&v, paused, Some(1)).unwrap();
    assert!(!snapshot.exists());
    assert_eq!(
        connection_setup::health(&v, &entry.id, "personal")
            .unwrap()
            .readiness,
        "paused"
    );
    let resumed =
        connections::configure(&v, entry.grant, Some(paused.permission_revision)).unwrap();
    assert_eq!(
        connection_setup::health(&v, &entry.id, "personal")
            .unwrap()
            .installation,
        "configured"
    );
    assert_eq!(
        connection_setup::verify(&v, &entry.id, "personal", &binary)
            .unwrap()
            .readiness,
        "read_verified"
    );
    assert_eq!(
        std::fs::read(project.join(".codex/config.toml")).unwrap(),
        original_config
    );
    let mut changed = resumed.grant;
    changed.recall_scopes = vec!["project:other".into()];
    connections::configure(&v, changed, Some(resumed.permission_revision)).unwrap();
    assert_eq!(
        connection_setup::health(&v, &entry.id, "project:other")
            .unwrap()
            .installation,
        "configuration_changed"
    );
    assert!(connection_setup::verify(&v, &entry.id, "project:other", &binary).is_err());
}
