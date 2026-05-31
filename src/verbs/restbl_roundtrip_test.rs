//! Internal verb: read a RESTBL, write it back, and report whether the
//! round-trip is byte-identical (inflating a compressed `.rsizetable.zs`
//! first and checking the round-trip on the decompressed bytes).

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::compression;
use crate::restbl::{read_restbl, write_restbl};

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(short, long)]
    input: PathBuf,

    /// TotK `ZsDic.pack.zs` (or a directory of `*.zsdic`) for compressed input.
    #[arg(long)]
    dict: Option<PathBuf>,

    /// TotK RomFS root; auto-finds `Pack/ZsDic.pack.zs`.
    #[arg(long)]
    romfs: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let raw =
        std::fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let dicts = super::load_dict_registry(args.dict.as_deref(), args.romfs.as_deref())?;
    let original = compression::decompress(&raw, &dicts).map_err(|e| anyhow!("{e}"))?;

    let table = read_restbl(&original).map_err(|e| anyhow!("{e}"))?;
    let written = write_restbl(&table).map_err(|e| anyhow!("{e}"))?;

    if written == *original {
        println!(
            "OK: RESTBL round-trip is byte-identical ({} bytes, v{}, {} crc + {} name entries)",
            original.len(),
            table.version,
            table.crc_entries.len(),
            table.name_entries.len(),
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
    Ok(ExitCode::from(1))
}
