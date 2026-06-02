//! High-level BFLYT mutation operations.
//!
//! These are the editing building blocks the CLI verbs and library
//! consumers (e.g. SGPO) use to assemble a skin layout: add a texture
//! reference, clone a material or pane from a template, edit a pane's
//! transform, and rename a material. They operate on the public [`BFLYT`]
//! tree and return [`BflytError`] on validation failures.

use std::collections::HashSet;

use super::sections::PANE_NAME_LEN;
use super::{BasePane, BflytError, Group, TextBoxPane, BFLYT, MAT_NAME_LEN_USIZE};

/// Parameters for [`BFLYT::clone_pane`]. `None` overrides keep the
/// template's value; children are never copied.
#[derive(Debug, Clone, Default)]
pub struct ClonePaneSpec {
    /// Existing pane to clone (must be a pic1/txt1 if `bind_material` is set).
    pub template: String,
    /// Name for the new pane. Must be unique and `<= 24` bytes.
    pub new_name: String,
    /// New parent pane name. `None` makes the clone a sibling of the template.
    pub parent: Option<String>,
    pub translate_x: Option<f32>,
    pub translate_y: Option<f32>,
    pub translate_z: Option<f32>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub alpha: Option<u8>,
    pub visible: Option<bool>,
    /// Bind the clone to a material by name (pic1/txt1 only).
    pub bind_material: Option<String>,
}

/// Field edits for [`BFLYT::set_pane`]. `None` fields are left unchanged.
#[derive(Debug, Clone, Default)]
pub struct PaneEdit {
    pub translate_x: Option<f32>,
    pub translate_y: Option<f32>,
    pub translate_z: Option<f32>,
    pub scale_x: Option<f32>,
    pub scale_y: Option<f32>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub alpha: Option<u8>,
    pub visible: Option<bool>,
    /// Bind the pane to a material by name (pic1/txt1 only).
    pub bind_material: Option<String>,
}

/// Field edits for [`BFLYT::set_window`]. `None` fields are left unchanged.
/// Covers the `wnd1` stretch and frame-size borders (the decoded scalar
/// fields); the content/frame material tables are not edited here.
#[derive(Debug, Clone, Default)]
pub struct WindowEdit {
    pub stretch_l: Option<u16>,
    pub stretch_r: Option<u16>,
    pub stretch_t: Option<u16>,
    pub stretch_b: Option<u16>,
    pub frame_size_l: Option<u16>,
    pub frame_size_r: Option<u16>,
    pub frame_size_t: Option<u16>,
    pub frame_size_b: Option<u16>,
}

/// File offset of a `txt1` pane's text string, measured from the section
/// magic byte: 8 (section header) + 0x4C (pane base) + 0x54 (txt1 header).
/// A standard single-string text box stores its string right here, at the
/// start of the captured trailing bytes.
const TXT1_STRING_OFFSET: u32 = 8 + 0x4C + 0x54;

impl BFLYT {
    /// Add a texture name to txl1 if it isn't already present; return its
    /// index. Idempotent: a name already in txl1 returns its existing index.
    pub fn add_texture_ref(&mut self, name: &str) -> usize {
        if let Some(i) = self.textures.iter().position(|t| t == name) {
            i
        } else {
            self.textures.push(name.to_string());
            self.textures.len() - 1
        }
    }

