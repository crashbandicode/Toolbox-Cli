use crate::mc::{McError, MeshSection};
use crate::meshopt;
use crate::zstd_pure;
pub struct ForwardReader<'a> {
    buf: &'a [u8],
    /// Current read position.
    pub pos: usize,
}

impl<'a> ForwardReader<'a> {
    /// Start a forward cursor at `pos`.
    pub fn new(buf: &'a [u8], pos: usize) -> Self {
        Self { buf, pos }
    }

    /// Read one byte, advancing the cursor.
    pub fn byte(&mut self) -> Option<u8> {
        let b = self.buf.get(self.pos).copied();
        if b.is_some() {
            self.pos += 1;
        }
        b
    }

    /// Read an MSB-first base-128 var-int (`acc = (acc << 7) | (byte & 0x7f)`,
    /// continue while the high bit is set), advancing the cursor.
    pub fn varint(&mut self) -> u32 {
        let mut v: u32 = 0;
        while let Some(b) = self.byte() {
            v = (v << 7) | (b & 0x7f) as u32;
            if b & 0x80 == 0 {
                break;
            }
        }
        v
    }
}

/// Read 8 little-endian bytes at `ptr` (zero-padded past the end) — the backing
/// load for the reverse bit reader.
#[inline]
pub(super) fn u64_le(buf: &[u8], ptr: usize) -> u64 {
    let mut b = [0u8; 8];
    let n = buf.len().saturating_sub(ptr).min(8);
    if n > 0 {
        b[..n].copy_from_slice(&buf[ptr..ptr + n]);
    }
    u64::from_le_bytes(b)
}

/// Read one LSB-first LEB128 from `buf` at `pos`, returning `(value, new_pos)`.
pub(super) fn read_leb_lsb(buf: &[u8], mut pos: usize) -> (u64, usize) {
    let mut v: u64 = 0;
    let mut shift = 0u32;
    while pos < buf.len() {
        let b = buf[pos];
        pos += 1;
        v |= ((b & 0x7f) as u64) << shift;
        shift += 7;
        if b & 0x80 == 0 {
            break;
        }
    }
    (v, pos)
}

/// The super-block trailer: two LSB-LEB128 sizes at the start of the payload.
/// `(0, 0)` ⇒ this is the only / last super-block.
pub fn parse_super_block_trailer(payload: &[u8]) -> (u64, u64, usize) {
    let (a, p) = read_leb_lsb(payload, 0);
    let (b, p) = read_leb_lsb(payload, p);
    (a, b, p)
}

/// A parsed sub-block header (`0x10f9570`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubBlockHeader {
    /// Number of sub-meshes / attributes in this sub-block.
    pub count: u32,
    /// Low nibble of the descriptor byte.
    pub a: u8,
    /// High nibble of the descriptor byte.
    pub b: u8,
    /// Var-int `c`.
    pub c: u32,
    /// Var-int `d` (e.g. the first sub-mesh's index-count source).
    pub d: u32,
    /// Var-int `e`.
    pub e: u32,
    /// Derived first-sub-mesh index count: `d >> (b == 1 ? 1 : 0)`.
    pub f: u32,
}

/// Parse a sub-block header from the forward cursor. Returns `None` when
/// `count == 0` (the end-of-super-block marker).
pub fn parse_sub_block_header(fwd: &mut ForwardReader) -> Option<SubBlockHeader> {
    let count = fwd.varint();
    if count == 0 {
        return None;
    }
    let m = fwd.byte().unwrap_or(0);
    let a = m & 0xf;
    let b = m >> 4;
    let c = fwd.varint();
    let d = fwd.varint();
    let e = fwd.varint();
    let f = d >> u32::from(b == 1);
    Some(SubBlockHeader {
        count,
        a,
        b,
        c,
        d,
        e,
        f,
    })
}

