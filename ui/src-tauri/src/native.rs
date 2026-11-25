mod controls;
mod dialogs;
mod files;
mod images;
mod library;
mod operations;
mod state;

use self::{controls::*, state::*};
use crate::{
    preferences::{self, Preferences},
    runtime,
};
use anyhow::{Result, ensure};
use stashfinder_core::{Action, Response, assets::Snapshot, tasks::Task};
use stashfinder_windows::portable::{DesktopInstance, is_elevated};
use std::{
    cell::{Cell, RefCell},
    future::Future,
    path::PathBuf,
    rc::Rc,
    sync::mpsc::{Receiver, Sender},
};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};

const DRAIN: u32 = WM_APP + 10;
const TRAY: u32 = WM_APP + 11;
const TICK: usize = 1;
const DEBOUNCE: usize = 2;
const NEW_BROWSE: usize = 200;
const NEW_SEARCH: usize = 201;
const CLOSE_TAB: usize = 202;
const TASKS: usize = 203;
const SETTINGS: usize = 204;
const HELP: usize = 205;
const ABOUT: usize = 206;
const REFRESH: usize = 207;
const EXIT: usize = 208;
const STATUS: usize = 209;
const PIN: usize = 210;
const REGISTER: usize = 211;
const PICK_VOLUME: usize = 212;
const BACK: usize = 213;
const FORWARD: usize = 214;
const UP: usize = 215;
const FILTER: usize = 216;
const FOCUS_SEARCH: usize = 217;
const SHOW: usize = 218;
const SUBMIT: usize = 219;
const TAB_POPUP: usize = 220;
const QUERY: usize = 221;
const SCOPE: usize = 222;
const TREE: usize = 223;
const BREADCRUMBS: usize = 224;
const PREVIEW: usize = 225;
const SHORTCUTS: usize = 226;
const REFRESH_DEVICES: usize = 227;
const PARTITION_ACTIONS: usize = 228;
const TASK_ACTIONS: usize = 229;
const OVERVIEW: usize = 236;
const GROUP_ASSETS: usize = 237;

type Callback = Box<dyn FnOnce(&Desktop) + Send>;

struct Desktop {
    hwnd: HWND,
    overview: HWND,
    tab_strip: HWND,
    add: HWND,
    close_tab: HWND,
    pressed_tab_close: Cell<Option<usize>>,
    list: HWND,
    query: HWND,
    submit: HWND,
    scope: HWND,
    filter: HWND,
    path: HWND,
    back: HWND,
    forward: HWND,
    up: HWND,
    register: HWND,
    group_assets: HWND,
    partition_actions: HWND,
    task_actions: HWND,
    task_filter_button: HWND,
    task_cancel: HWND,
    task_retry: HWND,
    task_resume: HWND,
    tree: HWND,
    preview: HWND,
    picture: HWND,
    status: HWND,
    tabs: RefCell<Tabs>,
    snapshot: RefCell<Snapshot>,
    asset_rows: RefCell<Vec<library::AssetRow>>,
    tasks: RefCell<Vec<Task>>,
    task_indexes: RefCell<Vec<stashfinder_core::tasks::PartitionIndex>>,
    visible_tasks: RefCell<Vec<String>>,
    task_filter: Cell<usize>,
    task_partition: RefCell<Option<String>>,
    automatic: Cell<Option<bool>>,
    tasks_pending: Cell<bool>,
    preferences: RefCell<Preferences>,
    config_path: PathBuf,
    local_path: PathBuf,
    changing: Cell<bool>,
    composing: Cell<bool>,
    pending_input: Cell<bool>,
    pinned: Cell<bool>,
    assets_pending: Cell<bool>,
    task_refresh: Cell<u8>,
    tree_locations: RefCell<Vec<Location>>,
    tree_pending: RefCell<std::collections::HashSet<isize>>,
    tree_group: Cell<u64>,
    operation: std::sync::Arc<crate::file_operations::Operations>,
    exporting: crate::search_export::SearchExport,
    exported_snapshot: Cell<Option<u64>>,
    columns: RefCell<files::Columns>,
    preview_due: Cell<bool>,
    view_hint: Cell<Option<(usize, usize)>>,
    images: images::Images,
    visuals: std::sync::Arc<crate::visuals::Visuals>,
    tray: bool,
    sender: Sender<Callback>,
    receiver: Receiver<Callback>,
    runtime: tokio::runtime::Runtime,
}

thread_local! {
    static DESKTOP: RefCell<Option<Rc<Desktop>>> = const { RefCell::new(None) };
}

