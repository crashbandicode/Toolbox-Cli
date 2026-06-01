//! `mc-inspect`: structured snapshot of a TotK MeshCodec (`MCPK`) container —
//! version, flags, the decompressed-size descriptor (and the size/alignment it
//! decodes to), and the compressed-stream length. Read-only; does not
//! decompress. Use `--json`.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use serde_json::json;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::mc::read_mc;

#[derive(Parser, Debug)]
pub struct Args {
    /// MeshCodec container (`*.bfres.mc`).
    #[arg(short, long)]
    input: PathBuf,

    /// Emit JSON instead of human-readable text.
    #[arg(long)]
    json: bool,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bytes =
        std::fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let mc = read_mc(&bytes).map_err(|e| anyhow!("{e}"))?;

    if args.json {
        let out = json!({
            "path": args.input.display().to_string(),
            "file_size": bytes.len(),
            "version": mc.header.version,
            "flags": mc.header.flags,
            "size_descriptor": mc.header.size_descriptor,
            "decompressed_size": mc.decompressed_size(),
            "alignment_shift": mc.header.alignment_shift(),
            "compressed_stream_size": mc.compressed_stream().len(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("{} ({} bytes)", args.input.display(), bytes.len());
        println!("  magic = MCPK  version = {}  flags = {}", mc.header.version, mc.header.flags);
        println!(
            "  size_descriptor = 0x{:08x} -> decompressed {} bytes (align 1<<{})",
            mc.header.size_descriptor,
            mc.decompressed_size(),
            mc.header.alignment_shift()
        );
        println!(
            "  compressed stream = {} bytes (from +0x{:x})",
            mc.compressed_stream().len(),
            crate::mc::MC_HEADER_LEN
        );
        println!("  (inner stream is magicless-zstd + an executable-embedded dictionary; not decompressed here)");
    }
    Ok(ExitCode::SUCCESS)
}
