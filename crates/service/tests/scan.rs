use anyhow::{Result, bail};
use stashfinder_core::search::Query;
use stashfinder_service::scan::{Source, build};
use stashfinder_store::{search::SearchIndex, volume::Volume};
use stashfinder_windows::scan::ScanEntry;
use std::{collections::VecDeque, sync::atomic::AtomicBool};
struct Fixture {
    changes: VecDeque<bool>,
    broken: bool,
    name: String,
}
impl Source for Fixture {
    fn verify(&self) -> Result<()> {
        Ok(())
    }
    fn changed(&mut self) -> Result<bool> {
        Ok(self.changes.pop_front().unwrap_or(false))
    }
    fn enumerate(
        &self,
        relative: &[u16],
        _: &AtomicBool,
        consume: &mut dyn FnMut(&[ScanEntry]) -> Result<()>,
    ) -> Result<()> {
        if self.broken {
            bail!("拒绝访问：test");
        }
        assert!(relative.is_empty());
        consume(&[
            ScanEntry {
                name: self.name.encode_utf16().collect(),
                size: Some(0),
                created: None,
                modified: None,
                attributes: 0,
            },
            ScanEntry {
                name: "junction".encode_utf16().collect(),
                size: None,
                created: None,
                modified: None,
                attributes: 16 | 1024,
            },
        ])
    }
}
fn names(db: &Volume) -> Vec<String> {
    SearchIndex::load(db)
        .unwrap()
        .unwrap()
        .search(
            &Query::parse("", false).unwrap(),
            &[],
            100,
            &AtomicBool::new(false),
        )
        .unwrap()
        .rows
        .into_iter()
        .map(|r| r.name)
        .collect()
}
#[test]
fn scan_replaces_only_after_stable_complete_enumeration_and_preserves_on_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Volume::create(&dir.path().join("v.sqlite"), "{}").unwrap();
    let mut source = Fixture {
        changes: VecDeque::new(),
        broken: false,
        name: "旧.txt".into(),
    };
    build(&mut source, &mut db, &AtomicBool::new(false)).unwrap();
    assert!(names(&db).contains(&"旧.txt".into()));
    let old = db.active_generation().unwrap();
    source.name = "new.txt".into();
    source.broken = true;
    assert!(build(&mut source, &mut db, &AtomicBool::new(false)).is_err());
    assert_eq!(db.active_generation().unwrap(), old);
    assert!(names(&db).contains(&"旧.txt".into()));
    source.broken = false;
    source.changes = VecDeque::from([false, true, false, false]);
    build(&mut source, &mut db, &AtomicBool::new(false)).unwrap();
    assert!(!names(&db).contains(&"旧.txt".into()));
    assert!(names(&db).contains(&"new.txt".into()));
    let stable = db.active_generation().unwrap();
    source.changes = VecDeque::from([true; 8]);
    assert!(build(&mut source, &mut db, &AtomicBool::new(false)).is_err());
    assert_eq!(db.active_generation().unwrap(), stable);
    assert!(build(&mut source, &mut db, &AtomicBool::new(true)).is_err());
}
