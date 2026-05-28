//! Internal verb: dump the BNTX `_RLT` relocation table contents to
//! reverse-engineer the entry layout.

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
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bytes = fs::read(&args.input)
        .with_context(|| format!("reading {}", args.input.display()))?;
    let bntx = read_bntx(&bytes).map_err(|e| anyhow::anyhow!("{}", e))?;

    println!("=== Relocation table ===");
    println!("sections ({}):", bntx.relocation_table.sections.len());
    for (i, s) in bntx.relocation_table.sections.iter().enumerate() {
        println!(
            "  [{i}] ptr=0x{:x} pos=0x{:x} size=0x{:x} idx={} count={}",
            s.pointer, s.position, s.size, s.index, s.count
        );
    }
    println!("entries ({}):", bntx.relocation_table.entries.len());
    for (i, e) in bntx.relocation_table.entries.iter().enumerate() {
        let stride_qwords = (e.offset_count + e.padding_count) as u32;
        let total_bytes = e.struct_count as u32 * stride_qwords * 8;
        let end_pos = e.position + total_bytes;
        println!(
            "  [{i}] pos=0x{:08x}..0x{:08x}  struct_count={}  offset_count={}  padding_count={}  stride={}qw  total={} bytes",
            e.position,
            end_pos,
            e.struct_count,
            e.offset_count,
            e.padding_count,
            stride_qwords,
            total_bytes,
        );
    }
    Ok(ExitCode::SUCCESS)
}
