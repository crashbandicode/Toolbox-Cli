//! Errors for BYML (binary YAML) read/write.
//!
//! Mirrors the per-format error convention used by [`crate::sarc`],
//! [`crate::bflyt`], and [`crate::bntx`]: the parser variants carry the
//! byte offset (and node type / table / index) where decoding failed, so a
//! malformed document reports *where* it broke. Wired into the crate-level
//! [`crate::Error`] via `#[from]`.

use thiserror::Error;

/// An error reading or writing a BYML document.
#[derive(Debug, Error)]
pub enum BymlError {
    /// Buffer is smaller than the fixed 16-byte header.
    #[error("not a BYML: only {0} byte(s), need at least a 16-byte header")]
    TooSmall(usize),

    /// The 2-byte magic was neither `BY` (big-endian) nor `YB` (little-endian).
    #[error("bad BYML magic {0:02x?} (expected \"BY\" or \"YB\")")]
    BadMagic([u8; 2]),

    /// The version word was outside the documented `1..=7` range.
    #[error("unsupported BYML version {0} (supported range is 1..=7)")]
    UnsupportedVersion(u16),

    /// A read ran past the end of the buffer.
    #[error("truncated BYML: need {need} byte(s) at offset 0x{offset:x} (file is 0x{len:x})")]
    Truncated {
        offset: usize,
        need: usize,
        len: usize,
    },

    /// A header offset pointed at something that wasn't a string-table node.
    #[error("BYML {table} table at 0x{offset:x}: expected node type 0xc2, got 0x{node_type:02x}")]
    BadStringTable {
        table: &'static str,
        offset: usize,
        node_type: u8,
    },

    /// The root (or a referenced container) offset pointed at a non-container
    /// node type.
    #[error(
        "BYML node at 0x{offset:x}: expected a container (array 0xc0 / hash 0xc1), \
         got node type 0x{node_type:02x}"
    )]
    NotAContainer { offset: usize, node_type: u8 },

    /// A string/hash-key index referenced a slot past the end of its table.
    #[error("BYML {table} index {index} out of range ({count} entr(ies) in the table)")]
    StringIndexOutOfRange {
        table: &'static str,
        index: u32,
        count: usize,
    },

    /// A node carried a type byte we don't decode.
    #[error("BYML unknown node type 0x{node_type:02x} at offset 0x{offset:x}")]
    UnknownNodeType { node_type: u8, offset: usize },

    /// A string-table entry was not valid UTF-8 (BYML keys/strings are ASCII
    /// in practice).
    #[error("BYML string at 0x{offset:x} is not valid UTF-8: {source}")]
    NonUtf8 {
        offset: usize,
        source: std::str::Utf8Error,
    },

    /// Nesting exceeded the recursion guard (cyclic or maliciously deep
    /// offsets).
    #[error("BYML nesting exceeds the depth limit {limit} at offset 0x{offset:x}")]
    TooDeep { limit: usize, offset: usize },

    /// The canonical writer was handed a root that isn't an array/hash (BYML
    /// requires a container at the root).
    #[error("BYML root must be a container (array/hash), got {0}")]
    NonContainerRoot(&'static str),

    // ---- mutation (`set_by_path`) ----
    /// A path segment named a hash key that doesn't exist (or an array index
    /// past a missing parent).
    #[error("BYML path {path:?} not found")]
    PathNotFound { path: String },

    /// A path segment indexing an array wasn't a non-negative integer.
    #[error("BYML path segment {segment:?} is not a valid array index")]
    PathIndexNotInteger { segment: String },

    /// An array index ran past the end of the array.
    #[error("BYML array index {index} out of range ({len} element(s))")]
    PathIndexOutOfRange { index: usize, len: usize },

    /// The path tried to descend into a scalar leaf (only arrays/hashes have
    /// children).
    #[error("BYML path segment {segment:?} descends into a {node_type} (not a container)")]
    PathThroughScalar {
        segment: String,
        node_type: &'static str,
    },

    /// The target node is a container or binary blob; [`set_by_path`] only
    /// overwrites scalar leaves.
    #[error("BYML path {path:?} targets a {node_type}; only scalar leaves can be set")]
    TargetNotScalar {
        path: String,
        node_type: &'static str,
    },

    /// The `--type` spelling wasn't a known scalar kind.
    #[error(
        "unknown BYML scalar type {0:?} (expected one of bool, s32, u32, f32, s64, u64, f64, \
         string, null)"
    )]
    UnknownScalarType(String),

    /// The supplied value couldn't be parsed into the target scalar type.
    #[error("cannot parse {value:?} as BYML {ty}")]
    ValueParse { ty: &'static str, value: String },
}

/// Convenience alias for the BYML module's fallible operations.
pub type Result<T> = std::result::Result<T, BymlError>;
