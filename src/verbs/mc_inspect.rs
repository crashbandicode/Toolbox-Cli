//! `mc-inspect`: structured snapshot of a TotK MeshCodec (`MCPK`) container —
//! version, flags, the decompressed-size descriptor (and the size/alignment it
//! decodes to), and the compressed-stream length. Read-only; does not
//! decompress. Use `--json`.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use serde_json::json;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::mc::{read_mc, read_mesh_section};

#[derive(Parser, Debug)]
pub struct Args {
    /// MeshCodec container (`*.bfres.mc`).
    #[arg(short, long)]
    input: PathBuf,

    /// Emit JSON instead of human-readable text.
    #[arg(long)]
    json: bool,

    /// Also decode the BFRES frame and report the trailing FMSH mesh section
    /// (geometry buffer sizes / chunk framing). Slower (it decompresses).
    #[arg(long)]
    mesh: bool,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bytes =
        std::fs::read(&args.input).with_context(|| format!("reading {}", args.input.display()))?;
    let mc = read_mc(&bytes).map_err(|e| anyhow!("{e}"))?;

    let mesh = if args.mesh {
        read_mesh_section(&mc).map_err(|e| anyhow!("{e}"))?
    } else {
        None
    };

    if args.json {
        let mut out = json!({
            "path": args.input.display().to_string(),
            "file_size": bytes.len(),
            "version": mc.header.version,
            "flags": mc.header.flags,
            "size_descriptor": mc.header.size_descriptor,
            "decompressed_size": mc.decompressed_size(),
            "alignment_shift": mc.header.alignment_shift(),
            "compressed_stream_size": mc.compressed_stream().len(),
        });
        if args.mesh {
            out["mesh"] = match &mesh {
                Some(m) => json!({
                    "fmsh_offset": m.fmsh_offset,
                    "version": m.version,
                    "compressed_size": m.compressed_size,
                    "buf_a_size": m.buf_a_size,
                    "buf_b_size": m.buf_b_size,
                    "decoded_geometry_size": m.decoded_geometry_size(),
                    "align_a": m.align_a,
                    "align_b": m.align_b,
                    "chunk_kind": m.first_chunk.kind,
                    "chunk_val": m.first_chunk.val,
                    "sub_a_size": m.first_chunk.sub_a_size,
                    "sub_b_size": m.first_chunk.sub_b_size,
                }),
                None => serde_json::Value::Null,
            };
        }
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("{} ({} bytes)", args.input.display(), bytes.len());
        println!("  magic = MCPK  version = {}  flags = {}", mc.header.version, mc.header.flags);
        println!(
            "  size_descriptor = 0x{:08x} -> decompressed {} bytes (align 1<<{})",
            mc.header.size_descriptor,
            mc.decompressed_size(),
            mc.header.alignment_shift()
        );
        println!(
            "  compressed stream = {} bytes (from +0x{:x})",
            mc.compressed_stream().len(),
            crate::mc::MC_HEADER_LEN
        );
        println!("  (inner stream is a magicless-zstd BFRES frame, no dictionary; not decompressed here unless --mesh)");
        if args.mesh {
            match &mesh {
                Some(m) => {
                    println!(
                        "  mesh (FMSH @ +0x{:x}): geometry {} bytes = bufA {} (index) + bufB {} (vertex)",
                        crate::mc::MC_HEADER_LEN + m.fmsh_offset,
                        m.decoded_geometry_size(),
                        m.buf_a_size,
                        m.buf_b_size
                    );
                    println!(
                        "    payload {} bytes; chunk kind={} val={} sub_a={} sub_b={} (custom meshopt codec; geometry not decoded)",
                        m.compressed_size,
                        m.first_chunk.kind,
                        m.first_chunk.val,
                        m.first_chunk.sub_a_size,
                        m.first_chunk.sub_b_size
                    );
                }
                None => println!("  mesh: none (no FMSH section — e.g. a skeleton/animation resource)"),
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}
