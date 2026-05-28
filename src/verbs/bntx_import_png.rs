//! `bntx-import-png`: encode a PNG to BC7+swizzled, then append it to a
//! BNTX file as a new named texture. Writes the modified BNTX back to
//! disk.
//!
//! Used by SGPO to add custom face-button textures to the game's
//! `__Combined.bntx` while keeping the existing 200+ textures intact.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bntx::{read_bntx, write_bntx, AppendTextureSpec};
use crate::texpipe::{import_png, Bc7Quality};

#[derive(Parser, Debug)]
pub struct Args {
    /// Input BNTX file.
    #[arg(short, long)]
    input: PathBuf,

    /// Output BNTX (defaults to overwriting `input`).
    #[arg(short, long)]
    out: Option<PathBuf>,

    /// PNG (or JPG/BMP) source image.
    #[arg(long)]
    image: PathBuf,

    /// Texture name as it should appear in the BNTX dict (and as
    /// referenced from BFLYT.txl1).
    #[arg(long)]
    name: String,

    /// BC7 encoder quality. Use `slow` for production, `ultra-fast` for
    /// iteration. Defaults to `slow`.
    #[arg(long, default_value = "slow")]
    quality: String,

    /// Encode as BC7_UNORM_SRGB instead of BC7_UNORM.
    #[arg(long)]
    srgb: bool,

    /// Override the BC7 alignment (texture data alignment within the
    /// BRTD block). Defaults to 0x200, sufficient for BC7 textures up to
    /// 256x256. Use 0x1000 for 512x512+.
    #[arg(long)]
    align: Option<u32>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let quality = match args.quality.as_str() {
        "ultra-fast" | "ultrafast" => Bc7Quality::UltraFast,
        "fast" => Bc7Quality::Fast,
        "basic" => Bc7Quality::Basic,
        "slow" => Bc7Quality::Slow,
        other => return Err(anyhow!(
            "unknown --quality {other}; valid: ultra-fast, fast, basic, slow"
        )),
    };

    let bntx_bytes = fs::read(&args.input)
        .with_context(|| format!("reading {}", args.input.display()))?;
    let mut bntx = read_bntx(&bntx_bytes).map_err(|e| anyhow::anyhow!("{}", e))?;

    let compressed = import_png(&args.image, quality)
        .with_context(|| format!("encoding {}", args.image.display()))?;

    let mut spec = AppendTextureSpec::bc7_2d_default(
        compressed.width,
        compressed.height,
        compressed.block_height_log2 as i32,
        compressed.swizzled_data,
        args.srgb,
    );
    if let Some(a) = args.align {
        spec.align = a;
    }

    bntx.append_texture(args.name.clone(), spec)
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
        "ok: appended texture '{}' ({}x{} BC7{}, {} bytes swizzled), file is now {} bytes",
        args.name,
        compressed.width,
        compressed.height,
        if args.srgb { "_SRGB" } else { "" },
        compressed.image_size,
        written.len(),
    );
    Ok(ExitCode::SUCCESS)
}
