use super::*;
use crate::file_operations::{self as ops, FileAction, SelectionInput};
use anyhow::{Context, bail};
use stashfinder_core::{
    browse::DirectoryRequest,
    file_operations::{EntryRef, MAX_SELECTED, SelectionOrigin},
    index::{SearchHit, SearchRow},
};

#[derive(Clone)]
pub(super) struct FileRecord {
    pub source: Volume,
    pub row: SearchRow,
    pub reference: EntryRef,
    pub online: bool,
    pub partial: bool,
    pub history: bool,
}
impl FileRecord {
    fn search(result: &SearchHit) -> Self {
        Self {
            source: Volume {
                id: result.partition_id.clone(),
                filesystem: result.filesystem_id.clone(),
                name: result.partition_name.clone(),
            },
            reference: EntryRef {
                partition_id: result.partition_id.clone(),
                filesystem_id: result.filesystem_id.clone(),
                generation: result.generation.to_string(),
                revision: result.revision.to_string(),
                entry: result.file.entry.clone(),
            },
            row: result.file.clone(),
            online: result.connection == "online",
            partial: result.partial,
            history: result.connection == "historical",
        }
    }
}

enum Input {
    Ready(SelectionInput),
    Directory {
        input: SelectionInput,
        directory: Box<DirectoryRequest>,
        missing: Vec<usize>,
    },
}

impl Input {
    async fn resolve(self) -> Result<SelectionInput> {
        let Self::Directory {
            mut input,
            directory,
            missing,
        } = self
        else {
            let Self::Ready(input) = self else {
                unreachable!("known selection")
            };
            return Ok(input);
        };
        for [start, end] in spans(&missing) {
            let mut request = *directory.clone();
            request.offset = u32::try_from(start).context("选择超出目录位置范围")?;
            let (_, rows) = files::read_directory(request, end).await?;
            ensure!(rows.len() >= end - start, "选择已变化，请刷新目录");
            input
                .entries
                .extend(rows.into_iter().take(end - start).map(|row| row.entry));
        }
        Ok(input)
    }
}

fn spans(positions: &[usize]) -> Vec<[usize; 2]> {
    let mut spans: Vec<[usize; 2]> = vec![];
    for &position in positions {
        match spans.last_mut() {
            Some([_, end]) if *end == position => *end += 1,
            _ => spans.push([position, position + 1]),
        }
    }
    spans
}

