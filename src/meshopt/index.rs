//! meshoptimizer 0.15 **index buffer** + **index sequence** codecs, reimplemented
//! from the MIT specification (`zeux/meshoptimizer` `indexcodec.cpp`) — no GPL.
//!
//! * Index buffer (`0xe0`): triangle-list compression using vertex/edge FIFOs
//!   (Giesen/Stokes). Supports format versions 0 and 1.
//! * Index sequence (`0xd0`): a generic dual-baseline delta var-int stream.
//!
//! Indices are decoded as little-endian `u16` (`index_size == 2`) or `u32`
//! (`index_size == 4`).

use super::error::{MeshoptError, Result};

const INDEX_HEADER: u8 = 0xe0;
const SEQUENCE_HEADER: u8 = 0xd0;

const CODE_AUX_ENCODING_TABLE: [u8; 16] = [
    0x00, 0x76, 0x87, 0x56, 0x67, 0x78, 0xa9, 0x86, 0x65, 0x89, 0x68, 0x98, 0x01, 0x69, 0, 0,
];

const TRIANGLE_INDEX_ORDER: [[usize; 3]; 3] = [[0, 1, 2], [1, 2, 0], [2, 0, 1]];

// ---------------------------------------------------------------------------
// Var-byte / index primitives
// ---------------------------------------------------------------------------

fn decode_vbyte(data: &[u8], p: &mut usize) -> u32 {
    let lead = data[*p];
    *p += 1;
    if lead < 128 {
        return lead as u32;
    }
    let mut result = (lead & 127) as u32;
    let mut shift = 7u32;
    for _ in 0..4 {
        let group = data[*p];
        *p += 1;
        result |= ((group & 127) as u32) << shift;
        shift += 7;
        if group < 128 {
            break;
        }
    }
    result
}

fn encode_vbyte(data: &mut Vec<u8>, mut v: u32) {
    loop {
        data.push(((v & 127) as u8) | if v > 127 { 128 } else { 0 });
        v >>= 7;
        if v == 0 {
            break;
        }
    }
}

fn encode_index(data: &mut Vec<u8>, index: u32, last: u32) {
    let d = index.wrapping_sub(last);
    let v = (d << 1) ^ (((d as i32) >> 31) as u32);
    encode_vbyte(data, v);
}

#[inline]
fn put_index(out: &mut [u8], idx: usize, index_size: usize, val: u32) {
    let pos = idx * index_size;
    if index_size == 2 {
        out[pos..pos + 2].copy_from_slice(&(val as u16).to_le_bytes());
    } else {
        out[pos..pos + 4].copy_from_slice(&val.to_le_bytes());
    }
}

#[inline]
fn write_triangle(out: &mut [u8], i: usize, index_size: usize, a: u32, b: u32, c: u32) {
    put_index(out, i, index_size, a);
    put_index(out, i + 1, index_size, b);
    put_index(out, i + 2, index_size, c);
}

// ---------------------------------------------------------------------------
// FIFOs
// ---------------------------------------------------------------------------

type EdgeFifo = [[u32; 2]; 16];
type VertexFifo = [u32; 16];

#[derive(Clone, Copy)]
struct AuxEdge {
    first: u32,
    second: u32,
    opposite: u32,
}

type AuxEdgeFifo = [AuxEdge; 16];

fn get_edge_fifo(fifo: &EdgeFifo, a: u32, b: u32, c: u32, offset: usize) -> i32 {
    for i in 0..16 {
        let index = (offset.wrapping_sub(1).wrapping_sub(i)) & 15;
        let e0 = fifo[index][0];
        let e1 = fifo[index][1];
        if e0 == a && e1 == b {
            return (i as i32) << 2;
        }
        if e0 == b && e1 == c {
            return ((i as i32) << 2) | 1;
        }
        if e0 == c && e1 == a {
            return ((i as i32) << 2) | 2;
        }
    }
    -1
}

fn push_edge_fifo(fifo: &mut EdgeFifo, a: u32, b: u32, offset: &mut usize) {
    fifo[*offset][0] = a;
    fifo[*offset][1] = b;
    *offset = (*offset + 1) & 15;
}

fn push_aux_edge_fifo(fifo: &mut AuxEdgeFifo, edge: AuxEdge, offset: &mut usize) {
    fifo[*offset] = edge;
    *offset = (*offset + 1) & 15;
}