    /// Clone an existing material under `new_name`, optionally rebinding
    /// its first texture map to `bind_texture` (which must already be in
    /// txl1 — call [`add_texture_ref`](Self::add_texture_ref) first).
    /// Returns the new material's index.
    ///
    /// Only the name and (optionally) the first texture-map index change,
    /// so cloning a material whose `flags_raw` was untrusted at read time
    /// stays safe (no sub-section counts are mutated).
    pub fn add_material_from_template(
        &mut self,
        template: &str,
        new_name: &str,
        bind_texture: Option<&str>,
    ) -> Result<usize, BflytError> {
        if new_name.len() > MAT_NAME_LEN_USIZE {
            return Err(BflytError::Format(format!(
                "new material name '{new_name}' is {} bytes (max {MAT_NAME_LEN_USIZE})",
                new_name.len()
            )));
        }
        if self.materials.iter().any(|m| m.name == new_name) {
            return Err(BflytError::Format(format!(
                "material '{new_name}' already exists in mat1"
            )));
        }
        let template_idx = self
            .materials
            .iter()
            .position(|m| m.name == template)
            .ok_or_else(|| {
                BflytError::Format(format!("template material '{template}' not found"))
            })?;
        let mut clone = self.materials[template_idx].clone();
        clone.name = new_name.to_string();
        if let Some(tex_name) = bind_texture {
            let tex_idx = self
                .textures
                .iter()
                .position(|t| t == tex_name)
                .ok_or_else(|| {
                    BflytError::Format(format!(
                        "texture '{tex_name}' is not in txl1; add it first with add_texture_ref"
                    ))
                })?;
            if clone.texture_maps.is_empty() {
                return Err(BflytError::Format(format!(
                    "template material '{template}' has no texture map; cannot bind a texture"
                )));
            }
            clone.texture_maps[0].index = tex_idx as i16;
        }
        self.materials.push(clone);
        Ok(self.materials.len() - 1)
    }

    /// Rename an existing material in place. The new name must be unique
    /// and fit the 28-byte name slot.
    pub fn rename_material(&mut self, from: &str, to: &str) -> Result<(), BflytError> {
        if to.len() > MAT_NAME_LEN_USIZE {
            return Err(BflytError::Format(format!(
                "new material name '{to}' is {} bytes (max {MAT_NAME_LEN_USIZE})",
                to.len()
            )));
        }
        if self.materials.iter().any(|m| m.name == to) {
            return Err(BflytError::Format(format!(
                "material '{to}' already exists in mat1; refusing to create a duplicate"
            )));
        }
        let idx = self
            .materials
            .iter()
            .position(|m| m.name == from)
            .ok_or_else(|| BflytError::Format(format!("material '{from}' not found in mat1")))?;
        self.materials[idx].name = to.to_string();
        Ok(())
    }

    /// Clone a template pane under a new name (its children are not
    /// copied), apply the overrides in `spec`, and parent the clone under
    /// `spec.parent` (or the template's parent when `None`).
    pub fn clone_pane(&mut self, spec: &ClonePaneSpec) -> Result<(), BflytError> {
        if spec.new_name.len() > PANE_NAME_LEN {
            return Err(BflytError::Format(format!(
                "new pane name '{}' is {} bytes (max {PANE_NAME_LEN})",
                spec.new_name,
                spec.new_name.len()
            )));
        }
        if self.find_pane(&spec.new_name).is_some() {
            return Err(BflytError::Format(format!(
                "pane '{}' already exists; refusing to create a duplicate",
                spec.new_name
            )));
        }
        let mat_idx = match &spec.bind_material {
            Some(name) => Some(
                self.materials
                    .iter()
                    .position(|m| m.name == *name)
                    .ok_or_else(|| {
                        BflytError::Format(format!("material '{name}' not found in mat1"))
                    })? as u16,
            ),
            None => None,
        };
        let mut clone = self
            .find_pane(&spec.template)
            .ok_or_else(|| {
                BflytError::Format(format!("template pane '{}' not found", spec.template))
            })?
            .clone();
        let target_parent = spec.parent.clone().unwrap_or_else(|| {
            self.parent_pane_name(&spec.template)
                .unwrap_or_else(|| "RootPane".to_string())
        });
        if target_parent == spec.new_name {
            return Err(BflytError::Format("a pane cannot be its own parent".into()));
        }

        clone.name = spec.new_name.clone();
        clone.children.clear();
        if let Some(v) = spec.translate_x {
            clone.translate.x = v;
        }
        if let Some(v) = spec.translate_y {
            clone.translate.y = v;
        }
        if let Some(v) = spec.translate_z {
            clone.translate.z = v;
        }
        if let Some(v) = spec.width {
            clone.width = v;
        }
        if let Some(v) = spec.height {
            clone.height = v;
        }
        if let Some(a) = spec.alpha {
            clone.alpha = a;
        }
        if let Some(v) = spec.visible {
            clone.set_visible(v);
        }
        if let Some(idx) = mat_idx {
            if let Some(p) = clone.picture.as_mut() {
                p.material_index = idx;
            } else if let Some(t) = clone.text.as_mut() {
                t.material_index = idx;
            } else {
                return Err(BflytError::Format(format!(
                    "template pane '{}' is not a pic1/txt1; cannot bind a material",
                    spec.template
                )));
            }
        }

        let parent = self.find_pane_mut(&target_parent).ok_or_else(|| {
            BflytError::Format(format!("parent pane '{target_parent}' not found"))
        })?;
        parent.children.push(clone);
        Ok(())
    }

