//! BYML serialization.
//!
//! Two writers, by design:
//!
//! - [`write_byml`] is **verbatim**: it returns the bytes captured at read
//!   time, so an *unmodified* document round-trips byte-identically — the same
//!   discipline the [`crate::compression`] layer uses for unchanged files.
//! - [`write_byml_canonical`] is a **from-scratch** writer for mutated or
//!   synthesized trees. BYML's exact byte layout is writer-specific (node
//!   de-duplication, node ordering, padding), so this does *not* try to
//!   reproduce a third-party tool's bytes; its guarantee is the **semantic**
//!   round-trip `read(write(x)) == read(x)`. It emits sorted, de-duplicated
//!   hash-key and string tables, then lays the node tree out breadth-first
//!   with back-patched offsets.

use std::collections::{BTreeSet, HashMap, VecDeque};

use super::error::{BymlError, Result};
use super::*;

/// Serialize `doc` back to BYML bytes verbatim (byte-identical for an
/// unmodified document).
pub fn write_byml(doc: &BymlDocument) -> Result<Vec<u8>> {
    Ok(doc.raw.clone())
}

/// Serialize a document tree from scratch into a valid BYML buffer.
///
/// Use this for mutated or synthesized trees (the verbatim [`write_byml`] only
/// works for an untouched [`BymlDocument`]). The output is a *valid* BYML that
/// re-parses to the same [`Byml`] tree, but is not guaranteed byte-identical to
/// any particular game/tool encoder.
pub fn write_byml_canonical(version: u16, big_endian: bool, root: &Byml) -> Result<Vec<u8>> {
    // Empty document (no root) is a 16-byte header with a zero root offset.
    if matches!(root, Byml::Null) {
        let mut buf = vec![0u8; 16];
        buf[0..2].copy_from_slice(if big_endian { b"BY" } else { b"YB" });
        write_u16(&mut buf, 2, version, big_endian);
        return Ok(buf);
    }
    if !root.is_container() {
        return Err(BymlError::NonContainerRoot(root.type_name()));
    }

    // 1. Collect the hash keys and string values (deduped + sorted ascending,
    //    matching BYML's binary-searchable tables).
    let mut keys = BTreeSet::new();
    let mut strings = BTreeSet::new();
    collect_strings(root, &mut keys, &mut strings);
    let keys: Vec<&str> = keys.into_iter().collect();
    let strings: Vec<&str> = strings.into_iter().collect();
    let key_index: HashMap<&str, u32> = keys.iter().enumerate().map(|(i, &k)| (k, i as u32)).collect();
    let str_index: HashMap<&str, u32> =
        strings.iter().enumerate().map(|(i, &s)| (s, i as u32)).collect();

    // 2. Header placeholder + the two string tables.
    let mut buf = vec![0u8; 16];
    buf[0..2].copy_from_slice(if big_endian { b"BY" } else { b"YB" });
    write_u16(&mut buf, 2, version, big_endian);

    let hash_key_off = if keys.is_empty() {
        0
    } else {
        let off = buf.len();
        write_string_table(&mut buf, &keys, big_endian);
        off
    };
    let string_off = if strings.is_empty() {
        0
    } else {
        let off = buf.len();
        write_string_table(&mut buf, &strings, big_endian);
        off
    };
    write_u32(&mut buf, 4, hash_key_off as u32, big_endian);
    write_u32(&mut buf, 8, string_off as u32, big_endian);

    // 3. Lay out the node tree breadth-first. Each container/64-bit/binary
    //    value is placed later and its parent's value slot back-patched with
    //    the resulting absolute offset.
    let mut w = Writer {
        buf,
        big_endian,
        key_index: &key_index,
        str_index: &str_index,
        queue: VecDeque::new(),
    };

    align_to(&mut w.buf, 4);
    let root_off = w.buf.len();
    w.write_container_node(root);
    while let Some((fixup_pos, item)) = w.queue.pop_front() {
        let off = match item {
            Deferred::Container(c) => {
                align_to(&mut w.buf, 4);
                let off = w.buf.len();
                w.write_container_node(c);
                off
            }
            Deferred::I64(v) => {
                align_to(&mut w.buf, 8);
                let off = w.buf.len();
                w.buf.extend_from_slice(&i64_bytes(v, big_endian));
                off
            }
            Deferred::U64(v) => {
                align_to(&mut w.buf, 8);
                let off = w.buf.len();
                w.buf.extend_from_slice(&u64_bytes(v, big_endian));
                off
            }
            Deferred::F64(v) => {
                align_to(&mut w.buf, 8);
                let off = w.buf.len();
                w.buf.extend_from_slice(&u64_bytes(v.to_bits(), big_endian));
                off
            }
            Deferred::Binary(b) => {
                align_to(&mut w.buf, 4);
                let off = w.buf.len();
                write_u32_push(&mut w.buf, b.len() as u32, big_endian);
                w.buf.extend_from_slice(b);
                off
            }
        };
        write_u32(&mut w.buf, fixup_pos, off as u32, big_endian);
    }

    write_u32(&mut w.buf, 12, root_off as u32, big_endian);
    Ok(w.buf)
}

