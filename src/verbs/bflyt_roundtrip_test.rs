//! Internal verb used to validate the BFLYT round-trip without persisting
//! the output. Reads `--input`, writes back to memory, and reports whether
//! the byte-for-byte comparison succeeded along with the first divergence
//! offset if it didn't.
//!
//! This isn't a public-facing CLI feature; it's wired up so we can dogfood
//! the parser on real fixtures during development.

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
    let written = write_bflyt(&parsed).map_err(|e| anyhow::anyhow!("{}", e))?;

    if written == original {
        println!(
            "OK: round-trip is byte-identical ({} bytes)",
            original.len()
        );
        return Ok(ExitCode::SUCCESS);
    }

    let diff = super::first_diff(&original, &written);
    println!(
        "DIFF: original={} bytes, rewritten={} bytes, first_diff_at=0x{:x}",
        original.len(),
        written.len(),
        diff,
    );
    let context = 16usize;
    let lo = diff.saturating_sub(context);
    let hi_o = (diff + context).min(original.len());
    let hi_w = (diff + context).min(written.len());
    println!("  original[{lo:x}..{hi_o:x}] = {:02x?}", &original[lo..hi_o]);
    println!("  rewritten[{lo:x}..{hi_w:x}] = {:02x?}", &written[lo..hi_w]);
    Ok(ExitCode::from(1))
}
