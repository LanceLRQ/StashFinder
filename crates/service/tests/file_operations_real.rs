use anyhow::{Context, Result, ensure};
use stashfinder_core::{Action, Response, browse::*, file_operations::*};
use stashfinder_windows::{
    file_operations::{Edit, execute, verify},
    ipc,
};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn request(rt: &tokio::runtime::Runtime, action: Action) -> Result<Response> {
    rt.block_on(ipc::request(ipc::DEFAULT_PIPE, action))
}

#[test]
#[ignore = "writes only new samples on the explicitly authorized M6 NTFS VHD"]
fn registered_failures_preserve_replacements_and_report_partial_results() {
    use std::os::windows::fs::OpenOptionsExt;
    let manifest = PathBuf::from(std::env::var_os("SF_M6_MANIFEST").unwrap());
    assert!(manifest.starts_with(r"D:\tmp"));
    assert_eq!(manifest.file_name().unwrap(), "verification.json");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap();
    let image = PathBuf::from(saved["image"].as_str().unwrap());
    assert_eq!(image.parent(), manifest.parent());
    assert_eq!(image.file_name().unwrap(), "file-operations.vhdx");
    assert_eq!(saved["cleaned"], false);
    let fixture = saved["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["filesystem"] == "NTFS")
        .unwrap();
    let partition = fixture["id"].as_str().unwrap();
    let root = PathBuf::from(fixture["root"].as_str().unwrap());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let snapshot = rt.block_on(ipc::snapshot(ipc::DEFAULT_PIPE)).unwrap();
    let asset = snapshot
        .assets
        .iter()
        .find(|a| a.registration.partition_id == partition)
        .unwrap();
    assert_eq!(asset.registration.name, "M6 NTFS");
    assert_eq!(asset.connection, "online");
    assert_eq!(asset.mounts, vec![root.to_string_lossy().to_string()]);
    let fs = &asset.registration.filesystem_id;
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let source = format!("failures-source-{suffix}");
    let destination = format!("failures-target-{suffix}");
    let from = root.join("F1").join(&source);
    let to = root.join("F1").join(&destination);
    std::fs::create_dir(&from).unwrap();
    std::fs::create_dir(&to).unwrap();
    for name in [
        "copy1.txt",
        "copy2.txt",
        "move.txt",
        "case.txt",
        "delete.txt",
    ] {
        std::fs::write(from.join(name), format!("Original {name}")).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(45);
    let (rows, target) = loop {
        match prepare(&rt, partition, fs, &source, &destination) {
            Ok(value) => break value,
            Err(error) => {
                assert!(Instant::now() < deadline, "{error:#}");
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    };
    let selected = |name: &str| {
        rows.iter()
            .find(|r| String::from_utf16_lossy(&r.file.relative).ends_with(name))
            .unwrap()
            .clone()
    };
    let copy = selected("copy1.txt");
    let path = verify(&copy, false).unwrap();
    let cancelled = execute(
        std::slice::from_ref(&path),
        &[Some(copy.clone())],
        Some(target.clone()),
        &Edit::Paste {
            folder: verify(&target, true).unwrap(),
            move_files: false,
        },
        0,
        Arc::new(AtomicBool::new(true)),
    )
    .unwrap();
    assert!(
        cancelled.cancelled
            && cancelled
                .items
                .iter()
                .all(|r| r.state == OutcomeState::NotRun)
    );
    assert!(!to.join("copy1.txt").exists());
    let case = selected("case.txt");
    assert!(
        execute(
            &[verify(&case, false).unwrap()],
            &[Some(case)],
            None,
            &Edit::Rename("delete.txt".into()),
            0,
            Arc::new(AtomicBool::new(false))
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(from.join("case.txt")).unwrap(),
        "Original case.txt"
    );
    assert_eq!(
        std::fs::read_to_string(from.join("delete.txt")).unwrap(),
        "Original delete.txt"
    );
    let locked = selected("copy2.txt");
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(from.join("copy2.txt"))
        .unwrap();
    let locked_result = execute(
        &[verify(&locked, false).unwrap()],
        &[Some(locked.clone())],
        Some(target.clone()),
        &Edit::Paste {
            folder: verify(&target, true).unwrap(),
            move_files: false,
        },
        0,
        Arc::new(AtomicBool::new(false)),
    );
    if let Ok(report) = locked_result {
        assert!(
            !report.success(),
            "locked data was reported copied: {report:?}"
        );
    }
    assert!(!to.join("copy2.txt").exists());
    drop(held);
    assert!(verify(&locked, false).is_ok());
    let replaced = selected("move.txt");
    std::fs::rename(from.join("move.txt"), from.join("preserved.txt")).unwrap();
    std::fs::write(from.join("move.txt"), "Replacement must survive").unwrap();
    assert!(verify(&replaced, false).is_err());
    let partial = execute(
        &[path, from.join("move.txt")],
        &[Some(copy), Some(replaced)],
        Some(target.clone()),
        &Edit::Paste {
            folder: verify(&target, true).unwrap(),
            move_files: false,
        },
        0,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert!(!partial.success(), "{partial:?}");
    assert_eq!(partial.completed(), 1, "{partial:?}");
    assert_eq!(
        std::fs::read_to_string(to.join("copy1.txt")).unwrap(),
        "Original copy1.txt"
    );
    assert!(!to.join("move.txt").exists());
    assert_eq!(
        std::fs::read_to_string(from.join("move.txt")).unwrap(),
        "Replacement must survive"
    );
    assert_eq!(
        std::fs::read_to_string(from.join("preserved.txt")).unwrap(),
        "Original move.txt"
    );
    std::fs::write(
        manifest
            .parent()
            .unwrap()
            .join("file-operation-failures.json"),
        serde_json::to_vec_pretty(&partial).unwrap(),
    )
    .unwrap();
    println!(
        "cancel-before-start, rename collision, sharing denial, source replacement and actual partial copy verified"
    );
}
fn directory(
    rt: &tokio::runtime::Runtime,
    partition: &str,
    filesystem: &str,
    parent: Option<Vec<u8>>,
) -> Result<DirectoryPage> {
    let Response::Directory(page) = request(
        rt,
        Action::Directory(DirectoryRequest {
            partition_id: partition.into(),
            filesystem_id: filesystem.into(),
            generation: None,
            revision: None,
            parent,
            offset: 0,
            folders_only: false,
        }),
    )?
    else {
        anyhow::bail!("unexpected directory response")
    };
    Ok(*page)
}
fn prepare(
    rt: &tokio::runtime::Runtime,
    partition: &str,
    filesystem: &str,
    source: &str,
    destination: &str,
) -> Result<(Vec<PreparedTarget>, PreparedTarget)> {
    let root = directory(rt, partition, filesystem, None)?;
    let f1 = root
        .entries
        .iter()
        .find(|r| r.name == "F1")
        .context("F1 not indexed")?;
    let page = directory(rt, partition, filesystem, Some(f1.entry.clone()))?;
    let src = page
        .entries
        .iter()
        .find(|r| r.name == source)
        .context("probe source not indexed")?;
    let dst = page
        .entries
        .iter()
        .find(|r| r.name == destination)
        .context("probe destination not indexed")?;
    let context = EntryRef {
        partition_id: partition.into(),
        filesystem_id: filesystem.into(),
        generation: page.generation,
        revision: page.revision,
        entry: src.entry.clone(),
    };
    let Response::SelectionStarted(id) = request(
        rt,
        Action::FileSelectionBegin(SelectionOrigin::Directory {
            context: context.clone(),
            all: true,
        }),
    )?
    else {
        anyhow::bail!("selection unavailable")
    };
    let result = (|| -> Result<_> {
        let Response::SelectionCount(5) = request(rt, Action::FileSelectionSeal { id })? else {
            anyhow::bail!("waiting for complete fixture")
        };
        let Response::SelectionPage(rows) =
            request(rt, Action::FileSelectionPage { id, offset: 0 })?
        else {
            anyhow::bail!("selection unresolved")
        };
        ensure!(
            rows.rows.len() == 5 && rows.next.is_none(),
            "incomplete selection"
        );
        let Response::FileTarget(target) = request(
            rt,
            Action::FileDestination(EntryRef {
                entry: dst.entry.clone(),
                ..context
            }),
        )?
        else {
            anyhow::bail!("destination unresolved")
        };
        Ok((rows.rows, *target))
    })();
    let _ = request(rt, Action::FileSelectionCancel { id });
    result
}

/// Opt-in only: run after explicit authorization of a new VHD created by the verification script.
#[test]
#[ignore = "writes only explicitly authorized M6 fixture volumes; requires SF_M6_MANIFEST and the isolated service"]
fn registered_three_filesystems_verify_copy_move_rename_and_delete() {
    let manifest = PathBuf::from(
        std::env::var_os("SF_M6_MANIFEST")
            .expect("set SF_M6_MANIFEST to the authorized verification.json"),
    );
    assert!(
        manifest.starts_with(r"D:\tmp")
            && manifest
                .file_name()
                .is_some_and(|s| s == "verification.json")
    );
    let bytes = std::fs::read(&manifest).unwrap();
    let saved: serde_json::Value =
        serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes)).unwrap();
    assert_eq!(saved["ready"], true);
    assert_eq!(saved["cleaned"], false);
    let image = PathBuf::from(saved["image"].as_str().unwrap());
    assert_eq!(image.parent(), manifest.parent());
    assert_eq!(image.file_name().unwrap(), "file-operations.vhdx");
    assert!(image.is_file());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let snapshot = rt.block_on(ipc::snapshot(ipc::DEFAULT_PIPE)).unwrap();
    assert_eq!(snapshot.assets.len(), 3);
    let fixtures = saved["fixtures"].as_array().unwrap();
    assert_eq!(fixtures.len(), 3);
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let source = format!("native-source-{suffix}");
    let destination = format!("native-target-{suffix}");
    let mut roots = Vec::new();
    let mut ids = Vec::new();
    // Validate all fixture targets before the first write.
    for fixture in fixtures {
        let partition = fixture["id"].as_str().unwrap();
        let fs = fixture["filesystem"].as_str().unwrap();
        assert!(["NTFS", "exFAT", "FAT32"].contains(&fs));
        let root = fixture["root"].as_str().unwrap();
        let asset = snapshot
            .assets
            .iter()
            .find(|a| a.registration.partition_id == partition)
            .unwrap();
        assert_eq!(asset.registration.name, format!("M6 {fs}"));
        assert_eq!(asset.connection, "online");
        assert_eq!(asset.mounts, vec![root]);
        assert!(PathBuf::from(root).join("F1/source/01-first.txt").is_file());
        roots.push(PathBuf::from(root).join("F1"));
        ids.push((
            partition.to_string(),
            asset.registration.filesystem_id.clone(),
        ));
    }
    for root in &roots {
        std::fs::create_dir(root.join(&source)).unwrap();
        std::fs::create_dir(root.join(&destination)).unwrap();
        for name in [
            "copy1.txt",
            "copy2.txt",
            "move.txt",
            "case.txt",
            "delete.txt",
        ] {
            std::fs::write(
                root.join(&source).join(name),
                format!("M6 native {suffix} / {name}"),
            )
            .unwrap();
        }
    }
    let mut prepared = Vec::new();
    for (partition, filesystem) in &ids {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            match prepare(&rt, partition, filesystem, &source, &destination) {
                Ok(value) => {
                    prepared.push(value);
                    break;
                }
                Err(error) => {
                    assert!(Instant::now() < deadline, "{error:#}");
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        }
    }
    for index in 0..3 {
        let (rows, _) = &prepared[index];
        let other = (index + 1) % 3;
        let target = &prepared[other].1;
        let target_path = verify(target, true).unwrap();
        let copy = rows
            .iter()
            .filter(|r| String::from_utf16_lossy(&r.file.relative).contains("copy"))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(copy.len(), 2);
        let paths = copy
            .iter()
            .map(|r| verify(r, false).unwrap())
            .collect::<Vec<_>>();
        let guards = copy.into_iter().map(Some).collect::<Vec<_>>();
        let report = execute(
            &paths,
            &guards,
            Some(target.clone()),
            &Edit::Paste {
                folder: target_path.clone(),
                move_files: false,
            },
            0,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert!(report.success(), "{report:?}");
        for path in paths {
            assert_eq!(
                std::fs::read(&path).unwrap(),
                std::fs::read(target_path.join(path.file_name().unwrap())).unwrap()
            );
        }
        let selected = |name: &str| {
            rows.iter()
                .find(|r| String::from_utf16_lossy(&r.file.relative).ends_with(name))
                .unwrap()
        };
        let moving = selected("move.txt");
        let from = verify(moving, false).unwrap();
        let report = execute(
            std::slice::from_ref(&from),
            &[Some(moving.clone())],
            Some(target.clone()),
            &Edit::Paste {
                folder: target_path.clone(),
                move_files: true,
            },
            0,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert!(report.success(), "{report:?}");
        assert!(!from.exists());
        assert_eq!(
            std::fs::read_to_string(target_path.join("move.txt")).unwrap(),
            format!("M6 native {suffix} / move.txt")
        );
        let renaming = selected("case.txt");
        let path = verify(renaming, false).unwrap();
        let report = execute(
            &[path],
            &[Some(renaming.clone())],
            None,
            &Edit::Rename("CASE.txt".into()),
            0,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let exact = std::fs::read_dir(roots[index].join(&source))
            .unwrap()
            .any(|r| r.unwrap().file_name() == "CASE.txt");
        if exact {
            assert!(report.success(), "{report:?}");
        } else {
            assert!(
                !report.success(),
                "case-only no-op must not claim success: {report:?}"
            );
            assert_eq!(report.items[0].state, OutcomeState::Failed);
            assert!(roots[index].join(&source).join("case.txt").is_file());
        }
        let deleting = selected("delete.txt");
        let path = verify(deleting, false).unwrap();
        let report = execute(
            std::slice::from_ref(&path),
            &[Some(deleting.clone())],
            None,
            &Edit::Delete,
            0,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert!(report.success(), "{report:?}");
        assert!(!path.exists());
        println!(
            "{}: identity, batch cross-volume copy, cross-volume move and own-sample delete verified; case-only exact={exact}, truthful outcome verified",
            fixtures[index]["filesystem"]
        );
    }
}

#[test]
#[ignore = "read-only M6 VHD must already be mounted by the authorized fixture controller"]
fn registered_readonly_destinations_are_rejected_without_writes() {
    assert_eq!(std::env::var("SF_M6_READONLY").as_deref(), Ok("1"));
    let manifest = PathBuf::from(std::env::var_os("SF_M6_MANIFEST").unwrap());
    assert!(
        manifest.starts_with(r"D:\tmp")
            && manifest
                .file_name()
                .is_some_and(|s| s == "verification.json")
    );
    let bytes = std::fs::read(&manifest).unwrap();
    let saved: serde_json::Value =
        serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes)).unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let snapshot = loop {
        let value = rt.block_on(ipc::snapshot(ipc::DEFAULT_PIPE)).unwrap();
        if !value.busy
            && value.assets.len() == 3
            && value.assets.iter().all(|a| a.connection == "online")
        {
            break value;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(250));
    };
    for fixture in saved["fixtures"].as_array().unwrap() {
        let partition = fixture["id"].as_str().unwrap();
        let asset = snapshot
            .assets
            .iter()
            .find(|a| a.registration.partition_id == partition)
            .unwrap();
        assert_eq!(
            asset.registration.name,
            format!("M6 {}", fixture["filesystem"].as_str().unwrap())
        );
        assert!(
            asset
                .mounts
                .iter()
                .any(|m| m == fixture["root"].as_str().unwrap())
        );
        let page = directory(&rt, partition, &asset.registration.filesystem_id, None).unwrap();
        let Response::FileTarget(target) = request(
            &rt,
            Action::FileDestination(EntryRef {
                partition_id: partition.into(),
                filesystem_id: page.filesystem_id,
                generation: page.generation,
                revision: page.revision,
                entry: page.location,
            }),
        )
        .unwrap() else {
            panic!("missing destination")
        };
        let error = verify(&target, true).unwrap_err();
        assert!(error.to_string().contains("只读"), "{error:#}");
        println!(
            "{}: read-only destination rejected before Shell",
            fixture["filesystem"]
        );
    }
}

#[test]
#[ignore = "creates and recycles one new sample on the explicitly authorized NTFS fixture for manual Explorer restoration"]
fn registered_ntfs_recycle_sample_for_explorer_restore() {
    let manifest = PathBuf::from(std::env::var_os("SF_M6_MANIFEST").unwrap());
    assert!(
        manifest.starts_with(r"D:\tmp")
            && manifest
                .file_name()
                .is_some_and(|s| s == "verification.json")
    );
    let bytes = std::fs::read(&manifest).unwrap();
    let saved: serde_json::Value =
        serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes)).unwrap();
    assert_eq!(saved["ready"], true);
    assert_eq!(saved["cleaned"], false);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let snapshot = rt.block_on(ipc::snapshot(ipc::DEFAULT_PIPE)).unwrap();
    assert_eq!(snapshot.assets.len(), 3);
    let fixture = saved["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["filesystem"] == "NTFS")
        .unwrap();
    let asset = snapshot
        .assets
        .iter()
        .find(|a| a.registration.partition_id == fixture["id"].as_str().unwrap())
        .unwrap();
    assert_eq!(asset.registration.name, "M6 NTFS");
    assert_eq!(asset.connection, "online");
    assert_eq!(asset.mounts, vec![fixture["root"].as_str().unwrap()]);
    let name = format!(
        "native-recycle-{}.txt",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );
    let source = PathBuf::from(fixture["root"].as_str().unwrap())
        .join("F1/source")
        .join(&name);
    std::fs::write(&source, b"M6 recycle and restore sample").unwrap();
    let partition = &asset.registration.partition_id;
    let filesystem = &asset.registration.filesystem_id;
    let deadline = Instant::now() + Duration::from_secs(60);
    let target = loop {
        let found = (|| -> Result<PreparedTarget> {
            let root = directory(&rt, partition, filesystem, None)?;
            let f1 = root
                .entries
                .iter()
                .find(|r| r.name == "F1")
                .context("F1 not indexed")?;
            let f1 = directory(&rt, partition, filesystem, Some(f1.entry.clone()))?;
            let folder = f1
                .entries
                .iter()
                .find(|r| r.name == "source")
                .context("source not indexed")?;
            let page = directory(&rt, partition, filesystem, Some(folder.entry.clone()))?;
            let entry = page
                .entries
                .iter()
                .find(|r| r.name == name)
                .context("recycle sample not indexed")?;
            let context = EntryRef {
                partition_id: partition.clone(),
                filesystem_id: filesystem.clone(),
                generation: page.generation,
                revision: page.revision,
                entry: page.location,
            };
            let Response::SelectionStarted(id) = request(
                &rt,
                Action::FileSelectionBegin(SelectionOrigin::Directory {
                    context,
                    all: false,
                }),
            )?
            else {
                anyhow::bail!("selection missing")
            };
            let result = (|| -> Result<PreparedTarget> {
                request(
                    &rt,
                    Action::FileSelectionAdd {
                        id,
                        chunk: SelectionChunk::Entries(vec![entry.entry.clone()]),
                    },
                )?;
                let Response::SelectionCount(1) = request(&rt, Action::FileSelectionSeal { id })?
                else {
                    anyhow::bail!("wrong count")
                };
                let Response::SelectionPage(mut page) =
                    request(&rt, Action::FileSelectionPage { id, offset: 0 })?
                else {
                    anyhow::bail!("missing target")
                };
                page.rows.pop().context("empty selection")
            })();
            let _ = request(&rt, Action::FileSelectionCancel { id });
            result
        })();
        match found {
            Ok(value) => break value,
            Err(error) => {
                assert!(Instant::now() < deadline, "{error:#}");
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    };
    let path = verify(&target, false).unwrap();
    let report = execute(
        &[path],
        &[Some(target)],
        None,
        &Edit::Recycle,
        0,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert!(report.success(), "{report:?}");
    assert_eq!(report.items[0].note.as_deref(), Some("已移入回收站"));
    assert!(!source.exists());
    let recycled = report.items[0].destination.as_ref().unwrap();
    assert_eq!(
        std::fs::read(recycled).unwrap(),
        b"M6 recycle and restore sample"
    );
    let proof =
        serde_json::json!({"source":source,"name":name,"recycled":recycled,"restored":false});
    std::fs::write(
        manifest.parent().unwrap().join("recycle-verification.json"),
        serde_json::to_vec_pretty(&proof).unwrap(),
    )
    .unwrap();
    println!("{proof}");
}
