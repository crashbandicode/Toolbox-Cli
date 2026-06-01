//! MC (`MCPK`) writer.
//!
//! Today this is the **verbatim** path only: it re-emits the bytes captured at
//! parse time, so an unmodified container round-trips byte-identically by
//! construction (the inspect/no-op contract). A from-scratch re-pack
//! (`mc-repack`, which re-compresses an edited BFRES) is a separate operation
//! built on the decompression RE — it does not promise byte-identity with
//! Nintendo's encoder.

use super::McFile;

/// Serialize an MC container verbatim — byte-identical for an unmodified file.
pub fn write_mc(mc: &McFile) -> Vec<u8> {
    mc.raw.clone()
}
