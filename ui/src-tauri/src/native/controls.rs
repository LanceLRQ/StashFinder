use anyhow::{Context, Result, ensure};
pub(super) use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{
            COLOR_WINDOW, DEFAULT_GUI_FONT, GetStockObject, GetSysColorBrush, HBITMAP,
        },
        System::{LibraryLoader::GetModuleHandleW, SystemServices::SS_LEFTNOWORDWRAP},
        UI::{Controls::*, HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
    core::{HSTRING, PCWSTR, PWSTR, w},
};

pub(super) fn instance() -> Result<HINSTANCE> {
    // SAFETY: A null module name refers to this executable.
    Ok(unsafe { GetModuleHandleW(None)? }.into())
}

pub(super) fn send(hwnd: HWND, message: u32, w: usize, l: isize) -> isize {
    // SAFETY: The caller chooses the message and retains any synchronous parameter.
    unsafe { SendMessageW(hwnd, message, Some(WPARAM(w)), Some(LPARAM(l))).0 }
}

pub(super) fn child(
    parent: HWND,
    id: usize,
    class: PCWSTR,
    label: &str,
    style: u32,
    edge: bool,
) -> Result<HWND> {
    let module = instance()?;
    // SAFETY: Child controls belong to the UI thread. Windows copies the label during creation.
    unsafe {
        let hwnd = CreateWindowExW(
            if edge {
                WS_EX_CLIENTEDGE
            } else {
                WINDOW_EX_STYLE(0)
            },
            class,
            &HSTRING::from(label),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(style),
            0,
            0,
            0,
            0,
            Some(parent),
            Some(HMENU(id as *mut _)),
            Some(module),
            None,
        )?;
        send(
            hwnd,
            WM_SETFONT,
            GetStockObject(DEFAULT_GUI_FONT).0 as usize,
            0,
        );
        Ok(hwnd)
    }
}

pub(super) fn set_text(hwnd: HWND, text: &str) {
    // SAFETY: Windows copies this string synchronously.
    let _ = unsafe { SetWindowTextW(hwnd, &HSTRING::from(text)) };
}

pub(super) fn text(hwnd: HWND) -> String {
    // SAFETY: The string length and writable buffer are supplied to the calling thread's control.
    unsafe {
        let mut buffer = vec![0; GetWindowTextLengthW(hwnd).max(0) as usize + 1];
        let length = GetWindowTextW(hwnd, &mut buffer).max(0) as usize;
        String::from_utf16_lossy(&buffer[..length])
    }
}

pub(super) fn show(hwnd: HWND, visible: bool) {
    // SAFETY: This handle identifies a control owned by the calling UI thread.
    let _ = unsafe { ShowWindow(hwnd, if visible { SW_SHOWNA } else { SW_HIDE }) };
}

pub(super) fn enable(hwnd: HWND, enabled: bool) {
    // SAFETY: This handle identifies a control owned by the calling UI thread.
    let _ = unsafe { EnableWindow(hwnd, enabled) };
}

pub(super) fn focus(hwnd: HWND) {
    // SAFETY: Focus moves to a control on the current UI thread.
    let _ = unsafe { SetFocus(Some(hwnd)) };
}

pub(super) fn place(hwnd: HWND, x: i32, y: i32, width: i32, height: i32) {
    // SAFETY: Geometry is in pixels and belongs to the control's parent.
    let _ = unsafe { MoveWindow(hwnd, x, y, width.max(0), height.max(0), true) };
}

pub(super) fn check(hwnd: HWND, value: bool) {
    send(hwnd, BM_SETCHECK, usize::from(value), 0);
}

pub(super) fn checked(hwnd: HWND) -> bool {
    send(hwnd, BM_GETCHECK, 0, 0) == BST_CHECKED.0 as isize
}

pub(super) fn confirm(owner: HWND, title: &str, message: &str) -> bool {
    // SAFETY: The application's window owns the modal confirmation.
    unsafe {
        MessageBoxW(
            Some(owner),
            &HSTRING::from(message),
            &HSTRING::from(title),
            MB_YESNO | MB_DEFBUTTON2 | MB_ICONWARNING,
        ) == IDYES
    }
}

