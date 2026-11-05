use anyhow::{Context, Result, ensure};
use std::path::PathBuf;
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::ClientToScreen,
        UI::{Shell::*, WindowsAndMessaging::*},
    },
    core::{HSTRING, Interface, PCSTR, PCWSTR},
};

struct Items(Vec<*mut Common::ITEMIDLIST>);
impl Drop for Items {
    fn drop(&mut self) {
        for pidl in &self.0 {
            // SAFETY: Every PIDL was allocated by ILCreateFromPathW and is freed once.
            unsafe {
                ILFree(Some(*pidl));
            }
        }
    }
}
struct Popup(HMENU);
impl Drop for Popup {
    fn drop(&mut self) {
        // SAFETY: This popup exclusively owns the menu handle created by CreatePopupMenu.
        let _ = unsafe { DestroyMenu(self.0) };
    }
}
struct Handler {
    second: Option<IContextMenu2>,
    third: Option<IContextMenu3>,
}
impl Handler {
    fn forward(&self, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        if ![WM_INITMENUPOPUP, WM_DRAWITEM, WM_MEASUREITEM, WM_MENUCHAR].contains(&message) {
            return None;
        }
        // SAFETY: Shell receives only its documented menu messages on the owning GUI STA.
        unsafe {
            if let Some(menu) = &self.third {
                let mut result = LRESULT(0);
                if menu
                    .HandleMenuMsg2(message, wparam, lparam, Some(&mut result))
                    .is_ok()
                {
                    return Some(result);
                }
            }
            if message != WM_MENUCHAR
                && let Some(menu) = &self.second
                && menu.HandleMenuMsg(message, wparam, lparam).is_ok()
            {
                return Some(LRESULT(
                    if message == WM_DRAWITEM || message == WM_MEASUREITEM {
                        1
                    } else {
                        0
                    },
                ));
            }
        }
        None
    }
}
unsafe extern "system" fn menu_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: Subclass owns a pinned Box<Handler> until this procedure is removed on the same thread.
    let handler = unsafe { &*(data as *const Handler) };
    if let Some(result) = handler.forward(message, wparam, lparam) {
        return result;
    }
    // SAFETY: Unhandled messages retain the existing window subclass chain.
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}
struct Subclass {
    window: HWND,
    _handler: Box<Handler>,
}
impl Drop for Subclass {
    fn drop(&mut self) {
        // SAFETY: Remove before releasing Handler, on the same thread on which it was installed.
        unsafe {
            let _ = RemoveWindowSubclass(self.window, Some(menu_proc), 0x5354464d);
        }
    }
}

/// Invoke only on the window's GUI STA. Cross-directory selections are rejected in full.
pub fn show(paths: &[PathBuf], owner: usize, x: f64, y: f64, scale: f64) -> Result<bool> {
    ensure!(
        !paths.is_empty() && paths.len() <= stashfinder_core::file_operations::MAX_SELECTED,
        "请选择 1～10,000 项"
    );
    let parent = paths[0].parent().context("所选条目没有普通父目录")?;
    ensure!(
        paths.iter().all(|path| path.parent() == Some(parent)),
        "更多... 仅支持同一目录的选择"
    );
    let _apartment = crate::shell::Apartment::new()?;
    let owner = HWND(owner as *mut _);
    let mut items = Items(Vec::with_capacity(paths.len()));
    // SAFETY: Owned PIDLs, Shell interfaces, popup and subclass outlive tracking and invocation.
    unsafe {
        for path in paths {
            let pidl = ILCreateFromPathW(&HSTRING::from(path.as_os_str()));
            ensure!(!pidl.is_null(), "无法创建 Windows 菜单项");
            items.0.push(pidl);
        }
        let parent: IShellFolder = SHBindToParent(items.0[0], None)?;
        let children = items
            .0
            .iter()
            .map(|pidl| ILFindLastID(*pidl) as *const Common::ITEMIDLIST)
            .collect::<Vec<_>>();
        let context: IContextMenu = parent.GetUIObjectOf(owner, &children, None)?;
        let handler = Box::new(Handler {
            second: context.cast().ok(),
            third: context.cast().ok(),
        });
        ensure!(
            SetWindowSubclass(
                owner,
                Some(menu_proc),
                0x5354464d,
                (&*handler as *const Handler) as usize
            )
            .as_bool(),
            "无法建立原生菜单消息处理"
        );
        let _subclass = Subclass {
            window: owner,
            _handler: handler,
        };
        let popup = Popup(CreatePopupMenu()?);
        context
            .QueryContextMenu(popup.0, 0, 1, 0x7fff, CMF_NORMAL)
            .ok()?;
        let mut point = POINT {
            x: (x * scale).round() as i32,
            y: (y * scale).round() as i32,
        };
        ensure!(
            ClientToScreen(owner, &mut point).as_bool(),
            "无法定位原生菜单窗口"
        );
        let command = TrackPopupMenuEx(
            popup.0,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            point.x,
            point.y,
            owner,
            None,
        )
        .0 as u32;
        if command == 0 {
            return Ok(false);
        }
        ensure!((1..=0x7fff).contains(&command), "原生菜单命令无效");
        let info = CMINVOKECOMMANDINFOEX {
            cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
            fMask: SEE_MASK_UNICODE | CMIC_MASK_PTINVOKE,
            hwnd: owner,
            lpVerb: PCSTR((command - 1) as usize as *const u8),
            lpVerbW: PCWSTR((command - 1) as usize as *const u16),
            nShow: SW_SHOWNORMAL.0,
            ptInvoke: point,
            ..Default::default()
        };
        context.InvokeCommand((&info as *const CMINVOKECOMMANDINFOEX).cast())?;
    }
    Ok(true)
}
