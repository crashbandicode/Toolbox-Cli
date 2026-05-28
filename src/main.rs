//! Headless CLI for editing Nintendo Switch UI assets (BFLYT v8 layouts,
//! BNTX texture containers, SARC archives).
//!
//! Inspired by KillzXGaming/Switch-Toolbox; this is a pure-Rust
//! reimplementation that avoids GPL-3.0 dependencies. The format parsers
//! here are original work informed by public format documentation
//! (mk8.tockdom.com, switchbrew, nintendo-formats.com) and cross-checked
//! against round-trip tests on real game assets that we never redistribute.

mod bflyt;
mod bntx;
mod manifest;
mod texpipe;
mod verbs;

use clap::Parser;

/// Top-level CLI dispatcher. Each subcommand is a thin wrapper around a
/// function in `verbs::*` so the verbs are easy to unit-test independently.
#[derive(Parser, Debug)]
#[command(
    name = "toolbox-cli",
    version,
    about = "Pure-Rust CLI for BFLYT v8 / BNTX / SARC editing. Inspired by Switch-Toolbox.",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    command: verbs::Verb,
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    match verbs::dispatch(cli.command) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err}");
            if std::env::var("TOOLBOX_CLI_TRACE").as_deref() == Ok("1") {
                eprintln!("{err:?}");
            }
            std::process::ExitCode::from(1)
        }
    }
}
