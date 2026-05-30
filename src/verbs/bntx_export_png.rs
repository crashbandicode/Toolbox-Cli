//! `bntx-export-png`: deswizzle + decode one named texture from a BNTX and
//! write it out as a PNG. Honors the texture's channel-swizzle so the
//! exported image matches what the GPU samples (use `--raw` to bypass it
//! and see the decoder's natural channels).

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bntx::decode::decode_texture_image;
use crate::bntx::read_bntx;

#[derive(Parser, Debug)]
pub struct Args {
    /// Input BNTX file.
    #[arg(short, long)]
    input: PathBuf,

    /// Name of the texture to export.
    #[arg(long)]
    name: String,

    /// Output PNG path.
    #[arg(short, long)]
    out: PathBuf,

    /// Mip level to export (default 0 = full resolution).
    #[arg(long, default_value_t = 0)]
    mip: u32,

    /// Array layer / cube face to export (default 0).
    #[arg(long, default_value_t = 0)]
    layer: u32,

    /// Export the decoder's natural channels without applying the
    /// texture's channel-swizzle.
    #[arg(long)]
    raw: bool,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bytes =
        fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let bntx = read_bntx(&bytes).map_err(|e| anyhow!("{e}"))?;

    let tex_index = bntx
        .texture_index_by_name(&args.name)
        .ok_or_else(|| anyhow!("texture '{}' not found in {}", args.name, args.input.display()))?;

    let img = decode_texture_image(&bntx, tex_index, args.mip, args.layer, !args.raw)?;
    let buffer = image::RgbaImage::from_raw(img.width, img.height, img.rgba)
        .ok_or_else(|| anyhow!("internal: decoded RGBA buffer size mismatch"))?;

    if let Some(parent) = args.out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    buffer
        .save(&args.out)
        .with_context(|| format!("writing {}", args.out.display()))?;

    println!(
        "ok: exported '{}' ({}x{}, mip {}, layer {}) -> {}",
        args.name,
        img.width,
        img.height,
        args.mip,
        args.layer,
        args.out.display()
    );
    Ok(ExitCode::SUCCESS)
}
