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

mod read;
mod sections;
mod write;

pub use sections::{
    AlphaCompare, BFLYT, BasePane, BflytError, BlendMode, Color8, FontShadowParameter, Group,
    IndirectParameter, LayoutInfo, Material, PaneKind, PaneTexCoord, PartsPane, PartsProperty,
    PicturePane, ProjectionTexGenParam, TevStage, TexCoordGen, TextBoxPane, TextureRef,
    TextureTransform, UserData, Vec2, Vec3, WindowContent, WindowFrame, WindowPane,
};

pub use read::read_bflyt;
pub use write::write_bflyt;
