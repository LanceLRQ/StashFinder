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
