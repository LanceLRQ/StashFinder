use anyhow::{Context, Result, bail, ensure};
use stashfinder_core::assets::{Capacity, Identity, Layout, Observation, Serial, now};
use std::mem::offset_of;
use windows::{
    Win32::{
        Foundation::*,
        Storage::FileSystem::*,
        System::{IO::DeviceIoControl, Ioctl::*},
    },
    core::{GUID, HSTRING},
};

pub trait Scalar: Sized {
    fn decode(bytes: &[u8]) -> Result<Self>;
    fn width() -> usize;
}
macro_rules! scalar {
    ($t:ty, $n:expr) => {
        impl Scalar for $t {
            fn decode(bytes: &[u8]) -> Result<Self> {
                Ok(<$t>::from_le_bytes(bytes.try_into()?))
            }
            fn width() -> usize {
                $n
            }
        }
    };
}
scalar!(u16, 2);
scalar!(u32, 4);
scalar!(u64, 8);
scalar!(i64, 8);

pub fn read_field<T: Scalar>(bytes: &[u8], offset: usize) -> Result<T> {
    let end = offset.checked_add(T::width()).context("设备字段偏移溢出")?;
    T::decode(bytes.get(offset..end).context("设备返回数据截断")?)
}

fn guid(bytes: &[u8], offset: usize) -> Result<String> {
    let data = bytes
        .get(offset..offset.checked_add(16).context("GUID 偏移溢出")?)
        .context("设备 GUID 截断")?;
    let value = GUID::from_values(
        u32::from_le_bytes(data[0..4].try_into()?),
        u16::from_le_bytes(data[4..6].try_into()?),
        u16::from_le_bytes(data[6..8].try_into()?),
        data[8..16].try_into()?,
    );
    ensure!(value != GUID::zeroed(), "设备 GUID 为空");
    Ok(format!("{value:?}"))
}

pub fn decode_descriptor_string(bytes: &[u8], offset: u32) -> Result<Option<String>> {
    if offset == 0 {
        return Ok(None);
    }
    let tail = bytes.get(offset as usize..).context("设备字符串偏移无效")?;
    let end = tail
        .iter()
        .position(|b| *b == 0)
        .context("设备字符串未终止")?;
    let value = String::from_utf8_lossy(&tail[..end]).trim().to_owned();
    Ok((!value.is_empty()).then_some(value))
}

pub fn decode_extent(bytes: &[u8]) -> Result<(u32, i64, i64)> {
    ensure!(read_field::<u32>(bytes, 0)? == 1, "暂不支持跨盘或多范围卷");
    let start = offset_of!(VOLUME_DISK_EXTENTS, Extents);
    let disk = read_field::<u32>(bytes, start + offset_of!(DISK_EXTENT, DiskNumber))?;
    let offset = read_field::<i64>(bytes, start + offset_of!(DISK_EXTENT, StartingOffset))?;
    let length = read_field::<i64>(bytes, start + offset_of!(DISK_EXTENT, ExtentLength))?;
    ensure!(offset >= 0 && length > 0, "卷范围无效");
    Ok((disk, offset, length))
}

