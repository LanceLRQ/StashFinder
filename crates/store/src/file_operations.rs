use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use stashfinder_core::{MAX_FRAME, file_operations::*, search::FileFilter};
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
        matches!(version, 4..=6) || records_only && version == 3,
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
    filter: Option<&FileFilter>,
) -> Result<Vec<EntryRef>> {
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let predicate = crate::browse::filter_clause(&db, filter)?;
    let tx = db.transaction()?;
    ensure!(
        resolve_in(&tx, context, true, records_only)?.folder,
        "粘贴或选择目标不是目录"
    );
    let sql = if predicate.is_empty() {
        "SELECT id FROM paths WHERE generation=?1 AND parent=?2 ORDER BY id LIMIT ?3".into()
    } else {
        format!(
            "SELECT p.id FROM paths p JOIN objects o ON o.generation=p.generation AND o.id=p.object WHERE p.generation=?1 AND p.parent=?2{predicate} ORDER BY p.id LIMIT ?3"
        )
    };
    let mut stmt = tx.prepare(&sql)?;
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

#[derive(Default)]
pub struct ThumbnailCandidates {
    pub analyse_indices: Vec<usize>,
    pub generation: String,
    pub files: Vec<ResolvedEntry>,
    pub folders: Vec<Vec<u8>>,
    pub next: Option<i64>,
    pub unavailable: usize,
}

pub fn thumbnail_candidates(
    path: &Path,
    request: &ThumbnailPageRequest,
) -> Result<ThumbnailCandidates> {
    let mut types = request.types.clone();
    types.normalize()?;
    let extensions = types.selected_extensions();
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    if request.analyse
        && request.after.is_none()
        && db.query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))? < 6
    {
        crate::volume::Volume::open(path)?;
    }
    let tx = db.transaction()?;
    let (generation, revision): (i64, i64) = tx.query_row(
        "SELECT g.id,g.revision FROM index_state s JOIN generations g ON g.id=s.active_generation WHERE s.id=1",
        [],
        |r| Ok((r.get(0)?,r.get(1)?)),
    ).context("分区尚无可用索引")?;
    if request.analyse && request.after.is_none() {
        ensure!(
            tx.query_row(
                "SELECT coalesce(length(journal)=8,0) FROM generations WHERE id=?1",
                [generation],
                |r| r.get::<_, bool>(0)
            )?,
            "视频识别需要已建立 USN 索引的 NTFS 分区"
        );
    }
    let generation_text = generation.to_string();
    ensure!(
        request
            .generation
            .as_ref()
            .is_none_or(|g| g == &generation_text),
        "索引代次已变化，请重新开始"
    );
    let mut reference = EntryRef {
        partition_id: request.partition_id.clone(),
        filesystem_id: request.filesystem_id.clone(),
        generation: generation_text.clone(),
        revision: revision.to_string(),
        entry: request.folder.clone().unwrap_or_default(),
    };
    if request.folder.is_some() && request.after.is_none() {
        ensure!(
            resolve_in(&tx, &reference, true, false)?.folder,
            "缩略图范围不是文件夹"
        );
    }
    let query = if request.folder.is_some() {
        "SELECT p.rowid,p.id,p.name,o.kind,o.attributes,p.object FROM paths p INDEXED BY paths_parent JOIN objects o
         ON o.generation=p.generation AND o.id=p.object
         WHERE p.generation=?1 AND p.parent=?2 AND p.rowid>?3 ORDER BY p.rowid LIMIT 65"
    } else {
        "SELECT p.rowid,p.id,p.name,o.kind,o.attributes,p.object FROM paths p NOT INDEXED JOIN objects o
         ON o.generation=p.generation AND o.id=p.object
         WHERE p.generation=?1 AND ?2 IS NULL AND p.rowid>?3 ORDER BY p.rowid LIMIT 65"
    };
    let mut stmt = tx.prepare(query)?;
    let rows = stmt
        .query_map(
            params![generation, request.folder, request.after.unwrap_or(0)],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, u32>(4)?,
                    r.get::<_, Vec<u8>>(5)?,
                ))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut result = ThumbnailCandidates {
        generation: generation_text,
        ..Default::default()
    };
    let mut last = None;
    // Leave space for the shared volume identity and the IPC envelope.
    let mut bytes = 0;
    for (i, (rowid, id, name, kind, attributes, object)) in rows.into_iter().enumerate() {
        if i == 64 {
            result.next = last;
            break;
        }
        if kind == "folder" && request.folder.is_some() && attributes & 1024 == 0 {
            bytes += serde_json::to_vec(&id)?.len() + 1;
            if bytes > MAX_FRAME / 2 {
                result.next = last;
                break;
            }
            result.folders.push(id.clone());
        }
        if kind == "file" && attributes & 1024 == 0 {
            let ext = crate::search::extension(&crate::search::decode_name(&name)?).to_lowercase();
            let analyse = request.analyse && (VIDEO_EXTENSIONS.contains(&ext.as_str()) ||
                tx.prepare_cached("SELECT EXISTS(SELECT 1 FROM video_analysis WHERE generation=?1 AND object=?2)")?
                    .query_row(params![generation,object], |r| r.get::<_,bool>(0))?);
            if !analyse && !extensions.contains(ext.as_str()) {
                last = Some(rowid);
                continue;
            }
            reference.entry.clone_from(&id);
            match resolve_in(&tx, &reference, false, false) {
                Ok(file) => {
                    let n = serde_json::to_vec(&file)?.len() + 1;
                    if n > MAX_FRAME / 2 {
                        result.unavailable += 1;
                    } else if bytes + n > MAX_FRAME / 2 {
                        result.next = last;
                        break;
                    } else {
                        bytes += n;
                        if analyse {
                            result.analyse_indices.push(result.files.len());
                        }
                        result.files.push(file);
                    }
                }
                Err(_) => result.unavailable += 1,
            }
        }
        last = Some(rowid);
    }
    Ok(result)
}