/// Result of the state-0 canonical-Huffman table builder (`0x10f8d20`) — the
/// updated cursor positions, the parsed scalars, and (when `symbols != 0`) the
/// built decode table. The table drives the vertex byte-group coder; for the
/// index path only the cursor advance matters (validated byte-exact).
#[derive(Debug, Clone)]
pub struct TableBuild {
    /// New forward cursor position.
    pub fwd: usize,
    /// New reverse-A pointer (payload offset).
    pub rev_ptr: usize,
    /// New reverse-A accumulator.
    pub rev_acc: u64,
    /// New reverse-A bit position.
    pub rev_bitpos: u32,
    /// The forward var-int `w8` (block-size hint, e.g. vertex count 3327).
    pub w8: u32,
    /// Number of canonical-Huffman symbols (`w17`).
    pub symbols: u32,
    /// The trailing direction bit (`ctx[0x118]`; 0 ⇒ index path).
    pub dir_bit: u32,
    /// `ctx+0x240`: per-symbol packed entry (`0x10fb2e0` reads byte-count/type/
    /// bits/forward-byte from this).
    pub entries: Vec<u32>,
    /// `ctx+0x27c`: per-symbol bufB byte offset (where this attribute writes).
    pub offsets: Vec<u32>,
    /// `ctx+0x310`: per-symbol column offset within its vertex stream.
    pub cols: Vec<u8>,
    /// `ctx+0x2d4`: long-symbol markers (`(index<<16) | (w5<<8) | w18`).
    pub longs: Vec<u32>,
    /// `ctx+0x2c8`: total byte-group-decoded size (bufB minus the direct tail).
    pub byte_group_total: u32,
    /// `ctx+0x2d0`: max `w24*w26` product over symbols.
    pub max_prod: u32,
}

/// Port of state 0's continuation (`0x10f8cc8..0x10f9028`): read the forward
/// var-int `w8`, then build the canonical-Huffman table (`0x10f8d20`: a 4-bit
/// symbol count from reverse-A, then `symbols` entries of 11 reverse-A bits +
/// conditional forward byte-pairs), then one direction bit. Returns the advanced
/// cursors **and** the table (validated byte-exact against the decoder).
///
/// `rev_ptr` is the reverse-A pointer (payload offset; the decoder seeds it at
/// `sub_a_size - 8`); `w13` is the decoder's `ctx[0x2c0]` (an alignment-like
/// constant — `7` across the model fixtures — used only for the table values,
/// not the cursor advance, so the index path may pass `0`).
pub fn state0_table_builder(
    payload: &[u8],
    fwd: usize,
    rev_ptr: usize,
    rev_acc: u64,
    rev_bitpos: u32,
    w13: u32,
) -> TableBuild {
    let w12 = rev_bitpos;
    let mut x11 = rev_ptr;
    let x15 = u64_le(payload, x11) >> w12;
    let w14 = (w12 >> 3) ^ 7;

    // forward var-int -> w8
    let mut f = ForwardReader::new(payload, fwd);
    let w8 = f.varint();
    let mut x12 = f.pos;

    let x9 = x15 | rev_acc;
    let w15 = w12 | 0x38;
    x11 = x11.wrapping_sub(w14 as usize);
    let symbols = ((x9 >> 0x3b) & 0xf) as u32;
    let mut x14 = x9 << 5;
    let mut w10 = w15.wrapping_sub(5);

    let mut entries = Vec::new();
    let mut offsets = Vec::new();
    let mut cols = Vec::new();
    let mut longs = Vec::new();
    let mut byte_group_total = 0u32;
    let mut max_prod = 0u32;

    if symbols != 0 {
        // 0x10f8d20 value loop.
        let w0 = !w13; // ~ctx[0x2c0]
        let mut w16 = 0u32; // ctx[0x2c8] running offset
        let mut w1 = 0u32; // (index << 16)
        let byte_at = |p: usize| payload.get(p).copied().unwrap_or(0) as u32;
        let mut w18 = byte_at(x12);
        let mut w5 = byte_at(x12 + 1);
        x12 += 2;
        let mut w7 = 0u32;
        let mut w4 = 0u32;
        for _ in 0..symbols {
            x14 |= u64_le(payload, x11) >> w10;
            let w19p = ((x14 >> 0x3e) & 3) as u32; // packing + w23m (top 2 bits)
            let w19b = ((x14 >> 0x37) & 0x1ff) as u32; // branch (bit1) + reset (bit0)
            let x24 = ((x14 >> 0x39) & 0x1f) as u32;
            let x26 = ((x14 >> 0x35) & 3) as u32;
            let w23m = 0u32.wrapping_sub(8) << w19p; // -8 << w19p
            let w24 = x24 + 1;
            let w26 = x26 + 1;
            let bic = w7 & !w23m;
            let w23 = w4.wrapping_add(((w23m & w7) as i32 >> 3) as u32);
            let w25 = (bic << 16) | (w24 << 8) | (w19p << 3) | (w18 << 24) | w26;
            let w28 = w23.wrapping_add(w16);
            entries.push(w25);
            cols.push(w23 as u8);
            offsets.push(w28);
            let wmul = w24.wrapping_mul(w26);
            max_prod = max_prod.max(wmul);
            if w19b & 2 != 0 {
                // long symbol (0x10f8e28)
                longs.push(w1 | (w5 << 8) | w18);
                w5 = w5.wrapping_add(w13);
                w4 = 0;
                w7 = 0;
                w18 = (w18.wrapping_mul(w8).wrapping_add(w5)) & w0;
                w16 = w16.wrapping_add(w18);
                w18 = byte_at(x12);
                w5 = byte_at(x12 + 1);
                x12 += 2;
            } else {
                // short symbol (0x10f8d70)
                w7 = wmul.wrapping_add(w7);
                let bias = if (w7 as i32) < 0 { 14 } else { 7 };
                let w23s = w4.wrapping_add((w7.wrapping_add(bias) as i32 >> 3) as u32);
                if w19b & 1 != 0 {
                    w7 = 0;
                    w4 = w23s;
                }
            }
            let pre = w10;
            w10 = (w10 | 0x38).wrapping_sub(11);
            x14 <<= 11;
            x11 = x11.wrapping_sub(((pre >> 3) ^ 7) as usize);
            w1 = w1.wrapping_add(0x10000);
        }
        // done (0x10f8e68): final long marker + ctx[0x2c8] total.
        longs.push(((symbols << 16).wrapping_sub(0x10000)) | (w5 << 8) | w18);
        let w13f = ((w18.wrapping_mul(w8)).wrapping_add(w5.wrapping_add(w13))) & w0;
        byte_group_total = w16.wrapping_add(w13f);
    }

    // final direction bit (0x10f900c..0x10f9028)
    let dir_bit = (x14 >> 63) as u32 & 1;
    x14 <<= 1;
    w10 = w10.wrapping_sub(1);

    TableBuild {
        fwd: x12,
        rev_ptr: x11,
        rev_acc: x14,
        rev_bitpos: w10,
        w8,
        symbols,
        dir_bit,
        entries,
        offsets,
        cols,
        longs,
        byte_group_total,
        max_prod,
    }
}

