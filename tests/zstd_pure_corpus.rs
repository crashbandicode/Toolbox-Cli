//! Full-corpus validation of the pure-Rust `zstd_pure` codec against libzstd on
//! every real TotK decompressed BFRES payload.
//!
//! The MeshCodec oracle dir (`local-assets/mesh-codec-output/`, gitignored)
//! holds the 12,395 reference-decompressed model/skeleton buffers. We do not
//! have the matching compressed `.mc` for all of them locally (only the 3
//! `tests/fixtures/mc/` inputs — see `zstd_pure_bfres.rs` for the real-frame
//! decode-vs-oracle check), so here each oracle buffer is treated as a real
//! payload and run through the **magicless** codec path that `mc::codec` uses:
//!
//! * pure round-trip — `decompress(compress(x)) == x`,
//! * pure decoder vs libzstd encoder — `pure_decode(libzstd(x)) == x`,
//! * pure encoder vs libzstd decoder — `libzstd_decode(pure(x)) == x`.
//!
//! This exercises the pure decoder + encoder across the full breadth of real
//! game data and proves byte-for-byte agreement with libzstd (the **test-only**
//! oracle). Skipped unless the oracle dir exists; it is slow (a full sweep of
//! ~12k files), so it is gated rather than part of the default CI suite.

use std::path::{Path, PathBuf};

use nx_layout_toolbox::mc;
use zstd::zstd_safe::{self, CCtx, CParameter, DCtx, DParameter, FrameFormat, InBuffer, OutBuffer};

const LEVEL: i32 = 3;

fn oracle_files() -> Vec<PathBuf> {
    let dir = Path::new("local-assets/mesh-codec-output");
    if !dir.exists() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_file() {
            out.push(p);
        }
    }
    out.sort();
    out
}

/// libzstd magicless compress (test oracle), pledging the content size — the
/// same frame shape `mc::compress_stream` produces.
fn libzstd_compress(data: &[u8]) -> Vec<u8> {
    let mut cctx = CCtx::create();
    cctx.set_parameter(CParameter::Format(FrameFormat::Magicless))
        .unwrap();
    cctx.set_parameter(CParameter::CompressionLevel(LEVEL))
        .unwrap();
    cctx.set_parameter(CParameter::ContentSizeFlag(true))
        .unwrap();
    cctx.set_pledged_src_size(Some(data.len() as u64)).unwrap();
    let mut out = Vec::with_capacity(zstd_safe::compress_bound(data.len()));
    cctx.compress2(&mut out, data).expect("libzstd compress");
    out
}

/// libzstd magicless decode (test oracle) of a full frame, capped at `cap`.
fn libzstd_decompress(frame: &[u8], cap: usize) -> Vec<u8> {
    let mut dctx = DCtx::create();
    dctx.set_parameter(DParameter::Format(FrameFormat::Magicless))
        .unwrap();
    dctx.set_parameter(DParameter::WindowLogMax(27)).unwrap();
    let cap = cap.max(1);
    let mut out = vec![0u8; cap];
    let n = {
        let mut output = OutBuffer::around(&mut out[..]);
        let mut input = InBuffer::around(frame);
        loop {
            let prev_in = input.pos();
            let prev_out = output.pos();
            let hint = dctx
                .decompress_stream(&mut output, &mut input)
                .expect("libzstd decode");
            if hint == 0 || output.pos() == cap {
                break;
            }
            if input.pos() == prev_in && output.pos() == prev_out {
                break;
            }
        }
        output.pos()
    };
    out.truncate(n);
    out
}

// Heavy: sweeps ~12k files / ~2.9 GiB. Excluded from the default `cargo test`
// (`#[ignore]`) — run explicitly with `cargo test --release --test zstd_pure_corpus
// -- --ignored --nocapture`. Also no-ops cleanly when the oracle dir is absent.
#[test]
#[ignore = "full ~12k-file MeshCodec corpus sweep; run with --release --ignored"]
fn pure_zstd_round_trips_full_corpus_vs_libzstd() {
    let files = oracle_files();
    if files.is_empty() {
        eprintln!("skipping (no local-assets/mesh-codec-output corpus)");
        return;
    }
    let total = files.len();
    let mut bytes_in = 0u64;
    for (i, p) in files.iter().enumerate() {
        let data = std::fs::read(p).unwrap();
        bytes_in += data.len() as u64;

        // Pure encoder -> pure decoder (the repack/extract magicless path).
        let pure_comp = mc::compress_stream(&data, LEVEL).expect("pure compress");
        let pure_back = mc::decompress_stream(&pure_comp, data.len()).expect("pure decompress");
        assert_eq!(pure_back, data, "pure round-trip differs: {}", p.display());

        // Pure decoder must decode libzstd's frame byte-for-byte.
        let lib_comp = libzstd_compress(&data);
        let from_lib = mc::decompress_stream(&lib_comp, data.len()).expect("pure decode libzstd");
        assert_eq!(
            from_lib,
            data,
            "pure decode of libzstd differs: {}",
            p.display()
        );

        // libzstd must decode the pure encoder's frame byte-for-byte.
        let from_pure = libzstd_decompress(&pure_comp, data.len());
        assert_eq!(
            from_pure,
            data,
            "libzstd decode of pure differs: {}",
            p.display()
        );

        if (i + 1) % 2000 == 0 {
            eprintln!("  ... {} / {total} files", i + 1);
        }
    }
    eprintln!(
        "pure zstd round-tripped + cross-checked libzstd on {total} corpus files ({:.1} MiB)",
        bytes_in as f64 / (1024.0 * 1024.0)
    );
}
