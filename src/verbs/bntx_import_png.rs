//! `bntx-import-png`: encode a PNG to BC7 + swizzle and append it to a BNTX
//! as a new named texture. Thin wrapper over [`crate::bntx::pipeline`].

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bntx::pipeline::{self, ImportOptions};
use crate::bntx::{read_bntx, write_bntx};
use crate::texpipe::{natural_mip_count, Bc7Quality};

#[derive(Parser, Debug)]
pub struct Args {
    /// Input BNTX file.
    #[arg(short, long)]
    input: PathBuf,

    /// Output BNTX (defaults to overwriting `input`).
    #[arg(short, long)]
    out: Option<PathBuf>,

    /// PNG (or JPG/BMP) source image. For a cube map use `--cube-faces`.
    #[arg(long, conflicts_with = "cube_faces")]
    image: Option<PathBuf>,

    /// Six face images (in `+X, -X, +Y, -Y, +Z, -Z` order) for cube-map
    /// imports. Mutually exclusive with `--image`.
    #[arg(long, num_args = 6, conflicts_with = "image")]
    cube_faces: Vec<PathBuf>,

    /// Texture name as it should appear in the BNTX dict.
    #[arg(long)]
    name: String,

    /// BC7 encoder quality. Defaults to `slow`.
    #[arg(long, default_value = "slow")]
    quality: String,

    /// Encode as BC7_UNORM_SRGB instead of BC7_UNORM.
    #[arg(long)]
    srgb: bool,

    /// Override the BC7 alignment within BRTD. Defaults to 0x200. Accepts
    /// decimal or `0x...` hex.
    #[arg(long, value_parser = parse_u32_dec_or_hex)]
    align: Option<u32>,

    /// Number of mip levels. `1` = single-mip (default). `auto` = full
    /// chain. Or an explicit count.
    #[arg(long, default_value = "1")]
    mips: String,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let quality: Bc7Quality = args.quality.parse().map_err(|e| anyhow!("{e}"))?;
    let bntx_bytes =
        fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let mut bntx = read_bntx(&bntx_bytes).map_err(|e| anyhow!("{e}"))?;

    if !args.cube_faces.is_empty() {
        let faces: [PathBuf; 6] =
            args.cube_faces
                .clone()
                .try_into()
                .map_err(|v: Vec<PathBuf>| {
                    anyhow!("--cube-faces requires exactly 6 paths; got {}", v.len())
                })?;
        let first =
            image::open(&faces[0]).with_context(|| format!("opening {}", faces[0].display()))?;
        let (w, _) = image::GenericImageView::dimensions(&first);
        let mip_count = parse_mip_count(&args.mips, w, w)?;
        let opts = ImportOptions {
            quality,
            srgb: args.srgb,
            align: args.align,
            mip_count,
        };
        pipeline::import_cube_png_files(&mut bntx, &args.name, &faces, &opts)?;
    } else {
        let path = args
            .image
            .as_ref()
            .ok_or_else(|| anyhow!("must pass --image or --cube-faces"))?;
        let img = image::open(path).with_context(|| format!("opening {}", path.display()))?;
        let (w, h) = image::GenericImageView::dimensions(&img);
        let mip_count = parse_mip_count(&args.mips, w, h)?;
        let opts = ImportOptions {
            quality,
            srgb: args.srgb,
            align: args.align,
            mip_count,
        };
        pipeline::import_image(&mut bntx, &args.name, &img, &opts)?;
    }

    let written = write_bntx(&bntx).map_err(|e| anyhow!("{e}"))?;
    let out_path = args.out.as_ref().unwrap_or(&args.input);
    crate::verbs::write_output(out_path, &written)?;
    println!(
        "ok: appended texture '{}', file is now {} bytes",
        args.name,
        written.len()
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
fn parse_u32_dec_or_hex(s: &str) -> std::result::Result<u32, String> {
    if let Some(stripped) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(stripped, 16).map_err(|e| format!("not a hex number: {e}"))
    } else {
        s.parse::<u32>().map_err(|e| format!("not a number: {e}"))
    }
}
