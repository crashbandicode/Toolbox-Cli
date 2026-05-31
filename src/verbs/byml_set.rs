//! `byml-set`: edit a single scalar leaf in a BYML/BYAML document by path, then
//! re-serialize with the canonical writer. Addresses the node with a
//! `byml-diff`-style path (`/SystemData/Hp`, `/RecipeList/3/Name`). The target
//! type is preserved by default (editing an `f32` keeps it an `f32`); pass
//! `--type` to change the kind or to set a node currently `null`.
//!
//! Transparently inflates a zstd-compressed input (`--dict`/`--romfs`) and
//! writes the **uncompressed** BYML — re-compress with the `compress` verb if
//! the game needs `.zs`. The canonical writer is *semantically lossless* (the
//! output re-parses to the mutated tree) but not byte-identical to the original
//! by contract, since BYML byte layout is writer-specific.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::byml::{read_byml, set_by_path, write_byml_canonical, ScalarType};
use crate::compression;

#[derive(Parser, Debug)]
pub struct Args {
    /// Input BYML (`.byml`, `.bgyml`, or compressed `.byml.zs`).
    #[arg(short, long)]
    input: PathBuf,

    /// Output path for the (uncompressed) edited BYML.
    #[arg(short, long)]
    out: PathBuf,

    /// Path to the leaf to edit (`byml-diff` style: `/SystemData/Hp`,
    /// `/RecipeList/3/Name`; a leading slash is optional).
    #[arg(long)]
    path: String,

    /// New value, parsed into the target type. Ignored for `--type null`.
    #[arg(long)]
    value: String,

    /// Force the target scalar type (`bool`, `s32`/`int`, `u32`, `f32`,
    /// `s64`, `u64`, `f64`, `string`, `null`). Default: preserve the existing
    /// leaf's type.
    #[arg(long, value_name = "TYPE")]
    r#type: Option<String>,

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
    let bytes = compression::decompress(&raw, &dicts).map_err(|e| anyhow!("{e}"))?;

    let mut doc = read_byml(&bytes).map_err(|e| anyhow!("{e}"))?;
    let ty = match &args.r#type {
        Some(t) => Some(ScalarType::parse(t).map_err(|e| anyhow!("{e}"))?),
        None => None,
    };

    let report =
        set_by_path(&mut doc.root, &args.path, &args.value, ty).map_err(|e| anyhow!("{e}"))?;

    let written =
        write_byml_canonical(doc.version, doc.big_endian, &doc.root).map_err(|e| anyhow!("{e}"))?;
    super::write_output(&args.out, &written)?;

    println!("set {}: {} -> {}", report.path, report.old, report.new);
    println!(
        "wrote {} ({} bytes, BYML v{} {}-endian, canonical)",
        args.out.display(),
        written.len(),
        doc.version,
        if doc.big_endian { "big" } else { "little" },
    );
    println!(
        "note: canonical output re-parses to the edited tree (semantically lossless), \
         not byte-identical to the original; re-compress with `compress` if the game needs .zs"
    );
    Ok(ExitCode::SUCCESS)
}
