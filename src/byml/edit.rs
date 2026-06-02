//! BYML mutation by path.
//!
//! Edits a single scalar leaf in a decoded [`Byml`] tree, addressed by a
//! [`diff_byml`](super::diff_byml)-style path (`/Key/3/Sub` — hash keys by
//! name, array entries by index; a leading slash is optional). The intended
//! workflow is `read_byml` → [`set_by_path`] → `write_byml_canonical`: the
//! canonical writer is *semantically lossless*, so the edited document
//! re-parses to the mutated tree (it is not byte-identical to the original by
//! contract — BYML byte layout is writer-specific).
//!
//! Only existing **scalar leaves** can be set. The function refuses to descend
//! through a scalar, to index a non-array, or to overwrite a container / binary
//! node with a scalar — so a typo can't silently delete a subtree. Adding new
//! keys / array elements and removing nodes are deliberately out of scope here
//! (a future `add`/`remove` batch).

use super::diff::summary;
use super::error::{BymlError, Result};
use super::Byml;

/// The scalar BYML node kind a `--value` string is parsed into. Containers
/// (array / hash) and binary blobs are not settable through [`set_by_path`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarType {
    /// `0xd0` boolean.
    Bool,
    /// `0xd1` signed 32-bit integer.
    I32,
    /// `0xd3` unsigned 32-bit integer.
    U32,
    /// `0xd2` 32-bit float.
    F32,
    /// `0xd4` signed 64-bit integer.
    I64,
    /// `0xd5` unsigned 64-bit integer.
    U64,
    /// `0xd6` 64-bit double.
    F64,
    /// `0xa0` UTF-8 string.
    String,
    /// `0xff` null.
    Null,
}

impl ScalarType {
    /// Parse a `--type` spelling (case-insensitive; common aliases accepted).
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "bool" | "boolean" => ScalarType::Bool,
            "s32" | "i32" | "int" => ScalarType::I32,
            "u32" | "uint" => ScalarType::U32,
            "f32" | "float" => ScalarType::F32,
            "s64" | "i64" => ScalarType::I64,
            "u64" => ScalarType::U64,
            "f64" | "double" => ScalarType::F64,
            "string" | "str" => ScalarType::String,
            "null" | "none" => ScalarType::Null,
            _ => return Err(BymlError::UnknownScalarType(s.to_string())),
        })
    }

    /// The canonical label used in diagnostics / `--type`.
    pub fn label(self) -> &'static str {
        match self {
            ScalarType::Bool => "bool",
            ScalarType::I32 => "s32",
            ScalarType::U32 => "u32",
            ScalarType::F32 => "f32",
            ScalarType::I64 => "s64",
            ScalarType::U64 => "u64",
            ScalarType::F64 => "f64",
            ScalarType::String => "string",
            ScalarType::Null => "null",
        }
    }
}

/// The [`ScalarType`] a scalar [`Byml`] value corresponds to, or `None` for the
/// container / binary kinds (which can't be type-preserved or overwritten).
fn scalar_type_of(v: &Byml) -> Option<ScalarType> {
    Some(match v {
        Byml::Null => ScalarType::Null,
        Byml::Bool(_) => ScalarType::Bool,
        Byml::I32(_) => ScalarType::I32,
        Byml::U32(_) => ScalarType::U32,
        Byml::F32(_) => ScalarType::F32,
        Byml::I64(_) => ScalarType::I64,
        Byml::U64(_) => ScalarType::U64,
        Byml::F64(_) => ScalarType::F64,
        Byml::String(_) => ScalarType::String,
        Byml::Binary(_) | Byml::Array(_) | Byml::Hash(_) => return None,
    })
}

/// The outcome of a successful [`set_by_path`] edit (the normalized path plus a
/// before/after value summary, e.g. `f32(1) -> f32(1.5)`).
#[derive(Debug, Clone, PartialEq)]
pub struct SetReport {
    /// The normalized target path (`/Key/3/Sub`).
    pub path: String,
    /// A short summary of the value before the edit.
    pub old: String,
    /// A short summary of the value after the edit.
    pub new: String,
}

