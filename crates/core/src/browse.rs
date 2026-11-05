use crate::index::SearchRow;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryRequest {
    pub partition_id: String,
    pub filesystem_id: String,
    pub generation: Option<String>,
    pub revision: Option<String>,
    pub parent: Option<Vec<u8>>,
    pub offset: u32,
    pub folders_only: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ancestor {
    pub entry: Vec<u8>,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DirectoryPage {
    pub filesystem_id: String,
    pub generation: String,
    pub revision: String,
    pub location: Vec<u8>,
    pub ancestors: Vec<Ancestor>,
    pub entries: Vec<SearchRow>,
    pub next: Option<u32>,
    pub partial: bool,
    pub indexed_at: Option<u64>,
    pub needs_validation: bool,
}