pub(super) fn run() -> Result<()> {
    ensure!(!is_elevated(), "请以普通权限双击 StashFinder");
    let mut app: tauri::Context<tauri::Wry> = tauri::generate_context!();
    let identifier = app.config().identifier.clone();
    let Some(_instance) =
        DesktopInstance::acquire(&format!("{identifier}{}", crate::WORKSPACE_MUTEX_SUFFIX))?
    else {
        return Ok(());
    };
    let _session = runtime::start(app.config_mut())?;
    let overrides = app.config().app.app_directories_override.as_ref();
    let paths = match overrides {
        Some(tauri::utils::config::AppDirectoriesOverride::Directories(paths)) => Some(paths),
        _ => None,
    };
    let config = match paths.and_then(|p| p.config.as_ref()) {
        Some(path) => path.clone(),
        None => app_directory(&identifier, false)?,
    };
    let local = match paths.and_then(|p| p.local_data.as_ref()) {
        Some(path) => path.clone(),
        None => app_directory(&identifier, true)?,
    };
    drop(app);
    // SAFETY: DPI is configured before creation, and controls and messages stay on the OLE UI thread.
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES
                | ICC_TAB_CLASSES
                | ICC_TREEVIEW_CLASSES
                | ICC_PROGRESS_CLASS,
        })
        .ok()?;
        let hinstance = instance()?;
        let class = w!("StashFinderWorkspace");
        ensure!(
            RegisterClassW(&WNDCLASSW {
                hInstance: hinstance,
                lpszClassName: class,
                lpfnWndProc: Some(window_proc),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                hIcon: LoadIconW(Some(hinstance), PCWSTR(32512_usize as *const _))
                    .or_else(|_| LoadIconW(None, IDI_APPLICATION))?,
                hbrBackground: GetSysColorBrush(COLOR_WINDOW),
                ..Default::default()
            }) != 0,
            "原生工作区注册失败"
        );
        let dpi = GetDpiForSystem() as i32;
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("StashFinder"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1280 * dpi / 96,
            800 * dpi / 96,
            None,
            None,
            Some(hinstance),
            None,
        )?;
        let window = Rc::new(Desktop::new(hwnd, config, local)?);
        DESKTOP.set(Some(window.clone()));
        window.rebuild_tabs();
        window.present(true);
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        window.refresh_assets(false);
        SetTimer(Some(hwnd), TICK, 250, None);
        let mut msg = MSG::default();
        loop {
            let available = GetMessageW(&mut msg, None, 0, 0).0;
            ensure!(available >= 0, "无法读取 Windows 消息");
            if available == 0 {
                break;
            }
            if !window.key(&msg)
                && (!(msg.hwnd == hwnd || IsChild(hwnd, msg.hwnd).as_bool())
                    || !IsDialogMessageW(hwnd, &msg).as_bool())
            {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        DESKTOP.set(None);
        window.release_searches();
    }
    Ok(())
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let window = DESKTOP.with(|desktop| desktop.borrow().clone());
    if let Some(window) = window
        && let Some(result) = window.message(message, w, l)
    {
        return LRESULT(result);
    }
    // SAFETY: Parameters come from the window manager and were not consumed by our handler.
    unsafe { DefWindowProcW(hwnd, message, w, l) }
}

unsafe extern "system" fn search_edit(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    let app = DESKTOP.with(|desktop| desktop.borrow().clone());
    if message == WM_IME_STARTCOMPOSITION
        && let Some(app) = &app
    {
        app.composing.set(true);
        app.pending_input.set(false);
    }
    // SAFETY: Forward the edit's message to the original control before reading composed text.
    let result = unsafe { DefSubclassProc(hwnd, message, w, l) };
    if message == WM_IME_ENDCOMPOSITION
        && let Some(app) = app
    {
        app.composing.set(false);
        app.queue_query();
    }
    result
}

