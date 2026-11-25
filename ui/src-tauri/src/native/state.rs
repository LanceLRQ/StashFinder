use crate::preferences::{FileView, Preferences};
use stashfinder_core::{
    assets::Asset,
    browse::{Ancestor, DirectorySort},
    index::{SearchHit, SearchRequest, SearchRow},
    search::{FileFilter, QueryOptions},
};
use std::collections::BTreeMap;

pub(super) type TabId = u64;
pub(super) type Rows<T> = BTreeMap<usize, T>;
pub(super) type TreeKey = (String, Option<Vec<u8>>);

#[derive(Clone)]
pub(super) struct Volume {
    pub id: String,
    pub filesystem: String,
    pub name: String,
}
impl From<&Asset> for Volume {
    fn from(asset: &Asset) -> Self {
        let r = &asset.registration;
        Self {
            id: r.partition_id.clone(),
            filesystem: r.filesystem_id.clone(),
            name: r.name.clone(),
        }
    }
}

#[derive(Clone)]
pub(super) struct Location {
    pub volume: Volume,
    pub parent: Option<Vec<u8>>,
}
impl Location {
    pub fn tree_key(&self) -> TreeKey {
        (self.volume.id.clone(), self.parent.clone())
    }
}

pub(super) struct Directory {
    pub generation: String,
    pub revision: String,
    pub location: Vec<u8>,
    pub ancestors: Vec<Ancestor>,
    pub records: Rows<SearchRow>,
    pub total: usize,
    pub partial: bool,
}

#[derive(Default)]
pub(super) struct Browse {
    pub places: Vec<Location>,
    pub cursor: usize,
    pub sort: DirectorySort,
    pub filter: Option<FileFilter>,
    pub directory: Option<Directory>,
    pub stale: bool,
    pub tree: bool,
    pub breadcrumbs: bool,
    pub reveal: Option<Vec<u8>>,
    pub expanded: Vec<TreeKey>,
}
impl Browse {
    pub fn at(location: Option<Location>, filter: Option<FileFilter>) -> Self {
        Self {
            places: location.into_iter().collect(),
            filter,
            tree: true,
            breadcrumbs: true,
            ..Self::default()
        }
    }
    pub fn location(&self) -> Option<&Location> {
        self.places.get(self.cursor)
    }
    pub fn visit(&mut self, location: Location) {
        self.places.truncate(self.cursor + 1);
        self.cursor = self.places.len();
        self.places.push(location);
        self.directory = None;
        self.stale = false;
        self.reveal = None;
    }
}

pub(super) struct Search {
    pub request: SearchRequest,
    pub originating: Option<Location>,
    pub from_bookmark: bool,
    pub id: Option<u64>,
    pub rows: Rows<SearchHit>,
    pub total: usize,
    pub groups: usize,
}
impl Search {
    pub fn new(text: String, prefs: &Preferences) -> Self {
        Self {
            request: SearchRequest {
                text,
                names_only: !prefs.include_path,
                partitions: vec![],
                filesystem_id: None,
                replaces: None,
                options: Box::new(QueryOptions {
                    online_only: prefs.search_online_only,
                    include_history: prefs.search_include_history,
                    ..QueryOptions::default()
                }),
            },
            originating: None,
            from_bookmark: false,
            id: None,
            rows: Rows::new(),
            total: 0,
            groups: 0,
        }
    }
}

pub(super) enum Page {
    Overview,
    Browse(Box<Browse>),
    Search(Box<Search>),
    Tasks,
}

#[derive(Default)]
pub(super) enum Selection {
    #[default]
    None,
    All,
    Positions(Vec<usize>),
}

pub(super) struct Tab {
    pub id: TabId,
    pub origin: TabId,
    pub pane: Page,
    pub epoch: u64,
    pub pending: bool,
    pub index_dirty: bool,
    pub status: String,
    pub top: usize,
    pub selected: Selection,
    pub layout: FileView,
    pub preview: bool,
}
impl Tab {
    pub fn title(&self) -> String {
        match &self.pane {
            Page::Overview => "概览".into(),
            Page::Tasks => "任务".into(),
            Page::Browse(b) => b
                .location()
                .map_or("浏览".into(), |p| p.volume.name.clone()),
            Page::Search(s) if s.request.text.trim().is_empty() => "搜索".into(),
            Page::Search(s) => s.request.text.chars().take(28).collect(),
        }
    }
}

pub(super) struct Tabs {
    pub items: Vec<Tab>,
    pub active: usize,
    next_id: TabId,
}
impl Tabs {
    pub fn new(prefs: &Preferences) -> Self {
        Self {
            items: vec![Self::tab(0, 0, Page::Overview, prefs)],
            active: 0,
            next_id: 1,
        }
    }
    fn tab(id: TabId, origin: TabId, pane: Page, prefs: &Preferences) -> Tab {
        Tab {
            id,
            origin,
            pane,
            epoch: 0,
            pending: false,
            index_dirty: false,
            status: "".into(),
            top: 0,
            selected: Selection::None,
            layout: prefs.default_view.clone(),
            preview: prefs.default_preview,
        }
    }
    pub fn add(&mut self, page: Page, prefs: &Preferences) -> TabId {
        if matches!(page, Page::Tasks)
            && let Some(i) = self
                .items
                .iter()
                .position(|t| matches!(t.pane, Page::Tasks))
        {
            self.active = i;
            return self.items[i].id;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.items
            .push(Self::tab(id, self.active().id, page, prefs));
        self.active = self.items.len() - 1;
        id
    }
    pub fn active(&self) -> &Tab {
        &self.items[self.active]
    }
    pub fn active_mut(&mut self) -> &mut Tab {
        &mut self.items[self.active]
    }
    pub fn get(&mut self, id: TabId) -> Option<&mut Tab> {
        self.items.iter_mut().find(|t| t.id == id)
    }
    pub fn close(&mut self, index: usize) -> Option<Tab> {
        if index == 0 || index >= self.items.len() {
            return None;
        }
        let removed = self.items.remove(index);
        if self.active > index {
            self.active -= 1;
        } else if self.active == index {
            self.active = self
                .items
                .iter()
                .position(|t| t.id == removed.origin)
                .unwrap_or_else(|| index.min(self.items.len() - 1));
        }
        Some(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_keep_independent_query_settings_and_reveal_the_search_origin_on_close() {
        let prefs = Preferences::default();
        let mut tabs = Tabs::new(&prefs);
        let browse = tabs.add(Page::Browse(Box::default()), &prefs);
        let first = tabs.add(
            Page::Search(Box::new(Search::new("照片".into(), &prefs))),
            &prefs,
        );
        let second = tabs.add(
            Page::Search(Box::new(Search::new("报告".into(), &prefs))),
            &prefs,
        );
        assert!(tabs.close(0).is_none());
        assert_ne!(first, second);
        assert_eq!(tabs.active().title(), "报告");
        assert!(tabs.close(tabs.active).is_some());
        assert_eq!(tabs.active().id, first);
        assert_eq!(tabs.active().title(), "照片");
        assert!(tabs.close(tabs.active).is_some());
        assert_eq!(tabs.active().id, browse);
    }
}
