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

mod archive_extract;
mod bflan_inspect;
mod bflan_roundtrip_test;
mod bflyt_add_material;
mod bflyt_add_texture_ref;
mod bflyt_helpers;
mod bflyt_inspect;
mod bflyt_mat1_diff;
mod bflyt_roundtrip_test;
mod bflyt_section_diff;
mod bntx_dict_test;
mod bntx_export_all;
mod bntx_export_dds;
mod bntx_export_png;
mod bntx_import_dds;
mod bntx_import_png;
mod bntx_replace_dds;
mod bntx_inspect;
mod bntx_layout_dump;
mod bntx_remove_texture;
mod bntx_replace_png;
mod bntx_rlt_dump;
mod bntx_roundtrip_test;
mod byml_diff;
mod byml_inspect;
mod byml_roundtrip_test;
mod compress;
mod decompress;
mod layout_apply_arc;
mod layout_apply_manifest;
mod layout_audit;
mod layout_diff;
mod layout_validate_manifest;
mod mat_rename;
mod msbt_inspect;
mod msbt_roundtrip_test;
mod pane_clone;
mod pane_set;
mod restbl_inspect;
mod restbl_roundtrip_test;
mod restbl_set;
mod sarc_pack;
mod sarc_unpack;

use anyhow::{Context, Result};
use clap::Subcommand;
use std::path::Path;
use std::process::ExitCode;

use crate::bntx::pipeline::ImportTextureFormat;
use crate::compression::DictRegistry;

/// Load the zstd dictionary registry the compression verbs need.
///
/// `--dict` may be either TotK's `ZsDic.pack.zs` (a plain zstd frame
/// wrapping a SARC of `*.zsdic`) or a directory of extracted `*.zsdic`
/// files. `--romfs` points at a RomFS root and auto-finds
/// `Pack/ZsDic.pack.zs`. With neither, an empty registry is returned —
/// fine for plain zstd / Yaz0 / dictionary-less data.
pub(crate) fn load_dict_registry(
    dict: Option<&Path>,
    romfs: Option<&Path>,
) -> Result<DictRegistry> {
    if let Some(p) = dict {
        if p.is_dir() {
            return DictRegistry::from_dir(p).map_err(|e| anyhow::anyhow!("{e}"));
        }
        let bytes = std::fs::read(p).with_context(|| format!("reading dictionary {}", p.display()))?;
        return DictRegistry::from_zsdic_pack(&bytes)
            .map_err(|e| anyhow::anyhow!("loading {}: {e}", p.display()));
    }
    if let Some(root) = romfs {
        let p = root.join("Pack").join("ZsDic.pack.zs");
        let bytes = std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?;
        return DictRegistry::from_zsdic_pack(&bytes)
            .map_err(|e| anyhow::anyhow!("loading {}: {e}", p.display()));
    }
    Ok(DictRegistry::new())
}

