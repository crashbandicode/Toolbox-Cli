//! NSO (Nintendo Switch Object) executable read + segment decompression.
//!
//! NSO is the format of a Switch game's `exefs/main` (and `subsdk*`, `rtld`,
//! `sdk`) module. Its three segments — `.text`, `.rodata`, `.data` — are each
//! optionally **LZ4-block-compressed** (the per-segment flag in the header).
//! This module parses the header and inflates the segments so their contents
//! (strings, constant tables, embedded resources such as the TotK **MeshCodec**
//! zstd dictionary) can be inspected.
//!
//! Reading is the only operation; there is no NSO writer (we never repack an
//! executable). The LZ4 codec is the MIT `lz4_flex` crate (block API).
//!
//! ## Header (`NSO0`, little-endian, 0x100 bytes)
//!
//! ```text
//! 0x00 char[4] magic "NSO0"
//! 0x04 u32     version (0)
//! 0x08 u32     reserved
//! 0x0C u32     flags         (bit0 .text / bit1 .rodata / bit2 .data compressed)
//! 0x10 Segment .text         { fileOffset u32, memoryOffset u32, decompressedSize u32 }
//! 0x20 Segment .rodata
//! 0x30 Segment .data
//! 0x60 u32     .text   compressedSize
//! 0x64 u32     .rodata compressedSize
//! 0x68 u32     .data   compressedSize
//! ```
//!
//! (The remaining header bytes — module id, BuildID, dynstr/dynsym extents,
//! per-segment SHA-256 — are not needed for segment inflation.)

use thiserror::Error;

/// The 4-byte NSO magic.
pub const NSO_MAGIC: &[u8; 4] = b"NSO0";
/// NSO header length.
pub const NSO_HEADER_LEN: usize = 0x100;

/// An error reading an NSO module.
#[derive(Debug, Error)]
pub enum NsoError {
    /// Buffer is smaller than the fixed 0x100-byte header.
    #[error("not an NSO: only {0} byte(s), need at least a 0x100-byte header")]
    TooSmall(usize),

    /// The 4-byte magic was not `NSO0`.
    #[error("bad NSO magic {0:02x?} (expected \"NSO0\")")]
    BadMagic([u8; 4]),

    /// A segment's compressed range ran past the end of the file.
    #[error(
        "NSO {seg} segment compressed range [0x{off:x}, 0x{off:x}+0x{len:x}) \
         runs past the {file_len}-byte file"
    )]
    SegmentOutOfRange {
        seg: &'static str,
        off: usize,
        len: usize,
        file_len: usize,
    },

    /// LZ4 block decompression of a segment failed.
    #[error("NSO {seg} segment LZ4 decompress failed: {msg}")]
    Lz4 { seg: &'static str, msg: String },

    /// A segment decompressed to a different size than the header declared.
    #[error("NSO {seg} segment decompressed to {got} bytes, header declared {want}")]
    SizeMismatch {
        seg: &'static str,
        got: usize,
        want: usize,
    },
}

/// Convenience alias for the NSO module's fallible operations.
pub type Result<T> = std::result::Result<T, NsoError>;

/// One NSO segment header (`.text` / `.rodata` / `.data`).
#[derive(Debug, Clone, Copy)]
pub struct Segment {
    pub file_offset: u32,
    pub memory_offset: u32,
    pub decompressed_size: u32,
    pub compressed_size: u32,
    pub is_compressed: bool,
}

/// A parsed NSO module with its three segments inflated.
#[derive(Debug, Clone)]
pub struct NsoFile {
    pub version: u32,
    pub flags: u32,
    pub text: Segment,
    pub rodata: Segment,
    pub data: Segment,
    /// Inflated `.text` bytes.
    pub text_bytes: Vec<u8>,
    /// Inflated `.rodata` bytes.
    pub rodata_bytes: Vec<u8>,
    /// Inflated `.data` bytes.
    pub data_bytes: Vec<u8>,
}

impl NsoFile {
    /// The inflated bytes of a named segment (`"text"` / `"rodata"` / `"data"`).
    pub fn segment_bytes(&self, name: &str) -> Option<&[u8]> {
        match name {
            "text" => Some(&self.text_bytes),
            "rodata" => Some(&self.rodata_bytes),
            "data" => Some(&self.data_bytes),
            _ => None,
        }
    }
}

/// Parse an NSO module and inflate its three segments.
pub fn read_nso(data: &[u8]) -> Result<NsoFile> {
    if data.len() < NSO_HEADER_LEN {
        return Err(NsoError::TooSmall(data.len()));
    }
    if &data[0..4] != NSO_MAGIC {
        let mut m = [0u8; 4];
        m.copy_from_slice(&data[0..4]);
        return Err(NsoError::BadMagic(m));
    }
    let version = read_u32(data, 0x04);
    let flags = read_u32(data, 0x0C);

    let seg = |hoff: usize, comp_off: usize, flag_bit: u32| Segment {
        file_offset: read_u32(data, hoff),
        memory_offset: read_u32(data, hoff + 4),
        decompressed_size: read_u32(data, hoff + 8),
        compressed_size: read_u32(data, comp_off),
        is_compressed: (flags & (1 << flag_bit)) != 0,
    };
    let text = seg(0x10, 0x60, 0);
    let rodata = seg(0x20, 0x64, 1);
    let data_seg = seg(0x30, 0x68, 2);

    let text_bytes = inflate_segment(data, &text, "text")?;
    let rodata_bytes = inflate_segment(data, &rodata, "rodata")?;
    let data_bytes = inflate_segment(data, &data_seg, "data")?;

    Ok(NsoFile {
        version,
        flags,
        text,
        rodata,
        data: data_seg,
        text_bytes,
        rodata_bytes,
        data_bytes,
    })
}

