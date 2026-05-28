//! Pure-Rust BNTX (Nintendo Switch texture container) parser/writer.
//!
//! Targets BNTX version 0x00040000 as shipped in modern Switch titles
//! (Smash Ultimate, Mario Kart 8 DX, etc.). The on-disk format is:
//!
//! ```text
//! 0x0000  BNTX header (32 bytes)
//! 0x0020  NX   header (40 bytes) -- count, info_ptrs_off, data_blk_ptr,
//!                                   dict_off, dict_size_field
//! 0x0048  Memory pool (0x150 bytes of zeros)
//! 0x0198  Texture info pointer array (count * u64 -- each points to a BRTI)
//!         _STR section: u32 magic, u32 unk1, u64 block_size,
//!                       u32 str_count, [BntxStr: u16 len + bytes + null + pad-to-4]
//!         _DIC section: u32 magic, u32 count,
//!                       (count+1) * { u32 ref_bit, u16 left, u16 right, u64 name_ptr }
//!         BRTI sections: each 0xA0 header + 0x208 trailing block
//!                        (0x100 + 0x100 zeros + 8-byte indirect texture_data ptr)
//!         BRTD section: u32 magic, u32 0x00, u64 block_size, then concatenated
//!                       texture data
//!         _RLT relocation table: u32 magic, u32 self_offset, u32 section_count,
//!                                u32 padding, sections (24 bytes each),
//!                                entries (8 bytes each)
//! ```
//!
//! Reading captures the full structure so the writer can reproduce a
//! byte-identical file. Modifications (add texture, replace data) are
//! supported by edit operations on the parsed state followed by a full
//! re-serialize.

pub mod error;
pub mod dict_builder;
mod read;
mod write;

pub use error::Error as BntxError;
pub use read::read_bntx;
#[allow(unused_imports)]
pub use write::write_bntx;

// ============================================================
// Top-level parsed BNTX
// ============================================================

/// A fully-parsed BNTX file. Every byte is captured (either as a
/// structured field or as opaque preserved bytes) so the writer can
/// reproduce byte-identical output.
#[derive(Debug, Clone)]
pub struct BntxFile {
    /// File-level header (32 bytes on disk).
    pub header: BntxHeader,

    /// NX header (40 bytes on disk).
    pub nx_header: NxHeader,

    /// Container name (the "BNTX file name" — usually the basename).
    pub name: String,

    /// All strings used in the file: the container name and one entry
    /// per texture, in the order they appear in `_STR`. Index 0 is
    /// always the empty string (BNTX's "null" sentinel).
    pub strings: Vec<String>,

    /// Patricia-trie dictionary keying strings 1..=N (each indexes into
    /// `strings`). Preserved verbatim because rebuilding the trie
    /// requires understanding Nintendo's hash function. For round-trip
    /// we just emit it back; for adding textures we'll need to extend it.
    pub dict: DictSection,

    /// Per-texture metadata. Pixel data lives separately in `brtd` so
    /// the data block can be reconstructed atomically.
    pub textures: Vec<Texture>,

    /// BRTD block: header + concatenated texture data bytes.
    pub brtd: BrtdSection,

    /// `_RLT` relocation table — every pointer in the file is tracked.
    pub relocation_table: RelocationTable,
    /// Set when modifications (`append_texture`, etc.) have invalidated
    /// the original `relocation_table`. The writer regenerates a
    /// canonical RLT in this case; for unmodified files the original is
    /// emitted verbatim so round-trip is byte-identical even when the
    /// source tool used an idiosyncratic RLT layout (e.g., the C#
    /// Switch-Toolbox emits one RLT entry per pointer rather than a
    /// compact stride-encoded entry).
    pub relocation_table_dirty: bool,
}

/// BNTX file header (offsets are within the file).
#[derive(Debug, Clone)]
pub struct BntxHeader {
    /// Always `0x00040000` for the files we target.
    pub version: u32,
    /// `1u8 << alignment_shift` is the texture-data alignment.
    pub alignment_shift: u8,
    /// `64` for 64-bit BNTX (only mode we support).
    pub target_address_size: u8,
    pub flag: u16,
    /// Offset of the first block (typically `_STR`).
    pub first_block_offset: u16,
    pub filename_offset: u32,
}

/// NX header (40 bytes after the BNTX header).
#[derive(Debug, Clone)]
pub struct NxHeader {
    /// Field at +0x40 in the original; semantics aren't fully nailed
    /// down. We capture and emit it verbatim.
    pub dict_size_field: u64,
}

