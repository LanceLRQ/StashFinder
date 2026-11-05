use stashfinder_windows::scan::Scanner;
use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[test]
fn enumerates_unicode_zero_size_and_notifications_without_polling_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("中文")).unwrap();
    std::fs::write(dir.path().join("empty.txt"), []).unwrap();
    let mut scanner = Scanner::open(dir.path()).unwrap();
    assert!(!scanner.changed().unwrap());
    let mut rows = Vec::new();
    scanner
        .enumerate(&[], &AtomicBool::new(false), &mut |batch| {
            rows.extend_from_slice(batch);
            Ok(())
        })
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .any(|r| String::from_utf16_lossy(&r.name) == "empty.txt" && r.size == Some(0))
    );
    assert!(
        rows.iter()
            .any(|r| String::from_utf16_lossy(&r.name) == "中文" && r.attributes & 16 != 0)
    );
    for action in 0..3 {
        match action {
            0 => std::fs::write(dir.path().join("中文/new.txt"), b"123").unwrap(),
            1 => std::fs::rename(
                dir.path().join("中文/new.txt"),
                dir.path().join("中文/renamed.txt"),
            )
            .unwrap(),
            _ => std::fs::remove_file(dir.path().join("中文/renamed.txt")).unwrap(),
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while !scanner.changed().unwrap() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    assert!(
        scanner
            .enumerate(&[], &AtomicBool::new(true), &mut |_| Ok(()))
            .is_err()
    );
}
