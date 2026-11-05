use stashfinder_core::file_operations::*;
use stashfinder_store::{file_operations, volume::*};

fn fixture(path: &std::path::Path) -> (Volume, EntryRef) {
    let mut db = Volume::create(path, r#"{"partition_id":"p","filesystem_id":"fs"}"#).unwrap();
    let g = db.begin_generation().unwrap();
    let objects = [(5, true), (7, true), (8, false)].map(|(id, folder)| Object {
        identity: None,
        id: (id as u64).to_le_bytes().to_vec(),
        source: "ntfs64".into(),
        kind: if folder { "folder" } else { "file" }.into(),
        size: None,
        created: None,
        modified: None,
        attributes: if folder { 16 } else { 0 },
    });
    let entries = [
        Entry {
            id: vec![1],
            object: 5u64.to_le_bytes().to_vec(),
            parent: None,
            name: vec![92],
        },
        Entry {
            id: vec![2],
            object: 7u64.to_le_bytes().to_vec(),
            parent: Some(vec![1]),
            name: "资料".encode_utf16().collect(),
        },
        Entry {
            id: vec![3],
            object: 8u64.to_le_bytes().to_vec(),
            parent: Some(vec![2]),
            name: vec![0xd800, 46, 116, 120, 116],
        },
        Entry {
            id: vec![4],
            object: 8u64.to_le_bytes().to_vec(),
            parent: Some(vec![1]),
            name: "link.txt".encode_utf16().collect(),
        },
    ];
    db.write_batch(g, &objects, &entries, &[], None).unwrap();
    db.complete(g).unwrap();
    db.publish(g).unwrap();
    let sql = rusqlite::Connection::open(path).unwrap();
    let revision: i64 = sql
        .query_row("SELECT revision FROM generations WHERE id=?1", [g], |r| {
            r.get(0)
        })
        .unwrap();
    (
        db,
        EntryRef {
            partition_id: "p".into(),
            filesystem_id: "fs".into(),
            generation: g.to_string(),
            revision: revision.to_string(),
            entry: vec![3],
        },
    )
}

#[test]
fn previous_schema_text_records_remain_readable_without_migrating_offline_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.sqlite");
    let (volume, reference) = fixture(&path);
    drop(volume);
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch(
        "UPDATE objects SET source='path'; DROP TABLE native_identity; PRAGMA user_version=3;",
    )
    .unwrap();
    let record = file_operations::record(&path, &reference).unwrap();
    assert!(record.identity.is_none());
    assert!(!record.relative.is_empty());
    let root = EntryRef {
        entry: vec![1],
        ..reference.clone()
    };
    assert!(file_operations::record(&path, &root).unwrap().folder);
    assert_eq!(
        file_operations::directory_refs(&path, &root, true)
            .unwrap()
            .len(),
        2
    );
    assert!(file_operations::resolve(&path, &reference, false).is_err());
    assert_eq!(
        sql.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
}

#[test]
fn one_legitimate_deep_path_over_the_wire_budget_is_explicitly_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.sqlite");
    let (mut volume, mut reference) = fixture(&path);
    let generation = volume.begin_generation().unwrap();
    let mut objects = (100u8..=120)
        .map(|id| Object {
            identity: None,
            id: (id as u64).to_le_bytes().to_vec(),
            source: "ntfs64".into(),
            kind: if id == 120 { "file" } else { "folder" }.into(),
            size: None,
            created: None,
            modified: None,
            attributes: if id == 120 { 0 } else { 16 },
        })
        .collect::<Vec<_>>();
    objects.extend([5u64, 7].map(|id| Object {
        identity: None,
        id: id.to_le_bytes().to_vec(),
        source: "ntfs64".into(),
        kind: "folder".into(),
        size: None,
        created: None,
        modified: None,
        attributes: 16,
    }));
    let mut entries = (100u8..=120)
        .map(|id| Entry {
            id: vec![id],
            object: (id as u64).to_le_bytes().to_vec(),
            parent: Some(vec![if id == 100 { 2 } else { id - 1 }]),
            name: vec![0x6d4b; 220],
        })
        .collect::<Vec<_>>();
    entries.extend([
        Entry {
            id: vec![1],
            object: 5u64.to_le_bytes().to_vec(),
            parent: None,
            name: vec![92],
        },
        Entry {
            id: vec![2],
            object: 7u64.to_le_bytes().to_vec(),
            parent: Some(vec![1]),
            name: vec![65],
        },
    ]);
    volume
        .write_batch(generation, &objects, &entries, &[], None)
        .unwrap();
    volume.complete(generation).unwrap();
    volume.publish(generation).unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    let revision = sql
        .query_row(
            "SELECT revision FROM generations WHERE id=?1",
            [generation],
            |r| r.get::<_, i64>(0),
        )
        .unwrap();
    reference.revision = revision.to_string();
    reference.generation = generation.to_string();
    reference.entry = vec![120];
    assert!(
        file_operations::resolve(&path, &reference, false)
            .unwrap_err()
            .to_string()
            .contains("单帧")
    );
}
#[test]
fn resolves_raw_utf16_and_preserves_hard_link_paths() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.sqlite");
    let (_db, mut reference) = fixture(&path);
    let file = file_operations::resolve(&path, &reference, false).unwrap();
    let mut expected: Vec<u16> = "资料\\".encode_utf16().collect();
    expected.extend([0xd800, 46, 116, 120, 116]);
    assert_eq!(file.relative, expected);
    assert!(matches!(
        file.identity,
        Some(ExpectedIdentity::Ntfs { frn: 8 })
    ));
    reference.entry = vec![4];
    let link = file_operations::resolve(&path, &reference, false).unwrap();
    assert_eq!(file.object, link.object);
    assert_ne!(file.relative, link.relative);
    reference.entry = vec![2];
    let parent = file_operations::resolve(&path, &reference, false).unwrap();
    assert_eq!(
        file_operations::top_level(vec![file, parent, link]).len(),
        2
    );
}
#[test]
fn rejects_wrong_manifest_stale_revision_root_and_reparse_ancestors() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.sqlite");
    let (_db, reference) = fixture(&path);
    let mut bad = reference.clone();
    bad.filesystem_id = "other".into();
    assert!(file_operations::resolve(&path, &bad, false).is_err());
    bad = reference.clone();
    bad.revision = "999".into();
    assert!(file_operations::resolve(&path, &bad, false).is_err());
    bad = reference.clone();
    bad.entry = vec![1];
    assert!(file_operations::resolve(&path, &bad, false).is_err());
    assert!(
        file_operations::resolve(&path, &bad, true)
            .unwrap()
            .relative
            .is_empty()
    );
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute(
        "UPDATE objects SET attributes=1040 WHERE id=?1",
        [7u64.to_le_bytes().to_vec()],
    )
    .unwrap();
    assert!(
        file_operations::resolve(&path, &reference, false)
            .unwrap_err()
            .to_string()
            .contains("重解析")
    );
}
#[test]
fn missing_scan_identity_stays_explicit_and_malformed_names_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.sqlite");
    let (_db, reference) = fixture(&path);
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute("UPDATE objects SET source='path'", []).unwrap();
    assert!(
        file_operations::resolve(&path, &reference, false)
            .unwrap()
            .identity
            .is_none()
    );
    sql.execute(
        "UPDATE paths SET name=?1 WHERE id=?2",
        rusqlite::params![vec![46u8, 0, 46, 0], vec![3u8]],
    )
    .unwrap();
    assert!(file_operations::resolve(&path, &reference, false).is_err());
}
