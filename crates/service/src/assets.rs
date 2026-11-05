use anyhow::{Context, Result, bail, ensure};
use stashfinder_core::{Action, Page, SNAPSHOT_EXPIRED, assets::*};
use stashfinder_store::catalog::Catalog;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{SyncSender, sync_channel},
    },
    thread,
};

pub struct Assets {
    state: Arc<Mutex<Snapshot>>,
    sender: SyncSender<Action>,
}

impl Assets {
    pub fn start(
        root: PathBuf,
        mut discover: impl FnMut() -> Result<Vec<Observation>> + Send + 'static,
    ) -> Self {
        let state = Arc::new(Mutex::new(Snapshot {
            busy: true,
            version: 1,
            ..Snapshot::default()
        }));
        let (sender, receiver) = sync_channel(1);
        let shared = state.clone();
        thread::spawn(move || {
            let mut catalog = Catalog::open(&root);
            let mut observations = Vec::new();
            let mut discovered = false;
            let mut action = Action::Refresh;
            loop {
                let before = shared.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let result = (|| -> Result<()> {
                    let db = catalog
                        .as_mut()
                        .map_err(|e| anyhow::anyhow!("资产库不可用：{e:#}"))?;
                    match &action {
                        Action::Refresh => {
                            let found = discover()?;
                            observations = found;
                            discovered = true;
                        }
                        Action::Register {
                            candidate_id,
                            name,
                            allow_weak,
                            ..
                        } => {
                            let selected = before
                                .candidates
                                .iter()
                                .find(|c| c.observation.candidate_id == *candidate_id)
                                .context("候选不存在，请刷新")?;
                            ensure!(
                                selected.blocked.is_none(),
                                "{}",
                                selected.blocked.as_deref().unwrap_or_default()
                            );
                            ensure!(
                                !selected.weak || *allow_weak,
                                "身份信息不足，需要单独确认按新分区登记"
                            );
                            let fresh = discover()?;
                            let current = fresh
                                .iter()
                                .find(|o| o.candidate_id == *candidate_id)
                                .context("分区已断开，请刷新")?;
                            ensure!(
                                current.eligible()
                                    && current.identity == selected.observation.identity,
                                "设备身份已变化，请刷新后重新选择"
                            );
                            ensure!(!duplicate(current, &fresh), "设备身份冲突");
                            let existing = db.list()?;
                            ensure!(
                                !existing
                                    .iter()
                                    .any(|r| r.state != "historical"
                                        && related(&r.observation, current)),
                                "分区已登记或需要确认归属，请刷新"
                            );
                            let row = db.prepare(current, name)?;
                            db.finish(&row.partition_id)?;
                            observations = fresh;
                            discovered = true;
                        }
                        Action::Retry {
                            partition_id,
                            candidate_id,
                            ..
                        } => {
                            let fresh = discover()?;
                            let current = fresh
                                .iter()
                                .find(|o| o.candidate_id == *candidate_id)
                                .context("分区已断开")?;
                            ensure!(
                                current.eligible() && !duplicate(current, &fresh),
                                "设备不可用或身份冲突"
                            );
                            db.retry(partition_id, current)?;
                            observations = fresh;
                        }
                        Action::Rename { partition_id, name } => db.rename(partition_id, name)?,
                        _ => bail!("不是资产写操作"),
                    }
                    Ok(())
                })();
                let discovery_failed = matches!(action, Action::Refresh) && result.is_err();
                let built = catalog
                    .as_mut()
                    .map_err(|e| anyhow::anyhow!("{e:#}"))
                    .and_then(|db| {
                        build_snapshot(
                            db,
                            &observations,
                            discovered && !discovery_failed,
                            matches!(
                                action,
                                Action::Refresh | Action::Register { .. } | Action::Retry { .. }
                            ) && result.is_ok(),
                        )
                    });
                let mut next = match built {
                    Ok(s) => s,
                    Err(error) => Snapshot {
                        error: Some(format!("{error:#}")),
                        ..before.clone()
                    },
                };
                if let Err(error) = result {
                    next.error = Some(format!("{error:#}"));
                }
                next.busy = false;
                next.version = before.version + 1;
                *shared.lock().unwrap_or_else(|e| e.into_inner()) = next;
                match receiver.recv() {
                    Ok(next_action) => action = next_action,
                    Err(_) => break,
                }
            }
        });
        Self { state, sender }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn submit(&self, action: Action) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("资产状态不可用"))?;
        ensure!(!state.busy, "正在处理，请稍后重试");
        match &action {
            Action::Register { version, name, .. } => {
                ensure!(*version == state.version, "{SNAPSHOT_EXPIRED}");
                validate_name(name)?;
            }
            Action::Retry { version, .. } => {
                ensure!(*version == state.version, "{SNAPSHOT_EXPIRED}")
            }
            Action::Rename { name, .. } => {
                validate_name(name)?;
            }
            Action::Refresh => (),
            _ => bail!("不是资产写操作"),
        }
        self.sender.try_send(action).context("后台工作队列不可用")?;
        state.busy = true;
        state.error = None;
        state.version += 1;
        Ok(())
    }

    pub fn page(&self, version: Option<u64>, offset: usize) -> Result<Page> {
        let state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("资产状态不可用"))?;
        ensure!(
            version.is_none_or(|v| v == state.version),
            "{SNAPSHOT_EXPIRED}"
        );
        let length = state.assets.len() + state.candidates.len();
        ensure!(offset < length || offset == 0, "分页位置无效");
        Ok(Page {
            version: state.version,
            busy: state.busy,
            error: state.error.clone(),
            asset: state.assets.get(offset).cloned(),
            candidate: offset
                .checked_sub(state.assets.len())
                .and_then(|i| state.candidates.get(i))
                .cloned(),
            next: (offset + 1 < length).then_some(offset + 1),
        })
    }
}

