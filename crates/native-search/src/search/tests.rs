use super::*;
use stashfinder_core::{
    Action, PROTOCOL_VERSION, Reply, Request, Response,
    index::{SearchPage, SearchRow},
    read_frame, write_frame,
};
use stashfinder_windows::ipc::{create_pipe, current_user_sid};
use std::time::Duration;
use tokio::{sync::mpsc, time::timeout};

#[tokio::test]
async fn cold_start_retries_then_paging_sort_and_close_share_a_snapshot() {
    let pipe = format!(r"\\.\pipe\StashFinder.native-test-{}", std::process::id());
    let owner = current_user_sid().unwrap();
    let mut server = create_pipe(&pipe, &owner, true).unwrap();
    let endpoint = pipe.clone();
    let (trace, mut actions) = mpsc::unbounded_channel();
    let backend = tokio::spawn(async move {
        let mut next_id = 10;
        let mut loading = true;
        loop {
            server.connect().await.unwrap();
            let mut client = server;
            server = create_pipe(&endpoint, &owner, false).unwrap();
            let request: Request =
                serde_json::from_slice(&read_frame(&mut client).await.unwrap()).unwrap();
            let body = match &request.action {
                Action::SearchStart(query) => {
                    assert!(!query.names_only && query.partitions.is_empty());
                    if loading {
                        loading = false;
                        Response::Error("正在读取资产目录，请稍后搜索".into())
                    } else {
                        next_id += 1;
                        Response::SearchStarted(next_id)
                    }
                }
                Action::SearchBatch { id, offset, .. } => {
                    let mut rows = Vec::new();
                    for position in *offset..(*offset + 2).min(130) {
                        rows.push(SearchHit {
                            file: SearchRow {
                                name: format!("row-{position}"),
                                path: String::new(),
                                entry: vec![],
                                parent: vec![],
                                object: vec![],
                                kind: "file".into(),
                                attributes: 0,
                                size: None,
                                created: None,
                                modified: None,
                            },
                            partition_id: "p".into(),
                            filesystem_id: "f".into(),
                            partition_name: "disk".into(),
                            connection: "offline".into(),
                            mounts: vec![],
                            generation: 1,
                            revision: 2,
                            partial: false,
                            indexed_at: None,
                            needs_validation: false,
                            duplicate_group: None,
                            hard_link: false,
                        });
                    }
                    let end = offset + rows.len();
                    Response::SearchPage(Box::new(SearchPage {
                        rows,
                        total: 130,
                        groups: 0,
                        missing: 0,
                        id: *id,
                        done: true,
                        row: None,
                        next: (end < 130).then_some(end),
                        more: end < 130,
                        error: None,
                        unavailable: vec![],
                    }))
                }
                Action::SearchCancel { .. } => Response::Accepted,
                _ => panic!("unexpected request"),
            };
            trace.send(request.action).unwrap();
            let reply = Reply {
                id: request.id,
                version: PROTOCOL_VERSION,
                body,
            };
            write_frame(&mut client, &serde_json::to_vec(&reply).unwrap())
                .await
                .unwrap();
        }
    });
    let mut view = View {
        generation: 1,
        text: "row".into(),
        sort: SortField::Name,
        descending: false,
        range: 0..4,
    };
    let (request, updates) = watch::channel(Some(view.clone()));
    let (out, mut pages) = mpsc::unbounded_channel();
    let worker = tokio::spawn(async move {
        serve(&Endpoint::Direct(pipe), updates, |view, page| {
            out.send((view, page)).unwrap();
        })
        .await
    });
    let first = timeout(Duration::from_secs(3), pages.recv()).await.unwrap();
    assert!(
        first.is_some(),
        "the window needs a completed first viewport"
    );
    let (_, page) = first.unwrap();
    let page = page.unwrap();
    assert_eq!((page.id, page.total), (11, 130));
    assert_eq!(
        page.rows
            .iter()
            .map(|r| r.file.name.as_str())
            .collect::<Vec<_>>(),
        ["row-0", "row-1", "row-2", "row-3"]
    );
    view.range = 126..160;
    request.send(Some(view.clone())).unwrap();
    let (_, tail) = timeout(Duration::from_secs(3), pages.recv())
        .await
        .unwrap()
        .unwrap();
    let tail = tail.unwrap();
    assert_eq!(tail.id, 11);
    assert_eq!(tail.rows.last().unwrap().file.name, "row-129");
    view.generation = 2;
    view.sort = SortField::Size;
    view.descending = true;
    view.range = 0..2;
    request.send(Some(view)).unwrap();
    let (_, sorted) = timeout(Duration::from_secs(3), pages.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sorted.unwrap().id, 12);
    drop(request);
    timeout(Duration::from_secs(3), worker)
        .await
        .unwrap()
        .unwrap();
    let mut starts = Vec::new();
    let mut cancelled = Vec::new();
    while let Ok(action) = actions.try_recv() {
        match action {
            Action::SearchStart(query) => {
                starts.push((query.options.sort, query.options.descending));
                cancelled.extend(query.replaces);
            }
            Action::SearchCancel { id } => cancelled.push(id),
            _ => {}
        }
    }
    assert_eq!(
        starts,
        [
            (SortField::Name, false),
            (SortField::Name, false),
            (SortField::Size, true)
        ]
    );
    assert!(cancelled.contains(&11) && cancelled.contains(&12));
    backend.abort();
}
