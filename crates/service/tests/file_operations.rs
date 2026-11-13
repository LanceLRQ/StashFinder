use stashfinder_core::{assets::*, file_operations::*};
use stashfinder_service::jobs::Indexes;
use stashfinder_store::{catalog::Catalog, volume::*};

fn fixture() -> (tempfile::TempDir, Indexes, Snapshot, EntryRef) {
    fixture_with(4, false)
}

#[test]
fn visual_inspection_keeps_offline_identity_and_does_not_replace_an_operation_selection() {
    let (_dir, indexes, snapshot, context) = fixture();
    let selection = indexes
        .selection_begin(
            SelectionOrigin::Directory {
                context: context.clone(),
                all: false,
                filter: None,
            },
            &snapshot,
        )
        .unwrap();
    let mut file = context.clone();
    file.entry = 2u32.to_le_bytes().to_vec();
    indexes
        .selection_add(selection, SelectionChunk::Entries(vec![file.entry.clone()]))
        .unwrap();
    assert_eq!(indexes.selection_seal(selection).unwrap(), 1);
    assert!(indexes.selection_inspect(&file, &snapshot).unwrap().online);
    let mut reconnected = snapshot.clone();
    reconnected.assets[0].registration.observation.candidate_id = "current-volume".into();
    assert_eq!(
        indexes
            .selection_inspect(&file, &reconnected)
            .unwrap()
            .target
            .volume,
        "current-volume"
    );
    let mut offline = snapshot.clone();
    offline.assets[0].connection = "offline".into();
    offline.assets[0].mounts.clear();
    let record = indexes.selection_inspect(&file, &offline).unwrap();
    assert!(!record.online);
    assert_eq!(record.target.file.reference.entry, file.entry);
    assert!(matches!(
        record.target.file.identity,
        Some(ExpectedIdentity::Native(_))
    ));
    assert!(record.target.mounts.is_empty());
    assert_eq!(
        indexes
            .selection_page(selection, 0, &snapshot)
            .unwrap()
            .total,
        1
    );
    offline.assets[0].connection = "historical".into();
    offline.assets[0].registration.state = "historical".into();
    assert!(!indexes.selection_inspect(&file, &offline).unwrap().online);
    let mut stale = file.clone();
    stale.revision = "999999".into();
    assert!(indexes.selection_inspect(&stale, &offline).is_err());
    stale = file;
    stale.filesystem_id = "replacement".into();
    assert!(indexes.selection_inspect(&stale, &offline).is_err());
}
fn fixture_with(count: u32, long: bool) -> (tempfile::TempDir, Indexes, Snapshot, EntryRef) {
    let dir = tempfile::tempdir().unwrap();
    let mut catalog = Catalog::open(dir.path()).unwrap();
    let observation = Observation {
        candidate_id: r"\\?\Volume{test}\".into(),
        disk_number: None,
        model: None,
        disk_bytes: None,
        partition_bytes: None,
        volume_label: None,
        mounts: vec![],
        identity: Identity {
            layout: Layout::Gpt {
                disk: "disk".into(),
                partition: "p".into(),
            },
            filesystem: Some("exFAT".into()),
            serial: Some(Serial {
                source: "win32_volume32".into(),
                value: "10".into(),
            }),
        },
        capacity: None,
        error: None,
    };
    let registration = catalog.prepare(&observation, "Test").unwrap();
    catalog.finish(&registration.partition_id).unwrap();
    let path = catalog.volume_path(&registration.filesystem_id).unwrap();
    let mut db = Volume::open(&path).unwrap();
    let g = db.begin_generation().unwrap();
    let objects = (1..=count)
        .map(|id| Object {
            identity: Some(NativeIdentity {
                volume: 10,
                file: id as u64,
            }),
            id: id.to_le_bytes().to_vec(),
            source: "path".into(),
            kind: if id == 1 { "folder" } else { "file" }.into(),
            size: Some(0),
            created: None,
            modified: None,
            attributes: if id == 1 { 16 } else { 0 },
        })
        .collect::<Vec<_>>();
    let entries = (1..=count)
        .map(|id| Entry {
            id: id.to_le_bytes().to_vec(),
            object: id.to_le_bytes().to_vec(),
            parent: (id != 1).then_some(1u32.to_le_bytes().to_vec()),
            name: format!(
                "{id}{}.txt",
                if long {
                    "测".repeat(220)
                } else {
                    String::new()
                }
            )
            .encode_utf16()
            .collect(),
        })
        .collect::<Vec<_>>();
    db.write_batch(g, &objects, &entries, &[], None).unwrap();
    db.complete(g).unwrap();
    db.publish_snapshot(g).unwrap();
    let page = stashfinder_store::browse::read(
        &path,
        &stashfinder_core::browse::DirectoryRequest {
            partition_id: registration.partition_id.clone(),
            filesystem_id: registration.filesystem_id.clone(),
            parent: None,
            generation: None,
            revision: None,
            offset: 0,
            folders_only: false,
            sort: Default::default(),
            filter: None,
        },
    )
    .unwrap();
    let context = EntryRef {
        partition_id: registration.partition_id.clone(),
        filesystem_id: registration.filesystem_id.clone(),
        generation: page.generation,
        revision: page.revision,
        entry: 1u32.to_le_bytes().to_vec(),
    };
    let snapshot = Snapshot {
        assets: vec![Asset {
            registration: catalog.get(&registration.partition_id).unwrap(),
            connection: "online".into(),
            mounts: vec!["X:\\".into()],
            error: None,
            capacity_error: None,
        }],
        ..Default::default()
    };
    let jobs = Indexes::start(dir.path().into());
    (dir, jobs, snapshot, context)
}
#[test]
fn an_existing_snapshot_remains_usable_during_background_asset_refresh() {
    let (_dir, jobs, mut snapshot, context) = fixture();
    snapshot.busy = true;
    let id = jobs
        .selection_begin(
            SelectionOrigin::Directory {
                context: context.clone(),
                all: true,
                filter: None,
            },
            &snapshot,
        )
        .unwrap();
    assert_eq!(jobs.selection_seal(id).unwrap(), 3);
    assert_eq!(jobs.selection_page(id, 0, &snapshot).unwrap().rows.len(), 3);
    assert!(
        jobs.selection_destination(&context, &snapshot)
            .unwrap()
            .file
            .folder
    );
    snapshot.assets.clear();
    assert!(jobs.selection_page(id, 0, &snapshot).is_err());
    assert!(jobs.selection_destination(&context, &snapshot).is_err());
    assert!(
        jobs.selection_begin(
            SelectionOrigin::Directory {
                context,
                all: true,
                filter: None,
            },
            &snapshot
        )
        .is_err()
    );
}

