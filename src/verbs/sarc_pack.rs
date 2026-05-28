use anyhow::{Context, Result};
use clap::Parser;
use sarc::{Endian, SarcEntry, SarcFile};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use walkdir::WalkDir;

#[derive(Parser, Debug)]
pub struct Args {
    /// Source directory.
    #[arg(short, long)]
    input: PathBuf,

    /// Output SARC path.
    #[arg(short, long)]
    out: PathBuf,

    /// Use big-endian SARC (Wii U / 3DS). Default is little-endian (Switch).
    #[arg(long)]
    big_endian: bool,
}

pub fn run(args: Args) -> Result<ExitCode> {
    if !args.input.is_dir() {
        anyhow::bail!("input directory not found: {}", args.input.display());
    }

    let mut entries = Vec::new();
    let root = args.input.canonicalize()?;
    for entry in WalkDir::new(&root).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path();
        let rel = abs
            .strip_prefix(&root)
            .with_context(|| "computing relative path")?
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        entries.push(SarcEntry {
            name: Some(rel),
            data: fs::read(abs)?,
        });
    }

    let endian = if args.big_endian {
        Endian::Big
    } else {
        Endian::Little
    };
    let sarc = SarcFile {
        byte_order: endian,
        files: entries,
    };
    let mut out = Vec::new();
    sarc.write(&mut out)
        .map_err(|e| anyhow::anyhow!("writing SARC: {}", e))?;
    if let Some(parent) = args.out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&args.out, &out)?;

    println!("packed {} files -> {}", sarc.files.len(), args.out.display());
    Ok(ExitCode::SUCCESS)
}
