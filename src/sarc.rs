//! SARC (Sead ARChive) pack/unpack helpers.
//!
//! Reading uses the [`sarc`](https://crates.io/crates/sarc) crate.
//! **Writing uses our own [`write_sarc`]** so we can give each file the
//! alignment it actually needs (the `sarc` crate pads every entry to
//! `0x2000`, which roughly doubles a real `layout.arc`). Switch titles
//! use little-endian SARC; pass `big_endian = true` for Wii U / 3DS.

use std::path::Path;

use sarc::{Endian, SarcFile};
use walkdir::WalkDir;

use crate::error::{Error, Result};

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

/// Parse a SARC archive into an [`ArcFile`], preserving all entries
/// (named and hash-only) and the byte order.
pub fn read_arc(bytes: &[u8]) -> Result<ArcFile> {
    let sarc = SarcFile::read(bytes).map_err(|e| Error::Sarc(format!("parsing SARC: {e:?}")))?;
    let big_endian = matches!(sarc.byte_order, Endian::Big);
    let files = sarc
        .files
        .into_iter()
        .map(|e| ArcEntry {
            name: e.name,
            data: e.data,
        })
        .collect();
    Ok(ArcFile { big_endian, files })
}

/// Serialize an [`ArcFile`] back to SARC bytes via [`write_sarc`]. Named
/// entries are re-hashed into the SFAT/SFNT tables; each file's data is
/// aligned to the boundary it requires (see [`file_alignment`]). Not
/// guaranteed byte-identical to the source, but a valid archive
/// containing every entry — including hash-only ones.
pub fn write_arc(arc: &ArcFile) -> Result<Vec<u8>> {
    write_sarc(&arc.files, arc.big_endian)
}

/// Pack every file under `dir` (recursively) into a little-endian Switch
/// SARC archive. Archive entry names are the paths relative to `dir` with
/// `/` separators.
pub fn pack_directory(dir: &Path) -> Result<Vec<u8>> {
    pack_directory_with_endian(dir, false)
}

/// Like [`pack_directory`] but lets you choose endianness
/// (`big_endian = true` for Wii U / 3DS archives).
pub fn pack_directory_with_endian(dir: &Path, big_endian: bool) -> Result<Vec<u8>> {
    if !dir.is_dir() {
        return Err(Error::Sarc(format!(
            "input directory not found: {}",
            dir.display()
        )));
    }
    let root = dir.canonicalize()?;
    let mut files = Vec::new();
    for entry in WalkDir::new(&root).follow_links(false) {
        let entry = entry.map_err(|e| Error::Sarc(format!("walking {}: {e}", root.display())))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path();
        let rel = abs
            .strip_prefix(&root)
            .map_err(|e| Error::Sarc(format!("relativizing {}: {e}", abs.display())))?
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        files.push(ArcEntry {
            name: Some(rel),
            data: std::fs::read(abs)?,
        });
    }
    // Stable order so a given directory always packs identically;
    // `write_sarc` re-sorts the SFAT by hash internally as the format
    // requires.
    files.sort_by(|a, b| a.name.cmp(&b.name));

    write_sarc(&files, big_endian)
}

/// Parse a SARC archive into its named files. Hash-only entries (without a
/// stored name) are skipped.
pub fn unpack(bytes: &[u8]) -> Result<Vec<UnpackedFile>> {
    let sarc = SarcFile::read(bytes).map_err(|e| Error::Sarc(format!("parsing SARC: {e:?}")))?;
    let mut out = Vec::with_capacity(sarc.files.len());
    for entry in sarc.files {
        if let Some(name) = entry.name {
            out.push(UnpackedFile {
                name,
                data: entry.data,
            });
        }
    }
    Ok(out)
}

