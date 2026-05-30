//! PNG/image -> BC7 -> Tegra X1 swizzled bytes pipeline.
//!
//! Used by `bntx-import-png` and downstream verbs to convert a flat
//! source image into the swizzled BC7 layout that BNTX texture data
//! blocks expect.

use crate::bntx::TextureFormat;
use crate::error::{Error, Result};
use image::{DynamicImage, GenericImageView};
use intel_tex_2::{bc1, bc3, bc4, bc5, bc7, RSurface, RgSurface, RgbaSurface};
use tegra_swizzle::surface::{swizzle_surface, BlockDim};
use tegra_swizzle::{block_height_mip0, BlockHeight};

/// Settings for the BC7 encoder. We use `alpha_slow_settings` by default;
/// it produces the highest-quality output at the cost of encoding time
/// (acceptable for the small UI textures SGPO targets).
#[derive(Debug, Clone, Copy)]
pub enum Bc7Quality {
    UltraFast,
    Fast,
    Basic,
    Slow,
}

impl Bc7Quality {
    fn settings(self, has_alpha: bool) -> bc7::EncodeSettings {
        match (self, has_alpha) {
            (Bc7Quality::UltraFast, false) => bc7::opaque_ultra_fast_settings(),
            (Bc7Quality::Fast, false) => bc7::opaque_fast_settings(),
            (Bc7Quality::Basic, false) => bc7::opaque_basic_settings(),
            (Bc7Quality::Slow, false) => bc7::opaque_slow_settings(),
            (Bc7Quality::UltraFast, true) => bc7::alpha_ultra_fast_settings(),
            (Bc7Quality::Fast, true) => bc7::alpha_fast_settings(),
            (Bc7Quality::Basic, true) => bc7::alpha_basic_settings(),
            (Bc7Quality::Slow, true) => bc7::alpha_slow_settings(),
        }
    }
}

impl std::str::FromStr for Bc7Quality {
    type Err = String;

    /// Parse a `--quality` CLI value. Accepts `ultra-fast`/`ultrafast`,
    /// `fast`, `basic`, and `slow`.
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "ultra-fast" | "ultrafast" => Bc7Quality::UltraFast,
            "fast" => Bc7Quality::Fast,
            "basic" => Bc7Quality::Basic,
            "slow" => Bc7Quality::Slow,
            other => {
                return Err(format!(
                    "unknown quality '{other}'; valid: ultra-fast, fast, basic, slow"
                ))
            }
        })
    }
}

/// Convert tegra_swizzle's `BlockHeight` into the log2 value BNTX records
/// in the BRTI `size_range` field.
pub(crate) fn block_height_to_log2(bh: BlockHeight) -> u32 {
    match bh {
        BlockHeight::One => 0,
        BlockHeight::Two => 1,
        BlockHeight::Four => 2,
        BlockHeight::Eight => 3,
        BlockHeight::Sixteen => 4,
        BlockHeight::ThirtyTwo => 5,
    }
}

/// Result of compressing an image: the BC7-swizzled bytes ready to write
/// into a BNTX BRTD section, plus the metadata BNTX needs to record.
#[derive(Debug, Clone)]
pub struct CompressedTexture {
    pub width: u32,
    pub height: u32,
    pub mip_count: u32,
    pub array_count: u32,
    /// Swizzled BC7 bytes ready to splice into the BRTD block.
    pub swizzled_data: Vec<u8>,
    /// Block-height-log2 actually used for swizzling (caller wants this
    /// to write back into the BRTI metadata).
    pub block_height_log2: u32,
    /// Total size in bytes of the BC7 blocks before swizzling, summed
    /// over all mip / array levels.
    pub image_size: u32,
}

/// Compute the smallest power-of-two ≥ 1 mip-chain length for a texture
/// of the given dimensions. Each mip halves the dimension (rounded down)
/// until both are 1×1.
pub fn natural_mip_count(width: u32, height: u32) -> u32 {
    let mut count = 1u32;
    let mut w = width;
    let mut h = height;
    while w > 1 || h > 1 {
        w = (w / 2).max(1);
        h = (h / 2).max(1);
        count += 1;
    }
    count
}

