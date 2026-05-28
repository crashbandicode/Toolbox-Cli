//! BFLYT v8 section parsing and writing.
//!
//! The format is a header followed by a flat list of sections (`lyt1`,
//! `txl1`, `fnl1`, `mat1`, `pan1`/`pic1`/`txt1`/`wnd1`/`prt1`/`bnd1`,
//! `pas1`/`pae1`, `grp1`/`grs1`/`gre1`, `usd1`). Pane parent/child
//! relationships are encoded by `pas1`/`pae1` brackets between pane sections.
//!
//! We keep the in-memory layout shape close to what the file looks like:
//! a `BFLYT` carries `LayoutInfo`, `TextureList`, `FontList`, `Materials`,
//! and a single root `BasePane` whose `children` form the tree. Writing
//! reverses the parse exactly — section sizes and offsets are all
//! recomputed.

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{Read, Write};

/// Switch BFLYT material name slot is 28 bytes (v8). Matches Switch
/// Toolbox `MAT1.cs` (`Name = reader.ReadString(0x1C, true)`).
pub const MAT_NAME_LEN: usize = 0x1C;
/// Pane name slot is 24 bytes.
pub const PANE_NAME_LEN: usize = 0x18;
/// User-data field on each pane is 8 bytes.
pub const PANE_USER_DATA_LEN: usize = 0x08;

// ---------- Common value types ----------

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Color8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Vec2 {
    pub fn read<R: Read>(r: &mut R) -> std::io::Result<Self> {
        Ok(Self {
            x: r.read_f32::<LittleEndian>()?,
            y: r.read_f32::<LittleEndian>()?,
        })
    }
    pub fn write<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
        w.write_f32::<LittleEndian>(self.x)?;
        w.write_f32::<LittleEndian>(self.y)?;
        Ok(())
    }
}

impl Vec3 {
    pub fn read<R: Read>(r: &mut R) -> std::io::Result<Self> {
        Ok(Self {
            x: r.read_f32::<LittleEndian>()?,
            y: r.read_f32::<LittleEndian>()?,
            z: r.read_f32::<LittleEndian>()?,
        })
    }
    pub fn write<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
        w.write_f32::<LittleEndian>(self.x)?;
        w.write_f32::<LittleEndian>(self.y)?;
        w.write_f32::<LittleEndian>(self.z)?;
        Ok(())
    }
}

impl Color8 {
    pub fn read<R: Read>(r: &mut R) -> std::io::Result<Self> {
        let mut buf = [0u8; 4];
        r.read_exact(&mut buf)?;
        Ok(Self {
            r: buf[0],
            g: buf[1],
            b: buf[2],
            a: buf[3],
        })
    }
    pub fn write<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
        w.write_all(&[self.r, self.g, self.b, self.a])
    }
}

// ---------- Top-level BFLYT struct ----------

/// Parsed BFLYT file. Section ordering on disk is reconstructed when
/// writing; the in-memory representation is logical only.
#[derive(Debug, Clone)]
pub struct BFLYT {
    pub version: u32,
    pub layout: LayoutInfo,
    pub textures: Vec<String>,
    pub fonts: Vec<String>,
    pub materials: Vec<Material>,
    pub root_pane: Option<BasePane>,
    pub root_group: Option<Group>,
    pub user_data: Option<UserData>,
    /// `cnt1` (control data) section. Used by Smash Ultimate's player
    /// layouts. We don't decode the structure yet; the bytes are
    /// preserved verbatim for round-trip.
    pub control_data: Option<UserData>,
    /// Pane-tree-adjacent sections we don't decode (`scr1` scissor, `ali1`
    /// alignment, `spi1` shape-info, …). Re-emitted in the same position
    /// they appeared on disk so round-trip is byte-identical.
    pub opaque_sections: Vec<OpaqueSection>,
}

/// A pane-tree-adjacent section we preserve verbatim. `after_pane_name`
/// is the name of the pane this section follows in the original file's
/// section ordering; the writer re-emits it after that pane.
#[derive(Debug, Clone)]
pub struct OpaqueSection {
    pub magic: [u8; 4],
    pub payload: Vec<u8>,
    /// Name of the pane this section follows. `None` means file-level
    /// (before any pane). Multiple opaque sections may share the same
    /// `after_pane_name`; they're emitted in the order they appeared on
    /// disk.
    pub after_pane_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LayoutInfo {
    pub draw_centered: bool,
    pub width: f32,
    pub height: f32,
    pub max_parts_width: f32,
    pub max_parts_height: f32,
    pub name: String,
}

// ---------- Pane tree ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneKind {
    Pane,    // pan1
    Picture, // pic1
    Text,    // txt1
    Window,  // wnd1
    Parts,   // prt1
    Bounding,// bnd1
}

