//! Fixture-gated tests for the BYML structural diff against real TotK data.
//! Skipped unless `tests/fixtures/byml/` is populated (gitignored game data).

use std::path::Path;

use nx_layout_toolbox::byml::{diff_byml, read_byml, Byml};
use nx_layout_toolbox::compression::{self, DictRegistry};

fn byml_dir() -> &'static Path {
    Path::new("tests/fixtures/byml")
}

fn load_registry() -> DictRegistry {
    let pack = Path::new("tests/fixtures/totk/compression/ZsDic.pack.zs");
    if !pack.exists() {
        return DictRegistry::new();
    }
    DictRegistry::from_zsdic_pack(&std::fs::read(pack).unwrap()).expect("load ZsDic")
}

fn load_tree(path: &Path, reg: &DictRegistry) -> Byml {
    let raw = std::fs::read(path).unwrap();
    let bytes = compression::decompress(&raw, reg).unwrap().into_owned();
    read_byml(&bytes).expect("parse").root
}

/// Look up a mutable nested hash value by key.
fn get_mut<'a>(node: &'a mut Byml, key: &str) -> Option<&'a mut Byml> {
    match node {
        Byml::Hash(h) => h.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

#[test]
fn self_diff_of_real_file_is_empty() {
    let path = byml_dir().join("CookingTable.game__cooking__Table.bgyml");
    if !path.exists() {
        eprintln!("skipping (no {})", path.display());
        return;
    }
    let tree = read_byml(&std::fs::read(&path).unwrap()).unwrap().root;
    assert!(diff_byml(&tree, &tree).is_empty(), "a file should not differ from itself");
}

#[test]
fn mutated_clone_diff_is_precise() {
    let path = byml_dir().join("CookingTable.game__cooking__Table.bgyml");
    if !path.exists() {
        eprintln!("skipping (no {})", path.display());
        return;
    }
    let original = read_byml(&std::fs::read(&path).unwrap()).unwrap().root;

    // Clone and apply three known edits: add a root key, change one nested
    // string leaf, and remove one nested key.
    let mut edited = original.clone();
    if let Byml::Hash(h) = &mut edited {
        h.push(("__diff_test_added".into(), Byml::U32(42)));
    }
    let system = get_mut(&mut edited, "SystemData").expect("SystemData");
    if let Byml::Hash(sd) = system {
        for (k, v) in sd.iter_mut() {
            if k == "EnemyExtractActorName" {
                *v = Byml::String("Item_TEST".into());
            }
        }
        sd.retain(|(k, _)| k != "FairyActorName");
    }

    let d = diff_byml(&original, &edited);
    assert_eq!(
        d.added.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(),
        ["/__diff_test_added"]
    );
    assert_eq!(
        d.removed.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(),
        ["/SystemData/FairyActorName"]
    );
    assert_eq!(d.changed.len(), 1);
    assert_eq!(d.changed[0].path, "/SystemData/EnemyExtractActorName");
    assert_eq!(d.changed[0].old, "string(\"Item_Material_08\")");
    assert_eq!(d.changed[0].new, "string(\"Item_TEST\")");
}

/// Diff two real game versions of the actor database (when both are present)
/// and check the reverse diff is the mirror image.
#[test]
fn actor_info_version_pair_diff() {
    let old_p = byml_dir().join("ActorInfo.Product.121.rstbl.byml.zs");
    let new_p = byml_dir().join("ActorInfo.Product.143.rstbl.byml.zs");
    if !old_p.exists() || !new_p.exists() {
        eprintln!("skipping (need both ActorInfo.121/.143 fixtures)");
        return;
    }
    let reg = load_registry();
    if reg.is_empty() {
        eprintln!("skipping (no ZsDic.pack.zs fixture)");
        return;
    }
    let old = load_tree(&old_p, &reg);
    let new = load_tree(&new_p, &reg);

    let fwd = diff_byml(&old, &new);
    assert!(!fwd.is_empty(), "two different game versions should differ");

    // The reverse diff swaps additions/removals and keeps the same changes.
    let rev = diff_byml(&new, &old);
    assert_eq!(rev.added.len(), fwd.removed.len());
    assert_eq!(rev.removed.len(), fwd.added.len());
    assert_eq!(rev.changed.len(), fwd.changed.len());
    eprintln!(
        "ActorInfo 121->143: +{} -{} ~{}",
        fwd.added.len(),
        fwd.removed.len(),
        fwd.changed.len()
    );
}
