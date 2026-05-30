//! `layout-diff` against the original `info_melee` vs the generated SGPO
//! layout fixture.
//!
//! The generated SGPO layout adds an `sgpo_root` container and 24 marker
//! panes (cloned controller buttons) and reuses the stock materials and
//! textures — so the BFLYT diff is exactly 25 added panes and the BNTX
//! diff is empty. This test pins that, checks the diff direction
//! (reverse = 25 *removed*), and proves a layout diffed against itself is
//! empty.
//!
//! Skipped when the fixtures are absent.

use std::path::Path;

use nx_layout_toolbox::bflyt::{read_bflyt, BFLYT};
use nx_layout_toolbox::bntx::{read_bntx, BntxFile};
use nx_layout_toolbox::diff::diff_layouts;

fn load(bflyt: &str, bntx: &str) -> Option<(BFLYT, BntxFile)> {
    let bflyt_path = Path::new(bflyt);
    let bntx_path = Path::new(bntx);
    if !bflyt_path.exists() || !bntx_path.exists() {
        return None;
    }
    let b = read_bflyt(&std::fs::read(bflyt_path).unwrap()).expect("parse bflyt");
    let n = read_bntx(&std::fs::read(bntx_path).unwrap()).expect("parse bntx");
    Some((b, n))
}

#[test]
fn diff_original_vs_generated_sgpo() {
    let orig = load(
        "tests/fixtures/bflyt/info_melee_original/info_melee.bflyt",
        "tests/fixtures/bntx/info_melee_original__Combined.bntx",
    );
    let gen = load(
        "tests/fixtures/bflyt/sgpo_current_generated/info_melee.bflyt",
        "tests/fixtures/bntx/sgpo_current_generated__Combined.bntx",
    );
    let (Some((ob, on)), Some((gb, gn))) = (orig, gen) else {
        eprintln!("skipping layout-diff test (fixtures absent)");
        return;
    };

    let diff = diff_layouts(&ob, &on, &gb, &gn);
    assert!(!diff.is_empty(), "expected the SGPO layout to differ");

    // BFLYT: exactly the SGPO additions, nothing removed or changed.
    let b = &diff.bflyt;
    assert!(b.textures_added.is_empty(), "unexpected txl1 additions: {:?}", b.textures_added);
    assert!(b.textures_removed.is_empty());
    assert!(b.materials_added.is_empty(), "unexpected material additions: {:?}", b.materials_added);
    assert!(b.materials_removed.is_empty());
    assert!(b.materials_changed.is_empty());
    assert!(b.panes_removed.is_empty());
    assert!(b.panes_changed.is_empty());
    assert_eq!(b.panes_added.len(), 25, "expected sgpo_root + 24 markers");

    // The container is a pan1 under RootPane; the rest are pic1 markers
    // under sgpo_root.
    let root = b
        .panes_added
        .iter()
        .find(|p| p.name == "sgpo_root")
        .expect("sgpo_root container added");
    assert_eq!(root.kind, "pan1");
    assert_eq!(root.parent.as_deref(), Some("RootPane"));

    let markers: Vec<_> = b
        .panes_added
        .iter()
        .filter(|p| p.name != "sgpo_root")
        .collect();
    assert_eq!(markers.len(), 24);
    for m in &markers {
        assert_eq!(m.kind, "pic1", "marker {} should be a pic1", m.name);
        assert_eq!(
            m.parent.as_deref(),
            Some("sgpo_root"),
            "marker {} should hang under sgpo_root",
            m.name
        );
    }

    // BNTX is unchanged (the generated layout reuses stock textures).
    assert!(diff.bntx.is_empty(), "expected no BNTX changes; got {:?}", diff.bntx);

    // Direction check: reversing old/new turns additions into removals.
    let rev = diff_layouts(&gb, &gn, &ob, &on);
    assert_eq!(rev.bflyt.panes_removed.len(), 25);
    assert!(rev.bflyt.panes_added.is_empty());

    // Self-diff is empty for both layouts.
    assert!(diff_layouts(&ob, &on, &ob, &on).is_empty(), "self-diff (orig) not empty");
    assert!(diff_layouts(&gb, &gn, &gb, &gn).is_empty(), "self-diff (gen) not empty");

    println!(
        "OK: layout-diff original->SGPO = {} panes added (sgpo_root + {} markers), BNTX unchanged",
        b.panes_added.len(),
        markers.len()
    );
}