pub fn video_results(
    path: &Path,
    generation: i64,
) -> Result<std::collections::HashMap<Vec<u8>, Option<bool>>> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    if db.query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))? < 6 {
        return Ok(Default::default());
    }
    let mut stmt = db.prepare(
        "SELECT a.object, CASE WHEN o.video_current=1 THEN a.video END
        FROM video_analysis a LEFT JOIN objects o ON o.generation=a.generation AND o.id=a.object WHERE a.generation=?1",
    )?;
    Ok(stmt
        .query_map([generation], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?)
}

fn analysis_file(tx: &rusqlite::Transaction<'_>, reference: &EntryRef) -> Result<ResolvedEntry> {
    let file = resolve_in(tx, reference, false, false)?;
    ensure!(
        !file.folder
            && matches!(file.identity, Some(ExpectedIdentity::Ntfs { .. }))
            && tx.query_row(
                "SELECT phase='ready' AND length(journal)=8 FROM generations WHERE id=?1",
                [&reference.generation],
                |r| r.get::<_, Option<bool>>(0)
            )? == Some(true),
        "视频识别需要已建立 USN 索引的 NTFS 分区"
    );
    Ok(file)
}

pub fn current_video(path: &Path, reference: &EntryRef) -> Result<(ResolvedEntry, Option<bool>)> {
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let tx = db.transaction()?;
    let mut reference = reference.clone();
    reference.revision = tx
        .query_row(
            "SELECT revision FROM generations WHERE id=?1",
            [&reference.generation],
            |r| r.get::<_, i64>(0),
        )?
        .to_string();
    let file = analysis_file(&tx, &reference)?;
    let video = tx
        .query_row(
            "SELECT a.video FROM video_analysis a JOIN objects o ON o.generation=a.generation AND o.id=a.object WHERE a.generation=?1 AND a.object=?2 AND o.video_current=1",
            params![reference.generation, file.object],
            |r| r.get::<_, Option<bool>>(0),
        )
        .optional()?
        .flatten();
    Ok((file, video))
}

pub fn save_video(path: &Path, reference: &EntryRef, video: Option<bool>) -> Result<()> {
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let object = analysis_file(&tx, reference)?.object;
    tx.execute(
        "INSERT INTO video_analysis_history(generation,object,revision,video)
         SELECT a.generation,a.object,a.revision,a.video FROM video_analysis a
         JOIN objects o ON o.generation=a.generation AND o.id=a.object
         WHERE a.generation=?1 AND a.object=?2 AND a.video IS NOT NULL AND (o.video_current IS NULL OR a.video IS NOT ?3)",
        params![reference.generation, object, video],
    )?;
    tx.execute(
        "INSERT INTO video_analysis(generation,object,video,revision) VALUES(?1,?2,?3,?4)
        ON CONFLICT(generation,object) DO UPDATE SET video=excluded.video,revision=excluded.revision",
        params![reference.generation, object, video, reference.revision],
    )?;
    tx.execute(
        "UPDATE objects SET video_current=1 WHERE generation=?1 AND id=?2",
        params![reference.generation, object],
    )?;
    tx.commit()?;
    Ok(())
}
