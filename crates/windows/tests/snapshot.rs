use stashfinder_core::{PROTOCOL_VERSION, Page, Reply, Response, read_frame, write_frame};
use stashfinder_windows::ipc::{create_pipe, current_user_sid, snapshot};

#[tokio::test]
async fn expired_snapshot_restarts_pagination_from_the_first_page() {
    let name = format!(
        r"\\.\pipe\StashFinder.snapshot-retry-{}",
        std::process::id()
    );
    let sid = current_user_sid().unwrap();
    let mut server = create_pipe(&name, &sid, true).unwrap();
    let worker_name = name.clone();
    let worker = tokio::spawn(async move {
        let mut offsets = Vec::new();
        for body in [
            Response::Page(Box::new(Page {
                version: 1,
                busy: true,
                error: None,
                asset: None,
                candidate: None,
                next: Some(1),
            })),
            Response::Error("设备快照已更新，请刷新".into()),
            Response::Page(Box::new(Page {
                version: 2,
                busy: false,
                error: None,
                asset: None,
                candidate: None,
                next: None,
            })),
        ] {
            server.connect().await.unwrap();
            let next = create_pipe(&worker_name, &sid, false).unwrap();
            let request: stashfinder_core::Request =
                serde_json::from_slice(&read_frame(&mut server).await.unwrap()).unwrap();
            let stashfinder_core::Action::Page { offset, .. } = request.action else {
                panic!("expected page");
            };
            offsets.push(offset);
            write_frame(
                &mut server,
                &serde_json::to_vec(&Reply {
                    version: PROTOCOL_VERSION,
                    id: request.id,
                    body,
                })
                .unwrap(),
            )
            .await
            .unwrap();
            server = next;
        }
        offsets
    });
    let result = snapshot(&name).await.unwrap();
    assert_eq!(result.version, 2);
    assert!(!result.busy);
    assert_eq!(worker.await.unwrap(), [0, 1, 0]);
}
