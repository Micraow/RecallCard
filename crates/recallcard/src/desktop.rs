//! 桌面宿主的最小本地服务。路径只能来自原生文件选择器，不能成为通用文件/命令接口。
//!
//! 宿主应把整个服务放在互斥锁内，每条命令持锁到结束；切换 Vault 会废弃所有旧会话
//! 和审查令牌。预览只读；确认命令只接受保存在本机内存中的预览编号，不接受替代路径
//! 或替代内容。错误不转发解析器、操作系统或 Git 输出中的文件正文、秘密和路径。
use crate::{
    capture::redact_event,
    context::{truncate_utf8, Context, ReadArgs, SearchArgs},
    dream::{DreamJob, DreamReceipt, DreamResult, DreamReview},
    import::{import_text, parse_text},
    model::{hash, validate_scope, Origin, Result, Role},
    policy::Access,
    vault::reject_symlink,
    Vault,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const IMPORT_LIMIT: usize = 16 * 1024 * 1024;
const DREAM_LIMIT: usize = 1024 * 1024;
const RESPONSE_LIMIT: usize = 32768;
const SAMPLE_COUNT: usize = 6;
const STALE_SESSION: &str = "资料库会话已失效，请重新选择 Vault 后操作";
const STALE_FILE: &str = "文件已改变或被替换，请重新选择文件并审查";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultInfo {
    pub session_id: String,
    pub root: String,
    pub display_name: String,
    pub scopes: Vec<String>,
    pub event_count: usize,
    pub memory_count: usize,
    pub health: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSample {
    pub role: Role,
    pub origin: Origin,
    pub content: String,
    pub redacted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportPreview {
    pub preview_id: String,
    pub session_id: String,
    pub file_name: String,
    pub format: String,
    pub scope: String,
    pub file_hash: String,
    pub byte_count: usize,
    pub event_count: usize,
    pub redacted_event_count: usize,
    pub samples: Vec<ImportSample>,
    pub truncated: bool,
    pub warning: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamPreview {
    pub preview_id: String,
    pub session_id: String,
    pub file_name: String,
    pub file_hash: String,
    pub scope: String,
    pub review: DreamReview,
}

/// 除已选 Vault 与两份待确认文件标识外，不存登录信息、API key 或外发配置。
#[derive(Default)]
pub struct DesktopSession {
    selected: Option<SelectedVault>,
    pending_import: Option<PendingImport>,
    pending_dream: Option<PendingDream>,
}

pub type SessionService = DesktopSession;

struct SelectedVault {
    vault: Vault,
    session_id: String,
    identity: FileIdentity,
    marker: FileSnapshot,
}

struct PendingImport {
    preview_id: String,
    file: FileSnapshot,
    format: String,
    scope: String,
}

struct PendingDream {
    preview_id: String,
    file: FileSnapshot,
    scope: String,
    result_hash: String,
}

#[derive(Debug, PartialEq, Eq)]
struct FileSnapshot {
    path: PathBuf,
    identity: FileIdentity,
    hash: String,
    bytes: usize,
}

#[derive(Debug, PartialEq, Eq)]
struct FileIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(windows)]
    volume_serial: u32,
    #[cfg(windows)]
    file_index: u64,
    #[cfg(not(any(unix, windows)))]
    created: std::time::SystemTime,
}

impl DesktopSession {
    /// `create` 必须来自用户明确点击的创建操作；不把打开失败降级成创建。
    pub fn select_vault(&mut self, path: &Path, create: bool) -> Result<VaultInfo> {
        // 即使新目录无效，也不保留旧页面的写入权限。
        self.close_vault();
        let vault = if create {
            Vault::init(path)
        } else {
            Vault::open(path)
        }
        .map_err(|_| {
            "无法打开资料库，请选择有效的 RecallCard Vault；新目录请使用创建".to_owned()
        })?;
        let identity = path_identity(vault.root())?;
        let (marker, _) = bounded_file(&vault.root().join("control/schema-version.json"), 4096)?;
        let session_id = token();
        self.selected = Some(SelectedVault {
            vault,
            session_id: session_id.clone(),
            identity,
            marker,
        });
        match self.status(&session_id) {
            Ok(info) => Ok(info),
            Err(error) => {
                self.close_vault();
                Err(error)
            }
        }
    }

    pub fn close_vault(&mut self) {
        self.selected = None;
        self.pending_import = None;
        self.pending_dream = None;
    }

    pub fn status(&self, session_id: &str) -> Result<VaultInfo> {
        let vault = self.vault(session_id)?;
        let _guard = vault.read_guard().map_err(|_| health_error())?;
        let health = vault.doctor().map_err(|_| health_error())?;
        let events = vault.events().map_err(|_| health_error())?;
        let memories = vault.memories().map_err(|_| health_error())?;
        let mut scopes: BTreeSet<String> = events.iter().map(|e| e.data.scope.clone()).collect();
        scopes.extend(memories.iter().map(|m| m.data.scope.clone()));
        if scopes.is_empty() {
            scopes.insert("personal".into());
        }
        Ok(VaultInfo {
            session_id: session_id.into(),
            root: vault.root().to_string_lossy().into_owned(),
            display_name: file_name(vault.root()),
            scopes: scopes.into_iter().collect(),
            event_count: events.len(),
            memory_count: memories.len(),
            health,
        })
    }

    /// 不注入向量执行器，不联网；每条查询都显式绑定一个 scope。
    pub fn search(
        &self,
        session_id: &str,
        scope: &str,
        query: &str,
        target: &str,
    ) -> Result<Value> {
        validate_target(target)?;
        if query.trim().is_empty() || query.len() > 4096 {
            return Err("请输入 1–4096 字节的检索词".into());
        }
        self.context(session_id, scope)?
            .search(SearchArgs {
                query: query.into(),
                target: target.into(),
                session_ref: None,
                as_of: None,
                limit: 30,
                detail: "context".into(),
                budget_tokens: RESPONSE_LIMIT,
                cursor: None,
            })
            .map_err(|_| "检索失败，请刷新资料库并检查所选范围".into())
    }

    /// 首屏展示也经过 Context 的权限/抑制过滤，不直接返回 Vault 文件列表。
    pub fn browse(&self, session_id: &str, scope: &str, target: &str) -> Result<Value> {
        validate_target(target)?;
        let now = Utc::now();
        let mut docs = self
            .context(session_id, scope)?
            .documents()
            .map_err(|_| "无法读取资料列表，请刷新资料库".to_owned())?;
        docs.retain(|d| {
            (target == "all"
                || target == "events" && d.kind == "event"
                || target == "memories" && d.kind == "memory")
                && (d.kind != "memory" || d.current_memory_at(now))
        });
        docs.sort_by(|a, b| {
            b.occurred_at
                .cmp(&a.occurred_at)
                .then(a.reference.cmp(&b.reference))
        });
        let total = docs.len();
        let results = docs
            .into_iter()
            .take(30)
            .map(|mut doc| {
                let truncated = doc.text.len() > 2400;
                doc.text = truncate_utf8(&doc.text, 2400);
                let mut value = serde_json::to_value(doc).map_err(|_| health_error())?;
                value["ref"] = value["reference"].take();
                value.as_object_mut().unwrap().remove("reference");
                value["text_truncated"] = json!(truncated);
                Ok(value)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut response = json!({
            "results": results, "total": total, "truncated": total > 30,
            "coverage": {"scope_filtered": true, "semantic_search": "unavailable"},
            "next_cursor": null, "budget_unit": "conservative_utf8_bytes"
        });
        while serde_json::to_vec(&response)
            .map_err(|_| health_error())?
            .len()
            > RESPONSE_LIMIT
        {
            response["results"].as_array_mut().unwrap().pop();
            response["truncated"] = json!(true);
        }
        Ok(response)
    }

    pub fn read(&self, session_id: &str, scope: &str, reference: &str) -> Result<Value> {
        let args = read_args(reference)?;
        self.context(session_id, scope)?
            .read(args)
            .map_err(|_| "记录不可访问、已变化或已被抑制，请重新检索".into())
    }

    pub fn sources(&self, session_id: &str, scope: &str, reference: &str) -> Result<Value> {
        let args = read_args(reference)?;
        self.context(session_id, scope)?
            .sources(args)
            .map_err(|_| "来源不可访问、已变化或已被抑制，请重新检索".into())
    }

    pub fn preview_import(
        &mut self,
        session_id: &str,
        format: &str,
        path: &Path,
        scope: &str,
    ) -> Result<ImportPreview> {
        self.vault(session_id)?;
        self.pending_import = None;
        check_scope(scope)?;
        if !["manual-jsonl", "chatgpt-export", "claude-code"].contains(&format) {
            return Err("请选择 manual-jsonl、chatgpt-export 或 claude-code 格式".into());
        }
        let (file, text) = bounded_file(path, IMPORT_LIMIT)?;
        let mut events = parse_text(format, &text, scope)
            .map_err(|_| "导入文件无效：请检查格式、范围和 5000 条事件上限".to_owned())?;
        if events.is_empty() {
            return Err("此文件没有可导入的消息；不会收集隐藏推理".into());
        }
        // 与 capture 使用相同的脱敏函数。所有记录均先验证，界面不显示原始秘密。
        for event in &mut events {
            redact_event(event).map_err(|_| "导入记录无法安全处理，请检查原始文件".to_owned())?;
        }
        let mut truncated = events.len() > SAMPLE_COUNT;
        let samples = events
            .iter()
            .take(SAMPLE_COUNT)
            .map(|event| {
                let text = event.text();
                truncated |= text.len() > 1200;
                ImportSample {
                    role: event.role.clone(),
                    origin: event.origin.clone(),
                    content: truncate_utf8(&text, 1200),
                    redacted: event.capture.redacted,
                }
            })
            .collect();
        let preview = ImportPreview {
            preview_id: token(),
            session_id: session_id.into(),
            file_name: file_name(&file.path),
            format: format.into(),
            scope: scope.into(),
            file_hash: file.hash.clone(),
            byte_count: file.bytes,
            event_count: events.len(),
            redacted_event_count: events.iter().filter(|e| e.capture.redacted).count(),
            samples,
            truncated,
            warning: "仅显示最多 6 条脱敏样本；秘密检测是启发式的，请检查原文件。确认后追加 Event，不自动生成 Memory；中断后可重新导入，已写事件不回滚。".into(),
        };
        self.pending_import = Some(PendingImport {
            preview_id: preview.preview_id.clone(),
            file,
            format: format.into(),
            scope: scope.into(),
        });
        Ok(preview)
    }

    /// 只有用户确认预览后才能调用；读取并核对原文件，不信任前端替换的内容。
    pub fn confirm_import(&mut self, session_id: &str, preview_id: &str) -> Result<Value> {
        self.vault(session_id)?;
        let pending = self
            .pending_import
            .as_ref()
            .filter(|p| p.preview_id == preview_id)
            .ok_or("导入预览已失效，请重新选择并审查文件")?;
        let (_, text) = checked_file(&pending.file, IMPORT_LIMIT)?;
        let result = import_text(
            self.vault(session_id)?,
            &pending.format,
            &text,
            &pending.scope,
        )
        .map_err(|_| "导入未完成，请检查资料库后重新预览；已写入的事件可安全去重".to_owned())?;
        self.pending_import = None;
        Ok(result)
    }

    /// 此操作只生成本地 Job。宿主保存文件前须让用户选择保存位置；不发送任何网络请求。
    pub fn export_dream(
        &self,
        session_id: &str,
        scope: &str,
        source_refs: &[String],
        memory_refs: &[String],
    ) -> Result<DreamJob> {
        check_scope(scope)?;
        self.vault(session_id)?
            .dream_export(source_refs, memory_refs, scope)
            .map_err(|_| {
                "无法导出 Dream：请选择同一范围内 1–64 条有效来源，最多 32 条旧记忆".into()
            })
    }

    pub fn review_dream(
        &mut self,
        session_id: &str,
        path: &Path,
        scope: &str,
    ) -> Result<DreamPreview> {
        self.vault(session_id)?;
        self.pending_dream = None;
        check_scope(scope)?;
        let (file, text) = bounded_file(path, DREAM_LIMIT)?;
        let result = parse_dream(&text, scope)?;
        let review = self
            .vault(session_id)?
            .dream_review(&result)
            .map_err(|_| "Dream 审查未通过，请检查原任务、来源范围、版本和结果格式".to_owned())?;
        if review.changes.iter().any(|change| {
            [&change.before, &change.after]
                .into_iter()
                .flatten()
                .any(|memory| memory.data.scope != scope)
        }) {
            return Err("Dream 差异包含当前范围之外的内容".into());
        }
        // 不截断差异后允许发布；超限要求拆分任务再完整审查。
        if serde_json::to_vec(&review)
            .map_err(|_| health_error())?
            .len()
            > 4 * DREAM_LIMIT
        {
            return Err("Dream 审查超过 4 MiB，请拆分任务后重新审查".into());
        }
        let preview = DreamPreview {
            preview_id: token(),
            session_id: session_id.into(),
            file_name: file_name(&file.path),
            file_hash: file.hash.clone(),
            scope: scope.into(),
            review,
        };
        self.pending_dream = Some(PendingDream {
            preview_id: preview.preview_id.clone(),
            file,
            scope: scope.into(),
            result_hash: preview.review.result_hash.clone(),
        });
        Ok(preview)
    }

    /// `approve_protected` 必须来自单独、默认未勾选的用户确认控件。
    pub fn apply_dream(
        &mut self,
        session_id: &str,
        preview_id: &str,
        approve_protected: bool,
    ) -> Result<DreamReceipt> {
        self.vault(session_id)?;
        let pending = self
            .pending_dream
            .as_ref()
            .filter(|p| p.preview_id == preview_id)
            .ok_or("Dream 审查已失效，请重新选择并审查结果文件")?;
        let (_, text) = checked_file(&pending.file, DREAM_LIMIT)?;
        let result = parse_dream(&text, &pending.scope)?;
        let vault = self.vault(session_id)?;
        let review = vault
            .dream_review(&result)
            .map_err(|_| "Dream 来源或旧记忆已变化，请重新导出任务并审查".to_owned())?;
        if review.result_hash != pending.result_hash {
            return Err(STALE_FILE.into());
        }
        if !review.can_apply {
            return Err("Dream 仍含有未解决的冲突，不能发布".into());
        }
        if review.requires_protected_approval && !approve_protected {
            return Err("此结果会修改受保护记忆，请单独确认受保护内容的修改".into());
        }
        let receipt = vault
            .dream_apply(&result, &pending.result_hash, approve_protected)
            .map_err(|_| "Dream 未完成发布，请检查资料库状态；不要自动重试或跳过审查".to_owned())?;
        self.pending_dream = None;
        Ok(receipt)
    }

    pub fn cancel_previews(&mut self, session_id: &str) -> Result<()> {
        self.vault(session_id)?;
        self.pending_import = None;
        self.pending_dream = None;
        Ok(())
    }

    fn vault(&self, session_id: &str) -> Result<&Vault> {
        let selected = self
            .selected
            .as_ref()
            .filter(|s| s.session_id == session_id)
            .ok_or(STALE_SESSION)?;
        let root = selected.vault.root();
        reject_symlink(root).map_err(|_| STALE_SESSION)?;
        let metadata = fs::metadata(root).map_err(|_| STALE_SESSION)?;
        if !metadata.is_dir() || path_identity(root)? != selected.identity {
            return Err(STALE_SESSION.into());
        }
        checked_file(&selected.marker, 4096).map_err(|_| STALE_SESSION)?;
        Ok(&selected.vault)
    }

    fn context(&self, session_id: &str, scope: &str) -> Result<Context<'_>> {
        check_scope(scope)?;
        Ok(Context::new(
            self.vault(session_id)?,
            Access::new(vec![scope.into()]).map_err(|_| "请选择一个有效范围")?,
        ))
    }
}

fn parse_dream(text: &str, scope: &str) -> Result<DreamResult> {
    let result: DreamResult = serde_json::from_str(text)
        .map_err(|_| "请选择完整的 DreamResult JSON 文件；不接受自由聊天回复".to_owned())?;
    if result.proposals.is_empty()
        || result.proposals.len() > 32
        || result.proposals.iter().any(|p| p.scope != scope)
    {
        return Err("Dream 需要 1–32 条提议，且所有提议必须属于当前范围".into());
    }
    Ok(result)
}

fn read_args(reference: &str) -> Result<ReadArgs> {
    if reference.len() > 256
        || !(reference.starts_with("event:") || reference.starts_with("memory:"))
    {
        return Err("请从检索结果选择一条 Event 或 Memory 引用".into());
    }
    Ok(ReadArgs {
        refs: vec![reference.into()],
        budget_tokens: RESPONSE_LIMIT,
    })
}

fn check_scope(scope: &str) -> Result<()> {
    validate_scope(scope).map_err(|_| "请选择一个有效范围".into())
}

fn validate_target(target: &str) -> Result<()> {
    if ["all", "events", "memories"].contains(&target) {
        Ok(())
    } else {
        Err("检索类型必须是 all、events 或 memories".into())
    }
}

fn file_name(path: &Path) -> String {
    truncate_utf8(&path.file_name().unwrap_or_default().to_string_lossy(), 256)
}

fn token() -> String {
    Uuid::new_v4().simple().to_string()
}

fn health_error() -> String {
    "资料库校验失败或正在更新，请检查文件完整性、Git 冲突和待恢复的 Dream 事务".into()
}

fn file_identity(file: &File) -> Result<FileIdentity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata().map_err(|_| STALE_FILE)?;
        Ok(FileIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT,
        };
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: File 保持句柄有效，information 是可写且大小正确的完整结构；此调用不接管句柄。
        let success = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) };
        if success == 0 || information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("无法安全核对所选文件的身份，请选择本地普通文件或目录".into());
        }
        // NTFS tunneling 可以保留同名替代文件的创建时间，因此不得用时间当文件标识。
        Ok(FileIdentity {
            volume_serial: information.dwVolumeSerialNumber,
            file_index: (u64::from(information.nFileIndexHigh) << 32)
                | u64::from(information.nFileIndexLow),
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok(FileIdentity {
            created: file
                .metadata()
                .map_err(|_| STALE_FILE)?
                .created()
                .map_err(|_| "此文件系统无法提供稳定文件标识，请选择本地资料库")?,
        })
    }
}

