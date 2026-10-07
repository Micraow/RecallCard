//! 仅限本机的只读核心 IPC；权限、Vault 与端点由启动者绑定。
use crate::{
    context::Context, model::Result, policy::Access, semantic::SemanticSearch, transport, Vault,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    runtime::{Builder, Runtime},
    sync::{OwnedSemaphorePermit, Semaphore},
    time::timeout,
};

pub const PROTOCOL: &str = "recallcard.ipc/1";
pub const MAX_FRAME_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct Options {
    /// 整个读取帧、执行与写回的期限，而非每个字节单独重置。
    pub request_timeout: Duration,
    /// 包括已超时但尚未结束的本机只读计算。
    pub max_connections: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(5),
            max_connections: 16,
        }
    }
}
impl Options {
    fn validate(&self) -> Result<()> {
        validate_timeout(self.request_timeout)?;
        if !(1..=64).contains(&self.max_connections) {
            return Err("IPC 并发上限必须介于 1 与 64".into());
        }
        Ok(())
    }
}
fn validate_timeout(duration: Duration) -> Result<()> {
    if !(Duration::from_millis(10)..=Duration::from_secs(120)).contains(&duration) {
        return Err("IPC 超时必须介于 10 与 120000 毫秒".into());
    }
    Ok(())
}
fn runtime() -> Result<Runtime> {
    Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(16)
        .build()
        .map_err(|e| format!("无法创建 IPC 运行环境：{e}"))
}
fn binding(vault: &Vault, access: &Access) -> String {
    // 保留非 UTF-8 本机路径的身份；路径从来不在 IPC 消息中传输。
    let mut bytes = vault.root().as_os_str().as_encoded_bytes().to_vec();
    bytes.push(0);
    bytes.extend(serde_json::json!(access.scopes()).to_string().as_bytes());
    crate::model::hash(&bytes)
}

/// 端点只由本机调用者计算；不同 Vault 或 scope 集合使用不同端点。
/// Unix 返回路径但不创建目录；过长路径应由本机配置显式指定较短端点。
pub fn default_endpoint(vault: &Vault, access: &Access) -> Result<PathBuf> {
    let key = binding(vault, access);
    #[cfg(unix)]
    {
        let base = std::env::var_os("RECALLCARD_STATE_DIR")
            .map(PathBuf::from)
            .or_else(dirs::runtime_dir)
            .or_else(dirs::state_dir)
            .or_else(dirs::data_local_dir)
            .ok_or("无法确定 IPC 本机目录，请显式指定端点")?;
        // 系统默认目录可能经 /var、/tmp 别名；先固定真实祖先，绑定时仍逐级拒绝链接。
        let existing = base
            .ancestors()
            .find(|path| path.exists())
            .ok_or("IPC 状态目录没有可用祖先")?;
        let canonical = std::fs::canonicalize(existing)
            .map_err(|e| e.to_string())?
            .join(base.strip_prefix(existing).map_err(|e| e.to_string())?);
        Ok(canonical
            .join("recallcard-ipc")
            .join(format!("{}.sock", &key[..24])))
    }
    #[cfg(windows)]
    {
        let owner = crate::model::hash(platform::current_user_sid()?.as_bytes());
        Ok(PathBuf::from(format!(
            r"\\.\pipe\recallcard-{}-{}",
            &owner[..16],
            &key[..24]
        )))
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    protocol: String,
    binding: String,
    request_id: String,
    method: String,
    arguments: Value,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    protocol: String,
    binding: String,
    request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}
impl Response {
    fn new(binding: &str, request_id: String, result: Result<Value>) -> Self {
        let (result, error) = match result {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error)),
        };
        Self {
            protocol: PROTOCOL.into(),
            binding: binding.into(),
            request_id,
            result,
            error,
        }
    }
}
fn check_method(method: &str) -> Result<()> {
    if !["bootstrap", "search", "read", "sources"].contains(&method) {
        return Err("IPC 仅允许 bootstrap/search/read/sources 四个只读操作".into());
    }
    Ok(())
}
fn check_request(request: &Request, expected: &str) -> Result<()> {
    if request.protocol != PROTOCOL || request.binding != expected {
        return Err("IPC 协议或 Vault/scope 绑定不匹配".into());
    }
    if request.request_id.is_empty()
        || request.request_id.len() > 64
        || !request
            .request_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("IPC 请求编号无效".into());
    }
    check_method(&request.method)?;
    if !request.arguments.is_object() {
        return Err("IPC 参数必须为 JSON 对象".into());
    }
    Ok(())
}