fn get_vertex_fifo(fifo: &VertexFifo, v: u32, offset: usize) -> i32 {
    for i in 0..16 {
        let index = (offset.wrapping_sub(1).wrapping_sub(i)) & 15;
        if fifo[index] == v {
            return i as i32;
        }
    }
    -1
}

fn push_vertex_fifo(fifo: &mut VertexFifo, v: u32, offset: &mut usize, cond: usize) {
    fifo[*offset] = v;
    *offset = (*offset + cond) & 15;
}

fn get_code_aux_index(v: u8, table: &[u8]) -> i32 {
    table
        .iter()
        .take(16)
        .position(|&x| x == v)
        .map_or(-1, |i| i as i32)
}

fn rotate_triangle(_a: u32, b: u32, c: u32, next: u32) -> usize {
    if b == next {
        1
    } else if c == next {
        2
    } else {
        0
    }
}

fn pack_aux_distances(low: u32, mid: u32, high: u32) -> u64 {
    (low as u64 & 0x3f_ffff) | ((mid as u64 & 0x1f_ffff) << 22) | ((high as u64 & 0x1f_ffff) << 43)
}

fn max3(a: u32, b: u32, c: u32) -> u32 {
    a.max(b).max(c)
}

fn checked_aux_index(index: u32, vertex_limit: usize) -> Result<usize> {
    let index = index as usize;
    if index >= vertex_limit {
        return Err(MeshoptError::Invalid(format!(
            "aux index {index} outside table length {vertex_limit}"
        )));
    }
    Ok(index)
}

fn record_fresh_aux_triangle(aux: &mut [u64], a: u32, b: u32, c: u32) -> Result<()> {
    let max_vertex = max3(a, b, c);
    let index = checked_aux_index(max_vertex, aux.len())?;
    if aux[index] & 0x3f_ffff != 0 {
        return Ok(());
    }

    let (first, second) = if max_vertex == a {
        (b, c)
    } else if max_vertex == b {
        (c, a)
    } else {
        (a, b)
    };
    aux[index] = pack_aux_distances(0, max_vertex - first, max_vertex - second);
    Ok(())
}

fn record_edge_aux_triangle(aux: &mut [u64], edge: AuxEdge, new_vertex: u32) -> Result<()> {
    let max_vertex = max3(edge.first, edge.second, new_vertex);
    let index = checked_aux_index(max_vertex, aux.len())?;
    if aux[index] & 0x3f_ffff != 0 {
        return Ok(());
    }

    aux[index] = if max_vertex == new_vertex {
        pack_aux_distances(
            new_vertex.saturating_sub(edge.opposite),
            new_vertex - edge.first,
            new_vertex - edge.second,
        )
    } else if max_vertex == edge.first {
        pack_aux_distances(0, edge.first - edge.second, edge.first - new_vertex)
    } else {
        pack_aux_distances(0, edge.second - new_vertex, edge.second - edge.first)
    };
    Ok(())
}

// ---------------------------------------------------------------------------
// Index buffer decode
// ---------------------------------------------------------------------------

/// Bounds-checked variable-byte read from a separate `data` stream.
fn decode_vbyte_checked(data: &[u8], p: &mut usize) -> Result<u32> {
    if *p >= data.len() {
        return Err(MeshoptError::Truncated {
            what: "index data stream",
            have: data.len(),
            need: *p + 1,
        });
    }
    let lead = data[*p];
    *p += 1;
    if lead < 128 {
        return Ok(lead as u32);
    }

    let mut result = (lead & 127) as u32;
    let mut shift = 7u32;
    for _ in 0..4 {
        if *p >= data.len() {
            return Err(MeshoptError::Truncated {
                what: "index data stream",
                have: data.len(),
                need: *p + 1,
            });
        }
        let group = data[*p];
        *p += 1;
        result |= ((group & 127) as u32) << shift;
        shift += 7;
        if group < 128 {
            break;
        }
    }
    Ok(result)
}

