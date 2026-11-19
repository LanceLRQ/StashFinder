use super::{background, ffmpeg::wait_child};
use anyhow::{Context, Result};
use stashfinder_core::{
    Action, Response,
    file_operations::{VideoFinding, VisualRecord},
};
use stashfinder_windows::file_operations::open_for_analysis;
use std::{
    io::{Read, Seek, Write},
    os::windows::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    sync::atomic::AtomicBool,
    time::Duration,
};

const LIMIT: u64 = 256 * 1024;

pub(super) enum Analysis {
    Busy,
    Offline,
    Found(VideoFinding),
}

pub(super) async fn analyse(
    record: &VisualRecord,
    exe: &Path,
    rebuild: bool,
    mut request: impl AsyncFnMut(Action) -> Result<Response>,
) -> Result<Analysis> {
    let file = match request(Action::VideoAnalysis {
        reference: record.target.file.reference.clone(),
        result: None,
    })
    .await?
    {
        Response::AnalysisBusy => return Ok(Analysis::Busy),
        Response::AnalysisOffline => return Ok(Analysis::Offline),
        Response::VideoAnalysis {
            finding: Some(found),
            ..
        } if !rebuild => return Ok(Analysis::Found(found)),
        Response::VideoAnalysis { file, .. } => *file,
        _ => anyhow::bail!("视频分析状态响应无效"),
    };
    let reference = file.reference.clone();
    let mut target = record.target.clone();
    target.file = file;
    let exe = exe.to_owned();
    let (found, file) = background(move || {
        let mut file = open_for_analysis(&target)?;
        let found = probe(&exe, &mut file)?;
        Ok((found, file))
    })
    .await
    .map_err(anyhow::Error::msg)?;
    let reply = request(Action::VideoAnalysis {
        reference,
        result: Some(found),
    })
    .await?;
    drop(file);
    match reply {
        Response::Accepted => Ok(Analysis::Found(found)),
        Response::AnalysisOffline => Ok(Analysis::Offline),
        _ => anyhow::bail!("视频分析写回失败"),
    }
}

