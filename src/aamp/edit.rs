//! AAMP scalar mutation by path.
//!
//! Edits a single parameter's value, addressed by a `/`-separated name path:
//! zero or more **list** names (descending from the root Parameter IO), then an
//! **object** name, then the **parameter** name. Each segment is matched by its
//! CRC-32 (names are hashed with [`crate::restbl::crc32`]); a `0x…` segment is
//! taken as a raw hash. The intended workflow is `read_aamp` → [`set_by_path`]
//! → `write_aamp_canonical`.
//!
//! The edit is **type-preserving**: the new value is parsed into the existing
//! parameter's type (so editing an `f32` keeps it an `f32`, a `str32` stays a
//! `str32`). Curve and buffer parameters aren't settable this way (clear
//! error) — keeping the round-trip discipline.

use super::error::{AampError, Result};
use super::{ParameterList, Value};
use crate::restbl::crc32;

/// The outcome of a successful [`set_by_path`] edit.
#[derive(Debug, Clone, PartialEq)]
pub struct SetReport {
    pub path: String,
    pub old: String,
    pub new: String,
}

/// Resolve a path segment to a CRC-32 hash: a `0x…` segment is a raw hash,
/// otherwise the segment text is hashed.
fn resolve_hash(seg: &str) -> u32 {
    let t = seg.trim();
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        if let Ok(v) = u32::from_str_radix(hex, 16) {
            return v;
        }
    }
    crc32(t.as_bytes())
}

/// Set the value of the parameter addressed by `path`, parsing `raw` into the
/// parameter's existing type.
pub fn set_by_path(root: &mut ParameterList, path: &str, raw: &str) -> Result<SetReport> {
    let segs: Vec<&str> = path.split('/').filter(|s| !s.trim().is_empty()).collect();
    if segs.len() < 2 {
        return Err(AampError::Edit(format!(
            "path {path:?} must be /<lists…>/<object>/<param> (at least an object and a parameter)"
        )));
    }
    let param_seg = segs[segs.len() - 1];
    let object_seg = segs[segs.len() - 2];
    let list_segs = &segs[..segs.len() - 2];

    let mut cur: &mut ParameterList = root;
    for ls in list_segs {
        let h = resolve_hash(ls);
        cur = cur
            .lists
            .iter_mut()
            .find(|l| l.name_hash == h)
            .ok_or_else(|| AampError::Edit(format!("list {ls:?} (0x{h:08x}) not found")))?;
    }
    let oh = resolve_hash(object_seg);
    let obj = cur
        .objects
        .iter_mut()
        .find(|o| o.name_hash == oh)
        .ok_or_else(|| AampError::Edit(format!("object {object_seg:?} (0x{oh:08x}) not found")))?;
    let ph = resolve_hash(param_seg);
    let param = obj
        .params
        .iter_mut()
        .find(|p| p.name_hash == ph)
        .ok_or_else(|| AampError::Edit(format!("parameter {param_seg:?} (0x{ph:08x}) not found")))?;

    let old = param.value.summary();
    param.value = parse_into_type(&param.value, raw)?;
    let new = param.value.summary();
    Ok(SetReport {
        path: format!("/{}", segs.join("/")),
        old,
        new,
    })
}

/// Parse `raw` into a value matching `existing`'s type (type-preserving).
fn parse_into_type(existing: &Value, raw: &str) -> Result<Value> {
    let r = raw.trim();
    let err = || AampError::Edit(format!("cannot parse {raw:?} as {}", existing.param_type().label()));
    Ok(match existing {
        Value::Bool(_) => Value::Bool(match r.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => true,
            "false" | "0" | "no" | "off" => false,
            _ => return Err(err()),
        }),
        Value::F32(_) => Value::F32(r.parse().map_err(|_| err())?),
        Value::Int(_) => Value::Int(r.parse().map_err(|_| err())?),
        Value::U32(_) => Value::U32(parse_u32(r).ok_or_else(err)?),
        Value::Vec2(_) => Value::Vec2(parse_floats::<2>(r)?),
        Value::Vec3(_) => Value::Vec3(parse_floats::<3>(r)?),
        Value::Vec4(_) => Value::Vec4(parse_floats::<4>(r)?),
        Value::Color(_) => Value::Color(parse_floats::<4>(r)?),
        Value::Quat(_) => Value::Quat(parse_floats::<4>(r)?),
        Value::Str { ty, .. } => Value::Str {
            ty: *ty,
            value: raw.to_string(),
        },
        Value::Curve { .. }
        | Value::BufferInt(_)
        | Value::BufferF32(_)
        | Value::BufferU32(_)
        | Value::BufferBinary(_) => {
            return Err(AampError::Edit(format!(
                "parameter type {} is not settable via aamp-set (curve/buffer)",
                existing.param_type().label()
            )))
        }
    })
}

