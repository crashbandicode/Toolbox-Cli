//! `layout-apply-arc`: end-to-end skin application on a packed
//! `layout.arc`. Unpacks the archive in memory, applies an SGPO manifest
//! to the contained BFLYT+BNTX, validates the result, and re-packs every
//! entry into a new `layout.arc`. Thin wrapper over
//! [`crate::layout::apply_manifest_to_arc`].

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::layout::{apply_manifest_to_arc, ApplyOptions};
use crate::manifest::SkinManifest;
use crate::texpipe::Bc7Quality;

#[derive(Parser, Debug)]
pub struct Args {
    /// Input `layout.arc`.
    #[arg(short, long)]
    input: PathBuf,

    /// Output `layout.arc`.
    #[arg(short, long)]
    out: PathBuf,

    /// Path to the SGPO skin_manifest.json.
    #[arg(long)]
    manifest: PathBuf,

    /// Directory containing the manifest's image_filename files.
    #[arg(long)]
    skin_dir: PathBuf,

    /// BFLYT path within the archive.
    #[arg(long, default_value = "blyt/info_melee.bflyt")]
    bflyt: String,

    /// BNTX path within the archive.
    #[arg(long, default_value = "timg/__Combined.bntx")]
    bntx: String,

    /// Pane to clone for each new pane.
    #[arg(long, default_value = "set_rep_stock_01")]
    pane_template: String,

    /// Material to clone for each new material.
    #[arg(long, default_value = "set_rep_stock_01")]
    material_template: String,

    /// BC7 encoder quality.
    #[arg(long, default_value = "fast")]
    quality: String,

    /// Encode textures as SRGB.
    #[arg(long)]
    srgb: bool,

    /// Override the texture data alignment within BRTD (default 0x200).
    #[arg(long)]
    align: Option<u32>,

    /// Skip elements that already exist (idempotent re-runs).
    #[arg(long)]
    skip_existing: bool,

    /// Also fail validation when BNTX dimensions disagree with the manifest.
    #[arg(long)]
    strict_dimensions: bool,

    /// Write the output even if post-apply validation reports failures
    /// (the verb still exits non-zero).
    #[arg(long)]
    allow_invalid: bool,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let arc_bytes =
        fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let manifest_text = fs::read_to_string(&args.manifest)
        .with_context(|| format!("reading {}", args.manifest.display()))?;
    let manifest: SkinManifest = serde_json::from_str(&manifest_text)
        .with_context(|| format!("parsing {}", args.manifest.display()))?;
    let quality: Bc7Quality = args.quality.parse().map_err(|e| anyhow!("{e}"))?;

    let opts = ApplyOptions {
        bflyt_rel: args.bflyt,
        bntx_rel: args.bntx,
        pane_template: args.pane_template,
        material_template: args.material_template,
        quality,
        srgb: args.srgb,
        align: args.align,
        skip_existing: args.skip_existing,
    };

    let (out_arc, report) =
        apply_manifest_to_arc(&arc_bytes, &manifest, &args.skin_dir, &opts, args.strict_dimensions)?;

    let valid = report.validation.all_passed();
    println!(
        "applied {} element(s); skipped {}; validation {}/{} passed; archive has {} entries, {} bytes",
        report.applied,
        report.skipped,
        report.validation.passed,
        report.validation.element_count,
        report.file_count,
        report.out_arc_len
    );
    if !valid {
        eprintln!("validation FAILED for {} element(s):", report.validation.failed);
        for r in report.validation.results.iter().filter(|r| !r.ok) {
            eprintln!("  {}: {}", r.pane_name, r.failures.join("; "));
        }
        if !args.allow_invalid {
            return Ok(ExitCode::FAILURE);
        }
    }

    crate::verbs::write_output(&args.out, &out_arc)?;
    println!("wrote {}", args.out.display());
    if valid {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}
