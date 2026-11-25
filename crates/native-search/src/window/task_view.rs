use super::*;
use crate::tasks::TaskList;

impl Window {
    pub(super) fn viewing_tasks(&self) -> bool {
        self.browser
            .borrow()
            .as_ref()
            .is_some_and(|b| matches!(b.location, Location::Tasks))
    }

    pub(super) fn refresh_tasks(&self) {
        if self.viewing_tasks()
            && self.browse_task.borrow().is_none()
            && self.write_task.borrow().is_none()
        {
            self.load_listing(Location::Tasks, false);
        }
    }

    pub(super) fn retain_task_selection(&self, new: &TaskList) -> Option<usize> {
        let row = self.selected_row();
        let focus = {
            let browser = self.browser.borrow();
            let Some(Listing::Tasks(old)) = browser.as_ref().and_then(|b| b.listing.as_ref())
            else {
                return None;
            };
            if old
                .rows
                .iter()
                .map(|r| &r.task.id)
                .eq(new.rows.iter().map(|r| &r.task.id))
            {
                return None;
            }
            row.and_then(|row| old.rows.get(row))
                .and_then(|r| new.rows.iter().position(|n| n.task.id == r.task.id))
        };
        // An owner-data list keeps row numbers when its count changes. Discard those numbers
        // before displaying a reordered snapshot, then restore the selected task by ID.
        self.set_selected_row(None);
        focus
    }

    pub(super) fn update_selection(&self) {
        if !self.viewing_tasks() {
            self.update_partition_selection();
            return;
        }
        let selected = self.selected_row();
        let idle = self.write_task.borrow().is_none();
        let mut cancel = false;
        let mut retry = false;
        let mut label = None;
        {
            let browser = self.browser.borrow();
            if let Some(Listing::Tasks(list)) = browser.as_ref().and_then(|b| b.listing.as_ref())
                && !list.stale
                && idle
            {
                let selected = selected.filter(|row| *row < list.rows.len());
                let mut text = match selected {
                    Some(row) => {
                        cancel = list.action(row, Operation::Cancel).is_some();
                        retry = list.action(row, Operation::Retry).is_some();
                        list.detail(row)
                    }
                    None => list.summary(),
                };
                if let Some((id, notice)) = &*self.task_notice.borrow()
                    && selected.is_none_or(|row| list.rows[row].task.id == *id)
                {
                    text = format!("{notice} · {text}");
                }
                if let Some(error) = &list.device_error {
                    text.push_str(&format!(" · 分区状态不可用：{error}"));
                }
                label = Some(text);
            }
        }
        // SAFETY: Updating these controls may send synchronous focus notifications.
        unsafe {
            let _ = EnableWindow(self.cancel_task, cancel);
            let _ = EnableWindow(self.retry_task, retry);
        }
        if let Some(label) = label {
            self.status(&label);
        }
    }

    pub(super) fn task_command(&self, op: Operation) {
        if self.write_task.borrow().is_some() {
            return;
        }
        let Some(row) = self.selected_row() else {
            return;
        };
        let (id, action) = {
            let browser = self.browser.borrow();
            let Some(Listing::Tasks(list)) = browser.as_ref().and_then(|b| b.listing.as_ref())
            else {
                return;
            };
            let Some(action) = list.action(row, op) else {
                return;
            };
            (list.rows[row].task.id.clone(), action)
        };
        if let Some(task) = self.browse_task.take() {
            task.abort();
        }
        // A read that finished just before the write must not re-enable the old actions.
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let pipe = self.endpoint.pipe().to_owned();
        let send = self.send.clone();
        let hwnd = self.hwnd.0 as usize;
        self.write_task.replace(Some(self.runtime.spawn(async move {
            let result = crate::registration::accepted(&pipe, action).await;
            notify(&send, hwnd, Update::TaskAction(generation, id, op, result));
        })));
        self.update_selection();
        self.status(match op {
            Operation::Cancel => "正在取消任务…",
            Operation::Retry => "正在提交重试…",
        });
    }

    pub(super) fn task_action_finished(
        &self,
        generation: u64,
        id: String,
        op: Operation,
        result: Result<()>,
    ) {
        self.write_task.take();
        if generation == self.generation.get() {
            let notice = match (op, result) {
                (Operation::Cancel, Ok(())) => "已取消所选任务".to_owned(),
                (Operation::Retry, Ok(())) => "已提交重试".to_owned(),
                (Operation::Cancel, Err(e)) => format!("取消任务失败：{e:#}"),
                (Operation::Retry, Err(e)) => format!("重试任务失败：{e:#}"),
            };
            self.status(&notice);
            self.task_notice.replace(Some((id, notice)));
            if let Some(Listing::Tasks(list)) = self
                .browser
                .borrow_mut()
                .as_mut()
                .and_then(|b| b.listing.as_mut())
            {
                list.stale = true;
            }
        }
        self.update_selection();
        self.refresh_tasks();
    }
}
