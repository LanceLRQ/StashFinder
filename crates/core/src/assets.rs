use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Layout {
    Gpt {
        disk: String,
        partition: String,
    },
    Mbr {
        signature: u32,
        offset: u64,
        length: u64,
        device: Option<String>,
    },
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Serial {
    pub source: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub layout: Layout,
    pub filesystem: Option<String>,
    pub serial: Option<Serial>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityMatch {
    Same,
    FilesystemChanged,
    Uncertain,
    Different,
}

impl Identity {
    pub fn compare(&self, other: &Self) -> IdentityMatch {
        use IdentityMatch::*;
        let partition = match (&self.layout, &other.layout) {
            (
                Layout::Gpt {
                    disk: a,
                    partition: p,
                },
                Layout::Gpt {
                    disk: b,
                    partition: q,
                },
            ) => {
                if p != q {
                    return Different;
                }
                a == b
            }
            (
                Layout::Mbr {
                    signature: a,
                    offset: p,
                    length: n,
                    device: x,
                },
                Layout::Mbr {
                    signature: b,
                    offset: q,
                    length: m,
                    device: y,
                },
            ) => {
                if p != q {
                    return Different;
                }
                if a != b && (x.is_none() || y.is_none() || x != y) {
                    return Different;
                }
                a == b && n == m && x.is_some() && x == y
            }
            (Layout::Unknown, _) | (_, Layout::Unknown) => return Uncertain,
            _ => return Different,
        };
        if !partition {
            return Uncertain;
        }
        match (
            &self.filesystem,
            &other.filesystem,
            &self.serial,
            &other.serial,
        ) {
            (Some(a), Some(b), _, _) if a != b => FilesystemChanged,
            (Some(_), Some(_), Some(a), Some(b)) if a.source == b.source => {
                if a.value == b.value {
                    Same
                } else {
                    FilesystemChanged
                }
            }
            _ => Uncertain,
        }
    }

    pub fn disk_key(&self) -> Option<String> {
        match &self.layout {
            Layout::Gpt { disk, .. } => Some(format!("gpt:{disk}")),
            Layout::Mbr {
                signature,
                device: Some(device),
                ..
            } => Some(format!("mbr:{signature}:{device}")),
            _ => None,
        }
    }

    pub fn partition_key(&self) -> Option<String> {
        match &self.layout {
            Layout::Gpt { disk, partition } => Some(format!("gpt:{disk}:{partition}")),
            Layout::Mbr {
                signature,
                offset,
                length,
                ..
            } => Some(format!("mbr:{signature}:{offset}:{length}")),
            Layout::Unknown => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Capacity {
    pub total: u64,
    pub free: u64,
    pub sampled_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub candidate_id: String,
    pub disk_number: Option<u32>,
    pub model: Option<String>,
    pub disk_bytes: Option<u64>,
    pub partition_bytes: Option<u64>,
    pub mounts: Vec<String>,
    pub identity: Identity,
    pub capacity: Option<Capacity>,
    pub error: Option<String>,
}

impl Observation {
    pub fn eligible(&self) -> bool {
        self.error.is_none()
            && matches!(
                self.identity.filesystem.as_deref(),
                Some("NTFS" | "exFAT" | "FAT32")
            )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Registration {
    pub disk_id: String,
    pub partition_id: String,
    pub filesystem_id: String,
    pub name: String,
    pub revision: u64,
    pub state: String,
    pub observation: Observation,
    pub last_seen: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Asset {
    pub registration: Registration,
    pub connection: String,
    pub mounts: Vec<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub observation: Observation,
    pub weak: bool,
    pub blocked: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u64,
    pub busy: bool,
    pub error: Option<String>,
    pub assets: Vec<Asset>,
    pub candidates: Vec<Candidate>,
}

pub fn validate_name(name: &str) -> Result<String> {
    let value = name.trim();
    ensure!(
        !value.is_empty() && value.chars().count() <= 128 && !value.chars().any(char::is_control),
        "名称须为 1～128 个字符，且不能包含控制字符"
    );
    Ok(value.to_owned())
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
