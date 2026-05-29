//! `pane-set`: edit a pane's transform fields (translate, scale, size,
//! alpha, visibility, material binding). Thin wrapper over
//! [`crate::bflyt::BFLYT::set_pane`].

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bflyt::PaneEdit;
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

    /// Bind the pane to a material by name (pic1/txt1 only).
    #[arg(long)]
    bind_material: Option<String>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let pane = args.pane.clone();
    let edit = PaneEdit {
        translate_x: args.translate_x,
        translate_y: args.translate_y,
        translate_z: args.translate_z,
        scale_x: args.scale_x,
        scale_y: args.scale_y,
        width: args.width,
        height: args.height,
        alpha: args.alpha,
        visible: args.visible,
        bind_material: args.bind_material.clone(),
    };
    let n = rewrite_bflyt(&args.input, args.out.as_deref(), |bflyt| {
        bflyt.set_pane(&pane, &edit)?;
        Ok(())
    })?;
    println!("ok: pane '{}' updated ({} bytes)", args.pane, n);
    Ok(ExitCode::SUCCESS)
}
