use stashfinder_windows::ntfs::mft::{Geometry, ReadAt, read_mft_object};

#[path = "support/mft.rs"]
#[allow(dead_code)]
mod fixture;
use fixture::*;

struct Records {
    base: Vec<u8>,
    extension: Vec<u8>,
    returned_slot: u64,
}
impl ReadAt for Records {
    fn read_exact_at(&mut self, _: u64, _: &mut [u8]) -> anyhow::Result<()> {
        anyhow::bail!("must not read file contents")
    }
    fn bootstrap_record(&mut self, reference: u64, out: &mut [u8]) -> anyhow::Result<Option<u64>> {
        let slot = reference & 0x0000_ffff_ffff_ffff;
        out.copy_from_slice(if slot == 18 {
            &self.base
        } else {
            &self.extension
        });
        Ok(Some(if slot == 18 { self.returned_slot } else { slot }))
    }
}
fn records() -> (Geometry, Records) {
    let id = 0x2000000000012;
    let extension = 0x3000000000014;
    let root = 0x1000000000005;
    let mut info = vec![0; 48];
    put64(&mut info, 0, 123);
    put64(&mut info, 8, 456);
    put32(&mut info, 32, 0x26);
    let list = [
        list_entry(0x10, 0, id, 0),
        list_entry(0x30, 1, id, 0),
        list_entry(0x30, 2, extension, 0),
        list_entry(0x80, 3, extension, 0),
    ]
    .concat();
    (
        Geometry::new(512, 4096, 1024, 1024, 2, 1024 * 30).unwrap(),
        Records {
            base: record(
                1024,
                2,
                1,
                0,
                &[
                    resident(0x10, 0, &[], &info),
                    filename(1, root, 1, &[65]),
                    resident(0x20, 4, &[], &list),
                ],
            ),
            extension: record(
                1024,
                3,
                1,
                id,
                &[filename(2, root, 1, &[66]), resident(0x80, 3, &[], &[0; 7])],
            ),
            returned_slot: 18,
        },
    )
}
#[test]
fn targeted_metadata_includes_extension_links_size_and_standard_times() {
    let (geometry, mut disk) = records();
    let object = read_mft_object(&mut disk, &geometry, 0x2000000000012).unwrap();
    assert_eq!(object.entry.id, 0x2000000000012);
    assert_eq!(object.entry.size, 7);
    assert_eq!(
        (
            object.entry.created,
            object.entry.modified,
            object.entry.attributes
        ),
        (123, 456, 0x26)
    );
    assert_eq!(
        object.links,
        vec![(0x1000000000005, vec![65]), (0x1000000000005, vec![66])]
    );
}
#[test]
fn targeted_metadata_rejects_adjacent_reused_or_mismatched_extension_records() {
    for fault in 0..4 {
        let (geometry, mut disk) = records();
        match fault {
            0 => disk.returned_slot = 17,
            1 => put16(&mut disk.base, 16, 4),
            2 => put16(&mut disk.extension, 16, 4),
            _ => put64(&mut disk.extension, 32, 0x2000000000013),
        }
        assert!(read_mft_object(&mut disk, &geometry, 0x2000000000012).is_err());
    }
}

#[test]
fn control_records_accept_restored_tails_but_reject_torn_records() {
    use stashfinder_windows::ntfs::mft::{decode_record, prepare_control_record};
    let (geometry, disk) = records();
    let mut restored = disk.base.clone();
    for i in 1..3 {
        restored.copy_within(48 + i * 2..50 + i * 2, i * 512 - 2);
    }
    prepare_control_record(&mut restored).unwrap();
    assert_eq!(
        decode_record(&mut restored, 18, &geometry)
            .unwrap()
            .unwrap()
            .reference,
        0x2000000000012
    );
    let mut raw = disk.base.clone();
    prepare_control_record(&mut raw).unwrap();
    assert!(decode_record(&mut raw, 18, &geometry).unwrap().is_some());
    let mut torn = disk.base;
    put16(&mut torn, 510, 0xabcd);
    assert!(prepare_control_record(&mut torn).is_err());
}
