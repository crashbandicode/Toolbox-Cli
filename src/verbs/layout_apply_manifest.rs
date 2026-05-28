//! `layout-apply-manifest`: end-to-end orchestrator that takes an SGPO
//! skin manifest + a directory of PNGs + an unpacked layout directory
//! and produces a modified BFLYT + BNTX (and optionally a packed SARC).
//!
//! For each manifest element, the verb:
//!   1. Imports the element's PNG into the BNTX as a new BC7 texture
//!      named `tex_<pane_name>` (the SGPO convention).
//!   2. Adds a matching `tex_<pane_name>` entry to BFLYT.txl1.
//!   3. Clones a template material under `mat_<pane_name>` and binds it
//!      to the new texture.
//!   4. Clones the template pane (typically an SGPO marker) under
//!      `<pane_name>` with the manifest-specified transform, parents it
//!      to `manifest.root_pane_name`, and binds the new material.
//!
//! Used by SGPO's converter to materialize a 4-button skin from a
//! manifest in one call.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bflyt::{read_bflyt, write_bflyt};
use crate::bntx::{read_bntx, write_bntx, AppendTextureSpec};
use crate::manifest::SkinManifest;
use crate::texpipe::{import_png, Bc7Quality};

#[derive(Parser, Debug)]
pub struct Args {
    /// Unpacked layout directory containing `blyt/` and `timg/`.
    #[arg(long)]
    layout_dir: PathBuf,

    /// Path to the SGPO skin_manifest.json.
    #[arg(long)]
    manifest: PathBuf,

    /// Directory containing the manifest's image_filename files.
    #[arg(long)]
    skin_dir: PathBuf,

    /// BFLYT path relative to --layout-dir (default: blyt/info_melee.bflyt).
    #[arg(long, default_value = "blyt/info_melee.bflyt")]
    bflyt: String,

    /// BNTX path relative to --layout-dir (default: timg/__Combined.bntx).
    #[arg(long, default_value = "timg/__Combined.bntx")]
    bntx: String,

    /// Pane to clone for each new pane. Must already exist in BFLYT and
    /// be a pic1.
    #[arg(long, default_value = "set_rep_stock_01")]
    pane_template: String,

    /// Material to clone for each new material.
    #[arg(long, default_value = "set_rep_stock_01")]
    material_template: String,

    /// BC7 encoder quality.
    #[arg(long, default_value = "fast")]
    quality: String,

    /// Encode textures as SRGB.
    #[arg(long)]
    srgb: bool,

    /// Override the texture data alignment within BRTD (default 0x200).
    #[arg(long)]
    align: Option<u32>,

    /// Skip elements that already exist in the layout (idempotent re-runs).
    #[arg(long)]
    skip_existing: bool,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let manifest_text = fs::read_to_string(&args.manifest)
        .with_context(|| format!("reading {}", args.manifest.display()))?;
    let manifest: SkinManifest = serde_json::from_str(&manifest_text)
        .with_context(|| format!("parsing {}", args.manifest.display()))?;

    let bflyt_path = args.layout_dir.join(args.bflyt.replace('/', std::path::MAIN_SEPARATOR_STR));
    let bntx_path = args.layout_dir.join(args.bntx.replace('/', std::path::MAIN_SEPARATOR_STR));
    if !bflyt_path.exists() {
        anyhow::bail!("BFLYT not found: {}", bflyt_path.display());
    }
    if !bntx_path.exists() {
        anyhow::bail!("BNTX not found: {}", bntx_path.display());
    }

    let mut bflyt = read_bflyt(&fs::read(&bflyt_path)?)
        .map_err(|e| anyhow::anyhow!("parsing BFLYT: {}", e))?;
    let mut bntx = read_bntx(&fs::read(&bntx_path)?)
        .map_err(|e| anyhow::anyhow!("parsing BNTX: {}", e))?;

    let quality: Bc7Quality = args.quality.parse().map_err(|e| anyhow!("{e}"))?;

    let mut applied = 0usize;
    let mut skipped = 0usize;