/// A pane tree node. Holds the common ResPane fields, an optional kind-
/// specific extension, and child panes.
#[derive(Debug, Clone)]
pub struct BasePane {
    pub kind: PaneKind,
    pub flag: u8,
    pub base_position: u8,
    pub alpha: u8,
    pub flag_ex: u8,
    pub name: String,
    pub user_data_field: [u8; PANE_USER_DATA_LEN],
    pub translate: Vec3,
    pub rotate: Vec3,
    pub scale: Vec2,
    pub width: f32,
    pub height: f32,
    pub picture: Option<PicturePane>,
    pub text: Option<TextBoxPane>,
    pub window: Option<WindowPane>,
    pub parts: Option<PartsPane>,
    pub user_data: Option<UserData>,
    pub children: Vec<BasePane>,
    /// Trailing bytes within the pane section after the standard pane
    /// base (and any kind-specific extension we decode). Some HDR mods
    /// append 8 extra zero bytes per pane; we preserve them verbatim
    /// for byte-identical round-trip.
    pub trailing: Vec<u8>,
}

impl BasePane {
    /// Pane visible flag (bit 0 of `flag`).
    pub fn visible(&self) -> bool {
        (self.flag & 0x01) != 0
    }
    pub fn set_visible(&mut self, v: bool) {
        self.flag = (self.flag & !0x01) | if v { 1 } else { 0 };
    }

    /// Influence-alpha flag (bit 1 of `flag`).
    pub fn influence_alpha(&self) -> bool {
        (self.flag & 0x02) != 0
    }
}

