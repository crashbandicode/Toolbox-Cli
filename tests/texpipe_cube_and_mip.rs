//! Round-trip tests for the multi-mip 2D and cube-map paths in the
//! texpipe.
//!
//! `tests/texpipe_round_trip.rs` already covers single-mip 2D BC7. This
//! file extends the same round-trip strategy
//! (encode → swizzle → deswizzle → decode → compare) to the two
//! more-complex pipeline paths that previously only had CLI smoke tests:
//!
//! - `compress_image_bc7_with_mips` — concatenated per-mip BC7 blocks
//!   passed through `swizzle_surface(mip_count = N)`, then deswizzled
//!   and decoded.
//! - `compress_cube_bc7` — six face mip chains concatenated, passed
//!   through `swizzle_surface(layer_count = 6)`. Each face is
//!   extracted and decoded individually.
//!
//! The tests confirm mip 0 (the unresized source level) round-trips
//! within BC7's lossy budget. Higher mips are resized through Lanczos3
//! before encoding, so we don't have a cheap reference to compare them
//! against; the relevant invariant is that the layout is correct
//! enough for mip 0 to come back. Wrong layout would make the mip-0
//! decode produce garbage, blowing the budget by orders of magnitude.
//!
//! Skipped when the PNG fixtures are absent.

use std::path::Path;

use image::DynamicImage;
use tegra_swizzle::surface::{deswizzle_surface, BlockDim};
use tegra_swizzle::BlockHeight;
use toolbox_cli::texpipe::{
    compress_cube_bc7, compress_image_bc7_with_mips, Bc7Quality,
};

/// Loose budgets for BC7 `Fast` quality with alpha — same calibration
/// as the single-mip round-trip test. Mip 0 is the unresized source
/// so the budget here is the same as for a non-mipped encode.
const MAX_MEAN_ABS_ERROR: f64 = 12.0;
const MAX_PEAK_ABS_ERROR: u8 = 80;

/// Map `block_height_log2` back into a `BlockHeight`. Inverse of
/// `texpipe::compress_image_bc7_with_mips`'s match arm.
fn block_height_from_log2(log2: u32) -> Option<BlockHeight> {
    Some(match log2 {
        0 => BlockHeight::One,
        1 => BlockHeight::Two,
        2 => BlockHeight::Four,
        3 => BlockHeight::Eight,
        4 => BlockHeight::Sixteen,
        5 => BlockHeight::ThirtyTwo,
        _ => return None,
    })
}

/// Decode `linear_bc7` (BC7 blocks for a single image of `w` x `h`
/// pixels) into an RGBA8 buffer, swapping `texture2ddecoder`'s BGRA32
/// output to RGBA8 in the process. Both width and height must be
/// multiples of 4.
fn decode_bc7_to_rgba(linear_bc7: &[u8], w: u32, h: u32) -> Vec<u8> {
    let pixel_count = (w * h) as usize;
    let mut bgra = vec![0u32; pixel_count];
    texture2ddecoder::decode_bc7(linear_bc7, w as usize, h as usize, &mut bgra)
        .expect("decode_bc7");
    let mut rgba = vec![0u8; pixel_count * 4];
    for (i, packed) in bgra.iter().enumerate() {
        let b = (*packed & 0xFF) as u8;
        let g = ((*packed >> 8) & 0xFF) as u8;
        let r = ((*packed >> 16) & 0xFF) as u8;
        let a = ((*packed >> 24) & 0xFF) as u8;
        rgba[i * 4..i * 4 + 4].copy_from_slice(&[r, g, b, a]);
    }
    rgba
}

fn per_channel_error(a: &[u8], b: &[u8]) -> (f64, u8) {
    assert_eq!(a.len(), b.len());
    let mut sum: u64 = 0;
    let mut peak: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        let d = x.abs_diff(*y);
        sum += d as u64;
        if d > peak {
            peak = d;
        }
    }
    (sum as f64 / a.len() as f64, peak)
}

fn assert_within_budget(label: &str, mean: f64, peak: u8) {
    assert!(
        mean <= MAX_MEAN_ABS_ERROR && peak <= MAX_PEAK_ABS_ERROR,
        "{label}: error budget exceeded — mean={mean:.2} (max {MAX_MEAN_ABS_ERROR}), \
         peak={peak} (max {MAX_PEAK_ABS_ERROR}). Suggests a layout / mip / face indexing bug.",
    );
}

/// Compute the byte size of a single BC7 mip level given its (padded)
/// pixel dimensions. The texpipe rounds level dimensions up to the
/// next 4-pixel multiple before encoding, so width and height should
/// already be multiples of 4 here.
fn bc7_mip_size_bytes(w: u32, h: u32) -> usize {
    let blocks_w = w.div_ceil(4);
    let blocks_h = h.div_ceil(4);
    (blocks_w * blocks_h * 16) as usize
}

