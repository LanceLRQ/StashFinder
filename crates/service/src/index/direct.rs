//! First-generation NTFS indexing for registered volumes.
use super::*;
use std::collections::HashMap;
#[cfg(feature = "memory-index-bench")]
use std::path::Path;

#[cfg(feature = "memory-index-bench")]
pub struct DirectIndex {
    pub index: SearchIndex,
    pub cursor: Cursor,
    pub report: BuildReport,
    rows: Rows,
}

struct Rows {
    objects: Vec<Object>,
    entries: Vec<Entry>,
    root: Vec<u8>,
}

#[cfg(feature = "memory-index-bench")]
impl DirectIndex {
    pub fn persist(
        &self,
        path: &Path,
        cancelled: &AtomicBool,
        progress: &mut dyn FnMut() -> Result<()>,
    ) -> Result<()> {
        Volume::persist_initial_snapshot(
            &self.rows.objects,
            &self.rows.entries,
            &self.rows.root,
            &self.cursor,
            path,
            cancelled,
            progress,
        )
    }
}

#[cfg(feature = "memory-index-bench")]
pub fn build_direct(
    source: &dyn Source,
    cancelled: &AtomicBool,
    progress: &mut dyn FnMut(&str),
) -> Result<DirectIndex> {
    let (rows, cursor, mut report) = collect_rows(
        source,
        &source.journal()?.ok_or(JournalMissing)?,
        cancelled,
        progress,
    )?;
    let started = Instant::now();
    progress("search_build");
    let index = SearchIndex::from_initial(&rows.objects, &rows.entries, cancelled)?;
    progress("search_built");
    source.verify()?;
    ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
    report.validate_time += started.elapsed();
    Ok(DirectIndex {
        index,
        cursor,
        report,
        rows,
    })
}

#[cfg(feature = "memory-index-bench")]
pub fn build_direct_parallel(
    source: &dyn Source,
    cancelled: &AtomicBool,
    path: &Path,
    progress: &mut dyn FnMut(&str),
    searchable: impl FnOnce(&SearchIndex) -> Result<()>,
) -> Result<DirectIndex> {
    ensure!(!path.try_exists()?, "目标索引路径已存在");
    let (rows, cursor, mut report) = collect_rows(
        source,
        &source.journal()?.ok_or(JournalMissing)?,
        cancelled,
        progress,
    )?;
    let index = std::thread::scope(|scope| -> Result<_> {
        let writer = scope.spawn(|| {
            Volume::prepare_initial_snapshot(
                &rows.objects,
                &rows.entries,
                &rows.root,
                &cursor,
                cancelled,
                &mut || Ok(()),
            )?
            .stage_completed_index(path, cancelled, &mut || Ok(()))
        });
        let started = Instant::now();
        progress("search_build");
        let index = SearchIndex::from_initial(&rows.objects, &rows.entries, cancelled)?;
        progress("search_built");
        source.verify()?;
        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
        report.validate_time += started.elapsed();
        searchable(&index)?;
        let backup = writer
            .join()
            .map_err(|_| anyhow::anyhow!("SQLite 建库线程异常退出"))??;
        backup.publish(cancelled)?;
        Ok(index)
    })?;
    Ok(DirectIndex {
        index,
        cursor,
        report,
        rows,
    })
}

