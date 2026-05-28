//! BNTX parser. Captures every field needed for byte-identical
//! round-trip: file headers, string pool, dictionary trie, texture
//! metadata, raw BRTD pixel data, and the relocation table.

use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{Cursor, Read};

use super::*;
use super::error::Error;

const MAGIC_BNTX: [u8; 4] = *b"BNTX";
const MAGIC_NX: [u8; 4] = *b"NX  ";
const MAGIC_STR: [u8; 4] = *b"_STR";
const MAGIC_DIC: [u8; 4] = *b"_DIC";
const MAGIC_BRTI: [u8; 4] = *b"BRTI";
const MAGIC_BRTD: [u8; 4] = *b"BRTD";
const MAGIC_RLT: [u8; 4] = *b"_RLT";

const MEMORY_POOL_SIZE: usize = 0x150;
const MEMORY_POOL_START: usize = 0x48;
const TEX_PTR_ARRAY_START: usize = MEMORY_POOL_START + MEMORY_POOL_SIZE; // 0x198

pub fn read_bntx(data: &[u8]) -> Result<BntxFile, Error> {
    if data.len() < 0x48 {
        return Err(Error::Truncated("file is too small for BNTX+NX headers".into()));
    }

    // ---------- BNTX header (0x00-0x1F) ----------
    let header = read_bntx_header(data)?;
    let file_size_field = read_u32(data, 0x1C);
    if file_size_field as usize != data.len() {
        return Err(Error::Format(format!(
            "header file_size 0x{file_size_field:x} != actual file length 0x{:x}",
            data.len()
        )));
    }
    let reloc_table_offset = read_u32(data, 0x18) as usize;

    // ---------- NX header (0x20-0x47) ----------
    let nx_magic = &data[0x20..0x24];
    if nx_magic != MAGIC_NX {
        return Err(Error::Format(format!(
            "expected NX header magic, got {:?}",
            std::str::from_utf8(nx_magic).unwrap_or("?")
        )));
    }
    let count = read_u32(data, 0x24) as usize;
    let info_ptrs_off = read_u64(data, 0x28) as usize;
    let _data_blk_ptr = read_u64(data, 0x30) as usize;
    let dict_off = read_u64(data, 0x38) as usize;
    let dict_size_field = read_u64(data, 0x40);

    if info_ptrs_off != TEX_PTR_ARRAY_START {
        return Err(Error::Format(format!(
            "info_ptrs_off=0x{info_ptrs_off:x} (expected 0x{TEX_PTR_ARRAY_START:x})"
        )));
    }

    // ---------- Memory pool (must be all zeros) ----------
    if !data[MEMORY_POOL_START..TEX_PTR_ARRAY_START].iter().all(|&b| b == 0) {
        return Err(Error::Format(
            "memory pool is non-zero (unexpected for the BNTX format we target)".into(),
        ));
    }

    // ---------- Texture-info pointer array ----------
    let mut texture_info_ptrs = Vec::with_capacity(count);
    for i in 0..count {
        let p = read_u64(data, info_ptrs_off + i * 8) as usize;
        if p == 0 || p >= data.len() {
            return Err(Error::Format(format!(
                "texture info pointer[{i}] = 0x{p:x} is out of bounds"
            )));
        }
        texture_info_ptrs.push(p);
    }

    // ---------- Container name ----------
    // `filename_offset` points to the BODY of the container name string
    // (i.e., past the u16 length field) -- different from dict and BRTI
    // name pointers, which point to the BntxStr struct start. We read it
    // as a NUL-terminated C string.
    let name = read_c_string(data, header.filename_offset as usize)?;

    // ---------- _STR section ----------
    // The _STR section follows the texture-info pointer array. The exact
    // start offset is implied by `header.first_block_offset` for our files.
    let str_off = header.first_block_offset as usize;
    let (strings, _str_section_end) = read_str_section(data, str_off)?;

    // ---------- _DIC section ----------
    let (dict, _dict_end) = read_dict_section(data, dict_off, &strings)?;

    // ---------- BRTI sections (per texture) ----------
    let mut textures = Vec::with_capacity(count);
    for &brti_off in &texture_info_ptrs {
        textures.push(read_brti(data, brti_off, &strings)?);
    }
    // Suppress unused-mut: we keep `mut` for symmetry with future passes
    // that may post-process the texture list.
    let _ = &mut textures;

    // ---------- BRTD section (texture data block) ----------
    let brtd_off = _data_blk_ptr;
    if &data[brtd_off..brtd_off + 4] != MAGIC_BRTD {
        return Err(Error::Format(format!(
            "expected BRTD magic at 0x{brtd_off:x}"
        )));
    }
    let brtd_block_size = read_u64(data, brtd_off + 8);
    let brtd_data_start = brtd_off + 0x10;
    let brtd_data_end = brtd_off + brtd_block_size as usize;
    if brtd_data_end > reloc_table_offset {
        return Err(Error::Format(format!(
            "BRTD data extends past _RLT (data_end=0x{brtd_data_end:x}, reloc=0x{reloc_table_offset:x})"
        )));
    }
    let brtd_raw = data[brtd_data_start..brtd_data_end].to_vec();

    // ---------- _RLT relocation table ----------
    let relocation_table = read_relocation_table(data, reloc_table_offset)?;

    Ok(BntxFile {
        header,
        nx_header: NxHeader { dict_size_field },
        name,
        strings,
        dict,
        textures,
        brtd: BrtdSection {
            declared_block_size: brtd_block_size,
            data: brtd_raw,
        },
        relocation_table,
        relocation_table_dirty: false,
    })
}

