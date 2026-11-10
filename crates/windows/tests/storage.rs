#[test]
fn ordinary_user_directory_is_not_accepted_as_service_storage() {
    let root = tempfile::tempdir().unwrap();
    assert!(stashfinder_windows::storage::validate_service_root(root.path()).is_err());
}

#[test]
fn desktop_data_paths_are_resolved_for_the_explicit_user() {
    let sid = stashfinder_windows::ipc::current_user_sid().unwrap();
    let paths = stashfinder_windows::storage::desktop_data_directories(&sid).unwrap();
    assert!(paths.iter().any(|p| {
        *p == std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap())
            .join("io.github.lancelrq.stashfinder")
    }));
    assert!(stashfinder_windows::storage::desktop_data_directories("S-1-5\\other").is_err());
}