/// The shared meshoptimizer triangle-FIFO decode loop, reading the `code` and
/// `data` byte streams **separately** (the in-memory `0xe0` buffer keeps them
/// contiguous; TotK's MeshCodec hands them over as two slices). `codeaux_table`
/// is the 16-byte table used for `0xf0..0xfd` codes, and `fecmax` is the
/// vertex-FIFO/free-index boundary (`13` for format version 1, `15` for 0).
///
/// Returns the decoded index bytes plus how many `code` / `data` bytes were
/// consumed (so callers can validate exact stream consumption).
#[allow(clippy::too_many_arguments)]
fn decode_index_core(
    index_count: usize,
    index_size: usize,
    code: &[u8],
    data: &[u8],
    codeaux_table: &[u8],
    fecmax: i32,
) -> Result<(Vec<u8>, usize, usize)> {
    let mut edgefifo: EdgeFifo = [[u32::MAX; 2]; 16];
    let mut vertexfifo: VertexFifo = [u32::MAX; 16];
    let mut edgefifooffset = 0usize;
    let mut vertexfifooffset = 0usize;
    let mut next = 0u32;
    let mut last = 0u32;

    let mut out = vec![0u8; index_count * index_size];
    let mut cp = 0usize;
    let mut dp = 0usize;

    let mut i = 0;
    while i < index_count {
        if cp >= code.len() {
            return Err(MeshoptError::Truncated {
                what: "index code stream",
                have: code.len(),
                need: cp + 1,
            });
        }
        let codetri = code[cp];
        cp += 1;

        if codetri < 0xf0 {
            let fe = (codetri >> 4) as usize;
            let a = edgefifo[(edgefifooffset.wrapping_sub(1).wrapping_sub(fe)) & 15][0];
            let b = edgefifo[(edgefifooffset.wrapping_sub(1).wrapping_sub(fe)) & 15][1];
            let fec = (codetri & 15) as i32;

            if fec < fecmax {
                let cf =
                    vertexfifo[(vertexfifooffset.wrapping_sub(1).wrapping_sub(fec as usize)) & 15];
                let c = if fec == 0 { next } else { cf };
                let fec0 = (fec == 0) as u32;
                next = next.wrapping_add(fec0);

                write_triangle(&mut out, i, index_size, a, b, c);
                push_vertex_fifo(&mut vertexfifo, c, &mut vertexfifooffset, fec0 as usize);
                push_edge_fifo(&mut edgefifo, c, b, &mut edgefifooffset);
                push_edge_fifo(&mut edgefifo, a, c, &mut edgefifooffset);
            } else {
                let c = if fec != 15 {
                    last = last.wrapping_add((fec - (fec ^ 3)) as u32);
                    last
                } else {
                    last = decode_index_split_stream(data, &mut dp, last)?;
                    last
                };
                write_triangle(&mut out, i, index_size, a, b, c);
                push_vertex_fifo(&mut vertexfifo, c, &mut vertexfifooffset, 1);
                push_edge_fifo(&mut edgefifo, c, b, &mut edgefifooffset);
                push_edge_fifo(&mut edgefifo, a, c, &mut edgefifooffset);
            }
        } else if codetri < 0xfe {
            let codeaux = codeaux_table[(codetri & 15) as usize];
            let feb = (codeaux >> 4) as usize;
            let fec = (codeaux & 15) as usize;

            let a = next;
            next = next.wrapping_add(1);

            let bf = vertexfifo[(vertexfifooffset.wrapping_sub(feb)) & 15];
            let b = if feb == 0 { next } else { bf };
            let feb0 = (feb == 0) as u32;
            next = next.wrapping_add(feb0);

            let cf = vertexfifo[(vertexfifooffset.wrapping_sub(fec)) & 15];
            let c = if fec == 0 { next } else { cf };
            let fec0 = (fec == 0) as u32;
            next = next.wrapping_add(fec0);

            write_triangle(&mut out, i, index_size, a, b, c);
            push_vertex_fifo(&mut vertexfifo, a, &mut vertexfifooffset, 1);
            push_vertex_fifo(&mut vertexfifo, b, &mut vertexfifooffset, feb0 as usize);
            push_vertex_fifo(&mut vertexfifo, c, &mut vertexfifooffset, fec0 as usize);
            push_edge_fifo(&mut edgefifo, b, a, &mut edgefifooffset);
            push_edge_fifo(&mut edgefifo, c, b, &mut edgefifooffset);
            push_edge_fifo(&mut edgefifo, a, c, &mut edgefifooffset);
        } else {
            if dp >= data.len() {
                return Err(MeshoptError::Truncated {
                    what: "index data stream",
                    have: data.len(),
                    need: dp + 1,
                });
            }
            let codeaux = data[dp];
            dp += 1;
            let fea = if codetri == 0xfe { 0usize } else { 15 };
            let feb = (codeaux >> 4) as usize;
            let fec = (codeaux & 15) as usize;

            if codeaux == 0 {
                next = 0;
            }

            let mut a = if fea == 0 {
                let t = next;
                next = next.wrapping_add(1);
                t
            } else {
                0
            };
            let mut b = if feb == 0 {
                let t = next;
                next = next.wrapping_add(1);
                t
            } else {
                vertexfifo[(vertexfifooffset.wrapping_sub(feb)) & 15]
            };
            let mut c = if fec == 0 {
                let t = next;
                next = next.wrapping_add(1);
                t
            } else {
                vertexfifo[(vertexfifooffset.wrapping_sub(fec)) & 15]
            };

            if fea == 15 {
                last = decode_index_split_stream(data, &mut dp, last)?;
                a = last;
            }
            if feb == 15 {
                last = decode_index_split_stream(data, &mut dp, last)?;
                b = last;
            }
            if fec == 15 {
                last = decode_index_split_stream(data, &mut dp, last)?;
                c = last;
            }

            write_triangle(&mut out, i, index_size, a, b, c);
            push_vertex_fifo(&mut vertexfifo, a, &mut vertexfifooffset, 1);
            push_vertex_fifo(
                &mut vertexfifo,
                b,
                &mut vertexfifooffset,
                ((feb == 0) || (feb == 15)) as usize,
            );
            push_vertex_fifo(
                &mut vertexfifo,
                c,
                &mut vertexfifooffset,
                ((fec == 0) || (fec == 15)) as usize,
            );
            push_edge_fifo(&mut edgefifo, b, a, &mut edgefifooffset);
            push_edge_fifo(&mut edgefifo, c, b, &mut edgefifooffset);
            push_edge_fifo(&mut edgefifo, a, c, &mut edgefifooffset);
        }
        i += 3;
    }

    Ok((out, cp, dp))
}

