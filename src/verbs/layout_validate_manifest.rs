//! `layout-validate-manifest`: read-only verifier that an unpacked layout
//! directory matches an SGPO skin manifest.
//!
//! Doesn't write anything; can run against the live game asset tree (or a
//! copy) without risk. Exits 0 if every element passes, 1 if any element
//! has a structural mismatch, 2 on invocation errors.

use anyhow::{Context, Result};
use clap::Parser;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bflyt::{read_bflyt, BasePane, PaneKind, BFLYT};
use crate::bntx::{read_bntx, BntxFile};
use crate::manifest::{SkinElement, SkinManifest};

#[derive(Parser, Debug)]
pub struct Args {
    /// Unpacked layout directory (containing blyt/ and timg/).
    #[arg(long)]
    layout_dir: PathBuf,

    /// Path to the SGPO skin_manifest.json.
    #[arg(long)]
    manifest: PathBuf,

    /// BFLYT path relative to --layout-dir.
    #[arg(long, default_value = "blyt/info_melee.bflyt")]
    bflyt: String,

    /// BNTX path relative to --layout-dir.
    #[arg(long, default_value = "timg/__Combined.bntx")]
    bntx: String,

    /// Emit a JSON report instead of human-readable text.
    #[arg(long)]
    json: bool,

    /// Fail when BNTX texture dimensions disagree with the manifest size.
    #[arg(long)]
    strict_dimensions: bool,
}

#[derive(Serialize, Debug)]
struct ValidationReport {
    manifest: String,
    layout_dir: String,
    bflyt_path: String,
    bntx_path: String,
    element_count: usize,
    passed: usize,
    failed: usize,
    results: Vec<ElementValidation>,
}

#[derive(Serialize, Debug)]
struct ElementValidation {
    control_id: String,
    pane_name: String,
    material_name: String,
    texture_name: String,
    image_filename: String,
    ok: bool,
    failures: Vec<String>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let manifest_bytes = fs::read_to_string(&args.manifest)
        .with_context(|| format!("reading {}", args.manifest.display()))?;
    let manifest: SkinManifest = serde_json::from_str(&manifest_bytes)
        .with_context(|| format!("parsing {}", args.manifest.display()))?;

    let bflyt_path = args.layout_dir.join(args.bflyt.replace('/', std::path::MAIN_SEPARATOR_STR));
    let bntx_path = args.layout_dir.join(args.bntx.replace('/', std::path::MAIN_SEPARATOR_STR));
    if !bflyt_path.exists() {
        anyhow::bail!("BFLYT not found: {}", bflyt_path.display());
    }
    if !bntx_path.exists() {
        anyhow::bail!("BNTX not found: {}", bntx_path.display());
    }

    let bflyt = read_bflyt(&fs::read(&bflyt_path)?)
        .map_err(|e| anyhow::anyhow!("parsing BFLYT: {}", e))?;
    let bntx = read_bntx(&fs::read(&bntx_path)?)
        .map_err(|e| anyhow::anyhow!("parsing BNTX: {}", e))?;

    let mut results = Vec::with_capacity(manifest.elements.len());
    for el in &manifest.elements {
        results.push(validate_element(el, &manifest, &bflyt, &bntx, args.strict_dimensions));
    }
    let passed = results.iter().filter(|r| r.ok).count();
    let failed = results.len() - passed;

