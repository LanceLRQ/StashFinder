pub mod assets;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const PROTOCOL_VERSION: u32 = 2;
pub const MAX_FRAME: usize = 16 * 1024;
pub const SNAPSHOT_EXPIRED: &str = "设备快照已更新，请刷新";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub id: u64,
    pub action: Action,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Status,
    Page {
        version: Option<u64>,
        offset: usize,
    },
    Refresh,
    Register {
        version: u64,
        candidate_id: String,
        name: String,
        allow_weak: bool,
    },
    Retry {
        version: u64,
        partition_id: String,
        candidate_id: String,
    },
    Rename {
        partition_id: String,
        name: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Reply {
    pub version: u32,
    pub id: u64,
    pub body: Response,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Response {
    Status(Status),
    Page(Box<Page>),
    Accepted,
    Error(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Page {
    pub version: u64,
    pub busy: bool,
    pub error: Option<String>,
    pub asset: Option<assets::Asset>,
    pub candidate: Option<assets::Candidate>,
    pub next: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub version: u32,
    pub id: u64,
    pub process_id: u32,
    pub mode: String,
    pub registered_partitions: u32,
}

pub fn validate_request(request: &Request) -> Result<()> {
    ensure!(
        request.version == PROTOCOL_VERSION,
        "unsupported protocol version"
    );
    Ok(())
}

pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>> {
    let length = reader.read_u32_le().await? as usize;
    ensure!(length > 0 && length <= MAX_FRAME, "invalid frame size");
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).await?;
    Ok(bytes)
}

pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, data: &[u8]) -> Result<()> {
    ensure!(
        !data.is_empty() && data.len() <= MAX_FRAME,
        "invalid frame size"
    );
    writer.write_u32_le(data.len() as u32).await?;
    writer.write_all(data).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_protocol_mismatch_and_unknown_actions() {
        let request = Request {
            version: 99,
            id: 7,
            action: Action::Status,
        };
        assert!(validate_request(&request).is_err());
        assert!(
            serde_json::from_str::<Request>(r#"{"version":1,"id":1,"action":"copy_file"}"#)
                .is_err()
        );
        assert!(
            serde_json::from_str::<Request>(
                r#"{"version":1,"id":1,"action":"status","path":"C:\\"}"#
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn rejects_oversize_before_allocating_and_truncated_frames() {
        assert!(
            read_frame(&mut &(u32::MAX.to_le_bytes())[..])
                .await
                .is_err()
        );
        assert!(read_frame(&mut &[4, 0, 0, 0, 1][..]).await.is_err());
        assert!(read_frame(&mut &[0, 0, 0, 0][..]).await.is_err());
    }

    #[tokio::test]
    async fn round_trips_a_bounded_frame() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, b"hello").await.unwrap();
        assert_eq!(read_frame(&mut bytes.as_slice()).await.unwrap(), b"hello");
        assert!(
            write_frame(&mut bytes, &vec![0; MAX_FRAME + 1])
                .await
                .is_err()
        );
    }
}