unsafe extern "system" fn tab_mouse(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    if matches!(
        message,
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MBUTTONUP | WM_CAPTURECHANGED
    ) && let Some(app) = DESKTOP.with(|desktop| desktop.borrow().clone())
    {
        let point = POINT {
            x: i32::from(l.0 as i16),
            y: i32::from((l.0 >> 16) as i16),
        };
        match message {
            WM_LBUTTONDOWN => {
                if let Some(index) = app.tab_close_at(point) {
                    app.pressed_tab_close.set(Some(index));
                    // SAFETY: The tab strip captures its own button gesture until release.
                    let _ = unsafe { SetCapture(hwnd) };
                    return LRESULT(0);
                }
            }
            WM_LBUTTONUP => {
                if let Some(index) = app.pressed_tab_close.take() {
                    // SAFETY: The handler is running on the thread that acquired capture.
                    let _ = unsafe { ReleaseCapture() };
                    if app.tab_close_at(point) == Some(index) {
                        app.close_at(index);
                    }
                    return LRESULT(0);
                }
            }
            WM_MBUTTONUP => {
                if let Some(index) = app.tab_at(point) {
                    app.close_at(index);
                }
                return LRESULT(0);
            }
            _ => app.pressed_tab_close.set(None),
        }
    }
    // SAFETY: The tab control's original procedure handles all other input.
    unsafe { DefSubclassProc(hwnd, message, w, l) }
}

