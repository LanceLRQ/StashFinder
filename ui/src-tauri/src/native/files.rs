use super::*;
use crate::preferences::FileView;
use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use stashfinder_core::{
    SEARCH_CATALOG_LOADING,
    browse::{DirectoryPage, DirectoryRequest, DirectorySortField},
    index::{SearchHit, SearchPage, SearchRequest},
    search::{Scope, SortField},
};
use std::{ops::Range, path::Path};

pub(super) const OPEN: usize = 240;
pub(super) const OPEN_WITH: usize = 241;
pub(super) const BROWSE_PARENT: usize = 242;
pub(super) const LOCATE: usize = 243;
pub(super) const COPY: usize = 244;
pub(super) const CUT: usize = 245;
pub(super) const PASTE: usize = 246;
pub(super) const COPY_NAME: usize = 247;
pub(super) const COPY_PATH: usize = 248;
pub(super) const RENAME: usize = 249;
pub(super) const RECYCLE: usize = 250;
pub(super) const DELETE: usize = 251;
pub(super) const PROPERTIES: usize = 252;
pub(super) const MORE: usize = 253;
pub(super) const SELECT_ALL: usize = 254;
const INVERT: usize = 255;
pub(super) const EXPORT: usize = 256;
pub(super) const DETAILS: usize = 257;
pub(super) const SMALL: usize = 258;
pub(super) const MEDIUM: usize = 259;
pub(super) const LARGE: usize = 260;
pub(super) const COLUMNS: usize = 261;
pub(super) const FIT_COLUMNS: usize = 262;
pub(super) const SORT: usize = 263;
pub(super) const MATCH_CASE: usize = 264;
pub(super) const MATCH_WORD: usize = 265;
pub(super) const MATCH_PATH: usize = 266;
pub(super) const MATCH_REGEX: usize = 267;
pub(super) const BOOKMARK: usize = 268;
pub(super) const BOOKMARKS: usize = 269;
pub(super) const OPEN_BOOKMARK: usize = 270;
pub(super) const PRESETS: usize = 271;
pub(super) const CONTEXT_MENU: usize = 272;
pub(super) const DRAG: usize = 273;
pub(super) const CANCEL_EXPORT: usize = 274;
pub(super) const ONLINE: usize = 275;
pub(super) const HISTORY: usize = 276;
pub(super) const QUICK_FILTER: usize = 277;
const CRUMB_FIRST: usize = 500;

pub(super) const EDIT_MENU: &[(usize, &str)] = &[
    (CUT, "剪切\tCtrl+X"),
    (COPY, "复制文件\tCtrl+C"),
    (PASTE, "粘贴\tCtrl+V"),
    (COPY_NAME, "复制名称"),
    (COPY_PATH, "复制路径\tCtrl+Shift+C"),
    (0, ""),
    (SELECT_ALL, "全选\tCtrl+A"),
    (INVERT, "反选"),
    (0, ""),
    (RENAME, "重命名\tF2"),
    (RECYCLE, "删除\tDelete"),
    (DELETE, "永久删除\tShift+Delete"),
    (PROPERTIES, "属性\tAlt+Enter"),
];
const FILE_MENU: &[(usize, &str)] = &[
    (OPEN, "打开"),
    (OPEN_WITH, "打开方式…"),
    (BROWSE_PARENT, "在新标签页浏览所在目录"),
    (LOCATE, "在资源管理器显示"),
    (0, ""),
    (CUT, "剪切"),
    (COPY, "复制文件"),
    (PASTE, "粘贴"),
    (COPY_NAME, "复制名称"),
    (COPY_PATH, "复制路径"),
    (RENAME, "重命名"),
    (RECYCLE, "删除"),
    (PROPERTIES, "属性"),
    (MORE, "更多…"),
];
const FILE_COLUMNS: &[(SortField, &str, i32)] = &[
    (SortField::Name, "名称", 222),
    (SortField::Path, "所在目录 / 路径", 244),
    (SortField::Partition, "分区", 122),
    (SortField::Size, "大小", 106),
    (SortField::Type, "类型", 86),
    (SortField::Modified, "修改时间", 153),
    (SortField::Created, "创建时间", 153),
    (SortField::Extension, "扩展名", 83),
    (SortField::Attributes, "属性", 70),
    (SortField::Group, "重复组", 82),
];

#[derive(Serialize, Deserialize)]
struct ColumnSet {
    visible: Vec<usize>,
    widths: Vec<i32>,
    order: Vec<i32>,
}
impl ColumnSet {
    fn new(search: bool) -> Self {
        let visible = if search {
            vec![0, 1, 2, 3, 4, 5]
        } else {
            vec![0, 3, 4, 5]
        };
        let order = (0..visible.len() as i32).collect();
        Self {
            visible,
            order,
            widths: FILE_COLUMNS.iter().map(|(_, _, w)| *w).collect(),
        }
    }
    fn valid(&self) -> bool {
        !self.visible.is_empty()
            && self.widths.len() == FILE_COLUMNS.len()
            && self.visible.iter().all(|i| *i < FILE_COLUMNS.len())
            && self.order.len() == self.visible.len()
            && self
                .order
                .iter()
                .all(|i| (0..self.visible.len() as i32).contains(i))
    }
}
#[derive(Serialize, Deserialize)]
pub(super) struct Columns {
    #[serde(skip)]
    path: std::path::PathBuf,
    browse: ColumnSet,
    search: ColumnSet,
}
impl Columns {
    pub fn load(config: &Path) -> Self {
        let path = config.join("native-columns.json");
        let mut columns = std::fs::read(&path)
            .ok()
            .and_then(|v| serde_json::from_slice::<Self>(&v).ok())
            .filter(|v| v.browse.valid() && v.search.valid())
            .unwrap_or_else(|| Self {
                path: Default::default(),
                browse: ColumnSet::new(false),
                search: ColumnSet::new(true),
            });
        columns.path = path;
        columns
    }
    fn active(&mut self, search: bool) -> &mut ColumnSet {
        if search {
            &mut self.search
        } else {
            &mut self.browse
        }
    }
    fn save(&self) -> Result<()> {
        use std::io::Write;
        let folder = self.path.parent().context("列设置缺少父目录")?;
        std::fs::create_dir_all(folder)?;
        let mut file = tempfile::NamedTempFile::new_in(folder)?;
        file.write_all(&serde_json::to_vec(self)?)?;
        file.persist(&self.path)?;
        Ok(())
    }
}