/// Set the scalar value at `path` in `root`.
///
/// `ty` selects the target node kind; `None` **preserves** the existing leaf's
/// kind (so editing an `f32` keeps it an `f32`, and a `string` keeps its
/// quoting). `raw` is then parsed into that kind. Returns the before/after
/// summary or a typed [`BymlError`] (path not found, index out of range,
/// descending through a scalar, target is a container/binary, or a value that
/// can't be parsed into the target type).
pub fn set_by_path(
    root: &mut Byml,
    path: &str,
    raw: &str,
    ty: Option<ScalarType>,
) -> Result<SetReport> {
    let display = normalize_path(path);
    let node = navigate_mut(root, path, &display)?;

    // The target must be a scalar leaf: never clobber a whole subtree / binary
    // blob, even with an explicit `--type`.
    let preserved = scalar_type_of(node).ok_or_else(|| BymlError::TargetNotScalar {
        path: display.clone(),
        node_type: node.type_name(),
    })?;

    let target = ty.unwrap_or(preserved);
    let old = summary(node);
    let value = parse_value(target, raw)?;
    let new = summary(&value);
    *node = value;
    Ok(SetReport {
        path: display,
        old,
        new,
    })
}

/// Walk `root` to the node addressed by `path`, returning a mutable reference.
fn navigate_mut<'a>(root: &'a mut Byml, path: &str, display: &str) -> Result<&'a mut Byml> {
    let mut node = root;
    for seg in segments(path) {
        node = match node {
            Byml::Hash(entries) => entries
                .iter_mut()
                .find(|(k, _)| k == seg)
                .map(|(_, v)| v)
                .ok_or_else(|| BymlError::PathNotFound {
                    path: display.to_string(),
                })?,
            Byml::Array(items) => {
                let idx = seg
                    .parse::<usize>()
                    .map_err(|_| BymlError::PathIndexNotInteger {
                        segment: seg.to_string(),
                    })?;
                let len = items.len();
                items
                    .get_mut(idx)
                    .ok_or(BymlError::PathIndexOutOfRange { index: idx, len })?
            }
            other => {
                return Err(BymlError::PathThroughScalar {
                    segment: seg.to_string(),
                    node_type: other.type_name(),
                })
            }
        };
    }
    Ok(node)
}

/// Parse `raw` into a scalar [`Byml`] of the requested kind. `null` ignores the
/// value; strings are taken verbatim (only numeric/bool spellings are trimmed).
fn parse_value(ty: ScalarType, raw: &str) -> Result<Byml> {
    let r = raw.trim();
    let mkerr = || BymlError::ValueParse {
        ty: ty.label(),
        value: raw.to_string(),
    };
    Ok(match ty {
        ScalarType::Bool => match r.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Byml::Bool(true),
            "false" | "0" | "no" | "off" => Byml::Bool(false),
            _ => return Err(mkerr()),
        },
        ScalarType::I32 => Byml::I32(r.parse().map_err(|_| mkerr())?),
        ScalarType::U32 => Byml::U32(parse_unsigned(r).ok_or_else(mkerr)?),
        ScalarType::F32 => Byml::F32(r.parse().map_err(|_| mkerr())?),
        ScalarType::I64 => Byml::I64(r.parse().map_err(|_| mkerr())?),
        ScalarType::U64 => Byml::U64(parse_unsigned(r).ok_or_else(mkerr)?),
        ScalarType::F64 => Byml::F64(r.parse().map_err(|_| mkerr())?),
        ScalarType::String => Byml::String(raw.to_string()),
        ScalarType::Null => Byml::Null,
    })
}

/// Parse an unsigned integer, accepting an optional `0x`/`0X` hex prefix.
fn parse_unsigned<T>(s: &str) -> Option<T>
where
    T: std::str::FromStr,
{
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        // FromStr doesn't do radix; route hex through the widest unsigned and
        // narrow via the type's own FromStr on the decimal rendering.
        u64::from_str_radix(hex, 16).ok()?.to_string().parse().ok()
    } else {
        s.parse().ok()
    }
}

/// Split a path into non-empty segments (a leading/trailing/`//` slash is
/// ignored), matching the `diff_byml` convention.
fn segments(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|s| !s.is_empty())
}

