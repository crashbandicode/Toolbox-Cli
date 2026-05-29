//! High-level BNTX texture import/replace helpers.
//!
//! These tie the [`crate::texpipe`] PNG -> BC7 -> Tegra-swizzle pipeline to
//! the low-level [`BntxFile`] append/splice operations:
//!
//! - [`import_image`] / [`import_png_file`] / [`import_cube_png_files`]
//!   encode a source image and **append** it as a new named texture.
//! - [`replace_texture`] re-encodes a source over an **existing** texture's
//!   data in place (no structural change, so the `_RLT` is preserved).

use std::path::{Path, PathBuf};

use image::DynamicImage;

use super::{AppendTextureSpec, BntxFile, TextureFormat};
use crate::error::{Error, Result};
use crate::texpipe::{
    compress_cube_bc7, compress_image_bc7, compress_image_bc7_with_mips, Bc7Quality,
    CompressedTexture,
};

/// Options for the import helpers.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// BC7 encoder quality.
    pub quality: Bc7Quality,
    /// Encode as `BC7_UNORM_SRGB` (vs `BC7_UNORM`).
    pub srgb: bool,
    /// Override the texture-data alignment within BRTD (default 0x200).
    pub align: Option<u32>,
    /// Number of mip levels (1 = single mip). Use
    /// [`crate::texpipe::natural_mip_count`] to compute a full chain.
    pub mip_count: u32,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            quality: Bc7Quality::Slow,
            srgb: false,
            align: None,
            mip_count: 1,
        }
    }
}

