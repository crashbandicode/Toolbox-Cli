//! RESTBL (Resource Size Table) read + write + update.
//!
//! The Resource Size Table tells the game how much memory to reserve when
//! loading each resource; if a modified file is bigger than its recorded size,
//! the game crashes. Repacking BOTW/TotK mods therefore requires updating this
//! table. Tears of the Kingdom ships it as
//! `System/Resource/ResourceSizeTable.Product.NNN.rsizetable.zs` (zstd).
//!
//! ## Format (`RESTBL`, version 1)
//!
//! A fixed, fully-deterministic layout, so [`write_restbl`] reproduces an
//! unmodified table **byte-identically**:
//!
//! - 22-byte header: magic `"RESTBL"` (6) + `version: u32` (1) +
//!   `string_block_size: u32` (160 in TotK) + `crc_table_num: u32` +
//!   `name_table_num: u32`.
//! - CRC table: `crc_table_num` × `{ hash: u32, size: u32 }`, **sorted by
//!   `hash`** (binary-searchable). `hash` is the CRC-32 of the resource path.
//! - Name table: `name_table_num` × `{ name: char[string_block_size]
//!   (NUL-padded), size: u32 }`, **sorted by `name`** — the collision /
//!   overflow list for paths whose CRC-32 clashes.
//!
//! BOTW's older `RSTB` magic uses a different header (no version /
//! `string_block_size`; 128-byte names); only the TotK `RESTBL` form is
//! implemented here (it's the one we have real fixtures for).

use thiserror::Error;

/// The 6-byte RESTBL magic.
pub const RESTBL_MAGIC: &[u8; 6] = b"RESTBL";

const HEADER_SIZE: usize = 22;

/// An error reading, writing, or updating a RESTBL.
#[derive(Debug, Error)]
pub enum RestblError {
    /// Buffer is smaller than the 22-byte header.
    #[error("not a RESTBL: only {0} byte(s), need at least a 22-byte header")]
    TooSmall(usize),

    /// The 6-byte magic was not `RESTBL`.
    #[error("bad RESTBL magic {0:02x?} (expected \"RESTBL\")")]
    BadMagic([u8; 6]),

    /// `string_block_size` was zero or implausibly large.
    #[error("implausible RESTBL string_block_size {0}")]
    BadStringBlockSize(u32),

    /// The declared tables run past the end of the buffer.
    #[error("truncated RESTBL: need {need} byte(s) at offset 0x{offset:x} (file is 0x{len:x})")]
    Truncated {
        offset: usize,
        need: usize,
        len: usize,
    },

    /// A name-table entry was not valid UTF-8 (real paths are ASCII).
    #[error("RESTBL name entry {index} is not valid UTF-8: {source}")]
    NonUtf8Name {
        index: usize,
        source: std::str::Utf8Error,
    },

    /// A name doesn't fit in `string_block_size` bytes (need room for a NUL).
    #[error("RESTBL name {name:?} does not fit in a {max}-byte block (incl. NUL terminator)")]
    NameTooLong { name: String, max: usize },
}

/// Convenience alias for the RESTBL module's fallible operations.
pub type Result<T> = std::result::Result<T, RestblError>;

/// A CRC-table entry: the CRC-32 of a resource path and its reserved size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrcEntry {
    pub hash: u32,
    pub size: u32,
}

/// A name-table (collision overflow) entry: the full resource path and size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameEntry {
    pub name: String,
    pub size: u32,
}

/// The outcome of a [`Restbl::set_by_path`] update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetOutcome {
    /// An existing CRC-table entry's size was updated.
    UpdatedCrc,
    /// An existing name-table entry's size was updated.
    UpdatedName,
    /// A new CRC-table entry was inserted (sorted).
    Inserted,
    /// The path wasn't present and insertion wasn't requested.
    NotFound,
}

/// A parsed Resource Size Table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restbl {
    /// Format version (1 for TotK).
    pub version: u32,
    /// Fixed byte length of each name block (160 in TotK).
    pub string_block_size: u32,
    /// CRC table, sorted ascending by [`CrcEntry::hash`].
    pub crc_entries: Vec<CrcEntry>,
    /// Name (collision) table, sorted ascending by [`NameEntry::name`].
    pub name_entries: Vec<NameEntry>,
}

