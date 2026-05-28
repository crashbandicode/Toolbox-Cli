//! `bntx-replace-png`: re-encode a PNG into BC7+swizzled bytes and splice
//! them over an existing texture's pixel data without changing the BNTX
//! structure (string pool, dict, BRTI count, RLT layout).
//!
//! The replacement source must produce the same swizzled byte length as
//! the existing texture; in practice this means matching width, height,
//! mip count, array layer count, and BC7 family. Anything else would
//! shift subsequent textures' offsets and force an RLT regeneration —
//! which is what the future `bntx-remove-texture` + `bntx-import-png`
//! pair will be for.
//!
//! Used by SGPO when a user re-renders one button skin and wants to
//! refresh just that texture in-place. Because the file structure is
//! preserved, the original `_RLT` is emitted verbatim — no canonical
//! rebuild, no risk of bumping into the C#-Switch-Toolbox compatibility
//! gap.
//!
//! Round-trip invariant: the output differs from the input only in the
//! BRTD bytes covering the replaced texture (and possibly the BRTI
//! `size_range` field, which the writer resets if the encoder picked a
//! different value — which it shouldn't, given matched dimensions).

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bntx::{read_bntx, write_bntx, TextureFormat};
use crate::texpipe::{
    compress_cube_bc7, compress_image_bc7, compress_image_bc7_with_mips, Bc7Quality,
};

#[derive(Parser, Debug)]
pub struct Args {
    /// Input BNTX file.
    #[arg(short, long)]
    input: PathBuf,

    /// Output BNTX (defaults to overwriting `input`).
    #[arg(short, long)]
    out: Option<PathBuf>,

    /// PNG (or JPG/BMP) source for a 2D texture replacement. Mutually
    /// exclusive with `--cube-faces`.
    #[arg(long, conflicts_with = "cube_faces")]
    image: Option<PathBuf>,

    /// Six face images (in `+X, -X, +Y, -Y, +Z, -Z` order) for replacing
    /// a cube-map texture. Mutually exclusive with `--image`.
    #[arg(long, num_args = 6, conflicts_with = "image")]
    cube_faces: Vec<PathBuf>,

    /// Name of the texture to replace. Must already exist in the BNTX
    /// dict. Use `bntx-inspect` to list available names.
    #[arg(long)]
    name: String,

    /// BC7 encoder quality. Use `slow` for production, `ultra-fast` for
    /// iteration. Defaults to `slow`.
    #[arg(long, default_value = "slow")]
    quality: String,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let quality = match args.quality.as_str() {
        "ultra-fast" | "ultrafast" => Bc7Quality::UltraFast,
        "fast" => Bc7Quality::Fast,
        "basic" => Bc7Quality::Basic,
        "slow" => Bc7Quality::Slow,
        other => {
            return Err(anyhow!(
                "unknown --quality {other}; valid: ultra-fast, fast, basic, slow"
            ));
        }
    };

    let bntx_bytes = fs::read(&args.input)
        .with_context(|| format!("reading {}", args.input.display()))?;
    let mut bntx = read_bntx(&bntx_bytes).map_err(|e| anyhow::anyhow!("{}", e))?;

    let tex_idx = bntx.texture_index_by_name(&args.name).ok_or_else(|| {
        anyhow!(
            "texture '{}' not found in BNTX (file has {} texture(s))",
            args.name,
            bntx.textures.len()
        )
    })?;

    // Snapshot the existing texture's invariants. We validate the new
    // encode against these *before* writing anything so a mismatched
    // source aborts cleanly without leaving a partially-mutated `bntx`.
    let exp = {
        let t = &bntx.textures[tex_idx];
        ExpectedShape {
            width: t.width,
            height: t.height,
            mips: t.mips_count,
            array_len: t.array_len,
            dim: t.dim,
            format: t.format,
            image_size: t.image_size as usize,
            data_offset: t.data_offset_in_brtd,
            size_range: t.size_range,
        }
    };

    // 2 = 2D, 8 = cube. We don't currently encode 3D / texture-array
    // surfaces, so reject anything else with an explicit error rather
    // than silently producing a bad BRTI.
    let is_cube = match exp.dim {
        2 => false,
        8 => true,
        other => {
            return Err(anyhow!(
                "texture '{}' has dim={} (only 2 = 2D and 8 = cube are supported for replacement)",
                args.name,
                other
            ));
        }
    };

    // Format must be in the BC7 family (the only family our texpipe
    // encodes to). We preserve sRGB-ness from the existing texture so
    // that swapping a button skin doesn't accidentally change gamma.
    let srgb = match exp.format {
        TextureFormat::Bc7Unorm => false,
        TextureFormat::Bc7UnormSrgb => true,
        other => {
            return Err(anyhow!(
                "texture '{}' has format {} but bntx-replace-png only re-encodes to BC7. \
                 Use bntx-remove-texture + bntx-import-png if a format change is intended.",
                args.name,
                other.name()
            ));
        }
    };

