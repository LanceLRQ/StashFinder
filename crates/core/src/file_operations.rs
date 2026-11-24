use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeIdentity {
    pub volume: u32,
    pub file: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryRef {
    pub partition_id: String,
    pub filesystem_id: String,
    pub generation: String,
    pub revision: String,
    pub entry: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExpectedIdentity {
    Ntfs { frn: u64 },
    Native(NativeIdentity),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolvedEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_version: Option<i64>,
    pub reference: EntryRef,
    pub relative: Vec<u16>,
    pub object: Vec<u8>,
    pub parent: Option<Vec<u8>>,
    pub folder: bool,
    pub size: Option<i64>,
    pub created: Option<u64>,
    pub modified: Option<u64>,
    pub identity: Option<ExpectedIdentity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoFinding {
    Video,
    Other,
    Unknown,
}

pub const VIDEO_EXTENSIONS: &[&str] = &[
    "ts", "mts", "dat", "mod", "vdr", "mp4", "mkv", "avi", "mov", "wmv", "webm", "m2ts", "m4v",
    "asf",
];

pub const MAX_SELECTED: usize = 10_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SelectionOrigin {
    Directory {
        context: EntryRef,
        all: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<Box<crate::search::FileFilter>>,
    },
    Search {
        id: u64,
        range: Option<[usize; 2]>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionChunk {
    Entries(Vec<Vec<u8>>),
    Positions(Vec<usize>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreparedTarget {
    pub mounts: Vec<String>,
    pub file: ResolvedEntry,
    pub volume: String,
    pub identity: crate::assets::Identity,
    pub partition_name: String,
}

pub fn record_text(rows: &[PreparedTarget], names_only: bool) -> Vec<u16> {
    let mut text = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            text.extend([13, 10]);
        }
        if names_only {
            text.extend(row.file.relative.rsplit(|c| *c == 92).next().unwrap_or(&[]));
        } else {
            if let Some(root) = row.mounts.first() {
                text.extend(root.encode_utf16());
            } else {
                text.extend(row.partition_name.encode_utf16());
                text.extend(" / ".encode_utf16());
            }
            text.extend(&row.file.relative);
        }
    }
    text
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualRecord {
    pub target: PreparedTarget,
    pub online: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThumbnailTypes {
    pub images: bool,
    pub videos: bool,
    pub documents: bool,
    pub extensions: Vec<String>,
}

impl Default for ThumbnailTypes {
    fn default() -> Self {
        Self {
            images: true,
            videos: true,
            documents: true,
            extensions: Vec::new(),
        }
    }
}

impl ThumbnailTypes {
    pub fn normalize(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.extensions.iter().map(String::len).sum::<usize>() <= 4096,
            "自定义扩展名过长"
        );
        self.extensions = crate::search::normalize_extensions(&self.extensions)?;
        anyhow::ensure!(self.extensions.len() <= 64, "最多添加 64 个自定义扩展名");
        Ok(())
    }

    pub fn selected_extensions(&self) -> std::collections::HashSet<&str> {
        let mut selected = self
            .extensions
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>();
        if self.images {
            selected.extend([
                "jpg", "jpeg", "png", "webp", "gif", "bmp", "tiff", "tif", "heic", "heif", "avif",
                "svg",
            ]);
        }
        if self.videos {
            selected.extend(["mp4", "mkv", "mov", "avi", "webm", "wmv", "m4v"]);
        }
        if self.documents {
            selected.insert("pdf");
        }
        selected
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThumbnailScope {
    #[serde(default)]
    pub analyse: bool,
    pub partition_id: String,
    pub filesystem_id: String,
    pub generation: Option<String>,
    pub folder: Option<Vec<u8>>,
    pub types: ThumbnailTypes,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ThumbnailPage {
    pub analyse_indices: Vec<usize>,
    pub files: Vec<ResolvedEntry>,
    pub unavailable: usize,
    pub mounts: Vec<String>,
    pub volume: String,
    pub identity: crate::assets::Identity,
    pub partition_name: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct SelectionPage {
    pub id: u64,
    pub total: usize,
    pub next: Option<usize>,
    pub rows: Vec<PreparedTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeState {
    NotRun,
    Completed,
    Skipped,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ItemOutcome {
    pub index: usize,
    pub source: String,
    pub state: OutcomeState,
    pub destination: Option<String>,
    pub error: Option<String>,
    pub note: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperationReport {
    pub items: Vec<ItemOutcome>,
    pub cancelled: bool,
    pub error: Option<String>,
}
impl OperationReport {
    pub fn completed(&self) -> usize {
        self.items
            .iter()
            .filter(|r| r.state == OutcomeState::Completed)
            .count()
    }
    pub fn success(&self) -> bool {
        !self.cancelled && self.error.is_none() && self.completed() == self.items.len()
    }
    pub fn summary(&self) -> String {
        format!(
            "完成 {} 项，失败 {} 项，跳过 {} 项，未执行 {} 项{}",
            self.completed(),
            self.items
                .iter()
                .filter(|r| r.state == OutcomeState::Failed)
                .count(),
            self.items
                .iter()
                .filter(|r| r.state == OutcomeState::Skipped)
                .count(),
            self.items
                .iter()
                .filter(|r| r.state == OutcomeState::NotRun)
                .count(),
            if self.cancelled {
                "；操作已取消"
            } else {
                ""
            }
        )
    }
}

pub fn top_level(rows: Vec<ResolvedEntry>) -> Vec<ResolvedEntry> {
    let folders: std::collections::HashSet<_> = rows
        .iter()
        .filter(|r| r.folder)
        .map(|r| {
            (
                r.reference.partition_id.as_str(),
                r.reference.filesystem_id.as_str(),
                r.relative.as_slice(),
            )
        })
        .collect();
    let mut seen = std::collections::HashSet::new();
    let keep: Vec<bool> = rows
        .iter()
        .map(|r| {
            seen.insert((
                r.reference.partition_id.as_str(),
                r.reference.filesystem_id.as_str(),
                r.reference.entry.as_slice(),
            )) && !r.relative.iter().enumerate().any(|(i, c)| {
                *c == 92
                    && folders.contains(&(
                        r.reference.partition_id.as_str(),
                        r.reference.filesystem_id.as_str(),
                        &r.relative[..i],
                    ))
            })
        })
        .collect();
    rows.into_iter()
        .zip(keep)
        .filter_map(|(r, keep)| keep.then_some(r))
        .collect()
}
