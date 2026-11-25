use super::*;
use crate::preferences::{
    BookmarkScope, BookmarkedSearch, Category, FileView, FilterPreset, SearchBookmark,
};
use anyhow::Context;
use stashfinder_core::search::{
    DuplicateField, FileFilter, MatchMode, QueryOptions, Range, Scope, SortField,
    normalize_extensions,
};

const MATCHES: &[(MatchMode, &str)] = &[
    (MatchMode::Contains, "包含"),
    (MatchMode::WholeName, "完整文件名"),
    (MatchMode::WholeWord, "完整单词"),
    (MatchMode::Start, "文件名开头"),
    (MatchMode::End, "文件名末尾"),
    (MatchMode::Wildcard, "通配符"),
    (MatchMode::Regex, "正则表达式"),
];
const DUPLICATES: &[(&[DuplicateField], &str)] = &[
    (&[], "不限"),
    (&[DuplicateField::Name], "同名"),
    (&[DuplicateField::Size], "同大小"),
    (
        &[DuplicateField::Name, DuplicateField::Size],
        "同名且同大小",
    ),
    (&[DuplicateField::Modified], "同修改时间"),
    (
        &[
            DuplicateField::Name,
            DuplicateField::Size,
            DuplicateField::Modified,
        ],
        "同名、大小和修改时间",
    ),
];
const SORTS: &[(SortField, &str)] = &[
    (SortField::Name, "名称"),
    (SortField::Path, "路径"),
    (SortField::Partition, "分区"),
    (SortField::Size, "大小"),
    (SortField::Modified, "修改时间"),
    (SortField::Created, "创建时间"),
    (SortField::Type, "类型"),
    (SortField::Extension, "扩展名"),
    (SortField::Attributes, "属性"),
    (SortField::Group, "重复组"),
];
const LAYOUTS: &[(FileView, &str)] = &[
    (FileView::Details, "详情"),
    (FileView::Small, "小图标"),
    (FileView::Medium, "中图标"),
    (FileView::Large, "大图标"),
];

fn choice<T: PartialEq>(label: &'static str, options: &[(T, &str)], current: &T) -> Field {
    let index = options
        .iter()
        .position(|(value, _)| value == current)
        .unwrap_or(0);
    Field::Choice(
        label,
        options.iter().map(|(_, label)| (*label).into()).collect(),
        index,
    )
}
fn items(label: &'static str, options: &[&str]) -> Field {
    Field::Choice(
        label,
        options.iter().map(|label| (*label).into()).collect(),
        0,
    )
}
fn entries(prefs: &Preferences) -> Vec<(Option<String>, String, FileFilter)> {
    let mut values = vec![
        (None, "无筛选".into(), FileFilter::default()),
        (
            Some("@folder".into()),
            "文件夹".into(),
            FileFilter {
                kind: Some("folder".into()),
                ..Default::default()
            },
        ),
    ];
    values.extend(prefs.categories.iter().map(|cat| {
        let rule = if cat.name == "视频" {
            FileFilter {
                video_extensions: cat.extensions.clone(),
                ..Default::default()
            }
        } else {
            FileFilter {
                extensions: cat.extensions.clone(),
                ..Default::default()
            }
        };
        (
            Some(format!("category:{}", cat.name)),
            cat.name.clone(),
            rule,
        )
    }));
    values.extend(prefs.filter_presets.iter().map(|p| {
        (
            Some(p.id.clone()),
            format!("个人：{}", p.name),
            p.rule.clone(),
        )
    }));
    values
}
pub(super) fn extensions(input: &str) -> Result<Vec<String>> {
    normalize_extensions(
        &input
            .split([',', '，', ';', '\n'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect::<Vec<_>>(),
    )
}
fn optional(input: String) -> Option<String> {
    let value = input.trim();
    (!value.is_empty()).then(|| value.into())
}
fn date_input(value: Option<&str>) -> String {
    value
        .and_then(|v| v.parse::<i64>().ok())
        .and_then(|ticks| chrono::DateTime::from_timestamp(ticks / 10_000_000 - 11_644_473_600, 0))
        .map_or_else(String::new, |t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d")
                .to_string()
        })
}
fn date_boundary(value: String, end: bool) -> Result<Option<String>> {
    use chrono::{NaiveDate, TimeZone};
    let Some(input) = optional(value) else {
        return Ok(None);
    };
    let mut date =
        NaiveDate::parse_from_str(&input, "%Y-%m-%d").context("日期格式为 YYYY-MM-DD")?;
    if end {
        date = date.succ_opt().context("结束日期超出范围")?;
    }
    let local = chrono::Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).context("日期不受支持")?)
        .earliest()
        .context("本地日期不受支持")?;
    let ticks = (local.timestamp() + 11_644_473_600)
        .checked_mul(10_000_000)
        .context("日期超出范围")?
        - i64::from(end);
    ensure!(ticks >= 0, "日期必须在 1601 年以后");
    Ok(Some(ticks.to_string()))
}
fn filter_from(options: &QueryOptions) -> FileFilter {
    FileFilter {
        extensions: options.extensions.clone(),
        video_extensions: options.video_extensions.clone(),
        include_folders: options.include_folders,
        kind: options.kind.clone(),
        size: options.size.clone(),
        created: options.created.clone(),
        modified: options.modified.clone(),
    }
}
fn next_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let ticks = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}{ticks:x}-{:x}-{sequence:x}", std::process::id())
}

