use super::{
    Attribute, AttributeValue, DataRun, FastScanUnavailable, Geometry, Record, SEGMENT_MASK,
    decode_record, record::utf16,
};
use crate::{ntfs::NativeObject, volumes::read_field};
use anyhow::{Context, Result, ensure};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

const BLOCK_BYTES: usize = 64 * 1024;
const CACHE_BYTES: usize = 4 * 1024 * 1024;

pub fn read_mft_object(
    disk: &mut dyn ReadAt,
    geometry: &Geometry,
    reference: u64,
) -> Result<NativeObject> {
    let mut scanner = Scanner {
        disk,
        geometry,
        cancelled: &AtomicBool::new(false),
        stats: FastScanStats::default(),
    };
    let mut record = scanner.live_record(reference)?;
    ensure!(record.base == 0, "目标不是 MFT 基记录");
    let list = scanner.list(&mut record)?;
    let mut extensions = HashMap::new();
    for entry in list {
        if entry.reference == reference {
            ensure!(
                record.attributes.iter().any(|attr| entry.matches(attr)),
                "基记录属性列表不匹配"
            );
            continue;
        }
        if let std::collections::hash_map::Entry::Vacant(slot) = extensions.entry(entry.reference) {
            let extension = scanner.live_record(entry.reference)?;
            ensure!(extension.base == reference, "属性扩展记录身份不匹配");
            ensure!(
                !extension.attributes.iter().any(|attr| attr.kind == 0x20),
                "属性扩展记录包含嵌套列表"
            );
            slot.insert(extension);
        }
        let extension = extensions
            .get_mut(&entry.reference)
            .expect("extension is loaded");
        let index = extension
            .attributes
            .iter()
            .position(|attr| entry.matches(attr))
            .context("扩展属性列表不匹配")?;
        record
            .attributes
            .push(extension.attributes.swap_remove(index));
    }
    record.into_object()
}

pub trait ReadAt {
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<()>;

    // An optional source for bootstrap records beyond the currently known runs.
    fn bootstrap_record(&mut self, _reference: u64, _out: &mut [u8]) -> Result<Option<u64>> {
        Ok(None)
    }
}

#[derive(Debug, Default)]
pub struct FastScanStats {
    pub read_calls: u64,
    pub read_bytes: u64,
    pub read_time: Duration,
    pub parse_time: Duration,
    pub consumer_time: Duration,
    pub base_records: u64,
    pub extension_records: u64,
    pub auxiliary_reads: u64,
    pub objects: u64,
    pub links: u64,
    pub peak_cache_bytes: usize,
}

struct ListEntry {
    kind: u32,
    stream: Vec<u16>,
    vcn: u64,
    reference: u64,
    instance: u16,
}

impl ListEntry {
    fn matches(&self, attr: &Attribute) -> bool {
        self.kind == attr.kind
            && self.stream == attr.stream
            && self.instance == attr.instance
            && self.vcn
                == match attr.value {
                    AttributeValue::Nonresident { first_vcn, .. } => first_vcn,
                    _ => 0,
                }
    }
}

struct Scanner<'a> {
    disk: &'a mut dyn ReadAt,
    geometry: &'a Geometry,
    cancelled: &'a AtomicBool,
    stats: FastScanStats,
}

