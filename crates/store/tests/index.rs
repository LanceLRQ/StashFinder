use stashfinder_core::search::Query;
use stashfinder_store::{search::SearchIndex, volume::*};
use std::sync::atomic::AtomicBool;

fn object(id: u8, folder: bool, size: Option<i64>) -> Object {
    Object {
        id: vec![id],
        source: "ntfs".into(),
        kind: if folder { "folder" } else { "file" }.into(),
        size,
        created: None,
        modified: None,
        attributes: if folder { 16 } else { 0 },
    }
}
fn entry(id: u8, object: u8, parent: Option<u8>, name: &str) -> Entry {
    Entry {
        id: vec![id],
        object: vec![object],
        parent: parent.map(|v| vec![v]),
        name: name.encode_utf16().collect(),
    }
}

#[test]
fn published_paths_and_hard_links_survive_restart_without_a_source_disk() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("volume.sqlite");
    let mut db = Volume::create(&file, "{}").unwrap();
    let g = db.begin_generation().unwrap();
    db.write_batch(
        g,
        &[
            object(1, true, None),
            object(2, true, None),
            object(3, false, Some(0)),
        ],
        &[
            entry(1, 1, None, "\\"),
            entry(2, 2, Some(1), "aaa"),
            entry(3, 3, Some(2), "bbb.txt"),
            entry(4, 3, Some(1), "second.txt"),
        ],
        &[],
        None,
    )
    .unwrap();
    db.complete(g).unwrap();
    db.publish(g).unwrap();
    drop(db);
    let db = Volume::open(&file).unwrap();
    let index = SearchIndex::load(&db).unwrap().unwrap();
    let results = index
        .search(
            &Query::parse("aaa bbb", false).unwrap(),
            &[],
            200,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(results.rows.len(), 1);
    assert_eq!(results.rows[0].path, r"aaa\bbb.txt");
    assert_eq!(results.rows[0].size.as_deref(), Some("0"));
    let all = index
        .search(
            &Query::parse(".txt", false).unwrap(),
            &[],
            200,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(all.rows.len(), 2);
    assert_eq!(all.rows[0].object, all.rows[1].object);
    assert_eq!(db.index_status().unwrap().phase, "ready");
}

#[test]
fn interrupted_successor_keeps_old_searchable_generation_and_reports_failure() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&dir.path().join("v.sqlite"), "{}").unwrap();
    let old = db.begin_generation().unwrap();
    db.write_batch(
        old,
        &[object(1, true, None), object(2, false, None)],
        &[entry(1, 1, None, "\\"), entry(2, 2, Some(1), "old.txt")],
        &[],
        None,
    )
    .unwrap();
    db.complete(old).unwrap();
    db.publish(old).unwrap();
    let next = db.begin_generation().unwrap();
    db.fail_generation(next, "设备已断开").unwrap();
    assert_eq!(db.index_status().unwrap().phase, "failed");
    assert_eq!(db.active_generation().unwrap(), Some(old));
    let index = SearchIndex::load(&db).unwrap().unwrap();
    let found = index
        .search(
            &Query::parse("old", false).unwrap(),
            &[],
            200,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(found.rows[0].size, None);
    assert!(db.publish(next).is_err());
    assert!(
        index
            .search(
                &Query::parse("", false).unwrap(),
                &[],
                200,
                &AtomicBool::new(true)
            )
            .is_err()
    );
}

