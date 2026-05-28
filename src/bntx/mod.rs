//! Pure-Rust BNTX (Nintendo Switch texture container) parser/writer.
//!
//! BNTX is a relatively simple "header + sections" container; we model
//! enough of it to read existing files, edit the texture list (add /
//! replace / remove / rename), and write back. Texture pixel data is
//! stored swizzled for Tegra X1; encoding/decoding is handled by the
//! `texpipe` module on top of this.

pub mod error;
mod read;
mod write;

pub use error::Error as BntxError;
pub use read::read_bntx;
pub use write::write_bntx;

use std::collections::BTreeMap;

/// Top-level in-memory model.
#[derive(Debug, Clone)]
pub struct BNTX {
    /// Container name (the file's "BNTX" name string, usually equal to the
    /// filename without extension).
    pub name: String,
    pub textures: Vec<Texture>,
}

impl BNTX {
    pub fn empty(name: &str) -> Self {
        Self {
            name: name.to_string(),
            textures: Vec::new(),
        }
    }
    pub fn texture_index(&self, name: &str) -> Option<usize> {
        self.textures.iter().position(|t| t.name == name)
    }
    pub fn remove_texture(&mut self, name: &str) -> bool {
        if let Some(i) = self.texture_index(name) {
            self.textures.remove(i);
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone)]
pub struct Texture {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub mip_count: u32,
    pub array_count: u32,
    pub format: TextureFormat,
    pub channels: [Channel; 4],
    pub surface_dim: SurfaceDim,
    pub tile_mode: u8,
    pub texture_layout: u32,
    pub texture_layout2: u32,
    pub block_height_log2: u32,
    /// Swizzled (Tegra X1) image data, ready to write out. The texpipe
    /// module produces this from a PNG via BC7 + tegra_swizzle.
    pub data: Vec<u8>,
    /// Per-mip metadata (offsets within `data`, mip dimensions). Computed
    /// on read; recomputed on write so callers don't have to maintain it.
    pub mip_offsets: Vec<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureFormat {
    Bc1Unorm,
    Bc1UnormSrgb,
    Bc2Unorm,
    Bc2UnormSrgb,
    Bc3Unorm,
    Bc3UnormSrgb,
    Bc4Unorm,
    Bc4Snorm,
    Bc5Unorm,
    Bc5Snorm,
    Bc6UFloat,
    Bc6Float,
    Bc7Unorm,
    Bc7UnormSrgb,
    R8G8B8A8Unorm,
    R8G8B8A8UnormSrgb,
}

impl TextureFormat {
    /// BNTX `SurfaceFormat` 32-bit field. Values from
    /// `Syroot.NintenTools.NSW.Bntx.GFX.SurfaceFormat`. Each format gets a
    /// 16-bit "type" code in the high half, and a "channel format" code in
    /// the low half (0x01 = UNORM, 0x06 = SRGB, 0x02 = SNORM, etc.).
    pub fn to_surface_format(self) -> u32 {
        match self {
            TextureFormat::Bc1Unorm => 0x1A01,
            TextureFormat::Bc1UnormSrgb => 0x1A06,
            TextureFormat::Bc2Unorm => 0x1B01,
            TextureFormat::Bc2UnormSrgb => 0x1B06,
            TextureFormat::Bc3Unorm => 0x1C01,
            TextureFormat::Bc3UnormSrgb => 0x1C06,
            TextureFormat::Bc4Unorm => 0x1D01,
            TextureFormat::Bc4Snorm => 0x1D02,
            TextureFormat::Bc5Unorm => 0x1E01,
            TextureFormat::Bc5Snorm => 0x1E02,
            TextureFormat::Bc6UFloat => 0x1F05,
            TextureFormat::Bc6Float => 0x1F0A,
            TextureFormat::Bc7Unorm => 0x2001,
            TextureFormat::Bc7UnormSrgb => 0x2006,
            TextureFormat::R8G8B8A8Unorm => 0x0B01,
            TextureFormat::R8G8B8A8UnormSrgb => 0x0B06,
        }
    }

