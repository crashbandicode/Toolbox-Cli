//! High-level layout orchestration: apply or validate an SGPO skin
//! manifest against an unpacked layout directory (the `blyt/` + `timg/`
//! tree extracted from a `layout.arc`).
//!
//! [`apply_manifest`] is the end-to-end "materialize a skin" entry point;
//! [`validate_manifest`] is a read-only checker. Both are thin compositions
//! of the [`crate::bflyt`], [`crate::bntx`], and [`crate::texpipe`] building
//! blocks, exposed so consumers (e.g. SGPO) can drive the whole pipeline
//! from Rust without shelling out to the CLI.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::bflyt::{read_bflyt, write_bflyt, ClonePaneSpec, BFLYT};
use crate::bntx::{pipeline, pipeline::ImportOptions, read_bntx, write_bntx, BntxFile};
use crate::error::{Error, Result};
use crate::manifest::{SkinElement, SkinManifest};
use crate::sarc::{read_arc, write_arc};
use crate::texpipe::Bc7Quality;

/// Options for [`apply_manifest`]. [`Default`] matches the CLI defaults
/// (Smash `info_melee` layout, `set_rep_stock_01` templates, `fast` BC7).
#[derive(Debug, Clone)]
pub struct ApplyOptions {
    /// BFLYT path relative to the layout dir.
    pub bflyt_rel: String,
    /// BNTX path relative to the layout dir.
    pub bntx_rel: String,
    /// Existing pane to clone for each element.
    pub pane_template: String,
    /// Existing material to clone for each element.
    pub material_template: String,
    /// BC7 encoder quality.
    pub quality: Bc7Quality,
    /// Encode imported textures as sRGB.
    pub srgb: bool,
    /// Override BRTD texture-data alignment.
    pub align: Option<u32>,
    /// Skip elements already fully present (idempotent re-runs).
    pub skip_existing: bool,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        Self {
            bflyt_rel: "blyt/info_melee.bflyt".into(),
            bntx_rel: "timg/__Combined.bntx".into(),
            pane_template: "set_rep_stock_01".into(),
            material_template: "set_rep_stock_01".into(),
            quality: Bc7Quality::Fast,
            srgb: false,
            align: None,
            skip_existing: false,
        }
    }
}

/// Outcome of [`apply_manifest`].
#[derive(Debug, Clone)]
pub struct ApplyReport {
    pub applied: usize,
    pub skipped: usize,
    pub bflyt_bytes: usize,
    pub bntx_bytes: usize,
}

fn join_rel(layout_dir: &Path, rel: &str) -> PathBuf {
    layout_dir.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR))
}

/// Apply a skin manifest to an unpacked layout: for each element, encode
/// its PNG into the BNTX as `tex_<pane_name>`, add the matching
/// txl1/material/pane to the BFLYT, then write both files back to disk.
pub fn apply_manifest(
    layout_dir: &Path,
    manifest: &SkinManifest,
    skin_dir: &Path,
    opts: &ApplyOptions,
) -> Result<ApplyReport> {
    let bflyt_path = join_rel(layout_dir, &opts.bflyt_rel);
    let bntx_path = join_rel(layout_dir, &opts.bntx_rel);
    if !bflyt_path.exists() {
        return Err(Error::Manifest(format!(
            "BFLYT not found: {}",
            bflyt_path.display()
        )));
    }
    if !bntx_path.exists() {
        return Err(Error::Manifest(format!(
            "BNTX not found: {}",
            bntx_path.display()
        )));
    }

    let mut bflyt = read_bflyt(&std::fs::read(&bflyt_path)?)?;
    let mut bntx = read_bntx(&std::fs::read(&bntx_path)?)?;

    let (applied, skipped) =
        apply_manifest_in_memory(&mut bflyt, &mut bntx, manifest, skin_dir, opts)?;

    let bflyt_bytes = write_bflyt(&bflyt)?;
    let bntx_bytes = write_bntx(&bntx)?;
    std::fs::write(&bflyt_path, &bflyt_bytes)?;
    std::fs::write(&bntx_path, &bntx_bytes)?;

    Ok(ApplyReport {
        applied,
        skipped,
        bflyt_bytes: bflyt_bytes.len(),
        bntx_bytes: bntx_bytes.len(),
    })
}

