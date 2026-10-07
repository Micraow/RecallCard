//! ZIP 桌面选择/预览/确认与 CLI 的合成合同，不包含真实历史。
use recallcard::{
    desktop::{DesktopSession, ImportFilePreview, ImportSelection, VaultInfo},
    Vault,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Cursor, Write},
    path::Path,
    process::{Command, Stdio},
};
use tempfile::{tempdir, TempDir};
use zip::{write::SimpleFileOptions, ZipWriter};

fn setup() -> (TempDir, DesktopSession, VaultInfo) {
    let dir = tempdir().unwrap();
    let mut session = DesktopSession::default();
    let info = session
        .select_vault(&dir.path().join("vault"), true)
        .unwrap();
    (dir, session, info)
}
fn conversation(id: &str, count: usize) -> Value {
    let mut mapping = serde_json::Map::new();
    for index in 0..count {
        let key = format!("node-{index}");
        mapping.insert(
            key,
            json!({
                "parent":if index == 0 {None} else {Some(format!("node-{}",index-1))},
                "message":{"id":format!("message-{index}"),"create_time":1_700_000_000.25,
                    "author":{"role":if index % 2 == 0 {"user"} else {"assistant"}},
                    "content":{"content_type":"text","parts":[format!("合成文本 {index}")]}}
            }),
        );
    }
    json!({"id":id,"title":format!("合成会话 {id}"),"current_node":format!("node-{}",count-1),"mapping":mapping})
}
fn zip_bytes(conversations: &[Value], attachment: &str) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (i, conv) in conversations.iter().enumerate() {
        writer
            .start_file(format!("{i}.json"), SimpleFileOptions::default())
            .unwrap();
        writer.write_all(conv.to_string().as_bytes()).unwrap();
    }
    writer
        .start_file("0.md", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"synthetic markdown copy").unwrap();
    writer
        .start_file("attachment.txt", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(attachment.as_bytes()).unwrap();
    writer.finish().unwrap().into_inner()
}
fn select(session: &mut DesktopSession, info: &VaultInfo, path: &Path) -> ImportSelection {
    match session
        .select_import_file(&info.session_id, "auto", path, "personal")
        .unwrap()
    {
        ImportFilePreview::Selection(selection) => selection,
        _ => panic!("ZIP 必须先返回会话选择清单"),
    }
}
fn preview(
    session: &mut DesktopSession,
    info: &VaultInfo,
    selection: &ImportSelection,
    ids: &[&str],
) -> recallcard::desktop::ImportPreview {
    session
        .preview_import_selection(
            &info.session_id,
            &selection.selection_id,
            &ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
        )
        .unwrap()
}

#[test]
fn archive_selection_is_read_only_and_confirms_only_chosen_conversations() {
    let (dir, mut session, info) = setup();
    let path = dir.path().join("synthetic.zip");
    fs::write(
        &path,
        zip_bytes(&[conversation("one", 2), conversation("two", 2)], "asset"),
    )
    .unwrap();
    let selection = select(&mut session, &info, &path);
    assert_eq!(selection.coverage.conversations_available, 2);
    assert_eq!(selection.coverage.events_available, 4);
    assert_eq!(selection.coverage.markdown_files_skipped, 1);
    assert_eq!(selection.coverage.other_files_skipped, 1);
    assert_eq!(
        selection.conversations[0].title.as_deref(),
        Some("合成会话 one")
    );
    assert_eq!(session.status(&info.session_id).unwrap().event_count, 0);
    assert!(session
        .confirm_import(&info.session_id, &selection.selection_id)
        .is_err());
    let chosen = preview(&mut session, &info, &selection, &["two"]);
    assert_eq!(chosen.event_count, 2);
    assert_eq!(chosen.conversations.len(), 1);
    assert_eq!(chosen.conversations[0].source_id, "two");
    assert_eq!(chosen.samples[0].source.conversation_id, "two");
    assert_eq!(chosen.samples[0].role, recallcard::Role::User);
    assert_eq!(chosen.samples[1].role, recallcard::Role::Assistant);
    assert_eq!(
        chosen.samples[0].occurred_at.unwrap().timestamp_millis(),
        1_700_000_000_250
    );
    assert_eq!(session.status(&info.session_id).unwrap().event_count, 0);
    let result = session
        .confirm_import(&info.session_id, &chosen.preview_id)
        .unwrap();
    assert_eq!(result["events_added"], 2);
    assert!(session
        .confirm_import(&info.session_id, &chosen.preview_id)
        .is_err());
    let vault = Vault::open(Path::new(&info.root)).unwrap();
    assert!(vault
        .events()
        .unwrap()
        .iter()
        .all(|e| e.data.source.conversation_id == "two"));
    assert!(vault.memories().unwrap().is_empty());
    let again = preview(&mut session, &info, &selection, &["two"]);
    assert_eq!(
        session
            .confirm_import(&info.session_id, &again.preview_id)
            .unwrap()["events_added"],
        0
    );
    let next = preview(&mut session, &info, &selection, &["one"]);
    assert_eq!(
        session
            .confirm_import(&info.session_id, &next.preview_id)
            .unwrap()["events_added"],
        2
    );
}

