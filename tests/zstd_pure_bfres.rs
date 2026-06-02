//! Validate the pure-Rust Zstandard decoder on **real** data: the BFRES half of
//! every TotK MeshCodec `.mc` is a standard *magicless* zstd frame. We decode it
//! with `mc::decompress_first_frame` (pure `zstd_pure`) and require it to match
//! (a) **libzstd** decoding the same frame, and (b) — when present — the
//! reference decompressor's output (`local-assets/mesh-codec-output/`).
//!
//! Skipped unless `tests/fixtures/mc/` exists; the oracle check additionally
//! needs the gitignored oracle dir. libzstd here is a **test-only** oracle (a
//! dev-dependency); it is never a runtime dependency of the crate.

use std::path::{Path, PathBuf};

use nx_layout_toolbox::mc::{self, read_mc};
use zstd::zstd_safe::{DCtx, DParameter, FrameFormat, InBuffer, OutBuffer};

fn mc_fixtures() -> Vec<PathBuf> {
    let dir = Path::new("tests/fixtures/mc");
    if !dir.exists() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("mc") {
            out.push(p);
        }
    }
    out.sort();
    out
}

/// Decode a leading magicless zstd frame with **libzstd** (the test oracle),
/// returning the bytes + input consumed. Streaming decode tolerates the advisory
/// dictionary id the game's frames carry (the one-shot path rejects it).
fn libzstd_magicless(stream: &[u8], cap: usize) -> (Vec<u8>, usize) {
    let mut dctx = DCtx::create();
    dctx.set_parameter(DParameter::Format(FrameFormat::Magicless))
        .unwrap();
    dctx.set_parameter(DParameter::WindowLogMax(27)).unwrap();
    let cap = cap.max(1);
    let mut out = vec![0u8; cap];
    let (n, consumed) = {
        let mut output = OutBuffer::around(&mut out[..]);
        let mut input = InBuffer::around(stream);
        loop {
            let prev_in = input.pos();
            let prev_out = output.pos();
            let hint = dctx
                .decompress_stream(&mut output, &mut input)
                .expect("libzstd magicless decode");
            if hint == 0 || output.pos() == cap {
                break;
            }
            if input.pos() == prev_in && output.pos() == prev_out {
                break;
            }
        }
        (output.pos(), input.pos())
    };
    out.truncate(n);
    (out, consumed)
}

/// The reference-decompressor oracle for a `.mc` fixture, if present:
/// `Animal_Bear.Bear.bfres.mc` -> `local-assets/mesh-codec-output/Animal_Bear.Bear.bfres`.
fn oracle_for(mc_path: &Path) -> Option<PathBuf> {
    let stem = mc_path.file_name()?.to_str()?.strip_suffix(".mc")?;
    let p = Path::new("local-assets/mesh-codec-output").join(stem);
    p.exists().then_some(p)
}

#[test]
fn pure_zstd_matches_libzstd_and_oracle_on_bfres_frames() {
    let fixtures = mc_fixtures();
    if fixtures.is_empty() {
        eprintln!("skipping (no tests/fixtures/mc fixtures)");
        return;
    }
    let mut n = 0usize;
    let mut oracle_checked = 0usize;
    for p in &fixtures {
        let bytes = std::fs::read(p).unwrap();
        let mcfile = read_mc(&bytes).unwrap_or_else(|e| panic!("read_mc {}: {e}", p.display()));
        let stream = mcfile.compressed_stream();
        let cap = mcfile.decompressed_size();

        // Pure-Rust magicless decode (the production extract path).
        let (pure, consumed) = mc::decompress_first_frame(stream, cap)
            .unwrap_or_else(|e| panic!("pure decode {}: {e}", p.display()));
        assert_eq!(&pure[0..4], b"FRES", "{}: not a BFRES", p.display());

        // (a) libzstd decodes the same frame to the same bytes + consumed length.
        let (reference, ref_consumed) = libzstd_magicless(stream, cap);
        assert_eq!(pure, reference, "{}: pure decode != libzstd", p.display());
        assert_eq!(
            consumed,
            ref_consumed,
            "{}: pure consumed {consumed} vs libzstd {ref_consumed}",
            p.display(),
        );

        // (b) when the reference-decompressor oracle is present, the pure decode
        // must reproduce its BFRES prefix exactly (the `fileSize` at +0x1c).
        if let Some(oracle_path) = oracle_for(p) {
            let oracle = std::fs::read(&oracle_path).unwrap();
            let real = u32::from_le_bytes(oracle[0x1c..0x20].try_into().unwrap()) as usize;
            assert_eq!(
                pure,
                oracle[..real],
                "{}: pure decode != reference oracle",
                p.display()
            );
            oracle_checked += 1;
        }
        n += 1;
    }
    eprintln!("pure zstd matched libzstd on {n} BFRES frame(s); oracle-checked {oracle_checked}");
    assert!(n >= 1);
}
