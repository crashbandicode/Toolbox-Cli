//! BNTX writer. Reverses `read_bntx`, recomputing every offset and the
//! relocation-table positions so the output is byte-identical to the
//! original on a no-op round-trip.
//!
//! Key invariants preserved (matching real Switch BNTX layout):
//!
//! - File header is 32 bytes, NX header is 40 bytes, memory pool is 0x150
//!   zero bytes; texture-info pointer array starts at 0x198.
//! - Each BNTX string is `u16 len + bytes + 0x00 + (1 byte pad if len is
//!   even)`, i.e. the chars+null block is rounded up to an even number of
//!   bytes. The `_STR` block also includes one implicit empty string
//!   before the `str_count` named strings.
//! - The `_DIC` Patricia trie occupies `4 + 4 + 16 * (count + 1)` bytes
//!   immediately after `_STR`. Trie entries are emitted verbatim — we
//!   only update the embedded `name_ptr` to point at the current
//!   string-pool layout.
//! - Each BRTI entry is 0xA0 bytes followed by a 0x208-byte trailing
//!   block: `0x100` zeros + `0x100` zeros + an 8-byte u64 that holds the
//!   file-absolute offset of this texture's pixel data inside BRTD.
//! - BRTD = `magic(4) + 0x00000000(4) + block_size(u64=0x10 + data_len) + data`.
//! - `_RLT` is emitted verbatim (sections + entries copied as-is) when no
//!   structural changes were made; for structural changes it must be
//!   recomputed.

use byteorder::{LittleEndian, WriteBytesExt};
use std::io::Write;

use super::*;
use super::error::Error;

const BNTX_HEADER_SIZE: usize = 0x20;
const NX_HEADER_SIZE: usize = 0x28;
const MEMORY_POOL_SIZE: usize = 0x150;
const HEADER_SIZE: usize = BNTX_HEADER_SIZE + NX_HEADER_SIZE; // 0x48
const TEX_PTR_ARRAY_START: usize = HEADER_SIZE + MEMORY_POOL_SIZE; // 0x198

/// Each BRTI is followed by `0x100 + 0x100 + 8` bytes of fixed-format
/// trailing data: two zero blocks then the indirect texture-data pointer.
const BRTI_HEADER_SIZE: usize = 0xA0;
const BRTI_TRAILING_SIZE: usize = 0x208;
const BRTI_BLOCK_STRIDE: usize = BRTI_HEADER_SIZE + BRTI_TRAILING_SIZE; // 0x2A8

const BRTD_HEADER_SIZE: usize = 0x10;