impl Desktop {
    pub(super) fn rebuild_tabs(&self) {
        let (titles, active) = {
            let tabs = self.tabs.borrow();
            (
                tabs.items
                    .iter()
                    .skip(1)
                    .map(Tab::title)
                    .collect::<Vec<_>>(),
                tabs.active,
            )
        };
        send(self.tab_strip, TCM_DELETEALLITEMS, 0, 0);
        for (i, label) in titles.iter().enumerate() {
            let label = HSTRING::from(format!("{label}   ×"));
            let item = TCITEMW {
                mask: TCIF_TEXT,
                pszText: PWSTR(label.as_ptr().cast_mut()),
                ..Default::default()
            };
            send(
                self.tab_strip,
                TCM_INSERTITEMW,
                i,
                (&raw const item) as isize,
            );
        }
        send(self.tab_strip, TCM_SETCURSEL, active.wrapping_sub(1), 0);
        check(self.overview, active == 0);
        enable(self.close_tab, active != 0);
    }
    fn save_tab(&self) {
        if self.pending_input.replace(false) {
            self.files_submit(false);
        }
        let positions = self.selection();
        let top = send(self.list, LVM_GETTOPINDEX, 0, 0).max(0) as usize;
        let mut tabs = self.tabs.borrow_mut();
        let tab = tabs.active_mut();
        tab.selected = positions;
        tab.top = top;
    }
    pub(super) fn activate(&self, i: usize) {
        {
            let tabs = self.tabs.borrow();
            if i >= tabs.items.len() || i == tabs.active {
                return;
            }
        }
        self.save_tab();
        self.tabs.borrow_mut().active = i;
        self.rebuild_tabs();
        self.present(true);
        if matches!(self.tabs.borrow().active().pane, Page::Search(_)) {
            focus(self.query);
        } else {
            focus(self.list);
        }
    }
    pub(super) fn open_tab(&self, page: Page) {
        self.save_tab();
        self.tabs.borrow_mut().add(page, &self.preferences.borrow());
        self.rebuild_tabs();
        self.present(true);
        self.refresh_current();
    }
    pub(super) fn close_at(&self, index: usize) {
        if self.tabs.borrow().active != index {
            self.save_tab();
        }
        let removed = self.tabs.borrow_mut().close(index);
        if let Some(tab) = removed {
            if let Page::Search(s) = tab.pane
                && let Some(id) = s.id
            {
                self.cancel_snapshot(id);
            }
            self.rebuild_tabs();
            self.present(true);
        }
    }
    pub(super) fn cancel_snapshot(&self, id: u64) {
        if self.exported_snapshot.get() != Some(id) {
            self.spawn(runtime::request(Action::SearchCancel { id }), |_, _| {});
        }
    }
    pub(super) fn release_searches(&self) {
        let ids = self
            .tabs
            .borrow()
            .items
            .iter()
            .filter_map(|t| match &t.pane {
                Page::Search(s) => s.id,
                _ => None,
            })
            .collect::<Vec<_>>();
        if !ids.is_empty() {
            self.runtime.block_on(async {
                let _ = tokio::time::timeout(stashfinder_windows::ipc::IO_TIMEOUT, async {
                    for id in ids {
                        if runtime::request(Action::SearchCancel { id }).await.is_err() {
                            break;
                        }
                    }
                })
                .await;
            });
        }
    }
    pub(super) fn tab_at(&self, point: POINT) -> Option<usize> {
        let mut hit = TCHITTESTINFO {
            pt: point,
            ..Default::default()
        };
        let index = send(self.tab_strip, TCM_HITTEST, 0, (&raw mut hit) as isize);
        usize::try_from(index).ok().map(|index| index + 1)
    }
    pub(super) fn tab_close_at(&self, point: POINT) -> Option<usize> {
        let index = self.tab_at(point)?;
        let mut item = RECT::default();
        let visible = send(
            self.tab_strip,
            TCM_GETITEMRECT,
            index - 1,
            (&raw mut item) as isize,
        ) != 0;
        // SAFETY: This DPI is queried for the current thread's live tab strip.
        let dpi = unsafe { GetDpiForWindow(self.tab_strip) } as i32;
        (visible && point.x >= item.right - 22 * dpi / 96).then_some(index)
    }
    pub(super) fn tab_context(&self) {
        let mut point = POINT::default();
        // SAFETY: Hit-testing uses the cursor and the current thread's native tab control.
        unsafe {
            let _ = GetCursorPos(&mut point);
        }
        let mut local = point;
        // SAFETY: The point is writable and the tab-strip belongs to the current thread.
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(self.tab_strip, &mut local);
        }
        if let Some(index) = self.tab_at(local) {
            self.activate(index);
            self.context_menu(
                &[
                    (CLOSE_TAB, "关闭标签页"),
                    (NEW_BROWSE, "新建浏览标签页"),
                    (NEW_SEARCH, "新建搜索标签页"),
                ],
                Some(point),
            );
        }
    }
    pub(super) fn new_browse(&self, location: Option<Location>) {
        self.open_browse(location, None);
    }
    pub(super) fn open_browse(&self, location: Option<Location>, reveal: Option<Vec<u8>>) {
        let filter = self.default_filter();
        let mut browse = Browse::at(location, filter);
        browse.reveal = reveal;
        self.open_tab(Page::Browse(Box::new(browse)));
    }
    pub(super) fn new_search(&self, text: String, location: Option<Location>) {
        let mut search = Search::new(text, &self.preferences.borrow());
        search.originating = location.clone();
        if let Some(location) = location {
            search.request.partitions = vec![location.volume.id.clone()];
            search.request.filesystem_id = Some(location.volume.filesystem);
            search.request.options.scope = location.parent.map(|entry| Scope {
                partition_id: location.volume.id,
                entry,
                recursive: true,
            });
        }
        if let Some(filter) = self.default_filter() {
            apply_file_filter(&mut search.request, filter);
        }
        self.open_tab(Page::Search(Box::new(search)));
        focus(self.query);
    }
    pub(super) fn files_submit(&self, explicit: bool) {
        let query = text(self.query);
        let open = {
            let mut tabs = self.tabs.borrow_mut();
            match &mut tabs.active_mut().pane {
                Page::Search(s) => {
                    s.request.text = query.clone();
                    None
                }
                Page::Browse(b) if !query.trim().is_empty() => Some(b.location().cloned()),
                Page::Overview if explicit => Some(None),
                _ => return,
            }
        };
        if let Some(location) = open {
            self.pending_input.set(false);
            self.new_search(query, location);
        } else {
            self.rebuild_tabs();
            self.start_search();
        }
    }
    pub(super) fn change_scope(&self) {
        let selection = send(self.scope, CB_GETCURSEL, 0, 0);
        if selection < 0 || self.changing.get() {
            return;
        }
        let (choices, _) = self.search_scopes();
        let Some((_, scope)) = choices.get(selection as usize) else {
            return;
        };
        let mut tabs = self.tabs.borrow_mut();
        let Page::Search(s) = &mut tabs.active_mut().pane else {
            return;
        };
        if selection == 0 {
            s.request.partitions.clear();
            s.request.filesystem_id = None;
            s.request.options.scope = None;
        } else if let Some(location) = scope {
            s.request.partitions = vec![location.volume.id.clone()];
            s.request.filesystem_id = Some(location.volume.filesystem.clone());
            s.request.options.scope = location.parent.as_ref().map(|entry| Scope {
                partition_id: location.volume.id.clone(),
                entry: entry.clone(),
                recursive: true,
            });
        }
        drop(tabs);
        self.start_search();
    }
    fn search_scopes(&self) -> (Vec<(String, Option<Location>)>, usize) {
        let mut choices = vec![("全部已登记分区".into(), None)];
        let tabs = self.tabs.borrow();
        let Page::Search(s) = &tabs.active().pane else {
            return (choices, 0);
        };
        if let Some(origin) = &s.originating {
            if s.from_bookmark {
                choices.push((
                    format!("书签范围：{}", origin.volume.name),
                    Some(origin.clone()),
                ));
            } else {
                choices.push((
                    "原浏览分区".into(),
                    Some(Location {
                        volume: origin.volume.clone(),
                        parent: None,
                    }),
                ));
                if origin.parent.is_some() {
                    choices.push((
                        format!("{}及子目录", origin.volume.name),
                        Some(origin.clone()),
                    ));
                }
            }
        }
        choices.extend(self.snapshot.borrow().assets.iter().map(|a| {
            (
                a.registration.name.clone(),
                Some(Location {
                    volume: Volume::from(a),
                    parent: None,
                }),
            )
        }));
        let selected = choices
            .iter()
            .position(|(_, place)| {
                place.as_ref().is_some_and(|p| {
                    s.request.partitions.as_slice() == [p.volume.id.as_str()]
                        && s.request.filesystem_id.as_ref() == Some(&p.volume.filesystem)
                        && p.parent.as_ref()
                            == s.request.options.scope.as_ref().map(|scope| &scope.entry)
                })
            })
            .unwrap_or(0);
        (choices, selected)
    }
    pub(super) fn refresh_current(&self) {
        let kind = {
            let tabs = self.tabs.borrow();
            match tabs.active().pane {
                Page::Overview => 0,
                Page::Browse(_) => 1,
                Page::Search(_) => 2,
                Page::Tasks => 3,
            }
        };
        match kind {
            0 => self.refresh_assets(true),
            1 => self.start_directory(),
            2 => self.start_search(),
            _ => {
                self.refresh_assets(false);
                self.refresh_tasks();
            }
        }
    }
    pub(super) fn manual_refresh(&self) {
        if self.exporting.active() {
            self.status("正在导出搜索结果");
            return;
        }
        let request = match self.tabs.borrow().active().pane {
            Page::Search(_) => 1,
            Page::Browse(_) => 2,
            _ => 0,
        };
        match request {
            1 => self.refresh_search(),
            2 => self.refresh_directory(),
            _ => self.refresh_current(),
        }
    }
    pub(super) fn present(&self, reset: bool) {
        if self.changing.replace(true) {
            return;
        }
        let (count, label, query, top, layout, status) = {
            let tabs = self.tabs.borrow();
            let tab = tabs.active();
            let (count, query) = match &tab.pane {
                Page::Overview => (self.asset_rows.borrow().len(), None),
                Page::Search(s) => (s.total, Some(s.request.text.clone())),
                Page::Browse(b) => (b.directory.as_ref().map_or(0, |d| d.total), None),
                Page::Tasks => (self.visible_tasks.borrow().len(), None),
            };
            (
                count,
                tab.title(),
                query,
                tab.top,
                tab.layout.clone(),
                tab.status.clone(),
            )
        };
        if reset {
            self.view_hint.set(None);
            self.apply_selection(&Selection::None);
            self.apply_columns();
            self.set_view(layout);
            set_text(self.query, query.as_deref().unwrap_or(""));
            self.show_search_scope();
            self.rebuild_tree();
        }
        set_text(self.path, &self.breadcrumb().unwrap_or(label));
        if reset {
            self.layout();
        }
        if send(self.list, LVM_GETITEMCOUNT, 0, 0) != count as isize {
            send(self.list, LVM_SETITEMCOUNT, count, LVSICF_NOSCROLL as isize);
        }
        if count > 0 {
            if reset {
                self.apply_selection(&self.tabs.borrow().active().selected);
                send(self.list, LVM_ENSUREVISIBLE, top.min(count - 1), 0);
            }
            let (first, end) = self.visible_range();
            if first < count {
                send(
                    self.list,
                    LVM_REDRAWITEMS,
                    first,
                    (end.min(count) - 1) as isize,
                );
            }
        }
        set_text(self.status, &status);
        self.changing.set(false);
        self.update_preview();
        self.update_menu();
    }
    pub(super) fn show_search_scope(&self) {
        let (choices, current) = self.search_scopes();
        combo_fill(
            self.scope,
            &choices.into_iter().map(|(s, _)| s).collect::<Vec<_>>(),
            current,
        );
    }
    fn breadcrumb(&self) -> Option<String> {
        let tabs = self.tabs.borrow();
        let Page::Browse(b) = &tabs.active().pane else {
            return None;
        };
        let place = b.location()?;
        let mut label = place.volume.name.clone();
        if let Some(d) = &b.directory {
            for ancestor in &d.ancestors {
                if ancestor.name.is_empty() || ancestor.name == "\\" {
                    continue;
                }
                label.push_str("  ›  ");
                label.push_str(&ancestor.name);
            }
        }
        Some(label)
    }
    pub(super) fn change_visibility(&self, cmd: usize) {
        let mut tabs = self.tabs.borrow_mut();
        let tab = tabs.active_mut();
        if cmd == PREVIEW {
            tab.preview = !tab.preview;
        }
        if let Page::Browse(b) = &mut tab.pane {
            if cmd == TREE {
                b.tree = !b.tree;
            }
            if cmd == BREADCRUMBS {
                b.breadcrumbs = !b.breadcrumbs;
            }
        }
        drop(tabs);
        self.layout();
        self.update_preview();
    }

    fn search_mutate(&self, f: impl FnOnce(&mut SearchRequest)) {
        let mut tabs = self.tabs.borrow_mut();
        let Page::Search(s) = &mut tabs.active_mut().pane else {
            return;
        };
        f(&mut s.request);
        drop(tabs);
        self.start_search();
    }
    pub(super) fn file_command(&self, command: usize) {
        use stashfinder_core::search::MatchMode;
        match command {
            MATCH_CASE => {
                self.search_mutate(|r| r.options.case_sensitive = !r.options.case_sensitive)
            }
            MATCH_WORD => self.search_mutate(|r| {
                r.options.mode = if r.options.mode == MatchMode::WholeWord {
                    MatchMode::Contains
                } else {
                    MatchMode::WholeWord
                }
            }),
            MATCH_REGEX => self.search_mutate(|r| {
                r.options.mode = if r.options.mode == MatchMode::Regex {
                    MatchMode::Contains
                } else {
                    MatchMode::Regex
                }
            }),
            MATCH_PATH => self.search_mutate(|r| r.names_only = !r.names_only),
            ONLINE => self.search_mutate(|r| r.options.online_only = !r.options.online_only),
            HISTORY => {
                self.search_mutate(|r| r.options.include_history = !r.options.include_history)
            }
            QUICK_FILTER => self.quick_filter(),
            BOOKMARK => self.save_bookmark(),
            BOOKMARKS => self.bookmarks(true),
            OPEN_BOOKMARK => self.bookmarks(false),
            PRESETS => self.manage_presets(),
            EXPORT => self.export(),
            CANCEL_EXPORT => {
                self.exporting.cancel();
            }
            SELECT_ALL => set_row_state(self.list, usize::MAX, true),
            INVERT => self.invert_selection(),
            DETAILS..=LARGE => {
                let layout = match command {
                    SMALL => FileView::Small,
                    MEDIUM => FileView::Medium,
                    LARGE => FileView::Large,
                    _ => FileView::Details,
                };
                {
                    let mut tabs = self.tabs.borrow_mut();
                    if !matches!(tabs.active().pane, Page::Browse(_) | Page::Search(_)) {
                        return;
                    }
                    tabs.active_mut().layout = layout.clone();
                }
                self.set_view(layout);
                self.update_menu();
            }
            FIT_COLUMNS => self.fit_columns(),
            COLUMNS => self.columns_dialog(),
            SORT => self.sort_dialog(),
            CONTEXT_MENU => self.files_context(),
            i if i >= CRUMB_FIRST => {
                let place = {
                    let tabs = self.tabs.borrow();
                    let Page::Browse(b) = &tabs.active().pane else {
                        return;
                    };
                    let Some(place) = b.location() else {
                        return;
                    };
                    let id = i - CRUMB_FIRST;
                    let parent = if id == 0 {
                        None
                    } else {
                        let Some(a) = b.directory.as_ref().and_then(|d| d.ancestors.get(id - 1))
                        else {
                            return;
                        };
                        Some(a.entry.clone())
                    };
                    Location {
                        volume: place.volume.clone(),
                        parent,
                    }
                };
                self.visit(place);
            }
            OPEN..=MORE | DRAG => self.operate(command),
            _ => {}
        }
    }
    fn files_context(&self) {
        let kind = match self.tabs.borrow().active().pane {
            Page::Overview => 0,
            Page::Tasks => 1,
            _ => 2,
        };
        match kind {
            0 => self.context_menu(library::ASSET_MENU, None),
            1 => self.context_menu(library::TASK_MENU, None),
            _ => {
                let commands = FILE_MENU
                    .iter()
                    .copied()
                    .filter(|(id, _)| *id == 0 || self.may_operate(*id))
                    .collect::<Vec<_>>();
                self.context_menu(&commands, None);
            }
        }
    }
    fn apply_columns(&self) {
        while send(self.list, LVM_DELETECOLUMN, 0, 0) != 0 {}
        let tabs = self.tabs.borrow();
        let column_sets = self.columns.borrow();
        let file_columns = match &tabs.active().pane {
            Page::Browse(_) => Some(&column_sets.browse),
            Page::Search(_) => Some(&column_sets.search),
            _ => None,
        };
        if let Some(columns) = file_columns {
            for (i, &field) in columns.visible.iter().enumerate() {
                self.add_column(i, FILE_COLUMNS[field].1, columns.widths[field]);
            }
            send(
                self.list,
                LVM_SETCOLUMNORDERARRAY,
                columns.order.len(),
                columns.order.as_ptr() as isize,
            );
            return;
        }
        let columns = if matches!(tabs.active().pane, Page::Tasks) {
            library::TASK_COLUMNS
        } else {
            library::ASSET_COLUMNS
        };
        for (i, &(title, width)) in columns.iter().enumerate() {
            self.add_column(i, title, width);
        }
    }
    fn add_column(&self, i: usize, title: &str, width: i32) {
        let label = HSTRING::from(title);
        let column = LVCOLUMNW {
            mask: LVCF_TEXT | LVCF_WIDTH,
            pszText: PWSTR(label.as_ptr().cast_mut()),
            cx: width,
            ..Default::default()
        };
        send(
            self.list,
            LVM_INSERTCOLUMNW,
            i,
            (&raw const column) as isize,
        );
    }
    fn set_view(&self, layout: FileView) {
        let file = matches!(
            self.tabs.borrow().active().pane,
            Page::Browse(_) | Page::Search(_)
        );
        let size = if file {
            match layout {
                FileView::Details => 0,
                FileView::Small => 48,
                FileView::Medium => 96,
                FileView::Large => 160,
            }
        } else {
            0
        };
        self.set_images(if file && size == 0 { 18 } else { size });
        if size == 0 {
            send(self.list, LVM_SETVIEW, LV_VIEW_DETAILS as usize, 0);
        } else {
            send(self.list, LVM_SETVIEW, LV_VIEW_ICON as usize, 0);
            send(
                self.list,
                LVM_SETICONSPACING,
                0,
                (((size + 32) << 16) | (size + 42)) as isize,
            );
        }
    }
    fn file_page(&self) -> Option<bool> {
        match self.tabs.borrow().active().pane {
            Page::Browse(_) => Some(false),
            Page::Search(_) => Some(true),
            _ => None,
        }
    }
    fn fit_columns(&self) {
        if let Some(search) = self.file_page() {
            let count = self.columns.borrow_mut().active(search).visible.len();
            for i in 0..count {
                send(
                    self.list,
                    LVM_SETCOLUMNWIDTH,
                    i,
                    LVSCW_AUTOSIZE_USEHEADER as isize,
                );
            }
            self.remember_columns();
        }
    }
    fn remember_columns(&self) {
        if self.changing.get() {
            return;
        }
        let Some(search) = self.file_page() else {
            return;
        };
        let mut columns = self.columns.borrow_mut();
        let data = columns.active(search);
        for (i, &field) in data.visible.iter().enumerate() {
            data.widths[field] = (send(self.list, LVM_GETCOLUMNWIDTH, i, 0) as i32).max(24);
        }
        send(
            self.list,
            LVM_GETCOLUMNORDERARRAY,
            data.visible.len(),
            data.order.as_mut_ptr() as isize,
        );
        let saved = columns.save();
        drop(columns);
        if let Err(e) = saved {
            self.status(&format!("无法保存列布局：{e:#}"));
        }
    }
    fn columns_dialog(&self) {
        let Some(search) = self.file_page() else {
            return;
        };
        let fields = {
            let mut sets = self.columns.borrow_mut();
            let data = sets.active(search);
            FILE_COLUMNS
                .iter()
                .enumerate()
                .filter(|(i, _)| search || ![2, 9].contains(i))
                .map(|(i, (_, title, _))| (i, Field::Check(title, data.visible.contains(&i))))
                .collect::<Vec<_>>()
        };
        if let Some(mut values) = self.ask(
            "详情列",
            &fields.iter().map(|(_, f)| f.clone()).collect::<Vec<_>>(),
        ) {
            let mut columns = self.columns.borrow_mut();
            let column = columns.active(search);
            column.visible = fields
                .into_iter()
                .filter_map(|(i, _)| (values.check() || i == 0).then_some(i))
                .collect();
            column.order = (0..column.visible.len() as i32).collect();
            let result = columns.save();
            drop(columns);
            self.present(true);
            if let Err(e) = result {
                self.status(&format!("无法保存列布局：{e:#}"));
            }
        }
    }
    fn sort_dialog(&self) {
        let Some(search) = self.file_page() else {
            return;
        };
        let fields = FILE_COLUMNS
            .iter()
            .filter(|(f, _, _)| search || !matches!(f, SortField::Partition | SortField::Group))
            .collect::<Vec<_>>();
        let (sort, descending) = self.sort_state();
        let selected = fields.iter().position(|(f, _, _)| *f == sort).unwrap_or(0);
        if let Some(mut values) = self.ask(
            "排序",
            &[
                Field::Choice(
                    "排序依据",
                    fields.iter().map(|(_, s, _)| (*s).into()).collect(),
                    selected,
                ),
                Field::Check("降序", descending),
            ],
        ) && let Some(&(field, _, _)) = fields.get(values.choice())
        {
            self.sort_by(*field, values.check());
        }
    }
    fn sort_state(&self) -> (SortField, bool) {
        match &self.tabs.borrow().active().pane {
            Page::Search(s) => (s.request.options.sort, s.request.options.descending),
            Page::Browse(b) => (
                FILE_COLUMNS
                    .iter()
                    .map(|(f, _, _)| *f)
                    .find(|f| {
                        directory_sort(*f) as u8 == b.sort.field as u8
                            && !matches!(f, SortField::Group | SortField::Partition)
                    })
                    .unwrap_or_default(),
                b.sort.descending,
            ),
            _ => (SortField::Name, false),
        }
    }
    fn sort_column(&self, index: usize) {
        let Some(search) = self.file_page() else {
            return;
        };
        let field = self
            .columns
            .borrow_mut()
            .active(search)
            .visible
            .get(index)
            .map(|i| FILE_COLUMNS[*i].0);
        if let Some(field) = field {
            let (previous, desc) = self.sort_state();
            self.sort_by(field, previous == field && !desc);
        }
    }
    fn sort_by(&self, field: SortField, descending: bool) {
        let mut tabs = self.tabs.borrow_mut();
        match &mut tabs.active_mut().pane {
            Page::Browse(b) => {
                b.sort.field = directory_sort(field);
                b.sort.descending = descending;
            }
            Page::Search(s) => {
                s.request.options.sort = field;
                s.request.options.descending = descending;
            }
            _ => return,
        }
        drop(tabs);
        self.refresh_current();
    }

    pub(super) fn selection(&self) -> Selection {
        let total = send(self.list, LVM_GETITEMCOUNT, 0, 0).max(0) as usize;
        let selected = send(self.list, LVM_GETSELECTEDCOUNT, 0, 0).max(0) as usize;
        if selected == 0 {
            return Selection::None;
        }
        if total == selected && selected > 1 {
            return Selection::All;
        }
        let mut positions = Vec::with_capacity(selected);
        let mut index = usize::MAX;
        loop {
            let found = send(self.list, LVM_GETNEXTITEM, index, LVNI_SELECTED as isize);
            if found < 0 {
                break;
            }
            index = found as usize;
            positions.push(index);
        }
        Selection::Positions(positions)
    }
    pub(super) fn single(&self) -> Option<usize> {
        if send(self.list, LVM_GETSELECTEDCOUNT, 0, 0) != 1 {
            return None;
        }
        usize::try_from(send(
            self.list,
            LVM_GETNEXTITEM,
            usize::MAX,
            LVNI_SELECTED as isize,
        ))
        .ok()
    }
    pub(super) fn apply_selection(&self, selection: &Selection) {
        set_row_state(self.list, usize::MAX, matches!(selection, Selection::All));
        if let Selection::Positions(positions) = selection {
            for &index in positions {
                set_row_state(self.list, index, true);
            }
        }
    }
    fn invert_selection(&self) {
        match self.selection() {
            Selection::None => set_row_state(self.list, usize::MAX, true),
            Selection::All => set_row_state(self.list, usize::MAX, false),
            Selection::Positions(positions) => {
                set_row_state(self.list, usize::MAX, true);
                for index in positions {
                    set_row_state(self.list, index, false);
                }
            }
        }
        self.update_preview();
    }

    pub(super) fn files_notify(&self, hdr: &NMHDR, l: LPARAM) -> Option<isize> {
        if hdr.hwndFrom == self.tree {
            return self.tree_notify(hdr, l);
        }
        if hdr.hwndFrom != self.list {
            if hdr.hwndFrom.0 as isize == send(self.list, LVM_GETHEADER, 0, 0)
                && [HDN_ENDTRACKW, HDN_ENDDRAG].contains(&hdr.code)
            {
                // The header commits its new order after returning from the notification.
                let _ = self.sender.send(Box::new(|app| app.remember_columns()));
                // SAFETY: A scalar wakeup follows the queued callback.
                let _ = unsafe { PostMessageW(Some(self.hwnd), DRAIN, WPARAM(0), LPARAM(0)) };
            }
            return None;
        }
        // SAFETY: The Win32 notification is interpreted only after matching its control and code.
        unsafe {
            match hdr.code {
                LVN_GETDISPINFOW => {
                    let item = &mut (*(l.0 as *mut NMLVDISPINFOW)).item;
                    let row = item.iItem.max(0) as usize;
                    if item.mask.contains(LVIF_TEXT)
                        && !item.pszText.is_null()
                        && item.cchTextMax > 0
                    {
                        let text = self.list_cell(row, item.iSubItem.max(0) as usize);
                        for (index, unit) in text
                            .encode_utf16()
                            .take(item.cchTextMax as usize - 1)
                            .chain(std::iter::once(0))
                            .enumerate()
                        {
                            item.pszText.0.add(index).write_unaligned(unit);
                        }
                    }
                    if item.mask.contains(LVIF_IMAGE) {
                        item.iImage = self.row_image(row);
                    }
                }
                LVN_COLUMNCLICK if !self.changing.get() => {
                    self.sort_column((*(l.0 as *const NMLISTVIEW)).iSubItem.max(0) as usize);
                }
                LVN_ITEMCHANGED | LVN_ODSTATECHANGED if !self.changing.get() => {
                    self.defer_preview()
                }
                LVN_ODCACHEHINT => {
                    let hint = &*(l.0 as *const NMLVCACHEHINT);
                    self.view_hint.set(Some((
                        hint.iFrom.max(0) as usize,
                        hint.iTo.max(hint.iFrom) as usize + 1,
                    )));
                }
                LVN_BEGINDRAG => self.operate(DRAG),
                NM_DBLCLK => self.operate(OPEN),
                NM_RCLICK => self.files_context(),
                _ => return None,
            }
        }
        Some(0)
    }
    fn list_cell(&self, row: usize, column: usize) -> String {
        let tabs = self.tabs.borrow();
        match &tabs.active().pane {
            Page::Overview => self.asset_cell(row, column),
            Page::Tasks => self.task_cell(row, column),
            Page::Browse(b) => {
                let cols = self.columns.borrow();
                b.directory
                    .as_ref()
                    .and_then(|d| d.records.get(&row))
                    .map_or_else(
                        || "…".into(),
                        |r| cell(r, cols.browse.visible.get(column).copied(), None),
                    )
            }
            Page::Search(s) => {
                let cols = self.columns.borrow();
                s.rows.get(&row).map_or_else(
                    || "…".into(),
                    |r| cell(&r.file, cols.search.visible.get(column).copied(), Some(r)),
                )
            }
        }
    }

    pub(super) fn files_tick(&self) {
        let (top, end) = self.visible_range();
        let mut tabs = self.tabs.borrow_mut();
        let tab = tabs.active_mut();
        if tab.pending {
            return;
        }
        if tab.index_dirty
            && !self.exporting.active()
            && !self.file_operation_active()
            && send(self.list, LVM_GETSELECTEDCOUNT, 0, 0) <= 1
        {
            tab.index_dirty = false;
            let query = matches!(tab.pane, Page::Search(_));
            drop(tabs);
            if query {
                self.refresh_search();
            } else {
                self.refresh_directory();
            }
            return;
        }
        let range = top.saturating_sub(12)..end.max(top + 96).saturating_add(12);
        let id = tab.id;
        let epoch = tab.epoch;
        match &tab.pane {
            Page::Search(s) => {
                let Some(snapshot) = s.id else {
                    return;
                };
                let range = range.start..range.end.min(s.total);
                if range.clone().all(|i| s.rows.contains_key(&i)) {
                    return;
                }
                tab.pending = true;
                drop(tabs);
                self.fetch_search(id, epoch, snapshot, range);
            }
            Page::Browse(b) => {
                if b.stale {
                    return;
                }
                let Some(directory) = &b.directory else {
                    return;
                };
                let range = range.start..range.end.min(directory.total);
                if range.clone().all(|i| directory.records.contains_key(&i)) {
                    return;
                }
                let Some(mut request) = directory_request(b, true) else {
                    return;
                };
                request.offset = range.start as u32;
                tab.pending = true;
                drop(tabs);
                self.fetch_directory(id, epoch, request, range);
            }
            _ => {}
        }
    }

    fn start_search(&self) {
        let mut tabs = self.tabs.borrow_mut();
        let tab = tabs.active_mut();
        let Page::Search(search) = &mut tab.pane else {
            return;
        };
        let mut request = search.request.clone();
        request.replaces = search
            .id
            .take()
            .filter(|id| self.exported_snapshot.get() != Some(*id));
        search.total = 0;
        search.rows.clear();
        tab.epoch += 1;
        tab.index_dirty = false;
        let (id, epoch) = (tab.id, tab.epoch);
        tab.pending = true;
        tab.selected = Selection::None;
        tab.top = 0;
        tab.status = "正在搜索…".into();
        drop(tabs);
        self.present(false);
        self.spawn(start_query(request), move |app, started| {
            let mut tabs = app.tabs.borrow_mut();
            let active = tabs.active().id == id;
            let Some(tab) = tabs.get(id).filter(|t| t.epoch == epoch) else {
                drop(tabs);
                if let Ok(id) = started {
                    app.spawn(runtime::request(Action::SearchCancel { id }), |_, _| {});
                }
                return;
            };
            let Page::Search(s) = &mut tab.pane else {
                return;
            };
            match started {
                Ok(snapshot) => {
                    s.id = Some(snapshot);
                    drop(tabs);
                    app.fetch_search(id, epoch, snapshot, 0..96);
                }
                Err(error) => {
                    tab.pending = false;
                    tab.status = format!("无法完成搜索：{error:#}");
                    drop(tabs);
                    if active {
                        app.present(false);
                    }
                }
            }
        });
    }

    fn refresh_search(&self) {
        let selected = self.single();
        let (top, end) = self.visible_range();
        let mut tabs = self.tabs.borrow_mut();
        let tab = tabs.active_mut();
        let Page::Search(search) = &tab.pane else {
            return;
        };
        let Some(previous) = search.id else {
            drop(tabs);
            self.start_search();
            return;
        };
        let request = search.request.clone();
        let id = tab.id;
        tab.epoch += 1;
        let epoch = tab.epoch;
        tab.pending = true;
        tab.index_dirty = false;
        drop(tabs);
        self.status("正在刷新搜索结果…");
        self.spawn(
            async move {
                let snapshot = start_query(request).await?;
                let fresh = async {
                    let (meta, first) = read_search(snapshot, 0..96).await?;
                    let selected = if let Some(offset) = selected {
                        match runtime::request(Action::SearchReselect {
                            id: snapshot,
                            previous,
                            offset,
                        })
                        .await
                        {
                            Ok(Response::SearchPosition(index)) => index,
                            _ => None,
                        }
                    } else {
                        None
                    };
                    let offset = selected
                        .unwrap_or(top)
                        .min(meta.total.saturating_sub(1))
                        .saturating_sub(16);
                    let end = offset + (end - top).max(96);
                    let rows = if offset == 0 || meta.total == 0 {
                        first
                    } else {
                        read_search(snapshot, offset..end).await?.1
                    };
                    Ok::<_, anyhow::Error>((snapshot, meta, offset, rows, selected))
                }
                .await;
                if fresh.is_err() {
                    let _ = runtime::request(Action::SearchCancel { id: snapshot }).await;
                }
                fresh
            },
            move |app, result| {
                let old_selection = app.single();
                let visible_top = app.visible_range().0;
                let mut tabs = app.tabs.borrow_mut();
                let active = tabs.active().id == id;
                let Some(tab) = tabs.get(id).filter(|tab| tab.epoch == epoch) else {
                    drop(tabs);
                    if let Ok((id, ..)) = result {
                        app.spawn(runtime::request(Action::SearchCancel { id }), |_, _| {});
                    }
                    return;
                };
                let Page::Search(s) = &mut tab.pane else {
                    return;
                };
                tab.pending = false;
                match result {
                    Ok((snapshot, meta, offset, rows, new_selection)) => {
                        s.id = Some(snapshot);
                        s.total = meta.total;
                        s.groups = meta.groups;
                        s.rows.clear();
                        add_rows(&mut s.rows, offset, rows);
                        let live_top = if active { visible_top } else { tab.top };
                        tab.top = if live_top == top {
                            new_selection.unwrap_or(live_top)
                        } else {
                            live_top
                        }
                        .min(meta.total.saturating_sub(1));
                        tab.selected = if (!active || old_selection == selected)
                            && let Some(row) = new_selection
                        {
                            Selection::Positions(vec![row])
                        } else {
                            Selection::None
                        };
                        tab.status = search_summary(&meta);
                        app.spawn(
                            runtime::request(Action::SearchCancel { id: previous }),
                            |_, _| {},
                        );
                    }
                    Err(error) => {
                        tab.status = format!("刷新搜索失败：{error:#}（保留上次结果，F5 重试）");
                    }
                }
                drop(tabs);
                if active {
                    app.present(true);
                }
            },
        );
    }
    fn fetch_search(&self, id: TabId, epoch: u64, snapshot: u64, range: Range<usize>) {
        self.spawn(read_search(snapshot, range.clone()), move |app, result| {
            let mut tabs = app.tabs.borrow_mut();
            let active = tabs.active().id == id;
            let Some(tab) = tabs.get(id).filter(|t| t.epoch == epoch) else {
                return;
            };
            let Page::Search(search) = &mut tab.pane else {
                return;
            };
            tab.pending = false;
            match result {
                Ok((meta, rows)) => {
                    search.total = meta.total;
                    search.groups = meta.groups;
                    add_rows(&mut search.rows, range.start, rows);
                    tab.status = search_summary(&meta);
                }
                Err(error) => {
                    search.id = None;
                    search.rows.clear();
                    search.total = 0;
                    app.cancel_snapshot(snapshot);
                    tab.status = format!("无法完成搜索：{error:#}（F5 重试）");
                }
            }
            drop(tabs);
            if active {
                app.present(false);
            }
        });
    }

    pub(super) fn visit(&self, place: Location) {
        {
            let mut tabs = self.tabs.borrow_mut();
            if let Page::Browse(b) = &mut tabs.active_mut().pane {
                b.visit(place);
            }
        }
        self.rebuild_tabs();
        self.start_directory();
    }
    pub(super) fn navigate(&self, command: usize) {
        if command == TAB_POPUP {
            let mut labels = Vec::new();
            {
                let tabs = self.tabs.borrow();
                let Page::Browse(b) = &tabs.active().pane else {
                    return;
                };
                if let Some(p) = b.location() {
                    labels.push((CRUMB_FIRST, p.volume.name.clone()));
                }
                if let Some(d) = &b.directory {
                    labels.extend(
                        d.ancestors
                            .iter()
                            .enumerate()
                            .filter(|(_, a)| !a.name.is_empty() && a.name != "\\")
                            .map(|(i, a)| (CRUMB_FIRST + i + 1, a.name.clone())),
                    );
                }
            }
            self.context_menu(
                &labels
                    .iter()
                    .map(|(id, title)| (*id, title.as_str()))
                    .collect::<Vec<_>>(),
                None,
            );
            return;
        }
        let mut tabs = self.tabs.borrow_mut();
        let Page::Browse(b) = &mut tabs.active_mut().pane else {
            return;
        };
        match command {
            BACK if b.cursor > 0 => b.cursor -= 1,
            FORWARD if b.cursor + 1 < b.places.len() => b.cursor += 1,
            UP => {
                let Some(p) = b.location().cloned().filter(|p| p.parent.is_some()) else {
                    return;
                };
                let parent = b
                    .directory
                    .as_ref()
                    .and_then(|d| d.ancestors.iter().rev().nth(1))
                    .filter(|a| !a.name.is_empty() && a.name != "\\")
                    .map(|a| a.entry.clone());
                b.visit(Location { parent, ..p });
            }
            _ => return,
        }
        b.directory = None;
        drop(tabs);
        self.rebuild_tabs();
        self.start_directory();
    }
    fn start_directory(&self) {
        let mut tabs = self.tabs.borrow_mut();
        let tab = tabs.active_mut();
        let Page::Browse(b) = &mut tab.pane else {
            return;
        };
        b.directory = None;
        b.stale = false;
        tab.index_dirty = false;
        let request = directory_request(b, false);
        tab.selected = Selection::None;
        tab.top = 0;
        tab.epoch += 1;
        tab.pending = request.is_some();
        tab.status = if request.is_some() {
            "正在读取目录…"
        } else {
            "选择目录树中的已登记分区开始浏览"
        }
        .into();
        let (id, epoch) = (tab.id, tab.epoch);
        drop(tabs);
        self.present(false);
        if let Some(request) = request {
            self.fetch_directory(id, epoch, request, 0..96);
        }
    }
    fn refresh_directory(&self) {
        let selection = self.selection();
        let focus = self
            .single()
            .and_then(|row| self.record(row))
            .map(|record| record.row.entry);
        let mut tabs = self.tabs.borrow_mut();
        let tab = tabs.active_mut();
        let Page::Browse(browse) = &mut tab.pane else {
            return;
        };
        let Some(request) = directory_request(browse, false) else {
            return;
        };
        if tab.pending {
            return;
        }
        if browse.directory.is_none() {
            drop(tabs);
            self.start_directory();
            return;
        }
        tab.epoch += 1;
        tab.pending = true;
        tab.index_dirty = false;
        let (id, epoch) = (tab.id, tab.epoch);
        let top = send(self.list, LVM_GETTOPINDEX, 0, 0).max(0) as usize;
        tab.top = top;
        tab.selected = selection;
        browse.reveal = focus;
        drop(tabs);
        self.fetch_directory(id, epoch, request, 0..top.min(450) + 96);
    }
    fn fetch_directory(
        &self,
        id: TabId,
        epoch: u64,
        request: DirectoryRequest,
        range: Range<usize>,
    ) {
        let initial = request.revision.is_none();
        let reveal = if initial {
            self.tabs
                .borrow_mut()
                .get(id)
                .and_then(|t| match &mut t.pane {
                    Page::Browse(b) => b.reveal.take(),
                    _ => None,
                })
        } else {
            None
        };
        self.spawn(
            directory_with_focus(request, range.end, reveal),
            move |app, result| {
                let mut tabs = app.tabs.borrow_mut();
                let active = tabs.active().id == id;
                let Some(tab) = tabs.get(id).filter(|t| t.epoch == epoch) else {
                    return;
                };
                let Page::Browse(b) = &mut tab.pane else {
                    return;
                };
                tab.pending = false;
                match result {
                    Ok((meta, rows, revealed)) => {
                        b.stale = false;
                        if initial {
                            b.directory = None;
                            tab.selected = Selection::None;
                        }
                        let d = b.directory.get_or_insert_with(|| Directory {
                            generation: meta.generation,
                            revision: meta.revision,
                            location: meta.location,
                            ancestors: meta.ancestors,
                            records: Rows::new(),
                            total: meta.total,
                            partial: meta.partial,
                        });
                        add_rows(&mut d.records, range.start, rows);
                        if let Some((index, row)) = revealed {
                            d.records.insert(index, row);
                            tab.top = index;
                            tab.selected = Selection::Positions(vec![index]);
                        }
                        tab.status = format!("{} 个项目", d.total);
                        if meta.partial {
                            tab.status.push_str(" · 仅部分目录可用");
                        }
                        if meta.needs_validation {
                            tab.status.push_str(" · 索引待校验");
                        }
                    }
                    Err(error) => {
                        b.stale = b.directory.is_some();
                        tab.status = if b.stale {
                            format!("无法读取目录：{error:#}（保留上次结果，F5 重试）")
                        } else {
                            format!("无法读取目录：{error:#}（F5 重试）")
                        };
                    }
                }
                drop(tabs);
                if active {
                    app.present(initial);
                    if initial {
                        app.update_menu();
                    }
                }
            },
        );
    }

    fn rebuild_tree(&self) {
        self.tree_group.set(self.tree_group.get().wrapping_add(1));
        send(self.tree, TVM_DELETEITEM, 0, TVI_ROOT.0);
        self.tree_locations.borrow_mut().clear();
        self.tree_pending.borrow_mut().clear();
        let browse = matches!(self.tabs.borrow().active().pane, Page::Browse(_));
        if !browse {
            return;
        }
        let locations = self
            .snapshot
            .borrow()
            .assets
            .iter()
            .filter(|a| matches!(a.registration.state.as_str(), "ready" | "historical"))
            .map(|a| Location {
                volume: Volume::from(a),
                parent: None,
            })
            .collect::<Vec<_>>();
        for location in locations {
            self.add_tree(TVI_ROOT, &location.volume.name.clone(), location);
        }
    }
    pub(super) fn rebuild_tree_on_refresh(&self) {
        let changing = self.changing.replace(true);
        self.rebuild_tree();
        self.changing.set(changing);
        self.update_preview();
    }
    fn add_tree(&self, parent: HTREEITEM, name: &str, location: Location) {
        let expand = self.tree_expanded(&location);
        let id = {
            let mut places = self.tree_locations.borrow_mut();
            places.push(location.clone());
            places.len() as isize
        };
        let name = HSTRING::from(name);
        let insert = TVINSERTSTRUCTW {
            hParent: parent,
            hInsertAfter: TVI_LAST,
            Anonymous: TVINSERTSTRUCTW_0 {
                item: TVITEMW {
                    mask: TVIF_TEXT | TVIF_PARAM | TVIF_CHILDREN,
                    pszText: PWSTR(name.as_ptr().cast_mut()),
                    lParam: LPARAM(id),
                    cChildren: TVITEMEXW_CHILDREN(1),
                    ..Default::default()
                },
            },
        };
        let item = send(self.tree, TVM_INSERTITEMW, 0, (&raw const insert) as isize);
        if expand && item != 0 {
            self.load_tree_branch(item, location);
        }
    }
    fn tree_expanded(&self, place: &Location) -> bool {
        let tabs = self.tabs.borrow();
        matches!(&tabs.active().pane, Page::Browse(b) if b.expanded.contains(&place.tree_key()))
    }
    fn tree_notify(&self, hdr: &NMHDR, l: LPARAM) -> Option<isize> {
        if self.changing.get() || ![TVN_SELCHANGEDW, TVN_ITEMEXPANDINGW].contains(&hdr.code) {
            return None;
        }
        // SAFETY: The listed notification codes carry an NMTREEVIEWW from this tree.
        let note = unsafe { &*(l.0 as *const NMTREEVIEWW) };
        let index = usize::try_from(note.itemNew.lParam.0)
            .ok()?
            .checked_sub(1)?;
        let place = self.tree_locations.borrow().get(index).cloned()?;
        if hdr.code == TVN_SELCHANGEDW {
            self.visit(place);
        } else {
            let expand = note.action == TVE_EXPAND;
            let collapse = note.action == TVE_COLLAPSE;
            if !expand && !collapse {
                return Some(0);
            }
            let key = place.tree_key();
            if let Page::Browse(b) = &mut self.tabs.borrow_mut().active_mut().pane {
                b.expanded.retain(|k| k != &key);
                if expand {
                    b.expanded.push(key);
                }
            }
            if expand
                && send(
                    self.tree,
                    TVM_GETNEXTITEM,
                    TVGN_CHILD as usize,
                    note.itemNew.hItem.0,
                ) == 0
            {
                self.load_tree_branch(note.itemNew.hItem.0, place);
            }
        }
        Some(0)
    }
    fn load_tree_branch(&self, item: isize, place: Location) {
        if !self.tree_pending.borrow_mut().insert(item) {
            return;
        }
        let version = self.tree_group.get();
        let request = DirectoryRequest {
            partition_id: place.volume.id.clone(),
            filesystem_id: place.volume.filesystem.clone(),
            generation: None,
            revision: None,
            parent: place.parent.clone(),
            offset: 0,
            folders_only: true,
            sort: Default::default(),
            filter: None,
        };
        self.spawn(read_directory(request, usize::MAX), move |app, result| {
            if version != app.tree_group.get() {
                return;
            }
            app.tree_pending.borrow_mut().remove(&item);
            match result {
                Ok((_, rows)) => {
                    let has_children = !rows.is_empty();
                    for row in rows {
                        app.add_tree(
                            HTREEITEM(item),
                            &row.name,
                            Location {
                                volume: place.volume.clone(),
                                parent: Some(row.entry),
                            },
                        );
                    }
                    let mut update = TVITEMW {
                        mask: TVIF_CHILDREN,
                        hItem: HTREEITEM(item),
                        cChildren: TVITEMEXW_CHILDREN(i32::from(has_children)),
                        ..Default::default()
                    };
                    send(app.tree, TVM_SETITEMW, 0, (&raw mut update) as isize);
                    if has_children && app.tree_expanded(&place) {
                        send(app.tree, TVM_EXPAND, TVE_EXPAND.0 as usize, item);
                    }
                }
                Err(error) => app.status(&format!("无法展开目录：{error:#}")),
            }
        });
    }
}