impl Desktop {
    pub(super) fn record(&self, i: usize) -> Option<FileRecord> {
        let tabs = self.tabs.borrow();
        match &tabs.active().pane {
            Page::Search(search) => search.rows.get(&i).map(FileRecord::search),
            Page::Browse(browse) => {
                let source = browse.location()?.volume.clone();
                let directory = browse.directory.as_ref()?;
                let row = directory.records.get(&i)?.clone();
                let online = self.snapshot.borrow().assets.iter().any(|a| {
                    a.registration.partition_id == source.id
                        && a.registration.filesystem_id == source.filesystem
                        && a.connection == "online"
                        && a.error.is_none()
                });
                Some(FileRecord {
                    reference: EntryRef {
                        partition_id: source.id.clone(),
                        filesystem_id: source.filesystem.clone(),
                        generation: directory.generation.clone(),
                        revision: directory.revision.clone(),
                        entry: row.entry.clone(),
                    },
                    source,
                    row,
                    online,
                    partial: directory.partial,
                    history: false,
                })
            }
            _ => None,
        }
    }
    fn destination(&self) -> Option<EntryRef> {
        let tabs = self.tabs.borrow();
        let Page::Browse(b) = &tabs.active().pane else {
            return None;
        };
        let place = b.location()?;
        let d = b.directory.as_ref()?;
        if d.partial || tabs.active().pending {
            return None;
        }
        let asset = self.snapshot.borrow();
        if asset.error.is_some()
            || !asset.assets.iter().any(|a| {
                a.connection == "online"
                    && a.error.is_none()
                    && a.registration.partition_id == place.volume.id
                    && a.registration.filesystem_id == place.volume.filesystem
            })
        {
            return None;
        }
        Some(EntryRef {
            partition_id: place.volume.id.clone(),
            filesystem_id: place.volume.filesystem.clone(),
            generation: d.generation.clone(),
            revision: d.revision.clone(),
            entry: d.location.clone(),
        })
    }
    fn input(&self, selection: Selection) -> Result<Input> {
        let tabs = self.tabs.borrow();
        match &tabs.active().pane {
            Page::Search(s) => {
                let id = s.id.context("请先完成搜索")?;
                Ok(Input::Ready(SelectionInput {
                    origin: SelectionOrigin::Search {
                        id,
                        range: matches!(selection, Selection::All).then_some([0, s.total]),
                    },
                    entries: vec![],
                    positions: if let Selection::Positions(rows) = selection {
                        rows
                    } else {
                        vec![]
                    },
                }))
            }
            Page::Browse(b) => {
                let place = b.location().context("请选择分区")?;
                let d = b.directory.as_ref().context("请先读取目录")?;
                let all = matches!(selection, Selection::All);
                let mut input = SelectionInput {
                    origin: SelectionOrigin::Directory {
                        all,
                        context: EntryRef {
                            partition_id: place.volume.id.clone(),
                            filesystem_id: place.volume.filesystem.clone(),
                            generation: d.generation.clone(),
                            revision: d.revision.clone(),
                            entry: d.location.clone(),
                        },
                        filter: if all {
                            b.filter.clone().map(Box::new)
                        } else {
                            None
                        },
                    },
                    entries: vec![],
                    positions: vec![],
                };
                if let Selection::Positions(positions) = selection {
                    let mut missing = Vec::new();
                    for i in positions {
                        if let Some(row) = d.records.get(&i) {
                            input.entries.push(row.entry.clone());
                        } else {
                            missing.push(i);
                        }
                    }
                    if !missing.is_empty() {
                        return Ok(Input::Directory {
                            input,
                            missing,
                            directory: Box::new(
                                files::directory_request(b, true).context("目录状态不可用")?,
                            ),
                        });
                    }
                }
                Ok(Input::Ready(input))
            }
            _ => bail!("当前页没有文件选区"),
        }
    }
    pub(super) fn may_operate(&self, id: usize) -> bool {
        if matches!(&self.tabs.borrow().active().pane, Page::Browse(b) if b.stale) {
            return id == files::PROPERTIES;
        }
        if id == files::PASTE {
            return self.destination().is_some();
        }
        let count = send(self.list, LVM_GETSELECTEDCOUNT, 0, 0).max(0) as usize;
        if count == 0 {
            return false;
        }
        if id == files::PROPERTIES {
            return true;
        }
        if count > MAX_SELECTED {
            return false;
        }
        let selected = self.single().and_then(|i| self.record(i));
        if let Some(record) = &selected {
            return match id {
                files::OPEN => record.online || record.row.kind == "folder",
                files::OPEN_WITH => record.online && record.row.kind != "folder",
                files::BROWSE_PARENT | files::COPY_NAME | files::COPY_PATH => true,
                _ => record.online,
            };
        }
        !matches!(
            id,
            files::OPEN | files::OPEN_WITH | files::BROWSE_PARENT | files::LOCATE | files::RENAME
        )
    }
    pub(super) fn file_operation_active(&self) -> bool {
        matches!(
            ops::operation_status(&self.operation).phase.as_str(),
            "preparing" | "validating" | "executing" | "cancelling"
        )
    }
    pub(super) fn operate(&self, command: usize) {
        let home = matches!(self.tabs.borrow().active().pane, Page::Overview);
        if home && command == files::OPEN {
            if let Some(asset) = self.selected_asset() {
                self.new_browse(Some(Location {
                    volume: Volume::from(&asset),
                    parent: None,
                }));
            }
            return;
        }
        if matches!(
            self.tabs.borrow().active().pane,
            Page::Overview | Page::Tasks
        ) || !self.may_operate(command)
        {
            return;
        }
        let owner = self.hwnd.0 as usize;
        let state = self.operation.clone();
        if command == files::PASTE {
            let Some(target) = self.destination() else {
                return;
            };
            self.spawn(
                async move { ops::paste(&state, target, owner).await },
                |app, result| app.operation_done(result, true),
            );
            return;
        }
        let item = self.single().and_then(|i| self.record(i));
        if let Some(item) = &item {
            if command == files::BROWSE_PARENT {
                self.open_browse(
                    Some(Location {
                        volume: item.source.clone(),
                        parent: Some(item.row.parent.clone()),
                    }),
                    Some(item.row.entry.clone()),
                );
                return;
            }
            if command == files::OPEN && item.row.kind == "folder" {
                let place = Location {
                    volume: item.source.clone(),
                    parent: Some(item.row.entry.clone()),
                };
                if matches!(self.tabs.borrow().active().pane, Page::Browse(_)) {
                    self.visit(place);
                } else {
                    self.new_browse(Some(place));
                }
                return;
            }
        }
        if command == files::PROPERTIES && item.as_ref().is_none_or(|r| !r.online) {
            self.tabs.borrow_mut().active_mut().preview = true;
            self.update_preview();
            self.layout();
            return;
        }
        let selection = match self.input(self.selection()) {
            Ok(input) => input,
            Err(e) => {
                self.status(&format!("无法准备选区：{e:#}"));
                return;
            }
        };
        let records = matches!(command, files::COPY_PATH | files::COPY_NAME);
        let origin = {
            let t = self.tabs.borrow();
            (t.active().id, t.active().epoch)
        };
        let point = self.local_cursor();
        self.status("正在核验文件选区…");
        self.spawn(
            async move {
                let input = selection.resolve().await.map_err(|e| format!("{e:#}"))?;
                ops::prepare(&state, input, records).await
            },
            move |app, result| {
                let prepared = match result {
                    Ok(p) => p,
                    Err(e) => {
                        app.operation_done(Err(e), false);
                        return;
                    }
                };
                let unchanged = {
                    let t = app.tabs.borrow();
                    (t.active().id, t.active().epoch) == origin
                };
                if !unchanged || ops::operation_status(&app.operation).id != prepared.id {
                    return;
                }
                let mut refresh = false;
                let action = match command {
                    files::OPEN => FileAction::Open,
                    files::OPEN_WITH if prepared.single_file => FileAction::OpenWith,
                    files::LOCATE => FileAction::Locate,
                    files::COPY => FileAction::Copy,
                    files::CUT => FileAction::Cut,
                    files::COPY_NAME => FileAction::CopyName,
                    files::COPY_PATH => FileAction::CopyPath,
                    files::DRAG => FileAction::Drag,
                    files::PROPERTIES => FileAction::Properties,
                    files::MORE if prepared.same_parent => FileAction::More {
                        x: point.x as f64,
                        y: point.y as f64,
                    },
                    files::RENAME => {
                        let original = item
                            .as_ref()
                            .map(|i| i.row.name.clone())
                            .unwrap_or_default();
                        let Some(mut form) =
                            app.ask("重命名", &[Field::Edit("名称", original.clone())])
                        else {
                            return;
                        };
                        let name = form.text();
                        if name == original {
                            return;
                        }
                        refresh = true;
                        FileAction::Rename(name)
                    }
                    files::RECYCLE | files::DELETE => {
                        let permanent = command == files::DELETE;
                        let title = if permanent {
                            "永久删除"
                        } else {
                            "移入回收站"
                        };
                        let message = format!(
                            "{title} {} 项？\n\n{}",
                            prepared.count,
                            prepared.names.join("\n")
                        );
                        if !confirm(app.hwnd, title, &message) {
                            return;
                        }
                        refresh = true;
                        if permanent {
                            FileAction::Delete
                        } else {
                            FileAction::Recycle
                        }
                    }
                    _ => {
                        app.status("此操作不适用于当前选区");
                        return;
                    }
                };
                let state = app.operation.clone();
                let sender = app.sender.clone();
                app.spawn(
                    ops_run(state, prepared.id, action, owner, sender),
                    move |app, result| app.operation_done(result, refresh),
                );
            },
        );
    }
    fn operation_done(&self, result: Result<String, String>, refresh: bool) {
        match result {
            Ok(value) => {
                if refresh {
                    self.manual_refresh();
                }
                self.status(&value);
            }
            Err(error) => self.status(&format!("文件操作未完成：{error}")),
        }
    }
    fn local_cursor(&self) -> POINT {
        let mut point = POINT::default();
        // SAFETY: Reads and converts the cursor point for a window on this thread.
        unsafe {
            let _ = GetCursorPos(&mut point);
            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(self.hwnd, &mut point);
        }
        point
    }
    pub(super) fn cancel_operation(&self) {
        ops::cancel(&self.operation);
        self.status("正在取消文件操作…");
    }
    pub(super) fn operation_report(&self) {
        let mut offset = 0;
        loop {
            let report = match ops::report(&self.operation, None, offset) {
                Ok(Some(report)) => report,
                Ok(None) => {
                    info(self.hwnd, "文件操作结果", "暂无操作结果。");
                    return;
                }
                Err(e) => {
                    info(self.hwnd, "文件操作结果", &e);
                    return;
                }
            };
            let mut lines = report
                .rows
                .iter()
                .map(|row| {
                    use stashfinder_core::file_operations::OutcomeState;
                    let status = match row.state {
                        OutcomeState::Completed => "已完成",
                        OutcomeState::Failed => "失败",
                        OutcomeState::Skipped => "已跳过",
                        OutcomeState::NotRun => "未执行",
                    };
                    format!(
                        "{status} · {}{}",
                        row.source,
                        row.error
                            .as_deref()
                            .or(row.destination.as_deref())
                            .or(row.note.as_deref())
                            .map_or(String::new(), |s| format!(" · {s}"))
                    )
                })
                .collect::<Vec<_>>();
            if lines.is_empty() {
                lines.push("暂无明细".into());
            }
            let title = format!("文件操作结果 · {} / {}", report.completed, report.total);
            let mut details = vec![Field::Choice("逐项结果", lines, 0)];
            if report.next.is_some() {
                details.push(Field::Check("查看下一页", false));
            }
            let Some(mut form) = self.ask(&title, &details) else {
                return;
            };
            form.choice();
            if let Some(next) = report.next
                && form.check()
            {
                offset = next;
            } else {
                return;
            }
        }
    }
    pub(super) fn guard_close(&self) -> bool {
        if self.exporting.cancel() {
            self.status("正在取消导出…");
            return true;
        }
        if ops::prevent_close(&self.operation) {
            if confirm(
                self.hwnd,
                "文件操作仍在进行",
                "取消当前文件操作？可以在操作结束后退出。",
            ) {
                self.cancel_operation();
            } else {
                ops::keep_open(&self.operation);
            }
            return true;
        }
        false
    }
    pub(super) fn export(&self) {
        let (id, total) = {
            let tabs = self.tabs.borrow();
            let Page::Search(s) = &tabs.active().pane else {
                return;
            };
            let Some(id) = s.id.filter(|_| s.total > 0 && !tabs.active().pending) else {
                drop(tabs);
                self.status("搜索完成并有结果后可以导出");
                return;
            };
            (id, s.total)
        };
        // Reserve the search while the modal loops are processing background notifications.
        let active = match self.exporting.begin() {
            Ok(active) => active,
            Err(error) => {
                self.status(&error);
                return;
            }
        };
        let selection = self.selection();
        let ranges = match selection {
            Selection::Positions(rows) => spans(&rows),
            Selection::All => vec![[0, total]],
            Selection::None => vec![],
        };
        let scope = if ranges.is_empty() {
            vec!["全部结果".into()]
        } else {
            vec!["全部结果".into(), "所选结果".into()]
        };
        let Some(mut fields) = self.ask("导出搜索结果", &[Field::Choice("导出范围", scope, 0)])
        else {
            return;
        };
        let ranges = if fields.choice() == 0 {
            vec![[0, total]]
        } else {
            ranges
        };
        let destination =
            match stashfinder_windows::shell::choose_csv_destination(self.hwnd.0 as usize) {
                Ok(Some(path)) => path,
                Ok(None) => return,
                Err(e) => {
                    info(self.hwnd, "无法选择导出位置", &format!("{e:#}"));
                    return;
                }
            };
        self.exported_snapshot.set(Some(id));
        let sender = self.sender.clone();
        let hwnd = self.hwnd.0 as usize;
        self.spawn(
            async move {
                let result = crate::search_export::write_csv(
                    &destination,
                    id,
                    total,
                    ranges,
                    &active.cancel,
                    runtime::request,
                    |n, total| {
                        let _ = post(
                            &sender,
                            hwnd,
                            Box::new(move |app| app.status(&format!("正在导出 {n} / {total} 项"))),
                        );
                    },
                )
                .await;
                (active, result)
            },
            move |app, (active, result)| {
                app.exported_snapshot.set(None);
                if !app
                    .tabs
                    .borrow()
                    .items
                    .iter()
                    .any(|tab| matches!(&tab.pane, Page::Search(s) if s.id == Some(id)))
                {
                    app.cancel_snapshot(id);
                }
                if active.cancel.load(std::sync::atomic::Ordering::Acquire) && result.is_err() {
                    app.status("已取消 CSV 导出");
                } else {
                    match result {
                        Ok(count) => app.status(&format!("已导出 {count} 项")),
                        Err(e) => app.status(&format!("无法完成 CSV 导出：{e:#}")),
                    }
                }
            },
        );
    }
    pub(super) fn defer_preview(&self) {
        self.preview_due.set(true);
    }
}

async fn ops_run(
    state: std::sync::Arc<ops::Operations>,
    id: u64,
    action: FileAction,
    owner: usize,
    sender: Sender<Callback>,
) -> Result<String, String> {
    ops::run_prepared(&state, id, action, owner, 1.0, move |job| {
        post(&sender, owner, Box::new(move |_| job()))
    })
    .await
}
fn post(sender: &Sender<Callback>, hwnd: usize, callback: Callback) -> Result<(), String> {
    sender.send(callback).map_err(|e| e.to_string())?;
    // SAFETY: Posts a value-only wakeup for a closure carried by the Rust channel.
    unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), DRAIN, WPARAM(0), LPARAM(0)) }
        .map_err(|e| e.to_string())
}
