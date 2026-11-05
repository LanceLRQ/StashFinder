use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::OsString,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::MetadataExt,
    },
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
use windows::{
    Win32::{
        Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Storage::FileSystem::*,
        System::Threading::WaitForSingleObject,
    },
    core::HSTRING,
};

#[derive(Clone, Debug)]
pub struct ScanEntry {
    pub identity: Option<stashfinder_core::file_operations::NativeIdentity>,
    pub name: Vec<u16>,
    pub size: Option<i64>,
    pub created: Option<u64>,
    pub modified: Option<u64>,
    pub attributes: u32,
}

pub struct Scanner {
    root: PathBuf,
    notification: HANDLE,
}
impl Scanner {
    pub fn open(path: &Path) -> Result<Self> {
        let root = if path.as_os_str().to_string_lossy().starts_with(r"\\?\") {
            path.to_path_buf()
        } else {
            std::fs::canonicalize(path).context("无法打开扫描目录")?
        };
        let flags = FILE_NOTIFY_CHANGE_FILE_NAME
            | FILE_NOTIFY_CHANGE_DIR_NAME
            | FILE_NOTIFY_CHANGE_SIZE
            | FILE_NOTIFY_CHANGE_LAST_WRITE
            | FILE_NOTIFY_CHANGE_ATTRIBUTES
            | FILE_NOTIFY_CHANGE_SECURITY;
        // SAFETY: The absolute path is owned for the call; the returned notification is owned by Self.
        let notification =
            unsafe { FindFirstChangeNotificationW(&HSTRING::from(root.as_os_str()), true, flags) }
                .context("无法建立目录监控")?;
        Ok(Self { root, notification })
    }
    pub fn changed(&mut self) -> Result<bool> {
        // SAFETY: The owned notification remains open; a zero timeout never blocks the worker.
        match unsafe { WaitForSingleObject(self.notification, 0) } {
            WAIT_TIMEOUT => Ok(false),
            WAIT_OBJECT_0 => {
                // SAFETY: Re-arm only after a signalled wait, as required by the API.
                unsafe { FindNextChangeNotification(self.notification) }
                    .context("目录监控失效，需要完整校验")?;
                Ok(true)
            }
            _ => bail!("目录监控等待失败：{}", windows::core::Error::from_thread()),
        }
    }
    pub fn enumerate(
        &self,
        relative: &[u16],
        cancelled: &AtomicBool,
        consume: &mut dyn FnMut(&[ScanEntry]) -> Result<()>,
    ) -> Result<()> {
        ensure!(!cancelled.load(Ordering::Acquire), "扫描已取消");
        // Relative paths originate only from enumerated names in the service.
        let path = if relative.is_empty() {
            self.root.clone()
        } else {
            self.root.join(OsString::from_wide(relative))
        };
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("读取目录 {}", path.display()))?;
        ensure!(
            metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0,
            "目录已变成重解析点：{}",
            path.display()
        );
        let mut batch = Vec::with_capacity(256);
        for entry in
            std::fs::read_dir(&path).with_context(|| format!("枚举目录 {}", path.display()))?
        {
            ensure!(!cancelled.load(Ordering::Acquire), "扫描已取消");
            let entry = entry.with_context(|| format!("读取目录项 {}", path.display()))?;
            let metadata = std::fs::symlink_metadata(entry.path())
                .with_context(|| format!("读取元数据 {}", entry.path().display()))?;
            let attributes = metadata.file_attributes();
            let live = crate::file_operations::inspect(&entry.path()).ok();
            batch.push(ScanEntry {
                identity: live.as_ref().map(|m| m.identity.clone()),
                name: entry.file_name().encode_wide().collect(),
                size: if let Some(live) = &live {
                    live.size
                } else if attributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0 {
                    Some(metadata.file_size().try_into()?)
                } else {
                    None
                },
                created: live.as_ref().map_or_else(
                    || (metadata.creation_time() != 0).then_some(metadata.creation_time()),
                    |m| m.created,
                ),
                modified: live.as_ref().map_or_else(
                    || (metadata.last_write_time() != 0).then_some(metadata.last_write_time()),
                    |m| m.modified,
                ),
                attributes: live.as_ref().map_or(attributes, |m| m.attributes),
            });
            if batch.len() == 256 {
                consume(&batch)?;
                batch.clear();
            }
        }
        if !batch.is_empty() {
            consume(&batch)?;
        }
        Ok(())
    }
}
impl Drop for Scanner {
    fn drop(&mut self) {
        // SAFETY: This closes exactly the notification handle owned by this scanner.
        unsafe {
            let _ = FindCloseChangeNotification(self.notification);
        }
    }
}