#[test]
fn changing_even_skipped_bytes_invalidates_selection_and_preview_permanently() {
    for change_at_confirmation in [false, true] {
        let (dir, mut session, info) = setup();
        let path = dir.path().join("synthetic.zip");
        let original = zip_bytes(&[conversation("one", 2)], "original asset");
        fs::write(&path, &original).unwrap();
        let selection = select(&mut session, &info, &path);
        let pending =
            change_at_confirmation.then(|| preview(&mut session, &info, &selection, &["one"]));
        let changed = zip_bytes(&[conversation("one", 2)], "modified asset");
        assert_eq!(original.len(), changed.len());
        fs::write(&path, changed).unwrap();
        let error = if let Some(p) = &pending {
            session
                .confirm_import(&info.session_id, &p.preview_id)
                .unwrap_err()
        } else {
            session
                .preview_import_selection(
                    &info.session_id,
                    &selection.selection_id,
                    &["one".into()],
                )
                .unwrap_err()
        };
        assert!(error.contains("文件已改变"));
        fs::write(&path, original).unwrap();
        assert!(session
            .preview_import_selection(&info.session_id, &selection.selection_id, &["one".into()])
            .is_err());
        if let Some(p) = pending {
            assert!(session
                .confirm_import(&info.session_id, &p.preview_id)
                .is_err());
        }
        assert_eq!(session.status(&info.session_id).unwrap().event_count, 0);
    }
}

#[test]
fn replacing_archive_with_identical_bytes_invalidates_each_stage() {
    for after_preview in [false, true] {
        let (dir, mut session, info) = setup();
        let path = dir.path().join("synthetic.zip");
        let bytes = zip_bytes(&[conversation("one", 2)], "asset");
        fs::write(&path, &bytes).unwrap();
        let selection = select(&mut session, &info, &path);
        let pending = after_preview.then(|| preview(&mut session, &info, &selection, &["one"]));
        fs::rename(&path, dir.path().join("original.zip")).unwrap();
        fs::write(&path, &bytes).unwrap();
        if let Some(p) = pending {
            assert!(session
                .confirm_import(&info.session_id, &p.preview_id)
                .is_err());
        } else {
            assert!(session
                .preview_import_selection(
                    &info.session_id,
                    &selection.selection_id,
                    &["one".into()]
                )
                .is_err());
        }
        assert_eq!(session.status(&info.session_id).unwrap().event_count, 0);
    }
}

#[test]
fn empty_unknown_and_over_limit_selection_never_write_and_batching_remains_available() {
    let (dir, mut session, info) = setup();
    let path = dir.path().join("large.zip");
    fs::write(
        &path,
        zip_bytes(
            &[conversation("one", 2501), conversation("two", 2501)],
            "asset",
        ),
    )
    .unwrap();
    let selection = select(&mut session, &info, &path);
    assert_eq!(selection.coverage.events_available, 5002);
    assert!(session
        .preview_import_selection(&info.session_id, &selection.selection_id, &[])
        .unwrap_err()
        .contains("至少选择"));
    assert!(session
        .preview_import_selection(
            &info.session_id,
            &selection.selection_id,
            &["missing".into()]
        )
        .is_err());
    assert!(session
        .preview_import_selection(
            &info.session_id,
            &selection.selection_id,
            &["one".into(), "two".into()]
        )
        .unwrap_err()
        .contains("5000"));
    assert_eq!(
        preview(&mut session, &info, &selection, &["one"]).event_count,
        2501
    );
    assert_eq!(session.status(&info.session_id).unwrap().event_count, 0);
}