/// 由可信本机配置构造；同一 OS 用户属于同一个信任域，不是应用级认证令牌。
#[derive(Debug, Clone)]
pub struct Client {
    endpoint: PathBuf,
    binding: String,
    request_timeout: Duration,
}
impl Client {
    pub fn new(
        vault: &Vault,
        access: &Access,
        endpoint: PathBuf,
        request_timeout: Duration,
    ) -> Result<Self> {
        validate_timeout(request_timeout)?;
        platform::validate_endpoint(&endpoint)?;
        Ok(Self {
            endpoint,
            binding: binding(vault, access),
            request_timeout,
        })
    }
    pub fn invoke(&self, method: &str, arguments: Value) -> Result<Value> {
        check_method(method)?;
        let request = Request {
            protocol: PROTOCOL.into(),
            binding: self.binding.clone(),
            request_id: uuid::Uuid::new_v4().to_string(),
            method: method.into(),
            arguments,
        };
        check_request(&request, &self.binding)?;
        let bytes = encode(&request)?;
        let runtime = runtime()?;
        runtime.block_on(async {
            timeout(self.request_timeout, async {
                let mut stream = platform::connect(&self.endpoint).await?;
                write_frame(&mut stream, &bytes).await?;
                let body = read_frame(&mut stream).await?;
                // 确认已收完整帧，让 Windows 服务端安全关闭实例而不丢弃未读缓冲。
                stream.write_all(&[0x06]).await.map_err(|e| e.to_string())?;
                let response: Response =
                    serde_json::from_slice(&body).map_err(|_| "IPC 响应 JSON/schema 无效")?;
                if response.protocol != PROTOCOL
                    || response.binding != self.binding
                    || response.request_id != request.request_id
                {
                    return Err("IPC 响应版本、绑定或请求编号不匹配".into());
                }
                match (response.result, response.error) {
                    (Some(value), None) => Ok(value),
                    (None, Some(error)) => Err(error),
                    _ => Err("IPC 响应必须且只能包含一个结果或错误".into()),
                }
            })
            .await
            .map_err(|_| "IPC 请求超时；未返回任何检索结果".to_string())?
        })
    }
}

