//! Internal verb: dump per-texture layout info — data offsets, sizes, and
//! alignment — to inform the append-texture implementation.

use anyhow::{Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bntx::read_bntx;

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(short, long)]
    input: PathBuf,
    /// How many textures to print.
    #[arg(short, long, default_value_t = 10)]
    n: usize,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bytes = fs::read(&args.input)
        .with_context(|| format!("reading {}", args.input.display()))?;
    let bntx = read_bntx(&bytes).map_err(|e| anyhow::anyhow!("{}", e))?;

    let alignment = 1u64 << bntx.header.alignment_shift;
    println!(
        "alignment_shift = {} -> {} bytes",
        bntx.header.alignment_shift, alignment
    );
    println!("brtd block_size = 0x{:x}", bntx.brtd.declared_block_size);
    println!("brtd data length = {} bytes", bntx.brtd.data.len());

    let n = args.n.min(bntx.textures.len());
    println!("first {n} textures (offset/size/alignment in brtd):");
    for (i, tex) in bntx.textures.iter().take(n).enumerate() {
        let off = tex.data_offset_in_brtd as u64;
        let aligned = off & !(alignment - 1);
        let pad = off - aligned;
        println!(
            "  [{i:>3}] off=0x{:>8x}  size=0x{:>6x}  ({}x{} {})  pad-to-align={}",
            off,
            tex.image_size,
            tex.width,
            tex.height,
            tex.format.name(),
            pad,
        );
    }
    Ok(ExitCode::SUCCESS)
}