pub fn build_registered(
    source: &dyn Source,
    db: &mut Volume,
    mode: IndexMode,
    cancelled: &AtomicBool,
    progress: &mut dyn FnMut(&str),
    diagnostic: impl FnOnce(serde_json::Value),
) -> Result<SearchIndex> {
    if matches!(mode, IndexMode::ScanOnly) || db.has_generations()? {
        return build(source, db, mode, cancelled, progress);
    }
    ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
    source.verify()?;
    let journal = match mode {
        IndexMode::CreateJournal => source.create_journal()?,
        _ => source.journal()?.ok_or(JournalMissing)?,
    };
    let started = Instant::now();
    let (rows, cursor, report) = match collect_rows(source, &journal, cancelled, progress) {
        Ok(rows) => rows,
        Err(e)
            if e.is::<FastScanUnavailable>()
                || e.is::<InitialPathsIncomplete>()
                || e.is::<InitialPathConflict>() =>
        {
            source.verify()?;
            let current = source.journal()?.ok_or(JournalMissing)?;
            ensure!(
                current.id == journal.id
                    && current.first <= journal.next
                    && journal.next <= current.next,
                JournalCursorInvalid
            );
            let fallback_started = Instant::now();
            let result = build_with_strategy(
                source,
                db,
                IndexMode::Journal,
                BuildStrategy::Legacy,
                cancelled,
                progress,
            );
            diagnostic(serde_json::json!({
                "strategy": "directory_walk",
                "fallback_reason": e.to_string(),
                "success": result.is_ok(),
                "elapsed_ms": started.elapsed().as_millis(),
                "before_fallback_ms": (fallback_started - started).as_millis(),
                "legacy_ms": fallback_started.elapsed().as_millis(),
            }));
            return result.map(|(_, index)| index);
        }
        Err(e) => {
            diagnostic(serde_json::json!({
                "strategy": "mft",
                "success": false,
                "elapsed_ms": started.elapsed().as_millis(),
            }));
            return Err(e);
        }
    };
    let commit_started = Instant::now();
    let result = std::thread::scope(|scope| {
        let search =
            scope.spawn(|| SearchIndex::from_initial(&rows.objects, &rows.entries, cancelled));
        let (mut index, generation, revision, completed) = db.commit_first_snapshot(
            stashfinder_store::volume::InitialSnapshot {
                objects: &rows.objects,
                entries: &rows.entries,
                root: &rows.root,
                cursor: &cursor,
            },
            cancelled,
            &mut || Ok(()),
            || {
                let index = search
                    .join()
                    .map_err(|_| anyhow::anyhow!("搜索索引构建线程异常退出"))??;
                source.verify()?;
                Ok(index)
            },
        )?;
        index.generation = generation;
        index.revision = revision;
        index.indexed_at = Some(completed);
        index.needs_validation = false;
        Ok(index)
    });
    diagnostic(serde_json::json!({
        "strategy": "mft",
        "success": result.is_ok(),
        "elapsed_ms": started.elapsed().as_millis(),
        "collect_ms": (commit_started - started).as_millis(),
        "journal_replay_ms": report.replay_time.as_millis(),
        "commit_ms": commit_started.elapsed().as_millis(),
        "objects": rows.objects.len(),
        "paths": rows.entries.len(),
    }));
    result
}

