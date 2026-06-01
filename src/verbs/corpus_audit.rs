//! `corpus-audit`: scan a romfs / extracted-file root, run the safest
//! applicable operation per format on every file (recursing into SARC archives
//! and inflating `.zs`/`.szs`), and write a JSON confidence manifest. Read-only;
//! never writes game assets. Use it to measure the real-corpus breadth a verb
//! needs to graduate to *Trusted* in `TRUST_MATRIX.md`.

use anyhow::{anyhow, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::corpus_audit::{iso8601_utc, AuditConfig, AuditReport, Auditor, Format};

#[derive(Parser, Debug)]
pub struct Args {
    /// RomFS root to scan (also auto-loads `Pack/ZsDic.pack.zs` for `.zs`).
    #[arg(long)]
    romfs: Option<PathBuf>,

    /// One or more explicit roots (files or directories) to scan.
    #[arg(short, long)]
    input: Vec<PathBuf>,

    /// Game label recorded in the manifest (e.g. `totk`, `botw`).
    #[arg(long, default_value = "unknown")]
    game: String,

    /// Comma-separated formats to record (default: all). One of:
    /// byml,msbt,sarc,bntx,restbl,aamp,bfres,bflyt,bflan.
    #[arg(long)]
    formats: Option<String>,

    /// Max SARC-recursion depth.
    #[arg(long, default_value_t = 4)]
    max_depth: usize,

    /// Write the JSON manifest here (otherwise printed to stdout).
    #[arg(long)]
    json: Option<PathBuf>,

    /// Explicit zstd dictionary pack (else `--romfs` auto-finds it).
    #[arg(long)]
    dict: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let formats = match &args.formats {
        None => Format::all().to_vec(),
        Some(s) => {
            let mut out = Vec::new();
            for tok in s.split(',') {
                let tok = tok.trim();
                if tok.is_empty() {
                    continue;
                }
                out.push(
                    Format::from_key(tok)
                        .ok_or_else(|| anyhow!("unknown format '{tok}' in --formats"))?,
                );
            }
            if out.is_empty() {
                Format::all().to_vec()
            } else {
                out
            }
        }
    };

    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(r) = &args.romfs {
        roots.push(r.clone());
    }
    roots.extend(args.input.iter().cloned());
    if roots.is_empty() {
        return Err(anyhow!("provide --romfs and/or --input <path> to scan"));
    }

    let dicts = super::load_dict_registry(args.dict.as_deref(), args.romfs.as_deref())?;
    let cfg = AuditConfig {
        formats,
        max_depth: args.max_depth,
    };

    let started = now_secs();
    let mut auditor = Auditor::new(&cfg, &dicts);
    for root in &roots {
        if !root.exists() {
            return Err(anyhow!("path not found: {}", root.display()));
        }
        auditor.audit_path(root);
    }
    let finished = now_secs();
    let (files_scanned, decompress_failed, unclassified) = (
        auditor.files_scanned,
        auditor.decompress_failed,
        auditor.unclassified,
    );
    let stats = auditor.into_stats();

    let report = AuditReport {
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        git_commit: git_commit(),
        game: args.game.clone(),
        input_root: roots
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
        started_at: iso8601_utc(started),
        finished_at: iso8601_utc(finished),
        files_scanned,
        decompress_failed,
        unclassified,
        formats: stats,
    };

    let json = serde_json::to_string_pretty(&report)?;
    if let Some(path) = &args.json {
        super::write_output(path, json.as_bytes())?;
        println!("wrote audit manifest -> {}", path.display());
    } else {
        println!("{json}");
    }

    // Human summary + a clear nonzero exit on any UNEXPECTED failure.
    let mut unexpected = 0u64;
    eprintln!(
        "scanned {} file(s); {} decompress-failed, {} unclassified",
        report.files_scanned, report.decompress_failed, report.unclassified
    );
    for (fmt, s) in &report.formats {
        if s.files_seen == 0 {
            continue;
        }
        unexpected += s.failed;
        eprintln!(
            "  {fmt:<7} seen={:<6} byte-identical={:<6} semantic={:<5} inspect={:<5} unsupported={:<4} FAILED={}",
            s.files_seen,
            s.roundtrip_byte_identical,
            s.semantic_roundtrip_ok,
            s.inspect_ok,
            s.expected_unsupported,
            s.failed,
        );
    }
    if unexpected > 0 {
        eprintln!("{unexpected} unexpected failure(s) — see the manifest's `failures`.");
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Best-effort short git commit; `"unknown"` if git isn't available.
fn git_commit() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}