/// Decode a **zstd window**: read the forward var-int `srcsize`, then decode the
/// `srcsize`-byte zstd **block content** (a literals section followed by a
/// sequences section — the sequences RLE/back-reference-expand the literals to
/// the regenerated size) at the cursor. Returns the regenerated bytes and the
/// new forward position (advanced by `srcsize`).
///
/// Unlike a standalone literals block, the window's regenerated size is *not*
/// the literals header's value — it is driven by the sequences, exactly as the
/// decoder's `0x5ffb90` (block-content decode with a `0x20000` ceiling) does.
pub fn decode_zstd_window(
    payload: &[u8],
    fwd_pos: usize,
) -> Result<(Vec<u8>, usize), zstd_pure::ZstdError> {
    decode_zstd_window_with_history(payload, fwd_pos, &[])
}

/// Decode a zstd window whose block may refer backward into already-decoded
/// bytes. This is the state-2 direct-tail case: `0x11109e0` still calls the
/// same `0x5ffb30` block decoder, but its DCtx history already contains the
/// byte-group bufB prefix, so sequence offsets may reach before the window's own
/// first literal.
pub fn decode_zstd_window_with_history(
    payload: &[u8],
    fwd_pos: usize,
    history: &[u8],
) -> Result<(Vec<u8>, usize), zstd_pure::ZstdError> {
    let (src_start, body) = zstd_window_body(payload, fwd_pos)?;
    let history_len = history.len();
    let mut state = zstd_pure::block::BlockState {
        out: history.to_vec(),
        dict_len: history_len,
        max_output: 0x20000,
        huff: None,
        seq: zstd_pure::sequences::SeqTables::default(),
        rep: [1, 4, 8],
    };
    state.decode_compressed(body)?;
    let mut out = state.out;
    out.drain(..history_len);
    if out.len() > 0x20000 {
        return Err(zstd_pure::ZstdError::OutputTooLarge { limit: 0x20000 });
    }
    Ok((out, src_start + body.len()))
}

fn zstd_window_body(
    payload: &[u8],
    fwd_pos: usize,
) -> Result<(usize, &[u8]), zstd_pure::ZstdError> {
    let mut pos = fwd_pos;
    let mut srcsize = 0usize;
    for _ in 0..5 {
        let byte = payload
            .get(pos)
            .copied()
            .ok_or(zstd_pure::ZstdError::Truncated {
                what: "zstd window srcsize",
                needed: 1,
            })?;
        pos += 1;
        srcsize = srcsize
            .checked_shl(7)
            .and_then(|value| value.checked_add((byte & 0x7f) as usize))
            .ok_or_else(|| zstd_pure::ZstdError::Invalid {
                what: "zstd window srcsize",
                detail: "overflow".into(),
            })?;
        if byte & 0x80 == 0 {
            let end = pos
                .checked_add(srcsize)
                .ok_or_else(|| zstd_pure::ZstdError::Invalid {
                    what: "zstd window body",
                    detail: "offset overflow".into(),
                })?;
            let body = payload
                .get(pos..end)
                .ok_or(zstd_pure::ZstdError::Truncated {
                    what: "zstd window body",
                    needed: end.saturating_sub(payload.len()),
                })?;
            return Ok((pos, body));
        }
    }
    Err(zstd_pure::ZstdError::Invalid {
        what: "zstd window srcsize",
        detail: "oversized MSB varint".into(),
    })
}

