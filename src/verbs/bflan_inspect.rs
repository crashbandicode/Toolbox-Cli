//! `bflan-inspect`: structured snapshot of a BFLAN (Cafe Layout
//! Animation) — header, section list, and decoded `pat1`/`pai1`.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bflan::{decode_pai1, decode_pat1, read_bflan};

#[derive(Parser, Debug)]
pub struct Args {
    /// Path to the .bflan file.
    #[arg(short, long)]
    input: PathBuf,

    /// Emit JSON instead of human-readable text.
    #[arg(long)]
    json: bool,

    /// Indent JSON output. Pass --no-indent for compact JSON.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    indent: bool,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bytes =
        fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let bflan = read_bflan(&bytes).map_err(|e| anyhow!("{e}"))?;

    let sections: Vec<Value> = bflan
        .sections
        .iter()
        .map(|s| json!({ "magic": s.magic_str(), "payload_bytes": s.payload.len() }))
        .collect();

    let pat1 = bflan
        .section(b"pat1")
        .and_then(|s| decode_pat1(&s.payload, bflan.version_major()))
        .map(|p| {
            json!({
                "animation_order": p.animation_order,
                "start_frame": p.start_frame,
                "end_frame": p.end_frame,
                "child_binding": p.child_binding,
                "name": p.name,
                "groups": p.groups,
            })
        });

    let pai1 = bflan
        .section(b"pai1")
        .and_then(|s| decode_pai1(&s.payload))
        .map(|p| {
            let entries: Vec<Value> = p
                .entries
                .iter()
                .map(|e| json!({ "name": e.name, "target": e.target, "tag_count": e.tag_count }))
                .collect();
            json!({
                "frame_size": p.frame_size,
                "loop": p.loops,
                "textures": p.textures,
                "entries": entries,
            })
        });

    let doc = json!({
        "path": args.input.display().to_string(),
        "file_size": bytes.len(),
        "version": format!(
            "{}.{}.{}.{}",
            (bflan.version >> 24) & 0xff,
            (bflan.version >> 16) & 0xff,
            (bflan.version >> 8) & 0xff,
            bflan.version & 0xff
        ),
        "section_count": bflan.sections.len(),
        "sections": sections,
        "pat1": pat1,
        "pai1": pai1,
    });

    if args.json {
        if args.indent {
            println!("{}", serde_json::to_string_pretty(&doc)?);
        } else {
            println!("{}", serde_json::to_string(&doc)?);
        }
    } else {
        println!("{} ({} bytes)", args.input.display(), bytes.len());
        println!("  version = {}", doc["version"].as_str().unwrap_or("?"));
        println!("  sections = {}", bflan.sections.len());
        for s in &bflan.sections {
            println!("    {} ({} payload bytes)", s.magic_str(), s.payload.len());
        }
        if let Some(p) = bflan.section(b"pat1").and_then(|s| decode_pat1(&s.payload, bflan.version_major())) {
            println!(
                "  pat1: name='{}' frames {}..{} order={} child_binding={} groups={:?}",
                p.name, p.start_frame, p.end_frame, p.animation_order, p.child_binding, p.groups
            );
        }
        if let Some(p) = bflan.section(b"pai1").and_then(|s| decode_pai1(&s.payload)) {
            println!(
                "  pai1: frame_size={} loop={} textures={:?} entries={}",
                p.frame_size,
                p.loops,
                p.textures,
                p.entries.len()
            );
            for e in &p.entries {
                println!("    entry '{}' target={} tags={}", e.name, e.target, e.tag_count);
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}
