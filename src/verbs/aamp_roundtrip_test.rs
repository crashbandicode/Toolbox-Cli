//! Internal verb: read an AAMP, write it back, and report whether the
//! round-trip is byte-identical. The write path re-emits the original bytes
//! verbatim, so this primarily proves the parser walks the *entire* document
//! (every list / object / parameter, every type) without error. A compressed
//! input is inflated first and the round-trip is checked on the inflated bytes.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::aamp::{read_aamp, write_aamp};
use crate::compression;

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(short, long)]
    input: PathBuf,

    /// Dictionary pack (for a compressed `.zs` input).
    #[arg(long)]
    dict: Option<PathBuf>,

    /// RomFS root; auto-finds `Pack/ZsDic.pack.zs`.
    #[arg(long)]
    romfs: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let raw =
        std::fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let dicts = super::load_dict_registry(args.dict.as_deref(), args.romfs.as_deref())?;
    let original = compression::decompress(&raw, &dicts).map_err(|e| anyhow!("{e}"))?;

    let doc = read_aamp(&original).map_err(|e| anyhow!("{e}"))?;
    let written = write_aamp(&doc);

    if written == *original {
        let (lists, objects, params) = doc.counts();
        println!(
            "OK: AAMP round-trip is byte-identical ({} bytes, type {:?}, {} list(s) / {} object(s) / {} param(s))",
            original.len(),
            doc.pio_type,
            lists,
            objects,
            params,
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