    /// Edit an existing pane's transform / alpha / visibility / material
    /// binding. `None` fields in `edit` are left unchanged.
    pub fn set_pane(&mut self, pane: &str, edit: &PaneEdit) -> Result<(), BflytError> {
        let mat_idx = match &edit.bind_material {
            Some(name) => Some(
                self.materials
                    .iter()
                    .position(|m| m.name == *name)
                    .ok_or_else(|| {
                        BflytError::Format(format!("material '{name}' not found in mat1"))
                    })? as u16,
            ),
            None => None,
        };
        let p = self
            .find_pane_mut(pane)
            .ok_or_else(|| BflytError::Format(format!("pane '{pane}' not found")))?;
        if let Some(v) = edit.translate_x {
            p.translate.x = v;
        }
        if let Some(v) = edit.translate_y {
            p.translate.y = v;
        }
        if let Some(v) = edit.translate_z {
            p.translate.z = v;
        }
        if let Some(v) = edit.scale_x {
            p.scale.x = v;
        }
        if let Some(v) = edit.scale_y {
            p.scale.y = v;
        }
        if let Some(v) = edit.width {
            p.width = v;
        }
        if let Some(v) = edit.height {
            p.height = v;
        }
        if let Some(a) = edit.alpha {
            p.alpha = a;
        }
        if let Some(v) = edit.visible {
            p.set_visible(v);
        }
        if let Some(idx) = mat_idx {
            if let Some(pic) = p.picture.as_mut() {
                pic.material_index = idx;
            } else if let Some(t) = p.text.as_mut() {
                t.material_index = idx;
            } else {
                return Err(BflytError::Format(format!(
                    "pane '{pane}' is not a pic1/txt1; cannot bind material"
                )));
            }
        }
        Ok(())
    }

    /// Detach the pane named `name` (and its subtree) from its parent and
    /// return it. Errors if the pane doesn't exist or is the root pane (which
    /// has no parent to detach from). Shared by [`remove_pane`](Self::remove_pane)
    /// and [`move_pane`](Self::move_pane).
    fn detach_pane(&mut self, name: &str) -> Result<BasePane, BflytError> {
        if !self.pane_exists(name) {
            return Err(BflytError::Format(format!("pane '{name}' not found")));
        }
        let parent_name = self.parent_pane_name(name).ok_or_else(|| {
            BflytError::Format(format!(
                "pane '{name}' is the root pane and cannot be removed or moved"
            ))
        })?;
        let parent = self
            .find_pane_mut(&parent_name)
            .ok_or_else(|| BflytError::Format(format!("parent pane '{parent_name}' not found")))?;
        let pos = parent
            .children
            .iter()
            .position(|c| c.name == name)
            .ok_or_else(|| {
                BflytError::Format(format!("pane '{name}' not found under its parent"))
            })?;
        Ok(parent.children.remove(pos))
    }

    /// Remove the pane named `name` and its entire subtree, scrubbing the
    /// removed pane names from every group's pane list. Returns the number of
    /// panes removed (the subtree size). Errors if the pane is missing or is
    /// the root pane.
    pub fn remove_pane(&mut self, name: &str) -> Result<usize, BflytError> {
        let removed = self.detach_pane(name)?;
        let mut names = Vec::new();
        collect_subtree_names(&removed, &mut names);
        let count = names.len();
        let set: HashSet<String> = names.into_iter().collect();
        if let Some(g) = self.root_group.as_mut() {
            remove_names_from_groups(g, &set);
        }
        Ok(count)
    }

