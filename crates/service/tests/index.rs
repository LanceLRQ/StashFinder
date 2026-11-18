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

#[cfg(feature = "memory-index-bench")]
#[test]
fn parallel_snapshot_requires_successful_search_before_publication() {
    use stashfinder_service::index::direct::build_direct_parallel;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.sqlite");
    let cancel = AtomicBool::new(false);
    for fail in [true, false] {
        let source = FastFixture {
            failure: None,
            fast: Cell::new(0),
            legacy: Cell::new(0),
            journals: Cell::new(0),
        };
        let mut phases = Vec::new();
        let result = build_direct_parallel(
            &source,
            &cancel,
            &path,
            &mut |phase| phases.push(phase.to_owned()),
            |index| {
                assert!(!path.exists());
                assert_eq!(
                    index
                        .search(&Query::parse("alias.txt", false)?, &[], 1, &cancel)?
                        .rows
                        .len(),
                    1
                );
                anyhow::ensure!(!fail, "search probe failed");
                Ok(())
            },
        );
        assert_eq!(
            phases,
            [
                "scanning",
                "replaying",
                "publishing",
                "search_build",
                "search_built"
            ]
        );
        assert_eq!(result.is_ok(), !fail);
        assert_eq!(path.exists(), !fail);
    }
    let offline = Volume::open(&path).unwrap();
    assert_eq!(
        offline
            .cursor(offline.active_generation().unwrap().unwrap())
            .unwrap()
            .unwrap()
            .next,
        100
    );
}
#[cfg(feature = "memory-index-bench")]
#[test]
fn direct_memory_matches_disk_search_and_persisted_cursor() {
    use stashfinder_service::index::direct::build_direct;
    for (failure, records, next) in [
        (None, 3, 100),
        (Some("replay"), 1, 200),
        (Some("paged_delete"), 1, 300),
        (Some("exclude"), 1, 100),
        (Some("reparse"), 2, 100),
        (Some("directory_delete"), 1, 200),
        (Some("move"), 3, 200),
    ] {
        let source = || FastFixture {
            failure,
            fast: Cell::new(0),
            legacy: Cell::new(0),
            journals: Cell::new(0),
        };
        let dir = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        let mut baseline = Volume::create(&dir.path().join("baseline.sqlite"), "{}").unwrap();
        let before = build(
            &source(),
            &mut baseline,
            IndexMode::Journal,
            &cancel,
            &mut |_| {},
        )
        .unwrap();
        let candidate = build_direct(&source(), &cancel, &mut |_| {}).unwrap();
        let query = Query::parse("", false).unwrap();
        let rows = |index: &SearchIndex| {
            serde_json::to_value(index.search(&query, &[], 100, &cancel).unwrap().rows).unwrap()
        };
        assert_eq!(candidate.cursor.next, next);
        assert_eq!(
            candidate
                .index
                .search(&query, &[], 100, &cancel)
                .unwrap()
                .rows
                .len(),
            records
        );
        assert_eq!(rows(&candidate.index), rows(&before));
        let path = dir.path().join("direct.sqlite");
        candidate.persist(&path, &cancel, &mut || Ok(())).unwrap();
        let offline = Volume::open(&path).unwrap();
        let generation = offline.active_generation().unwrap().unwrap();
        assert_eq!(offline.cursor(generation).unwrap().unwrap().next, next);
        assert_eq!(
            rows(&SearchIndex::load(&offline).unwrap().unwrap()),
            rows(&before)
        );
    }
}

