//! `aamp-inspect`: structured snapshot of an AAMP (binary parameter archive):
//! header, and the decoded list / object / parameter tree. Keys are stored as
//! CRC-32 hashes; pass `--names <file>` (a newline-delimited name list) to
//! resolve hashes back to readable names. Transparently inflates a compressed
//! input. Use `--json`.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use serde_json::{json, Number, Value as Json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::aamp::{read_aamp, Parameter, ParameterList, Value};
use crate::compression;
use crate::restbl::crc32;

#[derive(Parser, Debug)]
pub struct Args {
    /// AAMP file (`.bxml`, `.bgparamlist`, … or a compressed `.zs`).
    #[arg(short, long)]
    input: PathBuf,

    /// Emit JSON instead of human-readable text.
    #[arg(long)]
    json: bool,

    /// Indent JSON output. Pass `--indent false` for compact JSON.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    indent: bool,

    /// Newline-delimited name list; names whose CRC-32 matches a stored hash
    /// are shown instead of the hex hash.
    #[arg(long)]
    names: Option<PathBuf>,

    /// Dictionary pack (for a compressed `.zs` input).
    #[arg(long)]
    dict: Option<PathBuf>,

    /// RomFS root; auto-finds `Pack/ZsDic.pack.zs`.
    #[arg(long)]
    romfs: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let raw =
        std::fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let codec = compression::detect(&raw);
    let dicts = super::load_dict_registry(args.dict.as_deref(), args.romfs.as_deref())?;
    let bytes = compression::decompress(&raw, &dicts).map_err(|e| anyhow!("{e}"))?;
    let doc = read_aamp(&bytes).map_err(|e| anyhow!("{e}"))?;

    // Optional CRC-32 → name table.
    let mut names: HashMap<u32, String> = HashMap::new();
    if let Some(path) = &args.names {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading names file {}", path.display()))?;
        for line in text.lines() {
            let name = line.trim();
            if !name.is_empty() {
                names.insert(crc32(name.as_bytes()), name.to_string());
            }
        }
    }
    let resolve = |hash: u32| -> String {
        names
            .get(&hash)
            .cloned()
            .unwrap_or_else(|| format!("0x{hash:08x}"))
    };
    let (lists, objects, params) = doc.counts();

    if args.json {
        let out = json!({
            "path": args.input.display().to_string(),
            "compression": codec.label(),
            "pio_version": doc.pio_version,
            "type": doc.pio_type,
            "endian": if doc.big_endian { "big" } else { "little" },
            "lists": lists, "objects": objects, "params": params,
            "root": list_to_json(&doc.root, &resolve),
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
        println!("  type = {:?}  pio_version = {}", doc.pio_type, doc.pio_version);
        println!("  {lists} list(s) / {objects} object(s) / {params} param(s)");
        print_list(&doc.root, 1, &resolve, "param_root");
    }
    Ok(ExitCode::SUCCESS)
}

fn print_list(l: &ParameterList, depth: usize, resolve: &impl Fn(u32) -> String, name: &str) {
    let pad = "  ".repeat(depth);
    println!("{pad}list {name} ({} obj, {} list)", l.objects.len(), l.lists.len());
    let pad2 = "  ".repeat(depth + 1);
    for o in &l.objects {
        println!("{pad2}obj {} ({} param)", resolve(o.name_hash), o.params.len());
        let pad3 = "  ".repeat(depth + 2);
        for p in &o.params {
            println!("{pad3}{} = {}", resolve(p.name_hash), p.value.summary());
        }
    }
    for child in &l.lists {
        print_list(child, depth + 1, resolve, &resolve(child.name_hash));
    }
}

fn float_json(x: f32) -> Json {
    match Number::from_f64(x as f64) {
        Some(n) => Json::Number(n),
        None => Json::String(x.to_string()),
    }
}

fn value_to_json(v: &Value) -> Json {
    let floats = |a: &[f32]| Json::Array(a.iter().map(|&x| float_json(x)).collect());
    match v {
        Value::Bool(b) => json!(*b),
        Value::F32(x) => float_json(*x),
        Value::Int(i) => json!(*i),
        Value::U32(u) => json!(*u),
        Value::Vec2(a) => floats(a),
        Value::Vec3(a) => floats(a),
        Value::Vec4(a) => floats(a),
        Value::Color(a) => floats(a),
        Value::Quat(a) => floats(a),
        Value::Str { value, .. } => json!(value),
        Value::Curve { raw, .. } => json!({ "__curve_bytes__": raw.len() }),
        Value::BufferInt(v) => json!(v),
        Value::BufferF32(v) => Json::Array(v.iter().map(|&x| float_json(x)).collect()),
        Value::BufferU32(v) => json!(v),
        Value::BufferBinary(v) => json!({ "__binary_bytes__": v.len() }),
    }
}

fn param_to_json(p: &Parameter, resolve: &impl Fn(u32) -> String) -> Json {
    json!({
        "name": resolve(p.name_hash),
        "type": p.value.param_type().label(),
        "value": value_to_json(&p.value),
    })
}

fn list_to_json(l: &ParameterList, resolve: &impl Fn(u32) -> String) -> Json {
    json!({
        "name": resolve(l.name_hash),
        "objects": l.objects.iter().map(|o| json!({
            "name": resolve(o.name_hash),
            "params": o.params.iter().map(|p| param_to_json(p, resolve)).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "lists": l.lists.iter().map(|c| list_to_json(c, resolve)).collect::<Vec<_>>(),
    })
}