fn duplicate(item: &Observation, observations: &[Observation]) -> bool {
    let key = item.identity.partition_key();
    key.is_some()
        && observations
            .iter()
            .filter(|o| o.identity.partition_key() == key)
            .count()
            > 1
}

fn related(a: &Observation, b: &Observation) -> bool {
    a.candidate_id == b.candidate_id
        || (!matches!(a.identity.layout, Layout::Unknown)
            && !matches!(b.identity.layout, Layout::Unknown)
            && a.identity.compare(&b.identity) != IdentityMatch::Different)
}

fn build_snapshot(
    db: &mut Catalog,
    observations: &[Observation],
    discovered: bool,
    update: bool,
) -> Result<Snapshot> {
    let rows = db.list()?;
    let mut assets = Vec::new();
    for mut row in rows.clone() {
        let matches: Vec<_> = observations
            .iter()
            .filter(|o| related(&row.observation, o))
            .collect();
        let mut asset = Asset {
            registration: row.clone(),
            connection: if discovered { "offline" } else { "unconfirmed" }.into(),
            mounts: Vec::new(),
            error: None,
        };
        if row.state == "historical" {
            asset.connection = "historical".into();
        } else if discovered && matches.len() > 1 {
            asset.connection = "uncertain".into();
            asset.error = Some("多个设备匹配此登记，未更新原记录".into());
        } else if discovered && let Some(current) = matches.first() {
            if duplicate(current, observations) {
                asset.connection = "uncertain".into();
                asset.error = Some("设备身份重复".into());
            } else if let Some(error) = &current.error {
                asset.connection = "inaccessible".into();
                asset.error = Some(error.clone());
            } else {
                match row.observation.identity.compare(&current.identity) {
                    IdentityMatch::Same => {
                        asset.connection = "online".into();
                        asset.mounts = current.mounts.clone();
                        // Retry uses the current access path only after identity matching.
                        row.observation.candidate_id = current.candidate_id.clone();
                        if update && row.state == "ready" {
                            match db.observe(&row.partition_id, current) {
                                Ok(()) => row = db.get(&row.partition_id)?,
                                Err(error) => asset.error = Some(format!("{error:#}")),
                            }
                        }
                    }
                    IdentityMatch::FilesystemChanged => {
                        if update && row.state == "ready" {
                            db.archive(&row.partition_id)?;
                            row = db.get(&row.partition_id)?;
                            asset.connection = "historical".into();
                        } else {
                            asset.connection = "changed".into();
                            asset.error = Some("文件系统已变化，旧索引保留".into());
                        }
                    }
                    _ => {
                        asset.connection = "uncertain".into();
                        asset.error = Some("身份信息不足，需要确认归属".into());
                    }
                }
            }
        }
        if let Err(error) = db.health(&row.filesystem_id) {
            asset.error = Some(format!("{error:#}"));
        }
        asset.registration = row;
        assets.push(asset);
    }
    let candidates = if discovered {
        observations
            .iter()
            .map(|o| {
                let registered = assets.iter().any(|a| {
                    a.registration.state != "historical" && related(&a.registration.observation, o)
                });
                let blocked = if duplicate(o, observations) {
                    Some("设备身份冲突".into())
                } else if registered {
                    Some("已登记或等待确认归属".into())
                } else if !o.eligible() {
                    Some(o.error.clone().unwrap_or_else(|| "不支持此文件系统".into()))
                } else {
                    None
                };
                Candidate {
                    observation: o.clone(),
                    weak: o.identity.compare(&o.identity) != IdentityMatch::Same,
                    blocked,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok(Snapshot {
        assets,
        candidates,
        ..Snapshot::default()
    })
}
