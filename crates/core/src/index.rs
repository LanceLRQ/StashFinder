use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IndexStatus {
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
