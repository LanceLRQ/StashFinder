#[test]
fn ordinary_user_directory_is_not_accepted_as_service_storage() {
    let root = tempfile::tempdir().unwrap();
    assert!(stashfinder_windows::storage::validate_service_root(root.path()).is_err());
}
