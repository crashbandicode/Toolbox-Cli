//! `bntx-replace-dds`: splice a DDS surface over an existing texture's
//! pixel data in place. The DDS must match the texture's format,
//! dimensions, mip count, and array/cube layout (no structural change).

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bntx::pipeline::replace_with_dds;
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

    /// Name of the texture to replace. Must already exist.
    #[arg(long)]
    name: String,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bntx_bytes =
        fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let mut bntx = read_bntx(&bntx_bytes).map_err(|e| anyhow!("{e}"))?;

    let dds_bytes =
        fs::read(&args.dds).with_context(|| format!("reading {}", args.dds.display()))?;
    let dds = Dds::read(&dds_bytes)?;

    replace_with_dds(&mut bntx, &args.name, &dds)?;

    let written = write_bntx(&bntx).map_err(|e| anyhow!("{e}"))?;
    let out_path = args.out.as_ref().unwrap_or(&args.input);
    crate::verbs::write_output(out_path, &written)?;
    println!(
        "ok: replaced '{}' from {}, file is now {} bytes",
        args.name,
        args.dds.display(),
        written.len()
    );
    Ok(ExitCode::SUCCESS)
}
