//! Fixture-gated BFRES round-trip tests against real BOTW + TotK resources.
//! Skipped unless `tests/fixtures/bfres/` is populated (gitignored game data:
//! decompressed BOTW `.sbfres` v5 + TotK v10 models/animations). The verbatim
//! writer re-emits the bytes captured at parse time, so a byte-identical
//! round-trip here proves the parser walks the header + structural scan without
//! error across the corpus's version/endianness/content variety.

use std::path::Path;

use nx_layout_toolbox::bfres::{read_bfres, write_bfres, VERSION_BOTW, VERSION_TOTK};
use nx_layout_toolbox::bntx::read_bntx;

fn bfres_dir() -> &'static Path {
    Path::new("tests/fixtures/bfres")
}

fn bfres_fixtures() -> Vec<std::path::PathBuf> {
    let dir = bfres_dir();
    if !dir.exists() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("bfres") {
            out.push(p);
        }
    }
    out.sort();
    out
}

#[test]
fn every_bfres_fixture_round_trips_byte_identically() {
    let fixtures = bfres_fixtures();
    if fixtures.is_empty() {
        eprintln!("skipping (no {} fixtures)", bfres_dir().display());
        return;
    }
    let mut n = 0usize;
    let mut saw_botw = false;
    let mut saw_totk = false;
    let mut saw_embedded_bntx = false;
    for p in &fixtures {
        let bytes = std::fs::read(p).unwrap();
        if bytes.get(0..4) != Some(b"FRES".as_slice()) {
            continue;
        }
        let doc = read_bfres(&bytes).unwrap_or_else(|e| panic!("parse {}: {e}", p.display()));
        // Verbatim round-trip is byte-identical by construction.
        assert_eq!(write_bfres(&doc), bytes, "{} not byte-identical", p.display());
        assert!(!doc.name.is_empty(), "{} empty name", p.display());
        // file_size header field never exceeds the actual buffer.
        assert!(
            doc.file_size as usize <= bytes.len(),
            "{} file_size {} > {}",
            p.display(),
            doc.file_size,
            bytes.len()
        );
        match doc.version {
            VERSION_BOTW => saw_botw = true,
            VERSION_TOTK => saw_totk = true,
            _ => {}
        }
        if doc.embedded_bntx_offset().is_some() {
            saw_embedded_bntx = true;
        }
        n += 1;
    }
    assert!(n >= 1, "expected at least one BFRES fixture");
    eprintln!(
        "BFRES fixtures round-tripped byte-identically: {n} (botw={saw_botw} totk={saw_totk} embedded_bntx={saw_embedded_bntx})"
    );
    // The curated corpus is meant to span both games.
    assert!(saw_botw, "corpus should contain a BOTW (v5) BFRES");
    assert!(saw_totk, "corpus should contain a TotK (v10) BFRES");
}

/// The embedded BNTX in a BOTW `.Tex.bfres` is decoded by the existing BNTX
/// reader (the bytes are bounded by the BNTX's own `file_size`).
#[test]
fn surfaces_embedded_bntx() {
    let fixtures = bfres_fixtures();
    if fixtures.is_empty() {
        eprintln!("skipping (no {} fixtures)", bfres_dir().display());
        return;
    }
    let mut checked = 0usize;
    for p in &fixtures {
        let bytes = std::fs::read(p).unwrap();
        if bytes.get(0..4) != Some(b"FRES".as_slice()) {
            continue;
        }
        let doc = read_bfres(&bytes).unwrap();
        if let Some(emb) = doc.embedded_bntx_bytes() {
            let bntx = read_bntx(emb)
                .unwrap_or_else(|e| panic!("embedded BNTX in {}: {e}", p.display()));
            assert!(
                !bntx.textures.is_empty(),
                "{} embedded BNTX has no textures",
                p.display()
            );
            // Every texture name resolves and has nonzero dimensions.
            for t in &bntx.textures {
                assert!(!t.name(&bntx).is_empty());
                assert!(t.width > 0 && t.height > 0);
            }
            eprintln!(
                "{}: embedded BNTX {:?} with {} textures",
                p.file_name().unwrap().to_string_lossy(),
                bntx.name,
                bntx.textures.len()
            );
            checked += 1;
        }
    }
    if checked == 0 {
        eprintln!("skipping (no embedded-BNTX fixture present)");
    }
}

/// Pin the decoded structure of known fixtures so a parser regression that
/// mis-decodes the header or drops scanned blocks is caught.
#[test]
fn pins_known_fixture_structure() {
    let model = bfres_dir().join("Animal_Bass.bfres");
    if model.exists() {
        let doc = read_bfres(&std::fs::read(&model).unwrap()).unwrap();
        assert_eq!(doc.name, "Animal_Bass");
        assert_eq!(doc.version, VERSION_BOTW);
        assert!(!doc.big_endian);
        assert_eq!(doc.block_count("FMDL"), 2, "Animal_Bass FMDL count");
    } else {
        eprintln!("skipping model pin (no {})", model.display());
    }

    let tex = bfres_dir().join("Animal_Bass.Tex.bfres");
    if tex.exists() {
        let doc = read_bfres(&std::fs::read(&tex).unwrap()).unwrap();
        assert_eq!(doc.name, "Animal_Bass.Tex");
        let emb = doc.embedded_bntx_bytes().expect("Tex has embedded BNTX");
        let bntx = read_bntx(emb).unwrap();
        assert_eq!(bntx.textures.len(), 8, "Animal_Bass.Tex embedded texture count");
    } else {
        eprintln!("skipping tex pin (no {})", tex.display());
    }
}
