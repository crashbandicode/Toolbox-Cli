//! AAMP serialization.
//!
//! Two writers, by design (matching [`crate::byml`]):
//!
//! - [`write_aamp`] is **verbatim**: it returns the bytes captured at read
//!   time, so an *unmodified* document round-trips byte-identically.
//! - [`write_aamp_canonical`] is a **from-scratch** writer for mutated /
//!   synthesized trees. AAMP's exact byte layout is writer-specific (value /
//!   string de-duplication, node ordering, alignment, the trailing
//!   unused-`u32` section), so this does not chase a specific tool's bytes;
//!   its guarantee is the **semantic** round-trip `read(write(x)) == read(x)`.
//!   It lays the sections out header → lists → objects → params → data →
//!   strings, 4-aligning every data/string entry (offsets are stored `/4`).

use std::collections::HashMap;

use super::error::{AampError, Result};
use super::{AampDocument, ParameterList, ParameterObject, Value, AAMP_MAGIC, HEADER_LEN};

/// Serialize `doc` back to AAMP bytes verbatim (byte-identical for an
/// unmodified [`AampDocument`]).
pub fn write_aamp(doc: &AampDocument) -> Vec<u8> {
    doc.raw.clone()
}

/// Serialize a document from scratch into a valid AAMP buffer.
///
/// Use this for mutated or synthesized trees (the verbatim [`write_aamp`] only
/// works for an untouched document). The output re-parses to the same tree,
/// but is not guaranteed byte-identical to any particular game/tool encoder.
pub fn write_aamp_canonical(doc: &AampDocument) -> Result<Vec<u8>> {
    let be = doc.big_endian;

    // ---- Pass 1: lists in BFS order (so each parent's children are a
    // contiguous run). ----
    let mut bfs: Vec<&ParameterList> = vec![&doc.root];
    let mut list_first_child: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < bfs.len() {
        let first = bfs.len();
        for c in &bfs[i].lists {
            bfs.push(c);
        }
        list_first_child.push(first);
        i += 1;
    }

    // ---- Pass 2: objects, grouped per owning list (contiguous runs). ----
    let mut obj_refs: Vec<&ParameterObject> = Vec::new();
    let mut list_obj_start: Vec<usize> = Vec::with_capacity(bfs.len());
    for l in &bfs {
        list_obj_start.push(obj_refs.len());
        for o in &l.objects {
            obj_refs.push(o);
        }
    }

    // ---- Pass 3: parameters, grouped per owning object. ----
    let mut param_refs: Vec<&super::Parameter> = Vec::new();
    let mut obj_param_start: Vec<usize> = Vec::with_capacity(obj_refs.len());
    for o in &obj_refs {
        obj_param_start.push(param_refs.len());
        for p in &o.params {
            param_refs.push(p);
        }
    }

    // ---- Section bases (everything 4-aligned). ----
    let type_bytes = doc.pio_type.len() + 1; // + NUL
    let lists_start = align4(HEADER_LEN + type_bytes);
    let pio_offset = lists_start - HEADER_LEN;
    let pos_list = |i: usize| lists_start + i * 0xC;

    let objects_start = align4(lists_start + bfs.len() * 0xC);
    let pos_obj = |j: usize| objects_start + j * 0x8;

    let params_start = align4(objects_start + obj_refs.len() * 0x8);
    let pos_param = |k: usize| params_start + k * 0x8;

    let data_start = align4(params_start + param_refs.len() * 0x8);

    // ---- Pass 4: build the data section + record each param's data offset;
    // strings are deferred to the string section. ----
    let mut data_bytes: Vec<u8> = Vec::new();
    let mut param_data_pos = vec![0usize; param_refs.len()];
    let mut string_params: Vec<(usize, &str)> = Vec::new();

    for (k, p) in param_refs.iter().enumerate() {
        if let Value::Str { value, .. } = &p.value {
            string_params.push((k, value.as_str()));
            continue;
        }
        pad4(&mut data_bytes);
        match &p.value {
            Value::BufferInt(v) => {
                push_u32(&mut data_bytes, v.len() as u32, be);
                param_data_pos[k] = data_start + data_bytes.len();
                for &x in v {
                    push_u32(&mut data_bytes, x as u32, be);
                }
            }
            Value::BufferU32(v) => {
                push_u32(&mut data_bytes, v.len() as u32, be);
                param_data_pos[k] = data_start + data_bytes.len();
                for &x in v {
                    push_u32(&mut data_bytes, x, be);
                }
            }
            Value::BufferF32(v) => {
                push_u32(&mut data_bytes, v.len() as u32, be);
                param_data_pos[k] = data_start + data_bytes.len();
                for &x in v {
                    push_u32(&mut data_bytes, x.to_bits(), be);
                }
            }
            Value::BufferBinary(v) => {
                push_u32(&mut data_bytes, v.len() as u32, be);
                param_data_pos[k] = data_start + data_bytes.len();
                data_bytes.extend_from_slice(v);
            }
            Value::Curve { raw, .. } => {
                param_data_pos[k] = data_start + data_bytes.len();
                data_bytes.extend_from_slice(raw);
            }
            scalar => {
                param_data_pos[k] = data_start + data_bytes.len();
                push_scalar(&mut data_bytes, scalar, be);
            }
        }
    }
    pad4(&mut data_bytes);
    let data_section_size = data_bytes.len();

    // ---- Pass 5: string section (de-duplicated, each string 4-aligned so its
    // `/4` offset is exact). ----
    let string_start = align4(data_start + data_section_size);
    let mut string_bytes: Vec<u8> = Vec::new();
    let mut string_off: HashMap<&str, usize> = HashMap::new();
    for (k, s) in &string_params {
        let off = match string_off.get(s) {
            Some(&o) => o,
            None => {
                pad4(&mut string_bytes);
                let o = string_bytes.len();
                string_bytes.extend_from_slice(s.as_bytes());
                string_bytes.push(0);
                string_off.insert(s, o);
                o
            }
        };
        param_data_pos[*k] = string_start + off;
    }
    pad4(&mut string_bytes);
    let string_section_size = string_bytes.len();

    // ---- Emit. ----
    let mut out = vec![0u8; HEADER_LEN];
    out[0..4].copy_from_slice(AAMP_MAGIC);
    write_u32(&mut out, 0x04, 2, be); // version
    write_u32(&mut out, 0x08, if be { 2 } else { 3 }, be); // flags: UTF8 (+ LE)
                                                           // 0x0C file_size — back-patched.
    write_u32(&mut out, 0x10, doc.pio_version, be);
    write_u32(&mut out, 0x14, pio_offset as u32, be);
    write_u32(&mut out, 0x18, bfs.len() as u32, be);
    write_u32(&mut out, 0x1C, obj_refs.len() as u32, be);
    write_u32(&mut out, 0x20, param_refs.len() as u32, be);
    write_u32(&mut out, 0x24, data_section_size as u32, be);
    write_u32(&mut out, 0x28, string_section_size as u32, be);
    write_u32(&mut out, 0x2C, 0, be); // unknown trailing-u32 count

    // Type string at 0x30, padded to the lists base.
    out.extend_from_slice(doc.pio_type.as_bytes());
    out.push(0);
    while out.len() < lists_start {
        out.push(0);
    }

    // List nodes.
    for (i, l) in bfs.iter().enumerate() {
        let here = pos_list(i);
        debug_assert_eq!(out.len(), here);
        push_u32(&mut out, l.name_hash, be);
        let lists_field = packed(
            if l.lists.is_empty() {
                0
            } else {
                pos_list(list_first_child[i]) - here
            },
            l.lists.len(),
        )?;
        let objs_field = packed(
            if l.objects.is_empty() {
                0
            } else {
                pos_obj(list_obj_start[i]) - here
            },
            l.objects.len(),
        )?;
        push_u32(&mut out, lists_field, be);
        push_u32(&mut out, objs_field, be);
    }
    pad_to(&mut out, objects_start);

    // Object nodes.
    for (j, o) in obj_refs.iter().enumerate() {
        let here = pos_obj(j);
        debug_assert_eq!(out.len(), here);
        push_u32(&mut out, o.name_hash, be);
        let params_field = packed(
            if o.params.is_empty() {
                0
            } else {
                pos_param(obj_param_start[j]) - here
            },
            o.params.len(),
        )?;
        push_u32(&mut out, params_field, be);
    }
    pad_to(&mut out, params_start);

    // Parameter nodes.
    for (k, p) in param_refs.iter().enumerate() {
        let here = pos_param(k);
        debug_assert_eq!(out.len(), here);
        push_u32(&mut out, p.name_hash, be);
        let data_off = param_data_pos[k] - here;
        if !data_off.is_multiple_of(4) {
            return Err(AampError::Edit(format!(
                "internal: param data offset {data_off} not 4-aligned"
            )));
        }
        let words = (data_off / 4) as u32;
        if words > 0x00FF_FFFF {
            return Err(AampError::Edit(
                "AAMP too large: parameter data offset exceeds 24 bits".into(),
            ));
        }
        let ty = p.value.param_type().to_u8() as u32;
        push_u32(&mut out, words | (ty << 24), be);
    }
    pad_to(&mut out, data_start);

    out.extend_from_slice(&data_bytes);
    pad_to(&mut out, string_start);
    out.extend_from_slice(&string_bytes);

    let total = out.len() as u32;
    write_u32(&mut out, 0x0C, total, be);
    Ok(out)
}

