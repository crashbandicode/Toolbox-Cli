//! `bntx-export-dds`: deswizzle one named texture and write it as a DDS
//! file (DX10 header) for lossless compressed-texture interchange.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bntx::pipeline::export_texture_dds;
use crate::bntx::read_bntx;

#[derive(Parser, Debug)]
pub struct Args {
    /// Input BNTX file.
    #[arg(short, long)]
    input: PathBuf,

    /// Name of the texture to export.
    #[arg(long)]
    name: String,

    /// Output DDS path.
    #[arg(short, long)]
    out: PathBuf,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bytes =
        fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let bntx = read_bntx(&bytes).map_err(|e| anyhow!("{e}"))?;

    let dds = export_texture_dds(&bntx, &args.name)?;
    let out_bytes = dds.write();
    crate::verbs::write_output(&args.out, &out_bytes)?;

    println!(
        "ok: exported '{}' ({}x{}, {}, {} mip(s), {} layer(s)) -> {} ({} bytes)",
        args.name,
        dds.width,
        dds.height,
        dds.format.name(),
        dds.mip_count,
        dds.array_count,
        args.out.display(),
        out_bytes.len()
    );
    Ok(ExitCode::SUCCESS)
}
