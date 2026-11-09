use stashfinder_windows::ntfs::mft::{AttributeValue, Geometry, decode_record, decode_runs};

#[path = "support/mft.rs"]
mod fixture;

use fixture::*;

#[test]
fn volume_geometry_requires_supported_version_and_bounded_lengths() {
    let mut data = vec![0; 104];
    put64(&mut data, 16, 1024);
    put32(&mut data, 40, 512);
    put32(&mut data, 44, 4096);
    put32(&mut data, 48, 1024);
    put64(&mut data, 56, 32768);
    put64(&mut data, 64, 2);
    put32(&mut data, 96, 8);
    put16(&mut data, 100, 3);
    put16(&mut data, 102, 1);
    assert_eq!(
        Geometry::from_volume_data(&data).unwrap(),
        Some(Geometry::new(512, 4096, 1024, 1024, 2, 32768).unwrap())
    );
    assert!(Geometry::from_volume_data(&data[..96]).unwrap().is_none());
    put16(&mut data, 100, 4);
    assert!(Geometry::from_volume_data(&data).unwrap().is_none());
    put16(&mut data, 100, 3);
    put64(&mut data, 64, 1024);
    assert!(Geometry::from_volume_data(&data).is_err());
}

#[test]
fn fragmented_mft_emits_base_objects_once_with_earlier_and_later_extensions() {
    use stashfinder_windows::ntfs::mft::scan_mft;
    use std::sync::atomic::AtomicBool;

    let (geometry, mut disk) = fragmented_disk();
    let mut items = Vec::new();
    let stats = scan_mft(
        &mut disk,
        &geometry,
        0x1000000000005,
        &AtomicBool::new(false),
        &mut |batch| {
            items.extend(
                batch
                    .iter()
                    .map(|object| (object.entry.id, object.entry.size, object.links.clone())),
            );
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].0, 0x1000000000005);
    assert_eq!(
        items[1],
        (
            0x2000000000012,
            3,
            vec![(0x1000000000005, vec![65]), (0x1000000000005, vec![66])]
        )
    );
    assert_eq!(items[2].0, 0x3000000000041);
    assert!(
        disk.reads
            .iter()
            .any(|&(offset, count)| offset == 8 * 512 && count > 1024)
    );
    assert_eq!(stats.objects, 3);
    assert_eq!(stats.auxiliary_reads, 2);
}

#[test]
fn mft_scan_propagates_consumer_errors_and_cancellation() {
    use stashfinder_windows::ntfs::mft::scan_mft;
    use std::sync::atomic::AtomicBool;

    let (geometry, mut disk) = fragmented_disk();
    let error = scan_mft(
        &mut disk,
        &geometry,
        0x1000000000005,
        &AtomicBool::new(false),
        &mut |_| Err(std::io::Error::other("write failure").into()),
    )
    .unwrap_err();
    assert!(error.is::<std::io::Error>());
    disk.reads.clear();
    assert!(
        scan_mft(
            &mut disk,
            &geometry,
            0x1000000000005,
            &AtomicBool::new(true),
            &mut |_| panic!("cancelled scan emitted data")
        )
        .is_err()
    );
    assert!(disk.reads.is_empty());
}

#[test]
fn mft_bootstrap_checks_returned_slot_before_parsing_the_record() {
    use stashfinder_windows::ntfs::mft::{ReadAt, scan_mft};
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
        let result = scan_mft(
            &mut disk,
            &geometry,
            0x1000000000005,
            &AtomicBool::new(false),
            &mut |_| Ok(()),
        );
        if wrong_slot {
            assert!(result.unwrap_err().to_string().contains("返回不同记录"));
        } else {
            assert_eq!(result.unwrap().objects, 3);
        }
    }
}

#[test]
fn nonresident_attribute_list_resolves_extensions_and_short_reads_fail() {
    use stashfinder_windows::ntfs::mft::scan_mft;
    use std::sync::atomic::AtomicBool;
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
    assert_eq!(
        scan_mft(&mut disk, &geometry, 0x1000000000005, &cancel, &mut |_| Ok(
            ()
        ))
        .unwrap()
        .objects,
        3
    );
    disk.bytes.truncate(400 * 512 + list.len() - 1);
    assert!(
        scan_mft(&mut disk, &geometry, 0x1000000000005, &cancel, &mut |_| Ok(
            ()
        ))
        .is_err()
    );
}