fn probe(exe: &Path, file: &mut std::fs::File) -> Result<VideoFinding> {
    let complete = file.metadata()?.len() <= LIMIT;
    let mut data = Vec::new();
    file.take(LIMIT).read_to_end(&mut data)?;
    if complete
        && data.len() >= 8
        && data.iter().all(|b| *b >= 32 || matches!(b, 9 | 10 | 13))
        && std::str::from_utf8(&data).is_ok()
    {
        return Ok(VideoFinding::Other);
    }
    let mut input = tempfile::tempfile()?;
    input.write_all(&data)?;
    input.rewind()?;
    let mut output = tempfile::tempfile()?;
    let mut child = Command::new(exe)
        .creation_flags(0x0800_0000)
        .args([
            "-v", "error", "-protocol_whitelist", "fd",
            "-format_whitelist", "mpeg,mpegts,mp3,mov,matroska,webm,avi,asf,libopenmpt",
            "-show_entries", "stream=codec_type,codec_name,width,height:stream_disposition=attached_pic,timed_thumbnails,still_image",
            "-of", "json", "-i", "fd:",
        ])
        .stdin(Stdio::from(input))
        .stdout(Stdio::from(output.try_clone()?))
        .stderr(Stdio::null())
        .spawn()
        .context("无法启动 ffprobe")?;
    if !wait_child(&mut child, Duration::from_secs(3), &AtomicBool::new(false))?.success() {
        return Ok(VideoFinding::Unknown);
    }
    output.rewind()?;
    let json: serde_json::Value = serde_json::from_reader(output.take(1024 * 1024))?;
    let tracks = json["streams"]
        .as_array()
        .context("ffprobe 未返回轨道信息")?;
    let picture = |s: &serde_json::Value| {
        ["attached_pic", "timed_thumbnails", "still_image"]
            .iter()
            .any(|k| s["disposition"][k].as_i64() == Some(1))
    };
    if tracks.iter().any(|s| {
        s["codec_type"] == "video"
            && s["codec_name"].as_str().is_some_and(|n| n != "unknown")
            && s["width"].as_u64().unwrap_or(0) > 0
            && s["height"].as_u64().unwrap_or(0) > 0
            && !picture(s)
    }) {
        Ok(VideoFinding::Video)
    } else if complete
        && tracks.iter().any(|s| s["codec_type"] == "audio")
        && tracks
            .iter()
            .all(|s| s["codec_type"] == "audio" || picture(s))
    {
        Ok(VideoFinding::Other)
    } else {
        Ok(VideoFinding::Unknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use VideoFinding::{Other, Unknown, Video};
    use stashfinder_core::{assets::Serial, file_operations::ExpectedIdentity};
    use stashfinder_windows::file_operations::inspect;
    use std::path::PathBuf;

    #[test]
    #[ignore = "requires an external ffprobe and the downloaded media fixtures"]
    fn verifies_downloaded_video_samples() {
        let samples = PathBuf::from(std::env::var_os("STASHFINDER_MEDIA_SAMPLES").unwrap());
        let exe = PathBuf::from(std::env::var_os("STASHFINDER_FFPROBE").unwrap());
        for (name, expected) in [
            ("broadcast.ts", Video),
            ("sony.mts", Video),
            ("vcd03.dat", Video),
            ("vcd06.dat", Video),
            ("mpeg2.dat", Video),
            ("jvc.mod", Video),
            ("recording.vdr", Video),
            ("renamed.bin", Video),
            ("archive.dat", Video),
            ("module.mts", Other),
            ("music.mod", Other),
            ("audio.mp4", Other),
            ("source.ts", Unknown),
            ("translation.ts", Unknown),
            ("public_suffix_list.dat", Unknown),
            ("opaque.dat", Unknown),
            ("cut.dat", Unknown),
            ("empty.ts", Unknown),
            ("covers.mp3", Unknown),
            ("resume.vdr", Unknown),
            ("tail-moov.mp4", Unknown),
        ] {
            let info = inspect(&samples.join(name)).unwrap();
            let mut target = crate::visuals::tests::record().target;
            target.volume = samples.display().to_string();
            target.identity.serial = Some(Serial {
                source: "ntfs64".into(),
                value: format!("{:x}", info.volume64.unwrap()),
            });
            target.file.relative = name.encode_utf16().collect();
            target.file.identity = Some(ExpectedIdentity::Ntfs {
                frn: info.identity.file,
            });
            target.file.size = info.size;
            target.file.modified = info.modified;
            let mut file = open_for_analysis(&target).unwrap();
            let finding = probe(&exe, &mut file).unwrap();
            println!("{name}: {finding:?}");
            assert_eq!(finding, expected, "{name}");
            assert!(file.stream_position().unwrap() <= LIMIT);
        }
    }
    #[tokio::test]
    async fn disconnect_while_writing_a_finding_is_resumable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.ts");
        std::fs::write(&path, "export const answer = 42;").unwrap();
        let info = inspect(&path).unwrap();
        let mut record = crate::visuals::tests::record();
        record.target.volume = dir.path().display().to_string();
        record.target.identity.serial = Some(Serial {
            source: "ntfs64".into(),
            value: format!("{:x}", info.volume64.unwrap()),
        });
        let file = &mut record.target.file;
        file.relative = "test.ts".encode_utf16().collect();
        file.identity = Some(ExpectedIdentity::Ntfs {
            frn: info.identity.file,
        });
        file.size = info.size;
        file.modified = info.modified;
        let result = analyse(
            &record,
            Path::new("unused"),
            false,
            async |action| match action {
                Action::VideoAnalysis { result: None, .. } => Ok(Response::VideoAnalysis {
                    file: Box::new(record.target.file.clone()),
                    finding: None,
                }),
                Action::VideoAnalysis {
                    result: Some(Other),
                    ..
                } => Ok(Response::AnalysisOffline),
                _ => panic!("unexpected analysis request"),
            },
        )
        .await;
        assert!(
            matches!(result, Ok(Analysis::Offline)),
            "{:?}",
            result.err()
        );
    }
}
