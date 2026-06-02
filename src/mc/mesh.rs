//! TotK MeshCodec **mesh-section framing** (`FMSH`) — the geometry container.
//!
//! A model `.mc` is `[MCPK header][BFRES frame: magicless zstd][mesh section]`.
//! When the BFRES has geometry, the bytes after the zstd frame (4-aligned) are
//! an `FMSH` block: a 34-byte header followed by a chunk stream that decodes to
//! the vertex/index buffers. This module parses that **framing** — the part of
//! the format that is fully reverse-engineered and verified against the
//! `mesh-codec-output` oracle (header field sizes, chunk descriptor, payload
//! location).
//!
//! ## What this does NOT do (yet)
//!
//! It does **not** decode the geometry. The chunk payload is a *custom Nintendo
//! entropy codec* (a meshopt-derived, `clz`-based variable-length bitstream with
//! forward+reverse readers, plus zstd-compressed windows) — distinct from the
//! stock meshoptimizer byte-group format in [`crate::meshopt`]. Porting that
//! decoder is tracked in `local-assets/re/FINDINGS.md`. This parser is the
//! verified container layer the eventual decoder plugs into, and lets
//! `mc-inspect` report whether a model carries a mesh and how big it is.
//!
//! ## FMSH header (little-endian, 34 bytes), verified vs the oracle
//!
//! ```text
//! +0x00 'FMSH'
//! +0x04 u32 version (=1)
//! +0x08 u32 workspace size hint (large; e.g. Bear 0x1372d0)
//! +0x0C u32 compressed payload size (= bytes after the 34-byte header)
//! +0x10 u32 buffer A decoded size (the index buffer)
//! +0x14 u32 buffer B decoded size (the vertex buffer)
//! +0x18 u8  align A      +0x19 u8 align B
//! +0x1a ..  first chunk descriptor (8 bytes: u16 type/val + two u24 sizes)
//! ```

use super::codec::decompress_first_frame;
use super::error::{McError, Result};
use super::McFile;

/// The 4-byte FMSH mesh-section magic.
pub const FMSH_MAGIC: &[u8; 4] = b"FMSH";
/// FMSH header length; the chunk payload begins here.
pub const FMSH_HEADER_LEN: usize = 0x22;

/// The first chunk descriptor (inlined into the FMSH header at `+0x1a`).
///
/// The `u16` header splits as `kind = u16 & 3` / `val = u16 >> 2`; the two
/// 24-bit fields are the byte lengths of the chunk's two sub-streams (whose sum
/// equals the FMSH `compressed_size`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshChunk {
    /// Decoder selector (`u16 & 3`): 2 = the vertex+index decoder, 1 / 0 = the
    /// other meshopt decoder objects (see FINDINGS).
    pub kind: u8,
    /// Auxiliary value (`u16 >> 2`).
    pub val: u32,
    /// Compressed length of sub-stream A (u24).
    pub sub_a_size: u32,
    /// Compressed length of sub-stream B (u24).
    pub sub_b_size: u32,
}

/// A parsed FMSH mesh section (framing only — geometry not decoded).
#[derive(Debug, Clone)]
pub struct MeshSection {
    /// `bfres[0xEE] & 8` — the BFRES "has-mesh" external-flags bit (informational;
    /// detection is by FMSH magic).
    pub has_mesh_flag: bool,
    /// Offset of `FMSH` within the inner stream (i.e. after the MCPK header).
    pub fmsh_offset: usize,
    /// `+0x04` format version.
    pub version: u32,
    /// `+0x08` decoder workspace-size hint.
    pub workspace_hint: u32,
    /// `+0x0C` compressed payload size (chunk bytes after the 34-byte header).
    pub compressed_size: u32,
    /// `+0x10` decoded buffer A size (index buffer).
    pub buf_a_size: u32,
    /// `+0x14` decoded buffer B size (vertex buffer).
    pub buf_b_size: u32,
    /// `+0x18` buffer A alignment.
    pub align_a: u8,
    /// `+0x19` buffer B alignment.
    pub align_b: u8,
    /// The first chunk descriptor (`+0x1a`).
    pub first_chunk: MeshChunk,
    /// Offset of the chunk payload (`fmsh_offset + 34`) within the inner stream.
    pub payload_offset: usize,
}

