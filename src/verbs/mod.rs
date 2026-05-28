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

mod bflyt_add_material;
mod bflyt_add_texture_ref;
mod bflyt_helpers;
mod bflyt_inspect;
mod bflyt_mat1_diff;
mod bflyt_roundtrip_test;
mod bflyt_section_diff;
mod bntx_inspect;
mod layout_validate_manifest;
mod mat_rename;
mod pane_clone;
mod pane_set;
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

    /// Internal: section-by-section size diff between an original BFLYT
    /// and our rewrite. Used to localize writer bugs.
    BflytSectionDiff(bflyt_section_diff::Args),

    /// Internal: per-material size diff. Reports each material whose
    /// rewritten size differs from the original.
    BflytMat1Diff(bflyt_mat1_diff::Args),

    /// Add a texture name to BFLYT txl1 (idempotent).
    BflytAddTextureRef(bflyt_add_texture_ref::Args),

    /// Clone a template material under a new name; optionally rebind its
    /// first texture map.
    BflytAddMaterial(bflyt_add_material::Args),

    /// Rename a material in mat1 in-place.
    MatRename(mat_rename::Args),

    /// Edit a pane's transform / alpha / visibility / material binding.
    PaneSet(pane_set::Args),

    /// Clone a template pane (e.g. an SGPO marker) under a new name.
    PaneClone(pane_clone::Args),

    /// Print a structured snapshot of a BNTX. Use --json for tool consumption.
    BntxInspect(bntx_inspect::Args),

    /// Validate that an unpacked layout directory matches an SGPO skin
    /// manifest. Exits 0 on full match, 1 on any element mismatch.
    LayoutValidateManifest(layout_validate_manifest::Args),

    /// Extract a SARC archive to a directory tree.
    SarcUnpack(sarc_unpack::Args),

    /// Pack a directory tree into a SARC archive.
    SarcPack(sarc_pack::Args),
}

pub fn dispatch(verb: Verb) -> Result<ExitCode> {
    match verb {
        Verb::BflytInspect(args) => Ok(bflyt_inspect::run(args)?),
        Verb::BflytRoundtripTest(args) => Ok(bflyt_roundtrip_test::run(args)?),
        Verb::BflytSectionDiff(args) => Ok(bflyt_section_diff::run(args)?),
        Verb::BflytMat1Diff(args) => Ok(bflyt_mat1_diff::run(args)?),
        Verb::BflytAddTextureRef(args) => Ok(bflyt_add_texture_ref::run(args)?),
        Verb::BflytAddMaterial(args) => Ok(bflyt_add_material::run(args)?),
        Verb::MatRename(args) => Ok(mat_rename::run(args)?),
        Verb::PaneSet(args) => Ok(pane_set::run(args)?),
        Verb::PaneClone(args) => Ok(pane_clone::run(args)?),
        Verb::BntxInspect(args) => Ok(bntx_inspect::run(args)?),
        Verb::LayoutValidateManifest(args) => Ok(layout_validate_manifest::run(args)?),
        Verb::SarcUnpack(args) => Ok(sarc_unpack::run(args)?),
        Verb::SarcPack(args) => Ok(sarc_pack::run(args)?),
    }
}
