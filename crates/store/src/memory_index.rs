use crate::volume::{Cursor, Entry, Object, Volume};
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{
    Connection,
    backup::{Backup, StepResult},
    params,
};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

impl Volume {
    /// Build and validate a disposable database before publishing its durable copy.
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
        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
        let (objects, entries) = snapshot_rows(objects, entries, root, cancelled)?;
        progress()?;
        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
        let mut db = Self::create_in_memory("{}")?;
        // This connection never escapes. Check the entire graph and all foreign
        // keys before publication instead of probing a B-tree on every insert.
        db.db.pragma_update(None, "foreign_keys", false)?;
        let generation = db.begin_generation()?;
        {
            let tx = db.db.transaction()?;
            // The validated rows already have final IDs and parents. Avoid copying
            // them through a second path table and rebuilding its lookup indexes.
            tx.execute_batch("DROP INDEX paths_object; DROP INDEX paths_parent;")?;
            {
                let mut insert = tx.prepare("INSERT INTO objects(generation,id,source,kind,size,created,modified,attributes) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)")?;
                for batch in objects.chunks(4096) {
                    ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
                    for object in batch {
                        insert.execute(params![
                            generation,
                            object.id,
                            object.source,
                            object.kind,
                            object.size,
                            object.created.as_ref().map(|v| v.as_slice()),
                            object.modified.as_ref().map(|v| v.as_slice()),
                            object.attributes
                        ])?;
                    }
                    progress()?;
                }
            }
            {
                let mut insert = tx.prepare(
                    "INSERT INTO paths(generation,id,object,parent,name) VALUES(?1,?2,?3,?4,?5)",
                )?;
                let mut name = Vec::new();
                for batch in entries.chunks(4096) {
                    ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
                    for entry in batch {
                        name.clear();
                        name.extend(entry.name.iter().flat_map(|unit| unit.to_le_bytes()));
                        insert.execute(params![
                            generation,
                            entry.id,
                            entry.object,
                            entry.parent,
                            name
                        ])?;
                    }
                    progress()?;
                }
            }
            ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
            tx.execute_batch(
                "CREATE INDEX paths_object ON paths(generation,object);
                CREATE INDEX paths_parent ON paths(generation,parent);",
            )?;
            tx.commit()?;
        }
        db.write_batch(generation, &[], &[], &[], Some(cursor))?;
        {
            let mut statement = db.db.prepare("PRAGMA foreign_key_check")?;
            ensure!(
                statement.query([])?.next()?.is_none(),
                "内存索引外键校验失败"
            );
        }
        db.db.pragma_update(None, "foreign_keys", true)?;
        db.complete(generation)?;
        db.publish(generation)?;
        db.backup_completed_index(path, cancelled, progress)
    }

    /// Materialize a committed in-memory catalog under a new path.
    pub fn backup_completed_index(
        &self,
        path: &Path,
        cancelled: &AtomicBool,
        progress: &mut dyn FnMut() -> Result<()>,
    ) -> Result<()> {
        ensure!(
            self.db.path().is_none_or(str::is_empty),
            "索引库必须在内存中"
        );
        ensure!(!path.try_exists()?, "目标索引路径已存在");
        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
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
        staging.as_file().sync_all()?;
        ensure!(!cancelled.load(Ordering::Relaxed), "索引已取消");
        staging.persist_noclobber(path)?;
        Ok(())
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
    for object in objects {
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
            object_ids.insert(object.id.as_slice(), object).is_none(),
            "文件对象重复"
        );
    }
    let mut ids = HashMap::with_capacity(entries.len());
    let mut names = HashSet::with_capacity(entries.len());
    let mut root_index = None;
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
        let object = object_ids
            .get(entry.object.as_slice())
            .context("初始路径缺少文件对象")?;
        if entry.parent.is_none() {
            ensure!(
                entry.id == root && object.kind == "folder" && root_index.replace(index).is_none(),
                "初始路径根目录不唯一或无效"
            );
        }
        ensure!(
            names.insert((entry.parent.as_deref(), entry.name.as_slice())),
            crate::initial::InitialPathConflict
        );
    }
    drop(names);
    let root_index = root_index.context("初始路径根目录缺失")?;
    let mut parents = Vec::with_capacity(entries.len());
    for entry in entries {
        let parent = entry
            .parent
            .as_deref()
            .map(|id| {
                let index = *ids.get(id).ok_or(crate::initial::InitialPathsIncomplete)?;
                ensure!(
                    object_ids[entries[index].object.as_slice()].kind == "folder",
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
    states[root_index] =
        if object_ids[entries[root_index].object.as_slice()].attributes & 0x400 == 0 {
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
            let object = object_ids[entries[index].object.as_slice()];
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
    let mut used = HashSet::with_capacity(objects.len());
    let mut visible = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        if parents[index].is_none_or(|parent| states[parent] == 3) {
            used.insert(entry.object.as_slice());
            visible.push(entry);
        }
    }
    let mut objects: Vec<_> = objects
        .iter()
        .filter(|object| used.contains(object.id.as_slice()))
        .collect();
    objects.sort_unstable_by(|a, b| a.id.cmp(&b.id));
    visible.sort_unstable_by(|a, b| a.id.cmp(&b.id));
    Ok((objects, visible))
}
