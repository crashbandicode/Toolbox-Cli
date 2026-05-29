//! `sarc-pack`: pack a directory tree into a SARC archive. Thin wrapper
//! over [`crate::sarc::pack_directory_with_endian`].

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::sarc;

#[derive(Parser, Debug)]
pub struct Args {
    /// Source directory.
    #[arg(short, long)]
    input: PathBuf,

    /// Output SARC path.
    #[arg(short, long)]
    out: PathBuf,

    /// Use big-endian SARC (Wii U / 3DS). Default is little-endian (Switch).
    #[arg(long)]
    big_endian: bool,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bytes = sarc::pack_directory_with_endian(&args.input, args.big_endian)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if let Some(parent) = args.out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(&args.out, &bytes).with_context(|| format!("writing {}", args.out.display()))?;
    println!("packed -> {} ({} bytes)", args.out.display(), bytes.len());
    Ok(ExitCode::SUCCESS)
}
