use anyhow::Result;
use stashfinder_core::{
    PROTOCOL_VERSION, Request, Status, read_frame, validate_request, write_frame,
};
use stashfinder_windows::ipc::{IO_TIMEOUT, create_pipe};
use tokio::{net::windows::named_pipe::NamedPipeServer, sync::watch, time::timeout};

pub async fn serve(
    mut pipe: NamedPipeServer,
    name: &str,
    sid: &str,
    mode: &str,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    loop {
        tokio::select! {
            _ = stop.changed() => return Ok(()),
            connected = pipe.connect() => connected?,
        }
        // Keep a listener alive so another first-instance server cannot claim the name.
        let next = create_pipe(name, sid, false)?;
        let exchange = async {
            let request: Request = serde_json::from_slice(&read_frame(&mut pipe).await?)?;
            validate_request(&request)?;
            let response = Status {
                version: PROTOCOL_VERSION,
                id: request.id,
                process_id: std::process::id(),
                mode: mode.to_owned(),
                registered_partitions: 0,
            };
            write_frame(&mut pipe, &serde_json::to_vec(&response)?).await
        };
        tokio::select! {
            _ = stop.changed() => return Ok(()),
            result = timeout(IO_TIMEOUT, exchange) => {
                if !matches!(result, Ok(Ok(()))) { eprintln!("request rejected or timed out: {result:?}"); }
            }
        }
        pipe = next;
    }
}
