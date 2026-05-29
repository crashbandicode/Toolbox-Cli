//! High-level BFLYT mutation operations.
//!
//! These are the editing building blocks the CLI verbs and library
//! consumers (e.g. SGPO) use to assemble a skin layout: add a texture
//! reference, clone a material or pane from a template, edit a pane's
//! transform, and rename a material. They operate on the public [`BFLYT`]
//! tree and return [`BflytError`] on validation failures.

use super::sections::PANE_NAME_LEN;
use super::{BflytError, BFLYT, MAT_NAME_LEN_USIZE};

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
}
