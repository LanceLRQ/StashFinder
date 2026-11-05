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
fn automatic_worker_requires_registration_online_identity_and_enabled_policy() {
    let dir = tempfile::tempdir().unwrap();
    let mut catalog = Catalog::open(dir.path()).unwrap();
    let observed = Observation {
        candidate_id: "fixture".into(),
        disk_number: None,
        model: None,
        disk_bytes: None,
        partition_bytes: None,
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
}
