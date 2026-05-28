//! Focused `prt1` / `wnd1` round-trip tests.
//!
//! `tests/bflyt_real_fixtures.rs` already walks the entire 508-file
//! corpus and asserts byte-identical round-trip on every BFLYT. That's
//! great as a regression net but gives no signal about *which* feature
//! broke when something fails. The tests here pin down two specific
//! complexity surfaces — `wnd1` panes with multiple frames /
//! tex-coord sets, and `prt1` panes with multiple `PartsProperty`
//! entries — and round-trip the most-extreme example from the fixture
//! corpus for each. A regression in either area lights up these tests
//! by name, pointing future contributors at the right code.
//!
//! Both tests are programmatic-discovery: they walk the fixture tree,
//! find the BFLYT containing the highest-frame-count `wnd1` (or
//! highest-property-count `prt1`), and run their assertions on that
//! file. This keeps the tests robust to additions / removals in the
//! fixture set without losing the "interesting case" coverage.
//!
//! Skipped when `tests/fixtures/` is absent.

use std::fs;
use std::path::{Path, PathBuf};

use toolbox_cli::bflyt::*;

fn walk_bflyts(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !dir.exists() {
        return out;
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = match fs::read_dir(&d) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|s| s.to_str()) == Some("bflyt") {
                out.push(path);
            }
        }
    }
    out
}

fn find_pane<'a>(root: Option<&'a BasePane>, name: &str) -> Option<&'a BasePane> {
    fn rec<'a>(p: &'a BasePane, name: &str) -> Option<&'a BasePane> {
        if p.name == name {
            return Some(p);
        }
        for c in &p.children {
            if let Some(found) = rec(c, name) {
                return Some(found);
            }
        }
        None
    }
    root.and_then(|r| rec(r, name))
}

fn for_each_pane(p: &BasePane, f: &mut dyn FnMut(&BasePane)) {
    f(p);
    for c in &p.children {
        for_each_pane(c, f);
    }
}

/// Round-trip the BFLYT containing the most-complex `wnd1` pane in the
/// fixture corpus, and verify the wnd1's structural fields are
/// preserved bit-for-bit through the parse → write → parse cycle.
#[test]
fn most_complex_wnd1_round_trips_byte_identically() {
    let root = Path::new("tests/fixtures");
    if !root.exists() {
        eprintln!("skipping prt1/wnd1 test (no fixtures at {})", root.display());
        return;
    }
    let bflyts = walk_bflyts(root);
    assert!(!bflyts.is_empty(), "fixture sweep should find at least one BFLYT");

    // Score = frame_count * 100 + tex_coord_count * 10 + has_picture-style
    // packing. We weight frame_count highest because that's the most
    // structurally interesting wnd1 dimension (each frame is a separate
    // sub-section in the file).
    let mut best: Option<(PathBuf, String, usize, usize, BFLYT)> = None;
    for path in &bflyts {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let bflyt = match read_bflyt(&bytes) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if let Some(rp) = &bflyt.root_pane {
            let mut local_best: Option<(String, usize, usize)> = None;
            for_each_pane(rp, &mut |p| {
                if let Some(w) = &p.window {
                    let score = (w.frame_count as usize) * 100 + w.content.tex_coords.len() * 10;
                    let challenger_score = local_best
                        .as_ref()
                        .map(|(_, fc, tc)| fc * 100 + tc * 10)
                        .unwrap_or(0);
                    if score > challenger_score {
                        local_best = Some((
                            p.name.clone(),
                            w.frame_count as usize,
                            w.content.tex_coords.len(),
                        ));
                    }
                }
            });
            if let Some((pane_name, frames, tex_coords)) = local_best {
                let our_score = frames * 100 + tex_coords * 10;
                let best_score = best
                    .as_ref()
                    .map(|(_, _, fc, tc, _)| fc * 100 + tc * 10)
                    .unwrap_or(0);
                if our_score > best_score {
                    best = Some((path.clone(), pane_name, frames, tex_coords, bflyt));
                }
            }
        }
    }

    let (path, pane_name, frames, tex_coords, parsed) =
        best.expect("expected at least one wnd1 in the fixture corpus");
    assert!(
        frames >= 1,
        "the most-complex wnd1 should have at least 1 frame; pane '{}' from {} has {}",
        pane_name,
        path.display(),
        frames,
    );
    println!(
        "most-complex wnd1: pane '{pane_name}' ({frames} frames, {tex_coords} tex_coords) in {}",
        path.display()
    );

    // (1) byte-identical round-trip on the whole file.
    let original_bytes = fs::read(&path).expect("re-read fixture");
    let written = write_bflyt(&parsed).expect("write");
    assert_eq!(
        written,
        original_bytes,
        "BFLYT containing complex wnd1 ({}) did not round-trip byte-identically",
        path.display()
    );

    // (2) the wnd1's structural detail survives a re-parse.
    let reparsed = read_bflyt(&written).expect("re-parse the written bytes");
    let original_pane = find_pane(parsed.root_pane.as_ref(), &pane_name)
        .expect("locate wnd1 in original parse");
    let recovered_pane = find_pane(reparsed.root_pane.as_ref(), &pane_name)
        .expect("locate wnd1 in re-parsed copy");
    let orig = original_pane
        .window
        .as_ref()
        .expect("original pane should be a wnd1");
    let again = recovered_pane
        .window
        .as_ref()
        .expect("re-parsed pane should also be a wnd1");

    assert_eq!(orig.frame_count, again.frame_count);
    assert_eq!(orig.flag, again.flag);
    assert_eq!(orig.frames.len(), again.frames.len());
    for (i, (a, b)) in orig.frames.iter().zip(again.frames.iter()).enumerate() {
        assert_eq!(
            a.material_index, b.material_index,
            "wnd1 frame[{i}] material_index regressed: {} != {}",
            a.material_index, b.material_index,
        );
        assert_eq!(
            a.texture_flip, b.texture_flip,
            "wnd1 frame[{i}] texture_flip regressed",
        );
    }
    assert_eq!(
        orig.content.tex_coords.len(),
        again.content.tex_coords.len(),
        "wnd1 content tex_coord count regressed",
    );
    for (i, (a, b)) in orig
        .content
        .tex_coords
        .iter()
        .zip(again.content.tex_coords.iter())
        .enumerate()
    {
        assert_eq!(
            (a.top_left.x, a.top_left.y),
            (b.top_left.x, b.top_left.y),
            "wnd1 content tex_coord[{i}].top_left regressed",
        );
        assert_eq!(
            (a.bottom_right.x, a.bottom_right.y),
            (b.bottom_right.x, b.bottom_right.y),
            "wnd1 content tex_coord[{i}].bottom_right regressed",
        );
    }
    assert_eq!(orig.stretch_l, again.stretch_l);
    assert_eq!(orig.stretch_r, again.stretch_r);
    assert_eq!(orig.stretch_t, again.stretch_t);
    assert_eq!(orig.stretch_b, again.stretch_b);
    assert_eq!(orig.frame_size_l, again.frame_size_l);
    assert_eq!(orig.frame_size_r, again.frame_size_r);
    assert_eq!(orig.frame_size_t, again.frame_size_t);
    assert_eq!(orig.frame_size_b, again.frame_size_b);
}

