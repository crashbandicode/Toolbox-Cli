//! BFLYT cleanup / repair operations.
//!
//! These tidy a layout that mod tooling (or hand-editing) has left in a
//! questionable state, while preserving the round-trip discipline: the writer
//! rebuilds all section sizes/offsets, so removing materials/textures or
//! renaming panes just needs the in-memory cross-references kept consistent.
//!
//! Cross-reference model (verified against the reader/writer):
//! - Panes reference **materials** by index: `pic1`/`txt1`/`wnd1` content +
//!   `wnd1` frames each carry a `material_index`.
//! - Materials reference **textures** by index: `texture_maps[].index` into
//!   `txl1` (a negative or out-of-range value is a dangling reference).
//! - Groups (`grp1`) reference panes **by name**.
//!
//! `prt1` (parts) panes reference an *external* part layout (and that part's
//! own materials), not this file's `mat1`, so material pruning is safe in their
//! presence — but their embedded property data is opaque to us, so
//! [`BFLYT::repair`] still skips material pruning when such data is present
//! unless the caller has confirmed it (defensive default).

use std::collections::HashSet;

use serde::Serialize;

use super::sections::{BasePane, PANE_NAME_LEN};
use super::BFLYT;

/// Summary of what a [`BFLYT::repair`] pass changed.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct RepairReport {
    /// Panes renamed to resolve duplicate names: `(old, new)`.
    pub renamed_panes: Vec<(String, String)>,
    /// Dangling material→texture references clamped into range (or dropped
    /// when the layout has no textures at all).
    pub fixed_texture_refs: usize,
    /// Names of materials removed as unreferenced.
    pub removed_materials: Vec<String>,
    /// Names of textures removed as unreferenced.
    pub removed_textures: Vec<String>,
    /// True when material pruning was requested but skipped because the layout
    /// has `prt1` panes carrying opaque property data (which may reference
    /// materials in ways we can't see).
    pub materials_prune_skipped: bool,
}

impl RepairReport {
    /// True when nothing was changed.
    pub fn is_empty(&self) -> bool {
        self.renamed_panes.is_empty()
            && self.fixed_texture_refs == 0
            && self.removed_materials.is_empty()
            && self.removed_textures.is_empty()
    }
}

impl BFLYT {
    /// True if any `prt1` pane carries (opaque) embedded property data.
    pub fn has_parts_data(&self) -> bool {
        fn walk(p: &BasePane) -> bool {
            if p.parts
                .as_ref()
                .is_some_and(|pp| !pp.raw_property_data.is_empty())
            {
                return true;
            }
            p.children.iter().any(walk)
        }
        self.root_pane.as_ref().is_some_and(walk)
    }

    /// Rename panes whose names duplicate an earlier pane (depth-first, file
    /// order). The first occurrence keeps the name; later ones get a unique
    /// `name_2` / `name_3` / … (truncated to fit the 24-byte slot). Group
    /// references (which point at the surviving first occurrence) are left
    /// intact. Returns `(old, new)` pairs.
    pub fn dedupe_pane_names(&mut self) -> Vec<(String, String)> {
        let mut used: HashSet<String> = HashSet::new();
        let mut renames: Vec<(String, String)> = Vec::new();
        if let Some(root) = self.root_pane.as_mut() {
            dedupe_walk(root, &mut used, &mut renames);
        }
        renames
    }

    /// Clamp any dangling material→texture reference (`texture_maps[].index`
    /// that is negative or `>= textures.len()`) into the valid range, so the
    /// runtime can't index past `txl1`. When the layout has no textures at
    /// all, such maps are dropped instead (and the material's flags are
    /// recomputed). Clamping leaves sub-section counts unchanged, so it's safe
    /// even for `flags_untrusted` materials. Returns how many refs were fixed.
    pub fn fix_dangling_texture_refs(&mut self) -> usize {
        let n = self.textures.len();
        let mut fixed = 0usize;
        for m in &mut self.materials {
            if n == 0 {
                if !m.texture_maps.is_empty() {
                    fixed += m.texture_maps.len();
                    m.texture_maps.clear();
                    m.clear_untrusted_flag();
                }
                continue;
            }
            let max = (n - 1) as i16;
            for t in &mut m.texture_maps {
                let clamped = t.index.clamp(0, max);
                if clamped != t.index {
                    t.index = clamped;
                    fixed += 1;
                }
            }
        }
        fixed
    }

