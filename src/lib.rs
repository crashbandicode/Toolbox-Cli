//! # nx-layout-toolbox
//!
//! Pure-Rust library (and CLI) for editing Nintendo Switch UI assets:
//! **BFLYT** (Cafe Layout v8/v9), **BNTX** (texture container), and
//! **SARC** archives. It round-trips real Smash Ultimate assets
//! byte-identically and can append PNG-backed BC7 textures, clone
//! materials/panes, and apply a JSON skin manifest end-to-end.
//!
//! Inspired by [Switch-Toolbox] (GPL-3.0); all parsers and writers here are
//! original implementations informed by public format documentation, and
//! this crate is MIT-licensed with no GPL dependencies.
//!
//! ## Cargo features
//!
//! - `cli` *(default)* — builds the `nx-layout-toolbox` binary and the
//!   [`verbs`] module, pulling in `clap` + `anyhow`. Library consumers
//!   should disable it to depend on just the format library:
//!
//!   ```toml
//!   nx-layout-toolbox = { version = "0.1", default-features = false }
//!   ```
//!
//! ## Modules
//!
//! - [`bflyt`] — BFLYT parse/write plus mutation ops on [`bflyt::BFLYT`].
//! - [`bflan`] — BFLAN (layout animation) parse/write (byte-identical) +
//!   `pat1`/`pai1` inspect.
//! - [`bntx`] — BNTX parse/write and texture append/remove;
//!   [`bntx::pipeline`] adds PNG/DDS import/replace and
//!   [`bntx::decode`] does deswizzle + decode to RGBA.
//! - [`byml`] — BYML (binary YAML) read + verbatim round-trip + a decoded
//!   [`byml::Byml`] value tree (inspect / diff / [`byml::set_by_path`] edit).
//! - [`restbl`] — RESTBL (Resource Size Table) read/write (byte-identical)
//!   + CRC-32 path lookup / size update.
//! - [`aamp`] — AAMP (binary parameter archive, BOTW) read + verbatim
//!   round-trip + a decoded [`aamp::ParameterList`] tree (inspect).
//! - [`bfres`] — BFRES (`FRES`, BOTW/TotK 3D-resource container) header
//!   inspect + verbatim byte-identical round-trip.
//! - [`nso`] — NSO (Switch `exefs/main`) read + LZ4 segment decompression
//!   (for inspecting executable contents, e.g. the TotK MeshCodec dictionary).
//! - [`mc`] — MC/MCPK (TotK MeshCodec) container inspect + verbatim
//!   byte-identical round-trip (decompression/repack are in progress).
//! - [`msbt`] — MSBT (LibMessageStudio message) read + verbatim round-trip +
//!   decoded label/message tree (inspect).
//! - [`compression`] — zstd (with TotK dictionaries) and Yaz0/Yaz1
//!   (`.szs`) decode/encode + codec detection.
//! - [`texpipe`] — PNG → BC1/BC3/BC4/BC5/BC7 (intel_tex_2) → Tegra
//!   block-linear swizzle.
//! - [`dds`] — DDS (DX10) read/write for texture interchange.
//! - [`sarc`] — SARC archive read (via the `sarc` crate) + a custom
//!   per-file-alignment writer.
//! - [`manifest`] — SGPO skin-manifest schema.
//! - [`layout`] — high-level [`layout::apply_manifest`] /
//!   [`layout::validate_manifest`] / [`layout::apply_manifest_to_arc`].
//! - [`diff`] — structured BFLYT+BNTX before/after diff.
//! - [`audit`] — recursive scan for unsupported/suspicious structures.
//! - [`corpus_audit`] — multi-format real-corpus confidence measure
//!   (per-format byte-identical / semantic / inspect / unsupported / fail
//!   tally → JSON), recursing into SARC archives.
//!
//! Most names you need are re-exported from [`prelude`].
//!
//! ## Example
//!
//! ```no_run
//! use nx_layout_toolbox::bntx::pipeline::{import_png_file, ImportOptions};
//! use nx_layout_toolbox::bntx::{read_bntx, write_bntx};
//! use nx_layout_toolbox::texpipe::Bc7Quality;
//! use std::path::Path;
//!
//! # fn main() -> nx_layout_toolbox::Result<()> {
//! let mut bntx = read_bntx(&std::fs::read("__Combined.bntx")?)?;
//! let opts = ImportOptions { quality: Bc7Quality::Fast, ..Default::default() };
//! import_png_file(&mut bntx, "tex_my_button", Path::new("button.png"), &opts)?;
//! std::fs::write("__Combined.bntx", write_bntx(&bntx)?)?;
//! # Ok(())
//! # }
//! ```
//!
//! [Switch-Toolbox]: https://github.com/KillzXGaming/Switch-Toolbox

mod error;
pub use error::{Error, Result};

pub mod aamp;
pub mod audit;
pub mod bflan;
pub mod bflyt;
pub mod bfres;
pub mod bntx;
pub mod byml;
pub mod compression;
pub mod corpus_audit;
pub mod dds;
pub mod diff;
pub mod layout;
pub mod manifest;
pub mod mc;
pub mod meshopt;
pub mod msbt;
pub mod nso;
pub mod restbl;
pub mod sarc;
pub mod texpipe;
pub mod zstd_pure;

/// Commonly used imports. `use nx_layout_toolbox::prelude::*;` pulls in the
/// format read/write entry points, the BNTX import/replace helpers, the
/// BFLYT mutation specs, the manifest types, and [`Error`]/[`Result`].
pub mod prelude {
    pub use crate::aamp::{read_aamp, write_aamp, AampDocument, ParameterList, Value};
    pub use crate::bflyt::{read_bflyt, write_bflyt, ClonePaneSpec, PaneEdit, BFLYT};
    pub use crate::bfres::{read_bfres, write_bfres, BfresDocument, DetectedBlock};
    pub use crate::bntx::pipeline::{
        import_cube_png_files, import_image, import_png_file, replace_texture, ImportOptions,
        ImportTextureFormat, ReplaceSource,
    };
    pub use crate::bntx::{read_bntx, write_bntx, AppendTextureSpec, BntxFile, TextureFormat};
    pub use crate::byml::{
        diff_byml, read_byml, set_by_path, write_byml, write_byml_canonical, Byml, BymlDiff,
        BymlDocument, ScalarType, SetReport,
    };
    pub use crate::compression::{compress_yaz0, compress_zstd, decompress, Codec, DictRegistry};
    pub use crate::layout::{
        apply_manifest, validate_manifest, ApplyOptions, ApplyReport, ValidateOptions,
        ValidateReport,
    };
    pub use crate::manifest::{SkinElement, SkinManifest};
    pub use crate::msbt::{read_msbt, write_msbt, write_msbt_canonical, MsbtDocument, TextChunk};
    pub use crate::restbl::{read_restbl, write_restbl, Restbl, SetOutcome};
    pub use crate::texpipe::Bc7Quality;
    pub use crate::{sarc, Error, Result};
}

/// CLI verbs that back the `nx-layout-toolbox` binary. Gated behind the
/// `cli` feature (enabled by default) so library consumers can opt out of
/// the `clap`/`anyhow` dependencies with `default-features = false`.
#[cfg(feature = "cli")]
pub mod verbs;