#[derive(Debug, Clone, Default)]
pub struct PicturePane {
    pub vertex_colors: [Color8; 4],
    pub material_index: u16,
    pub tex_coords: Vec<PaneTexCoord>,
    /// `flags` byte that follows `tex_coord_count` in pic1 (always 0 for v8
    /// in our experience but preserved for round-trip).
    pub flags: u8,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PaneTexCoord {
    pub top_left: Vec2,
    pub top_right: Vec2,
    pub bottom_left: Vec2,
    pub bottom_right: Vec2,
}

/// txt1 (text box) pane. The text payload itself is preserved as raw bytes
/// so we don't have to pick a specific encoding (UTF-16LE is typical but
/// some games store other things).
#[derive(Debug, Clone, Default)]
pub struct TextBoxPane {
    pub text_buf_bytes: u16,
    pub text_str_bytes: u16,
    pub material_index: u16,
    pub font_index: u16,
    pub text_position: u8,
    pub text_alignment: u8,
    pub text_box_flag: u16,
    pub italic_ratio: f32,
    pub text_str_offset: u32,
    pub text_cols: [Color8; 2],
    pub font_size: Vec2,
    pub char_space: f32,
    pub line_space: f32,
    pub text_id_offset: u32,
    pub shadow_offset: Vec2,
    pub shadow_scale: Vec2,
    pub shadow_cols: [Color8; 2],
    pub shadow_italic_ratio: f32,
    pub line_width_offset_offset: u32,
    pub per_character_transform_offset: u32,
    /// Trailing data: text bytes, optional text_id string, optional
    /// per-character transform, optional line-width offset table. We keep
    /// it all as opaque bytes to preserve round-trip fidelity.
    pub trailing: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct WindowPane {
    pub stretch_l: u16,
    pub stretch_r: u16,
    pub stretch_t: u16,
    pub stretch_b: u16,
    pub frame_size_l: u16,
    pub frame_size_r: u16,
    pub frame_size_t: u16,
    pub frame_size_b: u16,
    pub frame_count: u8,
    pub flag: u8,
    pub content: WindowContent,
    pub frames: Vec<WindowFrame>,
}

#[derive(Debug, Clone, Default)]
pub struct WindowContent {
    pub vertex_colors: [Color8; 4],
    pub material_index: u16,
    pub tex_coords: Vec<PaneTexCoord>,
}

#[derive(Debug, Clone, Default)]
pub struct WindowFrame {
    pub material_index: u16,
    pub texture_flip: u8,
    /// 1 byte of padding/alignment; preserved for round-trip.
    pub _padding: u8,
}

#[derive(Debug, Clone, Default)]
pub struct PartsPane {
    pub property_count: u32,
    pub magnify: Vec2,
    pub properties: Vec<PartsProperty>,
    pub part_name: String,
    /// Properties' embedded sub-sections (resolved during parsing) kept as
    /// raw bytes for now to preserve round-trip fidelity. Editing these is
    /// out of scope for SGPO.
    pub raw_property_data: Vec<u8>,
    /// Total declared section size; recomputed on write but cached for
    /// debug output.
    pub declared_size: u32,
}

#[derive(Debug, Clone, Default)]
pub struct PartsProperty {
    pub name: String, // 24 bytes, null-padded
    pub usage_flag: u8,
    pub basic_usage_flag: u8,
    pub material_usage_flag: u8,
    pub system_ext_user_data_override_flag: u8,
    pub property_offset: u32,
    pub ext_user_data_offset: u32,
    pub pane_basic_info_offset: u32,
}

// ---------- Materials ----------

#[derive(Debug, Clone, Default)]
pub struct Material {
    pub name: String,
    pub flags_unknown: u32, // the "unknown" int that follows flags in v8
    pub black_color: Color8,
    pub white_color: Color8,
    pub flags_raw: u32,
    pub texture_maps: Vec<TextureRef>,
    pub texture_transforms: Vec<TextureTransform>,
    pub tex_coord_gens: Vec<TexCoordGen>,
    pub tev_stages: Vec<TevStage>,
    pub alpha_compare: Option<AlphaCompare>,
    pub blend_mode: Option<BlendMode>,
    pub blend_mode_logic: Option<BlendMode>,
    pub indirect_param: Option<IndirectParameter>,
    pub proj_tex_gen_params: Vec<ProjectionTexGenParam>,
    pub font_shadow_param: Option<FontShadowParameter>,
    /// Trailing bytes within a material that we don't yet decode. v9 BFLYT
    /// adds an undocumented extension after the known sub-sections in some
    /// materials (gated by an unknown flag bit). We preserve them verbatim
    /// to keep round-trip byte-identical without committing to a decode
    /// that may be wrong.
    pub trailing: Vec<u8>,
    /// True when the reader had to shrink sub-section counts to fit the
    /// material's declared byte budget (e.g., HDR mod produces materials
    /// whose `flags_raw` says `mtx_count=2` but only one transform's
    /// worth of data is present). When this is set, the writer must
    /// emit `flags_raw` verbatim rather than recomputing from the
    /// in-memory sub-section counts.
    ///
    /// Mutating an untrusted material's sub-section counts WITHOUT
    /// first calling `clear_untrusted_flag()` is a programmer error:
    /// the writer would emit a file whose `flags_raw` disagrees with
    /// its sub-section data, and the runtime would parse the wrong
    /// number of entries. The writer's `debug_assert!` catches this
    /// in dev builds.
    pub flags_untrusted: bool,
    /// On-disk byte size of the material section as it was when read
    /// from disk. `None` for materials constructed in code (synthetic
    /// tests, builders). Used solely by the writer's
    /// `debug_assert!` to verify untrusted materials haven't been
    /// silently mutated since read time.
    pub original_section_size: Option<u32>,
}

impl Material {
    /// Raw section data after the name slot for bits we don't fully decode.
    /// Currently every field above IS decoded, so this stays empty unless
    /// we hit an unknown sub-section.
    pub fn texture_count(&self) -> u8 {
        self.texture_maps.len().min(3) as u8
    }

