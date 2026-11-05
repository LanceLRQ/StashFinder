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
