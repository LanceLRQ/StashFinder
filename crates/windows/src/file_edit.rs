use super::{inspect, verify};
use crate::shell::Apartment;
use anyhow::{Context, Result, ensure};
use stashfinder_core::file_operations::*;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use windows::{
    Win32::{
        Foundation::{E_ABORT, E_FAIL, ERROR_CANCELLED, HWND, S_OK},
        Storage::FileSystem::{FindClose, FindFirstFileW, WIN32_FIND_DATAW},
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree},
        UI::Shell::*,
    },
    core::{HRESULT, HSTRING, PCWSTR, Ref, implement},
};

pub enum Edit {
    Rename(String),
    Recycle,
    Delete,
    Paste { folder: PathBuf, move_files: bool },
}

fn is_cancelled(hr: HRESULT) -> bool {
    hr == E_ABORT || hr == COPYENGINE_E_USER_CANCELLED || hr == ERROR_CANCELLED.to_hresult()
}

fn shell_path(item: &IShellItem) -> Option<String> {
    // SAFETY: The returned Shell string is read before freeing its COM task allocation.
    unsafe {
        let value = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let text = value.to_string().ok();
        CoTaskMemFree(Some(value.0.cast()));
        text
    }
}

#[implement(IFileOperationProgressSink)]
struct Sink {
    index: Option<usize>,
    root: Option<IShellItem>,
    source: Option<PreparedTarget>,
    destination: Option<PreparedTarget>,
    destination_path: Option<PathBuf>,
    initial: Option<(PathBuf, NativeIdentity)>,
    checked: AtomicBool,
    cancel: Arc<AtomicBool>,
    report: Arc<Mutex<OperationReport>>,
    recycle: bool,
}
impl Sink_Impl {
    fn check(&self, item: Option<&IShellItem>) -> windows::core::Result<()> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(windows::core::Error::from_hresult(E_ABORT));
        }
        let is_root = match (&self.root, item) {
            (Some(root), Some(item)) => {
                // SAFETY: Both interfaces are owned by this STA and stay alive during the comparison.
                unsafe {
                    root.Compare(item, SICHINT_CANONICAL.0 as u32)
                        .is_ok_and(|v| v == 0)
                }
            }
            _ => false,
        };
        if is_root && !self.checked.load(Ordering::Acquire) {
            let verified = (|| -> Result<()> {
                if let Some(source) = &self.source {
                    let expected = verify(source, false)?;
                    if let Some((path, id)) = &self.initial {
                        let queued = inspect(path)?.identity;
                        ensure!(
                            queued == *id && queued == inspect(&expected)?.identity,
                            "排队的源路径身份已变化，请重新操作"
                        );
                    }
                } else if let Some((path, id)) = &self.initial {
                    ensure!(inspect(path)?.identity == *id, "源文件身份已变化");
                }
                if let Some(destination) = &self.destination {
                    let expected = verify(destination, true)?;
                    if let Some(path) = &self.destination_path {
                        ensure!(
                            inspect(path)?.identity == inspect(&expected)?.identity,
                            "排队的目标路径身份已变化，请重新操作"
                        );
                    }
                }
                Ok(())
            })();
            if let Err(error) = verified {
                if let Some(index) = self.index {
                    let mut report = self.report.lock().unwrap_or_else(|e| e.into_inner());
                    report.items[index].state = OutcomeState::Failed;
                    report.items[index].error = Some(format!("{error:#}"));
                }
                return Err(windows::core::Error::new(E_FAIL, format!("{error:#}")));
            }
            self.checked.store(true, Ordering::Release);
        }
        Ok(())
    }
    fn post(
        &self,
        hr: HRESULT,
        item: Option<&IShellItem>,
        deletion: bool,
    ) -> windows::core::Result<()> {
        if let Some(index) = self.index {
            let mut report = self.report.lock().unwrap_or_else(|e| e.into_inner());
            let cancelled = is_cancelled(hr);
            report.cancelled |= cancelled;
            let row = &mut report.items[index];
            if cancelled {
                if !matches!(row.state, OutcomeState::Completed | OutcomeState::Failed) {
                    row.state = OutcomeState::Skipped;
                    row.note = Some("用户已取消，未完成".into());
                }
            } else if hr.is_err() {
                row.state = OutcomeState::Failed;
                if row.error.is_none() {
                    row.error = Some(windows::core::Error::from_hresult(hr).to_string());
                }
            } else if !matches!(
                hr,
                S_OK | COPYENGINE_S_DONT_PROCESS_CHILDREN
                    | COPYENGINE_S_ALREADY_DONE
                    | COPYENGINE_S_COLLISIONRESOLVED
                    | COPYENGINE_S_MERGE
                    | COPYENGINE_S_KEEP_BOTH
            ) {
                if row.state != OutcomeState::Failed {
                    row.state = OutcomeState::Skipped;
                    row.note = Some(format!("Windows 返回状态 0x{:08x}", hr.0 as u32));
                }
            } else if !matches!(row.state, OutcomeState::Failed | OutcomeState::Skipped) {
                row.state = OutcomeState::Completed;
                row.destination = item.and_then(shell_path);
                if deletion {
                    row.note = Some(
                        if self.recycle && item.is_some() {
                            "已移入回收站"
                        } else {
                            "已永久删除"
                        }
                        .into(),
                    );
                }
            }
        }
        Ok(())
    }
}
impl IFileOperationProgressSink_Impl for Sink_Impl {
    fn StartOperations(&self) -> windows::core::Result<()> {
        self.check(None)
    }
    fn FinishOperations(&self, hr: HRESULT) -> windows::core::Result<()> {
        if self.index.is_none() && is_cancelled(hr) {
            self.report
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .cancelled = true;
            return Ok(());
        }
        if self.index.is_none() && hr.is_err() && hr != E_ABORT {
            self.report.lock().unwrap_or_else(|e| e.into_inner()).error =
                Some(windows::core::Error::from_hresult(hr).to_string());
        }
        Ok(())
    }
    fn PreRenameItem(
        &self,
        _: u32,
        item: Ref<IShellItem>,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.check(item.as_ref())
    }
    fn PostRenameItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: &PCWSTR,
        hr: HRESULT,
        item: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        self.post(hr, item.as_ref(), false)
    }
    fn PreMoveItem(
        &self,
        _: u32,
        item: Ref<IShellItem>,
        _: Ref<IShellItem>,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.check(item.as_ref())
    }
    fn PostMoveItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: Ref<IShellItem>,
        _: &PCWSTR,
        hr: HRESULT,
        item: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        self.post(hr, item.as_ref(), false)
    }
    fn PreCopyItem(
        &self,
        _: u32,
        item: Ref<IShellItem>,
        _: Ref<IShellItem>,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.check(item.as_ref())
    }
    fn PostCopyItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: Ref<IShellItem>,
        _: &PCWSTR,
        hr: HRESULT,
        item: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        self.post(hr, item.as_ref(), false)
    }
    fn PreDeleteItem(&self, _: u32, item: Ref<IShellItem>) -> windows::core::Result<()> {
        self.check(item.as_ref())
    }
    fn PostDeleteItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        hr: HRESULT,
        item: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        self.post(hr, item.as_ref(), true)
    }
    fn PreNewItem(&self, _: u32, _: Ref<IShellItem>, _: &PCWSTR) -> windows::core::Result<()> {
        self.check(None)
    }
    fn PostNewItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: &PCWSTR,
        _: &PCWSTR,
        _: u32,
        _: HRESULT,
        _: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn UpdateProgress(&self, _: u32, _: u32) -> windows::core::Result<()> {
        self.check(None)
    }
    fn ResetTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
}

