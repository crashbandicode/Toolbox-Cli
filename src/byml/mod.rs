//! BYML (a.k.a. BYAML) read + write.
//!
//! BYML is Nintendo's binary-YAML container — the most-edited Switch data
//! format (game parameters, actor/resource databases, cooking/recipe tables,
//! event data, …). A 16-byte header points at two string tables (hash keys +
//! string values) and a root container; nodes are a small tagged-union tree
//! (arrays, hashes, scalars). Both endians are handled (little-endian `YB` for
//! Switch, big-endian `BY` for Wii U / 3DS) and versions `1..=7` (BOTW Switch
//! is v2, Tears of the Kingdom is v7).
//!
//! ## Round-trip discipline
//!
//! Unlike BFLYT/BNTX — whose section layout our writer reproduces exactly —
//! BYML's on-disk bytes depend on writer-specific choices (node de-duplication,
//! the order non-inline nodes are laid out, padding). Reproducing a *specific*
//! tool's bytes from a decoded tree is a separate problem, so [`read_byml`]
//! retains the original bytes and [`write_byml`] re-emits them verbatim for an
//! unmodified document — byte-identical by construction, the same discipline
//! the [`crate::compression`] layer uses for unchanged files. For mutated or
//! synthesized trees, [`write_byml_canonical`] is a from-scratch writer whose
//! guarantee is the *semantic* round-trip `read(write(x)) == read(x)` (it does
//! not chase a specific tool's byte layout). [`diff_byml`] produces a
//! path-keyed structural diff of two trees, and [`set_by_path`] edits a scalar
//! leaf in place (then serialize with [`write_byml_canonical`]).

mod diff;
mod edit;
mod error;
mod read;
mod write;

pub use diff::{diff_byml, BymlDiff, ChangedEntry, DiffEntry};
pub use edit::{set_by_path, ScalarType, SetReport};
pub use error::{BymlError, Result};
pub use read::read_byml;
pub use write::{write_byml, write_byml_canonical};

// ---- BYML node type tags (shared by the reader and writer) ----
pub(crate) const NODE_STRING: u8 = 0xa0;
pub(crate) const NODE_BINARY: u8 = 0xa1;
pub(crate) const NODE_ARRAY: u8 = 0xc0;
pub(crate) const NODE_HASH: u8 = 0xc1;
pub(crate) const NODE_STRING_TABLE: u8 = 0xc2;
pub(crate) const NODE_BOOL: u8 = 0xd0;
pub(crate) const NODE_I32: u8 = 0xd1;
pub(crate) const NODE_F32: u8 = 0xd2;
pub(crate) const NODE_U32: u8 = 0xd3;
pub(crate) const NODE_I64: u8 = 0xd4;
pub(crate) const NODE_U64: u8 = 0xd5;
pub(crate) const NODE_F64: u8 = 0xd6;
pub(crate) const NODE_NULL: u8 = 0xff;

/// Recursion guard for [`read_byml`] (real documents nest only a handful of
/// levels deep; this only trips on cyclic / malicious offsets).
pub(crate) const MAX_DEPTH: usize = 200;

/// A decoded BYML value (a node in the document tree).
///
/// Integer/float widths are kept distinct (matching the on-disk node tags) so
/// a round-trip preserves `s32` vs `u32` vs `s64`/`u64`/`f32`/`f64` exactly
/// rather than collapsing them.
#[derive(Debug, Clone, PartialEq)]
pub enum Byml {
    /// The null node (`0xff`).
    Null,
    /// A boolean (`0xd0`).
    Bool(bool),
    /// A signed 32-bit integer (`0xd1`).
    I32(i32),
    /// An unsigned 32-bit integer (`0xd3`).
    U32(u32),
    /// A 32-bit float (`0xd2`).
    F32(f32),
    /// A signed 64-bit integer (`0xd4`).
    I64(i64),
    /// An unsigned 64-bit integer (`0xd5`).
    U64(u64),
    /// A 64-bit double (`0xd6`).
    F64(f64),
    /// A UTF-8 string (`0xa0`), interned in the string-value table.
    String(String),
    /// Opaque binary data (`0xa1`).
    Binary(Vec<u8>),
    /// An array (`0xc0`) of heterogeneous values.
    Array(Vec<Byml>),
    /// A dictionary/hash (`0xc1`). Stored as ordered `(key, value)` pairs;
    /// BYML keeps keys sorted on disk and we preserve that order so inspect /
    /// round-trip mirror the file.
    Hash(Vec<(String, Byml)>),
}

impl Byml {
    /// A short label for the value's node kind (diagnostics / inspect text).
    pub fn type_name(&self) -> &'static str {
        match self {
            Byml::Null => "null",
            Byml::Bool(_) => "bool",
            Byml::I32(_) => "s32",
            Byml::U32(_) => "u32",
            Byml::F32(_) => "f32",
            Byml::I64(_) => "s64",
            Byml::U64(_) => "u64",
            Byml::F64(_) => "f64",
            Byml::String(_) => "string",
            Byml::Binary(_) => "binary",
            Byml::Array(_) => "array",
            Byml::Hash(_) => "hash",
        }
    }

    /// True for the two container kinds (array / hash).
    pub fn is_container(&self) -> bool {
        matches!(self, Byml::Array(_) | Byml::Hash(_))
    }

    /// The dictionary entries, if this is a [`Byml::Hash`].
    pub fn as_hash(&self) -> Option<&[(String, Byml)]> {
        match self {
            Byml::Hash(h) => Some(h),
            _ => None,
        }
    }

    /// The array elements, if this is a [`Byml::Array`].
    pub fn as_array(&self) -> Option<&[Byml]> {
        match self {
            Byml::Array(a) => Some(a),
            _ => None,
        }
    }

    /// The string, if this is a [`Byml::String`].
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Byml::String(s) => Some(s),
            _ => None,
        }
    }

    /// Look a key up in a [`Byml::Hash`] (linear scan; dictionaries are small).
    pub fn get(&self, key: &str) -> Option<&Byml> {
        match self {
            Byml::Hash(h) => h.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

/// A parsed BYML document.
///
/// Retains the original bytes (`raw`) so an unmodified document re-emits
/// byte-identically via [`write_byml`]. Mutating tools should rebuild from
/// [`root`](BymlDocument::root) and serialize with [`write_byml_canonical`].
#[derive(Debug, Clone)]
pub struct BymlDocument {
    /// Format version (`1..=7`).
    pub version: u16,
    /// `true` for big-endian (`BY`, Wii U / 3DS), `false` for little-endian
    /// (`YB`, Switch).
    pub big_endian: bool,
    /// The root node (always a container in well-formed files; [`Byml::Null`]
    /// for an empty document with a zero root offset).
    pub root: Byml,
    /// The original file bytes, used for the verbatim [`write_byml`] path.
    pub(crate) raw: Vec<u8>,
}

impl BymlDocument {
    /// The root node.
    pub fn root(&self) -> &Byml {
        &self.root
    }

    /// The original bytes captured at parse time.
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }
}
