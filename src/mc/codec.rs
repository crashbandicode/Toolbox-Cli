//! MeshCodec (`MCPK`) decompress + repack.
//!
//! **Key finding (verified on the real corpus):** the MCPK inner stream is a
//! *magicless* zstd frame that needs **no dictionary** — decoding `mc[+0xC..]`
//! as `ZSTD_f_zstd1_magicless` reproduces the model's BFRES exactly (validated
//! byte-identical against the decompressed-`.bfres` oracle). The frame
//! self-describes its content size (the real BFRES length); the MCPK header's
//! size descriptor is the alignment-padded allocation size. The community's
//! "executable dictionary" is not required for these model `.bfres.mc`.
//!
//! Decode + encode go through the pure-Rust [`zstd_pure`] codec — **no libzstd
//! (C) at runtime**. The game's frames carry an *advisory* dictionary id but
//! reference no real dictionary; the pure decoder ignores it when no dictionary
//! is supplied, matching the reference decompressor.
//!
//! Repack mirrors that: compress the (edited) BFRES as a magicless zstd frame
//! with the content size pledged, then wrap it in an MCPK header (version/flags
//! copied from the source, size descriptor recomputed). It does **not** promise
//! byte-identity with Nintendo's encoder — only a `.mc` that decodes back to the
//! exact BFRES (`extract(repack(x)) == x`), which is what a mod pipeline needs.

use super::error::{McError, Result};
use super::{McFile, MC_HEADER_LEN, MC_MAGIC};

/// Map a [`zstd_pure::ZstdError`] into [`McError::Zstd`] tagged with the stage.
fn zstd_err(stage: &'static str) -> impl Fn(zstd_pure::ZstdError) -> McError {
    move |e| McError::Zstd {
        stage,
        message: e.to_string(),
    }
}

/// Decompress the **leading magicless zstd frame** of a stream, returning the
/// decompressed bytes and the number of input bytes the frame consumed.
///
/// A TotK model `.mc` stream is `[BFRES frame (magicless zstd)] [mesh buffers
/// (custom MeshCodec encoding — NOT zstd)]`. This decodes only the first frame
/// (the BFRES); the returned `consumed` marks where the untouched mesh tail
/// begins. `dest_capacity` (the MCPK declared allocation size) is the output
/// ceiling.
pub fn decompress_first_frame(stream: &[u8], dest_capacity: usize) -> Result<(Vec<u8>, usize)> {
    let frame = zstd_pure::decompress_magicless(stream, dest_capacity.max(1))
        .map_err(zstd_err("decode"))?;
    Ok((frame.data, frame.consumed))
}

/// Decompress the leading magicless zstd frame, returning just the bytes.
pub fn decompress_stream(stream: &[u8], dest_capacity: usize) -> Result<Vec<u8>> {
    decompress_first_frame(stream, dest_capacity).map(|(b, _)| b)
}

/// Compress `data` as a magicless zstd frame with the content size pledged
/// (so the frame self-describes its length, matching the game's frames).
///
/// `level` selects the `zstd_pure` compression parameters; no content checksum
/// and no frame magic (magicless), matching the game's BFRES frames.
pub fn compress_stream(data: &[u8], level: i32) -> Result<Vec<u8>> {
    Ok(zstd_pure::compress(data, level, false, false))
}

/// Extract the **BFRES structure** from an MCPK container — the leading
/// magicless zstd frame, byte-identical to the reference decompressor's BFRES.
///
/// IMPORTANT: a TotK model `.mc` continues with a trailing **mesh** section
/// (vertex / index buffers) in a *custom* MeshCodec encoding (not zstd) that
/// this does NOT decode. The returned BFRES is the model's *structure*
/// (FMDL / FSKL / FMAT / FSHP headers + vertex-attribute defs + `_STR` / `_RLT`)
/// — it is a complete, valid BFRES file, but the geometry buffers live in the
/// undecoded tail. For round-tripping, [`repack`] preserves that tail verbatim.
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