/// Compress an image to swizzled BC7 ready for BNTX.
///
/// BC7 requires a 4x4 block grid; if the image dimensions aren't a
/// multiple of 4 we transparent-pad the right and bottom edges to the
/// next multiple. The compressed texture's `width`/`height` will reflect
/// the padded dimensions, not the source.
pub fn compress_image_bc7(image: &DynamicImage, quality: Bc7Quality) -> Result<CompressedTexture> {
    let (src_w, src_h) = image.dimensions();
    let width = (src_w + 3) & !3;
    let height = (src_h + 3) & !3;

    let rgba = image.to_rgba8();
    let src_raw = rgba.as_raw();

    // Build a (width × height × 4) RGBA buffer, copying the source and
    // zero-padding the right/bottom strips when needed.
    let raw_owned: Vec<u8>;
    let raw: &[u8] = if (src_w, src_h) == (width, height) {
        src_raw
    } else {
        let mut buf = vec![0u8; (width * height * 4) as usize];
        for y in 0..src_h {
            let src_row_start = (y * src_w * 4) as usize;
            let dst_row_start = (y * width * 4) as usize;
            buf[dst_row_start..dst_row_start + (src_w * 4) as usize]
                .copy_from_slice(&src_raw[src_row_start..src_row_start + (src_w * 4) as usize]);
        }
        raw_owned = buf;
        &raw_owned
    };

    // Determine if the image has any non-fully-opaque pixels; this picks
    // between the "opaque" and "alpha" BC7 settings.
    let has_alpha = raw.chunks_exact(4).any(|px| px[3] != 0xFF);

    let surface = RgbaSurface {
        width,
        height,
        stride: width * 4,
        data: raw,
    };

    // intel_tex_2 expects an output buffer sized to bc7::calc_output_size(w, h).
    let output_size = bc7::calc_output_size(width, height) as usize;
    let mut bc7_blocks = vec![0u8; output_size];
    bc7::compress_blocks_into(&quality.settings(has_alpha), &surface, &mut bc7_blocks);

    // BC7 has a 4x4 block size and 16 bytes/block. Swizzle into Tegra
    // block-linear layout.
    let bytes_per_block: u32 = 16;
    let block_dim = BlockDim::block_4x4();

    let swizzled = swizzle_surface(
        width,
        height,
        1, // depth
        &bc7_blocks,
        block_dim,
        None, // infer block_height
        bytes_per_block,
        1, // mip count
        1, // layer count
    )
    .map_err(|e| Error::Texpipe(format!("Tegra swizzle failed: {e:?}")))?;

    // The swizzler picks `block_height` internally when we pass `None`;
    // BNTX records the value as a log2 in BRTI's `size_range` field. Use
    // tegra_swizzle's canonical helper so we always agree with what the
    // swizzler chose. `block_height_mip0` takes height in BLOCKS, so we
    // divide by the format's block dim.
    let height_in_blocks = height / 4;
    let block_height_log2 = block_height_to_log2(block_height_mip0(height_in_blocks));

    Ok(CompressedTexture {
        width,
        height,
        mip_count: 1,
        array_count: 1,
        swizzled_data: swizzled,
        block_height_log2,
        image_size: bc7_blocks.len() as u32, // pre-swizzle linear size matches BRTI image_size in real files
    })
}

/// Open a PNG/JPG/BMP image and run the BC7+swizzle pipeline.
pub fn import_png(path: &std::path::Path, quality: Bc7Quality) -> Result<CompressedTexture> {
    let img = image::open(path)
        .map_err(|e| Error::Texpipe(format!("opening {}: {e}", path.display())))?;
    compress_image_bc7(&img, quality)
}

