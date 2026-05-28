//! PNG/image -> BC7 -> Tegra X1 swizzled bytes pipeline.
//!
//! Used by `bntx-import-png` and downstream verbs to convert a flat
//! source image into the swizzled BC7 layout that BNTX texture data
//! blocks expect.

use anyhow::{anyhow, Context, Result};
use image::{DynamicImage, GenericImageView};
use intel_tex_2::{bc7, RgbaSurface};
use tegra_swizzle::surface::{swizzle_surface, BlockDim};

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

    // Recover the block_height the swizzler chose. `tegra_swizzle` uses
    // a heuristic based on dimensions; the canonical value the BNTX
    // BRTI records is log2(block_height_blocks). For a 256x256 image the
    // swizzler typically picks block_height=16, log2=4.
    // We compute the same way `tegra_swizzle` does internally.
    let block_height_log2 = block_height_log2_for(height / 4);

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

/// Mirror `tegra_swizzle::surface::block_height_mip0` for our metadata
/// bookkeeping. The block height is the highest power-of-two such that
/// the resulting tile size doesn't exceed Tegra's GOB constraints.
fn block_height_log2_for(height_in_blocks: u32) -> u32 {
    // From tegra_swizzle: max block_height = 16, capped by smallest power-of-two
    // >= ceil(height_in_blocks / 8).
    let mut bh = 16u32;
    while bh > 1 && height_in_blocks * 8 <= bh * 8 {
        bh /= 2;
    }
    // Clamp to [1, 16] then take log2.
    let bh = bh.clamp(1, 16);
    (bh as f32).log2() as u32
}
