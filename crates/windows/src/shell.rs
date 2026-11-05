use anyhow::{Result, ensure};
use std::{marker::PhantomData, os::windows::ffi::OsStrExt, path::PathBuf, rc::Rc};
use windows::{
    Win32::{
        Foundation::{
            DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, DV_E_FORMATETC,
            E_NOTIMPL, E_POINTER, OLE_E_ADVISENOTSUPPORTED, S_OK,
        },
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, CoCreateInstance, DATADIR_GET, DVASPECT_CONTENT, FORMATETC,
                IAdviseSink, IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA,
                STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
            },
            DataExchange::RegisterClipboardFormatW,
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
            Ole::{
                DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_MOVE, DoDragDrop, IDropSource,
                IDropSource_Impl, OleFlushClipboard, OleInitialize, OleSetClipboard,
                OleUninitialize, ReleaseStgMedium,
            },
            SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS},
        },
        UI::Shell::{
            DROPFILES, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING,
            FOFX_ADDUNDORECORD, FOFX_EARLYFAILURE, FOFX_RECYCLEONDELETE, FileOperation,
            IFileOperation, IShellItem, SHCreateItemFromParsingName, SHCreateStdEnumFmtEtc,
        },
    },
    core::{BOOL, HRESULT, HSTRING, Ref, implement, w},
};

pub(crate) struct Apartment(PhantomData<Rc<()>>);
impl Apartment {
    pub(crate) fn new() -> Result<Self> {
        // SAFETY: Each successful initialization is balanced on the same thread.
        unsafe {
            OleInitialize(None)?;
        }
        Ok(Self(PhantomData))
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: Apartment is !Send and exists only after successful initialization.
        unsafe {
            OleUninitialize();
        }
    }
}

// Physical files use absolute paths rather than parent-relative Shell PIDLs.
#[implement(IDataObject)]
struct FileData {
    names: Vec<u16>,
    effect_format: u16,
    effect: DROPEFFECT,
}
impl IDataObject_Impl for FileData_Impl {
    fn GetData(&self, format: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
        self.QueryGetData(format).ok()?;
        // SAFETY: QueryGetData checked the argument; COM supplies a live FORMATETC.
        let files = unsafe { (*format).cfFormat == 15 };
        let bytes = if files {
            size_of::<DROPFILES>() + self.names.len() * 2
        } else {
            4
        };
        // SAFETY: Fully initialize the movable block before transferring ownership.
        // Failed locks release it; consumers release successful media using ReleaseStgMedium.
        unsafe {
            let memory = GlobalAlloc(GMEM_MOVEABLE, bytes)?;
            let mut medium = STGMEDIUM {
                tymed: TYMED_HGLOBAL.0 as u32,
                u: STGMEDIUM_0 { hGlobal: memory },
                ..Default::default()
            };
            let pointer = GlobalLock(memory).cast::<u8>();
            if pointer.is_null() {
                let error = windows::core::Error::from_thread();
                ReleaseStgMedium(&mut medium);
                return Err(error);
            }
            if files {
                pointer.cast::<DROPFILES>().write(DROPFILES {
                    pFiles: size_of::<DROPFILES>() as u32,
                    fWide: BOOL(1),
                    ..Default::default()
                });
                std::ptr::copy_nonoverlapping(
                    self.names.as_ptr(),
                    pointer.add(size_of::<DROPFILES>()).cast::<u16>(),
                    self.names.len(),
                );
            } else {
                pointer.cast::<u32>().write(self.effect.0);
            }
            let _ = GlobalUnlock(memory);
            Ok(medium)
        }
    }
    fn QueryGetData(&self, format: *const FORMATETC) -> HRESULT {
        // SAFETY: COM supplies either null or a live FORMATETC for this call.
        let Some(format) = (unsafe { format.as_ref() }) else {
            return E_POINTER;
        };
        if (format.cfFormat == 15 || format.cfFormat == self.effect_format)
            && format.dwAspect == DVASPECT_CONTENT.0
            && format.lindex == -1
            && format.tymed & TYMED_HGLOBAL.0 as u32 != 0
            && format.ptd.is_null()
        {
            S_OK
        } else {
            DV_E_FORMATETC
        }
    }
    fn EnumFormatEtc(&self, direction: u32) -> windows::core::Result<IEnumFORMATETC> {
        if direction != DATADIR_GET.0 as u32 {
            return Err(E_NOTIMPL.into());
        }
        let formats = [15, self.effect_format].map(|cf_format| FORMATETC {
            cfFormat: cf_format,
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
            ..Default::default()
        });
        // SAFETY: The system enumerator copies these descriptors; neither owns a target device.
        unsafe { SHCreateStdEnumFmtEtc(&formats) }
    }
    fn GetDataHere(&self, _: *const FORMATETC, _: *mut STGMEDIUM) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn GetCanonicalFormatEtc(&self, _: *const FORMATETC, output: *mut FORMATETC) -> HRESULT {
        // SAFETY: COM supplies writable output storage or null.
        if let Some(output) = unsafe { output.as_mut() } {
            *output = FORMATETC::default();
        }
        E_NOTIMPL
    }
    fn SetData(
        &self,
        _: *const FORMATETC,
        _: *const STGMEDIUM,
        _: BOOL,
    ) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn DAdvise(
        &self,
        _: *const FORMATETC,
        _: u32,
        _: Ref<'_, IAdviseSink>,
    ) -> windows::core::Result<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
    fn DUnadvise(&self, _: u32) -> windows::core::Result<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
    fn EnumDAdvise(&self) -> windows::core::Result<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}

