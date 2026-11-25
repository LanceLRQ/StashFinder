use std::{
    process::{Child, Command},
    thread::sleep,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, RECT, WPARAM},
        UI::{Controls::TCM_GETITEMCOUNT, WindowsAndMessaging::*},
    },
    core::{PCWSTR, w},
};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "opens the Win32 workspace on an interactive desktop"]
fn native_workspace_keeps_overview_and_opens_multiple_tabs_in_one_window() {
    let process = Process(
        Command::new(env!("CARGO_BIN_EXE_stashfinder-desktop"))
            .arg("--native-ui")
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(6);
    // SAFETY: Only sends value-only messages to the child process's controls.
    unsafe {
        let window = loop {
            let mut after = None;
            let mut found = None;
            while let Ok(hwnd) =
                FindWindowExW(None, after, w!("StashFinderWorkspace"), PCWSTR::null())
            {
                let mut pid = 0;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid == process.0.id() {
                    found = Some(hwnd);
                    break;
                }
                after = Some(hwnd);
            }
            if let Some(hwnd) = found {
                break hwnd;
            }
            assert!(Instant::now() < deadline, "native workspace did not open");
            sleep(Duration::from_millis(40));
        };
        let tabs = wait_for_tab_strip(window);
        let overview = GetDlgItem(Some(window), 236).expect("overview stays outside the tab strip");
        let count = || SendMessageW(tabs, TCM_GETITEMCOUNT, None, None).0;
        let command = |id| SendMessageW(window, WM_COMMAND, Some(WPARAM(id)), None);
        let rect = |hwnd| {
            let mut rect = RECT::default();
            GetWindowRect(hwnd, &mut rect).unwrap();
            rect
        };
        let is_overview = || SendMessageW(overview, BM_GETCHECK, None, None).0 == 1;
        assert!(is_overview());
        let group = GetDlgItem(Some(window), 237).expect("physical disk grouping control");
        assert_eq!(SendMessageW(group, BM_GETCHECK, None, None).0, 0);
        SendMessageW(group, BM_CLICK, None, None);
        assert_eq!(SendMessageW(group, BM_GETCHECK, None, None).0, 1);
        SendMessageW(group, BM_CLICK, None, None);
        assert_eq!(SendMessageW(group, BM_GETCHECK, None, None).0, 0);
        assert_eq!(count(), 0);
        command(202); // Close at the overview.
        assert_eq!(count(), 0);
        command(200); // New browse tab.
        command(201); // New search tab.
        command(201);
        assert!(!is_overview());
        assert_eq!(count(), 3);
        command(203); // Index tasks.
        command(203); // Select the existing tasks tab.
        assert_eq!(count(), 4);
        command(202);
        assert_eq!(count(), 3);
        PostMessageW(Some(window), WM_COMMAND, WPARAM(204), LPARAM(0)).unwrap();
        let dialog = wait_for_owned_dialog(window);
        let categories = GetDlgItem(Some(dialog), 10).unwrap();
        assert_eq!(SendMessageW(categories, CB_GETCOUNT, None, None).0, 7);
        PostMessageW(
            Some(dialog),
            WM_COMMAND,
            WPARAM(IDCANCEL.0 as usize),
            LPARAM(0),
        )
        .unwrap();
        assert_eq!(count(), 3);

        let pinned = rect(overview);
        for _ in 0..25 {
            command(201);
        }
        assert_eq!(count(), 28);
        assert!(IsWindowVisible(overview).as_bool());
        assert_eq!(rect(overview), pinned);
        assert!(pinned.right <= rect(tabs).left);
        SendMessageW(overview, BM_CLICK, None, None);
        assert!(is_overview());
        command(202);
        assert_eq!(count(), 28);
    }
}

fn wait_for_owned_dialog(owner: HWND) -> HWND {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        // SAFETY: The Windows lookup is limited to an owned modal window from the test app.
        let dialog = unsafe { GetLastActivePopup(owner) };
        // SAFETY: The modal belongs to the test process's window.
        let ready = unsafe { GetDlgItem(Some(dialog), 10) }.is_ok();
        if dialog != owner && !dialog.is_invalid() && ready {
            return dialog;
        }
        assert!(Instant::now() < deadline, "settings modal did not open");
        sleep(Duration::from_millis(20));
    }
}
fn wait_for_tab_strip(window: HWND) -> HWND {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        // SAFETY: The window belongs to the already identified test process.
        let tabs =
            unsafe { FindWindowExW(Some(window), None, w!("SysTabControl32"), PCWSTR::null()) };
        if let Ok(tabs) = tabs {
            return tabs;
        }
        assert!(Instant::now() < deadline, "native workspace has no tabs");
        sleep(Duration::from_millis(20));
    }
}
