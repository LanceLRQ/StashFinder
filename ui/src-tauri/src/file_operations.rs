use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use stashfinder_core::{Action, MAX_FRAME, Response, file_operations::*};
use stashfinder_windows::{file_operations::verify, ipc, shell};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, State};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionInput {
    pub origin: SelectionOrigin,
    pub entries: Vec<Vec<u8>>,
    pub positions: Vec<usize>,
}
#[derive(Clone, Serialize, Default)]
pub struct OperationStatus {
    pub close_requested: bool,
    pub id: u64,
    pub phase: String,
    pub total: usize,
    pub processed: usize,
    pub error: Option<String>,
}
#[derive(Serialize)]
pub struct PreparedSummary {
    pub id: u64,
    pub count: usize,
    pub names: Vec<String>,
    pub same_parent: bool,
    pub single_file: bool,
}
#[derive(Default)]
struct Work {
    report: Option<OperationReport>,
    records_only: bool,
    status: OperationStatus,
    cancel: Arc<AtomicBool>,
    targets: Option<Vec<PreparedTarget>>,
    prepared_at: Option<Instant>,
}
#[derive(Default)]
pub struct Operations {
    inner: Arc<Mutex<Work>>,
    sequence: AtomicU64,
}

fn entry_chunks(entries: Vec<Vec<u8>>) -> Result<Vec<SelectionChunk>> {
    let mut chunks = Vec::new();
    let mut current = Vec::new();
    for entry in entries {
        ensure!(!entry.is_empty(), "条目标识为空");
        ensure!(
            serde_json::to_vec(&entry)?.len() < MAX_FRAME - 1024,
            "单个条目标识超过通信上限"
        );
        current.push(entry);
        if serde_json::to_vec(&current)?.len() >= MAX_FRAME - 1024 {
            let last = current.pop().context("缺少条目")?;
            chunks.push(SelectionChunk::Entries(std::mem::take(&mut current)));
            current.push(last);
        }
    }
    if !current.is_empty() {
        chunks.push(SelectionChunk::Entries(current));
    }
    Ok(chunks)
}

#[tauri::command]
pub fn file_status(state: State<'_, Operations>) -> OperationStatus {
    state
        .inner
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .status
        .clone()
}
#[tauri::command]
pub fn file_cancel(state: State<'_, Operations>) {
    let mut work = state.inner.lock().unwrap_or_else(|e| e.into_inner());
    work.cancel.store(true, Ordering::Release);
    work.targets = None;
    work.status.close_requested = false;
    work.status.phase = if matches!(work.status.phase.as_str(), "executing" | "cancelling") {
        "cancelling"
    } else {
        "cancelled"
    }
    .into();
}