#[test]
fn directory_rename_and_result_limit_have_explicit_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&dir.path().join("v.sqlite"), "{}").unwrap();
    let g = db.begin_generation().unwrap();
    db.write_batch(
        g,
        &[
            object(1, true, None),
            object(2, true, None),
            object(3, false, Some(7)),
        ],
        &[
            entry(1, 1, None, "\\"),
            entry(2, 2, Some(1), "old"),
            entry(3, 3, Some(2), "child.txt"),
        ],
        &[],
        None,
    )
    .unwrap();
    db.write_batch(g, &[], &[entry(2, 2, Some(1), "renamed")], &[], None)
        .unwrap();
    db.complete(g).unwrap();
    db.publish(g).unwrap();
    let index = SearchIndex::load(&db).unwrap().unwrap();
    let found = index
        .search(
            &Query::parse("renamed child", false).unwrap(),
            &[],
            1,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(found.rows[0].path, r"renamed\child.txt");
    assert!(!found.more);
    let limited = index
        .search(
            &Query::parse("", false).unwrap(),
            &[],
            1,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(limited.rows.len(), 1);
    assert!(limited.more);
}

#[test]
fn m1_database_migrates_without_losing_manifest_capacity_or_active_records() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("old.sqlite");
    let sql = rusqlite::Connection::open(&file).unwrap();
    sql.execute_batch("CREATE TABLE manifest(id INTEGER PRIMARY KEY,value TEXT NOT NULL);
        INSERT INTO manifest VALUES(1,'original');
        CREATE TABLE generations(id INTEGER PRIMARY KEY,complete INTEGER NOT NULL DEFAULT 0,journal BLOB,next_usn INTEGER);
        INSERT INTO generations VALUES(1,1,NULL,NULL);
        CREATE TABLE index_state(id INTEGER PRIMARY KEY,active_generation INTEGER);
        INSERT INTO index_state VALUES(1,1);
        CREATE TABLE capacity_snapshot(id INTEGER PRIMARY KEY,total INTEGER,free INTEGER,sampled_at INTEGER);
        INSERT INTO capacity_snapshot VALUES(1,1000,300,123);
        CREATE TABLE objects(generation INTEGER,id BLOB,source TEXT,kind TEXT,size INTEGER,created BLOB,modified BLOB,PRIMARY KEY(generation,id));
        CREATE TABLE paths(generation INTEGER,id BLOB,object BLOB,parent BLOB,name BLOB,PRIMARY KEY(generation,id));
        CREATE TABLE pending_metadata(generation INTEGER,object BLOB,PRIMARY KEY(generation,object));
        INSERT INTO objects VALUES(1,X'01','ntfs','folder',NULL,NULL,NULL);
        INSERT INTO objects VALUES(1,X'02','ntfs','file',7,NULL,NULL);
        INSERT INTO paths VALUES(1,X'01',X'01',NULL,X'5C00');
        INSERT INTO paths VALUES(1,X'02',X'02',X'01',X'61002E00740078007400');
        PRAGMA user_version=1;").unwrap();
    drop(sql);
    let db = Volume::open(&file).unwrap();
    assert_eq!(db.manifest().unwrap(), "original");
    assert_eq!(db.capacity().unwrap().unwrap().free, 300);
    assert_eq!(db.index_status().unwrap().phase, "ready");
    let index = SearchIndex::load(&db).unwrap().unwrap();
    let found = index
        .search(
            &Query::parse("a.txt", false).unwrap(),
            &[],
            200,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(found.rows[0].path, "a.txt");
    assert_eq!(found.rows[0].size.as_deref(), Some("7"));
    drop(db);
    let sql = rusqlite::Connection::open(file).unwrap();
    assert_eq!(
        sql.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        sql.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}

#[test]
fn invalid_utf16_is_preserved_and_displayed_as_an_escape() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&dir.path().join("v.sqlite"), "{}").unwrap();
    let g = db.begin_generation().unwrap();
    let mut bad = entry(2, 2, Some(1), "x");
    bad.name = vec![0xd800];
    db.write_batch(
        g,
        &[object(1, true, None), object(2, false, None)],
        &[entry(1, 1, None, "\\"), bad],
        &[],
        None,
    )
    .unwrap();
    db.complete(g).unwrap();
    db.publish(g).unwrap();
    let index = SearchIndex::load(&db).unwrap().unwrap();
    let found = index
        .search(
            &Query::parse("", false).unwrap(),
            &[],
            200,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(found.rows[0].name, r"\u{d800}");
}

#[test]
fn failed_incremental_transaction_preserves_paths_pending_and_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&dir.path().join("v.sqlite"), "{}").unwrap();
    let g = db.begin_generation().unwrap();
    db.write_batch(
        g,
        &[object(1, true, None), object(2, false, Some(7))],
        &[entry(1, 1, None, "\\"), entry(2, 2, Some(1), "old.txt")],
        &[],
        Some(&Cursor {
            journal: vec![1],
            next: 100,
        }),
    )
    .unwrap();
    db.complete(g).unwrap();
    db.publish(g).unwrap();
    assert!(
        db.apply_incremental(
            g,
            Changes {
                objects: &[object(2, false, Some(99))],
                entries: &[entry(2, 2, Some(99), "bad.txt")],
                removed: &[],
                pending: &[vec![2]],
                cursor: Some(&Cursor {
                    journal: vec![1],
                    next: 200
                }),
                replace_links: true
            }
        )
        .is_err()
    );
    assert_eq!(db.cursor(g).unwrap().unwrap().next, 100);
    let hits = SearchIndex::load(&db)
        .unwrap()
        .unwrap()
        .search(
            &Query::parse("old", false).unwrap(),
            &[],
            200,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(hits.rows[0].size.as_deref(), Some("7"));
    db.publish(g).unwrap();
}
