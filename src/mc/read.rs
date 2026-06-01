//! MCPK header parser. Bounds-checked; validates the magic, the reserved field,
//! and the flags exactly as the game's decoder does (magic; `+6` u16 == 0;
//! `+5` flags <= 1), then records the size descriptor. Captures the original
//! bytes for the verbatim [`write_mc`](super::write_mc) round-trip; does NOT
//! decompress the inner stream.

use super::error::{McError, Result};
use super::*;

/// Parse an MC (`MCPK`) container header, retaining the original bytes for the
/// verbatim [`write_mc`](super::write_mc) round-trip.
pub fn read_mc(data: &[u8]) -> Result<McFile> {
    if data.len() < MC_HEADER_LEN {
        return Err(McError::TooSmall(data.len()));
    }
    if &data[0..4] != MC_MAGIC {
        let mut m = [0u8; 4];
        m.copy_from_slice(&data[0..4]);
        return Err(McError::BadMagic(m));
    }
    let version = data[4];
    let flags = data[5];
    if flags > 1 {
        return Err(McError::BadFlags(flags));
    }
    let reserved = u16::from_le_bytes([data[6], data[7]]);
    if reserved != 0 {
        return Err(McError::BadReserved(reserved));
    }
    let size_descriptor = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    let header = McpkHeader {
        version,
        flags,
        size_descriptor,
    };
    let size = header.decompressed_size();
    // A model's decompressed BFRES is at least a header; guard against a 0 or
    // wildly-out-of-range descriptor (it shouldn't dwarf the compressed input
    // by more than ~1000x, which would indicate a misparse).
    if size == 0 || size > data.len().saturating_mul(4096) {
        return Err(McError::BadSize {
            descriptor: size_descriptor,
            size,
        });
    }
    Ok(McFile {
        header,
        raw: data.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_mc(size_descriptor: u32) -> Vec<u8> {
        let mut b = vec![0u8; MC_HEADER_LEN];
        b[0..4].copy_from_slice(MC_MAGIC);
        b[4] = 1; // version
        b[5] = 1; // flags
        b[8..12].copy_from_slice(&size_descriptor.to_le_bytes());
        // A few bytes of (pretend) compressed stream so the size guard passes.
        b.extend_from_slice(&[0u8; 64]);
        b
    }

    #[test]
    fn parses_header_and_size_descriptor() {
        // 0x11c -> shift=0xC, size=(0x11c>>5)<<12 = 8<<12 = 0x8000 (verified on
        // real Animal_Bass.Bass.bfres.mc).
        let bytes = minimal_mc(0x11c);
        let mc = read_mc(&bytes).expect("parse");
        assert_eq!(mc.header.version, 1);
        assert_eq!(mc.header.flags, 1);
        assert_eq!(mc.header.alignment_shift(), 0xC);
        assert_eq!(mc.decompressed_size(), 0x8000);
        assert_eq!(mc.compressed_stream().len(), 64);
        // Verbatim round-trip.
        assert_eq!(super::super::write_mc(&mc), bytes);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(matches!(read_mc(&[0u8; 4]), Err(McError::TooSmall(4))));
        let mut bad = vec![0u8; MC_HEADER_LEN + 8];
        bad[0..4].copy_from_slice(b"NOPE");
        assert!(matches!(read_mc(&bad), Err(McError::BadMagic(_))));
        // flags > 1
        let mut badflags = minimal_mc(0x11c);
        badflags[5] = 2;
        assert!(matches!(read_mc(&badflags), Err(McError::BadFlags(2))));
        // reserved != 0
        let mut badres = minimal_mc(0x11c);
        badres[6] = 1;
        assert!(matches!(read_mc(&badres), Err(McError::BadReserved(_))));
        // size descriptor 0 -> size 0
        let mut zero = minimal_mc(0);
        zero[8..12].copy_from_slice(&0u32.to_le_bytes());
        assert!(matches!(read_mc(&zero), Err(McError::BadSize { .. })));
    }
}
