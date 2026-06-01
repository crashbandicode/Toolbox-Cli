//! MC / MCPK (TotK **MeshCodec**) container read + verbatim round-trip.
//!
//! TotK ships model BFRES as `Model/*.bfres.mc` — a MeshCodec (`MCPK`)
//! container wrapping a **magicless zstd** stream that uses a *raw-content
//! dictionary embedded in the game executable* (`exefs/main`, not RomFS). This
//! module is being built in the cautious, test-driven order the project
//! requires:
//!
//! 1. **Inspect + verbatim byte-identical round-trip** (this module today):
//!    parse the MCPK header and re-emit the captured bytes unchanged. Safe and
//!    proven across the real corpus — it never decompresses or mutates.
//! 2. **Extract** (`mc-extract`): decompress the inner stream to BFRES — needs
//!    the executable dictionary + the exact framing (a focused RE effort, see
//!    `local-assets/re/FINDINGS.md`); validated byte-exact against the
//!    decompressed-`.bfres` oracle.
//! 3. **Repack** (`mc-repack`): re-compress an edited BFRES into a `.mc` the
//!    game accepts (byte-identity to Nintendo's encoder is *not* the contract).
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
mod read;
mod write;

pub use codec::{
    compress_stream, decompress_first_frame, decompress_stream, extract, repack, repack_default,
    size_descriptor,
};
pub use error::{McError, Result};
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
