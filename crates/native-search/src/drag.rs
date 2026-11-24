use anyhow::{Result, bail, ensure};
use stashfinder_core::{
    Action, Response,
    file_operations::{MAX_SELECTED, SelectionChunk, SelectionOrigin, top_level},
};
use stashfinder_windows::{file_operations::verify, ipc};
use std::{collections::HashSet, path::PathBuf};

pub async fn prepare(pipe: &str, snapshot: u64, positions: Vec<usize>) -> Result<Vec<PathBuf>> {
    ensure!(
        (1..=MAX_SELECTED).contains(&positions.len()),
        "请选择 1～10,000 项"
    );
    let origin = SelectionOrigin::Search {
        id: snapshot,
        range: None,
    };
    let Response::SelectionStarted(id) =
        ipc::request(pipe, Action::FileSelectionBegin(origin)).await?
    else {
        bail!("文件操作准备响应无效");
    };
    let selection = async {
        for chunk in positions.chunks(512) {
            ipc::request(
                pipe,
                Action::FileSelectionAdd {
                    id,
                    chunk: SelectionChunk::Positions(chunk.to_vec()),
                },
            )
            .await?;
        }
        let Response::SelectionCount(total) =
            ipc::request(pipe, Action::FileSelectionSeal { id }).await?
        else {
            bail!("文件选择响应无效");
        };
        ensure!(total == positions.len(), "文件选择已变化，请重新拖出");
        let mut rows = Vec::with_capacity(total);
        while rows.len() < total {
            let offset = rows.len();
            let Response::SelectionPage(page) =
                ipc::request(pipe, Action::FileSelectionPage { id, offset }).await?
            else {
                bail!("文件选择分页响应无效");
            };
            rows.extend(page.rows);
            ensure!(
                page.id == id
                    && page.total == total
                    && rows.len() > offset
                    && rows.len() <= total
                    && (rows.len() == total || page.next == Some(rows.len())),
                "文件选择分页不完整"
            );
        }
        Ok(rows)
    }
    .await;
    let _ = ipc::request(pipe, Action::FileSelectionCancel { id }).await;
    let rows = selection?;
    tokio::task::spawn_blocking(move || {
        let kept: HashSet<_> = top_level(rows.iter().map(|r| r.file.clone()).collect())
            .into_iter()
            .map(|r| (r.reference.filesystem_id, r.reference.entry))
            .collect();
        rows.iter()
            .filter(|r| {
                kept.contains(&(
                    r.file.reference.filesystem_id.clone(),
                    r.file.reference.entry.clone(),
                ))
            })
            .map(|r| verify(r, false))
            .collect()
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;
    use stashfinder_core::{
        Action, PROTOCOL_VERSION, Reply, Request, Response,
        file_operations::{SelectionChunk, SelectionOrigin},
        read_frame, write_frame,
    };
    use stashfinder_windows::ipc::{create_pipe, current_user_sid};

    #[tokio::test]
    async fn cross_page_selection_keeps_exact_positions_and_releases_offline_selection() {
        let pipe = format!(r"\\.\pipe\StashFinder.native-drag-{}", std::process::id());
        let sid = current_user_sid().unwrap();
        let mut server = create_pipe(&pipe, &sid, true).unwrap();
        let endpoint = pipe.clone();
        let backend = tokio::spawn(async move {
            for step in 0..5 {
                server.connect().await.unwrap();
                let mut client = server;
                server = create_pipe(&endpoint, &sid, false).unwrap();
                let request: Request =
                    serde_json::from_slice(&read_frame(&mut client).await.unwrap()).unwrap();
                let body = match (step, request.action) {
                    (
                        0,
                        Action::FileSelectionBegin(SelectionOrigin::Search {
                            id: 17,
                            range: None,
                        }),
                    ) => Response::SelectionStarted(18),
                    (
                        1,
                        Action::FileSelectionAdd {
                            id: 18,
                            chunk: SelectionChunk::Positions(positions),
                        },
                    ) => {
                        assert_eq!(positions, [3, 1002]);
                        Response::Accepted
                    }
                    (2, Action::FileSelectionSeal { id: 18 }) => Response::SelectionCount(2),
                    (3, Action::FileSelectionPage { id: 18, offset: 0 }) => {
                        Response::Error("所选分区离线".into())
                    }
                    (4, Action::FileSelectionCancel { id: 18 }) => Response::Accepted,
                    other => panic!("unexpected selection step: {other:?}"),
                };
                let reply = Reply {
                    version: PROTOCOL_VERSION,
                    id: request.id,
                    body,
                };
                write_frame(&mut client, &serde_json::to_vec(&reply).unwrap())
                    .await
                    .unwrap();
            }
        });
        let result = prepare(&pipe, 17, vec![3, 1002]).await;
        assert!(
            result.is_err_and(|e| e.to_string().contains("离线")),
            "an offline selection must not yield paths"
        );
        backend.await.unwrap();
    }
}
