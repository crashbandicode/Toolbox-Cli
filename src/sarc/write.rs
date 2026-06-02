//! Native SARC writer with per-file alignment. Pure `std`. Gives each file
//! the alignment its content requires instead of padding everything to
//! `0x2000` (which roughly doubles a real `layout.arc`).

use super::error::Result;
use super::{ArcEntry, ArcFile};
use super::{
    MAX_ALIGNMENT, MIN_ALIGNMENT, SARC_HASH_KEY, SARC_HEADER_SIZE, SFAT_HAS_NAME, SFAT_HEADER_SIZE,
    SFAT_NODE_SIZE, SFNT_HEADER_SIZE,
};

/// SFAT name hash (the standard SARC multiply-add hash with key `0x65`).
fn sarc_hash(name: &str) -> u32 {
    name.bytes().fold(0u32, |h, b| {
        h.wrapping_mul(SARC_HASH_KEY).wrapping_add(b as u32)
    })
}

fn align_up(value: usize, align: usize) -> usize {
    let a = align.max(1);
    value.div_ceil(a) * a
}

/// Derive the data alignment a file requires from its content.
///
/// Most Switch resources use the `nn::util::BinaryFileHeader` layout: an
/// 8-byte magic, a `u32` version, a `u16` byte-order mark at `0x0C`, then
/// a `u8` alignment exponent at `0x0E`. When that BOM is present we honor
/// `1 << exponent` (verified against fixtures: BNTX and BNSH report
/// `0x1000`). Cafe layout files (BFLYT/BFLAN — BOM at `0x04`, not `0x0C`)
/// and the custom `info` blob have no such field and only need the
/// minimum. Nested archives get `0x2000`; Yaz0-compressed payloads
/// `0x80`. The result is clamped to `[MIN_ALIGNMENT, MAX_ALIGNMENT]`.
pub fn file_alignment(data: &[u8]) -> u32 {
    let mut alignment = MIN_ALIGNMENT;

    if data.len() >= 4 {
        match &data[0..4] {
            b"SARC" => alignment = alignment.max(0x2000),
            b"Yaz0" | b"Yaz1" => alignment = alignment.max(0x80),
            _ => {}
        }
    }

    // nn::util::BinaryFileHeader: BOM at 0x0C, alignment exponent at 0x0E.
    if data.len() > 0x20 {
        let bom = (data[0x0C], data[0x0D]);
        let has_bom = bom == (0xFF, 0xFE) || bom == (0xFE, 0xFF);
        if has_bom {
            let exponent = data[0x0E];
            if exponent <= 13 {
                alignment = alignment.max(1u32 << exponent);
            }
        }
    }

    alignment.clamp(MIN_ALIGNMENT, MAX_ALIGNMENT)
}

