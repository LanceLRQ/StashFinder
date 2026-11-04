use anyhow::{Context, Result, ensure};
use stashfinder_core::{
    assets::{Asset, IdentityMatch, Snapshot},
    index::*,
    search::Query,
};
use stashfinder_store::{catalog::Catalog, search::SearchIndex, volume::Volume};
use stashfinder_windows::{ntfs::VolumeReader, volumes};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
    thread,
};

type Cache = Arc<Mutex<HashMap<String, Arc<SearchIndex>>>>;
struct Scan {
    asset: Asset,
    mode: IndexMode,
}
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
    sender: SyncSender<Scan>,
    busy: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    states: Arc<Mutex<HashMap<String, IndexStatus>>>,
    mailbox: Arc<(Mutex<Mailbox>, Condvar)>,
    search_state: Arc<Mutex<SearchState>>,
    sequence: AtomicU64,
}

impl Indexes {
    pub fn start(root: PathBuf) -> Self {
        let states = Arc::new(Mutex::new(HashMap::new()));
        let caches: Cache = Arc::new(Mutex::new(HashMap::new()));
        let busy = Arc::new(AtomicBool::new(true));
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = sync_channel::<Scan>(1);
        let worker_root = root.clone();
        let worker_states = states.clone();
        let worker_caches = caches.clone();
        let worker_busy = busy.clone();
        let worker_cancel = cancel.clone();
        thread::spawn(move || {
            if let Ok(catalog) = Catalog::open(&worker_root)
                && let Ok(rows) = catalog.list()
            {
                for row in rows {
                    let loaded = (|| -> Result<IndexStatus> {
                        catalog.health(&row.filesystem_id)?;
                        let db = Volume::open(&catalog.volume_path(&row.filesystem_id)?)?;
                        let status = db.index_status()?;
                        if status.phase == "building" {
                            db.fail_generation(
                                status.generation.context("构建代次缺失")?,
                                "上次索引中断，请重新建立索引",
                            )?;
                        }
                        if let Some(index) = SearchIndex::load(&db)? {
                            worker_caches
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .insert(row.partition_id.clone(), Arc::new(index));
                        }
                        db.index_status()
                    })();
                    worker_states
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(
                            row.partition_id,
                            loaded.unwrap_or_else(|e| IndexStatus {
                                phase: "failed".into(),
                                error: Some(format!("{e:#}")),
                                ..Default::default()
                            }),
                        );
                }
            }
            worker_busy.store(false, Ordering::Release);
            while let Ok(job) = receiver.recv() {
                let partition = job.asset.registration.partition_id.clone();
                let mut completed_status = None;
                let result = (|| -> Result<()> {
                    let mut catalog = Catalog::open(&worker_root)?;
                    let registered = catalog.get(&partition)?;
                    ensure!(
                        registered.filesystem_id == job.asset.registration.filesystem_id,
                        "登记已变化"
                    );
                    catalog.health(&registered.filesystem_id)?;
                    let found = volumes::discover()?;
                    let matching = found
                        .iter()
                        .filter(|o| {
                            registered.observation.identity.compare(&o.identity)
                                == IdentityMatch::Same
                        })
                        .collect::<Vec<_>>();
                    ensure!(
                        matching.len() == 1 && matching[0].eligible(),
                        "分区已断开、身份变化或无法唯一识别"
                    );
                    let current = matching[0];
                    let source = VolumeReader::open(&current.candidate_id)?;
                    ensure!(
                        registered
                            .observation
                            .identity
                            .serial
                            .as_ref()
                            .is_some_and(|s| s.source == "ntfs64"
                                && s.value == format!("{:016x}", source.volume_serial())),
                        "NTFS 序列号与登记不符"
                    );
                    let mut db = Volume::open(&catalog.volume_path(&registered.filesystem_id)?)?;
                    let built = crate::index::build(
                        &source,
                        &mut db,
                        job.mode,
                        &worker_cancel,
                        &mut |phase| {
                            let mut states =
                                worker_states.lock().unwrap_or_else(|e| e.into_inner());
                            let state = states.entry(partition.clone()).or_default();
                            state.phase = phase.into();
                            state.error = None;
                        },
                    );
                    completed_status = Some(db.index_status()?);
                    if let Some(index) = SearchIndex::load(&db)? {
                        worker_caches
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(partition.clone(), Arc::new(index));
                    }
                    built?;
                    let after = volumes::probe(&current.candidate_id);
                    ensure!(
                        after.eligible()
                            && registered.observation.identity.compare(&after.identity)
                                == IdentityMatch::Same,
                        "索引后分区身份无法确认"
                    );
                    catalog.observe(&partition, &after)?;
                    Ok(())
                })();
                let mut states = worker_states.lock().unwrap_or_else(|e| e.into_inner());
                let status = states.entry(partition).or_default();
                if let Some(completed) = completed_status {
                    *status = completed;
                }
                if let Err(error) = result {
                    status.phase = "failed".into();
                    status.error = Some(format!("{error:#}"));
                }
                // Publish a terminal status and release the slot under the same lock.
                worker_busy.store(false, Ordering::Release);
            }
        });
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
                            if let Some(error) = &asset.error {
                                anyhow::bail!("{error}");
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
                                        partition_id: partition.clone(),
                                        partition_name: asset.registration.name.clone(),
                                        connection: asset.connection.clone(),
                                        mounts: mounts.clone(),
                                        generation: index.generation,
                                        partial: index.partial,
                                        indexed_at: index.indexed_at,
                                        needs_validation: index.needs_validation,
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
            sender,
            busy,
            cancel,
            states,
            mailbox,
            search_state,
            sequence: AtomicU64::new(1),
        }
    }

    pub fn scan(&self, partition: &str, mode: IndexMode, snapshot: &Snapshot) -> Result<()> {
        let asset = snapshot
            .assets
            .iter()
            .find(|a| a.registration.partition_id == partition)
            .context("分区未登记")?;
        ensure!(
            asset.registration.state == "ready"
                && asset.connection == "online"
                && asset.error.is_none(),
            "分区未就绪或身份无法确认"
        );
        ensure!(
            asset
                .registration
                .observation
                .identity
                .filesystem
                .as_deref()
                == Some("NTFS"),
            "本阶段只支持 NTFS 索引"
        );
        ensure!(
            !self.busy.swap(true, Ordering::AcqRel),
            "已有索引任务或正在恢复状态"
        );
        self.cancel.store(false, Ordering::Release);
        {
            let mut states = self.states.lock().unwrap_or_else(|e| e.into_inner());
            let status = states.entry(partition.into()).or_default();
            status.phase = "queued".into();
            status.error = None;
        }
        if let Err(error) = self.sender.try_send(Scan {
            asset: asset.clone(),
            mode,
        }) {
            self.busy.store(false, Ordering::Release);
            return Err(error.into());
        }
        Ok(())
    }
    pub fn cancel_scan(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub fn status(&self, partition: &str) -> IndexStatus {
        self.states
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(partition)
            .cloned()
            .unwrap_or_else(|| IndexStatus {
                phase: if self.busy.load(Ordering::Acquire) {
                    "loading"
                } else {
                    "not_indexed"
                }
                .into(),
                ..Default::default()
            })
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
        self.cancel_scan();
        let mut mailbox = self.mailbox.0.lock().unwrap_or_else(|e| e.into_inner());
        mailbox.stop = true;
        if let Some(cancel) = &mailbox.cancel {
            cancel.store(true, Ordering::Release);
        }
        self.mailbox.1.notify_one();
    }
}
