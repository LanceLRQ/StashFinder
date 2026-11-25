use crate::{
    Endpoint,
    actions::{self, FileAction, Prepared},
    search::{self, Page, View},
};
use anyhow::{Result, ensure};
use stashfinder_core::{
    SEARCH_SNAPSHOT_EXPIRED, file_operations::MAX_SELECTED, index::SearchHit, search::SortField,
};
use std::{
    cell::{Cell, RefCell},
    mem::size_of,
    rc::Rc,
    sync::mpsc,
};
use tokio::{
    runtime::{Builder, Handle},
    sync::watch,
    task::JoinHandle,
};
use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{COLOR_WINDOW, CreateFontIndirectW, DeleteObject, GetSysColorBrush, HFONT},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::*,
            HiDpi::*,
            Input::KeyboardAndMouse::{
                GetKeyState, SetFocus, VK_A, VK_C, VK_CONTROL, VK_RETURN, VK_SHIFT, VK_TAB,
            },
            Shell::{DefSubclassProc, SetWindowSubclass},
            WindowsAndMessaging::*,
        },
    },
    core::{HSTRING, PWSTR, w},
};

const RESULTS: u32 = WM_APP + 1;
const SCROLL: u32 = WM_APP + 2;
const DEBOUNCE: usize = 1;
const OPEN: usize = 101;
const COPY_PATH: usize = 102;
const COLUMNS: [(&str, i32, Option<SortField>); 5] = [
    ("名称", 235, Some(SortField::Name)),
    ("卷内路径", 375, Some(SortField::Path)),
    ("分区", 120, Some(SortField::Partition)),
    ("大小（字节）", 110, Some(SortField::Size)),
    ("查询时状态", 105, None),
];

enum Update {
    Page(View, Result<Page>),
    Files(u64, FileAction, Result<Prepared>),
}

struct Window {
    hwnd: HWND,
    edit: HWND,
    list: HWND,
    status: HWND,
    font: Cell<HFONT>,
    dpi: Cell<u32>,
    endpoint: Endpoint,
    runtime: Handle,
    requests: watch::Sender<Option<View>>,
    send: mpsc::Sender<Update>,
    updates: mpsc::Receiver<Update>,
    generation: Cell<u64>,
    sort: Cell<SortField>,
    descending: Cell<bool>,
    composing: Cell<bool>,
    scroll_posted: Cell<bool>,
    page: RefCell<Option<(usize, Page)>>,
    file_task: RefCell<Option<JoinHandle<()>>>,
}

thread_local! { static WINDOW: RefCell<Option<Rc<Window>>> = const { RefCell::new(None) }; }

