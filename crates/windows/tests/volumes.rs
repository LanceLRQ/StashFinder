use stashfinder_windows::volumes::{decode_descriptor_string, decode_extent, read_field};

#[test]
fn rejects_truncated_or_invalid_device_buffers() {
    assert!(read_field::<u64>(&[1, 2, 3], 0).is_err());
    assert!(read_field::<u32>(&[0; 8], usize::MAX).is_err());
    assert_eq!(read_field::<u32>(&[9, 1, 0, 0, 0], 1).unwrap(), 1);
    assert_eq!(
        decode_descriptor_string(b"\0Model\0", 1).unwrap(),
        Some("Model".into())
    );
    assert_eq!(decode_descriptor_string(b"\0", 0).unwrap(), None);
    assert!(decode_descriptor_string(b"\0broken", 1).is_err());
    assert!(decode_descriptor_string(b"\0", 20).is_err());
}

#[test]
fn rejects_multiple_extents_and_reads_only_complete_single_extent() {
    use std::mem::{offset_of, size_of};
    use windows::Win32::System::Ioctl::{DISK_EXTENT, VOLUME_DISK_EXTENTS};
    let mut bytes = vec![0; size_of::<VOLUME_DISK_EXTENTS>()];
    bytes[0..4].copy_from_slice(&2u32.to_le_bytes());
    assert!(decode_extent(&bytes).is_err());
    bytes[0..4].copy_from_slice(&1u32.to_le_bytes());
    let start = offset_of!(VOLUME_DISK_EXTENTS, Extents);
    bytes[start..start + 4].copy_from_slice(&3u32.to_le_bytes());
    let offset = start + offset_of!(DISK_EXTENT, StartingOffset);
    bytes[offset..offset + 8].copy_from_slice(&4096i64.to_le_bytes());
    let length = start + offset_of!(DISK_EXTENT, ExtentLength);
    bytes[length..length + 8].copy_from_slice(&8192i64.to_le_bytes());
    assert_eq!(decode_extent(&bytes).unwrap(), (3, 4096, 8192));
    assert!(decode_extent(&bytes[..length + 7]).is_err());
}
