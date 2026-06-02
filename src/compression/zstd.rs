//! Thin wrapper over the `zstd` crate (vendored libzstd, BSD-3 — GPL-free)
//! plus a small pure-Rust parser for the zstd *frame header* so we can read
//! the referenced `Dictionary_ID` without decompressing.
//!
//! TotK `.zs` files are single zstd frames; the frame header names which
//! dictionary (if any) is required (`zs.zsdic` = id 1, `pack.zsdic` = id 3,
//! …). We read that id, pick the matching dictionary from a
//! [`crate::compression::dict::DictRegistry`], and decode. Encoding with a
//! dictionary embeds that id back into the frame so the game can reload it.

use std::io::{Cursor, Read};

use crate::error::{Error, Result};

// Refer to the external crate explicitly to avoid any confusion with this
// module's own name (`crate::compression::zstd`).
use ::zstd as libzstd;

/// zstd frame magic, little-endian `0xFD2FB528`.
pub const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// True if `bytes` begins with the zstd frame magic.
pub fn is_zstd(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[0..4] == ZSTD_MAGIC
}

/// Read the `Dictionary_ID` a zstd frame references, or `0` if the frame
/// uses no dictionary. Implements the RFC 8878 frame-header layout up to the
/// dictionary-id field (magic → frame-header descriptor → optional window
/// descriptor → dictionary id). Pure Rust; no decompression.
pub fn frame_dictionary_id(bytes: &[u8]) -> Result<u32> {
    if !is_zstd(bytes) {
        return Err(Error::Compression("not a zstd frame (bad magic)".into()));
    }
    let descriptor = *bytes
        .get(4)
        .ok_or_else(|| Error::Compression("zstd: truncated frame header descriptor".into()))?;

    let dict_id_flag = descriptor & 0x03; // bits 1-0
    let single_segment = descriptor & 0x20 != 0; // bit 5
    let did_size = match dict_id_flag {
        0 => 0,
        1 => 1,
        2 => 2,
        3 => 4,
        _ => unreachable!("2-bit field"),
    };

    // The window descriptor byte is present only when Single_Segment is 0.
    let mut offset = 5;
    if !single_segment {
        offset += 1;
    }
    if did_size == 0 {
        return Ok(0);
    }
    let raw = bytes
        .get(offset..offset + did_size)
        .ok_or_else(|| Error::Compression("zstd: truncated frame header (dictionary id)".into()))?;
    let mut id = 0u32;
    for (i, b) in raw.iter().enumerate() {
        id |= (*b as u32) << (8 * i); // little-endian
    }
    Ok(id)
}

/// Decompress a single zstd frame. `dictionary` must be supplied iff the
/// frame references one (see [`frame_dictionary_id`]); pass `None` for plain
/// frames.
pub fn decompress(bytes: &[u8], dictionary: Option<&[u8]>) -> Result<Vec<u8>> {
    match dictionary {
        None => {
            libzstd::decode_all(bytes).map_err(|e| Error::Compression(format!("zstd decode: {e}")))
        }
        Some(dict) => {
            let mut decoder =
                libzstd::stream::read::Decoder::with_dictionary(Cursor::new(bytes), dict)
                    .map_err(|e| Error::Compression(format!("zstd decode (dict): {e}")))?;
            let mut out = Vec::new();
            decoder
                .read_to_end(&mut out)
                .map_err(|e| Error::Compression(format!("zstd decode (dict): {e}")))?;
            Ok(out)
        }
    }
}

/// Compress `payload` into a single zstd frame at `level`. When `dictionary`
/// is supplied, the frame embeds the dictionary's id so the game (and our
/// own [`decompress`]) can reselect it.
pub fn compress(payload: &[u8], dictionary: Option<&[u8]>, level: i32) -> Result<Vec<u8>> {
    let mut compressor = match dictionary {
        None => libzstd::bulk::Compressor::new(level),
        Some(dict) => libzstd::bulk::Compressor::with_dictionary(level, dict),
    }
    .map_err(|e| Error::Compression(format!("zstd encoder init: {e}")))?;
    compressor
        .compress(payload)
        .map_err(|e| Error::Compression(format!("zstd encode: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_round_trip() {
        let data = b"the quick brown fox jumps over the lazy dog".repeat(50);
        let packed = compress(&data, None, 3).unwrap();
        assert!(is_zstd(&packed));
        assert_eq!(frame_dictionary_id(&packed).unwrap(), 0);
        assert_eq!(decompress(&packed, None).unwrap(), data);
    }

    #[test]
    fn raw_dictionary_round_trip() {
        // A raw-content dictionary is just bytes shared by encoder + decoder
        // (frame carries dict id 0). Proves the wrapper threads a dictionary
        // through both directions. The non-zero-id path is covered
        // deterministically by `frame_header_parsing_cases` and, on real
        // libzstd frames, by the fixture-gated TotK tests.
        let dict = b"common-prefix-payload-data-shared-across-files".repeat(8);
        let payload = b"common-prefix-payload-data and a unique tail here".to_vec();
        let packed = compress(&payload, Some(&dict), 3).unwrap();
        assert_eq!(decompress(&packed, Some(&dict)).unwrap(), payload);
    }

    #[test]
    fn frame_header_parsing_cases() {
        // Hand-built headers covering the dict-id-flag and single-segment
        // permutations we see in TotK (matches the real ZsDic/blarc/pack.zs
        // frames inspected on disk).
        // Single-segment, no dict (ZsDic.pack.zs: descriptor 0xA0).
        let f0 = [0x28, 0xB5, 0x2F, 0xFD, 0xA0, 0x00, 0x00];
        assert_eq!(frame_dictionary_id(&f0).unwrap(), 0);
        // Single-segment, 1-byte dict id = 1 (blarc.zs: descriptor 0x61).
        let f1 = [0x28, 0xB5, 0x2F, 0xFD, 0x61, 0x01, 0x00];
        assert_eq!(frame_dictionary_id(&f1).unwrap(), 1);
        // Window descriptor present, 1-byte dict id = 3 (pack.zs: 0x81).
        let f3 = [0x28, 0xB5, 0x2F, 0xFD, 0x81, 0x68, 0x03, 0x00];
        assert_eq!(frame_dictionary_id(&f3).unwrap(), 3);
        // 4-byte dict id, window descriptor present (dict_id_flag = 3).
        let f4 = [0x28, 0xB5, 0x2F, 0xFD, 0x83, 0x00, 0x78, 0x56, 0x34, 0x12];
        assert_eq!(frame_dictionary_id(&f4).unwrap(), 0x1234_5678);
    }

    #[test]
    fn rejects_non_zstd() {
        assert!(frame_dictionary_id(b"Yaz0....").is_err());
        assert!(!is_zstd(b"SARC"));
    }
}
