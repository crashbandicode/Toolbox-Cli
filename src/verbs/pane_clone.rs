//! `pane-clone`: clone a template pane (typically a marker like
//! `sgpo_pro_a_marker`) under a new name, optionally setting the new
//! pane's transform and material binding in the same operation. Used by
//! SGPO to materialize one pic1 pane per skin element.
//!
//! The clone is appended as a sibling of the template (same parent in
//! the pane tree). Children of the template are NOT copied — face
//! markers don't have children in practice, and recursive cloning would
//! make pane-name uniqueness much trickier.

use anyhow::{anyhow, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::verbs::bflyt_helpers::rewrite_bflyt;

#[derive(Parser, Debug)]
pub struct Args {
    /// BFLYT file to modify.
    #[arg(short, long)]
    input: PathBuf,

    /// Output BFLYT (defaults to overwriting the input).
    #[arg(short, long)]
    out: Option<PathBuf>,

    /// Template pane to clone.
    #[arg(long)]
    template: String,

    /// Name for the new pane. Must be unique and ≤ 24 bytes.
    #[arg(long)]
    name: String,

    /// Optional new parent name. If omitted, the clone is added as a
    /// sibling of the template.
    #[arg(long)]
    parent: Option<String>,

    #[arg(long, allow_negative_numbers = true)]
    translate_x: Option<f32>,
    #[arg(long, allow_negative_numbers = true)]
    translate_y: Option<f32>,
    #[arg(long, allow_negative_numbers = true)]
    translate_z: Option<f32>,
    #[arg(long, allow_negative_numbers = true)]
    width: Option<f32>,
    #[arg(long, allow_negative_numbers = true)]
    height: Option<f32>,
    #[arg(long)]
    alpha: Option<u8>,
    #[arg(long)]
    visible: Option<bool>,

    /// Bind the cloned pane to a material by name (only valid if the
    /// template is a pic1 or txt1).
    #[arg(long)]
    bind_material: Option<String>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let template_name = args.template.clone();
    let new_name = args.name.clone();
    let parent_name = args.parent.clone();
    let bind_material = args.bind_material.clone();
    if new_name.len() > 24 {
        return Err(anyhow!(
            "new pane name '{}' is {} bytes (max 24)",
            new_name,
            new_name.len()
        ));
    }
    let n = rewrite_bflyt(&args.input, args.out.as_deref(), |bflyt| {
        if bflyt.find_pane(&new_name).is_some() {
            return Err(anyhow!(
                "pane '{}' already exists; refusing to create a duplicate",
                new_name
            ));
        }
        let mat_idx = match &bind_material {
            Some(name) => Some(
                bflyt
                    .materials
                    .iter()
                    .position(|m| m.name == *name)
                    .ok_or_else(|| anyhow!("material '{}' not found in mat1", name))?
                    as u16,
            ),
            None => None,
        };
        let template = bflyt
            .find_pane(&template_name)
            .ok_or_else(|| anyhow!("template pane '{}' not found", template_name))?
            .clone();

        let target_parent_name = parent_name.unwrap_or_else(|| {
            bflyt
                .parent_pane_name(&template_name)
                .unwrap_or_else(|| "RootPane".to_string())
        });
        if target_parent_name == new_name {
            return Err(anyhow!("a pane cannot be its own parent"));
        }

        let mut clone = template.clone();
        clone.name = new_name.clone();
        clone.children.clear();
        if let Some(v) = args.translate_x { clone.translate.x = v; }
        if let Some(v) = args.translate_y { clone.translate.y = v; }
        if let Some(v) = args.translate_z { clone.translate.z = v; }
        if let Some(v) = args.width { clone.width = v; }
        if let Some(v) = args.height { clone.height = v; }
        if let Some(a) = args.alpha { clone.alpha = a; }
        if let Some(v) = args.visible { clone.set_visible(v); }
        if let Some(idx) = mat_idx {
            if let Some(p) = clone.picture.as_mut() {
                p.material_index = idx;
            } else if let Some(t) = clone.text.as_mut() {
                t.material_index = idx;
            } else {
                return Err(anyhow!(
                    "template pane '{}' is not a pic1/txt1; cannot bind a material",
                    template_name
                ));
            }
        }

        let parent = bflyt
            .find_pane_mut(&target_parent_name)
            .ok_or_else(|| anyhow!("parent pane '{}' not found", target_parent_name))?;
        parent.children.push(clone);
        Ok(())
    })?;
    println!(
        "ok: cloned pane '{}' -> '{}' ({} bytes)",
        args.template, args.name, n
    );
    Ok(ExitCode::SUCCESS)
}