#[test]
fn directory_selection_includes_unloaded_rows_and_preparation_can_be_cancelled() {
    let (_dir, jobs, snapshot, context) = fixture();
    let id = jobs
        .selection_begin(
            SelectionOrigin::Directory {
                context,
                all: true,
                filter: None,
            },
            &snapshot,
        )
        .unwrap();
    jobs.selection_add(
        id,
        SelectionChunk::Entries(vec![3u32.to_le_bytes().to_vec()]),
    )
    .unwrap();
    assert_eq!(jobs.selection_seal(id).unwrap(), 2);
    let page = jobs.selection_page(id, 0, &snapshot).unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.rows.len(), 2);
    assert_eq!(
        page.rows[0].file.relative,
        "2.txt".encode_utf16().collect::<Vec<_>>()
    );
    jobs.selection_cancel(id);
    assert!(jobs.selection_page(id, 0, &snapshot).is_err());
}
#[test]
fn preflight_rejects_offline_identity_change_and_stale_directory_versions() {
    let (_dir, jobs, snapshot, context) = fixture();
    let id = jobs
        .selection_begin(
            SelectionOrigin::Directory {
                context: context.clone(),
                all: false,
                filter: None,
            },
            &snapshot,
        )
        .unwrap();
    jobs.selection_add(
        id,
        SelectionChunk::Entries(vec![2u32.to_le_bytes().to_vec()]),
    )
    .unwrap();
    jobs.selection_seal(id).unwrap();
    let mut offline = snapshot.clone();
    offline.assets[0].connection = "offline".into();
    assert!(jobs.selection_page(id, 0, &offline).is_err());
    let mut historical = snapshot.clone();
    historical.assets[0].registration.state = "historical".into();
    assert!(jobs.selection_page(id, 0, &historical).is_err());
    let mut bad = context;
    bad.revision = "999".into();
    assert!(
        jobs.selection_begin(
            SelectionOrigin::Directory {
                context: bad,
                all: true,
                filter: None,
            },
            &snapshot
        )
        .is_err()
    );
}