/// Core of [`apply_manifest`] operating on already-parsed `bflyt` + `bntx`
/// in memory (no file I/O beyond reading each element's source PNG from
/// `skin_dir`). Returns `(applied, skipped)`. Used by both the on-disk
/// [`apply_manifest`] and the archive-level [`apply_manifest_to_arc`].
pub fn apply_manifest_in_memory(
    bflyt: &mut BFLYT,
    bntx: &mut BntxFile,
    manifest: &SkinManifest,
    skin_dir: &Path,
    opts: &ApplyOptions,
) -> Result<(usize, usize)> {
    let mut applied = 0usize;
    let mut skipped = 0usize;

    for el in &manifest.elements {
        let texture_name = el.texture_name();
        let in_bntx = bntx.texture_index_by_name(&texture_name).is_some();
        let in_bflyt = bflyt.pane_exists(&el.pane_name);

        if opts.skip_existing && in_bntx && in_bflyt {
            skipped += 1;
            continue;
        }
        if !opts.skip_existing && (in_bntx || in_bflyt) {
            return Err(Error::Manifest(format!(
                "element '{}' already partially present (texture_in_bntx={in_bntx} \
                 pane_in_bflyt={in_bflyt}); set skip_existing for idempotent re-runs",
                el.pane_name
            )));
        }

        // 1. Encode the PNG and append to BNTX.
        if !in_bntx {
            let image_path = skin_dir.join(&el.image_filename);
            if !image_path.exists() {
                return Err(Error::Manifest(format!(
                    "image '{}' not found (looked at {})",
                    el.image_filename,
                    image_path.display()
                )));
            }
            let import_opts = ImportOptions {
                quality: opts.quality,
                srgb: opts.srgb,
                align: opts.align,
                mip_count: 1,
            };
            pipeline::import_png_file(bntx, &texture_name, &image_path, &import_opts)?;
        }

        // 2. txl1 texture reference (idempotent).
        bflyt.add_texture_ref(&texture_name);

        // 3. Material cloned from the template, bound to the new texture.
        if !bflyt.materials.iter().any(|m| m.name == el.material_name) {
            bflyt.add_material_from_template(
                &opts.material_template,
                &el.material_name,
                Some(&texture_name),
            )?;
        }

        // 4. Pane cloned from the template under the manifest root pane.
        if !bflyt.pane_exists(&el.pane_name) {
            bflyt.clone_pane(&ClonePaneSpec {
                template: opts.pane_template.clone(),
                new_name: el.pane_name.clone(),
                parent: Some(manifest.root_pane_name.clone()),
                translate_x: Some(el.base_x),
                translate_y: Some(el.base_y),
                translate_z: Some(0.0),
                width: Some(el.width),
                height: Some(el.height),
                alpha: Some(el.released_alpha),
                visible: None,
                bind_material: Some(el.material_name.clone()),
            })?;
        }

        applied += 1;
    }

    Ok((applied, skipped))
}

/// Options for [`validate_manifest`].
#[derive(Debug, Clone)]
pub struct ValidateOptions {
    pub bflyt_rel: String,
    pub bntx_rel: String,
    /// Also fail when BNTX texture dimensions disagree with the manifest.
    pub strict_dimensions: bool,
}

impl Default for ValidateOptions {
    fn default() -> Self {
        Self {
            bflyt_rel: "blyt/info_melee.bflyt".into(),
            bntx_rel: "timg/__Combined.bntx".into(),
            strict_dimensions: false,
        }
    }
}

/// Per-element validation result.
#[derive(Debug, Clone, Serialize)]
pub struct ElementValidation {
    pub control_id: String,
    pub pane_name: String,
    pub material_name: String,
    pub texture_name: String,
    pub image_filename: String,
    pub ok: bool,
    pub failures: Vec<String>,
}

