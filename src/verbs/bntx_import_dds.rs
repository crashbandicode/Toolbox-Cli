//! `bntx-import-dds`: swizzle a DDS surface and append it as a new named
//! texture in a BNTX. The DDS's format/dimensions/mip/array are
//! preserved; the canonical Tegra block height is inferred.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bntx::pipeline::import_dds;
use crate::bntx::{read_bntx, write_bntx};
use crate::dds::Dds;

#[derive(Parser, Debug)]
pub struct Args {
    /// Input BNTX file.
    #[arg(short, long)]
    input: PathBuf,

    /// Output BNTX (defaults to overwriting `input`).
    #[arg(short, long)]
    out: Option<PathBuf>,

    /// Source DDS file.
    #[arg(long)]
    dds: PathBuf,

    /// Texture name as it should appear in the BNTX dict.
    #[arg(long)]
    name: String,

    /// Override the BRTD data alignment. Defaults to 0x200. Accepts
    /// decimal or `0x...` hex.
    #[arg(long, value_parser = parse_u32_dec_or_hex)]
    align: Option<u32>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bntx_bytes =
        fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let mut bntx = read_bntx(&bntx_bytes).map_err(|e| anyhow!("{e}"))?;

    let dds_bytes =
        fs::read(&args.dds).with_context(|| format!("reading {}", args.dds.display()))?;
    let dds = Dds::read(&dds_bytes)?;

    import_dds(&mut bntx, &args.name, &dds, args.align)?;

    let written = write_bntx(&bntx).map_err(|e| anyhow!("{e}"))?;
    let out_path = args.out.as_ref().unwrap_or(&args.input);
    crate::verbs::write_output(out_path, &written)?;
    println!(
        "ok: imported '{}' ({}x{} {}) from {}, file is now {} bytes",
        args.name,
        dds.width,
        dds.height,
        dds.format.name(),
        args.dds.display(),
        written.len()
    );
    Ok(ExitCode::SUCCESS)
}

/// Accept either `0x123` (hex) or `123` (decimal) for u32 CLI flags.
fn parse_u32_dec_or_hex(s: &str) -> std::result::Result<u32, String> {
    if let Some(stripped) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(stripped, 16).map_err(|e| format!("not a hex number: {e}"))
    } else {
        s.parse::<u32>().map_err(|e| format!("not a number: {e}"))
    }
}
