//! BFLAN (Cafe Layout Animation) parser/writer.
//!
//! BFLAN shares BFLYT's container shape: a 0x14-byte `FLAN` header
//! followed by a flat list of `magic + u32 size` sections (`pat1`
//! animation tag, `pai1` animation info, and occasionally others). We
//! capture every section's bytes verbatim so the writer reproduces a
//! **byte-identical** file, and additionally decode `pat1`/`pai1` enough
//! for a useful `bflan-inspect` (read-only; never affects round-trip).

use crate::error::{Error, Result};

const MAGIC_FLAN: [u8; 4] = *b"FLAN";
const HEADER_FIXED: usize = 0x14;

/// A parsed BFLAN file. Every byte is either a header field or captured
/// verbatim, so [`write_bflan`] reproduces the input exactly.
#[derive(Debug, Clone)]
pub struct Bflan {
    /// Byte-order mark as stored (little-endian `0xFEFF` on Switch).
    pub bom: u16,
    /// Header size (`0x14` for every file we've seen).
    pub header_size: u16,
    /// Packed version (`major << 24 | ...`); Switch BFLAN is v8/v9.
    pub version: u32,
    /// The `u16` after `section_count` (always 0; preserved).
    pub padding: u16,
    /// Bytes between the fixed 0x14-byte header and `header_size` (empty
    /// when `header_size == 0x14`).
    pub header_extra: Vec<u8>,
    /// Sections in file order. `file_size`/`section_count` are recomputed
    /// on write.
    pub sections: Vec<BflanSection>,
    /// Any bytes after the last section (file padding); normally empty.
    pub trailing: Vec<u8>,
}

/// One BFLAN section: its 4-byte magic and payload (the bytes after the
/// `magic + u32 size` section header).
#[derive(Debug, Clone)]
pub struct BflanSection {
    pub magic: [u8; 4],
    pub payload: Vec<u8>,
    /// The section's declared size field as stored on disk. Normally
    /// `payload.len() + 8`, but some real-world HDR animation files
    /// truncate the *last* section by a few bytes while leaving its size
    /// field claiming the un-truncated length. We re-emit the declared
    /// value (and the actual payload bytes) so the round-trip stays
    /// byte-identical.
    pub declared_size: u32,
}

impl BflanSection {
    /// The section's `magic` as a string (lossy).
    pub fn magic_str(&self) -> String {
        String::from_utf8_lossy(&self.magic).into_owned()
    }
}

impl Bflan {
    pub fn version_major(&self) -> u32 {
        (self.version >> 24) & 0xff
    }

    /// First section with the given magic, if any.
    pub fn section(&self, magic: &[u8; 4]) -> Option<&BflanSection> {
        self.sections.iter().find(|s| &s.magic == magic)
    }
}

fn read_u16(d: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([d[off], d[off + 1]])
}
fn read_u32(d: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([d[off], d[off + 1], d[off + 2], d[off + 3]])
}

