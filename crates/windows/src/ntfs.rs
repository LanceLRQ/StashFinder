use crate::volumes::read_field;
use anyhow::{Context, Result, ensure};
use std::{
    os::windows::ffi::OsStringExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
use windows::{
    Win32::{
        Foundation::*,
        Storage::FileSystem::*,
        System::{IO::DeviceIoControl, Ioctl::*},
    },
    core::{HSTRING, PWSTR},
};

#[derive(Clone, Debug)]
pub struct UsnRecord {
    pub id: u64,
    pub parent: u64,
    pub usn: i64,
    pub reason: u32,
    pub attributes: u32,
    pub name: Vec<u16>,
}
pub struct UsnPage {
    pub next: u64,
    pub records: Vec<UsnRecord>,
}
#[derive(Clone, Debug)]
pub struct DirectoryEntry {
    pub id: u64,
    pub name: Vec<u16>,
    pub attributes: u32,
    pub size: i64,
    pub created: u64,
    pub modified: u64,
}
#[derive(Clone, Debug)]
pub struct Journal {
    pub id: u64,
    pub first: i64,
    pub next: i64,
}
pub struct NativeObject {
    pub entry: DirectoryEntry,
    pub links: Vec<(u64, Vec<u16>)>,
}

pub fn decode_usn(bytes: &[u8]) -> Result<UsnPage> {
    let next = read_field::<u64>(bytes, 0)?;
    let mut records = Vec::new();
    let mut offset = 8usize;
    while offset < bytes.len() {
        let record = bytes.get(offset..).context("USN 记录截断")?;
        let length = read_field::<u32>(record, 0)? as usize;
        ensure!(
            length >= 60 && length.is_multiple_of(8) && length <= record.len(),
            "USN 记录长度无效"
        );
        let record = &record[..length];
        ensure!(
            u16::from_le_bytes(record[4..6].try_into()?) == 2,
            "暂不支持此 USN 记录版本，未跳过记录"
        );
        let size = u16::from_le_bytes(record[56..58].try_into()?) as usize;
        let start = u16::from_le_bytes(record[58..60].try_into()?) as usize;
        ensure!(start >= 60 && size.is_multiple_of(2), "USN 名称字段无效");
        let name = record
            .get(start..start.checked_add(size).context("USN 名称溢出")?)
            .context("USN 名称截断")?;
        records.push(UsnRecord {
            id: read_field(record, 8)?,
            parent: read_field(record, 16)?,
            usn: read_field(record, 24)?,
            reason: read_field(record, 40)?,
            attributes: read_field(record, 52)?,
            name: name
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect(),
        });
        offset += length;
    }
    Ok(UsnPage { next, records })
}

pub fn decode_directory(bytes: &[u8]) -> Result<Vec<DirectoryEntry>> {
    let mut rows = Vec::new();
    let mut offset = 0usize;
    loop {
        let record = bytes.get(offset..).context("目录记录截断")?;
        let next = read_field::<u32>(record, 0)? as usize;
        let length = read_field::<u32>(record, 60)? as usize;
        ensure!(length > 0 && length.is_multiple_of(2), "目录名称长度无效");
        let end = 104usize.checked_add(length).context("目录名称溢出")?;
        let name = record.get(104..end).context("目录名称截断")?;
        let name: Vec<u16> = name
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        if name != [46] && name != [46, 46] {
            let size = read_field::<i64>(record, 40)?;
            ensure!(size >= 0, "文件大小无效");
            rows.push(DirectoryEntry {
                id: read_field(record, 96)?,
                name,
                attributes: read_field(record, 56)?,
                size,
                created: read_field(record, 8)?,
                modified: read_field(record, 24)?,
            });
        }
        if next == 0 {
            break;
        }
        ensure!(
            next >= end && next.is_multiple_of(8) && next < record.len(),
            "目录下一条偏移无效"
        );
        offset = offset.checked_add(next).context("目录偏移溢出")?;
    }
    Ok(rows)
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: This wrapper uniquely owns the file handle and closes it once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
impl Handle {
    fn open(path: &Path, access: u32) -> Result<Self> {
        // SAFETY: The owned wide string and flags are valid for the synchronous call.
        Ok(Self(unsafe {
            CreateFileW(
                &HSTRING::from(path.as_os_str()),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                None,
            )?
        }))
    }
    fn info(&self) -> Result<BY_HANDLE_FILE_INFORMATION> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: An owned live handle and fully sized writable structure are supplied.
        unsafe {
            GetFileInformationByHandle(self.0, &mut info)?;
        }
        Ok(info)
    }
    fn ioctl(&self, code: u32, input: &[u8]) -> windows::core::Result<Vec<u8>> {
        let mut bytes = vec![0u8; 256 * 1024];
        let mut length = 0;
        // SAFETY: Buffers remain alive for synchronous IO; lengths match allocations.
        unsafe {
            DeviceIoControl(
                self.0,
                code,
                (!input.is_empty()).then_some(input.as_ptr().cast()),
                input.len() as u32,
                Some(bytes.as_mut_ptr().cast()),
                bytes.len() as u32,
                Some(&mut length),
                None,
            )?;
        }
        bytes.truncate(length as usize);
        Ok(bytes)
    }
}

fn directory(
    handle: &Handle,
    cancelled: &AtomicBool,
    mut consume: impl FnMut(&[DirectoryEntry]) -> Result<()>,
) -> Result<()> {
    let info = handle.info()?;
    ensure!(
        info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0
            && info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0,
        "不会遍历重解析点或非目录"
    );
    let mut buffer = vec![0u64; 8192];
    loop {
        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
        buffer.fill(0);
        // SAFETY: Vec<u64> provides the required eight-byte alignment; Windows uses its exact capacity.
        let result = unsafe {
            GetFileInformationByHandleEx(
                handle.0,
                FileIdBothDirectoryInfo,
                buffer.as_mut_ptr().cast(),
                (buffer.len() * 8) as u32,
            )
        };
        match result {
            Ok(()) => {
                // SAFETY: Read-only byte view of initialized storage, within its allocation.
                let bytes = unsafe {
                    std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), buffer.len() * 8)
                };
                consume(&decode_directory(bytes)?)?;
            }
            Err(e) if e.code() == ERROR_NO_MORE_FILES.to_hresult() => return Ok(()),
            Err(e) => return Err(e.into()),
        }
    }
}