    /// Remove materials not referenced by any pane (`pic1`/`txt1`/`wnd1`
    /// content + frames) and remap the surviving indices. Returns the removed
    /// material names.
    ///
    /// Note: `prt1` material overrides are opaque to us; see the module docs.
    /// Use [`has_parts_data`](Self::has_parts_data) to gate this when parts are
    /// present.
    pub fn prune_unused_materials(&mut self) -> Vec<String> {
        let n = self.materials.len();
        if n == 0 {
            return Vec::new();
        }
        let mut used = vec![false; n];
        if let Some(root) = self.root_pane.as_ref() {
            visit_material_indices(root, &mut |idx| {
                if (idx as usize) < n {
                    used[idx as usize] = true;
                }
            });
        }
        let removed: Vec<String> = (0..n)
            .filter(|&i| !used[i])
            .map(|i| self.materials[i].name.clone())
            .collect();
        if removed.is_empty() {
            return removed;
        }

        let mut new_index = vec![0u16; n];
        let mut next = 0u16;
        for (i, slot) in new_index.iter_mut().enumerate() {
            if used[i] {
                *slot = next;
                next += 1;
            }
        }
        let mut kept = Vec::with_capacity(next as usize);
        for (i, m) in self.materials.iter().enumerate() {
            if used[i] {
                kept.push(m.clone());
            }
        }
        self.materials = kept;

        if let Some(root) = self.root_pane.as_mut() {
            visit_material_indices_mut(root, &mut |idx| {
                if (*idx as usize) < n {
                    *idx = new_index[*idx as usize];
                }
            });
        }
        removed
    }

    /// Remove textures not referenced by any material's `texture_maps` and
    /// remap the surviving indices. Textures are referenced *only* by
    /// materials, so this is always safe. Returns the removed texture names.
    pub fn prune_unused_textures(&mut self) -> Vec<String> {
        let n = self.textures.len();
        if n == 0 {
            return Vec::new();
        }
        let mut used = vec![false; n];
        for m in &self.materials {
            for t in &m.texture_maps {
                if t.index >= 0 && (t.index as usize) < n {
                    used[t.index as usize] = true;
                }
            }
        }
        let removed: Vec<String> = (0..n)
            .filter(|&i| !used[i])
            .map(|i| self.textures[i].clone())
            .collect();
        if removed.is_empty() {
            return removed;
        }

        let mut new_index = vec![0i16; n];
        let mut next = 0i16;
        for (i, slot) in new_index.iter_mut().enumerate() {
            if used[i] {
                *slot = next;
                next += 1;
            }
        }
        self.textures = (0..n)
            .filter(|&i| used[i])
            .map(|i| self.textures[i].clone())
            .collect();
        for m in &mut self.materials {
            for t in &mut m.texture_maps {
                if t.index >= 0 && (t.index as usize) < n {
                    t.index = new_index[t.index as usize];
                }
            }
        }
        removed
    }

    /// Run the full repair pass: dedupe duplicate pane names, clamp dangling
    /// texture refs, optionally prune unused materials, then prune unused
    /// textures (after material pruning, which can orphan textures). Material
    /// pruning is skipped (and flagged in the report) when the layout has
    /// `prt1` panes with opaque property data, unless `prune_materials` is set
    /// *and* there is no such data.
    pub fn repair(&mut self, prune_materials: bool) -> RepairReport {
        let renamed_panes = self.dedupe_pane_names();
        let fixed_texture_refs = self.fix_dangling_texture_refs();

        let mut materials_prune_skipped = false;
        let removed_materials = if prune_materials {
            if self.has_parts_data() {
                materials_prune_skipped = true;
                Vec::new()
            } else {
                self.prune_unused_materials()
            }
        } else {
            Vec::new()
        };

        let removed_textures = self.prune_unused_textures();

        RepairReport {
            renamed_panes,
            fixed_texture_refs,
            removed_materials,
            removed_textures,
            materials_prune_skipped,
        }
    }
}

/// Depth-first dedupe: rename any pane whose name is already taken.
fn dedupe_walk(
    pane: &mut BasePane,
    used: &mut HashSet<String>,
    renames: &mut Vec<(String, String)>,
) {
    if !pane.name.is_empty() {
        if used.contains(&pane.name) {
            let new = unique_name(&pane.name, used);
            renames.push((pane.name.clone(), new.clone()));
            pane.name = new.clone();
            used.insert(new);
        } else {
            used.insert(pane.name.clone());
        }
    }
    for c in &mut pane.children {
        dedupe_walk(c, used, renames);
    }
}

