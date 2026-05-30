//! BNTX texture -> linear surface -> RGBA decode pipeline.
//!
//! This is the inverse of [`crate::texpipe`]: it takes a parsed
//! [`BntxFile`] texture, **deswizzles** its block-linear (Tegra X1) data
//! back to a tightly-packed linear surface, **decodes** the block-
//! compressed bytes to RGBA, and applies the texture's stored
//! channel-swizzle so the exported pixels match what the GPU samples.
//!
//! Every surface format the parser accepts (`BC1`-`BC7` plus
//! `R8G8B8A8`) is handled. Decoding uses the pure-Rust, MIT/Apache
//! `texture2ddecoder` crate; deswizzling uses `tegra_swizzle`.

use tegra_swizzle::surface::{deswizzle_surface, BlockDim};
use tegra_swizzle::BlockHeight;

use super::{BntxFile, Texture, TextureFormat};
use crate::error::{Error, Result};

/// A fully-decoded RGBA8 image for one mip level of one array layer.
#[derive(Debug, Clone)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// Tightly-packed RGBA8 pixels (`width * height * 4` bytes).
    pub rgba: Vec<u8>,
}

/// The deswizzled (linear, tightly-packed) surface for a texture: all
/// mip levels of all array layers, in `layer0 mip0, layer0 mip1, ...`
/// order. This is the layout DDS files expect, so [`crate::dds`] can
/// write it out directly.
#[derive(Debug, Clone)]
pub struct DeswizzledSurface {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub mip_count: u32,
    pub layer_count: u32,
    pub format: TextureFormat,
    /// Linear surface bytes.
    pub linear: Vec<u8>,
}

/// One of the six BNTX channel-swizzle sources (the per-texture
/// `channel_swizzle` packs four of these, one per output channel).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChannelSource {
    Zero,
    One,
    Red,
    Green,
    Blue,
    Alpha,
}

impl ChannelSource {
    fn from_byte(b: u8) -> ChannelSource {
        match b {
            0 => ChannelSource::Zero,
            1 => ChannelSource::One,
            2 => ChannelSource::Red,
            3 => ChannelSource::Green,
            4 => ChannelSource::Blue,
            // 5 = Alpha; anything unexpected falls back to a passthrough
            // so we never panic on a hand-crafted file.
            _ => ChannelSource::Alpha,
        }
    }
}

/// Map the BNTX `size_range` (block-height-log2) to `tegra_swizzle`'s
/// `BlockHeight`. Returns [`None`] for out-of-range values so the caller
/// can fall back to inference.
pub(crate) fn block_height_from_log2(log2: i32) -> Option<BlockHeight> {
    Some(match log2 {
        0 => BlockHeight::One,
        1 => BlockHeight::Two,
        2 => BlockHeight::Four,
        3 => BlockHeight::Eight,
        4 => BlockHeight::Sixteen,
        5 => BlockHeight::ThirtyTwo,
        _ => return None,
    })
}

/// `BlockDim` for a surface format: 4x4 for the BCn families, 1x1 for the
/// uncompressed `R8G8B8A8` formats.
pub(crate) fn block_dim_for(format: TextureFormat) -> BlockDim {
    match format {
        TextureFormat::R8G8B8A8Unorm | TextureFormat::R8G8B8A8UnormSrgb => BlockDim::uncompressed(),
        _ => BlockDim::block_4x4(),
    }
}

/// Number of blocks needed to cover `value` pixels given a block edge of
/// `align` pixels.
fn div_round_up(value: u32, align: u32) -> u32 {
    value.div_ceil(align.max(1))
}

/// Per-mip linear size in bytes (one array layer), in the same tightly-
/// packed layout `tegra_swizzle::surface::deswizzle_surface` emits.
fn mip_linear_size(format: TextureFormat, width: u32, height: u32, mip: u32) -> usize {
    let (bw, bh) = format.block_dim();
    let bpp = format.block_size();
    let mip_w = (width >> mip).max(1);
    let mip_h = (height >> mip).max(1);
    let blocks_w = div_round_up(mip_w, bw);
    let blocks_h = div_round_up(mip_h, bh);
    (blocks_w * blocks_h * bpp) as usize
}

/// Total linear size of one array layer (sum over all mips).
fn layer_linear_size(format: TextureFormat, width: u32, height: u32, mip_count: u32) -> usize {
    (0..mip_count)
        .map(|m| mip_linear_size(format, width, height, m))
        .sum()
}