#[derive(Debug, Clone)]
pub struct DictSection {
    /// `count` field at +0x04. `entries.len()` is `count + 1` (root sentinel).
    pub count: u32,
    pub entries: Vec<DictEntry>,
}

#[derive(Debug, Clone, Copy)]
pub struct DictEntry {
    pub ref_bit: u32,
    pub left: u16,
    pub right: u16,
    /// Index into `BntxFile.strings`. Stored as an index for portability;
    /// the on-disk pointer is computed at write time from the layout.
    pub string_index: u32,
}

#[derive(Debug, Clone)]
pub struct BrtdSection {
    /// Block size as recorded in the BRTD header (u64). Recomputed on
    /// write but cached for sanity checks.
    pub declared_block_size: u64,
    /// Concatenated raw texture data, laid out the same way the file
    /// has it. Each `Texture.data_offset_in_brtd` indexes into this.
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct RelocationTable {
    pub sections: Vec<RltSection>,
    pub entries: Vec<RltEntry>,
}

#[derive(Debug, Clone, Copy)]
pub struct RltSection {
    pub pointer: u64,
    pub position: u32,
    pub size: u32,
    pub index: u32,
    pub count: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct RltEntry {
    pub position: u32,
    pub struct_count: u16,
    pub offset_count: u8,
    pub padding_count: u8,
}

// ============================================================
// Texture model
// ============================================================

#[derive(Debug, Clone)]
pub struct Texture {
    /// Index into `BntxFile.strings` for the texture name.
    pub name_string_index: u32,

    pub flags: u8,
    pub dim: u8,
    pub tile_mode: u16,
    pub swizzle: u16,
    pub mips_count: u16,
    pub num_multi_sample: u32,
    pub format: TextureFormat,
    pub unk2: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub array_len: u32,
    /// `block_height_log2` / "size_range" — the Tegra block-linear height
    /// shift used by `tegra_swizzle`.
    pub size_range: i32,
    pub unk4: [u32; 6],
    pub image_size: u32,
    pub align: u32,
    pub channel_swizzle: u32,
    pub ty: u32,
    pub parent_addr: u64,

    /// Offset of this texture's data within `BrtdSection.data`. Stored so
    /// we can locate the pixel bytes after parsing.
    pub data_offset_in_brtd: usize,
}

impl Texture {
    /// Convenience: pixel data slice from a parent `BntxFile.brtd`.
    pub fn pixel_data<'a>(&self, brtd: &'a BrtdSection) -> &'a [u8] {
        let end = self.data_offset_in_brtd + self.image_size as usize;
        &brtd.data[self.data_offset_in_brtd..end]
    }

    pub fn name<'a>(&self, file: &'a BntxFile) -> &'a str {
        &file.strings[self.name_string_index as usize]
    }
}

// ============================================================
// Surface format enum
// ============================================================

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

// ============================================================
// Helpers used by the consumer (CLI verbs).
// ============================================================

impl BntxFile {
    /// Find a texture by exact name match.
    pub fn texture_index_by_name(&self, name: &str) -> Option<usize> {
        self.textures
            .iter()
            .position(|t| self.strings.get(t.name_string_index as usize).map(String::as_str) == Some(name))
    }

    /// Look up the channel-swizzle bytes for a texture (4 entries: R,G,B,A
    /// channels).
    pub fn channel_swizzle(&self, tex: &Texture) -> [u8; 4] {
        [
            (tex.channel_swizzle & 0xFF) as u8,
            ((tex.channel_swizzle >> 8) & 0xFF) as u8,
            ((tex.channel_swizzle >> 16) & 0xFF) as u8,
            ((tex.channel_swizzle >> 24) & 0xFF) as u8,
        ]
    }