/// Like `compress_image_bc7` but generates a full mip chain. Each mip
/// is the previous level's image scaled down by half (rounded toward
/// zero, with min size 1) using a Lanczos3 filter. All mips are BC7-
/// encoded, then the entire chain is Tegra-swizzled as a multi-mip
/// surface.
pub fn compress_image_bc7_with_mips(
    image: &DynamicImage,
    quality: Bc7Quality,
    mip_count: u32,
) -> Result<CompressedTexture> {
    if mip_count == 0 {
        return Err(Error::Texpipe("mip_count must be >= 1".into()));
    }
    let (src_w, src_h) = image.dimensions();
    let width = (src_w + 3) & !3;
    let height = (src_h + 3) & !3;
    if mip_count == 1 {
        return compress_image_bc7(image, quality);
    }

    // Generate each mip's RGBA buffer, padded to its own 4×4-aligned
    // dimensions, then BC7-encode each one and concatenate the linear
    // BC7 blocks before passing through the swizzler.
    let mut linear_blocks: Vec<u8> = Vec::new();
    let mut has_alpha = false;
    let base_rgba = image.to_rgba8();
    let base_raw = base_rgba.as_raw();
    if base_raw.chunks_exact(4).any(|px| px[3] != 0xFF) {
        has_alpha = true;
    }

    for level in 0..mip_count {
        let lw = (width >> level).max(4);
        let lh = (height >> level).max(4);
        let lvl_w = (lw + 3) & !3;
        let lvl_h = (lh + 3) & !3;

        // Resize from the source image with Lanczos3 to lvl_w × lvl_h.
        let resized = if level == 0 {
            // Pad source to (width, height) without resizing.
            let mut buf = vec![0u8; (width * height * 4) as usize];
            for y in 0..src_h {
                let src_off = (y * src_w * 4) as usize;
                let dst_off = (y * width * 4) as usize;
                buf[dst_off..dst_off + (src_w * 4) as usize]
                    .copy_from_slice(&base_raw[src_off..src_off + (src_w * 4) as usize]);
            }
            buf
        } else {
            let mip_img = image
                .resize_exact(lvl_w, lvl_h, image::imageops::FilterType::Lanczos3)
                .to_rgba8();
            mip_img.into_raw()
        };

        let surface = RgbaSurface {
            width: lvl_w,
            height: lvl_h,
            stride: lvl_w * 4,
            data: &resized,
        };
        let block_bytes = bc7::calc_output_size(lvl_w, lvl_h) as usize;
        let mut mip_blocks = vec![0u8; block_bytes];
        bc7::compress_blocks_into(&quality.settings(has_alpha), &surface, &mut mip_blocks);
        linear_blocks.extend_from_slice(&mip_blocks);
    }

    let block_dim = BlockDim::block_4x4();
    let bytes_per_block: u32 = 16;
    let swizzled = swizzle_surface(
        width,
        height,
        1,
        &linear_blocks,
        block_dim,
        None,
        bytes_per_block,
        mip_count,
        1,
    )
    .map_err(|e| Error::Texpipe(format!("Tegra swizzle failed (mip): {e:?}")))?;

    let block_height_log2 = block_height_to_log2(block_height_mip0(height / 4));

    Ok(CompressedTexture {
        width,
        height,
        mip_count,
        array_count: 1,
        swizzled_data: swizzled,
        block_height_log2,
        image_size: linear_blocks.len() as u32,
    })
}

/// Encode a cube map from 6 face PNGs (in `+X, -X, +Y, -Y, +Z, -Z`
/// order). All faces must share dimensions, which must be square and a
/// multiple of 4. The result includes any mip chain for each face.
pub fn compress_cube_bc7(
    face_paths: &[std::path::PathBuf; 6],
    quality: Bc7Quality,
    mip_count: u32,
) -> Result<CompressedTexture> {
    if mip_count == 0 {
        return Err(Error::Texpipe("mip_count must be >= 1".into()));
    }
    let mut faces = Vec::with_capacity(6);
    for p in face_paths.iter() {
        faces.push(
            image::open(p).map_err(|e| Error::Texpipe(format!("opening {}: {e}", p.display())))?,
        );
    }
    let (w0, h0) = faces[0].dimensions();
    if w0 != h0 {
        return Err(Error::Texpipe(format!(
            "cube-map face 0 is {w0}x{h0}; cube faces must be square"
        )));
    }
    if w0 % 4 != 0 {
        return Err(Error::Texpipe(format!(
            "cube-map face dimension {w0} is not a multiple of 4 (required for BC7)"
        )));
    }
    for (i, f) in faces.iter().enumerate() {
        let (w, h) = f.dimensions();
        if (w, h) != (w0, h0) {
            return Err(Error::Texpipe(format!(
                "cube-map face {i} is {w}x{h}; expected {w0}x{h0}"
            )));
        }
    }
    let size = w0;

    // Concatenate per-face mip-encoded BC7 blocks: face0 mips, face1 mips, ...
    let mut linear_blocks: Vec<u8> = Vec::new();
    let mut has_alpha = false;
    for face in &faces {
        let raw = face.to_rgba8();
        if raw.as_raw().chunks_exact(4).any(|p| p[3] != 0xFF) {
            has_alpha = true;
        }
        for level in 0..mip_count {
            let lvl = (size >> level).max(4);
            let resized = if level == 0 {
                raw.as_raw().clone()
            } else {
                face.resize_exact(lvl, lvl, image::imageops::FilterType::Lanczos3)
                    .to_rgba8()
                    .into_raw()
            };
            let surface = RgbaSurface {
                width: lvl,
                height: lvl,
                stride: lvl * 4,
                data: &resized,
            };
            let block_bytes = bc7::calc_output_size(lvl, lvl) as usize;
            let mut mip_blocks = vec![0u8; block_bytes];
            bc7::compress_blocks_into(&quality.settings(has_alpha), &surface, &mut mip_blocks);
            linear_blocks.extend_from_slice(&mip_blocks);
        }
    }

    let block_dim = BlockDim::block_4x4();
    let bytes_per_block: u32 = 16;
    let swizzled = swizzle_surface(
        size,
        size,
        1,
        &linear_blocks,
        block_dim,
        None,
        bytes_per_block,
        mip_count,
        6, // 6 cube faces
    )
    .map_err(|e| Error::Texpipe(format!("Tegra swizzle failed (cube): {e:?}")))?;

    let block_height_log2 = block_height_to_log2(block_height_mip0(size / 4));

    Ok(CompressedTexture {
        width: size,
        height: size,
        mip_count,
        array_count: 6,
        swizzled_data: swizzled,
        block_height_log2,
        image_size: linear_blocks.len() as u32,
    })
}

