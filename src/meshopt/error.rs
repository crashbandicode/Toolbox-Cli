//! Typed errors for the pure-Rust meshoptimizer codec.
//!
//! Kept `std`- and `thiserror`-only so this module can be lifted into a
//! standalone crate later (no dependency on the rest of `nx-layout-toolbox`).

use thiserror::Error;

/// An error encoding or decoding a meshoptimizer vertex / index stream.
#[derive(Debug, Error)]
pub enum MeshoptError {
    /// The stream's leading header byte didn't carry the expected tag
    /// (`0xa0` vertex, `0xe0` index, `0xd0` index-sequence) — or the encoded
    /// format version is newer than this decoder supports.
    #[error("meshopt: bad {what} header byte 0x{byte:02x} (or unsupported version)")]
    BadHeader {
        /// Which stream kind was expected.
        what: &'static str,
        /// The offending header byte.
        byte: u8,
    },

    /// The input buffer was too small for the declared element count.
    #[error("meshopt: input too small for {what} ({have} < {need} bytes)")]
    Truncated {
        /// What was being decoded.
        what: &'static str,
        /// Bytes available.
        have: usize,
        /// Minimum bytes required.
        need: usize,
    },

    /// Decoding consumed a different number of bytes than the stream length
    /// implies (extra/insufficient trailing data) — i.e. corruption.
    #[error("meshopt: {what} did not consume the stream cleanly (off by {leftover} bytes)")]
    ExtraBytes {
        /// What was being decoded.
        what: &'static str,
        /// Bytes left unconsumed (or short).
        leftover: usize,
    },

    /// A caller argument violated the codec's contract (e.g. `vertex_size`
    /// not a multiple of 4, or an unsupported `index_size`).
    #[error("meshopt: invalid argument: {0}")]
    Invalid(String),
}

/// Convenience alias for this module's fallible operations.
pub type Result<T> = core::result::Result<T, MeshoptError>;
