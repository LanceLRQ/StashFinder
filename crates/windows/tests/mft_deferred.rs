#![cfg(feature = "mft-prototype")]

#[path = "support/mft.rs"]
mod fixture;
use fixture::*;
use stashfinder_windows::ntfs::mft::{Geometry, scan_mft, scan_mft_deferred};
use std::sync::atomic::AtomicBool;

#[test]
fn captured_raw_input_roundtrips_and_rejects_corruption_and_missing_ranges() {
    use stashfinder_windows::ntfs::mft::{ReadAt, capture::Capture};
    use std::io::{Seek, SeekFrom, Write};
    let (geometry, mut disk) = fragmented_disk();
    let cancel = AtomicBool::new(false);
    let (capture, _) = Capture::record(&mut disk, geometry, 0x1000000000005, &cancel).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("raw");
    capture.save(&path).unwrap();
    assert!(capture.save(&path).is_err());
    let mut loaded = Capture::load(&path).unwrap();
    assert!(loaded.read_exact_at(0, &mut [0; 512]).is_err());
    let stats = scan_mft_deferred(
        &mut loaded,
        &geometry,
        0x1000000000005,
        &cancel,
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!((stats.objects, stats.links), (3, 4));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(path.join("data.bin"))
        .unwrap();
    file.seek(SeekFrom::Start(8)).unwrap();
    file.write_all(&[0xaa]).unwrap();
    assert!(Capture::load(&path).is_err());
}

#[test]
fn capture_refuses_inconsistent_repeated_reads() {
    use stashfinder_windows::ntfs::mft::{ReadAt, capture::Capture};
    struct Changing {
        disk: MemoryDisk,
        mapping_reads: usize,
    }
    impl ReadAt for Changing {
        fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> anyhow::Result<()> {
            self.disk.read_exact_at(offset, out)?;
            if offset == 160 * 512 && out.len() == 1024 {
                self.mapping_reads += 1;
                if self.mapping_reads > 1 {
                    out[100] ^= 1;
                }
            }
            Ok(())
        }
    }
    let (geometry, disk) = fragmented_disk();
    let error = Capture::record(
        &mut Changing {
            disk,
            mapping_reads: 0,
        },
        geometry,
        0x1000000000005,
        &AtomicBool::new(false),
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("捕获期间元数据发生变化"));
}

#[test]
fn deferred_scan_matches_immediate_scan_and_accounts_for_every_read() {
    let (geometry, mut disk) = fragmented_disk();
    let mut expected = Vec::new();
    let cancel = AtomicBool::new(false);
    let before = scan_mft(
        &mut disk,
        &geometry,
        0x1000000000005,
        &cancel,
        &mut |rows| {
            expected.extend(rows.iter().map(|r| {
                (
                    r.entry.id,
                    r.entry.size,
                    r.entry.created,
                    r.entry.modified,
                    r.entry.attributes,
                    r.links.clone(),
                )
            }));
            Ok(())
        },
    )
    .unwrap();
    let mut actual = Vec::new();
    let after = scan_mft_deferred(
        &mut disk,
        &geometry,
        0x1000000000005,
        &cancel,
        &mut |rows| {
            actual.extend(rows.iter().map(|r| {
                (
                    r.entry.id,
                    r.entry.size,
                    r.entry.created,
                    r.entry.modified,
                    r.entry.attributes,
                    r.links.clone(),
                )
            }));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(after.extension_io.calls, 0);
    assert_eq!(after.retained_extensions, 2);
    for stats in [before, after] {
        let classes = [
            stats.sequential_io,
            stats.extension_io,
            stats.attribute_list_io,
            stats.mapping_io,
            stats.bootstrap_io,
        ];
        assert_eq!(
            classes.iter().map(|s| s.calls).sum::<u64>(),
            stats.read_calls
        );
        assert_eq!(
            classes.iter().map(|s| s.bytes).sum::<u64>(),
            stats.read_bytes
        );
        assert_eq!(
            classes
                .iter()
                .map(|s| s.elapsed)
                .sum::<std::time::Duration>(),
            stats.read_time
        );
    }
}

#[test]
fn deferred_scan_rejects_stale_missing_and_wrong_base_extensions() {
    for mutation in 0..4 {
        let (geometry, mut disk) = fragmented_disk();
        let at = 160 * 512 + 20 * 1024;
        match mutation {
            0 => put16(&mut disk.bytes, at + 16, 9),
            1 => put16(&mut disk.bytes, at + 22, 0),
            2 => put64(&mut disk.bytes, at + 32, 0x9000000000012),
            _ => {
                disk.bytes[at + 510] ^= 1;
            }
        }
        assert!(
            scan_mft_deferred(
                &mut disk,
                &geometry,
                0x1000000000005,
                &AtomicBool::new(false),
                &mut |_| Ok(())
            )
            .is_err()
        );
    }
    let (geometry, mut disk) = fragmented_disk();
    assert!(
        scan_mft_deferred(
            &mut disk,
            &geometry,
            0x1000000000005,
            &AtomicBool::new(true),
            &mut |_| panic!("cancelled")
        )
        .is_err()
    );
    assert!(disk.reads.is_empty());
}

#[test]
fn deferred_assembly_removes_far_extension_reads_on_identical_captured_bytes() {
    use stashfinder_windows::ntfs::mft::capture::Capture;
    let length = 6 * 1024 * 1024;
    let mut disk = MemoryDisk {
        bytes: vec![0; length + 8 * 512],
        reads: Vec::new(),
    };
    let root = 0x1000000000005;
    let base = 0x2000000000012;
    let first = 0x1000000000000 + 1100;
    let last = 0x1000000000000 + 5800;
    let list = [
        list_entry(0x10, 0, base, 0),
        list_entry(0x30, 1, first, 0),
        list_entry(0x30, 2, last, 0),
        list_entry(0x80, 3, last, 0),
    ]
    .concat();
    for (slot, bytes) in [
        (
            0,
            record(
                1024,
                1,
                1,
                0,
                &[nonresident(
                    0x80,
                    1,
                    0,
                    0,
                    length as u64,
                    12288,
                    &[0x12, 0, 0x30, 8, 0],
                )],
            ),
        ),
        (
            5,
            record(
                1024,
                1,
                3,
                0,
                &[
                    resident(0x10, 0, &[], &[0; 48]),
                    filename(1, root, 1, &[46]),
                ],
            ),
        ),
        (
            18,
            record(
                1024,
                2,
                1,
                0,
                &[
                    resident(0x10, 0, &[], &[0; 48]),
                    resident(0x20, 4, &[], &list),
                ],
            ),
        ),
        (
            1100,
            record(1024, 1, 1, base, &[filename(1, root, 1, &[65])]),
        ),
        (
            5800,
            record(
                1024,
                1,
                1,
                base,
                &[filename(2, root, 1, &[66]), resident(0x80, 3, &[], &[0; 3])],
            ),
        ),
    ] {
        let offset = 8 * 512 + slot * 1024;
        disk.bytes[offset..offset + 1024].copy_from_slice(&bytes);
    }
    let geometry = Geometry::new(512, 512, 1024, 12296, 8, length as u64).unwrap();

    let cancel = AtomicBool::new(false);
    let (mut image, _) = Capture::record(&mut disk, geometry, root, &cancel).unwrap();
    let mut expected = Vec::new();
    let before = scan_mft(&mut image, &geometry, root, &cancel, &mut |rows| {
        expected.extend(
            rows.iter()
                .map(|r| (r.entry.id, r.entry.size, r.links.clone())),
        );
        Ok(())
    })
    .unwrap();
    let mut actual = Vec::new();
    let after = scan_mft_deferred(&mut image, &geometry, root, &cancel, &mut |rows| {
        actual.extend(
            rows.iter()
                .map(|r| (r.entry.id, r.entry.size, r.links.clone())),
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(actual, expected);
    assert!(before.extension_io.calls > 0);
    assert_eq!(after.extension_io.calls, 0);
    assert_eq!(after.reread_mft_bytes, 0);
    assert!(after.read_calls < before.read_calls);
    assert!(after.read_bytes < before.read_bytes);
    println!(
        "raw fixture: calls {} -> {}, bytes {} -> {}, extension reads {} -> {}",
        before.read_calls,
        after.read_calls,
        before.read_bytes,
        after.read_bytes,
        before.extension_io.calls,
        after.extension_io.calls
    );
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("raw");
    image.save(&path).unwrap();
    let mut loaded = Capture::load(&path).unwrap();
    let saved = scan_mft_deferred(&mut loaded, &geometry, root, &cancel, &mut |_| Ok(())).unwrap();
    assert_eq!(saved.objects, after.objects);
    assert_eq!(saved.links, after.links);
}

#[test]
fn deferred_external_attribute_lists_are_captured_and_timed_separately() {
    use stashfinder_windows::ntfs::mft::capture::Capture;
    let (geometry, mut disk) = fragmented_disk();
    let base = 0x2000000000012;
    let list = [
        list_entry(0x10, 0, base, 0),
        list_entry(0x30, 2, 0x1000000000010, 0),
        list_entry(0x30, 3, 0x1000000000014, 0),
        list_entry(0x80, 4, 0x1000000000014, 0),
    ]
    .concat();
    disk.bytes[400 * 512..400 * 512 + list.len()].copy_from_slice(&list);
    let bytes = record(
        1024,
        2,
        1,
        0,
        &[
            resident(0x10, 0, &[], &[0; 48]),
            nonresident(0x20, 1, 0, 0, list.len() as u64, 1, &[0x21, 1, 144, 1, 0]),
        ],
    );
    disk.bytes[160 * 512 + 18 * 1024..160 * 512 + 19 * 1024].copy_from_slice(&bytes);

    let cancel = AtomicBool::new(false);
    let (mut image, before) =
        Capture::record(&mut disk, geometry, 0x1000000000005, &cancel).unwrap();
    assert_eq!(before.attribute_list_io.calls, 1);
    assert_eq!(before.attribute_list_io.bytes, list.len() as u64);
    let after = scan_mft_deferred(&mut image, &geometry, 0x1000000000005, &cancel, &mut |_| {
        Ok(())
    })
    .unwrap();
    assert_eq!(after.objects, 3);
    assert_eq!(after.attribute_list_io.calls, 1);
    assert_eq!(after.attribute_list_io.bytes, list.len() as u64);
}

#[test]
fn capture_preserves_bootstrap_identity_and_replays_ioctl_results() {
    use stashfinder_windows::ntfs::mft::{ReadAt, capture::Capture};
    use std::sync::atomic::AtomicBool;

    struct BootstrapDisk(MemoryDisk, Vec<u8>, bool);
    impl ReadAt for BootstrapDisk {
        fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> anyhow::Result<()> {
            self.0.read_exact_at(offset, out)
        }
        fn bootstrap_record(
            &mut self,
            reference: u64,
            out: &mut [u8],
        ) -> anyhow::Result<Option<u64>> {
            assert_eq!(reference, 0x1000000000046);
            out.copy_from_slice(&self.1);
            Ok(Some(reference - u64::from(self.2)))
        }
    }
    for wrong_slot in [true, false] {
        let (geometry, mut disk) = fragmented_disk();
        let base = 0x1000000000000;
        let list = [
            list_entry(0x80, 1, base, 0),
            list_entry(0x80, 2, 0x1000000000046, 131),
        ]
        .concat();
        let first = record(
            1024,
            1,
            1,
            0,
            &[
                nonresident(0x80, 1, 0, 0, 90 * 1024, 131, &[0x21, 131, 160, 0, 0]),
                resident(0x20, 3, &[], &list),
            ],
        );
        disk.bytes[160 * 512..160 * 512 + 1024].copy_from_slice(&first);
        let extent = record(
            1024,
            1,
            1,
            base,
            &[nonresident(0x80, 2, 131, 0, 0, 49, &[0x11, 49, 8, 0])],
        );
        let mut disk = BootstrapDisk(disk, extent, wrong_slot);
        let result = Capture::record(
            &mut disk,
            geometry,
            0x1000000000005,
            &AtomicBool::new(false),
        );
        if wrong_slot {
            assert!(result.err().unwrap().to_string().contains("返回不同记录"));
        } else {
            let (image, stats) = result.unwrap();
            assert_eq!(stats.bootstrap_io.calls, 2);
            assert_eq!(stats.bootstrap_io.bytes, 2048);
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join("raw");
            image.save(&path).unwrap();
            let mut loaded = Capture::load(&path).unwrap();
            let replay = scan_mft_deferred(
                &mut loaded,
                &geometry,
                0x1000000000005,
                &AtomicBool::new(false),
                &mut |_| Ok(()),
            )
            .unwrap();
            assert_eq!(replay.objects, 3);
            assert_eq!(replay.bootstrap_io.calls, 2);
        }
    }
}
