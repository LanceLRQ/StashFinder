use anyhow::{Context, Result, ensure};
use stashfinder_core::file_operations::{ExpectedIdentity, NativeIdentity, PreparedTarget};
use std::{
    fs::OpenOptions,
    os::windows::{ffi::OsStringExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::{Path, PathBuf},
};
use windows::Win32::{Foundation::HANDLE, Storage::FileSystem::*};
#[path = "file_edit.rs"]
mod edit;
pub use edit::{Edit, execute};
#[path = "file_clipboard.rs"]
mod clipboard;
pub use clipboard::paste;

#[derive(Clone)]
pub struct FileInfo {
    pub identity: NativeIdentity,
    pub volume64: Option<u64>,
    pub attributes: u32,
    pub size: Option<i64>,
    pub created: Option<u64>,
    pub modified: Option<u64>,
}

pub fn inspect(path: &Path) -> Result<FileInfo> {
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: The ordinary file handle and output buffer remain owned throughout the call.
    unsafe {
        GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)?;
    }
    ensure!(
        info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0,
        "暂不操作重解析点"
    );
    let created = (u64::from(info.ftCreationTime.dwHighDateTime) << 32)
        | u64::from(info.ftCreationTime.dwLowDateTime);
    let modified = (u64::from(info.ftLastWriteTime.dwHighDateTime) << 32)
        | u64::from(info.ftLastWriteTime.dwLowDateTime);
    let size = (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow);
    let mut id = FILE_ID_INFO::default();
    // SAFETY: The same owned handle is queried into an exactly sized FILE_ID_INFO buffer.
    let volume64 = unsafe {
        GetFileInformationByHandleEx(
            HANDLE(file.as_raw_handle()),
            FileIdInfo,
            (&mut id as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    }
    .ok()
    .map(|_| id.VolumeSerialNumber);
    Ok(FileInfo {
        volume64,
        identity: NativeIdentity {
            volume: info.dwVolumeSerialNumber,
            file: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        },
        attributes: info.dwFileAttributes,
        size: if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0 {
            Some(size.try_into()?)
        } else {
            None
        },
        created: (created != 0).then_some(created),
        modified: (modified != 0).then_some(modified),
    })
}

pub fn verify(target: &PreparedTarget, destination: bool) -> Result<PathBuf> {
    if destination {
        let mut flags = 0;
        // SAFETY: The owned volume GUID name and flags output remain valid for the call.
        unsafe {
            GetVolumeInformationW(
                &windows::core::HSTRING::from(&target.volume),
                None,
                None,
                None,
                Some(&mut flags),
                None,
            )?;
        }
        ensure!(
            flags & windows::Win32::System::SystemServices::FILE_READ_ONLY_VOLUME == 0,
            "目标分区只读"
        );
    }
    let path = verify_with(target, destination, inspect)?;
    shell_path(&path)
}

/// Shell operations need a mounted DOS path; FILE_NAME_OPENED preserves the selected hard-link name.
pub(crate) fn shell_path(path: &Path) -> Result<PathBuf> {
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)?;
    let mut name = vec![0u16; 32768];
    // SAFETY: The handle and writable buffer remain owned throughout this path query.
    let len = unsafe {
        GetFinalPathNameByHandleW(HANDLE(file.as_raw_handle()), &mut name, FILE_NAME_OPENED)
    } as usize;
    ensure!(
        len > 0 && len < name.len(),
        "无法取得当前挂载路径：{}",
        std::io::Error::last_os_error()
    );
    name.truncate(len);
    ensure!(
        name.starts_with(&[92, 92, 63, 92]) && name.get(5) == Some(&58),
        "该分区未挂载为可操作的本地路径"
    );
    let resolved = PathBuf::from(std::ffi::OsString::from_wide(&name[4..]));
    ensure!(
        inspect(path)?.identity == inspect(&resolved)?.identity,
        "挂载路径身份已变化"
    );
    Ok(resolved)
}

/// The adapter permits deterministic tests of the native boundary without writing a disk.
pub fn verify_with(
    target: &PreparedTarget,
    destination: bool,
    inspect: impl Fn(&Path) -> Result<FileInfo>,
) -> Result<PathBuf> {
    let root = PathBuf::from(&target.volume);
    let root_info = inspect(&root)?;
    let serial = target.identity.serial.as_ref().context("分区序列号未知")?;
    let expected = u64::from_str_radix(&serial.value, 16).context("分区序列号无效")?;
    ensure!(
        match serial.source.as_str() {
            "ntfs64" => root_info.volume64 == Some(expected),
            "win32_volume32" => u64::from(root_info.identity.volume) == expected,
            _ => false,
        },
        "分区身份已变化"
    );
    let volume32 = root_info.identity.volume;
    let mut path = root;
    let mut last = root_info;
    for component in target
        .file
        .relative
        .split(|c| *c == 92)
        .filter(|c| !c.is_empty())
    {
        ensure!(
            last.attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0
                && last.attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0,
            "路径父节点不是普通目录"
        );
        path.push(std::ffi::OsString::from_wide(component));
        last = inspect(&path)?;
        ensure!(last.identity.volume == volume32, "路径进入了其他分区");
    }
    ensure!(
        last.attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0,
        "暂不操作重解析点"
    );
    ensure!(
        (last.attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0) == target.file.folder,
        "文件类型已变化"
    );
    if !target.file.relative.is_empty() {
        ensure!(
            match &target.file.identity {
                Some(ExpectedIdentity::Ntfs { frn }) => last.identity.file == *frn,
                Some(ExpectedIdentity::Native(id)) =>
                    last.identity == *id
                        && (target.file.folder
                            || target.file.size.is_none_or(|v| last.size == Some(v)))
                        && target.file.created.is_none_or(|v| last.created == Some(v))
                        && (target.file.folder
                            || target
                                .file
                                .modified
                                .is_none_or(|v| last.modified == Some(v))),
                None => false,
            },
            "文件身份未知或已变化，请更新索引后重试"
        );
    } else {
        ensure!(destination, "不能操作分区根目录");
    }
    if destination {
        ensure!(target.file.folder, "粘贴目标不是目录");
    }
    Ok(path)
}