/// Outcome of [`validate_manifest`].
#[derive(Debug, Clone, Serialize)]
pub struct ValidateReport {
    pub element_count: usize,
    pub passed: usize,
    pub failed: usize,
    pub results: Vec<ElementValidation>,
}

impl ValidateReport {
    /// True if every element passed.
    pub fn all_passed(&self) -> bool {
        self.failed == 0
    }
}

/// Verify that an unpacked layout matches a skin manifest (read-only).
pub fn validate_manifest(
    layout_dir: &Path,
    manifest: &SkinManifest,
    opts: &ValidateOptions,
) -> Result<ValidateReport> {
    let bflyt_path = join_rel(layout_dir, &opts.bflyt_rel);
    let bntx_path = join_rel(layout_dir, &opts.bntx_rel);
    if !bflyt_path.exists() {
        return Err(Error::Manifest(format!(
            "BFLYT not found: {}",
            bflyt_path.display()
        )));
    }
    if !bntx_path.exists() {
        return Err(Error::Manifest(format!(
            "BNTX not found: {}",
            bntx_path.display()
        )));
    }

    let bflyt = read_bflyt(&std::fs::read(&bflyt_path)?)?;
    let bntx = read_bntx(&std::fs::read(&bntx_path)?)?;

    Ok(validate_manifest_in_memory(
        &bflyt,
        &bntx,
        manifest,
        opts.strict_dimensions,
    ))
}

/// Core of [`validate_manifest`] operating on already-parsed `bflyt` +
/// `bntx`. Used by both the on-disk [`validate_manifest`] and
/// [`apply_manifest_to_arc`].
pub fn validate_manifest_in_memory(
    bflyt: &BFLYT,
    bntx: &BntxFile,
    manifest: &SkinManifest,
    strict_dimensions: bool,
) -> ValidateReport {
    let results: Vec<ElementValidation> = manifest
        .elements
        .iter()
        .map(|el| validate_element(el, manifest, bflyt, bntx, strict_dimensions))
        .collect();
    let passed = results.iter().filter(|r| r.ok).count();
    let failed = results.len() - passed;

    ValidateReport {
        element_count: manifest.elements.len(),
        passed,
        failed,
        results,
    }
}

fn validate_element(
    el: &SkinElement,
    manifest: &SkinManifest,
    bflyt: &BFLYT,
    bntx: &BntxFile,
    strict_dimensions: bool,
) -> ElementValidation {
    let mut failures = Vec::new();
    let texture_name = el.texture_name();

    let finish = |failures: Vec<String>| ElementValidation {
        control_id: el.control_id.clone(),
        pane_name: el.pane_name.clone(),
        material_name: el.material_name.clone(),
        texture_name: texture_name.clone(),
        image_filename: el.image_filename.clone(),
        ok: failures.is_empty(),
        failures,
    };

    // Pane lookup + parent check.
    let pane = match bflyt.find_pane(&el.pane_name) {
        Some(p) => p,
        None => {
            failures.push(format!("pane '{}' not found in BFLYT", el.pane_name));
            return finish(failures);
        }
    };
    let parent_name = bflyt
        .parent_pane_name(&el.pane_name)
        .unwrap_or_else(|| "<none>".to_string());
    if parent_name != manifest.root_pane_name {
        failures.push(format!(
            "parent is '{parent_name}', expected '{}'",
            manifest.root_pane_name
        ));
    }

    // Pane -> material binding.
    let mat_idx = match (&pane.picture, &pane.text) {
        (Some(p), _) => p.material_index as usize,
        (_, Some(t)) => t.material_index as usize,
        _ => {
            failures.push(format!(
                "pane '{}' is not a pic1/txt1 with a material index",
                el.pane_name
            ));
            return finish(failures);
        }
    };
    let mat = bflyt.materials.get(mat_idx);
    match mat {
        Some(m) if m.name == el.material_name => {}
        Some(m) => failures.push(format!(
            "pane binds material '{}', expected '{}'",
            m.name, el.material_name
        )),
        None => failures.push(format!(
            "pane material_idx={mat_idx} is out of range (mat1 has {})",
            bflyt.materials.len()
        )),
    }

    // Material -> texture binding.
    if let Some(m) = mat {
        if m.texture_maps.is_empty() {
            failures.push(format!("material '{}' has no texture map", m.name));
        } else {
            let bound_idx = m.texture_maps[0].index as usize;
            match bflyt.textures.get(bound_idx) {
                Some(name) if *name == texture_name => {}
                Some(name) => failures.push(format!(
                    "material '{}' binds texture '{name}', expected '{texture_name}'",
                    m.name
                )),
                None => failures.push(format!(
                    "material '{}' references invalid txl1 index {bound_idx}",
                    m.name
                )),
            }
        }
    }

    // BFLYT txl1 contains the expected texture name.
    if !bflyt.textures.iter().any(|t| t == &texture_name) {
        failures.push(format!("texture '{texture_name}' not in BFLYT txl1"));
    }

    // BNTX contains the texture; optional dimension check.
    match bntx.textures.iter().find(|t| t.name(bntx) == texture_name) {
        None => failures.push(format!("texture '{texture_name}' not in BNTX")),
        Some(t)
            if strict_dimensions
                && (t.width != el.width as u32 || t.height != el.height as u32) =>
        {
            failures.push(format!(
                "BNTX texture is {}x{}, manifest expects {}x{}",
                t.width, t.height, el.width as u32, el.height as u32
            ));
        }
        _ => {}
    }

    finish(failures)
}