/// Round-trip the BFLYT containing the most-complex `prt1` pane in the
/// fixture corpus. `prt1` panes embed `PartsProperty` entries plus
/// raw property/ext-user-data sub-sections that we capture verbatim
/// (`raw_property_data`); both axes contribute to the "complex" score.
#[test]
fn most_complex_prt1_round_trips_byte_identically() {
    let root = Path::new("tests/fixtures");
    if !root.exists() {
        eprintln!("skipping prt1/wnd1 test (no fixtures at {})", root.display());
        return;
    }
    let bflyts = walk_bflyts(root);
    assert!(!bflyts.is_empty());

    // Score weights: property_count is the primary axis (each entry
    // is a 40-byte struct + offsets into `raw_property_data`), and
    // raw_property_data length is secondary. A prt1 with 0 properties
    // is uninteresting from a regression standpoint — those don't get
    // scored.
    let mut best: Option<(PathBuf, String, usize, usize, BFLYT)> = None;
    for path in &bflyts {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let bflyt = match read_bflyt(&bytes) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if let Some(rp) = &bflyt.root_pane {
            let mut local_best: Option<(String, usize, usize)> = None;
            for_each_pane(rp, &mut |p| {
                if let Some(prt) = &p.parts {
                    if (prt.property_count as usize) == 0 {
                        return;
                    }
                    let score = (prt.property_count as usize) * 1_000_000
                        + prt.raw_property_data.len();
                    let challenger_score = local_best
                        .as_ref()
                        .map(|(_, pc, rd)| pc * 1_000_000 + rd)
                        .unwrap_or(0);
                    if score > challenger_score {
                        local_best = Some((
                            p.name.clone(),
                            prt.property_count as usize,
                            prt.raw_property_data.len(),
                        ));
                    }
                }
            });
            if let Some((name, pc, rd)) = local_best {
                let our_score = pc * 1_000_000 + rd;
                let best_score = best
                    .as_ref()
                    .map(|(_, _, pc, rd, _)| pc * 1_000_000 + rd)
                    .unwrap_or(0);
                if our_score > best_score {
                    best = Some((path.clone(), name, pc, rd, bflyt));
                }
            }
        }
    }

    let Some((path, pane_name, property_count, raw_len, parsed)) = best else {
        eprintln!(
            "skipping prt1 round-trip: fixture corpus contains no prt1 with property_count > 0"
        );
        return;
    };
    println!(
        "most-complex prt1: pane '{pane_name}' ({property_count} properties, {raw_len} raw bytes) in {}",
        path.display()
    );

    let original_bytes = fs::read(&path).expect("re-read fixture");
    let written = write_bflyt(&parsed).expect("write");
    assert_eq!(
        written,
        original_bytes,
        "BFLYT containing complex prt1 ({}) did not round-trip byte-identically",
        path.display()
    );

    // Re-parse and verify the `PartsProperty` table + raw_property_data
    // survives intact. `declared_size` is recomputed by the writer, so
    // we don't compare it directly — the byte-identical round-trip
    // above already proves the recomputation matches the original.
    let reparsed = read_bflyt(&written).expect("re-parse");
    let orig_pane = find_pane(parsed.root_pane.as_ref(), &pane_name).expect("orig prt1");
    let again_pane = find_pane(reparsed.root_pane.as_ref(), &pane_name).expect("re-parsed prt1");
    let orig = orig_pane.parts.as_ref().expect("original is prt1");
    let again = again_pane.parts.as_ref().expect("re-parsed is prt1");

    assert_eq!(orig.property_count, again.property_count);
    assert_eq!(orig.properties.len(), again.properties.len());
    assert_eq!(orig.part_name, again.part_name);
    assert_eq!(orig.raw_property_data, again.raw_property_data);
    assert_eq!(
        (orig.magnify.x, orig.magnify.y),
        (again.magnify.x, again.magnify.y)
    );
    for (i, (a, b)) in orig
        .properties
        .iter()
        .zip(again.properties.iter())
        .enumerate()
    {
        assert_eq!(a.name, b.name, "prt1 property[{i}].name regressed");
        assert_eq!(
            a.usage_flag, b.usage_flag,
            "prt1 property[{i}].usage_flag regressed",
        );
        assert_eq!(
            a.basic_usage_flag, b.basic_usage_flag,
            "prt1 property[{i}].basic_usage_flag regressed",
        );
        assert_eq!(
            a.material_usage_flag, b.material_usage_flag,
            "prt1 property[{i}].material_usage_flag regressed",
        );
        assert_eq!(
            a.system_ext_user_data_override_flag, b.system_ext_user_data_override_flag,
            "prt1 property[{i}].system_ext_user_data_override_flag regressed",
        );
        assert_eq!(
            a.property_offset, b.property_offset,
            "prt1 property[{i}].property_offset regressed",
        );
        assert_eq!(
            a.ext_user_data_offset, b.ext_user_data_offset,
            "prt1 property[{i}].ext_user_data_offset regressed",
        );
        assert_eq!(
            a.pane_basic_info_offset, b.pane_basic_info_offset,
            "prt1 property[{i}].pane_basic_info_offset regressed",
        );
    }
}

