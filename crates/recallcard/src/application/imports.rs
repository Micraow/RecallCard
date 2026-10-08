//! 可跨 CLI/GUI 恢复的导入服务。预检暂存与事实提交分离，状态不依赖窗口存活。
use super::{
    import_reader::{self, ImportedConversation, ReadReport},
    *,
};
use crate::{
    filesystem::{file_identity, open_local_file, FileIdentity},
    vault::{reject_symlink, sync_parent},
    EventInput, EventStreamWriter, Vault,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Instant, SystemTime},
};

const MAX_STAGE_BYTES: u64 = 64 * 1024 * 1024;
const MANIFEST_BYTES: u64 = 128 * 1024;

#[derive(Clone)]
pub struct ImportService {
    root: PathBuf,
    root_identity: FileIdentity,
    marker: FileIdentity,
    directory: PathBuf,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
struct SourceFile {
    path: PathBuf,
    identity: FileIdentity,
    bytes: u64,
    modified: SystemTime,
}
#[derive(Clone, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    root: PathBuf,
    root_identity: FileIdentity,
    marker: FileIdentity,
    request_hash: String,
    sources: Vec<SourceFile>,
    source_hashes: Vec<String>,
    format: String,
    limits: ImportLimits,
    run_id: String,
    status: JobStatus,
    staged: usize,
    staged_bytes: u64,
    staged_hash: String,
    staging_complete: bool,
    coverage: ReadReport,
}
#[derive(Serialize, Deserialize)]
struct StagedConversation {
    platform: String,
    source_id: String,
    title: Option<String>,
    events: Vec<EventInput>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub job: JobStatus,
    pub coverage: ReadReport,
}
struct Lease(File);
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn error(code: ErrorCode, message: &str, action: &str) -> AppError {
    AppError::new(code, message, action)
}
fn storage() -> AppError {
    let mut e = error(
        ErrorCode::Storage,
        "无法安全读取或保存本机任务状态",
        "检查磁盘空间与目录权限后重试；已提交数据保留",
    );
    e.retryable = true;
    e
}
fn conflict() -> AppError {
    let mut e = error(
        ErrorCode::Conflict,
        "已有写入任务正在处理这个资料库",
        "等待当前任务完成或暂停后重试",
    );
    e.retryable = true;
    e
}
fn changed() -> AppError {
    error(
        ErrorCode::SourceChanged,
        "导出文件或资料库身份已改变",
        "重新选择当前文件建立新任务；不会把不同内容续接到旧任务",
    )
}
fn paused() -> AppError {
    let mut e = error(
        ErrorCode::Cancelled,
        "导入已暂停，已提交资料保留",
        "继续同一任务即可恢复",
    );
    e.retryable = true;
    e
}
fn basename(path: &Path) -> Option<String> {
    path.file_name().map(|s| s.to_string_lossy().into_owned())
}
fn job_id(request: &str) -> String {
    format!("job_{}", crate::hash(request.as_bytes()))
}
fn valid_job(id: &str) -> bool {
    id.len() == 68
        && id.starts_with("job_")
        && id[4..]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn lease(path: &Path) -> AppResult<Option<Lease>> {
    reject_symlink(path).map_err(|_| storage())?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
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
    match file.try_lock() {
        Ok(()) => Ok(Some(Lease(file))),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(_) => Err(storage()),
    }
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path, max: u64) -> AppResult<T> {
    reject_symlink(path).map_err(|_| storage())?;
    let file = open_local_file(path).map_err(|_| storage())?;
    let meta = file.metadata().map_err(|_| storage())?;
    if !meta.is_file() || meta.len() > max {
        return Err(storage());
    }
    serde_json::from_reader(file.take(max + 1)).map_err(|_| storage())
}
fn source(path: &Path) -> AppResult<SourceFile> {
    reject_symlink(path).map_err(|_| {
        error(
            ErrorCode::InvalidRequest,
            "导入源不允许符号链接",
            "请选择本机普通文件",
        )
    })?;
    let path = fs::canonicalize(path).map_err(|_| {
        error(
            ErrorCode::PermissionDenied,
            "无法打开导出文件",
            "检查文件是否存在且允许读取",
        )
    })?;
    let file = open_local_file(&path).map_err(|_| storage())?;
    let meta = file.metadata().map_err(|_| storage())?;
    if !meta.is_file() {
        return Err(error(
            ErrorCode::InvalidRequest,
            "导入源必须是普通文件",
            "请选择官方 JSON 或 ZIP 文件",
        ));
    }
    Ok(SourceFile {
        path,
        identity: file_identity(&file).map_err(|_| storage())?,
        bytes: meta.len(),
        modified: meta.modified().map_err(|_| storage())?,
    })
}
fn create_directory(path: &Path) -> AppResult<()> {
    reject_symlink(path).map_err(|_| storage())?;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|_| storage())
}
impl ImportService {
    pub fn new(vault: &Vault) -> AppResult<Self> {
        let directory = vault
            .state_dir()
            .map_err(|_| storage())?
            .join("application/imports");
        create_directory(&directory)?;
        Ok(Self {
            root: vault.root().into(),
            root_identity: file_identity(&open_local_file(vault.root()).map_err(|_| storage())?)
                .map_err(|_| storage())?,
            marker: file_identity(
                &open_local_file(&vault.root().join("control/schema-version.json"))
                    .map_err(|_| storage())?,
            )
            .map_err(|_| storage())?,
            directory,
        })
    }
    /// 一个进程中每个 Vault 只有一个导入队列 worker，重复点击不会不断创建线程。
    /// 独立 CLI 进程仍由磁盘 lease 协调。没有窗口会话引用，不因切页/换库取消。
    pub fn start_worker(&self) -> AppResult<()> {
        self.recover_interrupted()?;
        use std::sync::{Mutex, OnceLock};
        static WORKERS: OnceLock<Mutex<std::collections::BTreeSet<PathBuf>>> = OnceLock::new();
        let registry = WORKERS.get_or_init(|| Mutex::new(std::collections::BTreeSet::new()));
        let mut guard = registry.lock().map_err(|_| storage())?;
        if !guard.insert(self.directory.clone()) {
            return Ok(());
        }
        let service = self.clone();
        let key = self.directory.clone();
        let spawned = std::thread::Builder::new()
            .name("recallcard-import-runtime".into())
            .spawn(move || {
                while let Ok(queued) = service.queued() {
                    if queued.is_empty() {
                        if let Ok(mut registry) = WORKERS.get().unwrap().lock() {
                            if service.queued().is_ok_and(|q| q.is_empty()) {
                                registry.remove(&service.directory);
                                return;
                            }
                        } else {
                            break;
                        }
                        continue;
                    }
                    for job in queued {
                        match service.run(&job.job_id, &job.scope) {
                            Ok(_) => {}
                            Err(e) if e.code == ErrorCode::Conflict => {
                                std::thread::sleep(std::time::Duration::from_millis(200))
                            }
                            Err(_) => {
                                if let Ok(mut registry) = WORKERS.get().unwrap().lock() {
                                    registry.remove(&service.directory);
                                }
                                return;
                            }
                        }
                    }
                }
                if let Ok(mut registry) = WORKERS.get().unwrap().lock() {
                    registry.remove(&service.directory);
                }
            });
        if spawned.is_err() {
            guard.remove(&key);
            return Err(storage());
        }
        Ok(())
    }
    /// 服务停止只暂停其已授权任务；不会撤销已提交资料，也不重放模型请求。
    pub fn pause_all_pending(&self) -> AppResult<usize> {
        let mut count = 0;
        for entry in fs::read_dir(&self.directory).map_err(|_| storage())? {
            let entry = entry.map_err(|_| storage())?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if !valid_job(&id) {
                continue;
            }
            let m: Manifest = read_json(&entry.path().join("manifest.json"), MANIFEST_BYTES)?;
            if let Ok(valid) = self.load(&id, &m.status.scope) {
                if matches!(valid.status.state, JobState::Queued | JobState::Running) {
                    self.pause(&id, &valid.status.scope)?;
                    count += 1;
                }
            }
        }
        Ok(count)
    }
    pub fn has_pending_work(&self) -> AppResult<bool> {
        for entry in fs::read_dir(&self.directory).map_err(|_| storage())? {
            let entry = entry.map_err(|_| storage())?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if !valid_job(&id) {
                continue;
            }
            let m: Manifest = read_json(&entry.path().join("manifest.json"), MANIFEST_BYTES)?;
            if let Ok(valid) = self.load(&id, &m.status.scope) {
                if matches!(valid.status.state, JobState::Queued | JobState::Running) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    /// 只恢复持久 running 且已失去 OS lease 的任务；用户明确暂停的任务保持暂停。
    pub fn recover_interrupted(&self) -> AppResult<usize> {
        let mut count = 0;
        for entry in fs::read_dir(&self.directory).map_err(|_| storage())? {
            let entry = entry.map_err(|_| storage())?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if !valid_job(&id) {
                continue;
            }
            let Some(_lease) = lease(&entry.path().join("run.lock"))? else {
                continue;
            };
            let m: Manifest = read_json(&entry.path().join("manifest.json"), MANIFEST_BYTES)?;
            if let Ok(mut valid) = self.load(&id, &m.status.scope) {
                if valid.status.state == JobState::Running {
                    if entry
                        .path()
                        .join(format!("pause-{}", valid.run_id))
                        .exists()
                    {
                        valid.status.state = JobState::Paused;
                        valid.status.can_resume = true;
                    } else {
                        valid.status.state = JobState::Queued;
                        valid.status.can_resume = false;
                        valid.run_id = uuid::Uuid::new_v4().to_string();
                    }
                    valid.status.updated_at = Utc::now();
                    self.save(&valid)?;
                    count += 1;
                }
            }
        }
        Ok(count)
    }
    fn queued(&self) -> AppResult<Vec<JobStatus>> {
        let mut jobs = Vec::new();
        for entry in fs::read_dir(&self.directory).map_err(|_| storage())? {
            let entry = entry.map_err(|_| storage())?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if !valid_job(&id) {
                continue;
            }
            let m: Manifest = read_json(&entry.path().join("manifest.json"), MANIFEST_BYTES)?;
            if let Ok(valid) = self.load(&id, &m.status.scope) {
                if valid.status.state == JobState::Queued {
                    jobs.push(valid.status);
                }
            }
        }
        jobs.sort_by_key(|a| a.created_at);
        Ok(jobs)
    }
    fn vault(&self) -> AppResult<Vault> {
        let vault = Vault::open_existing(&self.root).map_err(|_| changed())?;
        if file_identity(&open_local_file(&self.root).map_err(|_| changed())?)
            .map_err(|_| changed())?
            != self.root_identity
            || file_identity(
                &open_local_file(&self.root.join("control/schema-version.json"))
                    .map_err(|_| changed())?,
            )
            .map_err(|_| changed())?
                != self.marker
        {
            return Err(changed());
        }
        Ok(vault)
    }
    fn load(&self, id: &str, scope: &str) -> AppResult<Manifest> {
        if !valid_job(id) {
            return Err(error(
                ErrorCode::InvalidRequest,
                "任务编号无效",
                "从当前任务列表选择任务",
            ));
        }
        let m: Manifest = read_json(
            &self.directory.join(id).join("manifest.json"),
            MANIFEST_BYTES,
        )?;
        if m.version != 1
            || m.root != self.root
            || m.root_identity != self.root_identity
            || m.marker != self.marker
            || m.status.scope != scope
            || m.status.job_id != id
            || m.status.schema != JOB_SCHEMA
        {
            return Err(error(
                ErrorCode::PermissionDenied,
                "该任务不属于当前资料库或范围",
                "在正确的资料范围中查看任务",
            ));
        }
        if m.sources.len() > m.limits.files
            || m.sources.len() != m.source_hashes.len()
            || m.staged as u64 > m.limits.events
        {
            return Err(storage());
        }
        Ok(m)
    }
    fn save(&self, m: &Manifest) -> AppResult<()> {
        self.vault()?
            .write_replace(
                &self.directory.join(&m.status.job_id).join("manifest.json"),
                m,
            )
            .map_err(|_| storage())
    }
    pub fn submit(&self, request: ImportRequest) -> AppResult<JobStatus> {
        self.submit_with_limits(request, ImportLimits::default())
    }
    pub fn submit_with_limits(
        &self,
        request: ImportRequest,
        limits: ImportLimits,
    ) -> AppResult<JobStatus> {
        self.vault()?;
        crate::validate_scope(&request.scope).map_err(|_| {
            error(
                ErrorCode::InvalidRequest,
                "资料范围无效",
                "选择有效资料范围",
            )
        })?;
        if request.request_id.is_empty()
            || request.request_id.len() > 128
            || !request
                .request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || request.paths.is_empty()
            || request.paths.len() > limits.files
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "导入请求编号或文件数量无效",
                "选择 1–32 个文件后重新开始",
            ));
        }
        if !matches!(
            request.format.as_str(),
            "auto" | "chatgpt-export" | "deepseek-export"
        ) {
            return Err(error(
                ErrorCode::UnsupportedFormat,
                "此服务只接受支持的官方导出格式",
                "选择自动识别、ChatGPT 或 DeepSeek 官方导出",
            ));
        }
        let mut sources = Vec::new();
        let mut total = 0u64;
        for path in &request.paths {
            let f = source(path)?;
            total = total.checked_add(f.bytes).ok_or_else(storage)?;
            if total > limits.source_bytes {
                return Err(error(
                    ErrorCode::ResourceLimit,
                    "本次文件总大小超过配置的输入上限",
                    "查看导入资源设置；不会截断内容",
                ));
            }
            if sources
                .iter()
                .any(|old: &SourceFile| old.identity == f.identity)
            {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "同一文件被选择了多次",
                    "移除重复文件后重试",
                ));
            }
            sources.push(f);
        }
        let id = job_id(&request.request_id);
        let path = self.directory.join(&id);
        let request_hash = crate::hash(
            &serde_json::to_vec(&(&request.scope, &request.format, &sources, limits))
                .map_err(|_| storage())?,
        );
        create_directory(&path)?;
        let _guard = lease(&path.join("run.lock"))?.ok_or_else(conflict)?;
        if path.join("manifest.json").exists() {
            let m = self.load(&id, &request.scope)?;
            if m.request_hash != request_hash {
                return Err(error(
                    ErrorCode::Conflict,
                    "同一请求编号已绑定不同文件或配置",
                    "为新的导入建立新的请求编号",
                ));
            }
            return Ok(m.status);
        }
        create_directory(&path.join("staged"))?;
        let now = Utc::now();
        let status = JobStatus {
            schema: JOB_SCHEMA.into(),
            job_id: id,
            request_id: request.request_id,
            kind: "import".into(),
            scope: request.scope,
            state: JobState::Queued,
            phase: JobPhase::Preflight,
            progress: ImportProgress {
                files_total: sources.len() as u64,
                ..Default::default()
            },
            error: None,
            can_resume: false,
            created_at: now,
            updated_at: now,
        };
        let manifest = Manifest {
            version: 1,
            root: self.root.clone(),
            root_identity: self.root_identity.clone(),
            marker: self.marker.clone(),
            request_hash,
            source_hashes: vec![String::new(); sources.len()],
            sources,
            format: request.format,
            limits,
            run_id: uuid::Uuid::new_v4().to_string(),
            status: status.clone(),
            staged: 0,
            staged_bytes: 0,
            staged_hash: crate::hash(b""),
            staging_complete: false,
            coverage: ReadReport::default(),
        };
        self.save(&manifest)?;
        Ok(status)
    }
    pub fn status(&self, id: &str, scope: &str) -> AppResult<JobStatus> {
        self.vault()?;
        let mut m = self.load(id, scope)?;
        if let Some(_idle) = lease(&self.directory.join(id).join("run.lock"))? {
            if m.status.state == JobState::Running {
                m.status.state = JobState::Paused;
                m.status.can_resume = true;
                m.status.error = Some(error(
                    ErrorCode::Cancelled,
                    "任务执行已中断，已有检查点可恢复",
                    "继续此任务将校验暂存并跳过已提交块",
                ));
            }
        } else {
            m.status.can_resume = false;
        }
        Ok(m.status)
    }
    pub fn result(&self, id: &str, scope: &str) -> AppResult<ImportResult> {
        let m = self.load(id, scope)?;
        Ok(ImportResult {
            job: self.status(id, scope)?,
            coverage: m.coverage,
        })
    }
    pub fn list(&self, scope: &str) -> AppResult<Vec<JobStatus>> {
        self.vault()?;
        let mut result = Vec::new();
        for entry in fs::read_dir(&self.directory).map_err(|_| storage())? {
            let entry = entry.map_err(|_| storage())?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if valid_job(&id) {
                if let Ok(job) = self.status(&id, scope) {
                    result.push(job);
                }
            }
        }
        result.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then(a.job_id.cmp(&b.job_id))
        });
        Ok(result)
    }
    pub fn pause(&self, id: &str, scope: &str) -> AppResult<JobStatus> {
        let mut m = self.load(id, scope)?;
        if matches!(m.status.state, JobState::Completed | JobState::Cancelled) {
            return self.status(id, scope);
        }
        let directory = self.directory.join(id);
        self.vault()?
            .write_bytes(&directory.join(format!("pause-{}", m.run_id)), b"pause\n")
            .map_err(|_| storage())?;
        if let Some(_idle) = lease(&directory.join("run.lock"))? {
            m = self.load(id, scope)?;
            m.status.state = JobState::Paused;
            m.status.can_resume = true;
            m.status.updated_at = Utc::now();
            self.save(&m)?;
        }
        self.status(id, scope)
    }
    /// 只排队，不在调用线程执行耗时工作；同一服务的 run 负责实际处理。
    pub fn resume(&self, id: &str, scope: &str) -> AppResult<JobStatus> {
        self.load(id, scope)?;
        let _guard = lease(&self.directory.join(id).join("run.lock"))?.ok_or_else(conflict)?;
        let mut m = self.load(id, scope)?;
        if m.status.state == JobState::Completed {
            return Ok(m.status);
        }
        if m.status.state == JobState::NeedsInput
            || m.status
                .error
                .as_ref()
                .is_some_and(|e| !e.retryable && e.code != ErrorCode::Cancelled)
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "此任务需要新的文件或配置，不能盲目重试",
                "根据具体错误重新选择来源",
            ));
        }
        m.run_id = uuid::Uuid::new_v4().to_string();
        m.status.state = JobState::Queued;
        m.status.error = None;
        m.status.can_resume = false;
        m.status.updated_at = Utc::now();
        self.save(&m)?;
        Ok(m.status)
    }
    /// 独立宿主线程/CLI worker 调用；全局租约避免多个任务争写同一库。
    pub fn run(&self, id: &str, scope: &str) -> AppResult<JobStatus> {
        self.load(id, scope)?;
        let _global = lease(&self.directory.join("active.lock"))?.ok_or_else(conflict)?;
        let directory = self.directory.join(id);
        let _job = lease(&directory.join("run.lock"))?.ok_or_else(conflict)?;
        let mut m = self.load(id, scope)?;
        if matches!(
            m.status.state,
            JobState::Completed
                | JobState::Paused
                | JobState::NeedsInput
                | JobState::Cancelled
                | JobState::Failed
        ) {
            return Ok(m.status);
        }
        m.status.state = JobState::Running;
        m.status.can_resume = false;
        m.status.error = None;
        m.status.updated_at = Utc::now();
        self.save(&m)?;
        let outcome = self.execute(&mut m, &directory);
        match outcome {
            Ok(()) => {
                m.status.state = JobState::Completed;
                m.status.phase = JobPhase::Finished;
                m.status.can_resume = false;
                m.status.error = None;
            }
            Err(mut e) => {
                e.committed_events = m.status.progress.events_added;
                m.status.state = if e.code == ErrorCode::Cancelled {
                    JobState::Paused
                } else if matches!(
                    e.code,
                    ErrorCode::SourceChanged | ErrorCode::PermissionDenied
                ) {
                    JobState::NeedsInput
                } else {
                    JobState::Failed
                };
                m.status.can_resume = e.retryable;
                m.status.error = Some(e);
            }
        }
        m.status.updated_at = Utc::now();
        self.save(&m)?;
        Ok(m.status)
    }
    fn check_pause(&self, m: &Manifest, directory: &Path) -> AppResult<()> {
        if directory.join(format!("pause-{}", m.run_id)).exists() {
            Err(paused())
        } else {
            Ok(())
        }
    }
    fn fingerprint(
        &self,
        m: &Manifest,
        source: &SourceFile,
        directory: &Path,
    ) -> AppResult<String> {
        if &crate_source(source)? != source {
            return Err(changed());
        }
        let mut file = open_local_file(&source.path).map_err(|_| changed())?;
        let mut hash = Sha256::new();
        let mut bytes = 0u64;
        let mut buf = [0; 65536];
        loop {
            self.check_pause(m, directory)?;
            let n = file.read(&mut buf).map_err(|_| changed())?;
            if n == 0 {
                break;
            }
            bytes += n as u64;
            if bytes > m.limits.source_bytes {
                return Err(error(
                    ErrorCode::ResourceLimit,
                    "来源读取时超过输入字节上限",
                    "检查文件是否仍在变化",
                ));
            }
            hash.update(&buf[..n]);
        }
        if bytes != source.bytes || &crate_source(source)? != source {
            return Err(changed());
        }
        Ok(format!("{:x}", hash.finalize()))
    }
    fn execute(&self, m: &mut Manifest, directory: &Path) -> AppResult<()> {
        if !m.staging_complete {
            self.stage(m, directory)?;
        }
        self.check_pause(m, directory)?;
        let files = stages(directory, m.staged)?;
        let mut digest = crate::hash(b"");
        // 提交前核验完整暂存链；任何损坏都不能让此前无写入的任务部分落盘。
        for (_, path, expected) in &files {
            self.check_pause(m, directory)?;
            let bytes = read_stage_bytes(path)?;
            if crate::hash(&bytes) != *expected {
                return Err(error(
                    ErrorCode::SourceChanged,
                    "导入暂存已改变",
                    "重新选择原始导出建立任务",
                ));
            }
            digest = chain(&digest, expected);
        }
        if digest != m.staged_hash {
            return Err(error(
                ErrorCode::SourceChanged,
                "导入暂存清单不一致",
                "重新选择原始导出建立任务",
            ));
        }
        m.status.phase = JobPhase::Committing;
        m.status.progress.events_total = Some(m.status.progress.events_staged);
        self.save(m)?;
        let vault = self.vault()?;
        let mut writer = EventStreamWriter::open(&vault).map_err(|message| {
            if message.starts_with("Vault 正在被另一个写入操作使用") {
                conflict()
            } else {
                let mut e = storage();
                e.retryable = false;
                e.message = "正本或未完成事务无法核验，不能继续提交".into();
                e.action = "检查资料库诊断与恢复记录；不会覆盖损坏正本".into();
                e
            }
        })?;
        let mut replay = ImportProgress::default();
        let mut chunk = Vec::new();
        let mut bytes = 0;
        let mut chunk_id = 0;
        for (_, path, expected) in files {
            let raw = read_stage_bytes(&path)?;
            if crate::hash(&raw) != expected {
                return Err(error(
                    ErrorCode::SourceChanged,
                    "导入暂存在提交期间改变",
                    "保留已提交记录，重新选择完整来源",
                ));
            }
            let staged: StagedConversation = serde_json::from_slice(&raw).map_err(|_| storage())?;
            for event in staged.events {
                if event.scope != m.status.scope || event.validate().is_err() {
                    return Err(error(
                        ErrorCode::SourceChanged,
                        "暂存事件范围或内容无效",
                        "重新选择来源",
                    ));
                }
                let size = serde_json::to_vec(&event).map_err(|_| storage())?.len();
                if !chunk.is_empty() && (chunk.len() >= 64 || bytes + size > 8 * 1024 * 1024) {
                    self.commit(
                        m,
                        directory,
                        &mut writer,
                        &mut chunk,
                        &mut chunk_id,
                        &mut replay,
                    )?;
                    bytes = 0;
                }
                bytes += size;
                chunk.push(event);
            }
        }
        if !chunk.is_empty() {
            self.commit(
                m,
                directory,
                &mut writer,
                &mut chunk,
                &mut chunk_id,
                &mut replay,
            )?;
        }
        if replay.events_processed != m.status.progress.events_staged {
            return Err(storage());
        }
        Ok(())
    }
    fn commit(
        &self,
        m: &mut Manifest,
        directory: &Path,
        writer: &mut EventStreamWriter<'_>,
        chunk: &mut Vec<EventInput>,
        index: &mut usize,
        replay: &mut ImportProgress,
    ) -> AppResult<()> {
        self.check_pause(m, directory)?;
        let receipt = writer
            .commit_chunk(
                &format!("{}:{}", m.status.job_id, index),
                std::mem::take(chunk),
            )
            .map_err(|_| storage())?;
        replay.events_processed += receipt.events_processed;
        replay.events_added += receipt.events_added;
        replay.events_duplicates += receipt.events_duplicates;
        // 先重放到已有确认前缀；锁忙/暂停不能把已写入结果清零。
        if replay.events_processed >= m.status.progress.events_processed {
            m.status.progress.events_processed = replay.events_processed;
            m.status.progress.events_added = replay.events_added;
            m.status.progress.events_duplicates = replay.events_duplicates;
        }
        *index += 1;
        m.status.updated_at = Utc::now();
        self.save(m)
    }
    fn stage(&self, m: &mut Manifest, directory: &Path) -> AppResult<()> {
        m.status.phase = JobPhase::Preflight;
        self.save(m)?;
        for i in 0..m.sources.len() {
            let fingerprint = self
                .fingerprint(m, &m.sources[i], directory)
                .map_err(|mut e| {
                    e.file_name = basename(&m.sources[i].path);
                    e
                })?;
            if !m.source_hashes[i].is_empty() && m.source_hashes[i] != fingerprint {
                return Err(changed());
            }
            m.source_hashes[i] = fingerprint;
            self.save(m)?;
        }
        m.status.phase = JobPhase::Parsing;
        m.status.progress.expanded_bytes_read = 0;
        m.status.progress.source_bytes_read = 0;
        self.save(m)?;
        let existing = stages(directory, m.staged)?;
        let known: BTreeMap<usize, (PathBuf, String)> =
            existing.into_iter().map(|(i, p, h)| (i, (p, h))).collect();
        let state = RefCell::new((
            m.clone(),
            0usize,
            crate::hash(b""),
            0u64,
            Instant::now(),
            0u64,
        ));
        let mut aggregate = ReadReport::default();
        for i in 0..m.sources.len() {
            let f = m.sources[i].clone();
            let mut file = open_local_file(&f.path).map_err(|_| changed())?;
            let result = import_reader::read_source(
                &mut file,
                &m.format,
                &m.status.scope,
                m.limits,
                |conversation: ImportedConversation| {
                    let mut st = state.borrow_mut();
                    self.check_pause(&st.0, directory)?;
                    let staged = StagedConversation {
                        platform: conversation.platform,
                        source_id: conversation.source_id,
                        title: conversation.title,
                        events: conversation.events,
                    };
                    let bytes = encode_stage(&staged)?;
                    if bytes.len() as u64 > MAX_STAGE_BYTES {
                        return Err(error(
                            ErrorCode::ResourceLimit,
                            "规范化后单个会话超过暂存上限",
                            "查看受影响会话，不会截断内容",
                        ));
                    }
                    let digest = crate::hash(&bytes);
                    let ordinal = st.1;
                    if let Some((path, old)) = known.get(&ordinal) {
                        if *old != digest || crate::hash(&read_stage_bytes(path)?) != digest {
                            return Err(changed());
                        }
                    } else {
                        let path = directory
                            .join("staged")
                            .join(format!("{ordinal:010}-{digest}.json"));
                        if path.exists() {
                            if crate::hash(&read_stage_bytes(&path)?) != digest {
                                return Err(changed());
                            }
                        } else {
                            let vault = self.vault()?;
                            vault
                                .staged(&path, &bytes)
                                .map_err(|_| storage())?
                                .persist_noclobber(&path)
                                .map_err(|_| storage())?;
                            sync_parent(&path).map_err(|_| storage())?;
                        }
                    }
                    st.1 += 1;
                    st.2 = chain(&st.2, &digest);
                    st.3 += staged.events.len() as u64;
                    st.5 += bytes.len() as u64;
                    if st.3 > st.0.limits.events || st.5 > st.0.limits.expanded_bytes {
                        return Err(error(
                            ErrorCode::ResourceLimit,
                            "规范化事件数量或暂存总量超过配置上限",
                            "查看导入资源设置；暂存不会写入事实源",
                        ));
                    }
                    if st.1 > st.0.staged {
                        st.0.staged = st.1;
                        st.0.staged_bytes = st.5;
                        st.0.staged_hash = st.2.clone();
                        st.0.status.progress.conversations = st.1 as u64;
                        st.0.status.progress.events_staged = st.3;
                        st.0.status.updated_at = Utc::now();
                        self.save(&st.0)?;
                    }
                    Ok(())
                },
                |report| {
                    let mut st = state.borrow_mut();
                    self.check_pause(&st.0, directory)?;
                    st.0.status.progress.expanded_bytes_read =
                        aggregate.expanded_bytes + report.expanded_bytes;
                    if st.0.status.progress.expanded_bytes_read > st.0.limits.expanded_bytes {
                        return Err(error(
                            ErrorCode::ResourceLimit,
                            "多文件展开总量超过配置上限",
                            "查看导入资源设置",
                        ));
                    }
                    if st.4.elapsed().as_millis() >= 150 {
                        st.0.status.updated_at = Utc::now();
                        self.save(&st.0)?;
                        st.4 = Instant::now();
                    }
                    Ok(())
                },
            );
            match result {
                Ok(report) => {
                    merge_report(&mut aggregate, &report);
                    let mut st = state.borrow_mut();
                    st.0.status.progress.files_processed = (i + 1) as u64;
                    st.0.status.progress.source_bytes_read += f.bytes;
                    st.0.coverage = aggregate.clone();
                    self.save(&st.0)?;
                }
                Err(mut e) => {
                    *m = state.into_inner().0;
                    e.file_name = basename(&f.path);
                    return Err(e);
                }
            }
            if self.fingerprint(&state.borrow().0, &f, directory)? != m.source_hashes[i] {
                *m = state.into_inner().0;
                return Err(changed());
            }
            file.seek(SeekFrom::Start(0)).map_err(|_| changed())?;
        }
        let st = state.into_inner();
        *m = st.0;
        if st.1 != m.staged
            || st.2 != m.staged_hash
            || st.3 != m.status.progress.events_staged
            || st.5 != m.staged_bytes
        {
            return Err(changed());
        }
        if st.3 == 0 {
            return Err(error(
                ErrorCode::UnsupportedFormat,
                "所选文件没有可保存的支持内容",
                "查看覆盖报告并选择支持的官方导出",
            ));
        }
        m.coverage = aggregate;
        m.staging_complete = true;
        m.status.progress.events_total = Some(st.3);
        self.save(m)
    }
}
fn crate_source(f: &SourceFile) -> AppResult<SourceFile> {
    source(&f.path)
}
fn chain(previous: &str, next: &str) -> String {
    crate::hash(format!("{previous}:{next}").as_bytes())
}
fn read_stage_bytes(path: &Path) -> AppResult<Vec<u8>> {
    reject_symlink(path).map_err(|_| storage())?;
    let mut file = open_local_file(path).map_err(|_| storage())?;
    let meta = file.metadata().map_err(|_| storage())?;
    if !meta.is_file() || meta.len() > MAX_STAGE_BYTES {
        return Err(storage());
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_STAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| storage())?;
    if bytes.len() as u64 > MAX_STAGE_BYTES {
        return Err(storage());
    }
    Ok(bytes)
}
fn stages(directory: &Path, count: usize) -> AppResult<Vec<(usize, PathBuf, String)>> {
    let mut files = BTreeMap::new();
    let staged = directory.join("staged");
    reject_symlink(&staged).map_err(|_| storage())?;
    for entry in fs::read_dir(staged).map_err(|_| storage())? {
        let entry = entry.map_err(|_| storage())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Some((number, hash)) = name.strip_suffix(".json").and_then(|s| s.split_once('-'))
        else {
            return Err(storage());
        };
        let number: usize = number.parse().map_err(|_| storage())?;
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(storage());
        }
        if number < count
            && files
                .insert(number, (entry.path(), hash.to_owned()))
                .is_some()
        {
            return Err(storage());
        }
    }
    if files.len() != count || files.keys().copied().ne(0..count) {
        return Err(storage());
    }
    Ok(files.into_iter().map(|(i, (p, h))| (i, p, h)).collect())
}
fn merge_report(total: &mut ReadReport, part: &ReadReport) {
    total.expanded_bytes += part.expanded_bytes;
    total.archive_entries += part.archive_entries;
    total.conversations += part.conversations;
    total.events += part.events;
    total.ignored_values += part.ignored_values;
    total.ignored_files += part.ignored_files;
    total.hidden_fragments += part.hidden_fragments;
    total.unsupported_fragments += part.unsupported_fragments;
    total.omitted_messages += part.omitted_messages;
    total.file_references += part.file_references;
    total.citations += part.citations;
    total.trace_placeholders += part.trace_placeholders;
}

fn encode_stage(staged: &StagedConversation) -> AppResult<Vec<u8>> {
    struct Bounded(Vec<u8>);
    impl std::io::Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.0.len().saturating_add(bytes.len()) > MAX_STAGE_BYTES as usize {
                return Err(std::io::Error::other("stage limit"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut out = Bounded(Vec::new());
    serde_json::to_writer(&mut out, staged).map_err(|_| {
        error(
            ErrorCode::ResourceLimit,
            "规范化后单个会话超过暂存上限",
            "查看受影响会话，不会截断内容",
        )
    })?;
    Ok(out.0)
}
