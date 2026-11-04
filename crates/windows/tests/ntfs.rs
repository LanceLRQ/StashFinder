use stashfinder_windows::ntfs::{decode_directory, decode_usn, read_directory};
use std::sync::atomic::AtomicBool;

#[test]
fn usn_keeps_full_ids_reasons_and_utf16_and_rejects_bad_records() {
    let mut bytes = vec![0u8; 80];
    bytes[..8].copy_from_slice(&200u64.to_le_bytes());
    bytes[8..12].copy_from_slice(&72u32.to_le_bytes());
    bytes[12..14].copy_from_slice(&2u16.to_le_bytes());
    bytes[16..24].copy_from_slice(&0xabcd000000000009u64.to_le_bytes());
    bytes[24..32].copy_from_slice(&5u64.to_le_bytes());
    bytes[32..40].copy_from_slice(&100i64.to_le_bytes());
    bytes[48..52].copy_from_slice(&0x2000u32.to_le_bytes());
    bytes[60..64].copy_from_slice(&16u32.to_le_bytes());
    bytes[64..66].copy_from_slice(&4u16.to_le_bytes());
    bytes[66..68].copy_from_slice(&60u16.to_le_bytes());
    bytes[68..72].copy_from_slice(&[0x2d, 0x4e, 0x00, 0xd8]);
    let page = decode_usn(&bytes).unwrap();
    assert_eq!(page.next, 200);
    assert_eq!(page.records[0].id, 0xabcd000000000009);
    assert_eq!(page.records[0].parent, 5);
    assert_eq!(page.records[0].usn, 100);
    assert_eq!(page.records[0].reason, 0x2000);
    assert_eq!(page.records[0].name, [0x4e2d, 0xd800]);
    assert!(decode_usn(&bytes[..70]).is_err());
    bytes[12..14].copy_from_slice(&4u16.to_le_bytes());
    assert!(decode_usn(&bytes).is_err());
    bytes[12..14].copy_from_slice(&2u16.to_le_bytes());
    bytes[8..12].copy_from_slice(&0u32.to_le_bytes());
    assert!(decode_usn(&bytes).is_err());
}

#[test]
fn directory_parser_reads_metadata_without_using_usn_timestamp() {
    let mut bytes = vec![0u8; 112];
    bytes[8..16].copy_from_slice(&123u64.to_le_bytes());
    bytes[24..32].copy_from_slice(&456u64.to_le_bytes());
    bytes[40..48].copy_from_slice(&7i64.to_le_bytes());
    bytes[56..60].copy_from_slice(&32u32.to_le_bytes());
    bytes[60..64].copy_from_slice(&2u32.to_le_bytes());
    bytes[96..104].copy_from_slice(&0x123400000000002au64.to_le_bytes());
    bytes[104..106].copy_from_slice(&[0x41, 0]);
    let rows = decode_directory(&bytes).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, 0x123400000000002a);
    assert_eq!(rows[0].name, [65]);
    assert_eq!(rows[0].size, 7);
    assert_eq!(rows[0].created, 123);
    assert_eq!(rows[0].modified, 456);
    assert!(decode_directory(&bytes[..105]).is_err());
    bytes[..4].copy_from_slice(&8u32.to_le_bytes());
    assert!(decode_directory(&bytes).is_err());
}

#[test]
fn real_directory_enumeration_keeps_zero_size_and_hard_link_identity() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("零字节.txt"), []).unwrap();
    std::fs::hard_link(dir.path().join("零字节.txt"), dir.path().join("second.txt")).unwrap();
    std::fs::create_dir(dir.path().join("folder")).unwrap();
    let mut rows = Vec::new();
    read_directory(dir.path(), &AtomicBool::new(false), |batch| {
        rows.extend_from_slice(batch);
        Ok(())
    })
    .unwrap();
    assert_eq!(rows.len(), 3);
    let files = rows
        .iter()
        .filter(|r| r.attributes & 16 == 0)
        .collect::<Vec<_>>();
    assert_eq!(files[0].size, 0);
    assert_eq!(files[0].id, files[1].id);
    assert!(read_directory(dir.path(), &AtomicBool::new(true), |_| Ok(())).is_err());
}
