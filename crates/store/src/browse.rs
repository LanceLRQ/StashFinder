use crate::search::{decode_name, decode_time};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use stashfinder_core::{MAX_FRAME, browse::*, index::SearchRow};
use std::{collections::HashSet, path::Path};

/// Read only the local catalog. One SQLite snapshot covers the version, ancestors and children.
pub fn read(path: &Path, request: &DirectoryRequest) -> Result<DirectoryPage> {
    ensure!(
        request.generation.is_some() == request.revision.is_some(),
        "目录分页版本不完整"
    );
    ensure!(
        request.offset == 0 || request.revision.is_some(),
        "目录分页缺少版本"
    );
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let tx = db.transaction()?;
    ensure!(
        matches!(
            tx.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
            3..=5
        ),
        "目录库需要升级，请等待服务初始化"
    );
    let manifest: String =
        tx.query_row("SELECT value FROM manifest WHERE id=1", [], |r| r.get(0))?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest)?;
    ensure!(
        manifest["filesystem_id"].as_str() == Some(&request.filesystem_id)
            && manifest["partition_id"].as_str() == Some(&request.partition_id),
        "目录库归属不一致"
    );
    let active: Option<i64> = tx.query_row(
        "SELECT active_generation FROM index_state WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    let latest: Option<i64> = tx.query_row("SELECT max(id) FROM generations", [], |r| r.get(0))?;
    let generation = active.or(latest).context("尚无目录索引")?;
    let (revision, phase, completed): (i64, String, Option<i64>) = tx.query_row(
        "SELECT revision,phase,completed FROM generations WHERE id=?1",
        [generation],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    ensure!(active.is_some() || phase == "failed", "目录索引尚未完成");
    ensure!(
        request
            .generation
            .as_ref()
            .is_none_or(|g| g == &generation.to_string())
            && request
                .revision
                .as_ref()
                .is_none_or(|r| r == &revision.to_string()),
        "目录索引已更新，请刷新"
    );
    let location = match &request.parent {
        Some(id) => id.clone(),
        None => tx
            .query_row(
                "SELECT id FROM paths WHERE generation=?1 AND parent IS NULL",
                [generation],
                |r| r.get(0),
            )
            .optional()?
            .with_context(|| {
                if active.is_none() {
                    "尚无目录索引"
                } else {
                    "目录根节点缺失"
                }
            })?,
    };
    let mut ancestors = Vec::new();
    let mut next = Some(location.clone());
    let mut seen = HashSet::new();
    while let Some(id) = next {
        ensure!(seen.insert(id.clone()), "目录父链存在循环");
        let (name, parent, kind): (Vec<u8>,Option<Vec<u8>>,String) = tx.query_row("SELECT p.name,p.parent,o.kind FROM paths p JOIN objects o ON o.generation=p.generation AND o.id=p.object WHERE p.generation=?1 AND p.id=?2", params![generation,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?.context("目录已不存在，请返回上级目录")?;
        ensure!(kind == "folder", "所选条目不是目录");
        ancestors.push(Ancestor {
            entry: id,
            name: decode_name(&name)?,
        });
        next = parent;
    }
    ancestors.reverse();
    let prefix = ancestors
        .iter()
        .skip(1)
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join("\\");
    let mut page = DirectoryPage {
        total:usize::try_from(tx.query_row("SELECT count(*) FROM paths p JOIN objects o ON o.generation=p.generation AND o.id=p.object WHERE p.generation=?1 AND p.parent=?2 AND (?3=0 OR o.kind='folder')",params![generation,location,request.folders_only],|r|r.get::<_,i64>(0))?)?,
        filesystem_id: request.filesystem_id.clone(),
        generation: generation.to_string(),
        revision: revision.to_string(),
        location: location.clone(),
        ancestors,
        entries: Vec::new(),
        next: None,
        partial: active.is_none(),
        indexed_at: completed.map(u64::try_from).transpose()?,
        needs_validation: phase != "ready" || latest != Some(generation),
    };
    let mut bytes = serde_json::to_vec(&page)?.len();
    ensure!(bytes < MAX_FRAME - 256, "目录路径超过单页传输上限");
    let mut statement=tx.prepare("SELECT p.id,p.object,p.name,o.kind,o.size,o.created,o.modified,o.attributes FROM paths p JOIN objects o ON o.generation=p.generation AND o.id=p.object WHERE p.generation=?1 AND p.parent=?2 AND (?3=0 OR o.kind='folder') ORDER BY o.kind DESC,p.name,p.id LIMIT 101 OFFSET ?4")?;
    let mut rows = statement.query(params![
        generation,
        location,
        request.folders_only,
        request.offset
    ])?;
    while let Some(row) = rows.next()? {
        let name = decode_name(&row.get::<_, Vec<u8>>(2)?)?;
        let entry = SearchRow {
            entry: row.get(0)?,
            parent: location.clone(),
            object: row.get(1)?,
            path: if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}\\{name}")
            },
            name,
            kind: row.get(3)?,
            size: row.get::<_, Option<i64>>(4)?.map(|s| s.to_string()),
            created: decode_time(row.get(5)?)?,
            modified: decode_time(row.get(6)?)?,
            attributes: row.get(7)?,
        };
        let size = serde_json::to_vec(&entry)?.len() + 1;
        if page.entries.len() == 100 || bytes + size >= MAX_FRAME - 256 {
            ensure!(!page.entries.is_empty(), "目录条目超过单页传输上限");
            page.next = Some(request.offset + page.entries.len() as u32);
            break;
        }
        bytes += size;
        page.entries.push(entry);
    }
    Ok(page)
}
