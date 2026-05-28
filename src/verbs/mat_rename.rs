//! `mat-rename`: change a material's name in-place. Used by SGPO when
//! renaming materials to follow the `mat_<pane_name>` convention.

use anyhow::{anyhow, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bflyt::MAT_NAME_LEN_USIZE;
use crate::verbs::bflyt_helpers::rewrite_bflyt;

/// Re-exported here to avoid leaking the private `MAT_NAME_LEN` symbol
/// from the bflyt module. The size is constrained by the format spec.
pub const MAT_NAME_MAX_BYTES: usize = MAT_NAME_LEN_USIZE - 1;

#[derive(Parser, Debug)]
pub struct Args {
    /// BFLYT file to modify.
    #[arg(short, long)]
    input: PathBuf,

    /// Output BFLYT (defaults to overwriting the input).
    #[arg(short, long)]
    out: Option<PathBuf>,

    /// Current material name.
    #[arg(long)]
    from: String,

    /// New material name. Must be unique within the BFLYT and ≤ 27 bytes.
    #[arg(long)]
    to: String,
}

pub fn run(args: Args) -> Result<ExitCode> {
    if args.to.len() > MAT_NAME_MAX_BYTES {
        return Err(anyhow!(
            "new material name '{}' is {} bytes (max {})",
            args.to,
            args.to.len(),
            MAT_NAME_MAX_BYTES
        ));
    }
    let from = args.from.clone();
    let to = args.to.clone();
    let n = rewrite_bflyt(&args.input, args.out.as_deref(), |bflyt| {
        if bflyt.materials.iter().any(|m| m.name == to) {
            return Err(anyhow!(
                "material '{}' already exists in mat1; refusing to create a duplicate",
                to
            ));
        }
        let idx = bflyt
            .materials
            .iter()
            .position(|m| m.name == from)
            .ok_or_else(|| anyhow!("material '{}' not found in mat1", from))?;
        bflyt.materials[idx].name = to.clone();
        Ok(())
    })?;
    println!("ok: renamed '{}' -> '{}' ({} bytes)", args.from, args.to, n);
    Ok(ExitCode::SUCCESS)
}
