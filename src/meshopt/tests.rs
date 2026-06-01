//! Fixture-free tests for the meshoptimizer codec: exact-format vectors that
//! anchor the byte layout to the real meshoptimizer 0.15 spec, plus encode↔
//! decode round-trips on synthetic vertex/index/sequence data.

use super::*;

/// Deterministic LCG so tests are reproducible without a dependency.
fn lcg(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    *seed
}

const CODE_AUX_TABLE: [u8; 16] = [
    0x00, 0x76, 0x87, 0x56, 0x67, 0x78, 0xa9, 0x86, 0x65, 0x89, 0x68, 0x98, 0x01, 0x69, 0, 0,
];

// --- Exact-format vectors (anchor to the real meshopt byte layout) ----------

#[test]
fn vertex_exact_format_vector_single() {
    // One 4-byte vertex: header 0xa0, four zero plane headers (deltas are zero
    // for the first vertex), 28 bytes of tail padding, then the first vertex.
    let v = vec![0x11u8, 0x22, 0x33, 0x44];
    let enc = encode_vertex_buffer(&v, 1, 4).unwrap();
    let mut expect = vec![0xa0u8, 0, 0, 0, 0];
    expect.extend(std::iter::repeat_n(0u8, 28));
    expect.extend_from_slice(&[0x11, 0x22, 0x33, 0x44]);
    assert_eq!(enc, expect, "vertex stream must match the meshopt 0.15 layout");
    assert_eq!(decode_vertex_buffer(1, 4, &enc).unwrap(), v);
}

#[test]
fn index_exact_format_vector_single_triangle() {
    // One triangle {0,1,2}: header 0xe0, one code byte 0xf0 (table ref to
    // codeaux[0]=0), then the 16-byte codeaux table (also the data padding).
    let enc = encode_index_buffer(&[0, 1, 2], 3, 0).unwrap();
    let mut expect = vec![0xe0u8, 0xf0];
    expect.extend_from_slice(&CODE_AUX_TABLE);
    assert_eq!(enc, expect, "index stream must match the meshopt 0.15 layout");
    let dec = decode_index_buffer(3, 4, &enc).unwrap();
    assert_eq!(read_indices(&dec, 3, 4).unwrap(), vec![0, 1, 2]);
}

#[test]
fn sequence_exact_format_vector() {
    // Sequence {0,1,2}: header 0xd0, var-bytes 00/04/04, 4-byte zero tail.
    let enc = encode_index_sequence(&[0, 1, 2], 3, 0).unwrap();
    assert_eq!(enc, vec![0xd0u8, 0x00, 0x04, 0x04, 0, 0, 0, 0]);
    let dec = decode_index_sequence(3, 4, &enc).unwrap();
    assert_eq!(read_indices(&dec, 3, 4).unwrap(), vec![0, 1, 2]);
}

// --- Round-trips ------------------------------------------------------------

#[test]
fn vertex_roundtrip_random_multiblock() {
    for &vsize in &[4usize, 8, 12, 16, 24, 28, 32, 64] {
        for &vcount in &[0usize, 1, 15, 16, 17, 255, 256, 257, 1000] {
            let mut seed = 0x1234_5678u32 ^ (vsize as u32 * 131 + vcount as u32);
            let data: Vec<u8> = (0..vcount * vsize).map(|_| (lcg(&mut seed) >> 24) as u8).collect();
            let enc = encode_vertex_buffer(&data, vcount, vsize).unwrap();
            let dec = decode_vertex_buffer(vcount, vsize, &enc).unwrap();
            assert_eq!(dec, data, "vsize={vsize} vcount={vcount}");
        }
    }
}

#[test]
fn vertex_roundtrip_structured() {
    // Structured data (low per-byte deltas) exercises the 2/4-bit group paths.
    let vsize = 16usize;
    let vcount = 500usize;
    let mut data = vec![0u8; vcount * vsize];
    for (i, b) in data.iter_mut().enumerate() {
        *b = ((i / vsize) as u8).wrapping_mul(3).wrapping_add((i % vsize) as u8);
    }
    let enc = encode_vertex_buffer(&data, vcount, vsize).unwrap();
    assert_eq!(decode_vertex_buffer(vcount, vsize, &enc).unwrap(), data);
}

fn grid_indices(cells: u32) -> Vec<u32> {
    let mut idx = Vec::new();
    let w = cells + 1;
    for y in 0..cells {
        for x in 0..cells {
            let i0 = y * w + x;
            let i1 = i0 + 1;
            let i2 = i0 + w;
            let i3 = i2 + 1;
            idx.extend_from_slice(&[i0, i2, i1, i1, i2, i3]);
        }
    }
    idx
}

#[test]
fn index_roundtrip_grid_and_random_both_versions() {
    for version in 0u8..=1 {
        // Locality-rich grid mesh exercises the vertex/edge FIFO hit paths.
        let grid = grid_indices(20);
        let n = grid.len();
        let enc = encode_index_buffer(&grid, n, version).unwrap();
        for &isz in &[2usize, 4] {
            let dec = decode_index_buffer(n, isz, &enc).unwrap();
            assert_eq!(read_indices(&dec, n, isz).unwrap(), grid, "grid v{version} isz{isz}");
        }
        // Random triangles exercise the free-index (delta) paths.
        let mut seed = 0x0000_ABCDu32 ^ version as u32;
        let rnd: Vec<u32> = (0..3 * 200).map(|_| lcg(&mut seed) % 5000).collect();
        let m = rnd.len();
        let enc = encode_index_buffer(&rnd, m, version).unwrap();
        let dec = decode_index_buffer(m, 4, &enc).unwrap();
        assert_eq!(read_indices(&dec, m, 4).unwrap(), rnd, "random v{version}");
    }
}

#[test]
fn sequence_roundtrip_random_both_versions() {
    for version in 0u8..=1 {
        let mut seed = 0x0000_0055u32 ^ version as u32;
        let seq: Vec<u32> = (0..500).map(|_| lcg(&mut seed) % 100_000).collect();
        let n = seq.len();
        let enc = encode_index_sequence(&seq, n, version).unwrap();
        let dec = decode_index_sequence(n, 4, &enc).unwrap();
        assert_eq!(read_indices(&dec, n, 4).unwrap(), seq, "seq v{version}");
    }
}

// --- Negative / argument validation ----------------------------------------

#[test]
fn rejects_bad_headers_and_args() {
    assert!(matches!(
        decode_vertex_buffer(1, 4, &[0u8; 40]),
        Err(MeshoptError::BadHeader { .. })
    ));
    assert!(matches!(
        decode_vertex_buffer(1, 3, &[0xa0u8; 40]),
        Err(MeshoptError::Invalid(_))
    ));
    assert!(matches!(
        decode_vertex_buffer(1, 4, &[0xa0u8, 0, 0]),
        Err(MeshoptError::Truncated { .. })
    ));
    assert!(matches!(
        decode_index_buffer(3, 4, &[0u8; 30]),
        Err(MeshoptError::BadHeader { .. })
    ));
    assert!(matches!(
        decode_index_buffer(4, 4, &[0xe0u8; 30]),
        Err(MeshoptError::Invalid(_))
    ));
    assert!(matches!(
        decode_index_sequence(3, 4, &[0u8; 30]),
        Err(MeshoptError::BadHeader { .. })
    ));
    assert!(matches!(
        read_indices(&[0u8; 3], 2, 4),
        Err(MeshoptError::Truncated { .. })
    ));
}
