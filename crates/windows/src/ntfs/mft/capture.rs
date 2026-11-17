//! Benchmark-only frozen raw input. Captures stay local and contain filesystem metadata.
use super::{FastScanStats, Geometry, ReadAt, scan_mft};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufReader, BufWriter, Read, Write},
    path::Path,
    sync::atomic::AtomicBool,
};

pub struct Capture {
    pub geometry: Geometry,
    pub root: u64,
    ranges: BTreeMap<u64, Vec<u8>>,
    bootstrap: BTreeMap<u64, (u64, Vec<u8>)>,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    geometry: [u64; 6],
    root: u64,
    ranges: Vec<(u64, u64)>,
    bootstrap: Vec<(u64, u64, u64)>,
    sha256: String,
}

impl Capture {
    pub fn record(
        source: &mut dyn ReadAt,
        geometry: Geometry,
        root: u64,
        cancelled: &AtomicBool,
    ) -> Result<(Self, FastScanStats)> {
        let mut image = Self {
            geometry,
            root,
            ranges: BTreeMap::new(),
            bootstrap: BTreeMap::new(),
        };
        let stats = scan_mft(
            &mut Recorder {
                source,
                image: &mut image,
            },
            &geometry,
            root,
            cancelled,
            &mut |_| Ok(()),
        )?;
        Ok((image, stats))
    }

    fn insert(&mut self, offset: u64, bytes: &[u8]) -> Result<()> {
        let end = offset
            .checked_add(bytes.len() as u64)
            .context("捕获范围溢出")?;
        let mut cursor = offset;
        while cursor < end {
            if let Some((&start, previous)) = self.ranges.range(..=cursor).next_back()
                && cursor < start + previous.len() as u64
            {
                let length = (end - cursor).min(start + previous.len() as u64 - cursor) as usize;
                ensure!(
                    previous[(cursor - start) as usize..(cursor - start) as usize + length]
                        == bytes[(cursor - offset) as usize..(cursor - offset) as usize + length],
                    "捕获期间元数据发生变化，请使用新目录重试"
                );
                cursor += length as u64;
            } else {
                let stop = self
                    .ranges
                    .range(cursor..)
                    .next()
                    .map_or(end, |(&next, _)| next.min(end));
                self.ranges.insert(
                    cursor,
                    bytes[(cursor - offset) as usize..(stop - offset) as usize].to_vec(),
                );
                cursor = stop;
            }
        }
        Ok(())
    }

    pub fn save(&self, directory: &Path) -> Result<()> {
        fs::create_dir(directory)?;
        let file = File::create_new(directory.join("data.bin"))?;
        let mut output = BufWriter::new(file);
        let mut digest = Sha256::new();
        let mut write = |bytes: &[u8]| -> Result<()> {
            output.write_all(bytes)?;
            digest.update(bytes);
            Ok(())
        };
        for bytes in self.ranges.values() {
            write(bytes)?;
        }
        for (_, bytes) in self.bootstrap.values() {
            write(bytes)?;
        }
        output.flush()?;
        output.get_ref().sync_all()?;
        let g = &self.geometry;
        let manifest = Manifest {
            version: 1,
            geometry: [
                g.sector_bytes as u64,
                g.cluster_bytes as u64,
                g.record_bytes as u64,
                g.total_clusters,
                g.mft_lcn,
                g.mft_len,
            ],
            root: self.root,
            ranges: self
                .ranges
                .iter()
                .map(|(&offset, data)| (offset, data.len() as u64))
                .collect(),
            bootstrap: self
                .bootstrap
                .iter()
                .map(|(&reference, (found, data))| (reference, *found, data.len() as u64))
                .collect(),
            sha256: format!("{:x}", digest.finalize()),
        };
        let mut file = File::create_new(directory.join("capture.json"))?;
        serde_json::to_writer_pretty(&mut file, &manifest)?;
        file.sync_all()?;
        Ok(())
    }

    pub fn load(directory: &Path) -> Result<Self> {
        let manifest: Manifest =
            serde_json::from_reader(File::open(directory.join("capture.json"))?)?;
        ensure!(manifest.version == 1, "不支持的原始捕获版本");
        let [sector, cluster, record, total, lcn, length] = manifest.geometry;
        let geometry = Geometry::new(
            sector.try_into()?,
            cluster.try_into()?,
            record.try_into()?,
            total,
            lcn,
            length,
        )?;
        let file = File::open(directory.join("data.bin"))?;
        let mut remaining = file.metadata()?.len();
        let mut input = BufReader::new(file);
        let mut digest = Sha256::new();
        let mut read = |length: u64| -> Result<Vec<u8>> {
            ensure!(
                length != 0 && length <= remaining,
                "原始捕获长度无效或文件截断"
            );
            let mut bytes = vec![0; usize::try_from(length)?];
            input.read_exact(&mut bytes)?;
            digest.update(&bytes);
            remaining -= length;
            Ok(bytes)
        };
        let mut image = Self {
            geometry,
            root: manifest.root,
            ranges: BTreeMap::new(),
            bootstrap: BTreeMap::new(),
        };
        let mut previous_end = 0;
        for (offset, length) in manifest.ranges {
            ensure!(offset >= previous_end, "原始捕获范围重叠或乱序");
            previous_end = offset.checked_add(length).context("原始捕获范围溢出")?;
            image.ranges.insert(offset, read(length)?);
        }
        for (reference, found, length) in manifest.bootstrap {
            ensure!(
                image
                    .bootstrap
                    .insert(reference, (found, read(length)?))
                    .is_none(),
                "原始捕获记录重复"
            );
        }
        ensure!(
            remaining == 0 && format!("{:x}", digest.finalize()) == manifest.sha256,
            "原始捕获校验失败"
        );
        Ok(image)
    }
}

impl ReadAt for Capture {
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<()> {
        let mut cursor = offset;
        let mut remaining = out;
        while !remaining.is_empty() {
            let (&start, bytes) = self
                .ranges
                .range(..=cursor)
                .next_back()
                .context("原始捕获未包含请求范围")?;
            let index = usize::try_from(cursor - start)?;
            ensure!(index < bytes.len(), "原始捕获范围存在空洞");
            let length = remaining.len().min(bytes.len() - index);
            let (head, rest) = remaining.split_at_mut(length);
            head.copy_from_slice(&bytes[index..index + length]);
            cursor += length as u64;
            remaining = rest;
        }
        Ok(())
    }
    fn bootstrap_record(&mut self, reference: u64, out: &mut [u8]) -> Result<Option<u64>> {
        let Some((found, bytes)) = self.bootstrap.get(&reference) else {
            return Ok(None);
        };
        ensure!(bytes.len() == out.len(), "原始捕获记录长度不一致");
        out.copy_from_slice(bytes);
        Ok(Some(*found))
    }
}

struct Recorder<'a> {
    source: &'a mut dyn ReadAt,
    image: &'a mut Capture,
}
impl ReadAt for Recorder<'_> {
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<()> {
        self.source.read_exact_at(offset, out)?;
        self.image.insert(offset, out)
    }
    fn bootstrap_record(&mut self, reference: u64, out: &mut [u8]) -> Result<Option<u64>> {
        let found = self.source.bootstrap_record(reference, out)?;
        if let Some(found) = found {
            if let Some(previous) = self.image.bootstrap.get(&reference) {
                ensure!(
                    previous.0 == found && previous.1 == out,
                    "捕获期间辅助记录发生变化"
                );
            } else {
                self.image
                    .bootstrap
                    .insert(reference, (found, out.to_vec()));
            }
        }
        Ok(found)
    }
}
