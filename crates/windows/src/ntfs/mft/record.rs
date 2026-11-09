use super::{FastScanUnavailable, Geometry, SEGMENT_MASK};
use crate::{
    ntfs::{DirectoryEntry, NativeObject},
    volumes::read_field,
};
use anyhow::{Context, Result, ensure};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRun {
    pub vcn: u64,
    pub lcn: Option<u64>,
    pub clusters: u64,
}

pub fn decode_runs(mut bytes: &[u8], mut vcn: u64, total_clusters: u64) -> Result<Vec<DataRun>> {
    let mut runs = Vec::new();
    let mut previous_lcn = 0i64;
    while let Some((&header, rest)) = bytes.split_first() {
        if header == 0 {
            return Ok(runs);
        }
        let length_width = usize::from(header & 15);
        let offset_width = usize::from(header >> 4);
        ensure!(
            (1..=8).contains(&length_width) && offset_width <= 8,
            "MFT 数据运行字段宽度无效"
        );
        let (length, rest) = rest
            .split_at_checked(length_width)
            .context("MFT 数据运行长度截断")?;
        let (offset, rest) = rest
            .split_at_checked(offset_width)
            .context("MFT 数据运行偏移截断")?;
        bytes = rest;
        let mut length_bytes = [0u8; 8];
        length_bytes[..length_width].copy_from_slice(length);
        let clusters = u64::from_le_bytes(length_bytes);
        ensure!(clusters != 0, "MFT 数据运行长度为零");
        let lcn = if let Some(last) = offset.last() {
            let mut delta = [if last & 0x80 == 0 { 0 } else { 0xff }; 8];
            delta[..offset_width].copy_from_slice(offset);
            previous_lcn = previous_lcn
                .checked_add(i64::from_le_bytes(delta))
                .context("MFT 数据运行偏移溢出")?;
            let lcn = u64::try_from(previous_lcn).context("MFT 数据运行偏移为负")?;
            ensure!(
                lcn.checked_add(clusters)
                    .is_some_and(|end| end <= total_clusters),
                "MFT 数据运行超出卷范围"
            );
            Some(lcn)
        } else {
            None
        };
        let next = vcn.checked_add(clusters).context("MFT 逻辑簇范围溢出")?;
        runs.push(DataRun { vcn, lcn, clusters });
        vcn = next;
    }
    anyhow::bail!("MFT 数据运行表未终止")
}

#[derive(Debug)]
pub struct Attribute {
    pub kind: u32,
    pub instance: u16,
    pub stream: Vec<u16>,
    pub value: AttributeValue,
}

#[derive(Debug)]
pub enum AttributeValue {
    StandardInformation {
        created: u64,
        modified: u64,
        attributes: u32,
    },
    FileName {
        parent: u64,
        namespace: u8,
        name: Vec<u16>,
    },
    ResidentData {
        size: u64,
    },
    AttributeList(Vec<u8>),
    Nonresident {
        first_vcn: u64,
        last_vcn: u64,
        size: Option<u64>,
        initialized: Option<u64>,
        flags: u16,
        runs: Vec<DataRun>,
    },
}

#[derive(Debug)]
pub struct Record {
    pub reference: u64,
    pub base: u64,
    pub directory: bool,
    pub attributes: Vec<Attribute>,
}