    /// Recompute `flags_raw` from the in-memory sub-section counts and
    /// option presence. Called automatically on write.
    ///
    /// Bits we own (recomputed):
    ///   0-1   texture-map count
    ///   2-3   texture-transform count
    ///   4-5   tex-coord-gen count
    ///   6-8   tev-stage count
    ///   9     alpha compare present
    ///   10    blend mode present
    ///   12    blend mode logic present
    ///   14    indirect param present
    ///   15-16 projection tex-gen count
    ///   17    font shadow present
    ///
    /// All other bits (notably 11 = use-texture-only, 13 = alpha
    /// interpolation, 18, and any v9-specific bits like 19 that gate
    /// undocumented trailing data) are preserved verbatim from the
    /// original `flags_raw`.
    pub fn rebuild_flags(&mut self) {
        let owned_mask: u32 = 0b111
            | (0b11 << 2)        // mtx
            | (0b11 << 4)        // tex_coord_gen
            | (0b111 << 6)       // tev_stage
            | (1 << 9)           // alpha_compare
            | (1 << 10)          // blend_mode
            | (1 << 12)          // blend_mode_logic
            | (1 << 14)          // indirect
            | (0b11 << 15)       // proj_tex_gen
            | (1 << 17);         // font_shadow

        let tex_count = self.texture_maps.len().min(3) as u32;
        let mtx_count = self.texture_transforms.len().min(3) as u32;
        let tex_coord_gen_count = self.tex_coord_gens.len().min(3) as u32;
        let tev_stage_count = self.tev_stages.len().min(7) as u32;
        let proj_tex_gen_count = self.proj_tex_gen_params.len().min(3) as u32;

        let mut owned = 0u32;
        owned |= tex_count & 0x3;
        owned |= (mtx_count & 0x3) << 2;
        owned |= (tex_coord_gen_count & 0x3) << 4;
        owned |= (tev_stage_count & 0x7) << 6;
        if self.alpha_compare.is_some() { owned |= 1 << 9; }
        if self.blend_mode.is_some() { owned |= 1 << 10; }
        if self.blend_mode_logic.is_some() { owned |= 1 << 12; }
        if self.indirect_param.is_some() { owned |= 1 << 14; }
        owned |= (proj_tex_gen_count & 0x3) << 15;
        if self.font_shadow_param.is_some() { owned |= 1 << 17; }

        // Keep every bit we don't own; overwrite the bits we do.
        self.flags_raw = (self.flags_raw & !owned_mask) | (owned & owned_mask);
    }

    /// Guard for save paths: returns an error if this material is still
    /// in the untrusted state (i.e., the on-disk source had a malformed
    /// `flags_raw` / sub-section-count mismatch). Call this before
    /// performing structural mutations or before saving when you need
    /// strong consistency guarantees on a particular material.
    pub fn assert_flags_trusted(&self) -> Result<(), super::BflytError> {
        if self.flags_untrusted {
            Err(super::BflytError::Format(format!(
                "material '{}' has flags_untrusted=true (the source BFLYT had a malformed mat1 \
                 sub-section count); call `clear_untrusted_flag()` after verifying the \
                 sub-section state is consistent before mutating or saving",
                self.name
            )))
        } else {
            Ok(())
        }
    }

    /// Re-canonicalize this material: recompute `flags_raw` from the
    /// in-memory sub-section counts, drop any `original_section_size`
    /// snapshot, and clear the `flags_untrusted` bit. After this call
    /// the material is in the same state a freshly-built material
    /// would be in — fully owned by the in-memory representation —
    /// and the writer will recompute `flags_raw` automatically when
    /// counts change.
    ///
    /// Use this after manually fixing up an untrusted material's
    /// sub-sections (e.g., padding `texture_transforms` with default
    /// entries to match the count `flags_raw` originally claimed). The
    /// writer will then trust the in-memory counts.
    pub fn clear_untrusted_flag(&mut self) {
        self.rebuild_flags();
        self.flags_untrusted = false;
        self.original_section_size = None;
    }

