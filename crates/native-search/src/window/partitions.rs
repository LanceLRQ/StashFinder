use super::*;
use crate::registration::{self, accepted, index_label, selectable, start_mode, volume_name};
use anyhow::bail;
use stashfinder_core::{
    Action, Response,
    assets::{Asset, Candidate},
    index::IndexMode,
};
use stashfinder_windows::ipc;
use std::time::Duration;

pub(super) struct IndexWatch {
    partition: String,
    mode: Option<IndexMode>,
    task: JoinHandle<()>,
}

impl Drop for IndexWatch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Window {
    pub(super) fn open_candidates(&self) {
        // SAFETY: This is the window's own auto-check button.
        unsafe {
            SendMessageW(
                self.auto_index,
                BM_SETCHECK,
                Some(WPARAM(BST_CHECKED.0 as usize)),
                None,
            )
        };
        self.browse_to(Location::Candidates, true);
    }

    fn selected_candidate(&self) -> Option<(u64, Candidate)> {
        let row = self.selected_row()?;
        let browser = self.browser.borrow();
        let Listing::Candidates(snapshot) = browser.as_ref()?.listing.as_ref()? else {
            return None;
        };
        let candidate = snapshot.candidates.get(row)?;
        (snapshot.error.is_none() && selectable(candidate))
            .then(|| (snapshot.version, candidate.clone()))
    }

    fn selected_asset(&self) -> Option<Asset> {
        let row = self.selected_row()?;
        let browser = self.browser.borrow();
        let Listing::Partitions(snapshot) = browser.as_ref()?.listing.as_ref()? else {
            return None;
        };
        snapshot.assets.get(row).cloned()
    }

    pub(super) fn register_selected(&self) {
        if self.write_task.borrow().is_some() {
            return;
        }
        if self
            .browser
            .borrow()
            .as_ref()
            .is_some_and(|b| b.location.is_partitions())
        {
            self.open_candidates();
            return;
        }
        let Some((version, candidate)) = self.selected_candidate() else {
            return;
        };
        // SAFETY: The modal dialog belongs to this window and defaults to declining.
        let confirmed = candidate.weak
            && unsafe {
                MessageBoxW(
                    Some(self.hwnd),
                    &HSTRING::from(format!(
                        "分区「{}」身份信息不足，无法可靠匹配已有记录。仍按新分区登记？",
                        volume_name(&candidate.observation)
                    )),
                    w!("确认登记分区"),
                    MB_YESNO | MB_DEFBUTTON2 | MB_ICONWARNING,
                ) == IDYES
            };
        if candidate.weak && !confirmed {
            return;
        }
        // SAFETY: BM_GETCHECK returns the scalar check state of this checkbox.
        let automatic = unsafe {
            SendMessageW(self.auto_index, BM_GETCHECK, None, None).0 == BST_CHECKED.0 as isize
        };
        let pipe = self.endpoint.pipe().to_owned();
        let generation = self.generation.get();
        let send = self.send.clone();
        let target = self.hwnd.0 as usize;
        self.write_task.replace(Some(self.runtime.spawn(async move {
            let result =
                registration::register(&pipe, version, &candidate, confirmed, automatic).await;
            notify(&send, target, Update::Registered(generation, result));
        })));
        self.update_partition_selection();
        self.status("正在登记所选分区…");
    }

    pub(super) fn registration_finished(&self, generation: u64, result: Result<String>) {
        self.write_task.take();
        self.update_partition_selection();
        if generation != self.generation.get() {
            return;
        }
        match result {
            Ok(partition) => {
                self.browse_to(Location::Partitions, false);
                self.focus_partition.replace(Some(partition));
            }
            Err(error) => self.status(&format!("登记结果：{error:#}（F5 刷新）")),
        }
    }

