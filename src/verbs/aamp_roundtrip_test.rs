//! Internal verb: read an AAMP, write it back, and report whether the
//! round-trip is byte-identical. The write path re-emits the original bytes
//! verbatim, so this primarily proves the parser walks the *entire* document
//! (every list / object / parameter, every type) without error. A compressed
//! input is inflated first and the round-trip is checked on the inflated bytes.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::aamp::{read_aamp, write_aamp, write_aamp_canonical};
use crate::compression;

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(short, long)]
    input: PathBuf,

    /// Test the from-scratch canonical writer's *semantic* round-trip
    /// (read -> write_canonical -> read decodes to the same tree) instead of
    /// the verbatim byte-identical round-trip.
    #[arg(long)]
    canonical: bool,

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

    if args.canonical {
        let rebuilt = write_aamp_canonical(&doc).map_err(|e| anyhow!("{e}"))?;
        let doc2 = read_aamp(&rebuilt).map_err(|e| anyhow!("{e}"))?;
        if doc.root == doc2.root && doc.pio_type == doc2.pio_type {
            println!(
                "OK: AAMP canonical semantic round-trip ({} -> {} bytes{})",
                original.len(),
                rebuilt.len(),
                if rebuilt == *original {
                    ", byte-identical"
                } else {
                    ""
                },
            );
            return Ok(ExitCode::SUCCESS);
        }
        println!("MISMATCH: canonical round-trip decodes to a different tree");
        return Ok(ExitCode::from(1));
    }

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