    for el in &manifest.elements {
        let texture_name = el.texture_name();
        let already_in_bntx = bntx.texture_index_by_name(&texture_name).is_some();
        let already_in_bflyt = bflyt.pane_exists(&el.pane_name);

        if args.skip_existing && already_in_bntx && already_in_bflyt {
            println!("skip: {} already present", el.pane_name);
            skipped += 1;
            continue;
        }
        if !args.skip_existing && (already_in_bntx || already_in_bflyt) {
            return Err(anyhow!(
                "element '{}' already partially present (texture_in_bntx={} pane_in_bflyt={}); pass --skip-existing to make this idempotent",
                el.pane_name,
                already_in_bntx,
                already_in_bflyt
            ));
        }

        // 1. Encode the PNG and append to BNTX.
        let image_path = args.skin_dir.join(&el.image_filename);
        if !image_path.exists() {
            return Err(anyhow!(
                "image '{}' not found (looked at {})",
                el.image_filename,
                image_path.display()
            ));
        }
        let compressed = import_png(&image_path, quality)
            .with_context(|| format!("encoding {}", image_path.display()))?;
        let mut spec = AppendTextureSpec::bc7_2d_default(
            compressed.width,
            compressed.height,
            compressed.block_height_log2 as i32,
            compressed.swizzled_data,
            args.srgb,
        );
        if let Some(a) = args.align {
            spec.align = a;
        }
        bntx.append_texture(texture_name.clone(), spec)
            .map_err(|e| anyhow::anyhow!("BNTX append for {}: {}", el.pane_name, e))?;

        // 2. Add texture name to BFLYT.txl1 (idempotent — match by name).
        let txl_index = bflyt
            .textures
            .iter()
            .position(|t| t == &texture_name)
            .unwrap_or_else(|| {
                bflyt.textures.push(texture_name.clone());
                bflyt.textures.len() - 1
            });

        // 3. Clone the template material under mat_<pane_name>, bind to texture.
        let material_name = el.material_name.clone();
        if !bflyt.materials.iter().any(|m| m.name == material_name) {
            let template_idx = bflyt
                .materials
                .iter()
                .position(|m| m.name == args.material_template)
                .ok_or_else(|| {
                    anyhow!(
                        "material template '{}' not found in BFLYT",
                        args.material_template
                    )
                })?;
            let mut clone = bflyt.materials[template_idx].clone();
            clone.name = material_name.clone();
            if !clone.texture_maps.is_empty() {
                clone.texture_maps[0].index = txl_index as i16;
            }
            bflyt.materials.push(clone);
        }
        let material_index = bflyt
            .materials
            .iter()
            .position(|m| m.name == material_name)
            .unwrap();

        // 4. Clone the template pane under <pane_name> with manifest transform.
        if !bflyt.pane_exists(&el.pane_name) {
            let template = bflyt
                .find_pane(&args.pane_template)
                .ok_or_else(|| anyhow!("pane template '{}' not found", args.pane_template))?
                .clone();
            let mut clone = template.clone();
            clone.name = el.pane_name.clone();
            clone.children.clear();
            clone.translate.x = el.base_x;
            clone.translate.y = el.base_y;
            clone.translate.z = 0.0;
            clone.width = el.width;
            clone.height = el.height;
            clone.alpha = el.released_alpha;
            if let Some(p) = clone.picture.as_mut() {
                p.material_index = material_index as u16;
            }
            let parent = bflyt
                .find_pane_mut(&manifest.root_pane_name)
                .ok_or_else(|| {
                    anyhow!(
                        "root pane '{}' (from manifest) not found in BFLYT",
                        manifest.root_pane_name
                    )
                })?;
            parent.children.push(clone);
        }

        println!(
            "ok: {} -> texture+material+pane added (txl idx {}, mat idx {})",
            el.pane_name, txl_index, material_index,
        );
        applied += 1;
    }

    // Persist the modified files.
    let bflyt_bytes = write_bflyt(&bflyt).map_err(|e| anyhow::anyhow!("writing BFLYT: {}", e))?;
    let bntx_bytes = write_bntx(&bntx).map_err(|e| anyhow::anyhow!("writing BNTX: {}", e))?;
    fs::write(&bflyt_path, &bflyt_bytes)
        .with_context(|| format!("writing {}", bflyt_path.display()))?;
    fs::write(&bntx_path, &bntx_bytes)
        .with_context(|| format!("writing {}", bntx_path.display()))?;

    println!();
    println!(
        "applied {applied} element(s); skipped {skipped}; BFLYT now {} bytes; BNTX now {} bytes",
        bflyt_bytes.len(),
        bntx_bytes.len()
    );
    Ok(ExitCode::SUCCESS)
}
