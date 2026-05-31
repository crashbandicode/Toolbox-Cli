//! `bflyt-prune`: remove unreferenced materials and/or textures from a BFLYT,
//! remapping the surviving indices. With no flags it prunes both. Material
//! pruning is skipped when the layout has `prt1` panes with opaque property
//! data (which may reference materials we can't see) unless `--force` is given.

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::verbs::bflyt_helpers::rewrite_bflyt;

#[derive(Parser, Debug)]
pub struct Args {
    /// BFLYT file to modify.
    #[arg(short, long)]
    input: PathBuf,

    /// Output BFLYT (defaults to overwriting the input).
    #[arg(short, long)]
    out: Option<PathBuf>,

    /// Prune unused materials. (If neither --materials nor --textures is
    /// given, both are pruned.)
    #[arg(long)]
    materials: bool,

    /// Prune unused textures.
    #[arg(long)]
    textures: bool,

    /// Prune materials even when the layout has prt1 property data.
    #[arg(long)]
    force: bool,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let both = !args.materials && !args.textures;
    let do_mats = both || args.materials;
    let do_texs = both || args.textures;
    let force = args.force;

    let mut removed_mats: Vec<String> = Vec::new();
    let mut removed_texs: Vec<String> = Vec::new();
    let mut skipped = false;

    let n = rewrite_bflyt(&args.input, args.out.as_deref(), |b| {
        if do_mats {
            if b.has_parts_data() && !force {
                skipped = true;
            } else {
                removed_mats = b.prune_unused_materials();
            }
        }
        if do_texs {
            removed_texs = b.prune_unused_textures();
        }
        Ok(())
    })?;

    if skipped {
        println!(
            "note: skipped material pruning (layout has prt1 property data; pass --force to \
             override)"
        );
    }
    println!(
        "ok: pruned {} material(s), {} texture(s) ({n} bytes)",
        removed_mats.len(),
        removed_texs.len()
    );
    if !removed_mats.is_empty() {
        println!("  materials: {}", removed_mats.join(", "));
    }
    if !removed_texs.is_empty() {
        println!("  textures: {}", removed_texs.join(", "));
    }
    Ok(ExitCode::SUCCESS)
}