fn path_identity(path: &Path) -> Result<FileIdentity> {
    file_identity(&open_local_file(path)?)
}

fn open_local_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // 检查后若变为 FIFO 或链接，也不能阻塞桌面线程或跟随最终链接。
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        };
        // BACKUP_SEMANTICS 允许读取目录身份；OPEN_REPARSE_POINT 不跟随最终重解析点。
        options.custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options
        .open(path)
        .map_err(|_| "无法读取所选文件，请检查访问权限".into())
}

fn reject_selected_file_symlinks(path: &Path) -> Result<()> {
    #[cfg(not(target_os = "macos"))]
    {
        reject_symlink(path)
    }
    #[cfg(target_os = "macos")]
    {
        for ancestor in path.ancestors() {
            if ancestor.as_os_str().is_empty() {
                continue;
            }
            let metadata = fs::symlink_metadata(ancestor).map_err(|_| STALE_FILE)?;
            if !metadata.is_symlink() {
                continue;
            }
            // macOS 原生选择器和系统临时目录会返回这些系统路径。只允许根部的
            // 三个确切系统别名及其确切目标，不放开任意父目录或文件符号链接。
            let expected = match ancestor.to_str() {
                Some("/var") => "private/var",
                Some("/tmp") => "private/tmp",
                Some("/etc") => "private/etc",
                _ => return Err("不支持符号链接".into()),
            };
            let target = fs::read_link(ancestor).map_err(|_| STALE_FILE)?;
            if target != Path::new(expected) && target != Path::new("/").join(expected) {
                return Err("系统目录别名目标无效".into());
            }
        }
        Ok(())
    }
}