// ============================================================
// Section parsers
// ============================================================

fn read_bntx_header(data: &[u8]) -> Result<BntxHeader, Error> {
    let mut c = Cursor::new(&data[..0x20]);
    let mut magic = [0u8; 4];
    c.read_exact(&mut magic)?;
    if magic != MAGIC_BNTX {
        return Err(Error::BadMagic(magic));
    }
    let _padding = c.read_u32::<LittleEndian>()?;
    let version = c.read_u32::<LittleEndian>()?;
    let bom = c.read_u16::<LittleEndian>()?;
    if bom != 0xFEFF {
        return Err(Error::Format(format!("unexpected BOM 0x{bom:04x}")));
    }
    let alignment_shift = c.read_u8()?;
    let target_address_size = c.read_u8()?;
    let filename_offset = c.read_u32::<LittleEndian>()?;
    let flag = c.read_u16::<LittleEndian>()?;
    let first_block_offset = c.read_u16::<LittleEndian>()?;
    let _reloc_off = c.read_u32::<LittleEndian>()?;
    let _file_size = c.read_u32::<LittleEndian>()?;

    if version != 0x00040000 {
        // Other versions exist but we don't try to support them here.
        return Err(Error::UnsupportedVersion(version));
    }
    if target_address_size != 64 {
        return Err(Error::Format(format!(
            "target_address_size = {target_address_size} (only 64-bit BNTX supported)"
        )));
    }

    Ok(BntxHeader {
        version,
        alignment_shift,
        target_address_size,
        flag,
        first_block_offset,
        filename_offset,
    })
}

fn read_str_section(data: &[u8], offset: usize) -> Result<(Vec<String>, usize), Error> {
    let magic = &data[offset..offset + 4];
    if magic != MAGIC_STR {
        return Err(Error::Format(format!(
            "expected _STR at 0x{offset:x}, got {:?}",
            std::str::from_utf8(magic).unwrap_or("?")
        )));
    }
    let _unk1 = read_u32(data, offset + 4);
    let block_size = read_u64(data, offset + 8) as usize;
    let str_count = read_u32(data, offset + 0x10) as usize;
    let strings_start = offset + 0x14;

    // The on-disk layout is `[_STR header][empty BntxStr][str_count BntxStrs]`,
    // for a total of (str_count + 1) entries. Index 0 is the empty
    // sentinel (BNTX's "no name" placeholder).
    let total_strings = str_count + 1;

    // Each entry is `u16 len + bytes + null + pad-to-2`. The chars+null
    // block is rounded up to an even number of bytes.
    let mut strings = Vec::with_capacity(total_strings);
    let mut cursor = strings_start;
    let section_end = offset + block_size;
    for i in 0..total_strings {
        if cursor + 2 > section_end {
            return Err(Error::Truncated(format!(
                "_STR string[{i}] header at 0x{cursor:x}"
            )));
        }
        let len = u16::from_le_bytes([data[cursor], data[cursor + 1]]) as usize;
        let body_start = cursor + 2;
        if body_start + len > section_end {
            return Err(Error::Truncated(format!(
                "_STR string[{i}] body (len={len})"
            )));
        }
        let body = String::from_utf8_lossy(&data[body_start..body_start + len]).into_owned();
        strings.push(body);
        let entry_end = body_start + len + 1; // +1 null terminator
        let aligned = (entry_end + 1) & !1;
        cursor = aligned;
    }
    Ok((strings, section_end))
}

