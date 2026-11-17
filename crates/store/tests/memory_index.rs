#![cfg(feature = "memory-index-bench")]

use stashfinder_core::search::Query;
use stashfinder_store::{
    search::SearchIndex,
    volume::{Cursor, Entry, Object, Volume},
};
use std::sync::atomic::AtomicBool;

#[test]
fn a_searchable_memory_volume_is_published_as_a_new_reopenable_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("index.sqlite");
    let mut db = Volume::create_in_memory("{}").unwrap();
    let generation = db.begin_generation().unwrap();
    db.begin_initial_paths(generation).unwrap();
    let object = |key, kind: &str, size| Object {
        identity: None,
        id: vec![key],
        source: "ntfs64".into(),
        kind: kind.into(),
        size,
        created: Some(1u64.to_le_bytes()),
        modified: Some(2u64.to_le_bytes()),
        attributes: 0,
    };
    db.write_initial_batch(
        generation,
        &[object(1, "folder", None), object(2, "file", Some(42))],
        &[
            Entry {
                id: vec![1],
                object: vec![1],
                parent: None,
                name: vec![92],
            },
            Entry {
                id: vec![2],
                object: vec![2],
                parent: Some(vec![1]),
                name: "见证.txt".encode_utf16().collect(),
            },
        ],
    )
    .unwrap();
    db.finish_initial_paths(generation, &[1]).unwrap();
    db.write_batch(
        generation,
        &[],
        &[],
        &[],
        Some(&Cursor {
            journal: vec![7; 8],
            next: 101,
        }),
    )
    .unwrap();
    db.complete(generation).unwrap();
    let cancel = AtomicBool::new(false);
    let index = SearchIndex::load_candidate(&db, generation, &cancel).unwrap();
    let index = db.publish_index(index, false).unwrap();
    let query = Query::parse("见证", false).unwrap();
    assert_eq!(
        index.search(&query, &[], 1, &cancel).unwrap().rows[0].name,
        "见证.txt"
    );
    assert!(!path.exists());
    assert!(
        db.backup_completed_index(&path, &AtomicBool::new(true), &mut || Ok(()))
            .is_err()
    );
    assert!(!path.exists());
    let during_backup = AtomicBool::new(false);
    assert!(
        db.backup_completed_index(&path, &during_backup, &mut || {
            during_backup.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        })
        .is_err()
    );
    assert!(!path.exists());
    assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);

    db.backup_completed_index(&path, &cancel, &mut || {
        assert!(!path.exists());
        Ok(())
    })
    .unwrap();
    assert!(
        db.backup_completed_index(&path, &cancel, &mut || Ok(()))
            .is_err()
    );
    drop(db);
    let offline = Volume::open(&path).unwrap();
    assert_eq!(offline.cursor(generation).unwrap().unwrap().next, 101);
    let index = SearchIndex::load(&offline).unwrap().unwrap();
    assert_eq!(
        index.search(&query, &[], 1, &cancel).unwrap().rows[0]
            .size
            .as_deref(),
        Some("42")
    );
}

#[test]
fn direct_search_preserves_raw_names_order_and_memory_batch_rollback() {
    let objects = vec![
        Object {
            id: vec![1],
            identity: None,
            source: "ntfs64".into(),
            kind: "folder".into(),
            size: None,
            created: Some(u64::MAX.to_le_bytes()),
            modified: Some(2u64.to_le_bytes()),
            attributes: 16,
        },
        Object {
            id: vec![2],
            identity: None,
            source: "ntfs64".into(),
            kind: "file".into(),
            size: Some(42),
            created: Some(u64::MAX.to_le_bytes()),
            modified: Some(2u64.to_le_bytes()),
            attributes: 32,
        },
    ];
    let paths = vec![
        Entry {
            id: vec![4],
            object: vec![2],
            parent: Some(vec![1]),
            name: vec![0x4e2d, 0xd800, 1],
        },
        Entry {
            id: vec![1],
            object: vec![1],
            parent: None,
            name: vec![92],
        },
        Entry {
            id: vec![3],
            object: vec![2],
            parent: Some(vec![1]),
            name: "Alias.JS".encode_utf16().collect(),
        },
    ];
    let cancel = AtomicBool::new(false);
    let direct = SearchIndex::from_initial(&objects, &paths, &cancel).unwrap();
    let query = Query::parse("", false).unwrap();
    let before = direct.search(&query, &[], 100, &cancel).unwrap().rows;
    assert_eq!(
        before
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>(),
        ["Alias.JS", "中\\u{d800}\\u{0001}"]
    );
    assert_eq!(before[1].created.as_deref(), Some("18446744073709551615"));
    let mut db = Volume::create_in_memory("{}").unwrap();
    let generation = db.begin_generation().unwrap();
    db.begin_initial_paths(generation).unwrap();
    db.write_initial_batch(generation, &objects[..1], &[])
        .unwrap();
    assert!(
        db.write_initial_batch(generation, &objects, &paths)
            .is_err()
    );
    assert_eq!(db.object_count(generation).unwrap(), 1);
    db.write_initial_batch(generation, &objects[1..], &paths)
        .unwrap();
    db.finish_initial_paths(generation, &[1]).unwrap();
    db.complete(generation).unwrap();
    db.publish(generation).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("index.sqlite");
    db.backup_completed_index(&file, &cancel, &mut || Ok(()))
        .unwrap();
    let reopened = Volume::open(&file).unwrap();
    let loaded = SearchIndex::load(&reopened).unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(loaded.search(&query, &[], 100, &cancel).unwrap().rows).unwrap()
    );
}