pub(crate) fn data_object(paths: &[PathBuf], effect: DROPEFFECT) -> Result<IDataObject> {
    ensure!(
        !paths.is_empty() && paths.len() <= stashfinder_core::file_operations::MAX_SELECTED,
        "请选择 1～10,000 项"
    );
    let mut names = Vec::new();
    for path in paths {
        let path = crate::file_operations::shell_path(path)?;
        ensure!(
            path.is_absolute() && path.exists(),
            "sample unavailable: {}",
            path.display()
        );
        names.extend(path.as_os_str().encode_wide());
        names.push(0);
    }
    names.push(0);
    // SAFETY: The static name is a valid null-terminated UTF-16 string.
    let effect_format = unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) } as u16;
    ensure!(effect_format != 0, "cannot register preferred drop effect");
    Ok(FileData {
        names,
        effect_format,
        effect,
    }
    .into())
}

/// Call from the GUI STA thread after the caller validates its registered targets or M0 fixtures.
pub fn copy_samples(paths: &[PathBuf]) -> Result<()> {
    clipboard_samples(paths, DROPEFFECT_COPY)
}

pub fn cut_samples(paths: &[PathBuf]) -> Result<()> {
    clipboard_samples(paths, DROPEFFECT_MOVE)
}

fn clipboard_samples(paths: &[PathBuf], effect: DROPEFFECT) -> Result<()> {
    let _apartment = Apartment::new()?;
    let data = data_object(paths, effect)?;
    // SAFETY: The data object is live in this OLE apartment; flushing gives Windows
    // ownership of the clipboard formats after this call returns.
    unsafe {
        OleSetClipboard(&data)?;
        OleFlushClipboard()?;
    }
    Ok(())
}

#[implement(IDropSource)]
struct CopySource;
impl IDropSource_Impl for CopySource_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if keys.0 & MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }
    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

pub fn drag_samples(paths: &[PathBuf]) -> Result<bool> {
    let _apartment = Apartment::new()?;
    let data = data_object(paths, DROPEFFECT_COPY)?;
    let source: IDropSource = CopySource.into();
    let mut effect = DROPEFFECT::default();
    // SAFETY: Runs on the GUI OLE thread, with owned COM objects alive for the modal
    // loop. COPY is the only allowed effect; no source deletion is implemented.
    let result = unsafe { DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect) };
    result.ok()?;
    Ok(result == DRAGDROP_S_DROP && effect == DROPEFFECT_COPY)
}

