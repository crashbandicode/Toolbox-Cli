//! Tests for `BntxFile::remove_texture` (and by extension the
//! `bntx-remove-texture` verb).
//!
//! Verifies the structural-change semantics:
//! - texture count drops by exactly 1
//! - the removed name disappears from the string pool, dict, and BRTI
//!   array
//! - every other texture's pixel data, dimensions, and metadata are
//!   preserved byte-identically
//! - the resulting BNTX writes successfully and re-parses back to the
//!   same shape
//! - works for textures at the start, middle, and end of the BRTI
//!   array (different BRTD compaction paths)
//!
//! Skipped when `tests/fixtures/bntx/` is absent.

use std::fs;
use std::path::Path;

use nx_layout_toolbox::bntx::{read_bntx, write_bntx, BntxFile};

/// Snapshot of a texture's identity + pixel bytes, captured before a
/// remove operation so we can verify it's preserved afterward.
struct TexSnapshot {
    name: String,
    width: u32,
    height: u32,
    image_size: u32,
    pixel_data: Vec<u8>,
}

fn snapshot_all_except(b: &BntxFile, removed_name: &str) -> Vec<TexSnapshot> {
    b.textures
        .iter()
        .filter(|t| t.name(b) != removed_name)
        .map(|t| TexSnapshot {
            name: t.name(b).to_string(),
            width: t.width,
            height: t.height,
            image_size: t.image_size,
            pixel_data: t.pixel_data(&b.brtd).to_vec(),
        })
        .collect()
}

fn assert_snapshots_preserved(snaps: &[TexSnapshot], modified: &BntxFile) {
    for snap in snaps {
        let idx = modified
            .texture_index_by_name(&snap.name)
            .unwrap_or_else(|| panic!("texture '{}' missing after remove", snap.name));
        let t = &modified.textures[idx];
        assert_eq!(t.width, snap.width, "width changed for '{}'", snap.name);
        assert_eq!(t.height, snap.height, "height changed for '{}'", snap.name);
        assert_eq!(
            t.image_size, snap.image_size,
            "image_size changed for '{}'",
            snap.name
        );
        assert_eq!(
            t.pixel_data(&modified.brtd),
            &snap.pixel_data[..],
            "pixel data changed for '{}'",
            snap.name
        );
    }
}

fn load_fixture() -> Option<BntxFile> {
    let path = Path::new("tests/fixtures/bntx/info_melee_original__Combined.bntx");
    if !path.exists() {
        eprintln!(
            "skipping bntx_remove_texture test (no fixture at {})",
            path.display()
        );
        return None;
    }
    let bytes = fs::read(path).expect("read fixture");
    Some(read_bntx(&bytes).expect("parse fixture"))
}

/// Remove a middle texture: this exercises the BRTD-compaction path
/// (subsequent textures' data slides forward, with each texture's own
/// alignment re-applied to the new offset).
#[test]
fn remove_middle_texture_preserves_others() {
    let Some(parsed) = load_fixture() else { return };
    let original_count = parsed.textures.len();
    assert!(
        original_count >= 3,
        "fixture must have at least 3 textures to exercise the middle path"
    );

    let target_idx = original_count / 2;
    let target_name = parsed.textures[target_idx].name(&parsed).to_string();
    let snapshots = snapshot_all_except(&parsed, &target_name);

    let mut modified = parsed.clone();
    modified
        .remove_texture(&target_name)
        .expect("remove_texture middle");

    assert_eq!(modified.textures.len(), original_count - 1);
    assert!(
        modified.texture_index_by_name(&target_name).is_none(),
        "removed texture '{target_name}' still present after remove",
    );
    assert!(
        !modified.strings.iter().any(|s| s == &target_name),
        "removed name '{target_name}' still in string pool",
    );
    assert_eq!(
        modified.dict.entries.len(),
        modified.textures.len() + 1,
        "dict entry count must equal textures + 1 (root)"
    );
    assert!(
        modified.relocation_table_dirty,
        "remove_texture must mark RLT dirty"
    );

    assert_snapshots_preserved(&snapshots, &modified);

    // The modified file must serialize and round-trip through the
    // parser cleanly.
    let written = write_bntx(&modified).expect("write modified BNTX");
    let reparsed = read_bntx(&written).expect("re-parse modified BNTX");
    assert_eq!(reparsed.textures.len(), original_count - 1);
    assert!(reparsed.texture_index_by_name(&target_name).is_none());
    // And every preserved snapshot still matches in the re-parsed copy.
    assert_snapshots_preserved(&snapshots, &reparsed);
}

