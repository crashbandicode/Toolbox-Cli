//! Fixture-gated tests for the RESTBL (Resource Size Table) reader/writer
//! against real TotK assets. Skipped unless `tests/fixtures/restbl/` is
//! populated (gitignored game data).

use std::path::{Path, PathBuf};

use nx_layout_toolbox::compression::{self, DictRegistry};
use nx_layout_toolbox::restbl::{crc32, read_restbl, write_restbl};

fn restbl_dir() -> &'static Path {
    Path::new("tests/fixtures/restbl")
}

fn load_registry() -> DictRegistry {
    let pack = Path::new("tests/fixtures/totk/compression/ZsDic.pack.zs");
    if !pack.exists() {
        return DictRegistry::new();
    }
    DictRegistry::from_zsdic_pack(&std::fs::read(pack).unwrap()).expect("load ZsDic")
}

fn fixtures() -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(restbl_dir()) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.contains("rsizetable") || n.ends_with(".rstbl"))
                .unwrap_or(false)
        })
        .collect();
    paths.sort();
    paths
}

#[test]
fn restbl_fixtures_round_trip_byte_identically() {
    let reg = load_registry();
    let fixtures = fixtures();
    if fixtures.is_empty() {
        eprintln!(
            "skipping RESTBL round-trip (no fixtures in {})",
            restbl_dir().display()
        );
        return;
    }
    if reg.is_empty() {
        eprintln!("skipping RESTBL round-trip (no ZsDic.pack.zs fixture)");
        return;
    }

    let mut processed = 0usize;
    for path in &fixtures {
        let name = path.file_name().unwrap().to_string_lossy();
        let raw = std::fs::read(path).unwrap();
        let bytes = compression::decompress(&raw, &reg)
            .unwrap_or_else(|e| panic!("decompress {name}: {e}"))
            .into_owned();
        let table = read_restbl(&bytes).unwrap_or_else(|e| panic!("parse {name}: {e}"));
        let written = write_restbl(&table).unwrap();
        assert_eq!(
            written, bytes,
            "{name} RESTBL round-trip not byte-identical"
        );

        assert_eq!(table.version, 1, "{name} version");
        assert_eq!(table.string_block_size, 160, "{name} string_block_size");
        // CRC table is sorted ascending and name table sorted by name.
        assert!(
            table.crc_entries.windows(2).all(|w| w[0].hash <= w[1].hash),
            "{name} crc table not sorted"
        );
        assert!(
            table
                .name_entries
                .windows(2)
                .all(|w| w[0].name <= w[1].name),
            "{name} name table not sorted"
        );
        processed += 1;
        eprintln!(
            "OK {name}: v{} {} crc + {} name entries, {} bytes",
            table.version,
            table.crc_entries.len(),
            table.name_entries.len(),
            bytes.len()
        );
    }
    assert!(processed > 0);
}

/// Pin decoded counts + a known name-table lookup on the 1.2.1 table.
#[test]
fn product_121_known_values() {
    let path = restbl_dir().join("ResourceSizeTable.Product.121.rsizetable.zs");
    if !path.exists() {
        eprintln!("skipping (no {})", path.display());
        return;
    }
    let reg = load_registry();
    if reg.is_empty() {
        eprintln!("skipping (no ZsDic.pack.zs fixture)");
        return;
    }
    let bytes = compression::decompress(&std::fs::read(&path).unwrap(), &reg)
        .unwrap()
        .into_owned();
    let table = read_restbl(&bytes).unwrap();

    assert_eq!(table.crc_entries.len(), 379715);
    assert_eq!(table.name_entries.len(), 32);
    // A known name-table (collision) entry resolves through get_by_path.
    assert_eq!(
        table.get_by_path("Bake/Scene/MainField_U_30_50.bkres"),
        Some(64416)
    );
    assert_eq!(
        table.get_by_name("Actor/TwnObj_HatenoObj_A_12.engine__actor__ActorParam.bgyml"),
        Some(6184)
    );
}

/// Editing a real table and re-serializing stays byte-identical except for the
/// touched entry, and an inserted CRC entry survives a write→read cycle.
#[test]
fn edit_round_trips_on_real_table() {
    let path = restbl_dir().join("ResourceSizeTable.Product.121.rsizetable.zs");
    if !path.exists() {
        eprintln!("skipping (no {})", path.display());
        return;
    }
    let reg = load_registry();
    if reg.is_empty() {
        eprintln!("skipping (no ZsDic.pack.zs fixture)");
        return;
    }
    let bytes = compression::decompress(&std::fs::read(&path).unwrap(), &reg)
        .unwrap()
        .into_owned();
    let mut table = read_restbl(&bytes).unwrap();
    let crc_count = table.crc_entries.len();

    // Insert a brand-new resource path; the CRC table grows by one and the
    // entry is resolvable + the table stays sorted after a write→read cycle.
    let new_path = "MyMod/Custom/NewFile.bgyml";
    let h = crc32(new_path.as_bytes());
    assert_eq!(table.get_by_hash(h), None, "test path should be absent");
    table.insert_by_hash(h, 12345);

    let written = write_restbl(&table).unwrap();
    assert_eq!(
        written.len(),
        bytes.len() + 8,
        "one new CRC entry = +8 bytes"
    );
    let reread = read_restbl(&written).unwrap();
    assert_eq!(reread.crc_entries.len(), crc_count + 1);
    assert_eq!(reread.get_by_hash(h), Some(12345));
    assert!(reread
        .crc_entries
        .windows(2)
        .all(|w| w[0].hash <= w[1].hash));
}
