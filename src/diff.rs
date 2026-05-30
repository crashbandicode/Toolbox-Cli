//! Structured before/after diff of a layout's BFLYT + BNTX.
//!
//! Compares two parsed layouts and reports what changed at the level a
//! modder cares about: which txl1 texture references, materials, and
//! panes were added / removed / changed in the BFLYT, and which textures
//! were added / removed / changed in the BNTX. Panes and materials are
//! matched by **name** (stable across index shifts); BNTX textures by
//! name. The result serializes to JSON for tooling.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::bflyt::{BasePane, PaneKind, BFLYT};
use crate::bntx::{BntxFile, TextureFormat};

/// Top-level diff of a layout (its BFLYT and BNTX).
#[derive(Debug, Clone, Serialize)]
pub struct LayoutDiff {
    pub bflyt: BflytDiff,
    pub bntx: BntxDiff,
}

impl LayoutDiff {
    /// True when nothing changed between the two layouts.
    pub fn is_empty(&self) -> bool {
        self.bflyt.is_empty() && self.bntx.is_empty()
    }
}

/// BFLYT-side changes.
#[derive(Debug, Clone, Serialize, Default)]
pub struct BflytDiff {
    pub textures_added: Vec<String>,
    pub textures_removed: Vec<String>,
    pub materials_added: Vec<String>,
    pub materials_removed: Vec<String>,
    pub materials_changed: Vec<NamedChange>,
    pub panes_added: Vec<PaneSummary>,
    pub panes_removed: Vec<String>,
    pub panes_changed: Vec<NamedChange>,
}

impl BflytDiff {
    pub fn is_empty(&self) -> bool {
        self.textures_added.is_empty()
            && self.textures_removed.is_empty()
            && self.materials_added.is_empty()
            && self.materials_removed.is_empty()
            && self.materials_changed.is_empty()
            && self.panes_added.is_empty()
            && self.panes_removed.is_empty()
            && self.panes_changed.is_empty()
    }
}

/// BNTX-side changes.
#[derive(Debug, Clone, Serialize, Default)]
pub struct BntxDiff {
    pub textures_added: Vec<TextureSummary>,
    pub textures_removed: Vec<String>,
    pub textures_changed: Vec<NamedChange>,
}

impl BntxDiff {
    pub fn is_empty(&self) -> bool {
        self.textures_added.is_empty()
            && self.textures_removed.is_empty()
            && self.textures_changed.is_empty()
    }
}

/// A named entity (pane / material / texture) plus a list of
/// human-readable field deltas.
#[derive(Debug, Clone, Serialize)]
pub struct NamedChange {
    pub name: String,
    pub changes: Vec<String>,
}

/// Summary of an added pane.
#[derive(Debug, Clone, Serialize)]
pub struct PaneSummary {
    pub name: String,
    pub kind: String,
    pub parent: Option<String>,
}

/// Summary of an added BNTX texture.
#[derive(Debug, Clone, Serialize)]
pub struct TextureSummary {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub format: String,
}

// ============================================================
// Entry points
// ============================================================

/// Diff two full layouts (BFLYT + BNTX).
pub fn diff_layouts(
    old_bflyt: &BFLYT,
    old_bntx: &BntxFile,
    new_bflyt: &BFLYT,
    new_bntx: &BntxFile,
) -> LayoutDiff {
    LayoutDiff {
        bflyt: diff_bflyt(old_bflyt, new_bflyt),
        bntx: diff_bntx(old_bntx, new_bntx),
    }
}

fn kind_str(k: PaneKind) -> &'static str {
    match k {
        PaneKind::Pane => "pan1",
        PaneKind::Picture => "pic1",
        PaneKind::Text => "txt1",
        PaneKind::Window => "wnd1",
        PaneKind::Parts => "prt1",
        PaneKind::Bounding => "bnd1",
    }
}

/// Flattened, name-keyed view of a pane used for comparison.
struct PaneInfo {
    kind: PaneKind,
    parent: Option<String>,
    translate: (f32, f32, f32),
    rotate: (f32, f32, f32),
    scale: (f32, f32),
    width: f32,
    height: f32,
    alpha: u8,
    visible: bool,
    material: Option<String>,
}

