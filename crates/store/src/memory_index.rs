#[cfg(feature = "memory-index-bench")]
use crate::volume::Cursor;
use crate::volume::{Entry, InitialSnapshot, Object, Volume};
#[cfg(feature = "memory-index-bench")]
use anyhow::bail;
use anyhow::{Context, Result, ensure};
#[cfg(feature = "memory-index-bench")]
use rusqlite::{
    Connection,
    backup::{Backup, StepResult},
};
use std::{
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicBool, Ordering},
};

#[cfg(feature = "memory-index-bench")]
use std::path::Path;

// Opt-in diagnostics for the indexing benchmark.
struct SnapshotTimer(Option<std::time::Instant>);

impl SnapshotTimer {
    fn start() -> Self {
        Self(
            std::env::var_os("STASHFINDER_SNAPSHOT_TIMINGS")
                .is_some()
                .then(std::time::Instant::now),
        )
    }

    fn finish(&mut self, stage: &str) {
        if let Some(started) = &mut self.0 {
            let micros = started.elapsed().as_micros();
            eprintln!(
                "{}",
                serde_json::json!({"stage":stage, "elapsed_us":micros})
            );
            *started = std::time::Instant::now();
        }
    }
}

#[cfg(feature = "memory-index-bench")]
pub struct StagedBackup<'a> {
    file: tempfile::NamedTempFile,
    target: &'a Path,
}

#[cfg(feature = "memory-index-bench")]
impl StagedBackup<'_> {
    pub fn publish(self, cancelled: &AtomicBool) -> Result<()> {
        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
        let mut timer = SnapshotTimer::start();
        self.file.persist_noclobber(self.target)?;
        timer.finish("publish_file");
        Ok(())
    }
}

impl Volume {
    /// Build and validate a disposable database before publishing its durable copy.
    #[cfg(feature = "memory-index-bench")]
    pub fn persist_initial_snapshot(
        objects: &[Object],
        entries: &[Entry],
        root: &[u8],
        cursor: &Cursor,
        path: &Path,
        cancelled: &AtomicBool,
        progress: &mut dyn FnMut() -> Result<()>,
    ) -> Result<()> {
        ensure!(!path.try_exists()?, "目标索引路径已存在");
        Self::prepare_initial_snapshot(objects, entries, root, cursor, cancelled, progress)?
            .backup_completed_index(path, cancelled, progress)
    }

    /// Build a committed private catalog that can be backed up after the search index is ready.
    #[cfg(feature = "memory-index-bench")]
    pub fn prepare_initial_snapshot(
        objects: &[Object],
        entries: &[Entry],
        root: &[u8],
        cursor: &Cursor,
        cancelled: &AtomicBool,
        progress: &mut dyn FnMut() -> Result<()>,
    ) -> Result<Self> {
        let mut db = Self::create_in_memory("{}")?;
        db.commit_first_snapshot(
            InitialSnapshot {
                objects,
                entries,
                root,
                cursor,
            },
            cancelled,
            progress,
            || Ok(()),
        )?;
        Ok(db)
    }