// ============================================================
// Archive-level: unpack -> apply -> validate -> repack
// ============================================================

/// Outcome of [`apply_manifest_to_arc`].
#[derive(Debug, Clone)]
pub struct ArcApplyReport {
    /// Elements newly applied.
    pub applied: usize,
    /// Elements skipped (already present, with `skip_existing`).
    pub skipped: usize,
    /// Validation of the modified layout (always run).
    pub validation: ValidateReport,
    /// Length in bytes of the produced archive.
    pub out_arc_len: usize,
    /// Number of entries in the produced archive (preserved from input).
    pub file_count: usize,
}

/// Apply a skin manifest to a packed `layout.arc` end-to-end, entirely in
/// memory: unpack the archive, locate the BFLYT (`opts.bflyt_rel`) and
/// BNTX (`opts.bntx_rel`) entries, apply the manifest, validate the
/// result, then re-pack **every** entry (the edited BFLYT/BNTX plus all
/// other files, including any hash-only entries) into a new archive.
///
/// Returns the new archive bytes and an [`ArcApplyReport`]. Validation is
/// always performed and surfaced in the report; the caller decides
/// whether a validation failure is fatal.
pub fn apply_manifest_to_arc(
    arc_bytes: &[u8],
    manifest: &SkinManifest,
    skin_dir: &Path,
    opts: &ApplyOptions,
    strict_dimensions: bool,
) -> Result<(Vec<u8>, ArcApplyReport)> {
    let mut arc = read_arc(arc_bytes)?;

    let bflyt_idx = arc.position(&opts.bflyt_rel).ok_or_else(|| {
        Error::Manifest(format!("BFLYT entry '{}' not found in archive", opts.bflyt_rel))
    })?;
    let bntx_idx = arc.position(&opts.bntx_rel).ok_or_else(|| {
        Error::Manifest(format!("BNTX entry '{}' not found in archive", opts.bntx_rel))
    })?;

    let mut bflyt = read_bflyt(&arc.files[bflyt_idx].data)?;
    let mut bntx = read_bntx(&arc.files[bntx_idx].data)?;

    let (applied, skipped) =
        apply_manifest_in_memory(&mut bflyt, &mut bntx, manifest, skin_dir, opts)?;
    let validation = validate_manifest_in_memory(&bflyt, &bntx, manifest, strict_dimensions);

    arc.files[bflyt_idx].data = write_bflyt(&bflyt)?;
    arc.files[bntx_idx].data = write_bntx(&bntx)?;

    let file_count = arc.files.len();
    let out = write_arc(&arc)?;
    let out_arc_len = out.len();

    Ok((
        out,
        ArcApplyReport {
            applied,
            skipped,
            validation,
            out_arc_len,
            file_count,
        },
    ))
}
