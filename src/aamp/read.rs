//! AAMP v2 parser: header → root Parameter IO → recursive list / object /
//! parameter tree, following each node's `/4` relative offsets.
//!
//! All reads are bounds-checked and report the failing offset. AAMP is
//! little-endian in practice (the format spec notes it's the same endianness
//! on every console); the endianness flag is honored, but a file whose version
//! word isn't `2` little-endian is rejected.

use super::error::{AampError, Result};
use super::*;

/// Parse an AAMP document, retaining the original bytes for the verbatim
/// [`write_aamp`](super::write_aamp) round-trip.
pub fn read_aamp(data: &[u8]) -> Result<AampDocument> {
    if data.len() < HEADER_LEN {
        return Err(AampError::TooSmall(data.len()));
    }
    if &data[0..4] != AAMP_MAGIC {
        let mut m = [0u8; 4];
        m.copy_from_slice(&data[0..4]);
        return Err(AampError::BadMagic(m));
    }

    // AAMP is little-endian; reject anything whose version word isn't 2 LE.
    let version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    if version != 2 {
        return Err(AampError::UnsupportedVersion(version));
    }
    let flags = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    let big_endian = (flags & 1) == 0;

    let pio_version = read_u32(data, 0x10, big_endian)?;
    let pio_offset = read_u32(data, 0x14, big_endian)? as usize;

    // Parameter IO type string sits at 0x30; the root list follows it.
    let pio_type = read_cstring(data, HEADER_LEN)?;
    let root_pos = HEADER_LEN
        .checked_add(pio_offset)
        .ok_or(AampError::Truncated {
            offset: 0x14,
            need: pio_offset,
            len: data.len(),
        })?;

    let ctx = Ctx { data, big_endian };
    let root = ctx.read_list(root_pos, 0)?;

    Ok(AampDocument {
        pio_version,
        pio_type,
        big_endian,
        root,
        raw: data.to_vec(),
    })
}

struct Ctx<'a> {
    data: &'a [u8],
    big_endian: bool,
}

