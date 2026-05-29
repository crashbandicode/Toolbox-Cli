//! Debugging verb: per-material size comparison between the original and
//! our rewrite.

use anyhow::{Context, Result};
use byteorder::{LittleEndian, ReadBytesExt};
use clap::Parser;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bflyt::{read_bflyt, write_bflyt};

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(short, long)]
    input: PathBuf,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let original =
        fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let parsed = read_bflyt(&original).map_err(|e| anyhow::anyhow!("{}", e))?;
    let rewritten = write_bflyt(&parsed).map_err(|e| anyhow::anyhow!("{}", e))?;

    let orig_sizes = mat1_material_sizes(&original);
    let mine_sizes = mat1_material_sizes(&rewritten);

    println!(
        "original materials: {} sizes; rewritten materials: {} sizes",
        orig_sizes.len(),
        mine_sizes.len()
    );
    let n = orig_sizes.len().min(mine_sizes.len());
    let mut total_diff = 0i64;
    let mut diff_count = 0usize;
    for i in 0..n {
        if orig_sizes[i] != mine_sizes[i] {
            let m = &parsed.materials[i];
            println!(
                "  [{i:>3}] orig={:<5} mine={:<5} diff={:+} name={} flags=0x{:08x}",
                orig_sizes[i],
                mine_sizes[i],
                orig_sizes[i] as i64 - mine_sizes[i] as i64,
                m.name,
                m.flags_raw,
            );
            total_diff += orig_sizes[i] as i64 - mine_sizes[i] as i64;
            diff_count += 1;
            if diff_count >= 30 {
                println!("  ...");
                break;
            }
        }
    }
    println!("total diff across the first {diff_count} differing materials: {total_diff}");
    Ok(ExitCode::SUCCESS)
}

/// Returns the size of each material in the mat1 section, derived from
/// the consecutive entries of the mat1 offset table. The last entry uses
/// `mat1_section_end - last_offset` so it includes any trailing alignment.
fn mat1_material_sizes(data: &[u8]) -> Vec<usize> {
    let header_size = 0x14usize;
    let section_count = u16::from_le_bytes([data[0x10], data[0x11]]) as usize;

    let mut offset = header_size;
    for _ in 0..section_count {
        let magic = &data[offset..offset + 4];
        let size = u32::from_le_bytes([
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]) as usize;
        if magic == b"mat1" {
            // Parse the offset table.
            let payload = &data[offset + 8..offset + size];
            let mut c = Cursor::new(payload);
            let count = c.read_u16::<LittleEndian>().unwrap() as usize;
            let _pad = c.read_u16::<LittleEndian>().unwrap();
            let mut offsets = Vec::with_capacity(count + 1);
            for _ in 0..count {
                offsets.push(c.read_u32::<LittleEndian>().unwrap() as usize);
            }
            // Sentinel = section_size (file-absolute relative to magic byte).
            offsets.push(size);

            let mut sizes = Vec::with_capacity(count);
            for i in 0..count {
                sizes.push(offsets[i + 1] - offsets[i]);
            }
            return sizes;
        }
        offset += size;
    }
    Vec::new()
}
