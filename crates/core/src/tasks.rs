use crate::index::IndexMode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub partition_id: String,
    pub kind: String,
    pub mode: IndexMode,
    pub automatic: bool,
    pub state: String,
    pub created: u64,
    pub updated: u64,
    pub error: Option<String>,
    pub phase: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskPage {
    pub version: u64,
    pub row: Option<Task>,
    pub next: Option<usize>,
    pub automatic: bool,
    pub paused: Vec<String>,
}

#[derive(Debug, Hash, Serialize, Deserialize)]
pub struct PartitionIndex {
    pub partition_id: String,
    pub paused: bool,
    pub index_version: Option<String>,
    pub indexed_at: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum TaskOverviewRow {
    Partition(PartitionIndex),
    Task(Task),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskOverviewPage {
    pub version: String,
    pub rows: Vec<TaskOverviewRow>,
    pub next: Option<usize>,
    pub automatic: bool,
}
