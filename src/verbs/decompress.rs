//! `decompress`: inflate a zstd (`.zs`/`.pack.zs`/`.blarc.zs`) or Yaz0/Yaz1
//! (`.szs`) file. The dictionary (for TotK `.zs`) is selected automatically
//! by the zstd frame's id from the registry loaded via `--dict`/`--romfs`.
//! Thin wrapper over [`crate::compression::decompress`].

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::compression::{self, Codec};

#[derive(Parser, Debug)]
pub struct Args {
    /// Compressed input file (zstd `.zs` or Yaz0 `.szs`).
    #[arg(short, long)]
    input: PathBuf,

    /// Output path for the decompressed bytes.
    #[arg(short, long)]
    out: PathBuf,

    /// TotK `ZsDic.pack.zs` (or a directory of `*.zsdic`) for
    /// dictionary-compressed `.zs`. Not needed for plain zstd / Yaz0.
    #[arg(long)]
    dict: Option<PathBuf>,

    /// TotK RomFS root; auto-finds `Pack/ZsDic.pack.zs`.
    #[arg(long)]
    romfs: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bytes =
        std::fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;

    let codec = compression::detect(&bytes);
    if codec == Codec::None {
        return Err(anyhow!(
            "{} has no zstd/Yaz0 magic — already decompressed?",
            args.input.display()
        ));
    }

    let dicts = super::load_dict_registry(args.dict.as_deref(), args.romfs.as_deref())?;
    let out = compression::decompress(&bytes, &dicts).map_err(|e| anyhow!("{e}"))?;
    super::write_output(&args.out, &out)?;

    println!(
        "decompressed {} [{}]: {} -> {} bytes -> {}",
        args.input.display(),
        codec.label(),
        bytes.len(),
        out.len(),
        args.out.display()
    );
    Ok(ExitCode::SUCCESS)
}
