use anyhow::{Context, Result, bail, ensure};
use stashfinder_core::{
    Action, Response,
    file_operations::{
        MAX_SELECTED, PreparedTarget, SelectionChunk, SelectionOrigin, record_text, top_level,
    },
};
use stashfinder_windows::{file_operations::verify, ipc, scm, shell};
use std::{collections::HashSet, path::PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FileAction {
    Open,
    CopyPath,
    Drag,
}

pub enum Prepared {
    Open(Box<PreparedTarget>),
    CopyPath(Vec<u16>),
    Drag(Vec<PathBuf>),
}

impl Prepared {
    pub fn perform(self, owner: usize) -> Result<&'static str> {
        match self {
            Self::Open(row) => {
                let path = verify(&row, false)?;
                shell::open_file(&path, shell::OpenAction::Open, owner)?;
                Ok("已交给 Windows 打开")
            }
            Self::CopyPath(text) => {
                shell::copy_text(&text, owner)?;
                Ok("已复制路径")
            }
            Self::Drag(paths) => {
                ensure!(shell::left_button_down(), "已松开鼠标，拖出已取消");
                Ok(if shell::drag_samples(&paths)? {
                    "目标已接受复制"
                } else {
                    "拖出已取消"
                })
            }
        }
    }
}

pub async fn prepare(
    pipe: &str,
    snapshot: u64,
    positions: Vec<usize>,
    action: FileAction,
) -> Result<Prepared> {
    ensure!(
        (1..=MAX_SELECTED).contains(&positions.len()),
        "请选择 1～10,000 项"
    );
    ensure!(
        action != FileAction::Open || positions.len() == 1,
        "打开文件时请选择一项"
    );
    if pipe == scm::INSTALLED_PIPE {
        scm::installed_status().await?;
    }
    let origin = SelectionOrigin::Search {
        id: snapshot,
        range: None,
    };
    let begin = if action == FileAction::CopyPath {
        Action::FileRecordsBegin(origin)
    } else {
        Action::FileSelectionBegin(origin)
    };
    let Response::SelectionStarted(id) = ipc::request(pipe, begin).await? else {
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
        ensure!(total == positions.len(), "文件选择已变化，请重新选择");
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
    let mut rows = selection?;
    if action == FileAction::CopyPath {
        return Ok(Prepared::CopyPath(record_text(&rows, false)));
    }
    if action == FileAction::Open {
        return Ok(Prepared::Open(Box::new(
            rows.pop().context("请选择一项文件")?,
        )));
    }
    tokio::task::spawn_blocking(move || {
        let kept: HashSet<_> = top_level(rows.iter().map(|r| r.file.clone()).collect())
            .into_iter()
            .map(|r| (r.reference.filesystem_id, r.reference.entry))
            .collect();
        let paths = rows
            .iter()
            .filter(|r| {
                kept.contains(&(
                    r.file.reference.filesystem_id.clone(),
                    r.file.reference.entry.clone(),
                ))
            })
            .map(|r| verify(r, false))
            .collect::<Result<_>>()?;
        Ok(Prepared::Drag(paths))
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;
    use stashfinder_core::{
        Action, PROTOCOL_VERSION, Reply, Request, Response,
        file_operations::{SelectionChunk, SelectionOrigin, SelectionPage},
        read_frame, write_frame,
    };
    use stashfinder_windows::ipc::{create_pipe, current_user_sid};

    #[tokio::test]
    async fn cross_page_selection_copies_offline_record_paths_but_rejects_open_and_drag() {
        let pipe = format!(r"\\.\pipe\StashFinder.native-drag-{}", std::process::id());
        let sid = current_user_sid().unwrap();
        let mut server = create_pipe(&pipe, &sid, true).unwrap();
        let endpoint = pipe.clone();
        let backend = tokio::spawn(async move {
            for (action, expected) in [
                (FileAction::Drag, vec![3, 1002]),
                (FileAction::Open, vec![3]),
                (FileAction::CopyPath, vec![3, 1002]),
            ] {
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
                        ) if action != FileAction::CopyPath => Response::SelectionStarted(18),
                        (
                            0,
                            Action::FileRecordsBegin(SelectionOrigin::Search {
                                id: 17,
                                range: None,
                            }),
                        ) if action == FileAction::CopyPath => Response::SelectionStarted(18),
                        (
                            1,
                            Action::FileSelectionAdd {
                                id: 18,
                                chunk: SelectionChunk::Positions(positions),
                            },
                        ) => {
                            assert_eq!(positions, expected);
                            Response::Accepted
                        }
                        (2, Action::FileSelectionSeal { id: 18 }) => {
                            Response::SelectionCount(expected.len())
                        }
                        (3, Action::FileSelectionPage { id: 18, offset: 0 })
                            if action == FileAction::CopyPath =>
                        {
                            let rows = ["一.txt", "子目录\\二.txt"]
                                .into_iter()
                                .enumerate()
                                .map(|(index, name)| {
                                    serde_json::from_value(serde_json::json!({
                                        "partition_name": "离线盘",
                                        "volume": "", "mounts": [],
                                        "identity": { "layout": { "kind": "unknown" } },
                                        "file": {
                                            "reference": {
                                                "partition_id": "p", "filesystem_id": "f",
                                                "generation": "1", "revision": "1", "entry": [index]
                                            },
                                            "relative": name.encode_utf16().collect::<Vec<_>>(),
                                            "object": [index], "folder": false
                                        }
                                    }))
                                    .unwrap()
                                })
                                .collect();
                            Response::SelectionPage(Box::new(SelectionPage {
                                id: 18,
                                total: 2,
                                next: None,
                                rows,
                            }))
                        }
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
            }
        });
        let result = prepare(&pipe, 17, vec![3, 1002], FileAction::Drag).await;
        assert!(
            result.is_err_and(|e| e.to_string().contains("离线")),
            "an offline selection must not yield paths"
        );
        assert!(
            prepare(&pipe, 17, vec![3], FileAction::Open)
                .await
                .is_err_and(|e| e.to_string().contains("离线"))
        );
        let text = prepare(&pipe, 17, vec![3, 1002], FileAction::CopyPath)
            .await
            .unwrap();
        let Prepared::CopyPath(text) = text else {
            panic!("copy path must produce text")
        };
        assert_eq!(
            String::from_utf16(&text).unwrap(),
            "离线盘 / 一.txt\r\n离线盘 / 子目录\\二.txt"
        );
        backend.await.unwrap();
    }
}