/// Remove the very last texture: no BRTD compaction is needed for any
/// other texture; the operation reduces to "drop the last slot".
#[test]
fn remove_last_texture_preserves_earlier() {
    let Some(parsed) = load_fixture() else { return };
    let original_count = parsed.textures.len();
    let target_name = parsed
        .textures
        .last()
        .expect("fixture has textures")
        .name(&parsed)
        .to_string();
    let snapshots = snapshot_all_except(&parsed, &target_name);

    let mut modified = parsed.clone();
    modified
        .remove_texture(&target_name)
        .expect("remove_texture last");

    assert_eq!(modified.textures.len(), original_count - 1);
    assert!(modified.texture_index_by_name(&target_name).is_none());
    assert_snapshots_preserved(&snapshots, &modified);

    let written = write_bntx(&modified).expect("write");
    let reparsed = read_bntx(&written).expect("re-parse");
    assert_snapshots_preserved(&snapshots, &reparsed);
}

/// Remove the first texture: every other texture's BRTD offset shifts.
#[test]
fn remove_first_texture_preserves_later() {
    let Some(parsed) = load_fixture() else { return };
    let original_count = parsed.textures.len();
    let target_name = parsed
        .textures
        .first()
        .expect("fixture has textures")
        .name(&parsed)
        .to_string();
    let snapshots = snapshot_all_except(&parsed, &target_name);

    let mut modified = parsed.clone();
    modified
        .remove_texture(&target_name)
        .expect("remove_texture first");

    assert_eq!(modified.textures.len(), original_count - 1);
    assert_snapshots_preserved(&snapshots, &modified);

    let written = write_bntx(&modified).expect("write");
    let reparsed = read_bntx(&written).expect("re-parse");
    assert_snapshots_preserved(&snapshots, &reparsed);
}

/// Removing a missing name must error cleanly and leave the BNTX
/// untouched.
#[test]
fn remove_nonexistent_texture_errors_and_preserves_state() {
    let Some(parsed) = load_fixture() else { return };
    let original_count = parsed.textures.len();
    let original_brtd_len = parsed.brtd.data.len();

    let mut modified = parsed.clone();
    let err = modified
        .remove_texture("definitely_not_a_real_texture_name_xyz")
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("not found"),
        "expected 'not found' in error, got: {err}",
    );
    assert_eq!(modified.textures.len(), original_count);
    assert_eq!(modified.brtd.data.len(), original_brtd_len);
    assert!(!modified.relocation_table_dirty);
}

/// Remove + append back round-trip: removing a texture and then
/// appending the same name with the same data must end up with the
/// same effective BNTX content (even if BRTI ordering / RLT layout
/// differs from the original).
#[test]
fn remove_then_append_back_yields_same_named_set() {
    use nx_layout_toolbox::bntx::AppendTextureSpec;

    let Some(parsed) = load_fixture() else { return };
    let original_count = parsed.textures.len();
    let target_idx = original_count / 2;
    let target_name = parsed.textures[target_idx].name(&parsed).to_string();

    // Capture the target's full spec and pixel bytes so we can re-add
    // it with byte-identical data.
    let target = &parsed.textures[target_idx];
    let target_bytes = target.pixel_data(&parsed.brtd).to_vec();
    let spec = AppendTextureSpec {
        format: target.format,
        width: target.width,
        height: target.height,
        depth: target.depth,
        mips_count: target.mips_count,
        array_len: target.array_len,
        size_range: target.size_range,
        channel_swizzle: target.channel_swizzle,
        align: target.align,
        flags: target.flags,
        dim: target.dim,
        tile_mode: target.tile_mode,
        swizzle: target.swizzle,
        num_multi_sample: target.num_multi_sample,
        unk2: target.unk2,
        unk4: target.unk4,
        ty: target.ty,
        parent_addr: target.parent_addr,
        swizzled_data: target_bytes.clone(),
    };

    let mut modified = parsed.clone();
    modified.remove_texture(&target_name).expect("remove");
    modified
        .append_texture(target_name.clone(), spec)
        .expect("append back");

    assert_eq!(modified.textures.len(), original_count);
    let new_idx = modified
        .texture_index_by_name(&target_name)
        .expect("re-added texture present");
    assert_eq!(
        modified.textures[new_idx].pixel_data(&modified.brtd),
        &target_bytes[..],
        "re-added texture must have the same pixel bytes"
    );

    // The re-added texture lives at the end of the array (append
    // pushes), so its index will differ from the original target_idx.
    // What matters is the file still serializes + re-parses.
    let written = write_bntx(&modified).expect("write");
    let reparsed = read_bntx(&written).expect("re-parse");
    assert_eq!(reparsed.textures.len(), original_count);
    let reparsed_idx = reparsed
        .texture_index_by_name(&target_name)
        .expect("texture present in re-parsed");
    assert_eq!(
        reparsed.textures[reparsed_idx].pixel_data(&reparsed.brtd),
        &target_bytes[..],
    );
}
