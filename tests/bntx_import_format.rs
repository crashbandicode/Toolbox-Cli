//! PNG-import surface-format selection: BC7 (default) vs uncompressed
//! RGBA8 (sharper small text/edges for SGPO skins).
//!
//! Fixture-gated (skips when the BNTX / archive fixtures are absent — CI
//! ships no game assets). Covers:
//! 1/2. `Rgba8` / `Rgba8Srgb` append as `R8G8B8A8_UNORM` / `_SRGB`.
//! 3.   The result reads back through `read_bntx`.
//! 4.   RGBA8 dimensions match the source image exactly (no block padding).
//! 5.   The default BC7 path is unchanged (`BC7_UNORM`, padded to 4).
//! 6.   `apply_manifest_to_arc` with `texture_format = Rgba8` imports the
//!      manifest PNGs as RGBA8 and the panes/materials/textures validate.

use std::path::Path;

use image::{DynamicImage, RgbaImage};
use nx_layout_toolbox::bntx::pipeline::{import_image, ImportOptions, ImportTextureFormat};
use nx_layout_toolbox::bntx::{read_bntx, write_bntx, TextureFormat};
use nx_layout_toolbox::layout::{apply_manifest_to_arc, ApplyOptions};
use nx_layout_toolbox::manifest::{SkinElement, SkinManifest};
use nx_layout_toolbox::sarc::read_arc;
use nx_layout_toolbox::texpipe::Bc7Quality;

const BASE_BNTX: &str = "tests/fixtures/bntx/info_training__Combined.bntx";
const ARC: &str = "tests/fixtures/archives/info_melee_original.layout.arc";

fn gen_image(w: u32, h: u32) -> DynamicImage {
    let mut img = RgbaImage::new(w, h);
    for (x, y, px) in img.enumerate_pixels_mut() {
        *px = image::Rgba([(x * 7) as u8, (y * 11) as u8, 0x40, ((x + y) * 3) as u8]);
    }
    DynamicImage::ImageRgba8(img)
}

/// Import a generated image with `tf`/`srgb`, write the BNTX, re-read it
/// (exercising the read-back path), and return the appended texture's
/// (format, width, height).
fn import_and_reread(
    tf: ImportTextureFormat,
    srgb: bool,
    name: &str,
    w: u32,
    h: u32,
) -> (TextureFormat, u32, u32) {
    let bytes = std::fs::read(BASE_BNTX).expect("read base bntx");
    let mut bntx = read_bntx(&bytes).expect("parse base bntx");
    let opts = ImportOptions {
        quality: Bc7Quality::UltraFast,
        srgb,
        align: None,
        mip_count: 1,
        texture_format: tf,
    };
    import_image(&mut bntx, name, &gen_image(w, h), &opts).expect("import image");
    let written = write_bntx(&bntx).expect("write bntx");
    let re = read_bntx(&written).expect("re-read written bntx");
    let idx = re
        .texture_index_by_name(name)
        .expect("imported texture present after write -> read");
    let t = &re.textures[idx];
    (t.format, t.width, t.height)
}