fn collect_rows(
    source: &dyn Source,
    journal: &Journal,
    cancelled: &AtomicBool,
    progress: &mut dyn FnMut(&str),
) -> Result<(Rows, Cursor, BuildReport)> {
    ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
    source.verify()?;
    let root = source.root_id();
    let excluded = source.excluded_roots();
    let mut records = HashMap::new();
    progress("scanning");
    let scan = source
        .full_inventory_deferred(cancelled, &mut |batch| {
            for record in batch {
                ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
                ensure!(
                    records.insert(record.entry.id, record.clone()).is_none(),
                    "MFT 对象身份重复"
                );
            }
            Ok(())
        })?
        .ok_or(FastScanUnavailable("当前卷无法使用顺序元数据读取"))?;
    let stage_time = scan.consumer_time;
    let started = Instant::now();
    let mut deleted_directories = HashSet::new();
    let initial_error = match prune(
        &mut records,
        root,
        &excluded,
        &deleted_directories,
        cancelled,
    ) {
        Ok(()) => None,
        // The MFT is live while we read it. A rename or replacement may leave
        // conflicting names or parent links until the journal is replayed.
        Err(error) if error.is::<InitialPathConflict>() || error.is::<InitialPathsIncomplete>() => {
            Some(error)
        }
        Err(error) => return Err(error),
    };
    let import_time = started.elapsed();
    progress("replaying");
    let started = Instant::now();
    let mut next = journal.next;
    for round in 0..16 {
        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
        let boundary = source.journal()?.ok_or(JournalMissing)?;
        ensure!(
            boundary.id == journal.id && next >= boundary.first && next <= boundary.next,
            JournalCursorInvalid
        );
        while next < boundary.next {
            let page = source.changes(journal.id, next)?;
            ensure!(
                page.next > next as u64 && page.next <= i64::MAX as u64,
                "USN 回放未前进"
            );
            let ids =
                replay_targets(&page, next, |record| {
                    Ok(records.contains_key(&record.id)
                        || !source.excluded_directory(record.parent)?)
                })?;
            let mut later_deletions = None;
            for (id, target) in ids {
                ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
                if !target.inspect
                    || (target.reason.is_none() && !records.contains_key(&id))
                    || (id & 0x0000ffffffffffff < 16 && id != root)
                    || excluded.contains(&id)
                {
                    continue;
                }
                match source.object(id)? {
                    Some(current) => {
                        if let Some(current) = visible_object(source, current)? {
                            let mut pending = vec![current];
                            let mut added = HashSet::new();
                            while let Some(current) = pending.pop() {
                                ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
                                if !added.insert(current.entry.id) {
                                    continue;
                                }
                                for (parent, _) in &current.links {
                                    if !records.contains_key(parent) && !added.contains(parent) {
                                        pending
                                            .push(source.object(*parent)?.context("父目录不可读")?);
                                    }
                                }
                                deleted_directories.remove(&current.entry.id);
                                records.insert(current.entry.id, current);
                            }
                            continue;
                        }
                    }
                    None if target.reason.is_none() => continue,
                    None => verify_deletion(
                        source,
                        journal.id,
                        page.next as i64,
                        id,
                        target.reason.unwrap_or_default(),
                        &mut later_deletions,
                        cancelled,
                    )?,
                }
                if records
                    .remove(&id)
                    .is_some_and(|r| r.entry.attributes & 16 != 0)
                {
                    deleted_directories.insert(id);
                }
            }
            next = page.next as i64;
        }
        let end = source.journal()?.ok_or(JournalMissing)?;
        ensure!(
            end.id == journal.id && next >= end.first && next <= end.next,
            JournalCursorInvalid
        );
        if next == end.next {
            break;
        }
        ensure!(round < 15, "分区持续变化，本次构建未追平，请稍后重试");
    }
    let replay_time = started.elapsed();
    progress("publishing");
    let started = Instant::now();
    if next != journal.next {
        prune(
            &mut records,
            root,
            &excluded,
            &deleted_directories,
            cancelled,
        )?;
    } else if let Some(error) = initial_error {
        return Err(error);
    }
    let mut objects = Vec::with_capacity(records.len());
    let mut entries = Vec::with_capacity(records.len());
    for (_, current) in records {
        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
        objects.push(object(&current.entry));
        if current.entry.id == root {
            entries.push(path(&current.entry, None, &[92]));
        } else {
            for (parent, name) in current.links {
                entries.push(path(&current.entry, Some(parent), &name));
            }
        }
    }
    Ok((
        Rows {
            objects,
            entries,
            root: directory_key(root),
        },
        Cursor {
            journal: journal.id.to_le_bytes().to_vec(),
            next,
        },
        BuildReport {
            strategy: BuildStrategy::Sequential,
            fallback: None,
            scan: Some(scan),
            stage_time,
            import_time,
            replay_time,
            validate_time: started.elapsed(),
        },
    ))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DirectoryState {
    Visiting,
    Traversable,
    ReparsePoint,
    Hidden,
}

fn prune(
    records: &mut HashMap<u64, NativeObject>,
    root: u64,
    excluded: &[u64],
    deleted: &HashSet<u64>,
    cancelled: &AtomicBool,
) -> Result<()> {
    let root_record = records.get(&root).context("卷根目录缺失")?;
    ensure!(
        root_record.entry.attributes & 16 != 0 && !excluded.contains(&root),
        "卷根目录无效"
    );
    let mut directories = HashMap::new();
    let mut names = HashSet::new();
    for (&id, record) in records.iter() {
        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
        let folder = record.entry.attributes & 16 != 0;
        ensure!(folder || record.entry.size >= 0, "文件大小无效");
        if id == root {
            continue;
        }
        ensure!(!record.links.is_empty(), "文件链接集合为空");
        if folder {
            ensure!(record.links.len() == 1, "目录路径不唯一");
            directories.insert(
                id,
                (record.links[0].0, record.entry.attributes & 0x400 == 0),
            );
        }
        for (parent, name) in &record.links {
            ensure!(!name.is_empty(), "文件名为空");
            ensure!(
                names.insert((*parent, name.as_slice())),
                InitialPathConflict
            );
        }
    }
    drop(names);
    // States distinguish a visible reparse point from a directory we may descend into.
    use DirectoryState::{Hidden, ReparsePoint, Traversable, Visiting};
    let mut states: HashMap<u64, DirectoryState> = deleted.iter().map(|id| (*id, Hidden)).collect();
    states.insert(
        root,
        if root_record.entry.attributes & 0x400 == 0 {
            Traversable
        } else {
            ReparsePoint
        },
    );
    let mut chain = Vec::new();
    for &id in directories.keys() {
        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
        let mut current = id;
        while states.get(&current).is_none_or(|s| *s == Visiting) {
            ensure!(
                states.insert(current, Visiting) != Some(Visiting),
                "目录父链存在循环"
            );
            let &(parent, _) = directories.get(&current).ok_or(InitialPathsIncomplete)?;
            chain.push(current);
            current = parent;
        }
        while let Some(id) = chain.pop() {
            let (parent, traversable) = directories[&id];
            let state = if states[&parent] != Traversable || excluded.contains(&id) {
                Hidden
            } else if traversable {
                Traversable
            } else {
                ReparsePoint
            };
            states.insert(id, state);
        }
    }
    for record in records.values() {
        for (parent, _) in &record.links {
            ensure!(states.contains_key(parent), InitialPathsIncomplete);
        }
    }
    records.retain(|id, record| {
        if *id == root {
            return true;
        }
        if record.entry.attributes & 16 != 0 {
            return states[id] != Hidden;
        }
        record
            .links
            .retain(|(parent, _)| states[parent] == Traversable);
        !record.links.is_empty()
    });
    Ok(())
}