impl Scanner<'_> {
    fn live_record(&mut self, reference: u64) -> Result<Record> {
        let mut bytes = vec![0; self.geometry.record_bytes];
        let found = self
            .disk
            .bootstrap_record(reference, &mut bytes)?
            .context("MFT 定点读取不可用")?;
        ensure!(
            found & SEGMENT_MASK == reference & SEGMENT_MASK,
            "MFT 定点读取返回不同记录"
        );
        let record = decode_record(&mut bytes, reference & SEGMENT_MASK, self.geometry)?
            .context("MFT 记录未使用")?;
        ensure!(record.reference == reference, "MFT 记录序列号已变化");
        Ok(record)
    }

    fn check_cancelled(&self) -> Result<()> {
        ensure!(!self.cancelled.load(Ordering::Relaxed), "索引已取消");
        Ok(())
    }

    fn record_offset(&self, reference: u64) -> Result<u64> {
        let ordinal = reference & SEGMENT_MASK;
        let size = self.geometry.record_bytes as u64;
        ensure!(
            ordinal < self.geometry.mft_len / size,
            "扩展记录超出扫描范围"
        );
        Ok(ordinal * size)
    }

    fn read(&mut self, physical: u64, out: &mut [u8]) -> Result<()> {
        self.check_cancelled()?;
        ensure!(
            physical.checked_add(out.len() as u64).is_some_and(
                |end| end <= self.geometry.total_clusters * self.geometry.cluster_bytes as u64
            ),
            "MFT 读取超出卷范围"
        );
        let start = Instant::now();
        self.disk.read_exact_at(physical, out)?;
        self.stats.read_time += start.elapsed();
        self.stats.read_calls += 1;
        self.stats.read_bytes += out.len() as u64;
        Ok(())
    }

    fn mapped(&mut self, runs: &[DataRun], mut logical: u64, mut out: &mut [u8]) -> Result<()> {
        let cluster = self.geometry.cluster_bytes as u64;
        while !out.is_empty() {
            let vcn = logical / cluster;
            let index = runs.partition_point(|run| run.vcn <= vcn);
            let run = index
                .checked_sub(1)
                .and_then(|i| runs.get(i))
                .context("MFT 逻辑簇没有映射")?;
            let offset = logical - run.vcn * cluster;
            let bytes = run.clusters * cluster;
            ensure!(offset < bytes, "MFT 逻辑簇映射不连续");
            let length = usize::try_from((bytes - offset).min(out.len() as u64))?;
            let physical = run.lcn.context("MFT 元数据流不应包含稀疏段")? * cluster + offset;
            let (block, rest) = out.split_at_mut(length);
            self.read(physical, block)?;
            out = rest;
            logical += length as u64;
        }
        Ok(())
    }

    fn list(&mut self, record: &mut Record) -> Result<Vec<ListEntry>> {
        let Some(index) = record.attributes.iter().position(|attr| attr.kind == 0x20) else {
            return Ok(Vec::new());
        };
        let attr = record.attributes.remove(index);
        ensure!(
            !record.attributes.iter().any(|attr| attr.kind == 0x20),
            "属性列表重复"
        );
        let data = match attr.value {
            AttributeValue::AttributeList(data) => data,
            AttributeValue::Nonresident {
                first_vcn: 0,
                flags: 0,
                size: Some(size),
                initialized: Some(initialized),
                runs,
                ..
            } => {
                if size > CACHE_BYTES as u64 {
                    return Err(FastScanUnavailable("属性列表超出顺序读取器的内存预算").into());
                }
                ensure!(size == initialized && size != 0, "属性列表初始化长度无效");
                let mut data = vec![0; usize::try_from(size)?];
                self.mapped(&runs, 0, &mut data)?;
                data
            }
            _ => return Err(FastScanUnavailable("不支持此属性列表布局").into()),
        };
        let mut bytes = data.as_slice();
        let mut entries = Vec::new();
        let mut seen = HashSet::new();
        while !bytes.is_empty() {
            let len = usize::from(read_field::<u16>(bytes, 4)?);
            ensure!(len >= 26 && len.is_multiple_of(8), "属性列表项长度无效");
            let (entry, rest) = bytes.split_at_checked(len).context("属性列表项截断")?;
            bytes = rest;
            let kind = read_field(entry, 0)?;
            if !matches!(kind, 0x10 | 0x30 | 0x80) {
                continue;
            }
            let stream = if entry[6] == 0 {
                Vec::new()
            } else {
                let offset = usize::from(entry[7]);
                ensure!(
                    offset >= 26 && offset.is_multiple_of(2),
                    "属性列表名称偏移无效"
                );
                utf16(
                    entry
                        .get(offset..offset + usize::from(entry[6]) * 2)
                        .context("属性列表名称截断")?,
                )
            };
            let reference = read_field(entry, 16)?;
            let instance = read_field(entry, 24)?;
            let vcn = read_field(entry, 8)?;
            ensure!(
                seen.insert((reference, instance, kind, vcn, stream.clone())),
                "属性列表项重复"
            );
            entries.push(ListEntry {
                kind,
                stream,
                vcn,
                reference,
                instance,
            });
        }
        Ok(entries)
    }

    fn extension(&mut self, bytes: &mut [u8], reference: u64, base: u64) -> Result<Record> {
        let record = decode_record(bytes, reference & SEGMENT_MASK, self.geometry)?
            .context("属性列表引用未使用记录")?;
        ensure!(
            record.reference == reference && record.base == base && reference != base,
            "属性扩展记录身份不匹配"
        );
        ensure!(
            !record.attributes.iter().any(|attr| attr.kind == 0x20),
            "属性扩展记录包含嵌套列表"
        );
        Ok(record)
    }

    fn mapping(&mut self) -> Result<Vec<DataRun>> {
        let record_bytes = self.geometry.record_bytes;
        let cluster = self.geometry.cluster_bytes as u64;
        let mut bytes = vec![0; record_bytes];
        self.read(self.geometry.mft_lcn * cluster, &mut bytes)?;
        let mut base = decode_record(&mut bytes, 0, self.geometry)?.context("MFT 基记录不存在")?;
        ensure!(base.base == 0 && !base.directory, "MFT 基记录无效");
        let mut list = self.list(&mut base)?;
        list.retain(|entry| entry.kind == 0x80 && entry.stream.is_empty());
        list.sort_by_key(|entry| entry.vcn);
        let first = base
            .attributes
            .iter()
            .find(|attr| {
                attr.kind == 0x80
                    && attr.stream.is_empty()
                    && matches!(attr.value, AttributeValue::Nonresident { first_vcn: 0, .. })
            })
            .context("MFT 缺少初始数据运行")?;
        let AttributeValue::Nonresident {
            size: Some(size),
            initialized: Some(initialized),
            ..
        } = first.value
        else {
            anyhow::bail!("MFT 缺少长度");
        };
        ensure!(
            initialized >= self.geometry.mft_len && size >= initialized,
            "MFT 有效长度超出数据流"
        );
        let mut runs = Vec::new();
        append_runs(&mut runs, first)?;
        ensure!(
            runs.first()
                .is_some_and(|run| run.lcn == Some(self.geometry.mft_lcn)
                    && run.clusters * cluster >= record_bytes as u64),
            "MFT 起始映射与卷几何不一致"
        );
        for entry in list {
            if entry.reference == base.reference && entry.matches(first) {
                continue;
            }
            if entry.reference == base.reference {
                let attr = base
                    .attributes
                    .iter()
                    .find(|attr| entry.matches(attr))
                    .context("MFT 数据属性列表不匹配")?;
                append_runs(&mut runs, attr)?;
                continue;
            }
            let logical = self.record_offset(entry.reference)?;
            let covered = runs.last().map_or(0, |run| run.vcn + run.clusters) * cluster;
            self.stats.auxiliary_reads += 1;
            if logical + record_bytes as u64 <= covered {
                self.mapped(&runs, logical, &mut bytes)?;
            } else {
                self.check_cancelled()?;
                let start = Instant::now();
                let found = self
                    .disk
                    .bootstrap_record(entry.reference, &mut bytes)?
                    .ok_or(FastScanUnavailable("无法从当前运行表定位 MFT 扩展记录"))?;
                self.stats.read_time += start.elapsed();
                ensure!(
                    found & SEGMENT_MASK == entry.reference & SEGMENT_MASK,
                    "MFT 辅助读取返回不同记录"
                );
            }
            let extension = self.extension(&mut bytes, entry.reference, base.reference)?;
            let attr = extension
                .attributes
                .iter()
                .find(|attr| entry.matches(attr))
                .context("MFT 扩展数据属性不匹配")?;
            append_runs(&mut runs, attr)?;
        }
        let covered = runs.last().map_or(0, |run| run.vcn + run.clusters);
        let needed = self.geometry.mft_len.div_ceil(cluster);
        ensure!(covered >= needed, "MFT 数据运行未覆盖有效长度");
        runs.retain(|run| run.vcn < needed);
        if let Some(last) = runs.last_mut() {
            last.clusters = needed - last.vcn;
        }
        Ok(runs)
    }
}

