//! MeshCodec (`MCPK`) decompress + repack.
//!
//! **Key finding (verified on the real corpus):** the MCPK inner stream is a
//! *magicless* zstd frame that needs **no dictionary** — decompressing
//! `mc[+0xC..]` with `ZSTD_f_zstd1_magicless` reproduces the model's BFRES
//! exactly (validated byte-identical against 496+ decompressed-`.bfres` oracle
//! files; the full corpus via `mc-extract --dir`). The frame self-describes its
//! content size (the real BFRES length); the MCPK header's size descriptor is
//! the alignment-padded allocation size. The community's "executable
//! dictionary" is not required for these model `.bfres.mc`.
//!
//! Repack mirrors that: compress the (edited) BFRES as a magicless zstd frame
//! with the content size pledged, then wrap it in an MCPK header (version/flags
//! copied from the source, size descriptor recomputed). It does **not** promise
//! byte-identity with Nintendo's encoder — only a `.mc` that decodes back to the
//! exact BFRES (`extract(repack(x)) == x`), which is what a mod pipeline needs.

use zstd::zstd_safe::{self, CCtx, CParameter, DCtx, DParameter, FrameFormat, InBuffer, OutBuffer};

use super::error::{McError, Result};
use super::{McFile, McpkHeader, MC_HEADER_LEN, MC_MAGIC};

/// Window-log ceiling for decode (covers every model size in the corpus).
const WINDOW_LOG_MAX: u32 = 27;

fn zstd_err(stage: &'static str, code: usize) -> McError {
    McError::Zstd {
        stage,
        message: zstd_safe::get_error_name(code).to_string(),
    }
}

/// Decompress a magicless zstd stream into a buffer of `dest_capacity` bytes,
/// returning exactly the decompressed bytes (the frame's content).
///
/// Uses the **streaming** decode (`ZSTD_decompressStream`), not the one-shot
/// path: the game's frames carry an advisory dictionary id that libzstd's
/// one-shot decode rejects ("Dictionary mismatch") even though no dictionary is
/// actually referenced; the streaming path tolerates it (matching the reference
/// decompressor's behavior, verified byte-identical against the oracle).
pub fn decompress_stream(stream: &[u8], dest_capacity: usize) -> Result<Vec<u8>> {
    let mut dctx = DCtx::create();
    dctx.set_parameter(DParameter::Format(FrameFormat::Magicless))
        .map_err(|c| zstd_err("set magicless", c))?;
    dctx.set_parameter(DParameter::WindowLogMax(WINDOW_LOG_MAX))
        .map_err(|c| zstd_err("set window-log-max", c))?;
    let cap = dest_capacity.max(1);
    let mut out = vec![0u8; cap];
    let n = {
        let mut output = OutBuffer::around(&mut out[..]);
        let mut input = InBuffer::around(stream);
        loop {
            let prev_in = input.pos();
            let prev_out = output.pos();
            let hint = dctx
                .decompress_stream(&mut output, &mut input)
                .map_err(|c| zstd_err("decompress", c))?;
            if hint == 0 {
                break; // frame complete
            }
            if output.pos() == cap {
                break; // destination filled to the declared capacity
            }
            if input.pos() == prev_in && output.pos() == prev_out {
                break; // no progress (truncated/garbage tail) — stop cleanly
            }
        }
        output.pos()
    };
    out.truncate(n);
    Ok(out)
}

/// Compress `data` as a magicless zstd frame with the content size pledged
/// (so the frame self-describes its length, matching the game's frames).
pub fn compress_stream(data: &[u8], level: i32) -> Result<Vec<u8>> {
    let mut cctx = CCtx::create();
    cctx.set_parameter(CParameter::Format(FrameFormat::Magicless))
        .map_err(|c| zstd_err("set magicless", c))?;
    cctx.set_parameter(CParameter::CompressionLevel(level))
        .map_err(|c| zstd_err("set level", c))?;
    cctx.set_parameter(CParameter::ContentSizeFlag(true))
        .map_err(|c| zstd_err("set content-size flag", c))?;
    cctx.set_pledged_src_size(Some(data.len() as u64))
        .map_err(|c| zstd_err("pledge src size", c))?;
    let mut out = Vec::with_capacity(zstd_safe::compress_bound(data.len()));
    cctx.compress2(&mut out, data)
        .map_err(|c| zstd_err("compress", c))?;
    Ok(out)
}

