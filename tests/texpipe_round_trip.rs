//! End-to-end round-trip test for the texpipe pipeline:
//!
//!     PNG -> BC7 (intel_tex_2) -> Tegra swizzle -> Tegra deswizzle
//!         -> BC7 decode (texture2ddecoder) -> compare against PNG source
//!
//! The intent is to catch structural bugs that would otherwise only
//! surface in-game on Switch hardware: axis transposition, byte-order
//! mistakes (BGRA vs RGBA), `block_height_log2` mismatches between
//! swizzle and deswizzle, alignment / padding errors at non-power-of-two
//! dimensions, and so on. We don't compare for byte-identical pixel
//! match — BC7 is lossy — but we DO bound the per-channel mean and max
//! absolute error so any real corruption (e.g., an off-by-one in the
//! swizzler) blows the budget by orders of magnitude.
//!
//! Skipped when `tests/fixtures/png-test-images/` is absent.

use std::fs;
use std::path::Path;

use tegra_swizzle::surface::{deswizzle_surface, BlockDim};
use tegra_swizzle::BlockHeight;
use toolbox_cli::texpipe::{compress_image_bc7, Bc7Quality};

/// Per-channel error budgets, sized for the BC7 `Fast` quality preset
/// with alpha. The numbers are deliberately loose enough to absorb
/// expected lossy-encoding error but tight enough that a swizzle
/// transposition or BGRA/RGBA flip will still fail the test by a
/// large margin (those produce mean errors >100 on natural images).
const MAX_MEAN_ABS_ERROR: f64 = 12.0;
const MAX_PEAK_ABS_ERROR: u8 = 80;

#[test]
fn png_to_bc7_to_swizzle_to_deswizzle_to_decode_round_trips() {
    let dir = Path::new("tests/fixtures/png-test-images");
    if !dir.exists() {
        eprintln!(
            "skipping texpipe round-trip test (drop rgba_alpha_*.png into {} to enable)",
            dir.display()
        );
        return;
    }

    let mut tested = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for entry in fs::read_dir(dir).expect("read fixtures dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("png") {
            continue;
        }

        match round_trip_one(&path) {
            Ok(()) => tested += 1,
            Err(msg) => failures.push(format!("{}: {msg}", path.display())),
        }
    }

    assert!(
        tested > 0,
        "expected at least one PNG fixture in {}",
        dir.display()
    );
    if !failures.is_empty() {
        for f in &failures {
            eprintln!("  {f}");
        }
        panic!(
            "{}/{} texpipe round-trip cases failed",
            failures.len(),
            tested + failures.len()
        );
    }
    println!("OK: {tested} PNG fixture(s) round-tripped through the texpipe");
}

fn round_trip_one(png_path: &Path) -> Result<(), String> {
    let img = image::open(png_path).map_err(|e| format!("opening PNG: {e}"))?;
    let src_rgba = img.to_rgba8();
    let (src_w, src_h) = src_rgba.dimensions();

    // Encode + swizzle in one shot via the same pipeline `bntx-import-png`
    // uses. `Fast` is enough quality to keep the test snappy across all
    // fixtures (including 1024x1024) while staying well inside the
    // tolerance budget for natural-looking images.
    let compressed = compress_image_bc7(&img, Bc7Quality::Fast)
        .map_err(|e| format!("compress_image_bc7 failed: {e}"))?;

    // The texpipe pads non-multiple-of-4 dimensions up to the BC7 block
    // grid. Today all our fixtures are already multiples of 4; assert
    // that explicitly so the dimension comparison below is meaningful
    // (the test will trip loudly if we add a non-aligned fixture
    // without first deciding how to compare it).
    if compressed.width != src_w || compressed.height != src_h {
        return Err(format!(
            "padded {}x{} != source {}x{}; non-multiple-of-4 fixtures aren't yet supported here",
            compressed.width, compressed.height, src_w, src_h
        ));
    }

    let block_height = block_height_from_log2(compressed.block_height_log2)
        .ok_or_else(|| format!("unsupported block_height_log2 {}", compressed.block_height_log2))?;

    let linear_bc7 = deswizzle_surface(
        compressed.width,
        compressed.height,
        1,
        &compressed.swizzled_data,
        BlockDim::block_4x4(),
        Some(block_height),
        16, // BC7: 16 bytes per 4x4 block
        1,
        1,
    )
    .map_err(|e| format!("deswizzle_surface: {e:?}"))?;

    if linear_bc7.len() != compressed.linear_size as usize {
        return Err(format!(
            "deswizzle output size {} != expected linear_size {}",
            linear_bc7.len(),
            compressed.linear_size
        ));
    }

    let pixel_count = (compressed.width * compressed.height) as usize;
    let mut bgra32 = vec![0u32; pixel_count];
    texture2ddecoder::decode_bc7(
        &linear_bc7,
        compressed.width as usize,
        compressed.height as usize,
        &mut bgra32,
    )
    .map_err(|e| format!("decode_bc7: {e}"))?;

    let mut decoded_rgba = vec![0u8; pixel_count * 4];
    for (i, packed) in bgra32.iter().enumerate() {
        // texture2ddecoder packs each pixel as BGRA in a u32 (B in the
        // low byte, A in the high byte). We compare against `image`'s
        // RGBA8 buffer, so unpack accordingly.
        let b = (*packed & 0xFF) as u8;
        let g = ((*packed >> 8) & 0xFF) as u8;
        let r = ((*packed >> 16) & 0xFF) as u8;
        let a = ((*packed >> 24) & 0xFF) as u8;
        decoded_rgba[i * 4..i * 4 + 4].copy_from_slice(&[r, g, b, a]);
    }

    let (mean_err, max_err) = per_channel_error(src_rgba.as_raw(), &decoded_rgba);
    if mean_err > MAX_MEAN_ABS_ERROR || max_err > MAX_PEAK_ABS_ERROR {
        return Err(format!(
            "error budget exceeded: mean={mean_err:.2} (max {MAX_MEAN_ABS_ERROR}), \
             peak={max_err} (max {MAX_PEAK_ABS_ERROR})",
        ));
    }
    Ok(())
}

/// Map `block_height_log2` (the value the texpipe records into BNTX
/// `size_range`) back into `tegra_swizzle::BlockHeight`. Inverse of the
/// match in `compress_image_bc7`.
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

/// Compute the mean and peak absolute per-channel error between two
/// RGBA8 buffers of equal size. Returned as `(mean, peak)`.
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
