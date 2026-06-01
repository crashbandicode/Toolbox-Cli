//! Errors for MC / MCPK (TotK MeshCodec) container read.
//!
//! Mirrors the per-format error convention used by [`crate::bfres`],
//! [`crate::aamp`], etc.: parser variants carry the offset/field where decoding
//! failed. Wired into the crate-level [`crate::Error`] via `#[from]`.

use thiserror::Error;

/// An error reading an MC (MeshCodec `MCPK`) container.
#[derive(Debug, Error)]
pub enum McError {
    /// Buffer is smaller than the fixed MCPK header.
    #[error("not an MCPK: only {0} byte(s), need at least a {len}-byte header", len = super::MC_HEADER_LEN)]
    TooSmall(usize),

    /// The 4-byte magic was not `MCPK`.
    #[error("bad MCPK magic {0:02x?} (expected \"MCPK\")")]
    BadMagic([u8; 4]),

    /// The `+0x06` reserved `u16` was non-zero (the game requires 0).
    #[error("MCPK reserved u16 at +0x06 is 0x{0:04x} (expected 0)")]
    BadReserved(u16),

    /// The `+0x05` flags byte exceeded 1 (the game's decoder rejects `> 1`).
    #[error("MCPK flags byte at +0x05 is {0} (expected 0 or 1)")]
    BadFlags(u8),

    /// The decoded decompressed-size descriptor is implausible (0 or absurd).
    #[error("MCPK decompressed-size descriptor 0x{descriptor:08x} -> {size} bytes is implausible")]
    BadSize { descriptor: u32, size: usize },

    /// A zstd (magicless) decompress/compress operation failed.
    #[error("MeshCodec zstd {stage}: {message}")]
    Zstd {
        stage: &'static str,
        message: String,
    },

    /// The trailing FMSH mesh-section framing was malformed (bad magic,
    /// truncated header, or inconsistent sizes).
    #[error("MeshCodec FMSH framing: {0}")]
    MeshFraming(String),

    /// Repack was given an edited BFRES of a different size than the original;
    /// the mesh tail references the original layout, so a resize would break it
    /// (pass `allow_resize` to override at your own risk).
    #[error(
        "mc-repack: edited BFRES is {edited} bytes but the original is {original}; \
         resizing would break the mesh-buffer layout (use --allow-resize to force)"
    )]
    ResizeNotAllowed { original: usize, edited: usize },
}

/// Convenience alias for the MC module's fallible operations.
pub type Result<T> = std::result::Result<T, McError>;
