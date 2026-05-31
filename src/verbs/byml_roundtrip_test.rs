//! Internal verb: read a BYML, write it back, and report whether the
//! round-trip is byte-identical. The write path re-emits the original bytes
//! verbatim, so this primarily proves the parser walks the *entire* document
//! (every node type, every offset) without error. A compressed `.byml.zs` is
//! inflated first and the round-trip is checked on the decompressed bytes.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::byml::{read_byml, write_byml, Byml};
use crate::compression;

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(short, long)]
    input: PathBuf,

    /// TotK `ZsDic.pack.zs` (or a directory of `*.zsdic`) for a
    /// dictionary-compressed `.byml.zs`.
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

    let doc = read_byml(&original).map_err(|e| anyhow!("{e}"))?;
    let written = write_byml(&doc).map_err(|e| anyhow!("{e}"))?;

    if written == *original {
        println!(
            "OK: BYML round-trip is byte-identical ({} bytes, v{}, {}-endian, {} node(s))",
            original.len(),
            doc.version,
            if doc.big_endian { "big" } else { "little" },
            count_nodes(&doc.root),
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

/// Total number of value nodes in the tree (including the root).
fn count_nodes(v: &Byml) -> usize {
    match v {
        Byml::Array(items) => 1 + items.iter().map(count_nodes).sum::<usize>(),
        Byml::Hash(entries) => 1 + entries.iter().map(|(_, c)| count_nodes(c)).sum::<usize>(),
        _ => 1,
    }
}
