//! `msbt-export-json` / `msbt-import-json`: round-trippable JSON for editing
//! MSBT message text (the translation-modding workflow).
//!
//! Export writes a label → message map, where each message is an array of
//! chunks: a JSON string is a literal text run, and a `{ "tag": … }` /
//! `{ "close": … }` object is a control tag (its payload as a hex string).
//! Import reads the original `.msbt`, overlays the edited messages by label,
//! and re-serializes with the canonical writer (semantically lossless). Tags
//! survive untouched; translators edit the string runs.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::process::ExitCode;

use crate::compression;
use crate::msbt::{read_msbt, write_msbt_canonical, Message, TextChunk};

// ---------------------------------------------------------------------------
// export
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
pub struct ExportArgs {
    /// Path to the MSBT file (`.msbt` or compressed `.msbt.zs`).
    #[arg(short, long)]
    input: PathBuf,

    /// Write JSON here instead of stdout.
    #[arg(short, long)]
    out: Option<PathBuf>,

    /// Compact JSON (default is pretty-printed).
    #[arg(long)]
    compact: bool,

    /// TotK `ZsDic.pack.zs` (or a directory of `*.zsdic`) for a
    /// dictionary-compressed `.msbt.zs`.
    #[arg(long)]
    dict: Option<PathBuf>,

    /// TotK RomFS root; auto-finds `Pack/ZsDic.pack.zs`.
    #[arg(long)]
    romfs: Option<PathBuf>,
}

pub fn export_run(args: ExportArgs) -> Result<ExitCode> {
    let raw =
        std::fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let dicts = super::load_dict_registry(args.dict.as_deref(), args.romfs.as_deref())?;
    let bytes = compression::decompress(&raw, &dicts).map_err(|e| anyhow!("{e}"))?;
    let doc = read_msbt(&bytes).map_err(|e| anyhow!("{e}"))?;

    let mut messages = Map::new();
    for (label, msg) in doc.entries() {
        let chunks: Vec<Value> = msg
            .chunks(doc.encoding, doc.big_endian)
            .iter()
            .map(chunk_to_json)
            .collect();
        messages.insert(label.to_string(), Value::Array(chunks));
    }

    let out = json!({
        "endian": if doc.big_endian { "big" } else { "little" },
        "encoding": doc.encoding.label(),
        "version": doc.version,
        "messages": messages,
    });
    let text = if args.compact {
        serde_json::to_string(&out)?
    } else {
        serde_json::to_string_pretty(&out)?
    };

    match &args.out {
        Some(path) => {
            super::write_output(path, text.as_bytes())?;
            println!(
                "exported {} message(s) -> {}",
                doc.entries().len(),
                path.display()
            );
        }
        None => println!("{text}"),
    }
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------------------
// import
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
pub struct ImportArgs {
    /// The original MSBT to patch (`.msbt` or compressed `.msbt.zs`).
    #[arg(short, long)]
    input: PathBuf,

    /// The edited JSON (from `msbt-export-json`).
    #[arg(short, long)]
    json: PathBuf,

    /// Where to write the rebuilt `.msbt` (uncompressed).
    #[arg(short, long)]
    out: PathBuf,

    /// Error out if the JSON references a label the MSBT doesn't have
    /// (default: warn and skip).
    #[arg(long)]
    strict: bool,

    /// TotK `ZsDic.pack.zs` (or a directory of `*.zsdic`) for a
    /// dictionary-compressed `.msbt.zs` input.
    #[arg(long)]
    dict: Option<PathBuf>,

    /// TotK RomFS root; auto-finds `Pack/ZsDic.pack.zs`.
    #[arg(long)]
    romfs: Option<PathBuf>,
}

pub fn import_run(args: ImportArgs) -> Result<ExitCode> {
    let raw =
        std::fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let dicts = super::load_dict_registry(args.dict.as_deref(), args.romfs.as_deref())?;
    let bytes = compression::decompress(&raw, &dicts).map_err(|e| anyhow!("{e}"))?;
    let mut doc = read_msbt(&bytes).map_err(|e| anyhow!("{e}"))?;

    let json_text = std::fs::read_to_string(&args.json)
        .with_context(|| format!("reading {}", args.json.display()))?;
    let parsed: Value = serde_json::from_str(&json_text)
        .with_context(|| format!("parsing {}", args.json.display()))?;
    let messages = parsed
        .get("messages")
        .and_then(|m| m.as_object())
        .ok_or_else(|| anyhow!("JSON has no \"messages\" object"))?;

    let mut applied = 0usize;
    let mut missing = 0usize;
    for (label, chunks_json) in messages {
        let arr = chunks_json
            .as_array()
            .ok_or_else(|| anyhow!("message {label:?} is not an array of chunks"))?;
        let chunks: Vec<TextChunk> = arr
            .iter()
            .map(json_to_chunk)
            .collect::<Result<_>>()
            .with_context(|| format!("decoding chunks for {label:?}"))?;
        let message = Message::from_chunks(&chunks, doc.encoding, doc.big_endian);
        if doc.set_message_by_label(label, message) {
            applied += 1;
        } else {
            missing += 1;
            if args.strict {
                return Err(anyhow!(
                    "label {label:?} not found in {}",
                    args.input.display()
                ));
            }
            eprintln!("  warning: label {label:?} not found — skipping");
        }
    }

    let rebuilt = write_msbt_canonical(&doc).map_err(|e| anyhow!("{e}"))?;
    super::write_output(&args.out, &rebuilt)?;
    println!(
        "applied {applied} message(s){} -> {} ({} bytes)",
        if missing > 0 {
            format!(", {missing} skipped")
        } else {
            String::new()
        },
        args.out.display(),
        rebuilt.len(),
    );
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------------------
// chunk <-> JSON
// ---------------------------------------------------------------------------

fn chunk_to_json(c: &TextChunk) -> Value {
    match c {
        TextChunk::Text(s) => Value::String(s.clone()),
        TextChunk::Tag { group, ty, data } => json!({
            "tag": { "g": group, "t": ty, "data": to_hex(data) }
        }),
        TextChunk::TagClose { group, ty } => json!({ "close": { "g": group, "t": ty } }),
    }
}

fn json_to_chunk(v: &Value) -> Result<TextChunk> {
    if let Some(s) = v.as_str() {
        return Ok(TextChunk::Text(s.to_string()));
    }
    let obj = v
        .as_object()
        .ok_or_else(|| anyhow!("chunk must be a string or object, got {v}"))?;
    if let Some(tag) = obj.get("tag").and_then(|t| t.as_object()) {
        let data = match tag.get("data").and_then(|d| d.as_str()) {
            Some(h) => from_hex(h)?,
            None => Vec::new(),
        };
        return Ok(TextChunk::Tag {
            group: field_u16(tag, "g")?,
            ty: field_u16(tag, "t")?,
            data,
        });
    }
    if let Some(close) = obj.get("close").and_then(|c| c.as_object()) {
        return Ok(TextChunk::TagClose {
            group: field_u16(close, "g")?,
            ty: field_u16(close, "t")?,
        });
    }
    Err(anyhow!(
        "chunk object must have a \"tag\" or \"close\" key: {v}"
    ))
}

fn field_u16(obj: &Map<String, Value>, key: &str) -> Result<u16> {
    let n = obj
        .get(key)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow!("missing/invalid u16 field {key:?}"))?;
    u16::try_from(n).map_err(|_| anyhow!("field {key:?} = {n} out of u16 range"))
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn from_hex(s: &str) -> Result<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return Err(anyhow!("hex string has odd length: {s:?}"));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| anyhow!("bad hex {s:?}: {e}")))
        .collect()
}