impl MeshSection {
    /// Total decoded geometry size (buffer A + buffer B), excluding the leading
    /// info header and trailing capacity padding.
    pub fn decoded_geometry_size(&self) -> usize {
        self.buf_a_size as usize + self.buf_b_size as usize
    }
}

/// The BFRES "has-mesh" external-flags bit: bit 3 of byte `+0xEE`.
pub fn has_mesh_flag(bfres: &[u8]) -> bool {
    matches!(bfres.get(0xEE), Some(b) if b & 0x08 != 0)
}

#[inline]
fn u24_le(b: &[u8]) -> u32 {
    b[0] as u32 | (b[1] as u32) << 8 | (b[2] as u32) << 16
}

/// Parse the FMSH mesh-section framing of a model `.mc`, if present.
///
/// Returns `Ok(None)` for a model with no geometry (no `FMSH` after the BFRES
/// frame — e.g. skeleton/animation resources). Decodes only the leading BFRES
/// frame (to locate the 4-aligned FMSH) and parses the header + first chunk
/// descriptor; it does not decode the geometry.
pub fn read_mesh_section(mc: &McFile) -> Result<Option<MeshSection>> {
    let stream = mc.compressed_stream();
    let (bfres, frame_len) = decompress_first_frame(stream, mc.decompressed_size())?;
    let flag = has_mesh_flag(&bfres);

    // FMSH starts at the 4-aligned position after the BFRES frame.
    let fmsh_offset = frame_len + (frame_len.wrapping_neg() & 3);
    if fmsh_offset + FMSH_HEADER_LEN > stream.len() {
        // No room for a mesh section -> this model has none.
        return Ok(None);
    }
    let h = &stream[fmsh_offset..];
    if &h[0..4] != FMSH_MAGIC {
        return Ok(None);
    }

    let version = u32::from_le_bytes([h[4], h[5], h[6], h[7]]);
    let workspace_hint = u32::from_le_bytes([h[8], h[9], h[10], h[11]]);
    let compressed_size = u32::from_le_bytes([h[12], h[13], h[14], h[15]]);
    let buf_a_size = u32::from_le_bytes([h[16], h[17], h[18], h[19]]);
    let buf_b_size = u32::from_le_bytes([h[20], h[21], h[22], h[23]]);
    let align_a = h[24];
    let align_b = h[25];

    let chunk_u16 = u16::from_le_bytes([h[0x1a], h[0x1b]]);
    let first_chunk = MeshChunk {
        kind: (chunk_u16 & 3) as u8,
        val: (chunk_u16 >> 2) as u32,
        sub_a_size: u24_le(&h[0x1c..0x1f]),
        sub_b_size: u24_le(&h[0x1f..0x22]),
    };

    let payload_offset = fmsh_offset + FMSH_HEADER_LEN;

    // Consistency checks against the framing we reverse-engineered: the payload
    // must fit, and the two sub-stream sizes must sum to the payload size.
    if payload_offset + compressed_size as usize > stream.len() {
        return Err(McError::MeshFraming(format!(
            "FMSH payload ({compressed_size} bytes @ {payload_offset}) overruns the {}-byte stream",
            stream.len()
        )));
    }
    let sub_sum = first_chunk.sub_a_size as usize + first_chunk.sub_b_size as usize;
    if sub_sum != compressed_size as usize {
        return Err(McError::MeshFraming(format!(
            "FMSH sub-stream sizes {}+{} != compressed_size {compressed_size}",
            first_chunk.sub_a_size, first_chunk.sub_b_size
        )));
    }

    Ok(Some(MeshSection {
        has_mesh_flag: flag,
        fmsh_offset,
        version,
        workspace_hint,
        compressed_size,
        buf_a_size,
        buf_b_size,
        align_a,
        align_b,
        first_chunk,
        payload_offset,
    }))
}

#[cfg(test)]
mod tests {
    use super::super::codec::compress_stream;
    use super::super::{read_mc, MC_MAGIC};
    use super::*;

