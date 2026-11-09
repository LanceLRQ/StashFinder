use rusqlite::Connection;
use stashfinder_core::file_operations::NativeIdentity;
use stashfinder_store::volume::*;

fn object(id: u8) -> Object {
    Object {
        id: vec![id],
        source: "path".into(),
        kind: "file".into(),
        size: Some(0),
        created: None,
        modified: None,
        attributes: 0,
        identity: Some(NativeIdentity {
            volume: 123,
            file: 456,
        }),
    }
}
#[test]
fn identity_and_metadata_commit_and_rollback_together() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.sqlite");
    let mut volume = Volume::create(&path, "{}").unwrap();
    let g = volume.begin_generation().unwrap();
    let row = object(1);
    let bad = Entry {
        id: vec![2],
        object: vec![99],
        parent: None,
        name: vec![65],
    };
    assert!(
        volume
            .write_batch(g, std::slice::from_ref(&row), &[bad], &[], None)
            .is_err()
    );
    let db = Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM native_identity", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(volume.object_count(g).unwrap(), 0);
    volume.write_batch(g, &[row], &[], &[], None).unwrap();
    assert_eq!(
        volume.native_identity(g, &[1]).unwrap(),
        Some(NativeIdentity {
            volume: 123,
            file: 456
        })
    );
    let mut unknown = object(1);
    unknown.identity = None;
    volume.write_batch(g, &[unknown], &[], &[], None).unwrap();
    assert_eq!(volume.native_identity(g, &[1]).unwrap(), None);
}
#[test]
fn migration_preserves_v3_rows_and_failure_leaves_v3_intact() {
    for fail in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.sqlite");
        let mut volume = Volume::create(&path, "manifest").unwrap();
        let g = volume.begin_generation().unwrap();
        let mut row = object(1);
        row.identity = None;
        volume.write_batch(g, &[row], &[], &[], None).unwrap();
        drop(volume);
        let db = Connection::open(&path).unwrap();
        db.execute_batch("DROP TABLE native_identity; PRAGMA user_version=3;")
            .unwrap();
        if fail {
            db.execute_batch("CREATE VIEW native_identity AS SELECT 1;")
                .unwrap();
        }
        drop(db);
        let result = Volume::open(&path);
        if fail {
            assert!(result.is_err());
            let db = Connection::open(&path).unwrap();
            assert_eq!(
                db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                3
            );
        } else {
            let volume = result.unwrap();
            assert_eq!(volume.manifest().unwrap(), "manifest");
            assert_eq!(volume.object_count(g).unwrap(), 1);
            assert_eq!(volume.native_identity(g, &[1]).unwrap(), None);
            let db = Connection::open(&path).unwrap();
            assert_eq!(
                db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                5
            );
        }
    }
}
