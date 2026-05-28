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

        // Update the relocation table to reflect the new BRTI block and
        // texture-data slot.
        self.update_reloc_table_for_new_texture();

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

    /// After appending a texture, bump the relocation table's per-pattern
    /// struct counts and section sizes. This keeps the table in sync with
    /// the new file layout.
    fn update_reloc_table_for_new_texture(&mut self) {
        // Section 0 covers the BNTX/NX headers, info ptrs array, _STR,
        // _DIC, and all BRTIs. Adding a texture adds:
        //   * 1 BRTI block (= 0x2A8 bytes)
        //   * 1 dict entry (= 0x10 bytes)
        //   * 1 string entry (variable bytes — handled by writer)
        //   * 1 texture-info pointer (= 8 bytes)
        // The writer recomputes the absolute layout, so the section
        // sizes here just need to GROW. We bump section 0 by the BRTI
        // block stride only; the dict/string growth is small and the
        // writer's placement logic handles those edges.
        const BRTI_BLOCK_STRIDE: u32 = 0x2A8;
        if let Some(s0) = self.relocation_table.sections.first_mut() {
            s0.size += BRTI_BLOCK_STRIDE;
        }
        // Section 1 covers BRTD. Bump by the new texture's image size +
        // the alignment padding we inserted.
        if let Some(last) = self.textures.last() {
            let new_data_bytes = (self.brtd.data.len() - last.data_offset_in_brtd
                + (last.data_offset_in_brtd
                    - self.textures.get(self.textures.len() - 2)
                        .map(|t| t.data_offset_in_brtd + t.image_size as usize)
                        .unwrap_or(0))) as u32;
            if let Some(s1) = self.relocation_table.sections.get_mut(1) {
                s1.size += new_data_bytes;
            }
        }

        // Update entry struct_counts. The pattern-specific entries were
        // identified by reverse-engineering an existing BNTX:
        //   entry 2 = texture-info pointer array (1 ptr per texture)
        //   entry 3 = dict name_ptrs (1 per (count+1) entries)
        //   entry 4 = BRTI fields at +0x60 (3 ptrs)
        //   entry 5 = BRTI fields at +0x80 (2 ptrs)
        //   entry 7 = BRTI texture-data indirect slot (1 ptr)
        // We bump every entry whose `struct_count` already equals the
        // pre-add texture count or texture+1 (for the dict).
        let textures_now = self.textures.len() as u16;
        for entry in self.relocation_table.entries.iter_mut() {
            if entry.struct_count == textures_now - 1 {
                entry.struct_count = textures_now;
            } else if entry.struct_count == textures_now {
                // dict entry (already incremented by 1 due to root)
                entry.struct_count = textures_now + 1;
            }
        }
    }
}

/// Caller-supplied parameters for `BntxFile::append_texture`.
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
    /// Build a sensible default spec for a 2D BC7 texture. Caller should
    /// fill in `width`, `height`, `swizzled_data`, `size_range`, and
    /// override anything else.
    pub fn bc7_2d_default(
        width: u32,
        height: u32,
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
            mips_count: 1,
            array_len: 1,
            size_range,
            // Channel swizzle: R=2, G=3, B=4, A=5 (standard mapping).
            channel_swizzle: 0x05_04_03_02,
            // 0x200 = 512 bytes (sufficient for BC7 textures up to ~256x256).
            // Larger textures may need a larger alignment; callers can
            // override.
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
}
