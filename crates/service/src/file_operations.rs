use anyhow::{Context, Result, ensure};
use stashfinder_core::{
    MAX_FRAME,
    assets::{Asset, IdentityMatch, Snapshot},
    file_operations::*,
};
use stashfinder_store::{catalog::Catalog, file_operations, search::query::ResultSet};
use std::{
    collections::{BTreeSet, HashSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

pub(crate) struct Selection {
    root: PathBuf,
    slot: Option<Slot>,
}
struct Slot {
    records_only: bool,
    id: u64,
    origin: SelectionOrigin,
    query: Option<Arc<ResultSet>>,
    directory: Vec<EntryRef>,
    entries: HashSet<Vec<u8>>,
    positions: HashSet<usize>,
    sealed: Option<Vec<EntryRef>>,
    assets: Vec<Asset>,
    touched: Instant,
}
impl Selection {
    pub fn inspect(&self, reference: &EntryRef, snapshot: &Snapshot) -> Result<VisualRecord> {
        let catalog = Catalog::read(&self.root)?;
        let registered = catalog.get(&reference.partition_id)?;
        ensure!(
            registered.filesystem_id == reference.filesystem_id,
            "文件系统登记已变化"
        );
        let file =
            file_operations::record(&catalog.volume_path(&reference.filesystem_id)?, reference)?;
        let asset = snapshot
            .assets
            .iter()
            .find(|a| a.registration.partition_id == reference.partition_id);
        let online = snapshot.error.is_none()
            && registered.state == "ready"
            && asset.is_some_and(|a| {
                a.registration.filesystem_id == reference.filesystem_id
                    && a.registration.state == "ready"
                    && a.connection == "online"
                    && a.error.is_none()
            });
        let observation = if online {
            &asset.unwrap().registration.observation
        } else {
            &registered.observation
        };
        Ok(VisualRecord {
            online,
            target: PreparedTarget {
                mounts: if online {
                    asset.unwrap().mounts.clone()
                } else {
                    vec![]
                },
                file,
                volume: observation.candidate_id.clone(),
                identity: observation.identity.clone(),
                partition_name: registered.name,
            },
        })
    }
    pub fn destination(&self, reference: &EntryRef, snapshot: &Snapshot) -> Result<PreparedTarget> {
        ensure!(
            snapshot.error.is_none() && (!snapshot.busy || !snapshot.assets.is_empty()),
            "资产状态不可用，请刷新"
        );
        let asset = snapshot
            .assets
            .iter()
            .find(|a| a.registration.partition_id == reference.partition_id)
            .context("目标分区未登记")?;
        ensure!(
            asset.registration.filesystem_id == reference.filesystem_id
                && asset.registration.state == "ready"
                && asset.connection == "online"
                && asset.error.is_none(),
            "目标目录离线、历史或身份尚未确认"
        );
        let catalog = Catalog::read(&self.root)?;
        let registered = catalog.get(&reference.partition_id)?;
        ensure!(
            registered.state == "ready" && registered.filesystem_id == reference.filesystem_id,
            "目标登记已变化"
        );
        let file = file_operations::resolve(
            &catalog.volume_path(&reference.filesystem_id)?,
            reference,
            true,
        )?;
        ensure!(file.folder, "粘贴目标不是目录");
        Ok(PreparedTarget {
            mounts: asset.mounts.clone(),
            file,
            volume: asset.registration.observation.candidate_id.clone(),
            identity: asset.registration.observation.identity.clone(),
            partition_name: asset.registration.name.clone(),
        })
    }
    pub fn new(root: PathBuf) -> Self {
        Self { root, slot: None }
    }
    pub fn begin(
        &mut self,
        id: u64,
        origin: SelectionOrigin,
        query: Option<Arc<ResultSet>>,
        snapshot: &Snapshot,
        records_only: bool,
    ) -> Result<u64> {
        self.slot = None;
        ensure!(
            snapshot.error.is_none() && (!snapshot.busy || !snapshot.assets.is_empty()),
            "资产状态不可用，请刷新"
        );
        let catalog = Catalog::read(&self.root)?;
        let directory = match &origin {
            SelectionOrigin::Directory {
                context,
                all,
                filter,
            } => {
                let registration = catalog.get(&context.partition_id)?;
                ensure!(
                    registration.filesystem_id == context.filesystem_id,
                    "文件系统登记已变化"
                );
                let path = catalog.volume_path(&context.filesystem_id)?;
                let parent = if records_only {
                    file_operations::record(&path, context)?
                } else {
                    file_operations::resolve(&path, context, true)?
                };
                ensure!(parent.folder, "所选条目不是目录");
                if *all {
                    file_operations::directory_refs(
                        &path,
                        context,
                        records_only,
                        filter.as_deref(),
                    )?
                } else {
                    vec![]
                }
            }
            SelectionOrigin::Search { range, .. } => {
                let query = query.as_ref().context("搜索已过期")?;
                if let Some([a, b]) = range {
                    ensure!(a <= b && *b < query.len(), "所选搜索范围无效");
                }
                vec![]
            }
        };
        self.slot = Some(Slot {
            records_only,
            id,
            origin,
            query,
            directory,
            entries: HashSet::new(),
            positions: HashSet::new(),
            sealed: None,
            assets: snapshot.assets.clone(),
            touched: Instant::now(),
        });
        Ok(id)
    }
    fn slot(&mut self, id: u64) -> Result<&mut Slot> {
        if self
            .slot
            .as_ref()
            .is_some_and(|s| s.touched.elapsed() > Duration::from_secs(300))
        {
            self.slot = None;
        }
        let slot = self.slot.as_mut().context("文件操作准备已取消或过期")?;
        ensure!(slot.id == id, "文件操作准备已过期");
        slot.touched = Instant::now();
        Ok(slot)
    }
    pub fn add(&mut self, id: u64, chunk: SelectionChunk) -> Result<()> {
        let slot = self.slot(id)?;
        ensure!(slot.sealed.is_none(), "选择已经固定");
        match (&slot.origin, chunk) {
            (SelectionOrigin::Directory { .. }, SelectionChunk::Entries(entries)) => {
                ensure!(
                    slot.entries.len() + entries.len() <= MAX_SELECTED,
                    "选择条目过多"
                );
                ensure!(
                    entries.iter().all(|entry| !entry.is_empty()),
                    "条目标识为空"
                );
                for entry in entries {
                    if !slot.entries.remove(&entry) {
                        slot.entries.insert(entry);
                    }
                }
            }
            (SelectionOrigin::Search { .. }, SelectionChunk::Positions(positions)) => {
                ensure!(
                    slot.positions.len() + positions.len() <= MAX_SELECTED,
                    "选择条目过多"
                );
                let total = slot.query.as_ref().context("搜索已过期")?.len();
                ensure!(
                    positions.iter().all(|position| *position < total),
                    "所选搜索位置无效"
                );
                for position in positions {
                    if !slot.positions.remove(&position) {
                        slot.positions.insert(position);
                    }
                }
            }
            _ => anyhow::bail!("选择类型不匹配"),
        }
        Ok(())
    }
    pub fn seal(&mut self, id: u64) -> Result<usize> {
        let slot = self.slot(id)?;
        if let Some(rows) = &slot.sealed {
            return Ok(rows.len());
        }
        let rows = match &slot.origin {
            SelectionOrigin::Directory { context, all, .. } => {
                if *all {
                    slot.directory
                        .iter()
                        .filter(|r| !slot.entries.contains(&r.entry))
                        .cloned()
                        .collect::<Vec<_>>()
                } else {
                    let mut entries = slot.entries.iter().cloned().collect::<Vec<_>>();
                    entries.sort();
                    entries
                        .into_iter()
                        .map(|entry| EntryRef {
                            entry,
                            ..context.clone()
                        })
                        .collect()
                }
            }
            SelectionOrigin::Search { range, .. } => {
                let query = slot.query.as_ref().context("搜索已过期")?;
                let mut selected = BTreeSet::new();
                if let Some([a, b]) = range {
                    ensure!(b - a < MAX_SELECTED * 2, "单次最多操作 10,000 项");
                    selected.extend(*a..=*b);
                }
                for position in &slot.positions {
                    if !selected.remove(position) {
                        selected.insert(*position);
                    }
                }
                ensure!(selected.len() <= MAX_SELECTED, "单次最多操作 10,000 项");
                selected
                    .into_iter()
                    .map(|i| query.reference(i))
                    .collect::<Result<Vec<_>>>()?
            }
        };
        ensure!(
            !rows.is_empty() && rows.len() <= MAX_SELECTED,
            "请选择 1～10,000 个顶层条目"
        );
        let len = rows.len();
        slot.sealed = Some(rows);
        Ok(len)
    }
    pub fn page(&mut self, id: u64, offset: usize, snapshot: &Snapshot) -> Result<SelectionPage> {
        let root = self.root.clone();
        let slot = self.slot(id)?;
        ensure!(
            snapshot.error.is_none() && (!snapshot.busy || !snapshot.assets.is_empty()),
            "资产状态不可用，请刷新"
        );
        let rows = slot.sealed.as_ref().context("选择尚未固定")?;
        ensure!(offset <= rows.len(), "操作分页位置无效");
        let mut page = SelectionPage {
            id,
            total: rows.len(),
            next: None,
            rows: vec![],
        };
        let catalog = Catalog::read(&root)?;
        let mut bytes = serde_json::to_vec(&page)?.len();
        for reference in rows.iter().skip(offset).take(32) {
            let asset = snapshot
                .assets
                .iter()
                .find(|a| a.registration.partition_id == reference.partition_id)
                .context("分区已取消登记")?;
            let captured = slot
                .assets
                .iter()
                .find(|a| a.registration.partition_id == reference.partition_id)
                .context("分区不在操作快照中")?;
            ensure!(
                asset.registration.filesystem_id == reference.filesystem_id
                    && (slot.records_only
                        || (asset.registration.state == "ready"
                            && asset.connection == "online"
                            && asset.error.is_none())),
                "{}：分区离线、历史或身份尚未确认",
                asset.registration.name
            );
            ensure!(
                slot.records_only
                    || captured
                        .registration
                        .observation
                        .identity
                        .compare(&asset.registration.observation.identity)
                        == IdentityMatch::Same,
                "分区身份已变化"
            );
            let registration = catalog.get(&reference.partition_id)?;
            ensure!(
                (slot.records_only || registration.state == "ready")
                    && registration.filesystem_id == reference.filesystem_id,
                "文件系统登记已变化"
            );
            let path = catalog.volume_path(&reference.filesystem_id)?;
            let file = if slot.records_only {
                file_operations::record(&path, reference)?
            } else {
                file_operations::resolve(&path, reference, false)?
            };
            if let SelectionOrigin::Directory { context, .. } = &slot.origin {
                ensure!(
                    file.parent.as_ref() == Some(&context.entry),
                    "所选条目已不在原目录"
                );
            }
            let target = PreparedTarget {
                mounts: if asset.connection == "online" {
                    asset.mounts.clone()
                } else {
                    vec![]
                },
                file,
                volume: asset.registration.observation.candidate_id.clone(),
                identity: asset.registration.observation.identity.clone(),
                partition_name: asset.registration.name.clone(),
            };
            let size = serde_json::to_vec(&target)?.len() + 1;
            if bytes + size > MAX_FRAME - 256 {
                ensure!(!page.rows.is_empty(), "操作条目超过单帧传输上限");
                break;
            }
            bytes += size;
            page.rows.push(target);
        }
        page.next = (offset + page.rows.len() < page.total).then_some(offset + page.rows.len());
        Ok(page)
    }
    pub fn cancel(&mut self, id: u64) {
        if self.slot.as_ref().is_some_and(|s| s.id == id) {
            self.slot = None;
        }
    }
}