    pub(super) fn update_partition_selection(&self) {
        let asset = self.selected_asset();
        let previous = self.index.borrow().as_ref().map(|i| i.partition.clone());
        if previous.as_ref() != asset.as_ref().map(|a| &a.registration.partition_id) {
            self.index.take();
            if let Some(asset) = &asset {
                let partition = asset.registration.partition_id.clone();
                let id = partition.clone();
                let pipe = self.endpoint.pipe().to_owned();
                let send = self.send.clone();
                let target = self.hwnd.0 as usize;
                let task = self.runtime.spawn(async move {
                    loop {
                        let result = ipc::request(
                            &pipe,
                            Action::IndexState {
                                partition_id: id.clone(),
                            },
                        )
                        .await
                        .and_then(|reply| match reply {
                            Response::Index(status) => Ok(status),
                            _ => bail!("索引状态响应无效"),
                        });
                        notify(&send, target, Update::Index(id.clone(), result));
                        tokio::time::sleep(Duration::from_millis(1500)).await;
                    }
                });
                self.index.replace(Some(IndexWatch {
                    partition,
                    mode: None,
                    task,
                }));
                self.status("正在读取分区索引状态…");
            } else if let Some(Listing::Partitions(snapshot)) = self
                .browser
                .borrow()
                .as_ref()
                .and_then(|b| b.listing.as_ref())
            {
                self.status(&format!("{} 个已登记分区", snapshot.assets.len()));
            }
        }
        let partitions = self
            .browser
            .borrow()
            .as_ref()
            .is_some_and(|b| b.location.is_partitions());
        let can_register = partitions || self.selected_candidate().is_some();
        let mode = self.index.borrow().as_ref().and_then(|i| i.mode);
        let idle = self.write_task.borrow().is_none();
        // SAFETY: All controls belong to the current window thread.
        unsafe {
            let _ = EnableWindow(self.register, idle && can_register);
            let _ = EnableWindow(self.auto_index, idle);
            let _ = EnableWindow(self.start_index, idle && mode.is_some());
            let _ = SetWindowTextW(
                self.start_index,
                if matches!(mode, Some(IndexMode::ScanOnly)) {
                    w!("仅扫描建索引")
                } else {
                    w!("建立索引")
                },
            );
        }
    }

    pub(super) fn show_index(&self, id: &str, result: Result<IndexStatus>) {
        let Some(asset) = self.selected_asset() else {
            return;
        };
        if asset.registration.partition_id != id || self.write_task.borrow().is_some() {
            return;
        }
        let label = match result {
            Ok(status) => {
                if let Some(watch) = self.index.borrow_mut().as_mut() {
                    watch.mode = start_mode(&asset, &status);
                }
                index_label(&status)
            }
            Err(error) => {
                if let Some(watch) = self.index.borrow_mut().as_mut() {
                    watch.mode = None;
                }
                format!("无法读取索引状态：{error:#}")
            }
        };
        self.update_partition_selection();
        self.status(&format!("{} · {label}", asset.registration.name));
    }

    pub(super) fn start_index(&self) {
        if self.write_task.borrow().is_some() {
            return;
        }
        let Some(asset) = self.selected_asset() else {
            return;
        };
        let id = asset.registration.partition_id;
        let mode = self
            .index
            .borrow()
            .as_ref()
            .filter(|i| i.partition == id)
            .and_then(|i| i.mode);
        let Some(mode) = mode else { return };
        let pipe = self.endpoint.pipe().to_owned();
        let send = self.send.clone();
        let target = self.hwnd.0 as usize;
        self.write_task.replace(Some(self.runtime.spawn(async move {
            let result = accepted(
                &pipe,
                Action::IndexStart {
                    partition_id: id.clone(),
                    mode,
                },
            )
            .await;
            notify(&send, target, Update::IndexStarted(id, result));
        })));
        self.update_partition_selection();
        self.status("正在提交索引任务…");
    }

    pub(super) fn index_started(&self, id: &str, result: Result<()>) {
        self.write_task.take();
        if self
            .index
            .borrow()
            .as_ref()
            .is_some_and(|i| i.partition == id)
        {
            if let Some(watch) = self.index.borrow_mut().as_mut() {
                watch.mode = None;
            }
            match result {
                Ok(()) => self.status("索引任务已提交"),
                Err(error) => {
                    // SAFETY: The modal error is owned by this window so polling cannot hide it.
                    unsafe {
                        MessageBoxW(
                            Some(self.hwnd),
                            &HSTRING::from(format!("{error:#}")),
                            w!("无法开始索引"),
                            MB_OK | MB_ICONERROR,
                        )
                    };
                }
            }
        }
        self.update_partition_selection();
    }
}
