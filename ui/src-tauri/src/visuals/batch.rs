use super::{
    Kind, Paths, Shared, THUMBNAIL_SIZE, Visuals, analysis::Analysis, background, file_key,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use stashfinder_core::{
    Action, Response,
    file_operations::{PreparedTarget, ThumbnailScope, ThumbnailTypes, VisualRecord},
};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    sync::{Arc, Mutex, MutexGuard, atomic::Ordering},
    time::Duration,
};
use tauri::{Emitter, State};

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BatchState {
    Running,
    Paused,
    Offline,
    WaitingForIndex,
    Quota,
    Full,
    Cancelling,
    Cancelled,
    Completed,
    Failed,
}
impl BatchState {
    fn finished(self) -> bool {
        matches!(self, Self::Cancelled | Self::Completed | Self::Failed)
    }
}

#[derive(Clone, Serialize)]
pub struct BatchStatus {
    partition_id: String,
    label: String,
    state: BatchState,
    total: Option<usize>,
    saved: usize,
    skipped: usize,
    unsupported: usize,
    analysing: bool,
    videos: usize,
    other: usize,
    unknown: usize,
    failed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Default)]
pub(super) struct Batch(Mutex<Option<BatchStatus>>);
impl Batch {
    fn lock(&self) -> MutexGuard<'_, Option<BatchStatus>> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }
    pub(super) fn cancel(&self) {
        if let Some(job) = self.lock().as_mut()
            && !job.state.finished()
        {
            job.state = BatchState::Cancelling;
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchAction {
    Pause,
    Resume,
    Cancel,
}

#[tauri::command]
pub async fn thumbnail_start(
    app: tauri::AppHandle,
    state: State<'_, Visuals>,
    options: BatchOptions,
) -> Result<(), String> {
    let paths = Paths::from_app(&app).map_err(|error| error.to_string())?;
    let shared = state.0.clone();
    let probe = options
        .analyse
        .then(|| {
            super::ffmpeg::media_executable("ffprobe.exe")
                .ok_or("未找到 ffprobe.exe，请将 FFmpeg 的 bin 目录加入 PATH 后重启应用")
        })
        .transpose()?;
    let mut types = {
        let shared = shared.clone();
        let cache_paths = paths.clone();
        background(move || shared.cached(&cache_paths, |cache| Ok(cache.thumbnail_types().clone())))
            .await
            .inspect_err(|error| log_error(&paths, &options.label, None, error))?
    };
    if !options.thumbnails {
        types.images = false;
        types.videos = false;
        types.documents = false;
        types.extensions.clear();
    }
    if probe.is_none() && types.selected_extensions().is_empty() {
        let error = "请先在「设置 → 图片缓存」选择预生成的文件类型";
        log_error(&paths, &options.label, None, error);
        return Err(error.into());
    }
    let mut slot = shared.batch.lock();
    if slot.as_ref().is_some_and(|job| !job.state.finished()) {
        drop(slot);
        let error = "已有缩略图任务在运行";
        log_error(&paths, &options.label, None, error);
        return Err(error.into());
    }
    *slot = Some(BatchStatus {
        partition_id: options.partition_id.clone(),
        label: options.label,
        state: BatchState::Running,
        total: None,
        saved: 0,
        skipped: 0,
        unsupported: 0,
        analysing: options.analyse,
        videos: 0,
        other: 0,
        unknown: 0,
        failed: 0,
        error: None,
    });
    let epoch = shared.epoch.load(Ordering::Acquire);
    drop(slot);
    tauri::async_runtime::spawn(async move {
        let context = &shared;
        let cache_paths = &paths;
        let result = async move {
            let scope = ThumbnailScope {
                analyse: options.analyse,
                partition_id: options.partition_id,
                filesystem_id: options.filesystem_id,
                generation: options.generation,
                folder: options.folder,
                types: types.clone(),
            };
            let Response::ThumbnailStarted(id) =
                crate::runtime::request(Action::ThumbnailBegin(scope)).await?
            else {
                anyhow::bail!("分析任务启动响应无效");
            };
            let result = run(
                context,
                cache_paths,
                types,
                options.rebuild,
                probe,
                epoch,
                async move |offset| {
                    crate::runtime::request(Action::ThumbnailPage { id, offset }).await
                },
            )
            .await;
            let ended = crate::runtime::request(Action::ThumbnailEnd { id }).await;
            result?;
            ended?;
            Ok::<(), anyhow::Error>(())
        }
        .await;
        if let Err(error) = &result {
            let label = shared.batch.lock().as_ref().unwrap().label.clone();
            log_error(&paths, &label, None, &format!("{error:#}"));
        }
        let saved = {
            let mut slot = shared.batch.lock();
            let job = slot.as_mut().unwrap();
            job.error = result.as_ref().err().map(|error| format!("{error:#}"));
            job.state = if job.state == BatchState::Cancelling {
                BatchState::Cancelled
            } else if result.is_err() {
                BatchState::Failed
            } else {
                BatchState::Completed
            };
            job.saved
        };
        if saved > 0 {
            let _ = app.emit("thumbnail-cache-updated", ());
        }
    });
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchOptions {
    partition_id: String,
    filesystem_id: String,
    folder: Option<Vec<u8>>,
    generation: Option<String>,
    label: String,
    rebuild: bool,
    #[serde(default)]
    analyse: bool,
    #[serde(default = "default_thumbnails")]
    thumbnails: bool,
}
fn default_thumbnails() -> bool {
    true
}

#[tauri::command]
pub fn thumbnail_status(state: State<'_, Visuals>) -> Option<BatchStatus> {
    state.0.batch.lock().clone()
}

#[tauri::command]
pub fn thumbnail_control(state: State<'_, Visuals>, action: BatchAction) {
    if let Some(job) = state.0.batch.lock().as_mut() {
        job.state = match (action, job.state) {
            (
                BatchAction::Pause,
                BatchState::Running | BatchState::Offline | BatchState::WaitingForIndex,
            ) => BatchState::Paused,
            (BatchAction::Resume, BatchState::Paused | BatchState::Quota | BatchState::Full) => {
                BatchState::Running
            }
            (BatchAction::Cancel, current) if !current.finished() => BatchState::Cancelling,
            (_, current) => current,
        };
    }
}

async fn wait(batch: &Batch) -> bool {
    loop {
        let state = batch.lock().as_ref().unwrap().state;
        match state {
            BatchState::Running | BatchState::Offline | BatchState::WaitingForIndex => return true,
            BatchState::Paused | BatchState::Quota | BatchState::Full => {
                tokio::time::sleep(Duration::from_millis(200)).await
            }
            _ => return false,
        }
    }
}

async fn run(
    shared: &Arc<Shared>,
    paths: &Paths,
    types: ThumbnailTypes,
    rebuild: bool,
    probe: Option<std::path::PathBuf>,
    epoch: u64,
    mut fetch: impl AsyncFnMut(Option<usize>) -> Result<Response>,
) -> Result<()> {
    let thumbnail_extensions = types.selected_extensions();
    let label = shared.batch.lock().as_ref().unwrap().label.clone();
    let mut offset = 0;
    let mut total = None;
    while wait(&shared.batch).await && total != Some(offset) {
        let page = match fetch(total.map(|_| offset)).await? {
            Response::ThumbnailCounting => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            Response::ThumbnailCount(count) if total.is_none() => {
                total = Some(count);
                shared.batch.lock().as_mut().unwrap().total = total;
                continue;
            }
            Response::AnalysisOffline => {
                {
                    let mut slot = shared.batch.lock();
                    let job = slot.as_mut().unwrap();
                    if job.state == BatchState::Running {
                        job.state = BatchState::Offline;
                    }
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
            Response::ThumbnailPage(page) => page,
            _ => anyhow::bail!("分析候选响应无效"),
        };
        let received = page.files.len() + page.unavailable;
        ensure!(
            received > 0 && total.is_some_and(|n| received <= n - offset),
            "分析候选数量与清点结果不一致"
        );
        offset += received;
        {
            let mut slot = shared.batch.lock();
            let job = slot.as_mut().unwrap();
            if job.state == BatchState::Offline {
                job.state = BatchState::Running;
            }
            job.failed += page.unavailable;
        }
        if page.unavailable > 0 {
            log_error(
                paths,
                &label,
                None,
                &format!("{} 个索引候选无法解析", page.unavailable),
            );
        }
        for (index, file) in page.files.into_iter().enumerate() {
            let record = VisualRecord {
                online: true,
                target: PreparedTarget {
                    file,
                    mounts: page.mounts.clone(),
                    volume: page.volume.clone(),
                    identity: page.identity.clone(),
                    partition_name: page.partition_name.clone(),
                },
            };
            let name = String::from_utf16_lossy(&record.target.file.relative).to_lowercase();
            let ext = name.rsplit_once('.').map_or("", |(_, x)| x);
            let mut finding = None;
            let mut analysis_failed = false;
            loop {
                if !wait(&shared.batch).await {
                    return Ok(());
                }
                if let Some(exe) = &probe
                    && !analysis_failed
                    && finding.is_none()
                    && page.analyse_indices.binary_search(&index).is_ok()
                {
                    match super::analysis::analyse(&record, exe, rebuild, crate::runtime::request)
                        .await
                    {
                        Ok(reason @ (Analysis::Busy | Analysis::Offline)) => {
                            let state = if matches!(reason, Analysis::Offline) {
                                BatchState::Offline
                            } else {
                                BatchState::WaitingForIndex
                            };
                            {
                                let mut lock = shared.batch.lock();
                                let job = lock.as_mut().unwrap();
                                if matches!(
                                    job.state,
                                    BatchState::Running
                                        | BatchState::WaitingForIndex
                                        | BatchState::Offline
                                ) {
                                    job.state = state;
                                }
                            }
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            continue;
                        }
                        Ok(Analysis::Found(value)) => {
                            let mut slot = shared.batch.lock();
                            let job = slot.as_mut().unwrap();
                            if matches!(
                                job.state,
                                BatchState::WaitingForIndex | BatchState::Offline
                            ) {
                                job.state = BatchState::Running;
                            }
                            match value {
                                stashfinder_core::file_operations::VideoFinding::Video => {
                                    job.videos += 1
                                }
                                stashfinder_core::file_operations::VideoFinding::Other => {
                                    job.other += 1
                                }
                                stashfinder_core::file_operations::VideoFinding::Unknown => {
                                    job.unknown += 1
                                }
                            }
                            finding = Some(value);
                        }
                        Err(error) => {
                            log_error(
                                paths,
                                &label,
                                Some(&record.target.file.relative),
                                &format!("{error:#}"),
                            );
                            analysis_failed = true;
                        }
                    }
                }
                if probe.is_some()
                    && !(thumbnail_extensions.contains(ext)
                        || types.videos
                            && finding
                                == Some(stashfinder_core::file_operations::VideoFinding::Video))
                {
                    let mut slot = shared.batch.lock();
                    let job = slot.as_mut().unwrap();
                    if analysis_failed {
                        job.failed += 1;
                    } else if finding.is_some() {
                        job.saved += 1;
                    } else {
                        job.skipped += 1;
                    }
                    break;
                }
                let permit = shared.slots.clone().acquire_owned().await?;
                let cache_paths = paths.clone();
                let shared_for_item = shared.clone();
                let candidate = record.clone();
                let outcome = background(move || {
                    let _permit = permit;
                    shared_for_item.prebuild(&cache_paths, &candidate, rebuild, epoch)
                })
                .await;
                if let Err(error) = &outcome {
                    log_error(paths, &label, Some(&record.target.file.relative), error);
                }
                let offline = matches!(outcome, Ok(Outcome::Offline));
                {
                    let mut slot = shared.batch.lock();
                    let job = slot.as_mut().unwrap();
                    match outcome {
                        Ok(Outcome::Saved | Outcome::Skipped | Outcome::Unsupported)
                            if analysis_failed =>
                        {
                            job.failed += 1
                        }
                        Ok(Outcome::Saved) => job.saved += 1,
                        Ok(Outcome::Skipped) => job.skipped += 1,
                        Ok(Outcome::Unsupported) => job.unsupported += 1,
                        Ok(reason @ (Outcome::Quota | Outcome::Full)) => {
                            if job.state == BatchState::Running {
                                job.state = if reason == Outcome::Quota {
                                    BatchState::Quota
                                } else {
                                    BatchState::Full
                                };
                            }
                            continue;
                        }
                        Ok(Outcome::Offline) => {
                            if job.state == BatchState::Running {
                                job.state = BatchState::Offline;
                            }
                        }
                        Ok(Outcome::Interrupted) => continue,
                        Err(_) => job.failed += 1,
                    }
                }
                if offline {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
                break;
            }
        }
    }
    Ok(())
}

fn log_error(paths: &Paths, label: &str, relative: Option<&[u16]>, error: &str) {
    let row = serde_json::json!({
        "time": stashfinder_core::assets::now(),
        "scope": label,
        "file": relative.map(String::from_utf16_lossy),
        "error": error,
    });
    eprintln!("{row}");
    let path = paths.root.with_file_name("thumbnail-jobs.log");
    let result = (|| -> std::io::Result<()> {
        fs::create_dir_all(path.parent().expect("cache directory has a parent"))?;
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(file, "{row}")
    })();
    if let Err(error) = result {
        eprintln!("cannot save thumbnail job log: {error}");
    }
}

#[derive(Debug, PartialEq)]
pub(super) enum Outcome {
    Saved,
    Skipped,
    Unsupported,
    Quota,
    Full,
    Offline,
    Interrupted,
}

impl Shared {
    pub(super) fn prebuild(
        &self,
        paths: &Paths,
        record: &VisualRecord,
        rebuild: bool,
        epoch: u64,
    ) -> Result<Outcome> {
        let active = || {
            self.epoch.load(Ordering::Acquire) == epoch
                && self.batch.lock().as_ref().is_some_and(|job| {
                    matches!(
                        job.state,
                        BatchState::Running | BatchState::Offline | BatchState::WaitingForIndex
                    )
                })
        };
        if !active() {
            return Ok(Outcome::Interrupted);
        }
        let key = file_key(record, Kind::Thumbnail, THUMBNAIL_SIZE)?;
        if !rebuild
            && self.cached(paths, |cache| {
                Ok(cache.get(key)?.is_some_and(|bytes| {
                    bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.ends_with(b"IEND\xaeB\x60\x82")
                }))
            })?
        {
            return Ok(Outcome::Skipped);
        }
        ensure!(record.online, "分区离线");
        let extracted = if rebuild || record.target.file.content_version.is_some() {
            self.provider
                .rebuild_thumbnail(&record.target, THUMBNAIL_SIZE)
        } else {
            // WTS_EXTRACT can reuse an existing Windows thumbnail.
            self.provider
                .thumbnail(&record.target, THUMBNAIL_SIZE, false)
        };
        let bytes = match extracted {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Ok(Outcome::Unsupported),
            Err(_) if !std::path::Path::new(&record.target.volume).exists() => {
                return Ok(Outcome::Offline);
            }
            Err(error) => return Err(error),
        };
        if let Some(job) = self.batch.lock().as_mut()
            && job.state == BatchState::Offline
        {
            job.state = BatchState::Running;
        }
        self.cached(paths, |cache| {
            if !active() {
                return Ok(Outcome::Interrupted);
            }
            if !cache.fits_without_eviction(key, bytes.len()) {
                return Ok(Outcome::Quota);
            }
            if cache
                .free_bytes()
                .is_some_and(|free| free < bytes.len() as u64)
            {
                return Ok(Outcome::Full);
            }
            match cache.put(key, &bytes) {
                Ok(_) => Ok(Outcome::Saved),
                Err(error)
                    if error.chain().any(|cause| {
                        cause
                            .downcast_ref::<std::io::Error>()
                            .is_some_and(|io| matches!(io.raw_os_error(), Some(39 | 112)))
                    }) =>
                {
                    Ok(Outcome::Full)
                }
                Err(error) => Err(error),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::visuals::{
        WindowsProvider,
        tests::{paths, record},
    };
    use stashfinder_core::{assets::Serial, file_operations::ExpectedIdentity};
    use stashfinder_windows::file_operations::inspect;
    use std::{fs, sync::atomic::AtomicBool};

    #[tokio::test]
    async fn counts_all_candidates_before_processing_and_logs_file_failures() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let shared = Arc::new(Shared::new(Arc::new(WindowsProvider)));
        *shared.batch.lock() = Some(BatchStatus {
            partition_id: "p".into(),
            label: "Photos".into(),
            state: BatchState::Running,
            total: None,
            saved: 0,
            skipped: 0,
            unsupported: 0,
            analysing: false,
            videos: 0,
            other: 0,
            unknown: 0,
            failed: 0,
            error: None,
        });
        let mut pages = 0;
        run(
            &shared,
            &paths,
            ThumbnailTypes::default(),
            false,
            None,
            0,
            async |offset| {
                let slot = shared.batch.lock();
                let status = slot.as_ref().unwrap();
                let Some(offset) = offset else {
                    assert_eq!(status.total, None);
                    return Ok(Response::ThumbnailCount(2));
                };
                assert_eq!(status.total, Some(2));
                assert_eq!(status.failed, offset);
                assert_eq!(offset, pages);
                pages += 1;
                let target = record().target;
                Ok(Response::ThumbnailPage(Box::new(
                    stashfinder_core::file_operations::ThumbnailPage {
                        analyse_indices: vec![],
                        files: vec![target.file],
                        unavailable: 0,
                        mounts: vec![],
                        volume: temp.path().display().to_string(),
                        identity: target.identity,
                        partition_name: "Photos".into(),
                    },
                )))
            },
        )
        .await
        .unwrap();
        let status = shared.batch.lock();
        let status = status.as_ref().unwrap();
        assert_eq!((pages, status.total, status.failed), (2, Some(2), 2));
        let errors = fs::read_to_string(paths.root.with_file_name("thumbnail-jobs.log")).unwrap();
        assert_eq!(errors.lines().count(), 2);
        let public = serde_json::to_value(status).unwrap();
        assert!(public.get("error").is_none());
    }

    #[test]
    fn windows_batch_reuses_and_rebuilds_one_png_then_reopens_it_without_the_source() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        let sample = source.join("sample.bmp");
        let write_bmp = |bgr: [u8; 3]| {
            let pixels = bgr.repeat(64 * 48);
            let mut header = vec![0u8; 54];
            header[..2].copy_from_slice(b"BM");
            header[2..6].copy_from_slice(&((54 + pixels.len()) as u32).to_le_bytes());
            header[10..14].copy_from_slice(&54u32.to_le_bytes());
            header[14..18].copy_from_slice(&40u32.to_le_bytes());
            header[18..22].copy_from_slice(&64u32.to_le_bytes());
            header[22..26].copy_from_slice(&48u32.to_le_bytes());
            header[26..28].copy_from_slice(&1u16.to_le_bytes());
            header[28..30].copy_from_slice(&24u16.to_le_bytes());
            header.extend(pixels);
            fs::write(&sample, header).unwrap();
        };
        write_bmp([32, 112, 217]);
        let mut row = record();
        let native = inspect(&sample).unwrap();
        row.target.file.size = native.size;
        row.target.file.modified = native.modified;
        row.target.file.created = native.created;
        row.target.volume = source.display().to_string();
        row.target.identity.serial = Some(Serial {
            source: "win32_volume32".into(),
            value: format!("{:x}", native.identity.volume),
        });
        row.target.file.identity = Some(ExpectedIdentity::Ntfs {
            frn: native.identity.file,
        });
        row.target.file.relative = "sample.bmp".encode_utf16().collect();

        let shared = Shared::new(Arc::new(WindowsProvider));
        *shared.batch.lock() = Some(BatchStatus {
            partition_id: "p".into(),
            label: "Photos".into(),
            state: BatchState::Running,
            total: Some(1),
            saved: 0,
            skipped: 0,
            unsupported: 0,
            analysing: false,
            videos: 0,
            other: 0,
            unknown: 0,
            failed: 0,
            error: None,
        });
        assert_eq!(
            shared.prebuild(&paths, &row, false, 0).unwrap(),
            Outcome::Saved
        );
        assert_eq!(
            shared.prebuild(&paths, &row, false, 0).unwrap(),
            Outcome::Skipped
        );
        let key = file_key(&row, Kind::Thumbnail, THUMBNAIL_SIZE).unwrap();
        let original = shared
            .cached(&paths, |cache| cache.get(key))
            .unwrap()
            .unwrap();
        assert!(
            !shared
                .cached(&paths, |cache| Ok(
                    cache.fits_without_eviction(key, 2_000_000_001)
                ))
                .unwrap()
        );
        let timestamp = fs::metadata(&sample).unwrap().modified().unwrap();
        write_bmp([191, 50, 22]);
        fs::File::options()
            .write(true)
            .open(&sample)
            .unwrap()
            .set_modified(timestamp)
            .unwrap();
        let after = inspect(&sample).unwrap();
        assert_eq!(after.identity, native.identity);
        assert_eq!(after.size, native.size);
        assert_eq!(after.modified, native.modified);
        row.target.file.reference.revision = "2".into();
        assert_eq!(
            shared.prebuild(&paths, &row, false, 0).unwrap(),
            Outcome::Skipped
        );
        row.target.file.content_version = Some(123);
        assert_eq!(
            shared.prebuild(&paths, &row, false, 0).unwrap(),
            Outcome::Saved
        );
        let key = file_key(&row, Kind::Thumbnail, THUMBNAIL_SIZE).unwrap();
        let rebuilt = shared
            .cached(&paths, |cache| cache.get(key))
            .unwrap()
            .unwrap();
        assert_ne!(original, rebuilt);
        assert_eq!(
            shared.prebuild(&paths, &row, true, 0).unwrap(),
            Outcome::Saved
        );
        fs::rename(&source, temp.path().join("detached")).unwrap();
        assert_eq!(
            shared.prebuild(&paths, &row, true, 0).unwrap(),
            Outcome::Offline
        );
        drop(shared);
        row.online = false;
        let restarted = Shared::new(Arc::new(WindowsProvider));
        let cancel = AtomicBool::new(false);
        let expected = super::super::Image::png(&rebuilt, "local_cache", true, None).data_url;
        for size in [32, 64, 128, THUMBNAIL_SIZE] {
            let image = restarted
                .load_file(&paths, &row, Kind::Thumbnail, size, &cancel, &|_| {})
                .unwrap();
            assert!(image.historical);
            assert_eq!(image.data_url, expected);
        }
        assert_eq!(restarted.status(&paths).unwrap().stats.files, 2);
    }
}