/// Parse a BFLAN file.
pub fn read_bflan(data: &[u8]) -> Result<Bflan> {
    if data.len() < HEADER_FIXED {
        return Err(Error::Other("file too small for a BFLAN header".into()));
    }
    if data[0..4] != MAGIC_FLAN {
        return Err(Error::Other(format!(
            "not a BFLAN (magic = {:?})",
            String::from_utf8_lossy(&data[0..4])
        )));
    }
    let bom = read_u16(data, 4);
    let header_size = read_u16(data, 6);
    let version = read_u32(data, 8);
    let file_size = read_u32(data, 0x0C) as usize;
    let section_count = read_u16(data, 0x10) as usize;
    let padding = read_u16(data, 0x12);

    if file_size != data.len() {
        return Err(Error::Other(format!(
            "BFLAN header file_size {file_size} != actual length {}",
            data.len()
        )));
    }
    let hsz = header_size as usize;
    if hsz < HEADER_FIXED || hsz > data.len() {
        return Err(Error::Other(format!(
            "BFLAN header_size {hsz} is out of range"
        )));
    }
    let header_extra = data[HEADER_FIXED..hsz].to_vec();

    let mut sections = Vec::with_capacity(section_count);
    let mut off = hsz;
    for i in 0..section_count {
        if off + 8 > data.len() {
            return Err(Error::Other(format!(
                "BFLAN section[{i}] header runs past EOF at 0x{off:x}"
            )));
        }
        let mut magic = [0u8; 4];
        magic.copy_from_slice(&data[off..off + 4]);
        let size = read_u32(data, off + 4) as usize;
        if size < 8 {
            return Err(Error::Other(format!(
                "BFLAN section[{i}] '{}' has invalid size {size} at 0x{off:x}",
                String::from_utf8_lossy(&magic)
            )));
        }
        // Some HDR animation files truncate the final section a few bytes
        // short of its declared size. Clamp the captured payload to the
        // bytes that actually exist; the declared size is preserved
        // separately so we re-emit the file verbatim. A short read like
        // this is only acceptable on the last section (there's nothing
        // after it).
        let payload_end = (off + size).min(data.len());
        let truncated = off + size > data.len();
        if truncated && i + 1 != section_count {
            return Err(Error::Other(format!(
                "BFLAN section[{i}] '{}' size {size} at 0x{off:x} runs past EOF \
                 but is not the last section",
                String::from_utf8_lossy(&magic)
            )));
        }
        sections.push(BflanSection {
            magic,
            payload: data[off + 8..payload_end].to_vec(),
            declared_size: size as u32,
        });
        off = payload_end;
    }

    let trailing = data[off..].to_vec();

    Ok(Bflan {
        bom,
        header_size,
        version,
        padding,
        header_extra,
        sections,
        trailing,
    })
}

/// Serialize a BFLAN back to bytes. `file_size` and `section_count` are
/// recomputed; everything else (header fields, section bytes, trailing)
/// is reproduced verbatim, so an unmodified parse round-trips
/// byte-identically.
pub fn write_bflan(b: &Bflan) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC_FLAN);
    out.extend_from_slice(&b.bom.to_le_bytes());
    out.extend_from_slice(&b.header_size.to_le_bytes());
    out.extend_from_slice(&b.version.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // file_size placeholder @ 0x0C
    out.extend_from_slice(&0u16.to_le_bytes()); // section_count placeholder @ 0x10
    out.extend_from_slice(&b.padding.to_le_bytes());
    out.extend_from_slice(&b.header_extra);

    if out.len() != b.header_size as usize {
        return Err(Error::Other(format!(
            "BFLAN header reconstruction is {} bytes, expected header_size {}",
            out.len(),
            b.header_size
        )));
    }

    for s in &b.sections {
        // Emit the section's *declared* size verbatim (it may exceed
        // `payload.len() + 8` for a truncated final section) while
        // writing only the payload bytes that exist.
        out.extend_from_slice(&s.magic);
        out.extend_from_slice(&s.declared_size.to_le_bytes());
        out.extend_from_slice(&s.payload);
    }
    out.extend_from_slice(&b.trailing);

    let file_size = out.len() as u32;
    out[0x0C..0x10].copy_from_slice(&file_size.to_le_bytes());
    let section_count = b.sections.len() as u16;
    out[0x10..0x12].copy_from_slice(&section_count.to_le_bytes());

    Ok(out)
}

// ============================================================
// Read-only decode for `bflan-inspect`
// ============================================================

/// Decoded `pat1` animation tag (for display only).
#[derive(Debug, Clone)]
pub struct Pat1Info {
    pub animation_order: u16,
    pub start_frame: i16,
    pub end_frame: i16,
    pub child_binding: bool,
    pub name: String,
    pub groups: Vec<String>,
}

/// Decoded `pai1` animation info (top-level; per-tag keyframes not
/// expanded).
#[derive(Debug, Clone)]
pub struct Pai1Info {
    pub frame_size: u16,
    pub loops: bool,
    pub textures: Vec<String>,
    pub entries: Vec<Pai1Entry>,
}

/// One `pai1` entry (the thing being animated).
#[derive(Debug, Clone)]
pub struct Pai1Entry {
    pub name: String,
    pub target: u8,
    pub tag_count: u8,
}

