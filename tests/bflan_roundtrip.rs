//! Byte-identical round-trip of every BFLAN under `tests/fixtures/`, plus
//! a coverage check that the `pat1`/`pai1` inspect decoders actually run
//! against the corpus. Skipped when the directory is missing.
//!
//! Drop unpacked archives (their `anim/*.bflan`) anywhere under
//! `tests/fixtures/`; the test walks recursively.

use std::fs;
use std::path::{Path, PathBuf};

use nx_layout_toolbox::bflan::{decode_pai1, decode_pat1, read_bflan, write_bflan};

fn collect_bflans(dir: &Path, out: &mut Vec<PathBuf>) {
    if !dir.is_dir() {
        return;
    }
    for entry in fs::read_dir(dir).expect("read dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_bflans(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("bflan") {
            out.push(path);
        }
    }
}

#[test]
fn every_bflan_in_fixtures_round_trips_byte_identically() {
    let dir = Path::new("tests/fixtures");
    if !dir.exists() {
        eprintln!(
            "skipping BFLAN round-trip test (drop BFLANs into {} to enable)",
            dir.display()
        );
        return;
    }

    let mut paths = Vec::new();
    collect_bflans(dir, &mut paths);
    if paths.is_empty() {
        eprintln!("no BFLANs in {}", dir.display());
        return;
    }

    let mut tested = 0usize;
    let mut failed = Vec::new();
    let mut pat1_decoded = 0usize;
    let mut pai1_decoded = 0usize;

    for path in &paths {
        let bytes = fs::read(path).expect("read fixture");
        let parsed = match read_bflan(&bytes) {
            Ok(p) => p,
            Err(e) => {
                failed.push(format!("{}: parse failed: {e}", path.display()));
                continue;
            }
        };

        // Exercise the inspect decoders (read-only; never affects the
        // round-trip, which uses the verbatim section bytes).
        if let Some(s) = parsed.section(b"pat1") {
            if decode_pat1(&s.payload, parsed.version_major()).is_some() {
                pat1_decoded += 1;
            }
        }
        if let Some(s) = parsed.section(b"pai1") {
            if decode_pai1(&s.payload).is_some() {
                pai1_decoded += 1;
            }
        }

        let written = match write_bflan(&parsed) {
            Ok(w) => w,
            Err(e) => {
                failed.push(format!("{}: write failed: {e}", path.display()));
                continue;
            }
        };
        if written != bytes {
            failed.push(format!(
                "{}: round-trip differs (orig={} bytes, rewritten={} bytes)",
                path.display(),
                bytes.len(),
                written.len()
            ));
            continue;
        }
        tested += 1;
    }

    if !failed.is_empty() {
        for f in failed.iter().take(40) {
            eprintln!("  {f}");
        }
        panic!(
            "{} of {} BFLAN fixture(s) failed round-trip",
            failed.len(),
            tested + failed.len()
        );
    }

    // The corpus must actually exercise both decoders.
    assert!(pat1_decoded > 0, "no pat1 sections decoded across the corpus");
    assert!(pai1_decoded > 0, "no pai1 sections decoded across the corpus");

    println!(
        "OK: {tested} BFLAN fixture(s) round-tripped byte-identically \
         (pat1 decoded {pat1_decoded}, pai1 decoded {pai1_decoded})"
    );
}