/// Free-index delta decode against the running `last`, reading from a separate
/// (bounds-checked) `data` stream.
fn decode_index_split_stream(data: &[u8], p: &mut usize, last: u32) -> Result<u32> {
    let v = decode_vbyte_checked(data, p)?;
    let d = (v >> 1) ^ (0u32.wrapping_sub(v & 1));
    Ok(last.wrapping_add(d))
}

/// Decode a meshoptimizer index buffer (triangle list) of `index_count`
/// indices (`index_count % 3 == 0`), each `index_size` (2 or 4) bytes.
pub fn decode_index_buffer(
    index_count: usize,
    index_size: usize,
    buffer: &[u8],
) -> Result<Vec<u8>> {
    if !index_count.is_multiple_of(3) {
        return Err(MeshoptError::Invalid(format!(
            "index_count {index_count} must be a multiple of 3"
        )));
    }
    if index_size != 2 && index_size != 4 {
        return Err(MeshoptError::Invalid(format!(
            "index_size {index_size} must be 2 or 4"
        )));
    }
    let need = 1 + index_count / 3 + 16;
    if buffer.len() < need {
        return Err(MeshoptError::Truncated {
            what: "index buffer",
            have: buffer.len(),
            need,
        });
    }
    if (buffer[0] & 0xf0) != INDEX_HEADER {
        return Err(MeshoptError::BadHeader {
            what: "index",
            byte: buffer[0],
        });
    }
    let version = buffer[0] & 0x0f;
    if version > 1 {
        return Err(MeshoptError::BadHeader {
            what: "index",
            byte: buffer[0],
        });
    }
    let fecmax: i32 = if version >= 1 { 13 } else { 15 };

    // The in-memory layout keeps code, data and the codeaux table in one
    // buffer: code = [1 .. 1+n/3], data = [1+n/3 .. len-16], table = last 16.
    let code_start = 1;
    let data_start = 1 + index_count / 3;
    let data_safe_end = buffer.len() - 16;
    let code = &buffer[code_start..data_start];
    let data = &buffer[data_start..data_safe_end];
    let codeaux_table = &buffer[data_safe_end..data_safe_end + 16];

    let (out, _code_used, data_used) =
        decode_index_core(index_count, index_size, code, data, codeaux_table, fecmax)?;

    if data_used != data.len() {
        return Err(MeshoptError::ExtraBytes {
            what: "index buffer",
            leftover: data.len().abs_diff(data_used),
        });
    }
    Ok(out)
}

