//! Regression test for the `_DIC` parallel-array invariant.
//!
//! The BNTX dictionary is a *parallel array* to the texture (BRTI) array:
//! a name lookup resolves to a node **index**, and the game's loader uses
//! that index to fetch `texture[index - 1]`. Therefore `rebuild_dict`
//! must emit `entries[i + 1]` for `textures[i]` — i.e. dict node order
//! must follow texture (BRTI) order, NOT string-pool order.
//!
//! Real Smash `__Combined.bntx` stores the string pool in a different
//! order than the BRTI array, so a dict built in string-pool order
//! scrambles every existing texture's name→texture mapping and corrupts
//! unrelated HUD textures in-game (confirmed on Switch/emulator). This
//! test reproduces that ordering skew synthetically so it fails loudly if
//! `rebuild_dict` ever reverts to iterating `strings` instead of
//! `textures`.

use toolbox_cli::bntx::{
    BntxFile, BntxHeader, BrtdSection, DictSection, NxHeader, RelocationTable, Texture,
    TextureFormat,
};

fn dummy_texture(name_string_index: u32) -> Texture {
    Texture {
        name_string_index,
        flags: 1,
        dim: 2,
        tile_mode: 0,
        swizzle: 0,
        mips_count: 1,
        num_multi_sample: 1,
        format: TextureFormat::Bc7Unorm,
        unk2: 32,
        width: 4,
        height: 4,
        depth: 1,
        array_len: 1,
        size_range: 0,
        unk4: [65543, 0, 0, 0, 0, 0],
        image_size: 16,
        align: 0x200,
        channel_swizzle: 0x05_04_03_02,
        ty: 1,
        parent_addr: 32,
        data_offset_in_brtd: 0,
    }
}

/// Build a BntxFile where the BRTI/texture order deliberately differs from
/// the string-pool order, then rebuild the dict and assert the dict node
/// order tracks texture order (the format invariant), not string order.
#[test]
fn rebuild_dict_follows_texture_order_not_string_order() {
    // String pool order: idx0 empty, idx1 container, then zzz, aaa, mmm.
    let strings = vec![
        String::new(),
        "__Container".to_string(),
        "zzz_texture".to_string(), // string idx 2
        "aaa_texture".to_string(), // string idx 3
        "mmm_texture".to_string(), // string idx 4
    ];

    // Texture (BRTI) order is intentionally NOT the string-pool order:
    // texture[0]=aaa(3), texture[1]=mmm(4), texture[2]=zzz(2).
    let textures = vec![dummy_texture(3), dummy_texture(4), dummy_texture(2)];

    let mut bntx = BntxFile {
        header: BntxHeader {
            version: 0x0004_0000,
            alignment_shift: 12,
            target_address_size: 64,
            flag: 0,
            first_block_offset: 0,
            filename_offset: 0,
        },
        nx_header: NxHeader { dict_size_field: 0 },
        name: "__Container".to_string(),
        strings,
        dict: DictSection {
            count: 0,
            entries: Vec::new(),
        },
        textures,
        brtd: BrtdSection {
            declared_block_size: 0,
            data: Vec::new(),
        },
        relocation_table: RelocationTable {
            sections: Vec::new(),
            entries: Vec::new(),
        },
        relocation_table_dirty: false,
    };

    bntx.rebuild_dict();

    // One root sentinel + one node per texture.
    assert_eq!(
        bntx.dict.entries.len(),
        bntx.textures.len() + 1,
        "dict must have root + one node per texture"
    );

    // THE INVARIANT: entries[i + 1] describes textures[i].
    for (i, tex) in bntx.textures.iter().enumerate() {
        assert_eq!(
            bntx.dict.entries[i + 1].string_index,
            tex.name_string_index,
            "dict node {} must be parallel to texture {} (got string_index {}, expected {}). \
             rebuild_dict is iterating string-pool order instead of texture order.",
            i + 1,
            i,
            bntx.dict.entries[i + 1].string_index,
            tex.name_string_index
        );
    }

    // All child indices must be in range (valid trie).
    let n = bntx.dict.entries.len();
    for (i, e) in bntx.dict.entries.iter().enumerate() {
        assert!(
            (e.left as usize) < n && (e.right as usize) < n,
            "node {i} has out-of-range child (left={}, right={}, n={n})",
            e.left,
            e.right
        );
    }
}
