//! Native SARC reader. Pure `std` (no third-party crate): parses the
//! `0x14` header, the `SFAT` node table, and the `SFNT` name blob, slicing
//! each node's data out of the data section. Every access is
//! bounds-checked, so a malformed/truncated archive errors cleanly instead
//! of panicking.

use super::error::{Result, SarcError};
use super::{ArcEntry, ArcFile, UnpackedFile};
use super::{SARC_HEADER_SIZE, SFAT_HAS_NAME, SFAT_HEADER_SIZE, SFAT_NODE_SIZE, SFNT_HEADER_SIZE};

fn read_u16(bytes: &[u8], off: usize, big_endian: bool) -> Result<u16> {
    let b = bytes.get(off..off + 2).ok_or(SarcError::Truncated {
        offset: off,
        need: 2,
    })?;
    Ok(if big_endian {
        u16::from_be_bytes([b[0], b[1]])
    } else {
        u16::from_le_bytes([b[0], b[1]])
    })
}

fn read_u32(bytes: &[u8], off: usize, big_endian: bool) -> Result<u32> {
    let b = bytes.get(off..off + 4).ok_or(SarcError::Truncated {
        offset: off,
        need: 4,
    })?;
    Ok(if big_endian {
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    } else {
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    })
}

/// Read a NUL-terminated UTF-8 string starting at `off`. SARC entry names
/// are always ASCII paths, so strict UTF-8 both succeeds in practice and
/// guarantees the name re-encodes to identical bytes.
fn read_c_string(bytes: &[u8], off: usize) -> Result<String> {
    let slice = bytes
        .get(off..)
        .ok_or(SarcError::NameOffsetOutOfBounds { offset: off })?;
    let end = slice
        .iter()
        .position(|&b| b == 0)
        .ok_or(SarcError::UnterminatedName { offset: off })?;
    std::str::from_utf8(&slice[..end])
        .map(str::to_owned)
        .map_err(|source| SarcError::NonUtf8Name {
            offset: off,
            source,
        })
}

/// Parse a SARC archive into an [`ArcFile`], preserving every entry (named
/// and hash-only) and the byte order.
///
/// Layout: a `0x14` header (`SARC` magic, header size, BOM, file size,
/// data offset, version), then an `SFAT` table of fixed `0x10` nodes
/// (hash, attrs, data start/end relative to `data_offset`), then an `SFNT`
/// name blob. A node owns a name when `attrs & SFAT_HAS_NAME` is set; the
/// low 24 bits are the name's offset in 4-byte units.
pub(super) fn parse_sarc(bytes: &[u8]) -> Result<ArcFile> {
    if bytes.len() < SARC_HEADER_SIZE {
        return Err(SarcError::TooSmall(bytes.len()));
    }
    if &bytes[0..4] != b"SARC" {
        let mut magic = [0u8; 4];
        magic.copy_from_slice(&bytes[0..4]);
        return Err(SarcError::BadMagic(magic));
    }
    // BOM at 0x06 selects endianness: written as 0xFEFF in the file's own
    // byte order, so the bytes are FE FF for big-endian and FF FE for
    // little-endian (Switch).
    let big_endian = match (bytes[0x06], bytes[0x07]) {
        (0xFE, 0xFF) => true,
        (0xFF, 0xFE) => false,
        (a, b) => return Err(SarcError::BadBom([a, b])),
    };
    let data_offset = read_u32(bytes, 0x0C, big_endian)? as usize;

    // ---- SFAT ----
    let sfat_off = SARC_HEADER_SIZE;
    if bytes.get(sfat_off..sfat_off + 4) != Some(b"SFAT".as_slice()) {
        return Err(SarcError::MissingSection("SFAT"));
    }
    let node_count = read_u16(bytes, sfat_off + 0x06, big_endian)? as usize;
    let nodes_off = sfat_off + SFAT_HEADER_SIZE;
    let nodes_end = nodes_off + node_count * SFAT_NODE_SIZE;

    // ---- SFNT (name blob follows the node table) ----
    if bytes.get(nodes_end..nodes_end + 4) != Some(b"SFNT".as_slice()) {
        return Err(SarcError::MissingSection("SFNT"));
    }
    let names_off = nodes_end + SFNT_HEADER_SIZE;

    let mut files = Vec::with_capacity(node_count);
    for i in 0..node_count {
        let node = nodes_off + i * SFAT_NODE_SIZE;
        let attrs = read_u32(bytes, node + 0x04, big_endian)?;
        let start = read_u32(bytes, node + 0x08, big_endian)? as usize;
        let end = read_u32(bytes, node + 0x0C, big_endian)? as usize;
        if end < start {
            return Err(SarcError::NodeBackwards {
                index: i,
                start,
                end,
            });
        }
        let abs_start = data_offset + start;
        let abs_end = data_offset + end;
        let data = bytes
            .get(abs_start..abs_end)
            .ok_or(SarcError::NodeOutOfBounds {
                index: i,
                start: abs_start,
                end: abs_end,
                len: bytes.len(),
            })?
            .to_vec();
        let name = if attrs & SFAT_HAS_NAME != 0 {
            let name_off = names_off + (attrs & 0x00FF_FFFF) as usize * 4;
            Some(read_c_string(bytes, name_off)?)
        } else {
            None
        };
        files.push(ArcEntry { name, data });
    }

    Ok(ArcFile { big_endian, files })
}

