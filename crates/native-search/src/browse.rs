use crate::Endpoint;
use crate::registration;
use anyhow::{Result, bail, ensure};
use stashfinder_core::{
    Action, Response,
    assets::{Asset, Snapshot},
    browse::{DirectoryPage, DirectoryRequest},
};
use stashfinder_windows::ipc;

#[derive(Clone)]
pub(super) enum Location {
    Partitions,
    Candidates,
    Folder {
        name: String,
        request: Box<DirectoryRequest>,
        previous: Vec<u32>,
    },
}

pub(super) enum Listing {
    Partitions(Snapshot),
    Candidates(Snapshot),
    Folder(Box<DirectoryPage>),
}

pub(super) struct Browser {
    pub location: Location,
    pub listing: Option<Listing>,
    pub restore_search: bool,
}

impl Location {
    pub fn is_partitions(&self) -> bool {
        matches!(self, Self::Partitions)
    }

    pub async fn load(&self, endpoint: &Endpoint, refresh: bool) -> Result<Listing> {
        endpoint.ready().await?;
        let pipe = endpoint.pipe();
        match self {
            Self::Candidates => {
                let mut snapshot = registration::idle_snapshot(pipe).await?;
                if refresh {
                    registration::accepted(pipe, Action::Refresh).await?;
                    snapshot = registration::idle_snapshot(pipe).await?;
                }
                Ok(Listing::Candidates(snapshot))
            }
            Self::Partitions => {
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
                loop {
                    let mut snapshot = ipc::snapshot(pipe).await?;
                    if !snapshot.busy || !snapshot.assets.is_empty() || snapshot.error.is_some() {
                        snapshot.assets.sort_by(|a, b| {
                            a.registration
                                .disk_id
                                .cmp(&b.registration.disk_id)
                                .then_with(|| a.registration.name.cmp(&b.registration.name))
                        });
                        return Ok(Listing::Partitions(snapshot));
                    }
                    ensure!(
                        tokio::time::Instant::now() < deadline,
                        "分区目录仍在读取，请刷新"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                }
            }
            Self::Folder { request, .. } => {
                let mut query = (**request).clone();
                if refresh {
                    query.offset = 0;
                    let parent = query.parent.take();
                    let previous_generation = query.generation.take();
                    query.revision = None;
                    let root = read_directory(pipe, &query).await?;
                    // Entry IDs belong to a generation. Do not reuse one after a rebuild.
                    if parent.is_none() || previous_generation.as_ref() != Some(&root.generation) {
                        return Ok(Listing::Folder(root));
                    }
                    query.parent = parent;
                    query.generation = Some(root.generation);
                    query.revision = Some(root.revision);
                }
                Ok(Listing::Folder(read_directory(pipe, &query).await?))
            }
        }
    }
}

async fn read_directory(pipe: &str, request: &DirectoryRequest) -> Result<Box<DirectoryPage>> {
    match ipc::request(pipe, Action::Directory(request.clone())).await? {
        Response::Directory(page) => Ok(page),
        _ => bail!("目录响应无效"),
    }
}

impl Browser {
    pub fn enter(&self, row: usize) -> Option<Location> {
        match self.listing.as_ref()? {
            Listing::Candidates(_) => None,
            Listing::Partitions(snapshot) => {
                let asset = snapshot.assets.get(row)?;
                let r = &asset.registration;
                if !matches!(r.state.as_str(), "ready" | "historical") {
                    return None;
                }
                Some(Location::Folder {
                    name: format!("{}（{}）", r.name, self.cell(row, 4)),
                    request: Box::new(DirectoryRequest {
                        partition_id: r.partition_id.clone(),
                        filesystem_id: r.filesystem_id.clone(),
                        generation: None,
                        revision: None,
                        parent: None,
                        offset: 0,
                        folders_only: false,
                        sort: Default::default(),
                        filter: None,
                    }),
                    previous: vec![],
                })
            }
            Listing::Folder(page) => {
                let entry = page.entries.get(row)?;
                (entry.kind == "folder").then(|| self.folder_at(Some(entry.entry.clone())))
            }
        }
    }

    fn folder_at(&self, parent: Option<Vec<u8>>) -> Location {
        let mut target = self.location.clone();
        if let Location::Folder {
            request, previous, ..
        } = &mut target
        {
            request.parent = parent;
            request.offset = 0;
            previous.clear();
        }
        target
    }

    pub fn up(&self) -> Option<Location> {
        if matches!(self.location, Location::Candidates) {
            return Some(Location::Partitions);
        }
        let Location::Folder { request, .. } = &self.location else {
            return None;
        };
        if let Some(Listing::Folder(page)) = &self.listing
            && page.ancestors.len() >= 2
        {
            let parent = &page.ancestors[page.ancestors.len() - 2];
            return Some(self.folder_at(Some(parent.entry.clone())));
        }
        if self.listing.is_none() && request.parent.is_some() {
            let mut root = self.folder_at(None);
            if let Location::Folder { request, .. } = &mut root {
                request.generation = None;
                request.revision = None;
            }
            return Some(root);
        }
        Some(Location::Partitions)
    }

