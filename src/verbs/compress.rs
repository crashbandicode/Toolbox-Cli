//! `compress`: compress a file as zstd (optionally with a TotK dictionary)
//! or Yaz0. The round-trip is lossless (`decompress(compress(x)) == x`) but
//! **not** byte-identical to the game's original encoder output — for an
//! unchanged file, keep its original `.zs`/`.szs` bytes instead of
//! re-compressing.

use anyhow::{anyhow, Context, Result};
use clap::{Parser, ValueEnum};
use std::path::PathBuf;
use std::process::ExitCode;

use crate::compression;

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum Format {
    /// zstd frame (TotK; supports dictionaries).
    Zstd,
    /// Yaz0 container (`.szs`; BOTW and older titles).
    Yaz0,
}

#[derive(Parser, Debug)]
pub struct Args {
    /// Uncompressed input file.
    #[arg(short, long)]
    input: PathBuf,

    /// Output path for the compressed bytes.
    #[arg(short, long)]
    out: PathBuf,

    /// Compression format.
    #[arg(long, value_enum, default_value_t = Format::Zstd)]
    format: Format,

    /// zstd compression level (1..=22). Ignored for Yaz0.
    #[arg(long, default_value_t = 19)]
    level: i32,

    /// zstd dictionary id to compress with (e.g. 1 = `zs`, 3 = `pack`).
    /// Requires `--dict`/`--romfs`. Omit for a dictionary-less frame.
    #[arg(long)]
    dict_id: Option<u32>,

    /// TotK `ZsDic.pack.zs` (or a directory of `*.zsdic`) supplying the
    /// dictionary referenced by `--dict-id`.
    #[arg(long)]
    dict: Option<PathBuf>,

    /// TotK RomFS root; auto-finds `Pack/ZsDic.pack.zs`.
    #[arg(long)]
    romfs: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bytes =
        std::fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;

    let out = match args.format {
        Format::Yaz0 => compression::compress_yaz0(&bytes),
        Format::Zstd => {
            let dicts = super::load_dict_registry(args.dict.as_deref(), args.romfs.as_deref())?;
            compression::compress_zstd(&bytes, &dicts, args.dict_id, args.level)
                .map_err(|e| anyhow!("{e}"))?
        }
    };
    super::write_output(&args.out, &out)?;

    let label = match args.format {
        Format::Zstd => "zstd",
        Format::Yaz0 => "Yaz0",
    };
    println!(
        "compressed {} [{}]: {} -> {} bytes -> {}",
        args.input.display(),
        label,
        bytes.len(),
        out.len(),
        args.out.display()
    );
    Ok(ExitCode::SUCCESS)
}
