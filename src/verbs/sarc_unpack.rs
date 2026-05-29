//! `sarc-unpack`: extract a SARC archive to a directory tree. Thin wrapper
//! over [`crate::sarc::unpack_to_dir`].

use anyhow::{Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::sarc;

#[derive(Parser, Debug)]
pub struct Args {
    /// Path to the SARC archive.
    #[arg(short, long)]
    input: PathBuf,

    /// Output directory (created if missing).
    #[arg(short, long)]
    out: PathBuf,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bytes =
        fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let count = sarc::unpack_to_dir(&bytes, &args.out).map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("unpacked {count} files -> {}", args.out.display());
    Ok(ExitCode::SUCCESS)
}
