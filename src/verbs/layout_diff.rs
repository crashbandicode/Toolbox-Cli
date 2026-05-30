//! `layout-diff`: structured before/after diff of two packed
//! `layout.arc` files (their BFLYT + BNTX). Reports txl1 / material /
//! pane changes and BNTX texture changes. `--json` for tooling.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bflyt::read_bflyt;
use crate::bntx::read_bntx;
use crate::diff::{diff_layouts, LayoutDiff};
use crate::sarc::{read_arc, ArcFile};

#[derive(Parser, Debug)]
pub struct Args {
    /// "Before" layout.arc.
    #[arg(long)]
    old: PathBuf,

    /// "After" layout.arc.
    #[arg(long)]
    new: PathBuf,

    /// BFLYT path within each archive.
    #[arg(long, default_value = "blyt/info_melee.bflyt")]
    bflyt: String,

    /// BNTX path within each archive.
    #[arg(long, default_value = "timg/__Combined.bntx")]
    bntx: String,

    /// Emit JSON instead of a human-readable summary.
    #[arg(long)]
    json: bool,

    /// Indent JSON output. Pass --no-indent for compact JSON.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    indent: bool,
}

fn entry<'a>(arc: &'a ArcFile, name: &str, which: &str) -> Result<&'a [u8]> {
    let idx = arc
        .position(name)
        .ok_or_else(|| anyhow!("{which} entry '{name}' not found in archive"))?;
    Ok(&arc.files[idx].data)
}

pub fn run(args: Args) -> Result<ExitCode> {
    let old_bytes = fs::read(&args.old).with_context(|| format!("reading {}", args.old.display()))?;
    let new_bytes = fs::read(&args.new).with_context(|| format!("reading {}", args.new.display()))?;
    let old_arc = read_arc(&old_bytes)?;
    let new_arc = read_arc(&new_bytes)?;

    let old_bflyt = read_bflyt(entry(&old_arc, &args.bflyt, "old BFLYT")?).map_err(|e| anyhow!("{e}"))?;
    let new_bflyt = read_bflyt(entry(&new_arc, &args.bflyt, "new BFLYT")?).map_err(|e| anyhow!("{e}"))?;
    let old_bntx = read_bntx(entry(&old_arc, &args.bntx, "old BNTX")?).map_err(|e| anyhow!("{e}"))?;
    let new_bntx = read_bntx(entry(&new_arc, &args.bntx, "new BNTX")?).map_err(|e| anyhow!("{e}"))?;

    let diff = diff_layouts(&old_bflyt, &old_bntx, &new_bflyt, &new_bntx);

    if args.json {
        let s = if args.indent {
            serde_json::to_string_pretty(&diff)?
        } else {
            serde_json::to_string(&diff)?
        };
        println!("{s}");
    } else {
        print_summary(&diff);
    }

    Ok(ExitCode::SUCCESS)
}

fn print_summary(diff: &LayoutDiff) {
    if diff.is_empty() {
        println!("no changes");
        return;
    }
    let b = &diff.bflyt;
    println!("BFLYT:");
    print_list("  + texture ref", &b.textures_added);
    print_list("  - texture ref", &b.textures_removed);
    print_list("  + material", &b.materials_added);
    print_list("  - material", &b.materials_removed);
    for c in &b.materials_changed {
        println!("  ~ material {}: {}", c.name, c.changes.join("; "));
    }
    for p in &b.panes_added {
        println!(
            "  + pane {} ({}, parent={})",
            p.name,
            p.kind,
            p.parent.as_deref().unwrap_or("<root>")
        );
    }
    print_list("  - pane", &b.panes_removed);
    for c in &b.panes_changed {
        println!("  ~ pane {}: {}", c.name, c.changes.join("; "));
    }

    let n = &diff.bntx;
    println!("BNTX:");
    for t in &n.textures_added {
        println!("  + texture {} ({}x{} {})", t.name, t.width, t.height, t.format);
    }
    print_list("  - texture", &n.textures_removed);
    for c in &n.textures_changed {
        println!("  ~ texture {}: {}", c.name, c.changes.join("; "));
    }
}

fn print_list(label: &str, items: &[String]) {
    for i in items {
        println!("{label} {i}");
    }
}