async fn directory_with_focus(
    mut request: DirectoryRequest,
    end: usize,
    focus: Option<Vec<u8>>,
) -> Result<(
    DirectoryPage,
    Vec<stashfinder_core::index::SearchRow>,
    Option<(usize, stashfinder_core::index::SearchRow)>,
)> {
    let (meta, rows) = read_directory(request.clone(), end).await?;
    let mut match_row = None;
    if let Some(focus) = focus {
        if let Some((index, row)) = rows.iter().enumerate().find(|(_, row)| row.entry == focus) {
            match_row = Some((index, row.clone()));
        } else {
            request.generation = Some(meta.generation.clone());
            request.revision = Some(meta.revision.clone());
            let mut next = meta.next;
            while let Some(offset) = next {
                request.offset = offset;
                let (page, rows) = read_directory(request.clone(), offset as usize + 96).await?;
                if let Some((index, row)) = rows
                    .into_iter()
                    .enumerate()
                    .find(|(_, row)| row.entry == focus)
                {
                    match_row = Some((offset as usize + index, row));
                    break;
                }
                next = page.next;
            }
        }
    }
    Ok((meta, rows, match_row))
}

fn search_summary(page: &SearchPage) -> String {
    let mut label = format!("{} 个结果", page.total);
    if page.groups > 0 {
        label.push_str(&format!(" · {} 个重复组", page.groups));
    }
    if page.missing > 0 {
        label.push_str(&format!(" · {} 项元数据缺失", page.missing));
    }
    if !page.unavailable.is_empty() {
        label.push_str(&format!(" · 未完成分区：{}", page.unavailable.join("、")));
    }
    label
}