fn read_dict_section(
    data: &[u8],
    offset: usize,
    strings: &[String],
) -> Result<(DictSection, usize), Error> {
    let magic = &data[offset..offset + 4];
    if magic != MAGIC_DIC {
        return Err(Error::Format(format!(
            "expected _DIC at 0x{offset:x}"
        )));
    }
    let count = read_u32(data, offset + 4);
    let entry_count = (count + 1) as usize; // +1 root sentinel
    let entries_start = offset + 8;
    let mut entries = Vec::with_capacity(entry_count);
    for i in 0..entry_count {
        let pos = entries_start + i * 16;
        let ref_bit = read_u32(data, pos);
        let left = read_u16(data, pos + 4);
        let right = read_u16(data, pos + 6);
        let name_ptr = read_u64(data, pos + 8) as usize;
        // Resolve the name pointer to a string index by matching against
        // the offset table we'd compute from `strings`. For a clean
        // round-trip without re-deriving offsets, we just match name_ptr
        // back to its string by looking it up in the file directly.
        let string_index = if name_ptr == 0 {
            0 // sentinel — empty string at index 0
        } else {
            string_index_at_offset(data, name_ptr, strings)?
        };
        entries.push(DictEntry {
            ref_bit,
            left,
            right,
            string_index,
        });
    }
    Ok((
        DictSection {
            count,
            entries,
        },
        entries_start + entry_count * 16,
    ))
}

/// Resolve a file-absolute `BntxStr` pointer to the matching index in
/// `strings`. We do this by reading the string at that offset and finding
/// the matching entry. (The strings are unique in well-formed BNTX files.)
fn string_index_at_offset(
    data: &[u8],
    offset: usize,
    strings: &[String],
) -> Result<u32, Error> {
    let s = read_bntx_str(data, offset)?;
    strings
        .iter()
        .position(|x| x == &s)
        .map(|i| i as u32)
        .ok_or_else(|| {
            Error::Format(format!(
                "dict references string '{s}' at 0x{offset:x} that isn't in _STR (parsed {} strings; first/last: '{}'/'{}')",
                strings.len(),
                strings.first().map(String::as_str).unwrap_or(""),
                strings.last().map(String::as_str).unwrap_or(""),
            ))
        })
}

fn read_brti(data: &[u8], offset: usize, strings: &[String]) -> Result<Texture, Error> {
    let magic = &data[offset..offset + 4];
    if magic != MAGIC_BRTI {
        return Err(Error::Format(format!(
            "expected BRTI at 0x{offset:x}, got {:?}",
            std::str::from_utf8(magic).unwrap_or("?")
        )));
    }
    let mut c = Cursor::new(&data[offset..offset + 0xA0]);
    c.read_exact(&mut [0u8; 4])?; // skip magic
    let _size = c.read_u32::<LittleEndian>()?;
    let _size2 = c.read_u64::<LittleEndian>()?;
    let flags = c.read_u8()?;
    let dim = c.read_u8()?;
    let tile_mode = c.read_u16::<LittleEndian>()?;
    let swizzle = c.read_u16::<LittleEndian>()?;
    let mips_count = c.read_u16::<LittleEndian>()?;
    let num_multi_sample = c.read_u32::<LittleEndian>()?;
    let format_code = c.read_u32::<LittleEndian>()?;
    let format = TextureFormat::from_surface_format(format_code)
        .ok_or(Error::UnsupportedFormat(format_code))?;
    let unk2 = c.read_u32::<LittleEndian>()?;
    let width = c.read_u32::<LittleEndian>()?;
    let height = c.read_u32::<LittleEndian>()?;
    let depth = c.read_u32::<LittleEndian>()?;
    let array_len = c.read_u32::<LittleEndian>()?;
    let size_range = c.read_i32::<LittleEndian>()?;
    let mut unk4 = [0u32; 6];
    for slot in &mut unk4 {
        *slot = c.read_u32::<LittleEndian>()?;
    }
    let image_size = c.read_u32::<LittleEndian>()?;
    let align = c.read_u32::<LittleEndian>()?;
    let channel_swizzle = c.read_u32::<LittleEndian>()?;
    let ty = c.read_u32::<LittleEndian>()?;
    let name_addr = c.read_u64::<LittleEndian>()? as usize;
    let parent_addr = c.read_u64::<LittleEndian>()?;
    let texture_addr_indirect = c.read_u64::<LittleEndian>()? as usize;

    // Look up the texture name index.
    let name_string_index = if name_addr == 0 {
        0
    } else {
        string_index_at_offset(data, name_addr, strings)?
    };

    // The indirection: read u64 at `texture_addr_indirect` to get the
    // file-absolute pixel-data offset, then convert to BRTD-relative.
    if texture_addr_indirect + 8 > data.len() {
        return Err(Error::Truncated(format!(
            "BRTI texture indirection at 0x{texture_addr_indirect:x}"
        )));
    }
    let pixel_data_off = read_u64(data, texture_addr_indirect) as usize;

    // Find BRTD start so we can convert.
    let brtd_off = find_brtd(data)?;
    let brtd_data_start = brtd_off + 0x10;
    if pixel_data_off < brtd_data_start {
        return Err(Error::Format(format!(
            "BRTI texture pointer 0x{pixel_data_off:x} is before BRTD data start 0x{brtd_data_start:x}"
        )));
    }
    let data_offset_in_brtd = pixel_data_off - brtd_data_start;

    Ok(Texture {
        name_string_index,
        flags,
        dim,
        tile_mode,
        swizzle,
        mips_count,
        num_multi_sample,
        format,
        unk2,
        width,
        height,
        depth,
        array_len,
        size_range,
        unk4,
        image_size,
        align,
        channel_swizzle,
        ty,
        parent_addr,
        data_offset_in_brtd,
    })
}