pub fn write_bntx(b: &BntxFile) -> Result<Vec<u8>, Error> {
    // ---- Compute the layout. We need these offsets to fill in pointers. ----
    let info_ptrs_off = TEX_PTR_ARRAY_START;
    let str_section_off = info_ptrs_off + b.textures.len() * 8;

    // String layout: emit the "_STR" header + count + strings.
    let str_layout = compute_str_layout(b, str_section_off);
    let dict_section_off = str_layout.end;

    // BNTX header `filename_offset` points to the BODY of the container
    // name string (i.e., past the u16 length field — different from dict
    // and BRTI name pointers, which point to the BntxStr start). The
    // container name lives at strings[1] in our model.
    let filename_offset = if b.strings.len() > 1 {
        (str_layout.string_offsets[1] + 2) as u32
    } else {
        0
    };

    // Dict size: 4 magic + 4 count + (count+1)*16 entries
    let dict_size = 8 + (b.dict.entries.len() * 16);
    let dict_end = dict_section_off + dict_size;

    // BRTI sections start immediately after dict (no padding observed in
    // real files — verify against fixture).
    let brti_array_start = dict_end;
    let brti_array_end = brti_array_start + b.textures.len() * BRTI_BLOCK_STRIDE;

    // BRTD starts at the next texture-data-alignment boundary after the
    // BRTI array (alignment = 1 << alignment_shift; 0x1000 for the Smash
    // files), accounting for the BRTD header. The last BRTI's recorded
    // block size absorbs the padding before BRTD, matching real files.
    let texture_data_alignment = 1usize << b.header.alignment_shift;
    let brti_padding_needed = align_up_pad(brti_array_end + BRTD_HEADER_SIZE, texture_data_alignment);
    let brtd_section_off = brti_array_end + brti_padding_needed;
    let brtd_data_start = brtd_section_off + BRTD_HEADER_SIZE;
    let brtd_data_end = brtd_data_start + b.brtd.data.len();

    // Choose between preserving the original RLT verbatim (round-trip
    // case for files written by other tools with idiosyncratic layouts)
    // or regenerating a canonical RLT (after structural modifications
    // like `append_texture`). Real Nintendo BNTX uses an 8-entry compact
    // table keyed by structural landmarks; some other tools emit one
    // entry per pointer. Either is functionally valid as long as every
    // relocated pointer's position is listed.
    let rlt_owned;
    let rlt: &RelocationTable = if b.relocation_table_dirty {
        rlt_owned = build_canonical_reloc_table(
            b,
            info_ptrs_off,
            dict_section_off,
            brti_array_start,
            brtd_section_off,
        );
        &rlt_owned
    } else {
        &b.relocation_table
    };
    let reloc_table_off = brtd_data_end;
    let reloc_table_size =
        16 + rlt.sections.len() * 24 + rlt.entries.len() * 8;
    let file_size = reloc_table_off + reloc_table_size;

    // ---- Emit bytes. ----
    let mut out = vec![0u8; file_size];

    // BNTX header.
    write_bntx_header(
        &mut out,
        b,
        file_size,
        reloc_table_off,
        str_section_off,
        filename_offset,
    )?;

    // NX header.
    write_nx_header(&mut out, b, info_ptrs_off, brtd_section_off, dict_section_off)?;

    // Memory pool already initialized to zero.

    // Texture-info pointer array.
    for (i, _tex) in b.textures.iter().enumerate() {
        let brti_off = brti_array_start + i * BRTI_BLOCK_STRIDE;
        write_u64_at(&mut out, info_ptrs_off + i * 8, brti_off as u64);
    }

    // _STR section.
    write_str_section(&mut out, b, str_section_off, &str_layout)?;

    // _DIC section.
    write_dict_section(&mut out, b, dict_section_off, &str_layout)?;

    // BRTI sections + per-texture trailing blocks.
    let last_idx = b.textures.len() - 1;
    for (i, tex) in b.textures.iter().enumerate() {
        let brti_off = brti_array_start + i * BRTI_BLOCK_STRIDE;
        let trailing_off = brti_off + BRTI_HEADER_SIZE;
        let texture_indirect_slot = trailing_off + 0x200;
        let pixel_data_abs = brtd_data_start + tex.data_offset_in_brtd;

        // Block size: distance to the next BRTI (or to BRTD for the last
        // entry, which absorbs any alignment padding before BRTD).
        let block_size = if i == last_idx {
            (brtd_section_off - brti_off) as u32
        } else {
            BRTI_BLOCK_STRIDE as u32
        };

        write_brti(
            &mut out,
            brti_off,
            tex,
            &str_layout,
            texture_indirect_slot,
            block_size,
        )?;
        // 0x100 + 0x100 zeros are already there. Write the indirect ptr.
        write_u64_at(&mut out, texture_indirect_slot, pixel_data_abs as u64);
    }

    // BRTD section.
    write_brtd(&mut out, brtd_section_off, b)?;

    // _RLT relocation table — emit either the preserved or canonical
    // version (chosen above based on `relocation_table_dirty`).
    write_reloc_table(&mut out, reloc_table_off, rlt)?;

    Ok(out)
}

