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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualRecord {
    pub target: PreparedTarget,
    pub online: bool,
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