fn read_fixed_string(d: &[u8], start: usize, len: usize) -> String {
    let end = (start + len).min(d.len());
    if start >= end {
        return String::new();
    }
    let slice = &d[start..end];
    let z = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    String::from_utf8_lossy(&slice[..z]).into_owned()
}

fn read_zstring(d: &[u8], start: usize) -> String {
    if start >= d.len() {
        return String::new();
    }
    let z = d[start..].iter().position(|&c| c == 0).map(|p| start + p).unwrap_or(d.len());
    String::from_utf8_lossy(&d[start..z]).into_owned()
}

/// Decode a `pat1` section payload. `version_major` selects the field
/// layout (v8+ has an extra u32 before the frame range). Returns `None`
/// if the payload is too short or the offsets are out of range.
pub fn decode_pat1(payload: &[u8], version_major: u32) -> Option<Pat1Info> {
    // Offsets inside pat1 are relative to the section's magic (8 bytes
    // before the payload start), so a payload-relative index is
    // `offset - 8`.
    if payload.len() < 12 {
        return None;
    }
    let animation_order = read_u16(payload, 0);
    let group_count = read_u16(payload, 2) as usize;
    let anim_name_offset = read_u32(payload, 4) as usize;
    let group_names_offset = read_u32(payload, 8) as usize;
    let frame_base = if version_major >= 8 { 16 } else { 12 };
    if payload.len() < frame_base + 5 {
        return None;
    }
    let start_frame = i16::from_le_bytes([payload[frame_base], payload[frame_base + 1]]);
    let end_frame = i16::from_le_bytes([payload[frame_base + 2], payload[frame_base + 3]]);
    let child_binding = payload[frame_base + 4] != 0;

    let name = anim_name_offset
        .checked_sub(8)
        .map(|i| read_zstring(payload, i))
        .unwrap_or_default();

    let mut groups = Vec::with_capacity(group_count);
    if let Some(base) = group_names_offset.checked_sub(8) {
        for g in 0..group_count {
            groups.push(read_fixed_string(payload, base + g * 28, 28));
        }
    }

    Some(Pat1Info {
        animation_order,
        start_frame,
        end_frame,
        child_binding,
        name,
        groups,
    })
}

/// Decode a `pai1` section payload (top-level fields, texture list, and
/// entry summaries). Returns `None` on a too-short / inconsistent payload.
pub fn decode_pai1(payload: &[u8]) -> Option<Pai1Info> {
    if payload.len() < 12 {
        return None;
    }
    let frame_size = read_u16(payload, 0);
    let loops = payload[2] != 0;
    let num_textures = read_u16(payload, 4) as usize;
    let num_entries = read_u16(payload, 6) as usize;
    let entry_offset_tbl = read_u32(payload, 8) as usize;

    // Texture name table starts right after the 12-byte header; its
    // offsets are relative to that position (payload index 12).
    let tex_start = 12usize;
    let mut textures = Vec::with_capacity(num_textures);
    for i in 0..num_textures {
        let off_pos = tex_start + i * 4;
        if off_pos + 4 > payload.len() {
            break;
        }
        let rel = read_u32(payload, off_pos) as usize;
        textures.push(read_zstring(payload, tex_start + rel));
    }

    // Entry offset table is at `entry_offset_tbl` (relative to the section
    // magic, i.e. payload index `entry_offset_tbl - 8`).
    let mut entries = Vec::with_capacity(num_entries);
    if let Some(tbl) = entry_offset_tbl.checked_sub(8) {
        for i in 0..num_entries {
            let off_pos = tbl + i * 4;
            if off_pos + 4 > payload.len() {
                break;
            }
            let rel = read_u32(payload, off_pos) as usize;
            let Some(entry_at) = rel.checked_sub(8) else {
                continue;
            };
            if entry_at + 30 > payload.len() {
                continue;
            }
            let name = read_fixed_string(payload, entry_at, 28);
            let tag_count = payload[entry_at + 28];
            let target = payload[entry_at + 29];
            entries.push(Pai1Entry {
                name,
                target,
                tag_count,
            });
        }
    }

    Some(Pai1Info {
        frame_size,
        loops,
        textures,
        entries,
    })
}
