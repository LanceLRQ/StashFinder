use anyhow::Result;
use stashfinder_service::index::{self, ObservedCursor, Source};
use stashfinder_store::volume::Volume;
use stashfinder_windows::ntfs::{DirectoryEntry, Journal, NativeObject, UsnPage, UsnRecord};
use std::sync::atomic::AtomicBool;
struct Disk {
    end: i64,
    step: i64,
    parent: u64,
    reads: std::cell::Cell<usize>,
}
impl Source for Disk {
    fn root_id(&self) -> u64 {
        5
    }
    fn excluded_roots(&self) -> Vec<u64> {
        vec![20]
    }
    fn excluded_directory(&self, id: u64) -> Result<bool> {
        Ok(id == 20)
    }
    fn journal(&self) -> Result<Option<Journal>> {
        Ok(Some(Journal {
            id: 1,
            first: 0,
            next: self.end,
        }))
    }
    fn create_journal(&self) -> Result<Journal> {
        unreachable!()
    }
    fn inventory(
        &self,
        _: &AtomicBool,
        _: &mut dyn FnMut(&[UsnRecord]) -> Result<()>,
    ) -> Result<()> {
        Ok(())
    }
    fn directory(
        &self,
        _: u64,
        _: &AtomicBool,
        _: &mut dyn FnMut(&[DirectoryEntry]) -> Result<()>,
    ) -> Result<()> {
        Ok(())
    }
    fn object(&self, id: u64) -> Result<Option<NativeObject>> {
        if self.parent >= 98 {
            return Ok(None);
        }
        Ok(Some(NativeObject {
            entry: DirectoryEntry {
                id,
                name: vec![65],
                attributes: if id == 5 { 16 } else { 0 },
                size: 7,
                created: 1,
                modified: 2,
            },
            links: if id == 5 {
                vec![]
            } else {
                vec![(self.parent, vec![65])]
            },
        }))
    }
    fn changes(&self, _: u64, start: i64) -> Result<UsnPage> {
        self.reads.set(self.reads.get() + 1);
        let next = if self.step == 0 {
            self.end
        } else {
            (start + self.step).min(self.end)
        };
        let mut records = vec![UsnRecord {
            id: 21,
            parent: if next < self.end { 20 } else { self.parent },
            usn: start,
            reason: 1,
            attributes: 0,
            name: vec![65],
        }];
        if self.parent == 99 {
            records[0].reason = 0x100;
            records.push(UsnRecord {
                id: 21,
                parent: 20,
                usn: start + 1,
                reason: 0x200,
                attributes: 0,
                name: vec![65],
            });
        }
        Ok(UsnPage {
            next: next as u64,
            records,
        })
    }
}
#[test]
fn ignored_changes_advance_only_memory_and_normal_changes_remain_pending() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("v.sqlite");
    let mut db = Volume::create(&file, "{}").unwrap();
    let g = db.begin_generation().unwrap();
    db.write_batch(
        g,
        &[],
        &[],
        &[],
        Some(&stashfinder_store::volume::Cursor {
            journal: 1u64.to_le_bytes().to_vec(),
            next: 100,
        }),
    )
    .unwrap();
    db.complete(g).unwrap();
    db.publish(g).unwrap();
    let mut observed = ObservedCursor::default();
    let mut disk = Disk {
        end: 200,
        step: 0,
        parent: 20,
        reads: Default::default(),
    };
    assert!(!index::has_relevant_changes(&disk, &db, &mut observed).unwrap());
    assert_eq!(db.cursor(g).unwrap().unwrap().next, 100);
    let reads = disk.reads.get();
    assert!(!index::has_relevant_changes(&disk, &db, &mut observed).unwrap());
    assert_eq!(disk.reads.get(), reads);
    disk.end = 300;
    disk.parent = 5;
    assert!(index::has_relevant_changes(&disk, &db, &mut observed).unwrap());
    assert_eq!(db.cursor(g).unwrap().unwrap().next, 100);
    let mut restarted = ObservedCursor::default();
    assert!(index::has_relevant_changes(&disk, &db, &mut restarted).unwrap());
    assert!(index::synchronize(&disk, &mut db, &AtomicBool::new(false)).unwrap());
    assert_eq!(db.cursor(g).unwrap().unwrap().next, 300);
    assert!(db.has_object(g, &21u64.to_le_bytes()).unwrap());
}

