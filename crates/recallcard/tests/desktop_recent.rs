//! 上次工作区恢复边界；目录、内容、项目名均为合成数据。
use recallcard::{desktop::DesktopSession, Vault};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path};
use tempfile::{tempdir, TempDir};

fn fixture() -> (TempDir, DesktopSession, String, std::path::PathBuf) {
    let directory = tempdir().unwrap();
    let mut session = DesktopSession::default();
    let info = session
        .select_vault(&directory.path().join("合成资料库"), true)
        .unwrap();
    let config = directory.path().join("app-config/workspace.json");
    (directory, session, info.session_id, config)
}

fn read_config(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn write_config(path: &Path, value: &Value) {
    // 原地编辑以保留文件标识和权限；解析器/摘要应当独立拒绝内容改变。
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

fn vault_tree(root: &Path) -> BTreeMap<std::path::PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, path: &Path, tree: &mut BTreeMap<std::path::PathBuf, Option<Vec<u8>>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                tree.insert(path.strip_prefix(root).unwrap().into(), None);
                visit(root, &path, tree);
            } else {
                tree.insert(
                    path.strip_prefix(root).unwrap().into(),
                    Some(fs::read(&path).unwrap()),
                );
            }
        }
    }
    let mut tree = BTreeMap::new();
    visit(root, root, &mut tree);
    tree
}

#[test]
fn first_start_without_configuration_does_not_create_any_directory() {
    let directory = tempdir().unwrap();
    let config = directory.path().join("not-created/app/workspace.json");
    assert!(DesktopSession::default()
        .restore_workspace(&config)
        .unwrap()
        .is_none());
    assert!(!config.parent().unwrap().exists());
}

#[test]
fn restart_restores_exact_vault_and_scope_with_new_session_and_no_write_tokens() {
    let (_directory, mut session, id, config) = fixture();
    let root = session.status(&id).unwrap().root;
    let preview = session
        .preview_note(&id, "project:合成", "合成未批准正文")
        .unwrap();
    session
        .remember_workspace(&id, "project:合成", &config)
        .unwrap();
    let before = vault_tree(Path::new(&root));
    let config_text = fs::read_to_string(&config).unwrap();
    assert!(!config_text.contains("合成未批准正文"));
    assert!(!config_text.contains(&id));
    assert!(!config_text.contains(&preview.preview_id));
    drop(session);
    let mut restarted = DesktopSession::default();
    let restored = restarted.restore_workspace(&config).unwrap().unwrap();
    assert_eq!(restored.vault.root, root);
    assert_eq!(restored.scope, "project:合成");
    assert_ne!(restored.vault.session_id, id);
    assert!(restarted
        .confirm_note(&restored.vault.session_id, &preview.preview_id)
        .is_err());
    assert!(restarted.status(&id).is_err());
    assert_eq!(vault_tree(Path::new(&root)), before);
    let response = serde_json::to_value(restored).unwrap();
    assert!(response["vault"]["session_id"].is_string());
    assert_eq!(response["scope"], "project:合成");
}

#[test]
fn latest_explicit_save_binds_scope_to_its_vault_and_stale_sessions_cannot_overwrite() {
    let (directory, mut session, first_id, config) = fixture();
    session
        .remember_workspace(&first_id, "project:first", &config)
        .unwrap();
    let second = session
        .select_vault(&directory.path().join("second-vault"), true)
        .unwrap();
    assert!(session
        .remember_workspace(&first_id, "project:wrong", &config)
        .is_err());
    let first = DesktopSession::default()
        .restore_workspace(&config)
        .unwrap()
        .unwrap();
    assert_eq!(first.scope, "project:first");
    assert_ne!(first.vault.root, second.root);
    session
        .remember_workspace(&second.session_id, "project:second", &config)
        .unwrap();
    let restored = DesktopSession::default()
        .restore_workspace(&config)
        .unwrap()
        .unwrap();
    assert_eq!(restored.vault.root, second.root);
    assert_eq!(restored.scope, "project:second");
}

#[test]
fn cancel_preview_close_or_failed_selection_does_not_rewrite_remembered_choice() {
    let (directory, mut session, id, config) = fixture();
    session
        .remember_workspace(&id, "personal", &config)
        .unwrap();
    let expected = fs::read(&config).unwrap();
    session
        .preview_note(&id, "personal", "合成取消内容")
        .unwrap();
    session.cancel_previews(&id).unwrap();
    session.close_vault();
    assert!(session
        .select_vault(&directory.path().join("not-a-vault"), false)
        .is_err());
    assert_eq!(fs::read(&config).unwrap(), expected);
    assert!(!directory.path().join("not-a-vault").exists());
    assert!(DesktopSession::default()
        .restore_workspace(&config)
        .unwrap()
        .is_some());
}