    pub fn from_surface_format(v: u32) -> Option<Self> {
        Some(match v {
            0x1A01 => TextureFormat::Bc1Unorm,
            0x1A06 => TextureFormat::Bc1UnormSrgb,
            0x1B01 => TextureFormat::Bc2Unorm,
            0x1B06 => TextureFormat::Bc2UnormSrgb,
            0x1C01 => TextureFormat::Bc3Unorm,
            0x1C06 => TextureFormat::Bc3UnormSrgb,
            0x1D01 => TextureFormat::Bc4Unorm,
            0x1D02 => TextureFormat::Bc4Snorm,
            0x1E01 => TextureFormat::Bc5Unorm,
            0x1E02 => TextureFormat::Bc5Snorm,
            0x1F05 => TextureFormat::Bc6UFloat,
            0x1F0A => TextureFormat::Bc6Float,
            0x2001 => TextureFormat::Bc7Unorm,
            0x2006 => TextureFormat::Bc7UnormSrgb,
            0x0B01 => TextureFormat::R8G8B8A8Unorm,
            0x0B06 => TextureFormat::R8G8B8A8UnormSrgb,
            _ => return None,
        })
    }

    pub fn block_dim(self) -> (u32, u32) {
        match self {
            TextureFormat::R8G8B8A8Unorm | TextureFormat::R8G8B8A8UnormSrgb => (1, 1),
            _ => (4, 4),
        }
    }

    /// Bytes per block (compressed) or per pixel (uncompressed).
    pub fn block_size(self) -> u32 {
        match self {
            TextureFormat::Bc1Unorm
            | TextureFormat::Bc1UnormSrgb
            | TextureFormat::Bc4Unorm
            | TextureFormat::Bc4Snorm => 8,
            TextureFormat::R8G8B8A8Unorm | TextureFormat::R8G8B8A8UnormSrgb => 4,
            _ => 16,
        }
    }

    /// Whether the format encodes alpha. Used by `bntx-inspect --json`.
    pub fn has_alpha(self) -> bool {
        !matches!(
            self,
            TextureFormat::Bc1Unorm
                | TextureFormat::Bc1UnormSrgb
                | TextureFormat::Bc4Unorm
                | TextureFormat::Bc4Snorm
                | TextureFormat::Bc5Unorm
                | TextureFormat::Bc5Snorm
                | TextureFormat::Bc6UFloat
                | TextureFormat::Bc6Float
        )
    }

    pub fn name(self) -> &'static str {
        match self {
            TextureFormat::Bc1Unorm => "BC1_UNORM",
            TextureFormat::Bc1UnormSrgb => "BC1_UNORM_SRGB",
            TextureFormat::Bc2Unorm => "BC2_UNORM",
            TextureFormat::Bc2UnormSrgb => "BC2_UNORM_SRGB",
            TextureFormat::Bc3Unorm => "BC3_UNORM",
            TextureFormat::Bc3UnormSrgb => "BC3_UNORM_SRGB",
            TextureFormat::Bc4Unorm => "BC4_UNORM",
            TextureFormat::Bc4Snorm => "BC4_SNORM",
            TextureFormat::Bc5Unorm => "BC5_UNORM",
            TextureFormat::Bc5Snorm => "BC5_SNORM",
            TextureFormat::Bc6UFloat => "BC6H_UFLOAT",
            TextureFormat::Bc6Float => "BC6H_FLOAT",
            TextureFormat::Bc7Unorm => "BC7_UNORM",
            TextureFormat::Bc7UnormSrgb => "BC7_UNORM_SRGB",
            TextureFormat::R8G8B8A8Unorm => "R8G8B8A8_UNORM",
            TextureFormat::R8G8B8A8UnormSrgb => "R8G8B8A8_UNORM_SRGB",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    Zero,
    One,
    Red,
    Green,
    Blue,
    Alpha,
}

impl Channel {
    pub fn name(self) -> &'static str {
        match self {
            Channel::Zero => "Zero",
            Channel::One => "One",
            Channel::Red => "Red",
            Channel::Green => "Green",
            Channel::Blue => "Blue",
            Channel::Alpha => "Alpha",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SurfaceDim {
    Dim2D,
    DimCube,
}

// Avoid unused-warning. BTreeMap may be useful when we extend support.
#[allow(dead_code)]
fn _btreemap_marker(_: &BTreeMap<u32, u32>) {}
