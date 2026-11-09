use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use stashfinder_core::{MAX_FRAME, file_operations::*};
use std::{collections::HashSet, path::Path};

/// Resolve raw names in one local database snapshot, without touching the source volume.
pub fn resolve(path: &Path, reference: &EntryRef, allow_root: bool) -> Result<ResolvedEntry> {
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let tx = db.transaction()?;
    resolve_in(&tx, reference, allow_root, false)
}
pub fn record(path: &Path, reference: &EntryRef) -> Result<ResolvedEntry> {
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let tx = db.transaction()?;
    resolve_in(&tx, reference, true, true)
}
fn resolve_in(
    tx: &rusqlite::Transaction<'_>,
    reference: &EntryRef,
    allow_root: bool,
    records_only: bool,
) -> Result<ResolvedEntry> {
    let generation: i64 = reference.generation.parse()?;
    let revision: i64 = reference.revision.parse()?;
    let version = tx.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?;
    ensure!(
        matches!(version, 4..=5) || records_only && version == 3,
        "目录库需要升级"
    );
    let manifest: String =
        tx.query_row("SELECT value FROM manifest WHERE id=1", [], |r| r.get(0))?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest)?;
    ensure!(
        manifest["partition_id"].as_str() == Some(&reference.partition_id)
            && manifest["filesystem_id"].as_str() == Some(&reference.filesystem_id),
        "目录库归属不一致"
    );
    let current: Option<i64> = tx.query_row(
        "SELECT active_generation FROM index_state WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    let latest: Option<i64> = tx.query_row("SELECT max(id) FROM generations", [], |r| r.get(0))?;
    ensure!(
        current == Some(generation)
            || records_only && current.is_none() && latest == Some(generation),
        "索引代次已变化，请刷新重选"
    );
    let current: i64 = tx.query_row(
        "SELECT revision FROM generations WHERE id=?1",
        [generation],
        |r| r.get(0),
    )?;
    ensure!(current == revision, "索引记录已变化，请刷新重选");
    let mut next = Some(reference.entry.clone());
    let mut seen = HashSet::new();
    let mut components = Vec::new();
    let mut selected = None;
    while let Some(id) = next {
        ensure!(seen.insert(id.clone()), "目录父链存在循环");
        let row=tx.query_row("SELECT p.object,p.parent,p.name,o.kind,o.attributes,o.source FROM paths p JOIN objects o ON o.generation=p.generation AND o.id=p.object WHERE p.generation=?1 AND p.id=?2",params![generation,id],|r|Ok((r.get::<_,Vec<u8>>(0)?,r.get::<_,Option<Vec<u8>>>(1)?,r.get::<_,Vec<u8>>(2)?,r.get::<_,String>(3)?,r.get::<_,u32>(4)?,r.get::<_,String>(5)?))).optional()?.context("文件记录已不存在，请刷新")?;
        let (object, parent, name, kind, attributes, source) = row;
        ensure!(
            records_only || attributes & 1024 == 0,
            "暂不操作重解析点或经重解析目录的路径"
        );
        if selected.is_none() {
            selected = Some((object, parent.clone(), kind == "folder", source));
        } else {
            ensure!(kind == "folder", "父节点不是目录");
        }
        if parent.is_some() {
            ensure!(name.len().is_multiple_of(2), "原始名称损坏");
            let name: Vec<u16> = name
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();
            ensure!(
                !name.is_empty()
                    && name != [46]
                    && name != [46, 46]
                    && !name.iter().any(|c| [0, 47, 58, 92].contains(c)),
                "原始名称不能安全解析"
            );
            components.push(name);
        }
        next = parent;
    }
    let (object, parent, folder, source) = selected.context("缺少条目标识")?;
    ensure!(allow_root || parent.is_some(), "不能对分区根目录执行该操作");
    let mut relative = Vec::new();
    for name in components.into_iter().rev() {
        if !relative.is_empty() {
            relative.push(92);
        }
        relative.extend(name);
        ensure!(relative.len() < 32767, "路径过长");
    }
    let identity = if source == "ntfs64" {
        Some(ExpectedIdentity::Ntfs {
            frn: u64::from_le_bytes(object.as_slice().try_into().context("NTFS 对象标识损坏")?),
        })
    } else if version >= 4 {
        crate::volume::read_native_identity(tx, generation, &object)?.map(ExpectedIdentity::Native)
    } else {
        None
    };
    let (size, created, modified) = tx.query_row(
        "SELECT size,created,modified FROM objects WHERE generation=?1 AND id=?2",
        params![generation, object],
        |r| {
            Ok((
                r.get::<_, Option<i64>>(0)?,
                r.get::<_, Option<Vec<u8>>>(1)?,
                r.get::<_, Option<Vec<u8>>>(2)?,
            ))
        },
    )?;
    let result = ResolvedEntry {
        size,
        created: crate::search::decode_time(created)?
            .map(|s| s.parse())
            .transpose()?,
        modified: crate::search::decode_time(modified)?
            .map(|s| s.parse())
            .transpose()?,
        reference: reference.clone(),
        relative,
        object,
        parent,
        folder,
        identity,
    };
    ensure!(
        serde_json::to_vec(&result)?.len() < MAX_FRAME - 512,
        "操作条目超过单帧传输上限"
    );
    Ok(result)
}

pub fn directory_refs(
    path: &Path,
    context: &EntryRef,
    records_only: bool,
) -> Result<Vec<EntryRef>> {
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let tx = db.transaction()?;
    ensure!(
        resolve_in(&tx, context, true, records_only)?.folder,
        "粘贴或选择目标不是目录"
    );
    let mut stmt =
        tx.prepare("SELECT id FROM paths WHERE generation=?1 AND parent=?2 ORDER BY id LIMIT ?3")?;
    let rows = stmt
        .query_map(
            params![
                context.generation.parse::<i64>()?,
                context.entry,
                (MAX_SELECTED * 2 + 1) as i64
            ],
            |r| r.get::<_, Vec<u8>>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(
        rows.len() <= MAX_SELECTED * 2,
        "选择过大，单次最多操作 10,000 项"
    );
    Ok(rows
        .into_iter()
        .map(|entry| EntryRef {
            entry,
            ..context.clone()
        })
        .collect())
}

pub use stashfinder_core::file_operations::top_level;