fn pane_material_name(p: &BasePane, b: &BFLYT) -> Option<String> {
    let idx = match (&p.picture, &p.text) {
        (Some(pic), _) => pic.material_index as usize,
        (_, Some(t)) => t.material_index as usize,
        _ => return None,
    };
    b.materials.get(idx).map(|m| m.name.clone())
}

fn flatten_panes(b: &BFLYT) -> BTreeMap<String, PaneInfo> {
    fn walk(p: &BasePane, parent: Option<&str>, b: &BFLYT, out: &mut BTreeMap<String, PaneInfo>) {
        out.insert(
            p.name.clone(),
            PaneInfo {
                kind: p.kind,
                parent: parent.map(str::to_string),
                translate: (p.translate.x, p.translate.y, p.translate.z),
                rotate: (p.rotate.x, p.rotate.y, p.rotate.z),
                scale: (p.scale.x, p.scale.y),
                width: p.width,
                height: p.height,
                alpha: p.alpha,
                visible: p.visible(),
                material: pane_material_name(p, b),
            },
        );
        for c in &p.children {
            walk(c, Some(&p.name), b, out);
        }
    }
    let mut map = BTreeMap::new();
    if let Some(root) = &b.root_pane {
        walk(root, None, b, &mut map);
    }
    map
}

/// Per-material comparison view: colors + bound texture names (resolved
/// through the owning BFLYT's txl1 so renamed/re-indexed textures still
/// compare by name).
struct MatInfo {
    white: [u8; 4],
    black: [u8; 4],
    textures: Vec<String>,
}

fn material_infos(b: &BFLYT) -> BTreeMap<String, MatInfo> {
    b.materials
        .iter()
        .map(|m| {
            let textures = m
                .texture_maps
                .iter()
                .map(|tr| {
                    b.textures
                        .get(tr.index as usize)
                        .cloned()
                        .unwrap_or_default()
                })
                .collect();
            (
                m.name.clone(),
                MatInfo {
                    white: [m.white_color.r, m.white_color.g, m.white_color.b, m.white_color.a],
                    black: [m.black_color.r, m.black_color.g, m.black_color.b, m.black_color.a],
                    textures,
                },
            )
        })
        .collect()
}

