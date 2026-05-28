use anyhow::{Context, Result};
use clap::Parser;
use sarc::SarcFile;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
pub struct Args {
    /// Path to the SARC archive.
    #[arg(short, long)]
    input: PathBuf,

    /// Output directory (created if missing).
    #[arg(short, long)]
    out: PathBuf,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bytes = fs::read(&args.input)
        .with_context(|| format!("reading {}", args.input.display()))?;
    let sarc = SarcFile::read(&bytes)
        .map_err(|e| anyhow::anyhow!("parsing SARC: {:?}", e))?;

    fs::create_dir_all(&args.out)
        .with_context(|| format!("creating {}", args.out.display()))?;

    let mut count = 0usize;
    for entry in sarc.files {
        let name = match entry.name {
            Some(n) => n,
            None => {
                eprintln!("warning: hash-only SARC entry skipped (no name)");
                continue;
            }
        };
        let rel = name.replace('/', std::path::MAIN_SEPARATOR_STR);
        let path = args.out.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &entry.data)?;
        count += 1;
    }
    println!("unpacked {count} files -> {}", args.out.display());
    Ok(ExitCode::SUCCESS)
}