    /// Reparent the pane named `name` under `new_parent`. Errors if either
    /// pane is missing, if `name` is the root, if `new_parent` is `name`
    /// itself, or if `new_parent` lives inside `name`'s own subtree (which
    /// would create a cycle).
    pub fn move_pane(&mut self, name: &str, new_parent: &str) -> Result<(), BflytError> {
        if name == new_parent {
            return Err(BflytError::Format("a pane cannot be its own parent".into()));
        }
        if !self.pane_exists(name) {
            return Err(BflytError::Format(format!("pane '{name}' not found")));
        }
        if !self.pane_exists(new_parent) {
            return Err(BflytError::Format(format!(
                "new parent pane '{new_parent}' not found"
            )));
        }
        // Cycle guard: the new parent must not live inside the moved subtree.
        if let Some(subtree) = self.find_pane(name) {
            if subtree.find(new_parent).is_some() {
                return Err(BflytError::Format(format!(
                    "cannot move '{name}' under '{new_parent}': '{new_parent}' is inside \
                     '{name}'s own subtree"
                )));
            }
        }
        let pane = self.detach_pane(name)?;
        let parent = self.find_pane_mut(new_parent).ok_or_else(|| {
            BflytError::Format(format!("new parent pane '{new_parent}' not found"))
        })?;
        parent.children.push(pane);
        Ok(())
    }

    /// Rename the pane `from` to `to`, updating any group pane-list
    /// references. `to` must be non-empty, unique, and fit the 24-byte name
    /// slot. Renaming a pane to its current name is a no-op.
    pub fn rename_pane(&mut self, from: &str, to: &str) -> Result<(), BflytError> {
        if to.is_empty() {
            return Err(BflytError::Format("new pane name must not be empty".into()));
        }
        if to.len() > PANE_NAME_LEN {
            return Err(BflytError::Format(format!(
                "new pane name '{to}' is {} bytes (max {PANE_NAME_LEN})",
                to.len()
            )));
        }
        if from == to {
            return Ok(());
        }
        if !self.pane_exists(from) {
            return Err(BflytError::Format(format!("pane '{from}' not found")));
        }
        if self.pane_exists(to) {
            return Err(BflytError::Format(format!(
                "pane '{to}' already exists; refusing to create a duplicate"
            )));
        }
        if let Some(p) = self.find_pane_mut(from) {
            p.name = to.to_string();
        }
        if let Some(g) = self.root_group.as_mut() {
            rename_in_groups(g, from, to);
        }
        Ok(())
    }

    /// Deep-copy the subtree rooted at `template` (children included) and
    /// attach the copy under `parent` (default: the template's own parent).
    /// The copied root is named `new_root_name` (default `template + suffix`)
    /// and `suffix` is appended to every copied *descendant* name, so the
    /// copy's names stay unique. Returns the number of panes copied. Errors if
    /// any resulting name is empty, too long, collides within the copy, or
    /// collides with an existing pane.
    pub fn copy_subtree(
        &mut self,
        template: &str,
        new_root_name: Option<&str>,
        parent: Option<&str>,
        suffix: &str,
    ) -> Result<usize, BflytError> {
        let src = self
            .find_pane(template)
            .ok_or_else(|| BflytError::Format(format!("template pane '{template}' not found")))?;
        let target_parent = parent
            .map(|s| s.to_string())
            .or_else(|| self.parent_pane_name(template))
            .unwrap_or_else(|| "RootPane".to_string());

        let mut clone = src.clone();
        clone.name = match new_root_name {
            Some(n) => n.to_string(),
            None => format!("{template}{suffix}"),
        };
        for child in &mut clone.children {
            append_suffix_recursive(child, suffix);
        }

        // Validate the resulting names: each fits the slot, is unique within
        // the copy, and doesn't collide with an existing pane. A typo can't
        // produce an invalid or ambiguous tree.
        let mut new_names = Vec::new();
        collect_subtree_names(&clone, &mut new_names);
        let mut seen: HashSet<&str> = HashSet::new();
        for n in &new_names {
            if n.is_empty() {
                return Err(BflytError::Format(
                    "a copied pane has an empty name (the subtree contains an unnamed/opaque \
                     pane); copy a named subtree instead"
                        .into(),
                ));
            }
            if n.len() > PANE_NAME_LEN {
                return Err(BflytError::Format(format!(
                    "copied pane name '{n}' is {} bytes (max {PANE_NAME_LEN}); use a shorter suffix",
                    n.len()
                )));
            }
            if !seen.insert(n.as_str()) {
                return Err(BflytError::Format(format!(
                    "copied subtree would contain duplicate name '{n}'; choose a different suffix"
                )));
            }
            if self.pane_exists(n) {
                return Err(BflytError::Format(format!(
                    "pane '{n}' already exists; choose a different name/suffix"
                )));
            }
        }

        let count = new_names.len();
        let parent_node = self.find_pane_mut(&target_parent).ok_or_else(|| {
            BflytError::Format(format!("parent pane '{target_parent}' not found"))
        })?;
        parent_node.children.push(clone);
        Ok(count)
    }

