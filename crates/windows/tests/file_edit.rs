use stashfinder_windows::file_operations::{Edit, execute};
use std::sync::{Arc, atomic::AtomicBool};
#[test]
fn cancelled_operations_do_not_change_generated_files() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("source.txt");
    std::fs::write(&file, b"keep").unwrap();
    let report = execute(
        std::slice::from_ref(&file),
        &[None],
        None,
        &Edit::Delete,
        0,
        Arc::new(AtomicBool::new(true)),
    )
    .unwrap();
    assert!(report.cancelled);
    assert_eq!(std::fs::read(file).unwrap(), b"keep");
}
#[test]
fn renames_and_deletes_only_generated_paths_and_preserves_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("source.txt");
    std::fs::write(&file, b"source").unwrap();
    let collision = dir.path().join("keep.txt");
    std::fs::write(&collision, b"keep").unwrap();
    assert!(
        execute(
            std::slice::from_ref(&file),
            &[None],
            None,
            &Edit::Rename("keep.txt".into()),
            0,
            Arc::new(AtomicBool::new(false))
        )
        .is_err()
    );
    let report = execute(
        std::slice::from_ref(&file),
        &[None],
        None,
        &Edit::Rename("renamed.txt".into()),
        0,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(report.completed(), 1, "{report:?}");
    let renamed = dir.path().join("renamed.txt");
    assert_eq!(std::fs::read(&renamed).unwrap(), b"source");
    let report = execute(
        &[renamed],
        &[None],
        None,
        &Edit::Delete,
        0,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(report.completed(), 1);
    assert_eq!(std::fs::read(collision).unwrap(), b"keep");
    assert!(!dir.path().join("renamed.txt").exists());
}
