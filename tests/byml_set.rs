//! Fixture-gated tests for `byml-set` (BYML mutation-by-path) against real TotK
//! data. Skipped unless `tests/fixtures/byml/` is populated (gitignored game
//! data). Exercises the library path the verb wraps: `read_byml` ->
//! `set_by_path` -> `write_byml_canonical` -> `read_byml`, asserting the edited
//! document re-parses and differs from the original by *exactly* the one leaf
//! we changed (the core safety property — a single edit must not perturb the
//! rest of the tree).

use std::path::{Path, PathBuf};

use nx_layout_toolbox::byml::{
    diff_byml, read_byml, set_by_path, write_byml_canonical, Byml, ScalarType,
};

fn cooking_table() -> PathBuf {
    Path::new("tests/fixtures/byml").join("CookingTable.game__cooking__Table.bgyml")
}

/// Fetch a leaf by a `byml-diff`-style path (immutable), for assertions.
fn get<'a>(root: &'a Byml, path: &str) -> Option<&'a Byml> {
    let mut node = root;
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        node = match node {
            Byml::Hash(h) => &h.iter().find(|(k, _)| k == seg)?.1,
            Byml::Array(a) => a.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(node)
}

#[test]
fn edit_real_string_leaf_changes_exactly_one_path() {
    let path = cooking_table();
    if !path.exists() {
        eprintln!("skipping (no {})", path.display());
        return;
    }
    let doc = read_byml(&std::fs::read(&path).unwrap()).unwrap();
    let original = doc.root.clone();
    assert_eq!(
        get(&original, "/RecipeList/0/ResultActorName"),
        Some(&Byml::String("Item_Cook_C_16".into())),
        "fixture shape changed; update the target path"
    );

    let mut root = doc.root;
    let report =
        set_by_path(&mut root, "/RecipeList/0/ResultActorName", "Item_Cook_TEST", None).unwrap();
    assert_eq!(report.path, "/RecipeList/0/ResultActorName");
    assert_eq!(report.old, "string(\"Item_Cook_C_16\")");
    assert_eq!(report.new, "string(\"Item_Cook_TEST\")");

    // Canonical write -> re-read: the edit must round-trip, and the only
    // difference from the original is the one leaf we set.
    let bytes = write_byml_canonical(doc.version, doc.big_endian, &root).unwrap();
    let reread = read_byml(&bytes).unwrap().root;
    assert_eq!(
        get(&reread, "/RecipeList/0/ResultActorName"),
        Some(&Byml::String("Item_Cook_TEST".into()))
    );

    let d = diff_byml(&original, &reread);
    assert!(d.added.is_empty(), "no additions, got {:?}", d.added);
    assert!(d.removed.is_empty(), "no removals, got {:?}", d.removed);
    assert_eq!(d.changed.len(), 1, "exactly one change, got {:?}", d.changed);
    assert_eq!(d.changed[0].path, "/RecipeList/0/ResultActorName");
}

#[test]
fn type_preserving_numeric_edit_keeps_kind() {
    let path = cooking_table();
    if !path.exists() {
        eprintln!("skipping (no {})", path.display());
        return;
    }
    let doc = read_byml(&std::fs::read(&path).unwrap()).unwrap();
    let original = doc.root.clone();
    let target = "/RecipeList/0/SingleRecipeMaterialNum";
    let before = get(&original, target).expect("numeric leaf present").clone();

    let mut root = doc.root;
    set_by_path(&mut root, target, "7", None).unwrap();
    let after = get(&root, target).expect("numeric leaf present").clone();

    // Type-preserving: same Byml variant, new value.
    assert_eq!(
        std::mem::discriminant(&before),
        std::mem::discriminant(&after),
        "type-preserving edit must keep the node kind"
    );
    assert_ne!(before, after);

    let bytes = write_byml_canonical(doc.version, doc.big_endian, &root).unwrap();
    let reread = read_byml(&bytes).unwrap().root;
    let d = diff_byml(&original, &reread);
    assert_eq!(d.total(), 1, "exactly one change, got {d:?}");
    assert_eq!(d.changed[0].path, target);
}

#[test]
fn type_override_changes_kind_on_real_file() {
    let path = cooking_table();
    if !path.exists() {
        eprintln!("skipping (no {})", path.display());
        return;
    }
    let doc = read_byml(&std::fs::read(&path).unwrap()).unwrap();
    let mut root = doc.root;

    // Force a string leaf to a u32 via an explicit type override.
    set_by_path(
        &mut root,
        "/RecipeList/0/ResultActorName",
        "123",
        Some(ScalarType::U32),
    )
    .unwrap();

    let bytes = write_byml_canonical(doc.version, doc.big_endian, &root).unwrap();
    let reread = read_byml(&bytes).unwrap().root;
    assert_eq!(
        get(&reread, "/RecipeList/0/ResultActorName"),
        Some(&Byml::U32(123))
    );
}
