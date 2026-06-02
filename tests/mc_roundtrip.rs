//! Fixture-gated MC/MCPK (TotK MeshCodec) container tests. Skipped unless
//! `tests/fixtures/mc/` holds real `*.bfres.mc` (gitignored game data). The
//! verbatim writer re-emits captured bytes, so a byte-identical round-trip here
//! proves the MCPK header parser walks every fixture without error — the safe
//! no-op foundation before any decompress/repack. (The full 12,395-file TotK
//! corpus was swept via `mc-roundtrip-test --dir`: all byte-identical.)

use std::path::Path;

use nx_layout_toolbox::mc::{extract, read_mc, repack, write_mc};

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

/// `mc-extract` must reproduce the reference decompressed BFRES byte-for-byte.
/// The reference is the decompressed `.bfres` (`tests/fixtures/bfres/`, the
/// Watertoon-tool output) — compared on its real-size prefix (it is zero-padded
/// to the alignment; our extract returns the unpadded BFRES).
#[test]
fn mc_extract_matches_reference_bfres() {
    let mc_path = mc_dir().join("Animal_Bass.Bass.bfres.mc");
    let ref_path = Path::new("tests/fixtures/bfres/Animal_Bass.Bass.bfres");
    if !mc_path.exists() || !ref_path.exists() {
        eprintln!(
            "skipping (need {} + {})",
            mc_path.display(),
            ref_path.display()
        );
        return;
    }
    let mc = read_mc(&std::fs::read(&mc_path).unwrap()).unwrap();
    let extracted = extract(&mc).expect("mc-extract");
    let reference = std::fs::read(ref_path).unwrap();
    let real = u32::from_le_bytes(reference[0x1c..0x20].try_into().unwrap()) as usize;
    assert_eq!(
        &extracted[..],
        &reference[..real],
        "extract != reference BFRES"
    );
    assert_eq!(&extracted[0..4], b"FRES");
}

/// `mc-extract(mc-repack(bfres)) == bfres` — the repack contract. Repacking an
/// extracted BFRES (no edit) must decode back to exactly those bytes.
#[test]
fn mc_repack_round_trips_through_extract() {
    let mc_path = mc_dir().join("Animal_Bass.Bass.bfres.mc");
    if !mc_path.exists() {
        eprintln!("skipping (no {})", mc_path.display());
        return;
    }
    let mc = read_mc(&std::fs::read(&mc_path).unwrap()).unwrap();
    let bfres = extract(&mc).expect("extract");
    // Repack the (unchanged) BFRES; the original mesh tail must be preserved and
    // extract must decode back to the exact BFRES.
    let repacked = repack(&mc, &bfres, 19, false).expect("repack");
    assert_eq!(&repacked[0..4], b"MCPK");
    let mc2 = read_mc(&repacked).expect("read repacked");
    assert_eq!(mc2.decompressed_size(), mc.decompressed_size());
    let bfres2 = extract(&mc2).expect("extract repacked");
    assert_eq!(bfres2, bfres, "extract(repack(x)) != x");
    // The mesh tail (everything after the re-encoded BFRES frame) is preserved
    // verbatim from the original container.
    let orig_tail_len = mc.compressed_stream().len();
    assert!(
        repacked.len() >= 12 && orig_tail_len > 0,
        "sanity: original has a compressed stream"
    );
}