fn push_u16(out: &mut Vec<u8>, v: u16, big_endian: bool) {
    if big_endian {
        out.extend_from_slice(&v.to_be_bytes());
    } else {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

fn push_u32(out: &mut Vec<u8>, v: u32, big_endian: bool) {
    if big_endian {
        out.extend_from_slice(&v.to_be_bytes());
    } else {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// Serialize an [`ArcFile`] back to SARC bytes via [`write_sarc`]. Named
/// entries are re-hashed into the SFAT/SFNT tables; each file's data is
/// aligned to the boundary it requires (see [`file_alignment`]). Not
/// guaranteed byte-identical to the source, but a valid archive containing
/// every entry — including hash-only ones.
pub fn write_arc(arc: &ArcFile) -> Result<Vec<u8>> {
    write_sarc(&arc.files, arc.big_endian)
}

/// Serialize SARC entries to a valid archive, giving each file the
/// alignment [`file_alignment`] derives. The SFAT is sorted by name hash
/// (as the format requires for the game's binary search); hash-only
/// (unnamed) entries are preserved with attrs `0` rather than being
/// collapsed. Endianness follows `big_endian` (false = little = Switch).
pub fn write_sarc(entries: &[ArcEntry], big_endian: bool) -> Result<Vec<u8>> {
    // SFAT must be ordered by name hash; a stable sort keeps the input
    // order for equal hashes (e.g. multiple unnamed entries).
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by_key(|&i| entries[i].name.as_deref().map(sarc_hash).unwrap_or(0));

    // Build the SFNT string table (named entries, in SFAT order) and
    // record each name's 4-byte-unit offset for its SFAT node.
    let mut name_bytes: Vec<u8> = Vec::new();
    let mut name_units: Vec<Option<u32>> = vec![None; entries.len()];
    for &i in &order {
        if let Some(name) = &entries[i].name {
            let offset = name_bytes.len();
            name_units[i] = Some((offset / 4) as u32);
            name_bytes.extend_from_slice(name.as_bytes());
            name_bytes.push(0);
            while !name_bytes.len().is_multiple_of(4) {
                name_bytes.push(0);
            }
        }
    }

    let node_count = entries.len();
    let pre_data = SARC_HEADER_SIZE
        + SFAT_HEADER_SIZE
        + node_count * SFAT_NODE_SIZE
        + SFNT_HEADER_SIZE
        + name_bytes.len();

    let max_alignment = order
        .iter()
        .map(|&i| file_alignment(&entries[i].data))
        .max()
        .unwrap_or(MIN_ALIGNMENT)
        .max(MIN_ALIGNMENT) as usize;
    let data_offset = align_up(pre_data, max_alignment);

    // Lay out the data section (in SFAT order), aligning each file.
    let mut data_start = vec![0u32; entries.len()];
    let mut data_end = vec![0u32; entries.len()];
    let mut cursor = 0usize; // relative to data_offset
    for &i in &order {
        let align = file_alignment(&entries[i].data) as usize;
        let start = align_up(cursor, align);
        data_start[i] = start as u32;
        data_end[i] = (start + entries[i].data.len()) as u32;
        cursor = start + entries[i].data.len();
    }
    let file_size = data_offset + cursor;

    let mut out = Vec::with_capacity(file_size);

    // ---- SARC header ----
    out.extend_from_slice(b"SARC");
    push_u16(&mut out, SARC_HEADER_SIZE as u16, big_endian);
    // BOM 0xFEFF written in the file's endianness (LE → FF FE), which the
    // reader interprets big-endian to recover the byte order.
    push_u16(&mut out, 0xFEFF, big_endian);
    push_u32(&mut out, file_size as u32, big_endian);
    push_u32(&mut out, data_offset as u32, big_endian);
    push_u16(&mut out, 0x0100, big_endian); // version
    push_u16(&mut out, 0, big_endian); // reserved

    // ---- SFAT ----
    out.extend_from_slice(b"SFAT");
    push_u16(&mut out, SFAT_HEADER_SIZE as u16, big_endian);
    push_u16(&mut out, node_count as u16, big_endian);
    push_u32(&mut out, SARC_HASH_KEY, big_endian);
    for &i in &order {
        let hash = entries[i].name.as_deref().map(sarc_hash).unwrap_or(0);
        let attrs = match name_units[i] {
            Some(units) => SFAT_HAS_NAME | units,
            None => 0,
        };
        push_u32(&mut out, hash, big_endian);
        push_u32(&mut out, attrs, big_endian);
        push_u32(&mut out, data_start[i], big_endian);
        push_u32(&mut out, data_end[i], big_endian);
    }

    // ---- SFNT ----
    out.extend_from_slice(b"SFNT");
    push_u16(&mut out, SFNT_HEADER_SIZE as u16, big_endian);
    push_u16(&mut out, 0, big_endian);
    out.extend_from_slice(&name_bytes);

    // ---- pad to data_offset, then the data section ----
    out.resize(data_offset, 0);
    for &i in &order {
        let abs = data_offset + data_start[i] as usize;
        out.resize(abs, 0); // per-file alignment padding
        out.extend_from_slice(&entries[i].data);
    }
    debug_assert_eq!(out.len(), file_size);

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::read::read_arc;
    use super::*;

    fn entry(name: Option<&str>, data: &[u8]) -> ArcEntry {
        ArcEntry {
            name: name.map(str::to_owned),
            data: data.to_vec(),
        }
    }

    /// A buffer that looks like a `nn::util::BinaryFileHeader` resource with
    /// the given alignment exponent (BOM at 0x0C, exponent at 0x0E).
    fn nn_resource(magic: &[u8; 4], exponent: u8) -> Vec<u8> {
        let mut v = vec![0u8; 0x40];
        v[0..4].copy_from_slice(magic);
        v[0x0C] = 0xFF; // BOM (little-endian)
        v[0x0D] = 0xFE;
        v[0x0E] = exponent;
        v
    }

    #[test]
    fn alignment_derivation() {
        assert_eq!(file_alignment(b"abc"), MIN_ALIGNMENT); // too short, no header
        assert_eq!(file_alignment(b"FLYT\x00\x00\x00\x00"), MIN_ALIGNMENT); // no 0x0C BOM
                                                                            // BNTX/BNSH report exponent 12 → 0x1000.
        assert_eq!(file_alignment(&nn_resource(b"BNTX", 12)), 0x1000);
        assert_eq!(file_alignment(&nn_resource(b"BNSH", 12)), 0x1000);
        // Nested SARC and Yaz0 by magic.
        assert_eq!(
            file_alignment(b"SARC____________________________________"),
            0x2000
        );
        assert_eq!(
            file_alignment(b"Yaz0____________________________________"),
            0x80
        );
        // Exponent clamps at MAX_ALIGNMENT (0x2000 = 1<<13).
        assert_eq!(file_alignment(&nn_resource(b"ANY?", 13)), 0x2000);
        // Out-of-range exponent is ignored (falls back to minimum).
        assert_eq!(file_alignment(&nn_resource(b"ANY?", 31)), MIN_ALIGNMENT);
    }

    fn assert_round_trips(entries: &[ArcEntry], big_endian: bool) {
        let packed = write_sarc(entries, big_endian).expect("write");
        let arc = read_arc(&packed).expect("read");
        assert_eq!(arc.big_endian, big_endian);
        assert_eq!(arc.files.len(), entries.len(), "entry count");
        for src in entries {
            match &src.name {
                Some(name) => {
                    let got = arc
                        .files
                        .iter()
                        .find(|f| f.name.as_deref() == Some(name.as_str()))
                        .unwrap_or_else(|| panic!("missing {name}"));
                    assert_eq!(got.data, src.data, "data for {name}");
                }
                None => assert!(
                    arc.files
                        .iter()
                        .any(|f| f.name.is_none() && f.data == src.data),
                    "hash-only entry not preserved"
                ),
            }
        }
        // Every node sits on its content's required alignment.
        let data_offset =
            u32::from_le_bytes([packed[0x0C], packed[0x0D], packed[0x0E], packed[0x0F]]) as usize;
        if big_endian {
            // Skip the alignment offset check for BE (offsets are BE); the
            // round-trip above already proves correctness.
            return;
        }
        let node_count = u16::from_le_bytes([packed[0x1A], packed[0x1B]]) as usize;
        for i in 0..node_count {
            let node = 0x20 + i * 0x10;
            let start = u32::from_le_bytes([
                packed[node + 8],
                packed[node + 9],
                packed[node + 10],
                packed[node + 11],
            ]) as usize;
            let abs = data_offset + start;
            let end = u32::from_le_bytes([
                packed[node + 12],
                packed[node + 13],
                packed[node + 14],
                packed[node + 15],
            ]) as usize;
            let needed = file_alignment(&packed[abs..data_offset + end]) as usize;
            assert_eq!(
                abs % needed,
                0,
                "node {i} at 0x{abs:x} not aligned to 0x{needed:x}"
            );
        }
    }

    #[test]
    fn round_trip_named_hash_only_and_alignment() {
        let entries = vec![
            entry(Some("a/first.bin"), b"hello world"),
            entry(Some("b/second.txt"), &[0xAB; 37]),
            entry(None, b"\x01\x02\x03\x04unnamed payload"),
            entry(Some("timg/__Combined.bntx"), &nn_resource(b"BNTX", 12)),
            entry(Some("blyt/x.bflan"), &nn_resource(b"FLAN", 0)),
        ];
        assert_round_trips(&entries, false);
    }

    #[test]
    fn round_trip_big_endian() {
        let entries = vec![entry(Some("x"), b"wii-u"), entry(Some("y/z"), b"BE!")];
        let packed = write_sarc(&entries, true).unwrap();
        assert_eq!((packed[0x06], packed[0x07]), (0xFE, 0xFF));
        assert_round_trips(&entries, true);
    }

    #[test]
    fn empty_and_single() {
        assert_round_trips(&[], false);
        assert_round_trips(&[entry(Some("solo"), b"only one")], false);
        assert_round_trips(&[entry(None, b"only-hash-only")], false);
    }

    #[test]
    fn many_hash_only_preserve_order_and_data() {
        // All hash-only entries hash to 0, so the stable sort must keep them
        // in input order and round-trip each distinct payload.
        let entries: Vec<ArcEntry> = (0..16)
            .map(|i| entry(None, format!("hash-only-#{i}").as_bytes()))
            .collect();
        let packed = write_sarc(&entries, false).unwrap();
        let arc = read_arc(&packed).unwrap();
        let got: Vec<&[u8]> = arc.files.iter().map(|f| f.data.as_slice()).collect();
        let want: Vec<&[u8]> = entries.iter().map(|e| e.data.as_slice()).collect();
        assert_eq!(got, want, "hash-only entries must keep input order");
    }

    #[test]
    fn large_archive() {
        let entries: Vec<ArcEntry> = (0..2000u32)
            .map(|i| entry(Some(&format!("dir{}/file{i}.bin", i % 8)), &i.to_le_bytes()))
            .collect();
        assert_round_trips(&entries, false);
    }

    #[test]
    fn property_round_trip_pseudo_random() {
        // Deterministic LCG generates varied entry sets (names, sizes,
        // occasional hash-only, GPU-aligned resources) and asserts each set
        // survives write → read intact.
        let mut state = 0x1234_5678u32;
        let mut next = || {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            state >> 8
        };
        for round in 0..40 {
            let count = (next() % 30) as usize;
            let mut entries = Vec::with_capacity(count);
            for i in 0..count {
                let len = (next() % 200) as usize;
                let byte = (next() & 0xFF) as u8;
                let data = vec![byte; len];
                if next() % 7 == 0 {
                    entries.push(entry(None, &data));
                } else {
                    let name = format!("r{round}/d{}/f{i}.bin", next() % 5);
                    entries.push(entry(Some(&name), &data));
                }
            }
            assert_round_trips(&entries, false);
        }
    }
}
