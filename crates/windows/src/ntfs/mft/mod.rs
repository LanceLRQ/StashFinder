mod reader;
mod record;

pub use reader::{FastScanStats, ReadAt, scan_mft};
pub use record::{Attribute, AttributeValue, DataRun, Record, decode_record, decode_runs};

use crate::volumes::read_field;
use anyhow::{Context, Result, ensure};

const SEGMENT_MASK: u64 = 0x0000_ffff_ffff_ffff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub(super) sector_bytes: usize,
    pub(super) cluster_bytes: usize,
    pub(super) record_bytes: usize,
    pub(super) total_clusters: u64,
    pub(super) mft_lcn: u64,
    pub(super) mft_len: u64,
}

impl Geometry {
    pub fn from_volume_data(bytes: &[u8]) -> Result<Option<Self>> {
        // NTFS_EXTENDED_VOLUME_DATA follows the 96-byte volume data structure.
        if bytes.len() < 104
            || read_field::<u32>(bytes, 96)? < 8
            || read_field::<u16>(bytes, 100)? != 3
            || read_field::<u16>(bytes, 102)? > 1
        {
            return Ok(None);
        }
        Ok(Some(Self::new(
            read_field::<u32>(bytes, 40)? as usize,
            read_field::<u32>(bytes, 44)? as usize,
            read_field::<u32>(bytes, 48)? as usize,
            u64::try_from(read_field::<i64>(bytes, 16)?)?,
            u64::try_from(read_field::<i64>(bytes, 64)?)?,
            u64::try_from(read_field::<i64>(bytes, 56)?)?,
        )?))
    }

    pub fn new(
        sector_bytes: usize,
        cluster_bytes: usize,
        record_bytes: usize,
        total_clusters: u64,
        mft_lcn: u64,
        mft_len: u64,
    ) -> Result<Self> {
        ensure!(
            sector_bytes >= 512
                && sector_bytes.is_power_of_two()
                && cluster_bytes >= sector_bytes
                && cluster_bytes.is_power_of_two()
                && (512..=65536).contains(&record_bytes)
                && record_bytes.is_power_of_two(),
            "NTFS 卷几何无效"
        );
        let bytes = total_clusters
            .checked_mul(cluster_bytes as u64)
            .context("NTFS 卷大小溢出")?;
        ensure!(
            mft_lcn != 0
                && mft_lcn < total_clusters
                && mft_len >= record_bytes as u64 * 6
                && mft_len <= bytes
                && mft_len.is_multiple_of(record_bytes as u64)
                && mft_len / record_bytes as u64 <= SEGMENT_MASK + 1,
            "MFT 起点或有效长度无效"
        );
        Ok(Self {
            sector_bytes,
            cluster_bytes,
            record_bytes,
            total_clusters,
            mft_lcn,
            mft_len,
        })
    }
}

#[derive(Debug)]
pub struct FastScanUnavailable(pub &'static str);

impl std::fmt::Display for FastScanUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for FastScanUnavailable {}
