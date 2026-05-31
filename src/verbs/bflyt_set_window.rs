//! `bflyt-set-window`: edit a `wnd1` window pane's stretch / frame-size
//! borders. Only the flags you pass are changed. Thin wrapper over
//! [`crate::bflyt::BFLYT::set_window`].

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bflyt::WindowEdit;
use crate::verbs::bflyt_helpers::rewrite_bflyt;

#[derive(Parser, Debug)]
pub struct Args {
    /// BFLYT file to modify.
    #[arg(short, long)]
    input: PathBuf,

    /// Output BFLYT (defaults to overwriting the input).
    #[arg(short, long)]
    out: Option<PathBuf>,

    /// wnd1 pane name.
    #[arg(long)]
    pane: String,

    #[arg(long)]
    stretch_l: Option<u16>,
    #[arg(long)]
    stretch_r: Option<u16>,
    #[arg(long)]
    stretch_t: Option<u16>,
    #[arg(long)]
    stretch_b: Option<u16>,
    #[arg(long)]
    frame_size_l: Option<u16>,
    #[arg(long)]
    frame_size_r: Option<u16>,
    #[arg(long)]
    frame_size_t: Option<u16>,
    #[arg(long)]
    frame_size_b: Option<u16>,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let pane = args.pane.clone();
    let edit = WindowEdit {
        stretch_l: args.stretch_l,
        stretch_r: args.stretch_r,
        stretch_t: args.stretch_t,
        stretch_b: args.stretch_b,
        frame_size_l: args.frame_size_l,
        frame_size_r: args.frame_size_r,
        frame_size_t: args.frame_size_t,
        frame_size_b: args.frame_size_b,
    };
    let n = rewrite_bflyt(&args.input, args.out.as_deref(), |b| {
        b.set_window(&pane, &edit)?;
        Ok(())
    })?;
    println!("ok: window '{pane}' updated ({n} bytes)");
    Ok(ExitCode::SUCCESS)
}