/// For a level `l` of a base `(width, height)` 2D image as the texpipe
/// builds it (clamped to >=4 in each dimension), return the level's
/// padded BC7 dimensions.
fn texpipe_mip_dims(width: u32, height: u32, level: u32) -> (u32, u32) {
    let lw = (width >> level).max(4);
    let lh = (height >> level).max(4);
    let lvl_w = (lw + 3) & !3;
    let lvl_h = (lh + 3) & !3;
    (lvl_w, lvl_h)
}

fn load_png(name: &str) -> Option<DynamicImage> {
    let path = Path::new("tests/fixtures/png-test-images").join(name);
    if !path.exists() {
        return None;
    }
    Some(image::open(&path).expect("open png"))
}

/// Multi-mip 2D round-trip. Encodes the 64×64 fixture with a 4-mip
/// chain, deswizzles, slices out mip 0 and mip 1, decodes mip 0 and
/// asserts within budget. Mip 1's BC7 bytes are decoded just to prove
/// the bytes are valid BC7 (not asserted against pixels because the
/// resize reference is non-trivial).
#[test]
fn multi_mip_2d_round_trip_64x64() {
    let Some(img) = load_png("rgba_alpha_64x64.png") else {
        eprintln!("skipping multi_mip_2d test (no rgba_alpha_64x64.png)");
        return;
    };
    let mip_count = 4u32;
    let compressed =
        compress_image_bc7_with_mips(&img, Bc7Quality::Fast, mip_count).expect("encode");
    assert_eq!(compressed.mip_count, mip_count);
    assert_eq!(compressed.array_count, 1);

    // Sanity: image_size == sum of per-mip BC7 sizes. Mismatch would
    // mean texpipe's mip-chain build disagrees with our layout
    // accounting (and thus the deswizzle output offsets we use below).
    let expected_linear: usize = (0..mip_count)
        .map(|l| {
            let (w, h) = texpipe_mip_dims(compressed.width, compressed.height, l);
            bc7_mip_size_bytes(w, h)
        })
        .sum();
    assert_eq!(
        compressed.image_size as usize, expected_linear,
        "image_size {} disagrees with per-mip BC7 size sum {}",
        compressed.image_size, expected_linear
    );

    let block_height =
        block_height_from_log2(compressed.block_height_log2).expect("valid block_height");
    let linear = deswizzle_surface(
        compressed.width,
        compressed.height,
        1,
        &compressed.swizzled_data,
        BlockDim::block_4x4(),
        Some(block_height),
        16,
        mip_count,
        1,
    )
    .expect("deswizzle multi-mip");

    assert_eq!(
        linear.len(),
        expected_linear,
        "deswizzled size {} != expected linear sum {} for {}-mip surface",
        linear.len(),
        expected_linear,
        mip_count,
    );

    // Slice mip 0 and decode.
    let (m0_w, m0_h) = texpipe_mip_dims(compressed.width, compressed.height, 0);
    let m0_size = bc7_mip_size_bytes(m0_w, m0_h);
    let m0_decoded = decode_bc7_to_rgba(&linear[..m0_size], m0_w, m0_h);

    let src_rgba = img.to_rgba8();
    assert_eq!(src_rgba.dimensions(), (m0_w, m0_h));
    let (mean, peak) = per_channel_error(src_rgba.as_raw(), &m0_decoded);
    assert_within_budget("multi-mip 2D / mip 0", mean, peak);

    // Slice mip 1 and run it through the decoder. We don't compare
    // against an expected image (the texpipe runs Lanczos3 before
    // encoding, so the reference would require us to mirror that), but
    // the decode itself returning Ok proves the bytes are well-formed
    // BC7 — i.e., the deswizzle didn't slice into a wrong-sized region.
    let (m1_w, m1_h) = texpipe_mip_dims(compressed.width, compressed.height, 1);
    let m1_size = bc7_mip_size_bytes(m1_w, m1_h);
    let _m1 = decode_bc7_to_rgba(&linear[m0_size..m0_size + m1_size], m1_w, m1_h);
}

