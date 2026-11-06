use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexMode {
    Journal,
    CreateJournal,
    ScanOnly,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IndexStatus {
    #[serde(default)]
    pub task_id: Option<String>,
    pub generation: Option<i64>,
    pub active_generation: Option<i64>,
    pub phase: String,
    pub started: Option<u64>,
    pub completed: Option<u64>,
    pub records: u64,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchRow {
    pub entry: Vec<u8>,
    pub parent: Vec<u8>,
    pub object: Vec<u8>,
    pub name: String,
    pub path: String,
    pub kind: String,
    pub attributes: u32,
    pub size: Option<String>,
    pub created: Option<String>,
    pub modified: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Matches {
    pub rows: Vec<SearchRow>,
    pub more: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchRequest {
    pub text: String,
    pub names_only: bool,
    pub partitions: Vec<String>,
    #[serde(default)]
    pub options: Box<crate::search::QueryOptions>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchHit {
    pub duplicate_group: Option<u32>,
    pub hard_link: bool,
    pub filesystem_id: String,
    pub partition_id: String,
    pub partition_name: String,
    pub connection: String,
    pub mounts: Vec<String>,
    pub generation: i64,
    pub revision: i64,
    pub partial: bool,
    pub indexed_at: Option<u64>,
    pub needs_validation: bool,
    pub file: SearchRow,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchPage {
    pub rows: Vec<SearchHit>,
    pub total: usize,
    pub groups: usize,
    pub missing: usize,
    pub id: u64,
    pub done: bool,
    pub row: Option<SearchHit>,
    pub next: Option<usize>,
    pub more: bool,
    pub error: Option<String>,
    pub unavailable: Vec<String>,
}
