//! `bfres-inspect`: structured snapshot of a BFRES (`FRES`) container — version,
//! endianness, embedded file name, file size, relocation-table offset, and a
//! structural scan of the well-known sub-block magics. When the BFRES embeds a
//! BNTX (BOTW `.Tex.bfres`), its textures are surfaced via the BNTX reader.
//! Transparently inflates a Yaz0 (`.sbfres`) / zstd (`.bfres.zs`) input. Use
//! `--json`.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use serde_json::json;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bfres::read_bfres;
use crate::bntx::read_bntx;
use crate::compression;

#[derive(Parser, Debug)]
pub struct Args {
    /// BFRES file (`.bfres`, or compressed `.sbfres` / `.bfres.zs`).
    #[arg(short, long)]
    input: PathBuf,

    /// Emit JSON instead of human-readable text.
    #[arg(long)]
    json: bool,

    /// Indent JSON output. Pass `--indent false` for compact JSON.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    indent: bool,

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
    let doc = read_bfres(&bytes).map_err(|e| anyhow!("{e}"))?;

    // Surface an embedded BNTX (BOTW `.Tex.bfres`) via the BNTX reader.
    let bntx = doc.embedded_bntx_bytes().and_then(|b| read_bntx(b).ok());

    if args.json {
        let out = json!({
            "path": args.input.display().to_string(),
            "compression": codec.label(),
            "name": doc.name,
            "version": doc.version_label(),
            "endian": if doc.big_endian { "big" } else { "little" },
            "file_size": doc.file_size,
            "actual_size": bytes.len(),
            "relocation_table_offset": doc.relocation_table_offset,
            "blocks": doc.blocks.iter().map(|b| json!({
                "magic": b.magic, "count": b.count, "first_offset": b.first_offset,
            })).collect::<Vec<_>>(),
            "embedded_bntx": bntx.as_ref().map(|f| json!({
                "name": f.name,
                "textures": f.textures.iter().map(|t| json!({
                    "name": t.name(f),
                    "format": t.format.name(),
                    "width": t.width, "height": t.height, "mips": t.mips_count,
                })).collect::<Vec<_>>(),
            })),
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
        println!("  name = {:?}", doc.name);
        println!("  version = {}", doc.version_label());
        println!(
            "  endian = {}",
            if doc.big_endian { "big" } else { "little" }
        );
        if (doc.file_size as usize) == bytes.len() {
            println!("  file_size = {} bytes", doc.file_size);
        } else {
            println!(
                "  file_size = {} bytes (file padded to {})",
                doc.file_size,
                bytes.len()
            );
        }
        println!(
            "  relocation_table_offset = 0x{:x}",
            doc.relocation_table_offset
        );
        println!("  blocks:");
        for b in &doc.blocks {
            println!(
                "    {:<5} x{:<4} (first @ 0x{:x})",
                b.magic, b.count, b.first_offset
            );
        }
        if let Some(f) = &bntx {
            println!(
                "  embedded BNTX {:?}: {} texture(s)",
                f.name,
                f.textures.len()
            );
            for t in &f.textures {
                println!(
                    "    {} {} {}x{} mips={}",
                    t.name(f),
                    t.format.name(),
                    t.width,
                    t.height,
                    t.mips_count
                );
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}
