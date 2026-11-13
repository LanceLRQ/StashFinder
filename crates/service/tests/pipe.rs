use stashfinder_windows::ipc::status;
use std::process::{Child, Command, Stdio};

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn developer_backend_exits_when_its_launcher_closes_stdin() {
    use std::io::Write;
    let name = format!(r"\\.\pipe\StashFinder.dev-lifetime-{}", std::process::id());
    let data = tempfile::tempdir().unwrap();
    let mut server = Server(
        Command::new(env!("CARGO_BIN_EXE_stashfinder-service"))
            .args(["--foreground", &name, "--watch-stdin", "--data-dir"])
            .arg(data.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    assert_eq!(status(&name).await.unwrap().process_id, server.0.id());
    // Windows PowerShell writes an encoding preamble when it opens the pipe.
    server
        .0
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"\xEF\xBB\xBF")
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert!(
        server.0.try_wait().unwrap().is_none(),
        "backend exited while the launcher was still connected"
    );
    drop(server.0.stdin.take());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(exit) = server.0.try_wait().unwrap() {
            assert!(exit.success());
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "backend outlived its launcher"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn foreground_backends_cannot_share_a_catalog_even_on_different_pipes() {
    let first = format!(r"\\.\pipe\StashFinder.lock-first-{}", std::process::id());
    let second = format!(r"\\.\pipe\StashFinder.lock-second-{}", std::process::id());
    let data = tempfile::tempdir().unwrap();
    let start = |pipe: &str| {
        Server(
            Command::new(env!("CARGO_BIN_EXE_stashfinder-service"))
                .args(["--foreground", pipe, "--watch-stdin", "--data-dir"])
                .arg(data.path())
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    };
    let mut original = start(&first);
    assert_eq!(status(&first).await.unwrap().process_id, original.0.id());
    let mut duplicate = start(&second);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if let Some(exit) = duplicate.0.try_wait().unwrap() {
            assert!(!exit.success());
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "second backend kept running"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(status(&first).await.unwrap().process_id, original.0.id());
    drop(original.0.stdin.take());
    assert!(original.0.wait().unwrap().success());
    let replacement = start(&second);
    assert_eq!(
        status(&second).await.unwrap().process_id,
        replacement.0.id()
    );
}

#[tokio::test]
async fn connects_to_real_process_rejects_bad_request_and_reconnects() {
    use stashfinder_core::{read_frame, write_frame};
    use tokio::net::windows::named_pipe::ClientOptions;
    let name = format!(r"\\.\pipe\StashFinder.integration-{}", std::process::id());
    let data = tempfile::tempdir().unwrap();
    let server = Server(
        Command::new(env!("CARGO_BIN_EXE_stashfinder-service"))
            .args(["--foreground", &name])
            .arg("--data-dir")
            .arg(data.path())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let response = status(&name).await.unwrap();
    assert_eq!(response.process_id, server.0.id());
    assert_eq!(response.mode, "foreground");
    assert_eq!(response.registered_partitions, 0);
    for _ in 0..100 {
        let mut client = ClientOptions::new().open(&name).unwrap();
        write_frame(&mut client, br#"{"version":99,"id":2,"action":"status"}"#)
            .await
            .unwrap();
        let reply: stashfinder_core::Reply =
            serde_json::from_slice(&read_frame(&mut client).await.unwrap()).unwrap();
        assert!(matches!(reply.body, stashfinder_core::Response::Error(_)));
        drop(client);
        assert_eq!(status(&name).await.unwrap().process_id, server.0.id());
    }
}

#[tokio::test]
async fn unavailable_service_has_a_bounded_wait() {
    let start = std::time::Instant::now();
    assert!(
        status(&format!(
            r"\\.\pipe\StashFinder.absent-{}",
            std::process::id()
        ))
        .await
        .is_err()
    );
    assert!(start.elapsed().as_secs() < 5);
}

#[tokio::test]
async fn rename_over_real_pipe_survives_process_termination_and_restart() {
    use stashfinder_core::{Action, assets::*};
    use stashfinder_store::catalog::Catalog;
    use stashfinder_windows::ipc::{request, snapshot};
    let data = tempfile::tempdir().unwrap();
    let mut catalog = Catalog::open(data.path()).unwrap();
    let observation = Observation {
        candidate_id: "synthetic-restart-fixture".into(),
        disk_number: None,
        model: None,
        disk_bytes: None,
        partition_bytes: None,
        volume_label: None,
        mounts: vec![],
        capacity: None,
        error: None,
        identity: Identity {
            layout: Layout::Gpt {
                disk: "fixture-disk".into(),
                partition: "fixture-partition".into(),
            },
            filesystem: Some("NTFS".into()),
            serial: Some(Serial {
                source: "ntfs64".into(),
                value: "1".into(),
            }),
        },
    };
    let row = catalog.prepare(&observation, "Before").unwrap();
    catalog.finish(&row.partition_id).unwrap();
    {
        use stashfinder_store::volume::*;
        let mut db = Volume::open(&catalog.volume_path(&row.filesystem_id).unwrap()).unwrap();
        let generation = db.begin_generation().unwrap();
        let objects = [
            Object {
                identity: None,
                id: vec![1],
                source: "ntfs64".into(),
                kind: "folder".into(),
                size: None,
                created: None,
                modified: None,
                attributes: 16,
            },
            Object {
                identity: None,
                id: vec![2],
                source: "ntfs64".into(),
                kind: "file".into(),
                size: Some(0),
                created: None,
                modified: None,
                attributes: 0,
            },
        ];
        let entries = [
            Entry {
                id: vec![1],
                object: vec![1],
                parent: None,
                name: vec![92],
            },
            Entry {
                id: vec![2],
                object: vec![2],
                parent: Some(vec![1]),
                name: "offline-result.txt".encode_utf16().collect(),
            },
        ];
        db.write_batch(generation, &objects, &entries, &[], None)
            .unwrap();
        db.complete(generation).unwrap();
        db.publish(generation).unwrap();
    }
    drop(catalog);
    let name = format!(r"\\.\pipe\StashFinder.restart-{}", std::process::id());
    for attempt in 0..2 {
        let server = Server(
            Command::new(env!("CARGO_BIN_EXE_stashfinder-service"))
                .args(["--foreground", &name, "--data-dir"])
                .arg(data.path())
                .stdout(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let mut connected = false;
        loop {
            let s = match snapshot(&name).await {
                Ok(snapshot) => snapshot,
                Err(error)
                    if !connected
                        && error.is::<tokio::time::error::Elapsed>()
                        && std::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                    continue;
                }
                Err(error) => panic!("{error:#}"),
            };
            connected = true;
            if !s.busy {
                assert!(s.error.is_none(), "{:?}", s.error);
                assert_eq!(s.assets[0].registration.filesystem_id, row.filesystem_id);
                if attempt == 1 {
                    assert_eq!(s.assets[0].registration.name, "After");
                    break;
                }
                if s.assets[0].registration.name == "After" {
                    break;
                }
                request(
                    &name,
                    Action::Rename {
                        partition_id: row.partition_id.clone(),
                        name: "After".into(),
                    },
                )
                .await
                .unwrap();
            }
            assert!(std::time::Instant::now() < deadline);
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
        let response = request(
            &name,
            Action::SearchStart(stashfinder_core::index::SearchRequest {
                replaces: None,
                text: "offline-result".into(),
                names_only: false,
                partitions: vec![],
                options: Default::default(),
            }),
        )
        .await
        .unwrap();
        let stashfinder_core::Response::SearchStarted(id) = response else {
            panic!("search did not start")
        };
        loop {
            let response = request(&name, Action::SearchPage { id, offset: 0 })
                .await
                .unwrap();
            let stashfinder_core::Response::SearchPage(page) = response else {
                panic!("search page missing")
            };
            if page.done {
                assert!(page.error.is_none());
                assert_eq!(page.row.unwrap().file.path, "offline-result.txt");
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        drop(server);
    }
}
