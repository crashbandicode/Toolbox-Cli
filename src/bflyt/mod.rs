//! Pure-Rust BFLYT v8 (Cafe Layout) parser and writer.
//!
//! Targets Switch BFLYT version 8.x.x.x as shipped in Smash Ultimate, MK8DX,
//! and other modern Switch titles. Wii U / 3DS BFLYT (v5/v7) are NOT supported.
//!
//! Format references:
//! - Switch-Toolbox source (KillzXGaming, GPL-3.0) — used as reading material
//!   only; no code copied.
//! - https://nintendo-formats.com/libs/nw/bflyt.html
//! - https://github.com/FuryBaguette/SwitchLayoutEditor (SwitchThemesCommon)
//! - http://mk8.tockdom.com/wiki/

mod ops;
mod read;
mod sections;
mod write;

pub use ops::{ClonePaneSpec, PaneEdit};

// Re-export the types CLI verbs and external callers need. Internal-only
// types stay private inside `sections` and `read`/`write`.
#[allow(unused_imports)]
pub use sections::{
    AlphaCompare, BasePane, BflytError, BlendMode, Color8, FontShadowParameter, Group,
    IndirectParameter, LayoutInfo, Material, OpaqueSection, PaneKind, PaneTexCoord, PartsPane,
    PartsProperty, PicturePane, ProjectionTexGenParam, TevStage, TexCoordGen, TextBoxPane,
    TextureRef, TextureTransform, UserData, Vec2, Vec3, WindowContent, WindowFrame, WindowPane,
    BFLYT,
};

/// BFLYT material name slot size (28 bytes), exposed for callers that
/// validate proposed names. The full slot is usable: real BFLYTs store
/// names of exactly this length without a trailing null, so the max
/// accepted name length is `MAT_NAME_LEN_USIZE` bytes.
pub const MAT_NAME_LEN_USIZE: usize = 0x1C;

pub use read::read_bflyt;
pub use write::write_bflyt;
