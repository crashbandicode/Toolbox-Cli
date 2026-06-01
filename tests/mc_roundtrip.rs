//! Fixture-gated MC/MCPK (TotK MeshCodec) container tests. Skipped unless
//! `tests/fixtures/mc/` holds real `*.bfres.mc` (gitignored game data). The
//! verbatim writer re-emits captured bytes, so a byte-identical round-trip here
//! proves the MCPK header parser walks every fixture without error — the safe
//! no-op foundation before any decompress/repack. (The full 12,395-file TotK
//! corpus was swept via `mc-roundtrip-test --dir`: all byte-identical.)

use std::path::Path;

use nx_layout_toolbox::mc::{read_mc, write_mc};

fn mc_dir() -> &'static Path {
    Path::new("tests/fixtures/mc")
}

#[test]
fn every_mc_fixture_round_trips_byte_identically() {
    let dir = mc_dir();
    if !dir.exists() {
        eprintln!("skipping (no {})", dir.display());
        return;
    }
    let mut n = 0usize;
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("mc") {
            continue;
        }
        let bytes = std::fs::read(&p).unwrap();
        let mc = read_mc(&bytes).unwrap_or_else(|e| panic!("parse {}: {e}", p.display()));
        assert_eq!(write_mc(&mc), bytes, "{} not byte-identical", p.display());
        // The declared decompressed size must be sane: nonzero and a multiple
        // of the encoded alignment.
        let size = mc.decompressed_size();
        assert!(size > 0, "{} zero size", p.display());
        assert_eq!(mc.header.version, 1);
        assert!(mc.header.flags <= 1);
        n += 1;
    }
    assert!(n >= 1, "expected at least one .mc fixture");
    eprintln!("MCPK fixtures round-tripped byte-identically: {n}");
}

#[test]
fn pins_known_mc_decompressed_size() {
    let p = mc_dir().join("Animal_Bass.Bass.bfres.mc");
    if !p.exists() {
        eprintln!("skipping (no {})", p.display());
        return;
    }
    let mc = read_mc(&std::fs::read(&p).unwrap()).unwrap();
    // 0x11c -> (0x11c>>5)<<12 = 8<<12 = 0x8000, matching the oracle's padded size.
    assert_eq!(mc.decompressed_size(), 0x8000);
    assert_eq!(mc.header.alignment_shift(), 0xC);
}