pub(super) fn run(endpoint: Endpoint) -> Result<()> {
    let runtime = Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    // SAFETY: The class, its windows and the message loop all belong to this thread. Win32 copies
    // creation parameters synchronously. The callbacks access only thread-local state.
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES,
        })
        .ok()?;
        let instance: HINSTANCE = GetModuleHandleW(None)?.into();
        let class = w!("StashFinderNativeSearch");
        let atom = RegisterClassW(&WNDCLASSW {
            hInstance: instance,
            lpszClassName: class,
            lpfnWndProc: Some(window_proc),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: GetSysColorBrush(COLOR_WINDOW),
            ..Default::default()
        });
        ensure!(atom != 0, "无法注册原生搜索窗口");
        let dpi = GetDpiForSystem() as i32;
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            w!("StashFinder · 快速搜索"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1000 * dpi / 96,
            650 * dpi / 96,
            None,
            None,
            Some(instance),
            None,
        )?;
        let menu = CreateMenu()?;
        let file = CreatePopupMenu()?;
        AppendMenuW(file, MF_STRING, OPEN, w!("打开(&O)\tEnter"))?;
        AppendMenuW(file, MF_STRING, COPY_PATH, w!("复制路径(&P)\tCtrl+Shift+C"))?;
        AppendMenuW(menu, MF_POPUP, file.0 as usize, w!("文件(&F)"))?;
        SetMenu(hwnd, Some(menu))?;
        let child = |class, text, style, edge, id| {
            CreateWindowExW(
                edge,
                class,
                text,
                WS_CHILD | WS_VISIBLE | style,
                0,
                0,
                0,
                0,
                Some(hwnd),
                Some(HMENU(id as *mut _)),
                Some(instance),
                None,
            )
        };
        let edit = child(
            w!("EDIT"),
            w!(""),
            WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            WS_EX_CLIENTEDGE,
            1,
        )?;
        let list = child(
            WC_LISTVIEWW,
            w!("搜索结果"),
            WS_TABSTOP | WINDOW_STYLE(LVS_REPORT | LVS_OWNERDATA | LVS_SHOWSELALWAYS),
            WS_EX_CLIENTEDGE,
            2,
        )?;
        let status = child(
            w!("STATIC"),
            w!("输入关键词搜索"),
            WINDOW_STYLE(0),
            WINDOW_EX_STYLE(0),
            3,
        )?;
        SetWindowSubclass(edit, Some(edit_proc), 1, 0).ok()?;
        SendMessageW(
            edit,
            EM_SETCUEBANNER,
            Some(WPARAM(1)),
            Some(LPARAM(w!("搜索文件和文件夹").as_ptr() as isize)),
        );
        SendMessageW(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            None,
            Some(LPARAM(
                (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize,
            )),
        );
        SendMessageW(list, LVM_SETUNICODEFORMAT, Some(WPARAM(1)), None);

        let (requests, rx) = watch::channel(None);
        let (send, updates) = mpsc::channel();
        let worker_send = send.clone();
        let target = hwnd.0 as usize;
        let search_endpoint = endpoint.clone();
        let worker = runtime.spawn(async move {
            search::serve(&search_endpoint, rx, |view, result| {
                notify(&worker_send, target, Update::Page(view, result));
            })
            .await;
        });
        let app = Rc::new(Window {
            hwnd,
            edit,
            list,
            status,
            endpoint,
            requests,
            send,
            updates,
            runtime: runtime.handle().clone(),
            generation: Cell::new(0),
            sort: Cell::new(SortField::Name),
            descending: Cell::new(false),
            composing: Cell::new(false),
            scroll_posted: Cell::new(false),
            font: Cell::new(HFONT::default()),
            dpi: Cell::new(GetDpiForWindow(hwnd)),
            page: RefCell::new(None),
            file_task: RefCell::new(None),
        });
        WINDOW.with(|slot| slot.replace(Some(app.clone())));
        app.scale(true)?;
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = SetFocus(Some(edit));
        let mut message = MSG::default();
        let result = loop {
            match GetMessageW(&mut message, None, 0, 0).0 {
                -1 => break Err(windows::core::Error::from_thread().into()),
                0 => break Ok(()),
                _ => {}
            }
            if message.message == WM_KEYDOWN && !app.composing.get() {
                let key = message.wParam.0 as u16;
                if key == VK_TAB.0 && (message.hwnd == edit || message.hwnd == list) {
                    let _ = SetFocus(Some(if message.hwnd == edit { list } else { edit }));
                    continue;
                }
                if message.hwnd == edit && key == VK_RETURN.0 {
                    app.submit();
                    continue;
                }
                if message.hwnd == list && key == VK_RETURN.0 {
                    app.start_action(FileAction::Open);
                    continue;
                }
                if message.hwnd == list
                    && key == VK_C.0
                    && GetKeyState(VK_CONTROL.0 as i32) < 0
                    && GetKeyState(VK_SHIFT.0 as i32) < 0
                {
                    app.start_action(FileAction::CopyPath);
                    continue;
                }
                if message.hwnd == list && key == VK_A.0 && GetKeyState(VK_CONTROL.0 as i32) < 0 {
                    let item = LVITEMW {
                        stateMask: LVIS_SELECTED,
                        state: LVIS_SELECTED,
                        ..Default::default()
                    };
                    SendMessageW(
                        list,
                        LVM_SETITEMSTATE,
                        Some(WPARAM(usize::MAX)),
                        Some(LPARAM(&item as *const _ as isize)),
                    );
                    continue;
                }
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        };
        WINDOW.with(|slot| slot.take());
        let file_task = app.file_task.take();
        drop(app);
        runtime.block_on(worker)?;
        if let Some(task) = file_task {
            runtime.block_on(task)?;
        }
        result
    }
}

