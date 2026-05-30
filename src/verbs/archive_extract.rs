//! `archive-extract`: decompress (if needed) and unpack a SARC archive to a
//! directory tree. Handles a plain `.arc`/`.sarc`, a zstd-compressed
//! `.pack.zs`/`.blarc.zs`, or a Yaz0 `.szs`, and inflates any compressed
//! entries inside (stripping their `.zs`/`.szs` suffix). Nested SARC entries
//! are written out decompressed; re-run `archive-extract` on them to descend
//! further.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::compression;
use crate::sarc;

#[derive(Parser, Debug)]
pub struct Args {
    /// Archive to extract (`.arc`, `.pack.zs`, `.blarc.zs`, `.szs`, …).
    #[arg(short, long)]
    input: PathBuf,

    /// Output directory (created if missing).
    #[arg(short, long)]
    out: PathBuf,

    /// TotK `ZsDic.pack.zs` (or a directory of `*.zsdic`) for
    /// dictionary-compressed `.zs` data.
    #[arg(long)]
    dict: Option<PathBuf>,

    /// TotK RomFS root; auto-finds `Pack/ZsDic.pack.zs`.
    #[arg(long)]
    romfs: Option<PathBuf>,

    /// Continue past entries that fail to decompress (e.g. a missing
    /// dictionary) instead of aborting.
    #[arg(long)]
    keep_going: bool,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bytes =
        std::fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;

    let dicts = super::load_dict_registry(args.dict.as_deref(), args.romfs.as_deref())?;

    // Top-level: decompress if the file is itself compressed.
    let top = compression::decompress(&bytes, &dicts).map_err(|e| anyhow!("{e}"))?;
    if top.len() < 4 || &top[0..4] != b"SARC" {
        return Err(anyhow!(
            "{} is not a SARC archive after decompression — use `decompress` for plain files",
            args.input.display()
        ));
    }

    let arc = sarc::unpack(&top).map_err(|e| anyhow!("{e}"))?;

    let mut written = 0usize;
    let mut inflated = 0usize;
    let mut nested = 0usize;
    for entry in &arc {
        let (data, rel) = if compression::detect(&entry.data).is_compressed() {
            match compression::decompress(&entry.data, &dicts) {
                Ok(d) => {
                    inflated += 1;
                    (d.into_owned(), strip_compression_suffix(&entry.name))
                }
                Err(e) => {
                    if args.keep_going {
                        eprintln!("  warning: skipping {} ({e})", entry.name);
                        continue;
                    }
                    return Err(anyhow!("decompressing entry {}: {e}", entry.name));
                }
            }
        } else {
            (entry.data.clone(), entry.name.clone())
        };

        if data.len() >= 4 && &data[0..4] == b"SARC" {
            nested += 1;
        }

        let target = safe_join(&args.out, &rel)?;
        super::write_output(&target, &data)?;
        written += 1;
    }

    let note = if nested > 0 {
        format!(
            "; {nested} nested archive(s) written decompressed (re-run archive-extract to descend)"
        )
    } else {
        String::new()
    };
    println!(
        "extracted {written} file(s) ({inflated} inflated) -> {}{note}",
        args.out.display()
    );
    Ok(ExitCode::SUCCESS)
}

/// Drop a trailing compression suffix from an entry name so an inflated
/// `foo.bntx.zs` lands as `foo.bntx`.
fn strip_compression_suffix(name: &str) -> String {
    for suffix in [".zs", ".zst", ".szs"] {
        if let Some(base) = name.strip_suffix(suffix) {
            return base.to_string();
        }
    }
    name.to_string()
}

/// Join an archive-relative entry name under `base`, rejecting any `..`
/// component so a malicious archive can't escape the output directory.
fn safe_join(base: &Path, rel: &str) -> Result<PathBuf> {
    let mut path = base.to_path_buf();
    for comp in rel.split(['/', '\\']) {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp == ".." {
            return Err(anyhow!("refusing path traversal in entry name: {rel}"));
        }
        path.push(comp);
    }
    Ok(path)
}