impl Desktop {
    fn new(hwnd: HWND, config: PathBuf, local_path: PathBuf) -> Result<Self> {
        let config_path = config.join("preferences.json");
        let prefs = preferences::read(&config_path);
        let preference_error = prefs.as_ref().err().map(|e| format!("设置读取失败：{e:#}"));
        let preferences = prefs.unwrap_or_default();
        let (sender, receiver) = std::sync::mpsc::channel();
        let make = |id, class, title, style, edge| child(hwnd, id, class, title, style, edge);
        let button = |id, title| make(id, w!("BUTTON"), title, WS_TABSTOP.0, false);
        popup(
            hwnd,
            &[
                (
                    "文件(&F)",
                    &[
                        (NEW_BROWSE, "新建浏览标签页\tCtrl+T"),
                        (NEW_SEARCH, "新建搜索标签页\tCtrl+Shift+F"),
                        (CLOSE_TAB, "关闭标签页\tCtrl+W"),
                        (0, ""),
                        (PICK_VOLUME, "切换浏览分区…"),
                        (REGISTER, "登记分区…"),
                        (files::EXPORT, "导出搜索结果…\tCtrl+S"),
                        (EXIT, "退出\tCtrl+Q"),
                    ],
                ),
                ("编辑(&E)", files::EDIT_MENU),
                (
                    "查看(&V)",
                    &[
                        (REFRESH, "刷新当前页\tF5"),
                        (PREVIEW, "预览窗格\tAlt+P"),
                        (TREE, "目录树"),
                        (BREADCRUMBS, "面包屑"),
                        (files::DETAILS, "详情\tCtrl+Shift+6"),
                        (files::SMALL, "缩略图（小图标）\tCtrl+Shift+4"),
                        (files::MEDIUM, "缩略图（中图标）\tCtrl+Shift+3"),
                        (files::LARGE, "缩略图（大图标）\tCtrl+Shift+2"),
                        (files::COLUMNS, "详情列…"),
                        (files::SORT, "排序…"),
                        (BACK, "后退\tAlt+←"),
                        (FORWARD, "前进\tAlt+→"),
                        (UP, "上一级\tAlt+↑"),
                        (STATUS, "显示状态栏"),
                        (PIN, "窗口置顶"),
                    ],
                ),
                (
                    "搜索(&S)",
                    &[
                        (FOCUS_SEARCH, "聚焦搜索框\tCtrl+F"),
                        (FILTER, "筛选器…"),
                        (files::MATCH_CASE, "区分大小写\tCtrl+I"),
                        (files::MATCH_WORD, "完整单词\tCtrl+B"),
                        (files::MATCH_PATH, "匹配路径\tCtrl+U"),
                        (files::MATCH_REGEX, "使用正则表达式\tCtrl+R"),
                        (files::ONLINE, "仅在线"),
                        (files::HISTORY, "包含历史记录"),
                        (files::QUICK_FILTER, "应用筛选预设…"),
                        (files::PRESETS, "管理筛选预设…"),
                    ],
                ),
                (
                    "书签(&B)",
                    &[
                        (files::BOOKMARK, "添加当前搜索…\tCtrl+D"),
                        (files::BOOKMARKS, "管理书签…"),
                        (files::OPEN_BOOKMARK, "打开书签…"),
                    ],
                ),
                (
                    "工具(&T)",
                    &[
                        (SETTINGS, "设置…\tCtrl+P"),
                        (TASKS, "任务"),
                        (REFRESH_DEVICES, "刷新设备"),
                        (library::ANALYZE, "分析文件…"),
                        (library::REPORT, "文件操作结果…"),
                        (library::CANCEL_OPERATION, "取消文件操作"),
                        (files::CANCEL_EXPORT, "取消导出"),
                        (images::REBUILD_THUMBNAIL, "使用 FFmpeg 补建缩略图…"),
                    ],
                ),
                (
                    "帮助(&H)",
                    &[
                        (HELP, "搜索语法\tF1"),
                        (SHORTCUTS, "快捷键"),
                        (ABOUT, "关于与诊断"),
                    ],
                ),
            ],
        )?;
        let list = make(
            230,
            WC_LISTVIEWW,
            "工作区内容",
            WS_TABSTOP.0 | LVS_REPORT | LVS_OWNERDATA | LVS_SHOWSELALWAYS | LVS_SHAREIMAGELISTS,
            true,
        )?;
        send(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            0,
            (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_HEADERDRAGDROP) as isize,
        );
        let tabs = Tabs::new(&preferences);
        let status = make(
            231,
            w!("STATIC"),
            preference_error.as_deref().unwrap_or(""),
            SS_LEFTNOWORDWRAP.0,
            false,
        )?;
        let result = Self {
            hwnd,
            overview: make(
                OVERVIEW,
                w!("BUTTON"),
                "概览",
                WS_TABSTOP.0 | BS_RADIOBUTTON as u32 | BS_PUSHLIKE as u32,
                false,
            )?,
            tab_strip: make(232, WC_TABCONTROLW, "工作区标签页", WS_TABSTOP.0, false)?,
            add: button(NEW_BROWSE, "+")?,
            close_tab: button(CLOSE_TAB, "关闭标签")?,
            pressed_tab_close: Cell::new(None),
            list,
            query: make(
                QUERY,
                w!("EDIT"),
                "",
                WS_TABSTOP.0 | ES_AUTOHSCROLL as u32,
                true,
            )?,
            submit: button(SUBMIT, "搜索")?,
            scope: make(
                SCOPE,
                w!("COMBOBOX"),
                "分区范围",
                WS_TABSTOP.0 | CBS_DROPDOWNLIST as u32 | WS_VSCROLL.0,
                false,
            )?,
            filter: button(FILTER, "筛选器…")?,
            path: button(TAB_POPUP, "当前位置")?,
            back: button(BACK, "后退")?,
            forward: button(FORWARD, "前进")?,
            up: button(UP, "上一级")?,
            register: button(REGISTER, "登记分区…")?,
            group_assets: make(
                GROUP_ASSETS,
                w!("BUTTON"),
                "按硬盘分组",
                WS_TABSTOP.0 | BS_AUTOCHECKBOX as u32 | BS_PUSHLIKE as u32,
                false,
            )?,
            partition_actions: button(PARTITION_ACTIONS, "分区操作…")?,
            task_actions: button(TASK_ACTIONS, "任务操作…")?,
            task_filter_button: button(library::TASK_FILTER, "筛选任务…")?,
            task_cancel: button(library::CANCEL_TASK, "取消任务")?,
            task_retry: button(library::RETRY_TASK, "重试任务")?,
            task_resume: button(library::RESUME, "继续分区…")?,
            tree: make(
                233,
                WC_TREEVIEWW,
                "目录树",
                WS_TABSTOP.0 | TVS_HASLINES | TVS_HASBUTTONS | TVS_LINESATROOT,
                true,
            )?,
            preview: make(
                234,
                w!("EDIT"),
                "",
                WS_VSCROLL.0 | ES_MULTILINE as u32 | ES_READONLY as u32,
                true,
            )?,
            picture: make(
                235,
                w!("STATIC"),
                "文件缩略图",
                images::PICTURE_STYLE,
                false,
            )?,
            status,
            tabs: RefCell::new(tabs),
            snapshot: RefCell::new(Snapshot::default()),
            asset_rows: RefCell::new(vec![]),
            tasks: RefCell::new(vec![]),
            task_indexes: RefCell::new(vec![]),
            visible_tasks: RefCell::new(vec![]),
            task_filter: Cell::new(0),
            task_partition: RefCell::new(None),
            automatic: Cell::new(None),
            tasks_pending: Cell::new(false),
            preferences: RefCell::new(preferences),
            config_path,
            local_path,
            changing: Cell::new(false),
            composing: Cell::new(false),
            pending_input: Cell::new(false),
            pinned: Cell::new(false),
            assets_pending: Cell::new(false),
            task_refresh: Cell::new(0),
            tray: runtime::active(),
            tree_locations: RefCell::new(vec![]),
            tree_pending: Default::default(),
            tree_group: Cell::new(0),
            operation: Default::default(),
            exporting: Default::default(),
            exported_snapshot: Cell::new(None),
            columns: RefCell::new(files::Columns::load(&config)),
            preview_due: Cell::new(false),
            view_hint: Cell::new(None),
            images: Default::default(),
            visuals: Default::default(),
            sender,
            receiver,
            runtime: tokio::runtime::Builder::new_multi_thread()
                .worker_threads(3)
                .enable_all()
                .build()?,
        };
        send(
            result.query,
            EM_SETCUEBANNER,
            1,
            w!("搜索文件名或路径…").as_ptr() as isize,
        );
        // SAFETY: The subclass and its callbacks are confined to this UI thread.
        unsafe {
            SetWindowSubclass(result.query, Some(search_edit), 1, 0).ok()?;
            SetWindowSubclass(result.tab_strip, Some(tab_mouse), 1, 0).ok()?;
        }
        if result.tray {
            result.tray_icon(true)?;
        }
        Ok(result)
    }

