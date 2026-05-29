//! `mat-rename`: change a material's name in-place. Thin wrapper over
//! [`crate::bflyt::BFLYT::rename_material`].

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::verbs::bflyt_helpers::rewrite_bflyt;

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

    /// New material name. Must be unique within the BFLYT and ≤ 28 bytes.
    #[arg(long)]
    to: String,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let from = args.from.clone();
    let to = args.to.clone();
    let n = rewrite_bflyt(&args.input, args.out.as_deref(), |bflyt| {
        bflyt.rename_material(&from, &to)?;
        Ok(())
    })?;
    println!("ok: renamed '{}' -> '{}' ({} bytes)", args.from, args.to, n);
    Ok(ExitCode::SUCCESS)
}