    /// Commit the first generation in the registered database, preserving its manifest and capacity.
    pub fn commit_first_snapshot<T>(
        &mut self,
        snapshot: InitialSnapshot<'_>,
        cancelled: &AtomicBool,
        progress: &mut dyn FnMut() -> Result<()>,
        verify: impl FnOnce() -> Result<T>,
    ) -> Result<(T, i64, i64, u64)> {
        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
        let mut timer = SnapshotTimer::start();
        let (objects, entries) =
            snapshot_rows(snapshot.objects, snapshot.entries, snapshot.root, cancelled)?;
        progress()?;
        timer.finish("validate_sort");
        // Only this writer disables incremental checks. Validate all foreign keys in the same transaction before commit.
        self.db.pragma_update(None, "cache_spill", false)?;
        self.db.pragma_update(None, "foreign_keys", false)?;
        let result = (|| {
            let tx = self
                .db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            ensure!(
                !tx.query_row("SELECT EXISTS(SELECT 1 FROM generations)", [], |r| r
                    .get::<_, bool>(0))?,
                "首次索引需要空的分区库"
            );
            tx.execute(
                "INSERT INTO generations(started) VALUES(?1)",
                [stashfinder_core::assets::now() as i64],
            )?;
            let generation = tx.last_insert_rowid();
            // The validated rows already have final IDs and parents. Avoid copying
            // them through a second path table and rebuilding its lookup indexes.
            tx.execute_batch("DROP INDEX paths_object; DROP INDEX paths_parent;")?;
            timer.finish("sqlite_setup");
            {
                for batch in objects.chunks(4096) {
                    ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
                    for group in batch.chunks(32) {
                        let values = vec!["(?,?,'ntfs64',?,?,?,?,?)"; group.len()].join(",");
                        let mut insert = tx.prepare_cached(&format!("INSERT INTO objects(generation,id,source,kind,size,created,modified,attributes) VALUES {values}"))?;
                        for (i, object) in group.iter().enumerate() {
                            let slot = i * 7 + 1;
                            insert.raw_bind_parameter(slot, generation)?;
                            insert.raw_bind_parameter(slot + 1, &object.id)?;
                            insert.raw_bind_parameter(slot + 2, &object.kind)?;
                            insert.raw_bind_parameter(slot + 3, object.size)?;
                            insert.raw_bind_parameter(
                                slot + 4,
                                object.created.as_ref().map(|v| v.as_slice()),
                            )?;
                            insert.raw_bind_parameter(
                                slot + 5,
                                object.modified.as_ref().map(|v| v.as_slice()),
                            )?;
                            insert.raw_bind_parameter(slot + 6, object.attributes)?;
                        }
                        insert.raw_execute()?;
                    }
                    progress()?;
                }
            }
            timer.finish("insert_objects");
            {
                let mut name = Vec::new();
                for batch in entries.chunks(4096) {
                    ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
                    for group in batch.chunks(32) {
                        let values = vec!["(?,?,?,?,?)"; group.len()].join(",");
                        let mut insert = tx.prepare_cached(&format!(
                            "INSERT INTO paths(generation,id,object,parent,name) VALUES {values}"
                        ))?;
                        for (i, entry) in group.iter().enumerate() {
                            let slot = i * 5 + 1;
                            name.clear();
                            name.extend(entry.name.iter().flat_map(|unit| unit.to_le_bytes()));
                            insert.raw_bind_parameter(slot, generation)?;
                            insert.raw_bind_parameter(slot + 1, &entry.id)?;
                            insert.raw_bind_parameter(slot + 2, &entry.object)?;
                            insert.raw_bind_parameter(slot + 3, &entry.parent)?;
                            insert.raw_bind_parameter(slot + 4, &name)?;
                        }
                        insert.raw_execute()?;
                    }
                    progress()?;
                }
            }
            ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
            timer.finish("insert_paths");
            tx.execute_batch(
                "CREATE INDEX paths_object ON paths(generation,object);
                CREATE INDEX paths_parent ON paths(generation,parent);",
            )?;
            timer.finish("secondary_indexes_commit");
            tx.execute(
                "UPDATE generations SET complete=1, journal=?2, next_usn=?3,revision=1 WHERE id=?1",
                rusqlite::params![generation, snapshot.cursor.journal, snapshot.cursor.next],
            )?;
            ensure!(
                tx.prepare("PRAGMA foreign_key_check")?
                    .query([])?
                    .next()?
                    .is_none(),
                "索引外键校验失败"
            );
            timer.finish("cursor_foreign_keys");
            let value = verify()?;
            ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
            let (revision, completed) =
                Self::publish_transaction(&tx, generation, "ready", false, Some(1))?;
            tx.commit()?;
            timer.finish("complete_generation");
            Ok((value, generation, revision, completed))
        })();
        self.db.pragma_update(None, "foreign_keys", true)?;
        self.db.pragma_update(None, "cache_spill", true)?;
        result
    }

    /// Materialize a committed in-memory catalog under a new path.
    #[cfg(feature = "memory-index-bench")]
    pub fn backup_completed_index(
        &self,
        path: &Path,
        cancelled: &AtomicBool,
        progress: &mut dyn FnMut() -> Result<()>,
    ) -> Result<()> {
        self.stage_completed_index(path, cancelled, progress)?
            .publish(cancelled)
    }

    /// Flush a private backup. The caller publishes it after all consumers accept the snapshot.
    #[cfg(feature = "memory-index-bench")]
    pub fn stage_completed_index<'a>(
        &self,
        path: &'a Path,
        cancelled: &AtomicBool,
        progress: &mut dyn FnMut() -> Result<()>,
    ) -> Result<StagedBackup<'a>> {
        ensure!(
            self.db.path().is_none_or(str::is_empty),
            "索引库必须在内存中"
        );
        ensure!(!path.try_exists()?, "目标索引路径已存在");
        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
        let mut timer = SnapshotTimer::start();
        let status = self.index_status()?;
        ensure!(
            status.phase == "ready"
                && status.active_generation.is_some()
                && status.generation == status.active_generation,
            "内存索引尚未提交"
        );
        let parent = path.parent().context("目标索引缺少父目录")?;
        let staging = tempfile::Builder::new()
            .prefix(".index-backup-")
            .tempfile_in(parent)?;
        {
            let mut target = Connection::open(staging.path())?;
            target.pragma_update(None, "synchronous", "FULL")?;
            timer.finish("backup_setup");
            let backup = Backup::new(&self.db, &mut target)?;
            loop {
                ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
                match backup.step(4096)? {
                    StepResult::Done => {
                        progress()?;
                        break;
                    }
                    StepResult::More => progress()?,
                    _ => bail!("临时索引库无法独占写入"),
                }
            }
        }
        timer.finish("backup_copy_close");
        staging.as_file().sync_all()?;
        timer.finish("sync_file");
        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
        Ok(StagedBackup {
            file: staging,
            target: path,
        })
    }
}

