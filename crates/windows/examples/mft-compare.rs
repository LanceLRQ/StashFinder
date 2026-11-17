//! Capture raw metadata once, then compare both scanners on exactly those bytes.
use anyhow::{Result, ensure};
use stashfinder_windows::ntfs::{
    NativeObject, VolumeReader,
    mft::{capture::Capture, scan_mft, scan_mft_deferred},
};
use std::{path::Path, sync::atomic::AtomicBool, time::Instant};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let cancel = AtomicBool::new(false);
    let mut image = match args.as_slice() {
        [mode, volume, output] if mode == "capture" => {
            ensure!(!Path::new(output).exists(), "捕获目录已存在");
            stashfinder_windows::portable::enable_backup_read()?;
            let reader = VolumeReader::open(volume)?;
            let start = Instant::now();
            let (image, stats) = reader.capture_mft(&cancel)?;
            println!(
                "{}",
                serde_json::json!({"event":"capture","elapsed_ms":start.elapsed().as_millis(),"stats":stats})
            );
            image.save(Path::new(output))?;
            image
        }
        [mode, input] if mode == "replay" => Capture::load(Path::new(input))?,
        _ => anyhow::bail!(
            "Usage: mft-compare capture <volume GUID> <new directory> | replay <capture directory>"
        ),
    };
    let geometry = image.geometry;
    let root = image.root;
    let mut reference: Option<Vec<NativeObject>> = None;
    for round in 1..=3 {
        for deferred in if round == 2 {
            [true, false]
        } else {
            [false, true]
        } {
            let mut objects = Vec::new();
            let start = Instant::now();
            let scan = if deferred {
                scan_mft_deferred
            } else {
                scan_mft
            };
            let stats = scan(&mut image, &geometry, root, &cancel, &mut |rows| {
                objects.extend_from_slice(rows);
                Ok(())
            })?;
            let elapsed = start.elapsed();
            objects.sort_unstable_by_key(|row| row.entry.id);
            if let Some(reference) = &reference {
                ensure!(reference.len() == objects.len(), "扫描对象数量不一致");
                for (a, b) in reference.iter().zip(&objects) {
                    ensure!(
                        a.entry.id == b.entry.id
                            && a.entry.name == b.entry.name
                            && a.entry.attributes == b.entry.attributes
                            && a.entry.size == b.entry.size
                            && a.entry.created == b.entry.created
                            && a.entry.modified == b.entry.modified
                            && a.links == b.links,
                        "扫描结果不一致: {}",
                        a.entry.id
                    );
                }
            } else {
                reference = Some(objects);
            }
            println!(
                "{}",
                serde_json::json!({"event":"raw_replay","round":round,
                "mode":if deferred {"deferred"} else {"immediate"},"elapsed_ms":elapsed.as_secs_f64()*1000.0,
                "equal":true,"input":"captured_memory","stats":stats})
            );
        }
    }
    Ok(())
}