    let report = ValidationReport {
        manifest: args.manifest.display().to_string(),
        layout_dir: args.layout_dir.display().to_string(),
        bflyt_path: bflyt_path.display().to_string(),
        bntx_path: bntx_path.display().to_string(),
        element_count: manifest.elements.len(),
        passed,
        failed,
        results,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("manifest: {}", report.manifest);
        println!("layout:   {}", report.layout_dir);
        println!(
            "elements: {}  passed: {}  failed: {}",
            report.element_count, report.passed, report.failed
        );
        println!();
        for r in &report.results {
            println!("[{}] {}", if r.ok { "OK  " } else { "FAIL" }, r.pane_name);
            for f in &r.failures {
                println!("        - {}", f);
            }
        }
    }
    Ok(if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn validate_element(
    el: &SkinElement,
    manifest: &SkinManifest,
    bflyt: &BFLYT,
    bntx: &BntxFile,
    strict_dimensions: bool,
) -> ElementValidation {
    let mut failures = Vec::new();

    // --- Pane lookup + parent check ---
    let pane = find_pane(bflyt.root_pane.as_ref(), &el.pane_name);
    let pane = match pane {
        Some(p) => p,
        None => {
            failures.push(format!("pane '{}' not found in BFLYT", el.pane_name));
            return ElementValidation {
                control_id: el.control_id.clone(),
                pane_name: el.pane_name.clone(),
                material_name: el.material_name.clone(),
                texture_name: el.texture_name(),
                image_filename: el.image_filename.clone(),
                ok: false,
                failures,
            };
        }
    };
    let parent_name = find_parent_name(bflyt.root_pane.as_ref(), &el.pane_name)
        .unwrap_or_else(|| "<none>".to_string());
    if parent_name != manifest.root_pane_name {
        failures.push(format!(
            "parent is '{}', expected '{}'",
            parent_name, manifest.root_pane_name
        ));
    }

    // --- Pane -> material binding ---
    let pic = match (&pane.picture, &pane.text) {
        (Some(p), _) => p.material_index as usize,
        (_, Some(t)) => t.material_index as usize,
        _ => {
            failures.push(format!(
                "pane '{}' is not a pic1/txt1 with a material index",
                el.pane_name
            ));
            return ElementValidation {
                control_id: el.control_id.clone(),
                pane_name: el.pane_name.clone(),
                material_name: el.material_name.clone(),
                texture_name: el.texture_name(),
                image_filename: el.image_filename.clone(),
                ok: false,
                failures,
            };
        }
    };
    let mat = bflyt.materials.get(pic);
    match mat {
        Some(m) if m.name == el.material_name => {}
        Some(m) => failures.push(format!(
            "pane binds material '{}', expected '{}'",
            m.name, el.material_name
        )),
        None => failures.push(format!(
            "pane material_idx={} is out of range (mat1 has {})",
            pic,
            bflyt.materials.len()
        )),
    }

    // --- Material -> texture binding ---
    if let Some(m) = mat {
        if m.texture_maps.is_empty() {
            failures.push(format!("material '{}' has no texture map", m.name));
        } else {
            let bound_idx = m.texture_maps[0].index as usize;
            let bound_name = bflyt.textures.get(bound_idx);
            let expected = el.texture_name();
            match bound_name {
                Some(name) if *name == expected => {}
                Some(name) => failures.push(format!(
                    "material '{}' binds texture '{}', expected '{}'",
                    m.name, name, expected
                )),
                None => failures.push(format!(
                    "material '{}' references invalid txl1 index {}",
                    m.name, bound_idx
                )),
            }
        }
    }

    // --- BFLYT txl1 has the expected texture name ---
    let expected_tex = el.texture_name();
    if !bflyt.textures.iter().any(|t| t == &expected_tex) {
        failures.push(format!(
            "texture '{}' not in BFLYT txl1",
            expected_tex
        ));
    }

    // --- BNTX has the texture; optional dimension check ---
    match bntx
        .textures
        .iter()
        .find(|t| t.name(bntx) == expected_tex)
    {
        None => failures.push(format!("texture '{}' not in BNTX", expected_tex)),
        Some(t) if strict_dimensions
            && (t.width != el.width as u32 || t.height != el.height as u32) =>
        {
            failures.push(format!(
                "BNTX texture is {}x{}, manifest expects {}x{}",
                t.width, t.height, el.width as u32, el.height as u32
            ));
        }
        _ => {}
    }

    let ok = failures.is_empty();
    ElementValidation {
        control_id: el.control_id.clone(),
        pane_name: el.pane_name.clone(),
        material_name: el.material_name.clone(),
        texture_name: expected_tex,
        image_filename: el.image_filename.clone(),
        ok,
        failures,
    }
}

fn find_pane<'a>(root: Option<&'a BasePane>, name: &str) -> Option<&'a BasePane> {
    let root = root?;
    if root.name == name {
        return Some(root);
    }
    for c in &root.children {
        if let Some(found) = find_pane(Some(c), name) {
            return Some(found);
        }
    }
    None
}

fn find_parent_name(root: Option<&BasePane>, target: &str) -> Option<String> {
    let root = root?;
    for c in &root.children {
        if c.name == target {
            return Some(root.name.clone());
        }
        if let Some(name) = find_parent_name(Some(c), target) {
            return Some(name);
        }
    }
    None
}
