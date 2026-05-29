//! `bflyt-add-material`: clone an existing material under a new name,
//! optionally rebinding its first texture map. Thin wrapper over
//! [`crate::bflyt::BFLYT::add_material_from_template`].

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

    /// Existing material to clone (e.g. an SGPO marker template).
    #[arg(long)]
    template: String,

    /// Name for the new material. Must be unique and ≤ 28 bytes.
    #[arg(long)]
    name: String,

    /// Optional texture name to bind to the new material's first texture
    /// map. The texture must already exist in BFLYT txl1.
    #[arg(long)]
    bind_texture: Option<String>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let template = args.template.clone();
    let new_name = args.name.clone();
    let bind_tex = args.bind_texture.clone();
    let n = rewrite_bflyt(&args.input, args.out.as_deref(), |bflyt| {
        bflyt.add_material_from_template(&template, &new_name, bind_tex.as_deref())?;
        Ok(())
    })?;
    println!(
        "ok: added material '{}' (cloned from '{}'){} ({} bytes)",
        args.name,
        args.template,
        match &args.bind_texture {
            Some(t) => format!(" bound to texture '{t}'"),
            None => String::new(),
        },
        n,
    );
    Ok(ExitCode::SUCCESS)
}