/// Unpack a SARC archive, writing each named file under `out_dir`
/// (creating parent directories as needed). Returns the number of files
/// written. Hash-only entries are skipped.
pub fn unpack_to_dir(bytes: &[u8], out_dir: &Path) -> Result<usize> {
    let files = unpack(bytes)?;
    std::fs::create_dir_all(out_dir)?;
    let mut count = 0usize;
    for f in files {
        let rel = f.name.replace('/', std::path::MAIN_SEPARATOR_STR);
        let path = out_dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &f.data)?;
        count += 1;
    }
    Ok(count)
}

// ============================================================
// Native SARC writer (per-file alignment)
// ============================================================

const SARC_HEADER_SIZE: usize = 0x14;
const SFAT_HEADER_SIZE: usize = 0x0C;
const SFNT_HEADER_SIZE: usize = 0x08;
const SFAT_NODE_SIZE: usize = 0x10;
const SARC_HASH_KEY: u32 = 0x65;
const SFAT_HAS_NAME: u32 = 0x0100_0000;

/// Minimum and maximum alignment the writer will assign to any entry.
const MIN_ALIGNMENT: u32 = 0x08;
const MAX_ALIGNMENT: u32 = 0x2000;

/// SFAT name hash (the standard SARC multiply-add hash with key `0x65`).
fn sarc_hash(name: &str) -> u32 {
    name.bytes()
        .fold(0u32, |h, b| h.wrapping_mul(SARC_HASH_KEY).wrapping_add(b as u32))
}

fn align_up(value: usize, align: usize) -> usize {
    let a = align.max(1);
    value.div_ceil(a) * a
}

/// Derive the data alignment a file requires from its content.
///
/// Most Switch resources use the `nn::util::BinaryFileHeader` layout: an
/// 8-byte magic, a `u32` version, a `u16` byte-order mark at `0x0C`, then
/// a `u8` alignment exponent at `0x0E`. When that BOM is present we honor
/// `1 << exponent` (verified against fixtures: BNTX and BNSH report
/// `0x1000`). Cafe layout files (BFLYT/BFLAN — BOM at `0x04`, not `0x0C`)
/// and the custom `info` blob have no such field and only need the
/// minimum. Nested archives get `0x2000`; Yaz0-compressed payloads
/// `0x80`. The result is clamped to `[MIN_ALIGNMENT, MAX_ALIGNMENT]`.
pub fn file_alignment(data: &[u8]) -> u32 {
    let mut alignment = MIN_ALIGNMENT;

    if data.len() >= 4 {
        match &data[0..4] {
            b"SARC" => alignment = alignment.max(0x2000),
            b"Yaz0" | b"Yaz1" => alignment = alignment.max(0x80),
            _ => {}
        }
    }

    // nn::util::BinaryFileHeader: BOM at 0x0C, alignment exponent at 0x0E.
    if data.len() > 0x20 {
        let bom = (data[0x0C], data[0x0D]);
        let has_bom = bom == (0xFF, 0xFE) || bom == (0xFE, 0xFF);
        if has_bom {
            let exponent = data[0x0E];
            if exponent <= 13 {
                alignment = alignment.max(1u32 << exponent);
            }
        }
    }

    alignment.clamp(MIN_ALIGNMENT, MAX_ALIGNMENT)
}