#[test]
fn mapping_pairs_preserve_signed_deltas_and_sparse_runs() {
    let runs = decode_runs(&[0x11, 2, 10, 0x11, 1, 0xfd, 0x01, 3, 0], 0, 100).unwrap();
    let actual: Vec<_> = runs.iter().map(|r| (r.vcn, r.lcn, r.clusters)).collect();
    assert_eq!(actual, [(0, Some(10), 2), (2, Some(7), 1), (3, None, 3)]);
    for bytes in [
        &[0x11, 1][..],
        &[0x11, 1, 10],
        &[0x11, 0, 10, 0],
        &[0x11, 2, 99, 0],
        &[0x11, 1, 0xff, 0],
    ] {
        assert!(decode_runs(bytes, 0, 100).is_err(), "{bytes:02x?}");
    }
    assert!(decode_runs(&[0x01, 1, 0], u64::MAX, 100).is_err());
}

#[test]
fn record_uses_standard_information_and_unnamed_data_and_keeps_long_links() {
    let geometry = Geometry::new(512, 4096, 1024, 100, 2, 32768).unwrap();
    let mut standard = vec![0; 48];
    put64(&mut standard, 0, 11);
    put64(&mut standard, 8, 22);
    put32(&mut standard, 32, 0x20);
    let first_name = [0x4e2d, 0xd800];
    let mut bytes = record(
        1024,
        3,
        1,
        0,
        &[
            resident(0x10, 0, &[], &standard),
            filename(1, 0x1000000000005, 1, &first_name),
            filename(2, 0x2000000000018, 3, &[65]),
            filename(3, 0x1000000000005, 2, &[66]),
            resident(0x80, 4, &[], &[]),
            resident(0x80, 5, &[65], &[1, 2, 3]),
        ],
    );
    let record = decode_record(&mut bytes, 42, &geometry).unwrap().unwrap();
    assert_eq!(record.reference, 0x300000000002a);
    let object = record.into_object().unwrap();
    assert_eq!(
        (
            object.entry.created,
            object.entry.modified,
            object.entry.size,
            object.entry.attributes
        ),
        (11, 22, 0, 0x20)
    );
    assert_eq!(
        object.links,
        [
            (0x1000000000005, first_name.to_vec()),
            (0x2000000000018, vec![65])
        ]
    );
}

#[test]
fn record_fixups_restore_attribute_bytes_and_detect_torn_records() {
    for size in [1024, 4096] {
        let geometry = Geometry::new(512, 4096, size, 100, 2, size as u64 * 16).unwrap();
        let name = vec![0x4e2d; 240];
        let mut good = record(size, 1, 1, 0, &[filename(0, 5, 1, &name)]);
        let mut bad = good.clone();
        bad[size - 2] ^= 1;
        assert!(decode_record(&mut bad, 17, &geometry).is_err());
        let record = decode_record(&mut good, 17, &geometry).unwrap().unwrap();
        assert!(
            matches!(&record.attributes[0].value, AttributeValue::FileName { name: restored, .. } if restored == &name)
        );
    }
}

#[test]
fn extension_and_deleted_records_are_distinct_and_sparse_size_is_logical() {
    let geometry = Geometry::new(512, 4096, 1024, 100, 2, 32768).unwrap();
    let base = 0x2000000000011;
    let mut bytes = record(
        1024,
        3,
        1,
        base,
        &[nonresident(0x80, 2, 0, 0x8000, 13000, 4, &[1, 4, 0])],
    );
    let parsed = decode_record(&mut bytes, 30, &geometry).unwrap().unwrap();
    assert_eq!(parsed.base, base);
    assert!(matches!(
        &parsed.attributes[0].value,
        AttributeValue::Nonresident {
            size: Some(13000),
            ..
        }
    ));
    assert!(parsed.into_object().is_err());
    let mut bytes = record(1024, 3, 0, 0, &[]);
    assert!(decode_record(&mut bytes, 30, &geometry).unwrap().is_none());
}

#[test]
fn record_rejects_attribute_overflow_and_does_not_invent_file_size() {
    let geometry = Geometry::new(512, 4096, 1024, 100, 2, 32768).unwrap();
    let mut attribute = filename(0, 5, 1, &[65]);
    put32(&mut attribute, 4, u32::MAX);
    let mut bytes = record(1024, 1, 1, 0, &[attribute]);
    assert!(decode_record(&mut bytes, 17, &geometry).is_err());
    let mut bytes = record(
        1024,
        1,
        1,
        0,
        &[resident(0x10, 0, &[], &[0; 48]), filename(1, 5, 1, &[65])],
    );
    assert!(
        decode_record(&mut bytes, 17, &geometry)
            .unwrap()
            .unwrap()
            .into_object()
            .is_err()
    );
}