/// Decode a meshoptimizer index buffer from **split** `code` and `data` streams
/// — the form TotK's MeshCodec transport hands to its index decoder
/// (`code` = the per-triangle code bytes, `data` = the free-index/codeaux var
/// stream), using the standard codeaux table. `version` selects the FIFO
/// boundary (`0` ⇒ `fecmax=15`, as the MeshCodec streams use; `1` ⇒ `13`).
///
/// Unlike [`decode_index_buffer`] there is no header byte, no trailing codeaux
/// table, and no exact-consumption check — the streams may carry trailing bytes
/// belonging to a later sub-mesh, so only `index_count` indices are decoded.
pub fn decode_index_buffer_split(
    index_count: usize,
    index_size: usize,
    code: &[u8],
    data: &[u8],
    version: u8,
) -> Result<Vec<u8>> {
    decode_index_buffer_split_used(index_count, index_size, code, data, version)
        .map(|(out, _, _)| out)
}

/// Like [`decode_index_buffer_split`] but also returns how many `code` and
/// `data` bytes were consumed (`(indices, code_used, data_used)`).
///
/// The MeshCodec transport packs several sub-meshes back-to-back into one shared
/// `code`/`data` stream pair; the consumed counts let a caller decode the next
/// sub-mesh by resuming at `code[code_used..]` / `data[data_used..]`.
pub fn decode_index_buffer_split_used(
    index_count: usize,
    index_size: usize,
    code: &[u8],
    data: &[u8],
    version: u8,
) -> Result<(Vec<u8>, usize, usize)> {
    if !index_count.is_multiple_of(3) {
        return Err(MeshoptError::Invalid(format!(
            "index_count {index_count} must be a multiple of 3"
        )));
    }
    if index_size != 2 && index_size != 4 {
        return Err(MeshoptError::Invalid(format!(
            "index_size {index_size} must be 2 or 4"
        )));
    }
    if version > 1 {
        return Err(MeshoptError::Invalid(format!(
            "index version {version} > 1"
        )));
    }
    let fecmax: i32 = if version >= 1 { 13 } else { 15 };
    decode_index_core(
        index_count,
        index_size,
        code,
        data,
        &CODE_AUX_ENCODING_TABLE,
        fecmax,
    )
}

