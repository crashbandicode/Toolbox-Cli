//! meshoptimizer 0.15 **vertex buffer** codec (`meshopt_encodeVertexBuffer` /
//! `meshopt_decodeVertexBuffer`), reimplemented from the MIT specification
//! (`zeux/meshoptimizer` `vertexcodec.cpp`) — no GPL code.
//!
//! ## Format (version 0)
//!
//! ```text
//! [u8 header = 0xa0 | version]
//! [vertex blocks ...]            (each <= 256 vertices)
//! [tail: max(vertex_size, 32) bytes ending with the FIRST vertex]
//! ```
//!
//! Vertices are encoded **structure-of-arrays**: within a block, each of the
//! `vertex_size` byte columns is delta-coded against the previous vertex,
//! zig-zag mapped, then packed into 16-value "byte groups" at 0/2/4/8 bits per
//! value (a 2-bit-per-group header selects the width). The decoder is the exact
//! inverse; SIMD and scalar implementations produce identical bytes, so this
//! scalar port matches the game's NEON decoder byte-for-byte.

use super::error::{MeshoptError, Result};

const VERTEX_HEADER: u8 = 0xa0;
const VERTEX_BLOCK_SIZE_BYTES: usize = 8192;
const VERTEX_BLOCK_MAX_SIZE: usize = 256;
const BYTE_GROUP_SIZE: usize = 16;
const BYTE_GROUP_DECODE_LIMIT: usize = 24;
const TAIL_MAX_SIZE: usize = 32;

#[inline]
fn zigzag8(v: u8) -> u8 {
    (((v as i8) >> 7) as u8) ^ (v << 1)
}

#[inline]
fn unzigzag8(v: u8) -> u8 {
    (0u8.wrapping_sub(v & 1)) ^ (v >> 1)
}

fn get_vertex_block_size(vertex_size: usize) -> usize {
    let mut result = VERTEX_BLOCK_SIZE_BYTES / vertex_size;
    result &= !(BYTE_GROUP_SIZE - 1);
    if result < VERTEX_BLOCK_MAX_SIZE {
        result
    } else {
        VERTEX_BLOCK_MAX_SIZE
    }
}

// ---------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------

/// Decode one 16-value byte group at `p` into `buffer[boff..boff+16]`; returns
/// the new input position.
fn decode_bytes_group(data: &[u8], p: usize, buffer: &mut [u8], boff: usize, bitslog2: u32) -> usize {
    match bitslog2 {
        0 => {
            for b in buffer.iter_mut().skip(boff).take(BYTE_GROUP_SIZE) {
                *b = 0;
            }
            p
        }
        1 => {
            let mut data_var = p + 4;
            let mut out = boff;
            for bi in 0..4 {
                let mut byte = data[p + bi];
                for _ in 0..4 {
                    let enc = byte >> 6;
                    byte <<= 2;
                    let encv = data[data_var];
                    if enc == 3 {
                        buffer[out] = encv;
                        data_var += 1;
                    } else {
                        buffer[out] = enc;
                    }
                    out += 1;
                }
            }
            data_var
        }
        2 => {
            let mut data_var = p + 8;
            let mut out = boff;
            for bi in 0..8 {
                let mut byte = data[p + bi];
                for _ in 0..2 {
                    let enc = byte >> 4;
                    byte <<= 4;
                    let encv = data[data_var];
                    if enc == 15 {
                        buffer[out] = encv;
                        data_var += 1;
                    } else {
                        buffer[out] = enc;
                    }
                    out += 1;
                }
            }
            data_var
        }
        _ => {
            buffer[boff..boff + BYTE_GROUP_SIZE].copy_from_slice(&data[p..p + BYTE_GROUP_SIZE]);
            p + BYTE_GROUP_SIZE
        }
    }
}

fn decode_bytes(data: &[u8], mut p: usize, buffer: &mut [u8], buffer_size: usize) -> Result<usize> {
    debug_assert!(buffer_size.is_multiple_of(BYTE_GROUP_SIZE));
    let header = p;
    let header_size = (buffer_size / BYTE_GROUP_SIZE).div_ceil(4);
    if data.len() - p < header_size {
        return Err(MeshoptError::Truncated {
            what: "vertex byte-group header",
            have: data.len() - p,
            need: header_size,
        });
    }
    p += header_size;
    let mut i = 0;
    while i < buffer_size {
        if data.len() - p < BYTE_GROUP_DECODE_LIMIT {
            return Err(MeshoptError::Truncated {
                what: "vertex byte-group",
                have: data.len() - p,
                need: BYTE_GROUP_DECODE_LIMIT,
            });
        }
        let header_offset = i / BYTE_GROUP_SIZE;
        let bitslog2 = (data[header + header_offset / 4] >> ((header_offset % 4) * 2)) & 3;
        p = decode_bytes_group(data, p, buffer, i, bitslog2 as u32);
        i += BYTE_GROUP_SIZE;
    }
    Ok(p)
}

