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

use image::{DynamicImage, GenericImageView};
use tegra_swizzle::block_height_mip0;
use tegra_swizzle::surface::swizzle_surface;

use super::decode::{block_dim_for, block_height_from_log2, deswizzle_texture};
use super::{AppendTextureSpec, BntxFile, TextureFormat};
use crate::dds::Dds;
use crate::error::{Error, Result};
use crate::texpipe::{
    block_height_to_log2, compress_cube_bc7, compress_image_bc7, compress_image_bc7_with_mips,
    compress_image_to_format, format_is_encodable, Bc7Quality, CompressedTexture,
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

/// Invert a BNTX channel-swizzle: for each *block* channel (0=R, 1=G,
/// 2=B, 3=A) return which *output* (PNG) channel feeds it. The
/// channel-swizzle records, per output channel, where the GPU sources it
/// (2=block.R, 3=block.G, 4=block.B, 5=block.A); we pick the first output
/// channel that references each block channel. Block channels no output
/// references default to the identity index so they pass through
/// harmlessly (e.g. an unreferenced alpha stays the source alpha rather
/// than becoming a zero that would flip BC1 into punch-through mode).
fn invert_channel_swizzle(swizzle: [u8; 4]) -> [usize; 4] {
    let mut inv = [0usize, 1, 2, 3];
    let mut assigned = [false; 4];
    for (out_ch, &src) in swizzle.iter().enumerate() {
        if (2..=5).contains(&src) {
            let block_ch = (src - 2) as usize;
            if !assigned[block_ch] {
                inv[block_ch] = out_ch;
                assigned[block_ch] = true;
            }
        }
    }
    inv
}

/// Remap a source image's channels into the texture's *block* channels so
/// that, after the GPU re-applies `channel_swizzle`, the rendered result
/// matches the source. This is the inverse of the export-side swizzle
/// application, so an `export-png` → `replace-png` round-trip preserves
/// the visible image (e.g. a BC4 alpha mask takes the PNG's alpha).
fn remap_image_for_format(img: &DynamicImage, channel_swizzle: [u8; 4]) -> DynamicImage {
    let inv = invert_channel_swizzle(channel_swizzle);
    if inv == [0, 1, 2, 3] {
        return img.clone();
    }
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let src = rgba.as_raw();
    let mut out = vec![0u8; src.len()];
    for (px_in, px_out) in src.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
        px_out[0] = px_in[inv[0]];
        px_out[1] = px_in[inv[1]];
        px_out[2] = px_in[inv[2]];
        px_out[3] = px_in[inv[3]];
    }
    DynamicImage::ImageRgba8(
        image::RgbaImage::from_raw(w, h, out).expect("remap preserves buffer size"),
    )
}