    // Match the cube/2D selection to the on-disk texture before we open
    // any source files — saves a confusing image-open failure when the
    // user picked the wrong flag.
    if is_cube {
        if args.cube_faces.len() != 6 {
            return Err(anyhow!(
                "texture '{}' is a cube map; pass exactly 6 paths via --cube-faces (got {})",
                args.name,
                args.cube_faces.len()
            ));
        }
    } else if !args.cube_faces.is_empty() {
        return Err(anyhow!(
            "texture '{}' is a 2D texture; use --image, not --cube-faces",
            args.name
        ));
    } else if args.image.is_none() {
        return Err(anyhow!("must pass --image for a 2D texture replacement"));
    }

    let compressed = if is_cube {
        let face_arr: [PathBuf; 6] = [
            args.cube_faces[0].clone(),
            args.cube_faces[1].clone(),
            args.cube_faces[2].clone(),
            args.cube_faces[3].clone(),
            args.cube_faces[4].clone(),
            args.cube_faces[5].clone(),
        ];
        compress_cube_bc7(&face_arr, quality, exp.mips as u32)?
    } else {
        let path = args.image.as_ref().expect("checked above");
        let img = image::open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        if exp.mips > 1 {
            compress_image_bc7_with_mips(&img, quality, exp.mips as u32)?
        } else {
            compress_image_bc7(&img, quality)?
        }
    };

    // Hard-validate every shape invariant the BRTD splice depends on. A
    // dimension or mip-count mismatch means the new bytes don't fit the
    // existing slot, which would shift downstream texture offsets — i.e.
    // exactly what this verb promises NOT to do. The error message
    // points to the structural-change pair (`bntx-remove-texture` +
    // `bntx-import-png`) for that workflow.
    if compressed.width != exp.width || compressed.height != exp.height {
        return Err(anyhow!(
            "replacement source dimensions {}x{} do not match existing texture '{}' ({}x{}); \
             replacement requires identical layout. Use bntx-remove-texture + bntx-import-png \
             to swap shape.",
            compressed.width,
            compressed.height,
            args.name,
            exp.width,
            exp.height
        ));
    }
    if is_cube && compressed.array_count != exp.array_len {
        return Err(anyhow!(
            "replacement source array_count={} does not match existing cube texture '{}' ({})",
            compressed.array_count,
            args.name,
            exp.array_len
        ));
    }
    if compressed.mip_count != exp.mips as u32 {
        return Err(anyhow!(
            "replacement source mip_count={} does not match existing texture '{}' ({})",
            compressed.mip_count,
            args.name,
            exp.mips
        ));
    }
    if compressed.swizzled_data.len() != exp.image_size {
        // Should be impossible if the dimensions/mips/cube-ness all
        // match (BC7 swizzle is a byte-permutation, not a re-encoding),
        // but check defensively so the failure is loud rather than
        // silent corruption of subsequent textures' data.
        return Err(anyhow!(
            "internal: swizzled byte count for replacement ({}) != existing texture '{}' \
             image_size ({}); please file a bug with a repro fixture",
            compressed.swizzled_data.len(),
            args.name,
            exp.image_size
        ));
    }
    if compressed.block_height_log2 as i32 != exp.size_range {
        // tegra_swizzle's `block_height_mip0` is deterministic on
        // height-in-blocks, so the only way this fires is if the
        // existing BRTI was hand-encoded with a non-canonical
        // block_height. Surface that loudly rather than overwriting it.
        return Err(anyhow!(
            "replacement block_height_log2={} does not match existing texture '{}' size_range={}; \
             the source BNTX uses a non-canonical block_height that this verb won't overwrite",
            compressed.block_height_log2,
            args.name,
            exp.size_range
        ));
    }

    // Splice the new bytes into BRTD at the existing texture's offset.
    // BRTI fields (image_size, size_range, width, height, mips,
    // array_len, format) all stay the same because we validated the
    // new encode against them. The file's structural offsets and RLT
    // are therefore unchanged — `relocation_table_dirty` stays false,
    // so the original RLT is emitted verbatim.
    bntx.brtd.data[exp.data_offset..exp.data_offset + exp.image_size]
        .copy_from_slice(&compressed.swizzled_data);

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
        "ok: replaced texture '{}' ({}x{} {}{}, {} mips, {} bytes swizzled), file is now {} bytes",
        args.name,
        compressed.width,
        compressed.height,
        kind,
        if srgb { "_SRGB" } else { "" },
        compressed.mip_count,
        compressed.image_size,
        written.len(),
    );
    Ok(ExitCode::SUCCESS)
}

/// Snapshot of the to-be-replaced texture's shape, captured before any
/// mutation so we can validate the new encode against it without
/// re-borrowing `bntx.textures`.
struct ExpectedShape {
    width: u32,
    height: u32,
    mips: u16,
    array_len: u32,
    dim: u8,
    format: TextureFormat,
    image_size: usize,
    data_offset: usize,
    size_range: i32,
}
