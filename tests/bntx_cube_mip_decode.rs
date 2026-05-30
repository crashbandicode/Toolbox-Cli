//! Exercises the multi-mip and cube-map decode / DDS paths that the
//! real fixtures (all single-mip 2D) don't reach. We synthesize the
//! textures by appending a 3-mip 2D texture and a 6-face / 3-mip cube to
//! a real BNTX, then:
//!   * decode mip 0/1/2 and assert the dimensions halve per level,
//!   * decode cube layer 0 and layer 5 (and a deeper mip of a middle
//!     face) and assert dimensions,
//!   * round-trip both through DDS (export -> serialize -> parse ->
//!     replace -> re-export) and assert the linear payload + metadata
//!     survive.
//!
//! Skipped when the base BNTX fixture is absent.

use std::path::{Path, PathBuf};

use nx_layout_toolbox::bntx::decode::decode_texture_image;
use nx_layout_toolbox::bntx::pipeline::{
    export_texture_dds, import_cube_png_files, import_image, replace_with_dds, ImportOptions,
};
use nx_layout_toolbox::bntx::{read_bntx, write_bntx, BntxFile};
use nx_layout_toolbox::dds::Dds;
use nx_layout_toolbox::texpipe::Bc7Quality;

const MIP2D: &str = "tex_agent_mip2d";
const CUBE: &str = "tex_agent_cube";

fn gen_png(path: &Path, w: u32, h: u32, seed: u32) {
    let mut img = image::RgbaImage::new(w, h);
    for (x, y, px) in img.enumerate_pixels_mut() {
        *px = image::Rgba([
            ((x.wrapping_mul(3)).wrapping_add(seed)) as u8,
            ((y.wrapping_mul(5)).wrapping_add(seed)) as u8,
            ((x ^ y).wrapping_add(seed)) as u8,
            (200 + (x + y) % 56) as u8,
        ]);
    }
    img.save(path).expect("save png");
}

/// Build a BNTX containing a 3-mip 2D texture and a 6-face 3-mip cube,
/// appended onto a real fixture and round-tripped through write/read.
fn build_fixture() -> Option<BntxFile> {
    let base = Path::new("tests/fixtures/bntx/info_training__Combined.bntx");
    if !base.exists() {
        return None;
    }
    let mut bntx = read_bntx(&std::fs::read(base).unwrap()).expect("parse base bntx");

    let tmp = tempfile::tempdir().expect("tempdir");
    // 3-mip 2D texture from an in-memory 64x64 image.
    let img2d = image::DynamicImage::ImageRgba8({
        let mut i = image::RgbaImage::new(64, 64);
        for (x, y, px) in i.enumerate_pixels_mut() {
            *px = image::Rgba([(x * 4) as u8, (y * 4) as u8, 128, 255]);
        }
        i
    });
    import_image(
        &mut bntx,
        MIP2D,
        &img2d,
        &ImportOptions {
            quality: Bc7Quality::UltraFast,
            mip_count: 3,
            ..Default::default()
        },
    )
    .expect("append 2d mip texture");

    // 6-face cube, 3 mips, from 64x64 face PNGs.
    let faces: [PathBuf; 6] = std::array::from_fn(|i| {
        let p = tmp.path().join(format!("face{i}.png"));
        gen_png(&p, 64, 64, (i as u32) * 40);
        p
    });
    import_cube_png_files(
        &mut bntx,
        CUBE,
        &faces,
        &ImportOptions {
            quality: Bc7Quality::UltraFast,
            mip_count: 3,
            ..Default::default()
        },
    )
    .expect("append cube texture");

    // Round-trip through the writer so we decode exactly what would land
    // on disk.
    let bytes = write_bntx(&bntx).expect("write");
    Some(read_bntx(&bytes).expect("re-read"))
}

#[test]
fn multi_mip_and_cube_decode_dimensions() {
    let Some(bntx) = build_fixture() else {
        eprintln!("skipping cube/mip decode test (base BNTX fixture absent)");
        return;
    };

    let mip2d = bntx.texture_index_by_name(MIP2D).expect("mip2d present");
    assert_eq!(bntx.textures[mip2d].mips_count, 3);
    assert_eq!(bntx.textures[mip2d].array_len, 1);
    assert_eq!(bntx.textures[mip2d].dim, 2);

    for (mip, expect) in [(0u32, 64u32), (1, 32), (2, 16)] {
        let img = decode_texture_image(&bntx, mip2d, mip, 0, true).expect("decode 2d mip");
        assert_eq!((img.width, img.height), (expect, expect), "2d mip {mip}");
        assert_eq!(img.rgba.len(), (expect * expect * 4) as usize);
    }

    let cube = bntx.texture_index_by_name(CUBE).expect("cube present");
    assert_eq!(bntx.textures[cube].array_len, 6);
    assert_eq!(bntx.textures[cube].dim, 8);
    assert_eq!(bntx.textures[cube].mips_count, 3);

    // Face 0 and face 5 at mip 0, plus a middle face at a deeper mip.
    for layer in [0u32, 5] {
        let img = decode_texture_image(&bntx, cube, 0, layer, true).expect("decode cube face");
        assert_eq!((img.width, img.height), (64, 64), "cube layer {layer} mip 0");
    }
    let deep = decode_texture_image(&bntx, cube, 2, 3, true).expect("decode cube deep mip");
    assert_eq!((deep.width, deep.height), (16, 16), "cube layer 3 mip 2");

    // Out-of-range mip/layer error cleanly (no panic).
    assert!(decode_texture_image(&bntx, cube, 3, 0, true).is_err());
    assert!(decode_texture_image(&bntx, cube, 0, 6, true).is_err());

    println!("OK: multi-mip + cube decode dimensions verified");
}

#[test]
fn multi_mip_and_cube_dds_round_trip() {
    let Some(bntx) = build_fixture() else {
        eprintln!("skipping cube/mip DDS test (base BNTX fixture absent)");
        return;
    };

    for (name, is_cube, array) in [(MIP2D, false, 1u32), (CUBE, true, 6)] {
        let d1 = export_texture_dds(&bntx, name).expect("export dds");
        assert_eq!(d1.is_cube, is_cube, "{name} is_cube");
        assert_eq!(d1.array_count, array, "{name} array_count");
        assert_eq!(d1.mip_count, 3, "{name} mip_count");
        assert_eq!((d1.width, d1.height), (64, 64));

        // DDS serialize -> parse is lossless.
        let d2 = Dds::read(&d1.write()).expect("re-read dds");
        assert_eq!(d2, d1, "{name} DDS serialize round-trip");

        // Replace from the parsed DDS and re-export: the linear payload
        // and metadata must survive the swizzle round-trip.
        let mut clone = bntx.clone();
        replace_with_dds(&mut clone, name, &d2).expect("replace_with_dds");
        let d3 = export_texture_dds(&clone, name).expect("re-export");
        assert_eq!(d3.data, d1.data, "{name} linear payload changed");
        assert_eq!(d3.format, d1.format);
        assert_eq!(d3.mip_count, d1.mip_count);
        assert_eq!(d3.array_count, d1.array_count);
    }

    println!("OK: multi-mip + cube DDS round-trip verified");
}