#[test]
fn ignored_incremental_batch_does_not_write_revision_or_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&dir.path().join("v.sqlite"), "{}").unwrap();
    let g = db.begin_generation().unwrap();
    db.write_batch(
        g,
        &[],
        &[],
        &[],
        Some(&stashfinder_store::volume::Cursor {
            journal: 1u64.to_le_bytes().to_vec(),
            next: 100,
        }),
    )
    .unwrap();
    db.complete(g).unwrap();
    db.publish(g).unwrap();
    let before = db.index_status().unwrap();
    let sql = rusqlite::Connection::open(dir.path().join("v.sqlite")).unwrap();
    let revision: i64 = sql
        .query_row("SELECT revision FROM generations WHERE id=?1", [g], |r| {
            r.get(0)
        })
        .unwrap();
    let disk = Disk {
        end: 200,
        step: 0,
        parent: 20,
        reads: Default::default(),
    };
    assert!(!index::synchronize(&disk, &mut db, &AtomicBool::new(false)).unwrap());
    assert_eq!(db.cursor(g).unwrap().unwrap().next, 100);
    assert_eq!(
        revision,
        sql.query_row("SELECT revision FROM generations WHERE id=?1", [g], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap()
    );
    assert_eq!(
        serde_json::to_value(db.index_status().unwrap()).unwrap(),
        serde_json::to_value(before).unwrap()
    );
}

#[test]
fn many_ignored_pages_do_not_starve_a_real_change() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&dir.path().join("v.sqlite"), "{}").unwrap();
    let g = db.begin_generation().unwrap();
    db.write_batch(
        g,
        &[],
        &[],
        &[],
        Some(&stashfinder_store::volume::Cursor {
            journal: 1u64.to_le_bytes().to_vec(),
            next: 100,
        }),
    )
    .unwrap();
    db.complete(g).unwrap();
    db.publish(g).unwrap();
    let disk = Disk {
        end: 6800,
        step: 100,
        parent: 5,
        reads: Default::default(),
    };
    let mut observed = ObservedCursor::default();
    assert!(!index::has_relevant_changes(&disk, &db, &mut observed).unwrap());
    assert!(index::has_relevant_changes(&disk, &db, &mut observed).unwrap());
    assert!(index::synchronize(&disk, &mut db, &AtomicBool::new(false)).unwrap());
    assert_eq!(db.cursor(g).unwrap().unwrap().next, 6800);
    assert!(db.has_object(g, &21u64.to_le_bytes()).unwrap());
}

#[test]
fn deleted_unindexed_temporary_files_do_not_schedule_or_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&dir.path().join("v.sqlite"), "{}").unwrap();
    let g = db.begin_generation().unwrap();
    db.write_batch(
        g,
        &[],
        &[],
        &[],
        Some(&stashfinder_store::volume::Cursor {
            journal: 1u64.to_le_bytes().to_vec(),
            next: 100,
        }),
    )
    .unwrap();
    db.complete(g).unwrap();
    db.publish(g).unwrap();
    let disk = Disk {
        end: 200,
        step: 0,
        parent: 99,
        reads: Default::default(),
    };
    assert!(!index::has_relevant_changes(&disk, &db, &mut ObservedCursor::default()).unwrap());
    assert!(!index::synchronize(&disk, &mut db, &AtomicBool::new(false)).unwrap());
    assert_eq!(db.cursor(g).unwrap().unwrap().next, 100);
    let missing = Disk { parent: 98, ..disk };
    assert!(index::has_relevant_changes(&missing, &db, &mut ObservedCursor::default()).is_err());
}
