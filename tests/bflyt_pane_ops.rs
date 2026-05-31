//! Fixture-gated tests for the BFLYT structural pane ops (rename / copy-subtree
//! / remove) against real layouts. Skipped unless `tests/fixtures/` contains
//! `*.bflyt` (gitignored game/mod data). Each op is applied to a freshly-read
//! copy, written, and re-parsed to prove the mutation produces a still-valid
//! BFLYT carrying the change. The fixture-free unit tests in `bflyt::ops` are
//! the exhaustive correctness net; this is the real-bytes guard.

use std::path::{Path, PathBuf};

use nx_layout_toolbox::bflyt::{read_bflyt, write_bflyt, BasePane, BFLYT};

/// First `*.bflyt` under `tests/fixtures` (recursive, sorted), if any.
fn first_bflyt() -> Option<PathBuf> {
    fn walk(dir: &Path, out: &mut Option<PathBuf>) {
        if out.is_some() {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if out.is_some() {
                return;
            }
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().and_then(|e| e.to_str()) == Some("bflyt") {
                *out = Some(p);
            }
        }
    }
    let mut out = None;
    walk(Path::new("tests/fixtures"), &mut out);
    out
}

/// Collect (name, depth, child_count) for every named pane.
fn collect(p: &BasePane, depth: usize, out: &mut Vec<(String, usize, usize)>) {
    if !p.name.is_empty() {
        out.push((p.name.clone(), depth, p.children.len()));
    }
    for c in &p.children {
        collect(c, depth + 1, out);
    }
}

/// Pick a non-root, uniquely-named leaf pane with a short name (so a suffix
/// still fits the 24-byte slot) to operate on safely.
fn pick_leaf(b: &BFLYT) -> Option<String> {
    let mut names = Vec::new();
    collect(b.root_pane.as_ref()?, 0, &mut names);
    names
        .iter()
        .find(|(n, depth, kids)| {
            *depth >= 1
                && *kids == 0
                && n.len() <= 20
                && names.iter().filter(|(m, _, _)| m == n).count() == 1
        })
        .map(|(n, _, _)| n.clone())
}

#[test]
fn rename_copy_remove_round_trip_on_real_bflyt() {
    let Some(path) = first_bflyt() else {
        eprintln!("skipping (no *.bflyt fixtures)");
        return;
    };
    let bytes = std::fs::read(&path).unwrap();
    let base = read_bflyt(&bytes).expect("parse fixture");
    let root_name = base.root_pane.as_ref().unwrap().name.clone();
    let Some(target) = pick_leaf(&base) else {
        eprintln!("skipping (no suitable leaf pane in {})", path.display());
        return;
    };

    // --- rename ---
    let mut b = read_bflyt(&bytes).unwrap();
    let renamed = format!("{target}_R");
    b.rename_pane(&target, &renamed).unwrap();
    let back = read_bflyt(&write_bflyt(&b).unwrap()).unwrap();
    assert!(back.pane_exists(&renamed), "renamed pane should exist");
    assert!(!back.pane_exists(&target), "old name should be gone");

    // --- copy-subtree (leaf) under the root ---
    let mut b = read_bflyt(&bytes).unwrap();
    let copy_name = format!("{target}_C");
    let copied = b
        .copy_subtree(&target, Some(&copy_name), Some(&root_name), "_c")
        .unwrap();
    assert_eq!(copied, 1, "copying a leaf copies exactly one pane");
    let back = read_bflyt(&write_bflyt(&b).unwrap()).unwrap();
    assert!(back.pane_exists(&copy_name), "copied pane should exist");
    assert!(back.pane_exists(&target), "original pane should be preserved");

    // --- remove (leaf) ---
    let mut b = read_bflyt(&bytes).unwrap();
    let removed = b.remove_pane(&target).unwrap();
    assert_eq!(removed, 1, "removing a leaf removes exactly one pane");
    let back = read_bflyt(&write_bflyt(&b).unwrap()).unwrap();
    assert!(!back.pane_exists(&target), "removed pane should be gone");
}

#[test]
fn repair_round_trips_on_real_bflyt() {
    let Some(path) = first_bflyt() else {
        eprintln!("skipping (no *.bflyt fixtures)");
        return;
    };
    let bytes = std::fs::read(&path).unwrap();
    let mut b = read_bflyt(&bytes).expect("parse fixture");
    // Repair without material pruning (the safe default for arbitrary files).
    let _ = b.repair(false);

    // The repaired layout must still serialize + re-parse, with self-consistent
    // cross-references.
    let back = read_bflyt(&write_bflyt(&b).unwrap()).expect("re-parse repaired");

    // Every material->texture reference is in range after repair.
    let n = back.textures.len();
    if n > 0 {
        for m in &back.materials {
            for t in &m.texture_maps {
                assert!(
                    t.index >= 0 && (t.index as usize) < n,
                    "texture ref {} out of range (len {n})",
                    t.index
                );
            }
        }
    }
    // No duplicate pane names remain.
    let mut names = Vec::new();
    collect(back.root_pane.as_ref().unwrap(), 0, &mut names);
    let mut seen = std::collections::HashSet::new();
    for (name, _, _) in &names {
        assert!(seen.insert(name.clone()), "duplicate pane name after repair: {name}");
    }
}
