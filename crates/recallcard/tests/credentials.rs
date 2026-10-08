//! 仅使用合成密钥与注入 backend；默认不写真实 OS keyring、不请求供应商。
use chrono::Utc;
use recallcard::{
    application::{background_memory::*, credentials::*, ErrorCode},
    Vault,
};
use std::cell::{Cell, RefCell};
fn target() -> MemoryProviderConfig {
    MemoryProviderConfig {
        endpoint: "https://example.invalid/api".into(),
        model: "synthetic".into(),
    }
}
struct Backend {
    value: RefCell<Option<String>>,
    available: bool,
    fail_store: bool,
    wrong_readback: bool,
    writes: Cell<usize>,
}
impl Backend {
    fn new() -> Self {
        Self {
            value: RefCell::new(None),
            available: true,
            fail_store: false,
            wrong_readback: false,
            writes: Cell::new(0),
        }
    }
}
impl CredentialBackend for Backend {
    fn store(&self, secret: &str) -> Result<(), CredentialBackendError> {
        self.writes.set(self.writes.get() + 1);
        if self.fail_store {
            return Err(CredentialBackendError);
        }
        *self.value.borrow_mut() = Some(if self.wrong_readback {
            "SYNTHETIC_WRONG".into()
        } else {
            secret.into()
        });
        Ok(())
    }
    fn load(&self) -> Result<Option<String>, CredentialBackendError> {
        if self.available {
            Ok(self.value.borrow().clone())
        } else {
            Err(CredentialBackendError)
        }
    }
    fn remove(&self) -> Result<(), CredentialBackendError> {
        *self.value.borrow_mut() = None;
        Ok(())
    }
}
#[test]
fn session_choice_never_attempts_persistent_store_and_debug_is_redacted() {
    let backend = Backend::new();
    let value = prepare_with_backend(
        target(),
        "SYNTHETIC_ONLY_KEY".into(),
        CredentialStorage::SessionOnly,
        Some(&backend),
    )
    .unwrap();
    assert_eq!(backend.writes.get(), 0);
    assert_eq!(value.status.storage, CredentialStorage::SessionOnly);
    assert_eq!(value.status.lifetime, "background_service_exit");
    assert!(!format!("{value:?}").contains("SYNTHETIC_ONLY_KEY"));
    assert!(!serde_json::to_string(&value.status)
        .unwrap()
        .contains("SYNTHETIC_ONLY_KEY"));
}
#[test]
fn protected_storage_requires_verified_readback() {
    let backend = Backend::new();
    let value = prepare_with_backend(
        target(),
        "SYNTHETIC_ONLY_KEY".into(),
        CredentialStorage::OsProtected,
        Some(&backend),
    )
    .unwrap();
    assert_eq!(backend.writes.get(), 1);
    assert_eq!(value.status.storage, CredentialStorage::OsProtected);
    assert_eq!(value.status.os_protected_available, Some(true));
}
#[test]
fn known_unavailable_backend_falls_back_explicitly_without_writing() {
    let backend = Backend {
        available: false,
        ..Backend::new()
    };
    let value = prepare_with_backend(
        target(),
        "SYNTHETIC_ONLY_KEY".into(),
        CredentialStorage::OsProtected,
        Some(&backend),
    )
    .unwrap();
    assert_eq!(backend.writes.get(), 0);
    assert_eq!(value.status.storage, CredentialStorage::SessionOnly);
    assert_eq!(value.status.os_protected_available, Some(false));
    assert!(value.status.message.contains("关闭窗口不会清除"));
}
#[test]
fn uncertain_persistent_write_is_not_retried_or_claimed_session_only() {
    let backend = Backend {
        fail_store: true,
        ..Backend::new()
    };
    let error = prepare_with_backend(
        target(),
        "SYNTHETIC_ONLY_KEY".into(),
        CredentialStorage::OsProtected,
        Some(&backend),
    )
    .unwrap_err();
    assert_eq!(backend.writes.get(), 1);
    assert_eq!(error.code, ErrorCode::ModelUnavailable);
    assert!(error.message.contains("未确认"));
    assert!(!error.to_string().contains("SYNTHETIC_ONLY_KEY"));
}
#[test]
fn incorrect_readback_fails_closed() {
    let backend = Backend {
        wrong_readback: true,
        ..Backend::new()
    };
    assert!(prepare_with_backend(
        target(),
        "SYNTHETIC_ONLY_KEY".into(),
        CredentialStorage::OsProtected,
        Some(&backend)
    )
    .is_err());
    assert_eq!(backend.writes.get(), 1);
}
#[test]
fn anonymous_pipe_contract_is_bounded_and_secret_free_in_diagnostics() {
    let value = PreparedCredential::session(target(), "SYNTHETIC_ONLY_KEY".into()).unwrap();
    let mut bytes = Vec::new();
    write_handoff(&mut bytes, &value).unwrap();
    let recovered = read_handoff(bytes.as_slice()).unwrap();
    assert_eq!(recovered.target, target());
    assert_eq!(recovered.status.storage, CredentialStorage::SessionOnly);
    assert!(!format!("{recovered:?}").contains("SYNTHETIC_ONLY_KEY"));
    let mut duplicate = String::from_utf8(bytes).unwrap();
    duplicate = duplicate.replacen("{", "{\"schema\":\"duplicate\",", 1);
    assert!(read_handoff(duplicate.as_bytes()).is_err());
    assert!(read_handoff(vec![b'x'; 16 * 1024 + 1].as_slice()).is_err());
}
#[test]
fn key_is_bound_to_exact_provider_and_never_falls_back_to_other_target() {
    let secret = PreparedCredential::session(target(), "SYNTHETIC_ONLY_KEY".into()).unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../python")
        .canonicalize()
        .unwrap();
    let provider = PythonMemoryProvider::new("python3".into(), path).with_credential(secret);
    let p = target();
    let mut config = MemoryConfig {
        enabled: true,
        provider: Some(p.clone()),
        consent: Some(MemoryConsent {
            endpoint: p.endpoint,
            model: p.model,
            scope: "personal".into(),
            send_source_snapshots: true,
            send_memory_snapshots: true,
            auto_apply: true,
            accepted_at: Utc::now(),
        }),
        ..Default::default()
    };
    assert!(provider.available(&config).is_ok());
    config.provider.as_mut().unwrap().endpoint = "https://other.invalid/api".into();
    config.consent.as_mut().unwrap().endpoint = "https://other.invalid/api".into();
    assert!(!provider.credential_status(&config).present);
    assert_eq!(
        provider.available(&config).unwrap_err().code,
        ErrorCode::ModelUnavailable
    );
}
#[test]
fn invalid_secret_or_target_never_reaches_storage() {
    let backend = Backend::new();
    assert!(prepare_with_backend(
        target(),
        "SYNTHETIC\nSECRET".into(),
        CredentialStorage::OsProtected,
        Some(&backend)
    )
    .is_err());
    let mut invalid = target();
    invalid.endpoint = "https://user:password@example.invalid/api".into();
    assert!(prepare_with_backend(
        invalid,
        "SYNTHETIC_ONLY_KEY".into(),
        CredentialStorage::OsProtected,
        Some(&backend)
    )
    .is_err());
    assert_eq!(backend.writes.get(), 0);
}
#[test]
fn real_keyring_adapter_uses_library_mock_for_crud_without_native_store() {
    keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
    let root = tempfile::tempdir().unwrap();
    let vault = Vault::init(&root.path().join("vault")).unwrap();
    let backend = OsKeyringBackend::new(&vault, &target()).unwrap();
    let value = prepare_with_backend(
        target(),
        "SYNTHETIC_ADAPTER_KEY".into(),
        CredentialStorage::OsProtected,
        Some(&backend),
    )
    .unwrap();
    assert_eq!(value.status.storage, CredentialStorage::OsProtected);
    assert_eq!(
        backend.load().unwrap().as_deref(),
        Some("SYNTHETIC_ADAPTER_KEY")
    );
    backend.remove().unwrap();
    assert!(backend.load().unwrap().is_none());
}