    /// Append a new texture to the BNTX. The caller supplies a fully-
    /// configured `AppendTextureSpec` (typically built by the texpipe
    /// module from a PNG). After the call, `self.textures.last()` is
    /// the new texture. The dict trie, relocation table struct counts,
    /// and BRTD data block are all updated automatically.
    pub fn append_texture(&mut self, name: String, spec: AppendTextureSpec) -> Result<(), BntxError> {
        if name.is_empty() {
            return Err(BntxError::Format("texture name cannot be empty".into()));
        }
        if self.strings.iter().any(|s| s == &name) {
            return Err(BntxError::Format(format!(
                "string '{name}' already exists in _STR"
            )));
        }

        // Pad BRTD data to the new texture's alignment boundary.
        let align = spec.align.max(1) as usize;
        let pad_to = (self.brtd.data.len() + align - 1) & !(align - 1);
        let pad_bytes = pad_to - self.brtd.data.len();
        self.brtd.data.extend(std::iter::repeat(0u8).take(pad_bytes));
        let data_offset_in_brtd = self.brtd.data.len();
        self.brtd.data.extend_from_slice(&spec.swizzled_data);

        // Append the name to the string pool.
        let name_string_index = self.strings.len() as u32;
        self.strings.push(name);

        // Construct the Texture metadata.
        let texture = Texture {
            name_string_index,
            flags: spec.flags,
            dim: spec.dim,
            tile_mode: spec.tile_mode,
            swizzle: spec.swizzle,
            mips_count: spec.mips_count,
            num_multi_sample: spec.num_multi_sample,
            format: spec.format,
            unk2: spec.unk2,
            width: spec.width,
            height: spec.height,
            depth: spec.depth,
            array_len: spec.array_len,
            size_range: spec.size_range,
            unk4: spec.unk4,
            image_size: spec.swizzled_data.len() as u32,
            align: spec.align,
            channel_swizzle: spec.channel_swizzle,
            ty: spec.ty,
            parent_addr: spec.parent_addr,
            data_offset_in_brtd,
        };
        self.textures.push(texture);

        // Rebuild the dict trie over all texture names (skipping idx 0 =
        // empty sentinel and idx 1 = container name).
        self.rebuild_dict();

        // Mark the RLT for regeneration. The writer rebuilds a fresh
        // canonical table when this is set; otherwise the original RLT
        // is preserved verbatim (round-trip case).
        self.relocation_table_dirty = true;
        Ok(())
    }

    /// Rebuild `self.dict` from the current `self.strings` list. Texture
    /// names live at `strings[2..]` (idx 0 = empty, idx 1 = container).
    pub fn rebuild_dict(&mut self) {
        let mut trie = crate::bntx::dict_builder::Trie::new();
        for (idx, s) in self.strings.iter().enumerate().skip(2) {
            trie.insert(s.as_bytes(), idx as u32);
        }
        let entries = trie.to_entries();
        self.dict = DictSection {
            count: (entries.len() - 1) as u32, // root excluded from count
            entries,
        };
    }

    /// Remove a texture by name. This is a structural change: it shrinks
    /// the BRTI array, the string pool, the dict, and the BRTD data
    /// block (compacting subsequent textures' data forward, re-applying
    /// each texture's own alignment so the resulting layout matches
    /// what `append_texture` would produce from scratch). The
    /// relocation table is marked dirty so the writer regenerates a
    /// canonical layout.
    ///
    /// Errors if `name` is not present, or if the resolved string
    /// happens to be the empty sentinel / container name (defensive —
    /// `append_texture` refuses to create such names, but we still
    /// guard against a hand-crafted BNTX that points a BRTI at the
    /// container-name slot).
    pub fn remove_texture(&mut self, name: &str) -> Result<(), BntxError> {
        let tex_idx = self
            .texture_index_by_name(name)
            .ok_or_else(|| BntxError::Format(format!("texture '{name}' not found")))?;
        let removed_string_idx = self.textures[tex_idx].name_string_index as usize;
        if removed_string_idx < 2 {
            return Err(BntxError::Format(format!(
                "texture '{name}' resolves to string index {removed_string_idx} (reserved \
                 for the empty sentinel / container name); refusing to remove"
            )));
        }

        // Snapshot every *other* texture's pixel-data slice from the
        // current BRTD before we touch anything. The slices are
        // computed against `self.brtd.data` (unchanged here), so
        // ordering is safe to do up-front. We can't borrow these
        // slices once we start rewriting `self.brtd.data` below.
        let mut remaining_data: Vec<Vec<u8>> = Vec::with_capacity(self.textures.len() - 1);
        for (i, t) in self.textures.iter().enumerate() {
            if i == tex_idx {
                continue;
            }
            remaining_data.push(t.pixel_data(&self.brtd).to_vec());
        }

        self.textures.remove(tex_idx);

        // Drop the removed texture's name from the string pool. Any
        // texture whose own `name_string_index` was *after* the removed
        // slot needs its index decremented by 1 — the underlying string
        // didn't change but its position in `self.strings` did.
        self.strings.remove(removed_string_idx);
        for t in &mut self.textures {
            if t.name_string_index as usize > removed_string_idx {
                t.name_string_index -= 1;
            }
        }

        // Rebuild BRTD by laying out the remaining textures back-to-back
        // with each one's own alignment, exactly the way
        // `append_texture` would produce them from scratch. Per-texture
        // `data_offset_in_brtd` is updated in lockstep.
        let mut new_data: Vec<u8> = Vec::with_capacity(self.brtd.data.len());
        for (t, data) in self.textures.iter_mut().zip(remaining_data.iter()) {
            let align = t.align.max(1) as usize;
            let pad_to = (new_data.len() + align - 1) & !(align - 1);
            new_data.extend(std::iter::repeat(0u8).take(pad_to - new_data.len()));
            t.data_offset_in_brtd = new_data.len();
            new_data.extend_from_slice(data);
        }
        self.brtd.data = new_data;

        self.rebuild_dict();
        self.relocation_table_dirty = true;
        Ok(())
    }
}

