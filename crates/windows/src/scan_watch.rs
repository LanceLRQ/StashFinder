use anyhow::{Context, Result, ensure};
use std::{
    fs::{File, OpenOptions},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::{Path, PathBuf},
};
use windows::Win32::{
    Foundation::*,
    Globalization::{CSTR_EQUAL, CompareStringOrdinal},
    Storage::FileSystem::*,
    System::{IO::*, Threading::*},
};

fn guid_path(path: &Path) -> Result<Vec<u16>> {
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode(7)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)?;
    let mut buffer = vec![0; 32768];
    // SAFETY: The owned handle and writable buffer remain valid for the call.
    let size = unsafe {
        GetFinalPathNameByHandleW(HANDLE(file.as_raw_handle()), &mut buffer, VOLUME_NAME_GUID)
    } as usize;
    ensure!(
        size > 0 && size < buffer.len(),
        "无法解析运行数据目录的卷路径"
    );
    buffer.truncate(size);
    while buffer.last() == Some(&92) {
        buffer.pop();
    }
    Ok(buffer)
}
pub(super) fn under(path: &[u16], root: &[u16]) -> bool {
    path.len()>=root.len() && (path.len()==root.len() || path[root.len()]==92)
        // SAFETY: Both bounded slices remain valid; ordinal comparison matches Windows path casing.
        && unsafe{CompareStringOrdinal(&path[..root.len()],root,true)}==CSTR_EQUAL
}
pub(super) fn relative_roots(root: &Path, excluded: &[PathBuf]) -> Result<Vec<Vec<u16>>> {
    let root = guid_path(root)?;
    let mut result = Vec::new();
    for path in excluded {
        if !path.try_exists()? {
            continue;
        }
        let path = guid_path(path)?;
        if under(&path, &root) {
            ensure!(path.len() > root.len(), "不能排除整个扫描根目录");
            result.push(path[root.len() + 1..].to_vec());
        }
    }
    Ok(result)
}

pub(super) struct Watch {
    file: File,
    buffer: Box<[u32; 16384]>,
    overlapped: Box<OVERLAPPED>,
    pending: bool,
}
impl Watch {
    pub(super) fn open(root: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .access_mode(FILE_LIST_DIRECTORY.0)
            .share_mode(7)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OVERLAPPED.0)
            .open(root)?;
        // SAFETY: Creates an unnamed event owned by this watcher.
        let event = unsafe { CreateEventW(None, true, false, None) }?;
        let mut watch = Self {
            file,
            buffer: Box::new([0; 16384]),
            overlapped: Box::new(OVERLAPPED {
                hEvent: event,
                ..Default::default()
            }),
            pending: false,
        };
        watch.arm()?;
        Ok(watch)
    }
    fn arm(&mut self) -> Result<()> {
        // SAFETY: Pinned heap allocations and file/event handles outlive completion or cancellation.
        unsafe {
            ResetEvent(self.overlapped.hEvent)?;
            ReadDirectoryChangesW(
                HANDLE(self.file.as_raw_handle()),
                self.buffer.as_mut_ptr().cast(),
                65536,
                true,
                FILE_NOTIFY_CHANGE_FILE_NAME
                    | FILE_NOTIFY_CHANGE_DIR_NAME
                    | FILE_NOTIFY_CHANGE_SIZE
                    | FILE_NOTIFY_CHANGE_LAST_WRITE
                    | FILE_NOTIFY_CHANGE_ATTRIBUTES
                    | FILE_NOTIFY_CHANGE_SECURITY,
                None,
                Some(self.overlapped.as_mut()),
                None,
            )?;
        }
        self.pending = true;
        Ok(())
    }
    pub(super) fn changed(&mut self, excluded: &[Vec<u16>]) -> Result<bool> {
        // SAFETY: The event belongs to the in-flight request; the zero timeout does not block.
        if unsafe { WaitForSingleObject(self.overlapped.hEvent, 0) } == WAIT_TIMEOUT {
            return Ok(false);
        }
        let mut count = 0;
        // SAFETY: The request and all of its storage remain alive until completion.
        unsafe {
            GetOverlappedResult(
                HANDLE(self.file.as_raw_handle()),
                self.overlapped.as_ref(),
                &mut count,
                false,
            )
        }?;
        self.pending = false;
        ensure!(count <= 65536, "目录监控返回长度无效");
        // SAFETY: IO has completed and count is validated against the initialized allocation.
        let bytes = unsafe {
            std::slice::from_raw_parts(self.buffer.as_ptr().cast::<u8>(), count as usize)
        };
        let mut changed = count == 0; // Buffer overflow requires a full validation.
        let mut offset = 0;
        while offset < bytes.len() {
            let record = &bytes[offset..];
            let next = crate::volumes::read_field::<u32>(record, 0)? as usize;
            let size = crate::volumes::read_field::<u32>(record, 8)? as usize;
            ensure!(size.is_multiple_of(2), "目录通知名称长度无效");
            let name = record
                .get(12..12 + size)
                .context("目录通知截断")?
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes([v[0], v[1]]))
                .collect::<Vec<_>>();
            if !excluded.iter().any(|root| under(&name, root)) {
                changed = true;
            }
            if next == 0 {
                break;
            }
            ensure!(next >= 12 + size && next < record.len(), "目录通知偏移无效");
            offset += next;
        }
        self.arm()?;
        Ok(changed)
    }
}
impl Drop for Watch {
    fn drop(&mut self) {
        // SAFETY: Cancel and drain before freeing the buffers; close the event exactly once.
        unsafe {
            if self.pending {
                let _ = CancelIoEx(
                    HANDLE(self.file.as_raw_handle()),
                    Some(self.overlapped.as_ref()),
                );
                let mut count = 0;
                let _ = GetOverlappedResult(
                    HANDLE(self.file.as_raw_handle()),
                    self.overlapped.as_ref(),
                    &mut count,
                    true,
                );
            }
            let _ = CloseHandle(self.overlapped.hEvent);
        }
    }
}
