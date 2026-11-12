pub mod assets;
mod file_operations;
pub mod index;
pub mod jobs;
pub mod scan;
pub mod supervisor;
use anyhow::{Context, Result, bail};
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
    pipe: NamedPipeServer,
    name: &str,
    sid: &str,
    mode: &str,
    stop: watch::Receiver<bool>,
    assets: Option<&assets::Assets>,
) -> Result<()> {
    serve_runtime(
        pipe,
        name,
        sid,
        mode,
        stop,
        Services {
            assets,
            indexes: None,
        },
    )
    .await
}

pub struct Services<'a> {
    pub assets: Option<&'a assets::Assets>,
    pub indexes: Option<&'a jobs::Indexes>,
}

pub async fn serve_runtime(
    mut pipe: NamedPipeServer,
    name: &str,
    sid: &str,
    mode: &str,
    mut stop: watch::Receiver<bool>,
    services: Services<'_>,
) -> Result<()> {
    let Services { assets, indexes } = services;
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(2));
    let mut refreshed = std::time::Instant::now();
    loop {
        tokio::select! {
            _ = stop.changed() => return Ok(()),
            _ = heartbeat.tick() => {
                if let Some(assets) = assets {
                    if let Some(indexes) = indexes { indexes.observe(assets.snapshot()); }
                    if refreshed.elapsed() >= std::time::Duration::from_secs(5) {
                        let _ = assets.refresh_in_background();
                        refreshed = std::time::Instant::now();
                    }
                }
                continue;
            },
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
                    Action::IndexStart { partition_id, mode } => {
                        indexes.context("索引服务未配置")?.scan(
                            &partition_id,
                            mode,
                            &assets.snapshot(),
                        )?;
                        Ok(Response::Accepted)
                    }
                    Action::TaskCancel { id } => {
                        indexes.context("索引服务未配置")?.runner.cancel(&id)?;
                        Ok(Response::Accepted)
                    }
                    Action::TaskRetry { id } => {
                        indexes
                            .context("索引服务未配置")?
                            .runner
                            .retry(&id, &assets.snapshot())?;
                        Ok(Response::Accepted)
                    }
                    Action::TaskPage { version, offset } => Ok(Response::Tasks(Box::new(
                        indexes
                            .context("索引服务未配置")?
                            .runner
                            .page(version, offset)?,
                    ))),
                    Action::TaskOverview { version, offset } => {
                        Ok(Response::TaskOverview(Box::new(
                            indexes
                                .context("索引服务未配置")?
                                .runner
                                .overview(version.as_deref(), offset)?,
                        )))
                    }
                    Action::AutoUpdate { enabled } => {
                        indexes
                            .context("索引服务未配置")?
                            .runner
                            .set_enabled(enabled)?;
                        Ok(Response::Accepted)
                    }
                    Action::PartitionPause {
                        partition_id,
                        paused,
                    } => {
                        indexes.context("索引服务未配置")?.runner.pause(
                            &partition_id,
                            paused,
                            &assets.snapshot(),
                        )?;
                        Ok(Response::Accepted)
                    }
                    Action::IndexState { partition_id } => {
                        anyhow::ensure!(
                            assets
                                .snapshot()
                                .assets
                                .iter()
                                .any(|a| a.registration.partition_id == partition_id),
                            "分区未登记"
                        );
                        Ok(Response::Index(
                            indexes.context("索引服务未配置")?.status(&partition_id),
                        ))
                    }
                    Action::SearchStart(request) => Ok(Response::SearchStarted(
                        indexes
                            .context("索引服务未配置")?
                            .search(request, &assets.snapshot())?,
                    )),
                    Action::Directory(request) => Ok(Response::Directory(Box::new(
                        indexes
                            .context("索引服务未配置")?
                            .runner
                            .directory(&request)?,
                    ))),
                    Action::SearchPage { id, offset } => Ok(Response::SearchPage(Box::new(
                        indexes.context("索引服务未配置")?.page(id, offset)?,
                    ))),
                    Action::SearchBatch { id, offset, limit } => {
                        Ok(Response::SearchPage(Box::new(
                            indexes
                                .context("索引服务未配置")?
                                .batch(id, offset, limit)?,
                        )))
                    }
                    Action::SearchCancel { id } => {
                        indexes.context("索引服务未配置")?.cancel_search(id);
                        Ok(Response::Accepted)
                    }
                    Action::FileRecordsBegin(origin) => Ok(Response::SelectionStarted(
                        indexes
                            .context("索引服务未配置")?
                            .selection_begin_records(origin, &assets.snapshot())?,
                    )),
                    Action::FileSelectionBegin(origin) => Ok(Response::SelectionStarted(
                        indexes
                            .context("索引服务未配置")?
                            .selection_begin(origin, &assets.snapshot())?,
                    )),
                    Action::FileSelectionAdd { id, chunk } => {
                        indexes
                            .context("索引服务未配置")?
                            .selection_add(id, chunk)?;
                        Ok(Response::Accepted)
                    }
                    Action::FileSelectionSeal { id } => Ok(Response::SelectionCount(
                        indexes.context("索引服务未配置")?.selection_seal(id)?,
                    )),
                    Action::FileSelectionPage { id, offset } => Ok(Response::SelectionPage(
                        Box::new(indexes.context("索引服务未配置")?.selection_page(
                            id,
                            offset,
                            &assets.snapshot(),
                        )?),
                    )),
                    Action::FileSelectionCancel { id } => {
                        indexes.context("索引服务未配置")?.selection_cancel(id);
                        Ok(Response::Accepted)
                    }
                    Action::FileDestination(reference) => Ok(Response::FileTarget(Box::new(
                        indexes
                            .context("索引服务未配置")?
                            .selection_destination(&reference, &assets.snapshot())?,
                    ))),
                    Action::FileInspect(reference) => Ok(Response::VisualRecord(Box::new(
                        indexes
                            .context("索引服务未配置")?
                            .selection_inspect(&reference, &assets.snapshot())?,
                    ))),
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
                response.body = Response::Error("单项信息超过通信帧上限，未截断数据".into());
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