impl Restbl {
    /// Reserved size for a CRC-32 hash (binary search; the CRC table is
    /// sorted).
    pub fn get_by_hash(&self, hash: u32) -> Option<u32> {
        self.crc_entries
            .binary_search_by(|e| e.hash.cmp(&hash))
            .ok()
            .map(|i| self.crc_entries[i].size)
    }

    /// Update an existing CRC entry's size. Returns `true` if found.
    pub fn set_by_hash(&mut self, hash: u32, size: u32) -> bool {
        match self.crc_entries.binary_search_by(|e| e.hash.cmp(&hash)) {
            Ok(i) => {
                self.crc_entries[i].size = size;
                true
            }
            Err(_) => false,
        }
    }

    /// Insert a CRC entry (keeping the table sorted), or update it if the hash
    /// already exists.
    pub fn insert_by_hash(&mut self, hash: u32, size: u32) {
        match self.crc_entries.binary_search_by(|e| e.hash.cmp(&hash)) {
            Ok(i) => self.crc_entries[i].size = size,
            Err(i) => self.crc_entries.insert(i, CrcEntry { hash, size }),
        }
    }

    /// Reserved size for a resource path in the name (collision) table.
    pub fn get_by_name(&self, name: &str) -> Option<u32> {
        self.name_entries
            .binary_search_by(|e| e.name.as_str().cmp(name))
            .ok()
            .map(|i| self.name_entries[i].size)
    }

    /// Update an existing name-table entry's size. Returns `true` if found.
    pub fn set_by_name(&mut self, name: &str, size: u32) -> bool {
        match self
            .name_entries
            .binary_search_by(|e| e.name.as_str().cmp(name))
        {
            Ok(i) => {
                self.name_entries[i].size = size;
                true
            }
            Err(_) => false,
        }
    }

    /// Insert a name-table entry (keeping the table sorted by name), or update
    /// it if the name already exists.
    pub fn insert_by_name(&mut self, name: &str, size: u32) {
        match self
            .name_entries
            .binary_search_by(|e| e.name.as_str().cmp(name))
        {
            Ok(i) => self.name_entries[i].size = size,
            Err(i) => self.name_entries.insert(
                i,
                NameEntry {
                    name: name.to_string(),
                    size,
                },
            ),
        }
    }

    /// Reserved size for a resource path: checks the CRC table first (by
    /// `crc32(path)`), then the name table.
    pub fn get_by_path(&self, path: &str) -> Option<u32> {
        self.get_by_hash(crc32(path.as_bytes()))
            .or_else(|| self.get_by_name(path))
    }

    /// Set the reserved size for a resource path. Updates the CRC entry if its
    /// `crc32(path)` is present, otherwise the name entry if `path` is present,
    /// otherwise inserts a new CRC entry when `allow_insert` is set.
    pub fn set_by_path(&mut self, path: &str, size: u32, allow_insert: bool) -> SetOutcome {
        let hash = crc32(path.as_bytes());
        if self.set_by_hash(hash, size) {
            return SetOutcome::UpdatedCrc;
        }
        if self.set_by_name(path, size) {
            return SetOutcome::UpdatedName;
        }
        if allow_insert {
            self.insert_by_hash(hash, size);
            return SetOutcome::Inserted;
        }
        SetOutcome::NotFound
    }
}