#[cfg(feature = "memory-index-bench")]
#[test]
fn direct_memory_remains_searchable_while_its_writer_is_running() {
    use stashfinder_service::index::direct::build_direct;
    let source = FastFixture {
        failure: None,
        fast: Cell::new(0),
        legacy: Cell::new(0),
        journals: Cell::new(0),
    };
    let cancel = AtomicBool::new(false);
    let candidate = build_direct(&source, &cancel, &mut |_| {}).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.sqlite");
    std::thread::scope(|scope| {
        let (started, writing) = std::sync::mpsc::sync_channel(0);
        let (resume, searched) = std::sync::mpsc::sync_channel(0);
        let snapshot = &candidate;
        let target = &path;
        let cancelled = &cancel;
        let writer = scope.spawn(move || {
            let mut first = true;
            snapshot.persist(target, cancelled, &mut || {
                if std::mem::take(&mut first) {
                    started.send(())?;
                    searched.recv()?;
                }
                Ok(())
            })
        });
        writing.recv().unwrap();
        let unpublished = !path.exists();
        let found =
            candidate
                .index
                .search(&Query::parse("alias.txt", false).unwrap(), &[], 1, &cancel);
        resume.send(()).unwrap();
        writer.join().unwrap().unwrap();
        assert!(unpublished);
        assert_eq!(found.unwrap().rows[0].size.as_deref(), Some("42"));
        assert!(path.exists());
    });
}

#[cfg(feature = "memory-index-bench")]
#[test]
fn direct_memory_rejects_incomplete_or_ambiguous_scans() {
    use stashfinder_service::index::direct::build_direct;
    for failure in [
        "orphan",
        "duplicate",
        "path_conflict",
        "io",
        "missing",
        "reused",
        "cycle",
        "file_parent",
    ] {
        let source = FastFixture {
            failure: Some(failure),
            fast: Cell::new(0),
            legacy: Cell::new(0),
            journals: Cell::new(0),
        };
        assert!(
            build_direct(&source, &AtomicBool::new(false), &mut |_| {}).is_err(),
            "{failure}"
        );
    }
}

#[cfg(feature = "memory-index-bench")]
#[test]
fn direct_memory_cancelled_save_never_publishes_or_overwrites_a_file() {
    use stashfinder_service::index::direct::build_direct;
    use std::sync::atomic::Ordering;
    let source = FastFixture {
        failure: None,
        fast: Cell::new(0),
        legacy: Cell::new(0),
        journals: Cell::new(0),
    };
    let cancel = AtomicBool::new(false);
    let candidate = build_direct(&source, &cancel, &mut |_| {}).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.sqlite");
    assert!(
        candidate
            .persist(&path, &cancel, &mut || {
                cancel.store(true, Ordering::Relaxed);
                Ok(())
            })
            .is_err()
    );
    assert!(!path.exists());
    cancel.store(false, Ordering::Relaxed);
    std::fs::write(&path, b"old snapshot").unwrap();
    assert!(candidate.persist(&path, &cancel, &mut || Ok(())).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"old snapshot");
    cancel.store(true, Ordering::Relaxed);
    assert!(build_direct(&source, &cancel, &mut |_| {}).is_err());
}

