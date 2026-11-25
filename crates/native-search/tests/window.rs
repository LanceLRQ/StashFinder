use std::{
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        UI::{Controls::LVM_GETITEMCOUNT, WindowsAndMessaging::*},
    },
    core::{PCWSTR, w},
};

struct WindowProcess(Child);
impl Drop for WindowProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "opens a native window on the interactive Windows desktop"]
fn search_window_accepts_input_and_reports_a_disconnected_backend() {
    let pipe = format!(r"\\.\pipe\StashFinder.unavailable-{}", std::process::id());
    let mut child = WindowProcess(
        Command::new(env!("CARGO_BIN_EXE_stashfinder-native-search"))
            .args(["--pipe", &pipe])
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    // SAFETY: The child is identified by its PID. Standard control messages marshal their text;
    // the list count and window close messages do not carry pointers.
    unsafe {
        let window = loop {
            let mut after = None;
            let mut matched = None;
            while let Ok(hwnd) =
                FindWindowExW(None, after, w!("StashFinderNativeSearch"), PCWSTR::null())
            {
                let mut pid = 0;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid == child.0.id() && IsWindowVisible(hwnd).as_bool() {
                    matched = Some(hwnd);
                    break;
                }
                after = Some(hwnd);
            }
            if let Some(hwnd) = matched {
                break hwnd;
            }
            assert!(
                Instant::now() < deadline && child.0.try_wait().unwrap().is_none(),
                "native search window did not open"
            );
            thread::sleep(Duration::from_millis(20));
        };
        let edit = FindWindowExW(Some(window), None, w!("Edit"), PCWSTR::null()).unwrap();
        let list = FindWindowExW(Some(window), None, w!("SysListView32"), PCWSTR::null()).unwrap();
        let status: HWND = GetDlgItem(Some(window), 3).unwrap();
        let menu = GetSubMenu(GetMenu(window), 0);
        assert!(
            !menu.is_invalid() && GetMenuItemCount(Some(menu)) == 2,
            "the native window must expose Open and Copy path without opening the desktop"
        );
        let browse = GetSubMenu(GetMenu(window), 1);
        assert!(
            !browse.is_invalid(),
            "the native window must offer indexed partition browsing"
        );
        let partitions = GetMenuItemID(browse, 0);
        let deadline = Instant::now() + Duration::from_secs(8);
        SendMessageW(window, WM_COMMAND, Some(WPARAM(partitions as usize)), None);
        loop {
            let mut label = [0u16; 512];
            let len = GetWindowTextW(status, &mut label) as usize;
            if String::from_utf16_lossy(&label[..len]).contains("无法读取") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "disconnected browsing must report a read error"
            );
            thread::sleep(Duration::from_millis(20));
        }
        assert!(!IsWindowVisible(edit).as_bool());
        let search = GetMenuItemID(browse, GetMenuItemCount(Some(browse)) - 1);
        SendMessageW(window, WM_COMMAND, Some(WPARAM(search as usize)), None);
        assert!(
            IsWindowVisible(edit).as_bool(),
            "returning to search restores its edit control"
        );
        let deadline = Instant::now() + Duration::from_secs(8);
        SendMessageW(
            edit,
            WM_SETTEXT,
            None,
            Some(LPARAM(w!("离线样本").as_ptr() as isize)),
        );
        loop {
            let mut text = [0u16; 512];
            let len = SendMessageW(
                status,
                WM_GETTEXT,
                Some(WPARAM(text.len())),
                Some(LPARAM(text.as_mut_ptr() as isize)),
            )
            .0 as usize;
            if String::from_utf16_lossy(&text[..len]).contains("无法完成搜索") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "a missing backend must be visible, not an empty successful search"
            );
            thread::sleep(Duration::from_millis(30));
        }
        assert_eq!(SendMessageW(list, LVM_GETITEMCOUNT, None, None).0, 0);
        PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0)).unwrap();
    }
    assert!(child.0.wait().unwrap().success());
}
