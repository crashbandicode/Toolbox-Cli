//! A pure-Rust **meshoptimizer 0.15** codec, implemented from the MIT
//! specification (`zeux/meshoptimizer`, version `0.15`) — *not* from GPL code.
//!
//! It exists because TotK's **MeshCodec** mesh-geometry stream is produced by
//! Nintendo's `NintendoWare_Meshoptimizer_For_MeshCodec-0_15_0` (the exact
//! version string is embedded in the executable): the trailing `FMSH` vertex /
//! index buffers are meshoptimizer-encoded. This module is the validated
//! *primitive* layer for that codec, mirroring how [`crate::zstd_pure`] is the
//! primitive layer for the `.mc` BFRES frame.
//!
//! It is std + `thiserror` only (no other crate deps) so it can later be lifted
//! into a standalone `meshopt` crate.
//!
//! ## What this is (and isn't)
//!
//! These are the *stock* meshoptimizer stream codecs:
//!
//! * [`decode_vertex_buffer`] / [`encode_vertex_buffer`] — vertex codec (`0xa0`).
//! * [`decode_index_buffer`] / [`encode_index_buffer`] — triangle-list index
//!   codec (`0xe0`, versions 0/1).
//! * [`decode_index_sequence`] / [`encode_index_sequence`] — index sequence
//!   codec (`0xd0`).
//!
//! **Scope / caveat.** TotK's MeshCodec does *not* use this stock byte-group
//! entropy layer directly: its `FMSH` geometry is decoded by a **custom Nintendo
//! entropy codec** (a `clz`-based variable-length bitstream with forward+reverse
//! readers and zstd-compressed windows — see `local-assets/re/FINDINGS.md`),
//! which almost certainly reuses meshopt's *geometry transforms* (vertex
//! delta/zig-zag, index FIFO) but replaces the byte-group entropy. So this
//! module is a faithful **reference codec + encoder foundation**, validated in
//! its own right; it is not yet a drop-in decoder for the game's streams (that
//! requires porting the custom entropy backend).
//!
//! Encode + decode are mutual inverses (`decode(encode(x)) == x`), validated by
//! round-trip on synthetic and real vertex/index data plus hand-computed format
//! vectors.

mod error;
mod index;
mod vertex;

pub use error::{MeshoptError, Result};
pub use index::{
    decode_index_buffer, decode_index_buffer_split, decode_index_buffer_split_used,
    decode_index_sequence, encode_index_buffer, encode_index_sequence,
};
pub use vertex::{decode_vertex_buffer, encode_vertex_buffer};

/// Read `count` little-endian indices of `index_size` (2 or 4) bytes from a
/// decoded index buffer into a `u32` vector — a small convenience for callers
/// (and round-trip tests) that need indices as integers.
pub fn read_indices(bytes: &[u8], count: usize, index_size: usize) -> Result<Vec<u32>> {
    if index_size != 2 && index_size != 4 {
        return Err(MeshoptError::Invalid(format!(
            "index_size {index_size} must be 2 or 4"
        )));
    }
    if bytes.len() < count * index_size {
        return Err(MeshoptError::Truncated {
            what: "index bytes",
            have: bytes.len(),
            need: count * index_size,
        });
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let pos = i * index_size;
        let v = if index_size == 2 {
            u16::from_le_bytes([bytes[pos], bytes[pos + 1]]) as u32
        } else {
            u32::from_le_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
        };
        out.push(v);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