/// Decode the MeshCodec `0x110ca30` split index helper's aux-table side
/// effect. This is the compact sibling of [`decode_index_buffer_split_used`]:
/// it walks the same meshopt 0.15 triangle FIFO stream but materializes the
/// per-vertex three-distance table that DirectionZero state-5 predictor writers
/// read through `ctx+0x220`.
///
/// The observed `0x10fa980 -> 0x110ca30` calls use the version-0 FIFO boundary
/// and a zeroed aux table. The returned tuple is `(aux_table, code_used,
/// data_used)`.
pub fn decode_index_buffer_split_aux_table(
    index_count: usize,
    code: &[u8],
    data: &[u8],
    vertex_limit: usize,
) -> Result<(Vec<u64>, usize, usize)> {
    if !index_count.is_multiple_of(3) {
        return Err(MeshoptError::Invalid(format!(
            "index_count {index_count} must be a multiple of 3"
        )));
    }

    let mut edgefifo = [AuxEdge {
        first: u32::MAX,
        second: u32::MAX,
        opposite: u32::MAX,
    }; 16];
    let mut vertexfifo: VertexFifo = [u32::MAX; 16];
    let mut edgefifooffset = 0usize;
    let mut vertexfifooffset = 0usize;
    let mut next = 0u32;
    let mut last = 0u32;
    let mut aux = vec![0u64; vertex_limit];

    let mut cp = 0usize;
    let mut dp = 0usize;
    let mut i = 0usize;
    while i < index_count {
        if cp >= code.len() {
            return Err(MeshoptError::Truncated {
                what: "index code stream",
                have: code.len(),
                need: cp + 1,
            });
        }
        let codetri = code[cp];
        cp += 1;

        if codetri < 0xf0 {
            let edge = edgefifo[(edgefifooffset
                .wrapping_sub(1)
                .wrapping_sub((codetri >> 4) as usize))
                & 15];
            let fec = (codetri & 15) as i32;
            let c = if fec < 15 {
                let cf =
                    vertexfifo[(vertexfifooffset.wrapping_sub(1).wrapping_sub(fec as usize)) & 15];
                let c = if fec == 0 { next } else { cf };
                let fec0 = (fec == 0) as usize;
                next = next.wrapping_add(fec0 as u32);
                push_vertex_fifo(&mut vertexfifo, c, &mut vertexfifooffset, fec0);
                c
            } else {
                last = decode_index_split_stream(data, &mut dp, last)?;
                push_vertex_fifo(&mut vertexfifo, last, &mut vertexfifooffset, 1);
                last
            };

            push_aux_edge_fifo(
                &mut edgefifo,
                AuxEdge {
                    first: c,
                    second: edge.second,
                    opposite: edge.first,
                },
                &mut edgefifooffset,
            );
            push_aux_edge_fifo(
                &mut edgefifo,
                AuxEdge {
                    first: edge.first,
                    second: c,
                    opposite: edge.second,
                },
                &mut edgefifooffset,
            );
            record_edge_aux_triangle(&mut aux, edge, c)?;
        } else if codetri < 0xfe {
            let codeaux = CODE_AUX_ENCODING_TABLE[(codetri & 15) as usize];
            let feb = (codeaux >> 4) as usize;
            let fec = (codeaux & 15) as usize;

            let a = next;
            next = next.wrapping_add(1);

            let bf = vertexfifo[(vertexfifooffset.wrapping_sub(feb)) & 15];
            let b = if feb == 0 { next } else { bf };
            let feb0 = (feb == 0) as usize;
            next = next.wrapping_add(feb0 as u32);

            let cf = vertexfifo[(vertexfifooffset.wrapping_sub(fec)) & 15];
            let c = if fec == 0 { next } else { cf };
            let fec0 = (fec == 0) as usize;
            next = next.wrapping_add(fec0 as u32);

            push_vertex_fifo(&mut vertexfifo, a, &mut vertexfifooffset, 1);
            push_vertex_fifo(&mut vertexfifo, b, &mut vertexfifooffset, feb0);
            push_vertex_fifo(&mut vertexfifo, c, &mut vertexfifooffset, fec0);
            push_aux_edge_fifo(
                &mut edgefifo,
                AuxEdge {
                    first: b,
                    second: a,
                    opposite: c,
                },
                &mut edgefifooffset,
            );
            push_aux_edge_fifo(
                &mut edgefifo,
                AuxEdge {
                    first: c,
                    second: b,
                    opposite: a,
                },
                &mut edgefifooffset,
            );
            push_aux_edge_fifo(
                &mut edgefifo,
                AuxEdge {
                    first: a,
                    second: c,
                    opposite: b,
                },
                &mut edgefifooffset,
            );
            record_fresh_aux_triangle(&mut aux, a, b, c)?;
        } else {
            if dp >= data.len() {
                return Err(MeshoptError::Truncated {
                    what: "index data stream",
                    have: data.len(),
                    need: dp + 1,
                });
            }
            let codeaux = data[dp];
            dp += 1;
            let fea = if codetri == 0xfe { 0usize } else { 15 };
            let feb = (codeaux >> 4) as usize;
            let fec = (codeaux & 15) as usize;

            if codeaux == 0 {
                next = 0;
            }

            let mut a = if fea == 0 {
                let t = next;
                next = next.wrapping_add(1);
                t
            } else {
                0
            };
            let mut b = if feb == 0 {
                let t = next;
                next = next.wrapping_add(1);
                t
            } else {
                vertexfifo[(vertexfifooffset.wrapping_sub(feb)) & 15]
            };
            let mut c = if fec == 0 {
                let t = next;
                next = next.wrapping_add(1);
                t
            } else {
                vertexfifo[(vertexfifooffset.wrapping_sub(fec)) & 15]
            };

            if fea == 15 {
                last = decode_index_split_stream(data, &mut dp, last)?;
                a = last;
            }
            if feb == 15 {
                last = decode_index_split_stream(data, &mut dp, last)?;
                b = last;
            }
            if fec == 15 {
                last = decode_index_split_stream(data, &mut dp, last)?;
                c = last;
            }

            push_vertex_fifo(&mut vertexfifo, a, &mut vertexfifooffset, 1);
            push_vertex_fifo(
                &mut vertexfifo,
                b,
                &mut vertexfifooffset,
                ((feb == 0) || (feb == 15)) as usize,
            );
            push_vertex_fifo(
                &mut vertexfifo,
                c,
                &mut vertexfifooffset,
                ((fec == 0) || (fec == 15)) as usize,
            );
            push_aux_edge_fifo(
                &mut edgefifo,
                AuxEdge {
                    first: b,
                    second: a,
                    opposite: c,
                },
                &mut edgefifooffset,
            );
            push_aux_edge_fifo(
                &mut edgefifo,
                AuxEdge {
                    first: c,
                    second: b,
                    opposite: a,
                },
                &mut edgefifooffset,
            );
            push_aux_edge_fifo(
                &mut edgefifo,
                AuxEdge {
                    first: a,
                    second: c,
                    opposite: b,
                },
                &mut edgefifooffset,
            );
            record_fresh_aux_triangle(&mut aux, a, b, c)?;
        }

        i += 3;
    }

    Ok((aux, cp, dp))
}

