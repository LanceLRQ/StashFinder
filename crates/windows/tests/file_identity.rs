use stashfinder_core::{assets::*, file_operations::*};
use stashfinder_windows::file_operations::inspect;
use stashfinder_windows::file_operations::{FileInfo, verify_with};
use std::path::Path;
#[test]
fn reads_identity_and_metadata_from_the_same_ordinary_handle() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("文件.txt");
    std::fs::write(&path, b"hello").unwrap();
    let first = inspect(&path).unwrap();
    let second = inspect(&path).unwrap();
    assert_eq!(first.identity, second.identity);
    assert_eq!(first.size, Some(5));
    assert_eq!(first.attributes & 16, 0);
    assert!(first.created.is_some());
    assert!(first.modified.is_some());
    assert_eq!(inspect(dir.path()).unwrap().attributes & 16, 16);
    assert!(inspect(&dir.path().join("missing")).is_err());
}

fn target() -> PreparedTarget {
    PreparedTarget {
        mounts: vec![],
        file: ResolvedEntry {
            reference: EntryRef {
                partition_id: "p".into(),
                filesystem_id: "fs".into(),
                generation: "1".into(),
                revision: "1".into(),
                entry: vec![2],
            },
            relative: "folder\\file.txt".encode_utf16().collect(),
            object: vec![2],
            parent: Some(vec![1]),
            folder: false,
            size: None,
            created: None,
            modified: None,
            identity: Some(ExpectedIdentity::Native(NativeIdentity {
                volume: 10,
                file: 20,
            })),
        },
        volume: r"\\?\Volume{00000000-0000-0000-0000-000000000001}\".into(),
        identity: Identity {
            layout: Layout::Unknown,
            filesystem: Some("exFAT".into()),
            serial: Some(Serial {
                source: "win32_volume32".into(),
                value: "0000000a".into(),
            }),
        },
        partition_name: "Test".into(),
    }
}
fn info(path: &Path) -> anyhow::Result<FileInfo> {
    let folder = path.file_name().is_none_or(|s| s != "file.txt");
    Ok(FileInfo {
        identity: NativeIdentity {
            volume: 10,
            file: if folder { 1 } else { 20 },
        },
        volume64: Some(10),
        attributes: if folder { 16 } else { 0 },
        size: None,
        created: None,
        modified: None,
    })
}
#[test]
fn ordinary_preflight_rejects_replacement_volume_change_and_reparse_ancestors() {
    let original = target();
    assert!(verify_with(&original, false, info).is_ok());
    assert!(
        verify_with(&original, false, |path| {
            let mut i = info(path)?;
            i.identity.volume = 11;
            Ok(i)
        })
        .is_err()
    );
    assert!(
        verify_with(&original, false, |path| {
            let mut i = info(path)?;
            if i.attributes & 16 == 0 {
                i.identity.file = 21;
            }
            Ok(i)
        })
        .is_err()
    );
    assert!(
        verify_with(&original, false, |path| {
            let mut i = info(path)?;
            if path.file_name().is_some_and(|s| s == "folder") {
                i.attributes |= 1024;
            }
            Ok(i)
        })
        .is_err()
    );
    let mut missing = target();
    missing.file.identity = None;
    assert!(verify_with(&missing, false, info).is_err());
    assert!(verify_with(&original, false, |_| anyhow::bail!("拒绝访问")).is_err());
}

#[test]
fn scan_fingerprints_and_ntfs_parent_volume_are_checked() {
    let mut scan = target();
    scan.file.size = Some(5);
    assert!(
        verify_with(&scan, false, |p| {
            let mut v = info(p)?;
            v.size = Some(9);
            Ok(v)
        })
        .is_err()
    );
    let mut ntfs = target();
    ntfs.identity.serial = Some(Serial {
        source: "ntfs64".into(),
        value: "a".into(),
    });
    ntfs.file.identity = Some(ExpectedIdentity::Ntfs { frn: 20 });
    assert!(
        verify_with(&ntfs, false, |p| {
            let mut v = info(p)?;
            if p.file_name().is_some_and(|s| s == "folder") {
                v.identity.volume = 11;
            }
            Ok(v)
        })
        .is_err()
    );
}

#[test]
fn destination_directory_timestamp_changes_do_not_invalidate_its_identity() {
    let mut destination = target();
    destination.file.relative = "folder".encode_utf16().collect();
    destination.file.folder = true;
    destination.file.identity = Some(ExpectedIdentity::Native(NativeIdentity {
        volume: 10,
        file: 1,
    }));
    destination.file.modified = Some(100);
    assert!(
        verify_with(&destination, true, |p| {
            let mut value = info(p)?;
            value.modified = Some(200);
            Ok(value)
        })
        .is_ok()
    );
}