#[tauri::command]
pub async fn file_prepare(
    state: State<'_, Operations>,
    input: SelectionInput,
    records_only: Option<bool>,
) -> Result<PreparedSummary, String> {
    let records_only = records_only.unwrap_or(false);
    let id = state.sequence.fetch_add(1, Ordering::Relaxed) + 1;
    let shared = state.inner.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut work = shared.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(work.status.phase.as_str(), "executing" | "cancelling") {
            return Err("请等待当前文件操作结束".into());
        }
        work.cancel.store(true, Ordering::Release);
        *work = Work {
            records_only,
            status: OperationStatus {
                id,
                phase: "preparing".into(),
                ..Default::default()
            },
            cancel: cancel.clone(),
            ..Default::default()
        };
    }
    let mut backend = None;
    let result = async {
        ensure!(
            input.entries.len() <= MAX_SELECTED && input.positions.len() <= MAX_SELECTED,
            "选择过大，单次最多操作 10,000 项"
        );
        let response = ipc::request(
            crate::runtime::pipe(),
            if records_only {
                Action::FileRecordsBegin(input.origin)
            } else {
                Action::FileSelectionBegin(input.origin)
            },
        )
        .await?;
        let Response::SelectionStarted(selection) = response else {
            anyhow::bail!("文件操作准备响应无效")
        };
        backend = Some(selection);
        for chunk in entry_chunks(input.entries)?.into_iter().chain(
            input
                .positions
                .chunks(512)
                .map(|v| SelectionChunk::Positions(v.to_vec())),
        ) {
            ensure!(!cancel.load(Ordering::Acquire), "操作准备已取消");
            ipc::request(
                crate::runtime::pipe(),
                Action::FileSelectionAdd {
                    id: selection,
                    chunk,
                },
            )
            .await?;
        }
        let Response::SelectionCount(total) = ipc::request(
            crate::runtime::pipe(),
            Action::FileSelectionSeal { id: selection },
        )
        .await?
        else {
            anyhow::bail!("文件选择响应无效")
        };
        {
            let mut work = shared.lock().unwrap_or_else(|e| e.into_inner());
            if work.status.id == id {
                work.status.total = total;
            }
        }
        let mut rows = Vec::with_capacity(total);
        let mut offset = 0;
        loop {
            ensure!(!cancel.load(Ordering::Acquire), "操作准备已取消");
            let Response::SelectionPage(page) = ipc::request(
                crate::runtime::pipe(),
                Action::FileSelectionPage {
                    id: selection,
                    offset,
                },
            )
            .await?
            else {
                anyhow::bail!("文件选择分页响应无效")
            };
            rows.extend(page.rows);
            if let Some(next) = page.next {
                ensure!(next > offset, "文件操作分页未前进");
                offset = next;
            } else {
                break;
            }
        }
        ensure!(rows.len() == total, "文件选择不完整，请重新选择");
        if records_only {
            return Ok(rows);
        }
        let kept = top_level(rows.iter().map(|r| r.file.clone()).collect());
        let keys: std::collections::HashSet<_> = kept
            .into_iter()
            .map(|r| (r.reference.partition_id, r.reference.entry))
            .collect();
        rows.retain(|r| {
            keys.contains(&(
                r.file.reference.partition_id.clone(),
                r.file.reference.entry.clone(),
            ))
        });
        let progress = shared.clone();
        let cancelled = cancel.clone();
        tauri::async_runtime::spawn_blocking(move || -> Result<Vec<PreparedTarget>> {
            let mut failures = Vec::new();
            for (index, row) in rows.iter().enumerate() {
                ensure!(!cancelled.load(Ordering::Acquire), "操作准备已取消");
                if let Err(error) = verify(row, false) {
                    failures.push(format!(
                        "{} / {}：{error:#}",
                        row.partition_name,
                        String::from_utf16_lossy(&row.file.relative)
                    ));
                }
                let mut work = progress.lock().unwrap_or_else(|e| e.into_inner());
                if work.status.id == id {
                    work.status.phase = "validating".into();
                    work.status.processed = index + 1;
                    work.status.total = rows.len();
                }
            }
            ensure!(
                failures.is_empty(),
                "{} 项不可操作；未执行任何文件操作。\n{}",
                failures.len(),
                failures.into_iter().take(20).collect::<Vec<_>>().join("\n")
            );
            Ok(rows)
        })
        .await
        .context("文件核验任务中断")?
    }
    .await;
    if let Some(selection) = backend {
        let _ = ipc::request(
            crate::runtime::pipe(),
            Action::FileSelectionCancel { id: selection },
        )
        .await;
    }
    let mut work = shared.lock().unwrap_or_else(|e| e.into_inner());
    if work.status.id != id || cancel.load(Ordering::Acquire) {
        return Err("操作准备已取消".into());
    }
    match result {
        Ok(rows) => {
            let summary = PreparedSummary {
                id,
                count: rows.len(),
                names: rows
                    .iter()
                    .take(5)
                    .map(|r| {
                        format!(
                            "{} / {}",
                            r.partition_name,
                            String::from_utf16_lossy(&r.file.relative)
                        )
                    })
                    .collect(),
                same_parent: rows.iter().all(|r| {
                    r.file.reference.filesystem_id == rows[0].file.reference.filesystem_id
                        && r.file.parent == rows[0].file.parent
                }),
                single_file: rows.len() == 1 && !rows[0].file.folder,
            };
            work.targets = Some(rows);
            work.prepared_at = Some(Instant::now());
            work.status.phase = "ready".into();
            Ok(summary)
        }
        Err(error) => {
            let error = format!("{error:#}");
            work.status.phase = "failed".into();
            work.status.error = Some(error.clone());
            Err(error)
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileAction {
    CopyName,
    CopyPath,
    Copy,
    Cut,
    Drag,
    Open,
    OpenWith,
    Locate,
    Properties,
    Rename(String),
    Recycle,
    Delete,
    More { x: f64, y: f64 },
}

fn finish(
    shared: &Arc<Mutex<Work>>,
    id: u64,
    result: Result<String, String>,
) -> Result<String, String> {
    let mut work = shared.lock().unwrap_or_else(|e| e.into_inner());
    if work.status.id == id {
        work.status.phase = if work.cancel.load(Ordering::Acquire)
            || work.report.as_ref().is_some_and(|report| report.cancelled)
        {
            "cancelled"
        } else if result.is_ok() {
            "completed"
        } else {
            "failed"
        }
        .into();
        work.status.error = result.as_ref().err().cloned();
        work.status.close_requested = false;
    }
    result
}

async fn operation_worker(
    shared: Arc<Mutex<Work>>,
    id: u64,
    job: impl FnOnce() -> Result<OperationReport> + Send + 'static,
) -> Result<String, String> {
    let (send, receive) = tokio::sync::oneshot::channel();
    let progress = shared.clone();
    let launched = std::thread::Builder::new()
        .name("StashFinder file operation".into())
        .spawn(move || {
            let result = job()
                .and_then(|report| {
                    let summary = report.summary();
                    let success = report.success()
                        || (report.cancelled
                            && report.error.is_none()
                            && report
                                .items
                                .iter()
                                .all(|row| row.state != OutcomeState::Failed));
                    let mut details = report
                        .items
                        .iter()
                        .filter(|r| r.error.is_some())
                        .take(20)
                        .map(|r| {
                            format!("{}：{}", r.source, r.error.as_deref().unwrap_or_default())
                        })
                        .collect::<Vec<_>>();
                    if let Some(error) = &report.error {
                        details.push(error.clone());
                    }
                    let mut work = progress.lock().unwrap_or_else(|e| e.into_inner());
                    if work.status.id == id {
                        work.report = Some(report);
                    }
                    ensure!(success, "{summary}\n{}", details.join("\n"));
                    Ok(summary)
                })
                .map_err(|e| format!("{e:#}"));
            let _ = send.send(result);
        });
    let result = match launched {
        Ok(_) => receive.await.map_err(|e| e.to_string()).and_then(|r| r),
        Err(error) => Err(error.to_string()),
    };
    finish(&shared, id, result)
}

#[derive(Serialize)]
pub struct ReportPage {
    pub id: u64,
    pub total: usize,
    pub completed: usize,
    pub rows: Vec<ItemOutcome>,
    pub next: Option<usize>,
}
#[tauri::command]
pub fn file_report(
    state: State<'_, Operations>,
    id: Option<u64>,
    offset: usize,
) -> Result<Option<ReportPage>, String> {
    let work = state.inner.lock().unwrap_or_else(|e| e.into_inner());
    if id.is_some_and(|id| id != work.status.id) {
        return Err("文件操作结果已过期".into());
    }
    let Some(report) = &work.report else {
        return Ok(None);
    };
    if offset > report.items.len() {
        return Err("文件操作结果分页无效".into());
    }
    let rows = report
        .items
        .iter()
        .skip(offset)
        .take(50)
        .cloned()
        .collect::<Vec<_>>();
    let next = (offset + rows.len() < report.items.len()).then_some(offset + rows.len());
    Ok(Some(ReportPage {
        id: work.status.id,
        total: report.items.len(),
        completed: report.completed(),
        rows,
        next,
    }))
}
pub fn prevent_close(state: &Operations) -> bool {
    let mut work = state.inner.lock().unwrap_or_else(|e| e.into_inner());
    if matches!(work.status.phase.as_str(), "executing" | "cancelling") {
        work.status.close_requested = true;
        true
    } else {
        false
    }
}
#[tauri::command]
pub fn file_keep_open(state: State<'_, Operations>) {
    state
        .inner
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .status
        .close_requested = false;
}

#[tauri::command]
pub async fn file_run(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, Operations>,
    id: u64,
    action: FileAction,
) -> Result<String, String> {
    let owner = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let shared = state.inner.clone();
    let rows = {
        let mut work = shared.lock().unwrap_or_else(|e| e.into_inner());
        if work.status.id != id
            || work.status.phase != "ready"
            || work
                .prepared_at
                .is_none_or(|at| at.elapsed() > Duration::from_secs(300))
        {
            return Err("文件选择已过期，请重新操作".into());
        }
        let rows = work.targets.take().ok_or("文件选择不可用")?;
        if work.records_only != matches!(action, FileAction::CopyName | FileAction::CopyPath) {
            return Err("文件操作准备类型不一致".into());
        }
        work.status.phase = "executing".into();
        rows
    };
    let edit = match &action {
        FileAction::Rename(name) => Some(stashfinder_windows::file_operations::Edit::Rename(
            name.clone(),
        )),
        FileAction::Recycle => Some(stashfinder_windows::file_operations::Edit::Recycle),
        FileAction::Delete => Some(stashfinder_windows::file_operations::Edit::Delete),
        _ => None,
    };
    if let Some(edit) = edit {
        let cancel = shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cancel
            .clone();
        return operation_worker(shared, id, move || {
            let paths = rows
                .iter()
                .map(|r| verify(r, false))
                .collect::<Result<Vec<_>>>()?;
            let guards = rows.into_iter().map(Some).collect::<Vec<_>>();
            stashfinder_windows::file_operations::execute(
                &paths, &guards, None, &edit, owner, cancel,
            )
        })
        .await;
    }
    let (send, receive) = tokio::sync::oneshot::channel();
    let scheduled = app
        .run_on_main_thread(move || {
            let result = (|| -> Result<String> {
                if matches!(action, FileAction::CopyName | FileAction::CopyPath) {
                    let mut text = Vec::new();
                    for (index, row) in rows.iter().enumerate() {
                        if index > 0 {
                            text.extend([13, 10]);
                        }
                        if matches!(action, FileAction::CopyName) {
                            text.extend(
                                row.file.relative.rsplit(|c| *c == 92).next().unwrap_or(&[]),
                            );
                        } else {
                            let root = row
                                .mounts
                                .first()
                                .cloned()
                                .unwrap_or_else(|| format!("{} / ", row.partition_name));
                            text.extend(root.encode_utf16());
                            text.extend(&row.file.relative);
                        }
                    }
                    shell::copy_text(&text, owner)?;
                    return Ok("已复制记录文字".into());
                }
                let paths = rows
                    .iter()
                    .map(|row| verify(row, false))
                    .collect::<Result<Vec<_>>>()?;
                match action {
                    FileAction::Rename(_) | FileAction::Recycle | FileAction::Delete => {
                        unreachable!("handled by STA worker")
                    }
                    FileAction::CopyName | FileAction::CopyPath => unreachable!("handled above"),
                    FileAction::More { x, y } => {
                        if stashfinder_windows::context_menu::show(&paths, owner, x, y, scale)? {
                            Ok("已调用 Windows 菜单命令；索引将按实际变化更新".into())
                        } else {
                            Ok("已关闭 Windows 菜单".into())
                        }
                    }
                    FileAction::Copy => {
                        shell::copy_samples(&paths)?;
                        Ok("已复制到剪贴板".into())
                    }
                    FileAction::Cut => {
                        shell::cut_samples(&paths)?;
                        Ok("已剪切到剪贴板，源文件尚未移动".into())
                    }
                    FileAction::Drag => {
                        ensure!(shell::left_button_down(), "已松开鼠标，拖出已取消");
                        if shell::drag_samples(&paths)? {
                            Ok("目标已接受复制，请核对目标文件".into())
                        } else {
                            Ok("拖出已取消或目标未接受复制".into())
                        }
                    }
                    FileAction::Open
                    | FileAction::OpenWith
                    | FileAction::Locate
                    | FileAction::Properties => {
                        ensure!(paths.len() == 1, "此操作只支持选择一项");
                        let operation = match action {
                            FileAction::Open => shell::OpenAction::Open,
                            FileAction::OpenWith => shell::OpenAction::OpenWith,
                            FileAction::Locate => shell::OpenAction::Locate,
                            _ => shell::OpenAction::Properties,
                        };
                        shell::open_file(&paths[0], operation, owner)?;
                        Ok("已交给 Windows 打开".into())
                    }
                }
            })()
            .map_err(|e| format!("{e:#}"));
            let _ = send.send(result);
        })
        .map_err(|e| e.to_string());
    let result = match scheduled {
        Ok(()) => receive.await.map_err(|e| e.to_string()).and_then(|r| r),
        Err(error) => Err(error),
    };
    finish(&shared, id, result)
}

#[tauri::command]
pub async fn file_paste(
    window: tauri::WebviewWindow,
    state: State<'_, Operations>,
    target: EntryRef,
) -> Result<String, String> {
    let owner = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
    let shared = state.inner.clone();
    let id = state.sequence.fetch_add(1, Ordering::Relaxed) + 1;
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut work = shared.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(work.status.phase.as_str(), "executing" | "cancelling") {
            return Err("请等待当前文件操作结束".into());
        }
        work.cancel.store(true, Ordering::Release);
        *work = Work {
            status: OperationStatus {
                id,
                phase: "executing".into(),
                ..Default::default()
            },
            cancel: cancel.clone(),
            ..Default::default()
        };
    }
    let prepared = ipc::request(crate::runtime::pipe(), Action::FileDestination(target))
        .await
        .map_err(|e| format!("{e:#}"));
    let target = match prepared {
        Ok(Response::FileTarget(target)) => *target,
        Ok(_) => return finish(&shared, id, Err("粘贴目标响应无效".into())),
        Err(error) => return finish(&shared, id, Err(error)),
    };
    operation_worker(shared, id, move || {
        stashfinder_windows::file_operations::paste(target, owner, cancel)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunks_respect_wire_budget_without_truncating_selections() {
        let entries = (0..200).map(|i| vec![i as u8; 1000]).collect::<Vec<_>>();
        let chunks = entry_chunks(entries.clone()).unwrap();
        let mut actual = Vec::new();
        for chunk in chunks {
            assert!(serde_json::to_vec(&chunk).unwrap().len() < stashfinder_core::MAX_FRAME - 512);
            if let SelectionChunk::Entries(rows) = chunk {
                actual.extend(rows);
            }
        }
        assert_eq!(actual, entries);
        assert!(entry_chunks(vec![vec![255; stashfinder_core::MAX_FRAME]]).is_err());
    }
    #[tokio::test]
    async fn cancelling_a_windows_dialog_returns_a_cancelled_report_without_a_false_error() {
        let shared = Arc::new(Mutex::new(Work::default()));
        shared.lock().unwrap().status.id = 7;
        let result = operation_worker(shared.clone(), 7, || {
            Ok(OperationReport {
                items: vec![ItemOutcome {
                    index: 0,
                    source: "sample".into(),
                    state: OutcomeState::Skipped,
                    destination: None,
                    error: None,
                    note: Some("用户已取消，未完成".into()),
                }],
                cancelled: true,
                error: None,
            })
        })
        .await;
        assert!(result.unwrap().contains("操作已取消"));
        let work = shared.lock().unwrap();
        assert_eq!(work.status.phase, "cancelled");
        assert!(work.status.error.is_none());
        assert!(work.report.as_ref().unwrap().cancelled);
    }
    #[tokio::test]
    async fn partial_worker_results_are_retained_instead_of_reported_as_full_success() {
        let shared = Arc::new(Mutex::new(Work::default()));
        shared.lock().unwrap().status.id = 7;
        let result = operation_worker(shared.clone(), 7, || {
            Ok(OperationReport {
                items: vec![
                    ItemOutcome {
                        index: 0,
                        source: "first".into(),
                        state: OutcomeState::Completed,
                        destination: None,
                        error: None,
                        note: None,
                    },
                    ItemOutcome {
                        index: 1,
                        source: "second".into(),
                        state: OutcomeState::Failed,
                        destination: None,
                        error: Some("拒绝访问".into()),
                        note: None,
                    },
                ],
                cancelled: false,
                error: None,
            })
        })
        .await;
        assert!(result.unwrap_err().contains("完成 1 项，失败 1 项"));
        assert_eq!(
            shared.lock().unwrap().report.as_ref().unwrap().completed(),
            1
        );
    }
}