#[test]
fn late_restore_cannot_replace_a_newly_opened_session() {
    let (directory, mut session, id, config) = fixture();
    session
        .remember_workspace(&id, "personal", &config)
        .unwrap();
    let second = session
        .select_vault(&directory.path().join("second-vault"), true)
        .unwrap();
    assert!(session.restore_workspace(&config).is_err());
    assert!(session.status(&second.session_id).is_ok());
}

#[test]
fn missing_vault_or_required_directory_never_gets_recreated() {
    let (directory, session, id, config) = fixture();
    let root = session.status(&id).unwrap().root;
    session
        .remember_workspace(&id, "personal", &config)
        .unwrap();
    drop(session);
    fs::rename(&root, directory.path().join("moved-vault")).unwrap();
    assert!(DesktopSession::default()
        .restore_workspace(&config)
        .is_err());
    assert!(!Path::new(&root).exists());
    fs::rename(directory.path().join("moved-vault"), &root).unwrap();
    for name in [
        "events",
        "memories",
        "objects",
        "control/dream-receipts",
        "control/suppressions",
    ] {
        let path = Path::new(&root).join(name);
        fs::remove_dir(&path).unwrap();
        let before = vault_tree(Path::new(&root));
        assert!(
            DesktopSession::default()
                .restore_workspace(&config)
                .is_err(),
            "{name}"
        );
        assert!(!path.exists(), "{name}");
        assert_eq!(vault_tree(Path::new(&root)), before);
        fs::create_dir(&path).unwrap();
    }
}

#[test]
fn replaced_root_is_rejected_even_when_it_is_another_valid_vault() {
    let (directory, session, id, config) = fixture();
    let root = session.status(&id).unwrap().root;
    session
        .remember_workspace(&id, "personal", &config)
        .unwrap();
    drop(session);
    fs::rename(&root, directory.path().join("original-vault")).unwrap();
    Vault::init(Path::new(&root)).unwrap();
    let before = vault_tree(Path::new(&root));
    assert!(DesktopSession::default()
        .restore_workspace(&config)
        .is_err());
    assert_eq!(vault_tree(Path::new(&root)), before);
}

#[test]
fn replacing_marker_with_identical_bytes_or_changing_it_in_place_is_rejected() {
    let (_directory, session, id, config) = fixture();
    let root = session.status(&id).unwrap().root;
    let marker = Path::new(&root).join("control/schema-version.json");
    let original = fs::read(&marker).unwrap();
    session
        .remember_workspace(&id, "personal", &config)
        .unwrap();
    fs::rename(&marker, marker.with_extension("old")).unwrap();
    fs::write(&marker, &original).unwrap();
    assert!(DesktopSession::default()
        .restore_workspace(&config)
        .is_err());
    fs::remove_file(&marker).unwrap();
    fs::rename(marker.with_extension("old"), &marker).unwrap();
    fs::write(
        &marker,
        b"{\"schema_version\": 900, \"application\":\"RecallCard\"}",
    )
    .unwrap();
    assert!(DesktopSession::default()
        .restore_workspace(&config)
        .is_err());
}

#[test]
fn replaced_config_file_with_identical_bytes_is_rejected() {
    let (_directory, session, id, config) = fixture();
    session
        .remember_workspace(&id, "personal", &config)
        .unwrap();
    fs::rename(&config, config.with_extension("original")).unwrap();
    fs::copy(config.with_extension("original"), &config).unwrap();
    assert!(DesktopSession::default()
        .restore_workspace(&config)
        .is_err());
}

