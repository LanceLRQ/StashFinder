use anyhow::{Context, Result, ensure};
pub use stashfinder_core::index::IndexMode;
use stashfinder_store::volume::{Changes, Cursor, Entry, Object, Volume};
use stashfinder_windows::ntfs::{
    DirectoryEntry, Journal, NativeObject, UsnPage, UsnRecord, VolumeReader,
};
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    sync::atomic::{AtomicBool, Ordering},
};

pub trait Source {
    fn verify(&self) -> Result<()> {
        Ok(())
    }
    fn root_id(&self) -> u64;
    fn journal(&self) -> Result<Option<Journal>>;
    fn create_journal(&self) -> Result<Journal>;
    fn inventory(
        &self,
        cancelled: &AtomicBool,
        consume: &mut dyn FnMut(&[UsnRecord]) -> Result<()>,
    ) -> Result<()>;
    fn directory(
        &self,
        id: u64,
        cancelled: &AtomicBool,
        consume: &mut dyn FnMut(&[DirectoryEntry]) -> Result<()>,
    ) -> Result<()>;
    fn object(&self, id: u64) -> Result<Option<NativeObject>>;
    fn changes(&self, journal: u64, start: i64) -> Result<UsnPage>;
}
impl Source for VolumeReader {
    fn verify(&self) -> Result<()> {
        self.verify()
    }
    fn root_id(&self) -> u64 {
        self.root_id
    }
    fn journal(&self) -> Result<Option<Journal>> {
        self.journal()
    }
    fn create_journal(&self) -> Result<Journal> {
        self.create_journal()
    }
    fn inventory(
        &self,
        c: &AtomicBool,
        f: &mut dyn FnMut(&[UsnRecord]) -> Result<()>,
    ) -> Result<()> {
        self.enumerate_mft(c, f)
    }
    fn directory(
        &self,
        id: u64,
        c: &AtomicBool,
        f: &mut dyn FnMut(&[DirectoryEntry]) -> Result<()>,
    ) -> Result<()> {
        self.directory(id, c, f)
    }
    fn object(&self, id: u64) -> Result<Option<NativeObject>> {
        self.lookup_object(id)
    }
    fn changes(&self, journal: u64, start: i64) -> Result<UsnPage> {
        self.changes(journal, start)
    }
}

fn directory_key(id: u64) -> Vec<u8> {
    let mut key = vec![0];
    key.extend_from_slice(&id.to_le_bytes());
    key
}
fn object(row: &DirectoryEntry) -> Object {
    Object {
        id: row.id.to_le_bytes().to_vec(),
        source: "ntfs64".into(),
        kind: if row.attributes & 16 != 0 {
            "folder"
        } else {
            "file"
        }
        .into(),
        size: (row.attributes & 16 == 0).then_some(row.size),
        created: Some(row.created.to_le_bytes()),
        modified: Some(row.modified.to_le_bytes()),
        attributes: row.attributes,
    }
}
fn path(row: &DirectoryEntry, parent: Option<u64>, name: &[u16]) -> Entry {
    let id = if row.attributes & 16 != 0 {
        directory_key(row.id)
    } else {
        let mut key = vec![1];
        key.extend_from_slice(&parent.unwrap_or(0).to_le_bytes());
        key.extend_from_slice(&row.id.to_le_bytes());
        for c in name {
            key.extend_from_slice(&c.to_le_bytes());
        }
        key
    };
    Entry {
        id,
        object: row.id.to_le_bytes().to_vec(),
        parent: parent.map(directory_key),
        name: name.to_vec(),
    }
}