fn push_u16(out: &mut Vec<u8>, v: u16, big_endian: bool) {
    if big_endian {
        out.extend_from_slice(&v.to_be_bytes());
    } else {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

fn push_u32(out: &mut Vec<u8>, v: u32, big_endian: bool) {
    if big_endian {
        out.extend_from_slice(&v.to_be_bytes());
    } else {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// Serialize SARC entries to a valid archive, giving each file the
/// alignment [`file_alignment`] derives. The SFAT is sorted by name hash
/// (as the format requires for the game's binary search); hash-only
/// (unnamed) entries are preserved with attrs `0` rather than being
/// collapsed. Endianness follows `big_endian` (false = little = Switch).
pub fn write_sarc(entries: &[ArcEntry], big_endian: bool) -> Result<Vec<u8>> {
    // SFAT must be ordered by name hash; a stable sort keeps the input
    // order for equal hashes (e.g. multiple unnamed entries).
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by_key(|&i| entries[i].name.as_deref().map(sarc_hash).unwrap_or(0));

    // Build the SFNT string table (named entries, in SFAT order) and
    // record each name's 4-byte-unit offset for its SFAT node.
    let mut name_bytes: Vec<u8> = Vec::new();
    let mut name_units: Vec<Option<u32>> = vec![None; entries.len()];
    for &i in &order {
        if let Some(name) = &entries[i].name {
            let offset = name_bytes.len();
            name_units[i] = Some((offset / 4) as u32);
            name_bytes.extend_from_slice(name.as_bytes());
            name_bytes.push(0);
            while !name_bytes.len().is_multiple_of(4) {
                name_bytes.push(0);
            }
        }
    }

    let node_count = entries.len();
    let pre_data = SARC_HEADER_SIZE
        + SFAT_HEADER_SIZE
        + node_count * SFAT_NODE_SIZE
        + SFNT_HEADER_SIZE
        + name_bytes.len();

    let max_alignment = order
        .iter()
        .map(|&i| file_alignment(&entries[i].data))
        .max()
        .unwrap_or(MIN_ALIGNMENT)
        .max(MIN_ALIGNMENT) as usize;
    let data_offset = align_up(pre_data, max_alignment);

    // Lay out the data section (in SFAT order), aligning each file.
    let mut data_start = vec![0u32; entries.len()];
    let mut data_end = vec![0u32; entries.len()];
    let mut cursor = 0usize; // relative to data_offset
    for &i in &order {
        let align = file_alignment(&entries[i].data) as usize;
        let start = align_up(cursor, align);
        data_start[i] = start as u32;
        data_end[i] = (start + entries[i].data.len()) as u32;
        cursor = start + entries[i].data.len();
    }
    let file_size = data_offset + cursor;

    let mut out = Vec::with_capacity(file_size);

    // ---- SARC header ----
    out.extend_from_slice(b"SARC");
    push_u16(&mut out, SARC_HEADER_SIZE as u16, big_endian);
    // BOM 0xFEFF written in the file's endianness (LE → FF FE), which the
    // reader interprets big-endian to recover the byte order.
    push_u16(&mut out, 0xFEFF, big_endian);
    push_u32(&mut out, file_size as u32, big_endian);
    push_u32(&mut out, data_offset as u32, big_endian);
    push_u16(&mut out, 0x0100, big_endian); // version
    push_u16(&mut out, 0, big_endian); // reserved

    // ---- SFAT ----
    out.extend_from_slice(b"SFAT");
    push_u16(&mut out, SFAT_HEADER_SIZE as u16, big_endian);
    push_u16(&mut out, node_count as u16, big_endian);
    push_u32(&mut out, SARC_HASH_KEY, big_endian);
    for &i in &order {
        let hash = entries[i].name.as_deref().map(sarc_hash).unwrap_or(0);
        let attrs = match name_units[i] {
            Some(units) => SFAT_HAS_NAME | units,
            None => 0,
        };
        push_u32(&mut out, hash, big_endian);
        push_u32(&mut out, attrs, big_endian);
        push_u32(&mut out, data_start[i], big_endian);
        push_u32(&mut out, data_end[i], big_endian);
    }

    // ---- SFNT ----
    out.extend_from_slice(b"SFNT");
    push_u16(&mut out, SFNT_HEADER_SIZE as u16, big_endian);
    push_u16(&mut out, 0, big_endian);
    out.extend_from_slice(&name_bytes);

    // ---- pad to data_offset, then the data section ----
    out.resize(data_offset, 0);
    for &i in &order {
        let abs = data_offset + data_start[i] as usize;
        out.resize(abs, 0); // per-file alignment padding
        out.extend_from_slice(&entries[i].data);
    }
    debug_assert_eq!(out.len(), file_size);

    Ok(out)
}
