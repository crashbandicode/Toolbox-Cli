//! Tests for the `flags_untrusted` guardrail on `Material`.
//!
//! Three layers of protection, each tested separately:
//!
//! - `Material::assert_flags_trusted()` — opt-in caller-side check that
//!   refuses to proceed if the material's flags came from a malformed
//!   source.
//! - `Material::clear_untrusted_flag()` — explicit "I've fixed this up,
//!   trust the in-memory state now" reset that recomputes `flags_raw`.
//! - `debug_assert!` in the BFLYT writer — fires in dev builds when an
//!   untrusted material's emit size diverges from its captured
//!   `original_section_size`, i.e. the caller mutated counts without
//!   first calling `clear_untrusted_flag()`.
//!
//! The first two are pure data tests; the writer test builds a synthetic
//! BFLYT (so it doesn't need real game assets) and gates on
//! `cfg(debug_assertions)` because release builds compile out the guard.

use nx_layout_toolbox::bflyt::*;

#[test]
fn trusted_material_passes_assert() {
    let mat = Material::default();
    assert!(mat.assert_flags_trusted().is_ok());
}

#[test]
fn untrusted_material_fails_assert_with_descriptive_error() {
    let mat = Material {
        name: "broken_mat".into(),
        flags_untrusted: true,
        ..Material::default()
    };
    let err = mat.assert_flags_trusted().unwrap_err().to_string();
    assert!(
        err.contains("flags_untrusted=true"),
        "error should explain the untrusted state: {err}",
    );
    assert!(
        err.contains("broken_mat"),
        "error should identify the material by name: {err}",
    );
    assert!(
        err.contains("clear_untrusted_flag"),
        "error should point at the recovery method: {err}",
    );
}

#[test]
fn clear_untrusted_flag_resets_state_and_unlocks_assert() {
    let mut mat = Material {
        name: "fixed_up".into(),
        flags_untrusted: true,
        original_section_size: Some(99),
        flags_raw: 0xFFFF_FFFF,
        texture_maps: vec![TextureRef {
            index: 0,
            wrap_mode_u: 0,
            wrap_mode_v: 0,
        }],
        ..Material::default()
    };
    mat.clear_untrusted_flag();
    assert!(!mat.flags_untrusted);
    assert!(mat.original_section_size.is_none());
    assert!(mat.assert_flags_trusted().is_ok());
    // `clear_untrusted_flag` recomputes `flags_raw` from in-memory
    // counts, so the texture-map count bits should match the Vec.
    assert_eq!(
        mat.flags_raw & 0x3,
        1,
        "flags_raw low bits should reflect the 1-element texture_maps Vec, got 0x{:x}",
        mat.flags_raw,
    );
}

/// Helper: build a synthetic BFLYT containing a single material, with
/// the given untrusted-state config. Mirrors `tests/bflyt_synthesis.rs`.
fn build_synthetic_bflyt(mat: Material) -> BFLYT {
    BFLYT {
        version: 0x09000000,
        layout: LayoutInfo {
            draw_centered: true,
            width: 1280.0,
            height: 720.0,
            max_parts_width: 1280.0,
            max_parts_height: 720.0,
            name: "guard_test".into(),
        },
        textures: vec!["tex".into()],
        fonts: vec![],
        materials: vec![mat],
        root_pane: Some(BasePane {
            kind: PaneKind::Pane,
            flag: 0x01,
            base_position: 0,
            alpha: 255,
            flag_ex: 0,
            name: "RootPane".into(),
            user_data_field: [0; 8],
            translate: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            rotate: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            scale: Vec2 { x: 1.0, y: 1.0 },
            width: 1280.0,
            height: 720.0,
            picture: None,
            text: None,
            window: None,
            parts: None,
            user_data: None,
            children: vec![],
            trailing: vec![],
            opaque: None,
        }),
        root_group: Some(Group {
            name: "RootGroup".into(),
            panes: vec![],
            children: vec![],
        }),
        user_data: None,
        control_data: None,
        opaque_sections: vec![],
        trailing_sections: vec![],
    }
}