/// Decode a **raw window**: read the forward var-int `srcsize`, then copy
/// `srcsize` literal bytes. Returns the bytes and the new forward position.
pub fn decode_raw_window(payload: &[u8], fwd_pos: usize) -> (Vec<u8>, usize) {
    let mut f = ForwardReader::new(payload, fwd_pos);
    let srcsize = f.varint() as usize;
    let src_start = f.pos;
    let end = (src_start + srcsize).min(payload.len());
    (payload[src_start..end].to_vec(), src_start + srcsize)
}

/// Decode the **index sub-meshes of the first sub-block** of a model `.mc`,
/// returning their assembled slice of the index buffer (buffer A).
///
/// This chains the validated transport from scratch: the super-block trailer,
/// then `w27`, the sub-block header, the state-0 table builder, the code window
/// (zstd block content) and data window (raw), then a per-sub-mesh index decode
/// via [`meshopt::decode_index_buffer_split_used`]. The first sub-mesh's count is
/// the header's `f`; each subsequent one reads a transform-loop var-int, and the
/// sub-meshes sit at `align_a`-aligned offsets.
///
/// It is the proven end-to-end index path. The per-window raw/zstd flag is
/// inferred from this validated layout (code = zstd, data = raw) rather than read
/// from the reverse-A bitstream, and only the first sub-block is decoded (later
/// sub-blocks follow the vertex states, which are not yet ported); both are
/// tracked in `local-assets/re/FINDINGS.md`.
pub fn decode_first_subblock_indices(
    section: &MeshSection,
    payload: &[u8],
) -> Result<Vec<u8>, McError> {
    let map_z = |e: zstd_pure::ZstdError| McError::MeshFraming(format!("window zstd: {e}"));
    let map_m = |e: meshopt::MeshoptError| McError::MeshFraming(format!("index decode: {e}"));

    let sub_a = section.first_chunk.sub_a_size as usize;
    let align_a = (section.align_a as usize).max(1);
    let (t0, t1, pos) = parse_super_block_trailer(payload);
    if (t0, t1) != (0, 0) {
        return Err(McError::MeshFraming(format!(
            "unexpected super-block trailer ({t0},{t1}); multi-super-block not yet supported"
        )));
    }
    let mut fwd = ForwardReader::new(payload, pos);
    let _w27 = fwd.varint();
    let hdr = parse_sub_block_header(&mut fwd)
        .ok_or_else(|| McError::MeshFraming("empty first sub-block".into()))?;

    // `w13 = 0`: the table values are unused on the index path (only the cursor
    // advance matters, which is `w13`-independent).
    let tb = state0_table_builder(payload, fwd.pos, sub_a.wrapping_sub(8), 0, 0, 0);

    // State 3 decodes the shared code (zstd) + data (raw) streams; the forward
    // cursor (`fwd`) then carries each subsequent sub-mesh's transform header.
    let (code, p2) = decode_zstd_window(payload, tb.fwd).map_err(map_z)?;
    let (data, p3) = decode_raw_window(payload, p2);
    fwd.pos = p3;

    let mut buf_a = Vec::new();
    let mut code_off = 0usize;
    let mut data_off = 0usize;
    for i in 0..hdr.count {
        let count = if i == 0 {
            hdr.f
        } else {
            // transform-loop per-sub-mesh header (0x10f982c): nibble + v20 + v28
            // (the index count) + v4.
            let _nibble = fwd.byte();
            let _v20 = fwd.varint();
            let v28 = fwd.varint();
            let _v4 = fwd.varint();
            v28
        } as usize;
        // sub-meshes are align_a-aligned within buffer A.
        while !buf_a.len().is_multiple_of(align_a) {
            buf_a.push(0);
        }
        let (out, cu, du) = meshopt::decode_index_buffer_split_used(
            count,
            2,
            &code[code_off..],
            &data[data_off..],
            0,
        )
        .map_err(map_m)?;
        code_off += cu;
        data_off += du;
        buf_a.extend_from_slice(&out);
    }
    Ok(buf_a)
}
