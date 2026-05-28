//! Internal verb: validate the dict-builder by rebuilding the trie for
//! an existing BNTX's strings and confirming every name still routes to
//! the correct entry.

use anyhow::{Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bntx::{dict_builder::Trie, read_bntx};

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(short, long)]
    input: PathBuf,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bytes = fs::read(&args.input)
        .with_context(|| format!("reading {}", args.input.display()))?;
    let bntx = read_bntx(&bytes).map_err(|e| anyhow::anyhow!("{}", e))?;

    println!(
        "rebuilding dict for {} strings (skipping the empty sentinel at index 0)...",
        bntx.strings.len()
    );

    // Dict only contains TEXTURE names. Strings list also has the empty
    // sentinel (idx 0) and the BNTX container name (idx 1 = "__Combined"
    // for our fixture); both are skipped.
    let mut trie = Trie::new();
    for (idx, s) in bntx.strings.iter().enumerate().skip(2) {
        trie.insert(s.as_bytes(), idx as u32);
    }

    let entries = trie.to_entries();
    println!(
        "built {} dict entries (expected {} = orig dict count + 1)",
        entries.len(),
        bntx.dict.entries.len()
    );

    if entries.len() != bntx.dict.entries.len() {
        println!(
            "FAIL: entry count mismatch — built {} vs original {}",
            entries.len(),
            bntx.dict.entries.len()
        );
        return Ok(ExitCode::from(1));
    }

    // Verify each TEXTURE-NAME string can be looked up via the rebuilt
    // trie. The empty sentinel (idx 0) and the container name (idx 1)
    // aren't in the dict, so we skip them.
    let mut ok = 0usize;
    let mut fail = 0usize;
    let mut fails = Vec::new();
    for (idx, s) in bntx.strings.iter().enumerate().skip(2) {
        let resolved_idx = lookup_via_trie(&entries, &bntx.strings, s.as_bytes());
        if resolved_idx == idx as u32 {
            ok += 1;
        } else {
            fail += 1;
            if fails.len() < 5 {
                let actual = bntx
                    .strings
                    .get(resolved_idx as usize)
                    .cloned()
                    .unwrap_or_else(|| format!("<idx {resolved_idx}>"));
                fails.push((s.clone(), actual));
            }
        }
    }
    println!("texture-name lookups: {ok} ok, {fail} fail");
    for (wanted, got) in &fails {
        println!("  searched '{wanted}', resolved to '{got}'");
    }
    Ok(if fail == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// Walk a flat dict entry list as a BNTX runtime would, returning the
/// `string_index` of the leaf the trie navigates to.
fn lookup_via_trie(
    entries: &[crate::bntx::DictEntry],
    strings: &[String],
    key: &[u8],
) -> u32 {
    if entries.len() <= 1 {
        return 0;
    }
    let mut node = entries[0].left as usize;
    let mut prev = node;
    loop {
        prev = node;
        let bit = bit_at(key, entries[node].ref_bit);
        node = if bit == 0 {
            entries[node].left as usize
        } else {
            entries[node].right as usize
        };
        // Back-edge: terminate at the candidate leaf.
        let prev_bit = entries[prev].ref_bit as i64;
        let node_bit = entries[node].ref_bit as i64;
        // The root sentinel has ref_bit = 0xFFFFFFFF which we want to
        // treat as a bit-index of -1; cast carefully.
        let prev_bit_signed = if prev_bit == 0xFFFF_FFFF { -1 } else { prev_bit };
        let node_bit_signed = if node_bit == 0xFFFF_FFFF { -1 } else { node_bit };
        if node_bit_signed <= prev_bit_signed {
            break;
        }
    }
    let _ = strings;
    entries[node].string_index
}

fn bit_at(bytes: &[u8], idx: u32) -> u8 {
    let total_bits = (bytes.len() * 8) as u32;
    if idx >= total_bits {
        return 0;
    }
    let byte_from_end = (idx / 8) as usize;
    let bit_in_byte = (idx % 8) as u8;
    let byte_idx = bytes.len() - 1 - byte_from_end;
    (bytes[byte_idx] >> bit_in_byte) & 1
}