/// bind 成功即持有单实例端点。run 持续服务；run_until 用于受控关闭与测试。
pub struct Server {
    runtime: Runtime,
    listener: platform::Listener,
    guard: platform::EndpointGuard,
    vault: Arc<Vault>,
    access: Access,
    binding: String,
    options: Options,
    semantic: Option<Arc<SemanticSearch>>,
}
impl Server {
    pub fn bind(vault: Vault, access: Access, endpoint: PathBuf, options: Options) -> Result<Self> {
        options.validate()?;
        platform::validate_endpoint(&endpoint)?;
        let runtime = runtime()?;
        let (listener, guard) = runtime.block_on(platform::bind(&endpoint))?;
        Ok(Self {
            runtime,
            listener,
            guard,
            binding: binding(&vault, &access),
            vault: Arc::new(vault),
            access,
            options,
            semantic: None,
        })
    }
    /// 可选后端只能在本机启动阶段绑定；IPC 请求不能启动或配置 worker。
    pub fn with_semantic(mut self, semantic: Arc<SemanticSearch>) -> Self {
        self.semantic = Some(semantic);
        self
    }
    pub fn run(self) -> Result<()> {
        self.run_until(Arc::new(AtomicBool::new(false)))
    }
    pub fn run_until(self, stop: Arc<AtomicBool>) -> Result<()> {
        let Self {
            runtime,
            mut listener,
            guard: _guard,
            vault,
            access,
            binding,
            options,
            semantic,
        } = self;
        let permits = Arc::new(Semaphore::new(options.max_connections));
        let result = runtime.block_on(async {
            while !stop.load(Ordering::Acquire) {
                let accepted = match timeout(Duration::from_millis(100), listener.accept()).await {
                    Err(_) => continue,
                    Ok(value) => value?,
                };
                let permit = match permits.clone().try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => {
                        drop(accepted);
                        continue;
                    }
                };
                let (vault, access, binding) = (vault.clone(), access.clone(), binding.clone());
                let semantic = semantic.clone();
                let duration = options.request_timeout;
                tokio::spawn(async move {
                    // 丢弃错误与超时连接；不记录请求正文、记忆内容或模型参数。
                    let _ = timeout(
                        duration,
                        handle(accepted, vault, access, binding, permit, semantic),
                    )
                    .await;
                });
            }
            Ok(())
        });
        drop(listener);
        runtime.shutdown_timeout(options.request_timeout);
        result
    }
}
async fn handle<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    vault: Arc<Vault>,
    access: Access,
    binding: String,
    permit: OwnedSemaphorePermit,
    semantic: Option<Arc<SemanticSearch>>,
) -> Result<()> {
    // 写回结束前保留连接名额；超时后若计算仍在运行，它的副本继续持有名额。
    let permit = Arc::new(permit);
    let body = read_frame(&mut stream).await?;
    let request: Request = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(_) => {
            return write_response(
                &mut stream,
                &encode(&Response::new(
                    &binding,
                    String::new(),
                    Err("IPC 请求 JSON/schema 无效".into()),
                ))?,
            )
            .await
        }
    };
    let request_id = request.request_id.clone();
    let result = match check_request(&request, &binding) {
        Err(error) => Err(error),
        Ok(()) => {
            let computation_permit = permit.clone();
            tokio::task::spawn_blocking(move || {
                // 如果客户端超时，permit 仍由未结束的计算持有，避免后台任务无限增长。
                let _permit = computation_permit;
                let context = match semantic.as_deref() {
                    Some(semantic) => Context::with_semantic(&vault, access, semantic),
                    None => Context::new(&vault, access),
                };
                transport::invoke(&context, &request.method, request.arguments)
            })
            .await
            .map_err(|_| "IPC 本机只读操作异常终止".to_string())?
        }
    };
    let response = Response::new(&binding, request_id.clone(), result);
    let bytes = encode(&response).or_else(|_| {
        encode(&Response::new(
            &binding,
            request_id,
            Err("IPC 响应超过大小上限，请缩小读取预算".into()),
        ))
    })?;
    write_response(&mut stream, &bytes).await
}
fn encode(value: &impl Serialize) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if bytes.is_empty() || bytes.len() > MAX_FRAME_BYTES {
        return Err("IPC 消息超过大小上限".into());
    }
    Ok(bytes)
}
async fn read_frame(stream: &mut (impl AsyncRead + Unpin)) -> Result<Vec<u8>> {
    let mut header = [0u8; 4];
    stream
        .read_exact(&mut header)
        .await
        .map_err(|e| format!("IPC 帧头读取失败：{e}"))?;
    let length = u32::from_le_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err("IPC 消息长度必须介于 1 与 262144 字节".into());
    }
    let mut body = vec![0; length];
    stream
        .read_exact(&mut body)
        .await
        .map_err(|e| format!("IPC 消息读取失败：{e}"))?;
    Ok(body)
}
async fn write_frame(stream: &mut (impl AsyncWrite + Unpin), body: &[u8]) -> Result<()> {
    stream
        .write_all(&(body.len() as u32).to_le_bytes())
        .await
        .map_err(|e| e.to_string())?;
    stream.write_all(body).await.map_err(|e| e.to_string())?;
    stream.flush().await.map_err(|e| e.to_string())
}
async fn write_response(
    stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
    body: &[u8],
) -> Result<()> {
    write_frame(stream, body).await?;
    let mut acknowledgement = [0];
    stream
        .read_exact(&mut acknowledgement)
        .await
        .map_err(|e| format!("IPC 响应确认失败：{e}"))?;
    if acknowledgement != [0x06] {
        return Err("IPC 响应确认字节无效".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn exchange(body: Vec<u8>) -> Response {
        let directory = tempfile::tempdir().unwrap();
        let vault = Arc::new(Vault::init(&directory.path().join("vault")).unwrap());
        let access = Access::new(vec!["personal".into()]).unwrap();
        let key = binding(&vault, &access);
        runtime().unwrap().block_on(async {
            let (mut client, server) = tokio::io::duplex(1024);
            let permit = Arc::new(Semaphore::new(1)).acquire_owned().await.unwrap();
            let task = tokio::spawn(handle(server, vault, access, key, permit, None));
            write_frame(&mut client, &body).await.unwrap();
            let response = serde_json::from_slice(&read_frame(&mut client).await.unwrap()).unwrap();
            client.write_all(&[0x06]).await.unwrap();
            task.await.unwrap().unwrap();
            response
        })
    }
    #[test]
    fn memory_transport_rejects_untrusted_envelope() {
        let response = exchange(br#"{"protocol":"recallcard.ipc/1","binding":"wrong","request_id":"r1","method":"bootstrap","arguments":{}}"#.to_vec());
        assert_eq!(response.request_id, "r1");
        assert!(response.error.unwrap().contains("绑定"));
        let response = exchange(br#"{"shell":"rm"}"#.to_vec());
        assert!(response.result.is_none());
        assert!(response.error.unwrap().contains("schema"));
    }
    #[test]
    fn memory_transport_reads_context_with_acknowledgement() {
        let directory = tempfile::tempdir().unwrap();
        let vault = Arc::new(Vault::init(&directory.path().join("vault")).unwrap());
        let access = Access::new(vec!["personal".into()]).unwrap();
        let key = binding(&vault, &access);
        let request = Request {
            protocol: PROTOCOL.into(),
            binding: key.clone(),
            request_id: "r1".into(),
            method: "bootstrap".into(),
            arguments: json!({}),
        };
        runtime().unwrap().block_on(async {
            let (mut client, server) = tokio::io::duplex(1024);
            let permit = Arc::new(Semaphore::new(1)).acquire_owned().await.unwrap();
            let task = tokio::spawn(handle(server, vault, access, key.clone(), permit, None));
            write_frame(&mut client, &encode(&request).unwrap())
                .await
                .unwrap();
            let response: Response =
                serde_json::from_slice(&read_frame(&mut client).await.unwrap()).unwrap();
            assert_eq!(response.binding, key);
            assert!(response.result.is_some());
            assert!(response.error.is_none());
            client.write_all(&[0x06]).await.unwrap();
            task.await.unwrap().unwrap();
        });
    }
    #[test]
    fn frame_reader_bounds_lengths_and_rejects_partial_body() {
        runtime().unwrap().block_on(async {
            for length in [0u32, MAX_FRAME_BYTES as u32 + 1] {
                assert!(read_frame(&mut &length.to_le_bytes()[..])
                    .await
                    .unwrap_err()
                    .contains("长度"));
            }
            assert!(read_frame(&mut &[1, 0][..]).await.is_err());
            assert!(read_frame(&mut &[3, 0, 0, 0, b'{'][..]).await.is_err());
            assert_eq!(
                read_frame(&mut &[2, 0, 0, 0, b'{', b'}'][..])
                    .await
                    .unwrap(),
                b"{}"
            );
        });
    }
    #[test]
    fn encoded_responses_and_requests_have_same_frame_bound() {
        assert!(encode(&json!({"data":"x".repeat(MAX_FRAME_BYTES)})).is_err());
        assert!(encode(&json!({"data":"small"})).is_ok());
    }
    #[test]
    fn slow_memory_reader_releases_connection_only_after_total_deadline() {
        runtime().unwrap().block_on(async {
            let (mut writer, mut reader) = tokio::io::duplex(32);
            writer.write_all(&[5, 0]).await.unwrap();
            assert!(timeout(Duration::from_millis(20), read_frame(&mut reader))
                .await
                .is_err());
        });
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::{
        fs::{self, File, OpenOptions},
        os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        path::Component,
    };
    use tokio::net::{UnixListener, UnixStream};
    pub struct Listener(UnixListener);
    pub struct EndpointGuard {
        path: PathBuf,
        dev: u64,
        ino: u64,
        _lock: File,
    }
    // SAFETY: geteuid 没有指针参数或前置条件。
    fn uid() -> u32 {
        unsafe { libc::geteuid() }
    }
    pub fn validate_endpoint(path: &Path) -> Result<()> {
        if !path.is_absolute()
            || path.file_name().is_none()
            || path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err("IPC 端点必须为无相对跳转的绝对路径".into());
        }
        // macOS sockaddr_un.sun_path 为 104；保守限制让路径在 Unix 平台一致。
        use std::os::unix::ffi::OsStrExt;
        if path.as_os_str().as_bytes().len() > 100 || path.as_os_str().as_bytes().contains(&0) {
            return Err("IPC socket 路径最多 100 字节，请显式配置较短的端点".into());
        }
        Ok(())
    }
    fn check_ancestors(path: &Path) -> Result<()> {
        for ancestor in path.ancestors() {
            let metadata = match fs::symlink_metadata(ancestor) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.to_string()),
            };
            if !metadata.is_dir() || metadata.is_symlink() {
                return Err("IPC 父路径不能包含符号链接或非目录".into());
            }
            let mode = metadata.mode();
            if metadata.uid() != uid() && metadata.uid() != 0 {
                return Err("IPC 父目录必须由当前用户或系统拥有".into());
            }
            if mode & 0o022 != 0 && !(metadata.uid() == 0 && mode & 0o1000 != 0) {
                return Err("IPC 父目录不能允许其他用户替换路径".into());
            }
        }
        Ok(())
    }
    fn private_parent(path: &Path, create: bool) -> Result<()> {
        let parent = path.parent().ok_or("IPC 端点没有父目录")?;
        check_ancestors(parent)?;
        if create {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)
                .map_err(|e| e.to_string())?;
        }
        let metadata =
            fs::symlink_metadata(parent).map_err(|e| format!("IPC 私有目录不可用：{e}"))?;
        if metadata.uid() != uid() || metadata.mode() & 0o077 != 0 {
            return Err("IPC 端点目录必须归当前用户拥有且权限为 0700".into());
        }
        Ok(())
    }
    fn check_socket(path: &Path) -> Result<fs::Metadata> {
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !metadata.file_type().is_socket()
            || metadata.uid() != uid()
            || metadata.mode() & 0o077 != 0
        {
            return Err("IPC 端点必须为当前用户的私有 socket；不会删除或替换其他文件".into());
        }
        Ok(metadata)
    }
    pub async fn bind(path: &Path) -> Result<(Listener, EndpointGuard)> {
        private_parent(path, true)?;
        let lock_path = path.with_extension("lock");
        if lock_path == path {
            return Err("IPC socket 不能使用 .lock 扩展名".into());
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(lock_path)
            .map_err(|e| format!("无法打开 IPC 实例锁：{e}"))?;
        let metadata = lock.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file()
            || metadata.uid() != uid()
            || metadata.nlink() != 1
            || metadata.mode() & 0o077 != 0
        {
            return Err("IPC 实例锁必须是当前用户的私有普通文件且只有一个硬链接".into());
        }
        lock.try_lock()
            .map_err(|_| "该 IPC 端点已有 daemon 持有实例锁".to_string())?;
        match fs::symlink_metadata(path) {
            Ok(_) => {
                let original = check_socket(path)?;
                match timeout(Duration::from_millis(200), UnixStream::connect(path)).await {
                    Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                        let current = check_socket(path)?;
                        if current.dev() != original.dev() || current.ino() != original.ino() {
                            return Err("IPC 失效端点发生变化，拒绝清理".into());
                        }
                        fs::remove_file(path).map_err(|e| e.to_string())?;
                    }
                    _ => return Err("IPC 端点仍可连接或状态不确定，拒绝替换".into()),
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
        let listener = UnixListener::bind(path).map_err(|e| format!("无法绑定 IPC socket：{e}"))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
        let metadata = check_socket(path)?;
        Ok((
            Listener(listener),
            EndpointGuard {
                path: path.into(),
                dev: metadata.dev(),
                ino: metadata.ino(),
                _lock: lock,
            },
        ))
    }
    impl Listener {
        pub async fn accept(&mut self) -> Result<UnixStream> {
            loop {
                let (stream, _) = self.0.accept().await.map_err(|e| e.to_string())?;
                if stream.peer_cred().map_err(|e| e.to_string())?.uid() == uid() {
                    return Ok(stream);
                }
            }
        }
    }
    pub async fn connect(path: &Path) -> Result<UnixStream> {
        private_parent(path, false)?;
        check_socket(path)?;
        let stream = UnixStream::connect(path)
            .await
            .map_err(|e| format!("无法连接本机 RecallCard daemon：{e}"))?;
        if stream.peer_cred().map_err(|e| e.to_string())?.uid() != uid() {
            return Err("IPC daemon 用户身份不匹配".into());
        }
        Ok(stream)
    }
    impl Drop for EndpointGuard {
        fn drop(&mut self) {
            if let Ok(metadata) = check_socket(&self.path) {
                if metadata.dev() == self.dev && metadata.ino() == self.ino {
                    let _ = fs::remove_file(&self.path);
                }
            }
            // 锁文件始终保留，避免 unlink 导致两代锁分别被持有。
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{ffi::c_void, mem::size_of, os::windows::io::AsRawHandle, ptr};
    use tokio::net::windows::named_pipe::{
        ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree, ERROR_PIPE_BUSY, HANDLE},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            },
            GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
        },
        System::{
            Pipes::GetNamedPipeServerProcessId,
            Threading::{
                GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
    };
    pub struct Listener {
        next: NamedPipeServer,
        path: PathBuf,
    }
    pub struct EndpointGuard;
    struct Token(HANDLE);
    impl Drop for Token {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    struct Local(*mut c_void);
    impl Drop for Local {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
    fn os_error() -> String {
        format!(
            "Windows IPC 安全配置失败：{}",
            std::io::Error::last_os_error()
        )
    }
    pub fn current_user_sid() -> Result<String> {
        // SAFETY: GetCurrentProcess 返回的伪句柄不需要关闭。
        user_sid(unsafe { GetCurrentProcess() })
    }
    fn user_sid(process: HANDLE) -> Result<String> {
        // SAFETY: 系统写入有效输出指针；句柄与 LocalAlloc 内存由 RAII 释放。
        unsafe {
            let mut raw = ptr::null_mut();
            if OpenProcessToken(process, TOKEN_QUERY, &mut raw) == 0 {
                return Err(os_error());
            }
            let token = Token(raw);
            let mut length = 0;
            GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut length);
            if length == 0 || length > 65536 {
                return Err(os_error());
            }
            // usize 保证 TOKEN_USER 所需的指针对齐。
            let mut data = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
            if GetTokenInformation(
                token.0,
                TokenUser,
                data.as_mut_ptr().cast(),
                length,
                &mut length,
            ) == 0
            {
                return Err(os_error());
            }
            let user = &*(data.as_ptr().cast::<TOKEN_USER>());
            let mut raw_sid = ptr::null_mut();
            if ConvertSidToStringSidW(user.User.Sid, &mut raw_sid) == 0 {
                return Err(os_error());
            }
            let _sid_memory = Local(raw_sid.cast());
            let mut count = 0;
            while count < 256 && *raw_sid.add(count) != 0 {
                count += 1;
            }
            if count == 256 {
                return Err("Windows 用户 SID 超长".into());
            }
            String::from_utf16(std::slice::from_raw_parts(raw_sid, count))
                .map_err(|_| "Windows 用户 SID 无效".into())
        }
    }
    pub fn validate_endpoint(path: &Path) -> Result<()> {
        let text = path.to_str().ok_or("Windows IPC 端点必须为 Unicode")?;
        let suffix = text
            .strip_prefix(r"\\.\pipe\recallcard-")
            .ok_or("Windows IPC 必须使用本机 recallcard- 命名管道")?;
        if suffix.is_empty()
            || suffix.len() > 100
            || !suffix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err("Windows IPC 管道名无效".into());
        }
        Ok(())
    }
    fn create(path: &Path, first: bool) -> Result<NamedPipeServer> {
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;{})", current_user_sid()?)
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // SAFETY: SDDL 已终止，安全描述符在 CreateNamedPipe 返回前保持有效。
        unsafe {
            let mut descriptor = ptr::null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            ) == 0
            {
                return Err(os_error());
            }
            let _descriptor = Local(descriptor);
            let mut attributes = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            };
            ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(
                    path,
                    (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
                )
                .map_err(|e| format!("无法创建本机私有 IPC 管道；可能已有 daemon：{e}"))
        }
    }
    pub async fn bind(path: &Path) -> Result<(Listener, EndpointGuard)> {
        Ok((
            Listener {
                next: create(path, true)?,
                path: path.into(),
            },
            EndpointGuard,
        ))
    }
    impl Listener {
        pub async fn accept(&mut self) -> Result<NamedPipeServer> {
            self.next.connect().await.map_err(|e| e.to_string())?;
            // 先建立下一实例再交出连接，管道名不会在两次请求间消失。
            let next = create(&self.path, false)?;
            Ok(std::mem::replace(&mut self.next, next))
        }
    }
    pub async fn connect(path: &Path) -> Result<NamedPipeClient> {
        loop {
            match ClientOptions::new().open(path) {
                Ok(client) => {
                    // 不把查询发送给其他用户抢先占据的同名管道。
                    // SAFETY: 使用仍由 client 持有的管道句柄，输出指针有效。
                    unsafe {
                        let mut pid = 0;
                        if GetNamedPipeServerProcessId(client.as_raw_handle(), &mut pid) == 0 {
                            return Err(os_error());
                        }
                        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                        if handle.is_null() {
                            return Err(os_error());
                        }
                        let process = Token(handle);
                        if user_sid(process.0)? != current_user_sid()? {
                            return Err("IPC daemon 用户身份不匹配".into());
                        }
                    }
                    return Ok(client);
                }
                Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                    tokio::time::sleep(Duration::from_millis(10)).await
                }
                Err(error) => return Err(format!("无法连接本机 RecallCard daemon：{error}")),
            }
        }
    }
}
