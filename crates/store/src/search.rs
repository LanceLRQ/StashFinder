use crate::volume::Volume;
#[path = "query.rs"]
pub mod query;
use anyhow::{Context, Result, ensure};
use stashfinder_core::{
    index::{Matches, SearchRow},
    search::Query,
};
use std::{
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicBool, Ordering},
};

pub struct SearchIndex {
    pub generation: i64,
    pub revision: i64,
    pub partial: bool,
    pub indexed_at: Option<u64>,
    pub needs_validation: bool,
    directories: Vec<String>,
    folded_directories: Vec<String>,
    directory_entries: Vec<Vec<u8>>,
    entries: Vec<CachedEntry>,
    directory_ids: HashMap<Vec<u8>, usize>,
    links: HashSet<Vec<u8>>,
}

struct CachedEntry {
    parent: usize,
    row: CachedRow,
}
struct CachedRow {
    entry: Box<[u8]>,
    object: Box<[u8]>,
    name: Box<str>,
    folded: Box<str>,
    kind: &'static str,
    size: Option<u64>,
    created: Option<u64>,
    modified: Option<u64>,
    attributes: u32,
}
impl CachedRow {
    fn record<'a>(
        &'a self,
        parent: &'a str,
        folded_parent: &'a str,
    ) -> stashfinder_core::search::Record<'a> {
        stashfinder_core::search::Record {
            name: &self.name,
            folded_name: &self.folded,
            parent,
            folded_parent,
            kind: self.kind,
            size: self.size,
            created: self.created,
            modified: self.modified,
        }
    }
    fn name_key(&self, case: bool) -> &str {
        if case { &self.name } else { &self.folded }
    }
    fn materialize(&self, parent: &[u8], prefix: &str) -> SearchRow {
        SearchRow {
            entry: self.entry.to_vec(),
            parent: parent.to_vec(),
            object: self.object.to_vec(),
            name: self.name.to_string(),
            path: if prefix.is_empty() {
                self.name.to_string()
            } else {
                format!("{prefix}\\{}", self.name)
            },
            kind: self.kind.into(),
            attributes: self.attributes,
            size: self.size.map(|n| n.to_string()),
            created: self.created.map(|n| n.to_string()),
            modified: self.modified.map(|n| n.to_string()),
        }
    }
}

impl SearchIndex {
    pub fn load(volume: &Volume) -> Result<Option<Self>> {
        Self::load_cancelled(volume, &AtomicBool::new(false))
    }
    pub fn load_cancelled(volume: &Volume, cancelled: &AtomicBool) -> Result<Option<Self>> {
        let status = volume.index_status()?;
        let generation = match status.active_generation {
            Some(id) => id,
            None if status.phase == "failed" => status.generation.context("缺少部分索引代次")?,
            None => return Ok(None),
        };
        Ok(Some(Self::load_generation_inner(
            volume,
            generation,
            status.active_generation.is_none(),
            cancelled,
        )?))
    }