/// Sanity check: confirm the fixture corpus actually contains both
/// non-trivial wnd1 (frame_count >= 1) AND non-trivial prt1
/// (property_count >= 1) examples. If this fails, the targeted
/// round-trip tests above are silently coverage-empty and someone
/// needs to expand the fixture set.
#[test]
fn fixture_corpus_contains_non_trivial_prt1_and_wnd1() {
    let root = Path::new("tests/fixtures");
    if !root.exists() {
        eprintln!("skipping fixture-coverage test (no fixtures at {})", root.display());
        return;
    }
    let bflyts = walk_bflyts(root);
    let mut wnd_with_frames = 0usize;
    let mut prt_with_properties = 0usize;
    for path in &bflyts {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let bflyt = match read_bflyt(&bytes) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if let Some(rp) = &bflyt.root_pane {
            for_each_pane(rp, &mut |p| {
                if let Some(w) = &p.window {
                    if w.frame_count >= 1 {
                        wnd_with_frames += 1;
                    }
                }
                if let Some(prt) = &p.parts {
                    if prt.property_count >= 1 {
                        prt_with_properties += 1;
                    }
                }
            });
        }
    }
    assert!(
        wnd_with_frames > 0,
        "fixture corpus has no wnd1 with frame_count >= 1; the wnd1 round-trip test is coverage-empty",
    );
    assert!(
        prt_with_properties > 0,
        "fixture corpus has no prt1 with property_count >= 1; the prt1 round-trip test is coverage-empty",
    );
    println!(
        "coverage: {wnd_with_frames} non-trivial wnd1 panes, {prt_with_properties} non-trivial prt1 panes in {} fixtures",
        bflyts.len(),
    );
}
