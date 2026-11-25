use crate::registration::phase_label;
use anyhow::{Result, bail, ensure};
use chrono::{DateTime, Local};
use stashfinder_core::{
    Action, Response,
    assets::Snapshot,
    tasks::{TASK_SNAPSHOT_EXPIRED, Task, TaskOverviewRow},
};
use stashfinder_windows::ipc;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy)]
pub(super) enum Operation {
    Cancel,
    Retry,
}

pub(super) struct TaskRow {
    pub task: Task,
    partition: String,
    updated: String,
    retry_blocked: Option<&'static str>,
}

pub(super) struct TaskList {
    pub rows: Vec<TaskRow>,
    pub device_error: Option<String>,
    pub stale: bool,
}

fn active(state: &str) -> bool {
    matches!(state, "queued" | "running" | "waiting")
}

fn attention(state: &str) -> bool {
    matches!(state, "failed" | "interrupted" | "needs_confirmation")
}

fn retryable(state: &str) -> bool {
    matches!(state, "failed" | "interrupted" | "cancelled")
}

impl TaskList {
    pub async fn load(pipe: &str) -> Result<Self> {
        let (tasks, snapshot) = tokio::try_join!(read_tasks(pipe), ipc::snapshot(pipe))?;
        Ok(Self::new(tasks, snapshot))
    }

    fn new(mut tasks: Vec<Task>, snapshot: Snapshot) -> Self {
        let partitions: HashMap<_, _> = snapshot
            .assets
            .iter()
            .map(|a| (a.registration.partition_id.as_str(), a))
            .collect();
        let busy_partitions: HashSet<_> = tasks
            .iter()
            .filter(|t| active(&t.state))
            .map(|t| t.partition_id.clone())
            .collect();
        tasks.retain(|t| !t.automatic || attention(&t.state));
        tasks.sort_by_key(|t| {
            let group = if active(&t.state) {
                0
            } else if attention(&t.state) {
                1
            } else {
                2
            };
            (group, std::cmp::Reverse(t.updated))
        });
        let rows = tasks
            .into_iter()
            .map(|task| {
                let asset = partitions.get(task.partition_id.as_str());
                let partition = asset
                    .map_or(&task.partition_id, |a| &a.registration.name)
                    .clone();
                let retry_blocked = if busy_partitions.contains(&task.partition_id) {
                    Some("该分区已有待执行或运行中的任务")
                } else if snapshot.busy || snapshot.error.is_some() {
                    Some("等待读取分区状态后可重试")
                } else if !asset.is_some_and(|a| {
                    a.connection == "online"
                        && a.registration.state == "ready"
                        && a.error.is_none()
                        && a.registration.observation.eligible()
                }) {
                    Some("接入并确认分区后可重试")
                } else {
                    None
                };
                let updated = i64::try_from(task.updated)
                    .ok()
                    .and_then(|t| DateTime::from_timestamp(t, 0))
                    .map(|date| {
                        date.with_timezone(&Local)
                            .format("%Y-%m-%d %H:%M:%S")
                            .to_string()
                    })
                    .unwrap_or_else(|| "未知".into());
                TaskRow {
                    task,
                    partition,
                    updated,
                    retry_blocked,
                }
            })
            .collect();
        Self {
            rows,
            device_error: snapshot.error,
            stale: false,
        }
    }

    pub fn summary(&self) -> String {
        if self.rows.is_empty() {
            "暂无索引任务".into()
        } else {
            format!("{} 项索引任务", self.rows.len())
        }
    }

    pub fn cell(&self, row: usize, column: i32) -> &str {
        let Some(r) = self.rows.get(row) else {
            return "";
        };
        let task = &r.task;
        match column {
            0 => &r.partition,
            1 => match task.kind.as_str() {
                "scan" => "扫描校验",
                "build" => "建立索引",
                "rebuild" => "完整校验",
                "sync" => "增量更新",
                other => other,
            },
            2 => match task.state.as_str() {
                "queued" => "等待执行",
                "running" => "进行中",
                "waiting" => "等待接入或恢复",
                "needs_confirmation" => "等待确认",
                "failed" => "失败",
                "interrupted" => "已中断",
                "completed" => "已完成",
                "cancelled" => "已取消",
                "superseded" => "已重试",
                other => other,
            },
            3 => task.phase.as_deref().map(phase_label).unwrap_or("—"),
            4 => &r.updated,
            _ => "",
        }
    }

    pub fn detail(&self, row: usize) -> String {
        let r = &self.rows[row];
        let mut details = vec![r.partition.as_str(), self.cell(row, 2)];
        if let Some(error) = r.task.error.as_deref() {
            details.push(error);
        }
        match r.task.state.as_str() {
            "needs_confirmation" => details.push("请在已登记分区中选择索引方式"),
            state if active(state) => details.push("取消任务会暂停此分区的自动更新"),
            state if retryable(state) => {
                if let Some(reason) = r.retry_blocked {
                    details.push(reason);
                } else if state == "cancelled" {
                    details.push("已保留提交的索引");
                }
            }
            _ => {}
        }
        details.join(" · ")
    }

    pub fn action(&self, row: usize, op: Operation) -> Option<Action> {
        let r = self.rows.get(row)?;
        if self.stale {
            return None;
        }
        match op {
            Operation::Cancel if active(&r.task.state) => Some(Action::TaskCancel {
                id: r.task.id.clone(),
            }),
            Operation::Retry if retryable(&r.task.state) && r.retry_blocked.is_none() => {
                Some(Action::TaskRetry {
                    id: r.task.id.clone(),
                })
            }
            _ => None,
        }
    }
}

async fn read_tasks(pipe: &str) -> Result<Vec<Task>> {
    for _ in 0..3 {
        let mut tasks = Vec::new();
        let mut version = None;
        let mut offset = 0;
        loop {
            let response = ipc::request(
                pipe,
                Action::TaskOverview {
                    version: version.clone(),
                    offset,
                },
            )
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
            tasks.extend(page.rows.into_iter().filter_map(|r| match r {
                TaskOverviewRow::Task(task) => Some(task),
                TaskOverviewRow::Partition(_) => None,
            }));
            let Some(next) = page.next else {
                return Ok(tasks);
            };
            ensure!(
                count > 0 && offset.checked_add(count) == Some(next),
                "任务分页无效"
            );
            offset = next;
            version = Some(page.version);
        }
    }
    bail!("任务持续更新，请稍后刷新")
}

#[cfg(test)]
mod tests;