/// Build a fresh `RelocationTable` matching the canonical Nintendo BNTX
/// layout for `b`'s current structure (texture count, dict size, BRTI
/// positions). This is what real `__Combined.bntx` files emit when
/// produced by Nintendo's tooling: 2 sections + 8 entries per file
/// regardless of texture count, with per-entry struct counts driven by
/// `b.textures.len()`.
fn build_canonical_reloc_table(
    b: &BntxFile,
    info_ptrs_off: usize,
    dict_section_off: usize,
    brti_array_start: usize,
    brtd_section_off: usize,
) -> RelocationTable {
    let n = b.textures.len() as u16;
    let dict_entry_count = b.dict.entries.len() as u16; // n + 1

    // Section 0 covers everything from file start through the last BRTI
    // block. Section 1 covers the BRTD section (data_blk_ptr +
    // texture-data indirection slots).
    let section_0_end = brti_array_start + (n as usize) * BRTI_BLOCK_STRIDE;
    let sections = vec![
        RltSection {
            pointer: 0,
            position: 0,
            size: section_0_end as u32,
            index: 0,
            count: 6,
        },
        RltSection {
            pointer: 0,
            position: brtd_section_off as u32,
            size: ((BRTD_HEADER_SIZE + b.brtd.data.len()) as u32),
            index: 6,
            count: 2,
        },
    ];

    let entries = vec![
        // Entry 0: NX header `info_ptrs_off` (1 ptr at file offset 0x28).
        RltEntry {
            position: 0x28,
            struct_count: 1,
            offset_count: 1,
            padding_count: 0,
        },
        // Entry 1: NX header `dict_off` + `dict_size_field` (2 consecutive
        // ptrs at 0x38).
        RltEntry {
            position: 0x38,
            struct_count: 1,
            offset_count: 2,
            padding_count: 0,
        },
        // Entry 2: texture-info pointer array (`n` ptrs starting at 0x198).
        RltEntry {
            position: info_ptrs_off as u32,
            struct_count: 1,
            offset_count: n as u8,
            padding_count: 0,
        },
        // Entry 3: dict name_ptrs (one ptr per dict entry, at +0x10 of
        // each 16-byte dict entry; stride 2 qwords -- offset_count=1,
        // padding_count=1).
        RltEntry {
            position: (dict_section_off + 0x10) as u32,
            struct_count: dict_entry_count,
            offset_count: 1,
            padding_count: 1,
        },
        // Entry 4: BRTI name_addr/parent_addr/texture_addr_indirect (3
        // consecutive ptrs at BRTI+0x60; stride is BRTI_BLOCK_STRIDE = 85
        // qwords = offset_count=3 + padding_count=82).
        RltEntry {
            position: (brti_array_start + 0x60) as u32,
            struct_count: n,
            offset_count: 3,
            padding_count: (BRTI_BLOCK_STRIDE / 8 - 3) as u8,
        },
        // Entry 5: BRTI's 2 trailing-block pointers at +0x80, +0x88
        // (stride 85 qwords; offset_count=2, padding_count=83).
        RltEntry {
            position: (brti_array_start + 0x80) as u32,
            struct_count: n,
            offset_count: 2,
            padding_count: (BRTI_BLOCK_STRIDE / 8 - 2) as u8,
        },
        // Entry 6: NX header `data_blk_ptr` (1 ptr at 0x30) -- belongs to
        // section 1 because it points at BRTD.
        RltEntry {
            position: 0x30,
            struct_count: 1,
            offset_count: 1,
            padding_count: 0,
        },
        // Entry 7: per-texture indirection slot at BRTI+0x2A0 (1 ptr per
        // BRTI; stride 85 qwords).
        RltEntry {
            position: (brti_array_start + 0x2A0) as u32,
            struct_count: n,
            offset_count: 1,
            padding_count: (BRTI_BLOCK_STRIDE / 8 - 1) as u8,
        },
    ];

    RelocationTable { sections, entries }
}

// ============================================================
// Sub-writers
// ============================================================

