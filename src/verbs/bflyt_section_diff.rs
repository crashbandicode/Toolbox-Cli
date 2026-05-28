//! Debugging verb: walk an original BFLYT and the rewritten output in
//! lockstep, reporting where section boundaries diverge. Helps localize
//! writer bugs.

use anyhow::{Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bflyt::{read_bflyt, write_bflyt};

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(short, long)]
    input: PathBuf,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let original = fs::read(&args.input)
        .with_context(|| format!("reading {}", args.input.display()))?;
    let parsed = read_bflyt(&original).map_err(|e| anyhow::anyhow!("{}", e))?;
    let rewritten = write_bflyt(&parsed).map_err(|e| anyhow::anyhow!("{}", e))?;

    println!(
        "original = {} bytes; rewritten = {} bytes; delta = {} bytes",
        original.len(),
        rewritten.len(),
        original.len() as i64 - rewritten.len() as i64
    );

    walk_sections("ORIGINAL ", &original);
    println!();
    walk_sections("REWRITTEN", &rewritten);
    Ok(ExitCode::SUCCESS)
}

fn walk_sections(label: &str, data: &[u8]) {
    println!(
        "{label}: {} bytes total, {} sections",
        data.len(),
        u16::from_le_bytes([data[0x10], data[0x11]])
    );
    let mut offset = 0x14usize;
    let n = u16::from_le_bytes([data[0x10], data[0x11]]) as usize;
    let mut total_section_bytes = 0u32;
    for i in 0..n {
        if offset + 8 > data.len() {
            println!("  [{i:>3}] truncated at 0x{offset:x}");
            return;
        }
        let magic = std::str::from_utf8(&data[offset..offset + 4]).unwrap_or("?").to_string();
        let size = u32::from_le_bytes([
            data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7],
        ]);
        // Always emit every section so external diff tools can walk
        // arbitrarily-large files. Trimming was easy for debugging the
        // 25-section info_melee but bites us on 345-section HDR layouts.
        println!("  [{i:>3}] @0x{offset:08x}  {magic:<5}  size={size}");
        total_section_bytes += size;
        offset += size as usize;
    }
    println!("  total section bytes (incl. headers) = {total_section_bytes}");
    println!("  trailing after last section: 0x{offset:x} (file size 0x{:x})", data.len());
}
