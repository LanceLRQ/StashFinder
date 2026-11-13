use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, SecondsFormat};
use stashfinder_core::{Action, Response, index::SearchHit};
use std::{
    future::Future,
    io::{BufWriter, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tauri::{Emitter, State, WebviewWindow};

#[derive(Default)]
pub struct SearchExport(Mutex<Option<Arc<AtomicBool>>>);

impl SearchExport {
    pub fn cancel(&self) -> bool {
        if let Some(active) = &*self.0.lock().unwrap_or_else(|e| e.into_inner()) {
            active.store(true, Ordering::Release);
            true
        } else {
            false
        }
    }
}

#[tauri::command]
pub fn cancel_search_export(state: State<'_, SearchExport>) {
    state.cancel();
}

#[tauri::command]
pub async fn export_search_csv(
    window: WebviewWindow,
    state: State<'_, SearchExport>,
    id: u64,
    total: usize,
    spans: Option<Vec<[usize; 2]>>,
) -> Result<Option<usize>, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut active = state.0.lock().unwrap_or_else(|e| e.into_inner());
        if active.is_some() {
            return Err("已有搜索结果正在导出".into());
        }
        *active = Some(cancel.clone());
    }
    let result = export(&window, id, total, spans, &cancel).await;
    *state.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    if cancel.load(Ordering::Acquire) && result.is_err() {
        return Ok(None);
    }
    result.map_err(|e| format!("{e:#}"))
}

async fn export(
    window: &WebviewWindow,
    id: u64,
    total: usize,
    spans: Option<Vec<[usize; 2]>>,
    cancel: &AtomicBool,
) -> Result<Option<usize>> {
    let spans = spans.unwrap_or_else(|| vec![[0, total]]);
    count_rows(total, &spans)?;
    let owner = window.hwnd()?.0 as usize;
    let (send, receive) = tokio::sync::oneshot::channel();
    window.run_on_main_thread(move || {
        let _ = send.send(stashfinder_windows::shell::choose_csv_destination(owner));
    })?;
    let Some(path) = receive.await?? else {
        return Ok(None);
    };
    let written = write_csv(
        &path,
        id,
        total,
        spans,
        cancel,
        crate::runtime::request,
        |written, count| {
            let _ = window.emit("search-export-progress", (written, count));
        },
    )
    .await?;
    Ok(Some(written))
}

async fn write_csv<F: Future<Output = Result<Response>>>(
    path: &Path,
    id: u64,
    total: usize,
    spans: Vec<[usize; 2]>,
    cancel: &AtomicBool,
    request: impl Fn(Action) -> F,
    progress: impl Fn(usize, usize),
) -> Result<usize> {
    check_cancel(cancel)?;
    let count = count_rows(total, &spans)?;
    let mut file = begin_csv(path)?;
    let mut written = 0usize;
    let mut reported = Instant::now();
    progress(0, count);
    for [mut offset, end] in spans {
        while offset < end {
            check_cancel(cancel)?;
            let reply = request(Action::SearchBatch {
                id,
                offset,
                limit: (end - offset).min(100),
                wait_ms: None,
            })
            .await?;
            let Response::SearchPage(page) = reply else {
                bail!("搜索结果分页响应无效");
            };
            check_cancel(cancel)?;
            if let Some(error) = page.error {
                bail!("{error}");
            }
            ensure!(
                page.id == id
                    && page.done
                    && page.total == total
                    && !page.rows.is_empty()
                    && page.rows.len() <= end - offset,
                "搜索结果已过期或分页不完整"
            );
            for hit in &page.rows {
                write_row(&mut file, hit).context("写入 CSV 失败")?;
            }
            offset += page.rows.len();
            written += page.rows.len();
            if reported.elapsed() >= Duration::from_millis(250) {
                progress(written, count);
                reported = Instant::now();
            }
        }
    }
    finish_csv(file, path, cancel)?;
    Ok(written)
}

fn count_rows(total: usize, spans: &[[usize; 2]]) -> Result<usize> {
    let mut previous = 0;
    let mut count = 0;
    for &[start, end] in spans {
        ensure!(
            previous <= start && start < end && end <= total,
            "导出范围无效"
        );
        previous = end;
        count += end - start;
    }
    ensure!(count > 0, "没有可导出的结果");
    Ok(count)
}

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Acquire), "导出已取消");
    Ok(())
}

fn begin_csv(path: &Path) -> Result<BufWriter<tempfile::NamedTempFile>> {
    let parent = path.parent().context("导出目标缺少目录")?;
    let mut file = BufWriter::with_capacity(
        64 * 1024,
        tempfile::NamedTempFile::new_in(parent).context("无法在目标目录创建导出文件")?,
    );
    file.write_all(
        "\u{feff}名称,卷内路径,分区,状态,类型,大小（字节）,创建时间（UTC）,修改时间（UTC）\r\n"
            .as_bytes(),
    )?;
    Ok(file)
}

