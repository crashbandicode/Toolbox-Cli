//! SARC (Sead ARChive) pack/unpack helpers.
//!
//! Both the reader ([`parse_sarc`], via [`read_arc`] / [`unpack`]) and the
//! writer ([`write_sarc`]) are native implementations — no third-party
//! SARC crate. The writer gives each file the alignment it actually needs
//! (a generic writer pads every entry to `0x2000`, which roughly doubles a
//! real `layout.arc`). Switch titles use little-endian SARC; pass
//! `big_endian = true` for Wii U / 3DS (the reader auto-detects via the
//! header BOM).

use std::path::Path;

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
    parse_sarc(bytes)
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
    let arc = parse_sarc(bytes)?;
    let mut out = Vec::with_capacity(arc.files.len());
    for entry in arc.files {
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
// Native SARC reader
// ============================================================

fn read_u16(bytes: &[u8], off: usize, big_endian: bool) -> Result<u16> {
    let b = bytes
        .get(off..off + 2)
        .ok_or_else(|| Error::Sarc(format!("truncated SARC: need u16 at 0x{off:x}")))?;
    Ok(if big_endian {
        u16::from_be_bytes([b[0], b[1]])
    } else {
        u16::from_le_bytes([b[0], b[1]])
    })
}

fn read_u32(bytes: &[u8], off: usize, big_endian: bool) -> Result<u32> {
    let b = bytes
        .get(off..off + 4)
        .ok_or_else(|| Error::Sarc(format!("truncated SARC: need u32 at 0x{off:x}")))?;
    Ok(if big_endian {
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    } else {
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    })
}

/// Read a NUL-terminated UTF-8 string starting at `off`. SARC entry names
/// are always ASCII paths, so strict UTF-8 both succeeds in practice and
/// guarantees the name re-encodes to identical bytes.
fn read_c_string(bytes: &[u8], off: usize) -> Result<String> {
    let slice = bytes
        .get(off..)
        .ok_or_else(|| Error::Sarc(format!("SARC name offset 0x{off:x} out of bounds")))?;
    let end = slice
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| Error::Sarc(format!("unterminated SARC name at 0x{off:x}")))?;
    std::str::from_utf8(&slice[..end])
        .map(|s| s.to_owned())
        .map_err(|e| Error::Sarc(format!("non-UTF8 SARC name at 0x{off:x}: {e}")))
}

