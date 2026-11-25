use stashfinder_windows::{ipc, portable::Layout};
use std::{
    fs::File,
    os::windows::process::CommandExt,
    process::{Child, Command},
    time::{Duration, Instant},
};

struct Backend(Child);

impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn desktop_executable_hosts_the_portable_index_by_itself() {
    exercise_portable_backend(true).await;
}

#[tokio::test]
async fn copying_only_the_executable_starts_a_portable_index() {
    exercise_portable_backend(false).await;
}

async fn exercise_portable_backend(keep_marker: bool) {
    let folder = tempfile::tempdir().unwrap();
    let app = folder.path().join("StashFinder.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_stashfinder-desktop"), &app).unwrap();
    std::fs::write(folder.path().join("portable.json"), br#"{"version":1}"#).unwrap();
    let layout = Layout::discover(&app).unwrap().unwrap();
    if !keep_marker {
        std::fs::remove_file(folder.path().join("portable.json")).unwrap();
    }
    std::fs::create_dir_all(&layout.runtime).unwrap();
    let client = File::create_new(layout.runtime.join("clients.lock")).unwrap();
    client.lock_shared().unwrap();
    let mut backend = Backend(
        Command::new(&app)
            .args(["--portable-shared", &std::process::id().to_string()])
            .creation_flags(0x08000000)
            .spawn()
            .unwrap(),
    );
    let status = ipc::status(&layout.pipe)
        .await
        .expect("the only executable must host the index");
    assert_eq!(
        (status.mode.as_str(), status.process_id),
        ("portable", backend.0.id())
    );
    assert!(layout.index.join("catalog.sqlite").is_file());
    drop(client);
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(exit) = backend.0.try_wait().unwrap() {
            assert!(exit.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the backend outlived its last client"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
