//! `bflyt-repair`: tidy a BFLYT — dedupe duplicate pane names, clamp dangling
//! material→texture references into range, prune unused textures, and
//! (optionally) prune unused materials. `--dry-run` reports what would change
//! without writing.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bflyt::{read_bflyt, write_bflyt};

#[derive(Parser, Debug)]
pub struct Args {
    /// BFLYT file to repair.
    #[arg(short, long)]
    input: PathBuf,

    /// Output BFLYT (defaults to overwriting the input).
    #[arg(short, long)]
    out: Option<PathBuf>,

    /// Also prune unused materials (skipped when the layout has prt1 property
    /// data that may reference materials we can't see).
    #[arg(long)]
    prune_materials: bool,

    /// Report what would change without writing any file.
    #[arg(long)]
    dry_run: bool,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bytes =
        std::fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let mut bflyt = read_bflyt(&bytes).map_err(|e| anyhow!("{e}"))?;

    let report = bflyt.repair(args.prune_materials);

    if report.is_empty() && !report.materials_prune_skipped {
        println!("ok: nothing to repair in {}", args.input.display());
        return Ok(ExitCode::SUCCESS);
    }

    for (old, new) in &report.renamed_panes {
        println!("  rename pane '{old}' -> '{new}'");
    }
    if report.fixed_texture_refs > 0 {
        println!(
            "  clamped {} dangling texture ref(s) into range",
            report.fixed_texture_refs
        );
    }
    if !report.removed_materials.is_empty() {
        println!(
            "  removed {} material(s): {}",
            report.removed_materials.len(),
            report.removed_materials.join(", ")
        );
    }
    if !report.removed_textures.is_empty() {
        println!(
            "  removed {} texture(s): {}",
            report.removed_textures.len(),
            report.removed_textures.join(", ")
        );
    }
    if report.materials_prune_skipped {
        println!("  note: material pruning skipped (layout has prt1 property data)");
    }

    if args.dry_run {
        println!("dry-run: no file written");
        return Ok(ExitCode::SUCCESS);
    }

    let written = write_bflyt(&bflyt).map_err(|e| anyhow!("{e}"))?;
    let target = args.out.as_deref().unwrap_or(&args.input);
    super::write_output(target, &written)?;
    println!(
        "ok: repaired -> {} ({} bytes)",
        target.display(),
        written.len()
    );
    Ok(ExitCode::SUCCESS)
}