/// Produce a unique `base_N` name (N ≥ 2) that fits the 24-byte slot and isn't
/// already in `used`.
fn unique_name(base: &str, used: &HashSet<String>) -> String {
    for i in 2u32.. {
        let suffix = format!("_{i}");
        let max_base = PANE_NAME_LEN.saturating_sub(suffix.len());
        let trimmed = truncate_bytes(base, max_base);
        let candidate = format!("{trimmed}{suffix}");
        if !used.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!("ran out of u32 suffixes")
}

/// Truncate `s` to at most `max` bytes on a char boundary.
fn truncate_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Visit every pane's material-index reference (read-only).
fn visit_material_indices(pane: &BasePane, f: &mut impl FnMut(u16)) {
    if let Some(p) = &pane.picture {
        f(p.material_index);
    }
    if let Some(t) = &pane.text {
        f(t.material_index);
    }
    if let Some(w) = &pane.window {
        f(w.content.material_index);
        for fr in &w.frames {
            f(fr.material_index);
        }
    }
    for c in &pane.children {
        visit_material_indices(c, f);
    }
}

/// Visit every pane's material-index reference (mutable, for remapping).
fn visit_material_indices_mut(pane: &mut BasePane, f: &mut impl FnMut(&mut u16)) {
    if let Some(p) = pane.picture.as_mut() {
        f(&mut p.material_index);
    }
    if let Some(t) = pane.text.as_mut() {
        f(&mut t.material_index);
    }
    if let Some(w) = pane.window.as_mut() {
        f(&mut w.content.material_index);
        for fr in &mut w.frames {
            f(&mut fr.material_index);
        }
    }
    for c in &mut pane.children {
        visit_material_indices_mut(c, f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bflyt::{
        read_bflyt, write_bflyt, Group, LayoutInfo, Material, PaneKind, PartsPane, PicturePane,
        TextureRef,
    };

    fn leaf(name: &str) -> BasePane {
        let mut p = BasePane::opaque(*b"pan1", Vec::new());
        p.kind = PaneKind::Pane;
        p.opaque = None;
        p.name = name.to_string();
        p
    }

    /// A pic1 pane bound to material index `mat`.
    fn pic(name: &str, mat: u16) -> BasePane {
        let mut p = leaf(name);
        p.kind = PaneKind::Picture;
        p.picture = Some(PicturePane {
            material_index: mat,
            ..Default::default()
        });
        p
    }

    fn mat(name: &str, tex: i16) -> Material {
        Material {
            name: name.to_string(),
            texture_maps: vec![TextureRef {
                index: tex,
                wrap_mode_u: 0,
                wrap_mode_v: 0,
            }],
            ..Default::default()
        }
    }

    fn sample() -> BFLYT {
        BFLYT {
            version: 0x0800_0000,
            layout: LayoutInfo {
                draw_centered: false,
                width: 1.0,
                height: 1.0,
                max_parts_width: 0.0,
                max_parts_height: 0.0,
                name: "t".into(),
            },
            // tex0 used by mat0, tex1 unused, tex2 used by mat1.
            textures: vec!["tex0".into(), "tex1".into(), "tex2".into()],
            fonts: Vec::new(),
            // mat0 used by P0, mat1 unused, mat2 used by P1.
            materials: vec![mat("mat0", 0), mat("mat1", 1), mat("mat2", 2)],
            root_pane: Some({
                let mut root = leaf("RootPane");
                root.children = vec![pic("P0", 0), pic("P1", 2)];
                root
            }),
            root_group: None,
            user_data: None,
            control_data: None,
            opaque_sections: Vec::new(),
            trailing_sections: Vec::new(),
        }
    }

    #[test]
    fn prune_unused_materials_remaps_pane_refs() {
        let mut b = sample();
        let removed = b.prune_unused_materials();
        assert_eq!(removed, vec!["mat1".to_string()]);
        assert_eq!(b.materials.len(), 2);
        // P0 still -> mat0 (index 0); P1 -> mat2 (now index 1).
        assert_eq!(
            b.find_pane("P0")
                .unwrap()
                .picture
                .as_ref()
                .unwrap()
                .material_index,
            0
        );
        assert_eq!(
            b.find_pane("P1")
                .unwrap()
                .picture
                .as_ref()
                .unwrap()
                .material_index,
            1
        );
    }

    #[test]
    fn prune_unused_textures_remaps_material_refs() {
        let mut b = sample();
        // Standalone texture pruning considers *all* materials' refs, so tex1
        // stays alive while mat1 references it. Drop mat1 first to orphan tex1
        // (repair() achieves this by pruning materials before textures).
        b.materials.remove(1);
        let removed = b.prune_unused_textures();
        assert_eq!(removed, vec!["tex1".to_string()]);
        assert_eq!(b.textures.len(), 2);
        // mat0 -> tex0 stays index 0; mat2 (now materials[1]) -> tex2 becomes 1.
        assert_eq!(b.materials[0].texture_maps[0].index, 0);
        assert_eq!(b.materials[1].texture_maps[0].index, 1);
    }

    #[test]
    fn prune_unused_textures_keeps_textures_referenced_by_unused_materials() {
        // Sanity-check the behavior the previous test relies on: a texture
        // referenced only by an (otherwise-unused) material is NOT pruned by
        // texture pruning alone.
        let mut b = sample();
        let removed = b.prune_unused_textures();
        assert!(
            removed.is_empty(),
            "all textures are referenced by some material"
        );
        assert_eq!(b.textures.len(), 3);
    }

    #[test]
    fn dedupe_renames_duplicate_pane_names() {
        let mut b = sample();
        // Introduce a duplicate "P0".
        b.find_pane_mut("P1").unwrap().name = "P0".into();
        let renames = b.dedupe_pane_names();
        assert_eq!(renames.len(), 1);
        assert_eq!(renames[0].0, "P0");
        assert_eq!(renames[0].1, "P0_2");
        assert!(b.pane_exists("P0") && b.pane_exists("P0_2"));
    }

    #[test]
    fn fix_dangling_clamps_into_range() {
        let mut b = sample();
        b.materials[0].texture_maps[0].index = 99; // out of range
        b.materials[1].texture_maps[0].index = -1; // negative
        let fixed = b.fix_dangling_texture_refs();
        assert_eq!(fixed, 2);
        assert_eq!(b.materials[0].texture_maps[0].index, 2); // clamped to len-1
        assert_eq!(b.materials[1].texture_maps[0].index, 0); // clamped to 0
    }

    #[test]
    fn repair_runs_clean_and_round_trips() {
        let mut b = sample();
        b.find_pane_mut("P1").unwrap().name = "P0".into(); // dup
        b.materials[0].texture_maps[0].index = 99; // dangling
        let report = b.repair(true);
        assert_eq!(report.renamed_panes.len(), 1);
        assert_eq!(report.fixed_texture_refs, 1);
        assert_eq!(report.removed_materials, vec!["mat1".to_string()]);
        // tex1 unused -> pruned (after material prune the set may change, but
        // tex1 was never referenced by a surviving material).
        assert!(report.removed_textures.contains(&"tex1".to_string()));
        // The repaired layout serializes + re-parses.
        let bytes = write_bflyt(&b).unwrap();
        let back = read_bflyt(&bytes).unwrap();
        assert!(back.pane_exists("P0") && back.pane_exists("P0_2"));
    }

    #[test]
    fn repair_skips_material_prune_with_parts_data() {
        let mut b = sample();
        // Attach a prt1 pane with opaque property data.
        let mut prt = leaf("Part");
        prt.kind = PaneKind::Parts;
        prt.parts = Some(PartsPane {
            raw_property_data: vec![1, 2, 3, 4],
            ..Default::default()
        });
        b.root_pane.as_mut().unwrap().children.push(prt);
        let report = b.repair(true);
        assert!(report.materials_prune_skipped);
        assert!(report.removed_materials.is_empty());
        assert_eq!(b.materials.len(), 3); // nothing pruned
    }

    fn group(panes: &[&str]) -> Group {
        Group {
            name: "G".into(),
            panes: panes.iter().map(|s| s.to_string()).collect(),
            children: Vec::new(),
        }
    }

    #[test]
    fn remove_pane_then_prune_is_consistent() {
        let mut b = sample();
        b.root_group = Some(group(&["P0", "P1"]));
        // Remove P1 (which used mat2/tex2); then mat2 + tex2 become prunable.
        b.remove_pane("P1").unwrap();
        let report = b.repair(true);
        assert!(report.removed_materials.contains(&"mat2".to_string()));
        assert!(report.removed_textures.contains(&"tex2".to_string()));
        // Group scrubbed of P1 by remove_pane.
        assert_eq!(b.root_group.as_ref().unwrap().panes, vec!["P0".to_string()]);
    }
}
