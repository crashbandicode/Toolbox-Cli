//! `aamp-set`: edit a single parameter's value in an AAMP by name path, then
//! re-serialize with the canonical writer. The path is `/<lists…>/<object>/
//! <param>` (segments matched by CRC-32; a `0x…` segment is a raw hash). The
//! edit is type-preserving — the value is parsed into the parameter's existing
//! type. Writes the **uncompressed** AAMP (re-compress with `compress` if the
//! game needs `.zs`).

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::aamp::{read_aamp, set_by_path, write_aamp_canonical};
use crate::compression;

#[derive(Parser, Debug)]
pub struct Args {
    /// Input AAMP (optionally compressed `.zs`).
    #[arg(short, long)]
    input: PathBuf,

    /// Output path for the (uncompressed) edited AAMP.
    #[arg(short, long)]
    out: PathBuf,

    /// Name path: `/<lists…>/<object>/<param>`. Segments are hashed with
    /// CRC-32; a `0x…` segment is taken as a raw hash.
    #[arg(long)]
    path: String,

    /// New value, parsed into the parameter's existing type (numbers; `true`/
    /// `false`; a string; or comma-separated floats for vec/color/quat).
    #[arg(long)]
    value: String,

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
    let bytes = compression::decompress(&raw, &dicts).map_err(|e| anyhow!("{e}"))?;

    let mut doc = read_aamp(&bytes).map_err(|e| anyhow!("{e}"))?;
    let report = set_by_path(&mut doc.root, &args.path, &args.value).map_err(|e| anyhow!("{e}"))?;

    let written = write_aamp_canonical(&doc).map_err(|e| anyhow!("{e}"))?;
    super::write_output(&args.out, &written)?;

    println!("set {}: {} -> {}", report.path, report.old, report.new);
    println!(
        "wrote {} ({} bytes, canonical)",
        args.out.display(),
        written.len()
    );
    println!(
        "note: canonical output re-parses to the edited tree (semantically lossless), not \
         byte-identical to the original; re-compress with `compress` if the game needs .zs"
    );
    Ok(ExitCode::SUCCESS)
}