// Validate the entire input, including hidden subtrees, before filtering reparse
// descendants. Borrow IDs and names so the validation does not copy file metadata.
fn snapshot_rows<'a>(
    objects: &'a [Object],
    entries: &'a [Entry],
    root: &[u8],
    cancelled: &AtomicBool,
) -> Result<(Vec<&'a Object>, Vec<&'a Entry>)> {
    let mut object_ids = HashMap::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
        ensure!(
            !object.id.is_empty()
                && object.source == "ntfs64"
                && object.identity.is_none()
                && matches!(object.kind.as_str(), "file" | "folder")
                && object.created.is_some()
                && object.modified.is_some()
                && (object.kind == "file") == object.size.is_some()
                && object.size.is_none_or(|size| size >= 0),
            "顺序索引对象的元数据不完整"
        );
        ensure!(
            object_ids.insert(object.id.as_slice(), index).is_none(),
            "文件对象重复"
        );
    }
    let mut ids = HashMap::with_capacity(entries.len());
    let mut names = HashSet::with_capacity(entries.len());
    let mut root_index = None;
    let mut entry_objects = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
        ensure!(
            !entry.id.is_empty() && !entry.name.is_empty(),
            "初始路径标识或名称为空"
        );
        ensure!(
            ids.insert(entry.id.as_slice(), index).is_none(),
            "路径标识重复"
        );
        let object = *object_ids
            .get(entry.object.as_slice())
            .context("初始路径缺少文件对象")?;
        entry_objects.push(object);
        if entry.parent.is_none() {
            ensure!(
                entry.id == root
                    && objects[object].kind == "folder"
                    && root_index.replace(index).is_none(),
                "初始路径根目录不唯一或无效"
            );
        }
        ensure!(
            names.insert((entry.parent.as_deref(), entry.name.as_slice())),
            crate::initial::InitialPathConflict
        );
    }
    drop(names);
    drop(object_ids);
    let root_index = root_index.context("初始路径根目录缺失")?;
    let mut parents = Vec::with_capacity(entries.len());
    for entry in entries {
        let parent = entry
            .parent
            .as_deref()
            .map(|id| {
                let index = *ids.get(id).ok_or(crate::initial::InitialPathsIncomplete)?;
                ensure!(
                    objects[entry_objects[index]].kind == "folder",
                    "初始路径父节点不是目录"
                );
                Ok(index)
            })
            .transpose()?;
        parents.push(parent);
    }
    drop(ids);
    // 0: unseen, 1: current chain, 2: cannot descend, 3: can descend.
    let mut states = vec![0u8; entries.len()];
    states[root_index] = if objects[entry_objects[root_index]].attributes & 0x400 == 0 {
        3
    } else {
        2
    };
    let mut chain = Vec::new();
    for index in 0..entries.len() {
        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
        let mut current = index;
        while states[current] < 2 {
            ensure!(states[current] != 1, "初始路径父链存在循环");
            states[current] = 1;
            chain.push(current);
            current = parents[current].context("初始路径根目录无效")?;
        }
        while let Some(index) = chain.pop() {
            let object = &objects[entry_objects[index]];
            states[index] = if states[parents[index].expect("non-root path")] == 3
                && object.kind == "folder"
                && object.attributes & 0x400 == 0
            {
                3
            } else {
                2
            };
        }
    }
    let mut used = vec![false; objects.len()];
    let mut visible = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        if parents[index].is_none_or(|parent| states[parent] == 3) {
            used[entry_objects[index]] = true;
            visible.push(entry);
        }
    }
    let mut objects: Vec<_> = objects
        .iter()
        .zip(used)
        .filter_map(|(object, visible)| visible.then_some(object))
        .collect();
    objects.sort_unstable_by(|a, b| a.id.cmp(&b.id));
    visible.sort_unstable_by(|a, b| a.id.cmp(&b.id));
    Ok((objects, visible))
}
