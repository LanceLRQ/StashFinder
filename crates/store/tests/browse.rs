use stashfinder_core::browse::DirectoryRequest;
use stashfinder_store::{browse, volume::*};

fn request() -> DirectoryRequest {
    DirectoryRequest {
        partition_id: "p".into(),
        filesystem_id: "fs".into(),
        generation: None,
        revision: None,
        parent: None,
        offset: 0,
        folders_only: false,
    }
}
fn fixture(path: &std::path::Path) -> Volume {
    let mut db = Volume::create(path, r#"{"filesystem_id":"fs","partition_id":"p"}"#).unwrap();
    let g = db.begin_generation().unwrap();
    let objects = [
        (1, "folder", None),
        (2, "folder", None),
        (3, "file", Some(0)),
        (4, "file", None),
    ]
    .map(|(id, kind, size)| Object {
        id: vec![id],
        source: "ntfs".into(),
        kind: kind.into(),
        size,
        created: None,
        modified: None,
        attributes: if kind == "folder" { 16 } else { 0 },
    });
    let entries = [
        (1, 1, None, "\\"),
        (2, 2, Some(1), "资料"),
        (3, 3, Some(1), "zero.txt"),
        (4, 4, Some(2), "unknown.txt"),
        (5, 3, Some(2), "linked.txt"),
    ]
    .map(|(id, object, parent, name)| Entry {
        id: vec![id],
        object: vec![object],
        parent: parent.map(|p| vec![p]),
        name: name.encode_utf16().collect(),
    });
    db.write_batch(
        g,
        &objects,
        &entries,
        &[],
        Some(&Cursor {
            journal: vec![1],
            next: 1,
        }),
    )
    .unwrap();
    db.complete(g).unwrap();
    db.publish(g).unwrap();
    db
}
#[test]
fn reads_direct_children_and_preserves_link_identity_without_source_io() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.sqlite");
    let _db = fixture(&path);
    let mut req = request();
    let root = browse::read(&path, &req).unwrap();
    assert_eq!(
        root.entries
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        ["资料", "zero.txt"]
    );
    assert_eq!(root.entries[1].size.as_deref(), Some("0"));
    req.parent = Some(vec![2]);
    let child = browse::read(&path, &req).unwrap();
    assert_eq!(
        child
            .ancestors
            .iter()
            .map(|a| a.entry.clone())
            .collect::<Vec<_>>(),
        [vec![1], vec![2]]
    );
    assert_eq!(child.entries[0].entry, vec![5]);
    assert_eq!(child.entries[0].object, vec![3]);
    assert_eq!(child.entries[0].path, "资料\\linked.txt");
    assert_eq!(child.entries[1].size, None);
    req.parent = None;
    req.folders_only = true;
    assert_eq!(browse::read(&path, &req).unwrap().entries.len(), 1);
    req.filesystem_id = "other".into();
    assert!(browse::read(&path, &req).is_err());
    req = request();
    req.parent = Some(vec![99]);
    assert!(
        browse::read(&path, &req)
            .unwrap_err()
            .to_string()
            .contains("目录已不存在")
    );
}
#[test]
fn rejects_pages_after_incremental_update_or_generation_switch_and_migrates_v2() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.sqlite");
    let mut db = fixture(&path);
    let mut req = request();
    let before = browse::read(&path, &req).unwrap();
    req.generation = Some(before.generation.clone());
    req.revision = Some(before.revision.clone());
    db.apply_incremental(
        before.generation.parse().unwrap(),
        Changes {
            objects: &[],
            entries: &[],
            removed: &[vec![2]],
            pending: &[],
            cursor: Some(&Cursor {
                journal: vec![1],
                next: 2,
            }),
            replace_links: false,
        },
    )
    .unwrap();
    assert!(
        browse::read(&path, &req)
            .unwrap_err()
            .to_string()
            .contains("目录索引已更新")
    );
    req = request();
    req.parent = Some(vec![2]);
    assert!(browse::read(&path, &req).is_err());
    let g = db.begin_generation().unwrap();
    db.complete(g).unwrap();
    db.publish(g).unwrap();
    req = request();
    req.generation = Some(before.generation);
    req.revision = Some(before.revision);
    assert!(
        browse::read(&path, &req)
            .unwrap_err()
            .to_string()
            .contains("目录索引已更新")
    );
    drop(db);
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch("ALTER TABLE generations DROP COLUMN revision; PRAGMA user_version=2;")
        .unwrap();
    drop(sql);
    let _db = Volume::open(&path).unwrap();
    let sql = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        sql.query_row("SELECT revision FROM generations WHERE id=?1", [g], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
}
#[test]
fn paginates_without_losing_children_and_bounds_the_wire_response() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.sqlite");
    let mut db = fixture(&path);
    let g = db.begin_generation().unwrap();
    let objects = (0u16..110)
        .map(|i| Object {
            id: i.to_le_bytes().to_vec(),
            source: "scan".into(),
            kind: if i == 0 { "folder" } else { "file" }.into(),
            size: Some(0),
            created: None,
            modified: None,
            attributes: 0,
        })
        .collect::<Vec<_>>();
    let entries = (0u16..110)
        .map(|i| Entry {
            id: i.to_le_bytes().to_vec(),
            object: i.to_le_bytes().to_vec(),
            parent: if i == 0 { None } else { Some(vec![0, 0]) },
            name: format!("file-{i:03}").encode_utf16().collect(),
        })
        .collect::<Vec<_>>();
    db.write_batch(g, &objects, &entries, &[], None).unwrap();
    db.complete(g).unwrap();
    db.publish_snapshot(g).unwrap();
    let mut req = request();
    let mut count = 0;
    loop {
        let page = browse::read(&path, &req).unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() < stashfinder_core::MAX_FRAME - 128);
        count += page.entries.len();
        req.generation = Some(page.generation);
        req.revision = Some(page.revision);
        match page.next {
            Some(n) => req.offset = n,
            None => break,
        }
    }
    assert_eq!(count, 109);
}
