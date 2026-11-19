use super::{Kind, Notify, Paths, Shared, THUMBNAIL_SIZE, Visuals, Work, background, file_key};
use anyhow::{Context, Result, bail, ensure};
use stashfinder_core::{
    Action, Response,
    file_operations::{EntryRef, VisualRecord},
};
use stashfinder_windows::file_operations::{inspect, verify};
use std::{
    env,
    fs::File,
    io::{Cursor, Read, Seek},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{Emitter, State};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MAX_PNG_BYTES: u64 = 3 * 1024 * 1024;
const EXTRACTION_TIMEOUT: Duration = Duration::from_secs(15);

fn executable() -> Option<PathBuf> {
    media_executable("ffmpeg.exe")
}
pub(super) fn media_executable(name: &str) -> Option<PathBuf> {
    env::split_paths(&env::var_os("PATH")?)
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

#[tauri::command]
pub async fn visual_ffmpeg_available() -> Result<bool, String> {
    background(|| Ok(executable().is_some())).await
}

#[tauri::command]
pub async fn visual_ffmpeg_rebuild(
    app: tauri::AppHandle,
    state: State<'_, Visuals>,
    request_id: String,
    reference: EntryRef,
) -> Result<(), String> {
    let paths = Paths::from_app(&app).map_err(|error| error.to_string())?;
    let shared = state.0.clone();
    let mut work = Work::new(shared.clone(), request_id).map_err(|error| error.to_string())?;
    let notify: Notify = Arc::new(|_| {});
    let _permit = work
        .enter(&notify)
        .await
        .map_err(|error| error.to_string())?;
    let response = crate::runtime::request(Action::FileInspect(reference))
        .await
        .map_err(|error| format!("{error:#}"))?;
    let Response::VisualRecord(record) = response else {
        return Err("图片条目响应无效".into());
    };
    let cancel = work.cancel.clone();
    background(move || {
        let ffmpeg = executable().context("未在 PATH 中找到 FFmpeg")?;
        shared.rebuild_with_ffmpeg(&paths, &record, &ffmpeg, &cancel)
    })
    .await?;
    let _ = app.emit("thumbnail-cache-updated", ());
    Ok(())
}

impl Shared {
    fn rebuild_with_ffmpeg(
        &self,
        paths: &Paths,
        record: &VisualRecord,
        ffmpeg: &Path,
        cancel: &AtomicBool,
    ) -> Result<()> {
        ensure!(
            record.online && !record.target.file.folder,
            "只支持补建在线文件的缩略图"
        );
        ensure!(!cancel.load(Ordering::Acquire), "已取消缩略图补建");
        let epoch = self.epoch.load(Ordering::Acquire);
        let key = file_key(record, Kind::Thumbnail, THUMBNAIL_SIZE)?;
        let path = verify(&record.target, false)?;
        let before = inspect(&path)?;
        ensure!(
            record
                .target
                .file
                .size
                .is_none_or(|size| before.size == Some(size))
                && record
                    .target
                    .file
                    .modified
                    .is_none_or(|modified| before.modified == Some(modified)),
            "索引记录已变化，请刷新重选"
        );
        let input = File::open(&path).context("无法读取待补建文件")?;
        let bytes = extract(ffmpeg, input, cancel)?;
        ensure!(!cancel.load(Ordering::Acquire), "已取消缩略图补建");
        let after = inspect(&verify(&record.target, false)?)?;
        ensure!(
            before.identity == after.identity
                && before.size == after.size
                && before.modified == after.modified,
            "文件在补建期间发生变化，请刷新后重试"
        );
        self.cached(paths, |cache| {
            ensure!(
                !cancel.load(Ordering::Acquire) && self.epoch.load(Ordering::Acquire) == epoch,
                "已取消缩略图补建"
            );
            ensure!(
                cache.fits_without_eviction(key, bytes.len()),
                "图片缓存空间不足，请调整缓存额度后重试"
            );
            cache.put(key, &bytes)?;
            Ok(())
        })
    }
}

fn extract(executable: &Path, input: File, cancel: &AtomicBool) -> Result<Vec<u8>> {
    ensure!(!cancel.load(Ordering::Acquire), "已取消缩略图补建");
    let mut output = tempfile::tempfile().context("无法创建缩略图临时文件")?;
    let scale = format!(
        "scale='min({THUMBNAIL_SIZE},iw)':'min({THUMBNAIL_SIZE},ih)':force_original_aspect_ratio=decrease"
    );
    // A seekable input handle supports containers such as MP4 without exposing another path.
    let mut child = Command::new(executable)
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-protocol_whitelist",
            "fd",
            "-threads",
            "1",
            "-i",
            "fd:",
            "-map",
            "0:v:0",
            "-frames:v",
            "1",
            "-filter_threads",
            "1",
            "-vf",
            &scale,
            "-c:v",
            "png",
            "-pix_fmt",
            "rgba",
            "-threads",
            "1",
            "-f",
            "image2pipe",
            "pipe:1",
        ])
        .stdin(Stdio::from(input))
        .stdout(Stdio::from(output.try_clone()?))
        .stderr(Stdio::null())
        .spawn()
        .context("无法启动 FFmpeg")?;
    let status = wait_child(&mut child, EXTRACTION_TIMEOUT, cancel)?;
    ensure!(
        status.success(),
        "FFmpeg 无法从此文件提取缩略图（{status}）"
    );
    output.rewind()?;
    let mut bytes = Vec::new();
    output.take(MAX_PNG_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_PNG_BYTES,
        "FFmpeg 生成的缩略图过大"
    );
    let mut reader = png::Decoder::new(Cursor::new(&bytes))
        .read_info()
        .context("FFmpeg 未返回有效缩略图")?;
    ensure!(
        (1..=THUMBNAIL_SIZE).contains(&reader.info().width)
            && (1..=THUMBNAIL_SIZE).contains(&reader.info().height),
        "FFmpeg 生成的缩略图尺寸无效"
    );
    let mut pixels = vec![
        0;
        reader
            .output_buffer_size()
            .context("FFmpeg 生成的缩略图过大")?
    ];
    reader
        .next_frame(&mut pixels)
        .context("FFmpeg 生成的缩略图损坏")?;
    reader.finish().context("FFmpeg 生成的缩略图不完整")?;
    Ok(bytes)
}