/// Whether `format` can be encoded by [`compress_image_to_format`].
/// `BC2` has no encoder in `intel_tex_2`, and `BC6` (HDR) cannot be
/// produced faithfully from an 8-bit source image.
pub fn format_is_encodable(format: TextureFormat) -> bool {
    matches!(
        format,
        TextureFormat::Bc1Unorm
            | TextureFormat::Bc1UnormSrgb
            | TextureFormat::Bc3Unorm
            | TextureFormat::Bc3UnormSrgb
            | TextureFormat::Bc4Unorm
            | TextureFormat::Bc4Snorm
            | TextureFormat::Bc5Unorm
            | TextureFormat::Bc5Snorm
            | TextureFormat::Bc7Unorm
            | TextureFormat::Bc7UnormSrgb
            | TextureFormat::R8G8B8A8Unorm
            | TextureFormat::R8G8B8A8UnormSrgb
    )
}

fn round_up(value: u32, multiple: u32) -> u32 {
    let m = multiple.max(1);
    value.div_ceil(m) * m
}

/// Encode one already-padded RGBA mip level to the linear block bytes for
/// `format`. The input `rgba` is `width * height * 4` bytes whose R,G,B,A
/// are the *block* channels (the caller is responsible for any
/// channel remapping). `width`/`height` must be multiples of the format's
/// block dimensions.
fn encode_mip_blocks(
    format: TextureFormat,
    rgba: &[u8],
    width: u32,
    height: u32,
    quality: Bc7Quality,
    has_alpha: bool,
) -> Result<Vec<u8>> {
    let rgba_surface = RgbaSurface {
        width,
        height,
        stride: width * 4,
        data: rgba,
    };
    Ok(match format {
        TextureFormat::Bc1Unorm | TextureFormat::Bc1UnormSrgb => {
            bc1::compress_blocks(&rgba_surface)
        }
        TextureFormat::Bc3Unorm | TextureFormat::Bc3UnormSrgb => {
            bc3::compress_blocks(&rgba_surface)
        }
        TextureFormat::Bc7Unorm | TextureFormat::Bc7UnormSrgb => {
            let mut out = vec![0u8; bc7::calc_output_size(width, height)];
            bc7::compress_blocks_into(&quality.settings(has_alpha), &rgba_surface, &mut out);
            out
        }
        TextureFormat::Bc4Unorm | TextureFormat::Bc4Snorm => {
            // BC4 is a single (red) channel.
            let r: Vec<u8> = rgba.iter().step_by(4).copied().collect();
            bc4::compress_blocks(&RSurface {
                width,
                height,
                stride: width,
                data: &r,
            })
        }
        TextureFormat::Bc5Unorm | TextureFormat::Bc5Snorm => {
            // BC5 is two (red, green) channels.
            let rg: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1]]).collect();
            bc5::compress_blocks(&RgSurface {
                width,
                height,
                stride: width * 2,
                data: &rg,
            })
        }
        TextureFormat::R8G8B8A8Unorm | TextureFormat::R8G8B8A8UnormSrgb => rgba.to_vec(),
        TextureFormat::Bc2Unorm | TextureFormat::Bc2UnormSrgb => {
            return Err(Error::Texpipe(
                "BC2 has no encoder in intel_tex_2; use remove + import or the DDS path".into(),
            ))
        }
        TextureFormat::Bc6UFloat | TextureFormat::Bc6Float => {
            return Err(Error::Texpipe(
                "BC6 (HDR) cannot be encoded from an 8-bit source; use the DDS path".into(),
            ))
        }
    })
}