struct Device(HANDLE);
impl Drop for Device {
    fn drop(&mut self) {
        // SAFETY: This wrapper owns a handle returned by CreateFileW exactly once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
impl Device {
    fn open(path: &str, access: u32) -> Result<Self> {
        // SAFETY: The path is NUL-terminated by HSTRING; no borrowed handle or buffer escapes.
        Ok(Self(unsafe {
            CreateFileW(
                &HSTRING::from(path),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )?
        }))
    }
    fn query(&self, code: u32, input: Option<&[u8]>) -> Result<Vec<u8>> {
        let mut size = 4096usize;
        loop {
            let mut output = vec![0u8; size];
            let mut count = 0;
            // SAFETY: Both buffers remain live, their exact lengths are supplied, and IO is synchronous.
            let result = unsafe {
                DeviceIoControl(
                    self.0,
                    code,
                    input.map(|b| b.as_ptr().cast()),
                    input.map_or(0, |b| b.len() as u32),
                    Some(output.as_mut_ptr().cast()),
                    size as u32,
                    Some(&mut count),
                    None,
                )
            };
            match result {
                Ok(()) => {
                    ensure!(count as usize <= output.len(), "设备响应长度无效");
                    output.truncate(count as usize);
                    return Ok(output);
                }
                Err(error)
                    if (error.code() == ERROR_MORE_DATA.to_hresult()
                        || error.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult())
                        && size < 1024 * 1024 =>
                {
                    size *= 2
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}

struct VolumeSearch(HANDLE);
impl Drop for VolumeSearch {
    fn drop(&mut self) {
        // SAFETY: This is the unique owner of a FindFirstVolumeW enumeration handle.
        unsafe {
            let _ = FindVolumeClose(self.0);
        }
    }
}

pub fn discover() -> Result<Vec<Observation>> {
    let mut name = [0u16; 1024];
    // SAFETY: Windows receives a writable array with its actual length.
    let search = VolumeSearch(unsafe { FindFirstVolumeW(&mut name)? });
    let mut observations = Vec::new();
    loop {
        let end = name.iter().position(|c| *c == 0).context("卷路径未终止")?;
        let path = String::from_utf16(&name[..end])?;
        observations.push(probe(&path));
        ensure!(observations.len() <= 4096, "卷数量超过本次发现上限");
        // SAFETY: The search handle and output array are valid for the entire call.
        match unsafe { FindNextVolumeW(search.0, &mut name) } {
            Ok(()) => (),
            Err(error) if error.code() == ERROR_NO_MORE_FILES.to_hresult() => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(observations)
}

pub fn probe(path: &str) -> Observation {
    let mut item = Observation {
        candidate_id: path.into(),
        disk_number: None,
        model: None,
        disk_bytes: None,
        partition_bytes: None,
        volume_label: None,
        mounts: Vec::new(),
        identity: Identity {
            layout: Layout::Unknown,
            filesystem: None,
            serial: None,
        },
        capacity: None,
        error: None,
    };
    let result = (|| -> Result<()> {
        ensure!(
            path.starts_with(r"\\?\Volume{") && path.ends_with("}\\"),
            "不是本地卷 GUID 路径"
        );
        let root = HSTRING::from(path);
        let mut mounts = vec![0u16; 32768];
        let mut required = 0;
        // SAFETY: The root path and destination buffers remain live for all calls.
        unsafe {
            GetVolumePathNamesForVolumeNameW(&root, Some(&mut mounts), &mut required)?;
        }
        item.mounts = mounts
            .split(|c| *c == 0)
            .take_while(|s| !s.is_empty())
            .map(String::from_utf16)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let volume = Device::open(path.trim_end_matches('\\'), 0)?;
        let extents = volume.query(IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, None)?;
        let (number, extent_offset, extent_length) = decode_extent(&extents)?;
        item.disk_number = Some(number);
        let disk_path = format!(r"\\.\PhysicalDrive{number}");
        let disk = Device::open(&disk_path, GENERIC_READ.0)
            .context("无法读取磁盘布局，请通过后台服务发现设备")?;
        let length = disk.query(IOCTL_DISK_GET_LENGTH_INFO, None)?;
        item.disk_bytes = Some(read_field::<i64>(&length, 0)?.try_into()?);
        let descriptor = disk
            .query(IOCTL_STORAGE_QUERY_PROPERTY, Some(&[0u8; 12]))
            .ok();
        let mut serial = None;
        if let Some(bytes) = descriptor {
            let model_offset = read_field::<u32>(
                &bytes,
                offset_of!(STORAGE_DEVICE_DESCRIPTOR, ProductIdOffset),
            )?;
            let serial_offset = read_field::<u32>(
                &bytes,
                offset_of!(STORAGE_DEVICE_DESCRIPTOR, SerialNumberOffset),
            )?;
            item.model = decode_descriptor_string(&bytes, model_offset)?;
            serial = decode_descriptor_string(&bytes, serial_offset)?;
        }
        let layout = disk.query(IOCTL_DISK_GET_DRIVE_LAYOUT_EX, None)?;
        let part = volume.query(IOCTL_DISK_GET_PARTITION_INFO_EX, None)?;
        let style = read_field::<u32>(&part, offset_of!(PARTITION_INFORMATION_EX, PartitionStyle))?;
        ensure!(
            style
                == read_field::<u32>(
                    &layout,
                    offset_of!(DRIVE_LAYOUT_INFORMATION_EX, PartitionStyle)
                )?,
            "卷和磁盘布局不一致"
        );
        let offset =
            read_field::<i64>(&part, offset_of!(PARTITION_INFORMATION_EX, StartingOffset))?;
        let length =
            read_field::<i64>(&part, offset_of!(PARTITION_INFORMATION_EX, PartitionLength))?;
        ensure!(
            offset == extent_offset && length == extent_length,
            "卷映射在探测期间发生变化"
        );
        item.partition_bytes = Some(length.try_into()?);
        let union = offset_of!(PARTITION_INFORMATION_EX, Anonymous);
        let disk_union = offset_of!(DRIVE_LAYOUT_INFORMATION_EX, Anonymous);
        item.identity.layout = if style == PARTITION_STYLE_GPT.0 as u32 {
            let kind = guid(
                &part,
                union + offset_of!(PARTITION_INFORMATION_GPT, PartitionType),
            )?;
            ensure!(
                kind.eq_ignore_ascii_case("ebd0a0a2-b9e5-4433-87c0-68b6b72699c7"),
                "不是受支持的基本数据分区"
            );
            Layout::Gpt {
                disk: guid(
                    &layout,
                    disk_union + offset_of!(DRIVE_LAYOUT_INFORMATION_GPT, DiskId),
                )?,
                partition: guid(
                    &part,
                    union + offset_of!(PARTITION_INFORMATION_GPT, PartitionId),
                )?,
            }
        } else if style == PARTITION_STYLE_MBR.0 as u32 {
            let kind = *part
                .get(union + offset_of!(PARTITION_INFORMATION_MBR, PartitionType))
                .context("MBR 信息截断")?;
            ensure!(
                matches!(kind, 0x01 | 0x04 | 0x06 | 0x07 | 0x0b | 0x0c | 0x0e),
                "不是受支持的 MBR 数据分区"
            );
            Layout::Mbr {
                signature: read_field::<u32>(&layout, disk_union)?,
                offset: offset.try_into()?,
                length: length.try_into()?,
                device: serial,
            }
        } else {
            bail!("暂不支持此磁盘布局")
        };
        let mut fs = [0u16; 64];
        let mut label = [0u16; 261];
        let mut volume_serial = 0;
        // SAFETY: Output buffers and scalar pointers are correctly sized; this call reads metadata only.
        unsafe {
            GetVolumeInformationW(
                &root,
                Some(&mut label),
                Some(&mut volume_serial),
                None,
                None,
                Some(&mut fs),
            )?;
        }
        let label = String::from_utf16_lossy(label.split(|c| *c == 0).next().unwrap_or_default());
        if !label.trim().is_empty() {
            item.volume_label = Some(label);
        }
        let fs = String::from_utf16(
            &fs[..fs
                .iter()
                .position(|c| *c == 0)
                .context("文件系统名称未终止")?],
        )?;
        item.identity.filesystem = Some(fs.clone());
        ensure!(
            matches!(fs.as_str(), "NTFS" | "exFAT" | "FAT32"),
            "不支持的文件系统：{fs}"
        );
        item.identity.serial = Some(if fs == "NTFS" {
            // A zero-access volume handle supports device metadata, but NTFS
            // filesystem controls require opening the filesystem for reading.
            let ntfs = Device::open(path.trim_end_matches('\\'), GENERIC_READ.0)
                .context("无法打开 NTFS 卷身份查询句柄")?;
            let data = ntfs
                .query(FSCTL_GET_NTFS_VOLUME_DATA, None)
                .context("无法读取 NTFS 卷身份")?;
            Serial {
                source: "ntfs64".into(),
                value: format!("{:016x}", read_field::<u64>(&data, 0)?),
            }
        } else {
            Serial {
                source: "win32_volume32".into(),
                value: format!("{volume_serial:08x}"),
            }
        });
        let (mut total, mut free) = (0, 0);
        // SAFETY: Both values are initialized u64 outputs and the volume root is valid.
        if unsafe { GetDiskFreeSpaceExW(&root, None, Some(&mut total), Some(&mut free)) }.is_ok()
            && free <= total
        {
            item.capacity = Some(Capacity {
                total,
                free,
                sampled_at: now(),
            });
        }
        Ok(())
    })();
    if let Err(error) = result {
        item.error = Some(format!("{error:#}"));
    }
    item
}
