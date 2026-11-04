use stashfinder_windows::ipc::{create_pipe, current_user_sid, status};
use tokio::{
    io::AsyncWriteExt,
    net::windows::named_pipe::ClientOptions,
    sync::watch,
    time::{Duration, timeout},
};

#[tokio::test]
async fn stop_interrupts_a_client_that_never_finishes_its_frame() {
    let name = format!(r"\\.\pipe\StashFinder.stop-{}", std::process::id());
    let sid = current_user_sid().unwrap();
    let pipe = create_pipe(&name, &sid, true).unwrap();
    let (stop, receiver) = watch::channel(false);
    let server_name = name.clone();
    let worker = tokio::spawn(async move {
        stashfinder_service::serve(pipe, &server_name, &sid, "test", receiver).await
    });
    status(&name).await.unwrap();
    let mut client = ClientOptions::new().open(&name).unwrap();
    client.write_all(&[100, 0, 0, 0, 1]).await.unwrap();
    stop.send(true).unwrap();
    timeout(Duration::from_secs(1), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn delayed_reader_receives_entire_reply_before_server_closes() {
    use stashfinder_core::{Reply, Response, read_frame, write_frame};
    let name = format!(r"\\.\pipe\StashFinder.slow-reader-{}", std::process::id());
    let sid = current_user_sid().unwrap();
    let pipe = create_pipe(&name, &sid, true).unwrap();
    let (stop, receiver) = watch::channel(false);
    let server_name = name.clone();
    let worker = tokio::spawn(async move {
        stashfinder_service::serve(pipe, &server_name, &sid, "test", receiver).await
    });
    for _ in 0..100 {
        assert_eq!(status(&name).await.unwrap().mode, "test");
        let mut client = ClientOptions::new().open(&name).unwrap();
        write_frame(&mut client, br#"{"version":99,"id":2,"action":"status"}"#)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        let response: Reply =
            serde_json::from_slice(&read_frame(&mut client).await.unwrap()).unwrap();
        assert!(matches!(response.body, Response::Error(_)));
        drop(client);
    }
    stop.send(true).unwrap();
    worker.await.unwrap().unwrap();
}