pub fn build(
    source: &dyn Source,
    db: &mut Volume,
    mode: IndexMode,
    cancelled: &AtomicBool,
    progress: &mut dyn FnMut(&str),
) -> Result<()> {
    ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
    let journal = match mode {
        IndexMode::ScanOnly => None,
        IndexMode::Journal => Some(
            source
                .journal()?
                .context("此分区没有 USN 日志，请选择基础扫描或单独授权创建日志")?,
        ),
        IndexMode::CreateJournal => Some(source.create_journal()?),
    };
    let generation = db.begin_generation()?;
    let result = (|| -> Result<()> {
        if journal.is_some() {
            progress("enumerating");
            source.inventory(cancelled, &mut |records| {
                let objects = records
                    .iter()
                    .filter(|r| r.id & 0x0000ffffffffffff >= 16)
                    .map(|r| Object {
                        id: r.id.to_le_bytes().to_vec(),
                        source: "ntfs64".into(),
                        kind: if r.attributes & 16 != 0 {
                            "folder"
                        } else {
                            "file"
                        }
                        .into(),
                        size: None,
                        created: None,
                        modified: None,
                        attributes: r.attributes,
                    })
                    .collect::<Vec<_>>();
                let pending = objects.iter().map(|o| o.id.clone()).collect::<Vec<_>>();
                db.write_batch(generation, &objects, &[], &pending, None)
            })?;
        }
        progress("scanning");
        let root = source.object(source.root_id())?.context("卷根目录不可读")?;
        db.write_batch(
            generation,
            &[object(&root.entry)],
            &[path(&root.entry, None, &[92])],
            &[],
            None,
        )?;
        let mut queue = VecDeque::from([source.root_id()]);
        let mut visited = HashSet::new();
        while let Some(parent) = queue.pop_front() {
            ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
            if !visited.insert(parent) {
                continue;
            }
            source
                .directory(parent, cancelled, &mut |rows| {
                    let objects = rows.iter().map(object).collect::<Vec<_>>();
                    let entries = rows
                        .iter()
                        .map(|r| path(r, Some(parent), &r.name))
                        .collect::<Vec<_>>();
                    db.write_batch(generation, &objects, &entries, &[], None)?;
                    queue.extend(
                        rows.iter()
                            .filter(|r| r.attributes & 16 != 0 && r.attributes & 0x400 == 0)
                            .map(|r| r.id),
                    );
                    Ok(())
                })
                .with_context(|| format!("目录 {parent:016x} 枚举不完整"))?;
        }
        if let Some(journal) = journal {
            progress("replaying");
            let mut next = journal.next;
            for round in 0..16 {
                ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
                let boundary = source.journal()?.context("USN 日志已失效")?;
                ensure!(
                    boundary.id == journal.id && next >= boundary.first && next <= boundary.next,
                    "USN 日志发生断档，需要重新建库"
                );
                while next < boundary.next {
                    ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
                    let page = source
                        .changes(journal.id, next)
                        .with_context(|| format!("读取 USN 日志失败，起点 {next}"))?;
                    ensure!(
                        page.next > next as u64 && page.next <= i64::MAX as u64,
                        "USN 回放未前进"
                    );
                    let mut ids = BTreeMap::new();
                    for record in page.records {
                        ensure!(record.usn >= next, "USN 记录倒退");
                        *ids.entry(record.id).or_insert(0u32) |= record.reason;
                    }
                    let mut objects = Vec::new();
                    let mut entries = Vec::new();
                    let mut removed = Vec::new();
                    let mut added = HashSet::new();
                    for (id, reason) in ids {
                        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
                        if id & 0x0000ffffffffffff < 16 && id != source.root_id() {
                            continue;
                        }
                        match source.object(id).with_context(|| {
                            format!("回放文件 {id:016x} 失败，原因 {reason:08x}")
                        })? {
                            Some(current) => collect_object(
                                source,
                                db,
                                generation,
                                current,
                                &mut objects,
                                &mut entries,
                                &mut added,
                            )?,
                            None if reason & 0x200 != 0 => removed.push(id.to_le_bytes().to_vec()),
                            None => anyhow::bail!("文件 {id:016x} 未读到，但没有删除证据"),
                        }
                    }
                    next = page.next as i64;
                    db.apply_changes(
                        generation,
                        Changes {
                            objects: &objects,
                            entries: &entries,
                            removed: &removed,
                            pending: &[],
                            cursor: Some(&Cursor {
                                journal: journal.id.to_le_bytes().to_vec(),
                                next,
                            }),
                            replace_links: true,
                        },
                    )?;
                }
                let end = source.journal()?.context("USN 日志已失效")?;
                ensure!(
                    end.id == journal.id && next >= end.first && next <= end.next,
                    "USN 日志发生断档"
                );
                if end.next == next {
                    db.write_batch(
                        generation,
                        &[],
                        &[],
                        &[],
                        Some(&Cursor {
                            journal: journal.id.to_le_bytes().to_vec(),
                            next,
                        }),
                    )?;
                    break;
                }
                ensure!(round < 15, "分区持续变化，本次构建未追平，请稍后重试");
            }
        }
        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
        db.prune_inventory(generation)?;
        db.complete(generation)?;
        stashfinder_store::search::SearchIndex::load_generation(db, generation, false)?;
        source.verify()?;
        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
        match mode {
            IndexMode::ScanOnly => db.publish_snapshot(generation)?,
            _ => db.publish(generation)?,
        }
        Ok(())
    })();
    if let Err(error) = &result {
        db.fail_generation(generation, &format!("{error:#}"))?;
    }
    result
}