fn find_brtd(data: &[u8]) -> Result<usize, Error> {
    // The BRTD offset is recorded in the NX header at 0x30.
    let off = read_u64(data, 0x30) as usize;
    if off + 4 > data.len() || &data[off..off + 4] != MAGIC_BRTD {
        return Err(Error::Format("BRTD section not found".into()));
    }
    Ok(off)
}

fn read_relocation_table(data: &[u8], offset: usize) -> Result<RelocationTable, Error> {
    let magic = &data[offset..offset + 4];
    if magic != MAGIC_RLT {
        return Err(Error::Format(format!(
            "expected _RLT at 0x{offset:x}"
        )));
    }
    let _self_off = read_u32(data, offset + 4);
    let section_count = read_u32(data, offset + 8) as usize;
    let _padding = read_u32(data, offset + 0xC);

    let mut sections = Vec::with_capacity(section_count);
    let mut total_entries = 0u32;
    for i in 0..section_count {
        let pos = offset + 0x10 + i * 24;
        let pointer = read_u64(data, pos);
        let position = read_u32(data, pos + 8);
        let size = read_u32(data, pos + 12);
        let index = read_u32(data, pos + 16);
        let count = read_u32(data, pos + 20);
        total_entries += count;
        sections.push(RltSection {
            pointer,
            position,
            size,
            index,
            count,
        });
    }
    let entries_start = offset + 0x10 + section_count * 24;
    let mut entries = Vec::with_capacity(total_entries as usize);
    for i in 0..total_entries as usize {
        let pos = entries_start + i * 8;
        let position = read_u32(data, pos);
        let struct_count = read_u16(data, pos + 4);
        let offset_count = data[pos + 6];
        let padding_count = data[pos + 7];
        entries.push(RltEntry {
            position,
            struct_count,
            offset_count,
            padding_count,
        });
    }
    Ok(RelocationTable { sections, entries })
}

// ============================================================
// Low-level helpers
// ============================================================

fn read_u16(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([data[off], data[off + 1]])
}

fn read_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

fn read_u64(data: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        data[off], data[off + 1], data[off + 2], data[off + 3],
        data[off + 4], data[off + 5], data[off + 6], data[off + 7],
    ])
}

fn read_bntx_str(data: &[u8], offset: usize) -> Result<String, Error> {
    if offset + 2 > data.len() {
        return Err(Error::Truncated(format!(
            "BntxStr header at 0x{offset:x}"
        )));
    }
    let len = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
    let start = offset + 2;
    if start + len > data.len() {
        return Err(Error::Truncated(format!(
            "BntxStr body at 0x{offset:x} (len={len})"
        )));
    }
    Ok(String::from_utf8_lossy(&data[start..start + len]).into_owned())
}

/// Read a NUL-terminated C string at `offset`. Used for the BNTX
/// container name, whose `filename_offset` skips the BntxStr length
/// field and points directly at the body bytes.
fn read_c_string(data: &[u8], offset: usize) -> Result<String, Error> {
    if offset >= data.len() {
        return Err(Error::Truncated(format!(
            "C string at 0x{offset:x} starts past EOF"
        )));
    }
    let end = data[offset..]
        .iter()
        .position(|&b| b == 0)
        .map(|p| offset + p)
        .unwrap_or(data.len());
    Ok(String::from_utf8_lossy(&data[offset..end]).into_owned())
}
