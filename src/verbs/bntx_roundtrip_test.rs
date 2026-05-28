//! Internal verb: read a BNTX, write it back, and report whether the
//! round-trip is byte-identical. Used to develop and validate the writer.

use anyhow::{Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bntx::{read_bntx, write_bntx};

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(short, long)]
    input: PathBuf,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let original = fs::read(&args.input)
        .with_context(|| format!("reading {}", args.input.display()))?;
    let parsed = read_bntx(&original).map_err(|e| anyhow::anyhow!("{}", e))?;
    let written = write_bntx(&parsed).map_err(|e| anyhow::anyhow!("{}", e))?;

    if written == original {
        println!(
            "OK: BNTX round-trip is byte-identical ({} bytes)",
            original.len()
        );
        return Ok(ExitCode::SUCCESS);
    }

    let diff = first_diff(&original, &written);
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
    println!("  original[0x{lo:x}..0x{hi_o:x}] = {:02x?}", &original[lo..hi_o]);
    println!("  rewritten[0x{lo:x}..0x{hi_w:x}] = {:02x?}", &written[lo..hi_w]);
    Ok(ExitCode::from(1))
}

fn first_diff(a: &[u8], b: &[u8]) -> usize {
    let n = a.len().min(b.len());
    for i in 0..n {
        if a[i] != b[i] {
            return i;
        }
    }
    n
}