fn append_runs(out: &mut Vec<DataRun>, attr: &Attribute) -> Result<()> {
    let AttributeValue::Nonresident { flags: 0, runs, .. } = &attr.value else {
        return Err(FastScanUnavailable("不支持此 MFT 数据流布局").into());
    };
    let next = out.last().map_or(0, |run| run.vcn + run.clusters);
    ensure!(
        runs.first().is_some_and(|run| run.vcn == next) && runs.iter().all(|run| run.lcn.is_some()),
        "MFT 数据运行缺失、重叠或稀疏"
    );
    out.extend_from_slice(runs);
    Ok(())
}

pub fn scan_mft(
    disk: &mut dyn ReadAt,
    geometry: &Geometry,
    root: u64,
    cancelled: &AtomicBool,
    consume: &mut dyn FnMut(&[NativeObject]) -> Result<()>,
) -> Result<FastScanStats> {
    let mut scanner = Scanner {
        disk,
        geometry,
        cancelled,
        stats: FastScanStats::default(),
    };
    let started = Instant::now();
    let runs = scanner.mapping()?;
    let mut block = vec![0; BLOCK_BYTES.max(geometry.record_bytes)];
    let mut offset = 0u64;
    let mut extension_bytes: HashMap<u64, Vec<u8>> = HashMap::new();
    let mut extension_order = VecDeque::new();
    let mut internal_directories = HashSet::new();
    let mut root_seen = false;
    while offset < geometry.mft_len {
        let length = (geometry.mft_len - offset).min(block.len() as u64) as usize;
        scanner.mapped(&runs, offset, &mut block[..length])?;
        let mut objects = Vec::new();
        for (slot, bytes) in block[..length]
            .chunks_exact_mut(geometry.record_bytes)
            .enumerate()
        {
            scanner.check_cancelled()?;
            let ordinal = offset / geometry.record_bytes as u64 + slot as u64;
            if ordinal < 16 && ordinal != root & SEGMENT_MASK {
                if bytes.starts_with(b"FILE") && read_field::<u16>(bytes, 22)? & 3 == 3 {
                    let directory = decode_record(bytes, ordinal, geometry)?
                        .context("NTFS 内部目录记录无效")?;
                    ensure!(directory.base == 0, "NTFS 内部目录不是基记录");
                    internal_directories.insert(directory.reference);
                }
                continue;
            }
            let Some(mut record) = decode_record(bytes, ordinal, geometry)
                .with_context(|| format!("读取 MFT 记录 {ordinal}"))?
            else {
                continue;
            };
            if record.base != 0 {
                scanner.stats.extension_records += 1;
                continue;
            }
            scanner.stats.base_records += 1;
            let list = scanner.list(&mut record)?;
            let mut extensions = HashMap::new();
            for entry in list {
                if entry.reference == record.reference {
                    ensure!(
                        record.attributes.iter().any(|attr| entry.matches(attr)),
                        "基记录属性列表不匹配"
                    );
                    continue;
                }
                if let std::collections::hash_map::Entry::Vacant(slot) =
                    extensions.entry(entry.reference)
                {
                    let logical = scanner.record_offset(entry.reference)?;
                    let mut bytes = if let Some(cached) = extension_bytes.get(&entry.reference) {
                        cached.clone()
                    } else {
                        let mut bytes = vec![0; geometry.record_bytes];
                        scanner.mapped(&runs, logical, &mut bytes)?;
                        scanner.stats.auxiliary_reads += 1;
                        if extension_bytes.len() == CACHE_BYTES / geometry.record_bytes {
                            extension_bytes.remove(
                                &extension_order.pop_front().expect("nonempty record cache"),
                            );
                        }
                        extension_bytes.insert(entry.reference, bytes.clone());
                        extension_order.push_back(entry.reference);
                        scanner.stats.peak_cache_bytes = scanner
                            .stats
                            .peak_cache_bytes
                            .max(extension_bytes.len() * geometry.record_bytes);
                        bytes
                    };
                    slot.insert(scanner.extension(
                        &mut bytes,
                        entry.reference,
                        record.reference,
                    )?);
                }
                let extension = extensions
                    .get_mut(&entry.reference)
                    .expect("extension is loaded");
                let index = extension
                    .attributes
                    .iter()
                    .position(|attr| entry.matches(attr))
                    .context("扩展属性列表不匹配")?;
                record
                    .attributes
                    .push(extension.attributes.swap_remove(index));
            }
            // Internal NTFS subtrees are outside the directory reader's namespace.
            // Match the full parent reference so a stale sequence cannot hide a path.
            let before = record.attributes.len();
            record.attributes.retain(|attr| {
                !matches!(attr.value,
                AttributeValue::FileName { parent, .. } if internal_directories.contains(&parent))
            });
            if record.attributes.len() != before
                && !record.attributes.iter().any(|attr| attr.kind == 0x30)
            {
                if record.directory {
                    internal_directories.insert(record.reference);
                }
                continue;
            }
            let object = record
                .into_object()
                .with_context(|| format!("组装 MFT 记录 {ordinal}"))?;
            if ordinal == root & SEGMENT_MASK {
                ensure!(
                    object.entry.id == root && object.entry.attributes & 16 != 0,
                    "MFT 根目录身份不一致"
                );
                root_seen = true;
            }
            scanner.stats.objects += 1;
            scanner.stats.links += object.links.len() as u64;
            objects.push(object);
        }
        if !objects.is_empty() {
            scanner.check_cancelled()?;
            let start = Instant::now();
            consume(&objects)?;
            scanner.stats.consumer_time += start.elapsed();
        }
        offset += length as u64;
    }
    ensure!(root_seen, "MFT 扫描没有覆盖根目录");
    ensure!(scanner.mapping()? == runs, "MFT 数据运行在读取期间变化");
    scanner.stats.parse_time =
        started.elapsed() - scanner.stats.read_time - scanner.stats.consumer_time;
    Ok(scanner.stats)
}