/// Source for [`replace_texture`].
pub enum ReplaceSource<'a> {
    /// A single 2D image.
    Image(&'a DynamicImage),
    /// Six cube-map face image paths (`+X, -X, +Y, -Y, +Z, -Z`).
    CubeFaces(&'a [PathBuf; 6]),
}

fn append_2d(
    bntx: &mut BntxFile,
    name: &str,
    c: CompressedTexture,
    srgb: bool,
    align: Option<u32>,
) -> Result<()> {
    let mut spec = if c.mip_count > 1 {
        AppendTextureSpec::bc7_2d_with_mips(
            c.width,
            c.height,
            c.mip_count as u16,
            c.block_height_log2 as i32,
            c.swizzled_data,
            srgb,
        )
    } else {
        AppendTextureSpec::bc7_2d_default(
            c.width,
            c.height,
            c.block_height_log2 as i32,
            c.swizzled_data,
            srgb,
        )
    };
    if let Some(a) = align {
        spec.align = a;
    }
    bntx.append_texture(name.to_string(), spec)?;
    Ok(())
}

/// Encode a 2D image and append it to the BNTX as a new named texture.
pub fn import_image(
    bntx: &mut BntxFile,
    name: &str,
    img: &DynamicImage,
    opts: &ImportOptions,
) -> Result<()> {
    let compressed = if opts.mip_count > 1 {
        compress_image_bc7_with_mips(img, opts.quality, opts.mip_count)?
    } else {
        compress_image_bc7(img, opts.quality)?
    };
    append_2d(bntx, name, compressed, opts.srgb, opts.align)
}

/// Open a PNG/JPG/BMP file and [`import_image`] it.
pub fn import_png_file(
    bntx: &mut BntxFile,
    name: &str,
    path: &Path,
    opts: &ImportOptions,
) -> Result<()> {
    let img = image::open(path)
        .map_err(|e| Error::Texpipe(format!("opening {}: {e}", path.display())))?;
    import_image(bntx, name, &img, opts)
}

/// Encode six cube-map faces (`+X, -X, +Y, -Y, +Z, -Z`) and append the
/// resulting cube texture to the BNTX as a new named texture.
pub fn import_cube_png_files(
    bntx: &mut BntxFile,
    name: &str,
    faces: &[PathBuf; 6],
    opts: &ImportOptions,
) -> Result<()> {
    let c = compress_cube_bc7(faces, opts.quality, opts.mip_count)?;
    let mut spec = AppendTextureSpec::bc7_cube_default(
        c.width,
        c.mip_count as u16,
        c.block_height_log2 as i32,
        c.swizzled_data,
        opts.srgb,
    );
    if let Some(a) = opts.align {
        spec.align = a;
    }
    bntx.append_texture(name.to_string(), spec)?;
    Ok(())
}

/// Re-encode a source image (or cube faces) over an existing texture's
/// pixel data in place. The replacement must match the existing texture's
/// dimensions, mip count, cube-ness, and BC7 family so the BNTX structure
/// (string pool, dict, BRTI count, `_RLT`) is left untouched. sRGB-ness is
/// preserved from the existing texture.
pub fn replace_texture(
    bntx: &mut BntxFile,
    name: &str,
    source: ReplaceSource,
    quality: Bc7Quality,
) -> Result<()> {
    let tex_idx = bntx.texture_index_by_name(name).ok_or_else(|| {
        Error::Bntx(super::BntxError::Format(format!(
            "texture '{name}' not found in BNTX (file has {} texture(s))",
            bntx.textures.len()
        )))
    })?;

    let (width, height, mips, array_len, dim, format, image_size, data_offset, size_range) = {
        let t = &bntx.textures[tex_idx];
        (
            t.width,
            t.height,
            t.mips_count,
            t.array_len,
            t.dim,
            t.format,
            t.image_size as usize,
            t.data_offset_in_brtd,
            t.size_range,
        )
    };

    // 2 = 2D, 8 = cube. Anything else we don't encode.
    let is_cube = match dim {
        2 => false,
        8 => true,
        other => {
            return Err(Error::Texpipe(format!(
                "texture '{name}' has dim={other} (only 2 = 2D and 8 = cube are supported)"
            )))
        }
    };
    if !matches!(
        format,
        TextureFormat::Bc7Unorm | TextureFormat::Bc7UnormSrgb
    ) {
        return Err(Error::Texpipe(format!(
            "texture '{name}' has format {} but replace only re-encodes to BC7; \
             use remove + import for a format change",
            format.name()
        )));
    }

    let compressed = match (&source, is_cube) {
        (ReplaceSource::CubeFaces(faces), true) => compress_cube_bc7(faces, quality, mips as u32)?,
        (ReplaceSource::Image(img), false) => {
            if mips > 1 {
                compress_image_bc7_with_mips(img, quality, mips as u32)?
            } else {
                compress_image_bc7(img, quality)?
            }
        }
        (ReplaceSource::Image(_), true) => {
            return Err(Error::Texpipe(format!(
                "texture '{name}' is a cube map; provide ReplaceSource::CubeFaces"
            )))
        }
        (ReplaceSource::CubeFaces(_), false) => {
            return Err(Error::Texpipe(format!(
                "texture '{name}' is 2D; provide ReplaceSource::Image"
            )))
        }
    };

    if compressed.width != width || compressed.height != height {
        return Err(Error::Texpipe(format!(
            "replacement {}x{} does not match existing texture '{name}' ({width}x{height}); \
             replacement requires identical layout",
            compressed.width, compressed.height
        )));
    }
    if is_cube && compressed.array_count != array_len {
        return Err(Error::Texpipe(format!(
            "replacement array_count={} does not match existing cube texture '{name}' ({array_len})",
            compressed.array_count
        )));
    }
    if compressed.mip_count != mips as u32 {
        return Err(Error::Texpipe(format!(
            "replacement mip_count={} does not match existing texture '{name}' ({mips})",
            compressed.mip_count
        )));
    }
    if compressed.swizzled_data.len() != image_size {
        return Err(Error::Texpipe(format!(
            "internal: swizzled byte count ({}) != existing texture '{name}' image_size ({image_size})",
            compressed.swizzled_data.len()
        )));
    }
    if compressed.block_height_log2 as i32 != size_range {
        return Err(Error::Texpipe(format!(
            "replacement block_height_log2={} does not match existing texture '{name}' size_range={size_range}",
            compressed.block_height_log2
        )));
    }

    bntx.brtd.data[data_offset..data_offset + image_size]
        .copy_from_slice(&compressed.swizzled_data);
    Ok(())
}
