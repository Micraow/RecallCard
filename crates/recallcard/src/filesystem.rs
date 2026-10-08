//! 本机文件身份与无跟随打开；应用和桌面共用。
use crate::model::Result;
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    path::Path,
};
const STALE_FILE: &str = "文件身份已改变或无法核验";
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileIdentity {
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

pub(crate) fn file_identity(file: &File) -> Result<FileIdentity> {
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

pub(crate) fn open_local_file(path: &Path) -> Result<File> {
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