#[test]
fn strict_schema_rejects_unknown_fields_bad_scope_digest_and_oversize_without_vault_changes() {
    let (_directory, session, id, config) = fixture();
    let root = session.status(&id).unwrap().root;
    session
        .remember_workspace(&id, "personal", &config)
        .unwrap();
    let original = read_config(&config);
    let before = vault_tree(Path::new(&root));
    let mut cases = Vec::new();
    let mut value = original.clone();
    value["unexpected"] = json!(true);
    cases.push(value);
    let mut value = original.clone();
    value["workspace"]["unexpected"] = json!(true);
    cases.push(value);
    let mut value = original.clone();
    value["workspace"]["identity"]["unexpected"] = json!(true);
    cases.push(value);
    let mut value = original.clone();
    value["workspace"]["marker"]["unexpected"] = json!(true);
    cases.push(value);
    let mut value = original.clone();
    value["storage"]["file_access"]["unexpected"] = json!(true);
    cases.push(value);
    let mut value = original.clone();
    value["schema"] = json!("recallcard.desktop-workspace/99");
    cases.push(value);
    let mut value = original.clone();
    value["workspace"]["scope"] = json!("*");
    cases.push(value);
    let mut value = original.clone();
    value["workspace"]["scope"] = json!("project:another");
    cases.push(value);
    let mut value = original.clone();
    value["digest"] = json!("bad");
    cases.push(value);
    for case in cases {
        write_config(&config, &case);
        assert!(DesktopSession::default()
            .restore_workspace(&config)
            .is_err());
    }
    for bytes in [vec![b'x'; 32769], vec![0xff; 20], b"{}".to_vec()] {
        fs::write(&config, bytes).unwrap();
        assert!(DesktopSession::default()
            .restore_workspace(&config)
            .is_err());
    }
    assert_eq!(vault_tree(Path::new(&root)), before);
    assert!(session.status(&id).is_ok());
}

#[test]
fn failed_save_leaves_open_session_and_pending_review_usable() {
    let (_directory, mut session, id, config) = fixture();
    let preview = session
        .preview_note(&id, "personal", "合成已审查笔记")
        .unwrap();
    fs::write(
        config.parent().unwrap(),
        "a file blocks the config directory",
    )
    .unwrap();
    assert!(session
        .remember_workspace(&id, "personal", &config)
        .is_err());
    assert!(session.status(&id).is_ok());
    session.confirm_note(&id, &preview.preview_id).unwrap();
    assert_eq!(session.status(&id).unwrap().event_count, 1);
}

#[test]
fn vault_internal_config_target_and_invalid_scope_are_rejected_before_writes() {
    let (_directory, session, id, config) = fixture();
    let root = session.status(&id).unwrap().root;
    let before = vault_tree(Path::new(&root));
    let internal = Path::new(&root).join("new-config/deeper/workspace.json");
    assert!(session
        .remember_workspace(&id, "personal", &internal)
        .is_err());
    assert!(session.remember_workspace(&id, "*", &config).is_err());
    assert!(session
        .remember_workspace(&id, "personal", Path::new("relative.json"))
        .is_err());
    assert!(!config.exists());
    assert_eq!(vault_tree(Path::new(&root)), before);
}

#[cfg(unix)]
#[test]
fn symbolic_and_hard_links_are_rejected_without_following_them() {
    use std::os::unix::fs::symlink;
    let (directory, session, id, config) = fixture();
    let root = session.status(&id).unwrap().root;
    session
        .remember_workspace(&id, "personal", &config)
        .unwrap();
    let saved = config.with_extension("original");
    fs::rename(&config, &saved).unwrap();
    symlink(&saved, &config).unwrap();
    assert!(DesktopSession::default()
        .restore_workspace(&config)
        .is_err());
    assert!(session
        .remember_workspace(&id, "personal", &config)
        .is_err());
    fs::remove_file(&config).unwrap();
    fs::hard_link(&saved, &config).unwrap();
    assert!(DesktopSession::default()
        .restore_workspace(&config)
        .is_err());
    fs::remove_file(&config).unwrap();
    fs::rename(&saved, &config).unwrap();
    let moved = directory.path().join("moved-vault");
    fs::rename(&root, &moved).unwrap();
    symlink(&moved, &root).unwrap();
    assert!(DesktopSession::default()
        .restore_workspace(&config)
        .is_err());
    fs::remove_file(&root).unwrap();
    fs::rename(&moved, &root).unwrap();
    let marker = Path::new(&root).join("control/schema-version.json");
    let old_marker = marker.with_extension("old");
    fs::rename(&marker, &old_marker).unwrap();
    symlink(&old_marker, &marker).unwrap();
    assert!(DesktopSession::default()
        .restore_workspace(&config)
        .is_err());
    fs::remove_file(&marker).unwrap();
    fs::rename(&old_marker, &marker).unwrap();
    let parent = config.parent().unwrap();
    let moved_parent = directory.path().join("moved-config");
    fs::rename(parent, &moved_parent).unwrap();
    symlink(&moved_parent, parent).unwrap();
    assert!(DesktopSession::default()
        .restore_workspace(&config)
        .is_err());
}