fn parse_u32(s: &str) -> Option<u32> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

/// Parse `N` comma-separated floats (e.g. `"1.0, 2.0, 3.0"`).
fn parse_floats<const N: usize>(raw: &str) -> Result<[f32; N]> {
    let parts: Vec<&str> = raw.split(',').map(|p| p.trim()).collect();
    if parts.len() != N {
        return Err(AampError::Edit(format!(
            "expected {N} comma-separated floats, got {} in {raw:?}",
            parts.len()
        )));
    }
    let mut out = [0f32; N];
    for (i, p) in parts.iter().enumerate() {
        out[i] = p
            .parse()
            .map_err(|_| AampError::Edit(format!("{p:?} is not a float in {raw:?}")))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aamp::{Parameter, ParameterObject, ParamType};

    fn tree() -> ParameterList {
        ParameterList {
            name_hash: crc32(b"param_root"),
            lists: vec![ParameterList {
                name_hash: crc32(b"AI"),
                lists: Vec::new(),
                objects: vec![ParameterObject {
                    name_hash: crc32(b"Logic"),
                    params: vec![Parameter {
                        name_hash: crc32(b"Speed"),
                        value: Value::F32(1.0),
                    }],
                }],
            }],
            objects: vec![ParameterObject {
                name_hash: crc32(b"WeaponCommon"),
                params: vec![
                    Parameter { name_hash: crc32(b"Atk"), value: Value::Int(10) },
                    Parameter {
                        name_hash: crc32(b"Name"),
                        value: Value::Str { ty: ParamType::String64, value: "old".into() },
                    },
                    Parameter { name_hash: crc32(b"Tint"), value: Value::Color([0.0, 0.0, 0.0, 1.0]) },
                    Parameter { name_hash: crc32(b"Curve"), value: Value::Curve { ty: ParamType::Curve1, raw: vec![0; 128] } },
                ],
            }],
        }
    }

    fn find_value(root: &ParameterList, obj: &str, param: &str) -> Value {
        let o = root.object(crc32(obj.as_bytes())).unwrap();
        o.params
            .iter()
            .find(|p| p.name_hash == crc32(param.as_bytes()))
            .unwrap()
            .value
            .clone()
    }

    #[test]
    fn sets_scalar_type_preserving() {
        let mut t = tree();
        let r = set_by_path(&mut t, "WeaponCommon/Atk", "99").unwrap();
        assert_eq!(r.old, "int(10)");
        assert_eq!(r.new, "int(99)");
        assert_eq!(find_value(&t, "WeaponCommon", "Atk"), Value::Int(99));
    }

    #[test]
    fn sets_string_preserving_kind() {
        let mut t = tree();
        set_by_path(&mut t, "WeaponCommon/Name", "Master Sword").unwrap();
        assert_eq!(
            find_value(&t, "WeaponCommon", "Name"),
            Value::Str { ty: ParamType::String64, value: "Master Sword".into() }
        );
    }

    #[test]
    fn sets_color_from_csv() {
        let mut t = tree();
        set_by_path(&mut t, "WeaponCommon/Tint", "1.0, 0.5, 0.25, 1.0").unwrap();
        assert_eq!(find_value(&t, "WeaponCommon", "Tint"), Value::Color([1.0, 0.5, 0.25, 1.0]));
    }

    #[test]
    fn descends_into_nested_list() {
        let mut t = tree();
        set_by_path(&mut t, "AI/Logic/Speed", "2.5").unwrap();
        let l = t.list(crc32(b"AI")).unwrap();
        let o = l.object(crc32(b"Logic")).unwrap();
        assert_eq!(o.params[0].value, Value::F32(2.5));
    }

    #[test]
    fn rejects_missing_and_uneditable() {
        let mut t = tree();
        assert!(set_by_path(&mut t, "WeaponCommon/Nope", "1").is_err());
        assert!(set_by_path(&mut t, "Nope/Atk", "1").is_err());
        assert!(set_by_path(&mut t, "Atk", "1").is_err()); // too short
        assert!(set_by_path(&mut t, "WeaponCommon/Atk", "abc").is_err()); // bad int
        assert!(set_by_path(&mut t, "WeaponCommon/Curve", "1").is_err()); // not settable
    }

    #[test]
    fn accepts_hex_hash_segments() {
        let mut t = tree();
        let path = format!("0x{:08x}/0x{:08x}", crc32(b"WeaponCommon"), crc32(b"Atk"));
        set_by_path(&mut t, &path, "7").unwrap();
        assert_eq!(find_value(&t, "WeaponCommon", "Atk"), Value::Int(7));
    }
}
