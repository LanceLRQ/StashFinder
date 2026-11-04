use crate::volume::Volume;
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
    pub partial: bool,
    directories: Vec<String>,
    entries: Vec<CachedEntry>,
}

struct CachedEntry {
    parent: usize,
    row: SearchRow,
}

impl SearchIndex {
    pub fn load(volume: &Volume) -> Result<Option<Self>> {
        let status = volume.index_status()?;
        let generation = match status.active_generation {
            Some(id) => id,
            None if status.phase == "failed" => status.generation.context("缺少部分索引代次")?,
            None => return Ok(None),
        };
        let mut directories = Vec::new();
        let mut ids = HashMap::new();
        let mut parents = Vec::new();
        let mut names = Vec::new();
        let mut stmt=volume.db.prepare("SELECT p.id,p.parent,p.name FROM paths p JOIN objects o ON o.generation=p.generation AND o.id=p.object WHERE p.generation=?1 AND o.kind='folder' ORDER BY p.id")?;
        let mut rows = stmt.query([generation])?;
        while let Some(row) = rows.next()? {
            let id: Vec<u8> = row.get(0)?;
            ids.insert(id, directories.len());
            parents.push(row.get::<_, Option<Vec<u8>>>(1)?);
            names.push(decode_name(&row.get::<_, Vec<u8>>(2)?)?);
            directories.push(None::<String>);
        }
        for start in 0..directories.len() {
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
        let mut stmt=volume.db.prepare("SELECT p.object,p.parent,p.name,o.kind,o.size,o.created,o.modified,o.attributes FROM paths p JOIN objects o ON o.generation=p.generation AND o.id=p.object WHERE p.generation=?1 AND p.parent IS NOT NULL ORDER BY p.id")?;
        let mut rows = stmt.query([generation])?;
        while let Some(row) = rows.next()? {
            let parent: Vec<u8> = row.get(1)?;
            entries.push(CachedEntry {
                parent: *ids.get(&parent).context("路径父目录缺失")?,
                row: SearchRow {
                    object: row.get(0)?,
                    name: decode_name(&row.get::<_, Vec<u8>>(2)?)?,
                    path: String::new(),
                    kind: row.get(3)?,
                    size: row.get::<_, Option<i64>>(4)?.map(|n| n.to_string()),
                    created: decode_time(row.get(5)?)?,
                    modified: decode_time(row.get(6)?)?,
                    attributes: row.get(7)?,
                },
            });
        }
        Ok(Some(Self {
            generation,
            partial: status.active_generation.is_none(),
            directories,
            entries,
        }))
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
            let path = if parent.is_empty() {
                entry.row.name.clone()
            } else {
                format!("{parent}\\{}", entry.row.name)
            };
            if query.matches(&entry.row.name, &path, mounts) {
                if rows.len() == limit {
                    return Ok(Matches { rows, more: true });
                }
                let mut row = entry.row.clone();
                row.path = path;
                rows.push(row);
            }
        }
        Ok(Matches { rows, more: false })
    }
}

fn decode_name(bytes: &[u8]) -> Result<String> {
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

fn decode_time(bytes: Option<Vec<u8>>) -> Result<Option<String>> {
    bytes
        .map(|b| {
            Ok(u64::from_le_bytes(
                b.try_into()
                    .map_err(|_| anyhow::anyhow!("FILETIME 长度无效"))?,
            )
            .to_string())
        })
        .transpose()
}