/// Render a path in canonical form (`/A/3/B`; the root is `/`).
fn normalize_path(path: &str) -> String {
    let segs: Vec<&str> = segments(path).collect();
    if segs.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", segs.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> Byml {
        Byml::Hash(vec![
            (
                "SystemData".into(),
                Byml::Hash(vec![
                    ("Hp".into(), Byml::U32(120)),
                    ("Speed".into(), Byml::F32(1.0)),
                    ("Name".into(), Byml::String("Link".into())),
                    ("Enabled".into(), Byml::Bool(false)),
                    ("Note".into(), Byml::Null),
                ]),
            ),
            (
                "Drops".into(),
                Byml::Array(vec![Byml::U32(1), Byml::U32(2), Byml::U32(3)]),
            ),
        ])
    }

    fn get<'a>(root: &'a Byml, path: &str) -> &'a Byml {
        let mut node = root;
        for seg in path.split('/').filter(|s| !s.is_empty()) {
            node = match node {
                Byml::Hash(h) => &h.iter().find(|(k, _)| k == seg).unwrap().1,
                Byml::Array(a) => &a[seg.parse::<usize>().unwrap()],
                _ => panic!("descended into scalar at {seg}"),
            };
        }
        node
    }

    #[test]
    fn sets_nested_leaf_type_preserving() {
        let mut t = tree();
        let r = set_by_path(&mut t, "/SystemData/Hp", "999", None).unwrap();
        assert_eq!(r.path, "/SystemData/Hp");
        assert_eq!(r.old, "u32(120)");
        assert_eq!(r.new, "u32(999)");
        assert_eq!(get(&t, "/SystemData/Hp"), &Byml::U32(999));
    }

    #[test]
    fn preserves_float_and_string_and_bool() {
        let mut t = tree();
        set_by_path(&mut t, "SystemData/Speed", "2.5", None).unwrap();
        set_by_path(&mut t, "SystemData/Name", "Zelda", None).unwrap();
        set_by_path(&mut t, "SystemData/Enabled", "true", None).unwrap();
        assert_eq!(get(&t, "/SystemData/Speed"), &Byml::F32(2.5));
        assert_eq!(get(&t, "/SystemData/Name"), &Byml::String("Zelda".into()));
        assert_eq!(get(&t, "/SystemData/Enabled"), &Byml::Bool(true));
    }

    #[test]
    fn sets_array_element_by_index() {
        let mut t = tree();
        set_by_path(&mut t, "/Drops/1", "42", None).unwrap();
        assert_eq!(get(&t, "/Drops/1"), &Byml::U32(42));
    }

    #[test]
    fn type_override_changes_kind() {
        let mut t = tree();
        let r = set_by_path(&mut t, "/SystemData/Hp", "full", Some(ScalarType::String)).unwrap();
        assert_eq!(r.old, "u32(120)");
        assert_eq!(get(&t, "/SystemData/Hp"), &Byml::String("full".into()));
    }

    #[test]
    fn type_override_promotes_null() {
        let mut t = tree();
        set_by_path(&mut t, "/SystemData/Note", "7", Some(ScalarType::I32)).unwrap();
        assert_eq!(get(&t, "/SystemData/Note"), &Byml::I32(7));
    }

    #[test]
    fn u32_accepts_hex() {
        let mut t = tree();
        set_by_path(&mut t, "/SystemData/Hp", "0x10", None).unwrap();
        assert_eq!(get(&t, "/SystemData/Hp"), &Byml::U32(16));
    }

    #[test]
    fn rejects_unknown_key() {
        let mut t = tree();
        assert!(matches!(
            set_by_path(&mut t, "/SystemData/Missing", "1", None),
            Err(BymlError::PathNotFound { .. })
        ));
    }

    #[test]
    fn rejects_index_out_of_range_and_non_integer() {
        let mut t = tree();
        assert!(matches!(
            set_by_path(&mut t, "/Drops/9", "1", None),
            Err(BymlError::PathIndexOutOfRange { index: 9, len: 3 })
        ));
        assert!(matches!(
            set_by_path(&mut t, "/Drops/x", "1", None),
            Err(BymlError::PathIndexNotInteger { .. })
        ));
    }

    #[test]
    fn rejects_descend_through_scalar() {
        let mut t = tree();
        assert!(matches!(
            set_by_path(&mut t, "/SystemData/Hp/Deeper", "1", None),
            Err(BymlError::PathThroughScalar { .. })
        ));
    }

    #[test]
    fn rejects_container_target() {
        let mut t = tree();
        // Targeting a hash/array (incl. the root) must fail rather than nuke it.
        assert!(matches!(
            set_by_path(&mut t, "/SystemData", "1", Some(ScalarType::U32)),
            Err(BymlError::TargetNotScalar { .. })
        ));
        assert!(matches!(
            set_by_path(&mut t, "/", "1", None),
            Err(BymlError::TargetNotScalar { .. })
        ));
    }

    #[test]
    fn rejects_unparseable_value_and_type() {
        let mut t = tree();
        assert!(matches!(
            set_by_path(&mut t, "/SystemData/Hp", "not-a-number", None),
            Err(BymlError::ValueParse { ty: "u32", .. })
        ));
        assert!(matches!(
            ScalarType::parse("vec3"),
            Err(BymlError::UnknownScalarType(_))
        ));
    }
}