/// Build a Material claiming on-disk size `original_size`, with sub-section
/// counts and `flags_raw` matching that claim. The result is in the
/// "untrusted-but-not-yet-mutated" state — its emit_size equals the
/// original_size, so the writer guard does not fire.
fn untrusted_consistent_material(original_size: u32) -> Material {
    let mut m = Material {
        name: "untrusted_mat".into(),
        flags_unknown: 0,
        black_color: Color8 {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        },
        white_color: Color8 {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        },
        flags_raw: 0,
        texture_maps: vec![TextureRef {
            index: 0,
            wrap_mode_u: 0,
            wrap_mode_v: 0,
        }],
        texture_transforms: vec![],
        tex_coord_gens: vec![],
        tev_stages: vec![],
        alpha_compare: None,
        blend_mode: None,
        blend_mode_logic: None,
        indirect_param: None,
        proj_tex_gen_params: vec![],
        font_shadow_param: None,
        trailing: vec![],
        flags_untrusted: false,
        original_section_size: None,
    };
    // Use `rebuild_flags` to encode the 1-element texture_map count in
    // `flags_raw` so the value isn't trivially zero.
    m.rebuild_flags();
    let probe_size = m.emit_size();
    // Force the captured size to `original_size`; if the caller picks
    // `probe_size` they get a baseline-consistent untrusted material.
    m.original_section_size = Some(original_size.max(probe_size));
    // Pad `trailing` so emit_size == claimed original_size when needed.
    if original_size > probe_size {
        m.trailing.resize((original_size - probe_size) as usize, 0);
    }
    m.flags_untrusted = true;
    m
}

/// The benign case: an untrusted material whose in-memory state still
/// matches its captured `original_section_size`. Writing must not
/// trip the guard, even in debug builds. Round-tripping the resulting
/// file should also succeed.
#[test]
fn untrusted_material_with_matching_size_writes_cleanly() {
    let mat = untrusted_consistent_material(0); // size == probe size
    let bflyt = build_synthetic_bflyt(mat);
    let written = write_bflyt(&bflyt).expect("write must succeed");
    let parsed = read_bflyt(&written).expect("re-read must succeed");
    assert_eq!(parsed.materials.len(), 1);
}

/// The misuse case the guard is designed to catch: an untrusted
/// material was loaded, a caller mutated its `texture_maps` (changing
/// `emit_size`), but never called `clear_untrusted_flag()`. The writer
/// would silently emit a file whose `flags_raw` disagrees with the
/// section bytes — exactly the kind of latent corruption the guard
/// aims to prevent. In debug builds the `debug_assert!` panics.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "flags_untrusted=true")]
fn untrusted_material_with_mutated_counts_panics_writer_in_debug() {
    let mut mat = untrusted_consistent_material(0);
    // Mutate sub-section counts WITHOUT clearing the untrusted flag.
    // emit_size now exceeds original_section_size by 4 bytes (one
    // additional TextureRef).
    mat.texture_maps.push(TextureRef {
        index: 1,
        wrap_mode_u: 0,
        wrap_mode_v: 0,
    });

    let bflyt = build_synthetic_bflyt(mat);
    let _ = write_bflyt(&bflyt);
}

/// The recovery path: after mutating an untrusted material, calling
/// `clear_untrusted_flag()` recomputes `flags_raw` and drops the
/// `original_section_size` snapshot, so subsequent writes go through
/// the trusted path with no guard tripping. The resulting file must
/// round-trip cleanly.
#[test]
fn mutated_then_cleared_material_writes_cleanly() {
    let mut mat = untrusted_consistent_material(0);
    mat.texture_maps.push(TextureRef {
        index: 1,
        wrap_mode_u: 0,
        wrap_mode_v: 0,
    });
    mat.clear_untrusted_flag();

    let bflyt = build_synthetic_bflyt(mat);
    let written = write_bflyt(&bflyt).expect("write");
    let parsed = read_bflyt(&written).expect("re-read");
    assert_eq!(parsed.materials.len(), 1);
    assert_eq!(parsed.materials[0].texture_maps.len(), 2);
    assert!(
        !parsed.materials[0].flags_untrusted,
        "round-tripped material should be in the trusted state",
    );
}
