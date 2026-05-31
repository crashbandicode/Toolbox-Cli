//! `restbl-inspect`: structured snapshot of a RESTBL (Resource Size Table) —
//! version, string-block size, table counts, the name (collision) table, and
//! an optional path/hash lookup. Inflates a compressed `.rsizetable.zs` first
//! (TotK dictionaries via `--dict`/`--romfs`).

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::ExitCode;

use crate::compression;
use crate::restbl::{crc32, read_restbl};

#[derive(Parser, Debug)]
pub struct Args {
    /// Path to the RESTBL (`.rsizetable`, `.rstbl`, or compressed `.zs`).
    #[arg(short, long)]
    input: PathBuf,

    /// Emit JSON instead of human-readable text.
    #[arg(long)]
    json: bool,

    /// Indent JSON output. Pass `--indent false` for compact JSON.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    indent: bool,

    /// Resolve a resource path's reserved size (CRC table, then name table).
    #[arg(long)]
    lookup: Option<String>,

    /// Resolve a raw CRC-32 hash (hex, optional `0x`).
    #[arg(long)]
    hash: Option<String>,

    /// Max name-table entries to print in text mode (0 = all).
    #[arg(long, default_value_t = 0)]
    max_names: usize,

    /// TotK `ZsDic.pack.zs` (or a directory of `*.zsdic`) for compressed input.
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
    let table = read_restbl(&bytes).map_err(|e| anyhow!("{e}"))?;

    let lookup_result: Option<Value> = args.lookup.as_ref().map(|path| {
        let h = crc32(path.as_bytes());
        json!({
            "path": path,
            "crc32": format!("0x{h:08x}"),
            "size": table.get_by_path(path),
        })
    });
    let hash_result: Option<Value> = args
        .hash
        .as_ref()
        .map(|h| -> Result<Value> {
            let parsed = parse_hex_u32(h)?;
            Ok(json!({
                "hash": format!("0x{parsed:08x}"),
                "size": table.get_by_hash(parsed),
            }))
        })
        .transpose()?;

    if args.json {
        let names: Vec<Value> = table
            .name_entries
            .iter()
            .map(|e| json!({ "name": e.name, "size": e.size }))
            .collect();
        let doc = json!({
            "path": args.input.display().to_string(),
            "file_size": raw.len(),
            "decompressed_size": bytes.len(),
            "compression": codec.label(),
            "version": table.version,
            "string_block_size": table.string_block_size,
            "crc_table_count": table.crc_entries.len(),
            "name_table_count": table.name_entries.len(),
            "name_table": names,
            "lookup": lookup_result,
            "hash_lookup": hash_result,
        });
        let s = if args.indent {
            serde_json::to_string_pretty(&doc)?
        } else {
            serde_json::to_string(&doc)?
        };
        println!("{s}");
    } else {
        println!("{} ({} bytes)", args.input.display(), raw.len());
        if codec.is_compressed() {
            println!("  compression = {} -> {} bytes", codec.label(), bytes.len());
        }
        println!(
            "  version = {}  string_block_size = {}",
            table.version, table.string_block_size
        );
        println!("  crc_table  = {} entries", table.crc_entries.len());
        println!("  name_table = {} entries", table.name_entries.len());
        if let Some(v) = &lookup_result {
            println!(
                "  lookup {} (crc {}): {}",
                v["path"].as_str().unwrap_or(""),
                v["crc32"].as_str().unwrap_or(""),
                size_text(&v["size"]),
            );
        }
        if let Some(v) = &hash_result {
            println!(
                "  hash {}: {}",
                v["hash"].as_str().unwrap_or(""),
                size_text(&v["size"]),
            );
        }
        let cap = if args.max_names == 0 {
            table.name_entries.len()
        } else {
            args.max_names
        };
        for e in table.name_entries.iter().take(cap) {
            println!("    {:>10}  {}", e.size, e.name);
        }
        if table.name_entries.len() > cap {
            println!("    … {} more", table.name_entries.len() - cap);
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn size_text(v: &Value) -> String {
    match v.as_u64() {
        Some(n) => format!("{n} bytes"),
        None => "not found".to_string(),
    }
}

fn parse_hex_u32(s: &str) -> Result<u32> {
    let t = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    u32::from_str_radix(t, 16).map_err(|e| anyhow!("invalid hex hash {s:?}: {e}"))
}