fn collect_object(
    source: &dyn Source,
    db: &Volume,
    generation: i64,
    current: NativeObject,
    objects: &mut Vec<Object>,
    entries: &mut Vec<Entry>,
    added: &mut HashSet<u64>,
) -> Result<()> {
    if !added.insert(current.entry.id) {
        return Ok(());
    }
    for (parent, _) in &current.links {
        if !db.has_path(generation, &directory_key(*parent))? && !added.contains(parent) {
            collect_object(
                source,
                db,
                generation,
                source.object(*parent)?.context("父目录不可读")?,
                objects,
                entries,
                added,
            )?;
        }
    }
    if current.entry.id == source.root_id() {
        entries.push(path(&current.entry, None, &[92]));
    } else {
        ensure!(!current.links.is_empty(), "文件链接集合为空，未确认删除");
        for (parent, name) in &current.links {
            entries.push(path(&current.entry, Some(*parent), name));
        }
    }
    objects.push(object(&current.entry));
    Ok(())
}

/// Returns false when the persisted journal boundary already matches the source.
pub fn synchronize(source: &dyn Source, db: &mut Volume, cancelled: &AtomicBool) -> Result<bool> {
    let generation = db.active_generation()?.context("尚无活动索引")?;
    let result = (|| -> Result<bool> {
        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
        let cursor = db
            .cursor(generation)?
            .context("索引没有 USN 游标，需要重建")?;
        let journal_id = u64::from_le_bytes(
            cursor
                .journal
                .as_slice()
                .try_into()
                .context("USN 日志标识无效")?,
        );
        let boundary = source.journal()?.context("USN 日志不可用")?;
        ensure!(
            boundary.id == journal_id
                && cursor.next >= boundary.first
                && cursor.next <= boundary.next,
            "USN 日志发生断档，需要重新建库"
        );
        source.verify()?;
        if cursor.next == boundary.next {
            if db.index_status()?.phase != "ready" {
                db.finish_incremental(generation)?;
                return Ok(true);
            }
            return Ok(false);
        }
        let mut next = cursor.next;
        // A bounded turn yields to other partitions on a continuously busy volume.
        for _ in 0..64 {
            ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
            let page = source.changes(journal_id, next)?;
            ensure!(
                page.next > next as u64 && page.next <= i64::MAX as u64,
                "USN 回放未前进"
            );
            let mut ids = BTreeMap::new();
            for record in page.records {
                ensure!(
                    record.usn >= next && record.usn < page.next as i64,
                    "USN 记录范围无效"
                );
                *ids.entry(record.id).or_insert(0u32) |= record.reason;
            }
            let mut objects = Vec::new();
            let mut entries = Vec::new();
            let mut removed = Vec::new();
            let mut added = HashSet::new();
            for (id, reason) in ids {
                ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
                if id & 0x0000ffffffffffff < 16 && id != source.root_id() {
                    continue;
                }
                match source
                    .object(id)
                    .with_context(|| format!("读取变更对象 {id:016x}"))?
                {
                    Some(current) => collect_object(
                        source,
                        db,
                        generation,
                        current,
                        &mut objects,
                        &mut entries,
                        &mut added,
                    )?,
                    None if reason & 0x200 != 0 => removed.push(id.to_le_bytes().to_vec()),
                    None => anyhow::bail!("文件 {id:016x} 未读到，但没有删除证据"),
                }
            }
            let end = source.journal()?.context("USN 日志已失效")?;
            ensure!(
                end.id == journal_id && next >= end.first && page.next <= end.next as u64,
                "USN 日志发生断档"
            );
            source.verify()?;
            ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
            next = page.next as i64;
            db.apply_incremental(
                generation,
                Changes {
                    objects: &objects,
                    entries: &entries,
                    removed: &removed,
                    pending: &[],
                    cursor: Some(&Cursor {
                        journal: journal_id.to_le_bytes().to_vec(),
                        next,
                    }),
                    replace_links: true,
                },
            )?;
            if next == end.next {
                db.finish_incremental(generation)?;
                return Ok(true);
            }
        }
        Ok(true)
    })();
    if let Err(error) = &result {
        db.mark_stale(generation, &format!("{error:#}"))?;
    }
    result
}