fn checked_file(expected: &FileSnapshot, limit: usize) -> Result<(FileSnapshot, String)> {
    let (current, text) = bounded_file(&expected.path, limit).map_err(|_| STALE_FILE)?;
    if &current != expected {
        return Err(STALE_FILE.into());
    }
    Ok((current, text))
}

fn bounded_file(path: &Path, limit: usize) -> Result<(FileSnapshot, String)> {
    reject_selected_file_symlinks(path).map_err(|_| "不支持符号链接，请选择本地普通文件")?;
    let path = fs::canonicalize(path).map_err(|_| "无法读取所选文件，请重新选择")?;
    let metadata = fs::metadata(&path).map_err(|_| "无法检查所选文件")?;
    if !metadata.is_file() {
        return Err("请选择普通文件，不支持目录或特殊设备".into());
    }
    let file = open_local_file(&path)?;
    let before = file.metadata().map_err(|_| "无法检查所选文件")?;
    if !before.is_file() {
        return Err("请选择普通文件，不支持目录或特殊设备".into());
    }
    if before.len() > limit as u64 {
        return Err(format!(
            "文件超过 {} MiB 上限，请分批处理",
            limit / 1024 / 1024
        ));
    }
    let identity = file_identity(&file)?;
    let mut bytes = Vec::with_capacity(before.len() as usize);
    (&file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "文件读取中断，请重新选择")?;
    let after = file.metadata().map_err(|_| STALE_FILE)?;
    if bytes.len() > limit
        || bytes.len() as u64 != before.len()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || file_identity(&file)? != identity
        || path_identity(&path)? != identity
    {
        return Err(STALE_FILE.into());
    }
    let digest = hash(&bytes);
    let text = String::from_utf8(bytes).map_err(|_| "文件不是有效 UTF-8 文本")?;
    Ok((
        FileSnapshot {
            path,
            identity,
            hash: digest,
            bytes: text.len(),
        },
        text,
    ))
}