    fn spawn<T: Send + 'static>(
        &self,
        future: impl Future<Output = T> + Send + 'static,
        complete: impl FnOnce(&Self, T) + Send + 'static,
    ) {
        let sender = self.sender.clone();
        let hwnd = self.hwnd.0 as usize;
        self.runtime.spawn(async move {
            let result = future.await;
            if sender
                .send(Box::new(move |app| complete(app, result)))
                .is_ok()
            {
                // SAFETY: Only a scalar wakeup is posted. The receiver owns completion data.
                let _ = unsafe {
                    PostMessageW(Some(HWND(hwnd as *mut _)), DRAIN, WPARAM(0), LPARAM(0))
                };
            }
        });
    }

    fn message(&self, message: u32, w: WPARAM, l: LPARAM) -> Option<isize> {
        match message {
            WM_SIZE => self.layout(),
            WM_DPICHANGED => {
                // SAFETY: WM_DPICHANGED supplies a recommended window rectangle.
                let rect = unsafe { &*(l.0 as *const RECT) };
                place(
                    self.hwnd,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                );
                self.layout();
            }
            WM_TIMER if w.0 == TICK => self.tick(),
            WM_TIMER if w.0 == DEBOUNCE => {
                // SAFETY: Trigger a single submission for the latest text.
                let _ = unsafe { KillTimer(Some(self.hwnd), DEBOUNCE) };
                if self.pending_input.replace(false) && !self.composing.get() {
                    self.files_submit(false);
                }
            }
            WM_COMMAND => self.command(w.0 & 0xffff, w.0 >> 16),
            WM_NOTIFY => return self.notification(l),
            WM_CONTEXTMENU if w.0 == self.overview.0 as usize => {
                self.context_menu(
                    &[
                        (NEW_BROWSE, "新建浏览标签页"),
                        (NEW_SEARCH, "新建搜索标签页"),
                    ],
                    None,
                );
            }
            WM_INITMENUPOPUP => self.update_menu(),
            DRAIN => {
                while let Ok(callback) = self.receiver.try_recv() {
                    callback(self);
                }
            }
            TRAY if [WM_LBUTTONUP, WM_LBUTTONDBLCLK].contains(&(l.0 as u32)) => self.restore(),
            TRAY if l.0 as u32 == WM_RBUTTONUP => {
                self.context_menu(&[(SHOW, "显示 StashFinder"), (EXIT, "退出")], None);
            }
            WM_CLOSE => self.close(false),
            WM_DESTROY => {
                if self.tray {
                    let _ = self.tray_icon(false);
                }
                // SAFETY: This is the root window of the current message loop.
                unsafe { PostQuitMessage(0) };
            }
            _ => return None,
        }
        Some(0)
    }

    fn command(&self, id: usize, event: usize) {
        match id {
            QUERY if event == EN_CHANGE as usize => self.queue_query(),
            SCOPE if event == CBN_SELCHANGE as usize => self.change_scope(),
            OVERVIEW => self.activate(0),
            NEW_BROWSE => self.new_browse(None),
            NEW_SEARCH => self.new_search(String::new(), None),
            CLOSE_TAB => {
                let active = self.tabs.borrow().active;
                self.close_at(active);
            }
            TASKS => self.open_tab(Page::Tasks),
            SETTINGS => self.settings(),
            HELP => self.help(),
            ABOUT => self.about(),
            SHORTCUTS => self.shortcuts(),
            REFRESH => self.manual_refresh(),
            REFRESH_DEVICES => self.refresh_assets(true),
            EXIT => self.close(true),
            STATUS => {
                let mut prefs = self.preferences.borrow().clone();
                prefs.show_status_bar = !prefs.show_status_bar;
                self.save_preferences(prefs);
                self.layout();
            }
            PIN => self.toggle_pin(),
            REGISTER => self.register_partition(),
            GROUP_ASSETS => self.group_assets(),
            PARTITION_ACTIONS => self.context_menu(library::ASSET_MENU, None),
            TASK_ACTIONS => self.context_menu(library::TASK_MENU, None),
            PICK_VOLUME => self.pick_volume(),
            BACK | FORWARD | UP | TAB_POPUP => self.navigate(id),
            FILTER => self.filter_dialog(),
            FOCUS_SEARCH => {
                let file = {
                    let tabs = self.tabs.borrow();
                    matches!(tabs.active().pane, Page::Tasks).then(|| {
                        tabs.items
                            .iter()
                            .rposition(|t| matches!(t.pane, Page::Browse(_) | Page::Search(_)))
                    })
                };
                if let Some(Some(i)) = file {
                    self.activate(i);
                } else if file.is_some() {
                    self.new_browse(None);
                }
                focus(self.query);
                send(self.query, EM_SETSEL, 0, -1);
            }
            SUBMIT => {
                self.pending_input.set(false);
                self.files_submit(true);
            }
            TREE | BREADCRUMBS | PREVIEW => self.change_visibility(id),
            SHOW => self.restore(),
            images::REBUILD_THUMBNAIL => self.rebuild_thumbnail(),
            _ => {
                if !self.library_command(id) {
                    self.file_command(id);
                }
            }
        }
    }

    fn key(&self, msg: &MSG) -> bool {
        if ![WM_KEYDOWN, WM_SYSKEYDOWN].contains(&msg.message) {
            return false;
        }
        // SAFETY: Reading current thread key and focus state has no pointer arguments.
        let (ctrl, shift, alt, focus_is_edit) = unsafe {
            (
                GetKeyState(VK_CONTROL.0 as i32) < 0,
                GetKeyState(VK_SHIFT.0 as i32) < 0,
                GetKeyState(VK_MENU.0 as i32) < 0,
                GetFocus() == self.query,
            )
        };
        let key = msg.wParam.0 as u16;
        if focus_is_edit && self.composing.get() {
            return false;
        }
        let command = match (ctrl, shift, alt, key) {
            (true, _, false, k) if k == VK_TAB.0 => {
                let (active, count) = {
                    let t = self.tabs.borrow();
                    (t.active, t.items.len())
                };
                self.activate((active + if shift { count - 1 } else { 1 }) % count);
                return true;
            }
            (true, false, false, 84) => NEW_BROWSE,
            (true, true, false, 70) => NEW_SEARCH,
            (true, false, false, 70) => FOCUS_SEARCH,
            (true, false, false, 87) => CLOSE_TAB,
            (true, false, false, 80) => SETTINGS,
            (true, false, false, 81) => EXIT,
            (true, false, false, 83) => files::EXPORT,
            (true, false, false, 68) => files::BOOKMARK,
            (true, false, false, 73) => files::MATCH_CASE,
            (true, false, false, 66) => files::MATCH_WORD,
            (true, false, false, 85) => files::MATCH_PATH,
            (true, false, false, 82) => files::MATCH_REGEX,
            (true, true, false, 54) => files::DETAILS,
            (true, true, false, 52) => files::SMALL,
            (true, true, false, 51) => files::MEDIUM,
            (true, true, false, 50) => files::LARGE,
            (true, false, false, 65) if !focus_is_edit => files::SELECT_ALL,
            (true, false, false, 67) if !focus_is_edit => files::COPY,
            (true, true, false, 67) => files::COPY_PATH,
            (true, false, false, 86) if !focus_is_edit => files::PASTE,
            (true, false, false, 88) if !focus_is_edit => files::CUT,
            (false, false, false, k) if k == VK_F5.0 => REFRESH,
            (false, false, false, k) if k == VK_F1.0 => HELP,
            (false, false, true, 80) => PREVIEW,
            (false, false, true, k) if k == VK_LEFT.0 => BACK,
            (false, false, true, k) if k == VK_RIGHT.0 => FORWARD,
            (false, false, true, k) if k == VK_UP.0 => UP,
            (false, false, true, k) if !focus_is_edit && k == VK_RETURN.0 => files::PROPERTIES,
            (false, false, false, k) if focus_is_edit && k == VK_RETURN.0 => SUBMIT,
            (false, false, false, k) if msg.hwnd == self.list && k == VK_RETURN.0 => files::OPEN,
            (false, shift, false, k) if !focus_is_edit && k == VK_DELETE.0 => {
                if shift {
                    files::DELETE
                } else {
                    files::RECYCLE
                }
            }
            (false, false, false, k) if !focus_is_edit && k == VK_F2.0 => files::RENAME,
            (false, true, false, k) if msg.hwnd == self.list && k == VK_F10.0 => {
                files::CONTEXT_MENU
            }
            (true, false, false, k) if msg.hwnd == self.list && k == VK_ADD.0 => files::FIT_COLUMNS,
            _ => return false,
        };
        self.command(command, 0);
        true
    }

    fn notification(&self, l: LPARAM) -> Option<isize> {
        // SAFETY: WM_NOTIFY borrows the notification for this call. The source and code determine its type.
        let hdr = unsafe { &*(l.0 as *const NMHDR) };
        if hdr.hwndFrom == self.tab_strip {
            match hdr.code {
                TCN_SELCHANGE => {
                    let index = send(self.tab_strip, TCM_GETCURSEL, 0, 0);
                    if index >= 0 {
                        self.activate(index as usize + 1);
                    }
                }
                NM_RCLICK => self.tab_context(),
                NM_CLICK => {}
                _ => return None,
            }
            return Some(0);
        }
        self.files_notify(hdr, l)
    }

    fn tick(&self) {
        self.files_tick();
        self.images_tick();
        if self.preview_due.replace(false) {
            self.update_preview();
            self.update_menu();
        }
        let poll = self.task_refresh.get();
        if poll >= 19 {
            self.task_refresh.set(0);
            self.refresh_assets(false);
        } else {
            self.task_refresh.set(poll + 1);
            if poll % 6 == 5 {
                self.refresh_tasks();
            }
        }
    }

    fn queue_query(&self) {
        if !self.changing.get()
            && !self.composing.get()
            && matches!(self.tabs.borrow().active().pane, Page::Search(_))
        {
            self.pending_input.set(true);
            // SAFETY: This timer debounces input on the workspace's thread.
            unsafe { SetTimer(Some(self.hwnd), DEBOUNCE, 220, None) };
        }
    }

    fn layout(&self) {
        let mut client = RECT::default();
        // SAFETY: Our root window and writable client rect belong to the current thread.
        if unsafe { GetClientRect(self.hwnd, &mut client) }.is_err() {
            return;
        }
        let (browse, search, home, tasks, tree, crumbs, preview) = {
            let tabs = self.tabs.borrow();
            let tab = tabs.active();
            let browse = match &tab.pane {
                Page::Browse(b) => Some(b),
                _ => None,
            };
            let search = matches!(tab.pane, Page::Search(_));
            let home = matches!(tab.pane, Page::Overview);
            (
                browse.is_some(),
                search,
                home,
                matches!(tab.pane, Page::Tasks),
                browse.is_some_and(|b| b.tree),
                browse.is_some_and(|b| b.breadcrumbs),
                (browse.is_some() || search) && tab.preview || home,
            )
        };
        let toolbar = home || search || browse;
        // SAFETY: This is the workspace's own live HWND.
        let dpi = (unsafe { GetDpiForWindow(self.hwnd) } as i32).max(96);
        let px = |n| n * dpi / 96;
        let width = client.right;
        let gap = px(8);
        let control = px(28);
        let mut y = gap;
        place(self.overview, gap, y, px(62), control);
        place(self.tab_strip, px(76), y, width - px(196), control);
        place(self.add, width - px(114), y, px(31), control);
        place(self.close_tab, width - px(77), y, px(69), control);
        y += control + gap;
        for (control, visible) in [
            (self.query, toolbar),
            (self.submit, toolbar),
            (self.scope, search),
            (self.filter, search || browse),
            (self.register, home),
            (self.group_assets, home),
            (self.partition_actions, home),
            (self.task_actions, tasks),
            (self.task_filter_button, tasks),
            (self.task_cancel, tasks),
            (self.task_retry, tasks),
            (self.task_resume, tasks),
            (self.back, browse),
            (self.forward, browse),
            (self.up, browse),
            (self.path, browse && crumbs),
            (self.tree, tree),
            (self.preview, preview),
        ] {
            show(control, visible);
        }
        if browse {
            for (i, button) in [self.back, self.forward, self.up].into_iter().enumerate() {
                place(button, gap + i as i32 * px(69), y, px(64), control);
            }
            place(self.path, px(219), y, width - px(227), control);
            y += control + gap;
        }
        if toolbar {
            let suffix = if home {
                px(422)
            } else if search {
                px(308)
            } else {
                px(203)
            };
            place(self.query, gap, y, width - suffix - gap, control);
            place(self.submit, width - suffix + gap, y, px(68), control);
            place(self.filter, width - px(112), y, px(103), control);
            place(self.register, width - px(112), y, px(103), control);
            place(self.group_assets, width - px(340), y, px(105), control);
            place(self.partition_actions, width - px(226), y, px(105), control);
            place(self.scope, width - px(226), y, px(109), px(200));
            y += control + gap;
        }
        if tasks {
            for (i, button) in [
                self.task_filter_button,
                self.task_actions,
                self.task_cancel,
                self.task_retry,
                self.task_resume,
            ]
            .into_iter()
            .enumerate()
            {
                place(button, gap + px(115) * i as i32, y, px(106), control);
            }
            y += control + gap;
        }
        let status = self.preferences.borrow().show_status_bar;
        show(self.status, status);
        let bottom = client.bottom - if status { px(30) } else { gap };
        let left = if tree { px(210) } else { gap };
        let right = if preview { px(308) } else { gap };
        place(self.tree, gap, y, left - 2 * gap, bottom - y);
        let with_picture = preview && !home && self.has_picture();
        let image_height = if with_picture {
            px(257).min((bottom - y).max(0) / 2)
        } else {
            0
        };
        show(self.picture, with_picture);
        place(
            self.picture,
            width - right + gap,
            y,
            right - 2 * gap,
            image_height,
        );
        place(
            self.preview,
            width - right + gap,
            y + image_height,
            right - 2 * gap,
            bottom - y - image_height,
        );
        place(self.list, left, y, width - left - right, bottom - y);
        place(self.status, gap, bottom + gap / 2, width - 2 * gap, px(24));
    }

    fn status(&self, text: &str) {
        self.tabs.borrow_mut().active_mut().status = text.into();
        set_text(self.status, text);
    }

    fn close(&self, force: bool) {
        if self.guard_close() {
            return;
        }
        if self.tray && !force {
            show(self.hwnd, false);
        } else {
            // SAFETY: The root window is owned by this thread.
            let _ = unsafe { DestroyWindow(self.hwnd) };
        }
    }

    fn tray_icon(&self, add: bool) -> Result<()> {
        use windows::Win32::UI::Shell::*;
        // SAFETY: The notification icon has a single owner and a same-process callback.
        unsafe {
            let mut data = NOTIFYICONDATAW {
                cbSize: size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: self.hwnd,
                uID: 1,
                uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
                uCallbackMessage: TRAY,
                hIcon: LoadIconW(Some(instance()?), PCWSTR(32512_usize as *const _))
                    .or_else(|_| LoadIconW(None, IDI_APPLICATION))?,
                ..Default::default()
            };
            for (n, c) in data
                .szTip
                .iter_mut()
                .zip("StashFinder 便携版".encode_utf16())
            {
                *n = c;
            }
            Shell_NotifyIconW(if add { NIM_ADD } else { NIM_DELETE }, &data).ok()?;
        }
        Ok(())
    }

    fn restore(&self) {
        // SAFETY: Only our application's window is shown and brought forward.
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(self.hwnd);
        }
    }

    fn toggle_pin(&self) {
        let pin = !self.pinned.get();
        // SAFETY: Only changes the z order of our own window.
        let result = unsafe {
            SetWindowPos(
                self.hwnd,
                Some(if pin { HWND_TOPMOST } else { HWND_NOTOPMOST }),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        };
        match result {
            Ok(()) => self.pinned.set(pin),
            Err(e) => self.status(&format!("无法切换置顶：{e}")),
        }
    }

    fn context_menu(&self, items: &[(usize, &str)], point: Option<POINT>) {
        // SAFETY: The temporary menu and all labels live through the Win32 modal popup.
        unsafe {
            let Ok(menu) = CreatePopupMenu() else {
                return;
            };
            for &(id, name) in items {
                let _ = AppendMenuW(
                    menu,
                    if id == 0 { MF_SEPARATOR } else { MF_STRING },
                    id,
                    &HSTRING::from(name),
                );
            }
            let mut point = point.unwrap_or_default();
            if point == POINT::default() {
                let _ = GetCursorPos(&mut point);
            }
            let _ = SetForegroundWindow(self.hwnd);
            let result = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                None,
                self.hwnd,
                None,
            )
            .0;
            let _ = DestroyMenu(menu);
            if result > 0 {
                self.command(result as usize, 0);
            }
        }
    }
}
