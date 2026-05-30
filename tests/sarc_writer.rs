//! Verifies the custom SARC writer: round-tripping a real `layout.arc`
//! through `read_arc` -> `write_arc` preserves every file's bytes, keeps
//! the entry count, stays re-readable, and — crucially — gives each file
//! the alignment it needs without the `0x2000`-everywhere bloat the old
//! `sarc`-crate writer produced. GPU resources (BNTX/BNSH) must land on a
//! `0x1000` boundary.
//!
//! Skipped when the fixture archive is absent.

use std::collections::BTreeMap;
use std::path::Path;

use nx_layout_toolbox::sarc::{file_alignment, read_arc};

fn rd_u16(d: &[u8], o: usize) -> usize {
    u16::from_le_bytes([d[o], d[o + 1]]) as usize
}
fn rd_u32(d: &[u8], o: usize) -> usize {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as usize
}

#[test]
fn write_arc_preserves_data_and_aligns_tightly() {
    let arc_path = Path::new("tests/fixtures/archives/info_melee_original.layout.arc");
    if !arc_path.exists() {
        eprintln!("skipping sarc writer test (no fixture at {})", arc_path.display());
        return;
    }
    let original = std::fs::read(arc_path).expect("read arc");
    let arc = read_arc(&original).expect("parse arc");
    let n = arc.files.len();

    let out = nx_layout_toolbox::sarc::write_arc(&arc).expect("write arc");

    // (1) Re-readable, same entry count, every file's bytes preserved.
    let reparsed = read_arc(&out).expect("re-parse written arc");
    assert_eq!(reparsed.files.len(), n, "entry count changed");

    let by_name: BTreeMap<&str, &[u8]> = arc
        .files
        .iter()
        .filter_map(|f| f.name.as_deref().map(|nm| (nm, f.data.as_slice())))
        .collect();
    let mut named = 0usize;
    for f in &reparsed.files {
        if let Some(name) = &f.name {
            named += 1;
            let orig = by_name.get(name.as_str()).expect("name present in original");
            assert_eq!(&f.data[..], *orig, "data for '{name}' changed");
        }
    }
    assert!(named > 0);

    // (2) Tight: the old writer ballooned this to ~4.7 MB; ours should be
    // close to the original 2.16 MB and definitely well under 3 MB.
    assert!(
        out.len() < 3_000_000,
        "output {} bytes is larger than expected (alignment bloat?)",
        out.len()
    );

    // (3) Walk the produced SFAT and assert every entry sits on the
    // alignment its content requires; GPU resources (BNTX/BNSH) on
    // 0x1000.
    assert_eq!(&out[0..4], b"SARC");
    let data_offset = rd_u32(&out, 0x0C);
    assert_eq!(&out[0x14..0x18], b"SFAT");
    let node_count = rd_u16(&out, 0x1A);
    assert_eq!(node_count, n, "SFAT node count");

    let mut checked_gpu = 0usize;
    for i in 0..node_count {
        let node = 0x20 + i * 0x10;
        let start = rd_u32(&out, node + 8);
        let end = rd_u32(&out, node + 12);
        let abs = data_offset + start;
        let bytes = &out[abs..data_offset + end];

        let needed = file_alignment(bytes) as usize;
        assert_eq!(
            abs % needed,
            0,
            "entry #{i} at 0x{abs:x} not aligned to its required 0x{needed:x}"
        );
        if bytes.len() >= 4 && (bytes[0..4] == *b"BNTX" || bytes[0..4] == *b"BNSH") {
            assert_eq!(abs % 0x1000, 0, "GPU resource at 0x{abs:x} not 0x1000-aligned");
            checked_gpu += 1;
        }
    }
    assert!(checked_gpu > 0, "expected at least one BNTX/BNSH entry");

    println!(
        "OK: SARC writer round-trip ({} entries, {} GPU resources 0x1000-aligned); \
         {} -> {} bytes (was ~4.7 MB with the crate writer)",
        n,
        checked_gpu,
        original.len(),
        out.len()
    );
}