pub fn read_directory(
    path: &Path,
    cancelled: &AtomicBool,
    consume: impl FnMut(&[DirectoryEntry]) -> Result<()>,
) -> Result<()> {
    directory(
        &Handle::open(path, FILE_LIST_DIRECTORY.0)?,
        cancelled,
        consume,
    )
}

pub struct VolumeReader {
    handle: Handle,
    root: PathBuf,
    pub root_id: u64,
    serial: u32,
}
impl VolumeReader {
    pub fn open(root: &str) -> Result<Self> {
        ensure!(
            root.starts_with(r"\\?\Volume{") && root.ends_with("}\\"),
            "需要已核验的卷 GUID 路径"
        );
        let handle = Handle::open(Path::new(root.trim_end_matches('\\')), GENERIC_READ.0)?;
        let info = Handle::open(Path::new(root), FILE_READ_ATTRIBUTES.0)?.info()?;
        Ok(Self {
            handle,
            root: root.into(),
            root_id: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
            serial: info.dwVolumeSerialNumber,
        })
    }
    fn by_id(&self, id: u64, access: u32) -> Result<Handle> {
        let descriptor = FILE_ID_DESCRIPTOR {
            dwSize: std::mem::size_of::<FILE_ID_DESCRIPTOR>() as u32,
            Type: FileIdType,
            Anonymous: FILE_ID_DESCRIPTOR_0 { FileId: id as i64 },
        };
        // SAFETY: Descriptor size/type matches the union; the returned handle is uniquely owned.
        Ok(Handle(unsafe {
            OpenFileById(
                self.handle.0,
                &descriptor,
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            )?
        }))
    }
    pub fn directory(
        &self,
        id: u64,
        cancelled: &AtomicBool,
        consume: impl FnMut(&[DirectoryEntry]) -> Result<()>,
    ) -> Result<()> {
        directory(&self.by_id(id, FILE_LIST_DIRECTORY.0)?, cancelled, consume)
    }
    pub fn journal(&self) -> Result<Option<Journal>> {
        match self.handle.ioctl(FSCTL_QUERY_USN_JOURNAL, &[]) {
            Ok(bytes) => {
                let first = read_field::<i64>(&bytes, 8)?;
                let next = read_field::<i64>(&bytes, 16)?;
                ensure!(first >= 0 && first <= next, "USN 日志边界无效");
                Ok(Some(Journal {
                    id: read_field(&bytes, 0)?,
                    first,
                    next,
                }))
            }
            Err(e) if e.code() == ERROR_JOURNAL_NOT_ACTIVE.to_hresult() => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    pub fn create_journal(&self) -> Result<Journal> {
        if let Some(existing) = self.journal()? {
            return Ok(existing);
        }
        let mut input = Vec::new();
        input.extend_from_slice(&(32u64 * 1024 * 1024).to_le_bytes());
        input.extend_from_slice(&(8u64 * 1024 * 1024).to_le_bytes());
        self.handle.ioctl(FSCTL_CREATE_USN_JOURNAL, &input)?;
        self.journal()?.context("创建后日志仍不可用")
    }
    pub fn enumerate_mft(
        &self,
        cancelled: &AtomicBool,
        mut consume: impl FnMut(&[UsnRecord]) -> Result<()>,
    ) -> Result<()> {
        let mut next = 0u64;
        loop {
            ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
            let mut input = Vec::new();
            input.extend_from_slice(&next.to_le_bytes());
            input.extend_from_slice(&0i64.to_le_bytes());
            input.extend_from_slice(&i64::MAX.to_le_bytes());
            match self.handle.ioctl(FSCTL_ENUM_USN_DATA, &input) {
                Ok(bytes) => {
                    let page = decode_usn(&bytes)?;
                    ensure!(page.next > next, "MFT 枚举未前进");
                    consume(&page.records)?;
                    next = page.next;
                }
                Err(e) if e.code() == ERROR_HANDLE_EOF.to_hresult() => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
    }
    pub fn changes(&self, journal: u64, start: i64) -> Result<UsnPage> {
        ensure!(start >= 0, "USN 起点无效");
        let mut input = vec![0u8; 40];
        input[..8].copy_from_slice(&start.to_le_bytes());
        input[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        input[32..40].copy_from_slice(&journal.to_le_bytes());
        decode_usn(&self.handle.ioctl(FSCTL_READ_USN_JOURNAL, &input)?)
    }
    pub fn object(&self, id: u64) -> Result<NativeObject> {
        let handle = self.by_id(id, FILE_READ_ATTRIBUTES.0)?;
        let info = handle.info()?;
        ensure!(
            (((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64) == id
                && info.dwVolumeSerialNumber == self.serial,
            "文件身份已变化"
        );
        let mut path = vec![0u16; 32768];
        // SAFETY: The output is a fully sized UTF-16 buffer associated with this live handle.
        let length =
            unsafe { GetFinalPathNameByHandleW(handle.0, &mut path, VOLUME_NAME_NONE) } as usize;
        ensure!(length > 0 && length < path.len(), "不能取得卷内路径");
        path.truncate(length);
        let mut names = vec![path.clone()];
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0 && info.nNumberOfLinks > 1 {
            let full = self.root.join(std::ffi::OsString::from_wide(
                path.strip_prefix(&[92]).unwrap_or(&path),
            ));
            let mut buffer = vec![0u16; 32768];
            let mut size = buffer.len() as u32;
            // SAFETY: The wide path and writable buffer remain live throughout the call.
            let search = unsafe {
                FindFirstFileNameW(
                    &HSTRING::from(full.as_os_str()),
                    0,
                    &mut size,
                    PWSTR(buffer.as_mut_ptr()),
                )?
            };
            struct Links(HANDLE);
            impl Drop for Links {
                fn drop(&mut self) {
                    // SAFETY: This wrapper uniquely owns a FindFirstFileNameW search handle.
                    unsafe {
                        let _ = FindClose(self.0);
                    }
                }
            }
            let search = Links(search);
            names.clear();
            loop {
                let end = buffer
                    .iter()
                    .position(|c| *c == 0)
                    .context("硬链接名称未终止")?;
                names.push(buffer[..end].to_vec());
                size = buffer.len() as u32;
                buffer.fill(0);
                // SAFETY: The enumeration handle and buffer are valid and uniquely owned.
                match unsafe { FindNextFileNameW(search.0, &mut size, PWSTR(buffer.as_mut_ptr())) }
                {
                    Ok(()) => (),
                    Err(e) if e.code() == ERROR_HANDLE_EOF.to_hresult() => break,
                    Err(e) => return Err(e.into()),
                }
            }
        }
        let mut links = Vec::new();
        for name in names {
            if id == self.root_id {
                continue;
            }
            let split = name
                .iter()
                .rposition(|c| *c == 92)
                .context("卷内路径缺少父目录")?;
            let parent = &name[..split];
            let parent = self.root.join(std::ffi::OsString::from_wide(
                parent.strip_prefix(&[92]).unwrap_or(parent),
            ));
            let parent_info = Handle::open(&parent, FILE_READ_ATTRIBUTES.0)?.info()?;
            ensure!(
                parent_info.dwVolumeSerialNumber == self.serial,
                "硬链接父目录不属于当前卷"
            );
            links.push((
                ((parent_info.nFileIndexHigh as u64) << 32) | parent_info.nFileIndexLow as u64,
                name[split + 1..].to_vec(),
            ));
        }
        let time = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
        Ok(NativeObject {
            entry: DirectoryEntry {
                id,
                name: links.first().map_or_else(|| vec![92], |(_, n)| n.clone()),
                attributes: info.dwFileAttributes,
                size: i64::try_from(
                    ((info.nFileSizeHigh as u64) << 32) | info.nFileSizeLow as u64,
                )?,
                created: time(info.ftCreationTime),
                modified: time(info.ftLastWriteTime),
            },
            links,
        })
    }
}