    /// Compute the byte size this material would emit through the BFLYT
    /// writer. Mirrors the layout in `write::write_material`. Used by
    /// the writer's `debug_assert!` to detect silent mutations on
    /// untrusted materials.
    pub fn emit_size(&self) -> u32 {
        // header (matches the reader's `header_bytes` accounting):
        //   name MAT_NAME_LEN + flags_raw 4 + flags_unknown 4 + black 4 + white 4
        let mut s = MAT_NAME_LEN + 4 + 4 + 4 + 4;
        s += self.texture_maps.len() * 4;
        s += self.texture_transforms.len() * 20;
        s += self.tex_coord_gens.len() * 16;
        s += self.tev_stages.len() * 4;
        if self.alpha_compare.is_some() {
            s += 8;
        }
        if self.blend_mode.is_some() {
            s += 4;
        }
        if self.blend_mode_logic.is_some() {
            s += 4;
        }
        if self.indirect_param.is_some() {
            s += 12;
        }
        s += self.proj_tex_gen_params.len() * 20;
        if self.font_shadow_param.is_some() {
            s += 8;
        }
        s += self.trailing.len();
        s as u32
    }
}

#[derive(Debug, Clone, Default)]
pub struct TextureRef {
    /// Index into BFLYT.textures.
    pub index: i16,
    pub wrap_mode_u: u8,
    pub wrap_mode_v: u8,
}

#[derive(Debug, Clone, Default)]
pub struct TextureTransform {
    pub translate: Vec2,
    pub rotate: f32,
    pub scale: Vec2,
}

#[derive(Debug, Clone, Default)]
pub struct TexCoordGen {
    pub matrix_type: u8,
    pub source: u8,
    pub unk: [u8; 2],
    pub raw: [u8; 16], // version-dependent; preserved verbatim
}

#[derive(Debug, Clone, Default)]
pub struct TevStage {
    pub color_blend: u8,
    pub alpha_blend: u8,
    pub unk: [u8; 2],
}

#[derive(Debug, Clone, Default)]
pub struct AlphaCompare {
    pub function: u8,
    pub _padding: [u8; 3],
    pub reference: f32,
}

#[derive(Debug, Clone, Default)]
pub struct BlendMode {
    pub blend_op: u8,
    pub src_factor: u8,
    pub dst_factor: u8,
    pub logic_op: u8,
}

/// IndirectParameter is 3 floats (rotation, scale_x, scale_y) = 12 bytes.
#[derive(Debug, Clone, Default)]
pub struct IndirectParameter {
    pub raw: [u8; 12],
}

/// ProjectionTexGenParam is 4 floats + 1 byte flags + 3 bytes padding = 20 bytes.
#[derive(Debug, Clone, Default)]
pub struct ProjectionTexGenParam {
    pub raw: [u8; 20],
}

/// FontShadowParameter is two RGBA8 colors = 8 bytes.
#[derive(Debug, Clone, Default)]
pub struct FontShadowParameter {
    pub raw: [u8; 8],
}

// ---------- Groups & user data ----------

#[derive(Debug, Clone, Default)]
pub struct Group {
    pub name: String, // 24 bytes
    pub panes: Vec<String>, // 24-byte names
    pub children: Vec<Group>,
}

#[derive(Debug, Clone, Default)]
pub struct UserData {
    pub raw: Vec<u8>,
}

// ---------- Errors ----------

pub mod error {
    use std::fmt;

    #[derive(Debug)]
    pub enum Error {
        Io(std::io::Error),
        BadMagic([u8; 4]),
        BadBom(u16),
        UnsupportedVersion(u32),
        TruncatedSection(String),
        UnknownSection([u8; 4]),
        InvalidPaneNesting(String),
        Format(String),
    }

    impl fmt::Display for Error {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Error::Io(e) => write!(f, "io: {e}"),
                Error::BadMagic(m) => {
                    write!(f, "not a BFLYT (magic = {:?})", std::str::from_utf8(m).unwrap_or("?"))
                }
                Error::BadBom(b) => write!(f, "unsupported BOM 0x{b:04x} (only little-endian supported)"),
                Error::UnsupportedVersion(v) => write!(
                    f,
                    "unsupported BFLYT version {}.{}.{}.{} (this CLI is Switch v8 only)",
                    (v >> 24) & 0xff,
                    (v >> 16) & 0xff,
                    (v >> 8) & 0xff,
                    v & 0xff
                ),
                Error::TruncatedSection(s) => write!(f, "truncated section: {s}"),
                Error::UnknownSection(m) => write!(
                    f,
                    "unknown section magic {:?}",
                    std::str::from_utf8(m).unwrap_or("?")
                ),
                Error::InvalidPaneNesting(s) => write!(f, "invalid pane nesting: {s}"),
                Error::Format(s) => write!(f, "format error: {s}"),
            }
        }
    }

    impl std::error::Error for Error {}

    impl From<std::io::Error> for Error {
        fn from(e: std::io::Error) -> Self {
            Error::Io(e)
        }
    }

    impl From<binrw::Error> for Error {
        fn from(e: binrw::Error) -> Self {
            Error::Format(format!("{e}"))
        }
    }
}

pub use error::Error as BflytError;
