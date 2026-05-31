//! BYML parser: header → string tables → recursive node tree.
//!
//! All reads are bounds-checked and report the failing offset. Both endians
//! are handled; node headers and hash-entry heads pack a type byte with a
//! 24-bit field, in opposite byte positions, so they get dedicated helpers.

use super::error::{BymlError, Result};
use super::*;

/// Parse a BYML document, retaining the original bytes for the verbatim
/// [`write_byml`](super::write_byml) round-trip.
pub fn read_byml(data: &[u8]) -> Result<BymlDocument> {
    if data.len() < 16 {
        return Err(BymlError::TooSmall(data.len()));
    }
    let magic = [data[0], data[1]];
    let big_endian = match &magic {
        b"BY" => true,
        b"YB" => false,
        _ => return Err(BymlError::BadMagic(magic)),
    };
    let version = read_u16(data, 2, big_endian)?;
    if version == 0 || version > 7 {
        return Err(BymlError::UnsupportedVersion(version));
    }

    let hash_key_off = read_u32(data, 4, big_endian)? as usize;
    let string_off = read_u32(data, 8, big_endian)? as usize;
    let root_off = read_u32(data, 12, big_endian)? as usize;

    let hash_keys = if hash_key_off != 0 {
        read_string_table(data, hash_key_off, big_endian, "hash-key")?
    } else {
        Vec::new()
    };
    let strings = if string_off != 0 {
        read_string_table(data, string_off, big_endian, "string")?
    } else {
        Vec::new()
    };

    let ctx = Ctx {
        data,
        big_endian,
        hash_keys: &hash_keys,
        strings: &strings,
    };

    let root = if root_off == 0 {
        Byml::Null
    } else {
        let node_type = *data
            .get(root_off)
            .ok_or(BymlError::Truncated {
                offset: root_off,
                need: 1,
                len: data.len(),
            })?;
        ctx.read_container(node_type, root_off, 0)?
    };

    Ok(BymlDocument {
        version,
        big_endian,
        root,
        raw: data.to_vec(),
    })
}

/// Decode context: borrows the buffer and the two resolved string tables.
struct Ctx<'a> {
    data: &'a [u8],
    big_endian: bool,
    hash_keys: &'a [String],
    strings: &'a [String],
}

impl Ctx<'_> {
    /// Bounds check: `data[offset .. offset + n]` must be in range.
    fn need(&self, offset: usize, n: usize) -> Result<()> {
        match offset.checked_add(n) {
            Some(end) if end <= self.data.len() => Ok(()),
            _ => Err(BymlError::Truncated {
                offset,
                need: n,
                len: self.data.len(),
            }),
        }
    }

    /// Dispatch a container node (the root, or any array/hash referenced by
    /// offset).
    fn read_container(&self, node_type: u8, offset: usize, depth: usize) -> Result<Byml> {
        if depth > MAX_DEPTH {
            return Err(BymlError::TooDeep {
                limit: MAX_DEPTH,
                offset,
            });
        }
        match node_type {
            NODE_ARRAY => self.read_array(offset, depth),
            NODE_HASH => self.read_hash(offset, depth),
            other => Err(BymlError::NotAContainer {
                offset,
                node_type: other,
            }),
        }
    }

    fn read_array(&self, offset: usize, depth: usize) -> Result<Byml> {
        let (_t, count) = read_node_header(self.data, offset, self.big_endian)?;
        let count = count as usize;

        // `count` element-type bytes follow the 4-byte header, then the value
        // array is padded to a 4-byte boundary.
        let types_off = offset + 4;
        self.need(types_off, count)?;
        let types: Vec<u8> = self.data[types_off..types_off + count].to_vec();
        let values_off = align_up(types_off + count, 4);

        let mut out = Vec::with_capacity(count);
        for (i, &node_type) in types.iter().enumerate() {
            let voff = values_off + i * 4;
            out.push(self.read_value(node_type, voff, depth)?);
        }
        Ok(Byml::Array(out))
    }

    fn read_hash(&self, offset: usize, depth: usize) -> Result<Byml> {
        let (_t, count) = read_node_header(self.data, offset, self.big_endian)?;
        let count = count as usize;

        let entries_off = offset + 4;
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let eoff = entries_off + i * 8;
            self.need(eoff, 8)?;
            let (key_index, node_type) = read_hash_entry_head(self.data, eoff, self.big_endian);
            let key = self
                .hash_keys
                .get(key_index as usize)
                .ok_or(BymlError::StringIndexOutOfRange {
                    table: "hash-key",
                    index: key_index,
                    count: self.hash_keys.len(),
                })?
                .clone();
            let value = self.read_value(node_type, eoff + 4, depth)?;
            out.push((key, value));
        }
        Ok(Byml::Hash(out))
    }

    /// Decode a 4-byte value slot at `voff` according to `node_type`. Inline
    /// types hold their value directly; container / 64-bit / binary types hold
    /// an absolute offset to the payload.
    fn read_value(&self, node_type: u8, voff: usize, depth: usize) -> Result<Byml> {
        match node_type {
            NODE_STRING => {
                let idx = read_u32(self.data, voff, self.big_endian)?;
                let s = self
                    .strings
                    .get(idx as usize)
                    .ok_or(BymlError::StringIndexOutOfRange {
                        table: "string",
                        index: idx,
                        count: self.strings.len(),
                    })?;
                Ok(Byml::String(s.clone()))
            }
            NODE_BINARY => {
                let off = read_u32(self.data, voff, self.big_endian)? as usize;
                let size = read_u32(self.data, off, self.big_endian)? as usize;
                self.need(off + 4, size)?;
                Ok(Byml::Binary(self.data[off + 4..off + 4 + size].to_vec()))
            }
            NODE_ARRAY | NODE_HASH => {
                let off = read_u32(self.data, voff, self.big_endian)? as usize;
                self.read_container(node_type, off, depth + 1)
            }
            NODE_BOOL => Ok(Byml::Bool(read_u32(self.data, voff, self.big_endian)? != 0)),
            NODE_I32 => Ok(Byml::I32(read_u32(self.data, voff, self.big_endian)? as i32)),
            NODE_U32 => Ok(Byml::U32(read_u32(self.data, voff, self.big_endian)?)),
            NODE_F32 => Ok(Byml::F32(f32::from_bits(read_u32(
                self.data,
                voff,
                self.big_endian,
            )?))),
            NODE_I64 => {
                let off = read_u32(self.data, voff, self.big_endian)? as usize;
                Ok(Byml::I64(read_u64(self.data, off, self.big_endian)? as i64))
            }
            NODE_U64 => {
                let off = read_u32(self.data, voff, self.big_endian)? as usize;
                Ok(Byml::U64(read_u64(self.data, off, self.big_endian)?))
            }
            NODE_F64 => {
                let off = read_u32(self.data, voff, self.big_endian)? as usize;
                Ok(Byml::F64(f64::from_bits(read_u64(
                    self.data,
                    off,
                    self.big_endian,
                )?)))
            }
            NODE_NULL => Ok(Byml::Null),
            other => Err(BymlError::UnknownNodeType {
                node_type: other,
                offset: voff,
            }),
        }
    }
}

