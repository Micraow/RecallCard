//! 桌面连接范围边界；只用合成的资料库、安装编号和待配对请求。
use recallcard::{
    application::{
        connections::{self, ConnectionGrant},
        ErrorCode,
    },
    desktop::DesktopSession,
    Vault,
};
const EXT: &str = "abcdefghijklmnopabcdefghijklmnop";
const INSTALL: &str = "11111111-1111-4111-8111-111111111111";
fn grant(scope: &str) -> ConnectionGrant {
    ConnectionGrant {
        client_kind: "browser".into(),
        host_identity: EXT.into(),
        installation_id: Some(INSTALL.into()),
        platform: "chatgpt".into(),
        recall_scopes: vec![scope.into()],
        capture_scopes: vec![scope.into()],
        provider_disclosure: true,
        auto_capture: true,
        auto_recall: true,
    }
}
#[test]
fn desktop_inventory_and_mutations_are_confined_to_the_selected_scope() {
    let root = tempfile::tempdir().unwrap();
    let mut session = DesktopSession::default();
    let info = session
        .select_vault(&root.path().join("vault"), true)
        .unwrap();
    let e = session
        .connection_configure(&info.session_id, "personal", grant("personal"), None)
        .unwrap();
    assert_eq!(
        session
            .connection_inventory(&info.session_id, "personal")
            .unwrap()["entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(session
        .connection_inventory(&info.session_id, "project:other")
        .unwrap()["entries"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        session
            .connection_configure(
                &info.session_id,
                "personal",
                grant("project:other"),
                Some(1)
            )
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        session
            .connection_revoke(&info.session_id, "project:other", &e.id, 1)
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        session
            .connection_configure(
                &info.session_id,
                "project:other",
                grant("project:other"),
                Some(1)
            )
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert!(
        session
            .connection_revoke(&info.session_id, "personal", &e.id, 1)
            .unwrap()
            .revoked
    );
    assert!(session
        .connection_revoke(&info.session_id, "personal", &e.id, 1)
        .is_err());
}
#[test]
fn desktop_pairing_only_shows_current_scope_caps_and_rechecks_approval() {
    let root = tempfile::tempdir().unwrap();
    let mut session = DesktopSession::default();
    let info = session
        .select_vault(&root.path().join("vault"), true)
        .unwrap();
    let vault = Vault::open(&root.path().join("vault")).unwrap();
    let request = connections::request_pairing(
        &vault,
        EXT,
        INSTALL,
        "chatgpt",
        vec!["personal".into(), "project:other".into()],
        vec!["personal".into()],
    )
    .unwrap();
    let visible = session
        .connection_inventory(&info.session_id, "personal")
        .unwrap();
    assert_eq!(
        visible["pending_pairings"][0]["recall_scope_cap"],
        serde_json::json!(["personal"])
    );
    assert!(session
        .connection_inventory(&info.session_id, "project:hidden")
        .unwrap()["pending_pairings"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        session
            .connection_approve_pairing(
                &info.session_id,
                "project:other",
                &request.request_id,
                grant("personal"),
                None
            )
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    let accepted = session
        .connection_approve_pairing(
            &info.session_id,
            "personal",
            &request.request_id,
            grant("personal"),
            None,
        )
        .unwrap();
    assert_eq!(accepted.grant.installation_id.as_deref(), Some(INSTALL));
    assert!(session
        .connection_approve_pairing(
            &info.session_id,
            "personal",
            &request.request_id,
            grant("personal"),
            None
        )
        .is_err());
}
#[test]
fn stale_desktop_session_and_replaced_vault_cannot_approve_a_connection() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("vault");
    let mut session = DesktopSession::default();
    let info = session.select_vault(&path, true).unwrap();
    session
        .select_vault(&root.path().join("other-vault"), true)
        .unwrap();
    assert!(session
        .connection_configure(&info.session_id, "personal", grant("personal"), None)
        .is_err());
    let current = session.select_vault(&path, false).unwrap();
    std::fs::rename(&path, root.path().join("previous-vault")).unwrap();
    Vault::init(&path).unwrap();
    assert_eq!(
        session
            .connection_inventory(&current.session_id, "personal")
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert!(session
        .connection_configure(&current.session_id, "personal", grant("personal"), None)
        .is_err());
}
#[test]
fn managed_agent_configs_bind_both_reading_and_lifecycle_to_the_approved_connection() {
    let root = tempfile::tempdir().unwrap();
    let mut session = DesktopSession::default();
    let info = session
        .select_vault(&root.path().join("vault"), true)
        .unwrap();
    let mut g = grant("personal");
    g.client_kind = "claude_code".into();
    g.host_identity = "claude-code".into();
    g.platform = "claude_code".into();
    g.installation_id = None;
    g.capture_scopes.clear();
    g.auto_capture = false;
    let entry = session
        .connection_configure(&info.session_id, "personal", g, None)
        .unwrap();
    let config = session
        .connection_agent_configs(
            &info.session_id,
            "personal",
            &entry.id,
            &root.path().join("recallcard"),
        )
        .unwrap();
    assert_eq!(config["registered"], false);
    assert!(config["mcp"]["mcpServers"]["recallcard"]["args"]
        .as_array()
        .unwrap()
        .iter()
        .any(|arg| arg == "--connection-id"));
    assert!(
        config["hooks"]["hooks"]["SessionStart"][0]["hooks"][0]["args"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == &entry.id)
    );
    assert!(session
        .connection_agent_configs(
            &info.session_id,
            "project:other",
            &entry.id,
            &root.path().join("recallcard")
        )
        .is_err());
    session
        .connection_revoke(&info.session_id, "personal", &entry.id, 1)
        .unwrap();
    assert!(session
        .connection_agent_configs(
            &info.session_id,
            "personal",
            &entry.id,
            &root.path().join("recallcard")
        )
        .is_err());
}
