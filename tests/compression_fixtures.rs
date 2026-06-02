//! Fixture-gated tests for the compression module against real TotK `.zs`
//! assets. Skipped unless `tests/fixtures/totk/compression/ZsDic.pack.zs`
//! (and at least one `.blarc.zs`) are present locally — the fixtures are
//! gitignored game data.
//!
//! These anchor the project's compression round-trip discipline on real
//! data: decode correctness is proved by the inflated archive being a valid
//! SARC whose inner BFLYT/BFLAN round-trip byte-identically through our
//! existing parsers, and `decompress(compress(x)) == x` for both codecs.
//! (The byte-for-byte match against Python's `compression.zstd` reference is
//! verified manually during development; `cargo test` stays Python-free.)

use std::path::{Path, PathBuf};

use nx_layout_toolbox::bflan::{read_bflan, write_bflan};
use nx_layout_toolbox::bflyt::{read_bflyt, write_bflyt};
use nx_layout_toolbox::compression::{self, DictRegistry};
use nx_layout_toolbox::sarc;

fn fixture_dir() -> &'static Path {
    Path::new("tests/fixtures/totk/compression")
}

fn zsdic_pack() -> PathBuf {
    fixture_dir().join("ZsDic.pack.zs")
}

fn load_registry() -> Option<DictRegistry> {
    let pack = zsdic_pack();
    if !pack.exists() {
        eprintln!("skipping compression fixture test (no {})", pack.display());
        return None;
    }
    let bytes = std::fs::read(&pack).expect("read ZsDic pack");
    Some(DictRegistry::from_zsdic_pack(&bytes).expect("load ZsDic pack"))
}

fn blarc_paths() -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(fixture_dir()) else {
        return Vec::new();
    };
    rd.filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with(".blarc.zs"))
                .unwrap_or(false)
        })
        .collect()
}

#[test]
fn zsdic_pack_yields_expected_dictionaries() {
    let Some(reg) = load_registry() else { return };
    let ids = reg.ids();
    eprintln!("ZsDic dictionary ids: {ids:?}");
    // TotK ships exactly three: zs (id 1), bcett (id 2), pack (id 3).
    assert_eq!(reg.len(), 3, "expected 3 dictionaries, got ids {ids:?}");
    assert!(reg.get(1).is_some(), "zs.zsdic should be id 1");
    assert!(reg.get(3).is_some(), "pack.zsdic should be id 3");
}

#[test]
fn blarc_zs_decompresses_to_sarc_and_inner_round_trips() {
    let Some(reg) = load_registry() else { return };
    let blarcs = blarc_paths();
    assert!(
        !blarcs.is_empty(),
        "expected at least one .blarc.zs fixture"
    );

    let mut bflyt_ok = 0usize;
    let mut bflan_ok = 0usize;
    for path in &blarcs {
        let name = path.file_name().unwrap().to_string_lossy();
        let comp = std::fs::read(path).unwrap();
        let raw = compression::decompress(&comp, &reg)
            .unwrap_or_else(|e| panic!("decompress {name}: {e}"));
        assert_eq!(&raw[0..4], b"SARC", "{name} should inflate to a SARC");

        for f in sarc::unpack(&raw).expect("unpack inner SARC") {
            if f.name.ends_with(".bflyt") {
                let parsed = read_bflyt(&f.data).expect("parse inner BFLYT");
                let back = write_bflyt(&parsed).expect("write inner BFLYT");
                assert_eq!(back, f.data, "inner BFLYT {} not byte-identical", f.name);
                bflyt_ok += 1;
            } else if f.name.ends_with(".bflan") {
                let parsed = read_bflan(&f.data).expect("parse inner BFLAN");
                let back = write_bflan(&parsed).expect("write inner BFLAN");
                assert_eq!(back, f.data, "inner BFLAN {} not byte-identical", f.name);
                bflan_ok += 1;
            }
        }
    }
    assert!(
        bflyt_ok > 0,
        "expected at least one inner BFLYT to round-trip"
    );
    eprintln!(
        "OK: {} blarc(s); {bflyt_ok} inner BFLYT + {bflan_ok} inner BFLAN round-tripped byte-identically",
        blarcs.len()
    );
}

#[test]
fn lossless_recompression_round_trip() {
    let Some(reg) = load_registry() else { return };
    let Some(path) = blarc_paths().into_iter().next() else {
        eprintln!("skipping (no .blarc.zs fixture)");
        return;
    };
    let raw = compression::decompress(&std::fs::read(&path).unwrap(), &reg)
        .unwrap()
        .into_owned();

    // zstd with the zs dictionary (id 1): the frame must advertise id 1 and
    // round-trip losslessly through our own decode path.
    let z = compression::compress_zstd(&raw, &reg, Some(1), 19).unwrap();
    assert_eq!(
        compression::zstd::frame_dictionary_id(&z).unwrap(),
        1,
        "re-compressed frame should reference dictionary id 1"
    );
    assert_eq!(&*compression::decompress(&z, &reg).unwrap(), &raw[..]);

    // Yaz0 (dictionary-less) round-trip on the same payload.
    let y = compression::compress_yaz0(&raw);
    assert_eq!(&*compression::decompress(&y, &reg).unwrap(), &raw[..]);
}