/// A node whose payload is placed outside its parent and referenced by offset.
enum Deferred<'a> {
    Container(&'a Byml),
    I64(i64),
    U64(u64),
    F64(f64),
    Binary(&'a [u8]),
}

struct Writer<'a> {
    buf: Vec<u8>,
    big_endian: bool,
    key_index: &'a HashMap<&'a str, u32>,
    str_index: &'a HashMap<&'a str, u32>,
    queue: VecDeque<(usize, Deferred<'a>)>,
}

impl<'a> Writer<'a> {
    /// Write an array/hash node at the current buffer end, enqueuing any
    /// offset-referenced children for later placement.
    fn write_container_node(&mut self, node: &'a Byml) {
        match node {
            Byml::Array(items) => {
                push_node_header(&mut self.buf, NODE_ARRAY, items.len() as u32, self.big_endian);
                for item in items {
                    self.buf.push(node_type_tag(item));
                }
                align_to(&mut self.buf, 4);
                for item in items {
                    self.write_value_slot(item);
                }
            }
            Byml::Hash(entries) => {
                push_node_header(&mut self.buf, NODE_HASH, entries.len() as u32, self.big_endian);
                // BYML requires hash keys sorted ascending; sort a borrowed
                // view so synthesized (unsorted) trees still emit valid files.
                let mut sorted: Vec<&(String, Byml)> = entries.iter().collect();
                sorted.sort_by(|a, b| a.0.cmp(&b.0));
                for (key, value) in sorted {
                    let ki = self.key_index[key.as_str()];
                    push_hash_entry_head(&mut self.buf, ki, node_type_tag(value), self.big_endian);
                    self.write_value_slot(value);
                }
            }
            // Only containers reach here.
            _ => unreachable!("write_container_node on a non-container"),
        }
    }

    /// Write a single 4-byte value slot: inline scalars directly, everything
    /// else as a placeholder enqueued for back-patching.
    fn write_value_slot(&mut self, value: &'a Byml) {
        let slot = self.buf.len();
        match value {
            Byml::Null => write_u32_push(&mut self.buf, 0, self.big_endian),
            Byml::Bool(b) => write_u32_push(&mut self.buf, u32::from(*b), self.big_endian),
            Byml::I32(n) => write_u32_push(&mut self.buf, *n as u32, self.big_endian),
            Byml::U32(n) => write_u32_push(&mut self.buf, *n, self.big_endian),
            Byml::F32(n) => write_u32_push(&mut self.buf, n.to_bits(), self.big_endian),
            Byml::String(s) => write_u32_push(&mut self.buf, self.str_index[s.as_str()], self.big_endian),
            Byml::Array(_) | Byml::Hash(_) => {
                write_u32_push(&mut self.buf, 0, self.big_endian);
                self.queue.push_back((slot, Deferred::Container(value)));
            }
            Byml::I64(n) => {
                write_u32_push(&mut self.buf, 0, self.big_endian);
                self.queue.push_back((slot, Deferred::I64(*n)));
            }
            Byml::U64(n) => {
                write_u32_push(&mut self.buf, 0, self.big_endian);
                self.queue.push_back((slot, Deferred::U64(*n)));
            }
            Byml::F64(n) => {
                write_u32_push(&mut self.buf, 0, self.big_endian);
                self.queue.push_back((slot, Deferred::F64(*n)));
            }
            Byml::Binary(b) => {
                write_u32_push(&mut self.buf, 0, self.big_endian);
                self.queue.push_back((slot, Deferred::Binary(b)));
            }
        }
    }
}

/// Recursively gather every hash key and string value into sorted, deduped
/// sets (borrowing from the tree — no allocation per string).
fn collect_strings<'a>(node: &'a Byml, keys: &mut BTreeSet<&'a str>, strings: &mut BTreeSet<&'a str>) {
    match node {
        Byml::String(s) => {
            strings.insert(s.as_str());
        }
        Byml::Array(items) => {
            for item in items {
                collect_strings(item, keys, strings);
            }
        }
        Byml::Hash(entries) => {
            for (k, v) in entries {
                keys.insert(k.as_str());
                collect_strings(v, keys, strings);
            }
        }
        _ => {}
    }
}

