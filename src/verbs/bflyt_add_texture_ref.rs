//! `bflyt-add-texture-ref`: add a texture name to BFLYT's txl1 list.
//! Idempotent — if the name already exists, returns its index. Thin
//! wrapper over [`crate::bflyt::BFLYT::add_texture_ref`].

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
        already_existed = bflyt.textures.iter().any(|t| t == &texture_name);
        resulting_index = bflyt.add_texture_ref(&texture_name);
        Ok(())
    })?;
    println!(
        "ok: texture '{}' {} txl1 at index {} ({} bytes)",
        args.name,
        if already_existed {
            "already in"
        } else {
            "added to"
        },
        resulting_index,
        n
    );
    Ok(ExitCode::SUCCESS)
}
