//! SARC (Sead ARChive) pack/unpack helpers.
//!
//! Thin, typed wrappers over the [`sarc`](https://crates.io/crates/sarc)
//! crate that the CLI and library consumers (e.g. SGPO) share. Switch
//! titles use little-endian SARC; pass `big_endian = true` for Wii U / 3DS.

use std::path::Path;

use sarc::{Endian, SarcEntry, SarcFile};
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
        files.push(SarcEntry {
            name: Some(rel),
            data: std::fs::read(abs)?,
        });
    }

    let sarc = SarcFile {
        byte_order: if big_endian {
            Endian::Big
        } else {
            Endian::Little
        },
        files,
    };
    let mut out = Vec::new();
    sarc.write(&mut out)
        .map_err(|e| Error::Sarc(format!("writing SARC: {e}")))?;
    Ok(out)
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
