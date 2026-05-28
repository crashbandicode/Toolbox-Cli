//! CLI verb definitions and dispatch.
//!
//! Each verb is a clap-derive subcommand that maps to a function in one of
//! the per-verb modules. The dispatcher returns a `std::process::ExitCode`
//! so the binary's exit semantics are explicit:
//!
//! - 0 = success
//! - 1 = semantic failure (e.g. file not found, validation mismatch)
//! - 2 = invocation error (bad flags) — handled by clap
//! - 64 = unhandled internal case

mod bflyt_inspect;
mod bflyt_roundtrip_test;
mod bntx_inspect;
mod sarc_pack;
mod sarc_unpack;

use anyhow::Result;
use clap::Subcommand;
use std::process::ExitCode;

#[derive(Subcommand, Debug)]
pub enum Verb {
    /// Print a structured snapshot of a BFLYT (v8/v9). Use --json for tool
    /// consumption.
    BflytInspect(bflyt_inspect::Args),

    /// Internal: read a BFLYT, write it back to memory, and report whether
    /// the parse+write round-trip is byte-identical. Used to validate the
    /// parser/writer against real fixtures.
    BflytRoundtripTest(bflyt_roundtrip_test::Args),

    /// Print a structured snapshot of a BNTX. Use --json for tool consumption.
    BntxInspect(bntx_inspect::Args),

    /// Extract a SARC archive to a directory tree.
    SarcUnpack(sarc_unpack::Args),

    /// Pack a directory tree into a SARC archive.
    SarcPack(sarc_pack::Args),
}

pub fn dispatch(verb: Verb) -> Result<ExitCode> {
    match verb {
        Verb::BflytInspect(args) => Ok(bflyt_inspect::run(args)?),
        Verb::BflytRoundtripTest(args) => Ok(bflyt_roundtrip_test::run(args)?),
        Verb::BntxInspect(args) => Ok(bntx_inspect::run(args)?),
        Verb::SarcUnpack(args) => Ok(sarc_unpack::run(args)?),
        Verb::SarcPack(args) => Ok(sarc_pack::run(args)?),
    }
}