/// Parse a RESTBL from bytes.
pub fn read_restbl(data: &[u8]) -> Result<Restbl> {
    if data.len() < HEADER_SIZE {
        return Err(RestblError::TooSmall(data.len()));
    }
    if &data[0..6] != RESTBL_MAGIC {
        let mut m = [0u8; 6];
        m.copy_from_slice(&data[0..6]);
        return Err(RestblError::BadMagic(m));
    }
    let version = read_u32(data, 6);
    let string_block_size = read_u32(data, 10);
    let crc_num = read_u32(data, 14) as usize;
    let name_num = read_u32(data, 18) as usize;

    let sbs = string_block_size as usize;
    if sbs == 0 || sbs > 4096 {
        return Err(RestblError::BadStringBlockSize(string_block_size));
    }
    let name_entry_size = sbs + 4;

    // Bounds-check the whole declared layout up front (overflow-safe).
    let crc_bytes = crc_num.checked_mul(8);
    let name_bytes = name_num.checked_mul(name_entry_size);
    let total = crc_bytes
        .zip(name_bytes)
        .and_then(|(c, n)| HEADER_SIZE.checked_add(c).and_then(|x| x.checked_add(n)));
    let Some(total) = total else {
        return Err(RestblError::Truncated {
            offset: HEADER_SIZE,
            need: usize::MAX,
            len: data.len(),
        });
    };
    if data.len() < total {
        return Err(RestblError::Truncated {
            offset: HEADER_SIZE,
            need: total - HEADER_SIZE,
            len: data.len(),
        });
    }

    let mut crc_entries = Vec::with_capacity(crc_num);
    for i in 0..crc_num {
        let o = HEADER_SIZE + i * 8;
        crc_entries.push(CrcEntry {
            hash: read_u32(data, o),
            size: read_u32(data, o + 4),
        });
    }

    let name_base = HEADER_SIZE + crc_num * 8;
    let mut name_entries = Vec::with_capacity(name_num);
    for i in 0..name_num {
        let o = name_base + i * name_entry_size;
        let block = &data[o..o + sbs];
        let nul = block.iter().position(|&b| b == 0).unwrap_or(sbs);
        let name = std::str::from_utf8(&block[..nul])
            .map_err(|e| RestblError::NonUtf8Name {
                index: i,
                source: e,
            })?
            .to_string();
        name_entries.push(NameEntry {
            name,
            size: read_u32(data, o + sbs),
        });
    }

    Ok(Restbl {
        version,
        string_block_size,
        crc_entries,
        name_entries,
    })
}

/// Serialize a RESTBL to bytes. Byte-identical to the source for an unmodified
/// [`Restbl`] (the format is fully deterministic).
pub fn write_restbl(r: &Restbl) -> Result<Vec<u8>> {
    let sbs = r.string_block_size as usize;
    let mut buf = Vec::with_capacity(
        HEADER_SIZE + r.crc_entries.len() * 8 + r.name_entries.len() * (sbs + 4),
    );
    buf.extend_from_slice(RESTBL_MAGIC);
    buf.extend_from_slice(&r.version.to_le_bytes());
    buf.extend_from_slice(&r.string_block_size.to_le_bytes());
    buf.extend_from_slice(&(r.crc_entries.len() as u32).to_le_bytes());
    buf.extend_from_slice(&(r.name_entries.len() as u32).to_le_bytes());

    for e in &r.crc_entries {
        buf.extend_from_slice(&e.hash.to_le_bytes());
        buf.extend_from_slice(&e.size.to_le_bytes());
    }
    for e in &r.name_entries {
        let nb = e.name.as_bytes();
        if nb.len() >= sbs {
            return Err(RestblError::NameTooLong {
                name: e.name.clone(),
                max: sbs,
            });
        }
        buf.extend_from_slice(nb);
        buf.resize(buf.len() + (sbs - nb.len()), 0);
        buf.extend_from_slice(&e.size.to_le_bytes());
    }
    Ok(buf)
}