/// Read a `0xc2` string table at `offset`: a node header (count), then
/// `count + 1` u32 offsets (relative to the table start), then the
/// NUL-terminated string bytes between consecutive offsets.
fn read_string_table(
    data: &[u8],
    offset: usize,
    big_endian: bool,
    table: &'static str,
) -> Result<Vec<String>> {
    let (node_type, count) = read_node_header(data, offset, big_endian)?;
    if node_type != NODE_STRING_TABLE {
        return Err(BymlError::BadStringTable {
            table,
            offset,
            node_type,
        });
    }
    let count = count as usize;

    let mut offsets = Vec::with_capacity(count + 1);
    for i in 0..=count {
        offsets.push(read_u32(data, offset + 4 + i * 4, big_endian)? as usize);
    }

    let mut out = Vec::with_capacity(count);
    for window in offsets.windows(2) {
        let start = offset + window[0];
        let end = offset + window[1];
        if start > end || end > data.len() {
            return Err(BymlError::Truncated {
                offset: start,
                need: end.saturating_sub(start),
                len: data.len(),
            });
        }
        // The slice [start, end) includes the NUL terminator; keep the bytes
        // up to the first NUL.
        let raw = &data[start..end];
        let s = raw.split(|&b| b == 0).next().unwrap_or(&[]);
        let text = std::str::from_utf8(s).map_err(|e| BymlError::NonUtf8 {
            offset: start,
            source: e,
        })?;
        out.push(text.to_string());
    }
    Ok(out)
}

/// Round `x` up to the next multiple of `align` (a power of two).
fn align_up(x: usize, align: usize) -> usize {
    (x + align - 1) & !(align - 1)
}

fn read_u16(data: &[u8], off: usize, big_endian: bool) -> Result<u16> {
    let end = off.checked_add(2);
    match end {
        Some(e) if e <= data.len() => {
            let b = [data[off], data[off + 1]];
            Ok(if big_endian {
                u16::from_be_bytes(b)
            } else {
                u16::from_le_bytes(b)
            })
        }
        _ => Err(BymlError::Truncated {
            offset: off,
            need: 2,
            len: data.len(),
        }),
    }
}

fn read_u32(data: &[u8], off: usize, big_endian: bool) -> Result<u32> {
    match off.checked_add(4) {
        Some(e) if e <= data.len() => {
            let b = [data[off], data[off + 1], data[off + 2], data[off + 3]];
            Ok(if big_endian {
                u32::from_be_bytes(b)
            } else {
                u32::from_le_bytes(b)
            })
        }
        _ => Err(BymlError::Truncated {
            offset: off,
            need: 4,
            len: data.len(),
        }),
    }
}