impl Source for FastFixture {
    fn verify(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.failure != Some("changed_before_commit") || self.journals.get() < 3,
            "source changed before commit"
        );
        Ok(())
    }
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
            } else if matches!(
                self.failure,
                Some("replay" | "directory_delete" | "move" | "replacement")
            ) && self.journals.get() > 1
            {
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
                entry: {
                    let mut folder = entry(20, "folder", true, 0);
                    if self.failure == Some("reparse") {
                        folder.attributes |= 0x400;
                    }
                    if self.failure == Some("file_parent") {
                        folder.attributes = 32;
                    }
                    folder
                },
                links: vec![(
                    if self.failure == Some("cycle") { 20 } else { 5 },
                    link("folder"),
                )],
            }])?,
        }
        if self.failure == Some("replacement") {
            consume(&[NativeObject {
                entry: entry(22, "alias.txt", false, 23),
                links: vec![(5, link("alias.txt"))],
            }])?;
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
        if self.failure == Some("replacement") {
            if id == 21 {
                return Ok(None);
            }
            if id == 22 {
                return Ok(Some(NativeObject {
                    entry: entry(22, "alias.txt", false, 23),
                    links: vec![(5, "alias.txt".encode_utf16().collect())],
                }));
            }
        }
        if self.failure == Some("directory_delete") && id == 20 {
            return Ok(None);
        }
        if self.failure == Some("move") {
            if id == 21 {
                return Ok(Some(NativeObject {
                    entry: entry(21, "moved.txt", false, 99),
                    links: vec![(30, "moved.txt".encode_utf16().collect())],
                }));
            }
            if id == 30 {
                return Ok(Some(NativeObject {
                    entry: entry(30, "new", true, 0),
                    links: vec![(5, "new".encode_utf16().collect())],
                }));
            }
        }
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
        if self.failure == Some("replacement") {
            assert_eq!((id, start), (1, 100));
            return Ok(UsnPage {
                next: 200,
                records: vec![
                    UsnRecord {
                        id: 21,
                        parent: 5,
                        usn: 140,
                        reason: 0x200,
                        attributes: 32,
                        name: vec![],
                    },
                    UsnRecord {
                        id: 22,
                        parent: 5,
                        usn: 150,
                        reason: 0x100,
                        attributes: 32,
                        name: vec![],
                    },
                ],
            });
        }
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
        assert!(matches!(
            self.failure,
            Some("replay" | "directory_delete" | "move")
        ));
        assert_eq!((id, start), (1, 100));
        Ok(UsnPage {
            next: 200,
            records: vec![UsnRecord {
                id: if self.failure == Some("directory_delete") {
                    20
                } else {
                    21
                },
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
                .map(drop)
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
    let (report, _) = build_with_strategy(
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
fn full_build_returns_the_committed_search_snapshot() {
    for mode in [IndexMode::Journal, IndexMode::ScanOnly] {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Volume::create(&dir.path().join("v.sqlite"), "{}").unwrap();
        let source = FastFixture {
            failure: None,
            fast: Cell::new(0),
            legacy: Cell::new(0),
            journals: Cell::new(0),
        };
        let cancelled = AtomicBool::new(false);
        let cached = build(&source, &mut db, mode, &cancelled, &mut |_| {}).unwrap();
        let fresh = SearchIndex::load(&db).unwrap().unwrap();
        assert_eq!(db.active_generation().unwrap(), Some(cached.generation));
        assert_eq!(cached.revision, fresh.revision);
        assert_eq!(cached.indexed_at, fresh.indexed_at);
        assert_eq!(cached.needs_validation, matches!(mode, IndexMode::ScanOnly));
        assert!(!cached.partial);
        let query = Query::parse("", false).unwrap();
        let as_json = |index: &SearchIndex| {
            serde_json::to_value(index.search(&query, &[], 200, &cancelled).unwrap()).unwrap()
        };
        assert_eq!(as_json(&cached), as_json(&fresh));
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
            let (report, _) = result.unwrap();
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

#[cfg(feature = "memory-index-bench")]
#[test]
fn memory_benchmark_search_and_offline_copy_use_the_same_replayed_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("offline.sqlite");
    let mut memory = Volume::create_in_memory("{}").unwrap();
    let source = FastFixture {
        failure: Some("replay"),
        fast: Cell::new(0),
        legacy: Cell::new(0),
        journals: Cell::new(0),
    };
    let cancelled = AtomicBool::new(false);
    let index = build(
        &source,
        &mut memory,
        IndexMode::Journal,
        &cancelled,
        &mut |_| {},
    )
    .unwrap();
    let alias = Query::parse("alias.txt", false).unwrap();
    let folder = Query::parse("folder", false).unwrap();
    assert!(
        index
            .search(&alias, &[], 1, &cancelled)
            .unwrap()
            .rows
            .is_empty()
    );
    assert_eq!(
        index
            .search(&folder, &[], 1, &cancelled)
            .unwrap()
            .rows
            .len(),
        1
    );
    assert!(!path.exists());
    memory
        .backup_completed_index(&path, &cancelled, &mut || Ok(()))
        .unwrap();
    drop(memory);

    let db = Volume::open(&path).unwrap();
    let generation = db.active_generation().unwrap().unwrap();
    assert_eq!(db.cursor(generation).unwrap().unwrap().next, 200);
    let offline = SearchIndex::load(&db).unwrap().unwrap();
    assert!(
        offline
            .search(&alias, &[], 1, &cancelled)
            .unwrap()
            .rows
            .is_empty()
    );
    assert_eq!(
        offline
            .search(&folder, &[], 1, &cancelled)
            .unwrap()
            .rows
            .len(),
        1
    );
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

#[test]
fn registered_first_build_keeps_ownership_and_publishes_the_search_revision() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("registered.sqlite");
    let mut db = Volume::create(&file, "registered-volume").unwrap();
    let source = FastFixture {
        failure: None,
        fast: Cell::new(0),
        legacy: Cell::new(0),
        journals: Cell::new(0),
    };
    let cancel = AtomicBool::new(false);
    let index = stashfinder_service::index::build_registered(
        &source,
        &mut db,
        IndexMode::Journal,
        &cancel,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(source.fast.get(), 1);
    assert_eq!(source.legacy.get(), 0);
    assert_eq!(db.manifest().unwrap(), "registered-volume");
    drop(db);
    let saved = Volume::open(&file).unwrap();
    let loaded = SearchIndex::load(&saved).unwrap().unwrap();
    assert_eq!(
        (
            index.generation,
            index.revision,
            index.indexed_at,
            index.needs_validation
        ),
        (
            loaded.generation,
            loaded.revision,
            loaded.indexed_at,
            loaded.needs_validation
        )
    );
    assert_eq!(saved.cursor(index.generation).unwrap().unwrap().next, 100);
    let query = Query::parse("alias.txt", false).unwrap();
    assert_eq!(
        index.search(&query, &[], 1, &cancel).unwrap().rows[0]
            .size
            .as_deref(),
        Some("42")
    );
    assert_eq!(
        loaded.search(&query, &[], 1, &cancel).unwrap().rows.len(),
        1
    );
}

#[test]
fn registered_first_build_replays_a_same_name_replacement_before_falling_back() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("registered.sqlite");
    let mut db = Volume::create(&file, "owned").unwrap();
    let source = FastFixture {
        failure: Some("replacement"),
        fast: Cell::new(0),
        legacy: Cell::new(0),
        journals: Cell::new(0),
    };
    let cancel = AtomicBool::new(false);
    let index = stashfinder_service::index::build_registered(
        &source,
        &mut db,
        IndexMode::Journal,
        &cancel,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!((source.fast.get(), source.legacy.get()), (1, 0));
    let query = Query::parse("alias.txt", false).unwrap();
    let check = |index: &SearchIndex| {
        let rows = index.search(&query, &[], 10, &cancel).unwrap().rows;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].size.as_deref(), Some("23"));
    };
    check(&index);
    assert_eq!(db.cursor(index.generation).unwrap().unwrap().next, 200);
    drop(db);
    check(
        &SearchIndex::load(&Volume::open(&file).unwrap())
            .unwrap()
            .unwrap(),
    );
}

#[test]
fn registered_first_build_rolls_back_on_identity_change_and_can_retry_or_fall_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("registered.sqlite");
    let mut db = Volume::create(&path, "owned").unwrap();
    let mut source = FastFixture {
        failure: Some("changed_before_commit"),
        fast: Cell::new(0),
        legacy: Cell::new(0),
        journals: Cell::new(0),
    };
    let cancel = AtomicBool::new(false);
    let result = stashfinder_service::index::build_registered(
        &source,
        &mut db,
        IndexMode::Journal,
        &cancel,
        &mut |_| {},
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("source changed before commit")
    );
    assert!(!db.has_generations().unwrap());
    assert_eq!(db.manifest().unwrap(), "owned");
    source.failure = Some("unsupported");
    let index = stashfinder_service::index::build_registered(
        &source,
        &mut db,
        IndexMode::Journal,
        &cancel,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(source.legacy.get(), 1);
    assert_eq!(db.active_generation().unwrap(), Some(index.generation));
}
