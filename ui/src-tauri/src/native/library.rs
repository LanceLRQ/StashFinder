use super::*;
use crate::native::files::{choice_label, set_row_state};
use anyhow::{Context, bail};
use stashfinder_core::{
    assets::{Asset, Candidate, Observation},
    index::IndexMode,
    tasks::{PartitionIndex, TASK_SNAPSHOT_EXPIRED, TaskOverviewRow},
};

const RENAME: usize = 300;
const REMOVE: usize = 301;
const RETRY_ASSET: usize = 302;
const INDEX: usize = 303;
pub(super) const CANCEL_TASK: usize = 304;
pub(super) const RETRY_TASK: usize = 305;
const PAUSE: usize = 306;
pub(super) const RESUME: usize = 307;
pub(super) const TASK_FILTER: usize = 308;
const TASK_DETAIL: usize = 309;
pub(super) const ANALYZE: usize = 310;
pub(super) const REPORT: usize = 311;
pub(super) const CANCEL_OPERATION: usize = 312;
pub(super) const BATCH: usize = 313;
const TASK_CHOOSE: usize = 314;
pub(super) const ASSET_MENU: &[(usize, &str)] = &[
    (files::OPEN, "浏览所选分区"),
    (REGISTER, "登记分区…"),
    (INDEX, "索引设置…"),
    (CANCEL_TASK, "取消索引"),
    (RETRY_ASSET, "重试初始化"),
    (ANALYZE, "分析文件…"),
    (RENAME, "重命名…"),
    (REMOVE, "移除登记…"),
    (REFRESH_DEVICES, "刷新设备"),
];
pub(super) const TASK_MENU: &[(usize, &str)] = &[
    (TASK_FILTER, "任务分类与分区…"),
    (TASK_DETAIL, "任务详情…"),
    (CANCEL_TASK, "取消任务"),
    (RETRY_TASK, "重试任务"),
    (PAUSE, "暂停分区"),
    (RESUME, "继续分区…"),
    (TASK_CHOOSE, "选择索引方式…"),
    (BATCH, "缩略图与文件分析进度…"),
    (REFRESH, "刷新任务"),
];
pub(super) const ASSET_COLUMNS: &[(&str, i32)] = &[
    ("分区", 175),
    ("物理硬盘", 175),
    ("挂载点", 120),
    ("文件系统", 87),
    ("连接状态", 115),
    ("容量", 148),
    ("索引", 120),
];
pub(super) const TASK_COLUMNS: &[(&str, i32)] = &[
    ("分区", 156),
    ("任务", 119),
    ("状态", 119),
    ("执行阶段", 165),
    ("更新时间", 155),
    ("原因", 250),
];

pub(super) enum AssetRow {
    Disk(usize),
    Partition(usize),
}

fn connection(asset: &Asset) -> &str {
    match asset.registration.state.as_str() {
        "preparing" => "正在初始化",
        "historical" => "历史记录",
        _ => choice_label(&asset.connection),
    }
}
fn bytes(count: u64) -> String {
    for (unit, divisor) in [
        ("TiB", 1_u64 << 40),
        ("GiB", 1 << 30),
        ("MiB", 1 << 20),
        ("KiB", 1 << 10),
    ] {
        if count >= divisor {
            return format!("{:.1} {unit}", count as f64 / divisor as f64);
        }
    }
    format!("{count} B")
}
fn date(time: Option<u64>) -> String {
    time.and_then(|t| i64::try_from(t).ok())
        .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        .map_or("未知".into(), |t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
}
fn volume_name(o: &Observation) -> String {
    o.volume_label
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| o.mounts.first().map(String::as_str))
        .unwrap_or("未命名分区")
        .into()
}
fn eligible(asset: &Asset) -> bool {
    asset.connection == "online"
        && asset.error.is_none()
        && asset.registration.state == "ready"
        && asset.registration.observation.eligible()
}
fn task_group(task: &Task) -> usize {
    match task.state.as_str() {
        "queued" | "running" | "waiting" => 1,
        "failed" | "interrupted" | "needs_confirmation" => 2,
        _ => 3,
    }
}
fn task_label(task: &Task) -> &str {
    match task.kind.as_str() {
        "scan" => "扫描校验",
        "build" => "建立索引",
        "rebuild" => "完整校验",
        "sync" => "增量更新",
        other => other,
    }
}
fn task_state(task: &Task) -> &str {
    match task.state.as_str() {
        "queued" => "等待执行",
        "running" => "进行中",
        "waiting" => "等待接入或恢复",
        "completed" => "已完成",
        "cancelled" => "已取消",
        "failed" => "失败",
        "interrupted" => "已中断",
        "needs_confirmation" => "等待确认",
        "superseded" => "已重试",
        other => other,
    }
}
fn phase(phase: &str) -> &str {
    stashfinder_native_search::phase_label(phase)
}