/// Repack an (edited) BFRES into a `.mc`, **preserving the original's mesh
/// tail** (the custom-coded vertex/index buffers we don't decode).
///
/// The output is `MCPK header + [new BFRES magicless frame] + [original mesh
/// tail, byte-for-byte]`, so the model keeps its original geometry and only the
/// BFRES structure changes. `extract` decodes back to `bfres` exactly.
///
/// Because the BFRES references the mesh by layout, an edit that changes the
/// BFRES byte-size would shift those references; such a resize is **rejected**
/// unless `allow_resize` is set (then it's best-effort and likely needs the
/// real mesh codec). Geometry edits are not supported (the mesh tail is opaque).
/// Not byte-identical to Nintendo's encoder.
pub fn repack(original: &McFile, bfres: &[u8], level: i32, allow_resize: bool) -> Result<Vec<u8>> {
    // Decode the original BFRES frame: its length + where the mesh tail starts.
    let (orig_bfres, frame_len) =
        decompress_first_frame(original.compressed_stream(), original.decompressed_size())?;
    if bfres.len() != orig_bfres.len() && !allow_resize {
        return Err(McError::ResizeNotAllowed {
            original: orig_bfres.len(),
            edited: bfres.len(),
        });
    }
    let tail = &original.compressed_stream()[frame_len..];
    let compressed = compress_stream(bfres, level)?;
    let mut out = Vec::with_capacity(MC_HEADER_LEN + compressed.len() + tail.len());
    out.extend_from_slice(MC_MAGIC);
    out.push(original.header.version);
    out.push(original.header.flags);
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
                                                // Preserve the original allocation (the mesh layout is unchanged).
    out.extend_from_slice(&original.header.size_descriptor.to_le_bytes());
    out.extend_from_slice(&compressed);
    out.extend_from_slice(tail); // custom-coded mesh buffers, byte-preserved
    Ok(out)
}

/// Build a *structure-only* `.mc` from a BFRES with no source container (no mesh
/// tail). Used for synthetic tests / non-model BFRES; real models must use
/// [`repack`] so their mesh tail is preserved.
pub fn repack_default(bfres: &[u8], version: u8, flags: u8, level: i32) -> Result<Vec<u8>> {
    let compressed = compress_stream(bfres, level)?;
    let descriptor = size_descriptor(bfres.len(), 12);
    let mut out = Vec::with_capacity(MC_HEADER_LEN + compressed.len());
    out.extend_from_slice(MC_MAGIC);
    out.push(version);
    out.push(flags);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&descriptor.to_le_bytes());
    out.extend_from_slice(&compressed);
    Ok(out)
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
    fn repack_default_then_extract_is_identity() {
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

    /// Repack must preserve the original mesh tail byte-for-byte and reject a
    /// size-changing edit (which would break the mesh layout).
    #[test]
    fn repack_preserves_mesh_tail_and_guards_resize() {
        let bfres: Vec<u8> = b"FRES    "
            .iter()
            .copied()
            .chain((0..1024u32).flat_map(|i| (i % 53).to_le_bytes()))
            .collect();
        // Build a synthetic .mc = [BFRES frame] + [fake custom-coded mesh tail].
        let frame = compress_stream(&bfres, 5).unwrap();
        let mesh_tail = vec![0xABu8; 777]; // opaque "mesh" bytes (not zstd)
        let mut raw = Vec::new();
        raw.extend_from_slice(MC_MAGIC);
        raw.push(1);
        raw.push(1);
        raw.extend_from_slice(&0u16.to_le_bytes());
        raw.extend_from_slice(&size_descriptor(bfres.len(), 12).to_le_bytes());
        raw.extend_from_slice(&frame);
        raw.extend_from_slice(&mesh_tail);
        let mc = super::super::read_mc(&raw).expect("read synthetic .mc");

        // Same-size edit: tail preserved, extract == edited.
        let mut edited = bfres.clone();
        edited[8] ^= 0xFF; // flip a structure byte, same length
        let repacked = repack(&mc, &edited, 5, false).expect("repack same-size");
        assert!(
            repacked.ends_with(&mesh_tail),
            "mesh tail must be byte-preserved"
        );
        let mc2 = super::super::read_mc(&repacked).unwrap();
        assert_eq!(extract(&mc2).unwrap(), edited, "extract(repack)=edited");

        // Size change without --allow-resize is rejected.
        let bigger = [edited.as_slice(), b"extra"].concat();
        assert!(matches!(
            repack(&mc, &bigger, 5, false),
            Err(McError::ResizeNotAllowed { .. })
        ));
        // With allow_resize it proceeds (best-effort).
        assert!(repack(&mc, &bigger, 5, true).is_ok());
    }

    #[test]
    fn size_descriptor_round_trips_through_header() {
        for (size, shift) in [(12768usize, 12u32), (1, 12), (0x10000, 12), (5000, 8)] {
            let d = size_descriptor(size, shift);
            let decoded = ((d >> 5) as usize) << (d & 0xf);
            assert!(decoded >= size, "descriptor must cover the size");
            assert!(
                decoded - size < (1 << shift),
                "no more than one unit of slack"
            );
        }
    }
}
