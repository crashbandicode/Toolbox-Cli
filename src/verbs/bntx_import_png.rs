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
use crate::texpipe::{
    compress_cube_bc7, compress_image_bc7, compress_image_bc7_with_mips, natural_mip_count,
    Bc7Quality,
};

#[derive(Parser, Debug)]
pub struct Args {
    /// Input BNTX file.
    #[arg(short, long)]
    input: PathBuf,

    /// Output BNTX (defaults to overwriting `input`).
    #[arg(short, long)]
    out: Option<PathBuf>,

    /// PNG (or JPG/BMP) source image. For a cube map, repeat 6 times in
    /// `+X, -X, +Y, -Y, +Z, -Z` order via `--cube-faces` instead.
    #[arg(long, conflicts_with = "cube_faces")]
    image: Option<PathBuf>,

    /// Six face images (in `+X, -X, +Y, -Y, +Z, -Z` order) for cube-map
    /// imports. Mutually exclusive with `--image`.
    #[arg(long, num_args = 6, conflicts_with = "image")]
    cube_faces: Vec<PathBuf>,

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
    /// 256x256. Use 0x1000 for 512x512+. Accepts decimal or `0x...` hex.
    #[arg(long, value_parser = parse_u32_dec_or_hex)]
    align: Option<u32>,

    /// Number of mip levels. `1` = single-mip (default for face icons).
    /// `auto` = full chain down to 1×1. Or pass an explicit count.
    #[arg(long, default_value = "1")]
    mips: String,
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

    let (compressed, is_cube) = if !args.cube_faces.is_empty() {
        if args.cube_faces.len() != 6 {
            return Err(anyhow!(
                "--cube-faces requires exactly 6 paths; got {}",
                args.cube_faces.len()
            ));
        }
        let face_arr: [PathBuf; 6] = [
            args.cube_faces[0].clone(),
            args.cube_faces[1].clone(),
            args.cube_faces[2].clone(),
            args.cube_faces[3].clone(),
            args.cube_faces[4].clone(),
            args.cube_faces[5].clone(),
        ];
        // Determine cube mip count from the first face.
        let first = image::open(&face_arr[0])
            .with_context(|| format!("opening {}", face_arr[0].display()))?;
        let (w, _) = image::GenericImageView::dimensions(&first);
        let mip_count = parse_mip_count(&args.mips, w, w)?;
        let c = compress_cube_bc7(&face_arr, quality, mip_count)?;
        (c, true)
    } else {
        let path = args.image.as_ref().ok_or_else(|| {
            anyhow!("must pass --image or --cube-faces")
        })?;
        let img = image::open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        let (w, h) = image::GenericImageView::dimensions(&img);
        let mip_count = parse_mip_count(&args.mips, w, h)?;
        let c = if mip_count == 1 {
            compress_image_bc7(&img, quality)?
        } else {
            compress_image_bc7_with_mips(&img, quality, mip_count)?
        };
        (c, false)
    };

    let mut spec = if is_cube {
        AppendTextureSpec::bc7_cube_default(
            compressed.width,
            compressed.mip_count as u16,
            compressed.block_height_log2 as i32,
            compressed.swizzled_data,
            args.srgb,
        )
    } else if compressed.mip_count > 1 {
        AppendTextureSpec::bc7_2d_with_mips(
            compressed.width,
            compressed.height,
            compressed.mip_count as u16,
            compressed.block_height_log2 as i32,
            compressed.swizzled_data,
            args.srgb,
        )
    } else {
        AppendTextureSpec::bc7_2d_default(
            compressed.width,
            compressed.height,
            compressed.block_height_log2 as i32,
            compressed.swizzled_data,
            args.srgb,
        )
    };
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

    let kind = if is_cube { "BC7-cube" } else { "BC7" };
    println!(
        "ok: appended texture '{}' ({}x{} {}{}, {} mips, {} bytes swizzled), file is now {} bytes",
        args.name,
        compressed.width,
        compressed.height,
        kind,
        if args.srgb { "_SRGB" } else { "" },
        compressed.mip_count,
        compressed.image_size,
        written.len(),
    );
    Ok(ExitCode::SUCCESS)
}

fn parse_mip_count(s: &str, width: u32, height: u32) -> Result<u32> {
    if s.eq_ignore_ascii_case("auto") {
        return Ok(natural_mip_count(width, height));
    }
    s.parse::<u32>()
        .map_err(|_| anyhow!("--mips must be a positive integer or 'auto', got '{}'", s))
}

/// Accept either `0x123` (hex) or `123` (decimal) for u32 CLI flags.
/// clap's default u32 parser only handles decimal.
fn parse_u32_dec_or_hex(s: &str) -> Result<u32, String> {
    if let Some(stripped) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(stripped, 16).map_err(|e| format!("not a hex number: {e}"))
    } else {
        s.parse::<u32>().map_err(|e| format!("not a number: {e}"))
    }
}