#[test]
fn import_texture_format_selection() {
    if !Path::new(BASE_BNTX).exists() {
        eprintln!("skipping import-format test (no fixture at {BASE_BNTX})");
        return;
    }

    // A deliberately non-4-aligned source so RGBA8's exact-dimension
    // behavior is distinguishable from BC7's block padding.
    let (sw, sh) = (30u32, 18u32);

    // (1) rgba8 -> R8G8B8A8_UNORM, (4) exact source dims, (3) re-readable.
    let (fmt, w, h) = import_and_reread(ImportTextureFormat::Rgba8, false, "sgpo_rgba8", sw, sh);
    assert_eq!(fmt, TextureFormat::R8G8B8A8Unorm, "rgba8 format");
    assert_eq!((w, h), (sw, sh), "rgba8 keeps exact source dims");

    // (2) rgba8-srgb -> R8G8B8A8_UNORM_SRGB.
    let (fmt, w, h) =
        import_and_reread(ImportTextureFormat::Rgba8Srgb, false, "sgpo_rgba8_srgb", sw, sh);
    assert_eq!(fmt, TextureFormat::R8G8B8A8UnormSrgb, "rgba8-srgb format");
    assert_eq!((w, h), (sw, sh), "rgba8-srgb keeps exact source dims");

    // (5) default BC7 unchanged: BC7_UNORM, dims padded up to a 4-grid.
    let (fmt, w, h) = import_and_reread(ImportTextureFormat::Bc7, false, "sgpo_bc7", sw, sh);
    assert_eq!(fmt, TextureFormat::Bc7Unorm, "default BC7 format");
    assert_eq!((w, h), (32, 20), "BC7 pads to the 4x4 block grid");

    // BC7 + srgb flag -> BC7_UNORM_SRGB (existing behavior).
    let (fmt, _, _) = import_and_reread(ImportTextureFormat::Bc7, true, "sgpo_bc7_srgb", sw, sh);
    assert_eq!(fmt, TextureFormat::Bc7UnormSrgb, "BC7 + srgb");

    println!("OK: import formats rgba8 / rgba8-srgb / bc7(+srgb) verified end-to-end");
}

fn element(pane: &str, image: &str) -> SkinElement {
    SkinElement {
        control_id: "A".into(),
        pane_name: pane.to_string(),
        image_filename: image.to_string(),
        material_name: format!("mat_{pane}"),
        base_x: 120.0,
        base_y: -40.0,
        width: 64.0,
        height: 64.0,
        released_alpha: 200,
        pressed_alpha: 255,
        released_scale: 1.0,
        pressed_scale: 1.05,
    }
}

#[test]
fn apply_manifest_rgba8_imports_and_validates() {
    if !Path::new(ARC).exists() {
        eprintln!("skipping apply-manifest rgba8 test (no fixture at {ARC})");
        return;
    }
    let arc_bytes = std::fs::read(ARC).expect("read arc fixture");

    let skin = tempfile::tempdir().expect("temp skin dir");
    let png_path = skin.path().join("btn.png");
    gen_image(64, 64)
        .save(&png_path)
        .expect("write test png");

    let element = element("sgpo_rgba8_pane", "btn.png");
    let texture_name = element.texture_name();
    let manifest = SkinManifest {
        schema_version: 1,
        skin_name: "rgba8_import_test".into(),
        root_pane_name: "RootPane".into(),
        expected_layout_flavor: String::new(),
        elements: vec![element],
    };

    let opts = ApplyOptions {
        quality: Bc7Quality::UltraFast,
        texture_format: ImportTextureFormat::Rgba8,
        ..Default::default()
    };

    let (out_arc, report) = apply_manifest_to_arc(&arc_bytes, &manifest, skin.path(), &opts, false)
        .expect("apply manifest (rgba8)");
    assert_eq!(report.applied, 1, "the element should apply");
    assert!(
        report.validation.all_passed(),
        "rgba8 apply failed validation: {:?}",
        report
            .validation
            .results
            .iter()
            .filter(|r| !r.ok)
            .collect::<Vec<_>>()
    );

    // The appended texture must be uncompressed RGBA8, not BC7.
    let out = read_arc(&out_arc).expect("parse output arc");
    let bntx_idx = out.position(&opts.bntx_rel).expect("bntx in output");
    let bntx = read_bntx(&out.files[bntx_idx].data).expect("parse output bntx");
    let idx = bntx
        .texture_index_by_name(&texture_name)
        .unwrap_or_else(|| panic!("output BNTX missing texture {texture_name}"));
    assert_eq!(
        bntx.textures[idx].format,
        TextureFormat::R8G8B8A8Unorm,
        "manifest import should store RGBA8 when --texture-format rgba8"
    );

    println!("OK: layout apply with texture_format=rgba8 imported {texture_name} as R8G8B8A8 + validated");
}