    /// Build a synthetic `.mc`: a real magicless-zstd BFRES frame (has-mesh flag
    /// set) + 4-align pad + a hand-built FMSH section with `payload` bytes.
    fn synthetic_mc(
        sub_a: u32,
        sub_b: u32,
        buf_a: u32,
        buf_b: u32,
        compressed_size: u32,
    ) -> Vec<u8> {
        let mut bfres = vec![0u8; 0x100];
        bfres[0..4].copy_from_slice(b"FRES");
        bfres[0xEE] = 0x08; // has-mesh flag
        let frame = compress_stream(&bfres, 3).unwrap();

        let mut inner = frame.clone();
        while !inner.len().is_multiple_of(4) {
            inner.push(0); // 4-align pad before FMSH
        }
        let mut fmsh = Vec::new();
        fmsh.extend_from_slice(FMSH_MAGIC);
        fmsh.extend_from_slice(&1u32.to_le_bytes()); // version
        fmsh.extend_from_slice(&0x1234u32.to_le_bytes()); // workspace hint
        fmsh.extend_from_slice(&compressed_size.to_le_bytes());
        fmsh.extend_from_slice(&buf_a.to_le_bytes());
        fmsh.extend_from_slice(&buf_b.to_le_bytes());
        fmsh.push(8); // align A
        fmsh.push(8); // align B
        let u16h: u16 = (33 << 2) | 2; // kind=2, val=33
        fmsh.extend_from_slice(&u16h.to_le_bytes());
        fmsh.extend_from_slice(&sub_a.to_le_bytes()[..3]);
        fmsh.extend_from_slice(&sub_b.to_le_bytes()[..3]);
        assert_eq!(fmsh.len(), FMSH_HEADER_LEN);
        inner.extend_from_slice(&fmsh);
        inner.extend(std::iter::repeat_n(0u8, compressed_size as usize)); // payload

        let mut mc = Vec::new();
        mc.extend_from_slice(MC_MAGIC);
        mc.push(1); // version
        mc.push(1); // flags
        mc.extend_from_slice(&0u16.to_le_bytes());
        mc.extend_from_slice(&super::super::size_descriptor(0x40000, 12).to_le_bytes());
        mc.extend_from_slice(&inner);
        mc
    }

    #[test]
    fn parses_synthetic_fmsh_framing() {
        let bytes = synthetic_mc(32833, 5094, 16664, 93600, 32833 + 5094);
        let mc = read_mc(&bytes).unwrap();
        let sec = read_mesh_section(&mc).unwrap().expect("mesh section");
        assert_eq!(sec.version, 1);
        assert_eq!(sec.compressed_size, 32833 + 5094);
        assert_eq!(sec.buf_a_size, 16664);
        assert_eq!(sec.buf_b_size, 93600);
        assert_eq!(sec.first_chunk.kind, 2);
        assert_eq!(sec.first_chunk.val, 33);
        assert_eq!(sec.first_chunk.sub_a_size, 32833);
        assert_eq!(sec.first_chunk.sub_b_size, 5094);
        assert_eq!(sec.decoded_geometry_size(), 16664 + 93600);
        assert!(sec.has_mesh_flag);
    }

    #[test]
    fn no_fmsh_returns_none() {
        // A BFRES frame with no trailing FMSH -> None (e.g. a skeleton resource).
        let mut bfres = vec![0u8; 0x100];
        bfres[0..4].copy_from_slice(b"FRES");
        let frame = compress_stream(&bfres, 3).unwrap();
        let mut mc = Vec::new();
        mc.extend_from_slice(MC_MAGIC);
        mc.push(1);
        mc.push(1);
        mc.extend_from_slice(&0u16.to_le_bytes());
        mc.extend_from_slice(&super::super::size_descriptor(0x2000, 12).to_le_bytes());
        mc.extend_from_slice(&frame);
        let mc = read_mc(&mc).unwrap();
        assert!(read_mesh_section(&mc).unwrap().is_none());
    }

    #[test]
    fn rejects_inconsistent_substream_sizes() {
        // compressed_size (150) != sub_a + sub_b (200) -> MeshFraming error.
        let bytes = synthetic_mc(100, 100, 16, 16, 150);
        let mc = read_mc(&bytes).unwrap();
        assert!(matches!(
            read_mesh_section(&mc),
            Err(McError::MeshFraming(_))
        ));
    }
}