pub(super) async fn accepted(action: Action) -> Result<()> {
    ensure!(
        matches!(runtime::request(action).await?, Response::Accepted),
        "后台响应无效"
    );
    Ok(())
}
async fn assets(refresh: bool) -> Result<Snapshot> {
    if refresh {
        accepted(Action::Refresh).await?;
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let snapshot = runtime::snapshot().await?;
        if !snapshot.busy || snapshot.error.is_some() || tokio::time::Instant::now() >= deadline {
            return Ok(snapshot);
        }
        tokio::time::sleep(std::time::Duration::from_millis(180)).await;
    }
}

struct TaskSnapshot {
    rows: Vec<Task>,
    partitions: Vec<PartitionIndex>,
    automatic: bool,
}
async fn read_tasks() -> Result<TaskSnapshot> {
    for _ in 0..3 {
        let mut rows = vec![];
        let mut partitions = vec![];
        let mut offset = 0;
        let mut version = None;
        loop {
            let response = runtime::request(Action::TaskOverview {
                version: version.clone(),
                offset,
            })
            .await;
            let page = match response {
                Ok(Response::TaskOverview(page)) => page,
                Err(e) if e.to_string() == TASK_SNAPSHOT_EXPIRED => break,
                Err(e) => return Err(e),
                _ => bail!("任务概览响应无效"),
            };
            ensure!(
                version.as_ref().is_none_or(|v| v == &page.version),
                "任务概览版本不一致"
            );
            let count = page.rows.len();
            for row in page.rows {
                match row {
                    TaskOverviewRow::Task(task) => rows.push(task),
                    TaskOverviewRow::Partition(p) => partitions.push(p),
                }
            }
            if let Some(next) = page.next {
                ensure!(
                    count > 0 && offset.checked_add(count) == Some(next),
                    "任务分页无效"
                );
                offset = next;
                version = Some(page.version);
            } else {
                return Ok(TaskSnapshot {
                    rows,
                    partitions,
                    automatic: page.automatic,
                });
            }
        }
    }
    bail!("任务持续更新，请稍后刷新");
}

