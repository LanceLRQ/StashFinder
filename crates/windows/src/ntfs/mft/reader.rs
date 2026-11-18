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
const READ_AHEAD_BYTES: usize = 1024 * 1024;
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

#[derive(Debug, Default, serde::Serialize)]
pub struct ReadStats {
    pub calls: u64,
    pub bytes: u64,
    pub elapsed: Duration,
}

#[derive(Clone, Copy)]
enum ReadKind {
    Sequential,
    Extension,
    AttributeList,
    Mapping,
    Bootstrap,
}

#[derive(Debug, Default, serde::Serialize)]
pub struct FastScanStats {
    pub read_calls: u64,
    pub read_bytes: u64,
    pub read_time: Duration,
    pub parse_time: Duration,
    pub consumer_time: Duration,
    pub queue_wait_time: Duration,
    pub base_records: u64,
    pub extension_records: u64,
    pub auxiliary_reads: u64,
    pub objects: u64,
    pub links: u64,
    pub peak_cache_bytes: usize,
    pub sequential_io: ReadStats,
    pub extension_io: ReadStats,
    pub attribute_list_io: ReadStats,
    pub mapping_io: ReadStats,
    pub bootstrap_io: ReadStats,
    pub extension_cache_hits: u64,
    pub extension_cache_misses: u64,
    pub reread_mft_bytes: u64,
    pub retained_extensions: usize,
    pub retained_bases: usize,
}

impl FastScanStats {
    fn record_read(&mut self, kind: ReadKind, bytes: usize, elapsed: Duration) {
        let class = match kind {
            ReadKind::Sequential => &mut self.sequential_io,
            ReadKind::Extension => &mut self.extension_io,
            ReadKind::AttributeList => &mut self.attribute_list_io,
            ReadKind::Mapping => &mut self.mapping_io,
            ReadKind::Bootstrap => &mut self.bootstrap_io,
        };
        class.calls += 1;
        class.bytes += bytes as u64;
        class.elapsed += elapsed;
        self.read_calls += 1;
        self.read_bytes += bytes as u64;
        self.read_time += elapsed;
    }
}

