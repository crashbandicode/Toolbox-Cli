//! `layout-apply-manifest`: end-to-end orchestrator that materializes an
//! SGPO skin from a manifest + a folder of PNGs into an unpacked layout.
//! Thin wrapper over [`crate::layout::apply_manifest`].

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::layout::{apply_manifest, ApplyOptions};
use crate::manifest::SkinManifest;
use crate::texpipe::Bc7Quality;

#[derive(Parser, Debug)]
pub struct Args {
    /// Unpacked layout directory containing `blyt/` and `timg/`.
    #[arg(long)]
    layout_dir: PathBuf,

    /// Path to the SGPO skin_manifest.json.
    #[arg(long)]
    manifest: PathBuf,

    /// Directory containing the manifest's image_filename files.
    #[arg(long)]
    skin_dir: PathBuf,

    /// BFLYT path relative to --layout-dir.
    #[arg(long, default_value = "blyt/info_melee.bflyt")]
    bflyt: String,

    /// BNTX path relative to --layout-dir.
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

    /// Skip elements that already exist in the layout (idempotent re-runs).
    #[arg(long)]
    skip_existing: bool,
}

pub fn run(args: Args) -> Result<ExitCode> {
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
    let report = apply_manifest(&args.layout_dir, &manifest, &args.skin_dir, &opts)?;

    println!(
        "applied {} element(s); skipped {}; BFLYT now {} bytes; BNTX now {} bytes",
        report.applied, report.skipped, report.bflyt_bytes, report.bntx_bytes
    );
    Ok(ExitCode::SUCCESS)
}
