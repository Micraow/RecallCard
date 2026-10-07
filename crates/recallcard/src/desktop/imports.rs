//! 多文件导入的本机任务。确认前只读；确认后冻结脱敏输入，恢复不再读取原文件。
//! 每个 Vault 同时只有一个导入写入者；状态查询和取消不获取 Vault 写锁。
use super::{
    bounded_bytes, check_scope, checked_bytes, file_name, path_identity, token, DesktopSession,
    FileIdentity, FileSnapshot, ImportSample, SAMPLE_COUNT, STALE_SESSION,
};
use crate::{
    capture::redact_event,
    context::truncate_utf8,
    import_bundle::{self, ConversationSummary, ImportCoverage},
    model::{hash, EventInput, Result},
    vault::reject_symlink,
    Vault,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

const SNAPSHOT_LIMIT: usize = 512 * 1024 * 1024;
const BASELINE_LIMIT: usize = 128 * 1024 * 1024;
const MANIFEST_LIMIT: usize = 64 * 1024;
const CHECKPOINT_EVENTS: usize = 64;
const JOB_ERROR: &str = "导入任务状态无法安全读取，请检查资料库和本机状态文件";
const IMPORT_ERROR: &str =
    "导入文件无法完整解析，请检查来源格式、会话结构和大小限制；尚未写入任何消息";

#[derive(Debug, Clone, Serialize)]
pub struct ImportJobFile {
    pub file_name: String,
    pub byte_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportJobPreview {
    pub preview_id: String,
    pub session_id: String,
    pub scope: String,
    pub format: String,
    pub files: Vec<ImportJobFile>,
    pub byte_count: usize,
    pub event_count: usize,
    pub redacted_event_count: usize,
    pub coverage: ImportCoverage,
    pub conversations: Vec<ConversationSummary>,
    pub samples: Vec<ImportSample>,
    pub warning: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportJobState {
    Running,
    Cancelling,
    Cancelled,
    Interrupted,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportJobStatus {
    pub job_id: String,
    pub scope: String,
    pub state: ImportJobState,
    pub events_total: usize,
    pub events_processed: usize,
    pub events_added: usize,
    pub events_duplicates: usize,
    pub files_total: usize,
    pub conversations_total: usize,
    pub can_resume: bool,
    pub message: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub(super) struct PendingImportJob {
    preview_id: String,
    scope: String,
    files: Vec<FileSnapshot>,
    events: Vec<EventInput>,
    conversations_total: usize,
}

pub(super) struct ImportJobControl {
    job_id: String,
    scope: String,
    cancel: Arc<AtomicBool>,
    status: Arc<Mutex<ImportJobStatus>>,
}

#[derive(Clone, Serialize, Deserialize)]
struct VaultBinding {
    root: PathBuf,
    identity: FileIdentity,
    marker: FileSnapshot,
}

#[derive(Serialize, Deserialize)]
struct JobManifest {
    version: u32,
    binding: VaultBinding,
    snapshot_hash: String,
    baseline_hash: Option<String>,
    run_id: String,
    status: ImportJobStatus,
}

struct Lease(File);
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

impl DesktopSession {
    /// 路径只能由宿主原生多选文件对话框传入；不接受目录，不扫描下载或账号数据。
    pub fn prepare_import_job(
        &mut self,
        session_id: &str,
        format: &str,
        paths: &[PathBuf],
        scope: &str,
    ) -> Result<ImportJobPreview> {
        self.vault(session_id)?;
        self.pending_import_job = None;
        self.pending_import = None;
        self.pending_import_selection = None;
        check_scope(scope)?;
        if paths.is_empty() || paths.len() > import_bundle::MAX_IMPORT_FILES {
            return Err("请一次选择 1–32 个官方导出的 JSON 或 ZIP 文件".into());
        }
        if !matches!(format, "auto" | "chatgpt-export" | "deepseek-export") {
            return Err("批量导入请选择自动识别、DeepSeek 或 ChatGPT 官方备份".into());
        }
        let mut files = Vec::new();
        let mut contents = Vec::new();
        let mut total: usize = 0;
        for path in paths {
            let (file, bytes) = bounded_bytes(path, import_bundle::MAX_IMPORT_BYTES - total)?;
            total = total.checked_add(bytes.len()).ok_or(IMPORT_ERROR)?;
            if files
                .iter()
                .any(|old: &FileSnapshot| old.identity == file.identity)
            {
                return Err("同一文件被选择了多次，请移除重复选择后重试".into());
            }
            files.push(file);
            contents.push(bytes);
        }
        let slices: Vec<_> = contents.iter().map(Vec::as_slice).collect();
        let summary = import_bundle::inspect_import_files(format, &slices, scope)
            .map_err(|_| IMPORT_ERROR)?;
        let selected: BTreeSet<_> = summary
            .conversation_summaries
            .iter()
            .filter(|c| c.event_count > 0)
            .map(|c| c.selection_key.clone())
            .collect();
        if selected.is_empty() {
            return Err("所选文件没有可导入的可见消息；隐藏推理和附件不会作为正文导入".into());
        }
        let mut parsed =
            import_bundle::parse_import_files_selected(format, &slices, scope, &selected)
                .map_err(|_| IMPORT_ERROR)?;
        for event in &mut parsed.events {
            redact_event(event).map_err(|_| IMPORT_ERROR)?;
        }
        if parsed.events.is_empty() {
            return Err(IMPORT_ERROR.into());
        }
        let preview = ImportJobPreview {
            preview_id: token(), session_id: session_id.into(), scope: scope.into(), format: format.into(),
            files: files.iter().map(|f| ImportJobFile { file_name: file_name(&f.path), byte_count: f.bytes }).collect(),
            byte_count: total, event_count: parsed.events.len(),
            redacted_event_count: parsed.events.iter().filter(|e| e.capture.redacted).count(),
            coverage: parsed.coverage,
            conversations: parsed.conversation_summaries,
            samples: parsed.events.iter().take(SAMPLE_COUNT).map(|event| ImportSample {
                role: event.role.clone(), origin: event.origin.clone(), source: event.source.clone(),
                occurred_at: event.occurred_at, content: truncate_utf8(&event.text(), 1200),
                redacted: event.capture.redacted,
            }).collect(),
            warning: "确认后保存以上对话，之后可以按需整理成记忆。请检查文字样本；暂停后已保存的消息会保留，继续时会跳过重复内容。".into(),
        };
        self.pending_import_job = Some(PendingImportJob {
            preview_id: preview.preview_id.clone(),
            scope: scope.into(),
            files,
            events: parsed.events,
            conversations_total: preview.coverage.conversations_selected,
        });
        Ok(preview)
    }

    /// 一次明确确认消耗一个预览令牌；任何源文件变化都使本次令牌永久失效。
    pub fn start_import_job(
        &mut self,
        session_id: &str,
        preview_id: &str,
        scope: &str,
    ) -> Result<ImportJobStatus> {
        self.vault(session_id)?;
        check_scope(scope)?;
        if !self
            .pending_import_job
            .as_ref()
            .is_some_and(|p| p.preview_id == preview_id && p.scope == scope)
        {
            return Err("导入确认已失效或资料范围已改变，请重新选择并检查文件".into());
        }
        let pending = self.pending_import_job.take().ok_or(JOB_ERROR)?;
        for file in &pending.files {
            checked_bytes(file, import_bundle::MAX_IMPORT_BYTES)?;
        }
        let vault = self.vault(session_id)?;
        let directory = jobs_dir(vault)?;
        let global_lease = acquire_lease(&directory.join("active.lock"))?;
        let job_id = token();
        let job_dir = directory.join(&job_id);
        fs::create_dir(&job_dir).map_err(|_| JOB_ERROR)?;
        let job_lease = acquire_lease(&job_dir.join("run.lock"))?;
        let bytes = serde_json::to_vec(&pending.events).map_err(|_| JOB_ERROR)?;
        if bytes.len() > SNAPSHOT_LIMIT {
            return Err("脱敏后的导入数据过大，请拆分后重试；尚未写入消息".into());
        }
        let selected = self.selected.as_ref().ok_or(STALE_SESSION)?;
        let now = Utc::now();
        let status = ImportJobStatus {
            job_id,
            scope: scope.into(),
            state: ImportJobState::Running,
            events_total: pending.events.len(),
            events_processed: 0,
            events_added: 0,
            events_duplicates: 0,
            files_total: pending.files.len(),
            conversations_total: pending.conversations_total,
            can_resume: false,
            message: "正在导入；可随时暂停，已写入消息会保留".into(),
            created_at: now,
            updated_at: now,
        };
        let manifest = JobManifest {
            version: 1,
            binding: VaultBinding {
                root: vault.root().into(),
                identity: selected.identity.clone(),
                marker: selected.marker.clone(),
            },
            snapshot_hash: hash(&bytes),
            baseline_hash: None,
            run_id: token(),
            status,
        };
        vault
            .write_new(&job_dir.join("manifest.json"), &manifest)
            .map_err(|_| JOB_ERROR)?;
        self.launch_import(
            manifest,
            pending.events,
            job_dir,
            global_lease,
            job_lease,
            Some(bytes),
        )
    }

    pub fn import_job_status(
        &self,
        session_id: &str,
        job_id: &str,
        scope: &str,
    ) -> Result<ImportJobStatus> {
        let vault = self.vault(session_id)?;
        check_scope(scope)?;
        if !valid_token(job_id) {
            return Err("导入任务编号无效".into());
        }
        let directory = jobs_dir(vault)?.join(job_id);
        if let Some(_idle) = idle_job_lease(&directory.join("run.lock"))? {
            let (manifest, directory) = load_manifest(vault, job_id, scope)?;
            return observed_status_with_lease(manifest.status, &directory, false);
        }
        if let Some(control) = self
            .active_import_job
            .as_ref()
            .filter(|c| c.job_id == job_id && c.scope == scope)
        {
            if let Ok(status) = control.status.lock() {
                // 写入者持有 lease 时，使用本进程已验证任务的实时状态，避免读取正在
                // 原子替换的 manifest。最终落盘未释放 lease 前仍显示处理中。
                if matches!(
                    status.state,
                    ImportJobState::Running | ImportJobState::Cancelling
                ) {
                    return observed_status_with_lease(status.clone(), &directory, true);
                }
                // 旧 control 结束后，同一 job 可能由另一实例继续；不能用它的旧进度
                // 覆盖新写入者的检查点，须走下面的磁盘核验。
            }
        }
        let (manifest, directory) = load_manifest(vault, job_id, scope)?;
        // lease 查询时确认仍有写入者；即使它刚结束，也只多显示一轮处理中，
        // 不能用较早读到的 running 检查点推断“已中断”。下一次从闲置读锁核实。
        observed_status_with_lease(manifest.status, &directory, true)
    }

    /// 列表只含当前资料库、当前 scope 的本机任务概要，不返回消息正文或源路径。
    pub fn list_import_jobs(&self, session_id: &str, scope: &str) -> Result<Vec<ImportJobStatus>> {
        let vault = self.vault(session_id)?;
        check_scope(scope)?;
        let directory = jobs_dir(vault)?;
        let mut jobs = Vec::new();
        for entry in fs::read_dir(&directory).map_err(|_| JOB_ERROR)? {
            let entry = entry.map_err(|_| JOB_ERROR)?;
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !valid_token(&id) {
                continue;
            }
            // 其他 scope 的任务不泄露，也不阻断本 scope；损坏任务无法获得继续令牌。
            if let Ok(status) = self.import_job_status(session_id, &id, scope) {
                jobs.push(status);
            }
        }
        jobs.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then(a.job_id.cmp(&b.job_id))
        });
        Ok(jobs)
    }

    /// 只读查看本批已核实处理、仍可访问的会话。恢复后也不依赖原始导出文件。
    /// 中断检查点之外的输入不冒充已完成结果；不把同会话的其他版本纳入批次。
    pub fn import_job_conversations(
        &self,
        session_id: &str,
        job_id: &str,
        scope: &str,
        offset: usize,
    ) -> Result<Value> {
        if offset > import_bundle::MAX_IMPORT_EVENTS {
            return Err("本批会话分页参数无效".into());
        }
        let vault = self.vault(session_id)?;
        if !valid_token(job_id) {
            return Err("导入任务编号无效".into());
        }
        let directory = jobs_dir(vault)?.join(job_id);
        let _idle =
            idle_job_lease(&directory.join("run.lock"))?.ok_or("请先暂停导入，再查看本批会话")?;
        let (manifest, directory) = load_manifest(vault, job_id, scope)?;
        let status = observed_status_with_lease(manifest.status, &directory, false)?;
        if matches!(
            status.state,
            ImportJobState::Running | ImportJobState::Cancelling
        ) {
            return Err("请先暂停导入，再查看本批会话".into());
        }
        let (_, bytes) = bounded_bytes(&directory.join("snapshot.json"), SNAPSHOT_LIMIT)
            .map_err(|_| JOB_ERROR)?;
        if hash(&bytes) != manifest.snapshot_hash {
            return Err("导入记录无法核实，请在全部会话中查找已保存资料".into());
        }
        let events: Vec<EventInput> = serde_json::from_slice(&bytes).map_err(|_| JOB_ERROR)?;
        drop(bytes);
        if events.len() != status.events_total
            || events
                .iter()
                .any(|e| e.scope != scope || e.validate().is_err())
        {
            return Err(JOB_ERROR.into());
        }
        let _guard = vault.read_guard()?;
        let context = self.context(session_id, scope)?;
        let records = super::records::WorkspaceRecords::load(vault, &context)?;
        let conversations =
            records.imported_input_conversations(&events[..status.events_processed])?;
        let total = conversations.len();
        let mut rows = Vec::new();
        let mut bytes_used = 2048;
        for row in conversations.into_iter().skip(offset).take(50) {
            let size = serde_json::to_vec(&row).map_err(|_| JOB_ERROR)?.len();
            if bytes_used + size > super::RESPONSE_LIMIT {
                if rows.is_empty() {
                    return Err("本批会话信息过长，请在全部会话中查找已保存资料".into());
                }
                break;
            }
            bytes_used += size;
            rows.push(row);
        }
        let next = offset + rows.len();
        let response = json!({"job_id":job_id,"scope":scope,"status":status,"conversations":rows,
            "total":total,"offset":offset,"next_offset":if next < total { Some(next) } else { None },
            "note":"这里只显示本批已核实处理、当前可访问的会话，包含重复消息。打开后可阅读该会话当前可访问的全部原话。异常退出前未记下进度的部分暂不计入本批。"});
        if serde_json::to_vec(&response).map_err(|_| JOB_ERROR)?.len() > super::RESPONSE_LIMIT {
            return Err("本批会话信息过长，请在全部会话中查找已保存资料".into());
        }
        Ok(response)
    }

    pub fn cancel_import_job(
        &mut self,
        session_id: &str,
        job_id: &str,
        scope: &str,
    ) -> Result<ImportJobStatus> {
        let vault = self.vault(session_id)?;
        let (manifest, directory) = load_manifest(vault, job_id, scope)?;
        let mut status = observed_status(manifest.status, &directory)?;
        if !matches!(
            status.state,
            ImportJobState::Running | ImportJobState::Cancelling
        ) {
            return Ok(status);
        }
        // 取消标记绑定单次运行；上一轮的迟到取消不能取消后续明确恢复的运行。
        vault
            .write_bytes(
                &directory.join(format!("cancel-{}", manifest.run_id)),
                b"pause\n",
            )
            .map_err(|_| JOB_ERROR)?;
        if let Some(control) = self
            .active_import_job
            .as_ref()
            .filter(|c| c.job_id == job_id && c.scope == scope)
        {
            control.cancel.store(true, Ordering::Release);
            if let Ok(mut current) = control.status.lock() {
                current.state = ImportJobState::Cancelling;
                current.message = "正在暂停；已写入的消息会保留".into();
                status = current.clone();
            }
        }
        status.state = ImportJobState::Cancelling;
        status.can_resume = false;
        status.message = "正在暂停；已写入的消息会保留".into();
        Ok(status)
    }

    /// 恢复必须来自当前用户操作；只读取已确认、脱敏并核对摘要的本机快照。
    pub fn resume_import_job(
        &mut self,
        session_id: &str,
        job_id: &str,
        scope: &str,
    ) -> Result<ImportJobStatus> {
        let vault = self.vault(session_id)?;
        let (mut manifest, directory) = load_manifest(vault, job_id, scope)?;
        if manifest.status.state == ImportJobState::Completed {
            return Err("此导入已经完成，无需再次继续".into());
        }
        let global_lease = acquire_lease(&jobs_dir(vault)?.join("active.lock"))?;
        let job_lease = acquire_lease(&directory.join("run.lock"))?;
        let (_, bytes) = bounded_bytes(&directory.join("snapshot.json"), SNAPSHOT_LIMIT)
            .map_err(|_| JOB_ERROR)?;
        if hash(&bytes) != manifest.snapshot_hash {
            return Err("导入快照已改变，不能继续；请重新选择源文件并审查".into());
        }
        let events: Vec<EventInput> = serde_json::from_slice(&bytes).map_err(|_| JOB_ERROR)?;
        if events.len() != manifest.status.events_total
            || events
                .iter()
                .any(|e| e.scope != scope || e.validate().is_err())
        {
            return Err(JOB_ERROR.into());
        }
        // 重播冻结输入可恢复「Event 已落盘、检查点尚未落盘」的崩溃窗口。
        // 底层单锁批处理会复用已有 Event，不产生额外修订；原始基线使计数仍然准确。
        manifest.status.state = ImportJobState::Running;
        manifest.status.can_resume = false;
        manifest.status.events_processed = 0;
        manifest.status.events_added = 0;
        manifest.status.events_duplicates = 0;
        manifest.status.message = "正在核对已写入消息并继续导入；不会重复写入".into();
        manifest.status.updated_at = Utc::now();
        manifest.run_id = token();
        vault
            .write_replace(&directory.join("manifest.json"), &manifest)
            .map_err(|_| JOB_ERROR)?;
        self.launch_import(manifest, events, directory, global_lease, job_lease, None)
    }

    fn launch_import(
        &mut self,
        manifest: JobManifest,
        events: Vec<EventInput>,
        directory: PathBuf,
        global_lease: Lease,
        job_lease: Lease,
        snapshot: Option<Vec<u8>>,
    ) -> Result<ImportJobStatus> {
        let status = manifest.status.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Mutex::new(status.clone()));
        let control = ImportJobControl {
            job_id: status.job_id.clone(),
            scope: status.scope.clone(),
            cancel: cancel.clone(),
            status: shared.clone(),
        };
        let spawn = std::thread::Builder::new()
            .name("recallcard-import".into())
            .spawn(move || {
                let _leases = (global_lease, job_lease);
                run_import(manifest, events, directory, cancel, shared, snapshot);
            });
        if spawn.is_err() {
            // 线程未创建时 OS lease 自动释放；磁盘 running 会被查询识别为 interrupted。
            return Err("导入线程暂时无法启动，请重新选择并确认文件".into());
        }
        self.active_import_job = Some(control);
        Ok(status)
    }

    pub(super) fn pause_import_job(&mut self) {
        if let Some(control) = self.active_import_job.take() {
            control.cancel.store(true, Ordering::Release);
        }
    }
}

impl Drop for DesktopSession {
    fn drop(&mut self) {
        self.pause_import_job();
    }
}

fn jobs_dir(vault: &Vault) -> Result<PathBuf> {
    let path = vault
        .state_dir()
        .map_err(|_| JOB_ERROR)?
        .join("import-jobs");
    if path.starts_with(vault.root()) {
        return Err("本机导入状态目录不能放在资料库内，请将状态目录设置到资料库之外".into());
    }
    reject_symlink(&path).map_err(|_| JOB_ERROR)?;
    fs::create_dir_all(&path).map_err(|_| JOB_ERROR)?;
    Ok(path)
}

fn valid_token(token: &str) -> bool {
    token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

fn binding_valid(binding: &VaultBinding) -> Result<()> {
    reject_symlink(&binding.root).map_err(|_| STALE_SESSION)?;
    if path_identity(&binding.root)? != binding.identity {
        return Err(STALE_SESSION.into());
    }
    checked_bytes(&binding.marker, 4096).map_err(|_| STALE_SESSION)?;
    Ok(())
}

fn load_manifest(vault: &Vault, job_id: &str, scope: &str) -> Result<(JobManifest, PathBuf)> {
    check_scope(scope)?;
    if !valid_token(job_id) {
        return Err("导入任务编号无效".into());
    }
    let directory = jobs_dir(vault)?.join(job_id);
    let path = directory.join("manifest.json");
    let (_, bytes) = bounded_bytes(&path, MANIFEST_LIMIT)
        .or_else(|error| {
            // 其他进程读取活动任务时，原子换入新检查点可能使旧 inode 核验失效。
            // 仅对明确的文件变化重读一次；仍执行相同大小、身份、范围与绑定核验。
            if error == super::STALE_FILE {
                bounded_bytes(&path, MANIFEST_LIMIT)
            } else {
                Err(error)
            }
        })
        .map_err(|_| JOB_ERROR)?;
    let manifest: JobManifest = serde_json::from_slice(&bytes).map_err(|_| JOB_ERROR)?;
    if manifest.version != 1
        || manifest.binding.root != vault.root()
        || manifest.binding.marker.path != vault.root().join("control/schema-version.json")
        || manifest.status.job_id != job_id
        || manifest.status.scope != scope
        || !valid_token(&manifest.run_id)
        || manifest.status.events_total > import_bundle::MAX_IMPORT_EVENTS
        || manifest.status.events_processed > manifest.status.events_total
        || manifest
            .status
            .events_added
            .checked_add(manifest.status.events_duplicates)
            != Some(manifest.status.events_processed)
    {
        return Err("导入任务与当前资料库或范围不匹配，不能操作".into());
    }
    binding_valid(&manifest.binding)?;
    Ok((manifest, directory))
}

fn lock_file(path: &Path) -> Result<File> {
    reject_symlink(path).map_err(|_| JOB_ERROR)?;
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .mode(0o600);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path).map_err(|_| JOB_ERROR)?;
    if !file.metadata().map_err(|_| JOB_ERROR)?.is_file() {
        return Err(JOB_ERROR.into());
    }
    super::file_identity(&file).map_err(|_| JOB_ERROR)?;
    Ok(file)
}

fn acquire_lease(path: &Path) -> Result<Lease> {
    let file = lock_file(path)?;
    file.try_lock()
        .map_err(|_| "此资料库已有正在运行的导入，请先等待完成或暂停")?;
    Ok(Lease(file))
}

fn lease_busy(path: &Path) -> Result<bool> {
    let file = lock_file(path)?;
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            Ok(false)
        }
        Err(std::fs::TryLockError::WouldBlock) => Ok(true),
        Err(_) => Err(JOB_ERROR.into()),
    }
}