fn add_rows<T>(cache: &mut Rows<T>, offset: usize, rows: Vec<T>) {
    cache.retain(|index, _| index.abs_diff(offset) < 550);
    cache.extend(
        rows.into_iter()
            .enumerate()
            .map(|(i, row)| (offset + i, row)),
    );
}

pub(super) fn directory_request(browse: &Browse, stable: bool) -> Option<DirectoryRequest> {
    let location = browse.location()?;
    let snapshot = browse.directory.as_ref().filter(|_| stable);
    Some(DirectoryRequest {
        partition_id: location.volume.id.clone(),
        filesystem_id: location.volume.filesystem.clone(),
        generation: snapshot.map(|s| s.generation.clone()),
        revision: snapshot.map(|s| s.revision.clone()),
        parent: location.parent.clone(),
        offset: 0,
        folders_only: false,
        sort: browse.sort,
        filter: browse.filter.clone().map(Box::new),
    })
}
pub(super) async fn read_directory(
    mut request: DirectoryRequest,
    end: usize,
) -> Result<(DirectoryPage, Vec<stashfinder_core::index::SearchRow>)> {
    let mut rows = Vec::new();
    loop {
        let old = request.offset;
        let Response::Directory(p) = runtime::request(Action::Directory(request.clone())).await?
        else {
            bail!("目录分页响应无效");
        };
        let mut p = *p;
        rows.append(&mut p.entries);
        match p.next {
            Some(next) if (next as usize) < end => {
                ensure!(next > old, "目录分页未前进");
                request.offset = next;
                request.generation = Some(p.generation);
                request.revision = Some(p.revision);
            }
            _ => return Ok((p, rows)),
        }
    }
}

