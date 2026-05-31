//! `byml-inspect`: structured snapshot of a BYML / BYAML document (header,
//! endianness, version, and the decoded value tree). Transparently inflates
//! a zstd-compressed `.byml.zs` (TotK dictionaries via `--dict`/`--romfs`);
//! uncompressed `.byml`/`.bgyml` are read directly. Use `--json` for tooling
//! and `--max-depth` to bound very large trees.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use serde_json::{json, Map, Number, Value};
use std::path::PathBuf;
use std::process::ExitCode;

use crate::byml::{read_byml, Byml};
use crate::compression;

#[derive(Parser, Debug)]
pub struct Args {
    /// Path to the BYML file (`.byml`, `.bgyml`, or compressed `.byml.zs`).
    #[arg(short, long)]
    input: PathBuf,

    /// Emit JSON instead of human-readable text.
    #[arg(long)]
    json: bool,

    /// Indent JSON output. Pass --no-indent for compact JSON.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    indent: bool,

    /// Maximum tree depth to render (0 = unlimited). Deeper containers are
    /// shown as a `{ "__array__": N }` / `{ "__hash__": N }` placeholder.
    #[arg(long, default_value_t = 0)]
    max_depth: usize,

    /// TotK `ZsDic.pack.zs` (or a directory of `*.zsdic`) for a
    /// dictionary-compressed `.byml.zs`.
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

    let doc = read_byml(&bytes).map_err(|e| anyhow!("{e}"))?;
    let limit = if args.max_depth == 0 {
        usize::MAX
    } else {
        args.max_depth
    };

    if args.json {
        let out = json!({
            "path": args.input.display().to_string(),
            "file_size": raw.len(),
            "decompressed_size": bytes.len(),
            "compression": codec.label(),
            "version": doc.version,
            "endian": if doc.big_endian { "big" } else { "little" },
            "root_type": doc.root.type_name(),
            "root": byml_to_json(&doc.root, 0, limit),
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
            "  version = {}  endian = {}",
            doc.version,
            if doc.big_endian { "big" } else { "little" }
        );
        let (kind, n) = root_summary(&doc.root);
        println!("  root = {kind} ({n})");
        print_tree(&doc.root, 1, limit);
    }
    Ok(ExitCode::SUCCESS)
}

/// `(kind, "N entries"/"N elements")` summary line for the root.
fn root_summary(v: &Byml) -> (&'static str, String) {
    match v {
        Byml::Hash(h) => ("hash", format!("{} entr(ies)", h.len())),
        Byml::Array(a) => ("array", format!("{} element(s)", a.len())),
        other => (other.type_name(), scalar_text(other)),
    }
}

/// Human-readable text for a scalar value.
fn scalar_text(v: &Byml) -> String {
    match v {
        Byml::Null => "null".into(),
        Byml::Bool(b) => b.to_string(),
        Byml::I32(n) => n.to_string(),
        Byml::U32(n) => n.to_string(),
        Byml::F32(n) => n.to_string(),
        Byml::I64(n) => n.to_string(),
        Byml::U64(n) => n.to_string(),
        Byml::F64(n) => n.to_string(),
        Byml::String(s) => format!("{s:?}"),
        Byml::Binary(b) => format!("<binary {} bytes>", b.len()),
        Byml::Array(a) => format!("<array {}>", a.len()),
        Byml::Hash(h) => format!("<hash {}>", h.len()),
    }
}

/// Print an indented tree, bounded by `limit` depth.
fn print_tree(v: &Byml, depth: usize, limit: usize) {
    let pad = "  ".repeat(depth);
    match v {
        Byml::Hash(entries) => {
            for (k, child) in entries {
                if child.is_container() && depth >= limit {
                    println!("{pad}{k}: {}", scalar_text(child));
                } else if child.is_container() {
                    let (kind, n) = root_summary(child);
                    println!("{pad}{k}: {kind} ({n})");
                    print_tree(child, depth + 1, limit);
                } else {
                    println!("{pad}{k}: {}", scalar_text(child));
                }
            }
        }
        Byml::Array(items) => {
            for (i, child) in items.iter().enumerate() {
                if child.is_container() && depth >= limit {
                    println!("{pad}[{i}]: {}", scalar_text(child));
                } else if child.is_container() {
                    let (kind, n) = root_summary(child);
                    println!("{pad}[{i}]: {kind} ({n})");
                    print_tree(child, depth + 1, limit);
                } else {
                    println!("{pad}[{i}]: {}", scalar_text(child));
                }
            }
        }
        _ => {}
    }
}

/// Convert a finite f64 to a JSON number, falling back to a string for
/// NaN/±inf (which JSON can't represent).
fn float_value(x: f64) -> Value {
    match Number::from_f64(x) {
        Some(n) => Value::Number(n),
        None => Value::String(x.to_string()),
    }
}

/// Render a [`Byml`] value as JSON. Containers past `limit` depth become a
/// `{ "__array__": N }` / `{ "__hash__": N }` placeholder.
fn byml_to_json(v: &Byml, depth: usize, limit: usize) -> Value {
    match v {
        Byml::Null => Value::Null,
        Byml::Bool(b) => Value::Bool(*b),
        Byml::I32(n) => json!(*n),
        Byml::U32(n) => json!(*n),
        Byml::F32(n) => float_value(*n as f64),
        Byml::I64(n) => json!(*n),
        Byml::U64(n) => json!(*n),
        Byml::F64(n) => float_value(*n),
        Byml::String(s) => Value::String(s.clone()),
        Byml::Binary(b) => json!({ "__binary_len__": b.len() }),
        Byml::Array(items) => {
            if depth >= limit {
                return json!({ "__array__": items.len() });
            }
            Value::Array(
                items
                    .iter()
                    .map(|c| byml_to_json(c, depth + 1, limit))
                    .collect(),
            )
        }
        Byml::Hash(entries) => {
            if depth >= limit {
                return json!({ "__hash__": entries.len() });
            }
            let mut map = Map::new();
            for (k, child) in entries {
                map.insert(k.clone(), byml_to_json(child, depth + 1, limit));
            }
            Value::Object(map)
        }
    }
}
