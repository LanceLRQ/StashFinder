use stashfinder_core::assets::*;
use stashfinder_store::catalog::Catalog;

#[test]
fn read_only_catalog_does_not_initialize_pending_children() {
    let root = tempfile::tempdir().unwrap();
    let mut catalog = Catalog::open(root.path()).unwrap();
    let pending = catalog.prepare(&observed("pending"), "Pending").unwrap();
    let path = catalog.volume_path(&pending.filesystem_id).unwrap();
    drop(catalog);
    let reader = Catalog::read(root.path()).unwrap();
    assert_eq!(
        reader.get(&pending.partition_id).unwrap().state,
        "preparing"
    );
    assert!(!path.exists());
    assert!(Catalog::read(&root.path().join("missing")).is_err());
    assert!(!root.path().join("missing").exists());
}

#[test]
fn foreign_m1_database_is_not_migrated_during_health_or_startup() {
    let root = tempfile::tempdir().unwrap();
    let mut catalog = Catalog::open(root.path()).unwrap();
    let row = catalog.prepare(&observed("a"), "A").unwrap();
    catalog.finish(&row.partition_id).unwrap();
    let path = catalog.volume_path(&row.filesystem_id).unwrap();
    let mut foreign = catalog.get(&row.partition_id).unwrap();
    foreign.partition_id = "foreign".into();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute(
        "UPDATE manifest SET value=?1",
        [serde_json::to_string(&foreign).unwrap()],
    )
    .unwrap();
    db.execute_batch(
        "ALTER TABLE objects DROP COLUMN attributes;
        ALTER TABLE generations DROP COLUMN phase; ALTER TABLE generations DROP COLUMN started;
        ALTER TABLE generations DROP COLUMN completed; ALTER TABLE generations DROP COLUMN error;
        DROP INDEX paths_parent; PRAGMA user_version=1;",
    )
    .unwrap();
    drop(db);
    assert!(catalog.health(&row.filesystem_id).is_err());
    drop(catalog);
    drop(Catalog::open(root.path()).unwrap());
    let db = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

fn observed(partition: &str) -> Observation {
    Observation {
        candidate_id: format!("volume-{partition}"),
        disk_number: Some(1),
        model: Some("test".into()),
        disk_bytes: Some(1000),
        partition_bytes: Some(500),
        mounts: vec!["X:\\".into()],
        identity: Identity {
            layout: Layout::Gpt {
                disk: "disk-a".into(),
                partition: partition.into(),
            },
            filesystem: Some("NTFS".into()),
            serial: Some(Serial {
                source: "ntfs64".into(),
                value: partition.into(),
            }),
        },
        capacity: Some(Capacity {
            total: 500,
            free: 250,
            sampled_at: 123,
        }),
        error: None,
    }
}

#[test]
fn interrupted_registration_recovers_and_rename_survives_restart() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Catalog::open(root.path()).unwrap();
    let a = db.prepare(&observed("a"), "Movies").unwrap();
    assert_eq!(a.state, "preparing");
    drop(db);
    let mut db = Catalog::open(root.path()).unwrap();
    let rows = db.list().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, "ready");
    assert_eq!(rows[0].filesystem_id, a.filesystem_id);
    db.rename(&a.partition_id, "电影").unwrap();
    drop(db);
    let db = Catalog::open(root.path()).unwrap();
    assert_eq!(db.list().unwrap()[0].name, "电影");
    assert!(db.health(&a.filesystem_id).is_ok());
}

#[test]
fn missing_ready_database_is_not_recreated_and_other_partition_survives() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Catalog::open(root.path()).unwrap();
    let a = db.prepare(&observed("a"), "A").unwrap();
    db.finish(&a.partition_id).unwrap();
    let b = db.prepare(&observed("b"), "B").unwrap();
    db.finish(&b.partition_id).unwrap();
    let path = db.volume_path(&a.filesystem_id).unwrap();
    std::fs::remove_file(&path).unwrap();
    drop(db);
    let db = Catalog::open(root.path()).unwrap();
    assert!(!path.exists());
    assert!(db.health(&a.filesystem_id).is_err());
    assert!(db.health(&b.filesystem_id).is_ok());
    assert_eq!(db.list().unwrap().len(), 2);
}

#[test]
fn corrupt_catalog_is_not_replaced_and_recovery_is_separate() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Catalog::open(root.path()).unwrap();
    let a = db.prepare(&observed("a"), "A").unwrap();
    db.finish(&a.partition_id).unwrap();
    drop(db);
    let file = root.path().join("catalog.sqlite");
    std::fs::write(&file, b"corrupt catalog").unwrap();
    assert!(Catalog::open(root.path()).is_err());
    let recovered = root.path().join("recovered");
    let report = Catalog::recover(root.path(), &recovered).unwrap();
    assert!(report.is_empty());
    assert_eq!(
        Catalog::open(&recovered).unwrap().list().unwrap()[0].name,
        "A"
    );
    assert_eq!(std::fs::read(file).unwrap(), b"corrupt catalog");
}

#[test]
fn foreign_child_is_not_modified_by_observation() {
    use stashfinder_store::volume::Volume;
    let root = tempfile::tempdir().unwrap();
    let mut db = Catalog::open(root.path()).unwrap();
    let a = db.prepare(&observed("a"), "A").unwrap();
    db.finish(&a.partition_id).unwrap();
    let child = Volume::open(&db.volume_path(&a.filesystem_id).unwrap()).unwrap();
    let mut foreign = db.get(&a.partition_id).unwrap();
    foreign.partition_id = "foreign".into();
    child
        .set_manifest(&serde_json::to_string(&foreign).unwrap())
        .unwrap();
    let mut fresh = observed("a");
    fresh.capacity.as_mut().unwrap().free = 100;
    assert!(db.observe(&a.partition_id, &fresh).is_err());
    assert_eq!(child.capacity().unwrap().unwrap().free, 250);
}

