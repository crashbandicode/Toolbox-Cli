//! Integration tests for the BFLYT writer using a synthetic minimal
//! layout. These don't require any game assets — they construct a BFLYT
//! in memory, write it, parse it back, and assert the structure round-
//! trips.
//!
//! For tests that need real game assets (Smash Ultimate `info_melee`),
//! drop them into `tests/fixtures/` (gitignored) and they'll be picked up
//! by `tests/bflyt_real_fixtures.rs` when present.

use toolbox_cli::bflyt;

#[test]
fn synthetic_minimal_layout_round_trips() {
    use bflyt::*;

    let mut bflyt = BFLYT {
        version: 0x09000000,
        layout: LayoutInfo {
            draw_centered: true,
            width: 1280.0,
            height: 720.0,
            max_parts_width: 1280.0,
            max_parts_height: 720.0,
            name: "test_layout".into(),
        },
        textures: vec!["tex_a".into(), "tex_b".into()],
        fonts: vec!["nintendo64".into()],
        materials: vec![Material {
            name: "mat_test".into(),
            flags_unknown: 0,
            black_color: Color8 { r: 0, g: 0, b: 0, a: 0 },
            white_color: Color8 { r: 255, g: 255, b: 255, a: 255 },
            flags_raw: 0, // rebuild_flags will fix this
            texture_maps: vec![TextureRef { index: 0, wrap_mode_u: 0, wrap_mode_v: 0 }],
            texture_transforms: vec![TextureTransform {
                translate: Vec2 { x: 0.0, y: 0.0 },
                rotate: 0.0,
                scale: Vec2 { x: 1.0, y: 1.0 },
            }],
            tex_coord_gens: vec![TexCoordGen::default()],
            tev_stages: vec![],
            alpha_compare: None,
            blend_mode: None,
            blend_mode_logic: None,
            indirect_param: None,
            proj_tex_gen_params: vec![],
            font_shadow_param: None,
            trailing: vec![],
            flags_untrusted: false,
        }],
        root_pane: Some(BasePane {
            kind: PaneKind::Pane,
            flag: 0x01,
            base_position: 0,
            alpha: 255,
            flag_ex: 0,
            name: "RootPane".into(),
            user_data_field: [0; 8],
            translate: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            rotate: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
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
        }),
        root_group: Some(Group {
            name: "RootGroup".into(),
            panes: vec![],
            children: vec![],
        }),
        user_data: None,
        control_data: None,
        opaque_sections: vec![],
    };

    let written = write_bflyt(&bflyt).expect("write");
    let parsed = read_bflyt(&written).expect("re-read");

    assert_eq!(parsed.version, bflyt.version);
    assert_eq!(parsed.layout.width, 1280.0);
    assert_eq!(parsed.layout.height, 720.0);
    assert_eq!(parsed.textures, bflyt.textures);
    assert_eq!(parsed.fonts, bflyt.fonts);
    assert_eq!(parsed.materials.len(), 1);
    assert_eq!(parsed.materials[0].name, "mat_test");
    assert_eq!(parsed.materials[0].texture_maps.len(), 1);
    assert!(parsed.root_pane.is_some());
    assert_eq!(parsed.root_pane.as_ref().unwrap().name, "RootPane");
}

#[test]
fn synthetic_layout_writes_then_re_reads_byte_identical_on_second_pass() {
    use bflyt::*;
    let _ = bflyt::MAT_NAME_LEN_USIZE; // ensure constant is exported

    let bflyt = BFLYT {
        version: 0x09000000,
        layout: LayoutInfo {
            draw_centered: false,
            width: 100.0,
            height: 200.0,
            max_parts_width: 0.0,
            max_parts_height: 0.0,
            name: "round_trip".into(),
        },
        textures: vec!["t".into()],
        fonts: vec![],
        materials: vec![],
        root_pane: Some(BasePane {
            kind: PaneKind::Pane,
            flag: 1,
            base_position: 0,
            alpha: 255,
            flag_ex: 0,
            name: "R".into(),
            user_data_field: [0; 8],
            translate: Default::default(),
            rotate: Default::default(),
            scale: Vec2 { x: 1.0, y: 1.0 },
            width: 100.0,
            height: 200.0,
            picture: None,
            text: None,
            window: None,
            parts: None,
            user_data: None,
            children: vec![],
            trailing: vec![],
        }),
        root_group: None,
        user_data: None,
        control_data: None,
        opaque_sections: vec![],
    };

    let pass_1 = write_bflyt(&bflyt).unwrap();
    let parsed = read_bflyt(&pass_1).unwrap();
    let pass_2 = write_bflyt(&parsed).unwrap();

    // Once a BFLYT has been through our pipeline, subsequent rewrites
    // must be byte-identical.
    assert_eq!(pass_1, pass_2);
}