    /// Edit a `wnd1` window pane's stretch / frame-size borders. `None` fields
    /// in `edit` are left unchanged. Errors if the pane is missing or isn't a
    /// window.
    pub fn set_window(&mut self, pane: &str, edit: &WindowEdit) -> Result<(), BflytError> {
        let p = self
            .find_pane_mut(pane)
            .ok_or_else(|| BflytError::Format(format!("pane '{pane}' not found")))?;
        let w = p
            .window
            .as_mut()
            .ok_or_else(|| BflytError::Format(format!("pane '{pane}' is not a wnd1 window")))?;
        if let Some(v) = edit.stretch_l {
            w.stretch_l = v;
        }
        if let Some(v) = edit.stretch_r {
            w.stretch_r = v;
        }
        if let Some(v) = edit.stretch_t {
            w.stretch_t = v;
        }
        if let Some(v) = edit.stretch_b {
            w.stretch_b = v;
        }
        if let Some(v) = edit.frame_size_l {
            w.frame_size_l = v;
        }
        if let Some(v) = edit.frame_size_r {
            w.frame_size_r = v;
        }
        if let Some(v) = edit.frame_size_t {
            w.frame_size_t = v;
        }
        if let Some(v) = edit.frame_size_b {
            w.frame_size_b = v;
        }
        Ok(())
    }

    /// The decoded string of a `txt1` text-box pane, for the standard
    /// single-string layout. Returns `None` if the pane isn't a text box or
    /// carries a layout [`set_text`](Self::set_text) doesn't model (a text id,
    /// per-character transform, line-width table, or a non-standard string
    /// offset).
    pub fn pane_text(&self, pane: &str) -> Option<String> {
        let t = self.find_pane(pane)?.text.as_ref()?;
        if !is_simple_text_layout(t) {
            return None;
        }
        let str_len = t.text_str_bytes as usize;
        if str_len > t.trailing.len() {
            return None;
        }
        Some(decode_utf16le(&t.trailing[..str_len]))
    }

    /// Replace the string of a `txt1` text-box pane (UTF-16LE + NUL), updating
    /// `text_str_bytes` / `text_buf_bytes`. Supports only the standard
    /// single-string layout — a pane that also carries a text id, a
    /// per-character transform, a line-width table, or extra data after its
    /// string is rejected (so we never corrupt data we don't fully model).
    pub fn set_text(&mut self, pane: &str, new_text: &str) -> Result<(), BflytError> {
        let p = self
            .find_pane_mut(pane)
            .ok_or_else(|| BflytError::Format(format!("pane '{pane}' not found")))?;
        let t = p
            .text
            .as_mut()
            .ok_or_else(|| BflytError::Format(format!("pane '{pane}' is not a txt1 text box")))?;

        if t.text_id_offset != 0
            || t.per_character_transform_offset != 0
            || t.line_width_offset_offset != 0
        {
            return Err(BflytError::Format(format!(
                "pane '{pane}' carries extra text data (text id / per-character transform / \
                 line-width table); set-text is unsupported for it"
            )));
        }
        if t.text_str_offset != TXT1_STRING_OFFSET {
            return Err(BflytError::Format(format!(
                "pane '{pane}' has a non-standard text layout (text_str_offset=0x{:x}, \
                 expected 0x{TXT1_STRING_OFFSET:x}); set-text is unsupported",
                t.text_str_offset
            )));
        }
        let str_len = t.text_str_bytes as usize;
        if str_len > t.trailing.len() || t.trailing.len() - str_len >= 4 {
            // The string must occupy the trailing bytes (modulo <4 bytes of
            // alignment padding); anything else means data we don't model.
            return Err(BflytError::Format(format!(
                "pane '{pane}' has unexpected data around its text string; set-text is unsupported"
            )));
        }

        let mut bytes = Vec::with_capacity((new_text.len() + 1) * 2);
        for u in new_text.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]); // NUL terminator
        if bytes.len() > u16::MAX as usize {
            return Err(BflytError::Format(
                "text is too long (exceeds 65535 bytes when UTF-16-encoded)".into(),
            ));
        }
        t.text_str_bytes = bytes.len() as u16;
        t.text_buf_bytes = bytes.len() as u16;
        t.trailing = bytes;
        Ok(())
    }
}