impl Record {
    pub fn into_object(self) -> Result<NativeObject> {
        ensure!(self.base == 0, "扩展记录不能作为独立对象");
        let mut metadata = None;
        let mut size = None;
        let mut links = Vec::new();
        let mut aliases = Vec::new();
        for attribute in self.attributes {
            ensure!(attribute.kind != 0x20, "属性列表尚未组装");
            match attribute.value {
                AttributeValue::StandardInformation {
                    created,
                    modified,
                    attributes,
                } => {
                    ensure!(
                        metadata.replace((created, modified, attributes)).is_none(),
                        "标准信息属性重复"
                    );
                }
                AttributeValue::FileName {
                    parent,
                    namespace: 2,
                    ..
                } => aliases.push(parent),
                AttributeValue::FileName { parent, name, .. } => links.push((parent, name)),
                AttributeValue::ResidentData { size: length } if attribute.stream.is_empty() => {
                    ensure!(size.replace(length).is_none(), "无名数据属性重复");
                }
                AttributeValue::Nonresident {
                    size: Some(length), ..
                } if attribute.kind == 0x80 && attribute.stream.is_empty() => {
                    ensure!(size.replace(length).is_none(), "无名数据属性重复");
                }
                _ => (),
            }
        }
        let (created, modified, mut attributes) =
            metadata.ok_or(FastScanUnavailable("记录缺少标准信息"))?;
        attributes &= !0x3000_0010;
        if self.directory {
            attributes |= 0x10;
        }
        if attributes & !0x80 != 0 {
            attributes &= !0x80;
        } else {
            attributes = 0x80;
        }
        let size = if self.directory {
            0
        } else {
            i64::try_from(size.ok_or(FastScanUnavailable("记录缺少无名数据长度"))?)?
        };
        if aliases
            .iter()
            .any(|parent| !links.iter().any(|(other, _)| other == parent))
        {
            return Err(FastScanUnavailable("记录包含无法匹配的短名称").into());
        }
        let mut unique = HashSet::new();
        links.retain(|link| unique.insert(link.clone()));
        let name = links
            .first()
            .ok_or(FastScanUnavailable("记录没有可索引名称"))?
            .1
            .clone();
        Ok(NativeObject {
            entry: DirectoryEntry {
                id: self.reference,
                name,
                attributes,
                size,
                created,
                modified,
            },
            links,
        })
    }
}

pub(super) fn utf16(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
        .collect()
}

