//! Minimal DDS (DirectDraw Surface) reader/writer for texture interchange.
//!
//! We always **write** the modern DX10 extended header so the exact DXGI
//! format (including sRGB-ness) round-trips losslessly. We **read** both
//! DX10 files and the common legacy FourCC encodings (`DXT1/3/5`,
//! `ATI1/BC4U`, `ATI2/BC5U`, and a standard 32-bit RGBA layout) so DDS
//! files produced by other tools (texconv, GIMP, Switch-Toolbox) can be
//! imported.
//!
//! The pixel payload is a tightly-packed *linear* surface ordered
//! `layer0 mip0, layer0 mip1, ..., layer1 mip0, ...` — exactly the layout
//! [`tegra_swizzle::surface::deswizzle_surface`] emits and
//! [`tegra_swizzle::surface::swizzle_surface`] consumes, so converting
//! between DDS and BNTX is a deswizzle/swizzle plus this header.

use crate::bntx::TextureFormat;
use crate::error::{Error, Result};

const DDS_MAGIC: u32 = 0x2053_4444; // "DDS "
const HEADER_SIZE: u32 = 124;
const PIXELFORMAT_SIZE: u32 = 32;

// DDS_HEADER.dwFlags
const DDSD_CAPS: u32 = 0x1;
const DDSD_HEIGHT: u32 = 0x2;
const DDSD_WIDTH: u32 = 0x4;
const DDSD_PIXELFORMAT: u32 = 0x1000;
const DDSD_MIPMAPCOUNT: u32 = 0x2_0000;
const DDSD_LINEARSIZE: u32 = 0x8_0000;

// DDS_PIXELFORMAT.dwFlags
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;
const DDPF_ALPHAPIXELS: u32 = 0x1;

// DDS_HEADER.dwCaps
const DDSCAPS_COMPLEX: u32 = 0x8;
const DDSCAPS_TEXTURE: u32 = 0x1000;
const DDSCAPS_MIPMAP: u32 = 0x40_0000;

// DDS_HEADER.dwCaps2
const DDSCAPS2_CUBEMAP: u32 = 0x200;
const DDSCAPS2_CUBEMAP_ALLFACES: u32 = 0xFC00; // the six per-face bits

// DX10 resourceDimension
const D3D10_RESOURCE_DIMENSION_TEXTURE2D: u32 = 3;
// DX10 miscFlag
const DDS_RESOURCE_MISC_TEXTURECUBE: u32 = 0x4;

const FOURCC_DX10: u32 = u32::from_le_bytes(*b"DX10");

/// A decoded DDS surface ready to convert to/from a BNTX texture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dds {
    pub format: TextureFormat,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub mip_count: u32,
    /// Total array layers (e.g. 6 for a single cube map).
    pub array_count: u32,
    pub is_cube: bool,
    /// Linear surface bytes (`layer`-major, then `mip`).
    pub data: Vec<u8>,
}

/// Map a BNTX surface format to its DXGI_FORMAT value.
fn dxgi_format(format: TextureFormat) -> u32 {
    match format {
        TextureFormat::Bc1Unorm => 71,
        TextureFormat::Bc1UnormSrgb => 72,
        TextureFormat::Bc2Unorm => 74,
        TextureFormat::Bc2UnormSrgb => 75,
        TextureFormat::Bc3Unorm => 77,
        TextureFormat::Bc3UnormSrgb => 78,
        TextureFormat::Bc4Unorm => 80,
        TextureFormat::Bc4Snorm => 81,
        TextureFormat::Bc5Unorm => 83,
        TextureFormat::Bc5Snorm => 84,
        TextureFormat::Bc6UFloat => 95,
        TextureFormat::Bc6Float => 96,
        TextureFormat::Bc7Unorm => 98,
        TextureFormat::Bc7UnormSrgb => 99,
        TextureFormat::R8G8B8A8Unorm => 28,
        TextureFormat::R8G8B8A8UnormSrgb => 29,
    }
}