/// Collect this pane's name and all descendant names (depth-first).
fn collect_subtree_names(pane: &BasePane, out: &mut Vec<String>) {
    out.push(pane.name.clone());
    for c in &pane.children {
        collect_subtree_names(c, out);
    }
}

/// Append `suffix` to this pane's name and every descendant's name.
fn append_suffix_recursive(pane: &mut BasePane, suffix: &str) {
    pane.name = format!("{}{}", pane.name, suffix);
    for c in &mut pane.children {
        append_suffix_recursive(c, suffix);
    }
}

/// Remove any pane names in `names` from this group and its descendant groups.
fn remove_names_from_groups(group: &mut Group, names: &HashSet<String>) {
    group.panes.retain(|p| !names.contains(p));
    for child in &mut group.children {
        remove_names_from_groups(child, names);
    }
}

/// Replace every `from` pane reference with `to` in this group and descendants.
fn rename_in_groups(group: &mut Group, from: &str, to: &str) {
    for p in &mut group.panes {
        if p == from {
            *p = to.to_string();
        }
    }
    for child in &mut group.children {
        rename_in_groups(child, from, to);
    }
}

/// True for the standard single-string `txt1` layout that
/// [`BFLYT::set_text`]/[`BFLYT::pane_text`] understand: the string sits at the
/// canonical offset with no text id / per-character transform / line-width
/// table.
fn is_simple_text_layout(t: &TextBoxPane) -> bool {
    t.text_id_offset == 0
        && t.per_character_transform_offset == 0
        && t.line_width_offset_offset == 0
        && t.text_str_offset == TXT1_STRING_OFFSET
}