pub(super) fn info(owner: HWND, title: &str, message: &str) {
    // SAFETY: The application's window owns the modal message.
    unsafe {
        MessageBoxW(
            Some(owner),
            &HSTRING::from(message),
            &HSTRING::from(title),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

pub(super) fn combo_fill(hwnd: HWND, values: &[String], selected: usize) {
    send(hwnd, CB_RESETCONTENT, 0, 0);
    for value in values {
        let value = HSTRING::from(value);
        send(hwnd, CB_ADDSTRING, 0, value.as_ptr() as isize);
    }
    send(hwnd, CB_SETCURSEL, selected, 0);
}

pub(super) fn popup(owner: HWND, groups: &[(&str, &[(usize, &str)])]) -> Result<()> {
    // SAFETY: The parent menu owns submenus once appended and owns all label copies.
    unsafe {
        let menu = CreateMenu()?;
        for (label, items) in groups {
            let submenu = CreatePopupMenu()?;
            for &(id, label) in *items {
                if id == 0 {
                    AppendMenuW(submenu, MF_SEPARATOR, 0, None)?;
                } else {
                    AppendMenuW(submenu, MF_STRING, id, &HSTRING::from(label))?;
                }
            }
            AppendMenuW(menu, MF_POPUP, submenu.0 as usize, &HSTRING::from(*label))?;
        }
        SetMenu(owner, Some(menu))?;
    }
    Ok(())
}

#[derive(Clone)]
pub(super) enum Field {
    Edit(&'static str, String),
    Check(&'static str, bool),
    Choice(&'static str, Vec<String>, usize),
    Label(String),
}

#[derive(Debug)]
pub(super) enum Value {
    Text(String),
    Check(bool),
    Choice(usize),
}

pub(super) struct Values(std::vec::IntoIter<Value>);
impl Values {
    pub fn text(&mut self) -> String {
        match self.0.next().expect("form field") {
            Value::Text(value) => value,
            _ => unreachable!("edit field"),
        }
    }
    pub fn check(&mut self) -> bool {
        match self.0.next().expect("form field") {
            Value::Check(value) => value,
            _ => unreachable!("checkbox field"),
        }
    }
    pub fn choice(&mut self) -> usize {
        match self.0.next().expect("form field") {
            Value::Choice(value) => value,
            _ => unreachable!("choice field"),
        }
    }
}

struct Form<'a> {
    title: &'a str,
    fields: &'a [Field],
    children: Vec<HWND>,
    result: Option<Result<Values>>,
}

pub(super) fn form(owner: HWND, title: &str, fields: &[Field]) -> Result<Option<Values>> {
    let mut form = Form {
        title,
        fields,
        children: vec![],
        result: None,
    };
    // DialogBox consumes the standard template synchronously, with no menu, custom class, or font.
    #[repr(C, align(4))]
    struct Template(DLGTEMPLATE, [u16; 3]);
    let template = Template(
        DLGTEMPLATE {
            style: WS_POPUP.0
                | WS_CAPTION.0
                | WS_SYSMENU.0
                | DS_MODALFRAME as u32
                | DS_CENTER as u32,
            dwExtendedStyle: 0,
            cdit: 0,
            x: 0,
            y: 0,
            cx: 264,
            cy: (28 + fields.len() * 16) as i16,
        },
        [0; 3],
    );
    // SAFETY: Both stack structures remain valid throughout the synchronous modal message loop.
    let result = unsafe {
        DialogBoxIndirectParamW(
            Some(instance()?),
            &template.0,
            Some(owner),
            Some(form_proc),
            LPARAM((&raw mut form) as isize),
        )
    };
    ensure!(
        result != -1,
        "无法打开弹层：{}",
        windows::core::Error::from_thread()
    );
    form.result.transpose()
}

unsafe extern "system" fn form_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> isize {
    // SAFETY: GWLP_USERDATA points to the live Form supplied by the modal caller. It is only
    // accessed for initialization and button commands, on one UI thread.
    unsafe {
        match message {
            WM_INITDIALOG => {
                let form = &mut *(l.0 as *mut Form<'_>);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, l.0);
                if let Err(error) = init_form(hwnd, form) {
                    form.result = Some(Err(error));
                    let _ = EndDialog(hwnd, 0);
                }
                1
            }
            WM_COMMAND if [IDOK.0, IDCANCEL.0].contains(&((w.0 & 0xffff) as i32)) => {
                let form = &mut *(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Form<'_>);
                if (w.0 & 0xffff) as i32 == IDOK.0 {
                    let values = form
                        .fields
                        .iter()
                        .zip(&form.children)
                        .filter_map(|(field, &h)| match field {
                            Field::Edit(..) => Some(Value::Text(text(h))),
                            Field::Check(..) => Some(Value::Check(checked(h))),
                            Field::Choice(..) => {
                                Some(Value::Choice(send(h, CB_GETCURSEL, 0, 0).max(0) as usize))
                            }
                            Field::Label(_) => None,
                        })
                        .collect::<Vec<_>>();
                    form.result = Some(Ok(Values(values.into_iter())));
                }
                let _ = EndDialog(hwnd, 0);
                1
            }
            _ => 0,
        }
    }
}

fn init_form(hwnd: HWND, form: &mut Form<'_>) -> Result<()> {
    set_text(hwnd, form.title);
    let mut unit = RECT {
        right: 4,
        bottom: 8,
        ..Default::default()
    };
    // SAFETY: Windows supplies this dialog's font units.
    unsafe { MapDialogRect(hwnd, &mut unit)? };
    let px = |x| x * unit.right / 4;
    let py = |y| y * unit.bottom / 8;
    for (i, field) in form.fields.iter().enumerate() {
        let id = 10 + i;
        let y = 7 + 16 * i as i32;
        let (label, control, height, full) = match field {
            Field::Label(label) => (
                "",
                child(hwnd, id, w!("STATIC"), label, 0, false)?,
                14,
                true,
            ),
            Field::Edit(label, value) => (
                *label,
                child(
                    hwnd,
                    id,
                    w!("EDIT"),
                    value,
                    WS_TABSTOP.0 | ES_AUTOHSCROLL as u32,
                    true,
                )?,
                13,
                false,
            ),
            Field::Check(label, value) => {
                let control = child(
                    hwnd,
                    id,
                    w!("BUTTON"),
                    label,
                    WS_TABSTOP.0 | BS_AUTOCHECKBOX as u32,
                    false,
                )?;
                check(control, *value);
                (*label, control, 14, true)
            }
            Field::Choice(label, options, selected) => {
                let control = child(
                    hwnd,
                    id,
                    w!("COMBOBOX"),
                    "",
                    WS_TABSTOP.0 | CBS_DROPDOWNLIST as u32 | WS_VSCROLL.0,
                    false,
                )?;
                combo_fill(control, options, *selected);
                (*label, control, 130, false)
            }
        };
        if !full {
            let label = child(hwnd, 0, w!("STATIC"), label, 0, false)?;
            place(label, px(8), py(y + 2), px(93), py(12));
        }
        let x = if full { 8 } else { 103 };
        place(control, px(x), py(y), px(256 - x), py(height));
        form.children.push(control);
    }
    let y = py(8 + 16 * form.fields.len() as i32);
    let ok = child(
        hwnd,
        IDOK.0 as usize,
        w!("BUTTON"),
        "确定",
        WS_TABSTOP.0 | BS_DEFPUSHBUTTON as u32,
        false,
    )?;
    let cancel = child(
        hwnd,
        IDCANCEL.0 as usize,
        w!("BUTTON"),
        "取消",
        WS_TABSTOP.0,
        false,
    )?;
    place(ok, px(145), y, px(52), py(14));
    place(cancel, px(204), y, px(52), py(14));
    send(hwnd, DM_SETDEFID, IDOK.0 as usize, 0);
    Ok(())
}

pub(super) fn app_directory(id: &str, local: bool) -> Result<std::path::PathBuf> {
    use windows::Win32::{System::Com::CoTaskMemFree, UI::Shell::*};
    let kind = if local {
        &FOLDERID_LocalAppData
    } else {
        &FOLDERID_RoamingAppData
    };
    // SAFETY: The returned shell allocation is valid until it is released through COM.
    unsafe {
        let path = SHGetKnownFolderPath(kind, KNOWN_FOLDER_FLAG(0), None)?;
        let directory = if path.is_null() {
            None
        } else {
            use std::os::windows::ffi::OsStringExt;
            Some(std::path::PathBuf::from(std::ffi::OsString::from_wide(
                path.as_wide(),
            )))
        };
        CoTaskMemFree(Some(path.0.cast()));
        Ok(directory.context("应用数据路径不可用")?.join(id))
    }
}
