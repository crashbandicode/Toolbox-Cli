//! `msbt-inspect`: structured snapshot of an MSBT (LibMessageStudio message)
//! file — header (endianness, encoding, version), sections, and the decoded
//! label → message text. Transparently inflates a zstd-compressed `.msbt.zs`
//! (TotK dictionaries via `--dict`/`--romfs`). Use `--json` for tooling and
//! `--limit` to cap how many entries are printed.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::ExitCode;

use crate::compression;
use crate::msbt::{read_msbt, SectionData};

#[derive(Parser, Debug)]
pub struct Args {
    /// Path to the MSBT file (`.msbt` or compressed `.msbt.zs`).
    #[arg(short, long)]
    input: PathBuf,

    /// Emit JSON instead of human-readable text.
    #[arg(long)]
    json: bool,

    /// Indent JSON output. Pass `--indent false` for compact JSON.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    indent: bool,

    /// Maximum number of label→message entries to print (0 = unlimited).
    #[arg(long, default_value_t = 50)]
    limit: usize,

    /// TotK `ZsDic.pack.zs` (or a directory of `*.zsdic`) for a
    /// dictionary-compressed `.msbt.zs`.
    #[arg(long)]
    dict: Option<PathBuf>,

    /// TotK RomFS root; auto-finds `Pack/ZsDic.pack.zs`.
    #[arg(long)]
    romfs: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let raw =
        std::fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let codec = compression::detect(&raw);
    let dicts = super::load_dict_registry(args.dict.as_deref(), args.romfs.as_deref())?;
    let bytes = compression::decompress(&raw, &dicts).map_err(|e| anyhow!("{e}"))?;

    let doc = read_msbt(&bytes).map_err(|e| anyhow!("{e}"))?;
    let entries = doc.entries();
    let limit = if args.limit == 0 {
        usize::MAX
    } else {
        args.limit
    };

    if args.json {
        let sections: Vec<Value> = doc
            .sections
            .iter()
            .map(|s| {
                let kind = match &s.data {
                    SectionData::Labels(l) => json!({ "kind": "labels", "count": l.len() }),
                    SectionData::Text(t) => json!({ "kind": "text", "count": t.len() }),
                    SectionData::Opaque(b) => json!({ "kind": "opaque", "bytes": b.len() }),
                };
                json!({ "magic": s.magic_str(), "data": kind })
            })
            .collect();
        let shown: Vec<Value> = entries
            .iter()
            .take(limit)
            .map(|(label, msg)| {
                json!({
                    "label": label,
                    "text": msg.to_display(doc.encoding, doc.big_endian),
                })
            })
            .collect();
        let out = json!({
            "path": args.input.display().to_string(),
            "file_size": raw.len(),
            "decompressed_size": bytes.len(),
            "compression": codec.label(),
            "endian": if doc.big_endian { "big" } else { "little" },
            "encoding": doc.encoding.label(),
            "version": doc.version,
            "sections": sections,
            "label_count": doc.labels().map(|l| l.len()).unwrap_or(0),
            "message_count": doc.messages().map(|m| m.len()).unwrap_or(0),
            "entries_shown": shown.len(),
            "entries": shown,
        });
        if args.indent {
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            println!("{}", serde_json::to_string(&out)?);
        }
    } else {
        println!("{} ({} bytes)", args.input.display(), raw.len());
        if codec.is_compressed() {
            println!("  compression = {} -> {} bytes", codec.label(), bytes.len());
        }
        println!(
            "  endian = {}  encoding = {}  version = {}",
            if doc.big_endian { "big" } else { "little" },
            doc.encoding.label(),
            doc.version,
        );
        print!("  sections =");
        for s in &doc.sections {
            let n = match &s.data {
                SectionData::Labels(l) => l.len(),
                SectionData::Text(t) => t.len(),
                SectionData::Opaque(b) => b.len(),
            };
            print!(" {}({n})", s.magic_str());
        }
        println!();
        println!(
            "  {} label(s), {} message(s)",
            doc.labels().map(|l| l.len()).unwrap_or(0),
            doc.messages().map(|m| m.len()).unwrap_or(0),
        );
        for (label, msg) in entries.iter().take(limit) {
            println!("  {label}: {:?}", msg.to_display(doc.encoding, doc.big_endian));
        }
        if entries.len() > limit {
            println!("  ... ({} more; raise --limit)", entries.len() - limit);
        }
    }
    Ok(ExitCode::SUCCESS)
}