async fn start_query(request: SearchRequest) -> Result<u64> {
    let previous = request.replaces;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    let result = loop {
        match runtime::request(Action::SearchStart(request.clone())).await {
            Ok(Response::SearchStarted(id)) => break Ok(id),
            Ok(_) => break Err(anyhow::anyhow!("搜索启动响应无效")),
            Err(error)
                if error.to_string() == SEARCH_CATALOG_LOADING
                    && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            Err(error) => break Err(error),
        }
    };
    if result.is_err()
        && let Some(id) = previous
    {
        let _ = runtime::request(Action::SearchCancel { id }).await;
    }
    result
}

async fn read_search(id: u64, range: Range<usize>) -> Result<(SearchPage, Vec<SearchHit>)> {
    let mut rows = Vec::new();
    loop {
        let offset = range.start + rows.len();
        let Response::SearchPage(p) = runtime::request(Action::SearchBatch {
            id,
            offset,
            limit: (range.end - offset).min(100),
            wait_ms: Some(900),
        })
        .await?
        else {
            bail!("搜索分页响应无效");
        };
        let mut p = *p;
        if let Some(error) = p.error.take() {
            bail!(error);
        }
        ensure!(p.id == id, "搜索快照已变化");
        if !p.done {
            continue;
        }
        rows.append(&mut p.rows);
        let end = range.start + rows.len();
        if end >= p.total.min(range.end) {
            return Ok((p, rows));
        }
        ensure!(end > offset && p.next == Some(end), "搜索分页未前进");
    }
}

