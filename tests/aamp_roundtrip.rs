//! Fixture-gated AAMP round-trip tests against real BOTW actor parameters.
//! Skipped unless `tests/fixtures/aamp/` is populated (gitignored BOTW data,
//! extracted from `Actor/Pack/*.sbactorpack`). The verbatim writer re-emits the
//! bytes captured at parse time, so a byte-identical round-trip here proves the
//! parser walks the *entire* document (every list / object / parameter / type)
//! without error.

use std::path::Path;

use nx_layout_toolbox::aamp::{read_aamp, set_by_path, write_aamp, write_aamp_canonical, Value};

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
        assert_eq!(
            write_aamp(&doc),
            bytes,
            "{} not byte-identical",
            p.display()
        );
        n += 1;
    }
    assert!(
        n >= 1,
        "expected at least one AAMP fixture in {}",
        dir.display()
    );
    eprintln!("AAMP fixtures round-tripped byte-identically: {n}");
}

#[test]
fn canonical_writer_semantic_round_trips_corpus() {
    let dir = aamp_dir();
    if !dir.exists() {
        eprintln!("skipping (no {})", dir.display());
        return;
    }
    let mut n = 0usize;
    let mut byte_identical = 0usize;
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let bytes = std::fs::read(&p).unwrap();
        if bytes.get(0..4) != Some(b"AAMP".as_slice()) {
            continue;
        }
        let doc = read_aamp(&bytes).unwrap();
        let rebuilt =
            write_aamp_canonical(&doc).unwrap_or_else(|e| panic!("canonical {}: {e}", p.display()));
        let doc2 = read_aamp(&rebuilt)
            .unwrap_or_else(|e| panic!("re-parse canonical {}: {e}", p.display()));
        // Semantic round-trip: the rebuilt document decodes to the same tree.
        assert_eq!(
            doc.root,
            doc2.root,
            "{} canonical tree mismatch",
            p.display()
        );
        assert_eq!(doc.pio_type, doc2.pio_type, "{} pio_type", p.display());
        assert_eq!(
            doc.pio_version,
            doc2.pio_version,
            "{} pio_version",
            p.display()
        );
        if rebuilt == bytes {
            byte_identical += 1;
        }
        n += 1;
    }
    assert!(n >= 1, "expected at least one AAMP fixture");
    eprintln!(
        "AAMP canonical semantic round-trip: {n} files ({byte_identical} also byte-identical)"
    );
}

/// Edit a real Int parameter via `set_by_path` (using hex-hash segments, since
/// the original names aren't stored), canonical-write, re-parse, and confirm
/// the new value — proving the set → canonical-write → read pipeline on real
/// BOTW bytes.
#[test]
fn set_by_path_edits_a_real_int_param() {
    let dir = aamp_dir();
    if !dir.exists() {
        eprintln!("skipping (no {})", dir.display());
        return;
    }
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let bytes = std::fs::read(&p).unwrap();
        if bytes.get(0..4) != Some(b"AAMP".as_slice()) {
            continue;
        }
        let doc = read_aamp(&bytes).unwrap();
        for obj in &doc.root.objects {
            for param in &obj.params {
                if let Value::Int(old) = param.value {
                    if old == 31337 {
                        continue;
                    }
                    let path = format!("0x{:08x}/0x{:08x}", obj.name_hash, param.name_hash);
                    let mut edited = read_aamp(&bytes).unwrap();
                    set_by_path(&mut edited.root, &path, "31337").unwrap();
                    let rebuilt = write_aamp_canonical(&edited).unwrap();
                    let back = read_aamp(&rebuilt).unwrap();
                    let o2 = back.root.object(obj.name_hash).unwrap();
                    let v = &o2
                        .params
                        .iter()
                        .find(|q| q.name_hash == param.name_hash)
                        .unwrap()
                        .value;
                    assert_eq!(*v, Value::Int(31337), "edited int in {}", p.display());
                    eprintln!("aamp-set edited {} at {path}", p.display());
                    return;
                }
            }
        }
    }
    eprintln!("skipping (no direct-object Int parameter found in fixtures)");
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
        assert_eq!(
            doc.counts(),
            (14, 23, 231),
            "Weapon_Sword_001.bphysics structure"
        );
    } else {
        eprintln!("skipping bphysics pin (no {})", phys.display());
    }
}