/// Pack a `{offset>>2 (low 16), count (high 16)}` list/object field, erroring
/// if either doesn't fit.
fn packed(offset_bytes: usize, count: usize) -> Result<u32> {
    if !offset_bytes.is_multiple_of(4) {
        return Err(AampError::Edit(format!(
            "internal: child offset {offset_bytes} not 4-aligned"
        )));
    }
    let words = offset_bytes / 4;
    if words > 0xFFFF {
        return Err(AampError::Edit(
            "AAMP too large: child offset exceeds 16 bits".into(),
        ));
    }
    if count > 0xFFFF {
        return Err(AampError::Edit("AAMP: too many children (>65535)".into()));
    }
    Ok((words as u32) | ((count as u32) << 16))
}

fn push_scalar(buf: &mut Vec<u8>, v: &Value, be: bool) {
    match v {
        Value::Bool(b) => push_u32(buf, u32::from(*b), be),
        Value::F32(x) => push_u32(buf, x.to_bits(), be),
        Value::Int(x) => push_u32(buf, *x as u32, be),
        Value::U32(x) => push_u32(buf, *x, be),
        Value::Vec2(a) => push_floats(buf, a, be),
        Value::Vec3(a) => push_floats(buf, a, be),
        Value::Vec4(a) => push_floats(buf, a, be),
        Value::Color(a) => push_floats(buf, a, be),
        Value::Quat(a) => push_floats(buf, a, be),
        // Strings/curves/buffers are handled by the caller.
        _ => unreachable!("push_scalar on a non-scalar value"),
    }
}