/// Encode `image` to swizzled bytes for an arbitrary BNTX surface format,
/// generating `mip_count` mip levels. The image's channels are taken to
/// be the block channels already (callers re-encoding over an existing
/// texture should remap through the texture's channel-swizzle first).
///
/// When `block_height_log2` is `Some`, the surface is tiled with exactly
/// that Tegra block height (used by in-place replacement so the swizzled
/// layout matches the texture being overwritten); `None` infers it.
pub fn compress_image_to_format(
    image: &DynamicImage,
    format: TextureFormat,
    quality: Bc7Quality,
    mip_count: u32,
    block_height_log2: Option<u32>,
) -> Result<CompressedTexture> {
    if mip_count == 0 {
        return Err(Error::Texpipe("mip_count must be >= 1".into()));
    }
    if !format_is_encodable(format) {
        return Err(Error::Texpipe(format!(
            "format {} is not encodable by compress_image_to_format",
            format.name()
        )));
    }

    let (bw, bh) = format.block_dim();
    let (src_w, src_h) = image.dimensions();
    let width = round_up(src_w, bw);
    let height = round_up(src_h, bh);

    let base_rgba = image.to_rgba8();
    let has_alpha = base_rgba.as_raw().chunks_exact(4).any(|p| p[3] != 0xFF);

    let mut linear_blocks: Vec<u8> = Vec::new();
    for level in 0..mip_count {
        let lvl_w = round_up((width >> level).max(bw), bw);
        let lvl_h = round_up((height >> level).max(bh), bh);

        let resized: Vec<u8> = if level == 0 && (src_w, src_h) == (width, height) {
            base_rgba.as_raw().clone()
        } else if level == 0 {
            // Pad the source up to the block-aligned base dimensions.
            let mut buf = vec![0u8; (width * height * 4) as usize];
            let src = base_rgba.as_raw();
            for y in 0..src_h {
                let s = (y * src_w * 4) as usize;
                let d = (y * width * 4) as usize;
                buf[d..d + (src_w * 4) as usize].copy_from_slice(&src[s..s + (src_w * 4) as usize]);
            }
            buf
        } else {
            image
                .resize_exact(lvl_w, lvl_h, image::imageops::FilterType::Lanczos3)
                .to_rgba8()
                .into_raw()
        };

        linear_blocks.extend_from_slice(&encode_mip_blocks(
            format, &resized, lvl_w, lvl_h, quality, has_alpha,
        )?);
    }

    let block_dim = if matches!(
        format,
        TextureFormat::R8G8B8A8Unorm | TextureFormat::R8G8B8A8UnormSrgb
    ) {
        BlockDim::uncompressed()
    } else {
        BlockDim::block_4x4()
    };
    let bytes_per_block = format.block_size();
    let block_height = block_height_log2.and_then(|l| crate::bntx::decode::block_height_from_log2(l as i32));

    let swizzled = swizzle_surface(
        width,
        height,
        1,
        &linear_blocks,
        block_dim,
        block_height,
        bytes_per_block,
        mip_count,
        1,
    )
    .map_err(|e| Error::Texpipe(format!("Tegra swizzle failed ({}): {e:?}", format.name())))?;

    let used_log2 = block_height_log2
        .unwrap_or_else(|| block_height_to_log2(block_height_mip0(height / bh)));

    Ok(CompressedTexture {
        width,
        height,
        mip_count,
        array_count: 1,
        swizzled_data: swizzled,
        block_height_log2: used_log2,
        image_size: linear_blocks.len() as u32,
    })
}
