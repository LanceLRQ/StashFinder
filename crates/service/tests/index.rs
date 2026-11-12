use stashfinder_core::search::Query;
use stashfinder_service::index::{IndexMode, Source, build};
use stashfinder_store::{search::SearchIndex, volume::Volume};
use stashfinder_windows::ntfs::{DirectoryEntry, Journal, NativeObject, UsnPage, UsnRecord};
use std::{cell::Cell, sync::atomic::AtomicBool};

struct FastFixture {
    failure: Option<&'static str>,
    fast: Cell<usize>,
    legacy: Cell<usize>,
    journals: Cell<usize>,
}

impl Source for FastFixture {
    fn excluded_roots(&self) -> Vec<u64> {
        if self.failure == Some("exclude") {
            vec![20]
        } else {
            vec![]
        }
    }
    fn root_id(&self) -> u64 {
        5
    }
    fn journal(&self) -> anyhow::Result<Option<Journal>> {
        self.journals.set(self.journals.get() + 1);
        Ok(Some(Journal {
            id: 1,
            first: 0,
            next: if matches!(self.failure, Some("paged_delete" | "missing" | "reused"))
                && self.journals.get() > 1
            {
                300
            } else if self.failure == Some("replay") && self.journals.get() > 1 {
                200
            } else {
                100
            },
        }))
    }
    fn create_journal(&self) -> anyhow::Result<Journal> {
        panic!("test must not create a journal")
    }
    fn full_inventory(
        &self,
        _: &AtomicBool,
        consume: &mut dyn FnMut(&[NativeObject]) -> anyhow::Result<()>,
    ) -> anyhow::Result<Option<stashfinder_windows::ntfs::mft::FastScanStats>> {
        self.fast.set(self.fast.get() + 1);
        if self.failure == Some("unsupported") {
            return Ok(None);
        }
        let link = |name: &str| name.encode_utf16().collect();
        consume(&[
            NativeObject {
                entry: entry(21, "中文.txt", false, 42),
                links: vec![(20, link("中文.txt")), (5, link("alias.txt"))],
            },
            NativeObject {
                entry: entry(5, "\\", true, 0),
                links: vec![],
            },
        ])?;
        match self.failure {
            Some("unavailable") => {
                return Err(stashfinder_windows::ntfs::mft::FastScanUnavailable(
                    "fixture unavailable",
                )
                .into());
            }
            Some("io") => anyhow::bail!("fixture disconnected"),
            Some("duplicate") => consume(&[NativeObject {
                entry: entry(5, "\\", true, 0),
                links: vec![],
            }])?,
            Some("path_conflict") => consume(&[NativeObject {
                entry: entry(22, "alias.txt", false, 23),
                links: vec![(5, link("alias.txt"))],
            }])?,
            Some("orphan") => (),
            _ => consume(&[NativeObject {
                entry: entry(20, "folder", true, 0),
                links: vec![(5, link("folder"))],
            }])?,
        }
        Ok(Some(Default::default()))
    }
    fn inventory(
        &self,
        _: &AtomicBool,
        _: &mut dyn FnMut(&[UsnRecord]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        self.legacy.set(self.legacy.get() + 1);
        Ok(())
    }
    fn directory(
        &self,
        id: u64,
        _: &AtomicBool,
        consume: &mut dyn FnMut(&[DirectoryEntry]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        assert_eq!(id, 5);
        consume(&[entry(40, "legacy.txt", false, 7)])
    }
    fn object(&self, id: u64) -> anyhow::Result<Option<NativeObject>> {
        if matches!(
            self.failure,
            Some("replay" | "paged_delete" | "missing" | "reused")
        ) && id == 21
        {
            return Ok(None);
        }
        assert_eq!(id, 5);
        Ok(Some(NativeObject {
            entry: entry(5, "\\", true, 0),
            links: vec![],
        }))
    }
    fn changes(&self, id: u64, start: i64) -> anyhow::Result<UsnPage> {
        if matches!(self.failure, Some("paged_delete" | "missing" | "reused")) {
            assert_eq!(id, 1);
            assert!(matches!(start, 100 | 200));
            return Ok(UsnPage {
                next: if start == 100 { 200 } else { 300 },
                records: if start == 200 && self.failure == Some("missing") {
                    vec![]
                } else {
                    vec![UsnRecord {
                        id: if start == 200 && self.failure == Some("reused") {
                            21 + (1 << 48)
                        } else {
                            21
                        },
                        parent: 20,
                        usn: start + 50,
                        reason: if start == 100 { 1 } else { 0x200 },
                        attributes: 32,
                        name: vec![],
                    }]
                },
            });
        }
        assert_eq!((self.failure, id, start), (Some("replay"), 1, 100));
        Ok(UsnPage {
            next: 200,
            records: vec![UsnRecord {
                id: 21,
                parent: 20,
                usn: 150,
                reason: 0x200,
                attributes: 32,
                name: vec![],
            }],
        })
    }
}

#[test]
fn replay_requires_deletion_evidence_across_pages_for_the_same_full_file_id() {
    for incremental in [false, true] {
        for failure in ["paged_delete", "missing", "reused"] {
            let dir = tempfile::tempdir().unwrap();
            let mut db = Volume::create(&dir.path().join("v.sqlite"), "{}").unwrap();
            let mut source = FastFixture {
                failure: None,
                fast: Cell::new(0),
                legacy: Cell::new(0),
                journals: Cell::new(0),
            };
            if incremental {
                build(
                    &source,
                    &mut db,
                    IndexMode::Journal,
                    &AtomicBool::new(false),
                    &mut |_| {},
                )
                .unwrap();
            }
            source.failure = Some(failure);
            let result = if incremental {
                stashfinder_service::index::synchronize(&source, &mut db, &AtomicBool::new(false))
                    .map(|_| ())
            } else {
                build(
                    &source,
                    &mut db,
                    IndexMode::Journal,
                    &AtomicBool::new(false),
                    &mut |_| {},
                )
            };
            if failure == "paged_delete" {
                result.unwrap();
                let generation = db.active_generation().unwrap().unwrap();
                assert_eq!(db.cursor(generation).unwrap().unwrap().next, 300);
                assert_eq!(db.index_status().unwrap().records, 1);
            } else {
                assert!(result.is_err());
                if incremental {
                    let generation = db.active_generation().unwrap().unwrap();
                    assert_eq!(db.cursor(generation).unwrap().unwrap().next, 100);
                    assert_eq!(db.index_status().unwrap().records, 3);
                } else {
                    assert!(db.active_generation().unwrap().is_none());
                }
            }
        }
    }
}

#[test]
fn sequential_build_keeps_hard_links_without_legacy_enumeration() {
    use stashfinder_service::index::{BuildStrategy, build_with_strategy};
    let dir = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&dir.path().join("v.sqlite"), "{}").unwrap();
    let source = FastFixture {
        failure: None,
        fast: Cell::new(0),
        legacy: Cell::new(0),
        journals: Cell::new(0),
    };
    let report = build_with_strategy(
        &source,
        &mut db,
        IndexMode::Journal,
        BuildStrategy::Sequential,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.strategy, BuildStrategy::Sequential);
    assert_eq!((source.fast.get(), source.legacy.get()), (1, 0));
    assert_eq!(db.index_status().unwrap().records, 3);
    let generation = db.active_generation().unwrap().unwrap();
    assert_eq!(db.cursor(generation).unwrap().unwrap().next, 100);
}

#[test]
fn default_build_prefers_sequential_and_uses_legacy_only_when_needed() {
    for (mode, failure, expected_fast, expected_inventory, expected_records) in [
        (IndexMode::Journal, None, 1, 0, 3),
        (IndexMode::Journal, Some("exclude"), 1, 0, 1),
        (IndexMode::Journal, Some("unsupported"), 1, 1, 1),
        (IndexMode::ScanOnly, None, 0, 0, 1),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Volume::create(&dir.path().join("v.sqlite"), "{}").unwrap();
        let source = FastFixture {
            failure,
            fast: Cell::new(0),
            legacy: Cell::new(0),
            journals: Cell::new(0),
        };
        build(&source, &mut db, mode, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(source.fast.get(), expected_fast);
        assert_eq!(source.legacy.get(), expected_inventory);
        assert_eq!(db.index_status().unwrap().records, expected_records);
    }
}

#[test]
fn fallback_restarts_a_clean_generation_only_for_eligible_failures() {
    use stashfinder_service::index::{BuildStrategy, build_with_strategy};
    for (failure, strategy, fallback) in [
        ("unsupported", BuildStrategy::Auto, true),
        ("unavailable", BuildStrategy::Auto, true),
        ("orphan", BuildStrategy::Auto, true),
        ("path_conflict", BuildStrategy::Auto, true),
        ("io", BuildStrategy::Auto, false),
        ("duplicate", BuildStrategy::Auto, false),
        ("unavailable", BuildStrategy::Sequential, false),
        ("path_conflict", BuildStrategy::Sequential, false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Volume::create(&dir.path().join("v.sqlite"), "{}").unwrap();
        let source = FastFixture {
            failure: Some(failure),
            fast: Cell::new(0),
            legacy: Cell::new(0),
            journals: Cell::new(0),
        };
        let result = build_with_strategy(
            &source,
            &mut db,
            IndexMode::Journal,
            strategy,
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(source.fast.get(), 1);
        assert_eq!(source.legacy.get(), usize::from(fallback));
        assert_eq!(db.object_count(1).unwrap(), 0);
        if fallback {
            let report = result.unwrap();
            assert_eq!(report.strategy, BuildStrategy::Legacy);
            assert!(report.fallback.is_some());
            assert_eq!(db.active_generation().unwrap(), Some(2));
            assert_eq!(db.index_status().unwrap().records, 1);
            assert!(source.journals.get() >= 3);
        } else {
            let error = result.unwrap_err();
            if failure == "path_conflict" {
                assert!(error.is::<stashfinder_store::initial::InitialPathConflict>());
            }
            assert_eq!(db.active_generation().unwrap(), None);
        }
    }
}

#[test]
fn sequential_replay_removes_all_links_of_a_deleted_file_before_publication() {
    use stashfinder_service::index::{BuildStrategy, build_with_strategy};
    let dir = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&dir.path().join("v.sqlite"), "{}").unwrap();
    let source = FastFixture {
        failure: Some("replay"),
        fast: Cell::new(0),
        legacy: Cell::new(0),
        journals: Cell::new(0),
    };
    build_with_strategy(
        &source,
        &mut db,
        IndexMode::Journal,
        BuildStrategy::Sequential,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    let generation = db.active_generation().unwrap().unwrap();
    assert_eq!(db.cursor(generation).unwrap().unwrap().next, 200);
    assert_eq!(db.index_status().unwrap().records, 1);
    assert_eq!(db.object_count(generation).unwrap(), 2);
    assert_eq!(source.legacy.get(), 0);
}

struct Delta {
    first: i64,
    next: i64,
    journal_id: u64,
    unreadable: bool,
}
impl Source for Delta {
    fn root_id(&self) -> u64 {
        5
    }
    fn journal(&self) -> anyhow::Result<Option<Journal>> {
        Ok(Some(Journal {
            id: self.journal_id,
            first: self.first,
            next: self.next,
        }))
    }
    fn create_journal(&self) -> anyhow::Result<Journal> {
        panic!("incremental must not create journals")
    }
    fn inventory(
        &self,
        _: &AtomicBool,
        _: &mut dyn FnMut(&[UsnRecord]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        panic!("incremental must not enumerate MFT")
    }
    fn directory(
        &self,
        _: u64,
        _: &AtomicBool,
        _: &mut dyn FnMut(&[DirectoryEntry]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        panic!("incremental must not scan directories")
    }
    fn object(&self, id: u64) -> anyhow::Result<Option<NativeObject>> {
        anyhow::ensure!(!self.unreadable, "device disconnected");
        assert_eq!(id, 20);
        Ok(Some(NativeObject {
            entry: entry(20, "later.txt", false, 42),
            links: vec![
                (5, "later.txt".encode_utf16().collect()),
                (5, "alias.txt".encode_utf16().collect()),
            ],
        }))
    }
    fn changes(&self, journal: u64, start: i64) -> anyhow::Result<UsnPage> {
        assert_eq!((journal, start), (1, 200));
        Ok(UsnPage {
            next: 300,
            records: vec![UsnRecord {
                id: 20,
                parent: 5,
                usn: 250,
                reason: 0x2000,
                attributes: 0,
                name: vec![65],
            }],
        })
    }
}

#[test]
fn incremental_survives_restart_and_does_not_rescan_or_replay_twice() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("v.sqlite");
    let mut db = Volume::create(&file, "{}").unwrap();
    build(
        &Fixture {
            changed: Cell::new(false),
            journal: true,
            broken: false,
            cancel: false,
        },
        &mut db,
        IndexMode::Journal,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    let generation = db.active_generation().unwrap();
    drop(db);
    let mut db = Volume::open(&file).unwrap();
    let source = Delta {
        first: 0,
        next: 300,
        journal_id: 1,
        unreadable: false,
    };
    assert!(
        stashfinder_service::index::synchronize(&source, &mut db, &AtomicBool::new(false)).unwrap()
    );
    assert_eq!(db.active_generation().unwrap(), generation);
    assert_eq!(db.cursor(generation.unwrap()).unwrap().unwrap().next, 300);
    assert!(
        !stashfinder_service::index::synchronize(&source, &mut db, &AtomicBool::new(false))
            .unwrap()
    );
    let index = SearchIndex::load(&db).unwrap().unwrap();
    let hits = index
        .search(
            &Query::parse(".txt", false).unwrap(),
            &[],
            200,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(hits.rows.len(), 2);
    assert!(hits.rows.iter().all(|r| r.size.as_deref() == Some("42")));
    assert!(hits.rows.iter().any(|r| r.path == "later.txt"));
    assert!(hits.rows.iter().any(|r| r.path == "alias.txt"));
    assert!(!index.needs_validation);
}

struct MetadataDelta {
    target: u64,
    reason: u32,
    caught_up: bool,
    unreadable: bool,
}

fn torn_control_record() -> anyhow::Result<()> {
    let mut record = [0; 1024];
    record[..4].copy_from_slice(b"FILE");
    record[4..6].copy_from_slice(&48u16.to_le_bytes());
    record[6..8].copy_from_slice(&3u16.to_le_bytes());
    record[48..50].copy_from_slice(&0x00edu16.to_le_bytes());
    record[50..52].copy_from_slice(&0x00f8u16.to_le_bytes());
    record[510..512].copy_from_slice(&0x32f8u16.to_le_bytes());
    stashfinder_windows::ntfs::mft::prepare_control_record(&mut record)
}

impl Source for MetadataDelta {
    fn root_id(&self) -> u64 {
        5
    }

    fn journal(&self) -> anyhow::Result<Option<Journal>> {
        Ok(Some(Journal {
            id: 1,
            first: 0,
            next: 200,
        }))
    }

    fn create_journal(&self) -> anyhow::Result<Journal> {
        unreachable!()
    }

    fn inventory(
        &self,
        _: &AtomicBool,
        _: &mut dyn FnMut(&[UsnRecord]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        unreachable!()
    }

    fn directory(
        &self,
        _: u64,
        _: &AtomicBool,
        _: &mut dyn FnMut(&[DirectoryEntry]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        unreachable!()
    }

    fn object(&self, id: u64) -> anyhow::Result<Option<NativeObject>> {
        if id == self.target {
            if self.unreadable {
                torn_control_record()?;
            }
            Ok(Some(NativeObject {
                entry: entry(id, "中文.txt", false, 84),
                links: vec![
                    (20, "中文.txt".encode_utf16().collect()),
                    (5, "alias.txt".encode_utf16().collect()),
                ],
            }))
        } else {
            assert_eq!(id, 22);
            Ok(Some(NativeObject {
                entry: entry(id, "neighbor.txt", false, 16),
                links: vec![(5, "neighbor.txt".encode_utf16().collect())],
            }))
        }
    }

    fn changes(&self, journal: u64, start: i64) -> anyhow::Result<UsnPage> {
        assert!(!self.caught_up);
        assert_eq!((journal, start), (1, 100));
        let mut records = vec![UsnRecord {
            id: self.target,
            parent: 20,
            usn: 150,
            reason: 2,
            attributes: 32,
            name: "中文.txt".encode_utf16().collect(),
        }];
        if self.reason != 0 {
            records.push(UsnRecord {
                id: self.target,
                parent: 20,
                usn: 160,
                reason: self.reason,
                attributes: 32,
                name: "中文.txt".encode_utf16().collect(),
            });
        }
        records.push(UsnRecord {
            id: 22,
            parent: 5,
            usn: 180,
            reason: 0x100,
            attributes: 32,
            name: "neighbor.txt".encode_utf16().collect(),
        });
        Ok(UsnPage { next: 200, records })
    }
}

fn metadata_delta_index(file: &std::path::Path) -> Volume {
    let mut db = Volume::create(file, "{}").unwrap();
    build(
        &FastFixture {
            failure: None,
            fast: Cell::new(0),
            legacy: Cell::new(0),
            journals: Cell::new(0),
        },
        &mut db,
        IndexMode::Journal,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    db
}

#[test]
fn incremental_commits_neighbors_and_retries_deferred_metadata_after_reopen() {
    use stashfinder_service::index::{ObservedCursor, has_relevant_changes, synchronize};
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("volume.sqlite");
    let mut db = metadata_delta_index(&file);
    let generation = db.active_generation().unwrap().unwrap();
    let cancelled = AtomicBool::new(false);
    let mut source = MetadataDelta {
        target: 21,
        reason: 0,
        caught_up: false,
        unreadable: true,
    };
    let sizes = |db: &Volume| {
        let index = SearchIndex::load(db).unwrap().unwrap();
        assert!(!index.needs_validation);
        index
            .search(&Query::parse(".txt", false).unwrap(), &[], 10, &cancelled)
            .unwrap()
            .rows
            .into_iter()
            .map(|row| (row.name, row.size.unwrap()))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    assert!(synchronize(&source, &mut db, &cancelled).unwrap());
    assert_eq!(db.cursor(generation).unwrap().unwrap().next, 200);
    let before = sizes(&db);
    assert_eq!(before["中文.txt"], "42");
    assert_eq!(before["alias.txt"], "42");
    assert_eq!(before["neighbor.txt"], "16");
    drop(db);

    let mut db = Volume::open(&file).unwrap();
    source.caught_up = true;
    let mut observed = ObservedCursor::default();
    assert!(has_relevant_changes(&source, &db, &mut observed).unwrap());
    assert!(!synchronize(&source, &mut db, &cancelled).unwrap());
    assert_eq!(sizes(&db), before);
    source.unreadable = false;
    assert!(synchronize(&source, &mut db, &cancelled).unwrap());
    assert_eq!(db.cursor(generation).unwrap().unwrap().next, 200);
    let after = sizes(&db);
    assert_eq!(after["中文.txt"], "84");
    assert_eq!(after["alias.txt"], "84");
    assert_eq!(after["neighbor.txt"], "16");
    assert!(!has_relevant_changes(&source, &db, &mut observed).unwrap());
}

#[test]
fn incremental_rolls_back_deferred_metadata_and_neighbors_if_cursor_write_fails() {
    use stashfinder_service::index::synchronize;
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("volume.sqlite");
    let mut db = metadata_delta_index(&file);
    let generation = db.active_generation().unwrap().unwrap();
    let connection = rusqlite::Connection::open(file).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_cursor BEFORE UPDATE OF next_usn ON generations
             BEGIN SELECT RAISE(ABORT, 'fixture cursor failure'); END;",
        )
        .unwrap();
    let error = synchronize(
        &MetadataDelta {
            target: 21,
            reason: 0,
            caught_up: false,
            unreadable: true,
        },
        &mut db,
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("fixture cursor failure"));
    assert_eq!(db.cursor(generation).unwrap().unwrap().next, 100);
    assert!(!db.has_pending_metadata(generation).unwrap());
    assert!(!db.has_object(generation, &22u64.to_le_bytes()).unwrap());
}

#[test]
fn incremental_does_not_defer_structural_reasons_or_a_different_file_id() {
    use stashfinder_service::index::synchronize;
    for (target, reason) in [
        (21, 0x100),
        (21, 0x200),
        (21, 0x2000),
        (21, 0x10000),
        (21 + (1 << 48), 0),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("volume.sqlite");
        let mut db = metadata_delta_index(&file);
        let generation = db.active_generation().unwrap().unwrap();
        let error = synchronize(
            &MetadataDelta {
                target,
                reason,
                caught_up: false,
                unreadable: true,
            },
            &mut db,
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("MFT 控制记录扇区尾部不一致"));
        assert_eq!(db.cursor(generation).unwrap().unwrap().next, 100);
        assert!(!db.has_object(generation, &22u64.to_le_bytes()).unwrap());
    }
}

#[test]
fn incremental_rejects_gap_read_failure_and_cancel_without_advancing_cursor() {
    for (first, next, journal_id, unreadable, cancel) in [
        (201, 300, 1, false, false),
        (0, 199, 1, false, false),
        (0, 300, 2, false, false),
        (0, 300, 1, true, false),
        (0, 300, 1, false, true),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut db = Volume::create(&temp.path().join("v.sqlite"), "{}").unwrap();
        build(
            &Fixture {
                changed: Cell::new(false),
                journal: true,
                broken: false,
                cancel: false,
            },
            &mut db,
            IndexMode::Journal,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        let generation = db.active_generation().unwrap().unwrap();
        assert!(
            stashfinder_service::index::synchronize(
                &Delta {
                    first,
                    next,
                    journal_id,
                    unreadable
                },
                &mut db,
                &AtomicBool::new(cancel)
            )
            .is_err()
        );
        assert_eq!(db.cursor(generation).unwrap().unwrap().next, 200);
        let index = SearchIndex::load(&db).unwrap().unwrap();
        assert_eq!(
            index
                .search(
                    &Query::parse("after.txt", false).unwrap(),
                    &[],
                    200,
                    &AtomicBool::new(false)
                )
                .unwrap()
                .rows
                .len(),
            1
        );
    }
}

struct Fixture {
    changed: Cell<bool>,
    journal: bool,
    broken: bool,
    cancel: bool,
}
fn entry(id: u64, name: &str, folder: bool, size: i64) -> DirectoryEntry {
    DirectoryEntry {
        id,
        name: name.encode_utf16().collect(),
        attributes: if folder { 16 } else { 0 },
        size,
        created: 123,
        modified: 456,
    }
}
impl Source for Fixture {
    fn root_id(&self) -> u64 {
        5
    }
    fn journal(&self) -> anyhow::Result<Option<Journal>> {
        Ok(self.journal.then_some(Journal {
            id: if self.broken && self.changed.get() {
                2
            } else {
                1
            },
            first: 0,
            next: if self.changed.get() { 200 } else { 100 },
        }))
    }
    fn create_journal(&self) -> anyhow::Result<Journal> {
        anyhow::bail!("must not create a journal implicitly")
    }
    fn inventory(
        &self,
        _: &AtomicBool,
        consume: &mut dyn FnMut(&[UsnRecord]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        consume(&[])
    }
    fn directory(
        &self,
        id: u64,
        cancelled: &AtomicBool,
        consume: &mut dyn FnMut(&[DirectoryEntry]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        assert_eq!(id, 5);
        consume(&[entry(20, "before.txt", false, 0)])?;
        self.changed.set(true);
        if self.cancel {
            cancelled.store(true, std::sync::atomic::Ordering::Release);
        }
        Ok(())
    }
    fn object(&self, id: u64) -> anyhow::Result<Option<NativeObject>> {
        Ok(match id {
            5 => Some(NativeObject {
                entry: entry(5, "\\", true, 0),
                links: vec![],
            }),
            20 => Some(NativeObject {
                entry: entry(20, "after.txt", false, 7),
                links: vec![(5, "after.txt".encode_utf16().collect())],
            }),
            _ => None,
        })
    }
    fn changes(&self, _: u64, start: i64) -> anyhow::Result<UsnPage> {
        Ok(UsnPage {
            next: 200,
            records: if start < 200 {
                vec![UsnRecord {
                    id: 20,
                    parent: 5,
                    usn: 150,
                    reason: 0x2000,
                    attributes: 0,
                    name: "after.txt".encode_utf16().collect(),
                }]
            } else {
                vec![]
            },
        })
    }
}

#[test]
fn scan_replays_rename_and_metadata_before_publication() {
    let temp = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&temp.path().join("v.sqlite"), "{}").unwrap();
    build(
        &Fixture {
            changed: Cell::new(false),
            journal: true,
            broken: false,
            cancel: false,
        },
        &mut db,
        IndexMode::Journal,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    let index = SearchIndex::load(&db).unwrap().unwrap();
    let results = index
        .search(
            &Query::parse("", false).unwrap(),
            &[],
            200,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(results.rows.len(), 1);
    assert_eq!(results.rows[0].path, "after.txt");
    assert_eq!(results.rows[0].size.as_deref(), Some("7"));
    assert_eq!(db.cursor(index.generation).unwrap().unwrap().next, 200);
}

#[test]
fn journal_loss_does_not_publish_and_missing_journal_requires_explicit_mode() {
    let temp = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&temp.path().join("v.sqlite"), "{}").unwrap();
    let absent = Fixture {
        changed: Cell::new(false),
        journal: false,
        broken: false,
        cancel: false,
    };
    assert!(
        build(
            &absent,
            &mut db,
            IndexMode::Journal,
            &AtomicBool::new(false),
            &mut |_| {}
        )
        .is_err()
    );
    assert_eq!(db.active_generation().unwrap(), None);
    build(
        &absent,
        &mut db,
        IndexMode::ScanOnly,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    let old = db.active_generation().unwrap();
    assert_eq!(db.index_status().unwrap().phase, "snapshot");
    assert!(
        build(
            &Fixture {
                changed: Cell::new(false),
                journal: true,
                broken: true,
                cancel: false,
            },
            &mut db,
            IndexMode::Journal,
            &AtomicBool::new(false),
            &mut |_| {}
        )
        .is_err()
    );
    assert_eq!(db.active_generation().unwrap(), old);
}

#[test]
fn cancellation_after_final_scan_batch_does_not_publish() {
    let temp = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&temp.path().join("v.sqlite"), "{}").unwrap();
    let source = Fixture {
        changed: Cell::new(false),
        journal: false,
        broken: false,
        cancel: true,
    };
    assert!(
        build(
            &source,
            &mut db,
            IndexMode::ScanOnly,
            &AtomicBool::new(false),
            &mut |_| {}
        )
        .is_err()
    );
    assert_eq!(db.active_generation().unwrap(), None);
}

struct ChangesFixture {
    scanned: Cell<bool>,
    deletion_reason: u32,
}
impl Source for ChangesFixture {
    fn root_id(&self) -> u64 {
        5
    }
    fn journal(&self) -> anyhow::Result<Option<Journal>> {
        Ok(Some(Journal {
            id: 1,
            first: 0,
            next: if self.scanned.get() { 1000 } else { 100 },
        }))
    }
    fn create_journal(&self) -> anyhow::Result<Journal> {
        anyhow::bail!("unexpected create")
    }
    fn inventory(
        &self,
        _: &AtomicBool,
        _: &mut dyn FnMut(&[UsnRecord]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    fn directory(
        &self,
        id: u64,
        _: &AtomicBool,
        consume: &mut dyn FnMut(&[DirectoryEntry]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        if id == 5 {
            consume(&[
                entry(20, "old-folder", true, 0),
                entry(22, "gone.txt", false, 1),
            ])?;
        } else {
            consume(&[entry(21, "child.txt", false, 1)])?;
            self.scanned.set(true);
        }
        Ok(())
    }
    fn object(&self, id: u64) -> anyhow::Result<Option<NativeObject>> {
        let (row, links) = match id {
            5 => (entry(5, "\\", true, 0), vec![]),
            20 => (entry(20, "renamed", true, 0), vec![(5, "renamed")]),
            21 => (entry(21, "child.txt", false, 9), vec![(20, "child.txt")]),
            23 => (
                entry(23, "new.txt", false, 0),
                vec![(20, "new.txt"), (5, "alias.txt")],
            ),
            _ => return Ok(None),
        };
        Ok(Some(NativeObject {
            entry: row,
            links: links
                .into_iter()
                .map(|(p, n)| (p, n.encode_utf16().collect()))
                .collect(),
        }))
    }
    fn changes(&self, _: u64, _: i64) -> anyhow::Result<UsnPage> {
        Ok(UsnPage {
            next: 1000,
            records: [
                (20, 0x2000),
                (21, 1),
                (22, self.deletion_reason),
                (23, 0x10000),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (id, reason))| UsnRecord {
                id,
                parent: 5,
                usn: 200 + i as i64 * 100,
                reason,
                attributes: 0,
                name: vec![65],
            })
            .collect(),
        })
    }
}

#[test]
fn replay_updates_directory_children_deletes_and_all_hard_links() {
    let temp = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&temp.path().join("v.sqlite"), "{}").unwrap();
    build(
        &ChangesFixture {
            scanned: Cell::new(false),
            deletion_reason: 0x200,
        },
        &mut db,
        IndexMode::Journal,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    let index = SearchIndex::load(&db).unwrap().unwrap();
    let result = index
        .search(
            &Query::parse(".txt", false).unwrap(),
            &[],
            200,
            &AtomicBool::new(false),
        )
        .unwrap();
    let mut paths = result
        .rows
        .iter()
        .map(|r| r.path.as_str())
        .collect::<Vec<_>>();
    paths.sort();
    assert_eq!(
        paths,
        ["alias.txt", r"renamed\child.txt", r"renamed\new.txt"]
    );
    assert_eq!(
        result
            .rows
            .iter()
            .find(|r| r.name == "child.txt")
            .unwrap()
            .size
            .as_deref(),
        Some("9")
    );
    assert_eq!(
        result
            .rows
            .iter()
            .filter(|r| r.object == 23u64.to_le_bytes())
            .count(),
        2
    );
}

#[test]
fn missing_object_without_delete_record_does_not_remove_persisted_path() {
    let temp = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&temp.path().join("v.sqlite"), "{}").unwrap();
    let error = build(
        &ChangesFixture {
            scanned: Cell::new(false),
            deletion_reason: 1,
        },
        &mut db,
        IndexMode::Journal,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_err();
    assert!(error.to_string().contains("没有删除证据"));
    assert_eq!(db.active_generation().unwrap(), None);
    let index = SearchIndex::load(&db).unwrap().unwrap();
    assert!(index.partial);
    assert_eq!(
        index
            .search(
                &Query::parse("gone.txt", false).unwrap(),
                &[],
                200,
                &AtomicBool::new(false)
            )
            .unwrap()
            .rows
            .len(),
        1
    );
}

#[test]
fn scan_guard_rejects_changed_snapshot_before_publication() {
    let temp = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&temp.path().join("v.sqlite"), "{}").unwrap();
    let source = Fixture {
        changed: Cell::new(false),
        journal: false,
        broken: false,
        cancel: false,
    };
    assert!(
        stashfinder_service::index::build_checked(
            &source,
            &mut db,
            IndexMode::ScanOnly,
            &AtomicBool::new(false),
            &mut |_| {},
            &mut || anyhow::bail!("monitor changed")
        )
        .is_err()
    );
    assert_eq!(db.active_generation().unwrap(), None);
}

struct SmallDirectories;
impl Source for SmallDirectories {
    fn root_id(&self) -> u64 {
        5
    }
    fn journal(&self) -> anyhow::Result<Option<Journal>> {
        Ok(Some(Journal {
            id: 1,
            first: 0,
            next: 100,
        }))
    }
    fn create_journal(&self) -> anyhow::Result<Journal> {
        panic!("unexpected journal creation")
    }
    fn inventory(
        &self,
        _: &AtomicBool,
        _: &mut dyn FnMut(&[UsnRecord]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    fn directory(
        &self,
        id: u64,
        _: &AtomicBool,
        consume: &mut dyn FnMut(&[DirectoryEntry]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        if id == 5 {
            for child in 20..148 {
                consume(&[entry(child, &format!("folder-{child}"), true, 0)])?;
            }
        } else {
            consume(&[
                entry(1000, "shared.txt", false, 7),
                entry(2000 + id * 2, "中文.txt", false, 0),
                entry(2001 + id * 2, "last.txt", false, 42),
            ])?;
        }
        Ok(())
    }
    fn object(&self, id: u64) -> anyhow::Result<Option<NativeObject>> {
        assert_eq!(id, 5);
        Ok(Some(NativeObject {
            entry: entry(5, "\\", true, 0),
            links: vec![],
        }))
    }
    fn changes(&self, _: u64, _: i64) -> anyhow::Result<UsnPage> {
        panic!("unchanged journal")
    }
}

#[test]
fn small_directories_share_durable_batches_and_preserve_all_links() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("volume.sqlite");
    let mut db = Volume::create(&file, "{}").unwrap();
    build(
        &SmallDirectories,
        &mut db,
        IndexMode::Journal,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(db.index_status().unwrap().records, 512);
    let index = SearchIndex::load(&db).unwrap().unwrap();
    let hits = index
        .search(
            &Query::parse("shared.txt", false).unwrap(),
            &[],
            200,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(hits.rows.len(), 128);
    assert!(hits.rows.iter().all(|row| row.size.as_deref() == Some("7")));
    assert!(
        index.revision < 20,
        "Small pages caused {} durable writes",
        index.revision
    );
    drop(db);
    let reopened = Volume::open(&file).unwrap();
    assert_eq!(reopened.index_status().unwrap().records, 512);
}

struct RemovedDirectory {
    scanned: Cell<bool>,
    delete_record: bool,
}
impl Source for RemovedDirectory {
    fn root_id(&self) -> u64 {
        5
    }
    fn journal(&self) -> anyhow::Result<Option<Journal>> {
        Ok(Some(Journal {
            id: 1,
            first: 0,
            next: if self.scanned.get() { 200 } else { 100 },
        }))
    }
    fn create_journal(&self) -> anyhow::Result<Journal> {
        panic!("unexpected journal creation")
    }
    fn inventory(
        &self,
        _: &AtomicBool,
        _: &mut dyn FnMut(&[UsnRecord]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    fn directory(
        &self,
        id: u64,
        _: &AtomicBool,
        consume: &mut dyn FnMut(&[DirectoryEntry]) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        if id == 5 {
            consume(&[
                entry(20, "temporary", true, 0),
                entry(21, "kept.txt", false, 7),
            ])
        } else {
            self.scanned.set(true);
            anyhow::bail!("directory disappeared during scan")
        }
    }
    fn object(&self, id: u64) -> anyhow::Result<Option<NativeObject>> {
        Ok((id == 5).then(|| NativeObject {
            entry: entry(5, "\\", true, 0),
            links: vec![],
        }))
    }
    fn changes(&self, _: u64, _: i64) -> anyhow::Result<UsnPage> {
        Ok(UsnPage {
            next: 200,
            records: if self.delete_record {
                vec![UsnRecord {
                    id: 20,
                    parent: 5,
                    usn: 150,
                    reason: 0x200,
                    attributes: 16,
                    name: "temporary".encode_utf16().collect(),
                }]
            } else {
                vec![]
            },
        })
    }
}

#[test]
fn directory_removed_during_initial_scan_requires_journal_delete_evidence() {
    for delete_record in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let mut db = Volume::create(&root.path().join("v.sqlite"), "{}").unwrap();
        let result = build(
            &RemovedDirectory {
                scanned: Cell::new(false),
                delete_record,
            },
            &mut db,
            IndexMode::Journal,
            &AtomicBool::new(false),
            &mut |_| {},
        );
        if delete_record {
            result.unwrap();
            assert_eq!(db.index_status().unwrap().records, 1);
            let hits = SearchIndex::load(&db)
                .unwrap()
                .unwrap()
                .search(
                    &Query::parse("", false).unwrap(),
                    &[],
                    200,
                    &AtomicBool::new(false),
                )
                .unwrap();
            assert_eq!(hits.rows[0].name, "kept.txt");
        } else {
            assert!(result.is_err());
            assert_eq!(db.active_generation().unwrap(), None);
        }
    }
}

#[test]
fn resumed_sync_prunes_existing_runtime_paths_even_without_new_events() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&dir.path().join("v.sqlite"), "{}").unwrap();
    let mut source = FastFixture {
        failure: None,
        fast: Cell::new(0),
        legacy: Cell::new(0),
        journals: Cell::new(0),
    };
    build(
        &source,
        &mut db,
        IndexMode::Journal,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(db.index_status().unwrap().records, 3);
    source.failure = Some("exclude");
    assert!(
        stashfinder_service::index::synchronize(&source, &mut db, &AtomicBool::new(false)).unwrap()
    );
    assert_eq!(db.index_status().unwrap().records, 1);
    assert!(
        !stashfinder_service::index::synchronize(&source, &mut db, &AtomicBool::new(false))
            .unwrap()
    );
}