pub fn left_button_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    // SAFETY: Read-only button state on the current interactive desktop.
    unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 }
}

pub enum OpenAction {
    Open,
    OpenWith,
    Locate,
    Properties,
}

pub fn copy_text(value: &[u16], owner: usize) -> Result<()> {
    use windows::Win32::{
        Foundation::{GlobalFree, HANDLE, HWND},
        System::{DataExchange::*, Ole::CF_UNICODETEXT},
    };
    ensure!(!value.contains(&0), "文字中包含不可复制的空字符");
    // SAFETY: The window belongs to this GUI thread. Allocated memory is transferred only on success.
    unsafe {
        OpenClipboard(Some(HWND(owner as *mut _)))?;
        let result = (|| -> Result<()> {
            let memory = GlobalAlloc(GMEM_MOVEABLE, (value.len() + 1) * 2)?;
            let copied = (|| -> Result<()> {
                let buffer = GlobalLock(memory).cast::<u16>();
                ensure!(!buffer.is_null(), "无法分配剪贴板内存");
                std::ptr::copy_nonoverlapping(value.as_ptr(), buffer, value.len());
                *buffer.add(value.len()) = 0;
                let _ = GlobalUnlock(memory);
                EmptyClipboard()?;
                SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(memory.0)))?;
                Ok(())
            })();
            if copied.is_err() {
                let _ = GlobalFree(Some(memory));
            }
            copied
        })();
        let closed = CloseClipboard();
        result?;
        closed?;
    }
    Ok(())
}

pub fn open_file(path: &std::path::Path, action: OpenAction, owner: usize) -> Result<()> {
    use windows::{
        Win32::{
            Foundation::HWND,
            UI::{Shell::*, WindowsAndMessaging::SW_SHOWNORMAL},
        },
        core::PCWSTR,
    };
    let _apartment = Apartment::new()?;
    let path = HSTRING::from(path.as_os_str());
    let owner = HWND(owner as *mut _);
    // SAFETY: All Shell strings and PIDLs stay owned through their synchronous calls on this STA.
    unsafe {
        match action {
            OpenAction::Locate => {
                let pidl = ILCreateFromPathW(&path);
                ensure!(!pidl.is_null(), "无法创建资源管理器定位项");
                let result = SHOpenFolderAndSelectItems(pidl, None, 0);
                ILFree(Some(pidl));
                result?;
            }
            OpenAction::OpenWith => SHOpenWithDialog(
                Some(owner),
                &OPENASINFO {
                    pcszFile: PCWSTR(path.as_ptr()),
                    pcszClass: PCWSTR::null(),
                    oaifInFlags: OAIF_EXEC,
                },
            )?,
            OpenAction::Open | OpenAction::Properties => {
                let verb = if matches!(action, OpenAction::Properties) {
                    w!("properties")
                } else {
                    w!("open")
                };
                let mut info = SHELLEXECUTEINFOW {
                    cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                    fMask: SEE_MASK_INVOKEIDLIST,
                    hwnd: owner,
                    lpVerb: verb,
                    lpFile: PCWSTR(path.as_ptr()),
                    nShow: SW_SHOWNORMAL.0,
                    ..Default::default()
                };
                ShellExecuteExW(&mut info)?;
            }
        }
    }
    Ok(())
}

pub enum SampleEdit {
    Rename(String),
    Recycle,
    Delete,
}

