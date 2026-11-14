pub mod assets;
mod file_operations;
pub mod index;
pub mod jobs;
pub mod scan;
pub mod supervisor;
use anyhow::{Context, Result, bail};
use futures_util::{StreamExt, stream::FuturesUnordered};
use stashfinder_core::{
    Action, MAX_FRAME, PROTOCOL_VERSION, Reply, Request, Response, Status, read_frame,
    validate_request, write_frame,
};
use stashfinder_windows::ipc::{IO_TIMEOUT, create_pipe};
use std::time::Duration;
use tokio::{
    net::windows::named_pipe::NamedPipeServer,
    sync::watch,
    time::{Instant, timeout_at},
};

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
    let mut searches = FuturesUnordered::new();
    loop {
        let result = tokio::select! {
            _ = stop.changed() => return Ok(()),
            Some(result) = searches.next(), if !searches.is_empty() => {
                if !matches!(result, Ok(Ok(()))) {
                    eprintln!("search response rejected or timed out: {result:?}");
                }
                continue;
            },
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
            connected = pipe.connect() => connected,
        };
        result?;
        // Keep a listener alive so another first-instance server cannot claim the name.
        let next = create_pipe(name, sid, false)?;
        let mut client = std::mem::replace(&mut pipe, next);
        let deadline = Instant::now() + IO_TIMEOUT;
        let read = async {
            serde_json::from_slice::<Request>(&read_frame(&mut client).await?)
                .context("invalid request")
        };
        let request = tokio::select! {
            _ = stop.changed() => return Ok(()),
            result = timeout_at(deadline, read) => result,
        };
        let request = match request {
            Ok(Ok(request)) => request,
            error => {
                eprintln!("request rejected or timed out: {error:?}");
                continue;
            }
        };
        let request_id = request.id;
        if let Action::SearchBatch {
            id,
            offset,
            limit,
            wait_ms: Some(wait),
        } = request.action
            && wait > 0
            && validate_request(&request).is_ok()
            && let Some(indexes) = indexes
            && assets.is_some()
        {
            const MAX_SEARCH_WAITERS: usize = 64;
            if searches.len() < MAX_SEARCH_WAITERS {
                searches.push(timeout_at(deadline, async move {
                    let wait = Duration::from_millis(wait.min(1000));
                    let body = indexes
                        .wait_batch(id, offset, limit, wait)
                        .await
                        .map(|page| Response::SearchPage(Box::new(page)))
                        .unwrap_or_else(|error| Response::Error(format!("{error:#}")));
                    send_reply(&mut client, request_id, body).await
                }));
                continue;
            }
            let body = Response::Error("等待搜索结果的请求过多，请稍后重试".into());
            let _ = timeout_at(deadline, send_reply(&mut client, request_id, body)).await;
            continue;
        }
        let exchange = async {
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
                    Action::SearchReselect {
                        id,
                        previous,
                        offset,
                    } => Ok(Response::SearchPosition(
                        indexes
                            .context("索引服务未配置")?
                            .reselect(id, previous, offset)?,
                    )),
                    Action::SearchBatch {
                        id, offset, limit, ..
                    } => Ok(Response::SearchPage(Box::new(
                        indexes
                            .context("索引服务未配置")?
                            .batch(id, offset, limit)?,
                    ))),
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
            send_reply(&mut client, request_id, body).await
        };
        tokio::select! {
            _ = stop.changed() => return Ok(()),
            result = timeout_at(deadline, exchange) => {
                if !matches!(result, Ok(Ok(()))) { eprintln!("request rejected or timed out: {result:?}"); }
            }
        }
    }
}

async fn send_reply(pipe: &mut NamedPipeServer, id: u64, body: Response) -> Result<()> {
    let mut response = Reply {
        version: PROTOCOL_VERSION,
        id,
        body,
    };
    let mut bytes = serde_json::to_vec(&response)?;
    if bytes.len() > MAX_FRAME {
        response.body = Response::Error("单项信息超过通信帧上限，未截断数据".into());
        bytes = serde_json::to_vec(&response)?;
    }
    write_frame(pipe, &bytes).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use stashfinder_windows::ipc::{current_user_sid, request, status};
    use tokio::net::windows::named_pipe::ClientOptions;
    use tokio::time::timeout;

    #[tokio::test]
    async fn search_wait_does_not_block_status_or_cancellation() {
        let name = format!(r"\\.\pipe\StashFinder.search-wait-{}", std::process::id());
        let sid = current_user_sid().unwrap();
        let pipe = create_pipe(&name, &sid, true).unwrap();
        let dir = tempfile::tempdir().unwrap();
        stashfinder_store::catalog::Catalog::open(dir.path()).unwrap();
        let assets = assets::Assets::start(dir.path().into(), || Ok(vec![]));
        let indexes = jobs::Indexes::start(dir.path().into());
        indexes.insert_pending_for_test(7);
        let (stop, receiver) = watch::channel(false);
        let server = serve_runtime(
            pipe,
            &name,
            &sid,
            "test",
            receiver,
            Services {
                assets: Some(&assets),
                indexes: Some(&indexes),
            },
        );
        let client = async {
            let mut waiting = ClientOptions::new().open(&name).unwrap();
            let wait = Request {
                version: PROTOCOL_VERSION,
                id: 1,
                action: Action::SearchBatch {
                    id: 7,
                    offset: 0,
                    limit: 32,
                    wait_ms: Some(1000),
                },
            };
            write_frame(&mut waiting, &serde_json::to_vec(&wait).unwrap())
                .await
                .unwrap();
            timeout(Duration::from_millis(500), async {
                status(&name).await.unwrap();
                request(&name, Action::SearchCancel { id: 7 })
                    .await
                    .unwrap();
                let reply: Reply =
                    serde_json::from_slice(&read_frame(&mut waiting).await.unwrap()).unwrap();
                assert!(matches!(reply.body, Response::Error(_)));
            })
            .await
            .unwrap();
            stop.send(true).unwrap();
        };
        let (result, ()) = tokio::join!(server, client);
        result.unwrap();
    }
}
