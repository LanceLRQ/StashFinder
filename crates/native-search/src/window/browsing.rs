use super::*;

type Columns = &'static [(&'static str, i32, Option<SortField>)];
pub(super) const TASK_COLUMNS: Columns = &[
    ("分区", 190, None),
    ("任务", 130, None),
    ("状态", 145, None),
    ("执行阶段", 215, None),
    ("更新时间", 180, None),
];
pub(super) const CANDIDATE_COLUMNS: Columns = &[
    ("分区名称", 200, None),
    ("挂载点", 145, None),
    ("所属硬盘", 225, None),
    ("文件系统", 100, None),
    ("登记状态", 250, None),
];
pub(super) const PARTITION_COLUMNS: Columns = &[
    ("分区名称", 225, None),
    ("所属硬盘", 250, None),
    ("文件系统", 110, None),
    ("登记状态", 155, None),
    ("连接状态", 110, None),
];
pub(super) const FOLDER_COLUMNS: Columns = &[
    ("名称", 255, None),
    ("卷内路径", 375, None),
    ("类型", 105, None),
    ("大小（字节）", 125, None),
];

impl Window {
    pub(super) fn browse_command(&self, command: usize) {
        let mut refresh = false;
        let target = match command {
            REFRESH if self.viewing_tasks() => {
                self.refresh_tasks();
                return;
            }
            PARTITIONS => Some(Location::Partitions),
            UP => self.browser.borrow().as_ref().and_then(Browser::up),
            PREVIOUS | NEXT => self
                .browser
                .borrow()
                .as_ref()
                .and_then(|b| b.page(command == NEXT)),
            REFRESH => {
                refresh = true;
                self.browser.borrow().as_ref().map(|b| b.location.clone())
            }
            SEARCH => {
                self.return_to_search();
                return;
            }
            _ => None,
        };
        if let Some(target) = target {
            self.browse_to(target, refresh);
        }
    }

    pub(super) fn selected_row(&self) -> Option<usize> {
        // SAFETY: Both list-view queries return scalar selection indices.
        let row = unsafe {
            if SendMessageW(self.list, LVM_GETSELECTEDCOUNT, None, None).0 != 1 {
                return None;
            }
            SendMessageW(
                self.list,
                LVM_GETNEXTITEM,
                Some(WPARAM(usize::MAX)),
                Some(LPARAM(LVNI_SELECTED as isize)),
            )
            .0
        };
        usize::try_from(row).ok()
    }

    pub(super) fn selected_folder(&self) -> Option<Location> {
        let row = self.selected_row()?;
        self.browser.borrow().as_ref()?.enter(row)
    }

    pub(super) fn enter_selected_folder(&self) {
        if let Some(folder) = self.selected_folder() {
            self.browse_to(folder, false);
        }
    }

    pub(super) fn browse_to(&self, mut target: Location, refresh: bool) {
        self.index.take();
        self.focus_partition.take();
        self.task_notice.take();
        if refresh
            && let Location::Folder {
                request, previous, ..
            } = &mut target
        {
            request.offset = 0;
            previous.clear();
        }
        if let Some(task) = self.browse_task.take() {
            task.abort();
        }
        let changed_columns =
            self.browser.borrow().as_ref().is_none_or(|b| {
                std::mem::discriminant(&b.location) != std::mem::discriminant(&target)
            });
        let restore_search = self.browser.borrow().as_ref().map_or_else(
            // SAFETY: The edit belongs to this thread and lives for the lifetime of the window.
            || self.requests.borrow().is_some() || unsafe { GetWindowTextLengthW(self.edit) } > 0,
            |b| b.restore_search,
        );
        self.generation.set(self.generation.get() + 1);
        self.composing.set(false);
        self.requests.send_replace(None);
        self.browser.replace(Some(Browser {
            location: target.clone(),
            listing: None,
            restore_search,
        }));
        self.clear();
        self.set_browse_controls(true, changed_columns);
        let caption = match &target {
            Location::Partitions => "已登记分区",
            Location::Candidates => "登记分区",
            Location::Tasks => "索引任务",
            Location::Folder { name, .. } => name,
        };
        // SAFETY: The text is copied synchronously and these are this window's own controls.
        unsafe {
            let _ = KillTimer(Some(self.hwnd), DEBOUNCE);
            let _ = SetWindowTextW(self.path, &HSTRING::from(caption));
            let _ = SetFocus(Some(self.list));
        }
        self.status(match target {
            Location::Candidates => "正在读取可登记分区…",
            Location::Tasks => "正在读取索引任务…",
            _ => "正在读取索引…",
        });
        self.load_listing(target, refresh);
    }

    pub(super) fn load_listing(&self, target: Location, refresh: bool) {
        let endpoint = self.endpoint.clone();
        let send = self.send.clone();
        let hwnd = self.hwnd.0 as usize;
        let generation = self.generation.get();
        self.browse_task
            .replace(Some(self.runtime.spawn(async move {
                let result = target.load(&endpoint, refresh).await;
                notify(&send, hwnd, Update::Browse(generation, result));
            })));
    }