/// Parse a SARC archive into an [`ArcFile`], preserving all entries (named
/// and hash-only) and the byte order.
pub fn read_arc(bytes: &[u8]) -> Result<ArcFile> {
    parse_sarc(bytes)
}

/// Parse a SARC archive into its named files. Hash-only entries (without a
/// stored name) are skipped.
pub fn unpack(bytes: &[u8]) -> Result<Vec<UnpackedFile>> {
    let arc = parse_sarc(bytes)?;
    let mut out = Vec::with_capacity(arc.files.len());
    for entry in arc.files {
        if let Some(name) = entry.name {
            out.push(UnpackedFile {
                name,
                data: entry.data,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::write::write_sarc;
    use super::*;

    fn entry(name: Option<&str>, data: &[u8]) -> ArcEntry {
        ArcEntry {
            name: name.map(str::to_owned),
            data: data.to_vec(),
        }
    }

    #[test]
    fn parses_a_hand_written_archive() {
        let entries = vec![
            entry(Some("dir/a.bin"), b"alpha"),
            entry(Some("dir/sub/b.bin"), b"beta-beta"),
        ];
        let packed = write_sarc(&entries, false).unwrap();
        let arc = parse_sarc(&packed).unwrap();
        assert!(!arc.big_endian);
        assert_eq!(arc.files.len(), 2);
        assert_eq!(
            arc.position("dir/a.bin").map(|i| &arc.files[i].data[..]),
            Some(&b"alpha"[..])
        );
    }

    // The malformed-input checklist below mirrors the failure modes the MIT
    // `jam1garner/sarc` crate guards against (bad magic, short buffers,
    // missing tables) — authored fresh here from the format spec, with no
    // third-party code or fixtures.
    #[test]
    fn rejects_too_small() {
        assert!(matches!(parse_sarc(&[]), Err(SarcError::TooSmall(0))));
        assert!(matches!(parse_sarc(b"SAR"), Err(SarcError::TooSmall(3))));
    }

    #[test]
    fn rejects_bad_magic() {
        let buf = [0u8; 0x40];
        assert!(matches!(parse_sarc(&buf), Err(SarcError::BadMagic(_))));
    }

    #[test]
    fn rejects_bad_bom() {
        let mut buf = vec![0u8; 0x40];
        buf[0..4].copy_from_slice(b"SARC");
        buf[0x06] = 0x12;
        buf[0x07] = 0x34;
        assert!(matches!(
            parse_sarc(&buf),
            Err(SarcError::BadBom([0x12, 0x34]))
        ));
    }

    #[test]
    fn rejects_missing_sfat() {
        // Valid header bytes, but no SFAT magic where it belongs.
        let mut buf = vec![0u8; 0x40];
        buf[0..4].copy_from_slice(b"SARC");
        buf[0x06] = 0xFF;
        buf[0x07] = 0xFE;
        assert!(matches!(
            parse_sarc(&buf),
            Err(SarcError::MissingSection("SFAT"))
        ));
    }

    #[test]
    fn rejects_node_count_past_end() {
        // Real header + SFAT magic, but claim more nodes than the buffer
        // holds, so the SFNT check (and node reads) fall off the end.
        let entries = vec![entry(Some("a"), b"x")];
        let mut packed = write_sarc(&entries, false).unwrap();
        // Bump the SFAT node_count (LE u16 at 0x1A) to 9999.
        packed[0x1A] = 0x0F;
        packed[0x1B] = 0x27;
        assert!(parse_sarc(&packed).is_err());
    }

    #[test]
    fn rejects_node_data_out_of_bounds() {
        let entries = vec![entry(Some("a"), b"hello")];
        let mut packed = write_sarc(&entries, false).unwrap();
        // The single node's data_end is the last u32 of the node at 0x20;
        // node layout is hash,attrs,start,end → end at 0x20 + 0x0C = 0x2C.
        packed[0x2C] = 0xFF;
        packed[0x2D] = 0xFF;
        packed[0x2E] = 0xFF;
        packed[0x2F] = 0x7F;
        assert!(matches!(
            parse_sarc(&packed),
            Err(SarcError::NodeOutOfBounds { .. })
        ));
    }
}