/// Parse a SARC archive into an [`ArcFile`], preserving every entry (named
/// and hash-only) and the byte order.
///
/// Layout: a `0x14` header (`SARC` magic, header size, BOM, file size,
/// data offset, version), then an `SFAT` table of fixed `0x10` nodes
/// (hash, attrs, data start/end relative to `data_offset`), then an `SFNT`
/// name blob. A node owns a name when `attrs & SFAT_HAS_NAME` is set; the
/// low 24 bits are the name's offset in 4-byte units. Every access is
/// bounds-checked so a malformed/truncated archive errors cleanly rather
/// than panicking.
fn parse_sarc(bytes: &[u8]) -> Result<ArcFile> {
    if bytes.len() < SARC_HEADER_SIZE {
        return Err(Error::Sarc(format!(
            "too small to be a SARC ({} bytes)",
            bytes.len()
        )));
    }
    if &bytes[0..4] != b"SARC" {
        return Err(Error::Sarc(format!("bad SARC magic: {:02x?}", &bytes[0..4])));
    }
    // BOM at 0x06 selects endianness: written as 0xFEFF in the file's own
    // byte order, so the bytes are FE FF for big-endian and FF FE for
    // little-endian (Switch).
    let big_endian = match (bytes[0x06], bytes[0x07]) {
        (0xFE, 0xFF) => true,
        (0xFF, 0xFE) => false,
        other => return Err(Error::Sarc(format!("bad SARC BOM: {other:02x?}"))),
    };
    let data_offset = read_u32(bytes, 0x0C, big_endian)? as usize;

    // ---- SFAT ----
    let sfat_off = SARC_HEADER_SIZE;
    if bytes.get(sfat_off..sfat_off + 4) != Some(b"SFAT".as_slice()) {
        return Err(Error::Sarc("missing SFAT header".into()));
    }
    let node_count = read_u16(bytes, sfat_off + 0x06, big_endian)? as usize;
    let nodes_off = sfat_off + SFAT_HEADER_SIZE;
    let nodes_end = nodes_off + node_count * SFAT_NODE_SIZE;

    // ---- SFNT (name blob follows the node table) ----
    if bytes.get(nodes_end..nodes_end + 4) != Some(b"SFNT".as_slice()) {
        return Err(Error::Sarc(
            "missing SFNT header (SFAT node count past end?)".into(),
        ));
    }
    let names_off = nodes_end + SFNT_HEADER_SIZE;

    let mut files = Vec::with_capacity(node_count);
    for i in 0..node_count {
        let node = nodes_off + i * SFAT_NODE_SIZE;
        let attrs = read_u32(bytes, node + 0x04, big_endian)?;
        let start = read_u32(bytes, node + 0x08, big_endian)? as usize;
        let end = read_u32(bytes, node + 0x0C, big_endian)? as usize;
        if end < start {
            return Err(Error::Sarc(format!(
                "SARC node {i}: data end 0x{end:x} precedes start 0x{start:x}"
            )));
        }
        let abs_start = data_offset + start;
        let abs_end = data_offset + end;
        let data = bytes
            .get(abs_start..abs_end)
            .ok_or_else(|| {
                Error::Sarc(format!(
                    "SARC node {i}: data 0x{abs_start:x}..0x{abs_end:x} out of bounds (len 0x{:x})",
                    bytes.len()
                ))
            })?
            .to_vec();
        let name = if attrs & SFAT_HAS_NAME != 0 {
            let name_off = names_off + (attrs & 0x00FF_FFFF) as usize * 4;
            Some(read_c_string(bytes, name_off)?)
        } else {
            None
        };
        files.push(ArcEntry { name, data });
    }

    Ok(ArcFile { big_endian, files })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: Option<&str>, data: &[u8]) -> ArcEntry {
        ArcEntry {
            name: name.map(str::to_owned),
            data: data.to_vec(),
        }
    }

    #[test]
    fn writer_reader_round_trip_named_and_hash_only() {
        // Mix named entries with a hash-only one and a fake BNTX (which the
        // writer forces onto 0x1000) to exercise per-file alignment + the
        // name/hash-only branches of the reader.
        let mut bntx = b"BNTX".to_vec();
        bntx.extend_from_slice(&[0u8; 0x40]);
        let entries = vec![
            entry(Some("a/first.bin"), b"hello world"),
            entry(Some("b/second.txt"), &[0xAB; 37]),
            entry(None, b"\x01\x02\x03\x04unnamed payload"),
            entry(Some("timg/__Combined.bntx"), &bntx),
        ];

        let packed = write_sarc(&entries, false).expect("write");
        let arc = parse_sarc(&packed).expect("parse");

        assert!(!arc.big_endian);
        assert_eq!(arc.files.len(), entries.len(), "entry count preserved");

        // Every input entry (matched by name, or the single hash-only one)
        // round-trips its exact bytes.
        for src in &entries {
            match &src.name {
                Some(name) => {
                    let got = arc
                        .files
                        .iter()
                        .find(|f| f.name.as_deref() == Some(name.as_str()))
                        .unwrap_or_else(|| panic!("missing {name}"));
                    assert_eq!(got.data, src.data, "data for {name}");
                }
                None => {
                    assert!(
                        arc.files.iter().any(|f| f.name.is_none() && f.data == src.data),
                        "hash-only entry not preserved"
                    );
                }
            }
        }
    }

    #[test]
    fn round_trip_big_endian() {
        let entries = vec![entry(Some("x"), b"wii-u-style"), entry(Some("y"), b"BE")];
        let packed = write_sarc(&entries, true).expect("write BE");
        // BOM bytes are FE FF for big-endian.
        assert_eq!((packed[0x06], packed[0x07]), (0xFE, 0xFF));
        let arc = parse_sarc(&packed).expect("parse BE");
        assert!(arc.big_endian);
        assert_eq!(arc.files.len(), 2);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_sarc(b"not a sarc at all").is_err());
        assert!(parse_sarc(&[]).is_err());
        // Right magic, truncated before SFAT.
        assert!(parse_sarc(b"SARC\x14\x00\xFF\xFE").is_err());
    }
}
