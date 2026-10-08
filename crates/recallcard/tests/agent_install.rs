//! 全部使用合成 HOME、项目、程序与授权；不运行 Codex/Claude，不修改真实宿主配置。
use recallcard::{
    application::{
        agent_install::{self, InstallRequest},
        connections, ErrorCode,
    },
    Vault,
};
use serde_json::{json, Value};
use std::{
    ffi::OsString,
    fs,
    path::PathBuf,
    sync::{Mutex, MutexGuard},
};

static ENV: Mutex<()> = Mutex::new(());
struct Fixture {
    _guard: MutexGuard<'static, ()>,
    _dir: tempfile::TempDir,
    old_home: Option<OsString>,
    old_state: Option<OsString>,
    vault: Vault,
    request: InstallRequest,
}
impl Fixture {
    fn new(client: &str) -> Self {
        let guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("synthetic-home");
        fs::create_dir(&home).unwrap();
        let old_home = std::env::var_os("HOME");
        let old_state = std::env::var_os("RECALLCARD_STATE_DIR");
        std::env::set_var("HOME", &home);
        std::env::set_var("RECALLCARD_STATE_DIR", dir.path().join("private-state"));
        let project_dir = dir.path().join("project with ' spaces");
        fs::create_dir(&project_dir).unwrap();
        let binary_path = dir.path().join("recallcard synthetic ' $(unexecuted)");
        fs::write(&binary_path, b"#!/bin/sh\nexit 77\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&binary_path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let vault = Vault::init(&dir.path().join("synthetic-vault")).unwrap();
        let request = InstallRequest {
            client: client.into(),
            connection_id: String::new(),
            scope: "personal".into(),
            project_dir,
            binary_path,
        };
        Self {
            _guard: guard,
            _dir: dir,
            old_home,
            old_state,
            vault,
            request,
        }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.request.project_dir.join(name)
    }
    fn write(&self, name: &str, text: &str) {
        let path = self.path(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn journal_path(&self, id: &str) -> PathBuf {
        self.vault
            .state_dir()
            .unwrap()
            .join("agent-install-v1")
            .join(format!("{id}.json"))
    }
    fn update_journal(&self, id: &str, update: impl FnOnce(&mut Value)) {
        let path = self.journal_path(id);
        let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        update(&mut value);
        fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for (key, value) in [
            ("HOME", self.old_home.as_ref()),
            ("RECALLCARD_STATE_DIR", self.old_state.as_ref()),
        ] {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
    }
}

#[test]
fn codex_plan_is_private_and_apply_preserves_user_config_without_host_claims() {
    let f = Fixture::new("codex");
    let existing = "# 用户保留注释\nmodel = \"synthetic-model\"\n\n[mcp_servers.other]\ncommand = \"unexecuted-other\"\n# 保留服务注释\n[mcp_servers.other.env]\nTOKEN = \"synthetic-secret-not-in-preview\"\n";
    f.write(".codex/config.toml", existing);
    f.write("AGENTS.md", "# 项目约定\n保留人工内容。\n");
    let p = agent_install::plan(&f.vault, &f.request).unwrap();
    assert!(p.grant_required);
    assert_eq!(p.permission_revision, None);
    assert!(connections::get(&f.vault, &p.connection_id)
        .unwrap()
        .is_none());
    assert_eq!(
        fs::read_to_string(f.path(".codex/config.toml")).unwrap(),
        existing
    );
    assert!(!serde_json::to_string(&p)
        .unwrap()
        .contains("synthetic-secret-not-in-preview"));
    let r = agent_install::apply(&f.vault, &p.plan_id).unwrap();
    assert_eq!(r.state, "configuration_written");
    assert!(!r.host_verified);
    let config = fs::read_to_string(f.path(".codex/config.toml")).unwrap();
    assert!(config.contains("# 用户保留注释\nmodel = \"synthetic-model\""));
    assert!(config.contains(
        "# 保留服务注释\n[mcp_servers.other.env]\nTOKEN = \"synthetic-secret-not-in-preview\""
    ));
    let parsed: toml_edit::DocumentMut = config.parse().unwrap();
    assert_eq!(
        parsed["mcp_servers"][&p.server_key]["command"].as_str(),
        f.request.binary_path.to_str()
    );
    assert!(parsed["mcp_servers"][&p.server_key]["args"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v.as_str() == Some(&p.connection_id)));
    let instructions = fs::read_to_string(f.path("AGENTS.md")).unwrap();
    assert!(instructions.starts_with("# 项目约定\n保留人工内容。\n"));
    assert!(instructions.contains("connection-context"));
    assert!(instructions.contains("'--json' 'connection-context'"));
    assert!(instructions.contains("file_path"));
    assert!(instructions.contains("主动执行下面固定的本机只读命令"));
    assert!(instructions.contains("目录覆盖可能不同"));
    let inspection = agent_install::inspect(&f.vault, &p.connection_id).unwrap();
    assert_eq!(inspection.configuration, "configured");
    assert_eq!(inspection.permission_revision, Some(r.permission_revision));
    assert!(!inspection.host_verified);
    assert!(connections::get(&f.vault, &p.connection_id)
        .unwrap()
        .unwrap()
        .last_handshake_at
        .is_none());
    agent_install::apply(&f.vault, &p.plan_id).unwrap();
    let again = agent_install::plan(&f.vault, &f.request).unwrap();
    assert!(again.files.iter().all(|file| !file.changed));
    assert!(agent_install::apply(&f.vault, &again.plan_id)
        .unwrap()
        .changed_files
        .is_empty());
}

#[test]
fn claude_preserves_servers_and_hooks_and_quotes_fixed_command_arguments() {
    let f = Fixture::new("claude_code");
    let other = json!({"mcpServers":{"other":{"command":"never-executed","env":{"TOKEN":"synthetic-private"}}},"unrelated":true});
    f.write(".mcp.json", &other.to_string());
    let existing_hook = json!({"matcher":"startup","hooks":[{"type":"command","command":"never-executed-existing"}]});
    f.write(".claude/settings.json", &json!({"hooks":{"SessionStart":[existing_hook],"Stop":[]},"permissions":{"deny":["Bash(rm *)"]}}).to_string());
    f.write("CLAUDE.md", "项目已有指引\n");
    let p = agent_install::plan(&f.vault, &f.request).unwrap();
    assert!(!serde_json::to_string(&p)
        .unwrap()
        .contains("synthetic-private"));
    agent_install::apply(&f.vault, &p.plan_id).unwrap();
    let mcp: Value = serde_json::from_slice(&fs::read(f.path(".mcp.json")).unwrap()).unwrap();
    assert_eq!(mcp["mcpServers"]["other"], other["mcpServers"]["other"]);
    assert_eq!(mcp["unrelated"], true);
    #[cfg(not(windows))]
    {
        let settings: Value =
            serde_json::from_slice(&fs::read(f.path(".claude/settings.json")).unwrap()).unwrap();
        assert_eq!(settings["hooks"]["SessionStart"][0], existing_hook);
        assert_eq!(settings["permissions"]["deny"][0], "Bash(rm *)");
        let added = &settings["hooks"]["SessionStart"][1];
        assert_eq!(added["matcher"], "startup|resume|compact|clear|fork");
        let command = added["hooks"][0]["command"].as_str().unwrap();
        assert!(command.contains("'\\''"));
        assert!(command.contains("'--connection-id'"));
        assert!(command.contains("'--budget-bytes' '4096'"));
    }
    let again = agent_install::plan(&f.vault, &f.request).unwrap();
    assert!(again.files.iter().all(|file| !file.changed));
}

#[test]
fn changed_files_reject_whole_plan_before_new_grant_and_inspection_detects_deletion() {
    let f = Fixture::new("codex");
    let p = agent_install::plan(&f.vault, &f.request).unwrap();
    f.write("AGENTS.md", "计划后用户新增内容\n");
    assert_eq!(
        agent_install::apply(&f.vault, &p.plan_id).unwrap_err().code,
        ErrorCode::Conflict
    );
    assert!(!f.path(".codex/config.toml").exists());
    assert!(connections::get(&f.vault, &p.connection_id)
        .unwrap()
        .is_none());
    let p = agent_install::plan(&f.vault, &f.request).unwrap();
    agent_install::apply(&f.vault, &p.plan_id).unwrap();
    fs::remove_file(f.path("AGENTS.md")).unwrap();
    assert_eq!(
        agent_install::inspect(&f.vault, &p.connection_id)
            .unwrap()
            .configuration,
        "configuration_changed"
    );
    assert!(agent_install::apply(&f.vault, &p.plan_id).is_err());
}

#[test]
fn foreign_server_and_modified_managed_instruction_are_not_overwritten() {
    let f = Fixture::new("claude_code");
    let initial = agent_install::plan(&f.vault, &f.request).unwrap();
    let foreign =
        json!({"mcpServers":{initial.server_key.clone():{"command":"foreign-server"}}}).to_string();
    f.write(".mcp.json", &foreign);
    assert_eq!(
        agent_install::plan(&f.vault, &f.request).unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(fs::read_to_string(f.path(".mcp.json")).unwrap(), foreign);
    fs::remove_file(f.path(".mcp.json")).unwrap();
    agent_install::apply(&f.vault, &initial.plan_id).unwrap();
    let text = fs::read_to_string(f.path("CLAUDE.md"))
        .unwrap()
        .replace("无需用户逐轮挑选记忆", "用户修改管理区域");
    f.write("CLAUDE.md", &text);
    assert_eq!(
        agent_install::plan(&f.vault, &f.request).unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(fs::read_to_string(f.path("CLAUDE.md")).unwrap(), text);
}

#[test]
fn revoked_or_changed_scope_grant_cannot_be_used_by_an_earlier_plan() {
    let f = Fixture::new("codex");
    let first = agent_install::plan(&f.vault, &f.request).unwrap();
    let grant = connections::configure(&f.vault, first.proposed_grant.clone(), None).unwrap();
    let p = agent_install::plan(&f.vault, &f.request).unwrap();
    let mut changed = grant.grant.clone();
    changed.recall_scopes = vec!["work".into()];
    connections::configure(&f.vault, changed, Some(grant.permission_revision)).unwrap();
    assert_eq!(
        agent_install::apply(&f.vault, &p.plan_id).unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    assert!(!f.path("AGENTS.md").exists());
    let restored = connections::configure(&f.vault, first.proposed_grant, Some(2)).unwrap();
    let p = agent_install::plan(&f.vault, &f.request).unwrap();
    connections::revoke(&f.vault, &restored.id, restored.permission_revision).unwrap();
    assert_eq!(
        agent_install::apply(&f.vault, &p.plan_id).unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    assert!(!f.path("AGENTS.md").exists());
}

#[test]
fn independent_projects_receive_independently_revocable_connections() {
    let mut f = Fixture::new("codex");
    let one = agent_install::plan(&f.vault, &f.request).unwrap();
    agent_install::apply(&f.vault, &one.plan_id).unwrap();
    f.request.project_dir = f
        .request
        .project_dir
        .parent()
        .unwrap()
        .join("second-project");
    fs::create_dir(&f.request.project_dir).unwrap();
    f.request.scope = "work".into();
    let two = agent_install::plan(&f.vault, &f.request).unwrap();
    assert_ne!(one.connection_id, two.connection_id);
    assert_ne!(
        one.proposed_grant.installation_id,
        two.proposed_grant.installation_id
    );
    agent_install::apply(&f.vault, &two.plan_id).unwrap();
    connections::revoke(&f.vault, &one.connection_id, 1).unwrap();
    assert!(
        !connections::get(&f.vault, &two.connection_id)
            .unwrap()
            .unwrap()
            .revoked
    );
}

#[test]
fn interruption_before_final_activation_resumes_without_premature_grant() {
    let f = Fixture::new("codex");
    let plan = agent_install::plan(&f.vault, &f.request).unwrap();
    f.update_journal(&plan.plan_id, |journal| {
        journal["phase"] = json!("ready");
        for change in journal["changes"].as_array().unwrap() {
            let path = PathBuf::from(change["public"]["path"].as_str().unwrap());
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, change["after"].as_str().unwrap()).unwrap();
        }
    });
    assert!(connections::get(&f.vault, &plan.connection_id)
        .unwrap()
        .is_none());
    let result = agent_install::apply(&f.vault, &plan.plan_id).unwrap();
    assert_eq!(result.permission_revision, 1);
    assert_eq!(
        agent_install::apply(&f.vault, &plan.plan_id)
            .unwrap()
            .permission_revision,
        1
    );
}

#[test]
fn expiry_binary_change_and_parent_traversal_are_rejected_before_writes() {
    let mut f = Fixture::new("codex");
    let p = agent_install::plan(&f.vault, &f.request).unwrap();
    f.update_journal(&p.plan_id, |journal| {
        journal["public"]["expires_at"] = json!("2000-01-01T00:00:00Z")
    });
    assert_eq!(
        agent_install::apply(&f.vault, &p.plan_id).unwrap_err().code,
        ErrorCode::Conflict
    );
    let p = agent_install::plan(&f.vault, &f.request).unwrap();
    fs::write(&f.request.binary_path, b"different trusted program bytes").unwrap();
    assert_eq!(
        agent_install::apply(&f.vault, &p.plan_id).unwrap_err().code,
        ErrorCode::Conflict
    );
    assert!(connections::get(&f.vault, &p.connection_id)
        .unwrap()
        .is_none());
    f.request.project_dir = f.request.project_dir.join("../project with ' spaces");
    assert_eq!(
        agent_install::plan(&f.vault, &f.request).unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        agent_install::apply(&f.vault, "../../outside")
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
}

#[cfg(unix)]
#[test]
fn symlink_replacement_of_project_parent_config_and_binary_are_rejected() {
    use std::os::unix::fs::symlink;
    let mut f = Fixture::new("codex");
    let p = agent_install::plan(&f.vault, &f.request).unwrap();
    let outside = f.request.project_dir.parent().unwrap().join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, f.path(".codex")).unwrap();
    assert_eq!(
        agent_install::apply(&f.vault, &p.plan_id).unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    assert!(!outside.join("config.toml").exists());
    fs::remove_file(f.path(".codex")).unwrap();
    let target = outside.join("instructions");
    fs::write(&target, "unchanged").unwrap();
    symlink(&target, f.path("AGENTS.md")).unwrap();
    assert_eq!(
        agent_install::plan(&f.vault, &f.request).unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    fs::remove_file(f.path("AGENTS.md")).unwrap();
    let fake_binary = outside.join("program");
    symlink(&f.request.binary_path, &fake_binary).unwrap();
    f.request.binary_path = fake_binary;
    assert_eq!(
        agent_install::plan(&f.vault, &f.request).unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "unchanged");
}

#[test]
fn interrupted_apply_resumes_without_duplicate_blocks_or_grant_revision() {
    let f = Fixture::new("codex");
    f.write("AGENTS.md", "手写项目说明\n");
    let p = agent_install::plan(&f.vault, &f.request).unwrap();
    // 模拟首个文件发布后进程结束；首次授权尚未创建。
    f.update_journal(&p.plan_id, |journal| {
        journal["phase"] = json!("applying");
        // 模拟首次授权之前的文件事务。
        let change = &journal["changes"][0];
        let path = PathBuf::from(change["public"]["path"].as_str().unwrap());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, change["after"].as_str().unwrap()).unwrap();
    });
    let result = agent_install::apply(&f.vault, &p.plan_id).unwrap();
    assert_eq!(result.permission_revision, 1);
    agent_install::apply(&f.vault, &p.plan_id).unwrap();
    let instructions = fs::read_to_string(f.path("AGENTS.md")).unwrap();
    assert_eq!(
        instructions
            .matches(&format!("<!-- recallcard:{}:begin -->", p.server_key))
            .count(),
        1
    );
    assert!(instructions.starts_with("手写项目说明\n"));
}

#[test]
fn interrupted_rollback_restores_before_then_retries_and_does_not_overwrite_foreign_edits() {
    let f = Fixture::new("claude_code");
    f.write("CLAUDE.md", "先前内容\n");
    let p = agent_install::plan(&f.vault, &f.request).unwrap();
    f.update_journal(&p.plan_id, |journal| {
        journal["phase"] = json!("rolling_back");
        let change = &journal["changes"][0];
        fs::write(
            change["public"]["path"].as_str().unwrap(),
            change["after"].as_str().unwrap(),
        )
        .unwrap();
    });
    assert!(agent_install::apply(&f.vault, &p.plan_id).is_err());
    assert!(connections::get(&f.vault, &p.connection_id)
        .unwrap()
        .is_none());
    let restored = agent_install::plan(&f.vault, &f.request).unwrap();
    agent_install::apply(&f.vault, &restored.plan_id).unwrap();
    let next = agent_install::plan(&f.vault, &f.request).unwrap();
    f.update_journal(&next.plan_id, |journal| {
        journal["phase"] = json!("applying")
    });
    f.write("CLAUDE.md", "中断后用户修改\n");
    assert_eq!(
        agent_install::apply(&f.vault, &next.plan_id)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        fs::read_to_string(f.path("CLAUDE.md")).unwrap(),
        "中断后用户修改\n"
    );
}

#[test]
fn owned_scope_update_preserves_other_configuration_and_uses_fresh_revision() {
    let mut f = Fixture::new("codex");
    let first = agent_install::plan(&f.vault, &f.request).unwrap();
    agent_install::apply(&f.vault, &first.plan_id).unwrap();
    let mut grant = first.proposed_grant;
    grant.recall_scopes.push("work".into());
    connections::configure(&f.vault, grant, Some(1)).unwrap();
    let prior = fs::read_to_string(f.path(".codex/config.toml")).unwrap();
    f.write(
        ".codex/config.toml",
        &format!("# 用户后来添加的注释\n{prior}"),
    );
    f.request.scope = "work".into();
    let updated = agent_install::plan(&f.vault, &f.request).unwrap();
    assert_eq!(updated.permission_revision, Some(2));
    agent_install::apply(&f.vault, &updated.plan_id).unwrap();
    let after = fs::read_to_string(f.path(".codex/config.toml")).unwrap();
    assert!(after.starts_with("# 用户后来添加的注释\n"));
    let parsed: toml_edit::DocumentMut = after.parse().unwrap();
    let args = parsed["mcp_servers"][&updated.server_key]["args"]
        .as_array()
        .unwrap();
    assert_eq!(args.get(args.len() - 1).unwrap().as_str(), Some("work"));
    assert_eq!(
        connections::get(&f.vault, &updated.connection_id)
            .unwrap()
            .unwrap()
            .permission_revision,
        2
    );
}

#[cfg(unix)]
#[test]
fn second_file_write_failure_restores_first_and_never_activates_new_grant() {
    use std::os::unix::fs::PermissionsExt;
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let f = Fixture::new("codex");
    f.write(".codex/config.toml", "# 原配置\nmodel = \"synthetic\"\n");
    let original = fs::read_to_string(f.path(".codex/config.toml")).unwrap();
    let plan = agent_install::plan(&f.vault, &f.request).unwrap();
    // .codex 仍可写，项目根目录不能创建第二个文件，复现真实中途 I/O 失败。
    fs::set_permissions(&f.request.project_dir, fs::Permissions::from_mode(0o500)).unwrap();
    let result = agent_install::apply(&f.vault, &plan.plan_id);
    fs::set_permissions(&f.request.project_dir, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(result.is_err());
    assert_eq!(
        fs::read_to_string(f.path(".codex/config.toml")).unwrap(),
        original
    );
    assert!(!f.path("AGENTS.md").exists());
    assert!(connections::get(&f.vault, &plan.connection_id)
        .unwrap()
        .is_none());
    assert!(plan.recovery_dir.join("0-before.backup").exists());
    assert!(plan.recovery_dir.join("0-rollback.backup").exists());
}

#[cfg(unix)]
#[test]
fn hardlinked_configuration_is_rejected_without_changing_alias_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new("codex");
    f.write("AGENTS.md", "硬链接保留\n");
    let alias = f
        .request
        .project_dir
        .parent()
        .unwrap()
        .join("external-alias");
    fs::hard_link(f.path("AGENTS.md"), &alias).unwrap();
    let mode = fs::metadata(&alias).unwrap().permissions().mode();
    assert_eq!(
        agent_install::plan(&f.vault, &f.request).unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(fs::metadata(&alias).unwrap().permissions().mode(), mode);
    assert_eq!(fs::read_to_string(alias).unwrap(), "硬链接保留\n");
}