#[test]
fn bulk_snapshot_failure_does_not_publish_and_valid_data_can_be_retried() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("index.sqlite");
    let objects = [Object {
        id: vec![1],
        identity: None,
        source: "ntfs64".into(),
        kind: "folder".into(),
        size: None,
        created: Some([0; 8]),
        modified: Some([0; 8]),
        attributes: 16,
    }];
    let mut paths = [Entry {
        id: vec![1],
        object: vec![9],
        parent: None,
        name: vec![92],
    }];
    let cursor = Cursor {
        journal: vec![7; 8],
        next: 42,
    };
    let cancel = AtomicBool::new(false);
    assert!(
        Volume::persist_initial_snapshot(
            &objects,
            &paths,
            &[1],
            &cursor,
            &file,
            &cancel,
            &mut || Ok(())
        )
        .is_err()
    );
    assert!(!file.exists());
    paths[0].object = vec![1];
    Volume::persist_initial_snapshot(&objects, &paths, &[1], &cursor, &file, &cancel, &mut || {
        Ok(())
    })
    .unwrap();
    let reopened = Volume::open(&file).unwrap();
    assert_eq!(
        reopened
            .cursor(reopened.active_generation().unwrap().unwrap())
            .unwrap()
            .unwrap()
            .next,
        42
    );
    let sql = rusqlite::Connection::open(&file).unwrap();
    assert_eq!(
        sql.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    assert!(
        Volume::persist_initial_snapshot(
            &objects,
            &paths,
            &[1],
            &cursor,
            &file,
            &cancel,
            &mut || Ok(())
        )
        .is_err()
    );
}

#[test]
fn an_unpublished_generation_cannot_be_backed_up() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("index.sqlite");
    let mut db = Volume::create_in_memory("{}").unwrap();
    db.begin_generation().unwrap();
    assert!(
        db.backup_completed_index(&path, &AtomicBool::new(false), &mut || Ok(()))
            .is_err()
    );
    assert!(!path.exists());
}

