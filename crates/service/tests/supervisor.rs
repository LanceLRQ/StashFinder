use stashfinder_core::{assets::*, index::IndexMode};
use stashfinder_service::{index::Source, supervisor::Runner};
use stashfinder_store::catalog::Catalog;
use stashfinder_windows::ntfs::{DirectoryEntry, Journal, NativeObject, UsnPage, UsnRecord};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

struct Disk(Arc<AtomicUsize>, Arc<AtomicBool>);
impl Source for Disk {
    fn root_id(&self) -> u64 {
        5
    }
    fn journal(&self) -> anyhow::Result<Option<Journal>> {
        if !self.1.load(Ordering::SeqCst) {
            return Ok(None);
        }
        Ok(Some(Journal {
            id: 1,
            first: 0,
            next: 100,
        }))
    }
    fn create_journal(&self) -> anyhow::Result<Journal> {
        panic!("no automatic consent")
    }
    fn inventory(
        &self,
        _: &AtomicBool,
        _: &mut dyn FnMut(&[UsnRecord]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn directory(
        &self,
        _: u64,
        _: &AtomicBool,
        _: &mut dyn FnMut(&[DirectoryEntry]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    fn object(&self, _: u64) -> anyhow::Result<Option<NativeObject>> {
        Ok(Some(NativeObject {
            entry: DirectoryEntry {
                id: 5,
                name: vec![92],
                attributes: 16,
                size: 0,
                created: 0,
                modified: 0,
            },
            links: vec![],
        }))
    }
    fn changes(&self, _: u64, _: i64) -> anyhow::Result<UsnPage> {
        panic!("empty journal must not replay")
    }
}
fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn startup_discards_unfinished_initial_paths_and_keeps_the_active_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let mut catalog = Catalog::open(dir.path()).unwrap();
    let observed = Observation {
        candidate_id: "fixture".into(),
        disk_number: None,
        model: None,
        disk_bytes: None,
        partition_bytes: None,
        volume_label: None,
        mounts: vec![],
        capacity: None,
        error: None,
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
    };
    let row = catalog.prepare(&observed, "Test").unwrap();
    catalog.finish(&row.partition_id).unwrap();
    let path = catalog.volume_path(&row.filesystem_id).unwrap();
    let mut volume = stashfinder_store::volume::Volume::open(&path).unwrap();
    let old = volume.begin_generation().unwrap();
    volume.complete(old).unwrap();
    volume.publish(old).unwrap();
    let candidate = volume.begin_generation().unwrap();
    volume.begin_initial_paths(candidate).unwrap();
    drop(volume);
    let worker = Runner::start_with_source(dir.path().into(), |_| {
        panic!("offline startup opened a source")
    });
    wait_until(|| worker.ready());
    let status = worker.status(&row.partition_id);
    assert_eq!(status.active_generation, Some(old));
    assert_eq!(status.phase, "failed");
    assert!(status.error.unwrap().contains("上次构建中断"));
}
#[test]
fn automatic_worker_requires_registration_online_identity_and_enabled_policy() {
    let dir = tempfile::tempdir().unwrap();
    let mut catalog = Catalog::open(dir.path()).unwrap();
    let observed = Observation {
        candidate_id: "fixture".into(),
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
    let row = catalog.prepare(&observed, "Test").unwrap();
    catalog.finish(&row.partition_id).unwrap();
    let mut snapshot = Snapshot {
        assets: vec![Asset {
            capacity_error: None,
            registration: catalog.get(&row.partition_id).unwrap(),
            connection: "offline".into(),
            mounts: vec![],
            error: None,
        }],
        ..Default::default()
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let source = calls.clone();
    let journal = Arc::new(AtomicBool::new(true));
    let journal_source = journal.clone();
    let worker = Runner::start_with_source(dir.path().into(), move |_| {
        Ok(Box::new(Disk(source.clone(), journal_source.clone())))
    });
    wait_until(|| worker.ready());
    worker.set_enabled(false).unwrap();
    worker.observe(snapshot.clone());
    assert!(
        worker
            .scan(&row.partition_id, IndexMode::Journal, &snapshot)
            .is_err()
    );
    snapshot.assets[0].connection = "online".into();
    worker.observe(snapshot.clone());
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    worker.set_enabled(true).unwrap();
    wait_until(|| {
        worker
            .tasks()
            .unwrap()
            .iter()
            .any(|t| t.state == "completed")
    });
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(worker.tasks().unwrap().len(), 1);
    let id = worker
        .scan(&row.partition_id, IndexMode::Journal, &snapshot)
        .unwrap();
    worker.cancel(&id).unwrap();
    assert!(worker.paused().unwrap().contains(&row.partition_id));
    assert!(worker.cancel("old-task-id").is_err());
    worker.set_enabled(false).unwrap();
    worker.pause(&row.partition_id, false, &snapshot).unwrap();
    let id = worker
        .scan(&row.partition_id, IndexMode::Journal, &snapshot)
        .unwrap();
    wait_until(|| {
        worker
            .tasks()
            .unwrap()
            .iter()
            .any(|t| t.id == id && t.state == "completed")
    });
    journal.store(false, Ordering::SeqCst);
    worker.set_enabled(true).unwrap();
    wait_until(|| {
        worker
            .tasks()
            .unwrap()
            .iter()
            .any(|t| t.state == "needs_confirmation")
    });
    assert_eq!(worker.status(&row.partition_id).phase, "needs_validation");
    assert!(worker.status(&row.partition_id).journal_missing);

    worker.set_enabled(false).unwrap();
    let id = worker
        .scan(&row.partition_id, IndexMode::Journal, &snapshot)
        .unwrap();
    wait_until(|| {
        worker
            .tasks()
            .unwrap()
            .iter()
            .any(|t| t.id == id && t.state == "needs_confirmation")
    });
    assert!(worker.status(&row.partition_id).journal_missing);

    journal.store(true, Ordering::SeqCst);
    worker.retry(&id, &snapshot).unwrap();
    wait_until(|| worker.status(&row.partition_id).phase == "ready");
    assert!(!worker.status(&row.partition_id).journal_missing);
}

struct ScanDisk {
    reads: Arc<AtomicUsize>,
    changed: Arc<AtomicBool>,
}

struct MountSession {
    epoch: Arc<AtomicUsize>,
    opened: usize,
    reads: Arc<AtomicUsize>,
}
impl stashfinder_service::scan::Source for MountSession {
    fn verify(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.opened == self.epoch.load(Ordering::SeqCst),
            "old mount session"
        );
        Ok(())
    }
    fn changed(&mut self) -> anyhow::Result<bool> {
        self.verify()?;
        Ok(false)
    }
    fn enumerate(
        &self,
        _: &[u16],
        _: &AtomicBool,
        _: &mut dyn FnMut(&[stashfinder_windows::scan::ScanEntry]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        self.verify()?;
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn an_invalid_scan_session_is_reopened_without_an_offline_observation() {
    let dir = tempfile::tempdir().unwrap();
    let mut catalog = Catalog::open(dir.path()).unwrap();
    let observed = Observation {
        candidate_id: "fat".into(),
        disk_number: None,
        model: None,
        disk_bytes: None,
        partition_bytes: None,
        volume_label: None,
        mounts: vec![],
        identity: Identity {
            layout: Layout::Gpt {
                disk: "disk".into(),
                partition: "fat".into(),
            },
            filesystem: Some("exFAT".into()),
            serial: Some(Serial {
                source: "win32_volume32".into(),
                value: "1".into(),
            }),
        },
        capacity: None,
        error: None,
    };
    let registered = catalog.prepare(&observed, "FAT").unwrap();
    catalog.finish(&registered.partition_id).unwrap();
    let snapshot = Snapshot {
        assets: vec![Asset {
            registration: catalog.get(&registered.partition_id).unwrap(),
            connection: "online".into(),
            mounts: vec![],
            error: None,
            capacity_error: None,
        }],
        ..Default::default()
    };
    let epoch = Arc::new(AtomicUsize::new(0));
    let reads = Arc::new(AtomicUsize::new(0));
    let opens = Arc::new(AtomicUsize::new(0));
    let (e, r, o) = (epoch.clone(), reads.clone(), opens.clone());
    let worker = Runner::start_with_sources(
        dir.path().into(),
        |_| panic!("not NTFS"),
        move |_| {
            o.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(MountSession {
                opened: e.load(Ordering::SeqCst),
                epoch: e.clone(),
                reads: r.clone(),
            }))
        },
    );
    wait_until(|| worker.ready());
    worker.observe(snapshot);
    wait_until(|| {
        reads.load(Ordering::SeqCst) >= 1
            && worker.status(&registered.partition_id).phase == "snapshot"
    });
    assert_eq!(opens.load(Ordering::SeqCst), 1);
    // Keep the snapshot online: the detach/reattach happened between device polls.
    epoch.store(1, Ordering::SeqCst);
    wait_until(|| {
        reads.load(Ordering::SeqCst) >= 2
            && worker.status(&registered.partition_id).phase == "snapshot"
    });
    assert!(opens.load(Ordering::SeqCst) >= 2);
}
impl stashfinder_service::scan::Source for ScanDisk {
    fn verify(&self) -> anyhow::Result<()> {
        Ok(())
    }
    fn changed(&mut self) -> anyhow::Result<bool> {
        Ok(self.changed.swap(false, Ordering::SeqCst))
    }
    fn enumerate(
        &self,
        _: &[u16],
        _: &AtomicBool,
        _: &mut dyn FnMut(&[stashfinder_windows::scan::ScanEntry]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
#[test]
fn scanning_filesystem_watches_only_registered_online_volumes_and_rescans_on_reconnect() {
    let dir = tempfile::tempdir().unwrap();
    let mut catalog = Catalog::open(dir.path()).unwrap();
    let observed = Observation {
        candidate_id: "fat".into(),
        disk_number: None,
        model: None,
        disk_bytes: None,
        partition_bytes: None,
        volume_label: None,
        mounts: vec![],
        identity: Identity {
            layout: Layout::Gpt {
                disk: "disk".into(),
                partition: "fat".into(),
            },
            filesystem: Some("exFAT".into()),
            serial: Some(Serial {
                source: "win32_volume32".into(),
                value: "1".into(),
            }),
        },
        capacity: None,
        error: None,
    };
    let row = catalog.prepare(&observed, "FAT").unwrap();
    catalog.finish(&row.partition_id).unwrap();
    let mut snapshot = Snapshot {
        assets: vec![Asset {
            capacity_error: None,
            registration: catalog.get(&row.partition_id).unwrap(),
            connection: "online".into(),
            mounts: vec![],
            error: None,
        }],
        ..Default::default()
    };
    let reads = Arc::new(AtomicUsize::new(0));
    let changed = Arc::new(AtomicBool::new(false));
    let (r, c) = (reads.clone(), changed.clone());
    let fail_open = AtomicBool::new(true);
    let worker = Runner::start_with_sources(
        dir.path().into(),
        |_| panic!("FAT must not open NTFS"),
        move |_| {
            anyhow::ensure!(
                !fail_open.swap(false, Ordering::SeqCst),
                "monitor unavailable"
            );
            Ok(Box::new(ScanDisk {
                reads: r.clone(),
                changed: c.clone(),
            }))
        },
    );
    wait_until(|| worker.ready());
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    worker.observe(snapshot.clone());
    wait_until(|| worker.tasks().unwrap().iter().any(|t| t.state == "failed"));
    let failed = worker
        .tasks()
        .unwrap()
        .into_iter()
        .find(|t| t.state == "failed")
        .unwrap();
    assert_eq!(failed.kind, "scan");
    assert!(matches!(failed.mode, IndexMode::ScanOnly));
    worker.retry(&failed.id, &snapshot).unwrap();
    wait_until(|| worker.status(&row.partition_id).phase == "snapshot");
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    std::thread::sleep(Duration::from_millis(2300));
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    changed.store(true, Ordering::SeqCst);
    wait_until(|| reads.load(Ordering::SeqCst) == 2);
    wait_until(|| worker.status(&row.partition_id).task_id.is_none());
    snapshot.assets[0].connection = "offline".into();
    worker.observe(snapshot.clone());
    std::thread::sleep(Duration::from_millis(250));
    snapshot.assets[0].connection = "online".into();
    worker.observe(snapshot.clone());
    wait_until(|| reads.load(Ordering::SeqCst) == 3);
    wait_until(|| worker.status(&row.partition_id).task_id.is_none());
    worker.pause(&row.partition_id, true, &snapshot).unwrap();
    std::thread::sleep(Duration::from_millis(250));
    changed.store(true, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(2200));
    assert_eq!(reads.load(Ordering::SeqCst), 3);
    worker.pause(&row.partition_id, false, &snapshot).unwrap();
    wait_until(|| reads.load(Ordering::SeqCst) == 4);
}
