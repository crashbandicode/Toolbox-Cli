//! `bflyt-add-texture-ref`: add a texture name to BFLYT's txl1 list.
//! Idempotent — if the name already exists, returns its index.

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

    /// Texture name to add (typically `tex_<pane_name>`).
    #[arg(long)]
    name: String,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let texture_name = args.name.clone();
    let mut resulting_index = 0usize;
    let mut already_existed = false;
    let n = rewrite_bflyt(&args.input, args.out.as_deref(), |bflyt| {
        if let Some(i) = bflyt.textures.iter().position(|t| t == &texture_name) {
            resulting_index = i;
            already_existed = true;
        } else {
            resulting_index = bflyt.textures.len();
            bflyt.textures.push(texture_name.clone());
        }
        Ok(())
    })?;
    if already_existed {
        println!(
            "ok: texture '{}' already in txl1 at index {} ({} bytes)",
            args.name, resulting_index, n
        );
    } else {
        println!(
            "ok: added texture '{}' to txl1 at index {} ({} bytes)",
            args.name, resulting_index, n
        );
    }
    Ok(ExitCode::SUCCESS)
}
