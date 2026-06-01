//! Internal verb: read an MC (`MCPK`) container, write it back verbatim, and
//! report whether the round-trip is byte-identical. Proves the header parser
//! walks the file without error and the no-op write is lossless — the safe
//! foundation before any decompress/repack. With `--dir`, sweeps every `.mc`
//! under a directory and tallies (for corpus auditing).

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::mc::{read_mc, write_mc};

#[derive(Parser, Debug)]
pub struct Args {
    /// A single `.mc` file.
    #[arg(short, long, conflicts_with = "dir")]
    input: Option<PathBuf>,

    /// Sweep every `*.mc` under this directory.
    #[arg(long)]
    dir: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    if let Some(dir) = &args.dir {
        return run_dir(dir);
    }
    let input = args
        .input
        .ok_or_else(|| anyhow!("provide --input <file> or --dir <directory>"))?;
    let bytes = std::fs::read(&input).with_context(|| format!("reading {}", input.display()))?;
    let mc = read_mc(&bytes).map_err(|e| anyhow!("{e}"))?;
    let written = write_mc(&mc);
    if written == bytes {
        println!(
            "OK: MCPK round-trip byte-identical ({} bytes, decompressed {} bytes, v{} flags={})",
            bytes.len(),
            mc.decompressed_size(),
            mc.header.version,
            mc.header.flags
        );
        Ok(ExitCode::SUCCESS)
    } else {
        let d = super::first_diff(&bytes, &written);
        println!("DIFF at 0x{d:x} (in={} out={})", bytes.len(), written.len());
        Ok(ExitCode::from(1))
    }
}

fn run_dir(dir: &std::path::Path) -> Result<ExitCode> {
    let mut seen = 0usize;
    let mut ok = 0usize;
    let mut failed = 0usize;
    let mut versions = std::collections::BTreeSet::new();
    let mut flagset = std::collections::BTreeSet::new();
    for entry in walkdir::WalkDir::new(dir).into_iter().flatten() {
        let p = entry.path();
        if !entry.file_type().is_file() {
            continue;
        }
        if p.extension().and_then(|e| e.to_str()) != Some("mc") {
            continue;
        }
        let Ok(bytes) = std::fs::read(p) else { continue };
        if bytes.get(0..4) != Some(b"MCPK".as_slice()) {
            continue;
        }
        seen += 1;
        match read_mc(&bytes) {
            Ok(mc) => {
                versions.insert(mc.header.version);
                flagset.insert(mc.header.flags);
                if write_mc(&mc) == bytes {
                    ok += 1;
                } else {
                    failed += 1;
                    eprintln!("DIFF: {}", p.display());
                }
            }
            Err(e) => {
                failed += 1;
                eprintln!("PARSE-FAIL: {} : {e}", p.display());
            }
        }
    }
    println!(
        "MCPK sweep: {seen} seen, {ok} byte-identical, {failed} failed; versions={versions:?} flags={flagset:?}"
    );
    if failed == 0 {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}
