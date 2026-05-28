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
mod bntx_dict_test;
mod bntx_import_png;
mod bntx_inspect;
mod bntx_remove_texture;
mod bntx_replace_png;
mod bntx_layout_dump;
mod bntx_rlt_dump;
mod bntx_roundtrip_test;
mod layout_apply_manifest;
mod layout_validate_manifest;
mod mat_rename;
mod pane_clone;
mod pane_set;
mod sarc_pack;
mod sarc_unpack;

use anyhow::{Context, Result};
use clap::Subcommand;
use std::path::Path;
use std::process::ExitCode;

/// Write `bytes` to `target`, creating parent directories as needed.
/// Shared by the mutating verbs so the "make parent dir, then write"
/// dance lives in one place.
pub(crate) fn write_output(target: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(target, bytes)
        .with_context(|| format!("writing {}", target.display()))?;
    Ok(())
}

/// Index of the first byte where `a` and `b` differ, or the length of
/// the shorter slice if one is a prefix of the other. Shared by the
/// round-trip test verbs.
pub(crate) fn first_diff(a: &[u8], b: &[u8]) -> usize {
    let n = a.len().min(b.len());
    for i in 0..n {
        if a[i] != b[i] {
            return i;
        }
    }
    n
}

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

    /// Encode a PNG to BC7 + Tegra swizzle, then append it as a new
    /// named texture in the BNTX. Writes the modified file back.
    BntxImportPng(bntx_import_png::Args),

    /// Re-encode a PNG into BC7 + Tegra swizzle and overwrite an
    /// existing texture's pixel data in place (no structural change to
    /// dict / RLT, no rename). Replacement source must match the target
    /// texture's dimensions and mip count.
    BntxReplacePng(bntx_replace_png::Args),

    /// Remove a named texture from a BNTX, shrinking the string pool,
    /// dict, BRTI array, and BRTD data block. Triggers a canonical RLT
    /// rebuild.
    BntxRemoveTexture(bntx_remove_texture::Args),

    /// Internal: read a BNTX, write it back, and report whether the
    /// round-trip is byte-identical.
    BntxRoundtripTest(bntx_roundtrip_test::Args),

    /// Internal: rebuild the BNTX `_DIC` Patricia trie for the file's
    /// existing strings and verify lookups still resolve correctly.
    BntxDictTest(bntx_dict_test::Args),

    /// Internal: dump the BNTX `_RLT` relocation table contents.
    BntxRltDump(bntx_rlt_dump::Args),

    /// Internal: dump per-texture data layout (offsets, alignment) within
    /// the BRTD block.
    BntxLayoutDump(bntx_layout_dump::Args),

    /// Apply an SGPO skin manifest to an unpacked layout: encode each
    /// element's PNG to BC7 + append to BNTX, then add the matching
    /// txl1/material/pane in BFLYT. Modifies files in place.
    LayoutApplyManifest(layout_apply_manifest::Args),

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
        Verb::BntxImportPng(args) => Ok(bntx_import_png::run(args)?),
        Verb::BntxReplacePng(args) => Ok(bntx_replace_png::run(args)?),
        Verb::BntxRemoveTexture(args) => Ok(bntx_remove_texture::run(args)?),
        Verb::BntxRoundtripTest(args) => Ok(bntx_roundtrip_test::run(args)?),
        Verb::BntxDictTest(args) => Ok(bntx_dict_test::run(args)?),
        Verb::BntxRltDump(args) => Ok(bntx_rlt_dump::run(args)?),
        Verb::BntxLayoutDump(args) => Ok(bntx_layout_dump::run(args)?),
        Verb::LayoutApplyManifest(args) => Ok(layout_apply_manifest::run(args)?),
        Verb::LayoutValidateManifest(args) => Ok(layout_validate_manifest::run(args)?),
        Verb::SarcUnpack(args) => Ok(sarc_unpack::run(args)?),
        Verb::SarcPack(args) => Ok(sarc_pack::run(args)?),
    }
}