fn write_bntx_header(
    out: &mut [u8],
    b: &BntxFile,
    file_size: usize,
    reloc_table_off: usize,
    str_section_off: usize,
    filename_offset: u32,
) -> Result<(), Error> {
    let mut c = std::io::Cursor::new(&mut out[..0x20]);
    c.write_all(b"BNTX")?;
    c.write_u32::<LittleEndian>(0)?; // padding
    c.write_u32::<LittleEndian>(b.header.version)?;
    c.write_u16::<LittleEndian>(0xFEFF)?;
    c.write_u8(b.header.alignment_shift)?;
    c.write_u8(b.header.target_address_size)?;
    // filename_offset and first_block_offset are recomputed because the
    // texture-info pointer array (which lives between the memory pool
    // and `_STR`) changes size when textures are added/removed.
    c.write_u32::<LittleEndian>(filename_offset)?;
    c.write_u16::<LittleEndian>(b.header.flag)?;
    c.write_u16::<LittleEndian>(str_section_off as u16)?;
    c.write_u32::<LittleEndian>(reloc_table_off as u32)?;
    c.write_u32::<LittleEndian>(file_size as u32)?;
    Ok(())
}

fn write_nx_header(
    out: &mut [u8],
    b: &BntxFile,
    info_ptrs_off: usize,
    brtd_off: usize,
    dict_off: usize,
) -> Result<(), Error> {
    let mut c = std::io::Cursor::new(&mut out[0x20..0x48]);
    c.write_all(b"NX  ")?;
    c.write_u32::<LittleEndian>(b.textures.len() as u32)?;
    c.write_u64::<LittleEndian>(info_ptrs_off as u64)?;
    c.write_u64::<LittleEndian>(brtd_off as u64)?;
    c.write_u64::<LittleEndian>(dict_off as u64)?;
    c.write_u64::<LittleEndian>(b.nx_header.dict_size_field)?;
    Ok(())
}

/// Layout of strings in the `_STR` section: per-entry start offsets so
/// the dictionary writer can emit `name_ptr` for each `string_index`.
struct StrLayout {
    /// File-absolute byte offset of each string's `BntxStr` start
    /// (i.e., the u16 length field). Indexed by `string_index`.
    string_offsets: Vec<usize>,
    /// File-absolute byte offset just past the `_STR` block (= start of
    /// `_DIC`).
    end: usize,
    /// Total `block_size` to write into the `_STR` header (covers `_STR`
    /// payload AND the `_DIC` block — empirically that's what real files
    /// record, even though it spans two sections).
    block_size: u64,
}

fn compute_str_layout(b: &BntxFile, section_off: usize) -> StrLayout {
    // Strings starting offset (after _STR header: 4 magic + 4 unk + 8 size + 4 count = 20 bytes).
    let strings_start = section_off + 0x14;
    let mut cur = strings_start;
    let mut offsets = Vec::with_capacity(b.strings.len());
    for s in &b.strings {
        offsets.push(cur);
        let entry_size = bntx_str_size(s);
        cur += entry_size;
    }
    // Pad the end of the string block so the `_DIC` section that follows
    // starts on an 8-byte boundary (its 16-byte entries assume 8-byte
    // alignment). Real Smash files have 0 to 6 bytes of trailing zero
    // padding here.
    let aligned_end = (cur + 7) & !7;
    let end = aligned_end;

    // The `_STR.block_size` empirically covers `_STR` AND `_DIC` together
    // (i.e., the size from `_STR` magic to the BRTI region).
    let dict_size = 8 + b.dict.entries.len() * 16;
    let block_size = (end - section_off + dict_size) as u64;

    StrLayout {
        string_offsets: offsets,
        end,
        block_size,
    }
}

/// Bytes consumed on disk by a `BntxStr` (u16 len + body + null + pad-to-2).
fn bntx_str_size(s: &str) -> usize {
    let body_plus_null = s.len() + 1;
    let aligned = (body_plus_null + 1) & !1;
    2 + aligned
}

