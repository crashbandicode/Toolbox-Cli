//! `restbl-update-dir`: after repacking modded resources, bump their RESTBL
//! entries so the game allocates enough (under-allocation crashes; over-
//! allocation only wastes memory). Walks a mod folder, computes each resource's
//! decompressed size, and updates the RESTBL — **only ever growing** an entry.
//!
//! RESTBL stores a per-resource buffer size that includes substantial,
//! format-specific parse overhead (e.g. a BFRES entry is ~1.9x its padded
//! decompressed size). We never know the exact overhead formula, so:
//!
//! - With `--romfs-base <original romfs>` (recommended, accurate): the new size
//!   scales the original RESTBL entry by `new_decompressed / old_decompressed`,
//!   preserving the proven overhead ratio. Unchanged files keep their entry.
//! - Without a base (best-effort, over-allocates): `new = decompressed * ratio
//!   + const` (conservative, safe but wasteful). Verify in-game.
//!
//! The RESTBL key is the resource path with the compression extension stripped
//! (`Model/X.bfres.mc` -> `Model/X.bfres`), verified against the real table.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::compression::{self, DictRegistry};
use crate::mc::read_mc;
use crate::restbl::{read_restbl, write_restbl, Restbl};

#[derive(Parser, Debug)]
pub struct Args {
    /// Input RESTBL (`*.rsizetable[.zs]`).
    #[arg(long)]
    restbl: PathBuf,

    /// Mod folder to scan (romfs-relative resource paths are derived from it).
    #[arg(long)]
    dir: PathBuf,

    /// Original romfs root — enables accurate proportional overhead scaling.
    #[arg(long)]
    romfs_base: Option<PathBuf>,

    /// Output RESTBL path (written uncompressed).
    #[arg(short, long)]
    out: PathBuf,

    /// Insert resources not already present in the table.
    #[arg(long)]
    insert: bool,

    /// Fallback multiplier when no `--romfs-base` (conservative; safe-high).
    #[arg(long, default_value_t = 3.0)]
    ratio: f64,

    /// Fallback additive constant (bytes) when no `--romfs-base`.
    #[arg(long, default_value_t = 0x10000)]
    constant: u32,

    /// zstd dictionary pack for `.zs` inputs (or `--romfs`).
    #[arg(long)]
    dict: Option<PathBuf>,

    /// RomFS root; auto-finds `Pack/ZsDic.pack.zs` (for `.zs` decompression).
    #[arg(long)]
    romfs: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let dicts = super::load_dict_registry(args.dict.as_deref(), args.romfs.as_deref())?;

    let restbl_raw = std::fs::read(&args.restbl)
        .with_context(|| format!("reading {}", args.restbl.display()))?;
    let restbl_bytes = compression::decompress(&restbl_raw, &dicts).map_err(|e| anyhow!("{e}"))?;
    let mut table: Restbl = read_restbl(&restbl_bytes).map_err(|e| anyhow!("{e}"))?;

    if !args.dir.is_dir() {
        return Err(anyhow!("--dir {} is not a directory", args.dir.display()));
    }

    let mut grown = 0usize;
    let mut inserted = 0usize;
    let mut unchanged = 0usize;
    let mut skipped = 0usize;

    for entry in walkdir::WalkDir::new(&args.dir).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let rel = path
            .strip_prefix(&args.dir)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let key = strip_compression_ext(&rel);

        let new_dec = match decompressed_size(path, &dicts) {
            Ok(n) => n,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };

        let current = table.get_by_path(&key);
        let estimate = if let Some(base) = &args.romfs_base {
            match (current, original_decompressed(base, &rel, &dicts)) {
                (Some(old_restbl), Some(old_dec)) if old_dec > 0 => {
                    // Scale the proven overhead ratio by the size change.
                    let scaled = (old_restbl as u64 * new_dec as u64).div_ceil(old_dec as u64);
                    scaled.min(u32::MAX as u64) as u32
                }
                _ => conservative(new_dec, args.ratio, args.constant),
            }
        } else {
            conservative(new_dec, args.ratio, args.constant)
        };

