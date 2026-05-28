//! `bntx-remove-texture`: remove a single named texture from a BNTX,
//! rebuilding the string pool, dict, BRTD layout, and `_RLT` so the
//! file remains internally consistent.
//!
//! This is the structural-change counterpart to `bntx-replace-png`
//! (which preserves layout). Use this verb when:
//!
//! - A skin no longer needs a particular face icon and you want the
//!   asset out entirely (size win on `__Combined.bntx`).
//! - You're about to re-import the same texture name with different
//!   shape (different dimensions, mip count, or format); `remove` +
//!   `import-png` is the supported workflow for that.
//!
//! The relocation table is regenerated canonically (matching the
//! Nintendo 8-entry compact layout), since structural changes shift
//! every downstream offset.

use anyhow::{Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bntx::{read_bntx, write_bntx};

#[derive(Parser, Debug)]
pub struct Args {
    /// Input BNTX file.
    #[arg(short, long)]
    input: PathBuf,

    /// Output BNTX (defaults to overwriting `input`).
    #[arg(short, long)]
    out: Option<PathBuf>,

    /// Name of the texture to remove. Must already exist in the BNTX
    /// dict. Use `bntx-inspect` to list available names.
    #[arg(long)]
    name: String,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bntx_bytes = fs::read(&args.input)
        .with_context(|| format!("reading {}", args.input.display()))?;
    let mut bntx = read_bntx(&bntx_bytes).map_err(|e| anyhow::anyhow!("{}", e))?;

    let before_count = bntx.textures.len();
    let before_brtd = bntx.brtd.data.len();

    bntx.remove_texture(&args.name)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let written = write_bntx(&bntx).map_err(|e| anyhow::anyhow!("{}", e))?;
    let out_path = args.out.as_ref().unwrap_or(&args.input);
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(out_path, &written)
        .with_context(|| format!("writing {}", out_path.display()))?;

    println!(
        "ok: removed texture '{}' ({} -> {} textures, BRTD {} -> {} bytes), file is now {} bytes",
        args.name,
        before_count,
        bntx.textures.len(),
        before_brtd,
        bntx.brtd.data.len(),
        written.len(),
    );
    Ok(ExitCode::SUCCESS)
}
