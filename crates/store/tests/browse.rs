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
        sort: Default::default(),
        filter: None,
    }
}

#[test]
fn filters_direct_children_before_count_sort_and_paging_with_search_semantics() {
    use stashfinder_core::search::{Query, QueryOptions};
    use stashfinder_store::search::{
        SearchIndex,
        query::{ResultSet, Source},
    };
    use std::sync::{Arc, atomic::AtomicBool};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filtered.sqlite");
    let mut db = Volume::create(&path, r#"{"filesystem_id":"fs","partition_id":"p"}"#).unwrap();
    let g = db.begin_generation().unwrap();
    let objects = (0..=242u8)
        .map(|id| Object {
            identity: None,
            id: vec![id],
            source: "scan".into(),
            kind: if id == 0 || id == 241 {
                "folder"
            } else {
                "file"
            }
            .into(),
            size: (id % 19 != 0).then_some(i64::from(id)),
            created: Some(u64::from(id).to_le_bytes()),
            modified: None,
            attributes: 0,
        })
        .collect::<Vec<_>>();
    let entries = (0..=242u8)
        .map(|id| Entry {
            id: vec![id],
            object: vec![id],
            parent: (id != 0).then_some(vec![if id == 242 { 241 } else { 0 }]),
            name: format!("file-{id:03}.{}", if id % 2 == 0 { "JPG" } else { "txt" })
                .encode_utf16()
                .collect(),
        })
        .collect::<Vec<_>>();
    db.write_batch(g, &objects, &entries, &[], None).unwrap();
    db.complete(g).unwrap();
    db.publish_snapshot(g).unwrap();

    let filter = serde_json::json!({"extensions":["jpg"],"kind":"file","size":{"min":"8","max":null},"created":{"min":"10","max":null}});
    let mut input = serde_json::to_value(request()).unwrap();
    input["filter"] = filter.clone();
    let mut req: DirectoryRequest = serde_json::from_value(input).unwrap();
    let first = browse::read(&path, &req).unwrap();
    req.generation = Some(first.generation.clone());
    req.revision = Some(first.revision.clone());
    req.offset = first
        .next
        .expect("matching children spill into the next page");
    let last = browse::read(&path, &req).unwrap();
    let actual = first
        .entries
        .iter()
        .chain(&last.entries)
        .map(|r| r.entry[0])
        .collect::<Vec<_>>();
    let expected = (10..=240u8)
        .filter(|id| id % 2 == 0 && id % 19 != 0)
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
    assert_eq!(
        (first.total, last.total, last.next),
        (expected.len(), expected.len(), None)
    );

    let mut options: QueryOptions = serde_json::from_value(filter).unwrap();
    options.scope = Some(stashfinder_core::search::Scope {
        partition_id: "p".into(),
        entry: vec![0],
        recursive: false,
    });
    let query = Query::compile("", false, &options).unwrap();
    let search = ResultSet::run(
        vec![Source {
            id: "p".into(),
            name: "P".into(),
            filesystem: "fs".into(),
            connection: "offline".into(),
            mounts: vec![],
            index: Arc::new(SearchIndex::load(&db).unwrap().unwrap()),
            needs_validation: false,
        }],
        &query,
        &options,
        &AtomicBool::new(false),
    )
    .unwrap();
    let hits = search
        .page(0, 100)
        .unwrap()
        .into_iter()
        .chain(search.page(100, 100).unwrap())
        .map(|r| r.file.entry[0])
        .collect::<Vec<_>>();
    assert_eq!(actual, hits);
    let selection = stashfinder_store::file_operations::directory_refs(
        &path,
        &stashfinder_core::file_operations::EntryRef {
            partition_id: "p".into(),
            filesystem_id: "fs".into(),
            generation: first.generation,
            revision: first.revision,
            entry: vec![0],
        },
        true,
        req.filter.as_deref(),
    )
    .unwrap();
    assert_eq!(
        selection.iter().map(|row| row.entry[0]).collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn directory_creation_sort_applies_before_paging_and_keeps_unknown_dates_last() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sorted.sqlite");
    let mut db = Volume::create(&path, r#"{"filesystem_id":"fs","partition_id":"p"}"#).unwrap();
    let g = db.begin_generation().unwrap();
    let objects: Vec<_> = (0u8..=103)
        .map(|id| Object {
            identity: None,
            id: vec![id],
            source: "ntfs".into(),
            kind: if id == 0 { "folder" } else { "file" }.into(),
            size: None,
            created: (id < 103).then(|| (u64::from(id) * 128).to_le_bytes()),
            modified: None,
            attributes: 0,
        })
        .collect();
    let entries: Vec<_> = (0u8..=103)
        .map(|id| Entry {
            id: vec![id],
            object: vec![id],
            parent: (id != 0).then_some(vec![0]),
            name: format!("file-{id:03}.txt").encode_utf16().collect(),
        })
        .collect();
    db.write_batch(g, &objects, &entries, &[], None).unwrap();
    db.complete(g).unwrap();
    db.publish(g).unwrap();
    let mut json = serde_json::to_value(request()).unwrap();
    json["sort"] = serde_json::json!({"field":"created","descending":true});
    let mut sorted: DirectoryRequest = serde_json::from_value(json).unwrap();
    let first = browse::read(&path, &sorted).unwrap();
    sorted.offset = first.next.unwrap();
    sorted.generation = Some(first.generation);
    sorted.revision = Some(first.revision);
    let last = browse::read(&path, &sorted).unwrap();
    assert_eq!(
        first
            .entries
            .iter()
            .chain(&last.entries)
            .map(|r| r.entry[0])
            .collect::<Vec<_>>(),
        (1..=102).rev().chain([103]).collect::<Vec<_>>()
    );
}

#[test]
fn a_failed_initial_generation_without_paths_still_has_no_browsable_index() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.sqlite");
    let mut db = Volume::create(&path, r#"{"filesystem_id":"fs","partition_id":"p"}"#).unwrap();
    let generation = db.begin_generation().unwrap();
    db.begin_initial_paths(generation).unwrap();
    db.discard_initial(generation, "fixture conflict").unwrap();
    assert_eq!(
        browse::read(&path, &request()).unwrap_err().to_string(),
        "尚无目录索引"
    );
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
        identity: None,
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
    assert_eq!(root.total, 2);
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
    sql.execute_batch("DROP TABLE native_identity; ALTER TABLE generations DROP COLUMN revision; PRAGMA user_version=2;")
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
            identity: None,
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
