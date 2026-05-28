//! PNG/image -> BC7 -> Tegra X1 swizzled bytes pipeline.
//!
//! Used by `bntx-import-png` and downstream verbs to convert a flat
//! source image into the swizzled BC7 layout that BNTX texture data
//! blocks expect.

use anyhow::{anyhow, Context, Result};
use image::{DynamicImage, GenericImageView};
use intel_tex_2::{bc7, RgbaSurface};
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
    /// Same as `swizzled_data.len()` — convenient pre-computed value
    /// since BNTX records `image_size` as a u32.
    pub image_size: u32,
    /// Total bytes the BC7 BLOCKS take before swizzling. Used for
    /// validation only.
    pub linear_size: u32,
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
pub fn compress_image_bc7(
    image: &DynamicImage,
    quality: Bc7Quality,
) -> Result<CompressedTexture> {
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
    bc7::compress_blocks_into(
        &quality.settings(has_alpha),
        &surface,
        &mut bc7_blocks,
    );

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
    .map_err(|e| anyhow!("Tegra swizzle failed: {e:?}"))?;

    // The swizzler picks `block_height` internally when we pass `None`;
    // BNTX records the value as a log2 in BRTI's `size_range` field. Use
    // tegra_swizzle's canonical helper so we always agree with what the
    // swizzler chose. `block_height_mip0` takes height in BLOCKS, so we
    // divide by the format's block dim.
    let height_in_blocks = height / 4;
    let block_height: BlockHeight = block_height_mip0(height_in_blocks);
    let block_height_log2 = match block_height {
        BlockHeight::One => 0,
        BlockHeight::Two => 1,
        BlockHeight::Four => 2,
        BlockHeight::Eight => 3,
        BlockHeight::Sixteen => 4,
        BlockHeight::ThirtyTwo => 5,
    };

    Ok(CompressedTexture {
        width,
        height,
        mip_count: 1,
        array_count: 1,
        swizzled_data: swizzled,
        block_height_log2,
        image_size: bc7_blocks.len() as u32, // pre-swizzle linear size matches BRTI image_size in real files
        linear_size: bc7_blocks.len() as u32,
    })
}

/// Open a PNG/JPG/BMP image and run the BC7+swizzle pipeline.
pub fn import_png(path: &std::path::Path, quality: Bc7Quality) -> Result<CompressedTexture> {
    let img = image::open(path).with_context(|| format!("opening {}", path.display()))?;
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
        return Err(anyhow!("mip_count must be >= 1"));
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
    .map_err(|e| anyhow!("Tegra swizzle failed (mip): {e:?}"))?;

    let block_height_log2 = match block_height_mip0(height / 4) {
        BlockHeight::One => 0,
        BlockHeight::Two => 1,
        BlockHeight::Four => 2,
        BlockHeight::Eight => 3,
        BlockHeight::Sixteen => 4,
        BlockHeight::ThirtyTwo => 5,
    };

    Ok(CompressedTexture {
        width,
        height,
        mip_count,
        array_count: 1,
        swizzled_data: swizzled,
        block_height_log2,
        image_size: linear_blocks.len() as u32,
        linear_size: linear_blocks.len() as u32,
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
        return Err(anyhow!("mip_count must be >= 1"));
    }
    let mut faces = Vec::with_capacity(6);
    for p in face_paths.iter() {
        faces.push(image::open(p).with_context(|| format!("opening {}", p.display()))?);
    }
    let (w0, h0) = faces[0].dimensions();
    if w0 != h0 {
        return Err(anyhow!(
            "cube-map face 0 is {}x{}; cube faces must be square",
            w0, h0
        ));
    }
    if w0 % 4 != 0 {
        return Err(anyhow!(
            "cube-map face dimension {} is not a multiple of 4 (required for BC7)",
            w0
        ));
    }
    for (i, f) in faces.iter().enumerate() {
        let (w, h) = f.dimensions();
        if (w, h) != (w0, h0) {
            return Err(anyhow!(
                "cube-map face {} is {}x{}; expected {}x{}",
                i, w, h, w0, h0
            ));
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
    .map_err(|e| anyhow!("Tegra swizzle failed (cube): {e:?}"))?;

    let block_height_log2 = match block_height_mip0(size / 4) {
        BlockHeight::One => 0,
        BlockHeight::Two => 1,
        BlockHeight::Four => 2,
        BlockHeight::Eight => 3,
        BlockHeight::Sixteen => 4,
        BlockHeight::ThirtyTwo => 5,
    };

    Ok(CompressedTexture {
        width: size,
        height: size,
        mip_count,
        array_count: 6,
        swizzled_data: swizzled,
        block_height_log2,
        image_size: linear_blocks.len() as u32,
        linear_size: linear_blocks.len() as u32,
    })
}

