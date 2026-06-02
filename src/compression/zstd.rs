//! zstd codec for TotK/BOTW `.zs` assets, backed by the pure-Rust [`zstd_pure`]
//! crate (no libzstd / C at runtime) plus a small frame-header parser so we can
//! read the referenced `Dictionary_ID` without decompressing.
//!
//! TotK `.zs` files are single zstd frames; the frame header names which
//! dictionary (if any) is required (`zs.zsdic` = id 1, `pack.zsdic` = id 3,
//! …). We read that id, pick the matching dictionary from a
//! [`crate::compression::dict::DictRegistry`], and decode. Encoding with a
//! dictionary embeds that id back into the frame so the game can reload it.

use crate::error::{Error, Result};

/// Output ceiling for a single decode (decompression-bomb guard). Far above any
/// real TotK `.zs` decompressed size; a frame that pledges/produces more is
/// rejected rather than allowed to exhaust memory.
const MAX_DECOMPRESSED: usize = 1 << 31; // 2 GiB

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
/// frames. Decoded with the pure-Rust [`zstd_pure`] codec.
pub fn decompress(bytes: &[u8], dictionary: Option<&[u8]>) -> Result<Vec<u8>> {
    match dictionary {
        None => zstd_pure::decompress_capped(bytes, MAX_DECOMPRESSED)
            .map_err(|e| Error::Compression(format!("zstd decode: {e}"))),
        Some(dict) => {
            // `Dictionary::parse` auto-detects structured (magic `0xEC30A437`,
            // as TotK's `.zsdic` files are) vs raw content, like libzstd's
            // `ZSTD_dct_auto`.
            let dict = zstd_pure::Dictionary::parse(dict)
                .map_err(|e| Error::Compression(format!("zstd dict parse: {e}")))?;
            zstd_pure::decompress_with_dict(bytes, &dict, MAX_DECOMPRESSED)
                .map_err(|e| Error::Compression(format!("zstd decode (dict): {e}")))
        }
    }
}

/// Compress `payload` into a single zstd frame at `level`. When `dictionary`
/// is supplied, the frame embeds the dictionary's id so the game (and our own
/// [`decompress`]) can reselect it. Encoded with the pure-Rust [`zstd_pure`]
/// codec; the contract is `decompress(compress(x)) == x` (not byte-identity
/// with libzstd's encoder).
pub fn compress(payload: &[u8], dictionary: Option<&[u8]>, level: i32) -> Result<Vec<u8>> {
    match dictionary {
        None => Ok(zstd_pure::compress(payload, level, false, true)),
        Some(dict) => {
            let dict = zstd_pure::Dictionary::parse(dict)
                .map_err(|e| Error::Compression(format!("zstd dict parse: {e}")))?;
            Ok(zstd_pure::compress_with_dict(
                payload, &dict, level, false, true,
            ))
        }
    }
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