/// Decompress an MCPK container to its BFRES bytes (the real, unpadded BFRES —
/// the zstd frame's content size, not the alignment-padded allocation size).
pub fn extract(mc: &McFile) -> Result<Vec<u8>> {
    decompress_stream(mc.compressed_stream(), mc.decompressed_size())
}

/// Build an MCPK size descriptor for `decompressed_size` at the given
/// alignment shift: `descriptor = (mantissa << 5) | shift`, where
/// `mantissa = ceil(size / (1<<shift))`, so `(d>>5)<<(d&0xf) >= size`.
pub fn size_descriptor(decompressed_size: usize, shift: u32) -> u32 {
    let unit = 1usize << shift;
    let mantissa = decompressed_size.div_ceil(unit);
    ((mantissa as u32) << 5) | (shift & 0xf)
}

/// Repack an (edited) BFRES into a `.mc`, copying the version/flags and
/// alignment shift from the original container. The result decodes back to
/// `bfres` exactly via [`extract`]; it is not byte-identical to Nintendo's
/// encoder.
pub fn repack(original: &McFile, bfres: &[u8], level: i32) -> Result<Vec<u8>> {
    let compressed = compress_stream(bfres, level)?;
    // Preserve the original allocation size when the edit fits (the game
    // allocates this buffer); only grow it if the edit is larger.
    let descriptor = if bfres.len() <= original.decompressed_size() {
        original.header.size_descriptor
    } else {
        size_descriptor(bfres.len(), original.header.alignment_shift())
    };
    let mut out = Vec::with_capacity(MC_HEADER_LEN + compressed.len());
    out.extend_from_slice(MC_MAGIC);
    out.push(original.header.version);
    out.push(original.header.flags);
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&descriptor.to_le_bytes());
    out.extend_from_slice(&compressed);
    Ok(out)
}

/// Build an [`McpkHeader`]-consistent `.mc` from scratch (no source container)
/// using a default alignment shift of 12 (`0x1000`, the corpus norm).
pub fn repack_default(bfres: &[u8], version: u8, flags: u8, level: i32) -> Result<Vec<u8>> {
    let dummy = McFile {
        header: McpkHeader {
            version,
            flags,
            size_descriptor: size_descriptor(bfres.len(), 12),
        },
        raw: Vec::new(),
    };
    repack(&dummy, bfres, level)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magicless_round_trip_self_consistent() {
        // A BFRES-ish payload with redundancy so zstd actually compresses.
        let mut data = Vec::new();
        data.extend_from_slice(b"FRES    ");
        for i in 0..4000u32 {
            data.extend_from_slice(&(i % 37).to_le_bytes());
        }
        let comp = compress_stream(&data, 3).expect("compress");
        // Magicless: must NOT start with the standard zstd magic.
        assert_ne!(&comp[0..4], &[0x28, 0xb5, 0x2f, 0xfd]);
        let back = decompress_stream(&comp, data.len() + 4096).expect("decompress");
        assert_eq!(back, data, "magicless zstd round-trip must be lossless");
    }

    #[test]
    fn repack_then_extract_is_identity() {
        let bfres: Vec<u8> = b"FRES    "
            .iter()
            .copied()
            .chain((0..2048u32).flat_map(|i| (i % 53).to_le_bytes()))
            .collect();
        let mc_bytes = repack_default(&bfres, 1, 1, 5).expect("repack");
        assert_eq!(&mc_bytes[0..4], MC_MAGIC);
        let mc = super::super::read_mc(&mc_bytes).expect("read repacked");
        assert!(mc.decompressed_size() >= bfres.len());
        let extracted = extract(&mc).expect("extract");
        assert_eq!(extracted, bfres, "extract(repack(x)) must equal x");
    }

    #[test]
    fn size_descriptor_round_trips_through_header() {
        for (size, shift) in [(12768usize, 12u32), (1, 12), (0x10000, 12), (5000, 8)] {
            let d = size_descriptor(size, shift);
            let decoded = ((d >> 5) as usize) << (d & 0xf);
            assert!(decoded >= size, "descriptor must cover the size");
            assert!(decoded - size < (1 << shift), "no more than one unit of slack");
        }
    }
}