    pub(super) fn display_browse(&self, result: Result<Listing>) {
        self.browse_task.take();
        let tasks = self.viewing_tasks();
        match result {
            Ok(listing) => {
                let focus = match (&listing, self.focus_partition.take()) {
                    (Listing::Tasks(new), _) => self.retain_task_selection(new),
                    (Listing::Partitions(snapshot), Some(id)) => snapshot
                        .assets
                        .iter()
                        .position(|a| a.registration.partition_id == id),
                    _ => None,
                };
                let Some((path, status, count)) =
                    self.browser.borrow_mut().as_mut().map(|b| b.apply(listing))
                else {
                    return;
                };
                // SAFETY: List rows come from the stored page before the count is published.
                unsafe {
                    let _ = SetWindowTextW(self.path, &HSTRING::from(path));
                    SendMessageW(
                        self.list,
                        LVM_SETITEMCOUNT,
                        Some(WPARAM(count)),
                        Some(LPARAM(if tasks { LVSICF_NOSCROLL as isize } else { 0 })),
                    );
                    if tasks && count != 0 {
                        SendMessageW(
                            self.list,
                            LVM_REDRAWITEMS,
                            Some(WPARAM(0)),
                            Some(LPARAM((count - 1) as isize)),
                        );
                    }
                }
                self.status(&status);
                if let Some(row) = focus {
                    self.set_selected_row(Some(row));
                    if !tasks {
                        // SAFETY: The selected partition is a row from the current listing.
                        unsafe {
                            SendMessageW(self.list, LVM_ENSUREVISIBLE, Some(WPARAM(row)), None)
                        };
                    }
                }
            }
            Err(error) if tasks => {
                let showing_last = if let Some(Listing::Tasks(list)) = self
                    .browser
                    .borrow_mut()
                    .as_mut()
                    .and_then(|b| b.listing.as_mut())
                {
                    list.stale = true;
                    "，显示上次读取的任务"
                } else {
                    ""
                };
                self.status(&format!(
                    "无法读取索引任务：{error:#}{showing_last}（F5 刷新）"
                ));
            }
            Err(error) => self.status(&format!("无法读取：{error:#}（F5 刷新）")),
        }
        self.update_browse_buttons();
    }

    pub(super) fn set_selected_row(&self, row: Option<usize>) {
        let mask = LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0);
        let item = LVITEMW {
            stateMask: mask,
            state: if row.is_some() {
                mask
            } else {
                Default::default()
            },
            ..Default::default()
        };
        // SAFETY: The list synchronously copies the state. Index -1 clears all row selections.
        unsafe {
            SendMessageW(
                self.list,
                LVM_SETITEMSTATE,
                Some(WPARAM(row.unwrap_or(usize::MAX))),
                Some(LPARAM(&item as *const _ as isize)),
            )
        };
    }

    pub(super) fn return_to_search(&self) {
        let Some(browser) = self.browser.take() else {
            return;
        };
        if let Some(task) = self.browse_task.take() {
            task.abort();
        }
        self.index.take();
        self.focus_partition.take();
        self.task_notice.take();
        self.generation.set(self.generation.get() + 1);
        self.clear();
        self.set_browse_controls(false, true);
        if browser.restore_search {
            self.submit();
        } else {
            self.status("输入关键词搜索");
        }
        // SAFETY: The edit has been made visible again.
        let _ = unsafe { SetFocus(Some(self.edit)) };
    }

    fn set_browse_controls(&self, browse: bool, changed_columns: bool) {
        let tasks = self.viewing_tasks();
        let (partitions, candidates) = {
            let b = self.browser.borrow();
            (
                b.as_ref().is_some_and(|b| b.location.is_partitions()),
                b.as_ref()
                    .is_some_and(|b| matches!(b.location, Location::Candidates)),
            )
        };
        // SAFETY: Only the window thread changes its controls or column descriptors.
        unsafe {
            let _ = SetWindowTextW(
                self.hwnd,
                if tasks {
                    w!("StashFinder · 索引任务")
                } else if browse {
                    w!("StashFinder · 分区与目录")
                } else {
                    w!("StashFinder · 快速搜索")
                },
            );
            let _ = SetWindowTextW(
                self.list,
                if tasks {
                    w!("索引任务")
                } else if browse {
                    w!("分区与目录")
                } else {
                    w!("搜索结果")
                },
            );
            let _ = ShowWindow(self.edit, if browse { SW_HIDE } else { SW_SHOW });
            let _ = SetWindowTextW(
                self.register,
                if candidates {
                    w!("登记所选分区")
                } else {
                    w!("登记分区…")
                },
            );
            for (child, visible) in [
                (self.path, browse),
                (self.up, browse && !partitions),
                (
                    self.previous,
                    browse && !partitions && !candidates && !tasks,
                ),
                (self.next, browse && !partitions && !candidates && !tasks),
                (self.register, partitions || candidates),
                (self.auto_index, candidates),
                (self.start_index, partitions),
                (self.cancel_task, tasks),
                (self.retry_task, tasks),
            ] {
                let _ = ShowWindow(child, if visible { SW_SHOW } else { SW_HIDE });
            }
            if tasks {
                SetTimer(Some(self.hwnd), TASK_POLL, 1500, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TASK_POLL);
            }
            if changed_columns {
                while SendMessageW(self.list, LVM_DELETECOLUMN, Some(WPARAM(0)), None).0 != 0 {}
                self.columns(true, true);
            }
        }
        self.update_browse_buttons();
        self.layout();
    }

    fn update_browse_buttons(&self) {
        let (up, previous, next) = {
            let browser = self.browser.borrow();
            let Some(browser) = browser.as_ref() else {
                return;
            };
            (
                !browser.location.is_partitions(),
                matches!((&browser.location, &browser.listing), (Location::Folder { previous, .. }, Some(Listing::Folder(_))) if !previous.is_empty()),
                matches!(&browser.listing, Some(Listing::Folder(page)) if page.next.is_some()),
            )
        };
        // SAFETY: No state borrow is held while Win32 sends focus notifications.
        unsafe {
            let _ = EnableWindow(self.up, up);
            let _ = EnableWindow(self.previous, previous);
            let _ = EnableWindow(self.next, next);
        }
        self.update_selection();
    }
}