/// Caller-supplied parameters for `BntxFile::append_texture`.
///
/// The default constructors cover the common cases:
/// - `bc7_2d_default` — single-mip 2D BC7 (the SGPO face-button case)
/// - `bc7_2d_with_mips` — multi-mip 2D BC7
/// - `bc7_cube_default` — cube map (6 array layers)
///
/// Callers can also build the struct directly to set every field.
#[derive(Debug, Clone)]
pub struct AppendTextureSpec {
    pub format: TextureFormat,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub mips_count: u16,
    pub array_len: u32,
    /// Tegra block-height-log2 (the swizzler picked this).
    pub size_range: i32,
    /// 4-byte channel-swizzle pack.
    pub channel_swizzle: u32,
    pub align: u32,
    pub flags: u8,
    /// `dim` is the BNTX surface dimension code: 2 = 2D, 8 = cube. Most
    /// callers should use the matching constructor rather than setting
    /// this directly.
    pub dim: u8,
    pub tile_mode: u16,
    pub swizzle: u16,
    pub num_multi_sample: u32,
    pub unk2: u32,
    pub unk4: [u32; 6],
    pub ty: u32,
    pub parent_addr: u64,
    pub swizzled_data: Vec<u8>,
}

impl AppendTextureSpec {
    /// Build a sensible default spec for a 2D BC7 texture with a single
    /// mip level. Caller fills in `width`, `height`, `swizzled_data`,
    /// and `size_range`, and overrides anything else.
    pub fn bc7_2d_default(
        width: u32,
        height: u32,
        size_range: i32,
        swizzled_data: Vec<u8>,
        srgb: bool,
    ) -> Self {
        Self::bc7_2d_with_mips(width, height, 1, size_range, swizzled_data, srgb)
    }

    /// Build a spec for a 2D BC7 texture with `mip_count` mip levels. The
    /// `swizzled_data` must already include all mips concatenated in
    /// the layout `tegra_swizzle::surface::swizzle_surface` produces
    /// (i.e., the result of running it with `mipmap_count = mip_count`).
    pub fn bc7_2d_with_mips(
        width: u32,
        height: u32,
        mip_count: u16,
        size_range: i32,
        swizzled_data: Vec<u8>,
        srgb: bool,
    ) -> Self {
        Self {
            format: if srgb {
                TextureFormat::Bc7UnormSrgb
            } else {
                TextureFormat::Bc7Unorm
            },
            width,
            height,
            depth: 1,
            mips_count: mip_count,
            array_len: 1,
            size_range,
            // Channel swizzle: R=2, G=3, B=4, A=5 (standard mapping).
            channel_swizzle: 0x05_04_03_02,
            // 0x200 = 512 bytes (sufficient for BC7 textures up to ~256x256).
            // Larger textures may need a larger alignment; callers can
            // override (use 0x1000 for 512x512+).
            align: 0x200,
            flags: 1,
            dim: 2, // 2D
            tile_mode: 0,
            swizzle: 0,
            num_multi_sample: 1,
            unk2: 32,
            unk4: [65543, 0, 0, 0, 0, 0],
            ty: 1,
            parent_addr: 32,
            swizzled_data,
        }
    }

    /// Build a spec for a cube-map BC7 texture (6 array layers in the
    /// canonical cube-map layout: +X, -X, +Y, -Y, +Z, -Z). The
    /// `swizzled_data` must include all 6 layers (and any mips per
    /// layer if `mip_count > 1`) concatenated in the layout
    /// `swizzle_surface(layer_count = 6)` produces.
    pub fn bc7_cube_default(
        size: u32,
        mip_count: u16,
        size_range: i32,
        swizzled_data: Vec<u8>,
        srgb: bool,
    ) -> Self {
        let mut s = Self::bc7_2d_with_mips(size, size, mip_count, size_range, swizzled_data, srgb);
        s.dim = 8; // cube
        s.array_len = 6;
        s
    }
}