/// The on-disk node-type tag for a value.
fn node_type_tag(value: &Byml) -> u8 {
    match value {
        Byml::Null => NODE_NULL,
        Byml::Bool(_) => NODE_BOOL,
        Byml::I32(_) => NODE_I32,
        Byml::U32(_) => NODE_U32,
        Byml::F32(_) => NODE_F32,
        Byml::I64(_) => NODE_I64,
        Byml::U64(_) => NODE_U64,
        Byml::F64(_) => NODE_F64,
        Byml::String(_) => NODE_STRING,
        Byml::Binary(_) => NODE_BINARY,
        Byml::Array(_) => NODE_ARRAY,
        Byml::Hash(_) => NODE_HASH,
    }
}

/// Write a `0xc2` string table: header (count), `count + 1` relative u32
/// offsets, then the NUL-terminated strings; padded to a 4-byte boundary.
fn write_string_table(buf: &mut Vec<u8>, strings: &[&str], big_endian: bool) {
    let node_start = buf.len();
    push_node_header(buf, NODE_STRING_TABLE, strings.len() as u32, big_endian);

    let offsets_pos = buf.len();
    buf.resize(buf.len() + (strings.len() + 1) * 4, 0);

    let mut rel = buf.len() - node_start;
    let mut offsets = Vec::with_capacity(strings.len() + 1);
    for s in strings {
        offsets.push(rel as u32);
        buf.extend_from_slice(s.as_bytes());
        buf.push(0);
        rel += s.len() + 1;
    }
    offsets.push(rel as u32);
    for (i, &off) in offsets.iter().enumerate() {
        write_u32(buf, offsets_pos + i * 4, off, big_endian);
    }
    align_to(buf, 4);
}

/// Push a 4-byte node header (type byte + 24-bit count) in file endianness.
fn push_node_header(buf: &mut Vec<u8>, node_type: u8, count: u32, big_endian: bool) {
    buf.push(node_type);
    if big_endian {
        buf.push((count >> 16) as u8);
        buf.push((count >> 8) as u8);
        buf.push(count as u8);
    } else {
        buf.push(count as u8);
        buf.push((count >> 8) as u8);
        buf.push((count >> 16) as u8);
    }
}

/// Push a hash entry head: 24-bit key index + value node-type byte.
fn push_hash_entry_head(buf: &mut Vec<u8>, key_index: u32, node_type: u8, big_endian: bool) {
    if big_endian {
        buf.push((key_index >> 16) as u8);
        buf.push((key_index >> 8) as u8);
        buf.push(key_index as u8);
    } else {
        buf.push(key_index as u8);
        buf.push((key_index >> 8) as u8);
        buf.push((key_index >> 16) as u8);
    }
    buf.push(node_type);
}

