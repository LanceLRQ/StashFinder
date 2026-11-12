use crate::{
    index::{self, Source},
    scan,
};
use anyhow::{Context, Result, ensure};
use stashfinder_core::{
    MAX_FRAME,
    assets::{Asset, IdentityMatch, Snapshot},
    index::{IndexMode, IndexStatus},
    tasks::{PartitionIndex, Task, TaskOverviewPage, TaskOverviewRow},
};
use stashfinder_store::{catalog::Catalog, search::SearchIndex, tasks::Tasks, volume::Volume};
use stashfinder_windows::{ntfs::VolumeReader, volumes};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub(crate) type Cache = Arc<Mutex<HashMap<String, Arc<SearchIndex>>>>;
type SourceFactory = dyn Fn(&Asset) -> Result<Box<dyn Source>> + Send + 'static;
type ScanFactory = dyn Fn(&Asset) -> Result<Box<dyn scan::Source>> + Send + 'static;

pub struct Runner {
    shared: Arc<Shared>,
    worker: Option<thread::JoinHandle<()>>,
}
struct Shared {
    root: PathBuf,
    tasks: Mutex<Result<Tasks, String>>,
    snapshot: Mutex<Snapshot>,
    pub states: Arc<Mutex<HashMap<String, IndexStatus>>>,
    pub caches: Cache,
    active: Mutex<Option<(String, Arc<AtomicBool>)>>,
    stop: AtomicBool,
    ready: AtomicBool,
}

fn eligible(asset: &Asset) -> bool {
    asset.registration.state == "ready"
        && asset.connection == "online"
        && asset.error.is_none()
        && matches!(
            asset
                .registration
                .observation
                .identity
                .filesystem
                .as_deref(),
            Some("NTFS" | "exFAT" | "FAT32")
        )
}

fn runtime_directories(root: &std::path::Path) -> Result<Vec<PathBuf>> {
    let mut paths = vec![root.to_path_buf()];
    if let Some(layout) =
        stashfinder_windows::portable::Layout::discover(&std::env::current_exe()?)?
    {
        if std::fs::canonicalize(&layout.index)? == std::fs::canonicalize(root)? {
            paths = vec![layout.root.join("data")];
        }
    } else {
        let owner_file = root.join("owner.sid");
        let owner = if owner_file.try_exists()? {
            std::fs::read_to_string(owner_file)?
        } else {
            stashfinder_windows::ipc::current_user_sid()?
        };
        paths.extend(stashfinder_windows::storage::desktop_data_directories(
            owner.trim(),
        )?);
    }
    Ok(paths)
}

