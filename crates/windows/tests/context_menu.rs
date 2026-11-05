use stashfinder_windows::context_menu::show;
#[test]
fn rejects_cross_directory_selection_before_showing_a_native_menu() {
    let paths = [
        std::path::PathBuf::from(r"X:\one\first.txt"),
        std::path::PathBuf::from(r"X:\two\second.txt"),
    ];
    assert!(
        show(&paths, 0, 0.0, 0.0, 1.0)
            .unwrap_err()
            .to_string()
            .contains("同一目录")
    );
    assert!(show(&[], 0, 0.0, 0.0, 1.0).is_err());
}
