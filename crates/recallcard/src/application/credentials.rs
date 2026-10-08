//! 模型凭据边界：系统 keyring 或明确的服务会话内存；绝不写入资料库或任务。
//! keyring 3.x 官方 API：https://docs.rs/keyring/3.6.3/keyring/
use super::{background_memory::MemoryProviderConfig, AppError, AppResult, ErrorCode};
use crate::{
    filesystem::{file_identity, open_local_file},
    model::hash,
    Vault,
};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use zeroize::Zeroizing;

const SERVICE: &str = "RecallCard model provider";
const HANDOFF_SCHEMA: &str = "recallcard.credential-handoff/1";
const MAX_HANDOFF: u64 = 16 * 1024;

/// 本类型不实现 Serialize/Display；Debug 永远隐藏秘密。
pub struct Secret(Zeroizing<String>);
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret([已隐藏])")
    }
}
impl Secret {
    pub fn from_user_input(value: String) -> AppResult<Self> {
        let value = Zeroizing::new(value);
        if value.is_empty() || value.len() > 8192 || !value.bytes().all(|b| (33..=126).contains(&b))
        {
            return Err(invalid());
        }
        Ok(Self(value))
    }
    pub(crate) fn expose(&self) -> &str {
        self.0.as_str()
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialStorage {
    OsProtected,
    SessionOnly,
    Environment,
    Unavailable,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CredentialStatus {
    #[serde(default)]
    pub provider_ref: Option<String>,
    pub present: bool,
    pub storage: CredentialStorage,
    pub lifetime: String,
    pub os_protected_available: Option<bool>,
    pub message: String,
}
impl Default for CredentialStatus {
    fn default() -> Self {
        Self {
            provider_ref: None,
            present: false,
            storage: CredentialStorage::Unavailable,
            lifetime: "not_configured".into(),
            os_protected_available: None,
            message: "尚未提供模型凭据".into(),
        }
    }
}
impl CredentialStatus {
    pub fn for_provider(mut self, target: &MemoryProviderConfig) -> Self {
        self.provider_ref = Some(provider_ref(target));
        self
    }

    pub fn environment() -> Self {
        Self {
            provider_ref: None,
            present: true,
            storage: CredentialStorage::Environment,
            lifetime: "process_environment".into(),
            os_protected_available: None,
            message: "使用高级启动环境提供的凭据，未存入设置文件".into(),
        }
    }
    fn session(unavailable: bool) -> Self {
        Self {
            provider_ref: None,
            present: true,
            storage: CredentialStorage::SessionOnly,
            lifetime: "background_service_exit".into(),
            os_protected_available: unavailable.then_some(false),
            message: if unavailable {
                "系统保护存储不可用；仅当前后台服务会话持有凭据，关闭窗口不会清除，停止服务后丢失"
            } else {
                "仅当前后台服务会话持有凭据；关闭窗口不会清除，停止服务后丢失"
            }
            .into(),
        }
    }
    fn protected() -> Self {
        Self {
            provider_ref: None,
            present: true,
            storage: CredentialStorage::OsProtected,
            lifetime: "until_deleted_from_os_store".into(),
            os_protected_available: Some(true),
            message: "已存入操作系统保护的凭据存储；设置与资料库中没有密钥".into(),
        }
    }
}
/// 只能在受信任本机调用栈中传递；不进入 DTO、作业或错误日志。
pub struct PreparedCredential {
    pub target: MemoryProviderConfig,
    pub status: CredentialStatus,
    secret: Secret,
}
impl std::fmt::Debug for PreparedCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedCredential")
            .field("status", &self.status)
            .field("secret", &"[已隐藏]")
            .finish()
    }
}
impl PreparedCredential {
    pub fn session(target: MemoryProviderConfig, key: String) -> AppResult<Self> {
        Ok(Self {
            status: CredentialStatus::session(false).for_provider(&target),
            target,
            secret: Secret::from_user_input(key)?,
        })
    }
    pub(crate) fn for_target(&self, target: &MemoryProviderConfig) -> Option<&str> {
        (&self.target == target).then(|| self.secret.expose())
    }
}

/// adapter 将平台错误转换为固定类别，不返回 Keychain/Windows/DBus 原始错误内容。
#[derive(Debug, Clone, Copy)]
pub struct CredentialBackendError;
static KEYRING_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
pub trait CredentialBackend {
    fn store(&self, secret: &str) -> std::result::Result<(), CredentialBackendError>;
    fn load(&self) -> std::result::Result<Option<String>, CredentialBackendError>;
    fn remove(&self) -> std::result::Result<(), CredentialBackendError>;
}
pub struct OsKeyringBackend {
    entry: keyring::Entry,
}
impl OsKeyringBackend {
    pub fn new(vault: &Vault, target: &MemoryProviderConfig) -> AppResult<Self> {
        if !cfg!(any(
            target_os = "linux",
            target_os = "macos",
            target_os = "windows"
        )) {
            return Err(unavailable());
        }
        let root = file_identity(&open_local_file(vault.root()).map_err(|_| unavailable())?)
            .map_err(|_| unavailable())?;
        let marker = file_identity(
            &open_local_file(&vault.root().join("control/schema-version.json"))
                .map_err(|_| unavailable())?,
        )
        .map_err(|_| unavailable())?;
        let identity =
            hash(&serde_json::to_vec(&(root, marker, target)).map_err(|_| unavailable())?);
        let entry = keyring::Entry::new(SERVICE, &identity).map_err(|_| unavailable())?;
        Ok(Self { entry })
    }
}
impl CredentialBackend for OsKeyringBackend {
    fn store(&self, secret: &str) -> std::result::Result<(), CredentialBackendError> {
        let _guard = KEYRING_LOCK.lock().map_err(|_| CredentialBackendError)?;
        self.entry
            .set_password(secret)
            .map_err(|_| CredentialBackendError)
    }
    fn load(&self) -> std::result::Result<Option<String>, CredentialBackendError> {
        let _guard = KEYRING_LOCK.lock().map_err(|_| CredentialBackendError)?;
        match self.entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(CredentialBackendError),
        }
    }
    fn remove(&self) -> std::result::Result<(), CredentialBackendError> {
        let _guard = KEYRING_LOCK.lock().map_err(|_| CredentialBackendError)?;
        match self.entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(CredentialBackendError),
        }
    }
}
/// 记住凭据须来自用户单独的保存选择；调用失败只能降级为明示的会话内存。
pub fn prepare(
    vault: &Vault,
    target: MemoryProviderConfig,
    key: String,
    storage: CredentialStorage,
) -> AppResult<PreparedCredential> {
    validate_target(&target)?;
    if storage == CredentialStorage::SessionOnly {
        return PreparedCredential::session(target, key);
    }
    let backend = OsKeyringBackend::new(vault, &target).ok();
    prepare_with_backend(
        target,
        key,
        storage,
        backend
            .as_ref()
            .map(|backend| backend as &dyn CredentialBackend),
    )
}
pub fn prepare_with_backend(
    target: MemoryProviderConfig,
    key: String,
    storage: CredentialStorage,
    backend: Option<&dyn CredentialBackend>,
) -> AppResult<PreparedCredential> {
    validate_target(&target)?;
    let secret = Secret::from_user_input(key)?;
    let status = match storage {
        CredentialStorage::SessionOnly => CredentialStatus::session(false),
        CredentialStorage::OsProtected => {
            if let Some(backend) = backend {
                match backend.load() {
                    Err(_) => CredentialStatus::session(true),
                    Ok(previous) => {
                        let _previous = previous.map(Zeroizing::new);
                        if backend.store(secret.expose()).is_err() {
                            return Err(AppError::new(
                                ErrorCode::ModelUnavailable,
                                "系统凭据保存状态未确认；未自动重试",
                                "请在系统凭据管理器核对后再决定；不会声称凭据只在内存",
                            ));
                        }
                        match backend.load() {
                            Ok(Some(read_back)) => {
                                let read_back = Zeroizing::new(read_back);
                                if read_back.as_str() != secret.expose() {
                                    return Err(unavailable());
                                }
                                CredentialStatus::protected()
                            }
                            _ => {
                                return Err(AppError::new(
                                    ErrorCode::ModelUnavailable,
                                    "系统可能已经保存凭据，但读回未确认",
                                    "请在系统凭据管理器核对；未自动重复保存",
                                ))
                            }
                        }
                    }
                }
            } else {
                CredentialStatus::session(true)
            }
        }
        _ => return Err(invalid()),
    };
    let status = status.for_provider(&target);
    Ok(PreparedCredential {
        target,
        status,
        secret,
    })
}
pub fn load_os(
    vault: &Vault,
    target: &MemoryProviderConfig,
) -> AppResult<Option<PreparedCredential>> {
    let backend = OsKeyringBackend::new(vault, target)?;
    match backend.load() {
        Ok(Some(key)) => Ok(Some(PreparedCredential {
            target: target.clone(),
            status: CredentialStatus::protected().for_provider(target),
            secret: Secret::from_user_input(key)?,
        })),
        Ok(None) => Ok(None),
        Err(_) => Err(unavailable()),
    }
}
pub fn status(vault: &Vault, target: Option<&MemoryProviderConfig>) -> CredentialStatus {
    let Some(target) = target else {
        return CredentialStatus::default();
    };
    match load_os(vault, target) {
        Ok(Some(credential)) => credential.status,
        Ok(None) => CredentialStatus {
            os_protected_available: Some(true),
            ..Default::default()
        },
        Err(_) => CredentialStatus {
            os_protected_available: Some(false),
            message: "系统凭据存储不可用；可明确选择仅本次后台服务会话".into(),
            ..Default::default()
        },
    }
}
pub fn forget_os(vault: &Vault, target: &MemoryProviderConfig) -> AppResult<()> {
    OsKeyringBackend::new(vault, target)?
        .remove()
        .map_err(|_| unavailable())
}

