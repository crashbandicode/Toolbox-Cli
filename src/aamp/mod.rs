//! AAMP (binary resource parameter archive) read + write.
//!
//! AAMP is Nintendo's `agl::utl::Parameter` container — the format Breath of
//! the Wild uses for almost every actor parameter (`.bxml`, `.bgparamlist`,
//! `.baiprog`, `.bphysics`, `.bdmgparam`, …), packed inside the actor
//! `.sbactorpack` (Yaz0 SARC) archives. (Tears of the Kingdom replaced AAMP
//! with BYML, so AAMP fixtures come from a BOTW dump.)
//!
//! A document is a tree: a root **Parameter IO** ([`ParameterList`]) holds
//! child lists and **objects** ([`ParameterObject`]); objects hold
//! **parameters** ([`Parameter`]); each parameter is a typed [`Value`]. Keys
//! are stored only as CRC-32 hashes of their names (recovering readable names
//! needs an external name table).
//!
//! ## Format (`AAMP`, version 2, little-endian / UTF-8)
//!
//! - 0x30-byte header: magic `AAMP`, `version: u32 = 2`, `flags: u32`
//!   (bit0 little-endian, bit1 UTF-8), `file_size`, `pio_version`,
//!   `pio_offset` (to the root list, relative to 0x30), then the
//!   list / object / parameter / data-section / string-section counts & sizes.
//!   The Parameter IO **type** string (`"xml"`) sits at 0x30.
//! - **List** node (0xC): name CRC-32, then two packed `u32`s — child-lists
//!   `{offset>>2, count}` and child-objects `{offset>>2, count}` (offsets are
//!   `/4` and relative to the list's own start).
//! - **Object** node (0x8): name CRC-32 + packed `{params offset>>2, count}`.
//! - **Parameter** node (0x8): name CRC-32 + packed `{data offset>>2
//!   (bits 0-23), type (bits 24-31)}`. The data offset is relative to the
//!   parameter's own start; for string types it lands in the string section.
//!
//! ## Round-trip discipline
//!
//! Like [`crate::byml`]/[`crate::msbt`], the on-disk bytes depend on
//! writer-specific choices (value/string de-duplication, node ordering,
//! alignment, the trailing unused-`u32` section), so [`read_aamp`] retains the
//! original bytes and [`write_aamp`] re-emits them **verbatim** — byte-identical
//! by construction for an unmodified document.

mod error;
mod read;
mod write;

pub use error::{AampError, Result};
pub use read::read_aamp;
pub use write::write_aamp;

/// The 4-byte AAMP magic.
pub const AAMP_MAGIC: &[u8; 4] = b"AAMP";
/// Fixed header length; the Parameter IO type string begins here.
pub const HEADER_LEN: usize = 0x30;
/// Recursion guard for [`read_aamp`] (real documents nest only a handful of
/// levels; this only trips on cyclic / malicious offsets).
pub(crate) const MAX_DEPTH: usize = 256;

/// An AAMP parameter's type tag (the high byte of its node's second word).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    Bool = 0,
    F32 = 1,
    Int = 2,
    Vec2 = 3,
    Vec3 = 4,
    Vec4 = 5,
    Color = 6,
    String32 = 7,
    String64 = 8,
    Curve1 = 9,
    Curve2 = 10,
    Curve3 = 11,
    Curve4 = 12,
    BufferInt = 13,
    BufferF32 = 14,
    String256 = 15,
    Quat = 16,
    U32 = 17,
    BufferU32 = 18,
    BufferBinary = 19,
    StringRef = 20,
}

impl ParamType {
    /// Decode a type byte (`0..=20`).
    pub fn from_u8(v: u8) -> Option<Self> {
        use ParamType::*;
        Some(match v {
            0 => Bool,
            1 => F32,
            2 => Int,
            3 => Vec2,
            4 => Vec3,
            5 => Vec4,
            6 => Color,
            7 => String32,
            8 => String64,
            9 => Curve1,
            10 => Curve2,
            11 => Curve3,
            12 => Curve4,
            13 => BufferInt,
            14 => BufferF32,
            15 => String256,
            16 => Quat,
            17 => U32,
            18 => BufferU32,
            19 => BufferBinary,
            20 => StringRef,
            _ => return None,
        })
    }

    /// The on-disk type byte.
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// True for the four string variants (stored in the string section).
    pub fn is_string(self) -> bool {
        matches!(
            self,
            ParamType::String32 | ParamType::String64 | ParamType::String256 | ParamType::StringRef
        )
    }

    /// The number of agl curves for a curve type (`Curve1..=Curve4`), else 0.
    pub fn curve_count(self) -> usize {
        match self {
            ParamType::Curve1 => 1,
            ParamType::Curve2 => 2,
            ParamType::Curve3 => 3,
            ParamType::Curve4 => 4,
            _ => 0,
        }
    }

