use stashfinder_windows::{ipc, scan::Scanner};
use std::{
    fs::OpenOptions,
    os::windows::fs::OpenOptionsExt,
    path::PathBuf,
    sync::atomic::AtomicBool,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE,
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

#[test]
#[ignore = "requires the authorized M6 VHD and a controller that remounts it after watch-ready"]
fn a_watcher_recovers_after_remount_even_without_an_offline_observation() {
    let manifest = PathBuf::from(std::env::var_os("SF_M6_MANIFEST").unwrap());
    assert!(manifest.starts_with(r"D:\tmp"));
    assert_eq!(manifest.file_name().unwrap(), "verification.json");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap();
    let image = PathBuf::from(saved["image"].as_str().unwrap());
    assert_eq!(image.parent(), manifest.parent());
    assert_eq!(image.file_name().unwrap(), "file-operations.vhdx");
    assert_eq!(saved["cleaned"], false);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let snapshot = runtime.block_on(ipc::snapshot(ipc::DEFAULT_PIPE)).unwrap();
    let fixtures = saved["fixtures"].as_array().unwrap();
    assert_eq!(fixtures.len(), 3);
    let mut scanners = Vec::new();
    for fixture in fixtures {
        let root = PathBuf::from(fixture["root"].as_str().unwrap());
        let fs = fixture["filesystem"].as_str().unwrap();
        let asset = snapshot
            .assets
            .iter()
            .find(|a| a.registration.partition_id == fixture["id"])
            .unwrap();
        assert_eq!(asset.registration.name, format!("M6 {fs}"));
        assert_eq!(asset.connection, "online");
        assert_eq!(asset.mounts, vec![root.to_string_lossy().to_string()]);
        let handle = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES.0)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
            .open(&root)
            .unwrap();
        scanners.push((root.clone(), Scanner::open(&root).unwrap(), handle));
    }
    let base = manifest.parent().unwrap();
    let ready = base.join("watch-ready");
    let remounted = base.join("watch-remounted");
    assert!(
        !ready.exists() && !remounted.exists(),
        "remove stale test markers first"
    );
    std::fs::write(&ready, "ready").unwrap();
    let deadline = Instant::now() + Duration::from_secs(90);
    while !remounted.exists() {
        assert!(
            Instant::now() < deadline,
            "controller did not remount the test VHD"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let mut missing = Vec::new();
    for (root, mut scanner, handle) in scanners {
        println!(
            "{} old mount handle metadata: {:?}",
            root.display(),
            handle.metadata()
        );
        for index in 0..3 {
            std::fs::write(
                root.join("F1")
                    .join(format!("watcher-{suffix}-{index}.txt")),
                "remount sample",
            )
            .unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                match scanner.changed() {
                    Ok(true) => break,
                    Err(error) => {
                        println!("{} retired old watcher: {error:#}", root.display());
                        scanner = Scanner::open(&root).unwrap();
                        break;
                    }
                    Ok(false) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(20))
                    }
                    Ok(false) => {
                        missing.push(format!("{} / change {index}", root.display()));
                        break;
                    }
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "stale monitors silently missed changes: {missing:?}"
    );
}