pub(super) fn apply_file_filter(
    request: &mut SearchRequest,
    filter: stashfinder_core::search::FileFilter,
) {
    let options = &mut request.options;
    options.extensions = filter.extensions;
    options.video_extensions = filter.video_extensions;
    options.include_folders = filter.include_folders;
    options.kind = filter.kind;
    options.size = filter.size;
    options.created = filter.created;
    options.modified = filter.modified;
}
pub(super) fn choice_label(connection: &str) -> &str {
    match connection {
        "online" => "在线",
        "offline" => "离线",
        "changed" | "historical" => "历史记录",
        "uncertain" => "身份待确认",
        "inaccessible" => "无法访问",
        _ => "状态未确认",
    }
}
fn cell(
    row: &stashfinder_core::index::SearchRow,
    column: Option<usize>,
    search: Option<&SearchHit>,
) -> String {
    match column {
        Some(0) => {
            let Some(hit) = search else {
                return row.name.clone();
            };
            if let Some(group) = hit.duplicate_group {
                return format!(
                    "组 {group} · {}{}",
                    row.name,
                    if hit.hard_link { "（硬链接）" } else { "" }
                );
            }
            row.name.clone()
        }
        Some(1) => row.path.clone(),
        Some(2) => search.map_or(String::new(), |s| {
            format!("{} · {}", s.partition_name, choice_label(&s.connection))
        }),
        Some(3) if row.kind == "folder" => "—".into(),
        Some(3) => row.size.as_deref().unwrap_or("未知").into(),
        Some(4) => if row.kind == "folder" {
            "文件夹"
        } else {
            "文件"
        }
        .into(),
        Some(5) => file_date(row.modified.as_deref()),
        Some(6) => file_date(row.created.as_deref()),
        Some(7) if row.kind != "folder" => {
            row.name.rsplit_once('.').map_or("", |(_, ext)| ext).into()
        }
        Some(8) => format!("{:X}", row.attributes),
        Some(9) => search
            .and_then(|s| s.duplicate_group)
            .map_or("—".into(), |g| g.to_string()),
        _ => "—".into(),
    }
}
pub(super) fn file_date(ticks: Option<&str>) -> String {
    ticks
        .and_then(|t| t.parse::<i64>().ok())
        .and_then(|t| chrono::DateTime::from_timestamp(t / 10_000_000 - 11_644_473_600, 0))
        .map_or("未知".into(), |t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
}
fn directory_sort(field: SortField) -> DirectorySortField {
    match field {
        SortField::Path => DirectorySortField::Path,
        SortField::Size => DirectorySortField::Size,
        SortField::Created => DirectorySortField::Created,
        SortField::Modified => DirectorySortField::Modified,
        SortField::Extension => DirectorySortField::Extension,
        SortField::Type => DirectorySortField::Type,
        SortField::Attributes => DirectorySortField::Attributes,
        _ => DirectorySortField::Name,
    }
}
pub(super) fn set_row_state(list: HWND, index: usize, select: bool) {
    let item = LVITEMW {
        stateMask: LVIS_SELECTED,
        state: if select {
            LVIS_SELECTED
        } else {
            LIST_VIEW_ITEM_STATE_FLAGS(0)
        },
        ..Default::default()
    };
    send(list, LVM_SETITEMSTATE, index, (&raw const item) as isize);
}