pub(super) fn wait_child(
    child: &mut Child,
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<ExitStatus> {
    let started = Instant::now();
    let result = (|| {
        loop {
            ensure!(!cancel.load(Ordering::Acquire), "已取消缩略图补建");
            if let Some(status) = child.try_wait().context("无法读取 FFmpeg 运行状态")? {
                return Ok(status);
            }
            if started.elapsed() >= timeout {
                bail!("媒体处理超时");
            }
            thread::sleep(Duration::from_millis(25));
        }
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::visuals::{
        WindowsProvider,
        tests::{paths, record},
    };
    use stashfinder_core::{assets::Serial, file_operations::ExpectedIdentity};
    use std::fs;

    #[test]
    fn cancelling_or_timing_out_stops_and_reaps_a_running_child() {
        let powershell = PathBuf::from(env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        for cancel_running in [false, true] {
            let mut child = Command::new(&powershell)
                .creation_flags(CREATE_NO_WINDOW)
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Start-Sleep -Seconds 10",
                ])
                .spawn()
                .unwrap();
            let cancel = Arc::new(AtomicBool::new(false));
            let worker = cancel_running.then(|| {
                let cancel = cancel.clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(60));
                    cancel.store(true, Ordering::Release);
                })
            });
            let timeout = if cancel_running {
                Duration::from_secs(5)
            } else {
                Duration::from_millis(60)
            };
            let error = wait_child(&mut child, timeout, &cancel)
                .unwrap_err()
                .to_string();
            if let Some(worker) = worker {
                worker.join().unwrap();
            }
            assert!(error.contains(if cancel_running { "取消" } else { "超时" }));
            assert!(child.try_wait().unwrap().is_some());
        }
    }

    #[test]
    #[ignore = "requires a local FFmpeg executable in STASHFINDER_TEST_FFMPEG"]
    fn real_ffmpeg_replaces_only_successful_cached_thumbnails() {
        let executable = PathBuf::from(env::var_os("STASHFINDER_TEST_FFMPEG").unwrap());
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        let video = source.join("video.mp4");
        let status = Command::new(&executable)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=96x64:rate=2",
                "-t",
                "1",
                "-c:v",
                "mpeg4",
            ])
            .arg(&video)
            .status()
            .unwrap();
        assert!(status.success());
        let invalid = source.join("broken.mp4");
        fs::write(&invalid, b"not a video").unwrap();
        let paths = paths(&temp);
        let shared = Shared::new(Arc::new(WindowsProvider));
        let original = stashfinder_windows::visuals::type_icon("old.png", false, 32).unwrap();
        for (input, success) in [(&video, true), (&invalid, false)] {
            let info = inspect(input).unwrap();
            let mut record = record();
            record.target.volume = source.display().to_string();
            record.target.identity.serial = Some(Serial {
                source: "win32_volume32".into(),
                value: format!("{:x}", info.identity.volume),
            });
            record.target.file.identity = Some(ExpectedIdentity::Native(info.identity));
            record.target.file.relative = input
                .file_name()
                .unwrap()
                .to_string_lossy()
                .encode_utf16()
                .collect();
            record.target.file.size = info.size;
            record.target.file.created = info.created;
            record.target.file.modified = info.modified;
            let key = file_key(&record, Kind::Thumbnail, THUMBNAIL_SIZE).unwrap();
            shared
                .cached(&paths, |cache| cache.put(key, &original))
                .unwrap();
            assert!(
                shared
                    .rebuild_with_ffmpeg(&paths, &record, &executable, &AtomicBool::new(true))
                    .is_err()
            );
            assert_eq!(
                shared
                    .cached(&paths, |cache| cache.get(key))
                    .unwrap()
                    .unwrap(),
                original
            );
            let result =
                shared.rebuild_with_ffmpeg(&paths, &record, &executable, &AtomicBool::new(false));
            if success {
                result.unwrap();
            } else {
                assert!(result.unwrap_err().to_string().contains("无法从此文件提取"));
            }
            let bytes = shared
                .cached(&paths, |cache| cache.get(key))
                .unwrap()
                .unwrap();
            if success {
                let decoded = png::Decoder::new(Cursor::new(&bytes)).read_info().unwrap();
                assert_eq!((decoded.info().width, decoded.info().height), (96, 64));
                assert_ne!(bytes, original);
                shared
                    .cached(&paths, |cache| {
                        cache.put(key, &original)?;
                        cache.set_limit(Some(original.len() as u64), |_| {})
                    })
                    .unwrap();
                let error = shared
                    .rebuild_with_ffmpeg(&paths, &record, &executable, &AtomicBool::new(false))
                    .unwrap_err();
                assert!(error.to_string().contains("额度"));
                assert_eq!(
                    shared
                        .cached(&paths, |cache| cache.get(key))
                        .unwrap()
                        .unwrap(),
                    original
                );
                shared
                    .cached(&paths, |cache| cache.set_limit(None, |_| {}))
                    .unwrap();
            } else {
                assert_eq!(bytes, original);
            }
        }
        let mut child = Command::new(&executable)
            .creation_flags(CREATE_NO_WINDOW)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-re",
                "-stream_loop",
                "-1",
                "-i",
            ])
            .arg(&video)
            .args(["-f", "null", "-"])
            .spawn()
            .unwrap();
        assert!(child.try_wait().unwrap().is_none());
        let cancel = AtomicBool::new(true);
        assert!(
            wait_child(&mut child, EXTRACTION_TIMEOUT, &cancel)
                .unwrap_err()
                .to_string()
                .contains("取消")
        );
        assert!(child.try_wait().unwrap().is_some());
    }
}
