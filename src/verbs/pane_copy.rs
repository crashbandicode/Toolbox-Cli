//! `pane-copy`: deep-copy a pane subtree (children included) under a new
//! parent, appending a suffix to the copied descendant names so they stay
//! unique. Thin wrapper over [`crate::bflyt::BFLYT::copy_subtree`].

use anyhow::Result;
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

    /// Pane subtree to copy.
    #[arg(long)]
    template: String,

    /// Name for the copied root pane. Defaults to `<template><suffix>`.
    #[arg(long)]
    name: Option<String>,

    /// Parent to attach the copy under. Defaults to the template's parent.
    #[arg(long)]
    parent: Option<String>,

    /// Suffix appended to every copied descendant name (and the root name
    /// unless `--name` is given) to keep names unique.
    #[arg(long, default_value = "_copy")]
    suffix: String,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let template = args.template.clone();
    let name = args.name.clone();
    let parent = args.parent.clone();
    let suffix = args.suffix.clone();
    let mut copied = 0usize;
    let n = rewrite_bflyt(&args.input, args.out.as_deref(), |b| {
        copied = b.copy_subtree(&template, name.as_deref(), parent.as_deref(), &suffix)?;
        Ok(())
    })?;
    println!("ok: copied subtree '{template}' ({copied} pane(s)) ({n} bytes)");
    Ok(ExitCode::SUCCESS)
}