fn idle_job_lease(path: &Path) -> Result<Option<Lease>> {
    let file = lock_file(path)?;
    match file.try_lock_shared() {
        Ok(()) => Ok(Some(Lease(file))),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(_) => Err(JOB_ERROR.into()),
    }
}

fn observed_status(status: ImportJobStatus, directory: &Path) -> Result<ImportJobStatus> {
    let active = lease_busy(&directory.join("run.lock"))?;
    observed_status_with_lease(status, directory, active)
}

fn observed_status_with_lease(
    mut status: ImportJobStatus,
    directory: &Path,
    active: bool,
) -> Result<ImportJobStatus> {
    if active
        && !matches!(
            status.state,
            ImportJobState::Running | ImportJobState::Cancelling
        )
    {
        status.state = ImportJobState::Running;
        status.message = "正在保存最后进度，请稍候".into();
    }
    if matches!(
        status.state,
        ImportJobState::Running | ImportJobState::Cancelling
    ) && !active
    {
        status.state = ImportJobState::Interrupted;
        status.message = "上次导入已中断；已写入消息保留，点击继续可安全恢复".into();
    }
    status.can_resume = !active
        && matches!(
            status.state,
            ImportJobState::Cancelled | ImportJobState::Interrupted | ImportJobState::Failed
        );
    if status.can_resume && !snapshot_exists(directory) {
        status.can_resume = false;
        status.message = "导入准备中断，尚未保存可恢复快照；请重新选择并确认文件".into();
    }
    Ok(status)
}

