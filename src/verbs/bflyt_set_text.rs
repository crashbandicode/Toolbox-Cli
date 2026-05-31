//! `bflyt-set-text`: replace the string of a `txt1` text-box pane. Supports the
//! standard single-string layout; panes carrying a text id, per-character
//! transform, or line-width table are rejected (to avoid corrupting data we
//! don't model). The string is written UTF-16LE with a NUL terminator.

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

    /// txt1 pane name.
    #[arg(long)]
    pane: String,

    /// New text string.
    #[arg(long)]
    text: String,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let pane = args.pane.clone();
    let text = args.text.clone();
    let n = rewrite_bflyt(&args.input, args.out.as_deref(), |b| {
        b.set_text(&pane, &text)?;
        Ok(())
    })?;
    println!("ok: set text of '{pane}' ({n} bytes)");
    Ok(ExitCode::SUCCESS)
}
