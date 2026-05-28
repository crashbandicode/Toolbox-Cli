//! Edge-case tests for the BNTX `_DIC` Patricia-trie builder.
//!
//! The build_trie / Trie::insert logic was developed against the
//! 207-name Smash Ultimate dict. These tests cover smaller and
//! adversarial cases: empty trie, single insert, prefix names,
//! non-ASCII names, and many short names with shared prefixes.

use toolbox_cli::bntx::dict_builder::Trie;
use toolbox_cli::bntx::DictEntry;

/// Walk the entries as the BNTX runtime would, returning the
/// `string_index` of the leaf reached by following `key`'s bits.
fn lookup(entries: &[DictEntry], key: &[u8]) -> u32 {
    if entries.len() <= 1 {
        return 0;
    }
    let mut node = entries[0].left as usize;
    let mut prev;
    loop {
        prev = node;
        let bit = bit_at(key, entries[node].ref_bit);
        node = if bit == 0 {
            entries[node].left as usize
        } else {
            entries[node].right as usize
        };
        let prev_bit = entries[prev].ref_bit as i64;
        let node_bit = entries[node].ref_bit as i64;
        let prev_signed = if prev_bit == 0xFFFF_FFFF { -1 } else { prev_bit };
        let node_signed = if node_bit == 0xFFFF_FFFF { -1 } else { node_bit };
        if node_signed <= prev_signed {
            break;
        }
    }
    entries[node].string_index
}

fn bit_at(bytes: &[u8], idx: u32) -> u8 {
    let total = (bytes.len() * 8) as u32;
    if idx >= total {
        return 0;
    }
    let byte_from_end = (idx / 8) as usize;
    let bit_in_byte = (idx % 8) as u8;
    (bytes[bytes.len() - 1 - byte_from_end] >> bit_in_byte) & 1
}

fn build_and_verify(keys: &[&str]) {
    let mut trie = Trie::new();
    for (i, k) in keys.iter().enumerate() {
        // Simulate the full BNTX strings list: idx 0 is the empty
        // sentinel, idx 1 is the container name, real keys start at 2.
        trie.insert(k.as_bytes(), (i + 2) as u32);
    }
    let entries = trie.to_entries();
    assert_eq!(entries.len(), keys.len() + 1, "expected count + 1 entries");

    for (i, k) in keys.iter().enumerate() {
        let resolved = lookup(&entries, k.as_bytes());
        assert_eq!(
            resolved,
            (i + 2) as u32,
            "key {k:?} resolved to string_index {resolved}, expected {}",
            i + 2
        );
    }
}

#[test]
fn empty_trie_has_only_root() {
    let trie = Trie::new();
    let entries = trie.to_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].ref_bit, 0xFFFF_FFFF);
}

#[test]
fn single_insertion() {
    build_and_verify(&["foo"]);
}

#[test]
fn two_diverging_keys() {
    build_and_verify(&["abc", "xyz"]);
}

#[test]
fn prefix_relationship_short_first() {
    // "tex" is a strict prefix of "tex_foo".
    build_and_verify(&["tex", "tex_foo"]);
}

#[test]
fn prefix_relationship_long_first() {
    build_and_verify(&["tex_foo", "tex"]);
}

#[test]
fn many_shared_prefixes() {
    let names: Vec<String> = (0..32).map(|i| format!("tex_face_{i:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    build_and_verify(&refs);
}

#[test]
fn ascii_punctuation_and_caret() {
    // Real BNTX names use `^t`, `^u`, `^s`, `^o` suffixes; make sure
    // those route correctly.
    build_and_verify(&[
        "com_eff_aura_03^t",
        "com_eff_flare_00^t",
        "com_eff_flash_00^t",
        "info_melee_chara_bg_fp^u",
        "info_melee_charge_max_00^s",
        "info_melee_lct_magic_gauge^o",
    ]);
}

#[test]
fn non_ascii_utf8_byte_sequences() {
    // Although BNTX names are conventionally ASCII, the format itself
    // stores raw bytes, so non-ASCII UTF-8 should still route.
    build_and_verify(&["tex_emoji_😀", "tex_émoji", "tex_日本"]);
}

#[test]
fn names_differing_only_in_last_bit() {
    // "ABCD" vs "ABCE" differ only in the last byte's last bit.
    build_and_verify(&["ABCD", "ABCE"]);
}

#[test]
fn power_of_two_count() {
    let names: Vec<String> = (0..64).map(|i| format!("k{i:03}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    build_and_verify(&refs);
}
