pub mod assets;
use anyhow::{Result, bail};
use stashfinder_core::{
    Action, MAX_FRAME, PROTOCOL_VERSION, Reply, Request, Response, Status, read_frame,
    validate_request, write_frame,
};
use stashfinder_windows::ipc::{IO_TIMEOUT, create_pipe};
use tokio::{net::windows::named_pipe::NamedPipeServer, sync::watch, time::timeout};

pub async fn serve(
    pipe: NamedPipeServer,
    name: &str,
    sid: &str,
    mode: &str,
    stop: watch::Receiver<bool>,
) -> Result<()> {
    serve_assets(pipe, name, sid, mode, stop, None).await
}

pub async fn serve_assets(
    mut pipe: NamedPipeServer,
    name: &str,
    sid: &str,
    mode: &str,
    mut stop: watch::Receiver<bool>,
    assets: Option<&assets::Assets>,
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
            let request_id = request.id;
            let body = (|| -> Result<Response> {
                validate_request(&request)?;
                if let Action::Status = request.action {
                    return Ok(Response::Status(Status {
                        version: PROTOCOL_VERSION,
                        id: request.id,
                        process_id: std::process::id(),
                        mode: mode.to_owned(),
                        registered_partitions: assets.map_or(0, |a| {
                            a.snapshot()
                                .assets
                                .iter()
                                .filter(|r| r.registration.state == "ready")
                                .count() as u32
                        }),
                    }));
                }
                let Some(assets) = assets else {
                    bail!("资产服务未配置")
                };
                match request.action {
                    Action::Page { version, offset } => {
                        Ok(Response::Page(Box::new(assets.page(version, offset)?)))
                    }
                    action => {
                        assets.submit(action)?;
                        Ok(Response::Accepted)
                    }
                }
            })()
            .unwrap_or_else(|e| Response::Error(format!("{e:#}")));
            let mut response = Reply {
                version: PROTOCOL_VERSION,
                id: request_id,
                body,
            };
            let mut bytes = serde_json::to_vec(&response)?;
            if bytes.len() > MAX_FRAME {
                response.body = Response::Error("单项设备信息超过通信帧上限，未截断数据".into());
                bytes = serde_json::to_vec(&response)?;
            }
            write_frame(&mut pipe, &bytes).await
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
