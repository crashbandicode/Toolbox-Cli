//! `nso-extract`: parse a Switch NSO (`exefs/main`, `subsdk*`, …) and write its
//! inflated `.text` / `.rodata` / `.data` segments to a directory. Useful for
//! inspecting executable contents — e.g. locating the TotK MeshCodec zstd
//! dictionary embedded in `main`'s `.rodata`/`.data`.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::nso::read_nso;

#[derive(Parser, Debug)]
pub struct Args {
    /// NSO module file (e.g. `exefs/main`).
    #[arg(short, long)]
    input: PathBuf,

    /// Output directory for the inflated segments.
    #[arg(short, long)]
    out: PathBuf,

    /// Which segment(s) to write: `text`, `rodata`, `data`, or `all`.
    #[arg(long, default_value = "all")]
    segment: String,

    /// Optional ASCII needle: report its byte offset(s) within each written
    /// segment (first few occurrences). Handy for locating known strings.
    #[arg(long)]
    grep: Option<String>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bytes =
        std::fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let nso = read_nso(&bytes).map_err(|e| anyhow!("{e}"))?;

    let want = args.segment.to_ascii_lowercase();
    let names: &[&str] = match want.as_str() {
        "all" => &["text", "rodata", "data"],
        "text" => &["text"],
        "rodata" => &["rodata"],
        "data" => &["data"],
        other => {
            return Err(anyhow!(
                "unknown --segment '{other}' (text|rodata|data|all)"
            ))
        }
    };

    println!(
        "{} ({} bytes): NSO0 v{} flags=0x{:x}",
        args.input.display(),
        bytes.len(),
        nso.version,
        nso.flags
    );
    std::fs::create_dir_all(&args.out)
        .with_context(|| format!("creating {}", args.out.display()))?;

    for name in names {
        let seg = nso.segment_bytes(name).expect("known segment name");
        let path = args.out.join(format!("{name}.bin"));
        std::fs::write(&path, seg).with_context(|| format!("writing {}", path.display()))?;
        let compressed = match *name {
            "text" => nso.text.is_compressed,
            "rodata" => nso.rodata.is_compressed,
            _ => nso.data.is_compressed,
        };
        println!(
            "  {name}: {} bytes ({}) -> {}",
            seg.len(),
            if compressed { "LZ4" } else { "stored" },
            path.display()
        );
        if let Some(needle) = &args.grep {
            let hits = find_all(seg, needle.as_bytes(), 8);
            if hits.is_empty() {
                println!("    grep {needle:?}: none");
            } else {
                let shown: Vec<String> = hits.iter().map(|o| format!("0x{o:x}")).collect();
                println!("    grep {needle:?}: {}", shown.join(", "));
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Byte offsets of the first `limit` occurrences of `needle` in `hay`.
fn find_all(hay: &[u8], needle: &[u8], limit: usize) -> Vec<usize> {
    let mut out = Vec::new();
    if needle.is_empty() || needle.len() > hay.len() {
        return out;
    }
    let mut i = 0usize;
    while i + needle.len() <= hay.len() {
        if &hay[i..i + needle.len()] == needle {
            out.push(i);
            if out.len() >= limit {
                break;
            }
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}
