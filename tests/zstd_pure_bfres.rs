//! Validate the pure-Rust Zstandard decoder against libzstd on **real** data:
//! the BFRES half of every TotK MeshCodec `.mc` is a standard *magicless* zstd
//! frame. We decode it both with libzstd (via `mc::decompress_first_frame`) and
//! with `zstd_pure::decompress_magicless` and require byte-for-byte agreement
//! (and the same consumed length). Skipped unless `tests/fixtures/mc/` exists.

use std::path::{Path, PathBuf};

use nx_layout_toolbox::mc::{self, read_mc};
use nx_layout_toolbox::zstd_pure;

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

#[test]
fn pure_zstd_matches_libzstd_on_bfres_frames() {
    let fixtures = mc_fixtures();
    if fixtures.is_empty() {
        eprintln!("skipping (no tests/fixtures/mc fixtures)");
        return;
    }
    let mut n = 0usize;
    for p in &fixtures {
        let bytes = std::fs::read(p).unwrap();
        let mcfile = read_mc(&bytes).unwrap_or_else(|e| panic!("read_mc {}: {e}", p.display()));
        let stream = mcfile.compressed_stream();
        let cap = mcfile.decompressed_size();

        // Reference: libzstd (streaming magicless) — the existing extract path.
        let (reference, consumed) = mc::decompress_first_frame(stream, cap)
            .unwrap_or_else(|e| panic!("libzstd decode {}: {e}", p.display()));

        // Pure-Rust magicless decode of the same leading frame.
        let frame = zstd_pure::decompress_magicless(stream, cap)
            .unwrap_or_else(|e| panic!("pure-zstd decode {}: {e}", p.display()));

        assert_eq!(
            frame.data.len(),
            reference.len(),
            "{}: pure decode length differs",
            p.display()
        );
        assert_eq!(
            frame.data, reference,
            "{}: pure decode bytes differ from libzstd",
            p.display()
        );
        assert_eq!(
            frame.consumed,
            consumed,
            "{}: pure decode consumed {} vs libzstd {}",
            p.display(),
            frame.consumed,
            consumed
        );
        assert_eq!(&frame.data[0..4], b"FRES", "{}: not a BFRES", p.display());
        n += 1;
    }
    eprintln!("pure-Rust zstd matched libzstd on {n} real BFRES frame(s)");
    assert!(n >= 1);
}
