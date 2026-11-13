use super::{CachedEntry, SearchIndex};
use crate::volume::Volume;
use anyhow::{Context, Result, ensure};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use stashfinder_core::assets::Registration;
use std::{
    collections::HashSet,
    fs::File,
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

const MAGIC: &[u8; 8] = b"SFSC\x02\0\0\0";
const MAX_BYTES: u64 = 512 * 1024 * 1024;

#[derive(PartialEq, Serialize, Deserialize)]
struct Version {
    generation: i64,
    revision: i64,
    completed: Option<u64>,
    needs_validation: bool,
}

fn filesystem_id(volume: &Volume) -> Result<String> {
    let manifest = volume.manifest()?;
    Ok(serde_json::from_str::<Registration>(&manifest).map_or(manifest, |row| row.filesystem_id))
}

#[derive(Serialize)]
struct ContentsRef<'a> {
    directories: &'a [String],
    folded_directories: &'a [String],
    directory_entries: Vec<&'a serde_bytes::Bytes>,
    links: Vec<&'a serde_bytes::Bytes>,
    entries: &'a [CachedEntry],
}

#[derive(Deserialize)]
struct Contents {
    directories: Vec<String>,
    folded_directories: Vec<String>,
    directory_entries: Vec<serde_bytes::ByteBuf>,
    links: Vec<serde_bytes::ByteBuf>,
    entries: Vec<CachedEntry>,
}

pub(crate) fn path(database: &Path) -> PathBuf {
    database.with_extension("search-cache")
}

fn current_version(volume: &Volume) -> Result<Option<Version>> {
    let row: Option<(i64, i64, Option<i64>, String)> = volume.db.query_row(
        "SELECT g.id,g.revision,g.completed,g.phase FROM index_state s JOIN generations g ON g.id=s.active_generation WHERE s.id=1",
        [],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional()?;
    row.map(|(generation, revision, completed, phase)| {
        Ok(Version {
            generation,
            revision,
            completed: completed.map(u64::try_from).transpose()?,
            needs_validation: phase != "ready",
        })
    })
    .transpose()
}

impl SearchIndex {
    pub fn load_startup(volume: &Volume, cancelled: &AtomicBool) -> Result<Option<Self>> {
        if let Ok(Some(index)) = Self::read_startup_cache(volume, cancelled) {
            return Ok(Some(index));
        }
        ensure!(!cancelled.load(Ordering::Relaxed), "搜索已取消");
        Self::load_cancelled(volume, cancelled)
    }

    fn read_startup_cache(volume: &Volume, cancelled: &AtomicBool) -> Result<Option<Self>> {
        ensure!(!cancelled.load(Ordering::Relaxed), "搜索已取消");
        let _snapshot = volume.db.unchecked_transaction()?;
        let Some(version) = current_version(volume)? else {
            return Ok(None);
        };
        let database = volume.db.path().context("分区库没有路径")?;
        let file = File::open(path(Path::new(database)))?;
        let length = file.metadata()?.len();
        ensure!((12..=MAX_BYTES).contains(&length), "搜索缓存大小无效");
        let mut bytes = Vec::with_capacity(length.try_into()?);
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 == length && bytes.starts_with(MAGIC),
            "搜索缓存格式无效"
        );
        ensure!(!cancelled.load(Ordering::Relaxed), "搜索已取消");
        let (payload, checksum) = bytes.split_at(bytes.len() - 4);
        ensure!(
            crc32fast::hash(payload).to_le_bytes() == checksum,
            "搜索缓存损坏"
        );
        let mut decoder = rmp_serde::Deserializer::from_read_ref(&payload[MAGIC.len()..]);
        decoder.set_max_depth(16);
        let (saved, identity) = <(Version, String)>::deserialize(&mut decoder)?;
        if saved != version || identity != filesystem_id(volume)? {
            return Ok(None);
        }
        let contents = Contents::deserialize(&mut decoder)?;
        ensure!(
            contents.directories.len() == contents.folded_directories.len()
                && contents.directories.len() == contents.directory_entries.len()
                && contents
                    .entries
                    .iter()
                    .all(|entry| entry.parent < contents.directories.len()),
            "搜索缓存目录无效"
        );
        let directory_entries: Vec<_> = contents
            .directory_entries
            .into_iter()
            .map(|id| id.into_vec())
            .collect();
        let directory_ids = directory_entries
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), i))
            .collect();
        let links: HashSet<_> = contents.links.into_iter().map(|id| id.into_vec()).collect();
        ensure!(!cancelled.load(Ordering::Relaxed), "搜索已取消");
        Ok(Some(Self {
            generation: version.generation,
            revision: version.revision,
            partial: false,
            indexed_at: version.completed,
            needs_validation: version.needs_validation,
            startup_cache_loaded: true,
            directories: contents.directories,
            folded_directories: contents.folded_directories,
            directory_entries,
            directory_ids,
            links,
            entries: contents.entries,
        }))
    }

    pub fn save_startup_cache(&self, volume: &Volume, deadline: Instant) -> Result<()> {
        if self.startup_cache_loaded || self.partial || Instant::now() >= deadline {
            return Ok(());
        }
        let database = volume.db.path().context("分区库没有路径")?;
        let destination = path(Path::new(database));
        let _snapshot = volume.db.unchecked_transaction()?;
        let version = Version {
            generation: self.generation,
            revision: self.revision,
            completed: self.indexed_at,
            needs_validation: self.needs_validation,
        };
        if current_version(volume)?.as_ref() != Some(&version) {
            return Ok(());
        }
        let identity = filesystem_id(volume)?;
        let contents = ContentsRef {
            directories: &self.directories,
            folded_directories: &self.folded_directories,
            directory_entries: self
                .directory_entries
                .iter()
                .map(|id| serde_bytes::Bytes::new(id))
                .collect(),
            links: self
                .links
                .iter()
                .map(|id| serde_bytes::Bytes::new(id))
                .collect(),
            entries: &self.entries,
        };
        let mut temporary =
            tempfile::NamedTempFile::new_in(destination.parent().context("搜索缓存无父目录")?)?;
        {
            let mut writer = CacheWriter {
                writer: BufWriter::with_capacity(256 * 1024, temporary.as_file_mut()),
                hash: crc32fast::Hasher::new(),
                deadline,
                length: 0,
            };
            writer.write_all(MAGIC)?;
            rmp_serde::encode::write(&mut writer, &(version, identity))?;
            rmp_serde::encode::write(&mut writer, &contents)?;
            let checksum = std::mem::take(&mut writer.hash).finalize();
            writer.writer.write_all(&checksum.to_le_bytes())?;
            writer.writer.flush()?;
        }
        temporary.persist(destination)?;
        Ok(())
    }
}

struct CacheWriter<'a> {
    writer: BufWriter<&'a mut File>,
    hash: crc32fast::Hasher,
    deadline: Instant,
    length: u64,
}
impl Write for CacheWriter<'_> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.length + data.len() as u64 + 4 > MAX_BYTES {
            return Err(std::io::Error::other("搜索缓存超过大小上限"));
        }
        if self.length.is_multiple_of(1 << 16)
            || (self.length >> 16) != ((self.length + data.len() as u64) >> 16)
        {
            ensure_deadline(self.deadline)?;
        }
        self.writer.write_all(data)?;
        self.hash.update(data);
        self.length += data.len() as u64;
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

fn ensure_deadline(deadline: Instant) -> std::io::Result<()> {
    if Instant::now() >= deadline {
        return Err(std::io::Error::other("搜索缓存保存超时"));
    }
    Ok(())
}