impl Desktop {
    pub(super) fn ask(&self, title: &str, fields: &[Field]) -> Option<Values> {
        match form(self.hwnd, title, fields) {
            Ok(fields) => fields,
            Err(e) => {
                self.status(&format!("{title}：{e:#}"));
                None
            }
        }
    }
    pub(super) fn save_preferences(&self, preferences: Preferences) -> bool {
        match preferences::write(&self.config_path, &preferences)
            .and_then(|_| preferences::read(&self.config_path))
        {
            Ok(saved) => {
                *self.preferences.borrow_mut() = saved;
                true
            }
            Err(e) => {
                info(self.hwnd, "设置未保存", &format!("{e:#}"));
                false
            }
        }
    }
    pub(super) fn default_filter(&self) -> Option<FileFilter> {
        let p = self.preferences.borrow();
        let id = p.browse_filter.as_ref()?;
        entries(&p)
            .into_iter()
            .find_map(|(key, _, rule)| (key.as_ref() == Some(id)).then_some(rule))
    }
    pub(super) fn quick_filter(&self) {
        let all = entries(&self.preferences.borrow());
        let names = all.iter().map(|(_, name, _)| name.clone()).collect();
        let Some(mut fields) = self.ask("应用筛选预设", &[Field::Choice("筛选预设", names, 0)])
        else {
            return;
        };
        let rule = all[fields.choice()].2.clone();
        let mut tabs = self.tabs.borrow_mut();
        match &mut tabs.active_mut().pane {
            Page::Browse(b) => b.filter = (rule != FileFilter::default()).then_some(rule),
            Page::Search(s) => files::apply_file_filter(&mut s.request, rule),
            _ => return,
        }
        drop(tabs);
        self.refresh_current();
    }
    pub(super) fn filter_dialog(&self) {
        let (mut rule, mut query) = {
            let tab = self.tabs.borrow();
            match &tab.active().pane {
                Page::Search(s) => (filter_from(&s.request.options), Some(s.request.clone())),
                Page::Browse(b) => (b.filter.clone().unwrap_or_default(), None),
                _ => return,
            }
        };
        let presets = entries(&self.preferences.borrow());
        let mut names = vec!["保持当前".into()];
        names.extend(presets.iter().map(|(_, name, _)| name.clone()));
        let Some(mut first) = self.ask(
            "筛选器",
            &[
                Field::Choice("筛选预设", names, 0),
                items(
                    "操作",
                    &[
                        "编辑条件并应用",
                        "直接应用",
                        "清除筛选",
                        "另存当前筛选为预设",
                    ],
                ),
            ],
        ) else {
            return;
        };
        let choice = first.choice();
        let command = first.choice();
        if let Some((_, _, selected)) = choice.checked_sub(1).and_then(|i| presets.get(i)) {
            rule = selected.clone();
        }
        match command {
            0 => {
                let Some((result, changes)) = self.filter_fields(
                    &rule,
                    query.as_ref().map(|r| (r.names_only, r.options.as_ref())),
                ) else {
                    return;
                };
                rule = result;
                if let Some((names, options)) = changes
                    && let Some(query) = query.as_mut()
                {
                    query.names_only = names;
                    *query.options = options;
                }
            }
            2 => {
                rule = FileFilter::default();
                if let Some(q) = &mut query {
                    q.options.duplicate.clear();
                    if q.options.sort == SortField::Group {
                        q.options.sort = SortField::Name;
                    }
                }
            }
            3 => {
                let Some(mut value) = self.ask(
                    "将当前筛选另存为预设",
                    &[Field::Edit("预设名称", "".into())],
                ) else {
                    return;
                };
                let mut prefs = self.preferences.borrow().clone();
                prefs.filter_presets.push(FilterPreset {
                    id: next_id("user:"),
                    name: value.text(),
                    rule: rule.clone(),
                });
                if !self.save_preferences(prefs) {
                    return;
                }
            }
            _ => {}
        }
        let mut tabs = self.tabs.borrow_mut();
        match (&mut tabs.active_mut().pane, query) {
            (Page::Search(s), Some(mut request)) => {
                files::apply_file_filter(&mut request, rule);
                s.request = request;
            }
            (Page::Browse(b), _) => b.filter = (rule != FileFilter::default()).then_some(rule),
            _ => return,
        }
        drop(tabs);
        self.refresh_current();
    }
    fn filter_fields(
        &self,
        rule: &FileFilter,
        options: Option<(bool, &QueryOptions)>,
    ) -> Option<(FileFilter, Option<(bool, QueryOptions)>)> {
        let kind = if rule.include_folders {
            3
        } else {
            match rule.kind.as_deref() {
                Some("file") => 1,
                Some("folder") => 2,
                _ => 0,
            }
        };
        let mut fields = vec![
            Field::Edit("扩展名（逗号分隔）", rule.extensions.join(",")),
            Field::Edit("视频扩展名", rule.video_extensions.join(",")),
            Field::Choice(
                "文件类型",
                ["不限", "仅文件", "仅文件夹", "同时保留文件夹"]
                    .map(str::to_string)
                    .to_vec(),
                kind,
            ),
            Field::Edit("最小字节数", rule.size.min.clone().unwrap_or_default()),
            Field::Edit("最大字节数", rule.size.max.clone().unwrap_or_default()),
            Field::Edit(
                "修改起始 YYYY-MM-DD",
                date_input(rule.modified.min.as_deref()),
            ),
            Field::Edit(
                "修改结束 YYYY-MM-DD",
                date_input(rule.modified.max.as_deref()),
            ),
            Field::Edit(
                "创建起始 YYYY-MM-DD",
                date_input(rule.created.min.as_deref()),
            ),
            Field::Edit(
                "创建结束 YYYY-MM-DD",
                date_input(rule.created.max.as_deref()),
            ),
        ];
        if let Some((only, options)) = options {
            fields.extend([
                Field::Check("仅文件名", only),
                choice("匹配方式", MATCHES, &options.mode),
                Field::Check("区分大小写", options.case_sensitive),
                choice("重复项", DUPLICATES, &options.duplicate.as_slice()),
                Field::Check("仅在线", options.online_only),
                Field::Check("包含历史记录", options.include_history),
                choice("搜索排序", SORTS, &options.sort),
                Field::Check("降序", options.descending),
            ]);
        }
        let mut values = self.ask("筛选条件", &fields)?;
        let parsed = (|| -> Result<FileFilter> {
            let extensions = extensions(&values.text())?;
            let video_extensions = self::extensions(&values.text())?;
            let kind = values.choice();
            let size = Range {
                min: optional(values.text()),
                max: optional(values.text()),
            };
            let modified = Range {
                min: date_boundary(values.text(), false)?,
                max: date_boundary(values.text(), true)?,
            };
            let created = Range {
                min: date_boundary(values.text(), false)?,
                max: date_boundary(values.text(), true)?,
            };
            let rule = FileFilter {
                extensions,
                video_extensions,
                include_folders: kind == 3,
                kind: [None, Some("file".into()), Some("folder".into()), None][kind].clone(),
                size,
                modified,
                created,
            };
            rule.compile()?;
            Ok(rule)
        })();
        match parsed {
            Ok(rule) => {
                let changes = options.map(|(_, previous)| {
                    let names_only = values.check();
                    let mode = MATCHES[values.choice()].0;
                    let case_sensitive = values.check();
                    let duplicate = DUPLICATES[values.choice()].0.to_vec();
                    let online_only = values.check();
                    let include_history = values.check();
                    let mut sort = SORTS[values.choice()].0;
                    if !duplicate.is_empty()
                        && duplicate != previous.duplicate
                        && sort == previous.sort
                    {
                        sort = SortField::Group;
                    }
                    if duplicate.is_empty() && sort == SortField::Group {
                        sort = SortField::Name;
                    }
                    (
                        names_only,
                        QueryOptions {
                            mode,
                            case_sensitive,
                            duplicate,
                            online_only,
                            include_history,
                            sort,
                            descending: values.check(),
                            ..previous.clone()
                        },
                    )
                });
                Some((rule, changes))
            }
            Err(e) => {
                info(self.hwnd, "筛选条件无效", &format!("{e:#}"));
                None
            }
        }
    }
    pub(super) fn manage_presets(&self) {
        let mut prefs = self.preferences.borrow().clone();
        let mut names = vec!["新建预设…".into()];
        names.extend(prefs.filter_presets.iter().map(|p| p.name.clone()));
        let Some(mut menu) = self.ask(
            "管理筛选预设",
            &[
                Field::Choice("预设", names, 0),
                items("操作", &["新建或编辑", "移除预设"]),
            ],
        ) else {
            return;
        };
        let index = menu.choice().checked_sub(1);
        if menu.choice() == 1 {
            let Some(i) = index else {
                return;
            };
            if !confirm(
                self.hwnd,
                "移除预设",
                &format!("移除「{}」？", prefs.filter_presets[i].name),
            ) {
                return;
            }
            prefs.filter_presets.remove(i);
        } else {
            let previous = index.map(|i| &prefs.filter_presets[i]);
            let Some(mut name) = self.ask(
                "编辑筛选预设",
                &[Field::Edit(
                    "预设名称",
                    previous.map(|p| p.name.clone()).unwrap_or_default(),
                )],
            ) else {
                return;
            };
            let empty = FileFilter::default();
            let Some((rule, _)) = self.filter_fields(previous.map_or(&empty, |p| &p.rule), None)
            else {
                return;
            };
            let preset = FilterPreset {
                id: previous
                    .map(|p| p.id.clone())
                    .unwrap_or_else(|| next_id("user:")),
                name: name.text(),
                rule,
            };
            if let Some(i) = index {
                prefs.filter_presets[i] = preset;
            } else {
                prefs.filter_presets.push(preset);
            }
        }
        self.save_preferences(prefs);
    }
    pub(super) fn save_bookmark(&self) {
        let (r, title) = {
            let tabs = self.tabs.borrow();
            let Page::Search(s) = &tabs.active().pane else {
                return;
            };
            (s.request.clone(), tabs.active().title())
        };
        let Some(mut value) = self.ask("添加当前搜索", &[Field::Edit("书签名称", title)])
        else {
            return;
        };
        let scope = r
            .partitions
            .first()
            .zip(r.filesystem_id.as_ref())
            .map(|(id, fs)| BookmarkScope {
                partition_id: id.clone(),
                filesystem_id: fs.clone(),
                entry: r.options.scope.as_ref().map(|s| s.entry.clone()),
            });
        let mut options = *r.options;
        options.scope = None;
        let mut prefs = self.preferences.borrow().clone();
        prefs.search_bookmarks.push(SearchBookmark {
            id: next_id("bookmark-"),
            name: value.text(),
            query: BookmarkedSearch {
                text: r.text,
                names_only: r.names_only,
                options,
                scope,
            },
        });
        if self.save_preferences(prefs) {
            self.status("已添加搜索书签");
        }
    }
    pub(super) fn bookmarks(&self, manage: bool) {
        let mut prefs = self.preferences.borrow().clone();
        if prefs.search_bookmarks.is_empty() {
            info(self.hwnd, "搜索书签", "暂无书签。");
            return;
        }
        let names = prefs
            .search_bookmarks
            .iter()
            .map(|b| b.name.clone())
            .collect();
        let mut fields = vec![Field::Choice("书签", names, 0)];
        if manage {
            fields.push(items("操作", &["重命名", "移除", "打开"]));
        }
        let Some(mut answer) = self.ask(
            if manage {
                "管理搜索书签"
            } else {
                "打开搜索书签"
            },
            &fields,
        ) else {
            return;
        };
        let i = answer.choice();
        match if manage { answer.choice() } else { 2 } {
            0 => {
                let Some(mut name) = self.ask(
                    "重命名搜索书签",
                    &[Field::Edit("名称", prefs.search_bookmarks[i].name.clone())],
                ) else {
                    return;
                };
                prefs.search_bookmarks[i].name = name.text();
                self.save_preferences(prefs);
            }
            1 => {
                if confirm(
                    self.hwnd,
                    "移除书签",
                    &format!("移除「{}」？", prefs.search_bookmarks[i].name),
                ) {
                    prefs.search_bookmarks.remove(i);
                    self.save_preferences(prefs);
                }
            }
            _ => {
                let query = prefs.search_bookmarks[i].query.clone();
                let mut search = Search::new(query.text, &prefs);
                search.request.names_only = query.names_only;
                search.request.options = Box::new(query.options);
                if let Some(scope) = query.scope {
                    let found = self
                        .snapshot
                        .borrow()
                        .assets
                        .iter()
                        .find(|a| {
                            a.registration.partition_id == scope.partition_id
                                && a.registration.filesystem_id == scope.filesystem_id
                        })
                        .map(Volume::from);
                    let Some(volume) = found else {
                        info(
                            self.hwnd,
                            "书签范围不可用",
                            "书签分区已移除或文件系统身份已变化。",
                        );
                        return;
                    };
                    search.from_bookmark = true;
                    search.originating = Some(Location {
                        volume,
                        parent: scope.entry.clone(),
                    });
                    search.request.partitions = vec![scope.partition_id.clone()];
                    search.request.filesystem_id = Some(scope.filesystem_id);
                    search.request.options.scope = scope.entry.map(|entry| Scope {
                        partition_id: scope.partition_id,
                        entry,
                        recursive: true,
                    });
                }
                self.open_tab(Page::Search(Box::new(search)));
                focus(self.query);
            }
        }
    }

