//! `restbl-set`: update a resource's reserved size in a RESTBL — the core of
//! repacking a mod without crashing the game. Target the entry by resource
//! `--path` (CRC-32'd), raw `--hash`, or `--name` (collision table). Inflates
//! a compressed input and writes the **uncompressed** RESTBL (re-compress with
//! the `compress` verb if the game needs `.zs`).

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::compression;
use crate::restbl::{crc32, read_restbl, write_restbl, SetOutcome};

#[derive(Parser, Debug)]
pub struct Args {
    /// Input RESTBL (`.rsizetable`/`.rstbl`, optionally `.zs`).
    #[arg(short, long)]
    input: PathBuf,

    /// Output path for the (uncompressed) updated RESTBL.
    #[arg(short, long)]
    out: PathBuf,

    /// Resource path to update (hashed with CRC-32, then matched in the CRC
    /// table; falls back to the name table).
    #[arg(long, conflicts_with_all = ["hash", "name"])]
    path: Option<String>,

    /// Raw CRC-32 hash to update (hex, optional `0x`).
    #[arg(long, conflicts_with_all = ["path", "name"])]
    hash: Option<String>,

    /// Exact name-table entry to update.
    #[arg(long, conflicts_with_all = ["path", "hash"])]
    name: Option<String>,

    /// New reserved size in bytes.
    #[arg(long)]
    size: u32,

    /// Insert the entry if it isn't already present (keeping the table sorted).
    #[arg(long)]
    insert: bool,

    /// TotK `ZsDic.pack.zs` (or a directory of `*.zsdic`) for compressed input.
    #[arg(long)]
    dict: Option<PathBuf>,

    /// TotK RomFS root; auto-finds `Pack/ZsDic.pack.zs`.
    #[arg(long)]
    romfs: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let raw =
        std::fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let dicts = super::load_dict_registry(args.dict.as_deref(), args.romfs.as_deref())?;
    let bytes = compression::decompress(&raw, &dicts).map_err(|e| anyhow!("{e}"))?;
    let mut table = read_restbl(&bytes).map_err(|e| anyhow!("{e}"))?;

    let (target, found) = if let Some(path) = &args.path {
        let outcome = table.set_by_path(path, args.size, args.insert);
        let found = !matches!(outcome, SetOutcome::NotFound);
        (
            format!("path {path:?} (crc 0x{:08x}) [{outcome:?}]", crc32(path.as_bytes())),
            found,
        )
    } else if let Some(hash) = &args.hash {
        let h = parse_hex_u32(hash)?;
        let found = if args.insert {
            table.insert_by_hash(h, args.size);
            true
        } else {
            table.set_by_hash(h, args.size)
        };
        (format!("hash 0x{h:08x}"), found)
    } else if let Some(name) = &args.name {
        let found = if args.insert {
            table.insert_by_name(name, args.size);
            true
        } else {
            table.set_by_name(name, args.size)
        };
        (format!("name {name:?}"), found)
    } else {
        bail!("specify one of --path, --hash, or --name");
    };

    if !found {
        return Err(anyhow!(
            "{target} not present in the RESTBL; pass --insert to add it"
        ));
    }

    let written = write_restbl(&table).map_err(|e| anyhow!("{e}"))?;
    super::write_output(&args.out, &written)?;
    println!(
        "set {target} = {} bytes -> {} ({} bytes, {} crc + {} name entries)",
        args.size,
        args.out.display(),
        written.len(),
        table.crc_entries.len(),
        table.name_entries.len(),
    );
    Ok(ExitCode::SUCCESS)
}

fn parse_hex_u32(s: &str) -> Result<u32> {
    let t = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    u32::from_str_radix(t, 16).map_err(|e| anyhow!("invalid hex hash {s:?}: {e}"))
}