/// Map a DXGI_FORMAT value to a BNTX surface format.
fn format_from_dxgi(v: u32) -> Option<TextureFormat> {
    Some(match v {
        71 => TextureFormat::Bc1Unorm,
        72 => TextureFormat::Bc1UnormSrgb,
        74 => TextureFormat::Bc2Unorm,
        75 => TextureFormat::Bc2UnormSrgb,
        77 => TextureFormat::Bc3Unorm,
        78 => TextureFormat::Bc3UnormSrgb,
        80 => TextureFormat::Bc4Unorm,
        81 => TextureFormat::Bc4Snorm,
        83 => TextureFormat::Bc5Unorm,
        84 => TextureFormat::Bc5Snorm,
        95 => TextureFormat::Bc6UFloat,
        96 => TextureFormat::Bc6Float,
        98 => TextureFormat::Bc7Unorm,
        99 => TextureFormat::Bc7UnormSrgb,
        28 => TextureFormat::R8G8B8A8Unorm,
        29 => TextureFormat::R8G8B8A8UnormSrgb,
        _ => return None,
    })
}

/// Linear bytes for mip 0 of a single layer (the value DDS records in
/// `dwPitchOrLinearSize` for block-compressed surfaces).
fn top_level_linear_size(format: TextureFormat, width: u32, height: u32) -> u32 {
    let (bw, bh) = format.block_dim();
    let bpp = format.block_size();
    width.div_ceil(bw) * height.div_ceil(bh) * bpp
}

