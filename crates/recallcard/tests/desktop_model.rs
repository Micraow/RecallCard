//! 桌面操作句柄与范围/身份边界；不填真实密钥，不访问真实系统凭据。
use recallcard::{
    application::{background_memory::*, credentials::CredentialStorage, ErrorCode},
    desktop::DesktopSession,
    Vault,
};
#[test]
fn owned_operation_rejects_other_scope_before_accepting_credentials() {
    let root = tempfile::tempdir().unwrap();
    let vault = Vault::init(&root.path().join("vault")).unwrap();
    let mut desktop = DesktopSession::default();
    let selected = desktop.select_vault(vault.root(), false).unwrap();
    let handle = desktop
        .model_operation(&selected.session_id, "personal")
        .unwrap();
    let config = MemoryConfig {
        scope: "project:other".into(),
        ..Default::default()
    };
    let error = handle
        .configure(
            config,
            Some("SYNTHETIC_ONLY".into()),
            Some(CredentialStorage::SessionOnly),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::PermissionDenied);
    assert!(!serde_json::to_string(&error)
        .unwrap()
        .contains("SYNTHETIC_ONLY"));
    assert!(handle.status().unwrap().runtime.jobs.is_empty());
}
#[test]
fn owned_operation_pins_vault_identity_across_background_waits() {
    let root = tempfile::tempdir().unwrap();
    let vault = Vault::init(&root.path().join("vault")).unwrap();
    let mut desktop = DesktopSession::default();
    let selected = desktop.select_vault(vault.root(), false).unwrap();
    let handle = desktop
        .model_operation(&selected.session_id, "personal")
        .unwrap();
    std::fs::rename(vault.root(), root.path().join("old-vault")).unwrap();
    Vault::init(vault.root()).unwrap();
    assert_eq!(
        handle.status().unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        handle.stop_service().unwrap_err().code,
        ErrorCode::PermissionDenied
    );
}
#[test]
fn stale_session_cannot_create_model_operation() {
    let root = tempfile::tempdir().unwrap();
    let first = Vault::init(&root.path().join("first")).unwrap();
    let second = Vault::init(&root.path().join("second")).unwrap();
    let mut desktop = DesktopSession::default();
    let old = desktop.select_vault(first.root(), false).unwrap();
    desktop.select_vault(second.root(), false).unwrap();
    assert!(desktop
        .model_operation(&old.session_id, "personal")
        .is_err());
}