    pub fn page(&self, forward: bool) -> Option<Location> {
        let Listing::Folder(data) = self.listing.as_ref()? else {
            return None;
        };
        let mut target = self.location.clone();
        let Location::Folder {
            request, previous, ..
        } = &mut target
        else {
            return None;
        };
        if forward {
            let offset = data.next?;
            previous.push(request.offset);
            request.offset = offset;
        } else {
            request.offset = previous.pop()?;
        }
        Some(target)
    }

    pub fn apply(&mut self, listing: Listing) -> (String, String, usize) {
        let (path, status, count) = match (&mut self.location, &listing) {
            (Location::Candidates, Listing::Candidates(snapshot)) => {
                let count = snapshot
                    .candidates
                    .iter()
                    .filter(|c| registration::selectable(c))
                    .count();
                let status = match &snapshot.error {
                    Some(error) => format!("无法读取完整分区状态：{error}"),
                    None if count == 0 => "没有可登记的分区".into(),
                    None => format!("{count} 个可登记分区，请选择一项"),
                };
                ("登记分区".into(), status, snapshot.candidates.len())
            }
            (Location::Partitions, Listing::Partitions(snapshot)) => {
                let count = snapshot.assets.len();
                let status = if let Some(error) = &snapshot.error {
                    format!("无法读取完整分区状态：{error}")
                } else if count == 0 {
                    "尚未登记分区".to_owned()
                } else {
                    format!("{count} 个已登记分区")
                };
                ("已登记分区".to_owned(), status, count)
            }
            (Location::Folder { name, request, .. }, Listing::Folder(page)) => {
                let reset = request.parent.is_some()
                    && page.ancestors.len() == 1
                    && request
                        .generation
                        .as_ref()
                        .is_some_and(|old| old != &page.generation);
                request.parent = (page.ancestors.len() > 1).then(|| page.location.clone());
                request.generation = Some(page.generation.clone());
                request.revision = Some(page.revision.clone());
                let mut path = name.clone();
                for part in page.ancestors.iter().skip(1) {
                    path.push('\\');
                    path.push_str(&part.name);
                }
                let count = page.entries.len();
                let mut status = if count == 0 {
                    "当前索引中没有条目".to_owned()
                } else {
                    format!(
                        "第 {}–{} 项，共 {} 项",
                        u64::from(request.offset) + 1,
                        u64::from(request.offset) + count as u64,
                        page.total
                    )
                };
                if page.partial {
                    status.push_str(" · 部分索引");
                }
                if page.needs_validation {
                    status.push_str(" · 索引待校验");
                }
                if reset {
                    status.push_str(" · 索引已重建，已返回分区根目录");
                }
                (path, status, count)
            }
            _ => unreachable!("a browse response belongs to its requested location"),
        };
        self.listing = Some(listing);
        (path, status, count)
    }

    pub fn cell(&self, row: usize, column: i32) -> &str {
        match &self.listing {
            Some(Listing::Candidates(snapshot)) => {
                snapshot.candidates.get(row).map_or("", |c| {
                    let o = &c.observation;
                    match column {
                        0 => registration::volume_name(o),
                        1 => o.mounts.first().map(String::as_str).unwrap_or("无挂载点"),
                        2 => o.model.as_deref().unwrap_or("未知"),
                        3 => o.identity.filesystem.as_deref().unwrap_or("未知"),
                        4 if c.registered => "已登记",
                        4 => c.blocked.as_deref().or(o.error.as_deref()).unwrap_or(
                            if !o.eligible() {
                                "不支持的文件系统"
                            } else if c.weak {
                                "身份信息不足，需确认"
                            } else {
                                "可登记"
                            },
                        ),
                        _ => "",
                    }
                })
            }
            Some(Listing::Partitions(snapshot)) => snapshot.assets.get(row).map_or("", |asset| {
                let r = &asset.registration;
                match column {
                    0 => &r.name,
                    1 => r.observation.model.as_deref().unwrap_or("未知"),
                    2 => r
                        .observation
                        .identity
                        .filesystem
                        .as_deref()
                        .unwrap_or("未知"),
                    3 => registration_state(asset),
                    4 if asset.connection == "changed" => "文件系统已变化",
                    4 => crate::window::connection_label(&asset.connection),
                    _ => "",
                }
            }),
            Some(Listing::Folder(page)) => page.entries.get(row).map_or("", |entry| match column {
                0 => &entry.name,
                1 => &entry.path,
                2 if entry.kind == "folder" => "文件夹",
                2 => "文件",
                3 if entry.kind == "folder" => "—",
                3 => entry.size.as_deref().unwrap_or("未知"),
                _ => "",
            }),
            None => "",
        }
    }
}

fn registration_state(asset: &Asset) -> &str {
    if let Some(error) = &asset.error {
        return error;
    }
    match asset.registration.state.as_str() {
        "ready" => "已登记",
        "historical" => "历史记录",
        "preparing" => "准备中",
        "failed" => "登记失败",
        "removing" => "移除中",
        _ => "状态未确认",
    }
}

#[cfg(test)]
mod tests;