/// 只写入新建子进程的匿名管道。调用者不得把 writer 换成文件或日志。
pub fn write_handoff(writer: &mut impl Write, credential: &PreparedCredential) -> AppResult<()> {
    #[derive(Serialize)]
    struct Handoff<'a> {
        schema: &'static str,
        target: &'a MemoryProviderConfig,
        storage: CredentialStorage,
        secret: &'a str,
    }
    serde_json::to_writer(
        &mut *writer,
        &Handoff {
            schema: HANDOFF_SCHEMA,
            target: &credential.target,
            storage: credential.status.storage,
            secret: credential.secret.expose(),
        },
    )
    .map_err(|_| unavailable())?;
    writer.flush().map_err(|_| unavailable())
}
/// 只读启动时继承的 stdin，最多 16 KiB，不接受路径或明文参数。
pub fn read_handoff(reader: impl Read) -> AppResult<PreparedCredential> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Handoff {
        schema: String,
        target: MemoryProviderConfig,
        storage: CredentialStorage,
        secret: String,
    }
    let mut bytes = Zeroizing::new(Vec::new());
    reader
        .take(MAX_HANDOFF + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid())?;
    if bytes.len() as u64 > MAX_HANDOFF {
        return Err(invalid());
    }
    let handoff: Handoff = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let secret = Secret::from_user_input(handoff.secret)?;
    if handoff.schema != HANDOFF_SCHEMA {
        return Err(invalid());
    }
    validate_target(&handoff.target)?;
    let status = match handoff.storage {
        CredentialStorage::SessionOnly => CredentialStatus::session(false),
        CredentialStorage::OsProtected => CredentialStatus::protected(),
        _ => return Err(invalid()),
    };
    let status = status.for_provider(&handoff.target);
    Ok(PreparedCredential {
        target: handoff.target,
        status,
        secret,
    })
}
fn invalid() -> AppError {
    AppError::new(
        ErrorCode::InvalidRequest,
        "凭据输入或安全交接格式无效；未记录输入内容",
        "在应用密码字段重新输入；不要把密钥放进配置文件或命令行参数",
    )
}
fn unavailable() -> AppError {
    AppError::new(
        ErrorCode::ModelUnavailable,
        "操作系统保护的凭据存储不可用或尚未解锁",
        "解锁系统凭据存储，或明确选择仅本次后台服务会话",
    )
}

fn validate_target(target: &MemoryProviderConfig) -> AppResult<()> {
    super::background_memory::MemoryConfig {
        provider: Some(target.clone()),
        ..Default::default()
    }
    .validate()
}

pub fn provider_ref(target: &MemoryProviderConfig) -> String {
    hash(&serde_json::to_vec(target).expect("固定供应商字段可序列化"))
}
