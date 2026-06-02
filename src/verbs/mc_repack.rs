//! `mc-repack`: re-compress an (edited) BFRES into a TotK MeshCodec (`MCPK`)
//! container, **preserving the original's mesh tail** (the custom-coded
//! vertex/index buffers we don't decode). The output is `[new BFRES frame] +
//! [original mesh bytes]`, so geometry is kept from the original and only the
//! BFRES structure changes. NOT byte-identical to Nintendo's encoder; the
//! contract is `mc-extract(mc-repack(x)) == x` (self-verified before writing).
//!
//! A size-changing edit would shift the mesh-buffer references and is rejected
//! unless `--allow-resize`. Geometry editing is not supported (the mesh tail is
//! opaque); in-game acceptance is untested.

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

    /// Allow an edited BFRES of a different size than the original (best-effort;
    /// likely breaks the mesh-buffer references — geometry edits are unsupported).
    #[arg(long)]
    allow_resize: bool,
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

    let packed =
        repack(&original, &bfres, args.level, args.allow_resize).map_err(|e| anyhow!("{e}"))?;

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
        "mc-repack: {} ({} bytes BFRES) -> {} ({} bytes .mc, level {}); BFRES re-encoded, original mesh tail preserved; self-check OK",
        args.bfres.display(),
        bfres.len(),
        args.out.display(),
        packed.len(),
        args.level,
    );
    eprintln!("note: only the BFRES structure was changed (geometry/mesh kept from the original); in-game acceptance is untested.");
    Ok(ExitCode::SUCCESS)
}