/// Pad `buf` with zeros up to a multiple of `align` (a power of two).
fn align_to(buf: &mut Vec<u8>, align: usize) {
    while !buf.len().is_multiple_of(align) {
        buf.push(0);
    }
}

fn write_u16(buf: &mut [u8], pos: usize, v: u16, big_endian: bool) {
    let b = if big_endian {
        v.to_be_bytes()
    } else {
        v.to_le_bytes()
    };
    buf[pos..pos + 2].copy_from_slice(&b);
}

fn write_u32(buf: &mut [u8], pos: usize, v: u32, big_endian: bool) {
    let b = if big_endian {
        v.to_be_bytes()
    } else {
        v.to_le_bytes()
    };
    buf[pos..pos + 4].copy_from_slice(&b);
}

fn write_u32_push(buf: &mut Vec<u8>, v: u32, big_endian: bool) {
    if big_endian {
        buf.extend_from_slice(&v.to_be_bytes());
    } else {
        buf.extend_from_slice(&v.to_le_bytes());
    }
}

fn i64_bytes(v: i64, big_endian: bool) -> [u8; 8] {
    if big_endian {
        v.to_be_bytes()
    } else {
        v.to_le_bytes()
    }
}

fn u64_bytes(v: u64, big_endian: bool) -> [u8; 8] {
    if big_endian {
        v.to_be_bytes()
    } else {
        v.to_le_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::super::read_byml;
    use super::*;

    /// Recursively sort hash keys so trees compare order-insensitively. The
    /// canonical writer emits keys sorted (BYML's binary-search requirement),
    /// so a round-tripped hash comes back sorted regardless of input order.
    fn normalized(v: &Byml) -> Byml {
        match v {
            Byml::Array(a) => Byml::Array(a.iter().map(normalized).collect()),
            Byml::Hash(h) => {
                let mut e: Vec<(String, Byml)> =
                    h.iter().map(|(k, c)| (k.clone(), normalized(c))).collect();
                e.sort_by(|a, b| a.0.cmp(&b.0));
                Byml::Hash(e)
            }
            other => other.clone(),
        }
    }

    /// Build a tree exercising every node kind, write it canonically, read it
    /// back, and assert the tree is preserved (the semantic round-trip).
    #[test]
    fn canonical_round_trip_all_kinds() {
        let root = Byml::Hash(vec![
            ("flag".into(), Byml::Bool(true)),
            ("count".into(), Byml::I32(-7)),
            ("ucount".into(), Byml::U32(7)),
            ("ratio".into(), Byml::F32(0.5)),
            ("big".into(), Byml::I64(-5_000_000_000)),
            ("ubig".into(), Byml::U64(9_000_000_000)),
            ("dbl".into(), Byml::F64(1.25)),
            ("name".into(), Byml::String("hello".into())),
            ("none".into(), Byml::Null),
            ("blob".into(), Byml::Binary(vec![1, 2, 3, 4, 5])),
            (
                "list".into(),
                Byml::Array(vec![
                    Byml::U32(1),
                    Byml::String("a".into()),
                    Byml::Hash(vec![("k".into(), Byml::String("v".into()))]),
                ]),
            ),
        ]);

        for big_endian in [false, true] {
            let bytes = write_byml_canonical(7, big_endian, &root).expect("write");
            let doc = read_byml(&bytes).expect("read back");
            assert_eq!(doc.version, 7);
            assert_eq!(doc.big_endian, big_endian);
            assert_eq!(
                normalized(&doc.root),
                normalized(&root),
                "canonical round-trip (be={big_endian})"
            );
        }
    }

    #[test]
    fn empty_root_is_header_only() {
        let bytes = write_byml_canonical(7, false, &Byml::Null).expect("write");
        assert_eq!(bytes.len(), 16);
        let doc = read_byml(&bytes).expect("read");
        assert_eq!(doc.root, Byml::Null);
    }

    #[test]
    fn rejects_scalar_root() {
        assert!(matches!(
            write_byml_canonical(7, false, &Byml::U32(1)),
            Err(BymlError::NonContainerRoot("u32"))
        ));
    }
}