fn read_u64(data: &[u8], off: usize, big_endian: bool) -> Result<u64> {
    match off.checked_add(8) {
        Some(e) if e <= data.len() => {
            let mut b = [0u8; 8];
            b.copy_from_slice(&data[off..off + 8]);
            Ok(if big_endian {
                u64::from_be_bytes(b)
            } else {
                u64::from_le_bytes(b)
            })
        }
        _ => Err(BymlError::Truncated {
            offset: off,
            need: 8,
            len: data.len(),
        }),
    }
}

/// Read a node header: a type byte followed by a 24-bit count. The count's
/// three bytes are in the file's endianness; the type is always the first
/// physical byte.
fn read_node_header(data: &[u8], off: usize, big_endian: bool) -> Result<(u8, u32)> {
    if off.checked_add(4).is_none_or(|e| e > data.len()) {
        return Err(BymlError::Truncated {
            offset: off,
            need: 4,
            len: data.len(),
        });
    }
    let node_type = data[off];
    let count = if big_endian {
        (data[off + 1] as u32) << 16 | (data[off + 2] as u32) << 8 | (data[off + 3] as u32)
    } else {
        (data[off + 1] as u32) | (data[off + 2] as u32) << 8 | (data[off + 3] as u32) << 16
    };
    Ok((node_type, count))
}

/// Read a hash entry head: a 24-bit hash-key index followed by the value's
/// node-type byte (the type is the *last* of the four bytes here, the mirror
/// of [`read_node_header`]). The caller must have bounds-checked 8 bytes.
fn read_hash_entry_head(data: &[u8], off: usize, big_endian: bool) -> (u32, u8) {
    let key = if big_endian {
        (data[off] as u32) << 16 | (data[off + 1] as u32) << 8 | (data[off + 2] as u32)
    } else {
        (data[off] as u32) | (data[off + 1] as u32) << 8 | (data[off + 2] as u32) << 16
    };
    (key, data[off + 3])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 48-byte little-endian BYML v7 whose root is an array of five inline
    /// scalars — no string tables required. Hand-built so the reader has a
    /// fixture-free correctness net (CI has no game assets).
    fn minimal_array_byml() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"YB"); // little-endian magic
        b.extend_from_slice(&7u16.to_le_bytes()); // version
        b.extend_from_slice(&0u32.to_le_bytes()); // hash-key table: none
        b.extend_from_slice(&0u32.to_le_bytes()); // string table: none
        b.extend_from_slice(&0x10u32.to_le_bytes()); // root node offset
        assert_eq!(b.len(), 0x10);
        // Array node: type 0xc0 + 24-bit count 5.
        b.push(NODE_ARRAY);
        b.extend_from_slice(&[5, 0, 0]);
        // Element type bytes, then pad to a 4-byte boundary (5 -> 8).
        b.extend_from_slice(&[NODE_BOOL, NODE_I32, NODE_U32, NODE_F32, NODE_NULL]);
        b.extend_from_slice(&[0, 0, 0]);
        // Inline value slots.
        b.extend_from_slice(&1u32.to_le_bytes()); // bool true
        b.extend_from_slice(&(-5i32).to_le_bytes()); // s32
        b.extend_from_slice(&7u32.to_le_bytes()); // u32
        b.extend_from_slice(&1.0f32.to_bits().to_le_bytes()); // f32
        b.extend_from_slice(&0u32.to_le_bytes()); // null
        assert_eq!(b.len(), 0x30);
        b
    }

    #[test]
    fn parses_minimal_array() {
        let bytes = minimal_array_byml();
        let doc = read_byml(&bytes).expect("parse");
        assert_eq!(doc.version, 7);
        assert!(!doc.big_endian);
        let arr = doc.root.as_array().expect("array root");
        assert_eq!(
            arr,
            &[
                Byml::Bool(true),
                Byml::I32(-5),
                Byml::U32(7),
                Byml::F32(1.0),
                Byml::Null,
            ]
        );
        // The verbatim writer reproduces the input byte-for-byte.
        assert_eq!(write_byml(&doc).unwrap(), bytes);
    }

    #[test]
    fn rejects_too_small() {
        assert!(matches!(read_byml(&[0u8; 4]), Err(BymlError::TooSmall(4))));
    }

    #[test]
    fn rejects_bad_magic() {
        let mut b = vec![0u8; 16];
        b[0] = b'X';
        b[1] = b'Z';
        assert!(matches!(read_byml(&b), Err(BymlError::BadMagic(_))));
    }

    #[test]
    fn rejects_truncated_node() {
        // Header points the root at 0x10 but the buffer ends there.
        let mut b = Vec::new();
        b.extend_from_slice(b"YB");
        b.extend_from_slice(&7u16.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&0x10u32.to_le_bytes());
        assert!(matches!(
            read_byml(&b),
            Err(BymlError::Truncated { offset: 0x10, .. })
        ));
    }
}