/// Re-encode a source image (or cube faces) over an existing texture's
/// pixel data in place, **preserving the texture's existing surface
/// format** (BC1/BC3/BC4/BC5/BC7/R8G8B8A8). The replacement must match
/// the existing texture's dimensions, mip count, cube-ness, and resulting
/// byte length so the BNTX structure (string pool, dict, BRTI count,
/// `_RLT`) is left untouched. The format's sRGB-ness is preserved (no
/// gamma flip), and the source channels are remapped through the
/// texture's channel-swizzle so the rendered result matches the source.
///
/// Cube maps are currently re-encoded only when the existing format is
/// BC7. `BC2` and `BC6` are not encodable; use remove + import for a
/// format change to one of those.
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
    let channel_swizzle = bntx.channel_swizzle(&bntx.textures[tex_idx]);

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

    // Tile the re-encode with exactly the texture's stored block height so
    // the swizzled layout matches the slot we're overwriting.
    let block_height_log2 = if (0..=5).contains(&size_range) {
        Some(size_range as u32)
    } else {
        None
    };

    let compressed = match (&source, is_cube) {
        (ReplaceSource::CubeFaces(faces), true) => {
            if !matches!(
                format,
                TextureFormat::Bc7Unorm | TextureFormat::Bc7UnormSrgb
            ) {
                return Err(Error::Texpipe(format!(
                    "cube-map replace currently supports only BC7; texture '{name}' is {}",
                    format.name()
                )));
            }
            compress_cube_bc7(faces, quality, mips as u32)?
        }
        (ReplaceSource::Image(img), false) => {
            if !format_is_encodable(format) {
                return Err(Error::Texpipe(format!(
                    "texture '{name}' has format {} which cannot be re-encoded; \
                     use remove + import for a format change",
                    format.name()
                )));
            }
            // Validate against the texture's *logical* dimensions; the
            // encoder pads up to the block grid internally, so we must
            // not compare the padded `compressed.width` against `width`
            // (e.g. a 5x5 BC1 texture stores width=5 but encodes 2x2
            // blocks == an 8x8 grid).
            let (sw, sh) = img.dimensions();
            if (sw, sh) != (width, height) {
                return Err(Error::Texpipe(format!(
                    "replacement image is {sw}x{sh} but texture '{name}' is {width}x{height}; \
                     replacement requires identical dimensions"
                )));
            }
            let remapped = remap_image_for_format(img, channel_swizzle);
            compress_image_to_format(&remapped, format, quality, mips as u32, block_height_log2)?
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

    // For cube maps the encoder reports the (square) face size; the 2D
    // path already validated source dims against the texture's logical
    // size before encoding.
    if is_cube && (compressed.width != width || compressed.height != height) {
        return Err(Error::Texpipe(format!(
            "replacement {}x{} does not match existing cube texture '{name}' ({width}x{height})",
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

// ============================================================
// DDS interchange (export / import / replace)
// ============================================================

/// Export an existing texture as a [`Dds`] surface: deswizzle its
/// block-linear data into the tightly-packed linear layout DDS expects
/// and pair it with the texture's format/dimensions/mip/array metadata.
pub fn export_texture_dds(bntx: &BntxFile, name: &str) -> Result<Dds> {
    let idx = bntx.texture_index_by_name(name).ok_or_else(|| {
        Error::Bntx(super::BntxError::Format(format!(
            "texture '{name}' not found in BNTX (file has {} texture(s))",
            bntx.textures.len()
        )))
    })?;
    let tex = &bntx.textures[idx];
    let surface = deswizzle_texture(bntx, tex)?;
    Ok(Dds {
        format: tex.format,
        width: tex.width,
        height: tex.height,
        depth: tex.depth.max(1),
        mip_count: (tex.mips_count as u32).max(1),
        array_count: tex.array_len.max(1),
        is_cube: tex.dim == 8,
        data: surface.linear,
    })
}

/// Swizzle a DDS surface's linear bytes into the Tegra block-linear
/// layout, returning `(swizzled, block_height_log2)`. When
/// `block_height_log2` is `Some` it tiles with exactly that block height
/// (used by replace so the layout matches the slot being overwritten);
/// otherwise the canonical block height is inferred and reported.
fn swizzle_dds(dds: &Dds, block_height_log2: Option<u32>) -> Result<(Vec<u8>, u32)> {
    let (_, bh) = dds.format.block_dim();
    let bytes_per_block = dds.format.block_size();
    let block_dim = block_dim_for(dds.format);
    let layers = dds.array_count.max(1);
    let mips = dds.mip_count.max(1);
    let depth = dds.depth.max(1);

    let block_height = match block_height_log2 {
        Some(l) => block_height_from_log2(l as i32),
        None => Some(block_height_mip0(dds.height.div_ceil(bh))),
    };
    let used_log2 = match block_height {
        Some(b) => block_height_to_log2(b),
        // Depth textures force block height 1; report log2 0 to match.
        None => 0,
    };

    let swizzled = swizzle_surface(
        dds.width,
        dds.height,
        depth,
        &dds.data,
        block_dim,
        block_height,
        bytes_per_block,
        mips,
        layers,
    )
    .map_err(|e| Error::Texpipe(format!("Tegra swizzle failed for DDS import: {e:?}")))?;
    Ok((swizzled, used_log2))
}

/// Import a [`Dds`] surface as a **new** named texture (append). The
/// canonical Tegra block height is inferred and recorded. `align`
/// overrides the BRTD data alignment (default 0x200).
pub fn import_dds(bntx: &mut BntxFile, name: &str, dds: &Dds, align: Option<u32>) -> Result<()> {
    let (swizzled, size_range) = swizzle_dds(dds, None)?;

    // Start from the 2D/cube default spec (format-agnostic fields), then
    // pin the actual format/dimensions/layout from the DDS.
    let mut spec = AppendTextureSpec::bc7_2d_with_mips(
        dds.width,
        dds.height,
        dds.mip_count.max(1) as u16,
        size_range as i32,
        swizzled,
        false,
    );
    spec.format = dds.format;
    spec.depth = dds.depth.max(1);
    if dds.is_cube {
        spec.dim = 8;
        spec.array_len = dds.array_count.max(6);
    } else {
        spec.array_len = dds.array_count.max(1);
    }
    if let Some(a) = align {
        spec.align = a;
    }
    bntx.append_texture(name.to_string(), spec)?;
    Ok(())
}

/// Replace an existing texture's pixel data in place from a [`Dds`]
/// surface, preserving the texture's format/dimensions/mip/array and the
/// surrounding BNTX structure. The DDS must match the texture's
/// format, dimensions, mip count, and array/cube layout, and re-tiling
/// must produce exactly the texture's stored byte length.
pub fn replace_with_dds(bntx: &mut BntxFile, name: &str, dds: &Dds) -> Result<()> {
    let idx = bntx.texture_index_by_name(name).ok_or_else(|| {
        Error::Bntx(super::BntxError::Format(format!(
            "texture '{name}' not found in BNTX (file has {} texture(s))",
            bntx.textures.len()
        )))
    })?;
    let (format, width, height, depth, mips, array_len, dim, image_size, data_offset, size_range) = {
        let t = &bntx.textures[idx];
        (
            t.format,
            t.width,
            t.height,
            t.depth.max(1),
            t.mips_count as u32,
            t.array_len.max(1),
            t.dim,
            t.image_size as usize,
            t.data_offset_in_brtd,
            t.size_range,
        )
    };

    if dds.format != format {
        return Err(Error::Texpipe(format!(
            "DDS format {} does not match texture '{name}' format {}",
            dds.format.name(),
            format.name()
        )));
    }
    if (dds.width, dds.height) != (width, height) {
        return Err(Error::Texpipe(format!(
            "DDS is {}x{} but texture '{name}' is {width}x{height}",
            dds.width, dds.height
        )));
    }
    if dds.mip_count.max(1) != mips {
        return Err(Error::Texpipe(format!(
            "DDS has {} mip(s) but texture '{name}' has {mips}",
            dds.mip_count
        )));
    }
    if dds.depth.max(1) != depth || dds.array_count.max(1) != array_len {
        return Err(Error::Texpipe(format!(
            "DDS depth/array ({}/{}) does not match texture '{name}' ({depth}/{array_len})",
            dds.depth.max(1),
            dds.array_count.max(1)
        )));
    }
    let _ = dim;

    let block_height_log2 = if (0..=5).contains(&size_range) {
        Some(size_range as u32)
    } else {
        None
    };
    let (swizzled, _) = swizzle_dds(dds, block_height_log2)?;
    if swizzled.len() != image_size {
        return Err(Error::Texpipe(format!(
            "re-tiled DDS is {} bytes but texture '{name}' slot is {image_size}; \
             cannot splice without a structural change",
            swizzled.len()
        )));
    }
    bntx.brtd.data[data_offset..data_offset + image_size].copy_from_slice(&swizzled);
    Ok(())
}