    pub(super) fn settings(&self) {
        let Some(mut fields) = self.ask(
            "设置",
            &[items(
                "设置分类",
                &[
                    "常用",
                    "文件分类",
                    "筛选预设",
                    "快捷键",
                    "索引与同步",
                    "图片缓存",
                    "关于与诊断",
                ],
            )],
        ) else {
            return;
        };
        match fields.choice() {
            0 => self.general(),
            1 => self.categories(),
            2 => self.manage_presets(),
            3 => self.shortcuts(),
            4 => self.sync_settings(),
            5 => self.cache_settings(),
            _ => self.about(),
        }
    }
    fn general(&self) {
        let mut prefs = self.preferences.borrow().clone();
        let choices = entries(&prefs);
        let selected = choices
            .iter()
            .position(|(id, _, _)| id == &prefs.browse_filter)
            .unwrap_or(0);
        let Some(mut fields) = self.ask(
            "设置 · 常用",
            &[
                Field::Check("默认包含路径", prefs.include_path),
                Field::Check("新搜索默认仅在线", prefs.search_online_only),
                Field::Check("新搜索默认包含历史记录", prefs.search_include_history),
                Field::Choice(
                    "浏览与新搜索默认筛选",
                    choices.iter().map(|(_, name, _)| name.clone()).collect(),
                    selected,
                ),
                Field::Check("显示状态栏", prefs.show_status_bar),
                Field::Check("新标签默认显示预览窗格", prefs.default_preview),
                choice("新标签默认布局", LAYOUTS, &prefs.default_view),
                Field::Check("恢复常用默认设置", false),
            ],
        ) else {
            return;
        };
        prefs.include_path = fields.check();
        prefs.search_online_only = fields.check();
        prefs.search_include_history = fields.check();
        prefs.browse_filter = choices[fields.choice()].0.clone();
        prefs.show_status_bar = fields.check();
        prefs.default_preview = fields.check();
        prefs.default_view = LAYOUTS[fields.choice()].0.clone();
        if fields.check() {
            let defaults = Preferences::default();
            prefs = Preferences {
                categories: prefs.categories,
                filter_presets: prefs.filter_presets,
                search_bookmarks: prefs.search_bookmarks,
                collapsed: prefs.collapsed,
                start_page: prefs.start_page,
                ..defaults
            };
        }
        self.save_preferences(prefs);
        self.layout();
    }
    fn categories(&self) {
        let mut prefs = self.preferences.borrow().clone();
        let mut names = vec!["新增分类…".into()];
        names.extend(
            prefs
                .categories
                .iter()
                .map(|c| format!("{} · {}", c.name, c.extensions.join(","))),
        );
        let Some(mut menu) = self.ask(
            "文件分类",
            &[
                Field::Choice("分类", names, 0),
                items("操作", &["新增或编辑", "移除分类", "恢复默认分类"]),
            ],
        ) else {
            return;
        };
        let index = menu.choice().checked_sub(1);
        match menu.choice() {
            0 => {
                let old = index.map(|i| &prefs.categories[i]);
                let Some(mut data) = self.ask(
                    "编辑文件分类",
                    &[
                        Field::Edit("分类名称", old.map(|c| c.name.clone()).unwrap_or_default()),
                        Field::Edit(
                            "扩展名",
                            old.map(|c| c.extensions.join(",")).unwrap_or_default(),
                        ),
                    ],
                ) else {
                    return;
                };
                let name = data.text();
                let ext = match extensions(&data.text()) {
                    Ok(ext) => ext,
                    Err(e) => {
                        info(self.hwnd, "扩展名无效", &e.to_string());
                        return;
                    }
                };
                let category = Category {
                    name,
                    extensions: ext,
                };
                if let Some(i) = index {
                    prefs.categories[i] = category;
                } else {
                    prefs.categories.push(category);
                }
            }
            1 => {
                let Some(i) = index else {
                    return;
                };
                if !confirm(
                    self.hwnd,
                    "移除分类",
                    &format!("移除「{}」？", prefs.categories[i].name),
                ) {
                    return;
                }
                prefs.categories.remove(i);
            }
            _ => prefs.categories = Preferences::default().categories,
        }
        self.save_preferences(prefs);
    }
    fn sync_settings(&self) {
        self.spawn(
            async {
                runtime::request(Action::TaskOverview {
                    version: None,
                    offset: 0,
                })
                .await
            },
            |app, result| match result {
                Ok(Response::TaskOverview(p)) => {
                    let Some(mut fields) = app.ask(
                        "设置 · 索引与同步",
                        &[Field::Check("自动更新已登记分区", p.automatic)],
                    ) else {
                        return;
                    };
                    let enabled = fields.check();
                    if enabled == p.automatic {
                        return;
                    }
                    if !enabled
                        && !confirm(
                            app.hwnd,
                            "关闭自动更新",
                            "文件变化将不再自动进入索引。已有任务会继续，离线记录仍可搜索。",
                        )
                    {
                        return;
                    }
                    app.spawn(
                        library::accepted(Action::AutoUpdate { enabled }),
                        move |app, result| match result {
                            Ok(()) => {
                                app.status("已保存自动更新设置");
                                app.refresh_tasks();
                            }
                            Err(e) => info(app.hwnd, "自动更新设置未保存", &format!("{e:#}")),
                        },
                    );
                }
                Err(e) => info(app.hwnd, "自动更新状态未知", &format!("{e:#}")),
                _ => info(app.hwnd, "自动更新状态未知", "后台返回了无法识别的响应。"),
            },
        );
    }
    pub(super) fn about(&self) {
        self.spawn(runtime::status(), |app, result| {
            let server = match result {
                Ok(s) => format!(
                    "后台模式：{}\n后台 PID：{}\n协议：{}\n已登记分区：{}",
                    s.mode, s.process_id, s.version, s.registered_partitions
                ),
                Err(e) => format!("后台连接失败：{e:#}\n关闭后可从「帮助 → 关于与诊断」重试。"),
            };
            let mut fields = vec![Field::Label(format!(
                "StashFinder {} · Windows 原生工作区",
                env!("CARGO_PKG_VERSION")
            ))];
            fields.extend(server.lines().map(|line| Field::Label(line.into())));
            if app.tray {
                fields.push(Field::Check("退出便携版", false));
            }
            if let Some(mut values) = app.ask("关于与诊断", &fields)
                && app.tray
                && values.check()
            {
                app.close(true);
            }
        });
    }
    pub(super) fn shortcuts(&self) {
        info(
            self.hwnd,
            "快捷键",
            "Ctrl+T  新建浏览标签页\nCtrl+Shift+F  新建搜索标签页\nCtrl+F  聚焦搜索框\nCtrl+Tab / Ctrl+Shift+Tab  切换标签\nCtrl+W  关闭标签\nF5  刷新\nCtrl+P  设置\nCtrl+Q  退出\n\nAlt+← / → / ↑  后退、前进、上一级\nAlt+P  预览窗格\nCtrl+Shift+6 / 4 / 3 / 2  详情、小、中、大图标\nCtrl+数字键盘加号  自动列宽\n\nCtrl+I / B / U / R  大小写、完整单词、路径、正则\nCtrl+S  导出搜索结果\nCtrl+D  添加书签\nF1  搜索语法\n\nCtrl+A  全选\nCtrl+C / X / V  复制、剪切、粘贴\nCtrl+Shift+C  复制路径\nF2  重命名\nDelete / Shift+Delete  回收或永久删除\nAlt+Enter  属性\nShift+F10  右键菜单",
        );
    }
    pub(super) fn help(&self) {
        info(
            self.hwnd,
            "搜索语法",
            "默认包含已登记的在线与离线分区。可在筛选器中设置范围。\n\nreport backup    同时包含两个词\n\"annual report\"    连续短语\n<report|invoice> !draft    或与排除条件\n*.pdf    通配符\npath:projects    匹配路径\nparent:projects    只匹配该父目录的直接子项\nancestor:projects    匹配目录及子目录\nfolder:    仅文件夹\next:pdf;docx size:>=1MiB    扩展名和大小\n dm:today    今天修改的记录\nregex:\"(?=report).*\\.pdf$\"    PCRE2 正则\n dupe:name;size    按元数据匹配重复候选",
        );
    }
    pub(super) fn update_menu(&self) {
        self.task_buttons();
        let (
            home,
            browse,
            search,
            tasks,
            view,
            preview,
            tree,
            crumbs,
            back,
            next,
            up,
            current,
            options,
        ) = {
            let tabs = self.tabs.borrow();
            let tab = tabs.active();
            let (home, browse, search, tasks, tree, crumbs, back, next, up, options) =
                match &tab.pane {
                    Page::Overview => (
                        true, false, false, false, false, false, false, false, false, None,
                    ),
                    Page::Tasks => (
                        false, false, false, true, false, false, false, false, false, None,
                    ),
                    Page::Browse(b) => (
                        false,
                        true,
                        false,
                        false,
                        b.tree,
                        b.breadcrumbs,
                        b.cursor > 0,
                        b.cursor + 1 < b.places.len(),
                        b.location().is_some_and(|l| l.parent.is_some()),
                        None,
                    ),
                    Page::Search(s) => (
                        false,
                        false,
                        true,
                        false,
                        false,
                        false,
                        false,
                        false,
                        false,
                        Some((
                            s.request.names_only,
                            s.request.options.case_sensitive,
                            s.request.options.mode,
                        )),
                    ),
                };
            (
                home,
                browse,
                search,
                tasks,
                tab.layout.clone(),
                tab.preview,
                tree,
                crumbs,
                back,
                next,
                up,
                tabs.active != 0,
                options,
            )
        };
        let file = browse || search;
        enable(self.back, back);
        enable(self.forward, next);
        enable(self.up, up);
        enable(self.close_tab, current);
        // SAFETY: The command IDs refer to the native menu owned by the workspace.
        unsafe {
            let menu = GetMenu(self.hwnd);
            for (id, show) in [
                (CLOSE_TAB, current),
                (FILTER, file),
                (FOCUS_SEARCH, !tasks),
                (PICK_VOLUME, browse || home),
                (PREVIEW, file),
                (TREE, browse),
                (BREADCRUMBS, browse),
                (BACK, back),
                (FORWARD, next),
                (UP, up),
                (files::MATCH_PATH, search),
                (files::MATCH_CASE, search),
                (files::MATCH_REGEX, search),
                (files::MATCH_WORD, search),
                (files::EXPORT, search),
                (files::BOOKMARK, search),
                (files::COLUMNS, file),
                (files::SORT, file),
                (files::DETAILS, file),
                (files::SMALL, file),
                (files::MEDIUM, file),
                (files::LARGE, file),
            ]
            .into_iter()
            .chain(files::EDIT_MENU.iter().map(|(id, _)| {
                (
                    *id,
                    file && (self.may_operate(*id) || !(*id >= files::OPEN && *id <= files::MORE)),
                )
            })) {
                let _ = EnableMenuItem(
                    menu,
                    id as u32,
                    MF_BYCOMMAND | if show { MF_ENABLED } else { MF_GRAYED },
                );
            }
            for (id, check) in [
                (STATUS, self.preferences.borrow().show_status_bar),
                (PIN, self.pinned.get()),
                (PREVIEW, preview),
                (TREE, tree),
                (BREADCRUMBS, crumbs),
                (files::DETAILS, view == FileView::Details),
                (files::SMALL, view == FileView::Small),
                (files::MEDIUM, view == FileView::Medium),
                (files::LARGE, view == FileView::Large),
                (
                    files::MATCH_PATH,
                    options.is_some_and(|(names, _, _)| !names),
                ),
                (files::MATCH_CASE, options.is_some_and(|(_, case, _)| case)),
                (
                    files::MATCH_REGEX,
                    options.is_some_and(|(_, _, mode)| mode == MatchMode::Regex),
                ),
                (
                    files::MATCH_WORD,
                    options.is_some_and(|(_, _, mode)| mode == MatchMode::WholeWord),
                ),
            ] {
                CheckMenuItem(
                    menu,
                    id as u32,
                    (MF_BYCOMMAND | if check { MF_CHECKED } else { MF_UNCHECKED }).0,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_date_filter_round_trips_an_inclusive_local_day() {
        let day = "2026-11-25";
        let min = date_boundary(day.into(), false).unwrap().unwrap();
        let max = date_boundary(day.into(), true).unwrap().unwrap();
        assert_eq!(date_input(Some(&min)), day);
        assert_eq!(date_input(Some(&max)), day);
        assert_eq!(
            max.parse::<i64>().unwrap() - min.parse::<i64>().unwrap(),
            864_000_000_000 - 1
        );
        assert!(date_boundary("11/25/2026".into(), true).is_err());
    }
}
