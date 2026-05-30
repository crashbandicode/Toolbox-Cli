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

pub mod audit;
pub mod bflan;
pub mod bflyt;
pub mod bntx;
pub mod dds;
pub mod diff;
pub mod layout;
pub mod manifest;
pub mod sarc;
pub mod texpipe;

/// Commonly used imports. `use nx_layout_toolbox::prelude::*;` pulls in the
/// format read/write entry points, the BNTX import/replace helpers, the
/// BFLYT mutation specs, the manifest types, and [`Error`]/[`Result`].
pub mod prelude {
    pub use crate::bflyt::{read_bflyt, write_bflyt, ClonePaneSpec, PaneEdit, BFLYT};
    pub use crate::bntx::pipeline::{
        import_cube_png_files, import_image, import_png_file, replace_texture, ImportOptions,
        ReplaceSource,
    };
    pub use crate::bntx::{read_bntx, write_bntx, AppendTextureSpec, BntxFile, TextureFormat};
    pub use crate::layout::{
        apply_manifest, validate_manifest, ApplyOptions, ApplyReport, ValidateOptions,
        ValidateReport,
    };
    pub use crate::manifest::{SkinElement, SkinManifest};
    pub use crate::texpipe::Bc7Quality;
    pub use crate::{sarc, Error, Result};
}

/// CLI verbs that back the `nx-layout-toolbox` binary. Gated behind the
/// `cli` feature (enabled by default) so library consumers can opt out of
/// the `clap`/`anyhow` dependencies with `default-features = false`.
#[cfg(feature = "cli")]
pub mod verbs;
