//! 共用持久运行进程：导入与记忆整理同属一个服务租约，不依赖窗口生命周期。
use super::{
    background_memory::{MemoryProgress, MemoryProvider, MemoryRuntime, PythonMemoryProvider},
    AppError, AppResult, ErrorCode, ImportService, JobStatus,
};
use crate::{
    filesystem::{file_identity, open_local_file},
    vault::{read_json, reject_symlink},
    Vault,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const SCHEMA: &str = "recallcard.local-service/1";
/// 启动包含本地 worker 预检与完整程序摘要校验。慢 CPU 上的 debug 程序
/// 可能超过 5 秒；持有租约不表示上述工作已完成。调用方共用同一有界预算。
pub const SERVICE_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
#[derive(Debug, Clone, Copy)]
pub struct ServiceOptions {
    pub once: bool,
    pub poll_interval_ms: u64,
}
impl Default for ServiceOptions {
    fn default() -> Self {
        Self {
            once: false,
            poll_interval_ms: 1000,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub schema: String,
    #[serde(default)]
    pub build: serde_json::Value,
    #[serde(default)]
    pub binary_hash: String,
    #[serde(default)]
    pub credential: super::credentials::CredentialStatus,
    #[serde(default)]
    pub provider_ready: bool,
    pub running: bool,
    /// 已持久记录停止请求；running 仍由 OS 租约表示是否到达停稳边界。
    #[serde(default)]
    pub stop_requested: bool,
    pub pid: Option<u32>,
    pub started_at: Option<DateTime<Utc>>,
    pub heartbeat_at: Option<DateTime<Utc>>,
    pub imports_pending: bool,
    pub last_memory_job: Option<JobStatus<MemoryProgress>>,
    pub error: Option<AppError>,
}
impl Default for ServiceStatus {
    fn default() -> Self {
        Self {
            schema: SCHEMA.into(),
            build: serde_json::json!(crate::build_info()),
            binary_hash: String::new(),
            credential: Default::default(),
            provider_ready: false,
            running: false,
            stop_requested: false,
            pid: None,
            started_at: None,
            heartbeat_at: None,
            imports_pending: false,
            last_memory_job: None,
            error: None,
        }
    }
}
impl ServiceStatus {
    /// 仅表示本实例已发布启动信息；不代表模型已配置或远程调用成功。
    /// 真正的启动握手还必须核对期望的程序摘要和本次启动时间。
    pub fn startup_published(&self) -> bool {
        self.running
            && self.pid.is_some()
            && self.started_at.is_some()
            && self.heartbeat_at.is_some()
            && self.binary_hash.len() == 64
            && self
                .binary_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    }
}
struct ServiceLease(File);
impl Drop for ServiceLease {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
fn storage() -> AppError {
    AppError::new(
        ErrorCode::Storage,
        "本机服务状态无法安全读取或保存",
        "检查本机状态目录；原始资料未被发送或删除",
    )
}
fn directory(vault: &Vault) -> AppResult<PathBuf> {
    let directory = vault.state_dir().map_err(|_| storage())?.join("runtime");
    reject_symlink(&directory).map_err(|_| storage())?;
    fs::create_dir_all(&directory).map_err(|_| storage())?;
    Ok(directory)
}
fn try_lease(path: &Path, shared: bool) -> AppResult<Option<ServiceLease>> {
    reject_symlink(path).map_err(|_| storage())?;
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| storage())?;
    if !file.metadata().map_err(|_| storage())?.is_file() {
        return Err(storage());
    }
    let acquired = if shared {
        file.try_lock_shared()
    } else {
        file.try_lock()
    };
    match acquired {
        Ok(()) => Ok(Some(ServiceLease(file))),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(_) => Err(storage()),
    }
}
fn read_status(vault: &Vault) -> AppResult<ServiceStatus> {
    let path = directory(vault)?.join("status.json");
    reject_symlink(&path).map_err(|_| storage())?;
    if !path.exists() {
        return Ok(ServiceStatus::default());
    }
    if fs::metadata(&path).map_err(|_| storage())?.len() > 128 * 1024 {
        return Err(storage());
    }
    let status: ServiceStatus = read_json(&path).map_err(|_| storage())?;
    if status.schema != SCHEMA {
        return Err(storage());
    }
    Ok(status)
}
fn save_status(vault: &Vault, status: &ServiceStatus) -> AppResult<()> {
    vault
        .write_replace(&directory(vault)?.join("status.json"), status)
        .map_err(|_| storage())
}
/// running 由实际文件租约确认，旧 pid/旧心跳不被当作当前进程。
pub fn service_status(vault: &Vault) -> AppResult<ServiceStatus> {
    let mut status = read_status(vault)?;
    // 读者之间共享探测；只有服务持有的独占租约会使此探测失败。
    status.running = try_lease(&directory(vault)?.join("service.lock"), true)?.is_none();
    let stop = directory(vault)?.join("stop.json");
    reject_symlink(&stop).map_err(|_| storage())?;
    status.stop_requested = stop.exists();
    if !status.running {
        status.pid = None;
        status.provider_ready = false;
        if matches!(
            status.credential.storage,
            super::credentials::CredentialStorage::SessionOnly
                | super::credentials::CredentialStorage::Environment
        ) {
            status.credential.present = false;
            status.credential.message = "后台服务会话已结束；会话凭据需要重新提供".into();
        }
    }
    Ok(status)
}

struct ImportScheduler {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}
impl ImportScheduler {
    fn start(service: ImportService, interval: Duration) -> AppResult<Self> {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::Builder::new()
            .name("recallcard-import-scheduler".into())
            .spawn(move || {
                while !flag.load(std::sync::atomic::Ordering::Acquire) {
                    let _ = service.start_worker();
                    std::thread::sleep(interval.min(Duration::from_millis(500)));
                }
            })
            .map_err(|_| storage())?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }
}
impl Drop for ImportScheduler {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// 调用方须是已授权本机入口。模型读取协议不导出本函数。
pub fn request_service_stop(vault: &Vault) -> AppResult<()> {
    vault
        .write_replace(
            &directory(vault)?.join("stop.json"),
            &serde_json::json!({"requested_at":Utc::now()}),
        )
        .map_err(|_| storage())?;
    MemoryRuntime::new(vault).pause()?;
    ImportService::new(vault)?.pause_all_pending()?;
    Ok(())
}

/// 服务进程负责保持导入线程存活；记忆 tick 等待网络时导入仍可继续。
/// once 等待已排队导入落到稳定边界，并运行一个记忆批次，不建立永久循环。
pub fn run_service<P: MemoryProvider + ?Sized>(
    vault: &Vault,
    provider: &mut P,
    options: ServiceOptions,
) -> AppResult<ServiceStatus> {
    if !(10..=60_000).contains(&options.poll_interval_ms) {
        return Err(AppError::new(
            ErrorCode::InvalidRequest,
            "服务轮询间隔必须为 10–60000 毫秒",
            "使用默认服务配置",
        ));
    }
    let dir = directory(vault)?;
    let until = Instant::now() + SERVICE_STARTUP_TIMEOUT;
    let _lease = loop {
        if let Some(lease) = try_lease(&dir.join("service.lock"), false)? {
            break lease;
        }
        let existing = service_status(vault)?;
        if existing.running {
            return Ok(existing);
        }
        // 独占失败而共享探测成功，只代表有状态读者；不能因此退出启动进程。
        if Instant::now() >= until {
            let mut error = AppError::new(
                ErrorCode::Conflict,
                "状态读取尚未释放租约，后台服务未启动",
                "等待状态读取结束后重试；没有启动重复服务",
            );
            error.retryable = true;
            return Err(error);
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let started = Utc::now();
    let mut status = ServiceStatus {
        running: true,
        pid: Some(std::process::id()),
        started_at: Some(started),
        heartbeat_at: Some(started),
        ..Default::default()
    };
    // 取得租约后立即清除旧进程身份，再恢复事务、预检 worker 与核验完整 hash。
    // 空 hash 表示尚在初始化，不能被当成与现有发行物版本不同。
    save_status(vault, &status)?;
    let stop = dir.join("stop.json");
    reject_symlink(&stop).map_err(|_| storage())?;
    if stop.exists() {
        fs::remove_file(&stop).map_err(|_| storage())?;
    }
    let root_identity = file_identity(&open_local_file(vault.root()).map_err(|_| storage())?)
        .map_err(|_| storage())?;
    let marker = file_identity(
        &open_local_file(&vault.root().join("control/schema-version.json"))
            .map_err(|_| storage())?,
    )
    .map_err(|_| storage())?;
    // 仅恢复已经持久批准的 Dream 提交；不会把候选结果当成新授权。
    if vault
        .state_dir()
        .map_err(|_| storage())?
        .join("dream-transaction.json")
        .exists()
    {
        vault.dream_recover().map_err(|_| {
            AppError::new(
                ErrorCode::Conflict,
                "已批准的记忆事务尚未安全恢复",
                "保留事务和来源，检查后恢复同一资料库",
            )
        })?;
    }
    let imports = ImportService::new(vault)?;
    let scheduler = ImportScheduler::start(
        imports.clone(),
        Duration::from_millis(options.poll_interval_ms),
    )?;
    let configuration = MemoryRuntime::new(vault).configuration()?;
    let readiness = if configuration.enabled && configuration.consent.is_some() {
        provider.available(&configuration)
    } else {
        Ok(())
    };
    status.binary_hash = executable_hash(&std::env::current_exe().map_err(|_| storage())?)?;
    status.credential = provider.credential_status(&configuration);
    status.provider_ready =
        configuration.enabled && configuration.consent.is_some() && readiness.is_ok();
    status.error = readiness.err();
    save_status(vault, &status)?;
    let mut memory_ran = false;
    loop {
        if file_identity(&open_local_file(vault.root()).map_err(|_| storage())?)
            .map_err(|_| storage())?
            != root_identity
            || file_identity(
                &open_local_file(&vault.root().join("control/schema-version.json"))
                    .map_err(|_| storage())?,
            )
            .map_err(|_| storage())?
                != marker
        {
            status.error = Some(AppError::new(
                ErrorCode::SourceChanged,
                "运行中的资料库身份改变，服务已停止",
                "重新打开并确认当前资料库",
            ));
            break;
        }
        if stop.exists() {
            imports.pause_all_pending()?;
            break;
        }
        status.error = None;
        if let Err(error) = imports.start_worker() {
            status.error = Some(error);
        }
        status.imports_pending = imports.has_pending_work()?;
        // once 首先让输入来源完成提交，避免在空库只 tick 一次后漏掉新导入。
        if !status.imports_pending && (!options.once || !memory_ran) {
            match MemoryRuntime::new(vault).tick(provider) {
                Ok(Some(job)) => status.last_memory_job = Some(job),
                Ok(None) => {}
                Err(error) => {
                    // 短暂写锁忙碌交由下一轮；不会变成永久模型失败或打印正文。
                    if error.code != ErrorCode::Conflict {
                        status.error = Some(error);
                    }
                }
            }
            memory_ran = true;
        }
        status.credential = provider.credential_status(&MemoryRuntime::new(vault).configuration()?);
        status.heartbeat_at = Some(Utc::now());
        status.imports_pending = imports.has_pending_work()?;
        save_status(vault, &status)?;
        if options.once && !status.imports_pending && memory_ran {
            break;
        }
        std::thread::sleep(Duration::from_millis(options.poll_interval_ms));
    }
    drop(scheduler);
    // 导入运行在本进程；停止前等待其本批落盘，避免让窗口关闭变成隐式中断。
    while imports.has_pending_work()? {
        std::thread::sleep(Duration::from_millis(options.poll_interval_ms));
    }
    status.running = false;
    status.stop_requested = stop.exists();
    status.pid = None;
    status.heartbeat_at = Some(Utc::now());
    save_status(vault, &status)?;
    Ok(status)
}

/// 发行物只从自身目录选择 worker 代码；开发构建允许当前源码树的已编译路径。
pub fn default_python_provider() -> PythonMemoryProvider {
    let executable = std::env::current_exe().ok();
    let installed = executable
        .as_ref()
        .and_then(|path| path.parent())
        .map(|path| path.join("python"))
        .unwrap_or_else(|| PathBuf::from("/unavailable/recallcard-python"));
    #[cfg(debug_assertions)]
    let python_path = {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../python");
        if source.join("recallcard_dream/runtime.py").is_file() {
            source.canonicalize().unwrap_or(source)
        } else {
            installed
        }
    };
    #[cfg(not(debug_assertions))]
    let python_path = installed;
    PythonMemoryProvider::new(
        if cfg!(windows) {
            "python".into()
        } else {
            "python3".into()
        },
        python_path,
    )
}

/// 优先使用当前设置指定的系统 keyring；缺失时保持 needs_input，不自动生成或共享凭据。
pub fn default_python_provider_for(vault: &Vault) -> AppResult<PythonMemoryProvider> {
    let config = MemoryRuntime::new(vault).configuration()?;
    let mut provider = default_python_provider();
    match config.credential_storage {
        Some(super::credentials::CredentialStorage::SessionOnly) => {
            provider = provider.without_environment()
        }
        Some(super::credentials::CredentialStorage::OsProtected) => {
            provider = provider.without_environment();
            if let Some(target) = config.provider.as_ref() {
                if let Ok(Some(credential)) = super::credentials::load_os(vault, target) {
                    provider = provider.with_credential(credential);
                }
            }
        }
        _ => {}
    }
    Ok(provider)
}
fn executable_hash(path: &Path) -> AppResult<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut reader = open_local_file(path).map_err(|_| storage())?;
    if !reader.metadata().map_err(|_| storage())?.is_file() {
        return Err(storage());
    }
    let mut hash = Sha256::new();
    let mut bytes = [0_u8; 64 * 1024];
    loop {
        let n = reader.read(&mut bytes).map_err(|_| storage())?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
static PACKAGED_PROGRAMS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::BTreeMap<PathBuf, String>>,
> = std::sync::OnceLock::new();
/// 仅由 Tauri 的内部 resource_dir 解析结果调用，不得接受前端提供的路径。
/// 只认可发行物固定子路径，并记录本次读到的程序摘要；交接密钥前再核验摘要。
pub fn register_packaged_cli(candidate: &Path, resource_dir: &Path) -> AppResult<PathBuf> {
    let denied = || {
        AppError::new(
            ErrorCode::PermissionDenied,
            "无法核验随包后台组件的固定位置",
            "使用完整发行物；不接受外部指定的可执行文件",
        )
    };
    if !resource_dir.is_absolute() || !candidate.is_absolute() {
        return Err(denied());
    }
    reject_symlink(resource_dir).map_err(|_| denied())?;
    reject_symlink(candidate).map_err(|_| denied())?;
    let expected = resource_dir.join("说明与许可证").join(if cfg!(windows) {
        "recallcard.exe"
    } else {
        "recallcard"
    });
    let legacy_windows = resource_dir.join("说明与许可证/recallcard");
    if candidate != expected && !(cfg!(windows) && candidate == legacy_windows) {
        return Err(denied());
    }
    if !candidate.is_file() {
        return Err(denied());
    }
    let path = candidate.canonicalize().map_err(|_| denied())?;
    let digest = executable_hash(&path)?;
    PACKAGED_PROGRAMS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| denied())?
        .insert(path.clone(), digest);
    Ok(path)
}
/// 仅启动同一安装目录的 CLI，并核验运行中的二进制身份和新心跳。
pub fn ensure_service(vault: &Vault, trusted_cli: &Path) -> AppResult<ServiceStatus> {
    ensure_service_inner(vault, trusted_cli, None)
}
/// 正常 GUI 密码入口：凭据仅走新进程匿名 stdin，不落盘、不进入参数或日志。
/// 更新凭据时先停止旧服务，等待当前边界；关闭 GUI 不会清除服务中的会话凭据。
pub fn ensure_service_with_credential(
    vault: &Vault,
    trusted_cli: &Path,
    credential: super::credentials::PreparedCredential,
) -> AppResult<ServiceStatus> {
    ensure_service_inner(vault, trusted_cli, Some(credential))
}
fn ensure_service_inner(
    vault: &Vault,
    trusted_cli: &Path,
    credential: Option<super::credentials::PreparedCredential>,
) -> AppResult<ServiceStatus> {
    let error = || {
        AppError::new(
            ErrorCode::ModelUnavailable,
            "后台服务尚未运行，未找到可验证的同目录 RecallCard CLI",
            "使用完整发行物或运行 recallcard start --foreground；配置已保留",
        )
    };
    let current = std::env::current_exe().map_err(|_| error())?;
    let parent = current.parent().ok_or_else(error)?;
    reject_symlink(trusted_cli).map_err(|_| error())?;
    let expected = if cfg!(windows) {
        "recallcard.exe"
    } else {
        "recallcard"
    };
    let registered = PACKAGED_PROGRAMS.get().and_then(|registry| {
        registry
            .lock()
            .ok()
            .and_then(|registry| registry.get(trusted_cli).cloned())
    });
    let adjacent = trusted_cli.file_name().and_then(|s| s.to_str()) == Some(expected)
        && trusted_cli.parent() == Some(parent);
    if !trusted_cli.is_absolute() || !trusted_cli.is_file() || (!adjacent && registered.is_none()) {
        return Err(error());
    }
    let expected_hash = executable_hash(trusted_cli)?;
    if registered.is_some_and(|registered| registered != expected_hash) {
        return Err(AppError::new(
            ErrorCode::SourceChanged,
            "随包后台组件在核验后改变；未交接凭据",
            "重新打开完整发行物并核验，不会运行被替换的文件",
        ));
    }
    let mut existing = service_status(vault)?;
    let deadline = Instant::now() + SERVICE_STARTUP_TIMEOUT;
    while existing.running && !existing.startup_published() {
        if Instant::now() >= deadline {
            let mut error = AppError::new(
                ErrorCode::Conflict,
                "已有服务正在初始化，尚未发布可核验的程序身份",
                "稍后查看服务状态；未替换服务或交接新的凭据",
            );
            error.retryable = true;
            return Err(error);
        }
        std::thread::sleep(Duration::from_millis(20));
        existing = service_status(vault)?;
    }
    if existing.running {
        if existing.binary_hash != expected_hash {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "运行中的后台程序与当前发行物不同",
                "先停止旧服务，再用当前发行物启动；不会静默混用版本",
            ));
        }
        if credential.is_none() {
            return Ok(existing);
        }
        let config = MemoryRuntime::new(vault).configuration()?;
        request_service_stop(vault)?;
        let until = Instant::now() + Duration::from_secs(125);
        while service_status(vault)?.running {
            if Instant::now() >= until {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "旧服务尚未到达安全停止边界；新凭据未交接",
                    "等待停止后重新提供凭据；没有自动重复启动或保存明文",
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        MemoryRuntime::new(vault).configure(config)?;
    }
    let mut command = Command::new(trusted_cli);
    command
        .arg("--vault")
        .arg(vault.root())
        .args(["start", "--foreground"])
        .stdin(if credential.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env_clear();
    if credential.is_some() {
        command.arg("--credential-stdin");
    }
    for name in [
        "PATH",
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "HOME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "XDG_STATE_HOME",
        "XDG_DATA_HOME",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XAUTHORITY",
        "RECALLCARD_STATE_DIR",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    if credential.is_none() {
        if let Some(value) = std::env::var_os("RECALLCARD_DREAM_API_KEY") {
            command.env("RECALLCARD_DREAM_API_KEY", value);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let launched_at = Utc::now();
    let mut child = command.spawn().map_err(|_| {
        AppError::new(
            ErrorCode::ModelUnavailable,
            "已核验后台程序，但无法启动服务进程",
            "检查程序执行权限或运行 start --foreground；配置已保留",
        )
    })?;
    if let Some(credential) = credential {
        let result = child
            .stdin
            .take()
            .ok_or_else(error)
            .and_then(|mut pipe| super::credentials::write_handoff(&mut pipe, &credential));
        if result.is_err() {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return Err(AppError::new(
                ErrorCode::ModelUnavailable,
                "服务凭据交接尚未确认；未重试或保存明文",
                "查看服务状态后再决定；不要重复提交密钥",
            ));
        }
    }
    let deadline = Instant::now() + SERVICE_STARTUP_TIMEOUT;
    loop {
        let status = service_status(vault)?;
        if status.startup_published()
            && status.pid == Some(child.id())
            && status.binary_hash == expected_hash
            && status.heartbeat_at.is_some_and(|at| at >= launched_at)
        {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return Ok(status);
        }
        if let Some(exit) = child.try_wait().map_err(|_| {
            AppError::new(
                ErrorCode::Storage,
                "无法读取已启动服务进程的退出状态",
                "查看实际服务状态；未重复启动",
            )
        })? {
            return Err(AppError::new(
                ErrorCode::ModelUnavailable,
                format!(
                    "后台进程在完成启动握手前退出（退出码：{}）",
                    exit.code()
                        .map_or_else(|| "无".into(), |code| code.to_string())
                ),
                "运行 start --foreground 查看启动原因；已核验的程序路径没有改变，未重复启动",
            ));
        }
        if Instant::now() >= deadline {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return Err(AppError::new(
                ErrorCode::ModelUnavailable,
                "服务启动尚未确认；未重复启动",
                "稍后检查实际服务状态，避免重复进程",
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSetupResult {
    pub runtime: super::background_memory::MemoryRuntimeStatus,
    pub credential: super::credentials::CredentialStatus,
    pub service: Option<ServiceStatus>,
    pub service_error: Option<AppError>,
}
/// CLI/GUI 共用的设置边界。密钥是瞬态独立参数，不属于可序列化配置。
pub fn configure_provider(
    vault: &Vault,
    mut config: super::background_memory::MemoryConfig,
    api_key: Option<String>,
    storage: Option<super::credentials::CredentialStorage>,
    trusted_cli: Option<&Path>,
) -> AppResult<ProviderSetupResult> {
    use super::credentials::{self, CredentialStatus, CredentialStorage};
    config.validate()?;
    if api_key.is_some() && (!config.enabled || config.paused || config.consent.is_none()) {
        return Err(AppError::new(
            ErrorCode::ConsentRequired,
            "请先确认模型目的地、资料范围、预算和自动应用用途",
            "确认后才保存和交接凭据；目前没有接收来源数据",
        ));
    }
    let prepared = if let Some(key) = api_key {
        let target = config.provider.clone().ok_or_else(|| {
            AppError::new(
                ErrorCode::InvalidRequest,
                "请先选择模型接收地址与名称",
                "凭据不会单独保存到未选择的目的地",
            )
        })?;
        let mode = storage.ok_or_else(|| {
            AppError::new(
                ErrorCode::InvalidRequest,
                "请选择系统保护存储或仅本次服务会话",
                "未保存凭据",
            )
        })?;
        let value = credentials::prepare(vault, target, key, mode)?;
        config.credential_storage = Some(value.status.storage);
        Some(value)
    } else if storage == Some(CredentialStorage::OsProtected) {
        config.credential_storage = Some(CredentialStorage::OsProtected);
        config
            .provider
            .as_ref()
            .and_then(|target| credentials::load_os(vault, target).ok().flatten())
    } else {
        None
    };
    let credential = prepared
        .as_ref()
        .map(|value| value.status.clone())
        .unwrap_or_else(|| {
            if config.credential_storage == Some(CredentialStorage::OsProtected) {
                credentials::status(vault, config.provider.as_ref())
            } else {
                CredentialStatus::default()
            }
        });
    let memory = MemoryRuntime::new(vault);
    let saved = memory.configure(config.clone()).map_err(|error| {
        if credential.storage == CredentialStorage::OsProtected {
            AppError::new(
                error.code,
                "凭据已保存到系统，但资料库配置保存失败",
                "系统中的凭据不会自动删除；检查本机状态目录后再继续",
            )
        } else {
            error
        }
    })?;
    let mut output = ProviderSetupResult {
        runtime: saved,
        credential,
        service: None,
        service_error: None,
    };
    if !config.enabled || config.paused || config.consent.is_none() {
        return Ok(output);
    }
    let Some(binary) = trusted_cli else {
        output.service_error = Some(AppError::new(
            ErrorCode::ModelUnavailable,
            "设置已保存，但缺少可信后台程序，未启动模型服务",
            "使用完整发行物；本次会话凭据未交给服务，需重新提供",
        ));
        if output.credential.storage == CredentialStorage::SessionOnly {
            output.credential.present = false;
        }
        return Ok(output);
    };
    let service = if let Some(credential) = prepared {
        ensure_service_with_credential(vault, binary, credential)
    } else {
        ensure_service(vault, binary)
    };
    match service {
        Ok(service) => {
            if config.provider.as_ref().is_some_and(|target| {
                service.credential.provider_ref.as_deref()
                    == Some(credentials::provider_ref(target).as_str())
                    && config
                        .credential_storage
                        .is_none_or(|storage| service.credential.storage == storage)
            }) {
                output.credential = service.credential.clone();
            }
            output.service = Some(service);
        }
        Err(error) => {
            output.service_error = Some(error);
            if output.credential.storage == CredentialStorage::SessionOnly {
                output.credential.present = false;
            }
        }
    }
    output.runtime = memory.status_for_scope(&config.scope)?;
    Ok(output)
}
