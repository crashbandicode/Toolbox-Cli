//! Filesystem helpers: pack a directory tree into a SARC and unpack a SARC
//! to disk. These are the **only** parts of the SARC module that touch the
//! filesystem (`std::fs`) or `walkdir`; the reader/writer core is pure
//! in-memory `std`. When this module is lifted into a standalone crate,
//! these would sit behind an optional `fs` feature.

use std::path::Path;

use walkdir::WalkDir;

use super::error::{Result, SarcError};
use super::write::write_sarc;
use super::{read::unpack, ArcEntry};

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
        return Err(SarcError::Fs(format!(
            "input directory not found: {}",
            dir.display()
        )));
    }
    let root = dir.canonicalize()?;
    let mut files = Vec::new();
    for entry in WalkDir::new(&root).follow_links(false) {
        let entry = entry.map_err(|e| SarcError::Fs(format!("walking {}: {e}", root.display())))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path();
        let rel = abs
            .strip_prefix(&root)
            .map_err(|e| SarcError::Fs(format!("relativizing {}: {e}", abs.display())))?
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
