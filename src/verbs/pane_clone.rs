//! `pane-clone`: clone a template pane under a new name, optionally setting
//! the clone's transform and material binding. Thin wrapper over
//! [`crate::bflyt::BFLYT::clone_pane`].

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bflyt::ClonePaneSpec;
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

    /// Bind the cloned pane to a material by name (pic1/txt1 only).
    #[arg(long)]
    bind_material: Option<String>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let template = args.template.clone();
    let spec = ClonePaneSpec {
        template: args.template.clone(),
        new_name: args.name.clone(),
        parent: args.parent.clone(),
        translate_x: args.translate_x,
        translate_y: args.translate_y,
        translate_z: args.translate_z,
        width: args.width,
        height: args.height,
        alpha: args.alpha,
        visible: args.visible,
        bind_material: args.bind_material.clone(),
    };
    let n = rewrite_bflyt(&args.input, args.out.as_deref(), |bflyt| {
        bflyt.clone_pane(&spec)?;
        Ok(())
    })?;
    println!(
        "ok: cloned pane '{}' -> '{}' ({} bytes)",
        template, args.name, n
    );
    Ok(ExitCode::SUCCESS)
}