/// Deswizzle a texture's block-linear data into a tightly-packed linear
/// surface (all mips of all layers). The texture's stored block height
/// (`size_range`) drives the deswizzle so it exactly inverts the
/// on-disk tiling.
pub fn deswizzle_texture(file: &BntxFile, tex: &Texture) -> Result<DeswizzledSurface> {
    let width = tex.width;
    let height = tex.height;
    let depth = tex.depth.max(1);
    let mip_count = (tex.mips_count as u32).max(1);
    let layer_count = tex.array_len.max(1);
    let format = tex.format;
    let block_dim = block_dim_for(format);
    let bytes_per_pixel = format.block_size();

    let block_height = block_height_from_log2(tex.size_range);

    // The texture's bytes live in BRTD starting at its data offset; pass
    // the slice through to the end of the block (deswizzle reads only the
    // swizzled-surface size it needs, so a longer tail is harmless).
    let start = tex.data_offset_in_brtd;
    if start > file.brtd.data.len() {
        return Err(Error::Bntx(super::BntxError::Format(format!(
            "texture data offset 0x{start:x} is past the BRTD block ({} bytes)",
            file.brtd.data.len()
        ))));
    }
    let swizzled = &file.brtd.data[start..];

    let linear = deswizzle_surface(
        width,
        height,
        depth,
        swizzled,
        block_dim,
        block_height,
        bytes_per_pixel,
        mip_count,
        layer_count,
    )
    .map_err(|e| Error::Texpipe(format!("deswizzle failed for '{}': {e:?}", tex.name(file))))?;

    Ok(DeswizzledSurface {
        width,
        height,
        depth,
        mip_count,
        layer_count,
        format,
        linear,
    })
}

/// Decode one mip level of one layer of a texture to RGBA8, applying the
/// texture's channel-swizzle (unless `apply_swizzle` is false, in which
/// case the raw decoded channels are returned). `mip` and `layer` are
/// zero-based.
pub fn decode_texture_image(
    file: &BntxFile,
    tex_index: usize,
    mip: u32,
    layer: u32,
    apply_swizzle: bool,
) -> Result<DecodedImage> {
    let tex = file
        .textures
        .get(tex_index)
        .ok_or_else(|| Error::Other(format!("texture index {tex_index} out of range")))?;

    let mip_count = (tex.mips_count as u32).max(1);
    let layer_count = tex.array_len.max(1);
    if mip >= mip_count {
        return Err(Error::Other(format!(
            "mip {mip} out of range (texture '{}' has {mip_count} mip(s))",
            tex.name(file)
        )));
    }
    if layer >= layer_count {
        return Err(Error::Other(format!(
            "layer {layer} out of range (texture '{}' has {layer_count} layer(s))",
            tex.name(file)
        )));
    }

    let surface = deswizzle_texture(file, tex)?;
    let format = tex.format;
    let layer_size = layer_linear_size(format, tex.width, tex.height, mip_count);

    // Offset of (layer, mip) inside the tightly-packed linear surface.
    let mut offset = layer as usize * layer_size;
    for m in 0..mip {
        offset += mip_linear_size(format, tex.width, tex.height, m);
    }
    let size = mip_linear_size(format, tex.width, tex.height, mip);
    if offset + size > surface.linear.len() {
        return Err(Error::Texpipe(format!(
            "decoded surface for '{}' is {} bytes; mip {mip}/layer {layer} wants [{offset}..{}]",
            tex.name(file),
            surface.linear.len(),
            offset + size
        )));
    }
    let slice = &surface.linear[offset..offset + size];

    let mip_w = (tex.width >> mip).max(1);
    let mip_h = (tex.height >> mip).max(1);

    let natural = decode_block_to_rgba(format, slice, mip_w, mip_h)?;
    let rgba = if apply_swizzle {
        apply_channel_swizzle(&natural, file.channel_swizzle(tex))
    } else {
        natural
    };

    Ok(DecodedImage {
        width: mip_w,
        height: mip_h,
        rgba,
    })
}