fn push_floats(buf: &mut Vec<u8>, a: &[f32], be: bool) {
    for &x in a {
        push_u32(buf, x.to_bits(), be);
    }
}

fn align4(x: usize) -> usize {
    (x + 3) & !3
}

fn pad4(buf: &mut Vec<u8>) {
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
}

fn pad_to(buf: &mut Vec<u8>, target: usize) {
    while buf.len() < target {
        buf.push(0);
    }
}

fn push_u32(buf: &mut Vec<u8>, v: u32, be: bool) {
    if be {
        buf.extend_from_slice(&v.to_be_bytes());
    } else {
        buf.extend_from_slice(&v.to_le_bytes());
    }
}

fn write_u32(buf: &mut [u8], pos: usize, v: u32, be: bool) {
    let b = if be { v.to_be_bytes() } else { v.to_le_bytes() };
    buf[pos..pos + 4].copy_from_slice(&b);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aamp::{read_aamp, ParamType, Parameter, ParameterObject};

    /// A nested doc exercising scalars, a string, a vec, and a buffer across
    /// two lists and two objects.
    fn sample_doc() -> AampDocument {
        let inner = ParameterList {
            name_hash: 0x0000_1111,
            lists: Vec::new(),
            objects: vec![ParameterObject {
                name_hash: 0x2222_3333,
                params: vec![
                    Parameter {
                        name_hash: 1,
                        value: Value::F32(1.5),
                    },
                    Parameter {
                        name_hash: 2,
                        value: Value::Int(-7),
                    },
                    Parameter {
                        name_hash: 3,
                        value: Value::Str {
                            ty: ParamType::String32,
                            value: "hello".into(),
                        },
                    },
                    Parameter {
                        name_hash: 4,
                        value: Value::Vec3([1.0, 2.0, 3.0]),
                    },
                    Parameter {
                        name_hash: 5,
                        value: Value::BufferInt(vec![10, 20, 30]),
                    },
                ],
            }],
        };
        let root = ParameterList {
            name_hash: 0xAABB_CCDD,
            lists: vec![inner],
            objects: vec![ParameterObject {
                name_hash: 0x1122_3344,
                params: vec![Parameter {
                    name_hash: 6,
                    value: Value::Bool(true),
                }],
            }],
        };
        AampDocument {
            pio_version: 0,
            pio_type: "xml".into(),
            big_endian: false,
            root,
            raw: Vec::new(),
        }
    }

    #[test]
    fn canonical_semantic_round_trips() {
        let doc = sample_doc();
        let bytes = write_aamp_canonical(&doc).unwrap();
        let doc2 = read_aamp(&bytes).expect("re-parse canonical");
        assert_eq!(doc.root, doc2.root);
        assert_eq!(doc.pio_type, doc2.pio_type);
        assert_eq!(doc.pio_version, doc2.pio_version);
    }

    /// The canonical writer is idempotent: re-encoding its own output (after a
    /// re-read) yields byte-identical bytes.
    #[test]
    fn canonical_write_is_idempotent() {
        let doc = sample_doc();
        let c1 = write_aamp_canonical(&doc).unwrap();
        let d1 = read_aamp(&c1).expect("re-parse");
        let c2 = write_aamp_canonical(&d1).unwrap();
        assert_eq!(
            c2, c1,
            "canonical writer must be byte-stable across re-writes"
        );
    }
}
