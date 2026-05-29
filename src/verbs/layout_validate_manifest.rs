//! `layout-validate-manifest`: read-only verifier that an unpacked layout
//! directory matches an SGPO skin manifest. Thin wrapper over
//! [`crate::layout::validate_manifest`]. Exits 0 if every element passes,
//! 1 if any element has a structural mismatch.

use anyhow::{Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::layout::{validate_manifest, ValidateOptions};
use crate::manifest::SkinManifest;

#[derive(Parser, Debug)]
pub struct Args {
    /// Unpacked layout directory (containing blyt/ and timg/).
    #[arg(long)]
    layout_dir: PathBuf,

    /// Path to the SGPO skin_manifest.json.
    #[arg(long)]
    manifest: PathBuf,

    /// BFLYT path relative to --layout-dir.
    #[arg(long, default_value = "blyt/info_melee.bflyt")]
    bflyt: String,

    /// BNTX path relative to --layout-dir.
    #[arg(long, default_value = "timg/__Combined.bntx")]
    bntx: String,

    /// Emit a JSON report instead of human-readable text.
    #[arg(long)]
    json: bool,

    /// Fail when BNTX texture dimensions disagree with the manifest size.
    #[arg(long)]
    strict_dimensions: bool,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let manifest_text = fs::read_to_string(&args.manifest)
        .with_context(|| format!("reading {}", args.manifest.display()))?;
    let manifest: SkinManifest = serde_json::from_str(&manifest_text)
        .with_context(|| format!("parsing {}", args.manifest.display()))?;

    let opts = ValidateOptions {
        bflyt_rel: args.bflyt.clone(),
        bntx_rel: args.bntx.clone(),
        strict_dimensions: args.strict_dimensions,
    };
    let report = validate_manifest(&args.layout_dir, &manifest, &opts)?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("manifest: {}", args.manifest.display());
        println!("layout:   {}", args.layout_dir.display());
        println!(
            "elements: {}  passed: {}  failed: {}",
            report.element_count, report.passed, report.failed
        );
        println!();
        for r in &report.results {
            println!("[{}] {}", if r.ok { "OK  " } else { "FAIL" }, r.pane_name);
            for f in &r.failures {
                println!("        - {f}");
            }
        }
    }
    Ok(if report.all_passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}
