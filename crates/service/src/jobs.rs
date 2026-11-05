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
    volume::Volume,
};
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
    options: QueryOptions,
    assets: Vec<Asset>,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
struct SearchState {
    id: u64,
    done: bool,
    result: Option<Arc<ResultSet>>,
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
                    state.job.take().expect("job checked")
                };
                let mut output = SearchState {
                    id: job.id,
                    done: true,
                    ..Default::default()
                };
                let result = (|| -> Result<()> {
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
                            catalog.health(&asset.registration.filesystem_id)?;
                            let db = Volume::open(
                                &catalog.volume_path(&asset.registration.filesystem_id)?,
                            )?;
                            let index =
                                SearchIndex::load_cancelled(&db, &job.cancel)?.map(Arc::new);
                            if let Some(index) = &index {
                                caches
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .insert(partition.clone(), index.clone());
                            }
                            Ok(index)
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
                            Ok(None) => output
                                .unavailable
                                .push(format!("{}：尚无可搜索索引", asset.registration.name)),
                            Err(e) => output
                                .unavailable
                                .push(format!("{}：{e:#}", asset.registration.name)),
                        }
                    }
                    output.result = Some(Arc::new(ResultSet::run(
                        sources,
                        &job.query,
                        &job.options,
                        &job.cancel,
                    )?));
                    Ok(())
                })();
                if let Err(e) = result {
                    output.error = Some(format!("{e:#}"));
                    output.result = None;
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
            options: *request.options,
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
        let mut page = self.batch(id, offset, 1)?;
        page.row = page.rows.pop();
        Ok(page)
    }
    pub fn batch(&self, id: u64, offset: usize, limit: usize) -> Result<SearchPage> {
        let state = self.search_state.lock().unwrap_or_else(|e| e.into_inner());
        ensure!(id == state.id, "搜索请求已过期");
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
        drop(state);
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
        if let Some(cancel) = &mailbox.cancel {
            cancel.store(true, Ordering::Release);
        }
        self.mailbox.1.notify_one();
    }
}