#[test]
fn snapshot_preserves_links_and_prunes_reparse_descendants() {
    let objects: Vec<_> = (1..=5)
        .map(|id| Object {
            id: vec![id],
            identity: None,
            source: "ntfs64".into(),
            kind: if id < 4 { "folder" } else { "file" }.into(),
            size: (id >= 4).then_some(42),
            created: Some(u64::MAX.to_le_bytes()),
            modified: Some(2u64.to_le_bytes()),
            attributes: if id == 2 {
                0x410
            } else if id < 4 {
                16
            } else {
                32
            },
        })
        .collect();
    let paths: Vec<_> = [
        (5, 5, Some(3)),
        (4, 4, Some(3)),
        (6, 4, Some(1)),
        (3, 3, Some(2)),
        (2, 2, Some(1)),
        (1, 1, None),
    ]
    .into_iter()
    .map(|(id, object, parent)| Entry {
        id: vec![id],
        object: vec![object],
        parent: parent.map(|id| vec![id]),
        name: vec![id as u16, 0xd800],
    })
    .collect();
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("snapshot.sqlite");
    let cursor = Cursor {
        journal: vec![7; 8],
        next: 101,
    };
    Volume::persist_initial_snapshot(
        &objects,
        &paths,
        &[1],
        &cursor,
        &file,
        &AtomicBool::new(false),
        &mut || Ok(()),
    )
    .unwrap();
    let db = Volume::open(&file).unwrap();
    let generation = db.active_generation().unwrap().unwrap();
    assert_eq!(db.object_count(generation).unwrap(), 3);
    for id in 1..=6 {
        assert_eq!(
            db.has_path(generation, &[id]).unwrap(),
            [1, 2, 6].contains(&id)
        );
    }
    let sql = rusqlite::Connection::open(&file).unwrap();
    let name: Vec<u8> = sql
        .query_row("SELECT name FROM paths WHERE id=x'06'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(name, [6, 0, 0, 0xd8]);
    assert_eq!(db.cursor(generation).unwrap().unwrap().next, 101);
}

#[test]
fn snapshot_rejects_invalid_graphs_without_publishing() {
    for case in 0..12 {
        let mut objects: Vec<_> = (1..=3)
            .map(|id| Object {
                id: vec![id],
                identity: None,
                source: "ntfs64".into(),
                kind: "folder".into(),
                size: None,
                created: Some([0; 8]),
                modified: Some([0; 8]),
                attributes: 16,
            })
            .collect();
        let mut paths: Vec<_> = (1..=3)
            .map(|id| Entry {
                id: vec![id],
                object: vec![id],
                parent: (id != 1).then(|| vec![1]),
                name: vec![id as u16],
            })
            .collect();
        match case {
            0 => paths[1].parent = Some(vec![99]),
            1 => paths[1].object = vec![99],
            2 => paths[1].name = paths[2].name.clone(),
            3 => paths[1].id = paths[2].id.clone(),
            4 => objects[1].id = objects[2].id.clone(),
            5 => paths[1].parent = None,
            6 => {
                paths[1].parent = Some(vec![3]);
                paths[2].parent = Some(vec![2]);
            }
            7 => {
                objects[1].kind = "file".into();
                objects[1].size = Some(0);
                paths[2].parent = Some(vec![2]);
            }
            8 => objects[0].kind = "file".into(),
            9 => paths[1].name.clear(),
            10 => objects[1].created = None,
            11 => {
                objects[1].kind = "file".into();
                objects[1].size = Some(-1);
            }
            _ => unreachable!(),
        }
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("snapshot.sqlite");
        assert!(
            Volume::persist_initial_snapshot(
                &objects,
                &paths,
                &[1],
                &Cursor {
                    journal: vec![7; 8],
                    next: 101
                },
                &file,
                &AtomicBool::new(false),
                &mut || Ok(())
            )
            .is_err(),
            "case {case}"
        );
        assert!(!file.exists(), "case {case}");
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }
}

#[test]
fn snapshot_write_cancellation_and_callback_failure_leave_no_published_file() {
    let objects: Vec<_> = (0..9000u64)
        .map(|id| Object {
            id: id.to_le_bytes().to_vec(),
            identity: None,
            source: "ntfs64".into(),
            kind: if id == 0 { "folder" } else { "file" }.into(),
            size: (id != 0).then_some(0),
            created: Some([0; 8]),
            modified: Some([0; 8]),
            attributes: if id == 0 { 16 } else { 32 },
        })
        .collect();
    let entries: Vec<_> = objects
        .iter()
        .enumerate()
        .map(|(id, object)| Entry {
            id: object.id.clone(),
            object: object.id.clone(),
            parent: (id != 0).then(|| vec![0; 8]),
            name: id.to_string().encode_utf16().collect(),
        })
        .collect();
    for fail in [false, true] {
        for stop in [1, 2, 5] {
            let tmp = tempfile::tempdir().unwrap();
            let file = tmp.path().join("snapshot.sqlite");
            let cancelled = AtomicBool::new(false);
            let mut progress = 0;
            let result = Volume::persist_initial_snapshot(
                &objects,
                &entries,
                &[0; 8],
                &Cursor {
                    journal: vec![7; 8],
                    next: 101,
                },
                &file,
                &cancelled,
                &mut || {
                    progress += 1;
                    if progress == stop {
                        if fail {
                            anyhow::bail!("injected callback failure");
                        }
                        cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    Ok(())
                },
            );
            assert!(result.is_err(), "stop {stop}, fail {fail}");
            assert_eq!(progress, stop, "unexpected early failure: {result:?}");
            assert!(!file.exists());
            assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
        }
    }
}
