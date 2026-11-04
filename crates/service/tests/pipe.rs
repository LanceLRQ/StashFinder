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
                id: vec![1],
                source: "ntfs64".into(),
                kind: "folder".into(),
                size: None,
                created: None,
                modified: None,
                attributes: 16,
            },
            Object {
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
        loop {
            let s = snapshot(&name).await.unwrap();
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
                text: "offline-result".into(),
                names_only: false,
                partitions: vec![],
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
