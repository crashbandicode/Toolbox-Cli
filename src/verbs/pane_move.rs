//! `pane-move`: reparent a pane (and its subtree) under a new parent. Thin
//! wrapper over [`crate::bflyt::BFLYT::move_pane`].

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

    /// Pane to move.
    #[arg(long)]
    pane: String,

    /// New parent pane to attach it under.
    #[arg(long)]
    new_parent: String,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let pane = args.pane.clone();
    let new_parent = args.new_parent.clone();
    let n = rewrite_bflyt(&args.input, args.out.as_deref(), |b| {
        b.move_pane(&pane, &new_parent)?;
        Ok(())
    })?;
    println!("ok: moved pane '{pane}' under '{new_parent}' ({n} bytes)");
    Ok(ExitCode::SUCCESS)
}
