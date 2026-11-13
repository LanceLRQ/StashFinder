use stashfinder_core::{assets::*, index::*};
use stashfinder_service::jobs::Indexes;
use stashfinder_store::{catalog::Catalog, volume::*};
use std::time::{Duration, Instant};

#[test]
fn offline_search_survives_worker_restart_and_rejects_unregistered_scans() {
    let temp = tempfile::tempdir().unwrap();
    let mut catalog = Catalog::open(temp.path()).unwrap();
    let observation = Observation {
        candidate_id: "not a source volume".into(),
        disk_number: None,
        model: None,
        disk_bytes: None,
        partition_bytes: None,
        volume_label: None,
        mounts: vec![],
        identity: Identity {
            layout: Layout::Gpt {
                disk: "disk".into(),
                partition: "partition".into(),
            },
            filesystem: Some("NTFS".into()),
            serial: Some(Serial {
                source: "ntfs64".into(),
                value: "1".into(),
            }),
        },
        capacity: None,
        error: None,
    };
    let registered = catalog.prepare(&observation, "Offline").unwrap();
    catalog.finish(&registered.partition_id).unwrap();
    let mut db = Volume::open(&catalog.volume_path(&registered.filesystem_id).unwrap()).unwrap();
    let g = db.begin_generation().unwrap();
    let objects = [(1, "folder", None), (2, "file", Some(7))].map(|(id, kind, size)| Object {
        identity: None,
        id: vec![id],
        source: "ntfs64".into(),
        kind: kind.into(),
        size,
        created: None,
        modified: None,
        attributes: 0,
    });
    let entries = [
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
            name: "offline.txt".encode_utf16().collect(),
        },
    ];
    db.write_batch(g, &objects, &entries, &[], None).unwrap();
    db.complete(g).unwrap();
    db.publish(g).unwrap();
    drop(db);
    let snapshot = Snapshot {
        assets: vec![Asset {
            capacity_error: None,
            registration: catalog.get(&registered.partition_id).unwrap(),
            connection: "offline".into(),
            mounts: vec![],
            error: None,
        }],
        ..Default::default()
    };
    drop(catalog);
    for _ in 0..2 {
        let jobs = Indexes::start(temp.path().into());
        let request = stashfinder_core::browse::DirectoryRequest {
            partition_id: registered.partition_id.clone(),
            filesystem_id: registered.filesystem_id.clone(),
            parent: None,
            generation: None,
            revision: None,
            offset: 0,
            folders_only: false,
            sort: Default::default(),
            filter: None,
        };
        let directory = jobs.runner.directory(&request).unwrap();
        assert_eq!(directory.entries[0].name, "offline.txt");
        assert_eq!(directory.entries[0].entry, [2]);
        assert_eq!(directory.entries[0].parent, [1]);
        assert!(
            jobs.runner
                .directory(&stashfinder_core::browse::DirectoryRequest {
                    filesystem_id: "wrong".into(),
                    ..request.clone()
                })
                .is_err()
        );
        assert!(
            jobs.runner
                .directory(&stashfinder_core::browse::DirectoryRequest {
                    partition_id: "unregistered".into(),
                    ..request
                })
                .is_err()
        );
        assert!(jobs.scan("missing", IndexMode::Journal, &snapshot).is_err());
        assert!(
            jobs.scan(&registered.partition_id, IndexMode::Journal, &snapshot)
                .is_err()
        );
        let id = jobs
            .search(
                SearchRequest {
                    replaces: None,
                    text: "offline".into(),
                    names_only: false,
                    partitions: vec![],
                    options: Default::default(),
                },
                &snapshot,
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let page = loop {
            let page = jobs.page(id, 0).unwrap();
            if page.done {
                break page;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(page.error.is_none(), "{:?}", page.error);
        let row = page.row.unwrap();
        assert_eq!(row.file.path, "offline.txt");
        assert_eq!(row.connection, "offline");
        assert_eq!(row.partition_name, "Offline");
        assert!(row.mounts.is_empty());
        assert!(!row.partial);
        assert!(row.indexed_at.is_some());
        assert!(!row.needs_validation);
        let batch = jobs.batch(id, 0, 100).unwrap();
        assert_eq!(batch.total, 1);
        assert_eq!(batch.rows[0].file.path, "offline.txt");
        assert!(batch.row.is_none());
        assert!(jobs.batch(id, 0, 0).is_err());
        assert!(jobs.batch(id, 3, 100).is_err());
        let first_id = id;
        let mut inaccessible = snapshot.clone();
        inaccessible.assets[0].connection = "inaccessible".into();
        inaccessible.assets[0].error = Some("device access unavailable".into());
        let id = jobs
            .search(
                SearchRequest {
                    replaces: None,
                    text: "offline".into(),
                    names_only: false,
                    partitions: vec![],
                    options: Default::default(),
                },
                &inaccessible,
            )
            .unwrap();
        let page = loop {
            let page = jobs.page(id, 0).unwrap();
            if page.done {
                break page;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(page.unavailable.is_empty(), "{:?}", page.unavailable);
        assert_eq!(page.row.unwrap().connection, "inaccessible");
        assert_eq!(
            jobs.page(first_id, 0).unwrap().row.unwrap().connection,
            "offline"
        );
        jobs.cancel_search(first_id);
        assert!(jobs.page(first_id, 0).is_err());
        assert_eq!(
            jobs.page(id, 0).unwrap().row.unwrap().connection,
            "inaccessible"
        );
        jobs.observe(Snapshot::default());
        assert!(jobs.page(id, 0).is_err());
    }
}

#[test]
fn catalog_error_is_not_reported_as_an_empty_search() {
    let temp = tempfile::tempdir().unwrap();
    let jobs = Indexes::start(temp.path().into());
    let snapshot = Snapshot {
        error: Some("catalog unavailable".into()),
        ..Default::default()
    };
    assert!(
        jobs.search(
            SearchRequest {
                replaces: None,
                text: "".into(),
                names_only: false,
                partitions: vec![],
                options: Default::default()
            },
            &snapshot
        )
        .is_err()
    );
}
