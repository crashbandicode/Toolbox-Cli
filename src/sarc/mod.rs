//! SARC (Sead ARChive) read + write.
//!
//! A fully **native** implementation — no third-party SARC crate. The
//! reader ([`read_arc`] / [`unpack`]) parses the `0x14` header + `SFAT`
//! node table + `SFNT` name blob; the writer ([`write_arc`] / [`write_sarc`])
//! gives each file the alignment its content requires (see [`file_alignment`])
//! instead of padding everything to `0x2000`, which roughly doubles a real
//! `layout.arc`. Switch titles use little-endian SARC; pass
//! `big_endian = true` for Wii U / 3DS (the reader auto-detects via the
//! header BOM).
//!
//! ## Module layout (toward a standalone `nx-sarc` crate)
//!
//! - [`read`] / [`write`] — the codec core, pure `std`.
//! - [`error`] — [`SarcError`] (`std + thiserror` only; no `walkdir`).
//! - [`fsutil`] — directory pack/unpack helpers; the only part that uses
//!   `std::fs` + `walkdir` (would become an optional `fs` feature).

mod error;
pub mod fsutil;
mod read;
mod write;

pub use error::{Result, SarcError};
pub use fsutil::{pack_directory, pack_directory_with_endian, unpack_to_dir};
pub use read::{read_arc, unpack};
pub use write::{file_alignment, write_arc, write_sarc};

// ---- SARC binary-format constants (shared by the reader and writer) ----
// Private to this module; child modules (`read`, `write`) reach them via
// `super::`.
const SARC_HEADER_SIZE: usize = 0x14;
const SFAT_HEADER_SIZE: usize = 0x0C;
const SFNT_HEADER_SIZE: usize = 0x08;
const SFAT_NODE_SIZE: usize = 0x10;
const SARC_HASH_KEY: u32 = 0x65;
const SFAT_HAS_NAME: u32 = 0x0100_0000;
/// Minimum and maximum alignment the writer will assign to any entry.
const MIN_ALIGNMENT: u32 = 0x08;
const MAX_ALIGNMENT: u32 = 0x2000;

/// A single unpacked SARC file: its archive-relative path and bytes.
#[derive(Debug, Clone)]
pub struct UnpackedFile {
    /// Archive-relative path, using `/` separators (e.g. `timg/__Combined.bntx`).
    pub name: String,
    /// File contents.
    pub data: Vec<u8>,
}

/// A single SARC entry preserving its (optional) name. Unlike
/// [`UnpackedFile`], hash-only entries (no stored name) survive a
/// [`read_arc`] → [`write_arc`] round-trip, so editing one named file in
/// an archive never silently drops the rest.
#[derive(Debug, Clone)]
pub struct ArcEntry {
    /// Archive-relative path (`/` separators), or `None` for a hash-only
    /// entry.
    pub name: Option<String>,
    pub data: Vec<u8>,
}

/// A full SARC archive parsed into memory, preserving every entry and the
/// endianness. Use [`read_arc`] / [`write_arc`] when you need to edit a
/// few files and re-pack the rest unchanged.
#[derive(Debug, Clone)]
pub struct ArcFile {
    pub big_endian: bool,
    pub files: Vec<ArcEntry>,
}

impl ArcFile {
    /// Index of the entry whose name equals `name`, if any.
    pub fn position(&self, name: &str) -> Option<usize> {
        self.files
            .iter()
            .position(|f| f.name.as_deref() == Some(name))
    }
}
