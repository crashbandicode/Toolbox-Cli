//! Tests for `bntx-replace-png`'s core invariant: splicing a same-size
//! pixel-data slice over an existing texture must (a) preserve the BNTX
//! file's overall byte length, (b) leave every other texture's pixel
//! data and metadata byte-identical, and (c) be parseable back to the
//! same structure, with the replaced texture's pixel data reflecting
//! the new bytes.
//!
//! The verb itself lives behind a `clap` CLI, so we exercise the
//! underlying library mutation directly (read_bntx → splice into
//! `brtd.data` → write_bntx). That's the same operation the verb
//! performs after validation.
//!
//! Skipped when `tests/fixtures/bntx/` is absent (CI doesn't ship real
//! game assets).

use std::fs;
use std::path::Path;

use toolbox_cli::bntx::{read_bntx, write_bntx, TextureFormat};
use toolbox_cli::texpipe::{compress_image_bc7, Bc7Quality};

#[test]
fn splice_same_size_preserves_layout_and_other_textures() {
    let bntx_path = Path::new("tests/fixtures/bntx/info_melee_original__Combined.bntx");
    if !bntx_path.exists() {
        eprintln!(
            "skipping bntx_replace_in_place test (no fixture at {})",
            bntx_path.display()
        );
        return;
    }
    let bytes = fs::read(bntx_path).expect("read fixture");
    let parsed = read_bntx(&bytes).expect("parse fixture");

    // Find a BC7 2D single-mip texture; that's the SGPO/face-button
    // shape `bntx-replace-png` is designed for and the only kind we can
    // re-encode with `compress_image_bc7` (the single-mip path).
    let target = parsed
        .textures
        .iter()
        .enumerate()
        .find(|(_, t)| {
            matches!(
                t.format,
                TextureFormat::Bc7Unorm | TextureFormat::Bc7UnormSrgb
            ) && t.dim == 2
                && t.mips_count == 1
                && t.array_len == 1
        })
        .expect("fixture must contain at least one BC7 2D single-mip texture");
    let target_idx = target.0;
    let exp_width = target.1.width;
    let exp_height = target.1.height;
    let exp_image_size = target.1.image_size as usize;
    let exp_data_offset = target.1.data_offset_in_brtd;
    let target_name = target.1.name(&parsed).to_string();

    // Generate a procedural image of identical dimensions. The xor
    // pattern guarantees the pixels differ from whatever is in the
    // fixture, so we can detect a faulty splice that wrote the wrong
    // bytes (or wrote them to the wrong offset).
    let mut img = image::RgbaImage::new(exp_width, exp_height);
    for (x, y, px) in img.enumerate_pixels_mut() {
        *px = image::Rgba([(x ^ y) as u8, ((x + y) & 0xFF) as u8, 0x40, 0xFF]);
    }
    let dyn_img = image::DynamicImage::ImageRgba8(img);
    let compressed =
        compress_image_bc7(&dyn_img, Bc7Quality::UltraFast).expect("compress test image");

    // The splice depends on this matching exactly; if it doesn't,
    // `bntx-replace-png` would refuse to splice. Asserting it here
    // ensures the test fixture is in a shape our pipeline can encode.
    assert_eq!(
        compressed.swizzled_data.len(),
        exp_image_size,
        "swizzled output ({}) must match BRTI image_size ({}) for in-place splice",
        compressed.swizzled_data.len(),
        exp_image_size,
    );

    // Capture each non-target texture's pre-splice pixel bytes so we
    // can verify they're untouched after the write.
    let other_pixels: Vec<(usize, Vec<u8>)> = parsed
        .textures
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != target_idx)
        .map(|(i, t)| (i, t.pixel_data(&parsed.brtd).to_vec()))
        .collect();

    // Apply the replacement (the operation `bntx-replace-png` performs
    // after passing all its shape validations).
    let mut modified = parsed.clone();
    modified.brtd.data[exp_data_offset..exp_data_offset + exp_image_size]
        .copy_from_slice(&compressed.swizzled_data);

    let written = write_bntx(&modified).expect("write modified BNTX");

    // (a) File size invariant.
    assert_eq!(
        written.len(),
        bytes.len(),
        "in-place replacement must preserve total file size (orig={}, modified={})",
        bytes.len(),
        written.len(),
    );

    // (c) Re-parse and confirm structure + replaced-texture data.
    let reparsed = read_bntx(&written).expect("re-parse modified BNTX");
    assert_eq!(reparsed.textures.len(), parsed.textures.len());
    let new_tex = &reparsed.textures[target_idx];
    assert_eq!(new_tex.name(&reparsed), target_name);
    assert_eq!(new_tex.width, exp_width);
    assert_eq!(new_tex.height, exp_height);
    assert_eq!(new_tex.image_size as usize, exp_image_size);
    assert_eq!(new_tex.data_offset_in_brtd, exp_data_offset);
    assert_eq!(
        new_tex.pixel_data(&reparsed.brtd),
        &compressed.swizzled_data[..],
        "replaced texture's pixel data must match the spliced bytes"
    );

    // (b) Other textures' pixel data must be byte-identical after the
    // splice (and the round-trip).
    for (i, orig_pixels) in &other_pixels {
        let new_pixels = reparsed.textures[*i].pixel_data(&reparsed.brtd);
        assert_eq!(
            new_pixels,
            &orig_pixels[..],
            "non-target texture #{} (name='{}') pixel data must not change",
            i,
            reparsed.textures[*i].name(&reparsed)
        );
    }
}

/// Sanity check: splicing a texture's *current* bytes back into itself
/// must produce a byte-identical file. This proves there's no implicit
/// re-canonicalization in the writer that would silently churn bytes
/// even when no real change was made.
#[test]
fn splice_identity_is_byte_identical() {
    let bntx_path = Path::new("tests/fixtures/bntx/info_melee_original__Combined.bntx");
    if !bntx_path.exists() {
        eprintln!(
            "skipping bntx_replace_in_place identity test (no fixture at {})",
            bntx_path.display()
        );
        return;
    }
    let bytes = fs::read(bntx_path).expect("read fixture");
    let parsed = read_bntx(&bytes).expect("parse fixture");

    // Find any texture; we don't need a BC7 one because we're not
    // re-encoding — just splicing the existing data over itself.
    let target = parsed
        .textures
        .first()
        .expect("fixture must contain at least one texture");
    let exp_size = target.image_size as usize;
    let exp_off = target.data_offset_in_brtd;

    let pixels = target.pixel_data(&parsed.brtd).to_vec();

    let mut modified = parsed.clone();
    modified.brtd.data[exp_off..exp_off + exp_size].copy_from_slice(&pixels);

    let written = write_bntx(&modified).expect("write");

    assert_eq!(
        written, bytes,
        "splicing a texture's own bytes back must yield a byte-identical file"
    );
}
