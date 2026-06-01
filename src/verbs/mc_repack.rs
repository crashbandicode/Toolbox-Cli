//! `mc-repack`: re-compress an (edited) BFRES into a TotK MeshCodec (`MCPK`)
//! container the game can decode. Copies the version/flags + alignment shift
//! from the original `.mc` and emits a magicless zstd frame. NOT byte-identical
//! to Nintendo's encoder — the contract is `mc-extract(mc-repack(x)) == x`
//! (self-verified here before writing).

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::mc::{extract, read_mc, repack};

#[derive(Parser, Debug)]
pub struct Args {
    /// The original MeshCodec container (for version/flags/alignment).
    #[arg(short, long)]
    input: PathBuf,

    /// The edited BFRES to pack.
    #[arg(long)]
    bfres: PathBuf,

    /// Output path for the repacked `.mc`.
    #[arg(short, long)]
    out: PathBuf,

    /// zstd compression level (1..=22).
    #[arg(long, default_value_t = 19)]
    level: i32,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let orig_bytes =
        std::fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let original = read_mc(&orig_bytes).map_err(|e| anyhow!("{e}"))?;
    let bfres =
        std::fs::read(&args.bfres).with_context(|| format!("reading {}", args.bfres.display()))?;
    if bfres.get(0..4) != Some(b"FRES".as_slice()) {
        return Err(anyhow!(
            "{} does not start with the FRES magic — is it a decompressed BFRES?",
            args.bfres.display()
        ));
    }

    let packed = repack(&original, &bfres, args.level).map_err(|e| anyhow!("{e}"))?;

    // Self-verify: the repacked container must decode back to the exact BFRES.
    let check = read_mc(&packed).map_err(|e| anyhow!("re-reading repacked: {e}"))?;
    let round = extract(&check).map_err(|e| anyhow!("verifying repacked: {e}"))?;
    if round != bfres {
        return Err(anyhow!(
            "self-check FAILED: mc-extract(mc-repack(bfres)) != bfres \
             ({} vs {} bytes) — refusing to write a bad file",
            round.len(),
            bfres.len()
        ));
    }

    super::write_output(&args.out, &packed)?;
    println!(
        "mc-repack: {} ({} bytes BFRES) -> {} ({} bytes .mc, level {}); self-check OK (decodes back identically)",
        args.bfres.display(),
        bfres.len(),
        args.out.display(),
        packed.len(),
        args.level,
    );
    Ok(ExitCode::SUCCESS)
}