#[allow(clippy::too_many_arguments)]
fn decode_vertex_block(
    data: &[u8],
    mut p: usize,
    vertex_data: &mut [u8],
    voff: usize,
    vertex_count: usize,
    vertex_size: usize,
    last_vertex: &mut [u8],
    buffer: &mut [u8],
    transposed: &mut [u8],
) -> Result<usize> {
    let vca = (vertex_count + BYTE_GROUP_SIZE - 1) & !(BYTE_GROUP_SIZE - 1);
    for (k, &lvk) in last_vertex.iter().enumerate() {
        p = decode_bytes(data, p, buffer, vca)?;
        let mut vo = k;
        let mut pr = lvk;
        for &b in buffer.iter().take(vertex_count) {
            let v = unzigzag8(b).wrapping_add(pr);
            transposed[vo] = v;
            pr = v;
            vo += vertex_size;
        }
    }
    let n = vertex_count * vertex_size;
    vertex_data[voff..voff + n].copy_from_slice(&transposed[..n]);
    last_vertex[..vertex_size]
        .copy_from_slice(&transposed[vertex_size * (vertex_count - 1)..vertex_size * vertex_count]);
    Ok(p)
}

/// Decode a meshoptimizer vertex buffer: `vertex_count` vertices of
/// `vertex_size` bytes each (`vertex_size` must be a non-zero multiple of 4,
/// `<= 256`). Returns `vertex_count * vertex_size` bytes.
pub fn decode_vertex_buffer(vertex_count: usize, vertex_size: usize, buffer: &[u8]) -> Result<Vec<u8>> {
    if vertex_size == 0 || vertex_size > 256 || !vertex_size.is_multiple_of(4) {
        return Err(MeshoptError::Invalid(format!(
            "vertex_size {vertex_size} must be a non-zero multiple of 4, <= 256"
        )));
    }
    let n = buffer.len();
    if n < 1 + vertex_size {
        return Err(MeshoptError::Truncated {
            what: "vertex buffer",
            have: n,
            need: 1 + vertex_size,
        });
    }
    let data_header = buffer[0];
    if (data_header & 0xf0) != VERTEX_HEADER || (data_header & 0x0f) != 0 {
        return Err(MeshoptError::BadHeader {
            what: "vertex",
            byte: data_header,
        });
    }

    let mut out = vec![0u8; vertex_count * vertex_size];
    let mut last_vertex = vec![0u8; vertex_size];
    last_vertex.copy_from_slice(&buffer[n - vertex_size..n]);

    let vbs = get_vertex_block_size(vertex_size);
    // Scratch reused across blocks (mirrors the C stack buffers).
    let mut buf = vec![0u8; VERTEX_BLOCK_MAX_SIZE];
    let mut transposed = vec![0u8; vertex_size * VERTEX_BLOCK_MAX_SIZE];

    let mut p = 1;
    let mut voff = 0;
    let mut vo = 0;
    while vo < vertex_count {
        let bs = if vo + vbs < vertex_count {
            vbs
        } else {
            vertex_count - vo
        };
        p = decode_vertex_block(
            buffer,
            p,
            &mut out,
            voff,
            bs,
            vertex_size,
            &mut last_vertex,
            &mut buf,
            &mut transposed,
        )?;
        voff += bs * vertex_size;
        vo += bs;
    }

    let tail = if vertex_size < TAIL_MAX_SIZE {
        TAIL_MAX_SIZE
    } else {
        vertex_size
    };
    if n - p != tail {
        return Err(MeshoptError::ExtraBytes {
            what: "vertex buffer",
            leftover: n - p,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Encode
// ---------------------------------------------------------------------------

fn encode_bytes_group_zero(buffer: &[u8]) -> bool {
    buffer.iter().take(BYTE_GROUP_SIZE).all(|&b| b == 0)
}

/// Measure the encoded size of a 16-value group at `bits` bits/value
/// (`u64::MAX` = "impossible", mirroring the C `size_t(-1)`).
fn encode_bytes_group_measure(buffer: &[u8], bits: u32) -> u64 {
    if bits == 1 {
        return if encode_bytes_group_zero(buffer) { 0 } else { u64::MAX };
    }
    if bits == 8 {
        return BYTE_GROUP_SIZE as u64;
    }
    let mut result = (BYTE_GROUP_SIZE as u64) * (bits as u64) / 8;
    let sentinel = (1u16 << bits) - 1;
    for &b in buffer.iter().take(BYTE_GROUP_SIZE) {
        result += (b as u16 >= sentinel) as u64;
    }
    result
}

fn encode_bytes_group(data: &mut Vec<u8>, buffer: &[u8], bits: u32) {
    if bits == 1 {
        return;
    }
    if bits == 8 {
        data.extend_from_slice(&buffer[..BYTE_GROUP_SIZE]);
        return;
    }
    let byte_size = 8 / bits as usize;
    let sentinel = (1u8 << bits) - 1;
    let mut i = 0;
    while i < BYTE_GROUP_SIZE {
        let mut byte: u8 = 0;
        for k in 0..byte_size {
            let enc = if buffer[i + k] >= sentinel {
                sentinel
            } else {
                buffer[i + k]
            };
            byte <<= bits;
            byte |= enc;
        }
        data.push(byte);
        i += byte_size;
    }
    for &b in buffer.iter().take(BYTE_GROUP_SIZE) {
        if b >= sentinel {
            data.push(b);
        }
    }
}

fn encode_bytes(data: &mut Vec<u8>, buffer: &[u8], buffer_size: usize) {
    debug_assert!(buffer_size.is_multiple_of(BYTE_GROUP_SIZE));
    let header_pos = data.len();
    let header_size = (buffer_size / BYTE_GROUP_SIZE).div_ceil(4);
    data.resize(header_pos + header_size, 0);

    let mut i = 0;
    while i < buffer_size {
        let group = &buffer[i..i + BYTE_GROUP_SIZE];
        let mut best_bits = 8u32;
        let mut best_size = encode_bytes_group_measure(group, 8);
        let mut bits = 1u32;
        while bits < 8 {
            let size = encode_bytes_group_measure(group, bits);
            if size < best_size {
                best_bits = bits;
                best_size = size;
            }
            bits *= 2;
        }
        let bitslog2 = match best_bits {
            1 => 0u8,
            2 => 1,
            4 => 2,
            _ => 3,
        };
        let header_offset = i / BYTE_GROUP_SIZE;
        data[header_pos + header_offset / 4] |= bitslog2 << ((header_offset % 4) * 2);
        encode_bytes_group(data, group, best_bits);
        i += BYTE_GROUP_SIZE;
    }
}

fn encode_vertex_block(
    data: &mut Vec<u8>,
    vertex_data: &[u8],
    voff: usize,
    vertex_count: usize,
    vertex_size: usize,
    last_vertex: &mut [u8],
) {
    let mut buffer = [0u8; VERTEX_BLOCK_MAX_SIZE];
    let aligned = (vertex_count + BYTE_GROUP_SIZE - 1) & !(BYTE_GROUP_SIZE - 1);
    for (k, &lvk) in last_vertex.iter().enumerate() {
        let mut p = lvk;
        let mut vo = voff + k;
        for b in buffer.iter_mut().take(vertex_count) {
            let cur = vertex_data[vo];
            *b = zigzag8(cur.wrapping_sub(p));
            p = cur;
            vo += vertex_size;
        }
        encode_bytes(data, &buffer, aligned);
    }
    last_vertex[..vertex_size].copy_from_slice(
        &vertex_data[voff + vertex_size * (vertex_count - 1)..voff + vertex_size * vertex_count],
    );
}

/// Encode `vertex_count` vertices of `vertex_size` bytes (a non-zero multiple
/// of 4, `<= 256`) into a meshoptimizer version-0 vertex buffer.
pub fn encode_vertex_buffer(vertices: &[u8], vertex_count: usize, vertex_size: usize) -> Result<Vec<u8>> {
    if vertex_size == 0 || vertex_size > 256 || !vertex_size.is_multiple_of(4) {
        return Err(MeshoptError::Invalid(format!(
            "vertex_size {vertex_size} must be a non-zero multiple of 4, <= 256"
        )));
    }
    if vertices.len() < vertex_count * vertex_size {
        return Err(MeshoptError::Truncated {
            what: "vertex source",
            have: vertices.len(),
            need: vertex_count * vertex_size,
        });
    }

    let mut data = Vec::with_capacity(1 + vertex_count * vertex_size / 2 + TAIL_MAX_SIZE);
    data.push(VERTEX_HEADER);

    let mut first_vertex = vec![0u8; vertex_size];
    if vertex_count > 0 {
        first_vertex.copy_from_slice(&vertices[..vertex_size]);
    }
    let mut last_vertex = first_vertex.clone();

    let vbs = get_vertex_block_size(vertex_size);
    let mut vo = 0;
    while vo < vertex_count {
        let bs = if vo + vbs < vertex_count {
            vbs
        } else {
            vertex_count - vo
        };
        encode_vertex_block(&mut data, vertices, vo * vertex_size, bs, vertex_size, &mut last_vertex);
        vo += bs;
    }

    if vertex_size < TAIL_MAX_SIZE {
        data.resize(data.len() + (TAIL_MAX_SIZE - vertex_size), 0);
    }
    data.extend_from_slice(&first_vertex);
    Ok(data)
}