fn finish_csv(
    mut file: BufWriter<tempfile::NamedTempFile>,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<()> {
    check_cancel(cancel)?;
    file.flush().context("写入 CSV 失败")?;
    file.get_ref()
        .as_file()
        .sync_all()
        .context("同步 CSV 失败")?;
    check_cancel(cancel)?;
    file.into_inner()?
        .persist(path)
        .map_err(|e| e.error)
        .context("无法保存 CSV，原有文件保持不变")?;
    Ok(())
}

fn write_text(out: &mut impl Write, value: &str) -> std::io::Result<()> {
    out.write_all(b"\"")?;
    if value.starts_with(['\t', '\r', '\n'])
        || value
            .trim_start()
            .starts_with(['=', '+', '-', '@', '＝', '＋', '－', '＠'])
    {
        out.write_all(b"'")?;
    }
    let mut remaining = value;
    while let Some((prefix, suffix)) = remaining.split_once('"') {
        out.write_all(prefix.as_bytes())?;
        out.write_all(b"\"\"")?;
        remaining = suffix;
    }
    out.write_all(remaining.as_bytes())?;
    out.write_all(b"\"")
}

fn date(value: Option<&str>) -> String {
    let Some(ticks) = value.and_then(|v| v.parse::<i64>().ok()) else {
        return String::new();
    };
    DateTime::from_timestamp(
        ticks.div_euclid(10_000_000) - 11_644_473_600,
        (ticks.rem_euclid(10_000_000) * 100) as u32,
    )
    .map(|value| value.to_rfc3339_opts(SecondsFormat::Nanos, true))
    .unwrap_or_default()
}

fn write_row(out: &mut impl Write, hit: &SearchHit) -> std::io::Result<()> {
    let row = &hit.file;
    for value in [
        &row.name,
        &row.path,
        &hit.partition_name,
        &hit.connection,
        &row.kind,
    ] {
        write_text(out, value)?;
        out.write_all(b",")?;
    }
    let size = row
        .size
        .as_deref()
        .filter(|value| value.parse::<u64>().is_ok())
        .unwrap_or_default();
    write!(
        out,
        "{size},{},{}\r\n",
        date(row.created.as_deref()),
        date(row.modified.as_deref())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use stashfinder_core::index::{SearchPage, SearchRow};
    use std::{cell::Cell, future::ready, io::BufRead};

    #[test]
    fn quotes_csv_cells_and_neutralizes_spreadsheet_formulas() {
        let mut out = Vec::new();
        write_text(&mut out, " \t=SUM(1,2)\r\n\"中文\"").unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\"' \t=SUM(1,2)\r\n\"\"中文\"\"\""
        );
    }

    #[test]
    fn cancellation_preserves_the_previous_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.csv");
        std::fs::write(&path, b"previous").unwrap();
        let cancel = AtomicBool::new(true);
        let file = begin_csv(&path).unwrap();
        assert!(finish_csv(file, &path, &cancel).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"previous");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    fn page(action: Action, total: usize) -> Response {
        let Action::SearchBatch {
            id, offset, limit, ..
        } = action
        else {
            panic!("invalid request")
        };
        let rows = (offset..(offset + limit.min(17)).min(total))
            .map(|n| SearchHit {
                duplicate_group: None,
                hard_link: false,
                filesystem_id: "fs".into(),
                partition_id: "p".into(),
                partition_name: "离线盘".into(),
                connection: "offline".into(),
                mounts: vec![],
                generation: 1,
                revision: 1,
                partial: false,
                indexed_at: None,
                needs_validation: false,
                file: SearchRow {
                    entry: vec![],
                    parent: vec![],
                    object: vec![],
                    name: format!("row-{n:07}"),
                    path: format!("folder\\row-{n:07}"),
                    kind: "file".into(),
                    attributes: 0,
                    size: None,
                    created: None,
                    modified: Some("116444736000000001".into()),
                },
            })
            .collect::<Vec<_>>();
        let next = offset + rows.len();
        Response::SearchPage(Box::new(SearchPage {
            rows,
            total,
            groups: 0,
            missing: 0,
            id,
            done: true,
            row: None,
            next: (next < total).then_some(next),
            more: next < total,
            error: None,
            unavailable: vec![],
        }))
    }

    #[tokio::test]
    async fn streams_a_million_offline_rows_in_order_and_preserves_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.csv");
        let total = 1_000_000;
        let written = write_csv(
            &path,
            7,
            total,
            vec![[0, total]],
            &AtomicBool::new(false),
            |action| ready(Ok(page(action, total))),
            |_, _| {},
        )
        .await
        .unwrap();
        assert_eq!(written, total);
        let mut lines = std::io::BufReader::new(std::fs::File::open(&path).unwrap()).lines();
        assert!(lines.next().unwrap().unwrap().starts_with('\u{feff}'));
        for (n, line) in lines.enumerate() {
            let line = line.unwrap();
            assert_eq!(
                line,
                format!(
                    "\"row-{n:07}\",\"folder\\row-{n:07}\",\"离线盘\",\"offline\",\"file\",,,1970-01-01T00:00:00.000000100Z"
                )
            );
        }
        let reader = std::io::BufReader::new(std::fs::File::open(&path).unwrap());
        assert_eq!(reader.lines().count(), total + 1);
    }

    #[tokio::test]
    async fn exports_selected_spans_and_discards_partial_output_when_query_expires() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.csv");
        let cancel = AtomicBool::new(false);
        let total = 100;
        assert_eq!(
            write_csv(
                &path,
                7,
                total,
                vec![[2, 4], [90, 91]],
                &cancel,
                |action| ready(Ok(page(action, total))),
                |_, _| {}
            )
            .await
            .unwrap(),
            3
        );
        let saved = std::fs::read_to_string(&path).unwrap();
        let names = saved
            .lines()
            .skip(1)
            .map(|line| line.split(',').next().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            ["\"row-0000002\"", "\"row-0000003\"", "\"row-0000090\""]
        );
        let requests = Cell::new(0);
        let result = write_csv(
            &path,
            7,
            total,
            vec![[0, total]],
            &cancel,
            |action| {
                let n = requests.get();
                requests.set(n + 1);
                ready(if n == 0 {
                    Ok(page(action, total))
                } else {
                    Err(anyhow::anyhow!("搜索请求已过期"))
                })
            },
            |_, _| {},
        )
        .await;
        assert!(result.unwrap_err().to_string().contains("已过期"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), saved);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        assert!(begin_csv(&dir.path().join("missing").join("out.csv")).is_err());
    }
}
