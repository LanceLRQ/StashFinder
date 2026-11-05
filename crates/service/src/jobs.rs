use anyhow::{Result, ensure};
use stashfinder_core::{
    assets::{Asset, Snapshot},
    index::*,
    search::Query,
};
use stashfinder_store::{catalog::Catalog, search::SearchIndex, volume::Volume};
use std::{
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
};

struct SearchJob {
    id: u64,
    query: Query,
    assets: Vec<Asset>,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
struct SearchState {
    id: u64,
    done: bool,
    rows: Vec<SearchHit>,
    more: bool,
    error: Option<String>,
    unavailable: Vec<String>,
}
#[derive(Default)]
struct Mailbox {
    job: Option<SearchJob>,
    cancel: Option<Arc<AtomicBool>>,
    stop: bool,
}

pub struct Indexes {
    pub runner: crate::supervisor::Runner,
    mailbox: Arc<(Mutex<Mailbox>, Condvar)>,
    search_state: Arc<Mutex<SearchState>>,
    sequence: AtomicU64,
}

impl Indexes {
    pub fn start(root: PathBuf) -> Self {
        let runner = crate::supervisor::Runner::start(root.clone());
        let caches = runner.caches();
        let states = runner.states();
        let mailbox = Arc::new((Mutex::new(Mailbox::default()), Condvar::new()));
        let search_state = Arc::new(Mutex::new(SearchState::default()));
        let work_mailbox = mailbox.clone();
        let work_state = search_state.clone();
        thread::spawn(move || {
            loop {
                let job = {
                    let (lock, wake) = &*work_mailbox;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    while state.job.is_none() && !state.stop {
                        state = wake.wait(state).unwrap_or_else(|e| e.into_inner());
                    }
                    if state.stop {
                        return;
                    }
                    state.job.take().expect("job checked above")
                };
                let mut output = SearchState {
                    id: job.id,
                    done: true,
                    ..Default::default()
                };
                let result = (|| -> Result<()> {
                    let catalog = Catalog::read(&root)?;
                    for asset in job.assets {
                        ensure!(!job.cancel.load(Ordering::Relaxed), "搜索已取消");
                        let partition = &asset.registration.partition_id;
                        let index = (|| -> Result<Option<Arc<SearchIndex>>> {
                            if asset.error.is_some() {
                                // Device access errors do not invalidate a healthy local snapshot.
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
                            catalog.health(&asset.registration.filesystem_id)?;
                            let db = Volume::open(
                                &catalog.volume_path(&asset.registration.filesystem_id)?,
                            )?;
                            Ok(SearchIndex::load(&db)?.map(Arc::new))
                        })();
                        match index {
                            Ok(Some(index)) => {
                                let mounts = if asset.connection == "online" {
                                    asset.mounts.clone()
                                } else {
                                    Vec::new()
                                };
                                let remaining = 200 - output.rows.len();
                                let matches = index.search(
                                    &job.query,
                                    &mounts,
                                    remaining.max(1),
                                    &job.cancel,
                                )?;
                                if remaining == 0 {
                                    if !matches.rows.is_empty() {
                                        output.more = true;
                                    }
                                    continue;
                                }
                                output.more |= matches.more;
                                output.rows.extend(matches.rows.into_iter().map(|file| {
                                    SearchHit {
                                        filesystem_id: asset.registration.filesystem_id.clone(),
                                        partition_id: partition.clone(),
                                        partition_name: asset.registration.name.clone(),
                                        connection: asset.connection.clone(),
                                        mounts: mounts.clone(),
                                        generation: index.generation,
                                        partial: index.partial,
                                        indexed_at: index.indexed_at,
                                        needs_validation: index.needs_validation
                                            || states
                                                .lock()
                                                .unwrap_or_else(|e| e.into_inner())
                                                .get(partition)
                                                .is_some_and(|s| s.phase != "ready"),
                                        file,
                                    }
                                }));
                            }
                            Ok(None) => output
                                .unavailable
                                .push(format!("{}：尚无可搜索索引", asset.registration.name)),
                            Err(error) => output
                                .unavailable
                                .push(format!("{}：{error:#}", asset.registration.name)),
                        }
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    output.error = Some(format!("{error:#}"));
                    output.rows.clear();
                }
                let mut current = work_state.lock().unwrap_or_else(|e| e.into_inner());
                if current.id == job.id {
                    *current = output;
                }
            }
        });
        Self {
            runner,
            mailbox,
            search_state,
            sequence: AtomicU64::new(1),
        }
    }

    pub fn scan(&self, partition: &str, mode: IndexMode, snapshot: &Snapshot) -> Result<()> {
        self.runner.scan(partition, mode, snapshot).map(|_| ())
    }
    pub fn status(&self, partition: &str) -> IndexStatus {
        self.runner.status(partition)
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
        let query = Query::parse(&request.text, request.names_only)?;
        ensure!(request.partitions.len() <= 4096, "分区筛选过多");
        ensure!(
            request
                .partitions
                .iter()
                .all(|id| assets.iter().any(|a| a.registration.partition_id == *id)),
            "搜索分区未登记"
        );
        if !request.partitions.is_empty() {
            assets.retain(|a| request.partitions.contains(&a.registration.partition_id));
        }
        assets.sort_by(|a, b| {
            a.registration
                .partition_id
                .cmp(&b.registration.partition_id)
        });
        let id = self.sequence.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        let mut mailbox = self.mailbox.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(old) = mailbox.cancel.take() {
            old.store(true, Ordering::Release);
        }
        *self.search_state.lock().unwrap_or_else(|e| e.into_inner()) = SearchState {
            id,
            ..Default::default()
        };
        mailbox.cancel = Some(cancel.clone());
        mailbox.job = Some(SearchJob {
            id,
            query,
            assets,
            cancel,
        });
        self.mailbox.1.notify_one();
        Ok(id)
    }
    pub fn cancel_search(&self, id: u64) {
        let current = self
            .search_state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .id;
        if id == current
            && let Some(cancel) = &self
                .mailbox
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .cancel
        {
            cancel.store(true, Ordering::Release);
        }
    }
    pub fn page(&self, id: u64, offset: usize) -> Result<SearchPage> {
        let state = self.search_state.lock().unwrap_or_else(|e| e.into_inner());
        ensure!(id == state.id, "搜索请求已过期");
        ensure!(offset == 0 || offset < state.rows.len(), "搜索分页位置无效");
        Ok(SearchPage {
            id,
            done: state.done,
            row: state.rows.get(offset).cloned(),
            next: (state.done && offset + 1 < state.rows.len()).then_some(offset + 1),
            more: state.more,
            error: state.error.clone(),
            unavailable: state.unavailable.clone(),
        })
    }
}
impl Drop for Indexes {
    fn drop(&mut self) {
        let mut mailbox = self.mailbox.0.lock().unwrap_or_else(|e| e.into_inner());
        mailbox.stop = true;
        if let Some(cancel) = &mailbox.cancel {
            cancel.store(true, Ordering::Release);
        }
        self.mailbox.1.notify_one();
    }
}
