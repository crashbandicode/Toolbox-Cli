//! MC / MCPK (TotK **MeshCodec**) container: inspect, verbatim round-trip,
//! extract, and repack.
//!
//! TotK ships model BFRES as `Model/*.bfres.mc` — a MeshCodec (`MCPK`)
//! container wrapping a **magicless zstd** stream. The leading frame is the
//! BFRES *structure* and needs **no dictionary** (the community's "executable
//! dictionary" lead was a dead end for model `.bfres.mc`; see `codec`). The
//! geometry buffers in the trailing mesh section use a custom MeshCodec
//! encoding (not zstd), decoded in [`geometry`] (still in progress).
//!
//! 1. **Inspect + verbatim byte-identical round-trip** ([`read_mc`] /
//!    [`write_mc`]): parse the MCPK header and re-emit the captured bytes
//!    unchanged — proven across the real corpus (all 12,395 `.mc`).
//! 2. **Extract** ([`extract`]): decode the inner magicless zstd frame to the
//!    BFRES structure with the pure-Rust [`zstd_pure`] codec; validated
//!    byte-exact against the decompressed-`.bfres` oracle.
//! 3. **Repack** ([`repack`]): re-compress an edited BFRES into a `.mc` the
//!    game accepts (byte-identity to Nintendo's encoder is *not* the contract;
//!    the mesh tail is preserved verbatim).
//!
//! ## Header (`MCPK`, little-endian)
//!
//! ```text
//! +0x00 u32 magic 'MCPK'
//! +0x04 u8  version           (1 across the TotK model corpus)
//! +0x05 u8  flags             (<= 1; the decoder rejects > 1)
//! +0x06 u16 reserved          (0)
//! +0x08 u32 size descriptor   -> decompressed_size = (d >> 5) << (d & 0xf)
//! +0x0C ..  magicless-zstd(+dict) stream
//! ```
//!
//! The size descriptor was verified against the oracle on hundreds of real
//! files: the computed size equals the (alignment-padded) decompressed length
//! (the real BFRES `fileSize` is `<=` it; the remainder is zero padding).

mod codec;
mod error;
pub mod geometry;
pub mod mesh;
mod read;
mod write;

pub use codec::{
    compress_stream, decompress_first_frame, decompress_stream, extract, repack, repack_default,
    size_descriptor,
};
pub use error::{McError, Result};
pub use geometry::{
    decode_first_subblock_indices, rans_decode, ForwardReader, SubBlockHeader, TableBuild,
};
pub use mesh::{has_mesh_flag, read_mesh_section, MeshChunk, MeshSection};
pub use read::read_mc;
pub use write::write_mc;

/// The 4-byte MeshCodec magic.
pub const MC_MAGIC: &[u8; 4] = b"MCPK";
/// MCPK header length; the compressed stream begins here.
pub const MC_HEADER_LEN: usize = 0x0C;

/// Decoded MCPK header fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpkHeader {
    /// Format version (`+0x04`; 1 in the TotK model corpus).
    pub version: u8,
    /// Flags (`+0x05`; `<= 1`).
    pub flags: u8,
    /// Raw `+0x08` size descriptor.
    pub size_descriptor: u32,
}

impl McpkHeader {
    /// The decompressed (alignment-padded) size: `(d >> 5) << (d & 0xf)`.
    pub fn decompressed_size(&self) -> usize {
        let shift = (self.size_descriptor & 0xf) as usize;
        ((self.size_descriptor >> 5) as usize) << shift
    }

    /// The alignment shift encoded in the low nibble (output is padded to
    /// `1 << shift`).
    pub fn alignment_shift(&self) -> u32 {
        self.size_descriptor & 0xf
    }
}

/// A parsed MC (`MCPK`) container.
///
/// Retains the original bytes so an unmodified file re-emits byte-identically
/// via [`write_mc`]. The inner zstd stream is **not** decompressed here.
#[derive(Debug, Clone)]
pub struct McFile {
    pub header: McpkHeader,
    /// The original file bytes (for the verbatim [`write_mc`] path).
    pub(crate) raw: Vec<u8>,
}

impl McFile {
    /// The original bytes captured at parse time.
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// The (still-compressed) inner stream — everything after the header.
    pub fn compressed_stream(&self) -> &[u8] {
        &self.raw[MC_HEADER_LEN..]
    }

    /// The decompressed size declared by the header.
    pub fn decompressed_size(&self) -> usize {
        self.header.decompressed_size()
    }
}