fn notify(sender: &mpsc::Sender<Update>, target: usize, update: Update) {
    if sender.send(update).is_ok() {
        // SAFETY: No pointer crosses the window boundary. The channel owns the payload.
        let _ =
            unsafe { PostMessageW(Some(HWND(target as *mut _)), RESULTS, WPARAM(0), LPARAM(0)) };
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // Release the thread-local borrow before calling Win32, which can reenter the window procedure.
    let app = WINDOW.with(|slot| slot.borrow().clone());
    if let Some(app) = app
        && app.hwnd == hwnd
        && app.message(msg, wp, lp)
    {
        return LRESULT(0);
    }
    // SAFETY: Forward unhandled messages and their original parameters.
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

unsafe extern "system" fn edit_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    let app = WINDOW.with(|slot| slot.borrow().clone());
    if msg == WM_IME_STARTCOMPOSITION
        && let Some(app) = &app
    {
        app.composing.set(true);
        // SAFETY: Stop any pending input timer on the parent window.
        let _ = unsafe { KillTimer(Some(app.hwnd), DEBOUNCE) };
    }
    // SAFETY: The edit must process the IME commit before its final text is read.
    let result = unsafe { DefSubclassProc(hwnd, msg, wp, lp) };
    if msg == WM_IME_ENDCOMPOSITION
        && let Some(app) = app
    {
        app.composing.set(false);
        app.edited();
    }
    result
}

impl Window {
    fn message(&self, msg: u32, wp: WPARAM, lp: LPARAM) -> bool {
        // SAFETY: Notification pointers are borrowed only during the matching synchronous Win32
        // callback. Check the source control and notification code before reading its payload.
        unsafe {
            match msg {
                WM_SIZE => self.layout(),
                WM_DPICHANGED => {
                    self.dpi.set((wp.0 >> 16) as u32);
                    let r = &*(lp.0 as *const RECT);
                    let _ = SetWindowPos(
                        self.hwnd,
                        None,
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    if let Err(error) = self.scale(false) {
                        self.status(&format!("{error:#}"));
                    }
                }
                WM_COMMAND
                    if HWND(lp.0 as *mut _) == self.edit && (wp.0 >> 16) as u32 == EN_CHANGE =>
                {
                    self.edited()
                }
                WM_COMMAND if lp.0 == 0 && wp.0 == OPEN => self.start_action(FileAction::Open),
                WM_COMMAND if lp.0 == 0 && wp.0 == COPY_PATH => {
                    self.start_action(FileAction::CopyPath)
                }
                WM_INITMENUPOPUP if (lp.0 >> 16) == 0 => {
                    let menu = HMENU(wp.0 as *mut _);
                    let count =
                        SendMessageW(self.list, LVM_GETSELECTEDCOUNT, None, None).0 as usize;
                    let idle = self.file_task.borrow().is_none();
                    for (id, enabled) in [
                        (OPEN, count == 1),
                        (COPY_PATH, (1..=MAX_SELECTED).contains(&count)),
                    ] {
                        let _ = EnableMenuItem(
                            menu,
                            id as u32,
                            if idle && enabled {
                                MF_ENABLED
                            } else {
                                MF_GRAYED
                            },
                        );
                    }
                }
                WM_TIMER if wp.0 == DEBOUNCE => self.submit(),
                WM_NOTIFY => {
                    let hdr = &*(lp.0 as *const NMHDR);
                    if hdr.hwndFrom != self.list {
                        return false;
                    }
                    match hdr.code {
                        LVN_GETDISPINFOW => {
                            let item = &mut (*(lp.0 as *mut NMLVDISPINFOW)).item;
                            if item.mask.contains(LVIF_TEXT)
                                && !item.pszText.is_null()
                                && item.cchTextMax > 0
                            {
                                let page = self.page.borrow();
                                let row = page.as_ref().and_then(|(start, page)| {
                                    page.rows.get((item.iItem as usize).checked_sub(*start)?)
                                });
                                let text = row.map_or("…", |row| cell_text(row, item.iSubItem));
                                let buf = std::slice::from_raw_parts_mut(
                                    item.pszText.0,
                                    item.cchTextMax as usize,
                                );
                                let mut n = 0;
                                for c in text.encode_utf16().take(buf.len() - 1) {
                                    buf[n] = c;
                                    n += 1;
                                }
                                buf[n] = 0;
                                if row.is_none() {
                                    self.schedule_scroll();
                                }
                            }
                        }
                        LVN_ODCACHEHINT => self.schedule_scroll(),
                        LVN_COLUMNCLICK => {
                            let col = (*(lp.0 as *const NMLISTVIEW)).iSubItem as usize;
                            if let Some((_, _, Some(sort))) = COLUMNS.get(col) {
                                self.descending
                                    .set(self.sort.get() == *sort && !self.descending.get());
                                self.sort.set(*sort);
                                self.columns(false, false);
                                self.submit();
                            }
                        }
                        NM_DBLCLK => {
                            if (*(lp.0 as *const NMITEMACTIVATE)).iItem >= 0 {
                                self.start_action(FileAction::Open);
                            }
                        }
                        LVN_BEGINDRAG => self.start_action(FileAction::Drag),
                        _ => return false,
                    }
                }
                RESULTS => self.receive(),
                SCROLL => {
                    self.scroll_posted.set(false);
                    self.viewport();
                }
                WM_DESTROY => PostQuitMessage(0),
                _ => return false,
            }
        }
        true
    }

    fn status(&self, text: &str) {
        // SAFETY: SetWindowTextW copies the temporary UTF-16 string synchronously.
        let _ = unsafe { SetWindowTextW(self.status, &HSTRING::from(text)) };
    }

    fn scale(&self, initial: bool) -> Result<()> {
        // SAFETY: Windows fills the sized metrics structure. All child controls switch fonts
        // before the previous owned font is released.
        unsafe {
            let dpi = self.dpi.get();
            let mut metrics = NONCLIENTMETRICSW {
                cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
                ..Default::default()
            };
            SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS.0,
                metrics.cbSize,
                Some((&mut metrics as *mut NONCLIENTMETRICSW).cast()),
                0,
                dpi,
            )?;
            let font = CreateFontIndirectW(&metrics.lfMessageFont);
            ensure!(!font.is_invalid(), "无法创建系统字体");
            for child in [self.edit, self.list, self.status] {
                SendMessageW(
                    child,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                );
            }
            let old = self.font.replace(font);
            if !old.is_invalid() {
                let _ = DeleteObject(old.into());
            }
        }
        self.columns(initial, true);
        self.layout();
        Ok(())
    }

    fn columns(&self, initial: bool, resize: bool) {
        // SAFETY: The list view synchronously copies each column descriptor and title.
        unsafe {
            for (i, (name, width, sort)) in COLUMNS.iter().enumerate() {
                let title = if *sort == Some(self.sort.get()) {
                    format!("{name} {}", if self.descending.get() { "▼" } else { "▲" })
                } else {
                    name.to_string()
                };
                let mut title: Vec<_> = title.encode_utf16().chain([0]).collect();
                let col = LVCOLUMNW {
                    mask: if resize {
                        LVCF_TEXT | LVCF_WIDTH
                    } else {
                        LVCF_TEXT
                    },
                    cx: width * self.dpi.get() as i32 / 96,
                    pszText: PWSTR(title.as_mut_ptr()),
                    ..Default::default()
                };
                SendMessageW(
                    self.list,
                    if initial {
                        LVM_INSERTCOLUMNW
                    } else {
                        LVM_SETCOLUMNW
                    },
                    Some(WPARAM(i)),
                    Some(LPARAM(&col as *const _ as isize)),
                );
            }
        }
    }

    fn layout(&self) {
        // SAFETY: All coordinates are in this window's client area and all children belong to it.
        unsafe {
            let mut r = RECT::default();
            if GetClientRect(self.hwnd, &mut r).is_err() {
                return;
            }
            let px = |value| value * self.dpi.get() as i32 / 96;
            let gap = px(10);
            let width = (r.right - 2 * gap).max(0);
            let top = 2 * gap + px(29);
            let bottom = (r.bottom - px(27)).max(top);
            let _ = MoveWindow(self.edit, gap, gap, width, px(29), true);
            let _ = MoveWindow(self.list, gap, top, width, bottom - top, true);
            let _ = MoveWindow(self.status, gap, bottom + px(3), width, px(22), true);
        }
        self.schedule_scroll();
    }

    fn edited(&self) {
        if self.composing.get() {
            return;
        }
        self.generation.set(self.generation.get() + 1);
        self.clear();
        self.status("正在搜索…");
        // SAFETY: Setting the same timer ID restarts the input debounce.
        unsafe {
            SetTimer(Some(self.hwnd), DEBOUNCE, 75, None);
        }
    }

    fn clear(&self) {
        self.page.take();
        // SAFETY: Resetting the owner-data count discards the previous snapshot's selection.
        unsafe {
            SendMessageW(self.list, LVM_SETITEMCOUNT, Some(WPARAM(0)), None);
        }
    }

    fn submit(&self) {
        // SAFETY: The text buffer is owned and sized for the synchronous edit query.
        unsafe {
            let _ = KillTimer(Some(self.hwnd), DEBOUNCE);
            if self.composing.get() {
                return;
            }
            let mut buf = vec![0; GetWindowTextLengthW(self.edit) as usize + 1];
            let n = GetWindowTextW(self.edit, &mut buf) as usize;
            self.generation.set(self.generation.get() + 1);
            self.clear();
            self.status("正在搜索…");
            self.requests.send_replace(Some(View {
                generation: self.generation.get(),
                text: String::from_utf16_lossy(&buf[..n]),
                sort: self.sort.get(),
                descending: self.descending.get(),
                range: 0..32,
            }));
        }
    }

    fn schedule_scroll(&self) {
        if !self.scroll_posted.replace(true) {
            // SAFETY: Defer reading the viewport until after the list finishes laying itself out.
            let _ = unsafe { PostMessageW(Some(self.hwnd), SCROLL, WPARAM(0), LPARAM(0)) };
        }
    }

    fn viewport(&self) {
        let page = self.page.borrow();
        let Some((start, page)) = page.as_ref() else {
            return;
        };
        // SAFETY: Scalar messages query the current native viewport.
        let (top, visible) = unsafe {
            (
                SendMessageW(self.list, LVM_GETTOPINDEX, None, None)
                    .0
                    .max(0) as usize,
                SendMessageW(self.list, LVM_GETCOUNTPERPAGE, None, None)
                    .0
                    .max(1) as usize
                    + 2,
            )
        };
        let visible_range = top..(top + visible).min(page.total);
        if visible_range.is_empty()
            || (*start <= top && *start + page.rows.len() >= visible_range.end)
        {
            return;
        }
        let range = top.saturating_sub(visible)..(visible_range.end + visible).min(page.total);
        self.requests.send_if_modified(|request| {
            let Some(view) = request else {
                return false;
            };
            if view.generation != self.generation.get() || view.range == range {
                return false;
            }
            view.range = range;
            true
        });
    }

    fn receive(&self) {
        while let Ok(update) = self.updates.try_recv() {
            match update {
                Update::Page(view, result) => {
                    if view.generation != self.generation.get()
                        || self.requests.borrow().as_ref() != Some(&view)
                    {
                        continue;
                    }
                    match result {
                        Ok(page) => {
                            let total = page.total;
                            let count = page.rows.len();
                            let mut label = format!("{total} 个结果");
                            if !page.unavailable.is_empty() {
                                label.push_str(&format!(
                                    " · 部分分区不可用：{}",
                                    page.unavailable.join("、")
                                ));
                            }
                            let first = self.page.replace(Some((view.range.start, page))).is_none();
                            // SAFETY: These scalar messages update only the count or fetched row range.
                            unsafe {
                                if first
                                    && SendMessageW(
                                        self.list,
                                        LVM_SETITEMCOUNT,
                                        Some(WPARAM(total)),
                                        None,
                                    )
                                    .0 == 0
                                {
                                    self.clear();
                                    self.status("结果超出原生列表容量，请缩小搜索范围");
                                    continue;
                                }
                                if count > 0 {
                                    SendMessageW(
                                        self.list,
                                        LVM_REDRAWITEMS,
                                        Some(WPARAM(view.range.start)),
                                        Some(LPARAM((view.range.start + count - 1) as isize)),
                                    );
                                }
                            }
                            self.status(&label);
                            self.schedule_scroll();
                        }
                        Err(e) => {
                            self.clear();
                            self.status(&format!("无法完成搜索：{e:#}（Enter 重试）"));
                        }
                    }
                }
                Update::Files(generation, action, result) => {
                    self.file_task.take();
                    if generation != self.generation.get() {
                        continue;
                    }
                    let outcome =
                        result.and_then(|prepared| prepared.perform(self.hwnd.0 as usize));
                    match outcome {
                        Ok(label) => self.status(label),
                        Err(e) => {
                            if e.to_string() == SEARCH_SNAPSHOT_EXPIRED {
                                self.generation.set(self.generation.get() + 1);
                                self.requests.send_replace(None);
                                self.clear();
                            }
                            let verb = match action {
                                FileAction::Open => "打开",
                                FileAction::CopyPath => "复制路径",
                                FileAction::Drag => "拖出",
                            };
                            self.status(&format!("无法{verb}：{e:#}"));
                        }
                    }
                }
            }
        }
    }

    fn start_action(&self, action: FileAction) {
        let Some(id) = self.page.borrow().as_ref().map(|(_, page)| page.id) else {
            return;
        };
        if self.file_task.borrow().is_some() {
            return;
        }
        // SAFETY: Owner-data selection indices refer to the whole list, including unloaded pages.
        let positions = unsafe {
            let count = SendMessageW(self.list, LVM_GETSELECTEDCOUNT, None, None).0 as usize;
            if count == 0 {
                return;
            }
            if action == FileAction::Open && count != 1 {
                self.status("打开文件时请选择一项");
                return;
            }
            if count > MAX_SELECTED {
                self.status("单次最多操作 10,000 项");
                return;
            }
            let mut positions = Vec::with_capacity(count);
            let mut last = -1isize;
            for _ in 0..count {
                last = SendMessageW(
                    self.list,
                    LVM_GETNEXTITEM,
                    Some(WPARAM(last as usize)),
                    Some(LPARAM(LVNI_SELECTED as isize)),
                )
                .0;
                if last < 0 {
                    self.status("选择已变化，请重新选择");
                    return;
                }
                positions.push(last as usize);
            }
            positions
        };
        self.status(if action == FileAction::CopyPath {
            "正在读取所选路径…"
        } else {
            "正在核验所选文件…"
        });
        let endpoint = self.endpoint.clone();
        let sender = self.send.clone();
        let target = self.hwnd.0 as usize;
        let generation = self.generation.get();
        self.file_task.replace(Some(self.runtime.spawn(async move {
            let result = actions::prepare(&endpoint, id, positions, action).await;
            notify(&sender, target, Update::Files(generation, action, result));
        })));
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        if !self.font.get().is_invalid() {
            // SAFETY: The owned font is no longer in use after the child windows are destroyed.
            let _ = unsafe { DeleteObject(self.font.get().into()) };
        }
    }
}

fn cell_text(row: &SearchHit, column: i32) -> &str {
    match column {
        0 => &row.file.name,
        1 => &row.file.path,
        2 => &row.partition_name,
        3 if row.file.kind == "folder" => "—",
        3 => row.file.size.as_deref().unwrap_or("未知"),
        4 => match row.connection.as_str() {
            "online" => "在线",
            "offline" => "离线",
            "changed" | "historical" => "历史记录",
            "uncertain" => "身份待确认",
            "inaccessible" => "无法访问",
            _ => "状态未确认",
        },
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        thread,
        time::{Duration, Instant},
    };

    #[test]
    #[ignore = "opens a native window on the interactive Windows desktop"]
    fn dpi_change_resizes_the_real_edit_and_list_columns() {
        let worker = thread::spawn(|| {
            run(Endpoint::Direct(
                stashfinder_windows::ipc::DEFAULT_PIPE.into(),
            ))
            .unwrap()
        });
        let deadline = Instant::now() + Duration::from_secs(6);
        // SAFETY: The window is in this test process. Its RECT remains valid across each
        // synchronous DPI message and the test reads only native control geometry.
        unsafe {
            let hwnd = loop {
                let mut after = None;
                let mut found = None;
                while let Ok(h) = FindWindowExW(None, after, w!("StashFinderNativeSearch"), None) {
                    let mut pid = 0;
                    GetWindowThreadProcessId(h, Some(&mut pid));
                    if pid == std::process::id() && IsWindowVisible(h).as_bool() {
                        found = Some(h);
                        break;
                    }
                    after = Some(h);
                }
                if let Some(h) = found {
                    break h;
                }
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(15));
            };
            let edit = GetDlgItem(Some(hwnd), 1).unwrap();
            let list = GetDlgItem(Some(hwnd), 2).unwrap();
            let mut rect = RECT::default();
            GetWindowRect(hwnd, &mut rect).unwrap();
            let mut sizes = Vec::new();
            for dpi in [96, 144, 192] {
                SendMessageW(
                    hwnd,
                    WM_DPICHANGED,
                    Some(WPARAM(dpi | dpi << 16)),
                    Some(LPARAM(&rect as *const _ as isize)),
                );
                let mut edit_rect = RECT::default();
                GetWindowRect(edit, &mut edit_rect).unwrap();
                sizes.push((
                    edit_rect.bottom - edit_rect.top,
                    SendMessageW(list, LVM_GETCOLUMNWIDTH, Some(WPARAM(0)), None).0,
                ));
            }
            PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).unwrap();
            worker.join().unwrap();
            assert_eq!(sizes, [(29, 235), (43, 352), (58, 470)]);
        }
    }
}
