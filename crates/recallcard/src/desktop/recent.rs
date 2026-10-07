//! 仅保存原生宿主选定的最后工作区；不恢复待确认内容、后台任务或写入授权。
use super::{
    bounded_bytes, check_scope, checked_bytes, file_identity, hash, open_local_file, path_identity,
    DesktopSession, FileIdentity, FileSnapshot, VaultInfo,
};
use crate::model::Result;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Write,
    path::{Component, Path, PathBuf},
};
use tempfile::NamedTempFile;

const SCHEMA: &str = "recallcard.desktop-workspace/1";
const CONFIG_LIMIT: usize = 32 * 1024;
const CONFIG_ERROR: &str = "无法安全读取上次工作区设置，请重新选择资料库；现有资料不会被替换";
const SAVE_ERROR: &str = "无法保存上次工作区设置；本次打开的资料库仍可继续使用";
const WORKSPACE_ERROR: &str =
    "上次资料库的位置、文件或访问权限已改变，请重新选择资料库；不会自动创建或替换";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoredWorkspace {
    pub vault: VaultInfo,
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkspaceAccess {
    root: AccessSnapshot,
    marker: AccessSnapshot,
}

impl WorkspaceAccess {
    pub(super) fn capture(root: &Path, marker: &Path) -> Result<Self> {
        Ok(Self {
            root: access_at(root)?,
            marker: access_at(marker)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessSnapshot {
    readonly: bool,
    #[cfg(unix)]
    owner: u32,
    #[cfg(unix)]
    group: u32,
    #[cfg(unix)]
    mode: u32,
    #[cfg(windows)]
    security_hash: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceBinding {
    root: PathBuf,
    identity: FileIdentity,
    marker: FileSnapshot,
    access: WorkspaceAccess,
    scope: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StorageBinding {
    directory_identity: FileIdentity,
    directory_access: AccessSnapshot,
    file_identity: FileIdentity,
    file_access: AccessSnapshot,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RememberedWorkspace {
    schema: String,
    workspace: WorkspaceBinding,
    storage: StorageBinding,
    digest: String,
}

impl RememberedWorkspace {
    fn digest(&self) -> Result<String> {
        let bytes = serde_json::to_vec(&(&self.schema, &self.workspace, &self.storage))
            .map_err(|_| CONFIG_ERROR)?;
        Ok(hash(&bytes))
    }
}

impl DesktopSession {
    /// `config_path` 只能由宿主固定，不能直接来自前端参数或导入文件。
    /// 保存失败不会撤销当前资料库会话，也不会改变任何 Vault 内容。
    pub fn remember_workspace(
        &self,
        session_id: &str,
        scope: &str,
        config_path: &Path,
    ) -> Result<()> {
        check_scope(scope)?;
        let vault = self.vault(session_id)?;
        let selected = self.selected.as_ref().ok_or(WORKSPACE_ERROR)?;
        let binding = WorkspaceBinding {
            root: vault.root().to_owned(),
            identity: selected.identity.clone(),
            marker: selected.marker.clone(),
            access: selected.access.clone(),
            scope: scope.into(),
        };
        validate_workspace(&binding)?;
        save_workspace(config_path, binding).map_err(|_| SAVE_ERROR.into())
    }

    /// 只打开已经存在且身份未改变的资料库；绝不把恢复失败降级成创建。
    /// 首次使用没有设置时返回 `None`，损坏/替换/权限变化返回中文错误。
    pub fn restore_workspace(&mut self, config_path: &Path) -> Result<Option<RestoredWorkspace>> {
        // 只允许在空白启动页恢复；延迟到达的启动请求不能覆盖用户刚打开的库。
        if self.selected.is_some() {
            return Err("已有打开的资料库，请继续使用当前工作区".into());
        }
        let Some((config, snapshot)) = load_workspace(config_path)? else {
            return Ok(None);
        };
        validate_workspace(&config.workspace)?;
        // 恢复前验证完整目录；select_vault(false) 也使用不补建的只读打开入口。
        validate_layout(&config.workspace.root)?;
        checked_bytes(&snapshot, CONFIG_LIMIT).map_err(|_| CONFIG_ERROR)?;
        let info = self
            .select_vault(&config.workspace.root, false)
            .map_err(|_| WORKSPACE_ERROR)?;
        // 选择过程同样会检查健康状态；返回前再核对身份和权限。
        if validate_workspace(&config.workspace).is_err()
            || checked_bytes(&snapshot, CONFIG_LIMIT).is_err()
        {
            self.close_vault();
            return Err(WORKSPACE_ERROR.into());
        }
        Ok(Some(RestoredWorkspace {
            vault: info,
            scope: config.workspace.scope,
        }))
    }
}

fn validate_workspace(binding: &WorkspaceBinding) -> Result<()> {
    check_scope(&binding.scope).map_err(|_| WORKSPACE_ERROR)?;
    if !normal_absolute(&binding.root)
        || binding.marker.path != binding.root.join("control/schema-version.json")
        || binding.marker.bytes > 4096
        || binding.marker.hash.len() != 64
    {
        return Err(WORKSPACE_ERROR.into());
    }
    reject_links(&binding.root).map_err(|_| WORKSPACE_ERROR)?;
    let file = open_local_file(&binding.root).map_err(|_| WORKSPACE_ERROR)?;
    if !file.metadata().map_err(|_| WORKSPACE_ERROR)?.is_dir()
        || file_identity(&file).map_err(|_| WORKSPACE_ERROR)? != binding.identity
        || WorkspaceAccess::capture(&binding.root, &binding.marker.path)
            .map_err(|_| WORKSPACE_ERROR)?
            != binding.access
    {
        return Err(WORKSPACE_ERROR.into());
    }
    checked_bytes(&binding.marker, 4096).map_err(|_| WORKSPACE_ERROR)?;
    Ok(())
}

fn validate_layout(root: &Path) -> Result<()> {
    for name in [
        "events",
        "memories",
        "control",
        "objects",
        "control/dream-receipts",
        "control/suppressions",
    ] {
        let path = root.join(name);
        reject_links(&path).map_err(|_| WORKSPACE_ERROR)?;
        let file = open_local_file(&path).map_err(|_| WORKSPACE_ERROR)?;
        if !file.metadata().map_err(|_| WORKSPACE_ERROR)?.is_dir() {
            return Err(WORKSPACE_ERROR.into());
        }
    }
    Ok(())
}

fn normal_absolute(path: &Path) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
}

// 与 Vault 文件选择一致接受 macOS 的三个确切系统别名，其余链接一律拒绝。
// Windows 必须同时拒绝 junction 等不是普通 symlink 的重解析点。
fn reject_links(path: &Path) -> Result<()> {
    for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        let metadata = match fs::symlink_metadata(ancestor) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(CONFIG_ERROR.into()),
        };
        #[cfg(target_os = "macos")]
        if metadata.is_symlink() {
            let expected = match ancestor.to_str() {
                Some("/var") => "private/var",
                Some("/tmp") => "private/tmp",
                Some("/etc") => "private/etc",
                _ => return Err(CONFIG_ERROR.into()),
            };
            let target = fs::read_link(ancestor).map_err(|_| CONFIG_ERROR)?;
            if target == Path::new(expected) || target == Path::new("/").join(expected) {
                continue;
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(CONFIG_ERROR.into());
            }
        }
        if metadata.is_symlink() {
            return Err(CONFIG_ERROR.into());
        }
    }
    Ok(())
}

fn config_target(path: &Path) -> Result<()> {
    if !normal_absolute(path) || path.file_name().is_none() {
        return Err(CONFIG_ERROR.into());
    }
    reject_links(path)
}

fn load_workspace(path: &Path) -> Result<Option<(RememberedWorkspace, FileSnapshot)>> {
    config_target(path)?;
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(CONFIG_ERROR.into()),
        Ok(_) => {}
    }
    let parent = path.parent().ok_or(CONFIG_ERROR)?;
    let directory = checked_config_directory(parent)?;
    let file = checked_config_file(path)?;
    let (snapshot, bytes) = bounded_bytes(path, CONFIG_LIMIT).map_err(|_| CONFIG_ERROR)?;
    let config: RememberedWorkspace = serde_json::from_slice(&bytes).map_err(|_| CONFIG_ERROR)?;
    if config.schema != SCHEMA
        || config.digest != config.digest()?
        || config.storage.directory_identity != file_identity(&directory)?
        || config.storage.directory_access != access(&directory)?
        || config.storage.file_identity != file_identity(&file)?
        || config.storage.file_access != access(&file)?
        || snapshot.identity != file_identity(&file)?
        || snapshot.path.starts_with(&config.workspace.root)
    {
        return Err(CONFIG_ERROR.into());
    }
    Ok(Some((config, snapshot)))
}

fn save_workspace(path: &Path, workspace: WorkspaceBinding) -> Result<()> {
    config_target(path)?;
    let parent = path.parent().ok_or(SAVE_ERROR)?;
    // 先解析最近存在的祖先，尚未创建设置目录前就阻止把配置放进 Vault。
    let mut existing = parent;
    while !existing.exists() {
        existing = existing.parent().ok_or(SAVE_ERROR)?;
    }
    let canonical_existing = fs::canonicalize(existing).map_err(|_| SAVE_ERROR)?;
    let relative = parent.strip_prefix(existing).map_err(|_| SAVE_ERROR)?;
    if canonical_existing
        .join(relative)
        .starts_with(&workspace.root)
    {
        return Err(SAVE_ERROR.into());
    }
    make_config_directory(parent)?;
    let directory = checked_config_directory(parent)?;
    let directory_identity = file_identity(&directory)?;
    let directory_access = access(&directory)?;
    let canonical_parent = fs::canonicalize(parent).map_err(|_| SAVE_ERROR)?;
    if canonical_parent.starts_with(&workspace.root) {
        return Err(SAVE_ERROR.into());
    }
    let target = canonical_parent.join(path.file_name().ok_or(SAVE_ERROR)?);
    let previous = match fs::symlink_metadata(&target) {
        Ok(_) => {
            let file = checked_config_file(&target)?;
            Some((file_identity(&file)?, access(&file)?))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err(SAVE_ERROR.into()),
    };
    // tempfile 在 Unix 上用 0600；Windows 继承宿主用户配置目录的 ACL。
    // 不修改任何现有目录或文件的权限。
    let mut temporary = NamedTempFile::new_in(&canonical_parent).map_err(|_| SAVE_ERROR)?;
    let mut config = RememberedWorkspace {
        schema: SCHEMA.into(),
        workspace,
        storage: StorageBinding {
            directory_identity: directory_identity.clone(),
            directory_access: directory_access.clone(),
            file_identity: file_identity(temporary.as_file())?,
            file_access: access(temporary.as_file())?,
        },
        digest: String::new(),
    };
    config.digest = config.digest()?;
    let bytes = serde_json::to_vec_pretty(&config).map_err(|_| SAVE_ERROR)?;
    if bytes.len() > CONFIG_LIMIT {
        return Err(SAVE_ERROR.into());
    }
    temporary.write_all(&bytes).map_err(|_| SAVE_ERROR)?;
    temporary.as_file().sync_all().map_err(|_| SAVE_ERROR)?;
    validate_workspace(&config.workspace)?;
    reject_links(path)?;
    if path_identity(&canonical_parent)? != directory_identity
        || access_at(&canonical_parent)? != directory_access
    {
        return Err(SAVE_ERROR.into());
    }
    match previous {
        Some((identity, permissions)) => {
            let file = checked_config_file(&target)?;
            if file_identity(&file)? != identity || access(&file)? != permissions {
                return Err(SAVE_ERROR.into());
            }
            drop(file);
            temporary.persist(&target).map_err(|_| SAVE_ERROR)?;
        }
        None => {
            temporary
                .persist_noclobber(&target)
                .map_err(|_| SAVE_ERROR)?;
        }
    }
    #[cfg(unix)]
    directory.sync_all().map_err(|_| SAVE_ERROR)?;
    Ok(())
}

fn make_config_directory(path: &Path) -> Result<()> {
    reject_links(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => return Ok(()),
        Ok(_) => return Err(SAVE_ERROR.into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(SAVE_ERROR.into()),
    }
    let parent = path.parent().ok_or(SAVE_ERROR)?;
    make_config_directory(parent)?;
    // 创建用户私有的新目录，已有目录从不 chmod；每层创建后再次验证链接。
    #[cfg(unix)]
    let mut builder = fs::DirBuilder::new();
    #[cfg(not(unix))]
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(SAVE_ERROR.into()),
    }
    reject_links(path)?;
    checked_config_directory(path)?;
    Ok(())
}

fn checked_config_directory(path: &Path) -> Result<File> {
    reject_links(path)?;
    let file = open_local_file(path).map_err(|_| CONFIG_ERROR)?;
    let metadata = file.metadata().map_err(|_| CONFIG_ERROR)?;
    if !metadata.is_dir() {
        return Err(CONFIG_ERROR.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid 无参数，无资源所有权或指针。
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o022 != 0 {
            return Err(CONFIG_ERROR.into());
        }
    }
    #[cfg(windows)]
    check_windows_private(&file)?;
    Ok(file)
}

fn checked_config_file(path: &Path) -> Result<File> {
    reject_links(path)?;
    let file = open_local_file(path).map_err(|_| CONFIG_ERROR)?;
    let metadata = file.metadata().map_err(|_| CONFIG_ERROR)?;
    // 恢复读取另由 bounded_bytes 限制大小。用户明确重新选库后的保存只核对
    // 旧文件身份/权限，不读取旧正文，因此也可以原子替换损坏或超限的设置。
    if !metadata.is_file() {
        return Err(CONFIG_ERROR.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // 拒绝共享读取/写入和硬链接，防止本机位置配置被其他用户控制。
        // SAFETY: geteuid 无参数，无资源所有权或指针。
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
        {
            return Err(CONFIG_ERROR.into());
        }
    }
    #[cfg(windows)]
    check_windows_private(&file)?;
    Ok(file)
}

fn access_at(path: &Path) -> Result<AccessSnapshot> {
    reject_links(path)?;
    access(&open_local_file(path).map_err(|_| WORKSPACE_ERROR)?)
}

fn access(file: &File) -> Result<AccessSnapshot> {
    let metadata = file.metadata().map_err(|_| WORKSPACE_ERROR)?;
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    Ok(AccessSnapshot {
        readonly: metadata.permissions().readonly(),
        #[cfg(unix)]
        owner: metadata.uid(),
        #[cfg(unix)]
        group: metadata.gid(),
        #[cfg(unix)]
        mode: metadata.mode(),
        #[cfg(windows)]
        security_hash: windows_security_hash(file)?,
    })
}

#[cfg(windows)]
fn windows_security_hash(file: &File) -> Result<String> {
    use std::{os::windows::io::AsRawHandle, ptr};
    use windows_sys::Win32::Security::{
        GetKernelObjectSecurity, DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION,
        OWNER_SECURITY_INFORMATION,
    };
    let requested =
        DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION;
    let mut size = 0;
    // SAFETY: 首次查询只取得长度；第二次传入足够且仍存活的可写字节缓冲区。
    unsafe {
        GetKernelObjectSecurity(
            file.as_raw_handle(),
            requested,
            ptr::null_mut(),
            0,
            &mut size,
        );
    }
    if size == 0 || size > 65536 {
        return Err(WORKSPACE_ERROR.into());
    }
    let mut descriptor = vec![0_u8; size as usize];
    let success = unsafe {
        GetKernelObjectSecurity(
            file.as_raw_handle(),
            requested,
            descriptor.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    };
    if success == 0 || size as usize > descriptor.len() {
        return Err(WORKSPACE_ERROR.into());
    }
    descriptor.truncate(size as usize);
    Ok(hash(&descriptor))
}

#[cfg(windows)]
fn check_windows_private(file: &File) -> Result<()> {
    use std::{ffi::c_void, mem::size_of, os::windows::io::AsRawHandle, ptr};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree, HANDLE},
        Security::{
            Authorization::{GetSecurityInfo, SE_FILE_OBJECT},
            EqualSid, GetAce, GetTokenInformation, IsValidSid, IsWellKnownSid, TokenUser,
            WinBuiltinAdministratorsSid, WinLocalSystemSid, ACCESS_ALLOWED_ACE, ACE_HEADER,
            DACL_SECURITY_INFORMATION, INHERIT_ONLY_ACE, OWNER_SECURITY_INFORMATION, TOKEN_QUERY,
            TOKEN_USER,
        },
        Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    struct Token(HANDLE);
    impl Drop for Token {
        fn drop(&mut self) {
            // SAFETY: 此结构独占 OpenProcessToken 返回的有效句柄。
            unsafe { CloseHandle(self.0) };
        }
    }
    struct Descriptor(*mut c_void);
    impl Drop for Descriptor {
        fn drop(&mut self) {
            // SAFETY: GetSecurityInfo 分配的描述符仅在此释放一次。
            unsafe { LocalFree(self.0) };
        }
    }
    // SAFETY: 系统填充的缓冲区和句柄均在作用域中保持有效；TokenUser 缓冲区
    // 按 usize 对齐，SID/ACE 指针只在所属系统描述符存活时读取。
    unsafe {
        let mut handle = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) == 0 {
            return Err(CONFIG_ERROR.into());
        }
        let token = Token(handle);
        let mut length = 0;
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut length);
        if length < size_of::<TOKEN_USER>() as u32 || length > 65536 {
            return Err(CONFIG_ERROR.into());
        }
        let mut user_data = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
        if GetTokenInformation(
            token.0,
            TokenUser,
            user_data.as_mut_ptr().cast(),
            length,
            &mut length,
        ) == 0
        {
            return Err(CONFIG_ERROR.into());
        }
        let user = &*user_data.as_ptr().cast::<TOKEN_USER>();
        let mut owner = ptr::null_mut();
        let mut dacl = ptr::null_mut();
        let mut descriptor = ptr::null_mut();
        if GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        ) != 0
        {
            return Err(CONFIG_ERROR.into());
        }
        let _descriptor = Descriptor(descriptor);
        if owner.is_null() || dacl.is_null() || EqualSid(owner, user.User.Sid) == 0 {
            return Err(CONFIG_ERROR.into());
        }
        for index in 0..u32::from((*dacl).AceCount) {
            let mut ace = ptr::null_mut();
            if GetAce(dacl, index, &mut ace) == 0 || ace.is_null() {
                return Err(CONFIG_ERROR.into());
            }
            let header = &*ace.cast::<ACE_HEADER>();
            // Windows SDK：ACCESS_ALLOWED_ACE_TYPE=0，ACCESS_DENIED_ACE_TYPE=1。
            // 非标准授权 ACE 一律拒绝；不自动修改原目录 ACL。
            if u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0 || header.AceType == 1 {
                continue;
            }
            if header.AceType != 0 || (header.AceSize as usize) < size_of::<ACCESS_ALLOWED_ACE>() {
                return Err(CONFIG_ERROR.into());
            }
            let allowed = &*ace.cast::<ACCESS_ALLOWED_ACE>();
            let sid = ptr::addr_of!(allowed.SidStart).cast_mut().cast();
            if IsValidSid(sid) == 0
                || (EqualSid(sid, user.User.Sid) == 0
                    && IsWellKnownSid(sid, WinLocalSystemSid) == 0
                    && IsWellKnownSid(sid, WinBuiltinAdministratorsSid) == 0)
            {
                return Err(CONFIG_ERROR.into());
            }
        }
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        if GetFileInformationByHandle(file.as_raw_handle(), &mut information) == 0
            || (file.metadata().map_err(|_| CONFIG_ERROR)?.is_file()
                && information.nNumberOfLinks != 1)
        {
            return Err(CONFIG_ERROR.into());
        }
    }
    Ok(())
}
