//! PNG -> BC7 -> Tegra-swizzled bytes pipeline.
//!
//! Currently a stub. The pipeline composes:
//!   `image::open` -> RGBA8 buffer -> `intel_tex_2::bc7::compress_blocks`
//!     -> `tegra_swizzle::swizzle::swizzle_block_linear` -> append to BNTX.
//!
//! Wiring this up requires a working `bntx::write_bntx`, which is the
//! current critical-path TODO. Until then, callers should fall back to
//! the C# CLI for BNTX import.

use anyhow::{anyhow, Result};

pub fn import_png_to_bntx(_bntx: &mut crate::bntx::BntxFile, _image_path: &str, _texture_name: &str)
    -> Result<()>
{
    Err(anyhow!(
        "Rust BNTX texture import is not yet wired up. The format reader \
         is implemented; writing will land in a follow-up commit. Use the \
         upstream Switch Toolbox or the C# Toolbox-Cli for now."
    ))
}
