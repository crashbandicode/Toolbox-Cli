//! End-to-end `layout-apply-arc` against `info_melee_original.layout.arc`.
//!
//! Applies a small in-code manifest (panes cloned from the stock
//! `set_rep_stock_01` template under `RootPane`) to the packed archive and
//! verifies the full unpack → apply → validate → repack pipeline:
//! every element validates, the entry count is preserved, only the
//! targeted BFLYT + BNTX entries change, the repacked archive re-opens
//! and re-validates, and a `skip_existing` re-run is a no-op.
//!
//! Skipped when the fixture archive is absent.

use std::collections::BTreeMap;
use std::path::Path;

use nx_layout_toolbox::bntx::read_bntx;
use nx_layout_toolbox::bflyt::read_bflyt;
use nx_layout_toolbox::layout::{apply_manifest_to_arc, validate_manifest_in_memory, ApplyOptions};
use nx_layout_toolbox::manifest::{SkinElement, SkinManifest};
use nx_layout_toolbox::sarc::read_arc;
use nx_layout_toolbox::texpipe::Bc7Quality;

fn element(control_id: &str, pane: &str, image: &str) -> SkinElement {
    SkinElement {
        control_id: control_id.to_string(),
        pane_name: pane.to_string(),
        image_filename: image.to_string(),
        material_name: format!("mat_{pane}"),
        base_x: 120.0,
        base_y: -40.0,
        width: 64.0,
        height: 64.0,
        released_alpha: 200,
        pressed_alpha: 255,
        released_scale: 1.0,
        pressed_scale: 1.05,
    }
}

fn write_test_png(path: &Path, w: u32, h: u32) {
    let mut img = image::RgbaImage::new(w, h);
    for (x, y, px) in img.enumerate_pixels_mut() {
        *px = image::Rgba([(x * 4) as u8, (y * 4) as u8, 0x80, ((x + y) * 2) as u8]);
    }
    img.save(path).expect("write test png");
}

#[test]
fn apply_arc_round_trips_against_info_melee() {
    let arc_path = Path::new("tests/fixtures/archives/info_melee_original.layout.arc");
    if !arc_path.exists() {
        eprintln!("skipping layout-apply-arc test (no fixture at {})", arc_path.display());
        return;
    }
    let arc_bytes = std::fs::read(arc_path).expect("read arc fixture");
    let input_arc = read_arc(&arc_bytes).expect("parse input arc");
    let input_count = input_arc.files.len();

    // Map of name -> data for every input entry, to confirm later that
    // only the BFLYT + BNTX entries changed.
    let input_by_name: BTreeMap<String, Vec<u8>> = input_arc
        .files
        .iter()
        .filter_map(|f| f.name.clone().map(|n| (n, f.data.clone())))
        .collect();

    let skin = tempfile::tempdir().expect("temp skin dir");
    write_test_png(&skin.path().join("btn.png"), 64, 64);

    let manifest = SkinManifest {
        schema_version: 1,
        skin_name: "agent_apply_arc_test".into(),
        root_pane_name: "RootPane".into(),
        expected_layout_flavor: String::new(),
        elements: vec![
            element("A", "sgpo_test_a", "btn.png"),
            element("B", "sgpo_test_b", "btn.png"),
        ],
    };

    let opts = ApplyOptions {
        quality: Bc7Quality::UltraFast,
        ..Default::default()
    };

    let (out_arc, report) =
        apply_manifest_to_arc(&arc_bytes, &manifest, skin.path(), &opts, false).expect("apply arc");

    assert_eq!(report.applied, 2, "both elements should apply");
    assert_eq!(report.skipped, 0);
    assert!(
        report.validation.all_passed(),
        "post-apply validation failed: {:?}",
        report.validation.results.iter().filter(|r| !r.ok).collect::<Vec<_>>()
    );
    assert_eq!(report.file_count, input_count, "entry count must be preserved");

    // Re-open the produced archive and re-validate independently.
    let out = read_arc(&out_arc).expect("parse output arc");
    assert_eq!(out.files.len(), input_count);

    let bflyt_idx = out.position(&opts.bflyt_rel).expect("bflyt in output");
    let bntx_idx = out.position(&opts.bntx_rel).expect("bntx in output");
    let bflyt = read_bflyt(&out.files[bflyt_idx].data).expect("parse output bflyt");
    let bntx = read_bntx(&out.files[bntx_idx].data).expect("parse output bntx");

    let revalidation = validate_manifest_in_memory(&bflyt, &bntx, &manifest, false);
    assert!(
        revalidation.all_passed(),
        "re-opened archive failed validation: {revalidation:?}"
    );

    // The two new textures + panes + materials must be present.
    for el in &manifest.elements {
        assert!(
            bntx.texture_index_by_name(&el.texture_name()).is_some(),
            "BNTX missing texture {}",
            el.texture_name()
        );
        assert!(
            bflyt.pane_exists(&el.pane_name),
            "BFLYT missing pane {}",
            el.pane_name
        );
        assert!(
            bflyt.materials.iter().any(|m| m.name == el.material_name),
            "BFLYT missing material {}",
            el.material_name
        );
    }

    // Every entry OTHER than the edited BFLYT/BNTX must be byte-identical
    // to the input (the repack preserved them verbatim).
    let mut changed = Vec::new();
    for f in &out.files {
        let Some(name) = &f.name else { continue };
        if name == &opts.bflyt_rel || name == &opts.bntx_rel {
            continue;
        }
        match input_by_name.get(name) {
            Some(orig) => {
                if orig != &f.data {
                    changed.push(name.clone());
                }
            }
            None => panic!("output gained an unexpected entry '{name}'"),
        }
    }
    assert!(
        changed.is_empty(),
        "non-target entries changed during repack: {changed:?}"
    );

    // Idempotent re-run: applying the same manifest with skip_existing on
    // the produced archive applies nothing.
    let opts_skip = ApplyOptions {
        skip_existing: true,
        quality: Bc7Quality::UltraFast,
        ..Default::default()
    };
    let (_out2, report2) =
        apply_manifest_to_arc(&out_arc, &manifest, skin.path(), &opts_skip, false)
            .expect("idempotent re-apply");
    assert_eq!(report2.applied, 0, "re-apply should add nothing");
    assert_eq!(report2.skipped, 2, "re-apply should skip both existing elements");
    assert!(report2.validation.all_passed());

    println!(
        "OK: layout-apply-arc applied 2 elements to info_melee ({} entries preserved, {} -> {} bytes)",
        input_count,
        arc_bytes.len(),
        out_arc.len()
    );
}
