//! Errors for BFRES (`FRES`, Binary caFe RESource) read/write.
//!
//! Mirrors the per-format error convention used by [`crate::byml`],
//! [`crate::msbt`], [`crate::aamp`], and [`crate::bntx`]: parser variants carry
//! the byte offset (and the field / magic) where decoding failed, so a
//! malformed file reports *where* it broke. Wired into the crate-level
//! [`crate::Error`] via `#[from]`.

use thiserror::Error;

/// An error reading or writing a BFRES container.
#[derive(Debug, Error)]
pub enum BfresError {
    /// Buffer is smaller than the fixed header.
    #[error("not a BFRES: only {0} byte(s), need at least a {len}-byte header", len = super::HEADER_LEN)]
    TooSmall(usize),

    /// The 8-byte magic was not `FRES    ` (`FRES` + four spaces).
    #[error("bad BFRES magic {0:02x?} (expected \"FRES    \")")]
    BadMagic([u8; 8]),

    /// The byte-order mark at 0x0C was neither `FFFE` (LE) nor `FEFF` (BE).
    #[error("bad BFRES byte-order mark 0x{0:04x} (expected FEFF or FFFE)")]
    BadBom(u16),

    /// A read ran past the end of the buffer.
    #[error("truncated BFRES: need {need} byte(s) at offset 0x{offset:x} (file is 0x{len:x})")]
    Truncated {
        offset: usize,
        need: usize,
        len: usize,
    },

    /// The embedded file name was not valid UTF-8 (BFRES names are ASCII).
    #[error("BFRES name at 0x{offset:x} is not valid UTF-8: {source}")]
    NonUtf8 {
        offset: usize,
        source: std::str::Utf8Error,
    },
}

/// Convenience alias for the BFRES module's fallible operations.
pub type Result<T> = std::result::Result<T, BfresError>;
