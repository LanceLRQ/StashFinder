//! Isolated first-search path. The candidate is searchable before its durable copy exists.
use super::*;
use std::{collections::HashMap, path::Path};

pub struct DirectIndex {
    pub index: SearchIndex,
    pub cursor: Cursor,
    pub report: BuildReport,
    objects: Vec<Object>,
    entries: Vec<Entry>,
    root: Vec<u8>,
}

impl DirectIndex {
    pub fn persist(
        &self,
        path: &Path,
        cancelled: &AtomicBool,
        progress: &mut dyn FnMut() -> Result<()>,
    ) -> Result<()> {
        Volume::persist_initial_snapshot(
            &self.objects,
            &self.entries,
            &self.root,
            &self.cursor,
            path,
            cancelled,
            progress,
        )
    }
}

pub fn build_direct(
    source: &dyn Source,
    cancelled: &AtomicBool,
    progress: &mut dyn FnMut(&str),
) -> Result<DirectIndex> {
    ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
    source.verify()?;
    let journal = source.journal()?.ok_or(JournalMissing)?;
    let root = source.root_id();
    let excluded = source.excluded_roots();
    let mut records = HashMap::new();
    progress("scanning");
    let scan = source
        .full_inventory(cancelled, &mut |batch| {
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
    prune(
        &mut records,
        root,
        &excluded,
        &deleted_directories,
        cancelled,
    )?;
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
            let mut ids = BTreeMap::<u64, (u32, bool)>::new();
            for record in page.records {
                ensure!(
                    record.usn >= next && record.usn < page.next as i64,
                    "USN 记录范围无效"
                );
                let candidate = ids.entry(record.id).or_default();
                candidate.0 |= record.reason;
                candidate.1 |= records.contains_key(&record.id)
                    || !source.excluded_directory(record.parent)?;
            }
            let mut later_deletions = None;
            for (id, (reason, inspect)) in ids {
                ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
                if !inspect
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
                    None => verify_deletion(
                        source,
                        journal.id,
                        page.next as i64,
                        id,
                        reason,
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
    let index = SearchIndex::from_initial(&objects, &entries, cancelled)?;
    source.verify()?;
    ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
    Ok(DirectIndex {
        index,
        cursor: Cursor {
            journal: journal.id.to_le_bytes().to_vec(),
            next,
        },
        objects,
        entries,
        root: directory_key(root),
        report: BuildReport {
            strategy: BuildStrategy::Sequential,
            fallback: None,
            scan: Some(scan),
            stage_time,
            import_time,
            replay_time,
            validate_time: started.elapsed(),
        },
    })
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