    /// A short label for diagnostics / inspect.
    pub fn label(self) -> &'static str {
        use ParamType::*;
        match self {
            Bool => "bool",
            F32 => "f32",
            Int => "int",
            Vec2 => "vec2",
            Vec3 => "vec3",
            Vec4 => "vec4",
            Color => "color",
            String32 => "str32",
            String64 => "str64",
            Curve1 => "curve1",
            Curve2 => "curve2",
            Curve3 => "curve3",
            Curve4 => "curve4",
            BufferInt => "buffer_int",
            BufferF32 => "buffer_f32",
            String256 => "str256",
            Quat => "quat",
            U32 => "u32",
            BufferU32 => "buffer_u32",
            BufferBinary => "buffer_binary",
            StringRef => "str_ref",
        }
    }
}

/// One agl curve is two `u32`s followed by 30 `f32`s = 128 bytes.
pub(crate) const CURVE_SIZE: usize = 8 + 30 * 4;

/// A decoded AAMP parameter value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    F32(f32),
    Int(i32),
    U32(u32),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    Color([f32; 4]),
    Quat([f32; 4]),
    /// A string value; `ty` records which string variant it was
    /// (`String32`/`String64`/`String256`/`StringRef`) so the type round-trips.
    Str { ty: ParamType, value: String },
    /// A curve value, kept as raw bytes (`ty` ∈ `Curve1..=Curve4`). Decoding
    /// the individual control points is a follow-up; the bytes round-trip.
    Curve { ty: ParamType, raw: Vec<u8> },
    BufferInt(Vec<i32>),
    BufferF32(Vec<f32>),
    BufferU32(Vec<u32>),
    BufferBinary(Vec<u8>),
}

impl Value {
    /// The [`ParamType`] tag this value serializes as.
    pub fn param_type(&self) -> ParamType {
        match self {
            Value::Bool(_) => ParamType::Bool,
            Value::F32(_) => ParamType::F32,
            Value::Int(_) => ParamType::Int,
            Value::U32(_) => ParamType::U32,
            Value::Vec2(_) => ParamType::Vec2,
            Value::Vec3(_) => ParamType::Vec3,
            Value::Vec4(_) => ParamType::Vec4,
            Value::Color(_) => ParamType::Color,
            Value::Quat(_) => ParamType::Quat,
            Value::Str { ty, .. } => *ty,
            Value::Curve { ty, .. } => *ty,
            Value::BufferInt(_) => ParamType::BufferInt,
            Value::BufferF32(_) => ParamType::BufferF32,
            Value::BufferU32(_) => ParamType::BufferU32,
            Value::BufferBinary(_) => ParamType::BufferBinary,
        }
    }
}

/// A single parameter: a name CRC-32 and its typed value.
#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    /// CRC-32 of the parameter's name.
    pub name_hash: u32,
    pub value: Value,
}

/// A parameter object: a dictionary of [`Parameter`]s under a name CRC-32.
#[derive(Debug, Clone, PartialEq)]
pub struct ParameterObject {
    pub name_hash: u32,
    pub params: Vec<Parameter>,
}

/// A parameter list: child lists + objects under a name CRC-32. The root list
/// is the Parameter IO.
#[derive(Debug, Clone, PartialEq)]
pub struct ParameterList {
    pub name_hash: u32,
    pub lists: Vec<ParameterList>,
    pub objects: Vec<ParameterObject>,
}

impl ParameterList {
    /// Find a child object by name CRC-32 (direct children only).
    pub fn object(&self, name_hash: u32) -> Option<&ParameterObject> {
        self.objects.iter().find(|o| o.name_hash == name_hash)
    }

    /// Find a child list by name CRC-32 (direct children only).
    pub fn list(&self, name_hash: u32) -> Option<&ParameterList> {
        self.lists.iter().find(|l| l.name_hash == name_hash)
    }
}

/// A parsed AAMP document.
///
/// Retains the original bytes (`raw`) so an unmodified document re-emits
/// byte-identically via [`write_aamp`].
#[derive(Debug, Clone)]
pub struct AampDocument {
    /// Data version (`pio_version`; typically 0).
    pub pio_version: u32,
    /// Parameter IO type string (typically `"xml"`).
    pub pio_type: String,
    /// `true` for big-endian (flags bit 0 clear). BOTW is little-endian.
    pub big_endian: bool,
    /// The root Parameter IO list.
    pub root: ParameterList,
    /// The original file bytes, used for the verbatim [`write_aamp`] path.
    pub(crate) raw: Vec<u8>,
}

impl AampDocument {
    /// The original bytes captured at parse time.
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// The root Parameter IO list.
    pub fn root(&self) -> &ParameterList {
        &self.root
    }

    /// `(lists, objects, parameters)` totals across the whole tree.
    pub fn counts(&self) -> (usize, usize, usize) {
        fn walk(l: &ParameterList, c: &mut (usize, usize, usize)) {
            c.0 += 1;
            for o in &l.objects {
                c.1 += 1;
                c.2 += o.params.len();
            }
            for child in &l.lists {
                walk(child, c);
            }
        }
        let mut c = (0, 0, 0);
        walk(&self.root, &mut c);
        c
    }
}