// ---------------------------------------------------------------------------
// Index buffer encode
// ---------------------------------------------------------------------------

/// Encode a triangle-list index buffer (`indices`, `index_count % 3 == 0`)
/// with format `version` (0 or 1). Returns the meshoptimizer index stream.
pub fn encode_index_buffer(indices: &[u32], index_count: usize, version: u8) -> Result<Vec<u8>> {
    if !index_count.is_multiple_of(3) {
        return Err(MeshoptError::Invalid(format!(
            "index_count {index_count} must be a multiple of 3"
        )));
    }
    if version > 1 {
        return Err(MeshoptError::Invalid(format!(
            "index version {version} > 1"
        )));
    }
    if indices.len() < index_count {
        return Err(MeshoptError::Truncated {
            what: "index source",
            have: indices.len(),
            need: index_count,
        });
    }

    let mut edgefifo: EdgeFifo = [[u32::MAX; 2]; 16];
    let mut vertexfifo: VertexFifo = [u32::MAX; 16];
    let mut edgefifooffset = 0usize;
    let mut vertexfifooffset = 0usize;
    let mut next = 0u32;
    let mut last = 0u32;
    let fecmax: i32 = if version >= 1 { 13 } else { 15 };
    let table = &CODE_AUX_ENCODING_TABLE;

    let mut code: Vec<u8> = Vec::with_capacity(index_count / 3);
    let mut data: Vec<u8> = Vec::new();

    let mut i = 0;
    while i < index_count {
        let fer = get_edge_fifo(
            &edgefifo,
            indices[i],
            indices[i + 1],
            indices[i + 2],
            edgefifooffset,
        );

        if fer >= 0 && (fer >> 2) < 15 {
            let order = TRIANGLE_INDEX_ORDER[(fer & 3) as usize];
            let a = indices[i + order[0]];
            let b = indices[i + order[1]];
            let c = indices[i + order[2]];

            let fe = fer >> 2;
            let fc = get_vertex_fifo(&vertexfifo, c, vertexfifooffset);
            let mut fec = if (1..fecmax).contains(&fc) {
                fc
            } else if c == next {
                next = next.wrapping_add(1);
                0
            } else {
                15
            };

            if fec == 15 && version >= 1 {
                if c.wrapping_add(1) == last {
                    fec = 13;
                    last = c;
                }
                if c == last.wrapping_add(1) {
                    fec = 14;
                    last = c;
                }
            }

            code.push(((fe << 4) | fec) as u8);

            if fec == 15 {
                encode_index(&mut data, c, last);
                last = c;
            }
            if fec == 0 || fec >= fecmax {
                push_vertex_fifo(&mut vertexfifo, c, &mut vertexfifooffset, 1);
            }
            push_edge_fifo(&mut edgefifo, c, b, &mut edgefifooffset);
            push_edge_fifo(&mut edgefifo, a, c, &mut edgefifooffset);
        } else {
            let rotation = rotate_triangle(indices[i], indices[i + 1], indices[i + 2], next);
            let order = TRIANGLE_INDEX_ORDER[rotation];
            let a = indices[i + order[0]];
            let b = indices[i + order[1]];
            let c = indices[i + order[2]];

            let mut reset = false;
            if a == 0 && b == 1 && c == 2 && next > 0 && version >= 1 {
                reset = true;
                next = 0;
                vertexfifo = [u32::MAX; 16];
            }

            let fb = get_vertex_fifo(&vertexfifo, b, vertexfifooffset);
            let fc = get_vertex_fifo(&vertexfifo, c, vertexfifooffset);

            let fea = if a == next {
                next = next.wrapping_add(1);
                0i32
            } else {
                15
            };
            let feb = if (0..14).contains(&fb) {
                fb + 1
            } else if b == next {
                next = next.wrapping_add(1);
                0
            } else {
                15
            };
            let fec = if (0..14).contains(&fc) {
                fc + 1
            } else if c == next {
                next = next.wrapping_add(1);
                0
            } else {
                15
            };

            let codeaux = ((feb << 4) | fec) as u8;
            let codeauxindex = get_code_aux_index(codeaux, table);

            if fea == 0 && (0..14).contains(&codeauxindex) && !reset {
                code.push(((15 << 4) | codeauxindex) as u8);
            } else {
                code.push(((15 << 4) | 14 | fea) as u8);
                data.push(codeaux);
            }

            if fea == 15 {
                encode_index(&mut data, a, last);
                last = a;
            }
            if feb == 15 {
                encode_index(&mut data, b, last);
                last = b;
            }
            if fec == 15 {
                encode_index(&mut data, c, last);
                last = c;
            }

            if fea == 0 || fea == 15 {
                push_vertex_fifo(&mut vertexfifo, a, &mut vertexfifooffset, 1);
            }
            if feb == 0 || feb == 15 {
                push_vertex_fifo(&mut vertexfifo, b, &mut vertexfifooffset, 1);
            }
            if fec == 0 || fec == 15 {
                push_vertex_fifo(&mut vertexfifo, c, &mut vertexfifooffset, 1);
            }
            push_edge_fifo(&mut edgefifo, b, a, &mut edgefifooffset);
            push_edge_fifo(&mut edgefifo, c, b, &mut edgefifooffset);
            push_edge_fifo(&mut edgefifo, a, c, &mut edgefifooffset);
        }
        i += 3;
    }

    let mut out = Vec::with_capacity(1 + code.len() + data.len() + 16);
    out.push(INDEX_HEADER | version);
    out.extend_from_slice(&code);
    out.extend_from_slice(&data);
    out.extend_from_slice(table);
    Ok(out)
}

