//! Internal verb: read an MSBT, write it back, and report whether the
//! round-trip is byte-identical. The write path re-emits the original bytes
//! verbatim, so this primarily proves the parser walks the *entire* document
//! (every section, label, and message) without error. A compressed `.msbt.zs`
//! is inflated first and the round-trip is checked on the decompressed bytes.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::compression;
use crate::msbt::{read_msbt, write_msbt};

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(short, long)]
    input: PathBuf,

    /// TotK `ZsDic.pack.zs` (or a directory of `*.zsdic`) for a
    /// dictionary-compressed `.msbt.zs`.
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

    let doc = read_msbt(&original).map_err(|e| anyhow!("{e}"))?;
    let written = write_msbt(&doc).map_err(|e| anyhow!("{e}"))?;

    if written == *original {
        println!(
            "OK: MSBT round-trip is byte-identical ({} bytes, v{}, {}-endian, {}, {} label(s), {} message(s))",
            original.len(),
            doc.version,
            if doc.big_endian { "big" } else { "little" },
            doc.encoding.label(),
            doc.labels().map(|l| l.len()).unwrap_or(0),
            doc.messages().map(|m| m.len()).unwrap_or(0),
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