#[test]
fn search_range_uses_all_positions_and_survives_query_replacement() {
    let (_dir, jobs, snapshot, _context) = fixture();
    let search = jobs
        .search(
            stashfinder_core::index::SearchRequest {
                replaces: None,
                text: "file:".into(),
                names_only: false,
                partitions: vec![],
                options: Default::default(),
            },
            &snapshot,
        )
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !jobs.batch(search, 0, 1).unwrap().done {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let selection = jobs
        .selection_begin(
            SelectionOrigin::Search {
                id: search,
                range: Some([0, 2]),
            },
            &snapshot,
        )
        .unwrap();
    jobs.selection_add(selection, SelectionChunk::Positions(vec![1]))
        .unwrap();
    assert_eq!(jobs.selection_seal(selection).unwrap(), 2);
    jobs.cancel_search(search);
    jobs.search(
        stashfinder_core::index::SearchRequest {
            replaces: None,
            text: "missing".into(),
            names_only: false,
            partitions: vec![],
            options: Default::default(),
        },
        &snapshot,
    )
    .unwrap();
    let page = jobs.selection_page(selection, 0, &snapshot).unwrap();
    assert_eq!(page.rows.len(), 2);
    assert_eq!(
        page.rows[0].file.relative,
        "2.txt".encode_utf16().collect::<Vec<_>>()
    );
    assert_eq!(
        page.rows[1].file.relative,
        "4.txt".encode_utf16().collect::<Vec<_>>()
    );
    assert!(
        jobs.selection_add(selection, SelectionChunk::Positions(vec![0]))
            .is_err()
    );
    assert!(jobs.selection_page(selection, 3, &snapshot).is_err());
}

#[test]
fn offline_text_records_use_complete_selection_without_source_access() {
    let (_dir, jobs, mut snapshot, context) = fixture();
    snapshot.assets[0].connection = "offline".into();
    snapshot.assets[0].mounts.clear();
    let id = jobs
        .selection_begin_records(
            SelectionOrigin::Directory {
                context,
                all: true,
                filter: None,
            },
            &snapshot,
        )
        .unwrap();
    assert_eq!(jobs.selection_seal(id).unwrap(), 3);
    let page = jobs.selection_page(id, 0, &snapshot).unwrap();
    assert_eq!(page.rows.len(), 3);
    assert!(page.rows.iter().all(|r| r.mounts.is_empty()));
}

#[test]
fn oversized_selection_is_rejected_instead_of_truncated() {
    let (_dir, jobs, snapshot, context) = fixture_with(10_002, false);
    let id = jobs
        .selection_begin(
            SelectionOrigin::Directory {
                context,
                all: true,
                filter: None,
            },
            &snapshot,
        )
        .unwrap();
    assert!(jobs.selection_seal(id).is_err());
    assert!(jobs.selection_page(id, 0, &snapshot).is_err());
}

#[test]
fn selection_pages_are_bounded_by_encoded_bytes_without_losing_rows() {
    let (_dir, jobs, snapshot, context) = fixture_with(65, true);
    let id = jobs
        .selection_begin(
            SelectionOrigin::Directory {
                context,
                all: true,
                filter: None,
            },
            &snapshot,
        )
        .unwrap();
    assert_eq!(jobs.selection_seal(id).unwrap(), 64);
    let mut offset = 0;
    let mut refs = std::collections::HashSet::new();
    loop {
        let page = jobs.selection_page(id, offset, &snapshot).unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() < stashfinder_core::MAX_FRAME - 128);
        assert!(page.rows.len() < 32);
        for row in page.rows {
            assert!(refs.insert(row.file.reference.entry));
        }
        match page.next {
            Some(next) => {
                assert!(next > offset);
                offset = next;
            }
            None => break,
        }
    }
    assert_eq!(refs.len(), 64);
}

#[test]
fn rejected_selection_chunk_does_not_apply_partial_toggles() {
    let (_dir, jobs, snapshot, context) = fixture();
    let id = jobs
        .selection_begin(
            SelectionOrigin::Directory {
                context,
                all: false,
                filter: None,
            },
            &snapshot,
        )
        .unwrap();
    assert!(
        jobs.selection_add(
            id,
            SelectionChunk::Entries(vec![2u32.to_le_bytes().to_vec(), vec![]])
        )
        .is_err()
    );
    jobs.selection_add(
        id,
        SelectionChunk::Entries(vec![3u32.to_le_bytes().to_vec()]),
    )
    .unwrap();
    assert_eq!(jobs.selection_seal(id).unwrap(), 1);
}
