//! `bflyt-add-material`: clone an existing material as a template and add
//! it under a new name, optionally rebinding its first texture map. Used
//! by SGPO to create one material per skin element while preserving the
//! v9-specific trailing extension (which we can't synthesize from
//! scratch).

use anyhow::{anyhow, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bflyt::MAT_NAME_LEN_USIZE;
use crate::verbs::bflyt_helpers::rewrite_bflyt;

#[derive(Parser, Debug)]
pub struct Args {
    /// BFLYT file to modify.
    #[arg(short, long)]
    input: PathBuf,

    /// Output BFLYT (defaults to overwriting the input).
    #[arg(short, long)]
    out: Option<PathBuf>,

    /// Existing material to clone (e.g. an SGPO marker template).
    #[arg(long)]
    template: String,

    /// Name for the new material. Must be unique and ≤ 27 bytes.
    #[arg(long)]
    name: String,

    /// Optional texture name to bind to the new material's first texture
    /// map. The texture must already exist in BFLYT txl1 (use
    /// `bflyt-add-texture-ref` first).
    #[arg(long)]
    bind_texture: Option<String>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    if args.name.len() > MAT_NAME_LEN_USIZE - 1 {
        return Err(anyhow!(
            "new material name '{}' is {} bytes (max {})",
            args.name,
            args.name.len(),
            MAT_NAME_LEN_USIZE - 1
        ));
    }
    let template_name = args.template.clone();
    let new_name = args.name.clone();
    let bind_tex = args.bind_texture.clone();
    let n = rewrite_bflyt(&args.input, args.out.as_deref(), |bflyt| {
        if bflyt.materials.iter().any(|m| m.name == new_name) {
            return Err(anyhow!(
                "material '{}' already exists in mat1",
                new_name
            ));
        }
        let template_idx = bflyt
            .materials
            .iter()
            .position(|m| m.name == template_name)
            .ok_or_else(|| anyhow!("template material '{}' not found", template_name))?;
        let mut clone = bflyt.materials[template_idx].clone();
        clone.name = new_name.clone();

        if let Some(tex_name) = &bind_tex {
            let tex_idx = bflyt
                .textures
                .iter()
                .position(|t| t == tex_name)
                .ok_or_else(|| {
                    anyhow!(
                        "texture '{}' is not in BFLYT txl1; add it first with bflyt-add-texture-ref",
                        tex_name
                    )
                })?;
            if clone.texture_maps.is_empty() {
                return Err(anyhow!(
                    "template material '{}' has no texture map; cannot bind a texture",
                    template_name
                ));
            }
            clone.texture_maps[0].index = tex_idx as i16;
        }

        bflyt.materials.push(clone);
        Ok(())
    })?;
    println!(
        "ok: added material '{}' (cloned from '{}'){} ({} bytes)",
        args.name,
        args.template,
        if let Some(t) = &args.bind_texture {
            format!(" bound to texture '{}'", t)
        } else {
            String::new()
        },
        n,
    );
    Ok(ExitCode::SUCCESS)
}