fn write_str_section(
    out: &mut [u8],
    b: &BntxFile,
    offset: usize,
    layout: &StrLayout,
) -> Result<(), Error> {
    {
        let mut c = std::io::Cursor::new(&mut out[offset..offset + 0x14]);
        c.write_all(b"_STR")?;
        c.write_u32::<LittleEndian>(layout.block_size as u32)?;
        c.write_u64::<LittleEndian>(layout.block_size)?;
        // str_count is `total_strings - 1` (the empty sentinel is implicit).
        c.write_u32::<LittleEndian>((b.strings.len() - 1) as u32)?;
    }

    for (i, s) in b.strings.iter().enumerate() {
        let pos = layout.string_offsets[i];
        let len = s.len() as u16;
        out[pos..pos + 2].copy_from_slice(&len.to_le_bytes());
        out[pos + 2..pos + 2 + s.len()].copy_from_slice(s.as_bytes());
        // Null + pad bytes already zero (vec was zero-initialized).
    }
    Ok(())
}

fn write_dict_section(
    out: &mut [u8],
    b: &BntxFile,
    offset: usize,
    str_layout: &StrLayout,
) -> Result<(), Error> {
    {
        let mut c = std::io::Cursor::new(&mut out[offset..offset + 8]);
        c.write_all(b"_DIC")?;
        c.write_u32::<LittleEndian>(b.dict.count)?;
    }

    for (i, e) in b.dict.entries.iter().enumerate() {
        let pos = offset + 8 + i * 16;
        out[pos..pos + 4].copy_from_slice(&e.ref_bit.to_le_bytes());
        out[pos + 4..pos + 6].copy_from_slice(&e.left.to_le_bytes());
        out[pos + 6..pos + 8].copy_from_slice(&e.right.to_le_bytes());
        // For the root sentinel (entry 0), the file we observed actually
        // points to the empty string. Handle index 0 specially only if
        // the captured `string_index` is the empty-string index.
        let name_ptr = str_layout.string_offsets[e.string_index as usize] as u64;
        out[pos + 8..pos + 16].copy_from_slice(&name_ptr.to_le_bytes());
    }
    Ok(())
}

fn write_brti(
    out: &mut [u8],
    offset: usize,
    tex: &Texture,
    str_layout: &StrLayout,
    texture_indirect_slot: usize,
    block_size: u32,
) -> Result<(), Error> {
    let mut c = std::io::Cursor::new(&mut out[offset..offset + BRTI_HEADER_SIZE]);
    c.write_all(b"BRTI")?;
    // size + size2: distance from this BRTI's magic to the start of the
    // next block. For all but the last BRTI this equals
    // `BRTI_BLOCK_STRIDE`; the last BRTI absorbs alignment padding to
    // BRTD.
    c.write_u32::<LittleEndian>(block_size)?;
    c.write_u64::<LittleEndian>(block_size as u64)?;
    c.write_u8(tex.flags)?;
    c.write_u8(tex.dim)?;
    c.write_u16::<LittleEndian>(tex.tile_mode)?;
    c.write_u16::<LittleEndian>(tex.swizzle)?;
    c.write_u16::<LittleEndian>(tex.mips_count)?;
    c.write_u32::<LittleEndian>(tex.num_multi_sample)?;
    c.write_u32::<LittleEndian>(tex.format.to_surface_format())?;
    c.write_u32::<LittleEndian>(tex.unk2)?;
    c.write_u32::<LittleEndian>(tex.width)?;
    c.write_u32::<LittleEndian>(tex.height)?;
    c.write_u32::<LittleEndian>(tex.depth)?;
    c.write_u32::<LittleEndian>(tex.array_len)?;
    c.write_i32::<LittleEndian>(tex.size_range)?;
    for v in &tex.unk4 {
        c.write_u32::<LittleEndian>(*v)?;
    }
    c.write_u32::<LittleEndian>(tex.image_size)?;
    c.write_u32::<LittleEndian>(tex.align)?;
    c.write_u32::<LittleEndian>(tex.channel_swizzle)?;
    c.write_u32::<LittleEndian>(tex.ty)?;
    let name_ptr = str_layout.string_offsets[tex.name_string_index as usize] as u64;
    c.write_u64::<LittleEndian>(name_ptr)?;
    c.write_u64::<LittleEndian>(tex.parent_addr)?;
    c.write_u64::<LittleEndian>(texture_indirect_slot as u64)?;

    // Trailing 0x30 bytes of the BRTI header. These are NOT documented in
    // any public spec; jam1garner/bntx writes them from a template:
    //
    //   +0x78  u64  always 0
    //   +0x80  u64  pointer to trailing-block start (= brti_off + 0xA0)
    //   +0x88  u64  pointer to trailing-block midpoint (= brti_off + 0x1A0)
    //   +0x90  u64  always 0
    //   +0x98  u64  always 0
    //
    // The two non-zero pointers are part of the relocation table because
    // they shift if the BRTI moves. We reproduce the exact pattern.
    c.write_u64::<LittleEndian>(0)?;
    c.write_u64::<LittleEndian>((offset + 0xA0) as u64)?;
    c.write_u64::<LittleEndian>((offset + 0xA0 + 0x100) as u64)?;
    c.write_u64::<LittleEndian>(0)?;
    c.write_u64::<LittleEndian>(0)?;
    Ok(())
}