/// Inflate one segment: slice out its compressed bytes, then LZ4-decompress
/// (when the per-segment flag is set) to the declared size, else copy verbatim.
fn inflate_segment(data: &[u8], seg: &Segment, name: &'static str) -> Result<Vec<u8>> {
    let off = seg.file_offset as usize;
    let clen = seg.compressed_size as usize;
    let dlen = seg.decompressed_size as usize;
    let end =
        off.checked_add(clen)
            .filter(|&e| e <= data.len())
            .ok_or(NsoError::SegmentOutOfRange {
                seg: name,
                off,
                len: clen,
                file_len: data.len(),
            })?;
    let raw = &data[off..end];

    if !seg.is_compressed {
        return Ok(raw.to_vec());
    }
    if dlen == 0 {
        return Ok(Vec::new());
    }
    let out = lz4_flex::block::decompress(raw, dlen).map_err(|e| NsoError::Lz4 {
        seg: name,
        msg: e.to_string(),
    })?;
    if out.len() != dlen {
        return Err(NsoError::SizeMismatch {
            seg: name,
            got: out.len(),
            want: dlen,
        });
    }
    Ok(out)
}

fn read_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lay out a 0x100-byte NSO0 header followed by the three segments'
    /// (already-encoded) payloads back-to-back, with the given flags.
    fn build_nso(flags: u32, segs: [(&[u8], u32); 3]) -> Vec<u8> {
        let mut b = vec![0u8; NSO_HEADER_LEN];
        b[0..4].copy_from_slice(NSO_MAGIC);
        b[0x0C..0x10].copy_from_slice(&flags.to_le_bytes());
        let mut off = NSO_HEADER_LEN as u32;
        let hoffs = [0x10usize, 0x20, 0x30];
        let coffs = [0x60usize, 0x64, 0x68];
        let mut payloads = Vec::new();
        for (i, (payload, decompressed_size)) in segs.iter().enumerate() {
            b[hoffs[i]..hoffs[i] + 4].copy_from_slice(&off.to_le_bytes()); // fileOffset
            b[hoffs[i] + 4..hoffs[i] + 8].copy_from_slice(&0u32.to_le_bytes()); // memoryOffset
            b[hoffs[i] + 8..hoffs[i] + 12].copy_from_slice(&decompressed_size.to_le_bytes());
            b[coffs[i]..coffs[i] + 4].copy_from_slice(&(payload.len() as u32).to_le_bytes());
            off += payload.len() as u32;
            payloads.extend_from_slice(payload);
        }
        b.extend_from_slice(&payloads);
        b
    }

    #[test]
    fn parses_uncompressed_segments() {
        let text = b"TEXT-segment".as_slice();
        let rodata = b"RODATA".as_slice();
        let data = b"DATA!".as_slice();
        let nso = build_nso(
            0, // no segment compressed
            [
                (text, text.len() as u32),
                (rodata, rodata.len() as u32),
                (data, data.len() as u32),
            ],
        );
        let f = read_nso(&nso).expect("parse");
        assert_eq!(f.flags, 0);
        assert_eq!(f.text_bytes, text);
        assert_eq!(f.rodata_bytes, rodata);
        assert_eq!(f.data_bytes, data);
        assert_eq!(f.segment_bytes("rodata"), Some(rodata));
        assert_eq!(f.segment_bytes("nope"), None);
    }

    #[test]
    fn inflates_lz4_compressed_segment() {
        // A buffer with enough redundancy that LZ4 actually compresses it.
        let original: Vec<u8> = b"MeshCodecMeshCodecMeshCodec_dictionary_payload_payload_payload"
            .iter()
            .cycle()
            .take(4096)
            .copied()
            .collect();
        let comp = lz4_flex::block::compress(&original);
        assert!(comp.len() < original.len(), "expected LZ4 to shrink it");
        let empty = b"".as_slice();
        // flags bit1 (rodata) compressed; text/data empty + uncompressed.
        let nso = build_nso(
            0b010,
            [
                (empty, 0),
                (comp.as_slice(), original.len() as u32),
                (empty, 0),
            ],
        );
        let f = read_nso(&nso).expect("parse");
        assert_eq!(f.rodata_bytes, original, "LZ4 segment did not round-trip");
        assert!(f.rodata.is_compressed);
        assert!(!f.text.is_compressed);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(matches!(read_nso(&[0u8; 8]), Err(NsoError::TooSmall(8))));
        let mut bad = vec![0u8; NSO_HEADER_LEN];
        bad[0..4].copy_from_slice(b"FAIL");
        assert!(matches!(read_nso(&bad), Err(NsoError::BadMagic(_))));
        // A compressed-size that overruns the file.
        let mut overrun = vec![0u8; NSO_HEADER_LEN];
        overrun[0..4].copy_from_slice(NSO_MAGIC);
        overrun[0x10..0x14].copy_from_slice(&(NSO_HEADER_LEN as u32).to_le_bytes());
        overrun[0x60..0x64].copy_from_slice(&0x10000u32.to_le_bytes()); // huge text comp size
        assert!(matches!(
            read_nso(&overrun),
            Err(NsoError::SegmentOutOfRange { .. })
        ));
    }
}