/// Diff the BFLYT half of a layout.
pub fn diff_bflyt(old: &BFLYT, new: &BFLYT) -> BflytDiff {
    let mut d = BflytDiff::default();

    // txl1 texture references (set difference, preserving new-order).
    let old_tex: BTreeSet<&String> = old.textures.iter().collect();
    let new_tex: BTreeSet<&String> = new.textures.iter().collect();
    d.textures_added = new
        .textures
        .iter()
        .filter(|t| !old_tex.contains(*t))
        .cloned()
        .collect();
    d.textures_removed = old
        .textures
        .iter()
        .filter(|t| !new_tex.contains(*t))
        .cloned()
        .collect();

    // Materials.
    let old_mats = material_infos(old);
    let new_mats = material_infos(new);
    for name in new_mats.keys() {
        if !old_mats.contains_key(name) {
            d.materials_added.push(name.clone());
        }
    }
    for name in old_mats.keys() {
        if !new_mats.contains_key(name) {
            d.materials_removed.push(name.clone());
        }
    }
    for (name, nm) in &new_mats {
        if let Some(om) = old_mats.get(name) {
            let mut changes = Vec::new();
            if om.white != nm.white {
                changes.push(format!("white_color {:?} -> {:?}", om.white, nm.white));
            }
            if om.black != nm.black {
                changes.push(format!("black_color {:?} -> {:?}", om.black, nm.black));
            }
            if om.textures != nm.textures {
                changes.push(format!("textures {:?} -> {:?}", om.textures, nm.textures));
            }
            if !changes.is_empty() {
                d.materials_changed.push(NamedChange {
                    name: name.clone(),
                    changes,
                });
            }
        }
    }

    // Panes.
    let old_panes = flatten_panes(old);
    let new_panes = flatten_panes(new);
    for (name, np) in &new_panes {
        if !old_panes.contains_key(name) {
            d.panes_added.push(PaneSummary {
                name: name.clone(),
                kind: kind_str(np.kind).to_string(),
                parent: np.parent.clone(),
            });
        }
    }
    for name in old_panes.keys() {
        if !new_panes.contains_key(name) {
            d.panes_removed.push(name.clone());
        }
    }
    for (name, np) in &new_panes {
        if let Some(op) = old_panes.get(name) {
            let mut changes = Vec::new();
            if op.kind as u8 != np.kind as u8 {
                changes.push(format!("kind {} -> {}", kind_str(op.kind), kind_str(np.kind)));
            }
            if op.parent != np.parent {
                changes.push(format!("parent {:?} -> {:?}", op.parent, np.parent));
            }
            if op.translate != np.translate {
                changes.push(format!("translate {:?} -> {:?}", op.translate, np.translate));
            }
            if op.rotate != np.rotate {
                changes.push(format!("rotate {:?} -> {:?}", op.rotate, np.rotate));
            }
            if op.scale != np.scale {
                changes.push(format!("scale {:?} -> {:?}", op.scale, np.scale));
            }
            if op.width != np.width || op.height != np.height {
                changes.push(format!(
                    "size {}x{} -> {}x{}",
                    op.width, op.height, np.width, np.height
                ));
            }
            if op.alpha != np.alpha {
                changes.push(format!("alpha {} -> {}", op.alpha, np.alpha));
            }
            if op.visible != np.visible {
                changes.push(format!("visible {} -> {}", op.visible, np.visible));
            }
            if op.material != np.material {
                changes.push(format!("material {:?} -> {:?}", op.material, np.material));
            }
            if !changes.is_empty() {
                d.panes_changed.push(NamedChange {
                    name: name.clone(),
                    changes,
                });
            }
        }
    }

    d
}

/// Per-texture comparison view.
struct TexInfo {
    width: u32,
    height: u32,
    format: TextureFormat,
    mips: u16,
    array: u32,
}

fn bntx_tex_infos(b: &BntxFile) -> BTreeMap<String, (TexInfo, Vec<u8>)> {
    b.textures
        .iter()
        .map(|t| {
            (
                t.name(b).to_string(),
                (
                    TexInfo {
                        width: t.width,
                        height: t.height,
                        format: t.format,
                        mips: t.mips_count,
                        array: t.array_len,
                    },
                    t.pixel_data(&b.brtd).to_vec(),
                ),
            )
        })
        .collect()
}

/// Diff the BNTX half of a layout.
pub fn diff_bntx(old: &BntxFile, new: &BntxFile) -> BntxDiff {
    let mut d = BntxDiff::default();
    let old_t = bntx_tex_infos(old);
    let new_t = bntx_tex_infos(new);

    for (name, (info, _)) in &new_t {
        if !old_t.contains_key(name) {
            d.textures_added.push(TextureSummary {
                name: name.clone(),
                width: info.width,
                height: info.height,
                format: info.format.name().to_string(),
            });
        }
    }
    for name in old_t.keys() {
        if !new_t.contains_key(name) {
            d.textures_removed.push(name.clone());
        }
    }
    for (name, (ni, ndata)) in &new_t {
        if let Some((oi, odata)) = old_t.get(name) {
            let mut changes = Vec::new();
            if (oi.width, oi.height) != (ni.width, ni.height) {
                changes.push(format!(
                    "size {}x{} -> {}x{}",
                    oi.width, oi.height, ni.width, ni.height
                ));
            }
            if oi.format != ni.format {
                changes.push(format!("format {} -> {}", oi.format.name(), ni.format.name()));
            }
            if oi.mips != ni.mips {
                changes.push(format!("mips {} -> {}", oi.mips, ni.mips));
            }
            if oi.array != ni.array {
                changes.push(format!("array {} -> {}", oi.array, ni.array));
            }
            if odata != ndata {
                changes.push("pixel data changed".to_string());
            }
            if !changes.is_empty() {
                d.textures_changed.push(NamedChange {
                    name: name.clone(),
                    changes,
                });
            }
        }
    }

    d
}
