use anyhow::{Context, Result};
use clap::Parser;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bntx::read_bntx;

#[derive(Parser, Debug)]
pub struct Args {
    /// Path to the .bntx file.
    #[arg(short, long)]
    input: PathBuf,

    /// Emit JSON instead of human-readable text.
    #[arg(long)]
    json: bool,

    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    indent: bool,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bytes =
        fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let bntx = read_bntx(&bytes).map_err(|e| anyhow::anyhow!("{}", e))?;

    let channel_name = |b: u8| match b {
        0 => "Zero",
        1 => "One",
        2 => "Red",
        3 => "Green",
        4 => "Blue",
        5 => "Alpha",
        _ => "?",
    };

    let textures: Vec<Value> = bntx
        .textures
        .iter()
        .map(|t| {
            let cs = bntx.channel_swizzle(t);
            json!({
                "name": t.name(&bntx),
                "width": t.width,
                "height": t.height,
                "depth": t.depth,
                "mip_count": t.mips_count,
                "array_count": t.array_len,
                "format": t.format.name(),
                "channels": [
                    channel_name(cs[0]),
                    channel_name(cs[1]),
                    channel_name(cs[2]),
                    channel_name(cs[3]),
                ],
                "has_alpha": t.format.has_alpha(),
            })
        })
        .collect();

    let doc = json!({
        "path": args.input.display().to_string(),
        "file_size": bytes.len(),
        "name": bntx.name,
        "texture_count": bntx.textures.len(),
        "textures": textures,
    });

    if args.json {
        if args.indent {
            println!("{}", serde_json::to_string_pretty(&doc)?);
        } else {
            println!("{}", serde_json::to_string(&doc)?);
        }
    } else {
        println!(
            "{} (textures={}, file_size={})",
            args.input.display(),
            bntx.textures.len(),
            bytes.len()
        );
        for t in &bntx.textures {
            println!(
                "  {:<32}  {}x{}  {}  mips={}  alpha={}",
                t.name(&bntx),
                t.width,
                t.height,
                t.format.name(),
                t.mips_count,
                t.format.has_alpha()
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}
