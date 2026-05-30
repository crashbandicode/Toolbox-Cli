//! `bntx-export-all`: deswizzle + decode every texture in a BNTX to PNGs
//! in an output directory. Each file is named `<texture-name>.png`.

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

    /// Output directory (created if missing). One PNG per texture.
    #[arg(short, long)]
    out_dir: PathBuf,

    /// Mip level to export for every texture (default 0).
    #[arg(long, default_value_t = 0)]
    mip: u32,

    /// Array layer / cube face to export for every texture (default 0).
    #[arg(long, default_value_t = 0)]
    layer: u32,

    /// Export the decoder's natural channels without applying each
    /// texture's channel-swizzle.
    #[arg(long)]
    raw: bool,

    /// Continue past textures that fail to decode (report them at the
    /// end) instead of aborting on the first error.
    #[arg(long)]
    keep_going: bool,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bytes =
        fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let bntx = read_bntx(&bytes).map_err(|e| anyhow!("{e}"))?;

    fs::create_dir_all(&args.out_dir)
        .with_context(|| format!("creating {}", args.out_dir.display()))?;

    let mut exported = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for tex_index in 0..bntx.textures.len() {
        let name = bntx.textures[tex_index].name(&bntx).to_string();
        let file_name = format!("{}.png", sanitize_file_stem(&name));
        let out_path = args.out_dir.join(&file_name);

        match export_one(&bntx, tex_index, args.mip, args.layer, !args.raw, &out_path) {
            Ok(()) => exported += 1,
            Err(e) => {
                let msg = format!("'{name}': {e}");
                if args.keep_going {
                    failures.push(msg);
                } else {
                    return Err(anyhow!("exporting {msg}"));
                }
            }
        }
    }

    println!(
        "ok: exported {}/{} texture(s) to {}",
        exported,
        bntx.textures.len(),
        args.out_dir.display()
    );
    if !failures.is_empty() {
        eprintln!("{} texture(s) failed:", failures.len());
        for f in &failures {
            eprintln!("  {f}");
        }
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

fn export_one(
    bntx: &crate::bntx::BntxFile,
    tex_index: usize,
    mip: u32,
    layer: u32,
    apply_swizzle: bool,
    out_path: &std::path::Path,
) -> Result<()> {
    let img = decode_texture_image(bntx, tex_index, mip, layer, apply_swizzle)?;
    let buffer = image::RgbaImage::from_raw(img.width, img.height, img.rgba)
        .ok_or_else(|| anyhow!("decoded RGBA buffer size mismatch"))?;
    buffer
        .save(out_path)
        .with_context(|| format!("writing {}", out_path.display()))?;
    Ok(())
}

/// Replace characters that are awkward in filenames. BNTX texture names
/// are normally plain identifiers, but guard against embedded path
/// separators just in case.
fn sanitize_file_stem(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect()
}