impl Runner {
    pub fn start(root: PathBuf) -> Self {
        let excluded = runtime_directories(&root);
        let scan_excluded = excluded
            .as_ref()
            .map(Clone::clone)
            .map_err(ToString::to_string);
        Self::start_with_sources(
            root,
            move |asset| {
                let found = volumes::discover()?;
                let matching = found
                    .iter()
                    .filter(|o| {
                        asset.registration.observation.identity.compare(&o.identity)
                            == IdentityMatch::Same
                    })
                    .collect::<Vec<_>>();
                ensure!(
                    matching.len() == 1 && matching[0].eligible(),
                    "分区已断开、身份变化或无法唯一识别"
                );
                let mut source = VolumeReader::open(&matching[0].candidate_id)?;
                source.exclude_directories(
                    excluded
                        .as_ref()
                        .map_err(|e| anyhow::anyhow!(e.to_string()))?,
                )?;
                ensure!(
                    asset
                        .registration
                        .observation
                        .identity
                        .serial
                        .as_ref()
                        .is_some_and(|s| s.source == "ntfs64"
                            && s.value == format!("{:016x}", source.volume_serial())),
                    "NTFS 序列号与登记不符"
                );
                Ok(Box::new(source))
            },
            move |asset| {
                scan::Native::open_excluding(
                    asset,
                    scan_excluded
                        .as_ref()
                        .map_err(|e| anyhow::anyhow!(e.clone()))?,
                )
            },
        )
    }
    pub fn start_with_source(
        root: PathBuf,
        source: impl Fn(&Asset) -> Result<Box<dyn Source>> + Send + 'static,
    ) -> Self {
        Self::start_with_sources(root, source, scan::Native::open)
    }
    pub fn start_with_sources(
        root: PathBuf,
        source: impl Fn(&Asset) -> Result<Box<dyn Source>> + Send + 'static,
        scanner: impl Fn(&Asset) -> Result<Box<dyn scan::Source>> + Send + 'static,
    ) -> Self {
        let tasks = Tasks::open(&root)
            .and_then(|mut tasks| {
                tasks.recover()?;
                Ok(tasks)
            })
            .map_err(|e| format!("{e:#}"));
        let shared = Arc::new(Shared {
            root,
            tasks: Mutex::new(tasks),
            snapshot: Mutex::new(Snapshot::default()),
            states: Arc::new(Mutex::new(HashMap::new())),
            caches: Arc::new(Mutex::new(HashMap::new())),
            active: Mutex::new(None),
            stop: AtomicBool::new(false),
            ready: AtomicBool::new(false),
        });
        let state = shared.clone();
        let worker = thread::spawn(move || run(state, Box::new(source), Box::new(scanner)));
        Self {
            shared,
            worker: Some(worker),
        }
    }
    pub(crate) fn caches(&self) -> Cache {
        self.shared.caches.clone()
    }
    pub(crate) fn states(&self) -> Arc<Mutex<HashMap<String, IndexStatus>>> {
        self.shared.states.clone()
    }
    pub fn ready(&self) -> bool {
        self.shared.ready.load(Ordering::Acquire)
    }
    pub fn directory(
        &self,
        request: &stashfinder_core::browse::DirectoryRequest,
    ) -> Result<stashfinder_core::browse::DirectoryPage> {
        let catalog = Catalog::read(&self.shared.root)?;
        let row = catalog.get(&request.partition_id)?;
        ensure!(
            row.filesystem_id == request.filesystem_id
                && matches!(row.state.as_str(), "ready" | "historical"),
            "目录所属文件系统已变化或登记未完成"
        );
        stashfinder_store::browse::read(&catalog.volume_path(&row.filesystem_id)?, request)
    }
    pub fn observe(&self, snapshot: Snapshot) {
        let active = self.shared.active.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self.shared.with_tasks(|tasks| {
            if let Some((id, cancel)) = &*active {
                let task = tasks.get(id)?;
                if snapshot.error.is_some()
                    || !snapshot
                        .assets
                        .iter()
                        .any(|a| a.registration.partition_id == task.partition_id && eligible(a))
                {
                    cancel.store(true, Ordering::Release);
                }
            }
            if !snapshot.busy && snapshot.error.is_none() {
                for task in tasks.list()? {
                    if !matches!(
                        task.state.as_str(),
                        "running"
                            | "queued"
                            | "waiting"
                            | "needs_confirmation"
                            | "failed"
                            | "interrupted"
                    ) {
                        continue;
                    }
                    let asset = snapshot
                        .assets
                        .iter()
                        .find(|a| a.registration.partition_id == task.partition_id);
                    if asset.is_none_or(|a| a.registration.state == "historical") {
                        let reason = if asset.is_some() {
                            "文件系统已归档，不再更新"
                        } else {
                            "分区已移除登记"
                        };
                        tasks.set_state(&task.id, "superseded", Some(reason))?;
                    }
                }
                for id in tasks.paused_partitions()? {
                    if snapshot.assets.iter().all(|a| {
                        a.registration.partition_id != id || a.registration.state == "historical"
                    }) {
                        tasks.set_paused(&id, false)?;
                    }
                }
            }
            Ok(())
        });
        if !snapshot.busy && snapshot.error.is_none() {
            let registered = |id: &String| {
                snapshot
                    .assets
                    .iter()
                    .any(|asset| asset.registration.partition_id == *id)
            };
            self.shared
                .caches
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|id, _| registered(id));
            self.shared
                .states
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|id, _| registered(id));
        }
        *self
            .shared
            .snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = snapshot;
    }
    pub fn tasks(&self) -> Result<Vec<Task>> {
        self.shared.with_tasks(|t| t.list())
    }
    pub fn enabled(&self) -> Result<bool> {
        self.shared.with_tasks(|t| t.enabled())
    }
    pub fn paused(&self) -> Result<Vec<String>> {
        self.shared.with_tasks(|t| t.paused_partitions())
    }
    pub fn page(
        &self,
        version: Option<u64>,
        offset: usize,
    ) -> Result<stashfinder_core::tasks::TaskPage> {
        self.shared.with_tasks(|t| {
            let revision = t.revision()?;
            ensure!(
                version.is_none_or(|v| v == revision),
                "任务快照已更新，请重试"
            );
            let mut rows = t.list()?;
            let states = self.shared.states.lock().unwrap_or_else(|e| e.into_inner());
            for row in &mut rows {
                if row.state == "running" {
                    row.phase = states.get(&row.partition_id).map(|s| s.phase.clone());
                }
            }
            ensure!(offset == 0 || offset < rows.len(), "分页位置无效");
            Ok(stashfinder_core::tasks::TaskPage {
                version: revision,
                row: rows.get(offset).cloned(),
                next: (offset + 1 < rows.len()).then_some(offset + 1),
                automatic: t.enabled()?,
                paused: t.paused_partitions()?,
            })
        })
    }
    pub fn set_enabled(&self, value: bool) -> Result<()> {
        self.shared.with_tasks(|t| t.set_enabled(value))
    }
    pub fn overview(&self, version: Option<&str>, offset: usize) -> Result<TaskOverviewPage> {
        self.shared.with_tasks(|tasks| {
            let mut partitions = BTreeMap::new();
            for (id, index) in self
                .shared
                .caches
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
            {
                partitions.insert(
                    id.clone(),
                    PartitionIndex {
                        partition_id: id.clone(),
                        paused: false,
                        index_version: Some(format!("{}:{}", index.generation, index.revision)),
                        indexed_at: index.indexed_at,
                    },
                );
            }
            for id in tasks.paused_partitions()? {
                partitions
                    .entry(id.clone())
                    .or_insert(PartitionIndex {
                        partition_id: id,
                        paused: false,
                        index_version: None,
                        indexed_at: None,
                    })
                    .paused = true;
            }
            let mut hash = DefaultHasher::new();
            partitions.hash(&mut hash);
            let current_version = format!("{:x}:{:x}", tasks.revision()?, hash.finish());
            ensure!(
                version.is_none_or(|v| v == current_version),
                "任务快照已更新，请重试"
            );
            let mut entries = tasks.list()?;
            {
                let states = self.shared.states.lock().unwrap_or_else(|e| e.into_inner());
                for task in &mut entries {
                    if task.state == "running" {
                        task.phase = states.get(&task.partition_id).map(|s| s.phase.clone());
                    }
                }
            }
            let total = partitions.len() + entries.len();
            ensure!(offset == 0 || offset < total, "分页位置无效");
            let mut rows = Vec::new();
            let mut size = 0;
            for mut row in partitions
                .into_values()
                .map(TaskOverviewRow::Partition)
                .chain(entries.into_iter().map(TaskOverviewRow::Task))
                .skip(offset)
            {
                let mut bytes = serde_json::to_vec(&row)?.len() + 1;
                if bytes > MAX_FRAME - 512
                    && let TaskOverviewRow::Task(task) = &mut row
                    && let Some(error) = &mut task.error
                {
                    error.truncate(error.floor_char_boundary(2048));
                    error.push('…');
                    bytes = serde_json::to_vec(&row)?.len() + 1;
                }
                size += bytes;
                if size > MAX_FRAME - 512 {
                    break;
                }
                rows.push(row);
            }
            ensure!(
                offset == total || !rows.is_empty(),
                "任务信息超过单页传输上限"
            );
            let next = offset + rows.len();
            Ok(TaskOverviewPage {
                version: current_version,
                rows,
                next: (next < total).then_some(next),
                automatic: tasks.enabled()?,
            })
        })
    }
    pub fn status(&self, id: &str) -> IndexStatus {
        let mut status = self
            .shared
            .states
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
            .unwrap_or_else(|| IndexStatus {
                phase: if self.ready() {
                    "not_indexed"
                } else {
                    "loading"
                }
                .into(),
                ..Default::default()
            });
        if let Ok(tasks) = self.tasks()
            && let Some(task) = tasks.iter().find(|t| {
                t.partition_id == id
                    && matches!(
                        t.state.as_str(),
                        "queued" | "running" | "waiting" | "needs_confirmation"
                    )
            })
        {
            if task.state == "needs_confirmation" {
                status.journal_missing = task.kind != "rebuild";
                if task.kind == "rebuild" {
                    status.phase = "needs_rebuild".into();
                    status.error.clone_from(&task.error);
                }
            } else {
                status.task_id = Some(task.id.clone());
                if task.state != "running" {
                    status.phase = task.state.clone();
                }
            }
        }
        status
    }
    pub fn scan(&self, partition: &str, mode: IndexMode, snapshot: &Snapshot) -> Result<String> {
        self.queue_index_task(partition, mode, snapshot, false)
    }
    fn queue_index_task(
        &self,
        partition: &str,
        mode: IndexMode,
        snapshot: &Snapshot,
        incremental: bool,
    ) -> Result<String> {
        ensure!(self.ready(), "正在恢复索引状态");
        let asset = snapshot
            .assets
            .iter()
            .find(|a| a.registration.partition_id == partition)
            .context("分区未登记")?;
        ensure!(eligible(asset), "分区未就绪、离线或身份无法确认");
        ensure!(
            matches!(mode, IndexMode::ScanOnly)
                || asset
                    .registration
                    .observation
                    .identity
                    .filesystem
                    .as_deref()
                    == Some("NTFS"),
            "此文件系统只能使用扫描模式"
        );
        self.observe(snapshot.clone());
        let task = self.shared.with_tasks(|tasks| {
            tasks.set_paused(partition, false)?;
            tasks.set_scan_mode(partition, matches!(mode, IndexMode::ScanOnly))?;
            let kind = if incremental {
                "sync"
            } else if matches!(mode, IndexMode::ScanOnly) {
                "scan"
            } else if tasks.rebuild_confirmation_pending(partition)? {
                "rebuild"
            } else {
                "build"
            };
            tasks.enqueue(partition, kind, mode, false)
        })?;
        Ok(task.id)
    }
    pub fn cancel(&self, id: &str) -> Result<()> {
        let active = self.shared.active.lock().unwrap_or_else(|e| e.into_inner());
        self.shared.with_tasks(|t| t.cancel(id))?;
        if let Some((running, cancel)) = &*active
            && running == id
        {
            cancel.store(true, Ordering::Release);
        }
        Ok(())
    }
    pub fn pause(&self, partition: &str, paused: bool, snapshot: &Snapshot) -> Result<()> {
        ensure!(
            snapshot.assets.iter().any(
                |a| a.registration.partition_id == partition && a.registration.state == "ready"
            ),
            "分区未登记或已归档"
        );
        let active = self.shared.active.lock().unwrap_or_else(|e| e.into_inner());
        self.shared.with_tasks(|t| {
            t.set_paused(partition, paused)?;
            if paused
                && let Some((id, cancel)) = &*active
                && t.get(id)?.partition_id == partition
            {
                cancel.store(true, Ordering::Release);
            }
            Ok(())
        })
    }
    pub fn retry(&self, id: &str, snapshot: &Snapshot) -> Result<String> {
        let task = self.shared.with_tasks(|t| t.get(id))?;
        ensure!(
            matches!(
                task.state.as_str(),
                "failed" | "cancelled" | "interrupted" | "needs_confirmation"
            ),
            "当前任务不能重试"
        );
        ensure!(
            task.kind != "rebuild" || task.state != "needs_confirmation",
            "请确认重建索引"
        );
        // A previous explicit grant never authorizes recreating a deleted journal.
        let mode = if matches!(task.mode, IndexMode::CreateJournal) {
            IndexMode::Journal
        } else {
            task.mode
        };
        self.queue_index_task(&task.partition_id, mode, snapshot, task.kind == "sync")
    }
}
impl Drop for Runner {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some((_, cancel)) = &*self.shared.active.lock().unwrap_or_else(|e| e.into_inner()) {
            cancel.store(true, Ordering::Release);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Shared {
    fn with_tasks<T>(&self, f: impl FnOnce(&mut Tasks) -> Result<T>) -> Result<T> {
        let mut guard = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        f(guard
            .as_mut()
            .map_err(|e| anyhow::anyhow!("任务库不可用：{e}"))?)
    }
    fn load(&self, partition: &str) -> Result<Volume> {
        let catalog = Catalog::read(&self.root)?;
        let row = catalog.get(partition)?;
        catalog.health(&row.filesystem_id)?;
        Volume::open(&catalog.volume_path(&row.filesystem_id)?)
    }
    fn publish(&self, partition: &str, db: &Volume) -> Result<()> {
        let status = db.index_status()?;
        if let Some(index) = SearchIndex::load(db)? {
            self.caches
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(partition.into(), Arc::new(index));
        }
        self.states
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(partition.into(), status);
        Ok(())
    }
}

fn run(shared: Arc<Shared>, factory: Box<SourceFactory>, scan_factory: Box<ScanFactory>) {
    if let Ok(catalog) = Catalog::read(&shared.root)
        && let Ok(rows) = catalog.list()
    {
        for row in rows {
            let result = (|| -> Result<()> {
                let mut db = shared.load(&row.partition_id)?;
                let status = db.index_status()?;
                if status.phase == "initial_paths" {
                    db.discard_initial(
                        status.generation.context("构建代次缺失")?,
                        "上次构建中断，将重新建立",
                    )?;
                } else if status.phase == "building" {
                    db.fail_generation(
                        status.generation.context("构建代次缺失")?,
                        "上次构建中断，将重新建立",
                    )?;
                }
                shared.publish(&row.partition_id, &db)
            })();
            if let Err(e) = result {
                shared
                    .states
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(
                        row.partition_id,
                        IndexStatus {
                            phase: "failed".into(),
                            error: Some(format!("{e:#}")),
                            ..Default::default()
                        },
                    );
            }
        }
    }
    shared.ready.store(true, Ordering::Release);
    let mut failures: HashMap<String, (u32, Instant)> = HashMap::new();
    let mut last_check = Instant::now() - Duration::from_secs(3);
    let mut position = 0usize;
    let mut watchers: HashMap<String, Box<dyn scan::Source>> = HashMap::new();
    let mut observed = HashMap::<String, index::ObservedCursor>::new();
    let mut dirty = HashSet::<String>::new();
    let mut scanned = HashMap::<String, Instant>::new();
    while !shared.stop.load(Ordering::Acquire) {
        let snapshot = shared
            .snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        failures.retain(|id, _| {
            snapshot
                .assets
                .iter()
                .any(|a| a.registration.partition_id == *id && a.connection == "online")
        });
        watchers.retain(|id, _| {
            snapshot.error.is_none()
                && snapshot
                    .assets
                    .iter()
                    .any(|a| a.registration.partition_id == *id && eligible(a))
                && shared
                    .with_tasks(|t| Ok(t.enabled()? && !t.paused(id)?))
                    .unwrap_or(false)
        });
        let selected = shared.with_tasks(|tasks| {
            let enabled = tasks.enabled()?;
            let mut rows = tasks.list()?;
            rows.reverse();
            for task in rows {
                if !matches!(task.state.as_str(), "queued" | "waiting")
                    || (task.automatic && !enabled)
                    || tasks.paused(&task.partition_id)?
                {
                    continue;
                }
                let asset = snapshot
                    .assets
                    .iter()
                    .find(|a| a.registration.partition_id == task.partition_id);
                if snapshot.error.is_some() || asset.is_none_or(|a| !eligible(a)) {
                    if task.state != "waiting" {
                        tasks.set_state(&task.id, "waiting", Some("等待分区接入且身份确认"))?;
                    }
                    continue;
                }
                return Ok(Some((task, asset.expect("checked").clone())));
            }
            Ok(None)
        });
        let mut selected = selected.unwrap_or(None);
        if selected.is_none()
            && last_check.elapsed() >= Duration::from_secs(2)
            && snapshot.error.is_none()
            && !snapshot.busy
        {
            last_check = Instant::now();
            let candidates = snapshot
                .assets
                .iter()
                .filter(|a| eligible(a))
                .collect::<Vec<_>>();
            for step in 0..candidates.len() {
                let asset = candidates[(position + step) % candidates.len()];
                let id = &asset.registration.partition_id;
                if failures.get(id).is_some_and(|(n, at)| {
                    *n >= 3 || at.elapsed() < Duration::from_secs(5u64.pow(*n))
                }) {
                    continue;
                }
                if !shared
                    .with_tasks(|t| Ok(t.enabled()? && !t.paused(id)?))
                    .unwrap_or(false)
                {
                    continue;
                }
                let mut detected_mode = IndexMode::Journal;
                let detected = (|| -> Result<Option<Task>> {
                    let scan_mode = asset
                        .registration
                        .observation
                        .identity
                        .filesystem
                        .as_deref()
                        != Some("NTFS")
                        || shared.with_tasks(|t| t.scan_mode(id))?;
                    if scan_mode {
                        detected_mode = IndexMode::ScanOnly;
                        if !watchers.contains_key(id) {
                            watchers.insert(id.clone(), scan_factory(asset)?);
                            dirty.insert(id.clone());
                            scanned.remove(id);
                        }
                        if watchers.get_mut(id).expect("inserted").changed()? {
                            dirty.insert(id.clone());
                        }
                        if scanned
                            .get(id)
                            .is_some_and(|at| at.elapsed() >= Duration::from_secs(900))
                        {
                            dirty.insert(id.clone());
                        }
                        if dirty.contains(id)
                            && scanned
                                .get(id)
                                .is_none_or(|at| at.elapsed() >= Duration::from_secs(2))
                        {
                            return shared.with_tasks(|t| {
                                t.enqueue(id, "scan", IndexMode::ScanOnly, true).map(Some)
                            });
                        }
                        return Ok(None);
                    }
                    watchers.remove(id);
                    let source = factory(asset)?;
                    let mut db = shared.load(id)?;
                    if let Some(g) = db.active_generation()?
                        && index::exclude_runtime_paths(source.as_ref(), &mut db, g)?
                    {
                        shared.publish(id, &db)?;
                    }
                    let status = db.index_status()?;
                    let journal = source.journal()?;
                    let Some(journal) = journal else {
                        if let Some(g) = status.active_generation {
                            db.mark_stale(g, "USN 日志不可用，等待选择扫描方式或单独授权")?;
                            shared
                                .states
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .insert(id.clone(), db.index_status()?);
                        }
                        return shared.with_tasks(|t| {
                            if t.list()?.iter().any(|r| {
                                r.partition_id == *id
                                    && r.state == "needs_confirmation"
                                    && r.kind != "rebuild"
                            }) {
                                return Ok(None);
                            }
                            let task = t.enqueue(id, "build", IndexMode::Journal, true)?;
                            if !task.automatic {
                                return Ok(None);
                            }
                            t.set_state(
                                &task.id,
                                "needs_confirmation",
                                Some("此分区没有 USN 日志"),
                            )?;
                            Ok(None)
                        });
                    };
                    let cursor = status
                        .active_generation
                        .map(|g| db.cursor(g))
                        .transpose()?
                        .flatten();
                    let pending = shared.with_tasks(|t| t.rebuild_confirmation_pending(id))?;
                    let kind = match cursor {
                        Some(c) if index::cursor_matches_journal(&c, &journal) => {
                            // An unfinished successor does not invalidate the active cursor.
                            let check_changes = !pending
                                && (status.generation != status.active_generation
                                    || status.phase == "ready");
                            if c.next == journal.next
                                && check_changes
                                && let Some(generation) = status.active_generation
                                && !db.has_pending_metadata(generation)?
                            {
                                return Ok(None);
                            }
                            if check_changes
                                && !index::has_relevant_changes(
                                    source.as_ref(),
                                    &db,
                                    observed
                                        .entry(asset.registration.filesystem_id.clone())
                                        .or_default(),
                                )?
                            {
                                return Ok(None);
                            }
                            "sync"
                        }
                        Some(_) => "rebuild",
                        None if pending => return Ok(None),
                        None if status.active_generation.is_some() => "sync",
                        _ => "build",
                    };
                    shared.with_tasks(|t| t.enqueue(id, kind, IndexMode::Journal, true).map(Some))
                })();
                match detected {
                    Ok(Some(task)) => {
                        selected = Some((task, asset.clone()));
                        position = (position + step + 1) % candidates.len();
                        break;
                    }
                    Ok(None) => (),
                    Err(e) => {
                        if matches!(detected_mode, IndexMode::ScanOnly) {
                            watchers.remove(id);
                            dirty.insert(id.clone());
                            scanned.remove(id);
                        }
                        {
                            let mut states =
                                shared.states.lock().unwrap_or_else(|e| e.into_inner());
                            let state = states.entry(id.clone()).or_default();
                            state.phase = "failed".into();
                            state.error = Some(format!("{e:#}"));
                        }
                        let count = failures.get(id).map_or(1, |(n, _)| n + 1);
                        failures.insert(id.clone(), (count, Instant::now()));
                        let _ = shared.with_tasks(|t| {
                            let kind = if matches!(detected_mode, IndexMode::ScanOnly) {
                                "scan"
                            } else {
                                "sync"
                            };
                            let row = t.enqueue(id, kind, detected_mode, true)?;
                            t.set_state(&row.id, "failed", Some(&format!("{e:#}")))
                        });
                    }
                }
            }
        }
        if let Some((task, asset)) = selected {
            let cancel = Arc::new(AtomicBool::new(false));
            let started = {
                let mut active = shared.active.lock().unwrap_or_else(|e| e.into_inner());
                shared
                    .with_tasks(|t| {
                        let current = t.get(&task.id)?;
                        if !matches!(current.state.as_str(), "queued" | "waiting")
                            || t.paused(&task.partition_id)?
                            || (task.automatic && !t.enabled()?)
                        {
                            return Ok(false);
                        }
                        t.set_state(&task.id, "running", None)?;
                        *active = Some((task.id.clone(), cancel.clone()));
                        Ok(true)
                    })
                    .unwrap_or(false)
            };
            if !started {
                continue;
            }
            {
                let mut states = shared.states.lock().unwrap_or_else(|e| e.into_inner());
                let status = states.entry(task.partition_id.clone()).or_default();
                status.phase = if task.kind == "sync" {
                    "syncing"
                } else {
                    "building"
                }
                .into();
                status.error = None;
            }
            let result = (|| -> Result<()> {
                let catalog = Catalog::read(&shared.root)?;
                let current = catalog.get(&task.partition_id)?;
                ensure!(
                    current.state == "ready"
                        && current.filesystem_id == asset.registration.filesystem_id,
                    "登记已变化"
                );
                let mut db = shared.load(&task.partition_id)?;
                let result = if matches!(task.mode, IndexMode::ScanOnly) {
                    let id = &task.partition_id;
                    if !watchers.contains_key(id) {
                        watchers.insert(id.clone(), scan_factory(&asset)?);
                    }
                    let watcher = watchers.get_mut(id).expect("inserted");
                    scanned.insert(id.clone(), Instant::now());
                    shared
                        .states
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .entry(id.clone())
                        .or_default()
                        .phase = "scanning".into();
                    let result = if asset
                        .registration
                        .observation
                        .identity
                        .filesystem
                        .as_deref()
                        == Some("NTFS")
                    {
                        let source = factory(&asset)?;
                        watcher.changed()?;
                        index::build_checked(
                            source.as_ref(),
                            &mut db,
                            IndexMode::ScanOnly,
                            &cancel,
                            &mut |_| {},
                            &mut || {
                                watcher.verify()?;
                                ensure!(!watcher.changed()?, "扫描期间发生变化，需要重新校验");
                                Ok(())
                            },
                        )
                    } else {
                        scan::build(watcher.as_mut(), &mut db, &cancel)
                    };
                    if result.is_ok() {
                        dirty.remove(id);
                    } else {
                        watchers.remove(id);
                        dirty.insert(id.clone());
                    }
                    result
                } else if task.kind == "sync" {
                    let source = factory(&asset)?;
                    {
                        let mut states = shared.states.lock().unwrap_or_else(|e| e.into_inner());
                        states.entry(task.partition_id.clone()).or_default().phase =
                            "syncing".into();
                    }
                    index::synchronize(source.as_ref(), &mut db, &cancel).map(|_| ())
                } else {
                    let source = factory(&asset)?;
                    let mode = if task.automatic && task.kind == "rebuild" {
                        let generation =
                            db.active_generation()?.ok_or(index::JournalCursorInvalid)?;
                        let cursor = db.cursor(generation)?.ok_or(index::JournalCursorInvalid)?;
                        match source.journal()? {
                            Some(journal) if index::cursor_matches_journal(&cursor, &journal) => {
                                shared.publish(&task.partition_id, &db)?;
                                shared.with_tasks(|t| {
                                    if t.get(&task.id)?.state == "running" {
                                        t.set_state(&task.id, "superseded", None)?;
                                    }
                                    Ok(())
                                })?;
                                return Ok(());
                            }
                            Some(_) => {
                                db.mark_stale(
                                    generation,
                                    &index::JournalCursorInvalid.to_string(),
                                )?;
                                Ok(IndexMode::Journal)
                            }
                            None => Err(index::JournalMissing.into()),
                        }
                    } else {
                        Ok(task.mode)
                    };
                    mode.and_then(|mode| {
                        index::build(source.as_ref(), &mut db, mode, &cancel, &mut |phase| {
                            let mut states =
                                shared.states.lock().unwrap_or_else(|e| e.into_inner());
                            let state = states.entry(task.partition_id.clone()).or_default();
                            state.phase = phase.into();
                            state.error = None;
                        })
                    })
                };
                if result
                    .as_ref()
                    .is_err_and(|e| e.is::<index::JournalMissing>())
                    && let Some(g) = db.active_generation()?
                {
                    db.mark_stale(g, "USN 日志不可用，等待选择扫描方式或单独授权")?;
                }
                if task.automatic
                    && task.kind == "rebuild"
                    && let Err(error) = &result
                    && error.is::<index::JournalCursorInvalid>()
                    && let Some(g) = db.active_generation()?
                {
                    db.mark_stale(g, &error.to_string())?;
                }
                shared.publish(&task.partition_id, &db)?;
                result
            })();
            let mut active = shared.active.lock().unwrap_or_else(|e| e.into_inner());
            let _ = shared.with_tasks(|t| {
                if matches!(t.get(&task.id)?.state.as_str(), "cancelled" | "superseded") {
                    return Ok(());
                }
                if cancel.load(Ordering::Acquire) {
                    return t.set_state(&task.id, "waiting", Some("已暂停，从已提交边界恢复"));
                }
                match &result {
                    Ok(()) => {
                        failures.remove(&task.partition_id);
                        let catching_up = shared
                            .states
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .get(&task.partition_id)
                            .is_some_and(|s| s.phase == "syncing");
                        t.set_state(
                            &task.id,
                            if catching_up { "queued" } else { "completed" },
                            None,
                        )
                    }
                    Err(e) if e.is::<index::JournalMissing>() => {
                        failures.remove(&task.partition_id);
                        let kind = if task.kind == "rebuild" {
                            "build"
                        } else {
                            &task.kind
                        };
                        t.require_confirmation(&task.id, kind, &e.to_string())
                    }
                    Err(e)
                        if e.is::<index::JournalCursorInvalid>()
                            && (task.kind == "sync"
                                || (task.automatic && task.kind == "rebuild")) =>
                    {
                        failures.remove(&task.partition_id);
                        t.require_confirmation(&task.id, "rebuild", &e.to_string())
                    }
                    Err(e) => {
                        {
                            let mut states =
                                shared.states.lock().unwrap_or_else(|e| e.into_inner());
                            let status = states.entry(task.partition_id.clone()).or_default();
                            status.phase = "failed".into();
                            status.error = Some(format!("{e:#}"));
                        }
                        let count = failures.get(&task.partition_id).map_or(1, |(n, _)| n + 1);
                        failures.insert(task.partition_id.clone(), (count, Instant::now()));
                        t.set_state(&task.id, "failed", Some(&format!("{e:#}")))
                    }
                }
            });
            *active = None;
        } else {
            thread::sleep(Duration::from_millis(100));
        }
    }
}
