//! Errors for AAMP (binary parameter archive) read/write.
//!
//! Mirrors the per-format error convention used by [`crate::byml`],
//! [`crate::restbl`], and [`crate::msbt`]: parser variants carry the byte
//! offset (and parameter type / index) where decoding failed. Wired into the
//! crate-level [`crate::Error`] via `#[from]`.

use thiserror::Error;

/// An error reading or writing an AAMP document.
#[derive(Debug, Error)]
pub enum AampError {
    /// Buffer is smaller than the fixed 0x30-byte header.
    #[error("not an AAMP: only {0} byte(s), need at least a 48-byte header")]
    TooSmall(usize),

    /// The 4-byte magic was not `AAMP`.
    #[error("bad AAMP magic {0:02x?} (expected \"AAMP\")")]
    BadMagic([u8; 4]),

    /// The version field was not 2 (only v2 is documented / shipped).
    #[error("unsupported AAMP version {0} (only version 2 is supported)")]
    UnsupportedVersion(u32),

    /// A read ran past the end of the buffer.
    #[error("truncated AAMP: need {need} byte(s) at offset 0x{offset:x} (file is 0x{len:x})")]
    Truncated {
        offset: usize,
        need: usize,
        len: usize,
    },

    /// A parameter carried a type byte outside the documented `0..=20` range.
    #[error("unknown AAMP parameter type {ty} at offset 0x{offset:x}")]
    UnknownType { ty: u8, offset: usize },

    /// A buffer parameter's data offset was too small to hold the preceding
    /// element-count `u32`.
    #[error("AAMP buffer at offset 0x{offset:x} has no room for its count prefix")]
    BufferCountUnderflow { offset: usize },

    /// A string parameter's bytes were not valid UTF-8 (AAMP strings are
    /// ASCII/UTF-8 in practice; the header flags UTF-8).
    #[error("AAMP string at offset 0x{offset:x} is not valid UTF-8: {source}")]
    NonUtf8 {
        offset: usize,
        source: std::str::Utf8Error,
    },

    /// Nesting exceeded the recursion guard (cyclic or maliciously deep
    /// offsets).
    #[error("AAMP nesting exceeds the depth limit {limit} at offset 0x{offset:x}")]
    TooDeep { limit: usize, offset: usize },

    /// A mutation targeted a path/type that doesn't exist or isn't settable.
    #[error("AAMP edit error: {0}")]
    Edit(String),
}

/// Convenience alias for the AAMP module's fallible operations.
pub type Result<T> = std::result::Result<T, AampError>;
