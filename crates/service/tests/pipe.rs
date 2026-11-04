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
    let server = Server(
        Command::new(env!("CARGO_BIN_EXE_stashfinder-service"))
            .args(["--foreground", &name])
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let response = status(&name).await.unwrap();
    assert_eq!(response.process_id, server.0.id());
    assert_eq!(response.mode, "foreground");
    assert_eq!(response.registered_partitions, 0);
    let mut client = ClientOptions::new().open(&name).unwrap();
    write_frame(&mut client, br#"{"version":99,"id":2,"action":"status"}"#)
        .await
        .unwrap();
    assert!(read_frame(&mut client).await.is_err());
    drop(client);
    assert_eq!(status(&name).await.unwrap().process_id, server.0.id());
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
