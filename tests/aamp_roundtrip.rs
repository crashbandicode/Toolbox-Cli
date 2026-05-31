//! Fixture-gated AAMP round-trip tests against real BOTW actor parameters.
//! Skipped unless `tests/fixtures/aamp/` is populated (gitignored BOTW data,
//! extracted from `Actor/Pack/*.sbactorpack`). The verbatim writer re-emits the
//! bytes captured at parse time, so a byte-identical round-trip here proves the
//! parser walks the *entire* document (every list / object / parameter / type)
//! without error.

use std::path::Path;

use nx_layout_toolbox::aamp::{read_aamp, write_aamp};

fn aamp_dir() -> &'static Path {
    Path::new("tests/fixtures/aamp")
}

#[test]
fn every_aamp_fixture_round_trips_byte_identically() {
    let dir = aamp_dir();
    if !dir.exists() {
        eprintln!("skipping (no {})", dir.display());
        return;
    }
    let mut n = 0usize;
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let bytes = std::fs::read(&p).unwrap();
        if bytes.get(0..4) != Some(b"AAMP".as_slice()) {
            continue;
        }
        let doc = read_aamp(&bytes).unwrap_or_else(|e| panic!("parse {}: {e}", p.display()));
        assert_eq!(write_aamp(&doc), bytes, "{} not byte-identical", p.display());
        n += 1;
    }
    assert!(n >= 1, "expected at least one AAMP fixture in {}", dir.display());
    eprintln!("AAMP fixtures round-tripped byte-identically: {n}");
}

/// Pin the decoded structure of two known fixtures (counts = `(lists,
/// objects, params)`), so a parser regression that drops nodes is caught.
#[test]
fn pins_known_fixture_structure() {
    let bxml = aamp_dir().join("Weapon_Sword_001.bxml");
    if bxml.exists() {
        let doc = read_aamp(&std::fs::read(&bxml).unwrap()).unwrap();
        assert_eq!(doc.pio_type, "xml");
        assert!(!doc.big_endian);
        assert_eq!(doc.counts(), (1, 3, 38), "Weapon_Sword_001.bxml structure");
    } else {
        eprintln!("skipping bxml pin (no {})", bxml.display());
    }

    let phys = aamp_dir().join("Weapon_Sword_001.bphysics");
    if phys.exists() {
        let doc = read_aamp(&std::fs::read(&phys).unwrap()).unwrap();
        assert_eq!(doc.counts(), (14, 23, 231), "Weapon_Sword_001.bphysics structure");
    } else {
        eprintln!("skipping bphysics pin (no {})", phys.display());
    }
}
