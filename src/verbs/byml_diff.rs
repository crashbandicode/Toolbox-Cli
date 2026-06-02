//! `byml-diff`: structural before/after diff of two BYML documents, matching
//! hashes by key and arrays by index. Each compressed `.byml.zs` is inflated
//! first (TotK dictionaries via `--dict`/`--romfs`). Use `--json` for tooling.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::byml::{diff_byml, read_byml};
use crate::compression::{self, DictRegistry};

#[derive(Parser, Debug)]
pub struct Args {
    /// The "before" BYML (`.byml`/`.bgyml`/`.byml.zs`).
    #[arg(long)]
    old: PathBuf,

    /// The "after" BYML.
    #[arg(long)]
    new: PathBuf,

    /// Emit JSON instead of human-readable text.
    #[arg(long)]
    json: bool,

    /// Indent JSON output. Pass `--indent false` for compact JSON.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    indent: bool,

    /// Cap how many entries per category (added/removed/changed) to print in
    /// text mode (0 = all). JSON always includes everything.
    #[arg(long, default_value_t = 50)]
    limit: usize,

    /// TotK `ZsDic.pack.zs` (or a directory of `*.zsdic`) for compressed input.
    #[arg(long)]
    dict: Option<PathBuf>,

    /// TotK RomFS root; auto-finds `Pack/ZsDic.pack.zs`.
    #[arg(long)]
    romfs: Option<PathBuf>,
}

fn load_tree(path: &Path, dicts: &DictRegistry) -> Result<crate::byml::Byml> {
    let raw = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let bytes = compression::decompress(&raw, dicts).map_err(|e| anyhow!("{e}"))?;
    Ok(read_byml(&bytes).map_err(|e| anyhow!("{e}"))?.root)
}

pub fn run(args: Args) -> Result<ExitCode> {
    let dicts = super::load_dict_registry(args.dict.as_deref(), args.romfs.as_deref())?;
    let old = load_tree(&args.old, &dicts)?;
    let new = load_tree(&args.new, &dicts)?;
    let diff = diff_byml(&old, &new);

    if args.json {
        let doc = json!({
            "old": args.old.display().to_string(),
            "new": args.new.display().to_string(),
            "added": diff.added,
            "removed": diff.removed,
            "changed": diff.changed,
            "total": diff.total(),
        });
        if args.indent {
            println!("{}", serde_json::to_string_pretty(&doc)?);
        } else {
            println!("{}", serde_json::to_string(&doc)?);
        }
        return Ok(ExitCode::SUCCESS);
    }

    if diff.is_empty() {
        println!(
            "no differences ({} vs {})",
            args.old.display(),
            args.new.display()
        );
        return Ok(ExitCode::SUCCESS);
    }

    let cap = if args.limit == 0 {
        usize::MAX
    } else {
        args.limit
    };
    println!(
        "{} -> {}: +{} -{} ~{}",
        args.old.display(),
        args.new.display(),
        diff.added.len(),
        diff.removed.len(),
        diff.changed.len(),
    );
    for e in diff.added.iter().take(cap) {
        println!("  + {} = {}", e.path, e.value);
    }
    print_more(diff.added.len(), cap);
    for e in diff.removed.iter().take(cap) {
        println!("  - {} = {}", e.path, e.value);
    }
    print_more(diff.removed.len(), cap);
    for e in diff.changed.iter().take(cap) {
        println!("  ~ {} : {} -> {}", e.path, e.old, e.new);
    }
    print_more(diff.changed.len(), cap);
    Ok(ExitCode::SUCCESS)
}

fn print_more(total: usize, cap: usize) {
    if total > cap {
        println!("    … {} more (raise --limit or use --json)", total - cap);
    }
}