/// Buffer up to 64 MFT blocks, covering a typical 4096-record database write batch.
pub fn pipeline_inventory(
    produce: impl FnOnce(&mut dyn FnMut(&[NativeObject]) -> Result<()>) -> Result<Option<FastScanStats>>
    + Send,
    consume: &mut dyn FnMut(&[NativeObject]) -> Result<()>,
) -> Result<Option<FastScanStats>> {
    std::thread::scope(|scope| {
        let (sender, receiver) = std::sync::mpsc::sync_channel(64);
        let reader = scope.spawn(move || {
            let mut waiting = Duration::ZERO;
            let stats = produce(&mut |batch| {
                let batch = batch.to_vec();
                let start = Instant::now();
                let result = sender
                    .send(batch)
                    .map_err(|_| anyhow::anyhow!("索引消费已停止"));
                waiting += start.elapsed();
                result
            })?;
            Ok::<_, anyhow::Error>(stats.map(|mut stats| {
                stats.queue_wait_time = waiting;
                stats
            }))
        });
        let mut consumed = Duration::ZERO;
        let result = (|| {
            for batch in &receiver {
                let start = Instant::now();
                consume(&batch)?;
                consumed += start.elapsed();
            }
            Ok::<_, anyhow::Error>(())
        })();
        // Wake a producer blocked on the bounded queue before joining it.
        drop(receiver);
        let scanned = reader
            .join()
            .map_err(|_| anyhow::anyhow!("索引读取线程异常结束"));
        result?;
        Ok(scanned??.map(|mut stats| {
            stats.consumer_time = consumed;
            stats
        }))
    })
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

#[derive(Default)]
struct BlockCache {
    blocks: HashMap<u64, Vec<u8>>,
    order: VecDeque<u64>,
    loaded: HashSet<u64>,
}

impl BlockCache {
    fn read<'a>(
        &'a mut self,
        scanner: &mut Scanner<'_>,
        runs: &[DataRun],
        offset: u64,
        auxiliary: bool,
    ) -> Result<&'a [u8]> {
        let block_size = BLOCK_BYTES.max(scanner.geometry.record_bytes);
        let start = offset / block_size as u64 * block_size as u64;
        if auxiliary {
            if self.blocks.contains_key(&start) {
                scanner.stats.extension_cache_hits += 1;
            } else {
                scanner.stats.extension_cache_misses += 1;
            }
        }
        if !self.blocks.contains_key(&start) {
            // Coalesce sequential I/O, but keep random extension reads and cache
            // eviction at record-block granularity. Stop before a prefetched block.
            let read_size = if auxiliary {
                block_size
            } else {
                READ_AHEAD_BYTES.max(block_size)
            };
            let mut length = (scanner.geometry.mft_len - start).min(read_size as u64) as usize;
            for next in (block_size..length).step_by(block_size) {
                if self.blocks.contains_key(&(start + next as u64)) {
                    length = next;
                    break;
                }
            }
            let mut bytes = vec![0; length];
            scanner.mapped(
                runs,
                start,
                &mut bytes,
                if auxiliary {
                    ReadKind::Extension
                } else {
                    ReadKind::Sequential
                },
            )?;
            scanner.stats.auxiliary_reads += u64::from(auxiliary);
            for (index, chunk) in bytes.chunks(block_size).enumerate() {
                let mut cached = if self.blocks.len() == CACHE_BYTES / block_size {
                    self.blocks
                        .remove(&self.order.pop_front().expect("nonempty block cache"))
                        .expect("cached block")
                } else {
                    Vec::new()
                };
                cached.clear();
                cached.extend_from_slice(chunk);
                let key = start + (index * block_size) as u64;
                if !self.loaded.insert(key) {
                    scanner.stats.reread_mft_bytes += chunk.len() as u64;
                }
                self.blocks.insert(key, cached);
                self.order.push_back(key);
            }
            scanner.stats.peak_cache_bytes = scanner
                .stats
                .peak_cache_bytes
                .max(self.blocks.len() * block_size);
        }
        Ok(&self.blocks[&start][(offset - start) as usize..])
    }
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

    fn read(&mut self, physical: u64, out: &mut [u8], kind: ReadKind) -> Result<()> {
        self.check_cancelled()?;
        ensure!(
            physical.checked_add(out.len() as u64).is_some_and(
                |end| end <= self.geometry.total_clusters * self.geometry.cluster_bytes as u64
            ),
            "MFT 读取超出卷范围"
        );
        let start = Instant::now();
        self.disk.read_exact_at(physical, out)?;
        self.stats.record_read(kind, out.len(), start.elapsed());
        Ok(())
    }

    fn mapped(
        &mut self,
        runs: &[DataRun],
        mut logical: u64,
        mut out: &mut [u8],
        kind: ReadKind,
    ) -> Result<()> {
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
            self.read(physical, block, kind)?;
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
                self.mapped(&runs, 0, &mut data, ReadKind::AttributeList)?;
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
        validate_extension(&record, reference, base)?;
        Ok(record)
    }

    fn mapping(&mut self) -> Result<Vec<DataRun>> {
        let record_bytes = self.geometry.record_bytes;
        let cluster = self.geometry.cluster_bytes as u64;
        let mut bytes = vec![0; record_bytes];
        self.read(
            self.geometry.mft_lcn * cluster,
            &mut bytes,
            ReadKind::Mapping,
        )?;
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
                self.mapped(&runs, logical, &mut bytes, ReadKind::Mapping)?;
            } else {
                self.check_cancelled()?;
                let start = Instant::now();
                let found = self
                    .disk
                    .bootstrap_record(entry.reference, &mut bytes)?
                    .ok_or(FastScanUnavailable("无法从当前运行表定位 MFT 扩展记录"))?;
                self.stats
                    .record_read(ReadKind::Bootstrap, bytes.len(), start.elapsed());
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
    scan(disk, geometry, root, cancelled, consume, false)
}

pub fn scan_mft_deferred(
    disk: &mut dyn ReadAt,
    geometry: &Geometry,
    root: u64,
    cancelled: &AtomicBool,
    consume: &mut dyn FnMut(&[NativeObject]) -> Result<()>,
) -> Result<FastScanStats> {
    scan(disk, geometry, root, cancelled, consume, true)
}

