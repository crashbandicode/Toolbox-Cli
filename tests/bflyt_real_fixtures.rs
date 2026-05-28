//! Optional integration tests that round-trip every BFLYT in
//! `tests/fixtures/`. Skipped when the directory is missing (CI does not
//! ship real game assets).
//!
//! Drop your unpacked `layout.arc/blyt/*.bflyt` into `tests/fixtures/blyt/`
//! to enable. The directory is gitignored.

use std::fs;
use std::path::Path;

use toolbox_cli::bflyt::{read_bflyt, write_bflyt};

#[test]
fn every_bflyt_in_fixtures_round_trips_byte_identically() {
    let dir = Path::new("tests/fixtures/blyt");
    if !dir.exists() {
        eprintln!(
            "skipping real-fixture round-trip test (drop BFLYTs into {} to enable)",
            dir.display()
        );
        return;
    }

    let mut tested = 0usize;
    let mut failed = Vec::new();
    for entry in fs::read_dir(dir).expect("read fixtures dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("bflyt") {
            continue;
        }
        let bytes = fs::read(&path).expect("read fixture");
        let parsed = match read_bflyt(&bytes) {
            Ok(p) => p,
            Err(e) => {
                failed.push(format!("{}: parse failed: {e}", path.display()));
                continue;
            }
        };
        let written = match write_bflyt(&parsed) {
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
        for f in &failed {
            eprintln!("  {}", f);
        }
        panic!(
            "{} of {} BFLYT fixture(s) failed round-trip",
            failed.len(),
            tested + failed.len()
        );
    }
    println!("OK: {tested} BFLYT fixture(s) round-tripped byte-identically");
}
