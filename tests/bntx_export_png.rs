//! BNTX -> PNG export coverage over `tests/fixtures/bntx/`.
//!
//! Verifies that every texture in every fixture deswizzles + decodes
//! (mip 0, layer 0) to an RGBA image whose dimensions match the BNTX
//! metadata, that the count of successfully-decoded textures equals the
//! texture count, that the fixture corpus exercises every BC format the
//! parser accepts, and that the channel-swizzle is actually applied
//! (BC4 alpha masks use `One,One,One,Red`, so their RGB must come out
//! fully white).
//!
//! Skipped when `tests/fixtures/bntx/` is absent (CI ships no game assets).

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use nx_layout_toolbox::bntx::decode::decode_texture_image;
use nx_layout_toolbox::bntx::read_bntx;

#[test]
fn every_texture_in_every_fixture_decodes_with_correct_dimensions() {
    let dir = Path::new("tests/fixtures/bntx");
    if !dir.exists() {
        eprintln!(
            "skipping BNTX export test (drop BNTXs into {} to enable)",
            dir.display()
        );
        return;
    }

    let mut formats_seen: BTreeSet<&'static str> = BTreeSet::new();
    let mut total_textures = 0usize;
    let mut total_decoded = 0usize;
    let mut fixtures = 0usize;
    let mut white_mask_checked = 0usize;

    for entry in fs::read_dir(dir).expect("read fixtures dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|s| s.to_str()) != Some("bntx") {
            continue;
        }
        let bytes = fs::read(&path).expect("read fixture");
        let bntx = read_bntx(&bytes).unwrap_or_else(|e| panic!("{}: parse: {e}", path.display()));
        fixtures += 1;

        for (i, tex) in bntx.textures.iter().enumerate() {
            total_textures += 1;
            formats_seen.insert(tex.format.name());

            let img = decode_texture_image(&bntx, i, 0, 0, true).unwrap_or_else(|e| {
                panic!(
                    "{}: texture '{}' (#{i}, {}) failed to decode: {e}",
                    path.display(),
                    tex.name(&bntx),
                    tex.format.name()
                )
            });

            assert_eq!(
                (img.width, img.height),
                (tex.width, tex.height),
                "{}: '{}' decoded dims {}x{} != metadata {}x{}",
                path.display(),
                tex.name(&bntx),
                img.width,
                img.height,
                tex.width,
                tex.height
            );
            assert_eq!(
                img.rgba.len(),
                (tex.width * tex.height * 4) as usize,
                "{}: '{}' RGBA byte count wrong",
                path.display(),
                tex.name(&bntx)
            );
            total_decoded += 1;

            // Channel-swizzle sanity: a texture whose R,G,B channels all
            // source the constant "One" (byte value 1) must decode to a
            // fully-white RGB regardless of the underlying block data.
            // BC4 alpha masks in these fixtures use `One,One,One,Red`.
            let cs = bntx.channel_swizzle(tex);
            if cs[0] == 1 && cs[1] == 1 && cs[2] == 1 {
                for px in img.rgba.chunks_exact(4) {
                    assert_eq!(
                        (px[0], px[1], px[2]),
                        (255, 255, 255),
                        "{}: '{}' has One,One,One RGB swizzle but RGB != white",
                        path.display(),
                        tex.name(&bntx)
                    );
                }
                white_mask_checked += 1;
            }
        }
    }

    assert!(fixtures > 0, "no BNTX fixtures found in {}", dir.display());
    assert_eq!(
        total_decoded, total_textures,
        "not every texture decoded ({total_decoded}/{total_textures})"
    );

    // The fixture corpus must exercise every BC format we claim to
    // support exporting. (BC1/BC4/BC5/BC7 appear across the game UIs.)
    for required in ["BC1_UNORM_SRGB", "BC4_UNORM", "BC5_UNORM", "BC7_UNORM_SRGB"] {
        assert!(
            formats_seen.contains(required),
            "expected fixture corpus to contain a {required} texture; saw {formats_seen:?}"
        );
    }
    assert!(
        white_mask_checked > 0,
        "expected at least one One,One,One alpha-mask texture in the corpus"
    );

    println!(
        "OK: decoded {total_decoded} textures across {fixtures} fixtures; \
         formats={formats_seen:?}; white-mask textures verified={white_mask_checked}"
    );
}
