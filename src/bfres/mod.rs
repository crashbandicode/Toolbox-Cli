//! BFRES (`FRES`, *Binary caFe RESource*) read + write.
//!
//! BFRES is Nintendo's container for 3D resources — models ([`FMDL`]), skeletal
//! / material / visibility / scene animations, embedded textures (a [`BNTX`]
//! block), and the shared string pool / radix dictionaries / relocation table.
//! It is the model format for Breath of the Wild (Switch) and Tears of the
//! Kingdom:
//!
//! - **BOTW** ships it as `Model/*.sbfres` (Yaz0) — version `0x00050003`.
//! - **TotK** ships animations/textures as `Model/*.bfres.zs` (plain zstd) and
//!   models as `Model/*.bfres.mc` (MeshCodec) — version `0x000A0000`.
//!
//! Both are little-endian on Switch (a Wii U BFRES would be big-endian; the
//! byte-order mark is honored). Decompress the container first (the toolbox's
//! [`crate::compression`] handles Yaz0 + zstd); MeshCodec `.mc` needs an
//! external decompressor today.
//!
//! ## What this module decodes
//!
//! BFRES is a large format whose model/vertex/material payloads are
//! offset-and-relocation heavy (like [`crate::bntx`]). This module is
//! **inspect + byte-identical round-trip** only: [`read_bfres`] decodes the
//! header (magic, version, endianness, embedded file name, file size,
//! relocation-table offset) and does a structural scan for the well-known
//! sub-block magics, while retaining the original bytes so [`write_bfres`]
//! re-emits them **verbatim** — byte-identical by construction for an unmodified
//! document. Decoding the model/animation sub-resources is a follow-up.
//!
//! ## Header (`FRES    `, little-endian on Switch)
//!
//! ```text
//! 0x00 char[8] magic "FRES    "
//! 0x08 u32     version           (BOTW 0x00050003 / TotK 0x000A0000)
//! 0x0C u16     byteOrderMark     (0xFEFF)
//! 0x10 u32     fileNameOffset    -> name chars (a u16 length sits at -2)
//! 0x18 u32     relocationTableOffset -> "_RLT"
//! 0x1C u32     fileSize
//! ```

mod error;
mod read;
mod write;

pub use error::{BfresError, Result};
pub use read::read_bfres;
pub use write::write_bfres;

/// The 8-byte BFRES magic: `FRES` followed by four spaces.
pub const BFRES_MAGIC: &[u8; 8] = b"FRES    ";
/// Bytes needed before the header is fully readable (through `fileSize`).
pub const HEADER_LEN: usize = 0x20;

/// BOTW (Switch) BFRES version.
pub const VERSION_BOTW: u32 = 0x0005_0003;
/// TotK BFRES version.
pub const VERSION_TOTK: u32 = 0x000A_0000;

/// Container / sub-resource magics scanned for in [`read_bfres`] to summarize a
/// file's contents. Each is a 4-byte ASCII tag at the start of a block.
pub(crate) const BLOCK_MAGICS: &[&[u8; 4]] = &[
    b"FMDL", // model
    b"FSKA", // skeletal animation
    b"FMAA", // material animation (v10)
    b"FSHU", // shader-param animation (older)
    b"FTXP", // texture-pattern animation
    b"FVIS", // bone-visibility animation
    b"FSHA", // shape animation
    b"FSCN", // scene animation
    b"FSKL", // skeleton
    b"FVTX", // vertex buffer
    b"FSHP", // shape (polygon)
    b"FMAT", // material
    b"BNTX", // embedded texture container
    b"_STR", // string pool
    b"_DIC", // radix dictionary
    b"_RLT", // relocation table
];

/// One detected sub-block magic and how often it appears.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedBlock {
    /// The 4-byte ASCII magic (e.g. `FMDL`).
    pub magic: String,
    /// How many times it occurs in the file.
    pub count: usize,
    /// The byte offset of the first occurrence.
    pub first_offset: usize,
}

/// A parsed BFRES container.
///
/// Retains the original bytes (`raw`) so an unmodified document re-emits
/// byte-identically via [`write_bfres`]. The decoded fields are the header
/// essentials plus a structural scan; the model/animation payloads are not
/// decoded (yet).
#[derive(Debug, Clone)]
pub struct BfresDocument {
    /// Raw version word (`0x00050003` BOTW, `0x000A0000` TotK).
    pub version: u32,
    /// `true` if big-endian (Wii U); Switch BFRES is little-endian.
    pub big_endian: bool,
    /// The embedded file name (e.g. `Animal_Bass`).
    pub name: String,
    /// The `fileSize` header field. For a tool-padded file (e.g. MeshCodec
    /// output zero-padded to an alignment) this is *less* than `raw.len()`.
    pub file_size: u32,
    /// Offset of the `_RLT` relocation table.
    pub relocation_table_offset: u32,
    /// Structural scan of the well-known sub-block magics present.
    pub blocks: Vec<DetectedBlock>,
    /// The original file bytes, used for the verbatim [`write_bfres`] path.
    pub(crate) raw: Vec<u8>,
}

impl BfresDocument {
    /// The original bytes captured at parse time.
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// A short human label for the version word, with a game hint when known.
    pub fn version_label(&self) -> String {
        match self.version {
            VERSION_BOTW => "0x00050003 (BOTW)".to_string(),
            VERSION_TOTK => "0x000A0000 (TotK)".to_string(),
            v => format!("0x{v:08x}"),
        }
    }

    /// The offset of the embedded `BNTX` texture block, if present (BOTW
    /// `.Tex.bfres` wraps a full BNTX). Returns the first occurrence.
    pub fn embedded_bntx_offset(&self) -> Option<usize> {
        self.blocks
            .iter()
            .find(|b| b.magic == "BNTX")
            .map(|b| b.first_offset)
    }

    /// The bytes of the embedded `BNTX` block, bounded by the BNTX's own
    /// `file_size` field (at its `+0x1C`, little-endian), ready to hand to
    /// [`crate::bntx::read_bntx`]. `None` if there's no embedded BNTX or its
    /// declared size runs past the file.
    pub fn embedded_bntx_bytes(&self) -> Option<&[u8]> {
        let start = self.embedded_bntx_offset()?;
        let size_pos = start.checked_add(0x1C)?;
        if size_pos.checked_add(4)? > self.raw.len() {
            return None;
        }
        let size = u32::from_le_bytes([
            self.raw[size_pos],
            self.raw[size_pos + 1],
            self.raw[size_pos + 2],
            self.raw[size_pos + 3],
        ]) as usize;
        let end = start.checked_add(size)?;
        self.raw.get(start..end)
    }

    /// Count of a given block magic in the structural scan.
    pub fn block_count(&self, magic: &str) -> usize {
        self.blocks
            .iter()
            .find(|b| b.magic == magic)
            .map_or(0, |b| b.count)
    }
}