/// Cube-map round-trip with 1 mip per face. Uses the same 64×64
/// fixture for all 6 faces; each face's mip 0 must round-trip to the
/// source within budget. Wrong face indexing (e.g., index treated as
/// mip count) would make at least one face decode produce garbage.
#[test]
fn cube_map_single_mip_round_trip_64x64() {
    let png = Path::new("tests/fixtures/png-test-images/rgba_alpha_64x64.png");
    if !png.exists() {
        eprintln!("skipping cube_map test (no rgba_alpha_64x64.png)");
        return;
    }
    let face_paths: [std::path::PathBuf; 6] = [
        png.into(), png.into(), png.into(), png.into(), png.into(), png.into(),
    ];
    let mip_count = 1u32;
    let compressed = compress_cube_bc7(&face_paths, Bc7Quality::Fast, mip_count).expect("encode");
    assert_eq!(compressed.mip_count, mip_count);
    assert_eq!(compressed.array_count, 6);
    assert_eq!(compressed.width, compressed.height);

    let face_size = bc7_mip_size_bytes(compressed.width, compressed.height);
    assert_eq!(
        compressed.image_size as usize,
        face_size * 6,
        "cube image_size disagrees with 6 * face_size",
    );

    let block_height =
        block_height_from_log2(compressed.block_height_log2).expect("valid block_height");
    let linear = deswizzle_surface(
        compressed.width,
        compressed.height,
        1,
        &compressed.swizzled_data,
        BlockDim::block_4x4(),
        Some(block_height),
        16,
        mip_count,
        6,
    )
    .expect("deswizzle cube");
    assert_eq!(linear.len(), face_size * 6);

    let src_rgba = image::open(png).expect("open").to_rgba8();
    for face_idx in 0..6 {
        let start = face_idx * face_size;
        let face_bytes = &linear[start..start + face_size];
        let decoded =
            decode_bc7_to_rgba(face_bytes, compressed.width, compressed.height);
        let (mean, peak) = per_channel_error(src_rgba.as_raw(), &decoded);
        assert_within_budget(&format!("cube face {face_idx}"), mean, peak);
    }
}

/// Cube-map + multi-mip round-trip. Combines both axes of complexity
/// so a layout bug in either dimension surfaces here. Tests that face
/// 0 mip 0 round-trips to the source within budget (no Lanczos
/// reference needed for that level).
#[test]
fn cube_map_multi_mip_round_trip_64x64() {
    let png = Path::new("tests/fixtures/png-test-images/rgba_alpha_64x64.png");
    if !png.exists() {
        eprintln!("skipping cube_map_multi_mip test (no rgba_alpha_64x64.png)");
        return;
    }
    let face_paths: [std::path::PathBuf; 6] = [
        png.into(), png.into(), png.into(), png.into(), png.into(), png.into(),
    ];
    let mip_count = 3u32;
    let compressed = compress_cube_bc7(&face_paths, Bc7Quality::Fast, mip_count).expect("encode");
    assert_eq!(compressed.mip_count, mip_count);
    assert_eq!(compressed.array_count, 6);

    // Linear layout: face 0 mip 0..N-1, face 1 mip 0..N-1, ..., face 5.
    // Per-face size = sum of per-mip BC7 sizes; total = 6 * per_face.
    let per_face: usize = (0..mip_count)
        .map(|l| {
            let (w, h) = texpipe_mip_dims(compressed.width, compressed.height, l);
            bc7_mip_size_bytes(w, h)
        })
        .sum();
    assert_eq!(
        compressed.image_size as usize,
        per_face * 6,
        "cube+mip image_size {} != 6 * per_face {}",
        compressed.image_size,
        per_face * 6,
    );

    let block_height =
        block_height_from_log2(compressed.block_height_log2).expect("valid block_height");
    let linear = deswizzle_surface(
        compressed.width,
        compressed.height,
        1,
        &compressed.swizzled_data,
        BlockDim::block_4x4(),
        Some(block_height),
        16,
        mip_count,
        6,
    )
    .expect("deswizzle cube+mip");
    assert_eq!(linear.len(), per_face * 6);

    // Decode face 0 mip 0 (the only level we have a cheap reference
    // for) and compare against the source.
    let (m0_w, m0_h) = texpipe_mip_dims(compressed.width, compressed.height, 0);
    let m0_size = bc7_mip_size_bytes(m0_w, m0_h);
    let face0_mip0 = &linear[..m0_size];
    let decoded = decode_bc7_to_rgba(face0_mip0, m0_w, m0_h);

    let src_rgba = image::open(png).expect("open").to_rgba8();
    let (mean, peak) = per_channel_error(src_rgba.as_raw(), &decoded);
    assert_within_budget("cube+mip face 0 / mip 0", mean, peak);

    // Sanity: also decode face 5 mip 0 (different face index, same
    // mip level) to prove the per-face stride is correct. Wrong face
    // indexing would land on a mip-1 or mid-data slice and the decode
    // would either fail or produce garbage.
    let face5_mip0 = &linear[per_face * 5..per_face * 5 + m0_size];
    let decoded_f5 = decode_bc7_to_rgba(face5_mip0, m0_w, m0_h);
    let (mean5, peak5) = per_channel_error(src_rgba.as_raw(), &decoded_f5);
    assert_within_budget("cube+mip face 5 / mip 0", mean5, peak5);
}