/// Operates only on the M0 fixture paths selected by the command boundary.
pub fn edit_samples(paths: &[PathBuf], edit: &SampleEdit) -> Result<()> {
    let _apartment = Apartment::new()?;
    let _validated = data_object(paths, DROPEFFECT_COPY)?;
    if let SampleEdit::Rename(name) = edit {
        ensure!(paths.len() == 1, "重命名只允许选择一项");
        let stem = name.split('.').next().unwrap_or_default().to_uppercase();
        let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            || (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.len() == 4
                && matches!(stem.as_bytes()[3], b'1'..=b'9');
        ensure!(
            !name.is_empty()
                && !name.ends_with(['.', ' '])
                && !name.chars().any(|c| c < ' ' || "<>:\"/\\|?*".contains(c))
                && !reserved,
            "名称无效，请输入单个文件名"
        );
        ensure!(
            !paths[0].with_file_name(name).try_exists()?,
            "目标名称已存在"
        );
    }
    // SAFETY: The STA owns all Shell interfaces and queues only validated fixture paths.
    // Recycling retains the Shell's permanent-deletion warning; no silent fallback is requested.
    unsafe {
        let operation: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)?;
        let mut flags = FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT | FOFX_EARLYFAILURE;
        match edit {
            SampleEdit::Recycle => {
                flags |= FOFX_RECYCLEONDELETE | FOFX_ADDUNDORECORD | FOF_WANTNUKEWARNING
            }
            SampleEdit::Rename(_) => flags |= FOFX_ADDUNDORECORD,
            SampleEdit::Delete => {}
        }
        operation.SetOperationFlags(flags)?;
        for path in paths {
            let item: IShellItem =
                SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None)?;
            match edit {
                SampleEdit::Rename(name) => {
                    operation.RenameItem(&item, &HSTRING::from(name), None)?
                }
                SampleEdit::Recycle | SampleEdit::Delete => operation.DeleteItem(&item, None)?,
            }
        }
        operation.PerformOperations()?;
        ensure!(
            !operation.GetAnyOperationsAborted()?.as_bool(),
            "操作取消或未全部完成，请刷新核对"
        );
    }
    for path in paths {
        ensure!(!path.try_exists()?, "操作未完成：{}", path.display());
        if let SampleEdit::Rename(name) = edit {
            ensure!(
                path.with_file_name(name).try_exists()?,
                "重命名后的文件不可用"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn volume_guid_file_list_copies_actual_generated_contents() {
        use std::os::windows::{ffi::OsStringExt, io::AsRawHandle};
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFinalPathNameByHandleW, VOLUME_NAME_GUID},
        };
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let source_file = source.path().join("guid-source.txt");
        std::fs::write(&source_file, b"guid contents").unwrap();
        let file = std::fs::File::open(&source_file).unwrap();
        let mut name = vec![0u16; 32768];
        // SAFETY: The owned file handle and writable UTF-16 buffer outlive the query.
        let len = unsafe {
            GetFinalPathNameByHandleW(HANDLE(file.as_raw_handle()), &mut name, VOLUME_NAME_GUID)
        } as usize;
        assert!(len > 0 && len < name.len());
        let path = PathBuf::from(std::ffi::OsString::from_wide(&name[..len]));
        assert!(path.to_string_lossy().starts_with(r"\\?\Volume{"));
        let _apartment = Apartment::new().unwrap();
        let data = data_object(&[path], DROPEFFECT_COPY).unwrap();
        // SAFETY: Copy generated physical files with STA-owned interfaces and suppress test UI.
        unsafe {
            let destination: IShellItem =
                SHCreateItemFromParsingName(&HSTRING::from(target.path().as_os_str()), None)
                    .unwrap();
            let operation: IFileOperation =
                CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER).unwrap();
            operation
                .SetOperationFlags(FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT)
                .unwrap();
            operation.CopyItems(&data, &destination).unwrap();
            operation.PerformOperations().unwrap();
            assert!(!operation.GetAnyOperationsAborted().unwrap().as_bool());
        }
        assert_eq!(
            std::fs::read(target.path().join("guid-source.txt")).unwrap(),
            b"guid contents"
        );
        assert_eq!(std::fs::read(source_file).unwrap(), b"guid contents");
    }
    use windows::{
        Win32::{
            System::{
                Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
                Ole::IDropTarget,
            },
            UI::Shell::{
                BHID_SFUIObject, DragQueryFileW, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT,
                FileOperation, HDROP, IFileOperation, IShellItem, SHCreateItemFromParsingName,
            },
        },
        core::HSTRING,
    };

    #[test]
    fn renames_without_overwriting_and_permanently_deletes_generated_samples() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.txt");
        let existing = root.path().join("existing.txt");
        std::fs::write(&source, "keep contents").unwrap();
        std::fs::write(&existing, "do not overwrite").unwrap();
        for name in [
            "",
            "../outside",
            "a/b",
            "a\\b",
            "a:stream",
            "NUL.txt",
            "trailing.",
            "existing.txt",
        ] {
            assert!(
                edit_samples(
                    std::slice::from_ref(&source),
                    &SampleEdit::Rename(name.into())
                )
                .is_err(),
                "{name}"
            );
            assert_eq!(std::fs::read_to_string(&source).unwrap(), "keep contents");
        }
        edit_samples(
            std::slice::from_ref(&source),
            &SampleEdit::Rename("已重命名.txt".into()),
        )
        .unwrap();
        let renamed = root.path().join("已重命名.txt");
        assert!(!source.exists());
        assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "keep contents");
        let directory = root.path().join("folder");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("inside.txt"), "nested").unwrap();
        edit_samples(&[renamed.clone(), directory.clone()], &SampleEdit::Delete).unwrap();
        assert!(!renamed.exists());
        assert!(!directory.exists());
        assert_eq!(
            std::fs::read_to_string(existing).unwrap(),
            "do not overwrite"
        );
    }

    #[test]
    fn cut_data_moves_only_when_received_by_shell() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.txt");
        let target = root.path().join("target");
        std::fs::write(&source, "move contents").unwrap();
        std::fs::create_dir(&target).unwrap();
        let _apartment = Apartment::new().unwrap();
        let data = data_object(std::slice::from_ref(&source), DROPEFFECT_MOVE).unwrap();
        assert_eq!(std::fs::read_to_string(&source).unwrap(), "move contents");
        // SAFETY: The owned Shell target receives only this test's generated source.
        unsafe {
            let format = FORMATETC {
                cfFormat: RegisterClipboardFormatW(w!("Preferred DropEffect")) as u16,
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_HGLOBAL.0 as u32,
                ..Default::default()
            };
            let mut medium = data.GetData(&format).unwrap();
            let pointer = GlobalLock(medium.u.hGlobal).cast::<u32>();
            assert!(!pointer.is_null());
            assert_eq!(pointer.read(), DROPEFFECT_MOVE.0);
            let _ = GlobalUnlock(medium.u.hGlobal);
            ReleaseStgMedium(&mut medium);
            let destination: IShellItem =
                SHCreateItemFromParsingName(&HSTRING::from(target.as_os_str()), None).unwrap();
            let receiver: IDropTarget = destination.BindToHandler(None, &BHID_SFUIObject).unwrap();
            let mut effect = DROPEFFECT_MOVE;
            receiver
                .DragEnter(&data, MK_LBUTTON, Default::default(), &mut effect)
                .unwrap();
            receiver
                .Drop(
                    &data,
                    MODIFIERKEYS_FLAGS(0),
                    Default::default(),
                    &mut effect,
                )
                .unwrap();
            assert_eq!(effect, DROPEFFECT_MOVE);
        }
        assert!(!source.exists());
        assert_eq!(
            std::fs::read_to_string(target.join("source.txt")).unwrap(),
            "move contents"
        );
    }

    #[test]
    fn exports_multiple_files_and_directory_as_copy_only() {
        let root = tempfile::tempdir().unwrap();
        let mut paths = vec![
            root.path().join("中文.txt"),
            root.path().join("second.txt"),
            root.path().join("folder"),
        ];
        std::fs::write(&paths[0], "one").unwrap();
        std::fs::write(&paths[1], "two").unwrap();
        std::fs::create_dir(&paths[2]).unwrap();
        std::fs::write(paths[2].join("nested.txt"), "three").unwrap();
        let mut long_directory = root.path().join("long-path");
        while long_directory.as_os_str().len() < 280 {
            long_directory = long_directory.join("segment-0123456789");
        }
        std::fs::create_dir_all(&long_directory).unwrap();
        paths.push(long_directory.join("long.txt"));
        std::fs::write(&paths[3], "long").unwrap();
        let _apartment = Apartment::new().unwrap();
        let data = data_object(&paths, DROPEFFECT_COPY).unwrap();
        let mut format = FORMATETC {
            cfFormat: 15,
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
            ..Default::default()
        };
        // SAFETY: GetData returns owned HGLOBAL media; read within reported buffer lengths
        // and release each medium with ReleaseStgMedium after use.
        unsafe {
            let mut medium = data.GetData(&format).unwrap();
            let drop = HDROP(medium.u.hGlobal.0);
            assert_eq!(DragQueryFileW(drop, u32::MAX, None), 4);
            for (index, expected) in paths.iter().enumerate() {
                let length = DragQueryFileW(drop, index as u32, None);
                let mut value = vec![0u16; length as usize + 1];
                DragQueryFileW(drop, index as u32, Some(&mut value));
                assert_eq!(
                    PathBuf::from(String::from_utf16(&value[..length as usize]).unwrap()),
                    *expected
                );
            }
            ReleaseStgMedium(&mut medium);
            format.cfFormat = RegisterClipboardFormatW(w!("Shell IDList Array")) as u16;
            assert!(data.QueryGetData(&format).is_err());
            format.cfFormat = RegisterClipboardFormatW(w!("Preferred DropEffect")) as u16;
            let mut medium = data.GetData(&format).unwrap();
            let pointer = GlobalLock(medium.u.hGlobal).cast::<u32>();
            assert!(!pointer.is_null());
            assert_eq!(pointer.read(), DROPEFFECT_COPY.0);
            let _ = GlobalUnlock(medium.u.hGlobal);
            ReleaseStgMedium(&mut medium);
        }
        let target = tempfile::tempdir().unwrap();
        // SAFETY: This STA owns the Shell objects. Copy only generated fixtures into a
        // new temporary directory; suppress all UI so tests never drive desktop dialogs.
        unsafe {
            let destination: IShellItem =
                SHCreateItemFromParsingName(&HSTRING::from(target.path().as_os_str()), None)
                    .unwrap();
            let operation: IFileOperation =
                CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER).unwrap();
            operation
                .SetOperationFlags(FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT)
                .unwrap();
            operation.CopyItems(&data, &destination).unwrap();
            operation.PerformOperations().unwrap();
            assert!(!operation.GetAnyOperationsAborted().unwrap().as_bool());
        }
        for (relative, contents) in [
            ("中文.txt", "one"),
            ("second.txt", "two"),
            ("folder/nested.txt", "three"),
            ("long.txt", "long"),
        ] {
            assert_eq!(
                std::fs::read_to_string(target.path().join(relative)).unwrap(),
                contents
            );
        }
        let drop_target = tempfile::tempdir().unwrap();
        // SAFETY: Exercise the real Shell folder drop target with owned generated fixtures.
        // All COM interfaces remain in this initialized apartment until the operation ends.
        unsafe {
            let destination: IShellItem =
                SHCreateItemFromParsingName(&HSTRING::from(drop_target.path().as_os_str()), None)
                    .unwrap();
            let receiver: IDropTarget = destination.BindToHandler(None, &BHID_SFUIObject).unwrap();
            let mut effect = DROPEFFECT_COPY;
            receiver
                .DragEnter(&data, MK_LBUTTON, Default::default(), &mut effect)
                .unwrap();
            assert_eq!(effect, DROPEFFECT_COPY);
            receiver
                .Drop(
                    &data,
                    MODIFIERKEYS_FLAGS(0),
                    Default::default(),
                    &mut effect,
                )
                .unwrap();
            assert_eq!(effect, DROPEFFECT_COPY);
        }
        assert_eq!(
            std::fs::read_to_string(drop_target.path().join("second.txt")).unwrap(),
            "two"
        );
        assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "one");
        assert_eq!(
            std::fs::read_to_string(paths[2].join("nested.txt")).unwrap(),
            "three"
        );
        assert!(data_object(&[], DROPEFFECT_COPY).is_err());
        assert!(data_object(&[root.path().join("missing")], DROPEFFECT_COPY).is_err());
    }
}