#[test]
fn missing_catalog_with_existing_children_requires_recovery() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Catalog::open(root.path()).unwrap();
    let row = db.prepare(&observed("a"), "A").unwrap();
    db.finish(&row.partition_id).unwrap();
    drop(db);
    std::fs::remove_file(root.path().join("catalog.sqlite")).unwrap();
    assert!(Catalog::open(root.path()).is_err());
    assert!(!root.path().join("catalog.sqlite").exists());
}

#[test]
fn completed_child_before_catalog_commit_recovers_and_corrupt_child_is_isolated() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Catalog::open(root.path()).unwrap();
    let a = db.prepare(&observed("a"), "A").unwrap();
    let mut ready = a.clone();
    ready.state = "ready".into();
    drop(
        stashfinder_store::volume::Volume::create(
            &db.volume_path(&a.filesystem_id).unwrap(),
            &serde_json::to_string(&ready).unwrap(),
        )
        .unwrap(),
    );
    drop(db);
    let mut db = Catalog::open(root.path()).unwrap();
    assert_eq!(db.get(&a.partition_id).unwrap().state, "ready");
    let b = db.prepare(&observed("b"), "B").unwrap();
    db.finish(&b.partition_id).unwrap();
    let broken = db.volume_path(&a.filesystem_id).unwrap();
    std::fs::write(&broken, b"broken child").unwrap();
    drop(db);
    let db = Catalog::open(root.path()).unwrap();
    assert!(db.health(&a.filesystem_id).is_err());
    assert!(db.health(&b.filesystem_id).is_ok());
    assert_eq!(std::fs::read(broken).unwrap(), b"broken child");
}

#[test]
fn retry_allocates_a_new_child_and_missing_capacity_keeps_last_success() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Catalog::open(root.path()).unwrap();
    let a = db.prepare(&observed("a"), "A").unwrap();
    let broken = db.volume_path(&a.filesystem_id).unwrap();
    std::fs::write(&broken, b"incomplete").unwrap();
    assert!(db.finish(&a.partition_id).is_err());
    db.retry(&a.partition_id, &observed("a")).unwrap();
    let ready = db.get(&a.partition_id).unwrap();
    assert_ne!(a.filesystem_id, ready.filesystem_id);
    assert_eq!(std::fs::read(broken).unwrap(), b"incomplete");
    let mut fresh = observed("a");
    fresh.capacity = None;
    fresh.mounts = vec!["Y:\\".into()];
    db.observe(&a.partition_id, &fresh).unwrap();
    let row = db.get(&a.partition_id).unwrap();
    assert_eq!(row.observation.capacity.unwrap().sampled_at, 123);
    assert_eq!(row.observation.mounts, ["Y:\\"]);
}

#[test]
fn recovery_does_not_choose_arbitrarily_between_conflicting_manifests() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Catalog::open(root.path()).unwrap();
    let a = db.prepare(&observed("a"), "Original").unwrap();
    db.finish(&a.partition_id).unwrap();
    let mut conflicting = db.get(&a.partition_id).unwrap();
    conflicting.filesystem_id = uuid::Uuid::new_v4().to_string();
    conflicting.name = "Conflicting".into();
    drop(
        stashfinder_store::volume::Volume::create(
            &db.volume_path(&conflicting.filesystem_id).unwrap(),
            &serde_json::to_string(&conflicting).unwrap(),
        )
        .unwrap(),
    );
    drop(db);
    let destination = root.path().join("recovered");
    assert_eq!(
        Catalog::recover(root.path(), &destination).unwrap().len(),
        2
    );
    assert!(
        Catalog::open(&destination)
            .unwrap()
            .list()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn crash_child() {
    let Ok(root) = std::env::var("STASHFINDER_TEST_CRASH_ROOT") else {
        return;
    };
    let stage = std::env::var("STASHFINDER_TEST_CRASH_STAGE").unwrap();
    let mut db = Catalog::open(std::path::Path::new(&root)).unwrap();
    let mut row = db.prepare(&observed("crash"), "Crash fixture").unwrap();
    if stage == "child" {
        row.state = "ready".into();
        let _child = stashfinder_store::volume::Volume::create(
            &db.volume_path(&row.filesystem_id).unwrap(),
            &serde_json::to_string(&row).unwrap(),
        )
        .unwrap();
        std::process::exit(43);
    }
    if stage == "ready" {
        db.finish(&row.partition_id).unwrap();
    }
    // Bypass Rust destructors/checkpointing; the OS closes SQLite handles.
    std::process::exit(43);
}

#[test]
fn all_registration_commit_boundaries_survive_abrupt_process_exit() {
    for stage in ["preparing", "child", "ready"] {
        let root = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "crash_child"])
            .env("STASHFINDER_TEST_CRASH_ROOT", root.path())
            .env("STASHFINDER_TEST_CRASH_STAGE", stage)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(43));
        let db = Catalog::open(root.path()).unwrap();
        let rows = db.list().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].state, "ready");
        assert!(db.health(&rows[0].filesystem_id).is_ok(), "{stage}");
    }
}
