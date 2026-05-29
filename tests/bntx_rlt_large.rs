//! Regression test for the canonical `_RLT` texture-info-array relocation
//! at large texture counts.
//!
//! `RltEntry::offset_count` is a `u8`. The texture-info pointer array is
//! `n` consecutive pointers, so encoding it as a single struct with
//! `offset_count = n` silently truncates once `n > 255`, leaving most of
//! the array un-relocated and corrupting every texture's load address.
//! The writer must switch to one-pointer-per-struct (`struct_count = n`)
//! past 255 while keeping the Nintendo-identical `<= 255` encoding.

use nx_layout_toolbox::bntx::{
    read_bntx, write_bntx, AppendTextureSpec, BntxFile, BntxHeader, BrtdSection, DictSection,
    NxHeader, RelocationTable,
};

fn minimal_bntx() -> BntxFile {
    BntxFile {
        header: BntxHeader {
            version: 0x0004_0000,
            alignment_shift: 12,
            target_address_size: 64,
            flag: 0,
            first_block_offset: 0,
            filename_offset: 0,
        },
        nx_header: NxHeader {
            dict_size_field: 0x58,
        },
        name: "__Container".to_string(),
        strings: vec![String::new(), "__Container".to_string()],
        dict: DictSection {
            count: 0,
            entries: Vec::new(),
        },
        textures: Vec::new(),
        brtd: BrtdSection {
            declared_block_size: 0,
            data: Vec::new(),
        },
        relocation_table: RelocationTable {
            sections: Vec::new(),
            entries: Vec::new(),
        },
        relocation_table_dirty: false,
    }
}

fn append_n_tiny(bntx: &mut BntxFile, n: usize) {
    for i in 0..n {
        // 4x4 BC7 = one 16-byte block. Content is irrelevant to the RLT.
        let spec = AppendTextureSpec::bc7_2d_default(4, 4, 0, vec![0u8; 16], false);
        bntx.append_texture(format!("tex_{i}"), spec).unwrap();
    }
}

const INFO_PTRS_OFF: u32 = 0x198;

#[test]
fn rlt_info_array_covers_all_pointers_past_255() {
    let mut bntx = minimal_bntx();
    let n = 300usize;
    append_n_tiny(&mut bntx, n);
    assert_eq!(bntx.textures.len(), n);

    let bytes = write_bntx(&bntx).unwrap();
    let parsed = read_bntx(&bytes).expect("written 300-texture BNTX must re-read");
    assert_eq!(
        parsed.textures.len(),
        n,
        "round-trips to the same texture count"
    );

    let entry = parsed
        .relocation_table
        .entries
        .iter()
        .find(|e| e.position == INFO_PTRS_OFF)
        .expect("info-array RLT entry present");
    let covered = entry.struct_count as usize * entry.offset_count as usize;
    assert_eq!(
        covered,
        n,
        "info-array relocation must cover all {n} pointers \
         (struct_count={}, offset_count={}); a u8 offset_count would truncate to {}",
        entry.struct_count,
        entry.offset_count,
        n % 256
    );
    // Past 255, the encoding is one pointer per struct.
    assert_eq!(entry.struct_count as usize, n);
    assert_eq!(entry.offset_count, 1);
}

#[test]
fn rlt_info_array_keeps_nintendo_encoding_at_or_below_255() {
    let mut bntx = minimal_bntx();
    let n = 10usize;
    append_n_tiny(&mut bntx, n);

    let bytes = write_bntx(&bntx).unwrap();
    let parsed = read_bntx(&bytes).unwrap();
    let entry = parsed
        .relocation_table
        .entries
        .iter()
        .find(|e| e.position == INFO_PTRS_OFF)
        .expect("info-array RLT entry present");
    // <= 255: single struct, n consecutive offsets (Nintendo's form).
    assert_eq!(entry.struct_count, 1);
    assert_eq!(entry.offset_count as usize, n);
}