impl Desktop {
    fn partition_at(&self, row: usize) -> Option<usize> {
        match self.asset_rows.borrow().get(row)? {
            AssetRow::Partition(index) => Some(*index),
            AssetRow::Disk(_) => None,
        }
    }
    fn rebuild_asset_rows(&self, selected: Option<&str>) -> Option<usize> {
        let snapshot = self.snapshot.borrow();
        let mut rows = self.asset_rows.borrow_mut();
        rows.clear();
        if checked(self.group_assets) {
            let mut by_disk = std::collections::HashMap::new();
            let mut groups = Vec::<Vec<usize>>::new();
            for (index, asset) in snapshot.assets.iter().enumerate() {
                let i = by_disk
                    .entry(&asset.registration.disk_id)
                    .or_insert_with(|| {
                        groups.push(vec![]);
                        groups.len() - 1
                    });
                groups[*i].push(index);
            }
            for group in groups {
                rows.push(AssetRow::Disk(group[0]));
                rows.extend(group.into_iter().map(AssetRow::Partition));
            }
        } else {
            rows.extend((0..snapshot.assets.len()).map(AssetRow::Partition));
        }
        let selected = selected?;
        rows.iter().position(|row| {
            matches!(row, AssetRow::Partition(index)
                if snapshot.assets[*index].registration.partition_id == selected)
        })
    }
    pub(super) fn group_assets(&self) {
        if matches!(self.tabs.borrow().active().pane, Page::Overview) {
            let selected = self.selected_asset().map(|a| a.registration.partition_id);
            let position = self.rebuild_asset_rows(selected.as_deref());
            self.present_assets(position);
        }
    }
    fn present_assets(&self, selection: Option<usize>) {
        self.apply_selection(&Selection::None);
        self.present(false);
        if let Some(row) = selection {
            set_row_state(self.list, row, true);
            send(self.list, LVM_ENSUREVISIBLE, row, 0);
        }
    }
    pub(super) fn selected_asset(&self) -> Option<Asset> {
        if !matches!(self.tabs.borrow().active().pane, Page::Overview) {
            return None;
        }
        let index = self.partition_at(self.single()?)?;
        self.snapshot.borrow().assets.get(index).cloned()
    }
    pub(super) fn asset_cell(&self, row: usize, col: usize) -> String {
        let data = self.snapshot.borrow();
        let rows = self.asset_rows.borrow();
        let asset = match rows.get(row) {
            Some(AssetRow::Disk(index)) if col == 0 => {
                let disk = &data.assets[*index].registration.observation;
                return format!("硬盘 · {}", disk.model.as_deref().unwrap_or("型号未知"));
            }
            Some(AssetRow::Partition(index)) => &data.assets[*index],
            _ => return String::new(),
        };
        let r = &asset.registration;
        let o = &r.observation;
        match col {
            0 => r.name.clone(),
            1 => o.model.clone().unwrap_or_else(|| "硬盘型号未知".into()),
            2 => {
                if asset.mounts.is_empty() {
                    o.mounts.join("、")
                } else {
                    asset.mounts.join("、")
                }
            }
            3 => o
                .identity
                .filesystem
                .clone()
                .unwrap_or_else(|| "未知".into()),
            4 => connection(asset).into(),
            5 => o.capacity.as_ref().map_or("未知".into(), |c| {
                format!(
                    "{} / {}",
                    bytes(c.total.saturating_sub(c.free)),
                    bytes(c.total)
                )
            }),
            6 => {
                if self
                    .task_indexes
                    .borrow()
                    .iter()
                    .any(|p| p.partition_id == r.partition_id && p.paused)
                {
                    return "已暂停".into();
                }
                self.tasks
                    .borrow()
                    .iter()
                    .find(|t| t.partition_id == r.partition_id && task_group(t) < 3)
                    .map_or_else(
                        || {
                            if self
                                .task_indexes
                                .borrow()
                                .iter()
                                .any(|p| p.partition_id == r.partition_id && p.indexed_at.is_some())
                            {
                                "索引已建立".into()
                            } else {
                                "尚未索引".into()
                            }
                        },
                        |t| task_state(t).into(),
                    )
            }
            _ => "".into(),
        }
    }
    pub(super) fn asset_detail(&self) -> String {
        let snapshot = self.snapshot.borrow();
        let Some(asset) = self.selected_asset() else {
            return if let Some(e) = &snapshot.error {
                format!("设备读取失败：{e}\r\n按 F5 重试。")
            } else {
                format!(
                    "已登记 {} 个分区\r\n涉及 {} 块硬盘\r\n\r\n双击分区可浏览，右键可管理分区。",
                    snapshot.assets.len(),
                    snapshot
                        .assets
                        .iter()
                        .map(|a| &a.registration.disk_id)
                        .collect::<std::collections::HashSet<_>>()
                        .len()
                )
            };
        };
        let r = &asset.registration;
        let o = &r.observation;
        let last = date(r.last_seen);
        let info = [
            r.name.clone(),
            format!("状态：{}", connection(&asset)),
            format!("物理硬盘：{}", o.model.as_deref().unwrap_or("未知")),
            format!(
                "文件系统：{}",
                o.identity.filesystem.as_deref().unwrap_or("未知")
            ),
            format!(
                "挂载点：{}",
                if asset.mounts.is_empty() {
                    o.mounts.join("、")
                } else {
                    asset.mounts.join("、")
                }
            ),
            format!("上次接入：{last}"),
            format!("分区 ID：{}", r.partition_id),
            format!("文件系统 ID：{}", r.filesystem_id),
        ];
        let mut lines = info.to_vec();
        if let Some(c) = &o.capacity {
            lines.push(format!(
                "容量：{} 剩余 / {} 共计（{}）",
                bytes(c.free),
                bytes(c.total),
                date(Some(c.sampled_at))
            ));
        }
        if let Some(index) = self
            .task_indexes
            .borrow()
            .iter()
            .find(|p| p.partition_id == r.partition_id)
        {
            lines.push(format!("最近索引：{}", date(index.indexed_at)));
            if index.paused {
                lines.push("自动更新已暂停".into());
            }
        }
        lines.extend([asset.error, asset.capacity_error].into_iter().flatten());
        lines.join("\r\n")
    }
    pub(super) fn refresh_assets(&self, refresh: bool) {
        if self.assets_pending.replace(true) {
            return;
        }
        self.spawn(assets(refresh), move |app, result| {
            app.assets_pending.set(false);
            let home = matches!(app.tabs.borrow().active().pane, Page::Overview);
            match result {
                Ok(snapshot) => {
                    let selected = if home {
                        app.selected_asset().map(|a| a.registration.partition_id)
                    } else {
                        let tabs = app.tabs.borrow();
                        match &tabs.items[0].selected {
                            Selection::Positions(positions) if positions.len() == 1 => {
                                app.partition_at(positions[0]).map(|i| {
                                    app.snapshot.borrow().assets[i]
                                        .registration
                                        .partition_id
                                        .clone()
                                })
                            }
                            _ => None,
                        }
                    };
                    let changed = {
                        let previous = app.snapshot.borrow();
                        previous.version != snapshot.version || previous.error != snapshot.error
                    };
                    let status = if let Some(e) = &snapshot.error {
                        format!("设备读取失败：{e}（F5 重试）")
                    } else if snapshot.busy {
                        "正在读取设备…".into()
                    } else if snapshot.assets.is_empty() {
                        "尚未登记分区".into()
                    } else {
                        format!("{} 个已登记分区", snapshot.assets.len())
                    };
                    *app.snapshot.borrow_mut() = snapshot;
                    let next = app.rebuild_asset_rows(selected.as_deref());
                    if home && (changed || refresh) {
                        app.status(&status);
                        app.present_assets(next);
                    } else if !home {
                        app.tabs.borrow_mut().items[0].selected =
                            next.map_or(Selection::None, |i| Selection::Positions(vec![i]));
                    }
                    if changed && matches!(app.tabs.borrow().active().pane, Page::Browse(_)) {
                        app.rebuild_tree_on_refresh();
                    } else if changed && matches!(app.tabs.borrow().active().pane, Page::Search(_))
                    {
                        app.show_search_scope();
                    }
                }
                Err(e) => {
                    let error = format!("无法读取分区：{e:#}");
                    app.snapshot.borrow_mut().error = Some(error.clone());
                    if home {
                        app.status(&error);
                        app.update_preview();
                    }
                }
            }
            app.refresh_tasks();
        });
    }
    pub(super) fn task_cell(&self, row: usize, col: usize) -> String {
        let ids = self.visible_tasks.borrow();
        let tasks = self.tasks.borrow();
        let Some(task) = ids
            .get(row)
            .and_then(|id| tasks.iter().find(|t| &t.id == id))
        else {
            return "".into();
        };
        match col {
            0 => self.asset_name(&task.partition_id),
            1 => task_label(task).into(),
            2 => task_state(task).into(),
            3 => task.phase.as_deref().map(phase).unwrap_or("—").into(),
            4 => date(Some(task.updated)),
            5 => task.error.clone().unwrap_or_default(),
            _ => "".into(),
        }
    }
    fn asset_name(&self, id: &str) -> String {
        self.snapshot
            .borrow()
            .assets
            .iter()
            .find(|a| a.registration.partition_id == id)
            .map_or_else(|| id.into(), |a| a.registration.name.clone())
    }
    fn selected_task(&self) -> Option<Task> {
        if !matches!(self.tabs.borrow().active().pane, Page::Tasks) {
            return None;
        }
        let index = self.single()?;
        let ids = self.visible_tasks.borrow();
        let id = ids.get(index)?;
        self.tasks.borrow().iter().find(|t| &t.id == id).cloned()
    }
    pub(super) fn task_buttons(&self) {
        let task = self.selected_task();
        let fresh = self.automatic.get().is_some();
        let can_retry = fresh && task.as_ref().is_some_and(|t| self.retry_eligible(t));
        let can_cancel = fresh && task.as_ref().is_some_and(|t| task_group(t) == 1);
        let paused = fresh && self.task_indexes.borrow().iter().any(|p| p.paused);
        enable(self.task_cancel, can_cancel);
        enable(self.task_retry, can_retry);
        enable(self.task_resume, paused);
    }
    fn retry_eligible(&self, task: &Task) -> bool {
        let snapshot = self.snapshot.borrow();
        matches!(task.state.as_str(), "failed" | "interrupted" | "cancelled")
            && !snapshot.busy
            && snapshot.error.is_none()
            && snapshot
                .assets
                .iter()
                .any(|a| a.registration.partition_id == task.partition_id && eligible(a))
            && !self
                .tasks
                .borrow()
                .iter()
                .any(|t| t.partition_id == task.partition_id && task_group(t) == 1)
    }
    pub(super) fn refresh_tasks(&self) {
        if self.tasks_pending.replace(true) {
            return;
        }
        self.spawn(read_tasks(), |app, result| {
            app.tasks_pending.set(false);
            let tasks_open = matches!(app.tabs.borrow().active().pane, Page::Tasks);
            match result {
                Ok(data) => {
                    app.automatic.set(Some(data.automatic));
                    app.mark_changed_indexes(&data.partitions);
                    *app.task_indexes.borrow_mut() = data.partitions;
                    *app.tasks.borrow_mut() = data.rows;
                    if tasks_open {
                        app.present_tasks();
                    }
                    if matches!(app.tabs.borrow().active().pane, Page::Overview) {
                        app.present(false);
                    }
                }
                Err(e) => {
                    app.automatic.set(None);
                    app.task_buttons();
                    if tasks_open {
                        app.status(&format!("无法读取索引任务：{e:#}（保留上次结果，F5 重试）"));
                    }
                }
            }
        });
    }
    fn mark_changed_indexes(&self, next: &[PartitionIndex]) {
        let previous = self.task_indexes.borrow();
        let changed = previous
            .iter()
            .chain(next)
            .filter_map(|p| {
                let id = &p.partition_id;
                let old = previous
                    .iter()
                    .find(|p| &p.partition_id == id)
                    .and_then(|p| p.index_version.as_ref());
                let new = next
                    .iter()
                    .find(|p| &p.partition_id == id)
                    .and_then(|p| p.index_version.as_ref());
                (old != new).then_some(id)
            })
            .collect::<std::collections::HashSet<_>>();
        if changed.is_empty() {
            return;
        }
        let mut tabs = self.tabs.borrow_mut();
        for tab in &mut tabs.items {
            let affected = match &tab.pane {
                Page::Browse(b) => b
                    .location()
                    .is_some_and(|place| changed.contains(&place.volume.id)),
                Page::Search(s) => {
                    s.id.is_some()
                        && (s.request.partitions.is_empty()
                            || s.request.partitions.iter().any(|id| changed.contains(id)))
                }
                _ => false,
            };
            tab.index_dirty |= affected;
        }
    }
    fn present_tasks(&self) {
        let selected = self.selected_task().map(|t| t.id);
        let group = self.task_filter.get();
        let partition = self.task_partition.borrow();
        let tasks = self.tasks.borrow();
        let mut rows = tasks
            .iter()
            .filter(|t| {
                (!t.automatic || task_group(t) == 2)
                    && (group == 0 || group == task_group(t))
                    && partition.as_ref().is_none_or(|p| p == &t.partition_id)
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(|t| std::cmp::Reverse(t.updated));
        if group == 0 {
            rows.truncate(5);
        }
        let rows = rows.into_iter().map(|t| t.id.clone()).collect::<Vec<_>>();
        let position = selected
            .as_ref()
            .and_then(|id| rows.iter().position(|row| row == id));
        let summary = if rows.is_empty() {
            "暂无索引任务".into()
        } else {
            format!(
                "{} · {} 项索引任务",
                ["最近动态", "当前", "需处理", "历史"][group],
                rows.len()
            )
        };
        drop(tasks);
        drop(partition);
        if *self.visible_tasks.borrow() != rows {
            self.apply_selection(&Selection::None);
            *self.visible_tasks.borrow_mut() = rows;
            self.present(false);
            if let Some(i) = position {
                set_row_state(self.list, i, true);
            }
        } else {
            self.present(false);
        }
        self.status(&summary);
    }

    pub(super) fn register_partition(&self) {
        if self.assets_pending.replace(true) {
            return;
        }
        self.status("正在读取可登记分区…");
        self.spawn(assets(true), |app, result| {
            app.assets_pending.set(false);
            let result = result.and_then(|s| {
                if let Some(e) = &s.error {
                    bail!(e.clone());
                }
                ensure!(!s.busy, "设备仍在读取，请刷新后再试");
                Ok(s)
            });
            let snapshot = match result {
                Ok(s) => s,
                Err(e) => {
                    app.status(&format!("无法读取可登记分区：{e:#}"));
                    return;
                }
            };
            let items = snapshot
                .candidates
                .iter()
                .filter(|c| !c.registered && c.blocked.is_none() && c.observation.eligible())
                .collect::<Vec<_>>();
            if items.is_empty() {
                info(app.hwnd, "登记分区", "没有可登记的分区。");
                return;
            }
            let choices = items.iter().map(|c| candidate_label(c)).collect();
            let Some(mut pick) = app.ask(
                "登记分区",
                &[
                    Field::Choice("分区", choices, 0),
                    Field::Check("登记后自动建立索引", true),
                ],
            ) else {
                return;
            };
            let Some(candidate) = items.get(pick.choice()) else {
                return;
            };
            let automatic = pick.check();
            let Some(mut name) = app.ask(
                "登记分区名称",
                &[Field::Edit("名称", volume_name(&candidate.observation))],
            ) else {
                return;
            };
            let name = name.text();
            if let Err(e) = stashfinder_core::assets::validate_name(&name) {
                info(app.hwnd, "名称无效", &e.to_string());
                return;
            }
            let allow_weak = candidate.weak
                && confirm(
                    app.hwnd,
                    "确认登记分区",
                    "所选分区身份信息不足，无法可靠匹配已有记录。仍按新分区登记？",
                );
            if candidate.weak && !allow_weak {
                return;
            }
            let candidate = (*candidate).clone();
            app.status("正在登记分区…");
            app.spawn(
                async move {
                    let latest = assets(false).await?;
                    let current = latest
                        .candidates
                        .iter()
                        .find(|c| {
                            c.observation.candidate_id == candidate.observation.candidate_id
                                && c.observation.identity == candidate.observation.identity
                                && !c.registered
                                && c.blocked.is_none()
                                && c.weak == candidate.weak
                        })
                        .context("登记状态已变化，请重新选择")?;
                    ensure!(latest.error.is_none() && !latest.busy, "设备状态暂不可用");
                    ensure!(
                        !allow_weak || latest.version == snapshot.version,
                        "所选分区已变化，请重新确认"
                    );
                    accepted(Action::Register {
                        version: latest.version,
                        candidate_id: current.observation.candidate_id.clone(),
                        name: name.trim().into(),
                        allow_weak,
                        auto_index: Some(automatic),
                    })
                    .await
                },
                |app, result| app.asset_command_finished(result),
            );
        });
    }
    pub(super) fn pick_volume(&self) {
        let assets = self
            .snapshot
            .borrow()
            .assets
            .iter()
            .filter(|a| matches!(a.registration.state.as_str(), "ready" | "historical"))
            .cloned()
            .collect::<Vec<_>>();
        if assets.is_empty() {
            info(self.hwnd, "切换浏览分区", "尚无可浏览的已登记分区。");
            return;
        }
        let choices = assets
            .iter()
            .map(|a| format!("{} · {}", a.registration.name, connection(a)))
            .collect();
        if let Some(mut choice) = self.ask("切换浏览分区", &[Field::Choice("分区", choices, 0)])
            && let Some(asset) = assets.get(choice.choice())
        {
            let location = Location {
                volume: Volume::from(asset),
                parent: None,
            };
            if matches!(self.tabs.borrow().active().pane, Page::Browse(_)) {
                self.visit(location);
            } else {
                self.new_browse(Some(location));
            }
        }
    }
    fn asset_command_finished(&self, result: Result<()>) {
        match result {
            Ok(()) => {
                self.status("操作已提交");
                self.refresh_assets(false);
            }
            Err(e) => info(self.hwnd, "分区操作失败", &format!("{e:#}")),
        }
    }
    fn asset_command(&self, action: Action) {
        self.spawn(accepted(action), |app, result| {
            app.asset_command_finished(result)
        });
    }
    fn selected_volume_for_command(&self) -> Option<Asset> {
        if let Some(asset) = self.selected_asset() {
            return Some(asset);
        }
        let partition = self.selected_task().map(|t| t.partition_id).or_else(|| {
            let tabs = self.tabs.borrow();
            if let Page::Browse(b) = &tabs.active().pane {
                b.location().map(|p| p.volume.id.clone())
            } else {
                None
            }
        })?;
        self.snapshot
            .borrow()
            .assets
            .iter()
            .find(|a| a.registration.partition_id == partition)
            .cloned()
    }
    fn index_dialog(&self, asset: Asset, rebuild: bool) {
        if !eligible(&asset) || self.snapshot.borrow().error.is_some() {
            info(self.hwnd, "索引设置", "分区未就绪、离线或身份待确认。");
            return;
        }
        let id = asset.registration.partition_id.clone();
        self.spawn(runtime::request(Action::IndexState { partition_id: id.clone() }), move |app, reply| {
            let index = match reply { Ok(Response::Index(i)) => i, Ok(_) => return, Err(e) => { info(app.hwnd, "索引状态", &format!("{e:#}")); return; } };
            let ntfs = asset.registration.observation.identity.filesystem.as_deref() == Some("NTFS");
            let mut options = vec![("基础扫描（不创建日志）", IndexMode::ScanOnly)];
            if ntfs {
                if !index.journal_missing { options.insert(0, ("使用已有 USN 日志", IndexMode::Journal)); }
                options.push(("创建 USN 日志并索引", IndexMode::CreateJournal));
            }
            let title = format!("{} · {}", if rebuild { "重建索引" } else { "选择索引方式" }, asset.registration.name);
            let Some(mut values) = app.ask(&title, &[Field::Choice("执行方式", options.iter().map(|(s, _)| (*s).into()).collect(), 0)]) else { return; };
            let mode = options[values.choice()].1;
            if matches!(mode, IndexMode::CreateJournal) && !confirm(app.hwnd, "授权创建 USN 日志",
                "允许在所选 NTFS 分区创建 USN 日志？新日志最大 32 MiB，分配增量 8 MiB。已有日志保留原配置。") { return; }
            if rebuild && !confirm(app.hwnd, "确认重建索引", "重建完成前保留已有索引。是否继续？") { return; }
            app.asset_command(Action::IndexStart { partition_id: id, mode });
        });
    }
    fn task_dialog_filter(&self) {
        let assets = self.snapshot.borrow().assets.clone();
        let mut names = vec!["全部分区".into()];
        names.extend(assets.iter().map(|a| a.registration.name.clone()));
        let id = self.task_partition.borrow().clone();
        let selected = assets
            .iter()
            .position(|a| Some(&a.registration.partition_id) == id.as_ref())
            .map_or(0, |i| i + 1);
        if let Some(mut fields) = self.ask(
            "筛选索引任务",
            &[
                Field::Choice(
                    "任务分类",
                    ["最近动态", "当前", "需处理", "历史"]
                        .map(str::to_string)
                        .to_vec(),
                    self.task_filter.get(),
                ),
                Field::Choice("分区", names, selected),
            ],
        ) {
            self.task_filter.set(fields.choice());
            *self.task_partition.borrow_mut() = fields
                .choice()
                .checked_sub(1)
                .and_then(|i| assets.get(i))
                .map(|a| a.registration.partition_id.clone());
            self.present_tasks();
        }
    }
    pub(super) fn library_command(&self, id: usize) -> bool {
        match id {
            RENAME | REMOVE | RETRY_ASSET | INDEX | TASK_CHOOSE => {
                let Some(asset) = self.selected_volume_for_command() else {
                    return true;
                };
                let r = &asset.registration;
                match id {
                    RENAME => {
                        if let Some(mut fields) =
                            self.ask("重命名分区", &[Field::Edit("名称", r.name.clone())])
                        {
                            self.asset_command(Action::Rename {
                                partition_id: r.partition_id.clone(),
                                name: fields.text(),
                            });
                        }
                    }
                    REMOVE => {
                        if confirm(
                            self.hwnd,
                            "移除登记",
                            &format!(
                                "移除「{}」的登记并删除此分区的索引？磁盘上的原文件会保留。",
                                r.name
                            ),
                        ) {
                            self.asset_command(Action::Unregister {
                                partition_id: r.partition_id.clone(),
                            });
                        }
                    }
                    RETRY_ASSET if r.state == "preparing" && asset.connection == "online" => self
                        .asset_command(Action::Retry {
                            version: self.snapshot.borrow().version,
                            partition_id: r.partition_id.clone(),
                            candidate_id: r.observation.candidate_id.clone(),
                        }),
                    INDEX | TASK_CHOOSE => {
                        let rebuild = self.tasks.borrow().iter().any(|t| {
                            t.partition_id == r.partition_id
                                && t.kind == "rebuild"
                                && t.state == "needs_confirmation"
                        });
                        self.index_dialog(asset, rebuild);
                    }
                    _ => {}
                }
            }
            TASK_FILTER => self.task_dialog_filter(),
            TASK_DETAIL => {
                if let Some(t) = self.selected_task() {
                    info(
                        self.hwnd,
                        "索引任务详情",
                        &format!(
                            "{}\n{} · {}\n执行阶段：{}\n创建：{}\n更新：{}\n{}",
                            self.asset_name(&t.partition_id),
                            task_label(&t),
                            task_state(&t),
                            t.phase.as_deref().map(phase).unwrap_or("—"),
                            date(Some(t.created)),
                            date(Some(t.updated)),
                            t.error.as_deref().unwrap_or("")
                        ),
                    );
                }
            }
            CANCEL_TASK | RETRY_TASK | PAUSE => {
                if self.automatic.get().is_none() {
                    return true;
                }
                let task = self.selected_task().or_else(|| {
                    let asset = self.selected_asset()?;
                    self.tasks
                        .borrow()
                        .iter()
                        .find(|t| {
                            t.partition_id == asset.registration.partition_id && task_group(t) == 1
                        })
                        .cloned()
                });
                let Some(task) = task else {
                    return true;
                };
                let action = match id {
                    CANCEL_TASK if task_group(&task) == 1 => Action::TaskCancel { id: task.id },
                    RETRY_TASK if self.retry_eligible(&task) => Action::TaskRetry { id: task.id },
                    PAUSE if task_group(&task) == 1 => Action::PartitionPause {
                        partition_id: task.partition_id,
                        paused: true,
                    },
                    _ => return true,
                };
                self.asset_command(action);
            }
            RESUME => {
                if self.automatic.get().is_none() {
                    return true;
                }
                let available = self
                    .task_indexes
                    .borrow()
                    .iter()
                    .filter(|p| p.paused)
                    .map(|p| p.partition_id.clone())
                    .collect::<Vec<_>>();
                if available.is_empty() {
                    info(self.hwnd, "继续分区", "没有已暂停的分区。");
                    return true;
                }
                let names = available.iter().map(|id| self.asset_name(id)).collect();
                if let Some(mut fields) =
                    self.ask("继续分区", &[Field::Choice("已暂停的分区", names, 0)])
                {
                    self.asset_command(Action::PartitionPause {
                        partition_id: available[fields.choice()].clone(),
                        paused: false,
                    });
                }
            }
            ANALYZE => self.analyze_files(self.selected_volume_for_command()),
            BATCH => self.batch_progress(),
            REPORT => self.operation_report(),
            CANCEL_OPERATION => self.cancel_operation(),
            _ => return false,
        }
        true
    }
}

fn candidate_label(candidate: &Candidate) -> String {
    let o = &candidate.observation;
    format!(
        "{} · {} · {}{}",
        volume_name(o),
        o.mounts.join("、"),
        o.identity.filesystem.as_deref().unwrap_or("未知"),
        if candidate.weak {
            " · 身份待确认"
        } else {
            ""
        }
    )
}
