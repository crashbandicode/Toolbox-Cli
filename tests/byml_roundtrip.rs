//! Fixture-gated tests for the BYML reader + verbatim round-trip against real
//! TotK assets. Skipped unless `tests/fixtures/byml/` exists with files — the
//! fixtures are gitignored game data.
//!
//! Coverage spans both endians and the compressed path: an uncompressed
//! little-endian `.bgyml` (`CookingTable`), a dictionary-compressed
//! little-endian `.byml.zs` (`Challenge` RSDB), and a compressed *big-endian*
//! `.byml.zs` (`GameDataList`, which TotK stores big-endian — a real-world
//! quirk our auto-detect handles). Every node in every fixture must decode
//! (no unknown-type / truncation errors) and re-emit byte-identically.

use std::path::{Path, PathBuf};

use nx_layout_toolbox::byml::{read_byml, write_byml, Byml, BymlDocument};
use nx_layout_toolbox::compression::{self, DictRegistry};

fn byml_dir() -> &'static Path {
    Path::new("tests/fixtures/byml")
}

/// Load the TotK dictionary registry if present (needed only for `.byml.zs`).
fn load_registry() -> DictRegistry {
    let pack = Path::new("tests/fixtures/totk/compression/ZsDic.pack.zs");
    if !pack.exists() {
        return DictRegistry::new();
    }
    let bytes = std::fs::read(pack).expect("read ZsDic pack");
    DictRegistry::from_zsdic_pack(&bytes).expect("load ZsDic pack")
}

fn byml_fixtures() -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(byml_dir()) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with(".byml") || n.ends_with(".bgyml") || n.ends_with(".byml.zs"))
                .unwrap_or(false)
        })
        .collect();
    paths.sort();
    paths
}

/// Read a fixture, inflating `.zs` via the dictionary registry.
fn load_doc(path: &Path, reg: &DictRegistry) -> Option<(Vec<u8>, BymlDocument)> {
    let name = path.file_name().unwrap().to_string_lossy();
    let raw = std::fs::read(path).unwrap();
    let bytes = if name.ends_with(".zs") {
        if reg.is_empty() {
            eprintln!("skipping {name}: compressed but no ZsDic.pack.zs fixture");
            return None;
        }
        compression::decompress(&raw, reg)
            .unwrap_or_else(|e| panic!("decompress {name}: {e}"))
            .into_owned()
    } else {
        raw
    };
    let doc = read_byml(&bytes).unwrap_or_else(|e| panic!("parse {name}: {e}"));
    Some((bytes, doc))
}

#[test]
fn byml_fixtures_round_trip_byte_identically() {
    let reg = load_registry();
    let fixtures = byml_fixtures();
    if fixtures.is_empty() {
        eprintln!("skipping BYML round-trip test (no fixtures in {})", byml_dir().display());
        return;
    }

    let mut processed = 0usize;
    let (mut saw_le, mut saw_be) = (false, false);
    for path in &fixtures {
        let Some((bytes, doc)) = load_doc(path, &reg) else {
            continue;
        };
        let name = path.file_name().unwrap().to_string_lossy();

        // The verbatim writer must reproduce the (decompressed) input exactly.
        let written = write_byml(&doc).expect("write");
        assert_eq!(written, bytes, "{name} BYML round-trip not byte-identical");

        assert!(
            (1..=7).contains(&doc.version),
            "{name}: unexpected version {}",
            doc.version
        );
        saw_le |= !doc.big_endian;
        saw_be |= doc.big_endian;
        processed += 1;
        eprintln!(
            "OK {name}: v{} {}-endian, {} bytes",
            doc.version,
            if doc.big_endian { "big" } else { "little" },
            bytes.len()
        );
    }

    assert!(processed > 0, "no BYML fixtures were processed (missing dict?)");
    // The shipped corpus covers both endians; only assert when we have them so
    // a partial local subset still passes.
    if saw_be {
        assert!(saw_le, "expected at least one little-endian fixture alongside big-endian");
    }
}

/// Pin decoded structure on the uncompressed TotK CookingTable (when present).
#[test]
fn cooking_table_decodes_known_values() {
    let path = byml_dir().join("CookingTable.game__cooking__Table.bgyml");
    if !path.exists() {
        eprintln!("skipping (no {})", path.display());
        return;
    }
    let bytes = std::fs::read(&path).unwrap();
    let doc = read_byml(&bytes).expect("parse CookingTable");

    assert_eq!(doc.version, 7);
    assert!(!doc.big_endian, "CookingTable is little-endian");

    let root = &doc.root;
    assert_eq!(
        root.get("RecipeList").and_then(Byml::as_array).map(<[Byml]>::len),
        Some(158)
    );
    assert_eq!(
        root.get("SingleRecipeList").and_then(Byml::as_array).map(<[Byml]>::len),
        Some(15)
    );

    let system = root.get("SystemData").expect("SystemData");
    assert_eq!(system.as_hash().map(<[(String, Byml)]>::len), Some(11));
    assert_eq!(
        system.get("EnemyExtractActorName").and_then(Byml::as_str),
        Some("Item_Material_08")
    );
    assert_eq!(
        system.get("FairyActorName").and_then(Byml::as_str),
        Some("Item_Cook_C_16")
    );
    // Nested arrays inside SystemData decode as arrays.
    assert_eq!(
        system.get("EffectList").and_then(Byml::as_array).map(<[Byml]>::len),
        Some(22)
    );
}

/// Prove the big-endian path works on real data: TotK's GameDataList is
/// big-endian BYML (when the fixture is present).
#[test]
fn game_data_list_is_big_endian() {
    let path = byml_dir().join("GameDataList.Product.110.byml.zs");
    if !path.exists() {
        eprintln!("skipping (no {})", path.display());
        return;
    }
    let reg = load_registry();
    if reg.is_empty() {
        eprintln!("skipping (no ZsDic.pack.zs fixture)");
        return;
    }
    let raw = std::fs::read(&path).unwrap();
    let bytes = compression::decompress(&raw, &reg).unwrap().into_owned();
    let doc = read_byml(&bytes).expect("parse GameDataList");

    assert!(doc.big_endian, "GameDataList.Product is big-endian BYML");
    assert_eq!(doc.version, 7);
    assert!(doc.root.is_container(), "root should be a container");
}