// ---------------------------------------------------------------------------
// Index sequence
// ---------------------------------------------------------------------------

/// Decode a meshoptimizer index *sequence* (`0xd0`) of `index_count` indices.
pub fn decode_index_sequence(
    index_count: usize,
    index_size: usize,
    buffer: &[u8],
) -> Result<Vec<u8>> {
    if index_size != 2 && index_size != 4 {
        return Err(MeshoptError::Invalid(format!(
            "index_size {index_size} must be 2 or 4"
        )));
    }
    let need = 1 + index_count + 4;
    if buffer.len() < need {
        return Err(MeshoptError::Truncated {
            what: "index sequence",
            have: buffer.len(),
            need,
        });
    }
    if (buffer[0] & 0xf0) != SEQUENCE_HEADER {
        return Err(MeshoptError::BadHeader {
            what: "index sequence",
            byte: buffer[0],
        });
    }
    if (buffer[0] & 0x0f) > 1 {
        return Err(MeshoptError::BadHeader {
            what: "index sequence",
            byte: buffer[0],
        });
    }

    let mut out = vec![0u8; index_count * index_size];
    let data_safe_end = buffer.len() - 4;
    let mut last = [0u32; 2];
    let mut p = 1usize;
    for i in 0..index_count {
        if p >= data_safe_end {
            return Err(MeshoptError::Truncated {
                what: "index sequence (data underrun)",
                have: data_safe_end,
                need: p + 1,
            });
        }
        let mut v = decode_vbyte(buffer, &mut p);
        let current = (v & 1) as usize;
        v >>= 1;
        let d = (v >> 1) ^ (0u32.wrapping_sub(v & 1));
        let index = last[current].wrapping_add(d);
        last[current] = index;
        put_index(&mut out, i, index_size, index);
    }
    if p != data_safe_end {
        return Err(MeshoptError::ExtraBytes {
            what: "index sequence",
            leftover: data_safe_end.abs_diff(p),
        });
    }
    Ok(out)
}

/// Encode an index sequence (`version` 0 or 1).
pub fn encode_index_sequence(indices: &[u32], index_count: usize, version: u8) -> Result<Vec<u8>> {
    if version > 1 {
        return Err(MeshoptError::Invalid(format!(
            "sequence version {version} > 1"
        )));
    }
    if indices.len() < index_count {
        return Err(MeshoptError::Truncated {
            what: "index sequence source",
            have: indices.len(),
            need: index_count,
        });
    }
    let mut data = Vec::with_capacity(1 + index_count + 4);
    data.push(SEQUENCE_HEADER | version);

    let mut last = [0u32; 2];
    let mut current = 0usize;
    for &index in indices.iter().take(index_count) {
        let cd = index.wrapping_sub(last[current]) as i32;
        current ^= (cd.unsigned_abs() >= 30) as usize;
        let d = index.wrapping_sub(last[current]);
        let v = (d << 1) ^ (((d as i32) >> 31) as u32);
        encode_vbyte(&mut data, (v << 1) | current as u32);
        last[current] = index;
    }
    data.extend_from_slice(&[0u8; 4]);
    Ok(data)
}