fn snapshot_exists(directory: &Path) -> bool {
    fs::symlink_metadata(directory.join("snapshot.json")).is_ok_and(|m| m.is_file())
}

fn run_import(
    manifest: JobManifest,
    events: Vec<EventInput>,
    directory: PathBuf,
    cancel: Arc<AtomicBool>,
    shared: Arc<Mutex<ImportJobStatus>>,
    snapshot: Option<Vec<u8>>,
) {
    let root = manifest.binding.root.clone();
    let binding = manifest.binding.clone();
    let cancellation_path = directory.join(format!("cancel-{}", manifest.run_id));
    let manifest = RefCell::new(manifest);
    let baseline = RefCell::new(BTreeSet::<String>::new());
    let mut seen = BTreeSet::new();
    let result = (|| -> Result<()> {
        binding_valid(&binding)?;
        let vault = Vault::open(&root).map_err(|_| JOB_ERROR)?;
        // 大快照落盘也在后台线程进行，状态和暂停不会被桌面会话互斥锁挡住。
        // 任何 Event 写入之前必须完成这个已脱敏、绑定摘要的冻结快照。
        if let Some(bytes) = snapshot {
            vault
                .write_bytes(&directory.join("snapshot.json"), &bytes)
                .map_err(|_| JOB_ERROR)?;
        }
        if let Some(expected) = &manifest.borrow().baseline_hash {
            let (_, bytes) = bounded_bytes(&directory.join("baseline.json"), BASELINE_LIMIT)
                .map_err(|_| JOB_ERROR)?;
            if hash(&bytes) != *expected {
                return Err(JOB_ERROR.into());
            }
            *baseline.borrow_mut() = serde_json::from_slice(&bytes).map_err(|_| JOB_ERROR)?;
        }
        let source_valid = AtomicBool::new(true);
        vault.capture_batch_with_callbacks(
            events,
            || {
                if cancel.load(Ordering::Acquire) || cancellation_path.exists() {
                    return false;
                }
                if binding_valid(&binding).is_err() {
                    source_valid.store(false, Ordering::Release);
                    return false;
                }
                true
            },
            |existing| {
                let mut job = manifest.borrow_mut();
                if job.baseline_hash.is_none() {
                    *baseline.borrow_mut() = existing.iter().map(|e| e.id.clone()).collect();
                    let bytes = serde_json::to_vec(&*baseline.borrow()).map_err(|_| JOB_ERROR)?;
                    if bytes.len() > BASELINE_LIMIT {
                        return Err(JOB_ERROR.into());
                    }
                    vault
                        .write_bytes(&directory.join("baseline.json"), &bytes)
                        .map_err(|_| JOB_ERROR)?;
                    job.baseline_hash = Some(hash(&bytes));
                    checkpoint(&vault, &directory, &job)?;
                }
                Ok(())
            },
            |completed, event, _added| {
                let mut job = manifest.borrow_mut();
                let added = !baseline.borrow().contains(&event.id) && seen.insert(event.id.clone());
                job.status.events_processed = completed;
                job.status.events_added += usize::from(added);
                job.status.events_duplicates += usize::from(!added);
                job.status.updated_at = Utc::now();
                if completed % CHECKPOINT_EVENTS == 0 {
                    checkpoint(&vault, &directory, &job)?;
                }
                if let Ok(mut current) = shared.lock() {
                    let cancelling = current.state == ImportJobState::Cancelling;
                    *current = job.status.clone();
                    if cancelling {
                        current.state = ImportJobState::Cancelling;
                    }
                }
                Ok(())
            },
        )?;
        if !source_valid.load(Ordering::Acquire) {
            return Err(STALE_SESSION.into());
        }
        Ok(())
    })();
    let mut job = manifest.into_inner();
    job.status.updated_at = Utc::now();
    if result.is_err() {
        job.status.state = ImportJobState::Failed;
        job.status.message =
            "导入未完成，请检查资料库后继续；已写入消息保留，恢复会核对并去重".into();
    } else if job.status.events_processed == job.status.events_total {
        job.status.state = ImportJobState::Completed;
        job.status.message = "导入完成；消息已保存，可在原始记录中查看".into();
    } else {
        job.status.state = ImportJobState::Cancelled;
        job.status.message = "已暂停；已写入消息保留，点击继续可恢复".into();
    }
    job.status.can_resume = job.status.state != ImportJobState::Completed;
    if job.status.can_resume && !snapshot_exists(&directory) {
        job.status.can_resume = false;
        job.status.message = "导入准备未完成，尚未写入消息；请重新选择并确认文件".into();
    }
    // 最终状态也必须落盘；失败时保持旧检查点，后续查询会据 lease 显示可恢复的中断。
    let persisted = Vault::open(&root)
        .ok()
        .filter(|_| binding_valid(&binding).is_ok())
        .is_some_and(|vault| checkpoint(&vault, &directory, &job).is_ok());
    if !persisted {
        job.status.state = ImportJobState::Interrupted;
        job.status.can_resume = true;
        job.status.message = "进度保存中断；已写入消息保留，继续时会重新核对".into();
    }
    if let Ok(mut current) = shared.lock() {
        *current = job.status;
    }
}

fn checkpoint(vault: &Vault, directory: &Path, job: &JobManifest) -> Result<()> {
    vault
        .write_replace(&directory.join("manifest.json"), job)
        .map_err(|_| JOB_ERROR.into())
}
