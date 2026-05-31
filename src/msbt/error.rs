//! Errors for MSBT (LibMessageStudio binary message) read/write.
//!
//! Mirrors the per-format error convention used by [`crate::byml`],
//! [`crate::sarc`], [`crate::bflyt`], and [`crate::bntx`]: parser variants
//! carry the byte offset (and section magic / index) where decoding failed, so
//! a malformed file reports *where* it broke. Wired into the crate-level
//! [`crate::Error`] via `#[from]`.

use thiserror::Error;

/// An error reading or writing an MSBT document.
#[derive(Debug, Error)]
pub enum MsbtError {
    /// Buffer is smaller than the fixed 0x20-byte header.
    #[error("not an MSBT: only {0} byte(s), need at least a 32-byte header")]
    TooSmall(usize),

    /// The 8-byte magic was not `MsgStdBn`.
    #[error("bad MSBT magic {0:02x?} (expected \"MsgStdBn\")")]
    BadMagic([u8; 8]),

    /// The byte-order mark at 0x08 was neither `FFFE` (LE) nor `FEFF` (BE).
    #[error("bad MSBT byte-order mark {0:02x?} (expected FEFF or FFFE)")]
    BadBom([u8; 2]),

    /// The encoding byte at 0x0C was outside the documented `0..=2` range.
    #[error("unsupported MSBT encoding {0} (0=UTF-8, 1=UTF-16, 2=UTF-32)")]
    UnsupportedEncoding(u8),

    /// A read ran past the end of the buffer.
    #[error("truncated MSBT: need {need} byte(s) at offset 0x{offset:x} (file is 0x{len:x})")]
    Truncated {
        offset: usize,
        need: usize,
        len: usize,
    },

    /// A section's declared size ran past the end of the buffer.
    #[error(
        "MSBT section {magic} (#{index}) at 0x{offset:x} declares size {size} \
         which runs past the {len}-byte file"
    )]
    SectionOutOfRange {
        magic: String,
        index: usize,
        offset: usize,
        size: usize,
        len: usize,
    },

    /// A label name or string offset referenced a slot past the end of its
    /// section.
    #[error(
        "MSBT {section} offset {offset} out of range at index {index} \
         (section is {size} byte(s))"
    )]
    OffsetOutOfRange {
        section: &'static str,
        index: usize,
        offset: usize,
        size: usize,
    },

    /// A label name was not valid ASCII/UTF-8 (MSBT labels are ASCII).
    #[error("MSBT label at 0x{offset:x} is not valid UTF-8: {source}")]
    NonUtf8 {
        offset: usize,
        source: std::str::Utf8Error,
    },
}

/// Convenience alias for the MSBT module's fallible operations.
pub type Result<T> = std::result::Result<T, MsbtError>;
