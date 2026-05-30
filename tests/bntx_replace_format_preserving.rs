//! Format-preserving `bntx-replace-png`: re-encoding a source image over
//! an existing texture must keep the texture's *existing* surface format
//! (BC1/BC4/BC5/BC7/...), dimensions, mip count, and byte layout, leave
//! every other texture untouched, and preserve the overall file size.
//!
//! This walks `tests/fixtures/bntx/`, and for each surface format present
//! in the corpus picks a 2D single-mip texture of that format and runs a
//! real `replace_texture` through `write_bntx` → re-read, asserting the
//! invariants. It requires the four formats the game UIs actually use
//! (BC1/BC4/BC5/BC7) to each be exercised.
//!
//! Skipped when `tests/fixtures/bntx/` is absent.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use nx_layout_toolbox::bntx::pipeline::{replace_texture, ReplaceSource};
use nx_layout_toolbox::bntx::{read_bntx, write_bntx};
use nx_layout_toolbox::texpipe::Bc7Quality;

/// Build a high-variance RGBA image so the re-encode differs from the
/// original art in *every* channel (covers BC4 alpha-mask and BC5
/// grayscale+alpha textures whose meaningful data isn't in R/G/B).
fn procedural_image(width: u32, height: u32) -> image::DynamicImage {
    let mut img = image::RgbaImage::new(width, height);
    for (x, y, px) in img.enumerate_pixels_mut() {
        *px = image::Rgba([
            ((x.wrapping_mul(7)) ^ (y.wrapping_mul(5))) as u8,
            (x & 0xFF) as u8,
            (y & 0xFF) as u8,
            ((x.wrapping_add(y)).wrapping_mul(3) & 0xFF) as u8,
        ]);
    }
    image::DynamicImage::ImageRgba8(img)
}

#[test]
fn replace_preserves_format_for_each_format_in_corpus() {
    let dir = Path::new("tests/fixtures/bntx");
    if !dir.exists() {
        eprintln!(
            "skipping format-preserving replace test (no fixtures at {})",
            dir.display()
        );
        return;
    }

    let mut covered: BTreeSet<&'static str> = BTreeSet::new();

    // Deterministic fixture order.
    let mut paths: Vec<_> = fs::read_dir(dir)
        .expect("read fixtures dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("bntx"))
        .collect();
    paths.sort();

    for path in &paths {
        let bytes = fs::read(path).expect("read fixture");
        let parsed = read_bntx(&bytes).unwrap_or_else(|e| panic!("{}: parse: {e}", path.display()));

        for (idx, tex) in parsed.textures.iter().enumerate() {
            // Only 2D single-mip single-layer textures; that's the shape
            // an in-place same-format splice is guaranteed to match.
            if tex.dim != 2 || tex.mips_count != 1 || tex.array_len != 1 {
                continue;
            }
            let fmt_name = tex.format.name();
            if covered.contains(fmt_name) {
                continue;
            }

            let name = tex.name(&parsed).to_string();
            let exp_format = tex.format;
            let exp_w = tex.width;
            let exp_h = tex.height;
            let exp_image_size = tex.image_size as usize;
            let exp_offset = tex.data_offset_in_brtd;
            let orig_pixels = tex.pixel_data(&parsed.brtd).to_vec();

            // Snapshot all other textures' pixel data.
            let others: Vec<(usize, Vec<u8>)> = parsed
                .textures
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != idx)
                .map(|(i, t)| (i, t.pixel_data(&parsed.brtd).to_vec()))
                .collect();

            let img = procedural_image(exp_w, exp_h);

            let mut modified = parsed.clone();
            match replace_texture(&mut modified, &name, ReplaceSource::Image(&img), Bc7Quality::UltraFast) {
                Ok(()) => {}
                Err(e) => {
                    // A padding/size mismatch on this particular texture:
                    // move on and let another texture of the same format
                    // (or another fixture) satisfy coverage.
                    eprintln!(
                        "  note: {} '{name}' ({fmt_name}, {exp_w}x{exp_h}) not replaceable: {e}",
                        path.display()
                    );
                    continue;
                }
            }

            let written = write_bntx(&modified).expect("write modified BNTX");
            assert_eq!(
                written.len(),
                bytes.len(),
                "{}: same-format replace changed file size",
                path.display()
            );

            let reparsed = read_bntx(&written).expect("re-parse modified BNTX");
            let new_tex = &reparsed.textures[idx];
            assert_eq!(new_tex.name(&reparsed), name);
            assert_eq!(new_tex.format, exp_format, "format must be preserved");
            assert_eq!((new_tex.width, new_tex.height), (exp_w, exp_h));
            assert_eq!(new_tex.mips_count, 1);
            assert_eq!(new_tex.image_size as usize, exp_image_size);
            assert_eq!(new_tex.data_offset_in_brtd, exp_offset);

            let new_pixels = new_tex.pixel_data(&reparsed.brtd);
            assert_eq!(
                new_pixels.len(),
                orig_pixels.len(),
                "replaced data length must equal image_size"
            );
            assert_ne!(
                new_pixels, &orig_pixels[..],
                "{}: '{name}' ({fmt_name}) bytes should change after replacing with a new image",
                path.display()
            );

            for (i, orig) in &others {
                assert_eq!(
                    reparsed.textures[*i].pixel_data(&reparsed.brtd),
                    &orig[..],
                    "{}: non-target texture #{i} changed after replace",
                    path.display()
                );
            }

            covered.insert(fmt_name);
            println!("OK: replaced one {fmt_name} texture '{name}' ({exp_w}x{exp_h}) in {}", path.display());
        }
    }

    for required in ["BC1_UNORM_SRGB", "BC4_UNORM", "BC5_UNORM", "BC7_UNORM_SRGB"] {
        assert!(
            covered.contains(required),
            "expected a replaceable {required} texture in the corpus; covered={covered:?}"
        );
    }
    println!("OK: format-preserving replace verified for {covered:?}");
}