pub fn decode_record(
    bytes: &mut [u8],
    ordinal: u64,
    geometry: &Geometry,
) -> Result<Option<Record>> {
    ensure!(
        bytes.len() == geometry.record_bytes && ordinal <= SEGMENT_MASK,
        "MFT 记录尺寸或编号无效"
    );
    if bytes.iter().all(|byte| *byte == 0) {
        return Ok(None);
    }
    ensure!(bytes.starts_with(b"FILE"), "MFT 记录签名无效");
    let usa = usize::from(read_field::<u16>(bytes, 4)?);
    let count = usize::from(read_field::<u16>(bytes, 6)?);
    let usa_end = usa + count * 2;
    ensure!(
        usa >= 42 && usa.is_multiple_of(2) && count == bytes.len() / 512 + 1 && usa_end <= 510,
        "MFT 修复数组边界无效"
    );
    let signature = read_field::<u16>(bytes, usa)?;
    for i in 1..count {
        let tail = i * 512 - 2;
        ensure!(
            read_field::<u16>(bytes, tail)? == signature,
            "MFT 记录扇区修复校验失败"
        );
    }
    for i in 1..count {
        let tail = i * 512 - 2;
        bytes.copy_within(usa + i * 2..usa + i * 2 + 2, tail);
    }
    let flags = read_field::<u16>(bytes, 22)?;
    if flags & 1 == 0 {
        return Ok(None);
    }
    let sequence = read_field::<u16>(bytes, 16)?;
    let first = usize::from(read_field::<u16>(bytes, 20)?);
    let used = read_field::<u32>(bytes, 24)? as usize;
    let allocated = read_field::<u32>(bytes, 28)? as usize;
    ensure!(
        allocated == bytes.len()
            && first >= usa_end
            && first.is_multiple_of(8)
            && first + 4 <= used
            && used <= allocated,
        "MFT 记录属性范围无效"
    );
    let base = read_field::<u64>(bytes, 32)?;
    let reference = (u64::from(sequence) << 48) | ordinal;
    let mut attributes = Vec::new();
    let mut remaining = &bytes[first..used];
    loop {
        let kind = read_field::<u32>(remaining, 0)?;
        if kind == u32::MAX {
            break;
        }
        let len = read_field::<u32>(remaining, 4)? as usize;
        ensure!(len >= 24 && len.is_multiple_of(8), "MFT 属性长度无效");
        let (raw, rest) = remaining
            .split_at_checked(len)
            .context("MFT 属性超出记录")?;
        remaining = rest;
        let header = match raw[8] {
            0 => 24,
            1 => 64,
            _ => anyhow::bail!("MFT 属性驻留标志无效"),
        };
        ensure!(len >= header, "MFT 属性头截断");
        let name_offset = usize::from(read_field::<u16>(raw, 10)?);
        let name_len = usize::from(raw[9]) * 2;
        let name_end = if name_len != 0 {
            ensure!(
                name_offset >= header
                    && name_offset.is_multiple_of(2)
                    && name_offset + name_len <= len,
                "MFT 属性名称越界"
            );
            name_offset + name_len
        } else {
            header
        };
        if !matches!(kind, 0x10 | 0x20 | 0x30 | 0x80) {
            continue;
        }
        let stream = if name_len == 0 {
            Vec::new()
        } else {
            utf16(&raw[name_offset..name_end])
        };
        let instance = read_field(raw, 14)?;
        let value = if raw[8] == 0 {
            let length = read_field::<u32>(raw, 16)? as usize;
            let offset = usize::from(read_field::<u16>(raw, 20)?);
            ensure!(offset >= name_end, "MFT 驻留属性与头部重叠");
            let data = raw
                .get(offset..offset.checked_add(length).context("MFT 驻留属性长度溢出")?)
                .context("MFT 驻留属性越界")?;
            match kind {
                0x10 => AttributeValue::StandardInformation {
                    created: read_field(data, 0)?,
                    modified: read_field(data, 8)?,
                    attributes: read_field(data, 32)?,
                },
                0x20 => AttributeValue::AttributeList(data.to_vec()),
                0x30 => {
                    let count = usize::from(*data.get(64).context("MFT 文件名头截断")?);
                    let namespace = data[65..].first().copied().context("MFT 文件名空间截断")?;
                    ensure!(count != 0 && namespace <= 3, "MFT 文件名空间或长度无效");
                    let name = utf16(data.get(66..66 + count * 2).context("MFT 文件名截断")?);
                    ensure!(
                        !name.iter().any(|unit| matches!(*unit, 0 | 92 | 47)),
                        "MFT 文件名包含路径分隔符或空字符"
                    );
                    AttributeValue::FileName {
                        parent: read_field(data, 0)?,
                        namespace,
                        name,
                    }
                }
                _ => AttributeValue::ResidentData {
                    size: length as u64,
                },
            }
        } else {
            ensure!(matches!(kind, 0x20 | 0x80), "MFT 元数据属性必须为驻留形式");
            let first_vcn = read_field::<u64>(raw, 16)?;
            let last_vcn = read_field::<u64>(raw, 24)?;
            let start = usize::from(read_field::<u16>(raw, 32)?);
            ensure!(start >= name_end && start < len, "MFT 数据运行表偏移无效");
            let mut runs = Vec::new();
            if kind == 0x20
                || (ordinal == 0 || base & SEGMENT_MASK == 0 && base != 0) && stream.is_empty()
            {
                runs = decode_runs(&raw[start..], first_vcn, geometry.total_clusters)?;
                ensure!(
                    last_vcn < geometry.total_clusters
                        && runs
                            .last()
                            .is_some_and(|run| run.vcn + run.clusters - 1 == last_vcn),
                    "MFT 数据运行表没有覆盖声明范围"
                );
            }
            let size = (first_vcn == 0)
                .then(|| read_field::<u64>(raw, 48))
                .transpose()?;
            let initialized = (first_vcn == 0)
                .then(|| read_field::<u64>(raw, 56))
                .transpose()?;
            ensure!(initialized <= size, "MFT 数据初始化长度无效");
            AttributeValue::Nonresident {
                first_vcn,
                last_vcn,
                size,
                initialized,
                flags: read_field(raw, 12)?,
                runs,
            }
        };
        attributes.push(Attribute {
            kind,
            instance,
            stream,
            value,
        });
    }
    Ok(Some(Record {
        reference,
        base,
        directory: flags & 2 != 0,
        attributes,
    }))
}
