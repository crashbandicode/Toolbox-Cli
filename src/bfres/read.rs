//! BFRES header parser: magic / version / endianness / embedded name /
//! file size / relocation-table offset, plus a structural scan for the
//! well-known sub-block magics.
//!
//! All reads are bounds-checked and report the failing offset. The model /
//! animation sub-resources are not decoded; the original bytes are retained for
//! the verbatim [`write_bfres`](super::write_bfres) round-trip.

use super::error::{BfresError, Result};
use super::*;

/// Parse a BFRES container, retaining the original bytes for the verbatim
/// [`write_bfres`](super::write_bfres) round-trip.
pub fn read_bfres(data: &[u8]) -> Result<BfresDocument> {
    if data.len() < HEADER_LEN {
        return Err(BfresError::TooSmall(data.len()));
    }
    if &data[0..8] != BFRES_MAGIC {
        let mut m = [0u8; 8];
        m.copy_from_slice(&data[0..8]);
        return Err(BfresError::BadMagic(m));
    }

    // Byte-order mark at 0x0C picks endianness (Switch BFRES is little-endian;
    // a Wii U file would be big-endian). 0xFEFF stored LE is `FF FE`, BE `FE FF`.
    let big_endian = match (data[0x0C], data[0x0D]) {
        (0xFF, 0xFE) => false,
        (0xFE, 0xFF) => true,
        (a, b) => return Err(BfresError::BadBom(u16::from_le_bytes([a, b]))),
    };

    let version = read_u32(data, 0x08, big_endian)?;
    let name_off = read_u32(data, 0x10, big_endian)? as usize;
    let relocation_table_offset = read_u32(data, 0x18, big_endian)?;
    let file_size = read_u32(data, 0x1C, big_endian)?;

    let name = read_cstring(data, name_off)?;
    let blocks = scan_blocks(data);

    Ok(BfresDocument {
        version,
        big_endian,
        name,
        file_size,
        relocation_table_offset,
        blocks,
        raw: data.to_vec(),
    })
}

/// Scan the file for the well-known [`BLOCK_MAGICS`], returning the count and
/// first offset of each present. 4-byte magics collide with arbitrary data only
/// at a ~1-in-4-billion rate, so this is a reliable content summary in practice.
fn scan_blocks(data: &[u8]) -> Vec<DetectedBlock> {
    let mut out = Vec::new();
    for magic in BLOCK_MAGICS {
        let mut count = 0usize;
        let mut first = None;
        let mut i = 0usize;
        while i + 4 <= data.len() {
            if &data[i..i + 4] == magic.as_slice() {
                count += 1;
                if first.is_none() {
                    first = Some(i);
                }
            }
            i += 1;
        }
        if let Some(first_offset) = first {
            out.push(DetectedBlock {
                magic: std::str::from_utf8(magic.as_slice())
                    .unwrap_or("?")
                    .to_string(),
                count,
                first_offset,
            });
        }
    }
    out
}

fn read_u32(data: &[u8], off: usize, big_endian: bool) -> Result<u32> {
    match off.checked_add(4) {
        Some(end) if end <= data.len() => {
            let b = [data[off], data[off + 1], data[off + 2], data[off + 3]];
            Ok(if big_endian {
                u32::from_be_bytes(b)
            } else {
                u32::from_le_bytes(b)
            })
        }
        _ => Err(BfresError::Truncated {
            offset: off,
            need: 4,
            len: data.len(),
        }),
    }
}

/// Read a NUL-terminated UTF-8 string at `off` (BFRES names are ASCII; the
/// switch container stores a u16 length at `off - 2`, which we don't need).
fn read_cstring(data: &[u8], off: usize) -> Result<String> {
    if off == 0 || off >= data.len() {
        return Err(BfresError::Truncated {
            offset: off,
            need: 1,
            len: data.len(),
        });
    }
    let rest = &data[off..];
    let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    std::str::from_utf8(&rest[..end])
        .map(|s| s.to_string())
        .map_err(|e| BfresError::NonUtf8 {
            offset: off,
            source: e,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid BFRES header naming `"Test"`. The name chars sit
    /// at 0x30 (with a u16 length at 0x2E), an `FMDL` magic at 0x40, and an
    /// `_RLT` near the end so the structural scan + header decode have content.
    fn minimal_bfres() -> Vec<u8> {
        let mut b = vec![0u8; 0x60];
        let total = b.len() as u32;
        b[0..8].copy_from_slice(BFRES_MAGIC);
        b[0x08..0x0C].copy_from_slice(&VERSION_TOTK.to_le_bytes());
        b[0x0C] = 0xFF; // BOM = 0xFEFF (LE)
        b[0x0D] = 0xFE;
        b[0x10..0x14].copy_from_slice(&0x30u32.to_le_bytes()); // name chars at 0x30
        b[0x18..0x1C].copy_from_slice(&0x50u32.to_le_bytes()); // _RLT at 0x50
        b[0x1C..0x20].copy_from_slice(&total.to_le_bytes()); // file size
                                                             // name: u16 len at 0x2E, "Test\0" at 0x30
        b[0x2E..0x30].copy_from_slice(&4u16.to_le_bytes());
        b[0x30..0x35].copy_from_slice(b"Test\0");
        b[0x40..0x44].copy_from_slice(b"FMDL");
        b[0x50..0x54].copy_from_slice(b"_RLT");
        b
    }

    #[test]
    fn parses_minimal() {
        let bytes = minimal_bfres();
        let doc = read_bfres(&bytes).expect("parse");
        assert_eq!(doc.version, VERSION_TOTK);
        assert!(!doc.big_endian);
        assert_eq!(doc.name, "Test");
        assert_eq!(doc.file_size as usize, bytes.len());
        assert_eq!(doc.relocation_table_offset, 0x50);
        assert_eq!(doc.block_count("FMDL"), 1);
        assert_eq!(doc.block_count("_RLT"), 1);
        assert_eq!(doc.embedded_bntx_offset(), None);
        // Verbatim writer reproduces the input.
        assert_eq!(super::super::write_bfres(&doc), bytes);
    }

    #[test]
    fn detects_big_endian_bom() {
        let mut bytes = minimal_bfres();
        bytes[0x0C] = 0xFE; // BOM = big-endian
        bytes[0x0D] = 0xFF;
        // Re-encode the header fields big-endian so they still decode sanely.
        bytes[0x08..0x0C].copy_from_slice(&VERSION_TOTK.to_be_bytes());
        bytes[0x10..0x14].copy_from_slice(&0x30u32.to_be_bytes());
        bytes[0x18..0x1C].copy_from_slice(&0x50u32.to_be_bytes());
        let len = bytes.len() as u32;
        bytes[0x1C..0x20].copy_from_slice(&len.to_be_bytes());
        let doc = read_bfres(&bytes).expect("parse BE");
        assert!(doc.big_endian);
        assert_eq!(doc.version, VERSION_TOTK);
        assert_eq!(doc.name, "Test");
    }

    #[test]
    fn rejects_bad_input() {
        assert!(matches!(
            read_bfres(&[0u8; 8]),
            Err(BfresError::TooSmall(8))
        ));
        let mut bad = vec![0u8; HEADER_LEN];
        bad[0..8].copy_from_slice(b"NOPE0000");
        assert!(matches!(read_bfres(&bad), Err(BfresError::BadMagic(_))));
        let mut bad_bom = minimal_bfres();
        bad_bom[0x0C] = 0x12;
        bad_bom[0x0D] = 0x34;
        assert!(matches!(read_bfres(&bad_bom), Err(BfresError::BadBom(_))));
    }
}