        match current {
            Some(old) if estimate <= old => unchanged += 1,
            Some(_) => {
                table.set_by_path(&key, estimate, false);
                grown += 1;
                eprintln!("  grow  {key} -> {estimate}");
            }
            None if args.insert => {
                table.set_by_path(&key, estimate, true);
                inserted += 1;
                eprintln!("  add   {key} -> {estimate}");
            }
            None => skipped += 1,
        }
    }

    let out_bytes = write_restbl(&table).map_err(|e| anyhow!("{e}"))?;
    super::write_output(&args.out, &out_bytes)?;
    println!(
        "restbl-update-dir: {grown} grown, {inserted} inserted, {unchanged} unchanged, {skipped} skipped -> {} ({} bytes, {} CRC entries)",
        args.out.display(),
        out_bytes.len(),
        table.crc_entries.len()
    );
    if args.romfs_base.is_none() {
        eprintln!(
            "note: no --romfs-base — sizes are conservative over-estimates (ratio {} + {} bytes). \
             Pass --romfs-base for accurate proportional sizing. Verify in-game.",
            args.ratio, args.constant
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Conservative safe-high estimate when the original size isn't known.
fn conservative(decompressed: usize, ratio: f64, constant: u32) -> u32 {
    let est = (decompressed as f64 * ratio) as u64 + constant as u64;
    est.min(u32::MAX as u64) as u32
}

/// Strip a trailing compression extension to get the RESTBL resource key.
fn strip_compression_ext(rel: &str) -> String {
    for ext in [".mc", ".zs", ".szs"] {
        if let Some(stripped) = rel.strip_suffix(ext) {
            return stripped.to_string();
        }
    }
    rel.to_string()
}

/// The decompressed size of a resource file (the size that drives its RESTBL
/// allocation). `.mc` uses the MCPK header's declared size (no full decode);
/// compressed files are inflated; everything else is the file length.
fn decompressed_size(path: &Path, dicts: &DictRegistry) -> Result<usize> {
    let bytes = std::fs::read(path)?;
    if bytes.get(0..4) == Some(b"MCPK".as_slice()) {
        let mc = read_mc(&bytes).map_err(|e| anyhow!("{e}"))?;
        return Ok(mc.decompressed_size());
    }
    match compression::detect(&bytes) {
        compression::Codec::None => Ok(bytes.len()),
        _ => {
            let out = compression::decompress(&bytes, dicts).map_err(|e| anyhow!("{e}"))?;
            Ok(out.len())
        }
    }
}

/// The decompressed size of the *original* resource at `rel` under `base`, if it
/// exists (for proportional overhead scaling). `None` if absent/unreadable.
fn original_decompressed(base: &Path, rel: &str, dicts: &DictRegistry) -> Option<usize> {
    let p = base.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    if p.is_file() {
        return decompressed_size(&p, dicts).ok();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_only_compression_extensions() {
        assert_eq!(strip_compression_ext("Model/X.bfres.mc"), "Model/X.bfres");
        assert_eq!(strip_compression_ext("Pack/Y.pack.zs"), "Pack/Y.pack");
        assert_eq!(strip_compression_ext("Map/Z.byml.szs"), "Map/Z.byml");
        // Format extensions (not compression) are preserved.
        assert_eq!(strip_compression_ext("Model/X.bfres"), "Model/X.bfres");
        assert_eq!(strip_compression_ext("a/b.byml"), "a/b.byml");
    }

    #[test]
    fn conservative_is_monotonic_and_covers_overhead() {
        // BFRES overhead observed ~1.9x padded; the default 3x + 64KB exceeds it.
        let est = conservative(32768, 3.0, 0x10000);
        assert!(est >= 61408, "must exceed the real BFRES RESTBL size 61408");
        // Larger input -> larger estimate (monotonic).
        assert!(conservative(65536, 3.0, 0x10000) > est);
        // Tiny inputs still get the constant floor.
        assert!(conservative(10, 3.0, 0x10000) >= 0x10000);
    }
}