/// Write `bytes` to `target`, creating parent directories as needed.
/// Shared by the mutating verbs so the "make parent dir, then write"
/// dance lives in one place.
pub(crate) fn write_output(target: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(target, bytes).with_context(|| format!("writing {}", target.display()))?;
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

/// Parse a `--texture-format` CLI value into the import format and the
/// effective sRGB flag for BC7. BC7 sRGB-ness can be requested either via
/// the explicit `bc7-srgb` alias or the separate `--srgb` flag; the RGBA8
/// variants carry their own sRGB-ness. Accepts the common spellings
/// (`_`/`-` interchangeable). Shared by the PNG-import verbs.
pub(crate) fn parse_import_texture_format(
    s: &str,
    srgb_flag: bool,
) -> Result<(ImportTextureFormat, bool)> {
    let key = s.trim().to_ascii_lowercase().replace('_', "-");
    Ok(match key.as_str() {
        "bc7" | "bc7-unorm" => (ImportTextureFormat::Bc7, srgb_flag),
        "bc7-srgb" | "bc7-unorm-srgb" => (ImportTextureFormat::Bc7, true),
        "rgba8" | "rgba8-unorm" | "r8g8b8a8" | "r8g8b8a8-unorm" => {
            if srgb_flag {
                (ImportTextureFormat::Rgba8Srgb, true)
            } else {
                (ImportTextureFormat::Rgba8, false)
            }
        }
        "rgba8-srgb" | "r8g8b8a8-srgb" => (ImportTextureFormat::Rgba8Srgb, true),
        other => {
            return Err(anyhow::anyhow!(
                "unknown --texture-format '{other}'; valid: bc7, bc7-srgb, rgba8, rgba8-srgb \
                 (aliases: bc7-unorm, bc7-unorm-srgb, rgba8-unorm, r8g8b8a8, r8g8b8a8-srgb)"
            ))
        }
    })
}

#[derive(Subcommand, Debug)]
pub enum Verb {
    /// Print a structured snapshot of a BFLAN (Cafe Layout Animation):
    /// header, sections, and decoded pat1/pai1. Use --json.
    BflanInspect(bflan_inspect::Args),

    /// Internal: read a BFLAN, write it back, and report whether the
    /// round-trip is byte-identical.
    BflanRoundtripTest(bflan_roundtrip_test::Args),

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

    /// Deswizzle + decode one named texture to a PNG (honors the
    /// texture's channel-swizzle; `--raw` shows the natural channels).
    BntxExportPng(bntx_export_png::Args),

    /// Deswizzle + decode every texture in a BNTX to PNGs in a directory.
    BntxExportAll(bntx_export_all::Args),

    /// Deswizzle one named texture and write it as a DDS file (DX10
    /// header) for lossless compressed-texture interchange.
    BntxExportDds(bntx_export_dds::Args),

    /// Swizzle a DDS surface and append it as a new named texture
    /// (format/dimensions/mips preserved from the DDS).
    BntxImportDds(bntx_import_dds::Args),

    /// Splice a DDS surface over an existing texture in place (must match
    /// the texture's format/dimensions/mips/layout).
    BntxReplaceDds(bntx_replace_dds::Args),

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

    /// Print a structured snapshot of a BYML/BYAML document (version,
    /// endianness, decoded value tree). Inflates `.byml.zs` via
    /// `--dict`/`--romfs`. Use --json / --max-depth.
    BymlInspect(byml_inspect::Args),

    /// Internal: read a BYML, write it back, and report whether the
    /// round-trip is byte-identical (inflating `.byml.zs` first).
    BymlRoundtripTest(byml_roundtrip_test::Args),

    /// Structural before/after diff of two BYML documents (hashes matched by
    /// key, arrays by index). Inflates `.byml.zs`. Use --json.
    BymlDiff(byml_diff::Args),

    /// Print a structured snapshot of an MSBT (LibMessageStudio message) file:
    /// endianness, encoding, sections, and decoded label→message text.
    /// Inflates `.msbt.zs`. Use --json / --limit.
    MsbtInspect(msbt_inspect::Args),

    /// Internal: read an MSBT, write it back, and report whether the
    /// round-trip is byte-identical (inflating `.msbt.zs` first).
    MsbtRoundtripTest(msbt_roundtrip_test::Args),

    /// Print a structured snapshot of a RESTBL (Resource Size Table): version,
    /// table counts, name (collision) table, and optional path/hash lookup.
    /// Inflates `.rsizetable.zs`. Use --json.
    RestblInspect(restbl_inspect::Args),

    /// Internal: read a RESTBL, write it back, and report whether the
    /// round-trip is byte-identical (inflating `.rsizetable.zs` first).
    RestblRoundtripTest(restbl_roundtrip_test::Args),

    /// Update a resource's reserved size in a RESTBL (by --path / --hash /
    /// --name), optionally inserting it. Required to repack mods without
    /// crashing the game.
    RestblSet(restbl_set::Args),

    /// Apply an SGPO skin manifest to a packed `layout.arc` end-to-end:
    /// unpack in memory, apply to the BFLYT+BNTX, validate, and re-pack
    /// every entry into a new archive.
    LayoutApplyArc(layout_apply_arc::Args),

    /// Apply an SGPO skin manifest to an unpacked layout: encode each
    /// element's PNG to BC7 + append to BNTX, then add the matching
    /// txl1/material/pane in BFLYT. Modifies files in place.
    LayoutApplyManifest(layout_apply_manifest::Args),

    /// Validate that an unpacked layout directory matches an SGPO skin
    /// manifest. Exits 0 on full match, 1 on any element mismatch.
    LayoutValidateManifest(layout_validate_manifest::Args),

    /// Structured before/after diff of two `layout.arc` files (BFLYT +
    /// BNTX). Use --json for tooling.
    LayoutDiff(layout_diff::Args),

    /// Recursively scan a directory/archive for unsupported or suspicious
    /// BFLYT/BNTX structures and emit a JSON report.
    LayoutAudit(layout_audit::Args),

    /// Extract a SARC archive to a directory tree.
    SarcUnpack(sarc_unpack::Args),

    /// Pack a directory tree into a SARC archive.
    SarcPack(sarc_pack::Args),

    /// Decompress a zstd (`.zs`/`.pack.zs`/`.blarc.zs`) or Yaz0 (`.szs`)
    /// file. zstd dictionaries (TotK) are selected by frame id from
    /// `--dict`/`--romfs`.
    Decompress(decompress::Args),

    /// Compress a file as zstd (optionally with a TotK dictionary) or Yaz0.
    /// Lossless, but not byte-identical to the game's original encoder.
    Compress(compress::Args),

    /// Decompress + unpack an archive (`.arc`/`.pack.zs`/`.blarc.zs`/`.szs`)
    /// to a directory tree, inflating any compressed entries inside.
    ArchiveExtract(archive_extract::Args),
}

pub fn dispatch(verb: Verb) -> Result<ExitCode> {
    match verb {
        Verb::BflanInspect(args) => Ok(bflan_inspect::run(args)?),
        Verb::BflanRoundtripTest(args) => Ok(bflan_roundtrip_test::run(args)?),
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
        Verb::BntxExportPng(args) => Ok(bntx_export_png::run(args)?),
        Verb::BntxExportAll(args) => Ok(bntx_export_all::run(args)?),
        Verb::BntxExportDds(args) => Ok(bntx_export_dds::run(args)?),
        Verb::BntxImportDds(args) => Ok(bntx_import_dds::run(args)?),
        Verb::BntxReplaceDds(args) => Ok(bntx_replace_dds::run(args)?),
        Verb::BntxImportPng(args) => Ok(bntx_import_png::run(args)?),
        Verb::BntxReplacePng(args) => Ok(bntx_replace_png::run(args)?),
        Verb::BntxRemoveTexture(args) => Ok(bntx_remove_texture::run(args)?),
        Verb::BntxRoundtripTest(args) => Ok(bntx_roundtrip_test::run(args)?),
        Verb::BntxDictTest(args) => Ok(bntx_dict_test::run(args)?),
        Verb::BntxRltDump(args) => Ok(bntx_rlt_dump::run(args)?),
        Verb::BntxLayoutDump(args) => Ok(bntx_layout_dump::run(args)?),
        Verb::BymlInspect(args) => Ok(byml_inspect::run(args)?),
        Verb::BymlRoundtripTest(args) => Ok(byml_roundtrip_test::run(args)?),
        Verb::BymlDiff(args) => Ok(byml_diff::run(args)?),
        Verb::MsbtInspect(args) => Ok(msbt_inspect::run(args)?),
        Verb::MsbtRoundtripTest(args) => Ok(msbt_roundtrip_test::run(args)?),
        Verb::RestblInspect(args) => Ok(restbl_inspect::run(args)?),
        Verb::RestblRoundtripTest(args) => Ok(restbl_roundtrip_test::run(args)?),
        Verb::RestblSet(args) => Ok(restbl_set::run(args)?),
        Verb::LayoutApplyArc(args) => Ok(layout_apply_arc::run(args)?),
        Verb::LayoutApplyManifest(args) => Ok(layout_apply_manifest::run(args)?),
        Verb::LayoutValidateManifest(args) => Ok(layout_validate_manifest::run(args)?),
        Verb::LayoutDiff(args) => Ok(layout_diff::run(args)?),
        Verb::LayoutAudit(args) => Ok(layout_audit::run(args)?),
        Verb::SarcUnpack(args) => Ok(sarc_unpack::run(args)?),
        Verb::SarcPack(args) => Ok(sarc_pack::run(args)?),
        Verb::Decompress(args) => Ok(decompress::run(args)?),
        Verb::Compress(args) => Ok(compress::run(args)?),
        Verb::ArchiveExtract(args) => Ok(archive_extract::run(args)?),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_import_texture_format;
    use crate::bntx::pipeline::ImportTextureFormat::{Bc7, Rgba8, Rgba8Srgb};

    #[test]
    fn texture_format_flag_aliases() {
        // BC7: sRGB comes from the --srgb flag, or the explicit -srgb alias.
        assert_eq!(parse_import_texture_format("bc7", false).unwrap(), (Bc7, false));
        assert_eq!(parse_import_texture_format("bc7", true).unwrap(), (Bc7, true));
        assert_eq!(parse_import_texture_format("BC7-UNORM", false).unwrap(), (Bc7, false));
        assert_eq!(parse_import_texture_format("bc7-srgb", false).unwrap(), (Bc7, true));
        assert_eq!(parse_import_texture_format("bc7_unorm_srgb", false).unwrap(), (Bc7, true));
        // RGBA8: the variant carries sRGB; --srgb promotes the plain alias.
        assert_eq!(parse_import_texture_format("rgba8", false).unwrap(), (Rgba8, false));
        assert_eq!(parse_import_texture_format("rgba8", true).unwrap(), (Rgba8Srgb, true));
        assert_eq!(parse_import_texture_format("r8g8b8a8", false).unwrap(), (Rgba8, false));
        assert_eq!(parse_import_texture_format("rgba8-srgb", false).unwrap(), (Rgba8Srgb, true));
        assert_eq!(parse_import_texture_format("r8g8b8a8_srgb", false).unwrap(), (Rgba8Srgb, true));
        // Unknown values are rejected.
        assert!(parse_import_texture_format("astc", false).is_err());
        assert!(parse_import_texture_format("", false).is_err());
    }
}
