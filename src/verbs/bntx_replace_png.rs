//! `bntx-replace-png`: re-encode a PNG into BC7+swizzle and splice it over
//! an existing texture's pixel data in place (no structural change, so the
//! `_RLT` is preserved verbatim). Thin wrapper over
//! [`crate::bntx::pipeline::replace_texture`].

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bntx::pipeline::{replace_texture, ReplaceSource};
use crate::bntx::{read_bntx, write_bntx};
use crate::texpipe::Bc7Quality;

#[derive(Parser, Debug)]
pub struct Args {
    /// Input BNTX file.
    #[arg(short, long)]
    input: PathBuf,

    /// Output BNTX (defaults to overwriting `input`).
    #[arg(short, long)]
    out: Option<PathBuf>,

    /// PNG (or JPG/BMP) source for a 2D texture replacement.
    #[arg(long, conflicts_with = "cube_faces")]
    image: Option<PathBuf>,

    /// Six face images (in `+X, -X, +Y, -Y, +Z, -Z` order) for replacing a
    /// cube-map texture. Mutually exclusive with `--image`.
    #[arg(long, num_args = 6, conflicts_with = "image")]
    cube_faces: Vec<PathBuf>,

    /// Name of the texture to replace. Must already exist in the BNTX.
    #[arg(long)]
    name: String,

    /// BC7 encoder quality. Defaults to `slow`.
    #[arg(long, default_value = "slow")]
    quality: String,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let quality: Bc7Quality = args.quality.parse().map_err(|e| anyhow!("{e}"))?;
    let bntx_bytes =
        fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let mut bntx = read_bntx(&bntx_bytes).map_err(|e| anyhow!("{e}"))?;

    if !args.cube_faces.is_empty() {
        let faces: [PathBuf; 6] = args
            .cube_faces
            .clone()
            .try_into()
            .map_err(|v: Vec<PathBuf>| anyhow!("expected exactly 6 cube faces; got {}", v.len()))?;
        replace_texture(
            &mut bntx,
            &args.name,
            ReplaceSource::CubeFaces(&faces),
            quality,
        )?;
    } else {
        let path = args
            .image
            .as_ref()
            .ok_or_else(|| anyhow!("must pass --image for a 2D texture replacement"))?;
        let img = image::open(path).with_context(|| format!("opening {}", path.display()))?;
        replace_texture(&mut bntx, &args.name, ReplaceSource::Image(&img), quality)?;
    }

    let written = write_bntx(&bntx).map_err(|e| anyhow!("{e}"))?;
    let out_path = args.out.as_ref().unwrap_or(&args.input);
    crate::verbs::write_output(out_path, &written)?;
    println!(
        "ok: replaced texture '{}', file is now {} bytes",
        args.name,
        written.len()
    );
    Ok(ExitCode::SUCCESS)
}
