use super::{Kind, Paths, Shared, THUMBNAIL_SIZE, Visuals, background, file_key};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use stashfinder_core::{
    Action, Response,
    file_operations::{PreparedTarget, ThumbnailPageRequest, VisualRecord},
};
use std::{
    collections::HashSet,
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
    discovered: usize,
    saved: usize,
    skipped: usize,
    unsupported: usize,
    failed: usize,
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
    let types = {
        let shared = shared.clone();
        let paths = paths.clone();
        background(move || shared.cached(&paths, |cache| Ok(cache.thumbnail_types().clone())))
            .await?
    };
    if types.selected_extensions().is_empty() {
        return Err("请先在「设置 → 图片缓存」选择预生成的文件类型".into());
    }
    let mut slot = shared.batch.lock();
    if slot.as_ref().is_some_and(|job| !job.state.finished()) {
        return Err("已有缩略图任务在运行".into());
    }
    *slot = Some(BatchStatus {
        partition_id: options.partition_id.clone(),
        label: options.label,
        state: BatchState::Running,
        discovered: 0,
        saved: 0,
        skipped: 0,
        unsupported: 0,
        failed: 0,
        error: None,
    });
    let epoch = shared.epoch.load(Ordering::Acquire);
    drop(slot);
    tauri::async_runtime::spawn(async move {
        let result = run(
            &shared,
            paths,
            ThumbnailPageRequest {
                partition_id: options.partition_id,
                filesystem_id: options.filesystem_id,
                generation: options.generation,
                folder: options.folder,
                after: None,
                types,
            },
            options.rebuild,
            epoch,
        )
        .await;
        let saved = {
            let mut slot = shared.batch.lock();
            let job = slot.as_mut().unwrap();
            job.state = if job.state == BatchState::Cancelling {
                BatchState::Cancelled
            } else if let Err(error) = result {
                job.error = Some(format!("{error:#}"));
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
}

#[tauri::command]
pub fn thumbnail_status(state: State<'_, Visuals>) -> Option<BatchStatus> {
    state.0.batch.lock().clone()
}

#[tauri::command]
pub fn thumbnail_control(state: State<'_, Visuals>, action: BatchAction) {
    if let Some(job) = state.0.batch.lock().as_mut() {
        job.state = match (action, job.state) {
            (BatchAction::Pause, BatchState::Running | BatchState::Offline) => BatchState::Paused,
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
            BatchState::Running | BatchState::Offline => return true,
            BatchState::Paused | BatchState::Quota | BatchState::Full => {
                tokio::time::sleep(Duration::from_millis(200)).await
            }
            _ => return false,
        }
    }
}

async fn run(
    shared: &Arc<Shared>,
    paths: Paths,
    mut request: ThumbnailPageRequest,
    rebuild: bool,
    epoch: u64,
) -> Result<()> {
    let mut folders = Vec::new();
    let mut seen = HashSet::new();
    if let Some(folder) = &request.folder {
        seen.insert(folder.clone());
    }
    while wait(&shared.batch).await {
        let response = crate::runtime::request(Action::ThumbnailPage(request.clone())).await?;
        let Response::ThumbnailPage(page) = response else {
            anyhow::bail!("缩略图候选响应无效");
        };
        let Some(page) = page else {
            {
                let mut lock = shared.batch.lock();
                let job = lock.as_mut().unwrap();
                if job.state == BatchState::Running {
                    job.state = BatchState::Offline;
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };
        {
            let mut lock = shared.batch.lock();
            let job = lock.as_mut().unwrap();
            if job.state == BatchState::Offline {
                job.state = BatchState::Running;
            }
            job.discovered += page.files.len() + page.unavailable;
            job.failed += page.unavailable;
        }
        request.generation = Some(page.generation);
        folders.extend(
            page.folders
                .into_iter()
                .filter(|id| seen.insert(id.clone())),
        );
        for file in page.files {
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
            loop {
                if !wait(&shared.batch).await {
                    return Ok(());
                }
                let permit = shared.slots.clone().acquire_owned().await?;
                let paths = paths.clone();
                let shared_for_item = shared.clone();
                let record = record.clone();
                let outcome = background(move || {
                    let _permit = permit;
                    shared_for_item.prebuild(&paths, &record, rebuild, epoch)
                })
                .await;
                let offline = matches!(outcome, Ok(Outcome::Offline));
                {
                    let mut slot = shared.batch.lock();
                    let job = slot.as_mut().unwrap();
                    match outcome {
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
                        Err(error) => {
                            job.failed += 1;
                            job.error = Some(error);
                        }
                    }
                }
                if offline {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
                break;
            }
        }
        if let Some(next) = page.next {
            request.after = Some(next);
        } else if let Some(folder) = folders.pop() {
            request.folder = Some(folder);
            request.after = None;
        } else {
            break;
        }
    }
    Ok(())
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
                    matches!(job.state, BatchState::Running | BatchState::Offline)
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
        let extracted = if rebuild {
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
            discovered: 1,
            saved: 0,
            skipped: 0,
            unsupported: 0,
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
        write_bmp([191, 50, 22]);
        assert_eq!(
            shared.prebuild(&paths, &row, true, 0).unwrap(),
            Outcome::Saved
        );
        let rebuilt = shared
            .cached(&paths, |cache| cache.get(key))
            .unwrap()
            .unwrap();
        assert_ne!(original, rebuilt);
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
        assert_eq!(restarted.status(&paths).unwrap().stats.files, 1);
    }
}