#[cfg(unix)]
#[test]
fn permission_changes_are_rejected_and_never_repaired_automatically() {
    use std::os::unix::fs::PermissionsExt;
    let (_directory, session, id, config) = fixture();
    let root = session.status(&id).unwrap().root;
    session
        .remember_workspace(&id, "personal", &config)
        .unwrap();
    for path in [
        Path::new(&root).to_owned(),
        Path::new(&root).join("control/schema-version.json"),
        config.clone(),
        config.parent().unwrap().to_owned(),
    ] {
        let original = fs::metadata(&path).unwrap().permissions().mode();
        let changed = original ^ 0o100;
        fs::set_permissions(&path, fs::Permissions::from_mode(changed)).unwrap();
        assert!(
            DesktopSession::default()
                .restore_workspace(&config)
                .is_err(),
            "{}",
            path.display()
        );
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode(), changed);
        if path == Path::new(&root) || path.ends_with("schema-version.json") {
            assert!(session
                .remember_workspace(&id, "personal", &config)
                .is_err());
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(original)).unwrap();
    }
    assert!(DesktopSession::default()
        .restore_workspace(&config)
        .unwrap()
        .is_some());
    assert_eq!(
        fs::metadata(&config).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn world_writable_configuration_directory_is_rejected_without_chmod() {
    use std::os::unix::fs::PermissionsExt;
    let (_directory, session, id, config) = fixture();
    fs::create_dir(config.parent().unwrap()).unwrap();
    fs::set_permissions(config.parent().unwrap(), fs::Permissions::from_mode(0o777)).unwrap();
    assert!(session
        .remember_workspace(&id, "personal", &config)
        .is_err());
    assert_eq!(
        fs::metadata(config.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o777
    );
    assert!(!config.exists());
    assert!(session.status(&id).is_ok());
}

#[test]
fn explicit_open_existing_also_refuses_to_repair_missing_directories() {
    let (_directory, mut session, id, _config) = fixture();
    let root = session.status(&id).unwrap().root;
    session.close_vault();
    let missing = Path::new(&root).join("events");
    fs::remove_dir(&missing).unwrap();
    assert!(Vault::open_existing(Path::new(&root)).is_err());
    assert!(!missing.exists());
    assert!(session.select_vault(Path::new(&root), false).is_err());
    assert!(!missing.exists());
    // CLI 兼容入口保持原有行为；只有明确调用该入口才会补全布局。
    Vault::open(Path::new(&root)).unwrap();
    assert!(missing.is_dir());
}

#[test]
fn corrupt_event_records_fail_restore_without_rewriting_the_vault() {
    let (_directory, session, id, config) = fixture();
    let root = session.status(&id).unwrap().root;
    session
        .remember_workspace(&id, "personal", &config)
        .unwrap();
    drop(session);
    fs::write(
        Path::new(&root).join("events/synthetic-broken.jsonl"),
        "合成损坏行\n",
    )
    .unwrap();
    let before = vault_tree(Path::new(&root));
    assert!(DesktopSession::default()
        .restore_workspace(&config)
        .is_err());
    assert_eq!(vault_tree(Path::new(&root)), before);
}

#[cfg(unix)]
#[test]
fn normal_755_app_data_directory_can_store_private_config_beside_default_vault() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempdir().unwrap();
    let app_data = directory.path().join("recallcard-app");
    fs::create_dir(&app_data).unwrap();
    fs::set_permissions(&app_data, fs::Permissions::from_mode(0o755)).unwrap();
    let mut session = DesktopSession::default();
    let info = session.select_vault(&app_data.join("vault"), true).unwrap();
    let config = app_data.join("recent-workspace.json");
    session
        .remember_workspace(&info.session_id, "personal", &config)
        .unwrap();
    assert_eq!(
        fs::metadata(&app_data).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert_eq!(
        fs::metadata(&config).unwrap().permissions().mode() & 0o777,
        0o600
    );
    drop(session);
    assert_eq!(
        DesktopSession::default()
            .restore_workspace(&config)
            .unwrap()
            .unwrap()
            .vault
            .root,
        info.root
    );
}

#[test]
fn explicit_remember_can_replace_a_corrupt_or_oversize_private_config() {
    let (_directory, session, id, config) = fixture();
    session
        .remember_workspace(&id, "personal", &config)
        .unwrap();
    for bytes in [b"{invalid}".to_vec(), vec![b'x'; 32769]] {
        fs::write(&config, bytes).unwrap();
        assert!(DesktopSession::default()
            .restore_workspace(&config)
            .is_err());
        session
            .remember_workspace(&id, "project:reselected", &config)
            .unwrap();
        assert_eq!(
            DesktopSession::default()
                .restore_workspace(&config)
                .unwrap()
                .unwrap()
                .scope,
            "project:reselected"
        );
    }
}