fn scan(
    disk: &mut dyn ReadAt,
    geometry: &Geometry,
    root: u64,
    cancelled: &AtomicBool,
    consume: &mut dyn FnMut(&[NativeObject]) -> Result<()>,
    deferred: bool,
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
    let mut blocks = BlockCache::default();
    let mut internal_directories = HashSet::new();
    let mut root_seen = false;
    let mut pending = Vec::new();
    let mut collected = HashMap::new();
    while offset < geometry.mft_len {
        let length = (geometry.mft_len - offset).min(block.len() as u64) as usize;
        block[..length].copy_from_slice(blocks.read(&mut scanner, &runs, offset, false)?);
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
                if deferred {
                    collected.insert(record.reference, record);
                }
                continue;
            }
            scanner.stats.base_records += 1;
            let list = scanner.list(&mut record)?;
            if deferred {
                pending.push((record, list));
                continue;
            }
            attach_attributes(&mut record, list, |reference, base| {
                let logical = scanner.record_offset(reference)?;
                let mut bytes = blocks.read(&mut scanner, &runs, logical, true)?
                    [..geometry.record_bytes]
                    .to_vec();
                scanner.extension(&mut bytes, reference, base)
            })?;
            if let Some(object) =
                scanner.emit(record, &mut internal_directories, root, &mut root_seen)?
            {
                objects.push(object);
            }
        }
        if !objects.is_empty() {
            scanner.check_cancelled()?;
            let start = Instant::now();
            consume(&objects)?;
            scanner.stats.consumer_time += start.elapsed();
        }
        offset += length as u64;
    }
    if deferred {
        scanner.stats.retained_bases = pending.len();
        scanner.stats.retained_extensions = collected.len();
        let mut missing = Vec::new();
        for (record, list) in &pending {
            for entry in list {
                if entry.reference != record.reference && !collected.contains_key(&entry.reference)
                {
                    missing.push((entry.reference, record.reference));
                }
            }
        }
        missing.sort_unstable_by_key(|(reference, base)| {
            (reference & SEGMENT_MASK, *reference, *base)
        });
        missing.dedup();
        for (reference, base) in missing {
            scanner.check_cancelled()?;
            let logical = scanner.record_offset(reference)?;
            let mut bytes =
                blocks.read(&mut scanner, &runs, logical, true)?[..geometry.record_bytes].to_vec();
            collected.insert(reference, scanner.extension(&mut bytes, reference, base)?);
        }
        let mut objects = Vec::with_capacity(64);
        for (mut record, list) in pending {
            scanner.check_cancelled()?;
            attach_attributes(&mut record, list, |reference, base| {
                let extension = collected
                    .remove(&reference)
                    .context("属性列表引用缺少扩展记录")?;
                validate_extension(&extension, reference, base)?;
                Ok(extension)
            })?;
            if let Some(object) =
                scanner.emit(record, &mut internal_directories, root, &mut root_seen)?
            {
                objects.push(object);
            }
            if objects.len() == 64 {
                let start = Instant::now();
                consume(&objects)?;
                scanner.stats.consumer_time += start.elapsed();
                objects.clear();
            }
        }
        if !objects.is_empty() {
            let start = Instant::now();
            consume(&objects)?;
            scanner.stats.consumer_time += start.elapsed();
        }
    }
    ensure!(root_seen, "MFT 扫描没有覆盖根目录");
    ensure!(scanner.mapping()? == runs, "MFT 数据运行在读取期间变化");
    scanner.stats.parse_time =
        started.elapsed() - scanner.stats.read_time - scanner.stats.consumer_time;
    Ok(scanner.stats)
}

impl Scanner<'_> {
    fn emit(
        &mut self,
        mut record: Record,
        internal_directories: &mut HashSet<u64>,
        root: u64,
        root_seen: &mut bool,
    ) -> Result<Option<NativeObject>> {
        let record_id = record.reference;
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
            return Ok(None);
        }
        let object = record
            .into_object()
            .with_context(|| format!("组装 MFT 记录 {record_id}"))?;
        if (record_id & SEGMENT_MASK) == root & SEGMENT_MASK {
            ensure!(
                object.entry.id == root && object.entry.attributes & 16 != 0,
                "MFT 根目录身份不一致"
            );
            *root_seen = true;
        }
        self.stats.objects += 1;
        self.stats.links += object.links.len() as u64;
        Ok(Some(object))
    }
}

fn validate_extension(record: &Record, reference: u64, base: u64) -> Result<()> {
    ensure!(
        record.reference == reference && record.base == base && reference != base,
        "属性扩展记录身份不匹配"
    );
    ensure!(
        !record.attributes.iter().any(|attr| attr.kind == 0x20),
        "属性扩展记录包含嵌套列表"
    );
    Ok(())
}

fn attach_attributes(
    record: &mut Record,
    list: Vec<ListEntry>,
    mut read: impl FnMut(u64, u64) -> Result<Record>,
) -> Result<()> {
    let mut extensions = HashMap::new();
    for entry in list {
        if entry.reference == record.reference {
            ensure!(
                record.attributes.iter().any(|attr| entry.matches(attr)),
                "基记录属性列表不匹配"
            );
            continue;
        }
        if let std::collections::hash_map::Entry::Vacant(slot) = extensions.entry(entry.reference) {
            slot.insert(read(entry.reference, record.reference)?);
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
    Ok(())
}
