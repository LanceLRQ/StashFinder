use stashfinder_core::{Action, assets::*};
use stashfinder_service::assets::Assets;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn observation() -> Observation {
    Observation {
        candidate_id: "volume-a".into(),
        disk_number: Some(0),
        model: Some("Test".into()),
        disk_bytes: Some(2000),
        partition_bytes: Some(1000),
        volume_label: None,
        mounts: vec!["X:\\".into()],
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
        capacity: Some(Capacity {
            total: 1000,
            free: 500,
            sampled_at: 1,
        }),
        error: None,
    }
}
fn settled(assets: &Assets) -> Snapshot {
    let start = Instant::now();
    loop {
        let s = assets.snapshot();
        if !s.busy {
            return s;
        };
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn index_writer_does_not_make_device_refresh_fail() {
    let root = tempfile::tempdir().unwrap();
    let fixtures = Arc::new(Mutex::new(vec![observation()]));
    let source = fixtures.clone();
    let assets = Assets::start(root.path().into(), move || {
        Ok(source.lock().unwrap().clone())
    });
    let snapshot = settled(&assets);
    assets
        .submit(Action::Register {
            version: snapshot.version,
            candidate_id: "volume-a".into(),
            name: "A".into(),
            allow_weak: false,
        })
        .unwrap();
    let registered = settled(&assets);
    let catalog = stashfinder_store::catalog::Catalog::read(root.path()).unwrap();
    let path = catalog
        .volume_path(&registered.assets[0].registration.filesystem_id)
        .unwrap();
    let mut writer = rusqlite::Connection::open(path).unwrap();
    let transaction = writer
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    fixtures.lock().unwrap()[0].capacity.as_mut().unwrap().free = 250;

    assets.submit(Action::Refresh).unwrap();
    let deferred = settled(&assets);
    assert!(deferred.error.is_none());
    assert_eq!(deferred.assets[0].connection, "online");
    assert!(
        deferred.assets[0].error.is_none(),
        "{:?}",
        deferred.assets[0].error
    );
    assert!(deferred.assets[0].capacity_error.is_some());
    assert_eq!(
        deferred.assets[0]
            .registration
            .observation
            .capacity
            .as_ref()
            .unwrap()
            .free,
        500
    );

    transaction.commit().unwrap();
    assets.submit(Action::Refresh).unwrap();
    let refreshed = settled(&assets);
    assert!(refreshed.assets[0].error.is_none());
    assert!(refreshed.assets[0].capacity_error.is_none());
    assert_eq!(
        refreshed.assets[0]
            .registration
            .observation
            .capacity
            .as_ref()
            .unwrap()
            .free,
        250
    );
}

#[test]
fn replaced_filesystem_is_historical_and_new_instance_requires_registration() {
    let root = tempfile::tempdir().unwrap();
    let fixtures = Arc::new(Mutex::new(vec![observation()]));
    let source = fixtures.clone();
    let assets = Assets::start(root.path().into(), move || {
        Ok(source.lock().unwrap().clone())
    });
    let s = settled(&assets);
    assets
        .submit(Action::Register {
            version: s.version,
            candidate_id: "volume-a".into(),
            name: "Old".into(),
            allow_weak: false,
        })
        .unwrap();
    let old = settled(&assets).assets[0].registration.clone();
    fixtures.lock().unwrap()[0]
        .identity
        .serial
        .as_mut()
        .unwrap()
        .value = "2".into();
    assets.submit(Action::Refresh).unwrap();
    let s = settled(&assets);
    assert_eq!(s.assets[0].connection, "historical");
    assert_eq!(s.assets[0].registration.state, "historical");
    assert!(s.candidates[0].blocked.is_none());
    assets
        .submit(Action::Register {
            version: s.version,
            candidate_id: "volume-a".into(),
            name: "New".into(),
            allow_weak: false,
        })
        .unwrap();
    let s = settled(&assets);
    assert_eq!(s.assets.len(), 2);
    assert_eq!(s.assets[0].registration.filesystem_id, old.filesystem_id);
    assert_eq!(s.assets[0].registration.name, "Old");
    assert_eq!(s.assets[1].connection, "online");
    assert_ne!(s.assets[1].registration.filesystem_id, old.filesystem_id);
}
#[test]
fn registers_only_selected_partition_and_rejects_changed_candidate() {
    let root = tempfile::tempdir().unwrap();
    let fixtures = Arc::new(Mutex::new(vec![observation()]));
    let source = fixtures.clone();
    let assets = Assets::start(root.path().into(), move || {
        Ok(source.lock().unwrap().clone())
    });
    let s = settled(&assets);
    assert!(s.assets.is_empty());
    assert_eq!(s.candidates.len(), 1);
    fixtures.lock().unwrap()[0]
        .identity
        .serial
        .as_mut()
        .unwrap()
        .value = "2".into();
    assets
        .submit(Action::Register {
            version: s.version,
            candidate_id: "volume-a".into(),
            name: "A".into(),
            allow_weak: false,
        })
        .unwrap();
    let s = settled(&assets);
    assert!(s.error.is_some());
    assert!(s.assets.is_empty());
    assets.submit(Action::Refresh).unwrap();
    let s = settled(&assets);
    assets
        .submit(Action::Register {
            version: s.version,
            candidate_id: "volume-a".into(),
            name: "A".into(),
            allow_weak: false,
        })
        .unwrap();
    let s = settled(&assets);
    assert_eq!(s.assets.len(), 1);
    assert!(s.error.is_none(), "{:?}", s.error);
    assert!(s.candidates[0].registered);
    let id = s.assets[0].registration.partition_id.clone();
    fixtures.lock().unwrap().clear();
    assets.submit(Action::Refresh).unwrap();
    let s = settled(&assets);
    assert_eq!(s.assets[0].connection, "offline");
    assert_eq!(
        s.assets[0]
            .registration
            .observation
            .capacity
            .as_ref()
            .unwrap()
            .free,
        500
    );
    assets
        .submit(Action::Rename {
            partition_id: id,
            name: "离线盘".into(),
        })
        .unwrap();
    assert_eq!(settled(&assets).assets[0].registration.name, "离线盘");
}

#[test]
fn duplicate_identity_is_blocked_without_registration() {
    let root = tempfile::tempdir().unwrap();
    let a = observation();
    let mut b = a.clone();
    b.candidate_id = "volume-b".into();
    let assets = Assets::start(root.path().into(), move || Ok(vec![a.clone(), b.clone()]));
    let s = settled(&assets);
    assert!(s.candidates.iter().all(|c| c.blocked.is_some()));
    assets
        .submit(Action::Register {
            version: s.version,
            candidate_id: "volume-a".into(),
            name: "A".into(),
            allow_weak: false,
        })
        .unwrap();
    assert!(settled(&assets).assets.is_empty());
}

#[test]
fn failed_discovery_keeps_registration_but_does_not_claim_offline_or_accept_stale_pages() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let root = tempfile::tempdir().unwrap();
    let fail = Arc::new(AtomicBool::new(false));
    let flag = fail.clone();
    let assets = Assets::start(root.path().into(), move || {
        anyhow::ensure!(!flag.load(Ordering::SeqCst), "discovery failed");
        Ok(vec![observation()])
    });
    let initial = settled(&assets);
    assets
        .submit(Action::Register {
            version: initial.version,
            candidate_id: "volume-a".into(),
            name: "A".into(),
            allow_weak: false,
        })
        .unwrap();
    let registered = settled(&assets);
    assert!(registered.error.is_none());
    assert!(assets.page(Some(initial.version), 0).is_err());
    assert!(
        assets
            .submit(Action::Register {
                version: initial.version,
                candidate_id: "volume-a".into(),
                name: "A".into(),
                allow_weak: false
            })
            .is_err()
    );
    fail.store(true, Ordering::SeqCst);
    assets.submit(Action::Refresh).unwrap();
    let failed = settled(&assets);
    assert!(failed.error.is_some());
    assert_eq!(failed.assets[0].connection, "unconfirmed");
    assert_eq!(
        failed.assets[0].registration.filesystem_id,
        registered.assets[0].registration.filesystem_id
    );
    assert!(failed.candidates.is_empty());
}

#[test]
fn capacity_failure_keeps_last_sample_without_marking_partition_inaccessible() {
    let root = tempfile::tempdir().unwrap();
    let fixtures = Arc::new(Mutex::new(vec![observation()]));
    let source = fixtures.clone();
    let assets = Assets::start(root.path().into(), move || {
        Ok(source.lock().unwrap().clone())
    });
    let s = settled(&assets);
    assets
        .submit(Action::Register {
            version: s.version,
            candidate_id: "volume-a".into(),
            name: "A".into(),
            allow_weak: false,
        })
        .unwrap();
    settled(&assets);
    fixtures.lock().unwrap()[0].capacity = None;
    assets.submit(Action::Refresh).unwrap();
    let s = settled(&assets);
    let a = &s.assets[0];
    assert_eq!(a.connection, "online");
    assert!(a.error.is_none());
    assert!(a.capacity_error.is_some());
    let c = a.registration.observation.capacity.as_ref().unwrap();
    assert_eq!((c.free, c.sampled_at), (500, 1));
    fixtures.lock().unwrap()[0].capacity = Some(Capacity {
        total: 1000,
        free: 250,
        sampled_at: 2,
    });
    assets.submit(Action::Refresh).unwrap();
    let s = settled(&assets);
    assert!(s.assets[0].capacity_error.is_none());
    assert_eq!(
        s.assets[0]
            .registration
            .observation
            .capacity
            .as_ref()
            .unwrap()
            .free,
        250
    );
}
