use anyhow::{Result, bail, ensure};
use stashfinder_core::{
    Action, Response,
    index::{SearchHit, SearchRequest},
    search::{QueryOptions, SortField},
};
use stashfinder_windows::ipc;
use std::ops::Range;
use tokio::sync::watch;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub generation: u64,
    pub text: String,
    pub sort: SortField,
    pub descending: bool,
    pub range: Range<usize>,
}

pub struct Page {
    pub id: u64,
    pub total: usize,
    pub rows: Vec<SearchHit>,
    pub unavailable: Vec<String>,
}

pub async fn serve(
    pipe: &str,
    mut requests: watch::Receiver<Option<View>>,
    deliver: impl Fn(View, Result<Page>),
) {
    let mut active: Option<(u64, u64)> = None;
    loop {
        let wanted = requests.borrow_and_update().clone();
        if let Some(view) = wanted {
            let result = async {
                let id = match active {
                    Some((generation, id)) if generation == view.generation => id,
                    _ => {
                        let query = SearchRequest {
                            text: view.text.clone(),
                            names_only: false,
                            partitions: Vec::new(),
                            filesystem_id: None,
                            replaces: active.map(|(_, id)| id),
                            options: Box::new(QueryOptions {
                                sort: view.sort,
                                descending: view.descending,
                                ..Default::default()
                            }),
                        };
                        // Finish the start handshake even if the input changes: we need the ID to release it.
                        let Response::SearchStarted(id) =
                            ipc::request(pipe, Action::SearchStart(query)).await?
                        else {
                            bail!("搜索启动响应无效");
                        };
                        active = Some((view.generation, id));
                        id
                    }
                };
                tokio::select! {
                    biased;
                    _ = requests.changed() => Ok(None),
                    page = read_range(pipe, id, view.range.clone()) => page.map(Some),
                }
            }
            .await;
            match result {
                Ok(None) if requests.has_changed().is_err() => break,
                Ok(None) => continue,
                Ok(Some(page)) => deliver(view, Ok(page)),
                Err(error) => deliver(view, Err(error)),
            }
        } else if let Some((_, id)) = active.take() {
            let _ = ipc::request(pipe, Action::SearchCancel { id }).await;
        }
        if requests.changed().await.is_err() {
            break;
        }
    }
    if let Some((_, id)) = active {
        let _ = ipc::request(pipe, Action::SearchCancel { id }).await;
    }
}

async fn read_range(pipe: &str, id: u64, range: Range<usize>) -> Result<Page> {
    let mut rows = Vec::new();
    loop {
        let offset = range.start + rows.len();
        let Response::SearchPage(page) = ipc::request(
            pipe,
            Action::SearchBatch {
                id,
                offset,
                limit: (range.end - offset).min(100),
                wait_ms: Some(1000),
            },
        )
        .await?
        else {
            bail!("搜索分页响应无效");
        };
        if let Some(error) = page.error {
            bail!(error);
        }
        ensure!(page.id == id, "搜索快照已变化");
        if !page.done {
            continue;
        }
        let end = offset + page.rows.len();
        rows.extend(page.rows);
        if end >= range.end.min(page.total) {
            return Ok(Page {
                id,
                total: page.total,
                rows,
                unavailable: page.unavailable,
            });
        }
        ensure!(
            end > offset && page.next == Some(end),
            "搜索结果分页不完整，请重试"
        );
    }
}

#[cfg(test)]
mod tests;