/// Decode a UTF-16LE byte slice, stopping at the first NUL code unit.
fn decode_utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bflyt::{LayoutInfo, PaneKind, TextBoxPane, WindowPane};

    /// A childless `pan1` with all-default fields and the given name.
    fn leaf(name: &str) -> BasePane {
        let mut p = BasePane::opaque(*b"pan1", Vec::new());
        // `opaque()` is just a convenient all-zeroed constructor; turn it
        // back into a real named pan1 node.
        p.kind = PaneKind::Pane;
        p.opaque = None;
        p.name = name.to_string();
        p
    }

    /// A txt1 text-box pane in the standard single-string layout carrying `s`.
    fn txt(name: &str, s: &str) -> BasePane {
        let mut bytes = Vec::new();
        for u in s.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]);
        let mut p = leaf(name);
        p.kind = PaneKind::Text;
        p.text = Some(TextBoxPane {
            text_str_offset: TXT1_STRING_OFFSET,
            text_str_bytes: bytes.len() as u16,
            text_buf_bytes: bytes.len() as u16,
            trailing: bytes,
            ..Default::default()
        });
        p
    }

    /// A wnd1 window pane with all-default borders.
    fn wnd(name: &str) -> BasePane {
        let mut p = leaf(name);
        p.kind = PaneKind::Window;
        p.window = Some(WindowPane::default());
        p
    }

    fn one_pane(child: BasePane) -> BFLYT {
        let mut b = sample();
        b.root_pane.as_mut().unwrap().children.push(child);
        b
    }

    #[test]
    fn set_text_round_trips_through_pane_text() {
        let mut b = one_pane(txt("Label", "Hello"));
        assert_eq!(b.pane_text("Label").as_deref(), Some("Hello"));
        b.set_text("Label", "Goodbye!").unwrap();
        assert_eq!(b.pane_text("Label").as_deref(), Some("Goodbye!"));
        // The byte length tracked the new string (8 chars + NUL) * 2.
        let t = b.find_pane("Label").unwrap().text.as_ref().unwrap();
        assert_eq!(t.text_str_bytes as usize, ("Goodbye!".len() + 1) * 2);
        assert_eq!(t.text_buf_bytes, t.text_str_bytes);
    }

    #[test]
    fn set_text_rejects_non_text_and_complex_layout() {
        let mut b = one_pane(leaf("NotText"));
        assert!(b.set_text("NotText", "x").is_err());
        // A pane with a per-character transform offset is rejected.
        let mut b = one_pane(txt("Fancy", "hi"));
        b.find_pane_mut("Fancy")
            .unwrap()
            .text
            .as_mut()
            .unwrap()
            .per_character_transform_offset = 0x100;
        assert!(b.set_text("Fancy", "x").is_err());
        assert!(b.pane_text("Fancy").is_none());
    }

    #[test]
    fn set_window_edits_borders() {
        let mut b = one_pane(wnd("Win"));
        let edit = WindowEdit {
            stretch_l: Some(4),
            frame_size_t: Some(7),
            ..Default::default()
        };
        b.set_window("Win", &edit).unwrap();
        let w = b.find_pane("Win").unwrap().window.as_ref().unwrap();
        assert_eq!(w.stretch_l, 4);
        assert_eq!(w.frame_size_t, 7);
        assert_eq!(w.stretch_r, 0); // untouched
                                    // Non-window pane errors.
        assert!(b.set_window("RootPane", &edit).is_err());
    }

    #[test]
    fn set_text_survives_write_read() {
        let mut b = one_pane(txt("T", "before"));
        b.set_text("T", "after-edit").unwrap();
        let back = crate::bflyt::read_bflyt(&crate::bflyt::write_bflyt(&b).unwrap()).unwrap();
        assert_eq!(back.pane_text("T").as_deref(), Some("after-edit"));
    }

    fn branch(name: &str, children: Vec<BasePane>) -> BasePane {
        let mut p = leaf(name);
        p.children = children;
        p
    }

    /// RootPane > [A > [A1, A2], B]; a group references A, A1, B.
    fn sample() -> BFLYT {
        BFLYT {
            version: 0x0800_0000,
            layout: LayoutInfo {
                draw_centered: false,
                width: 100.0,
                height: 100.0,
                max_parts_width: 0.0,
                max_parts_height: 0.0,
                name: "test".into(),
            },
            textures: Vec::new(),
            fonts: Vec::new(),
            materials: Vec::new(),
            root_pane: Some(branch(
                "RootPane",
                vec![branch("A", vec![leaf("A1"), leaf("A2")]), leaf("B")],
            )),
            root_group: Some(Group {
                name: "RootGroup".into(),
                panes: vec!["A".into(), "A1".into(), "B".into()],
                children: Vec::new(),
            }),
            user_data: None,
            control_data: None,
            opaque_sections: Vec::new(),
            trailing_sections: Vec::new(),
        }
    }

    fn group_panes(b: &BFLYT) -> Vec<String> {
        b.root_group.as_ref().unwrap().panes.clone()
    }

    #[test]
    fn remove_pane_drops_subtree_and_scrubs_groups() {
        let mut b = sample();
        // Removing A drops A, A1, A2 (3 panes) and scrubs A, A1 from the group.
        let n = b.remove_pane("A").unwrap();
        assert_eq!(n, 3);
        assert!(!b.pane_exists("A"));
        assert!(!b.pane_exists("A1"));
        assert!(!b.pane_exists("A2"));
        assert!(b.pane_exists("B"));
        assert_eq!(group_panes(&b), vec!["B".to_string()]);
    }

    #[test]
    fn remove_pane_rejects_root_and_missing() {
        let mut b = sample();
        assert!(b.remove_pane("RootPane").is_err());
        assert!(b.remove_pane("nope").is_err());
    }

    #[test]
    fn move_pane_reparents() {
        let mut b = sample();
        b.move_pane("A1", "B").unwrap();
        // A1 is no longer under A, and B now has it.
        let a = b.find_pane("A").unwrap();
        assert!(a.children.iter().all(|c| c.name != "A1"));
        let bp = b.find_pane("B").unwrap();
        assert!(bp.children.iter().any(|c| c.name == "A1"));
    }

    #[test]
    fn move_pane_rejects_cycles_and_root() {
        let mut b = sample();
        // Can't move A under its own descendant A1.
        assert!(b.move_pane("A", "A1").is_err());
        // Can't move a pane under itself.
        assert!(b.move_pane("A", "A").is_err());
        // Can't move the root.
        assert!(b.move_pane("RootPane", "A").is_err());
    }

    #[test]
    fn rename_pane_updates_groups() {
        let mut b = sample();
        b.rename_pane("A1", "A1_new").unwrap();
        assert!(!b.pane_exists("A1"));
        assert!(b.pane_exists("A1_new"));
        assert_eq!(
            group_panes(&b),
            vec!["A".to_string(), "A1_new".to_string(), "B".to_string()]
        );
    }

    #[test]
    fn rename_pane_rejects_duplicate_and_too_long() {
        let mut b = sample();
        assert!(b.rename_pane("A1", "B").is_err()); // duplicate
        assert!(b.rename_pane("A1", &"x".repeat(PANE_NAME_LEN + 1)).is_err());
        assert!(b.rename_pane("A1", "A1").is_ok()); // no-op
    }

    #[test]
    fn copy_subtree_deep_copies_with_suffix() {
        let mut b = sample();
        // Copy A (+ A1, A2) under B with suffix "_c".
        let n = b.copy_subtree("A", None, Some("B"), "_c").unwrap();
        assert_eq!(n, 3);
        assert!(b.pane_exists("A_c"));
        assert!(b.pane_exists("A1_c"));
        assert!(b.pane_exists("A2_c"));
        // Originals untouched.
        assert!(b.pane_exists("A") && b.pane_exists("A1"));
        // The copy is attached under B.
        let bp = b.find_pane("B").unwrap();
        assert!(bp.children.iter().any(|c| c.name == "A_c"));
        let copied_root = b.find_pane("A_c").unwrap();
        assert_eq!(copied_root.children.len(), 2);
    }

    #[test]
    fn copy_subtree_explicit_root_name() {
        let mut b = sample();
        b.copy_subtree("A", Some("Clone"), Some("RootPane"), "_2")
            .unwrap();
        assert!(b.pane_exists("Clone"));
        assert!(b.pane_exists("A1_2"));
    }

    #[test]
    fn copy_subtree_rejects_collisions() {
        let mut b = sample();
        // Empty suffix + descendants -> descendant names collide with originals.
        assert!(b.copy_subtree("A", Some("A_root"), None, "").is_err());
        // Explicit root name that already exists.
        assert!(b.copy_subtree("A1", Some("B"), None, "_x").is_err());
    }

    /// A mutated tree must still serialize + re-parse with the new structure.
    #[test]
    fn ops_survive_write_read_round_trip() {
        let mut b = sample();
        b.rename_pane("B", "B_renamed").unwrap();
        b.copy_subtree("A", Some("A_copy"), Some("RootPane"), "_cp")
            .unwrap();
        b.remove_pane("A2").unwrap();

        let bytes = crate::bflyt::write_bflyt(&b).unwrap();
        let back = crate::bflyt::read_bflyt(&bytes).unwrap();
        assert!(back.pane_exists("B_renamed"));
        assert!(back.pane_exists("A_copy"));
        assert!(back.pane_exists("A1_cp") && back.pane_exists("A2_cp"));
        // The original A2 is gone; the copy's A2_cp survives.
        assert!(!back.pane_exists("A2"));
        // A_copy keeps both copied children; the original A now has only A1.
        assert_eq!(back.find_pane("A_copy").unwrap().children.len(), 2);
        assert_eq!(back.find_pane("A").unwrap().children.len(), 1);
    }
}