    pub fn load_generation(volume: &Volume, generation: i64, partial: bool) -> Result<Self> {
        Self::load_generation_inner(volume, generation, partial, &AtomicBool::new(false))
    }
    fn load_generation_inner(
        volume: &Volume,
        generation: i64,
        partial: bool,
        cancelled: &AtomicBool,
    ) -> Result<Self> {
        let _snapshot = volume.db.unchecked_transaction()?;
        let (completed, phase, revision): (Option<i64>, String, i64) = volume.db.query_row(
            "SELECT completed,phase,revision FROM generations WHERE id=?1",
            [generation],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let mut directories = Vec::new();
        let mut ids = HashMap::new();
        let mut parents = Vec::new();
        let mut names = Vec::new();
        let mut stmt=volume.db.prepare("SELECT p.id,p.parent,p.name FROM paths p JOIN objects o ON o.generation=p.generation AND o.id=p.object WHERE p.generation=?1 AND o.kind='folder' ORDER BY p.id")?;
        let mut rows = stmt.query([generation])?;
        while let Some(row) = rows.next()? {
            ensure!(!cancelled.load(Ordering::Relaxed), "搜索已取消");
            let id: Vec<u8> = row.get(0)?;
            ids.insert(id, directories.len());
            parents.push(row.get::<_, Option<Vec<u8>>>(1)?);
            names.push(decode_name(&row.get::<_, Vec<u8>>(2)?)?);
            directories.push(None::<String>);
        }
        for start in 0..directories.len() {
            ensure!(!cancelled.load(Ordering::Relaxed), "搜索已取消");
            let mut chain = Vec::new();
            let mut seen = HashSet::new();
            let mut current = start;
            while directories[current].is_none() {
                ensure!(seen.insert(current), "目录父链存在循环");
                chain.push(current);
                match &parents[current] {
                    Some(parent) => current = *ids.get(parent).context("目录父节点缺失")?,
                    None => {
                        directories[current] = Some(String::new());
                        break;
                    }
                }
            }
            while let Some(index) = chain.pop() {
                if let Some(parent) = &parents[index] {
                    let parent = *ids.get(parent).context("目录父节点缺失")?;
                    let prefix = directories[parent].as_deref().context("目录父链未解析")?;
                    directories[index] = Some(if prefix.is_empty() {
                        names[index].clone()
                    } else {
                        format!("{prefix}\\{}", names[index])
                    });
                }
            }
        }
        let directories = directories
            .into_iter()
            .map(|d| d.context("目录路径缺失"))
            .collect::<Result<Vec<_>>>()?;
        let mut entries = Vec::new();
        let mut stmt=volume.db.prepare("SELECT p.object,p.parent,p.name,o.kind,o.size,o.created,o.modified,o.attributes,p.id FROM paths p JOIN objects o ON o.generation=p.generation AND o.id=p.object WHERE p.generation=?1 AND p.parent IS NOT NULL ORDER BY p.id")?;
        let mut rows = stmt.query([generation])?;
        while let Some(row) = rows.next()? {
            ensure!(!cancelled.load(Ordering::Relaxed), "搜索已取消");
            let parent: Vec<u8> = row.get(1)?;
            let name = decode_name(&row.get::<_, Vec<u8>>(2)?)?;
            entries.push(CachedEntry {
                parent: *ids.get(&parent).context("路径父目录缺失")?,
                row: CachedRow {
                    entry: row.get::<_, Vec<u8>>(8)?.into_boxed_slice(),
                    object: row.get::<_, Vec<u8>>(0)?.into_boxed_slice(),
                    folded: name.to_lowercase().into_boxed_str(),
                    name: name.into_boxed_str(),
                    kind: if row.get::<_, String>(3)? == "folder" {
                        "folder"
                    } else {
                        "file"
                    },
                    size: row.get::<_, Option<i64>>(4)?.map(|n| n as u64),
                    created: decode_time_value(row.get(5)?)?,
                    modified: decode_time_value(row.get(6)?)?,
                    attributes: row.get(7)?,
                },
            });
        }
        let mut statement = volume.db.prepare("SELECT object FROM paths WHERE generation=?1 AND parent IS NOT NULL GROUP BY object HAVING count(*)>1")?;
        let links = statement
            .query_map([generation], |r| r.get(0))?
            .collect::<rusqlite::Result<HashSet<Vec<u8>>>>()?;
        let mut directory_entries = vec![vec![]; ids.len()];
        for (id, index) in &ids {
            directory_entries[*index] = id.clone();
        }
        Ok(Self {
            revision,
            folded_directories: directories.iter().map(|s| s.to_lowercase()).collect(),
            directory_entries,
            directory_ids: ids,
            links,
            generation,
            partial,
            indexed_at: completed.map(u64::try_from).transpose()?,
            needs_validation: phase != "ready",
            directories,
            entries,
        })
    }

    pub fn search(
        &self,
        query: &Query,
        mounts: &[String],
        limit: usize,
        cancelled: &AtomicBool,
    ) -> Result<Matches> {
        ensure!(limit > 0 && limit <= 200, "搜索返回数量无效");
        let mut rows = Vec::new();
        for entry in &self.entries {
            ensure!(!cancelled.load(Ordering::Relaxed), "搜索已取消");
            let parent = &self.directories[entry.parent];
            if query.matches_record(
                &entry
                    .row
                    .record(parent, &self.folded_directories[entry.parent]),
                mounts,
            )? {
                if rows.len() == limit {
                    return Ok(Matches { rows, more: true });
                }
                rows.push(
                    entry
                        .row
                        .materialize(&self.directory_entries[entry.parent], parent),
                );
            }
        }
        Ok(Matches { rows, more: false })
    }
}

pub(crate) fn extension(name: &str) -> &str {
    name.rsplit_once('.').map_or("", |(_, extension)| extension)
}

pub(crate) fn decode_name(bytes: &[u8]) -> Result<String> {
    ensure!(bytes.len().is_multiple_of(2), "UTF-16 名称字节数无效");
    let units = bytes
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]));
    let mut value = String::new();
    for character in char::decode_utf16(units) {
        match character {
            Ok(c) if !c.is_control() => value.push(c),
            Ok(c) => value.push_str(&format!("\\u{{{:04x}}}", c as u32)),
            Err(error) => value.push_str(&format!("\\u{{{:04x}}}", error.unpaired_surrogate())),
        }
    }
    Ok(value)
}

pub(crate) fn decode_time(bytes: Option<Vec<u8>>) -> Result<Option<String>> {
    Ok(decode_time_value(bytes)?.map(|value| value.to_string()))
}

pub(crate) fn decode_time_value(bytes: Option<Vec<u8>>) -> Result<Option<u64>> {
    bytes
        .map(|b| {
            Ok(u64::from_le_bytes(
                b.try_into()
                    .map_err(|_| anyhow::anyhow!("FILETIME 长度无效"))?,
            ))
        })
        .transpose()
}
