//! SGPO skin manifest schema.
//!
//! Mirrors the JSON produced by `tools/analyze_retrospy_skin.py` in the
//! smash-gamepad-overlay project. Only fields used by the CLI are
//! represented; informational extras (source, defaults, coordinate_space)
//! are accepted but ignored.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct SkinManifest {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub skin_name: String,
    pub root_pane_name: String,
    #[serde(default)]
    pub expected_layout_flavor: String,
    pub elements: Vec<SkinElement>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SkinElement {
    pub control_id: String,
    pub pane_name: String,
    pub image_filename: String,
    pub material_name: String,
    pub base_x: f32,
    pub base_y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default = "default_alpha")]
    pub released_alpha: u8,
    #[serde(default = "default_alpha")]
    pub pressed_alpha: u8,
    #[serde(default = "default_scale")]
    pub released_scale: f32,
    #[serde(default = "default_scale")]
    pub pressed_scale: f32,
}

impl SkinElement {
    /// Convention used by SGPO: a pane's texture is `tex_<pane_name>`.
    /// The manifest doesn't carry an explicit texture_name field; this
    /// matches `material_name = "mat_" + pane_name`.
    pub fn texture_name(&self) -> String {
        format!("tex_{}", self.pane_name)
    }
}

fn default_alpha() -> u8 {
    255
}
fn default_scale() -> f32 {
    1.0
}
