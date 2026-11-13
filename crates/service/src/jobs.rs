use anyhow::{Result, ensure};
use stashfinder_core::{
    MAX_FRAME,
    assets::{Asset, Snapshot},
    index::*,
    search::{Query, QueryOptions},
};
use stashfinder_store::{
    catalog::Catalog,
    search::{
        SearchIndex,
        query::{ResultSet, Source},
    },
};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
};
const MAX_SEARCH_SNAPSHOTS: usize = 64;

struct SearchJob {
    id: u64,
    query: Query,
    options: QueryOptions,
    assets: Vec<Asset>,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
struct SearchState {
    cancel: Arc<AtomicBool>,
    partitions: Vec<String>,
    done: bool,
    result: Option<Arc<ResultSet>>,
    error: Option<String>,
    unavailable: Vec<String>,
}
#[derive(Default)]
struct Mailbox {
    jobs: VecDeque<SearchJob>,
    searches: HashMap<u64, SearchState>,
    stop: bool,
}
pub struct Indexes {
    selection: Mutex<crate::file_operations::Selection>,
    pub runner: crate::supervisor::Runner,
    mailbox: Arc<(Mutex<Mailbox>, Condvar)>,
    sequence: AtomicU64,
}
impl Indexes {
    pub fn start(root: PathBuf) -> Self {
        let selection = Mutex::new(crate::file_operations::Selection::new(root.clone()));
        let runner = crate::supervisor::Runner::start(root.clone());
        let caches = runner.caches();
        let states = runner.states();
        let mailbox = Arc::new((Mutex::new(Mailbox::default()), Condvar::new()));
        let work_mailbox = mailbox.clone();
        thread::spawn(move || {
            loop {
                let job = {
                    let (lock, wake) = &*work_mailbox;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    while state.jobs.is_empty() && !state.stop {
                        state = wake.wait(state).unwrap_or_else(|e| e.into_inner());
                    }
                    if state.stop {
                        return;
                    }
                    state.jobs.pop_front().expect("job checked")
                };
                let mut unavailable = Vec::new();
                let result = (|| -> Result<ResultSet> {
                    let catalog = Catalog::read(&root)?;
                    let mut sources = Vec::new();
                    for asset in job.assets {
                        ensure!(!job.cancel.load(Ordering::Relaxed), "搜索已取消");
                        let partition = &asset.registration.partition_id;
                        let index = (|| -> Result<Option<Arc<SearchIndex>>> {
                            if asset.error.is_some() {
                                catalog.health(&asset.registration.filesystem_id)?;
                            }
                            if let Some(index) = caches
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .get(partition)
                                .cloned()
                            {
                                return Ok(Some(index));
                            }
                            let row = catalog.get(partition)?;
                            ensure!(
                                row.filesystem_id == asset.registration.filesystem_id,
                                "搜索分区的文件系统已变化"
                            );
                            let db = catalog.open_volume(&row)?;
                            let index =
                                SearchIndex::load_cancelled(&db, &job.cancel)?.map(Arc::new);
                            Ok(index.map(|index| {
                                caches
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .entry(partition.clone())
                                    .or_insert(index)
                                    .clone()
                            }))
                        })();
                        match index {
                            Ok(Some(index)) => {
                                let needs_validation = states
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .get(partition)
                                    .is_some_and(|s| s.phase != "ready");
                                sources.push(Source {
                                    id: partition.clone(),
                                    name: asset.registration.name.clone(),
                                    filesystem: asset.registration.filesystem_id.clone(),
                                    connection: asset.connection.clone(),
                                    mounts: if asset.connection == "online" {
                                        asset.mounts.clone()
                                    } else {
                                        vec![]
                                    },
                                    index,
                                    needs_validation,
                                });
                            }
                            Ok(None) => unavailable
                                .push(format!("{}：尚无可搜索索引", asset.registration.name)),
                            Err(e) => {
                                unavailable.push(format!("{}：{e:#}", asset.registration.name))
                            }
                        }
                    }
                    ResultSet::run(sources, &job.query, &job.options, &job.cancel)
                })();
                let mut state = work_mailbox.0.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(current) = state.searches.get_mut(&job.id) {
                    current.done = true;
                    match result {
                        Ok(result) => current.result = Some(Arc::new(result)),
                        Err(error) => current.error = Some(format!("{error:#}")),
                    }
                    current.unavailable = unavailable;
                }
            }
        });
        Self {
            selection,
            runner,
            mailbox,
            sequence: AtomicU64::new(1),
        }
    }
    pub fn scan(&self, partition: &str, mode: IndexMode, snapshot: &Snapshot) -> Result<()> {
        self.runner.scan(partition, mode, snapshot).map(|_| ())
    }
    pub fn selection_begin(
        &self,
        origin: stashfinder_core::file_operations::SelectionOrigin,
        snapshot: &Snapshot,
    ) -> Result<u64> {
        self.begin_selection(origin, snapshot, false)
    }
    pub fn selection_begin_records(
        &self,
        origin: stashfinder_core::file_operations::SelectionOrigin,
        snapshot: &Snapshot,
    ) -> Result<u64> {
        self.begin_selection(origin, snapshot, true)
    }
    fn begin_selection(
        &self,
        origin: stashfinder_core::file_operations::SelectionOrigin,
        snapshot: &Snapshot,
        records_only: bool,
    ) -> Result<u64> {
        let query = if let stashfinder_core::file_operations::SelectionOrigin::Search {
            id, ..
        } = &origin
        {
            let mailbox = self.mailbox.0.lock().unwrap_or_else(|e| e.into_inner());
            let state = mailbox
                .searches
                .get(id)
                .filter(|state| state.done)
                .ok_or_else(|| anyhow::anyhow!("搜索尚未完成或已经过期"))?;
            Some(
                state
                    .result
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("搜索结果不可用"))?,
            )
        } else {
            None
        };
        let id = self.sequence.fetch_add(1, Ordering::Relaxed);
        self.selection
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .begin(id, origin, query, snapshot, records_only)
    }
    pub fn selection_add(
        &self,
        id: u64,
        chunk: stashfinder_core::file_operations::SelectionChunk,
    ) -> Result<()> {
        self.selection
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .add(id, chunk)
    }
    pub fn selection_seal(&self, id: u64) -> Result<usize> {
        self.selection
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .seal(id)
    }
    pub fn selection_page(
        &self,
        id: u64,
        offset: usize,
        snapshot: &Snapshot,
    ) -> Result<stashfinder_core::file_operations::SelectionPage> {
        self.selection
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .page(id, offset, snapshot)
    }
    pub fn selection_cancel(&self, id: u64) {
        self.selection
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cancel(id);
    }
    pub fn selection_destination(
        &self,
        reference: &stashfinder_core::file_operations::EntryRef,
        snapshot: &Snapshot,
    ) -> Result<stashfinder_core::file_operations::PreparedTarget> {
        self.selection
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .destination(reference, snapshot)
    }
    pub fn selection_inspect(
        &self,
        reference: &stashfinder_core::file_operations::EntryRef,
        snapshot: &Snapshot,
    ) -> Result<stashfinder_core::file_operations::VisualRecord> {
        self.selection
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .inspect(reference, snapshot)
    }
    pub fn status(&self, partition: &str) -> IndexStatus {
        self.runner.status(partition)
    }
    pub fn observe(&self, snapshot: Snapshot) {
        if !snapshot.busy && snapshot.error.is_none() {
            let mut mailbox = self.mailbox.0.lock().unwrap_or_else(|e| e.into_inner());
            mailbox.searches.retain(|_, state| {
                let registered = state.partitions.iter().all(|id| {
                    snapshot
                        .assets
                        .iter()
                        .any(|a| a.registration.partition_id == *id)
                });
                if !registered {
                    state.cancel.store(true, Ordering::Release);
                }
                registered
            });
            mailbox
                .jobs
                .retain(|job| !job.cancel.load(Ordering::Acquire));
        }
        self.runner.observe(snapshot);
    }
    pub fn search(&self, request: SearchRequest, snapshot: &Snapshot) -> Result<u64> {
        ensure!(
            snapshot.error.is_none(),
            "资产目录不可用：{}",
            snapshot.error.as_deref().unwrap_or_default()
        );
        ensure!(
            !snapshot.busy || !snapshot.assets.is_empty(),
            "正在读取资产目录，请稍后搜索"
        );
        let mut assets = snapshot.assets.clone();
        let query = Query::compile(&request.text, request.names_only, &request.options)?;
        ensure!(request.partitions.len() <= 4096, "分区筛选过多");
        ensure!(
            request
                .partitions
                .iter()
                .all(|id| assets.iter().any(|a| a.registration.partition_id == *id)),
            "搜索分区未登记"
        );
        if let Some(scope) = &request.options.scope {
            ensure!(
                assets
                    .iter()
                    .any(|a| a.registration.partition_id == scope.partition_id),
                "搜索目录分区未登记"
            );
        }
        assets.retain(|a| {
            (request.partitions.is_empty()
                || request.partitions.contains(&a.registration.partition_id))
                && (request.options.include_history || a.registration.state != "historical")
                && (!request.options.online_only || a.connection == "online")
        });
        assets.sort_by(|a, b| {
            a.registration
                .partition_id
                .cmp(&b.registration.partition_id)
        });
        let id = self.sequence.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        let mut mailbox = self.mailbox.0.lock().unwrap_or_else(|e| e.into_inner());
        ensure!(
            mailbox.searches.len() < MAX_SEARCH_SNAPSHOTS,
            "搜索快照过多，请关闭不需要的搜索标签页"
        );
        mailbox.searches.insert(
            id,
            SearchState {
                cancel: cancel.clone(),
                partitions: assets
                    .iter()
                    .map(|a| a.registration.partition_id.clone())
                    .collect(),
                ..Default::default()
            },
        );
        mailbox.jobs.push_back(SearchJob {
            id,
            query,
            options: *request.options,
            assets,
            cancel,
        });
        self.mailbox.1.notify_one();
        Ok(id)
    }
    pub fn cancel_search(&self, id: u64) {
        let mut mailbox = self.mailbox.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = mailbox.searches.remove(&id) {
            state.cancel.store(true, Ordering::Release);
            mailbox.jobs.retain(|job| job.id != id);
        }
    }
    pub fn page(&self, id: u64, offset: usize) -> Result<SearchPage> {
        let mut page = self.batch(id, offset, 1)?;
        page.row = page.rows.pop();
        Ok(page)
    }
    pub fn batch(&self, id: u64, offset: usize, limit: usize) -> Result<SearchPage> {
        let mailbox = self.mailbox.0.lock().unwrap_or_else(|e| e.into_inner());
        let state = mailbox
            .searches
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("搜索请求已过期"))?;
        ensure!((1..=100).contains(&limit), "搜索页大小无效");
        let result = state.result.clone();
        let mut page = SearchPage {
            id,
            done: state.done,
            row: None,
            rows: vec![],
            total: result.as_ref().map_or(0, |r| r.len()),
            groups: result.as_ref().map_or(0, |r| r.groups),
            missing: result.as_ref().map_or(0, |r| r.missing),
            next: None,
            more: false,
            error: state.error.clone(),
            unavailable: vec![],
        };
        let mut bytes = 0;
        for value in &state.unavailable {
            let value = value.chars().take(512).collect::<String>();
            if bytes + value.len() > 4096 {
                page.unavailable
                    .push("更多分区未能参与搜索，请在概览检查".into());
                break;
            }
            bytes += value.len();
            page.unavailable.push(value);
        }
        drop(mailbox);
        if let Some(result) = result {
            let rows = result.page(offset, limit)?;
            let mut size = serde_json::to_vec(&page)?.len();
            for row in rows {
                let bytes = serde_json::to_vec(&row)?.len();
                if size + bytes + 1 > MAX_FRAME - 256 {
                    ensure!(!page.rows.is_empty(), "搜索条目超过单帧上限");
                    break;
                }
                size += bytes + 1;
                page.rows.push(row);
            }
            page.next = (offset + page.rows.len() < page.total).then_some(offset + page.rows.len());
            page.more = page.next.is_some();
        }
        Ok(page)
    }
}
impl Drop for Indexes {
    fn drop(&mut self) {
        let mut mailbox = self.mailbox.0.lock().unwrap_or_else(|e| e.into_inner());
        mailbox.stop = true;
        for state in mailbox.searches.values() {
            state.cancel.store(true, Ordering::Release);
        }
        self.mailbox.1.notify_one();
    }
}