/// Only callers holding validated registered targets or explicit OS clipboard sources may call this.
pub fn execute(
    paths: &[PathBuf],
    guards: &[Option<PreparedTarget>],
    destination: Option<PreparedTarget>,
    edit: &Edit,
    owner: usize,
    cancel: Arc<AtomicBool>,
) -> Result<OperationReport> {
    ensure!(
        !paths.is_empty() && paths.len() <= MAX_SELECTED && paths.len() == guards.len(),
        "文件操作选择无效"
    );
    let report = Arc::new(Mutex::new(OperationReport {
        items: paths
            .iter()
            .enumerate()
            .map(|(index, p)| ItemOutcome {
                index,
                source: p.to_string_lossy().into(),
                state: OutcomeState::NotRun,
                destination: None,
                error: None,
                note: None,
            })
            .collect(),
        cancelled: false,
        error: None,
    }));
    if cancel.load(Ordering::Acquire) {
        let mut report = report.lock().unwrap_or_else(|e| e.into_inner());
        report.cancelled = true;
        return Ok(report.clone());
    }
    if let Edit::Rename(name) = edit {
        ensure!(paths.len() == 1, "重命名只允许选择一项");
        let stem = name.split('.').next().unwrap_or_default().to_uppercase();
        let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.chars().count() == 4
                && stem
                    .chars()
                    .last()
                    .is_some_and(|c| "123456789¹²³".contains(c)));
        ensure!(
            !name.is_empty()
                && !name.ends_with(['.', ' '])
                && !name.chars().any(|c| c < ' ' || "<>:\"/\\|?*".contains(c))
                && !reserved,
            "名称无效，请输入单个文件名"
        );
        let target = paths[0].with_file_name(name);
        if target.try_exists()? {
            let old = paths[0]
                .file_name()
                .context("缺少文件名")?
                .to_string_lossy();
            ensure!(
                old.to_lowercase() == name.to_lowercase()
                    && inspect(&target)?.identity == inspect(&paths[0])?.identity,
                "目标名称已存在"
            );
        }
    }
    let _apartment = Apartment::new()?;
    // SAFETY: All COM objects are created, invoked and released inside this ordinary STA.
    unsafe {
        let operation: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)?;
        operation.SetOwnerWindow(HWND(owner as *mut _))?;
        let mut flags = FOFX_EARLYFAILURE;
        match edit {
            Edit::Recycle => {
                flags |= FOF_NOCONFIRMATION
                    | FOFX_RECYCLEONDELETE
                    | FOFX_ADDUNDORECORD
                    | FOF_WANTNUKEWARNING
            }
            Edit::Delete => flags |= FOF_NOCONFIRMATION,
            Edit::Rename(_) => flags |= FOFX_ADDUNDORECORD,
            Edit::Paste { .. } => flags |= FOFX_ADDUNDORECORD,
        }
        operation.SetOperationFlags(flags)?;
        let global: IFileOperationProgressSink = Sink {
            index: None,
            root: None,
            source: None,
            destination: None,
            destination_path: None,
            initial: None,
            checked: AtomicBool::new(false),
            cancel: cancel.clone(),
            report: report.clone(),
            recycle: false,
        }
        .into();
        let cookie = operation.Advise(&global)?;
        let target: Option<IShellItem> = if let Edit::Paste { folder, .. } = edit {
            Some(SHCreateItemFromParsingName(
                &HSTRING::from(folder.as_os_str()),
                None,
            )?)
        } else {
            None
        };
        for (index, path) in paths.iter().enumerate() {
            ensure!(!cancel.load(Ordering::Acquire), "操作已取消");
            let source: IShellItem =
                SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None)?;
            let initial = inspect(path)?.identity;
            let sink: IFileOperationProgressSink = Sink {
                index: Some(index),
                root: Some(source.clone()),
                source: guards[index].clone(),
                destination: destination.clone(),
                destination_path: if let Edit::Paste { folder, .. } = edit {
                    Some(folder.clone())
                } else {
                    None
                },
                initial: Some((path.clone(), initial)),
                checked: AtomicBool::new(false),
                cancel: cancel.clone(),
                report: report.clone(),
                recycle: matches!(edit, Edit::Recycle),
            }
            .into();
            match edit {
                Edit::Rename(name) => operation.RenameItem(&source, &HSTRING::from(name), &sink)?,
                Edit::Recycle | Edit::Delete => operation.DeleteItem(&source, &sink)?,
                Edit::Paste { move_files, .. } => {
                    let target = target.as_ref().context("缺少粘贴目标")?;
                    if *move_files {
                        operation.MoveItem(&source, target, None, &sink)?;
                    } else {
                        operation.CopyItem(&source, target, None, &sink)?;
                    }
                }
            }
        }
        let performed = operation.PerformOperations();
        let aborted = operation.GetAnyOperationsAborted();
        let _ = operation.Unadvise(cookie);
        let mut report = report.lock().unwrap_or_else(|e| e.into_inner());
        report.cancelled |= cancel.load(Ordering::Acquire);
        if let Err(error) = performed {
            if is_cancelled(error.code()) {
                report.cancelled = true;
            } else {
                report.error = Some(error.to_string());
            }
        }
        if aborted.map(|v| v.as_bool()).unwrap_or(true)
            && report.completed() != report.items.len()
            && !report.cancelled
            && report.error.is_none()
        {
            report.error = Some("Windows 报告存在未完成的操作".into());
        }
        if let Edit::Rename(name) = edit
            && report.items[0].state == OutcomeState::Completed
        {
            let target = paths[0].with_file_name(name);
            let mut extended = std::ffi::OsString::from(r"\\?\");
            let spelling = if target
                .as_os_str()
                .as_encoded_bytes()
                .starts_with(b"\\\\?\\")
            {
                target.as_os_str()
            } else {
                extended.push(&target);
                extended.as_os_str()
            };
            let mut data = WIN32_FIND_DATAW::default();
            // Query stored spelling, rather than the spelling requested from Shell.
            let found = FindFirstFileW(&HSTRING::from(spelling), &mut data);
            match found {
                Ok(handle) => {
                    let _ = FindClose(handle);
                    let len = data
                        .cFileName
                        .iter()
                        .position(|c| *c == 0)
                        .unwrap_or(data.cFileName.len());
                    if !data.cFileName[..len]
                        .iter()
                        .copied()
                        .eq(name.encode_utf16())
                    {
                        report.items[0].state = OutcomeState::Failed;
                        report.items[0].error = Some(
                            "Windows 未按请求改变文件名大小写；可先改成不同的名称再重试".into(),
                        );
                    }
                }
                Err(error) => {
                    report.items[0].state = OutcomeState::Failed;
                    report.items[0].error = Some(format!("无法核对改名结果：{error}"));
                }
            }
        }
        Ok(report.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    #[test]
    fn a_cancelled_shell_item_is_not_reported_as_a_failed_copy() {
        for code in [
            COPYENGINE_E_USER_CANCELLED,
            E_ABORT,
            ERROR_CANCELLED.to_hresult(),
        ] {
            let report = Arc::new(Mutex::new(OperationReport {
                items: vec![ItemOutcome {
                    index: 0,
                    source: "test sample".into(),
                    state: OutcomeState::NotRun,
                    destination: None,
                    error: None,
                    note: None,
                }],
                cancelled: false,
                error: None,
            }));
            let sink: IFileOperationProgressSink = Sink {
                index: Some(0),
                root: None,
                source: None,
                destination: None,
                destination_path: None,
                initial: None,
                checked: AtomicBool::new(false),
                cancel: Arc::new(AtomicBool::new(false)),
                report: report.clone(),
                recycle: false,
            }
            .into();
            // SAFETY: This invokes only the report callback; no Shell item or file is operated on.
            unsafe {
                sink.PostCopyItem(0, None, None, PCWSTR::null(), code, None)
                    .unwrap()
            };
            let result = report.lock().unwrap();
            assert!(result.cancelled);
            assert_eq!(result.items[0].state, OutcomeState::Skipped);
            assert!(result.items[0].error.is_none() && result.error.is_none());
        }
    }
    #[test]
    fn a_queued_alias_must_not_replace_the_registered_source() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("registered.txt");
        let queued = directory.path().join("queued.txt");
        std::fs::write(&source, b"registered").unwrap();
        std::fs::write(&queued, b"replacement").unwrap();
        let native = inspect(&source).unwrap().identity;
        let volume = source.ancestors().last().unwrap();
        let registered = PreparedTarget {
            mounts: vec![],
            partition_name: "test".into(),
            volume: volume.to_string_lossy().into(),
            identity: stashfinder_core::assets::Identity {
                layout: stashfinder_core::assets::Layout::Unknown,
                filesystem: Some("NTFS".into()),
                serial: Some(stashfinder_core::assets::Serial {
                    source: "win32_volume32".into(),
                    value: format!("{:08x}", native.volume),
                }),
            },
            file: ResolvedEntry {
                reference: EntryRef {
                    partition_id: "p".into(),
                    filesystem_id: "fs".into(),
                    generation: "1".into(),
                    revision: "1".into(),
                    entry: vec![1],
                },
                relative: source
                    .strip_prefix(volume)
                    .unwrap()
                    .as_os_str()
                    .encode_wide()
                    .collect(),
                object: vec![1],
                parent: Some(vec![0]),
                folder: false,
                size: None,
                created: None,
                modified: None,
                identity: Some(ExpectedIdentity::Native(native)),
            },
        };
        let _apartment = Apartment::new().unwrap();
        // SAFETY: This STA owns the Shell item; the callback only checks generated files, never deletes them.
        unsafe {
            let item: IShellItem =
                SHCreateItemFromParsingName(&HSTRING::from(queued.as_os_str()), None).unwrap();
            let report = Arc::new(Mutex::new(OperationReport {
                items: vec![ItemOutcome {
                    index: 0,
                    source: "queued".into(),
                    state: OutcomeState::NotRun,
                    destination: None,
                    error: None,
                    note: None,
                }],
                cancelled: false,
                error: None,
            }));
            let sink: IFileOperationProgressSink = Sink {
                index: Some(0),
                root: Some(item.clone()),
                source: Some(registered.clone()),
                destination: None,
                destination_path: None,
                initial: Some((queued.clone(), inspect(&queued).unwrap().identity)),
                checked: AtomicBool::new(false),
                cancel: Arc::new(AtomicBool::new(false)),
                report: report.clone(),
                recycle: false,
            }
            .into();
            assert!(sink.PreDeleteItem(0, &item).is_err());
            assert_eq!(report.lock().unwrap().items[0].state, OutcomeState::Failed);
            let expected_directory = directory.path().join("registered-folder");
            let queued_directory = directory.path().join("queued-folder");
            std::fs::create_dir(&expected_directory).unwrap();
            std::fs::create_dir(&queued_directory).unwrap();
            let mut destination = registered.clone();
            destination.file.folder = true;
            destination.file.relative = expected_directory
                .strip_prefix(volume)
                .unwrap()
                .as_os_str()
                .encode_wide()
                .collect();
            destination.file.identity = Some(ExpectedIdentity::Native(
                inspect(&expected_directory).unwrap().identity,
            ));
            let source_item: IShellItem =
                SHCreateItemFromParsingName(&HSTRING::from(source.as_os_str()), None).unwrap();
            let target_item: IShellItem =
                SHCreateItemFromParsingName(&HSTRING::from(queued_directory.as_os_str()), None)
                    .unwrap();
            {
                let mut report = report.lock().unwrap();
                report.items[0].state = OutcomeState::NotRun;
                report.items[0].error = None;
            }
            let sink: IFileOperationProgressSink = Sink {
                index: Some(0),
                root: Some(source_item.clone()),
                source: Some(registered),
                destination: Some(destination),
                destination_path: Some(queued_directory.clone()),
                initial: Some((source.clone(), inspect(&source).unwrap().identity)),
                checked: AtomicBool::new(false),
                cancel: Arc::new(AtomicBool::new(false)),
                report: report.clone(),
                recycle: false,
            }
            .into();
            assert!(
                sink.PreCopyItem(0, &source_item, &target_item, PCWSTR::null())
                    .is_err()
            );
            assert!(
                report.lock().unwrap().items[0]
                    .error
                    .as_deref()
                    .unwrap()
                    .contains("目标")
            );
        }
        assert_eq!(std::fs::read(source).unwrap(), b"registered");
        assert_eq!(std::fs::read(queued).unwrap(), b"replacement");
    }
}
