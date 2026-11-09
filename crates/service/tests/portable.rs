use stashfinder_windows::{ipc, portable::Layout};
use std::{
    os::windows::process::CommandExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[tokio::test]
async fn portable_backend_uses_its_own_directory_and_stops_when_its_gui_session_ends() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("stashfinder-service.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_stashfinder-service"), &executable).unwrap();
    std::fs::write(temp.path().join("portable.json"), br#"{"version":1}"#).unwrap();
    let layout = Layout::discover(&executable).unwrap().unwrap();
    let _lock = layout.lock().unwrap();
    let marker = tempfile::NamedTempFile::new_in(&layout.runtime).unwrap();
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Child(
        Command::new(&executable)
            .arg("--portable")
            .arg(ipc::current_user_sid().unwrap())
            .arg(&layout.pipe)
            .arg(&layout.index)
            .arg(std::process::id().to_string())
            .arg(marker.path())
            .creation_flags(0x08000000)
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let status = ipc::status(&layout.pipe).await.unwrap();
    assert_eq!(status.process_id, child.0.id());
    assert_eq!(status.mode, "portable");
    assert_eq!(status.registered_partitions, 0);
    assert!(layout.index.join("catalog.sqlite").is_file());
    drop(marker);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(exit) = child.0.try_wait().unwrap() {
            assert!(exit.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "portable backend outlived its session"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(layout.index.join("catalog.sqlite").is_file());
}
