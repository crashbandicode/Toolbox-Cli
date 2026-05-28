//! Stress tests for the BNTX `_DIC` Patricia-trie builder at scale.
//!
//! `tests/bntx_dict_edge.rs` covers small / adversarial cases. The
//! tests here push the trie to N ≥ 10,000 names — one to two orders
//! of magnitude beyond any real BNTX we've seen — under three name
//! distributions:
//!
//! - sequential hex (low prefix overlap, full lexicographic spread)
//! - heavily shared prefixes (realistic BNTX naming, e.g. all names
//!   start with `info_melee_face_`)
//! - long shared prefix + short unique suffix (stresses the trie's
//!   high-bit-index comparison path; each distinct bit lives near
//!   the right edge of the byte string)
//!
//! Each test asserts that every inserted name resolves to its own
//! `string_index` and prints insertion + lookup timing for visibility
//! when running with `cargo test -- --nocapture`. We don't assert
//! timing thresholds — those vary by machine — but we log the numbers
//! so a 10× regression is obvious in CI logs.

use std::time::Instant;

use toolbox_cli::bntx::dict_builder::Trie;
use toolbox_cli::bntx::DictEntry;

/// Walks the entries the way the BNTX runtime would, returning the
/// `string_index` of the leaf reached by following `key`'s bits.
/// Verbatim copy of the helper in `tests/bntx_dict_edge.rs`; kept
/// inline because Cargo's integration tests don't share a module by
/// default.
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

/// Build a trie from `names`, time the insertion + a full sweep of
/// lookups, and assert each name resolves to the `string_index` it was
/// inserted with. Returns `(insert_elapsed_ms, lookup_elapsed_ms)` so
/// callers can print summary numbers.
fn stress_round_trip(label: &str, names: &[String]) -> (u128, u128) {
    let mut trie = Trie::new();
    let insert_start = Instant::now();
    for (i, name) in names.iter().enumerate() {
        // String pool slots 0 / 1 are the empty sentinel and container
        // name, so real names start at index 2 — same convention as
        // `BntxFile::rebuild_dict`.
        trie.insert(name.as_bytes(), (i + 2) as u32);
    }
    let insert_elapsed = insert_start.elapsed();

    let entries = trie.to_entries();
    assert_eq!(
        entries.len(),
        names.len() + 1,
        "{label}: expected {} entries (names + root sentinel), got {}",
        names.len() + 1,
        entries.len(),
    );

    let lookup_start = Instant::now();
    for (i, name) in names.iter().enumerate() {
        let resolved = lookup(&entries, name.as_bytes());
        assert_eq!(
            resolved,
            (i + 2) as u32,
            "{label}: name {name:?} (index {i}) resolved to {resolved}, expected {}",
            i + 2,
        );
    }
    let lookup_elapsed = lookup_start.elapsed();

    let n = names.len();
    println!(
        "{label}: N={n}  insert={:>5} ms  lookup_total={:>5} ms  ({} ns/lookup avg)",
        insert_elapsed.as_millis(),
        lookup_elapsed.as_millis(),
        lookup_elapsed.as_nanos() / n.max(1) as u128,
    );
    (insert_elapsed.as_millis(), lookup_elapsed.as_millis())
}

/// Flat hex-suffix names — `tex_00000000` through `tex_00002710`.
/// The 8-character hex suffix gives full lexicographic spread, so the
/// trie's branching is well-distributed and bit indices are spread
/// across the full key length.
#[test]
fn ten_thousand_sequential_hex_names() {
    let names: Vec<String> = (0..10_000).map(|i| format!("tex_{i:08x}")).collect();
    stress_round_trip("sequential_hex", &names);
}

/// Heavily-shared prefix names. Realistic for SGPO-style modders who
/// might add hundreds of variants of the same UI element. Forces the
/// trie to differentiate exclusively on the suffix bits.
#[test]
fn ten_thousand_heavy_shared_prefix_names() {
    let names: Vec<String> = (0..10_000)
        .map(|i| format!("info_melee_face_{:02}_{:04}", i % 100, i))
        .collect();
    stress_round_trip("heavy_shared_prefix", &names);
}

/// Long-prefix + short-suffix. Each name shares a 27-byte prefix; the
/// only distinguishing bytes are an 8-char hex tail. This stresses the
/// bit-comparison path at high `bit_inx` values (near the LSB of the
/// last byte, i.e. low bit indices in Nintendo's bit-from-end
/// convention).
#[test]
fn ten_thousand_long_shared_prefix_names() {
    let names: Vec<String> = (0..10_000)
        .map(|i| format!("very_long_shared_prefix_____{i:08x}"))
        .collect();
    stress_round_trip("long_shared_prefix", &names);
}

/// Confirm the trie scales beyond the 10,000-name lower bound by
/// pushing to 25,000 — gives us headroom against future BNTX archives
/// that might exceed 10k textures (Smash's `__Combined.bntx` is ~200,
/// HDR mods bring it to ~2k; 10k is a buffer of 5x and 25k is 12.5x).
#[test]
fn twenty_five_thousand_names_all_resolve() {
    let names: Vec<String> = (0..25_000).map(|i| format!("tex_{i:08x}")).collect();
    let (ins_ms, lookup_ms) = stress_round_trip("scale_25k", &names);
    // Soft budgets, only as a guard against a catastrophic regression
    // (e.g., accidentally O(n^2) lookups). On a modern dev machine
    // 25k insertions complete in <100ms and 25k lookups in <50ms;
    // the budgets here are 100x that headroom.
    assert!(
        ins_ms < 30_000,
        "25k insertion took {ins_ms} ms — suspect O(n^2) or worse complexity regression",
    );
    assert!(
        lookup_ms < 30_000,
        "25k lookup-sweep took {lookup_ms} ms — suspect lookup-time regression",
    );
}
