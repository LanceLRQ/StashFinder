use crate::volume::Volume;
#[path = "query.rs"]
pub mod query;
use anyhow::{Context, Result, ensure};
use rusqlite::OptionalExtension;
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
struct ObjectMetadata {
    folder: bool,
    size: Option<u64>,
    created: Option<u64>,
    modified: Option<u64>,
    attributes: u32,
    seen: bool,
}
struct DirectoryNode {
    parent: Option<usize>,
    name: String,
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
        let _snapshot = volume.db.unchecked_transaction()?;
        let (generation, partial) = if let Some(id) = volume.active_generation()? {
            (id, false)
        } else {
            let latest: Option<(i64, String)> = volume
                .db
                .query_row(
                    "SELECT id,phase FROM generations ORDER BY id DESC LIMIT 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            match latest {
                Some((id, phase)) if phase == "failed" => (id, true),
                _ => return Ok(None),
            }
        };
        Ok(Some(Self::load_generation_inner(
            volume, generation, partial, cancelled,
        )?))
    }

    pub fn load_generation(volume: &Volume, generation: i64, partial: bool) -> Result<Self> {
        let _snapshot = volume.db.unchecked_transaction()?;
        Self::load_generation_inner(volume, generation, partial, &AtomicBool::new(false))
    }
    fn load_generation_inner(
        volume: &Volume,
        generation: i64,
        partial: bool,
        cancelled: &AtomicBool,
    ) -> Result<Self> {
        let (completed, phase, revision): (Option<i64>, String, i64) = volume.db.query_row(
            "SELECT completed,phase,revision FROM generations WHERE id=?1",
            [generation],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        // Read the tables in row order to avoid a B-tree lookup per path.
        let mut objects = HashMap::new();
        let mut statement = volume.db.prepare(
            "SELECT id,kind,size,created,modified,attributes FROM objects NOT INDEXED WHERE generation=?1",
        )?;
        let mut rows = statement.query([generation])?;
        while let Some(row) = rows.next()? {
            ensure!(!cancelled.load(Ordering::Relaxed), "搜索已取消");
            objects.insert(
                row.get::<_, Vec<u8>>(0)?,
                ObjectMetadata {
                    folder: row.get_ref(1)?.as_str()? == "folder",
                    size: row.get::<_, Option<i64>>(2)?.map(|n| n as u64),
                    created: decode_time_value(row.get_ref(3)?.as_blob_or_null()?)?,
                    modified: decode_time_value(row.get_ref(4)?.as_blob_or_null()?)?,
                    attributes: row.get(5)?,
                    seen: false,
                },
            );
        }
        let mut ids = HashMap::new();
        let mut nodes: Vec<Option<DirectoryNode>> = Vec::new();
        let mut directory_id = |id: &[u8], nodes: &mut Vec<_>| {
            if let Some(index) = ids.get(id) {
                *index
            } else {
                let index = nodes.len();
                ids.insert(id.to_vec(), index);
                nodes.push(None);
                index
            }
        };
        let mut entries = Vec::with_capacity(objects.len().saturating_sub(1));
        let mut links = HashSet::new();
        let mut stmt = volume
            .db
            .prepare("SELECT id,object,parent,name FROM paths NOT INDEXED WHERE generation=?1")?;
        let mut rows = stmt.query([generation])?;
        while let Some(row) = rows.next()? {
            ensure!(!cancelled.load(Ordering::Relaxed), "搜索已取消");
            let id = row.get_ref(0)?.as_blob()?;
            let object_id = row.get_ref(1)?.as_blob()?;
            let object = objects.get_mut(object_id).context("路径缺少文件对象")?;
            let parent = row
                .get_ref(2)?
                .as_blob_or_null()?
                .map(|id| directory_id(id, &mut nodes));
            let name = decode_name(row.get_ref(3)?.as_blob()?)?;
            if object.folder {
                let index = directory_id(id, &mut nodes);
                nodes[index] = Some(DirectoryNode {
                    parent,
                    name: name.clone(),
                });
            }
            let Some(parent) = parent else {
                continue;
            };
            if std::mem::replace(&mut object.seen, true) && !links.contains(object_id) {
                links.insert(object_id.to_vec());
            }
            entries.push(CachedEntry {
                parent,
                row: CachedRow {
                    entry: id.into(),
                    object: object_id.into(),
                    folded: name.to_lowercase().into_boxed_str(),
                    name: name.into_boxed_str(),
                    kind: if object.folder { "folder" } else { "file" },
                    size: object.size,
                    created: object.created,
                    modified: object.modified,
                    attributes: object.attributes,
                },
            });
        }
        drop(objects);
        entries.sort_unstable_by(|a, b| a.row.entry.cmp(&b.row.entry));
        let mut directories = vec![None::<String>; nodes.len()];
        let mut visited = vec![false; nodes.len()];
        let mut chain = Vec::new();
        for start in 0..nodes.len() {
            ensure!(!cancelled.load(Ordering::Relaxed), "搜索已取消");
            let mut current = Some(start);
            while let Some(index) = current {
                if directories[index].is_some() {
                    break;
                }
                ensure!(
                    !std::mem::replace(&mut visited[index], true),
                    "目录父链存在循环"
                );
                let node = nodes[index].as_ref().context("路径父目录缺失")?;
                chain.push(index);
                current = node.parent;
            }
            while let Some(index) = chain.pop() {
                let node = nodes[index].take().context("目录节点缺失")?;
                directories[index] = Some(if let Some(parent) = node.parent {
                    let prefix = directories[parent].as_deref().context("目录父链未解析")?;
                    if prefix.is_empty() {
                        node.name
                    } else {
                        format!("{prefix}\\{}", node.name)
                    }
                } else {
                    String::new()
                });
            }
        }
        ensure!(!cancelled.load(Ordering::Relaxed), "搜索已取消");
        let directories = directories
            .into_iter()
            .map(|d| d.context("目录路径缺失"))
            .collect::<Result<Vec<_>>>()?;
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
    let mut value = String::with_capacity(bytes.len() / 2);
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
    Ok(decode_time_value(bytes.as_deref())?.map(|value| value.to_string()))
}

pub(crate) fn decode_time_value(bytes: Option<&[u8]>) -> Result<Option<u64>> {
    bytes
        .map(|b| {
            Ok(u64::from_le_bytes(
                b.try_into()
                    .map_err(|_| anyhow::anyhow!("FILETIME 长度无效"))?,
            ))
        })
        .transpose()
}
