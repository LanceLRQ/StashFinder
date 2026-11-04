use stashfinder_store::volume::*;

#[test]
fn failed_batch_does_not_advance_cursor_or_leave_objects() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("volume.sqlite");
    let mut db = Volume::create(&path, "{}").unwrap();
    let generation = db.begin_generation().unwrap();
    let object = Object {
        id: vec![1; 8],
        source: "ntfs".into(),
        kind: "file".into(),
        size: None,
        created: None,
        modified: None,
        attributes: 0,
    };
    let bad_path = Entry {
        id: vec![1],
        object: vec![2; 8],
        parent: None,
        name: vec![0x4e2d],
    };
    assert!(
        db.write_batch(
            generation,
            std::slice::from_ref(&object),
            &[bad_path],
            &[],
            Some(&Cursor {
                journal: vec![1; 8],
                next: 100
            })
        )
        .is_err()
    );
    assert_eq!(db.object_count(generation).unwrap(), 0);
    assert!(db.cursor(generation).unwrap().is_none());
    let entries = [
        Entry {
            id: vec![1],
            object: object.id.clone(),
            parent: None,
            name: vec![0x4e2d],
        },
        Entry {
            id: vec![2],
            object: object.id.clone(),
            parent: None,
            name: vec![0xd800],
        },
    ];
    db.write_batch(
        generation,
        std::slice::from_ref(&object),
        &entries,
        &[vec![1; 8]],
        Some(&Cursor {
            journal: vec![1; 8],
            next: 100,
        }),
    )
    .unwrap();
    assert!(db.publish(generation).is_err());
    db.write_batch(generation, &[object], &[], &[], None)
        .unwrap();
    db.complete(generation).unwrap();
    db.publish(generation).unwrap();
    drop(db);
    let db = Volume::open(&path).unwrap();
    assert_eq!(db.active_generation().unwrap(), Some(generation));
    assert_eq!(db.object_count(generation).unwrap(), 1);
    assert_eq!(db.cursor(generation).unwrap().unwrap().next, 100);
    let sql = rusqlite::Connection::open(path).unwrap();
    let size: Option<i64> = sql
        .query_row("SELECT size FROM objects", [], |r| r.get(0))
        .unwrap();
    assert_eq!(size, None);
    let violations: i64 = sql
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(violations, 0);
    assert_eq!(
        sql.query_row("SELECT count(*) FROM paths", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        sql.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        sql.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
}

#[test]
fn published_generation_is_immutable_and_failed_successor_keeps_it_active() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&root.path().join("v.sqlite"), "{}").unwrap();
    let old = db.begin_generation().unwrap();
    db.complete(old).unwrap();
    db.publish(old).unwrap();
    assert!(
        db.write_batch(
            old,
            &[],
            &[],
            &[],
            Some(&Cursor {
                journal: vec![1],
                next: 9
            })
        )
        .is_err()
    );
    assert!(db.write_batch(999, &[], &[], &[], None).is_err());
    let next = db.begin_generation().unwrap();
    assert!(db.publish(next).is_err());
    assert_eq!(db.active_generation().unwrap(), Some(old));
}

#[test]
fn missing_size_is_distinct_from_an_empty_file_in_the_persisted_schema() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("v.sqlite");
    let mut db = Volume::create(&path, "{}").unwrap();
    let generation = db.begin_generation().unwrap();
    let objects = [None, Some(0)]
        .into_iter()
        .enumerate()
        .map(|(i, size)| Object {
            id: vec![i as u8],
            source: "scan".into(),
            kind: "file".into(),
            size,
            created: None,
            modified: None,
            attributes: 0,
        })
        .collect::<Vec<_>>();
    db.write_batch(generation, &objects, &[], &[], None)
        .unwrap();
    drop(db);
    let db = rusqlite::Connection::open(path).unwrap();
    let sizes: (i64, i64) = db
        .query_row(
            "SELECT sum(size IS NULL),sum(size=0) FROM objects",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(sizes, (1, 1));
    let not_null: i64 = db
        .query_row(
            "SELECT [notnull] FROM pragma_table_info('objects') WHERE name='size'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(not_null, 0);
}
