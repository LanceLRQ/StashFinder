pub fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

pub fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

pub fn put64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

pub fn record(
    size: usize,
    sequence: u16,
    flags: u16,
    base: u64,
    attributes: &[Vec<u8>],
) -> Vec<u8> {
    let mut bytes = vec![0; size];
    bytes[..4].copy_from_slice(b"FILE");
    let usa = 48;
    let count = size / 512 + 1;
    let first = (usa + count * 2).next_multiple_of(8);
    put16(&mut bytes, 4, usa as u16);
    put16(&mut bytes, 6, count as u16);
    put16(&mut bytes, 16, sequence);
    put16(&mut bytes, 20, first as u16);
    put16(&mut bytes, 22, flags);
    put32(&mut bytes, 28, size as u32);
    put64(&mut bytes, 32, base);
    let mut at = first;
    for attribute in attributes {
        bytes[at..at + attribute.len()].copy_from_slice(attribute);
        at += attribute.len();
    }
    put32(&mut bytes, at, u32::MAX);
    put32(&mut bytes, 24, (at + 8) as u32);
    put16(&mut bytes, usa, 0x1234);
    for sector in 1..count {
        let end = sector * 512 - 2;
        bytes.copy_within(end..end + 2, usa + sector * 2);
        put16(&mut bytes, end, 0x1234);
    }
    bytes
}

pub fn resident(kind: u32, id: u16, name: &[u16], value: &[u8]) -> Vec<u8> {
    let offset = (24 + name.len() * 2).next_multiple_of(8);
    let mut bytes = vec![0; (offset + value.len()).next_multiple_of(8)];
    let length = bytes.len() as u32;
    put32(&mut bytes, 0, kind);
    put32(&mut bytes, 4, length);
    bytes[9] = name.len() as u8;
    put16(&mut bytes, 10, 24);
    put16(&mut bytes, 14, id);
    put32(&mut bytes, 16, value.len() as u32);
    put16(&mut bytes, 20, offset as u16);
    for (i, unit) in name.iter().enumerate() {
        put16(&mut bytes, 24 + i * 2, *unit);
    }
    bytes[offset..offset + value.len()].copy_from_slice(value);
    bytes
}

pub fn filename(id: u16, parent: u64, namespace: u8, name: &[u16]) -> Vec<u8> {
    let mut value = vec![0; 66 + name.len() * 2];
    put64(&mut value, 0, parent);
    value[64] = name.len() as u8;
    value[65] = namespace;
    for (i, unit) in name.iter().enumerate() {
        put16(&mut value, 66 + i * 2, *unit);
    }
    resident(0x30, id, &[], &value)
}

pub fn nonresident(
    kind: u32,
    id: u16,
    vcn: u64,
    flags: u16,
    size: u64,
    clusters: u64,
    pairs: &[u8],
) -> Vec<u8> {
    let offset = if flags & 0x8001 == 0 { 64 } else { 72 };
    let mut bytes = vec![0; (offset + pairs.len()).next_multiple_of(8)];
    let length = bytes.len() as u32;
    put32(&mut bytes, 0, kind);
    put32(&mut bytes, 4, length);
    bytes[8] = 1;
    put16(&mut bytes, 12, flags);
    put16(&mut bytes, 14, id);
    put64(&mut bytes, 16, vcn);
    put64(&mut bytes, 24, vcn + clusters - 1);
    put16(&mut bytes, 32, offset as u16);
    put64(&mut bytes, 48, size);
    put64(&mut bytes, 56, size);
    bytes[offset..offset + pairs.len()].copy_from_slice(pairs);
    bytes
}

pub struct MemoryDisk {
    pub bytes: Vec<u8>,
    pub reads: Vec<(u64, usize)>,
}

impl stashfinder_windows::ntfs::mft::ReadAt for MemoryDisk {
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> anyhow::Result<()> {
        use anyhow::Context;
        self.reads.push((offset, out.len()));
        let offset = offset as usize;
        out.copy_from_slice(
            self.bytes
                .get(offset..offset + out.len())
                .context("fixture read outside disk")?,
        );
        Ok(())
    }
}

pub fn list_entry(kind: u32, id: u16, reference: u64, vcn: u64) -> Vec<u8> {
    let mut bytes = vec![0; 32];
    put32(&mut bytes, 0, kind);
    put16(&mut bytes, 4, 32);
    put64(&mut bytes, 8, vcn);
    put64(&mut bytes, 16, reference);
    put16(&mut bytes, 24, id);
    bytes
}

pub fn fragmented_disk() -> (stashfinder_windows::ntfs::mft::Geometry, MemoryDisk) {
    use stashfinder_windows::ntfs::mft::Geometry;
    let root = 0x1000000000005;
    let base = 0x2000000000012;
    let first = 131 * 512;
    let length = 90 * 1024;
    let mut logical = vec![0; length];
    let mut place = |slot: usize, bytes: Vec<u8>| {
        logical[slot * 1024..(slot + 1) * 1024].copy_from_slice(&bytes)
    };
    place(
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
                180,
                &[0x21, 131, 160, 0, 0x21, 49, 0x68, 0xff, 0],
            )],
        ),
    );
    place(
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
    );
    let internal = 0x0b00000000000b;
    place(
        11,
        record(1024, 11, 3, 0, &[filename(1, root, 1, &[36, 69])]),
    );
    place(
        24,
        record(
            1024,
            1,
            1,
            0,
            &[
                resident(0x10, 0, &[], &[0; 48]),
                filename(1, internal, 1, &[36, 81]),
            ],
        ),
    );
    place(16, record(1024, 1, 1, base, &[filename(2, root, 1, &[65])]));
    place(
        20,
        record(
            1024,
            1,
            1,
            base,
            &[filename(3, root, 1, &[66]), resident(0x80, 4, &[], &[0; 3])],
        ),
    );
    let list = [
        list_entry(0x10, 0, base, 0),
        list_entry(0x30, 2, 0x1000000000010, 0),
        list_entry(0x30, 3, 0x1000000000014, 0),
        list_entry(0x80, 4, 0x1000000000014, 0),
    ]
    .concat();
    place(
        18,
        record(
            1024,
            2,
            1,
            0,
            &[
                resident(0x10, 0, &[], &[0; 48]),
                resident(0x20, 1, &[], &list),
            ],
        ),
    );
    place(
        65,
        record(
            1024,
            3,
            1,
            0,
            &[
                resident(0x10, 0, &[], &[0; 48]),
                filename(1, root, 1, &[67]),
                resident(0x80, 2, &[], &[]),
            ],
        ),
    );
    let mut bytes = vec![0; 512 * 512];
    bytes[160 * 512..160 * 512 + first].copy_from_slice(&logical[..first]);
    bytes[8 * 512..8 * 512 + length - first].copy_from_slice(&logical[first..]);
    (
        Geometry::new(512, 512, 1024, 512, 160, length as u64).unwrap(),
        MemoryDisk {
            bytes,
            reads: Vec::new(),
        },
    )
}