/// Standard CRC-32 (IEEE 802.3 / zlib: reflected, poly `0xEDB88320`, init/xor
/// `0xFFFFFFFF`) — the hash BOTW/TotK use for resource paths.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn read_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_check_value() {
        // The canonical CRC-32 check value for "123456789".
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    fn sample() -> Restbl {
        Restbl {
            version: 1,
            string_block_size: 160,
            crc_entries: vec![
                CrcEntry {
                    hash: 0x10,
                    size: 100,
                },
                CrcEntry {
                    hash: 0x20,
                    size: 200,
                },
                CrcEntry {
                    hash: 0x30,
                    size: 300,
                },
            ],
            name_entries: vec![
                NameEntry {
                    name: "Actor/A.bgyml".into(),
                    size: 10,
                },
                NameEntry {
                    name: "Actor/B.bgyml".into(),
                    size: 20,
                },
            ],
        }
    }

    #[test]
    fn round_trip_is_byte_identical() {
        let r = sample();
        let bytes = write_restbl(&r).unwrap();
        // 22 header + 3*8 crc + 2*164 name.
        assert_eq!(bytes.len(), 22 + 24 + 328);
        let back = read_restbl(&bytes).unwrap();
        assert_eq!(back, r);
        // Re-serialize is identical.
        assert_eq!(write_restbl(&back).unwrap(), bytes);
    }

    #[test]
    fn get_set_insert_by_hash() {
        let mut r = sample();
        assert_eq!(r.get_by_hash(0x20), Some(200));
        assert_eq!(r.get_by_hash(0x25), None);
        assert!(r.set_by_hash(0x20, 999));
        assert_eq!(r.get_by_hash(0x20), Some(999));
        assert!(!r.set_by_hash(0x25, 1));
        // Insert keeps the table sorted.
        r.insert_by_hash(0x25, 250);
        assert_eq!(r.get_by_hash(0x25), Some(250));
        assert!(r.crc_entries.windows(2).all(|w| w[0].hash < w[1].hash));
    }

    #[test]
    fn get_set_by_name() {
        let mut r = sample();
        assert_eq!(r.get_by_name("Actor/B.bgyml"), Some(20));
        assert_eq!(r.get_by_name("Actor/Z.bgyml"), None);
        assert!(r.set_by_name("Actor/A.bgyml", 11));
        assert_eq!(r.get_by_name("Actor/A.bgyml"), Some(11));
    }

    #[test]
    fn set_by_path_outcomes() {
        let mut r = sample();
        let path = "Pack/Actor/Test.pack.zs";
        let h = crc32(path.as_bytes());
        // Not present, no insert -> NotFound.
        assert_eq!(r.set_by_path(path, 4096, false), SetOutcome::NotFound);
        // With insert -> Inserted, then resolvable.
        assert_eq!(r.set_by_path(path, 4096, true), SetOutcome::Inserted);
        assert_eq!(r.get_by_hash(h), Some(4096));
        assert_eq!(r.get_by_path(path), Some(4096));
        // Second set updates the CRC entry.
        assert_eq!(r.set_by_path(path, 8192, false), SetOutcome::UpdatedCrc);
        assert_eq!(r.get_by_path(path), Some(8192));
        // A path present only in the name table updates there.
        assert_eq!(
            r.set_by_path("Actor/A.bgyml", 12, false),
            SetOutcome::UpdatedName
        );
        assert_eq!(r.get_by_name("Actor/A.bgyml"), Some(12));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(matches!(
            read_restbl(&[0u8; 4]),
            Err(RestblError::TooSmall(4))
        ));
        let mut bad = vec![0u8; 22];
        bad[0..6].copy_from_slice(b"NOPEZZ");
        assert!(matches!(read_restbl(&bad), Err(RestblError::BadMagic(_))));
    }

    /// Mutation diff-shape: `set_by_hash` changes *only* the targeted entry's
    /// size; every other CRC entry (hash + size + order) and the whole name
    /// table stay byte-stable, and a miss is a no-op.
    #[test]
    fn set_only_changes_target_entry() {
        let mut r = sample();
        let before = r.clone();
        assert!(r.set_by_hash(0x20, 999));
        assert_eq!(r.crc_entries.len(), before.crc_entries.len());
        for (after, orig) in r.crc_entries.iter().zip(&before.crc_entries) {
            assert_eq!(after.hash, orig.hash, "hash/order must stay stable");
            if after.hash == 0x20 {
                assert_eq!((orig.size, after.size), (200, 999), "target updated");
            } else {
                assert_eq!(
                    after.size, orig.size,
                    "unrelated entry 0x{:x} changed",
                    after.hash
                );
            }
        }
        assert_eq!(r.name_entries, before.name_entries, "name table untouched");

        // A set targeting a missing hash mutates nothing at all.
        let snapshot = r.clone();
        assert!(!r.set_by_hash(0x9999, 1));
        assert_eq!(r, snapshot, "a missed set must be a no-op");
    }
}
