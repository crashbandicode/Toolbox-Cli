//! `pane-set`: edit a pane's transform fields (translate, scale, size,
//! alpha, visibility, material binding). Used by SGPO to position cloned
//! face buttons.

use anyhow::{anyhow, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bflyt::{BasePane, BFLYT};
use crate::verbs::bflyt_helpers::rewrite_bflyt;

#[derive(Parser, Debug)]
pub struct Args {
    /// BFLYT file to modify.
    #[arg(short, long)]
    input: PathBuf,

    /// Output BFLYT (defaults to overwriting the input).
    #[arg(short, long)]
    out: Option<PathBuf>,

    /// Pane name.
    #[arg(long)]
    pane: String,

    #[arg(long, allow_negative_numbers = true)]
    translate_x: Option<f32>,
    #[arg(long, allow_negative_numbers = true)]
    translate_y: Option<f32>,
    #[arg(long, allow_negative_numbers = true)]
    translate_z: Option<f32>,

    #[arg(long, allow_negative_numbers = true)]
    scale_x: Option<f32>,
    #[arg(long, allow_negative_numbers = true)]
    scale_y: Option<f32>,

    #[arg(long, allow_negative_numbers = true)]
    width: Option<f32>,
    #[arg(long, allow_negative_numbers = true)]
    height: Option<f32>,

    #[arg(long)]
    alpha: Option<u8>,

    /// Set visibility flag (true/false).
    #[arg(long)]
    visible: Option<bool>,

    /// Bind the pane to a material by name (only for pic1/txt1).
    #[arg(long)]
    bind_material: Option<String>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let pane_name = args.pane.clone();
    let bind_material = args.bind_material.clone();
    let n = rewrite_bflyt(&args.input, args.out.as_deref(), |bflyt| {
        let mat_idx = match &bind_material {
            Some(name) => {
                let idx = bflyt
                    .materials
                    .iter()
                    .position(|m| m.name == *name)
                    .ok_or_else(|| anyhow!("material '{}' not found in mat1", name))?;
                Some(idx as u16)
            }
            None => None,
        };

        let pane = find_pane_mut(bflyt, &pane_name)
            .ok_or_else(|| anyhow!("pane '{}' not found", pane_name))?;

        if let Some(v) = args.translate_x { pane.translate.x = v; }
        if let Some(v) = args.translate_y { pane.translate.y = v; }
        if let Some(v) = args.translate_z { pane.translate.z = v; }
        if let Some(v) = args.scale_x { pane.scale.x = v; }
        if let Some(v) = args.scale_y { pane.scale.y = v; }
        if let Some(v) = args.width { pane.width = v; }
        if let Some(v) = args.height { pane.height = v; }
        if let Some(a) = args.alpha { pane.alpha = a; }
        if let Some(v) = args.visible { pane.set_visible(v); }
        if let Some(idx) = mat_idx {
            if let Some(p) = pane.picture.as_mut() {
                p.material_index = idx;
            } else if let Some(t) = pane.text.as_mut() {
                t.material_index = idx;
            } else {
                return Err(anyhow!(
                    "pane '{}' is not a pic1/txt1; cannot bind material",
                    pane_name
                ));
            }
        }
        Ok(())
    })?;
    println!("ok: pane '{}' updated ({} bytes)", args.pane, n);
    Ok(ExitCode::SUCCESS)
}

fn find_pane_mut<'a>(b: &'a mut BFLYT, name: &str) -> Option<&'a mut BasePane> {
    fn rec<'a>(p: &'a mut BasePane, name: &str) -> Option<&'a mut BasePane> {
        if p.name == name {
            return Some(p);
        }
        for c in &mut p.children {
            if let Some(found) = rec(c, name) {
                return Some(found);
            }
        }
        None
    }
    b.root_pane.as_mut().and_then(|r| rec(r, name))
}
