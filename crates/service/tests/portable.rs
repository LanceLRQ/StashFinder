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

#[test]
fn portable_backend_outlives_its_first_window_and_stops_after_the_last() {
    use stashfinder_windows::session::ManagedSession;
    use std::{fs::File, io::Write};
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("stashfinder-service.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_stashfinder-service"), &executable).unwrap();
    std::fs::write(temp.path().join("portable.json"), br#"{"version":1}"#).unwrap();
    let layout = Layout::discover(&executable).unwrap().unwrap();
    std::fs::create_dir_all(&layout.runtime).unwrap();
    let first = File::create_new(layout.runtime.join("clients.lock")).unwrap();
    first.lock_shared().unwrap();
    let mut service = Command::new(&executable)
        .arg("--portable-shared")
        .arg(std::process::id().to_string())
        .creation_flags(0x08000000)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let status = runtime
        .block_on(ipc::status(&layout.pipe))
        .expect("shared portable backend must start");
    assert_eq!(
        (status.process_id, status.mode.as_str()),
        (service.id(), "portable")
    );
    // An already listening backend must match the one recorded by the launcher.
    let already_running = || panic!("a running backend should not prompt for elevation");
    assert!(
        ManagedSession::start(&executable, Some(&layout), already_running)
            .err()
            .unwrap()
            .to_string()
            .contains("不属于当前会话")
    );
    let mut gate = File::options()
        .write(true)
        .open(layout.runtime.join("startup.lock"))
        .unwrap();
    gate.lock().unwrap();
    gate.write_all(&service.id().to_le_bytes()).unwrap();
    drop(gate);
    let second = ManagedSession::start(&executable, Some(&layout), already_running)
        .unwrap()
        .unwrap();
    drop(first);
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        runtime
            .block_on(ipc::status(&layout.pipe))
            .unwrap()
            .process_id,
        service.id()
    );
    drop(second);
    let deadline = Instant::now() + Duration::from_secs(8);
    while service.try_wait().unwrap().is_none() {
        assert!(
            Instant::now() < deadline,
            "backend outlived its last window"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(service.wait().unwrap().success());
    assert!(layout.index.join("catalog.sqlite").is_file());
}