impl Dds {
    /// Serialize to DDS bytes (always with the DX10 extended header).
    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + HEADER_SIZE as usize + 20 + self.data.len());
        let mut w = |v: u32| out.extend_from_slice(&v.to_le_bytes());

        w(DDS_MAGIC);

        // ---- DDS_HEADER ----
        let mut flags = DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT | DDSD_LINEARSIZE;
        if self.mip_count > 1 {
            flags |= DDSD_MIPMAPCOUNT;
        }
        w(HEADER_SIZE);
        w(flags);
        w(self.height);
        w(self.width);
        w(top_level_linear_size(self.format, self.width, self.height));
        w(self.depth.max(1));
        w(self.mip_count.max(1));
        for _ in 0..11 {
            w(0); // dwReserved1[11]
        }

        // ---- DDS_PIXELFORMAT (FourCC = DX10) ----
        w(PIXELFORMAT_SIZE);
        w(DDPF_FOURCC);
        w(FOURCC_DX10);
        w(0); // dwRGBBitCount
        w(0); // R mask
        w(0); // G mask
        w(0); // B mask
        w(0); // A mask

        // ---- caps ----
        let mut caps = DDSCAPS_TEXTURE;
        if self.mip_count > 1 || self.is_cube {
            caps |= DDSCAPS_COMPLEX;
        }
        if self.mip_count > 1 {
            caps |= DDSCAPS_MIPMAP;
        }
        w(caps);
        let caps2 = if self.is_cube {
            DDSCAPS2_CUBEMAP | DDSCAPS2_CUBEMAP_ALLFACES
        } else {
            0
        };
        w(caps2);
        w(0); // dwCaps3
        w(0); // dwCaps4
        w(0); // dwReserved2

        // ---- DDS_HEADER_DXT10 ----
        w(dxgi_format(self.format));
        w(D3D10_RESOURCE_DIMENSION_TEXTURE2D);
        w(if self.is_cube {
            DDS_RESOURCE_MISC_TEXTURECUBE
        } else {
            0
        });
        // arraySize counts whole cubes (6 faces) for cube maps.
        let array_size = if self.is_cube {
            (self.array_count / 6).max(1)
        } else {
            self.array_count.max(1)
        };
        w(array_size);
        w(0); // miscFlags2 (alpha mode = unknown)

        out.extend_from_slice(&self.data);
        out
    }

    /// Parse DDS bytes into a [`Dds`].
    pub fn read(bytes: &[u8]) -> Result<Dds> {
        if bytes.len() < 4 + HEADER_SIZE as usize {
            return Err(Error::Other("DDS file too small for header".into()));
        }
        let rd = |off: usize| -> u32 {
            u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
        };
        if rd(0) != DDS_MAGIC {
            return Err(Error::Other("not a DDS file (bad magic)".into()));
        }
        if rd(4) != HEADER_SIZE {
            return Err(Error::Other(format!(
                "unexpected DDS header size {} (want {HEADER_SIZE})",
                rd(4)
            )));
        }
        let height = rd(12);
        let width = rd(16);
        let depth = rd(24).max(1);
        let mip_count = rd(28).max(1);

        // DDS_PIXELFORMAT starts at file offset 76 (magic + 72 bytes of
        // header). The caps block follows the 32-byte pixelformat:
        // dwCaps=108, dwCaps2=112.
        let pf = 76;
        let pf_flags = rd(pf + 4);
        let fourcc = rd(pf + 8);
        let dw_caps2 = rd(112);
        let legacy_cube = (dw_caps2 & DDSCAPS2_CUBEMAP) != 0;

        let data_start;
        let format;
        let is_cube;
        let array_count;

        if pf_flags & DDPF_FOURCC != 0 && fourcc == FOURCC_DX10 {
            // DX10 extended header at offset 4 + 124 = 128.
            let dx = 128;
            if bytes.len() < dx + 20 {
                return Err(Error::Other("DDS DX10 header truncated".into()));
            }
            let dxgi = rd(dx);
            let misc_flag = rd(dx + 8);
            let array_size = rd(dx + 12).max(1);
            format = format_from_dxgi(dxgi)
                .ok_or_else(|| Error::Other(format!("unsupported DXGI format {dxgi}")))?;
            is_cube = misc_flag & DDS_RESOURCE_MISC_TEXTURECUBE != 0;
            array_count = if is_cube { array_size * 6 } else { array_size };
            data_start = dx + 20;
        } else if pf_flags & DDPF_FOURCC != 0 {
            // Legacy compressed FourCC.
            format = legacy_fourcc_format(fourcc)?;
            is_cube = legacy_cube;
            array_count = if is_cube { 6 } else { 1 };
            data_start = 4 + HEADER_SIZE as usize;
        } else if pf_flags & DDPF_RGB != 0 {
            // Legacy uncompressed: only standard 32bpp RGBA/BGRA accepted.
            let bit_count = rd(pf + 12);
            if bit_count != 32 {
                return Err(Error::Other(format!(
                    "unsupported uncompressed DDS bit count {bit_count} (only 32bpp RGBA)"
                )));
            }
            let has_alpha = pf_flags & DDPF_ALPHAPIXELS != 0;
            let _ = has_alpha;
            format = TextureFormat::R8G8B8A8Unorm;
            is_cube = legacy_cube;
            array_count = if is_cube { 6 } else { 1 };
            data_start = 4 + HEADER_SIZE as usize;
        } else {
            return Err(Error::Other(
                "unsupported DDS pixel format (need FourCC or RGB)".into(),
            ));
        }

        if data_start > bytes.len() {
            return Err(Error::Other("DDS data offset past end of file".into()));
        }
        let data = bytes[data_start..].to_vec();

        Ok(Dds {
            format,
            width,
            height,
            depth,
            mip_count,
            array_count,
            is_cube,
            data,
        })
    }
}

/// Map a legacy FourCC to a surface format (UNORM; legacy FourCC carries
/// no sRGB bit).
fn legacy_fourcc_format(fourcc: u32) -> Result<TextureFormat> {
    let tag = fourcc.to_le_bytes();
    Ok(match &tag {
        b"DXT1" => TextureFormat::Bc1Unorm,
        b"DXT3" => TextureFormat::Bc2Unorm,
        b"DXT5" => TextureFormat::Bc3Unorm,
        b"BC4U" | b"ATI1" => TextureFormat::Bc4Unorm,
        b"BC4S" => TextureFormat::Bc4Snorm,
        b"BC5U" | b"ATI2" => TextureFormat::Bc5Unorm,
        b"BC5S" => TextureFormat::Bc5Snorm,
        other => {
            return Err(Error::Other(format!(
                "unsupported legacy DDS FourCC {:?}",
                std::str::from_utf8(other).unwrap_or("????")
            )))
        }
    })
}