impl Ctx<'_> {
    fn need(&self, offset: usize, n: usize) -> Result<()> {
        match offset.checked_add(n) {
            Some(end) if end <= self.data.len() => Ok(()),
            _ => Err(AampError::Truncated {
                offset,
                need: n,
                len: self.data.len(),
            }),
        }
    }

    /// Parse a parameter list (0xC node): name + child-lists + child-objects.
    fn read_list(&self, pos: usize, depth: usize) -> Result<ParameterList> {
        if depth > MAX_DEPTH {
            return Err(AampError::TooDeep {
                limit: MAX_DEPTH,
                offset: pos,
            });
        }
        self.need(pos, 0xC)?;
        let name_hash = read_u32(self.data, pos, self.big_endian)?;
        let (lists_off, lists_count) = packed_offset_count(self.data, pos + 4, self.big_endian)?;
        let (objects_off, objects_count) =
            packed_offset_count(self.data, pos + 8, self.big_endian)?;

        let mut lists = Vec::with_capacity(lists_count);
        for i in 0..lists_count {
            let child = pos + lists_off + i * 0xC;
            lists.push(self.read_list(child, depth + 1)?);
        }
        let mut objects = Vec::with_capacity(objects_count);
        for i in 0..objects_count {
            let child = pos + objects_off + i * 8;
            objects.push(self.read_object(child)?);
        }
        Ok(ParameterList {
            name_hash,
            lists,
            objects,
        })
    }

    /// Parse a parameter object (0x8 node): name + parameters.
    fn read_object(&self, pos: usize) -> Result<ParameterObject> {
        self.need(pos, 8)?;
        let name_hash = read_u32(self.data, pos, self.big_endian)?;
        let (params_off, params_count) = packed_offset_count(self.data, pos + 4, self.big_endian)?;
        let mut params = Vec::with_capacity(params_count);
        for i in 0..params_count {
            params.push(self.read_param(pos + params_off + i * 8)?);
        }
        Ok(ParameterObject { name_hash, params })
    }

    /// Parse a parameter (0x8 node): name + `{data offset>>2, type}`.
    fn read_param(&self, pos: usize) -> Result<Parameter> {
        self.need(pos, 8)?;
        let name_hash = read_u32(self.data, pos, self.big_endian)?;
        let word = read_u32(self.data, pos + 4, self.big_endian)?;
        let data_off = (word & 0x00FF_FFFF) as usize * 4;
        let ty_byte = (word >> 24) as u8;
        let ty = ParamType::from_u8(ty_byte).ok_or(AampError::UnknownType {
            ty: ty_byte,
            offset: pos + 4,
        })?;
        let data_pos = pos + data_off;
        let value = self.read_value(ty, data_pos)?;
        Ok(Parameter { name_hash, value })
    }

    fn read_value(&self, ty: ParamType, at: usize) -> Result<Value> {
        Ok(match ty {
            ParamType::Bool => Value::Bool(read_u32(self.data, at, self.big_endian)? != 0),
            ParamType::F32 => Value::F32(f32::from_bits(read_u32(self.data, at, self.big_endian)?)),
            ParamType::Int => Value::Int(read_u32(self.data, at, self.big_endian)? as i32),
            ParamType::U32 => Value::U32(read_u32(self.data, at, self.big_endian)?),
            ParamType::Vec2 => Value::Vec2(self.read_floats::<2>(at)?),
            ParamType::Vec3 => Value::Vec3(self.read_floats::<3>(at)?),
            ParamType::Vec4 => Value::Vec4(self.read_floats::<4>(at)?),
            ParamType::Color => Value::Color(self.read_floats::<4>(at)?),
            ParamType::Quat => Value::Quat(self.read_floats::<4>(at)?),
            ParamType::String32 | ParamType::String64 | ParamType::String256 | ParamType::StringRef => {
                Value::Str {
                    ty,
                    value: read_cstring(self.data, at)?,
                }
            }
            ParamType::Curve1 | ParamType::Curve2 | ParamType::Curve3 | ParamType::Curve4 => {
                let size = ty.curve_count() * CURVE_SIZE;
                self.need(at, size)?;
                Value::Curve {
                    ty,
                    raw: self.data[at..at + size].to_vec(),
                }
            }
            ParamType::BufferInt => Value::BufferInt(self.read_buffer_u32(at)?.into_iter().map(|v| v as i32).collect()),
            ParamType::BufferU32 => Value::BufferU32(self.read_buffer_u32(at)?),
            ParamType::BufferF32 => Value::BufferF32(
                self.read_buffer_u32(at)?
                    .into_iter()
                    .map(f32::from_bits)
                    .collect(),
            ),
            ParamType::BufferBinary => {
                let count = self.buffer_count(at)?;
                self.need(at, count)?;
                Value::BufferBinary(self.data[at..at + count].to_vec())
            }
        })
    }

    fn read_floats<const N: usize>(&self, at: usize) -> Result<[f32; N]> {
        self.need(at, N * 4)?;
        let mut out = [0f32; N];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = f32::from_bits(read_u32(self.data, at + i * 4, self.big_endian)?);
        }
        Ok(out)
    }

    /// The element/byte count of a buffer, stored as a `u32` immediately before
    /// the buffer's data offset.
    fn buffer_count(&self, at: usize) -> Result<usize> {
        let count_pos = at
            .checked_sub(4)
            .ok_or(AampError::BufferCountUnderflow { offset: at })?;
        Ok(read_u32(self.data, count_pos, self.big_endian)? as usize)
    }

    /// Read a `u32`-element buffer (its count is at `at - 4`).
    fn read_buffer_u32(&self, at: usize) -> Result<Vec<u32>> {
        let count = self.buffer_count(at)?;
        self.need(at, count.saturating_mul(4))?;
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            out.push(read_u32(self.data, at + i * 4, self.big_endian)?);
        }
        Ok(out)
    }
}

/// Read a packed `{offset>>2 (low 16 bits), count (high 16 bits)}` word and
/// return `(offset_in_bytes, count)`.
fn packed_offset_count(data: &[u8], off: usize, big_endian: bool) -> Result<(usize, usize)> {
    let w = read_u32(data, off, big_endian)?;
    let offset = (w & 0xFFFF) as usize * 4;
    let count = (w >> 16) as usize;
    Ok((offset, count))
}

fn read_u32(data: &[u8], off: usize, big_endian: bool) -> Result<u32> {
    match off.checked_add(4) {
        Some(end) if end <= data.len() => {
            let b = [data[off], data[off + 1], data[off + 2], data[off + 3]];
            Ok(if big_endian {
                u32::from_be_bytes(b)
            } else {
                u32::from_le_bytes(b)
            })
        }
        _ => Err(AampError::Truncated {
            offset: off,
            need: 4,
            len: data.len(),
        }),
    }
}