/// Decode a single mip's worth of linear block data to RGBA8 in the
/// decoder's natural channel order (R,G,B,A). `width`/`height` are in
/// pixels.
pub fn decode_block_to_rgba(
    format: TextureFormat,
    linear: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    let w = width as usize;
    let h = height as usize;
    let pixel_count = w * h;

    // Uncompressed formats are already RGBA8 in the linear buffer.
    if matches!(
        format,
        TextureFormat::R8G8B8A8Unorm | TextureFormat::R8G8B8A8UnormSrgb
    ) {
        let need = pixel_count * 4;
        if linear.len() < need {
            return Err(Error::Texpipe(format!(
                "R8G8B8A8 surface is {} bytes; need {need} for {width}x{height}",
                linear.len()
            )));
        }
        return Ok(linear[..need].to_vec());
    }

    // Block-compressed formats: decode via texture2ddecoder into a packed
    // BGRA u32 buffer, then unpack to R,G,B,A bytes.
    let mut packed = vec![0u32; pixel_count];
    decode_bcn(format, linear, w, h, &mut packed)?;

    let mut rgba = vec![0u8; pixel_count * 4];
    for (i, p) in packed.iter().enumerate() {
        // texture2ddecoder packs B=bits0-7, G=8-15, R=16-23, A=24-31.
        rgba[i * 4] = ((*p >> 16) & 0xFF) as u8; // R
        rgba[i * 4 + 1] = ((*p >> 8) & 0xFF) as u8; // G
        rgba[i * 4 + 2] = (*p & 0xFF) as u8; // B
        rgba[i * 4 + 3] = ((*p >> 24) & 0xFF) as u8; // A
    }
    Ok(rgba)
}

/// Dispatch to the right `texture2ddecoder` entry point for `format`.
fn decode_bcn(
    format: TextureFormat,
    data: &[u8],
    width: usize,
    height: usize,
    out: &mut [u32],
) -> Result<()> {
    use texture2ddecoder as t2d;
    let res = match format {
        TextureFormat::Bc1Unorm | TextureFormat::Bc1UnormSrgb => {
            t2d::decode_bc1(data, width, height, out)
        }
        TextureFormat::Bc2Unorm | TextureFormat::Bc2UnormSrgb => {
            t2d::decode_bc2(data, width, height, out)
        }
        TextureFormat::Bc3Unorm | TextureFormat::Bc3UnormSrgb => {
            t2d::decode_bc3(data, width, height, out)
        }
        TextureFormat::Bc4Unorm | TextureFormat::Bc4Snorm => {
            t2d::decode_bc4(data, width, height, out)
        }
        TextureFormat::Bc5Unorm | TextureFormat::Bc5Snorm => {
            t2d::decode_bc5(data, width, height, out)
        }
        TextureFormat::Bc6UFloat => t2d::decode_bc6_unsigned(data, width, height, out),
        TextureFormat::Bc6Float => t2d::decode_bc6_signed(data, width, height, out),
        TextureFormat::Bc7Unorm | TextureFormat::Bc7UnormSrgb => {
            t2d::decode_bc7(data, width, height, out)
        }
        TextureFormat::R8G8B8A8Unorm | TextureFormat::R8G8B8A8UnormSrgb => {
            unreachable!("uncompressed handled by caller")
        }
    };
    res.map_err(|e| Error::Texpipe(format!("decoding {}: {e}", format.name())))
}

/// Remap the natural decoded RGBA channels through the texture's
/// channel-swizzle so the output matches what the GPU samples (e.g. a
/// BC4 alpha-mask with swizzle `One,One,One,Red` becomes white-with-
/// alpha; a BC5 with `Red,Red,Red,Green` becomes grayscale-with-alpha).
fn apply_channel_swizzle(natural: &[u8], swizzle: [u8; 4]) -> Vec<u8> {
    let sources = [
        ChannelSource::from_byte(swizzle[0]),
        ChannelSource::from_byte(swizzle[1]),
        ChannelSource::from_byte(swizzle[2]),
        ChannelSource::from_byte(swizzle[3]),
    ];
    // Fast path: identity (R,G,B,A) avoids a per-pixel copy decision.
    if sources
        == [
            ChannelSource::Red,
            ChannelSource::Green,
            ChannelSource::Blue,
            ChannelSource::Alpha,
        ]
    {
        return natural.to_vec();
    }

    let mut out = vec![0u8; natural.len()];
    for (px_in, px_out) in natural.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
        for (ch, src) in sources.iter().enumerate() {
            px_out[ch] = match src {
                ChannelSource::Zero => 0,
                ChannelSource::One => 255,
                ChannelSource::Red => px_in[0],
                ChannelSource::Green => px_in[1],
                ChannelSource::Blue => px_in[2],
                ChannelSource::Alpha => px_in[3],
            };
        }
    }
    out
}
