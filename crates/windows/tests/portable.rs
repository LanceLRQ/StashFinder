use stashfinder_windows::portable::{Backend, Layout, Process, quote_argument};

#[test]
fn backup_read_privilege_requires_an_existing_elevated_token() {
    assert_eq!(
        stashfinder_windows::portable::enable_backup_read().is_ok(),
        stashfinder_windows::portable::is_elevated()
    );
}

#[test]
#[ignore = "Requires elevation and an explicitly selected isolated test directory"]
fn backup_read_enumerates_a_protected_test_directory() {
    let root = std::env::var_os("STASHFINDER_BACKUP_TEST_ROOT").unwrap();
    stashfinder_windows::portable::enable_backup_read().unwrap();
    stashfinder_windows::ntfs::read_directory(
        std::path::Path::new(&root),
        &std::sync::atomic::AtomicBool::new(false),
        |_| Ok(()),
    )
    .unwrap();
}

#[test]
fn portable_data_is_explicit_versioned_and_moves_with_the_executable() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("便携 目录");
    std::fs::create_dir(&first).unwrap();
    let executable = first.join("StashFinder.exe");
    std::fs::write(&executable, b"test").unwrap();
    assert!(Layout::discover(&executable).unwrap().is_none());
    std::fs::write(first.join("portable.json"), b"not json").unwrap();
    assert!(Layout::discover(&executable).is_err());
    std::fs::write(first.join("portable.json"), br#"{"version":2}"#).unwrap();
    assert!(Layout::discover(&executable).is_err());
    std::fs::write(first.join("portable.json"), br#"{"version":1}"#).unwrap();
    let original = Layout::discover(&executable).unwrap().unwrap();
    assert_eq!(
        original.index,
        std::fs::canonicalize(&first).unwrap().join("data/index")
    );
    std::fs::create_dir_all(&original.config).unwrap();
    std::fs::write(original.config.join("preferences.json"), b"saved").unwrap();
    let second = temp.path().join("搬迁以后");
    std::fs::rename(&first, &second).unwrap();
    let moved = Layout::discover(&second.join("StashFinder.exe"))
        .unwrap()
        .unwrap();
    assert_ne!(original.pipe, moved.pipe);
    assert!(
        moved
            .index
            .starts_with(std::fs::canonicalize(&second).unwrap())
    );
    assert_eq!(
        std::fs::read(moved.config.join("preferences.json")).unwrap(),
        b"saved"
    );
    assert_ne!(moved.pipe, stashfinder_windows::ipc::DEFAULT_PIPE);
}

#[test]
fn one_portable_directory_has_one_session_and_failed_start_does_not_poison_it() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("StashFinder.exe");
    std::fs::write(&executable, b"test").unwrap();
    std::fs::write(temp.path().join("portable.json"), br#"{"version":1}"#).unwrap();
    let layout = Layout::discover(&executable).unwrap().unwrap();
    let first = layout.lock().unwrap();
    assert!(layout.lock().is_err());
    drop(first);
    assert!(layout.lock().is_ok());
}

#[test]
fn windows_arguments_preserve_spaces_quotes_and_trailing_backslashes() {
    assert_eq!(
        quote_argument(r"D:\便携 目录\data"),
        "\"D:\\便携 目录\\data\""
    );
    assert_eq!(quote_argument("a\"b"), "\"a\\\"b\"");
    assert_eq!(quote_argument("trailing\\"), "\"trailing\\\\\"");
    assert_eq!(quote_argument(""), "\"\"");
}

#[test]
fn portable_backend_rejects_a_different_data_directory_pipe_and_session() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("stashfinder-service.exe");
    std::fs::write(&executable, b"test").unwrap();
    std::fs::write(temp.path().join("portable.json"), br#"{"version":1}"#).unwrap();
    let layout = Layout::discover(&executable).unwrap().unwrap();
    let _lock = layout.lock().unwrap();
    let marker = tempfile::NamedTempFile::new_in(&layout.runtime).unwrap();
    let args = vec![
        "S-1-5-18".into(),
        layout.pipe.clone(),
        layout.index.to_string_lossy().into_owned(),
        std::process::id().to_string(),
        marker.path().to_string_lossy().into_owned(),
    ];
    assert!(Backend::parse(&executable, &args).is_ok());
    for (position, value) in [
        (1, "\\\\.\\pipe\\StashFinder.M0"),
        (2, "D:\\some-other-data"),
        (3, "0"),
        (4, "D:\\some-other-session"),
    ] {
        let mut invalid = args.clone();
        invalid[position] = value.into();
        assert!(Backend::parse(&executable, &invalid).is_err());
    }
}

#[test]
fn removing_the_gui_session_releases_a_waiting_background_process() {
    let session = tempfile::NamedTempFile::new().unwrap();
    let path = session.path().to_owned();
    let parent = Process::open(std::process::id()).unwrap();
    let worker = std::thread::spawn(move || parent.wait_session(&path));
    drop(session);
    assert!(worker.join().unwrap().is_ok());
}