/// Read a NUL-terminated UTF-8 string at `off`.
fn read_cstring(data: &[u8], off: usize) -> Result<String> {
    if off > data.len() {
        return Err(AampError::Truncated {
            offset: off,
            need: 1,
            len: data.len(),
        });
    }
    let rest = &data[off..];
    let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    std::str::from_utf8(&rest[..end])
        .map(|s| s.to_string())
        .map_err(|e| AampError::NonUtf8 { offset: off, source: e })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal AAMP v2: root list `param_root` with one object holding
    /// a single `f32` parameter (value 1.5). Hand-built so the parser has a
    /// fixture-free correctness net.
    fn minimal_aamp() -> Vec<u8> {
        let mut b = vec![0u8; HEADER_LEN];
        b[0..4].copy_from_slice(AAMP_MAGIC);
        b[4..8].copy_from_slice(&2u32.to_le_bytes()); // version
        b[8..12].copy_from_slice(&3u32.to_le_bytes()); // flags: LE + UTF8
        b[0x14..0x18].copy_from_slice(&4u32.to_le_bytes()); // pio_offset (after "xml\0")
        b[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes()); // num_lists
        b[0x1c..0x20].copy_from_slice(&1u32.to_le_bytes()); // num_objects
        b[0x20..0x24].copy_from_slice(&1u32.to_le_bytes()); // num_params
        b[0x24..0x28].copy_from_slice(&4u32.to_le_bytes()); // data section size
        // Type string "xml\0" at 0x30.
        b.extend_from_slice(b"xml\0");
        // Root list at 0x34.
        let root = b.len();
        assert_eq!(root, 0x34);
        let obj_off = 0xC; // object right after the 0xC list header
        let push_u32 = |b: &mut Vec<u8>, v: u32| b.extend_from_slice(&v.to_le_bytes());
        push_u32(&mut b, 0xAABB_CCDD); // name hash
        push_u32(&mut b, 0); // no child lists {offset:0, count:0}
        push_u32(&mut b, (obj_off as u32 / 4) | (1u32 << 16)); // 1 object
        // Object at root + 0xC.
        let obj = b.len();
        assert_eq!(obj - root, obj_off);
        let param_off = 8; // param right after the 0x8 object header
        push_u32(&mut b, 0x1122_3344); // object name
        push_u32(&mut b, (param_off as u32 / 4) | (1u32 << 16)); // 1 param
        // Param at obj + 8.
        let param = b.len();
        assert_eq!(param - obj, param_off);
        push_u32(&mut b, 0x5566_7788); // param name
        // data lives right after the 8-byte param node.
        let data_off_words = 8u32 / 4;
        let ty = ParamType::F32.to_u8() as u32;
        push_u32(&mut b, (data_off_words & 0x00FF_FFFF) | (ty << 24));
        // f32 data at param + 8.
        let data = b.len();
        assert_eq!(data - param, 8);
        push_u32(&mut b, 1.5f32.to_bits());
        b
    }

    #[test]
    fn parses_minimal() {
        let bytes = minimal_aamp();
        let doc = read_aamp(&bytes).expect("parse");
        assert_eq!(doc.pio_type, "xml");
        assert!(!doc.big_endian);
        assert_eq!(doc.root.name_hash, 0xAABB_CCDD);
        assert_eq!(doc.root.objects.len(), 1);
        let obj = &doc.root.objects[0];
        assert_eq!(obj.name_hash, 0x1122_3344);
        assert_eq!(obj.params.len(), 1);
        assert_eq!(obj.params[0].name_hash, 0x5566_7788);
        assert_eq!(obj.params[0].value, Value::F32(1.5));
        // Verbatim writer reproduces the input.
        assert_eq!(super::super::write_aamp(&doc), bytes);
        assert_eq!(doc.counts(), (1, 1, 1));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(matches!(read_aamp(&[0u8; 8]), Err(AampError::TooSmall(8))));
        let mut bad = vec![0u8; HEADER_LEN];
        bad[0..4].copy_from_slice(b"NOPE");
        assert!(matches!(read_aamp(&bad), Err(AampError::BadMagic(_))));
        let mut wrong_ver = vec![0u8; HEADER_LEN];
        wrong_ver[0..4].copy_from_slice(AAMP_MAGIC);
        wrong_ver[4..8].copy_from_slice(&1u32.to_le_bytes());
        assert!(matches!(
            read_aamp(&wrong_ver),
            Err(AampError::UnsupportedVersion(1))
        ));
    }
}
