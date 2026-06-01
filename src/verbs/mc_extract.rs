//! `mc-extract`: decompress a TotK MeshCodec (`MCPK`) container to its inner
//! BFRES. The inner stream is a magicless zstd frame (no dictionary needed for
//! model `.bfres.mc`); the output is the real, unpadded BFRES (the frame's
//! content size). Verified byte-identical against the decompressed-`.bfres`
//! reference corpus.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::mc::{extract, read_mc};

#[derive(Parser, Debug)]
pub struct Args {
    /// MeshCodec container (`*.bfres.mc`).
    #[arg(short, long)]
    input: PathBuf,

    /// Output path for the decompressed BFRES.
    #[arg(short, long)]
    out: PathBuf,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bytes =
        std::fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let mc = read_mc(&bytes).map_err(|e| anyhow!("{e}"))?;
    let bfres = extract(&mc).map_err(|e| anyhow!("{e}"))?;

    if bfres.get(0..4) != Some(b"FRES".as_slice()) {
        eprintln!(
            "warning: decompressed output does not start with the FRES magic \
             (got {:02x?}); writing anyway",
            &bfres[..bfres.len().min(4)]
        );
    }
    super::write_output(&args.out, &bfres)?;
    println!(
        "mc-extract: {} ({} bytes) -> {} ({} bytes BFRES, decompressed-capacity {})",
        args.input.display(),
        bytes.len(),
        args.out.display(),
        bfres.len(),
        mc.decompressed_size(),
    );
    Ok(ExitCode::SUCCESS)
}
