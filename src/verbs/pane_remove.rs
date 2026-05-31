//! `pane-remove`: delete a pane and its entire subtree, scrubbing the removed
//! names from any group pane lists. Thin wrapper over
//! [`crate::bflyt::BFLYT::remove_pane`].

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

    /// Pane to remove. Its whole subtree is removed too.
    #[arg(long)]
    pane: String,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let pane = args.pane.clone();
    let mut removed = 0usize;
    let n = rewrite_bflyt(&args.input, args.out.as_deref(), |b| {
        removed = b.remove_pane(&pane)?;
        Ok(())
    })?;
    println!("ok: removed pane '{pane}' ({removed} pane(s)) ({n} bytes)");
    Ok(ExitCode::SUCCESS)
}