fn write_brtd(out: &mut [u8], offset: usize, b: &BntxFile) -> Result<(), Error> {
    let block_size = (BRTD_HEADER_SIZE + b.brtd.data.len()) as u64;
    {
        let mut c = std::io::Cursor::new(&mut out[offset..offset + BRTD_HEADER_SIZE]);
        c.write_all(b"BRTD")?;
        c.write_u32::<LittleEndian>(0)?;
        c.write_u64::<LittleEndian>(block_size)?;
    }
    let data_start = offset + BRTD_HEADER_SIZE;
    out[data_start..data_start + b.brtd.data.len()].copy_from_slice(&b.brtd.data);
    Ok(())
}

fn write_reloc_table(
    out: &mut [u8],
    offset: usize,
    rlt: &RelocationTable,
) -> Result<(), Error> {
    {
        let mut c = std::io::Cursor::new(&mut out[offset..offset + 16]);
        c.write_all(b"_RLT")?;
        c.write_u32::<LittleEndian>(offset as u32)?;
        c.write_u32::<LittleEndian>(rlt.sections.len() as u32)?;
        c.write_u32::<LittleEndian>(0)?; // padding
    }

    for (i, s) in rlt.sections.iter().enumerate() {
        let pos = offset + 16 + i * 24;
        out[pos..pos + 8].copy_from_slice(&s.pointer.to_le_bytes());
        out[pos + 8..pos + 12].copy_from_slice(&s.position.to_le_bytes());
        out[pos + 12..pos + 16].copy_from_slice(&s.size.to_le_bytes());
        out[pos + 16..pos + 20].copy_from_slice(&s.index.to_le_bytes());
        out[pos + 20..pos + 24].copy_from_slice(&s.count.to_le_bytes());
    }

    let entries_start = offset + 16 + rlt.sections.len() * 24;
    for (i, e) in rlt.entries.iter().enumerate() {
        let pos = entries_start + i * 8;
        out[pos..pos + 4].copy_from_slice(&e.position.to_le_bytes());
        out[pos + 4..pos + 6].copy_from_slice(&e.struct_count.to_le_bytes());
        out[pos + 6] = e.offset_count;
        out[pos + 7] = e.padding_count;
    }
    Ok(())
}

// ============================================================
// Helpers
// ============================================================

fn write_u64_at(out: &mut [u8], offset: usize, value: u64) {
    out[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn align_up_pad(value: usize, align: usize) -> usize {
    let aligned = (value + align - 1) & !(align - 1);
    aligned - value
}
