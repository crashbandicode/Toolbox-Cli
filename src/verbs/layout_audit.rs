//! `layout-audit`: recursively scan a directory (or single file / archive)
//! for BFLYT/BNTX files and report unsupported or suspicious structures
//! as a JSON report. Thin wrapper over [`crate::audit::audit_path`].

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::audit::audit_path;

#[derive(Parser, Debug)]
pub struct Args {
    /// Directory, file, or archive to audit (directories are walked).
    #[arg(short, long)]
    path: PathBuf,

    /// Emit JSON instead of a human-readable summary.
    #[arg(long)]
    json: bool,

    /// Indent JSON output. Pass --no-indent for compact JSON.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    indent: bool,

    /// Exit non-zero if any file failed to parse (useful in CI).
    #[arg(long)]
    fail_on_error: bool,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let report =
        audit_path(&args.path).with_context(|| format!("auditing {}", args.path.display()))?;
    let t = &report.totals;

    if args.json {
        let s = if args.indent {
            serde_json::to_string_pretty(&report)?
        } else {
            serde_json::to_string(&report)?
        };
        println!("{s}");
    } else {
        println!("audit of {}", report.root);
        println!(
            "  BFLYT: {} scanned, {} failed, {} v9, {} with untrusted mat ({} mats), \
             {} with v9 extension ({} mats)",
            t.bflyt_scanned,
            t.bflyt_failed,
            t.bflyt_v9,
            t.bflyt_with_untrusted_mat,
            t.untrusted_materials,
            t.bflyt_with_v9_mat_extension,
            t.v9_extension_materials,
        );
        println!(
            "  BNTX:  {} scanned, {} failed ({} unsupported format)",
            t.bntx_scanned, t.bntx_failed, t.bntx_unsupported_format
        );
        println!(
            "  BFLAN: {} scanned, {} failed, {} with truncated final section",
            t.bflan_scanned, t.bflan_failed, t.bflan_truncated_section
        );
        println!("  ARC:   {} scanned, {} failed", t.arc_scanned, t.arc_failed);
        println!("  other: {} file(s)", t.other_files);
        if !report.files.is_empty() {
            println!("  findings:");
            for f in &report.files {
                if let Some(err) = &f.error {
                    println!("    [FAIL {}] {}: {}", f.kind, f.path, err);
                } else {
                    println!("    [{}] {}: {}", f.kind, f.path, f.findings.join("; "));
                }
            }
        }
    }

    let failed = t.bflyt_failed + t.bntx_failed + t.arc_failed;
    if args.fail_on_error && failed > 0 {
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}