#[test]
fn return_cancel_and_vault_switch_revoke_import_tokens() {
    let (dir, mut session, info) = setup();
    let path = dir.path().join("synthetic.zip");
    fs::write(&path, zip_bytes(&[conversation("one", 2)], "asset")).unwrap();
    let selection = select(&mut session, &info, &path);
    let first = preview(&mut session, &info, &selection, &["one"]);
    session
        .return_import_selection(&info.session_id, &selection.selection_id)
        .unwrap();
    assert!(session
        .confirm_import(&info.session_id, &first.preview_id)
        .is_err());
    let second = preview(&mut session, &info, &selection, &["one"]);
    session.cancel_previews(&info.session_id).unwrap();
    assert!(session
        .confirm_import(&info.session_id, &second.preview_id)
        .is_err());
    assert!(session
        .preview_import_selection(&info.session_id, &selection.selection_id, &["one".into()])
        .is_err());
    let selection = select(&mut session, &info, &path);
    let third = preview(&mut session, &info, &selection, &["one"]);
    let other = session
        .select_vault(&dir.path().join("other"), true)
        .unwrap();
    assert!(session
        .confirm_import(&other.session_id, &third.preview_id)
        .is_err());
    assert!(session
        .preview_import_selection(&other.session_id, &selection.selection_id, &["one".into()])
        .is_err());
}

#[test]
fn single_conversation_object_autodetects_and_keeps_source_role_and_time() {
    let (dir, mut session, info) = setup();
    let path = dir.path().join("single.json");
    fs::write(&path, conversation("single", 2).to_string()).unwrap();
    let p = match session
        .select_import_file(&info.session_id, "auto", &path, "personal")
        .unwrap()
    {
        ImportFilePreview::Preview(p) => p,
        _ => panic!("JSON 直接显示消息预览"),
    };
    assert_eq!(p.samples[0].source.platform, "chatgpt-export");
    assert_eq!(p.samples[0].source.conversation_id, "single");
    assert!(p.samples[0].occurred_at.is_some());
    assert_eq!(p.conversations.len(), 1);
    assert_eq!(
        session
            .confirm_import(&info.session_id, &p.preview_id)
            .unwrap()["events_added"],
        2
    );
}

#[cfg(unix)]
#[test]
fn archive_symlinks_are_rejected_before_inspection_and_confirmation() {
    use std::os::unix::fs::symlink;
    let (dir, mut session, info) = setup();
    let path = dir.path().join("synthetic.zip");
    let linked = dir.path().join("linked.zip");
    fs::write(&path, zip_bytes(&[conversation("one", 2)], "asset")).unwrap();
    symlink(&path, &linked).unwrap();
    assert!(session
        .select_import_file(&info.session_id, "auto", &linked, "personal")
        .is_err());
    let selection = select(&mut session, &info, &path);
    let p = preview(&mut session, &info, &selection, &["one"]);
    let moved = dir.path().join("moved.zip");
    fs::rename(&path, &moved).unwrap();
    symlink(&moved, &path).unwrap();
    assert!(session
        .confirm_import(&info.session_id, &p.preview_id)
        .is_err());
}

#[test]
fn cli_import_accepts_archive_files_and_preserves_json_stdin() {
    let (dir, _session, info) = setup();
    let path = dir.path().join("synthetic.zip");
    fs::write(&path, zip_bytes(&[conversation("one", 2)], "asset")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .args([
            "--vault",
            &info.root,
            "import",
            "--format",
            "chatgpt-export",
            "--scope",
            "personal",
            "--file",
            path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["events_added"], 2);
    assert_eq!(value["coverage"]["markdown_files_skipped"], 1);
    let mut child = Command::new(env!("CARGO_BIN_EXE_recallcard"))
        .args([
            "--vault",
            &info.root,
            "import",
            "--format",
            "chatgpt-export",
            "--scope",
            "personal",
            "--file",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(conversation("two", 2).to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        Vault::open(Path::new(&info.root))
            .unwrap()
            .events()
            .unwrap()
            .len(),
        4
    );
}
