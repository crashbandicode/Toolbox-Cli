//! Errors for SARC read/write.
//!
//! Mirrors the per-format error convention used by [`crate::bflyt`] and
//! [`crate::bntx`]. The parser variants are fully structured (offsets,
//! node indices, byte ranges) so a malformed archive reports *where* it
//! broke; the two filesystem-helper variants ([`SarcError::Io`],
//! [`SarcError::Fs`]) only apply to the directory pack/unpack helpers.
//!
//! This type intentionally references only `std` (no `walkdir`, no other
//! crate) so the SARC reader/writer can later be lifted into a standalone
//! `nx-sarc` crate with a `std + thiserror` core.

use thiserror::Error;

/// An error reading or writing a SARC archive.
#[derive(Debug, Error)]
pub enum SarcError {
    /// Buffer is smaller than the fixed `0x14`-byte header.
    #[error("not a SARC: only {0} byte(s), need at least a 0x14 header")]
    TooSmall(usize),

    /// The 4-byte magic was not `SARC`.
    #[error("bad SARC magic: {0:02x?} (expected \"SARC\")")]
    BadMagic([u8; 4]),

    /// The byte-order mark at `0x06` was neither `FE FF` nor `FF FE`.
    #[error("bad SARC byte-order mark: {0:02x?}")]
    BadBom([u8; 2]),

    /// A required sub-section header (`SFAT`/`SFNT`) was missing.
    #[error("missing {0} section header")]
    MissingSection(&'static str),

    /// A read ran past the end of the buffer.
    #[error("truncated SARC: need {need} byte(s) at offset 0x{offset:x}")]
    Truncated { offset: usize, need: usize },

    /// A node's name offset pointed outside the buffer.
    #[error("SARC name offset 0x{offset:x} is out of bounds")]
    NameOffsetOutOfBounds { offset: usize },

    /// A name had no NUL terminator before end-of-buffer.
    #[error("SARC name at 0x{offset:x} is not NUL-terminated within the file")]
    UnterminatedName { offset: usize },

    /// A name was not valid UTF-8 (real SARC names are ASCII paths).
    #[error("SARC name at 0x{offset:x} is not valid UTF-8: {source}")]
    NonUtf8Name {
        offset: usize,
        source: std::str::Utf8Error,
    },

    /// A node's data end preceded its start.
    #[error("SARC node {index}: data end 0x{end:x} precedes start 0x{start:x}")]
    NodeBackwards {
        index: usize,
        start: usize,
        end: usize,
    },

    /// A node's data range fell outside the buffer.
    #[error(
        "SARC node {index}: data 0x{start:x}..0x{end:x} out of bounds (file is 0x{len:x} byte(s))"
    )]
    NodeOutOfBounds {
        index: usize,
        start: usize,
        end: usize,
        len: usize,
    },

    /// An I/O failure in the directory pack/unpack helpers.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A filesystem-walk / path failure in the directory pack helper.
    #[error("{0}")]
    Fs(String),
}

/// Convenience alias for the SARC module's fallible operations.
pub type Result<T> = std::result::Result<T, SarcError>;
