//! TotK MeshCodec **geometry transport** — the custom entropy framing that wraps
//! the meshoptimizer geometry streams inside an `FMSH` chunk.
//!
//! The `FMSH` payload (see [`super::mesh`]) is **not** a stock meshoptimizer
//! stream; it is Nintendo's `NintendoWare_Meshoptimizer_For_MeshCodec` custom
//! transport: a streaming state machine with a canonical-Huffman table, a
//! forward var-int cursor + dual reverse (MSB-first / `clz`) bit readers, and
//! zstd-literals / raw "windows" that carry the decompressed meshopt code/data
//! streams. The geometry transforms underneath are stock meshopt (index FIFO in
//! [`crate::meshopt`]; vertex byte-group delta/zig-zag/transpose).
//!
//! This module ports the **transport framing primitives** that are fully
//! reverse-engineered and validated byte-exact against the decoder (the index
//! path's super-block → sub-block header → state-0 table builder → window
//! location → `decode_index_buffer_split` chain reproduces the oracle's index
//! buffer). They are the foundation the full streaming decoder is built on.
//!
//! ## Transport layout (validated)
//!
//! ```text
//! payload = FMSH + 0x22                       (chunk payload, sub_a then sub_b)
//! [super-block trailer: 2 LSB-LEB128 sizes]   (0,0) = single/last super-block
//! [w27 = sub-block count (forward var-int)]
//! per sub-block:
//!   [header: count + nibble(a,b) + var-ints c,d,e]   (0x10f9570)
//!   [forward var-int w8 = block-size hint]
//!   [canonical-Huffman table: w17 symbols]            (0x10f8d20; reverse-A bits)
//!   [1 direction bit]                                 (reverse-A)
//!   index sub-blocks  -> per sub-mesh: locate code+data windows, decode_index
//!   vertex sub-blocks -> custom byte-group coder (TODO: 0x10fb2e0 + transform)
//! ```
//!
//! Each **window** is located by a forward var-int = `srcsize`; a single
//! reverse-bit flag selects **raw** (copy `srcsize` bytes) vs **zstd** (a
//! [`crate::zstd_pure::literals`] block whose own header gives the regenerated
//! size). The forward cursor always advances by `srcsize`.
//!
//! ## What is NOT yet ported
//!
//! Threading the reverse-A reader through the per-window raw/zstd flag bits,
//! the segment loop (`0x110dc30`), and the kernel (`0x10fa980`) + vertex
//! byte-group transform (`0x10fb2e0`). Tracked in `local-assets/re/FINDINGS.md`.

use crate::meshopt;
use crate::zstd_pure;

/// A forward byte / MSB-first base-128 var-int cursor over the payload.
#[derive(Debug, Clone)]
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
fn u64_le(buf: &[u8], ptr: usize) -> u64 {
    let mut b = [0u8; 8];
    let n = buf.len().saturating_sub(ptr).min(8);
    if n > 0 {
        b[..n].copy_from_slice(&buf[ptr..ptr + n]);
    }
    u64::from_le_bytes(b)
}

/// Read one LSB-first LEB128 from `buf` at `pos`, returning `(value, new_pos)`.
fn read_leb_lsb(buf: &[u8], mut pos: usize) -> (u64, usize) {
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
    let mut f = ForwardReader::new(payload, fwd_pos);
    let srcsize = f.varint() as usize;
    let src_start = f.pos;
    let end = (src_start + srcsize).min(payload.len());
    let mut state = zstd_pure::block::BlockState {
        out: Vec::new(),
        dict_len: 0,
        max_output: 0x20000,
        huff: None,
        seq: zstd_pure::sequences::SeqTables::default(),
        rep: [1, 4, 8],
    };
    state.decode_compressed(&payload[src_start..end])?;
    Ok((state.out, src_start + srcsize))
}

/// Decode `count` symbols with the vertex coder's **rANS** decoder (`0x110e270`).
///
/// The custom vertex byte-group entropy is a standard range-ANS coder: `M =
/// 1 << log` states, a decode table of `step[idx] = (freq << 16) | (idx -
/// cumfreq)` and a spread map `sym[idx]` (both length `M`, built by `0x110de80`),
/// the step `state = (state >> log) * freq + low`, and a 32-bit forward renorm
/// whenever `state < 2^31`. Four states are interleaved (decoding output
/// positions `0, stride, 2*stride, 3*stride` per round, where `stride = w8`);
/// any `count % 4` tail is decoded with state 0. `stream` is read forward as
/// little-endian `u32`s for renormalization.
///
/// Built rANS decode tables (`step` at `+0`, `sym` at `+0x2000` in the segment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RansDecodeTable {
    pub log: u32,
    pub step: Vec<u32>,
    pub sym: Vec<u16>,
}

/// Contiguous spread into decode tables (`0x110e6f8`..`0x110e7a4` after `0x110e7b0`).
///
/// For each symbol `s` with frequency `f`, slots `[cum..cum+f)` map to `sym[i]=s` and
/// `step[i]=(f<<16)|(i-cum)` (slot index within the symbol's range, not global `i`).
/// Zero-frequency symbols are skipped. `symbol_freqs` must sum to `1 << log`.
pub fn rans_spread(log: u32, symbol_freqs: &[u16]) -> RansDecodeTable {
    let m = 1usize << log;
    let mut step = vec![0u32; m];
    let mut sym = vec![0u16; m];
    let mut cum = 0usize;
    for (s, &f) in symbol_freqs.iter().enumerate() {
        let f = f as usize;
        if f == 0 {
            continue;
        }
        let hi = (f as u32) << 16;
        for i in 0..f {
            sym[cum + i] = s as u16;
            step[cum + i] = hi | (i as u32);
        }
        cum += f;
    }
    debug_assert_eq!(cum, m);
    RansDecodeTable { log, step, sym }
}

/// Errors from four-lane rANS state initialization (`0x110dfa0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RansInitError {
    /// `prod < 4` hits the scalar path (`0x110e140`) — not yet traced for commit.
    ProdTooSmall,
    /// `prod & 3 != 0` tail uses `0x110e128` — not yet traced for commit.
    UnsupportedProdTail,
    /// `step`/`sym` length must equal `1 << table.log`.
    TableSizeMismatch,
    /// Forward renorm at `0x110e05c` could not read four bytes for a lane.
    StreamTooShort,
}

/// Mutable state buffer used by `0x110dfa0`.
///
/// The first four lanes are the rANS states at `x0+0..0x18`; `flag` is the word
/// at `x0+0x20`. When `flag & 0xf != 0xf`, the cold loader at `0x110e1bc` reads
/// the missing lane states from the forward stream, stores them low-lane first,
/// and sets `flag |= 0xf` at `0x110e264`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RansStateBuffer {
    pub states: [u64; 4],
    pub flag: u32,
}

impl RansStateBuffer {
    /// A warm buffer whose four states are already loaded.
    pub fn warm(states: [u64; 4]) -> Self {
        Self { states, flag: 0xf }
    }

    /// A cold buffer matching the first call on a stream (`flag == 0`).
    pub fn cold() -> Self {
        Self {
            states: [0; 4],
            flag: 0,
        }
    }
}

/// Shared forward stream cursor (`[x2+12]`) for one stream descriptor.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RansStreamCursor {
    pub offset: usize,
}

/// Result of [`rans_init_states`]: final four states and forward bytes consumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RansInitResult {
    pub states: [u64; 4],
    pub flag: u32,
    /// Absolute stream offset after the call (`[x2+12]` on return).
    pub stream_offset: usize,
    /// Bytes consumed by this call from the stream cursor.
    pub stream_used: usize,
}

fn read_stream_byte(stream: &[u8], cursor: &mut RansStreamCursor) -> Result<u8, RansInitError> {
    let byte = stream
        .get(cursor.offset)
        .copied()
        .ok_or(RansInitError::StreamTooShort)?;
    cursor.offset += 1;
    Ok(byte)
}

fn read_stream_u32(stream: &[u8], cursor: &mut RansStreamCursor) -> Result<u32, RansInitError> {
    let end = cursor
        .offset
        .checked_add(4)
        .ok_or(RansInitError::StreamTooShort)?;
    let chunk = stream
        .get(cursor.offset..end)
        .ok_or(RansInitError::StreamTooShort)?;
    cursor.offset = end;
    Ok(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
}

fn load_cold_rans_states(
    stream: &[u8],
    state: &mut RansStateBuffer,
    cursor: &mut RansStreamCursor,
) -> Result<(), RansInitError> {
    let mut missing = 0xf & !state.flag;
    while missing != 0 {
        let head = read_stream_byte(stream, cursor)?;
        let extra = (head & 0xf) as usize;
        let mut value = (head >> 4) as u64;
        for _ in 0..extra {
            value = (value << 8) | read_stream_byte(stream, cursor)? as u64;
        }

        let lane_bit = missing & missing.wrapping_neg();
        let lane = lane_bit.trailing_zeros() as usize;
        state.states[lane] = value + 0x8000_0000;
        missing ^= lane_bit;
    }
    state.flag |= 0xf;
    Ok(())
}

/// Advance four interleaved rANS states with the generic `0x110dfa0` primitive.
///
/// Each round, for lanes 0..3: index `state & mask`, take the spread entry, emit
/// `table.sym[idx]` (`ldrh`/`strb`), apply the range step `state = (state >> log)
/// * freq + low` (`lsr`/`mul`/`add` from `table.step`), then renormalize lanes
/// 0..3 in cascade order when `state < 2^31` by pulling a forward `u32`
/// (`0x110e05c`..`0x110e10c`). States are written back at `0x110e120`. The
/// `(log, table)` are per-segment data from the freq-reader + spread
/// (`0x110de80`) — there is no fixed init table.
///
/// The cold loader is `0x110e1bc`: the `bics w10,w10,w9` + `b.ne` at
/// `0x110dfbc..0x110dfc4` proves it runs when `(flag & 0xf) != 0xf`. The loader
/// reads one varint per missing lane (low nibble = extra byte count, high nibble
/// = initial value, extra bytes appended big-endian), adds `0x80000000`, stores
/// lanes selected by `w10 & -w10`, then sets `flag |= 0xf` at `0x110e264`.
///
/// `cursor.offset` is the shared forward stream offset (`[x2+12]`): entry adds
/// it to the base pointer (`0x110dfac..0x110dfb8`), and return writes back
/// `stream_pos - base` (`0x110e1a0..0x110e1a8`).
///
/// Still guarded because the captured population does not exercise it: scalar
/// `prod < 4` (`0x110e140`) and `prod & 3` tail (`0x110e128`).
pub fn rans_init_states_with_cursor(
    table: &RansDecodeTable,
    stream: &[u8],
    prod: u32,
    _stride: usize,
    state: &mut RansStateBuffer,
    cursor: &mut RansStreamCursor,
) -> Result<RansInitResult, RansInitError> {
    let m = 1usize << table.log;
    if table.step.len() != m || table.sym.len() != m {
        return Err(RansInitError::TableSizeMismatch);
    }
    if prod < 4 {
        return Err(RansInitError::ProdTooSmall);
    }
    if prod & 3 != 0 {
        return Err(RansInitError::UnsupportedProdTail);
    }

    let start_offset = cursor.offset;
    if state.flag & 0xf != 0xf {
        load_cold_rans_states(stream, state, cursor)?;
    }

    let mask = (1u64 << table.log) - 1;
    let log = table.log;
    let step = &table.step;
    let sym = &table.sym;

    for _ in 0..(prod >> 2) {
        for lane_state in &mut state.states {
            let idx = (*lane_state & mask) as usize;
            let entry = step[idx];
            let shifted = *lane_state >> log;
            let _ = sym[idx];
            *lane_state = shifted * (entry >> 16) as u64 + (entry & 0xffff) as u64;
        }
        for lane_state in &mut state.states {
            if *lane_state >> 31 == 0 {
                let word = read_stream_u32(stream, cursor)?;
                *lane_state = (*lane_state << 32) | word as u64;
            }
        }
    }

    Ok(RansInitResult {
        states: state.states,
        flag: state.flag,
        stream_offset: cursor.offset,
        stream_used: cursor.offset - start_offset,
    })
}

/// Warm-buffer convenience wrapper for the already-loaded-state slice of
/// `0x110dfa0`. Use [`rans_init_states_with_cursor`] for the generic primitive.
pub fn rans_init_states(
    table: &RansDecodeTable,
    stream: &[u8],
    prod: u32,
    stride: usize,
    states_in: [u64; 4],
) -> Result<RansInitResult, RansInitError> {
    let mut state = RansStateBuffer::warm(states_in);
    let mut cursor = RansStreamCursor::default();
    rans_init_states_with_cursor(table, stream, prod, stride, &mut state, &mut cursor)
}

/// Errors from the vertex coder's rANS decoder (`0x110e270`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RansDecodeError {
    /// `step`/`sym` length must equal `1 << log`.
    TableSizeMismatch,
    /// A zero stride would repeatedly overwrite the same output slot.
    ZeroStride,
    /// `count * stride` overflowed or does not fit the caller-provided output.
    OutputTooSmall,
    /// Forward renorm could not read four bytes.
    StreamTooShort,
}

/// Inputs for one `0x110e270` rANS decode call.
#[derive(Debug, Clone, Copy)]
pub struct RansDecodeSpec<'a> {
    /// Number of symbols this call writes (`w2` at `x1+0xc`).
    pub count: usize,
    /// `log2(M)` for the decode table.
    pub log: u32,
    /// Output spacing in u16 slots.
    pub stride: usize,
    /// Decode table steps (`freq << 16 | low`).
    pub step: &'a [u32],
    /// Decode table symbols.
    pub sym: &'a [u16],
    /// Four interleaved rANS states.
    pub init_states: [u64; 4],
    /// Forward renorm bytes.
    pub stream: &'a [u8],
}

/// Decode `count` symbols with the vertex coder's rANS decoder (`0x110e270`) into
/// an existing output buffer.
///
/// Four interleaved lanes decode in round-robin; symbol index `i` writes to
/// output position `i * stride`. In the `0x110de00` wrapper the product at
/// `x1+8` is the caller's full buffer length, while `w2` at `x1+0xc` is the
/// number of symbols this rANS call writes. For the observed stride-3 case
/// (Bass), `w2=320` and `stride=3`, so this function writes 320 symbols into
/// every third slot of a 960-slot buffer and leaves sibling lanes untouched.
///
/// The `count % 4` tail continues the **first `count&3` lanes** — tail symbol
/// `k` reads and advances `states[k]`, not `states[0]`. This is the `0x110e410`
/// tail loop: after the main loop stores the four lane states to `x0[0..4]`
/// (`stp` at `0x110e3f0`), the tail does `ldr x17,[x0]` +
/// `str x17,[x0],#8` (post-increment by one state slot per symbol), so
/// successive tail symbols consume successive lanes.
pub fn rans_decode_into(
    out: &mut [u16],
    spec: RansDecodeSpec<'_>,
) -> Result<usize, RansDecodeError> {
    rans_decode_into_with_states(out, spec).map(|(used, _states)| used)
}

fn rans_decode_into_with_states(
    out: &mut [u16],
    spec: RansDecodeSpec<'_>,
) -> Result<(usize, [u64; 4]), RansDecodeError> {
    let RansDecodeSpec {
        count,
        log,
        stride,
        step,
        sym,
        init_states,
        stream,
    } = spec;
    let m = 1usize
        .checked_shl(log)
        .ok_or(RansDecodeError::TableSizeMismatch)?;
    if step.len() != m || sym.len() != m {
        return Err(RansDecodeError::TableSizeMismatch);
    }
    if stride == 0 {
        return Err(RansDecodeError::ZeroStride);
    }
    let min_len = count
        .checked_mul(stride)
        .ok_or(RansDecodeError::OutputTooSmall)?;
    if out.len() < min_len {
        return Err(RansDecodeError::OutputTooSmall);
    }

    let mask = (1u64 << log) - 1;
    let mut states = init_states;
    let mut spos = 0usize;

    let decode_lane = |st: u64, spos: &mut usize| -> Result<(u16, u64), RansDecodeError> {
        let idx = (st & mask) as usize;
        let s = sym[idx];
        let e = step[idx];
        let mut ns = (st >> log) * (e >> 16) as u64 + (e & 0xffff) as u64;
        if ns >> 31 == 0 {
            let b = stream
                .get(*spos..*spos + 4)
                .ok_or(RansDecodeError::StreamTooShort)?;
            ns = (ns << 32) | u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u64;
            *spos += 4;
        }
        Ok((s, ns))
    };

    for i in 0..count {
        let lane = i & 3;
        let (s, ns) = decode_lane(states[lane], &mut spos)?;
        out[i * stride] = s;
        states[lane] = ns;
    }

    Ok((spos, states))
}

/// Decode `count` symbols with `0x110e270`, returning a freshly zeroed output
/// buffer of length `count * stride`.
pub fn rans_decode(spec: RansDecodeSpec<'_>) -> Result<Vec<u16>, RansDecodeError> {
    let out_len = spec
        .count
        .checked_mul(spec.stride)
        .ok_or(RansDecodeError::OutputTooSmall)?;
    let mut out = vec![0u16; out_len];
    rans_decode_into(&mut out, spec)?;
    Ok(out)
}

/// Errors from the segment RLE fill helper (`0x110f930`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RansRleFillError {
    /// A zero stride would repeatedly overwrite the same output slot.
    ZeroStride,
    /// `count * stride` overflowed or does not fit the caller-provided output.
    OutputTooSmall,
}

/// Fill `count` u16 symbols at `out[i * stride]` with `value` (`0x110f930`).
///
/// The disassembly stores a scalar tail first (`w2 & 3`) and then unrolls groups
/// of four (`0x110f950` and `0x110f970..0x110f98c`), but both paths are just the
/// same strided fill. `stride` is in u16 slots (`sxtw x8,w3; lsl x8,#1`), so
/// sibling lanes in the caller's product-sized buffer are preserved.
pub fn rans_rle_fill(
    out: &mut [u16],
    value: u16,
    count: usize,
    stride: usize,
) -> Result<(), RansRleFillError> {
    if stride == 0 {
        return Err(RansRleFillError::ZeroStride);
    }
    let min_len = count
        .checked_mul(stride)
        .ok_or(RansRleFillError::OutputTooSmall)?;
    if out.len() < min_len {
        return Err(RansRleFillError::OutputTooSmall);
    }
    for i in 0..count {
        out[i * stride] = value;
    }
    Ok(())
}

/// Parsed segment header from `0x110de80`, before the mode-specific table build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RansSegmentHeader {
    /// Descriptor mode: 0 = rANS, 1 = three-lane, 2 = RLE.
    pub mode: u32,
    /// Table log for modes 0 and 1.
    pub log: u32,
    /// Count argument passed to `0x110e540` or `0x110f3c0` for modes 0 and 1.
    pub table_count: Option<u32>,
    /// RLE value for mode 2.
    pub value: u32,
    /// Reader state after consuming the segment header.
    pub reader: RansFreqReader,
}

/// Errors from the segment-header parser (`0x110de80`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RansSegmentHeaderError {
    /// The header reader tried to load an 8-byte word outside the payload.
    PayloadTooSmall,
    /// Mode-2 value varint did not terminate inside the 32-bit field shape.
    VarintTooLong,
}

#[inline]
fn checked_header_u64_le(buf: &[u8], ptr: usize) -> Result<u64, RansSegmentHeaderError> {
    let end = ptr
        .checked_add(8)
        .ok_or(RansSegmentHeaderError::PayloadTooSmall)?;
    let bytes = buf
        .get(ptr..end)
        .ok_or(RansSegmentHeaderError::PayloadTooSmall)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

/// Parse the reverse-bit segment header (`0x110de80`) without running the
/// following mode-specific table build.
///
/// The first five bits select either the short mode-0 form, the mode-2 RLE
/// value form, or the long form. The long form's two `csel ... eq` blocks are
/// polarity-sensitive: after `tst`, `eq` means the tested top bit is clear.
pub fn rans_read_segment_header(
    payload: &[u8],
    reader: RansFreqReader,
) -> Result<RansSegmentHeader, RansSegmentHeaderError> {
    const MASK64: u64 = u64::MAX;

    let bitpos = reader.bitpos;
    let ptr_step = ((bitpos >> 3) ^ 7) as usize;
    let ptr = reader
        .ptr
        .checked_sub(ptr_step)
        .ok_or(RansSegmentHeaderError::PayloadTooSmall)?;
    let bits =
        ((checked_header_u64_le(payload, reader.ptr)? >> (bitpos & 63)) | reader.acc) & MASK64;
    let mut shifted = bits << 5;
    let short_class = ((bits >> 59) & 0xf) as u32;
    let refill_bitpos = bitpos | 0x38;

    let (table_count, next_bitpos) = if bits >> 63 == 0 {
        let next_bitpos = refill_bitpos.wrapping_sub(5);
        if short_class == 0 {
            let mut value = 0u32;
            for _ in 0..5 {
                let prev = value;
                let byte = (shifted >> 56) as u32;
                shifted <<= 8;
                value = (byte & 0x7f) | ((prev & 0x01ff_ffff) << 7);
                let value_bitpos = next_bitpos.wrapping_sub(8);
                if byte <= 0x7f {
                    return Ok(RansSegmentHeader {
                        mode: 2,
                        log: 0,
                        table_count: None,
                        value,
                        reader: RansFreqReader {
                            ptr,
                            acc: shifted,
                            bitpos: value_bitpos,
                        },
                    });
                }
            }
            return Err(RansSegmentHeaderError::VarintTooLong);
        }
        (short_class + 1, next_bitpos)
    } else {
        let high_bits = bits << 9;
        let low_count = (short_class | (((bits >> 0x33) as u32) & 0x70)).wrapping_add(0x11);
        let mid_count = (((bits >> 0x2d) as u32) & 0x180)
            .wrapping_add(low_count)
            .wrapping_add(0x80);

        let wide_bits = bits << 0x0c;
        let wider_bits = bits << 0x13;
        let wide_count = (((wide_bits >> 0x30) as u32) & 0xfe00)
            .wrapping_add(mid_count)
            .wrapping_add(0x200);

        let (selected_bits, selected_count, selected_bitpos) = if high_bits >> 63 == 0 {
            (wide_bits, mid_count, refill_bitpos.wrapping_sub(0x0c))
        } else {
            (wider_bits, wide_count, refill_bitpos.wrapping_sub(0x13))
        };

        if shifted >> 63 == 0 {
            (low_count, refill_bitpos.wrapping_sub(9))
        } else {
            shifted = selected_bits;
            (selected_count, selected_bitpos)
        }
    };

    let mode = ((shifted >> 63) & 1) as u32;
    let log = ((shifted >> 59) & 0xf) as u32;
    shifted <<= 5;
    let next_bitpos = next_bitpos.wrapping_sub(5);
    Ok(RansSegmentHeader {
        mode,
        log,
        table_count: Some(table_count),
        value: 0,
        reader: RansFreqReader {
            ptr,
            acc: shifted,
            bitpos: next_bitpos,
        },
    })
}

/// Reverse-reader state for the 3-lane segment decoder (`0x110ef70`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RansThreeLaneReader {
    /// Payload-relative byte pointer.
    pub ptr: usize,
    /// Pending high bits, with the next bit at the MSB.
    pub acc: u64,
    /// Bit position inside the next loaded `u64`.
    pub bitpos: u32,
}

/// Errors from the 3-lane segment decoder (`0x110ef70`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RansThreeLaneDecodeError {
    /// `table` length must equal `1 << log`.
    TableSizeMismatch,
    /// A zero stride would repeatedly overwrite the same output slot.
    ZeroStride,
    /// `count * stride` overflowed or does not fit the caller-provided output.
    OutputTooSmall,
    /// A reader tried to load an 8-byte word outside the payload.
    PayloadTooSmall,
    /// A table entry consumed more bits than the reloaded reader had available.
    ReaderUnderflow,
}

/// Inputs for `0x110ef70`, the mode-1 3-lane segment decoder.
pub struct RansThreeLaneDecodeSpec<'a> {
    /// Number of symbols this call writes (`w2`).
    pub count: usize,
    /// `log2` table selector width (`w5`).
    pub log: u32,
    /// Output spacing in u16 slots (`w1`).
    pub stride: usize,
    /// Packed decode table. Low 16 bits are the symbol; high 16 bits are bits consumed.
    pub table: &'a [u32],
    /// Three reader states at `x3+0`, `x3+0x18`, and `x3+0x30`. Updated in place.
    pub readers: &'a mut [RansThreeLaneReader; 3],
    /// Payload bytes addressed by the payload-relative reader pointers.
    pub payload: &'a [u8],
}

#[inline]
fn checked_u64_le(buf: &[u8], ptr: usize) -> Result<u64, RansThreeLaneDecodeError> {
    let end = ptr
        .checked_add(8)
        .ok_or(RansThreeLaneDecodeError::PayloadTooSmall)?;
    let bytes = buf
        .get(ptr..end)
        .ok_or(RansThreeLaneDecodeError::PayloadTooSmall)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn reload_backward(
    payload: &[u8],
    reader: &mut RansThreeLaneReader,
) -> Result<(), RansThreeLaneDecodeError> {
    let bitpos = reader.bitpos;
    let chunk = checked_u64_le(payload, reader.ptr)?;
    let step = ((bitpos >> 3) ^ 7) as usize;
    reader.ptr = reader
        .ptr
        .checked_sub(step)
        .ok_or(RansThreeLaneDecodeError::PayloadTooSmall)?;
    reader.acc |= chunk >> (bitpos & 63);
    reader.bitpos = bitpos | 0x38;
    Ok(())
}

fn reload_forward_rev(
    payload: &[u8],
    reader: &mut RansThreeLaneReader,
) -> Result<(), RansThreeLaneDecodeError> {
    let bitpos = reader.bitpos;
    let chunk = checked_u64_le(payload, reader.ptr)?.swap_bytes();
    let step = ((bitpos >> 3) ^ 7) as usize;
    reader.ptr = reader
        .ptr
        .checked_add(step)
        .filter(|&p| p <= payload.len())
        .ok_or(RansThreeLaneDecodeError::PayloadTooSmall)?;
    reader.acc |= chunk >> (bitpos & 63);
    reader.bitpos = bitpos | 0x38;
    Ok(())
}

fn take_three_lane_symbol(
    reader: &mut RansThreeLaneReader,
    table: &[u32],
    log: u32,
) -> Result<u16, RansThreeLaneDecodeError> {
    let idx = if log == 0 {
        0
    } else {
        (reader.acc >> (64 - log)) as usize
    };
    let entry = *table
        .get(idx)
        .ok_or(RansThreeLaneDecodeError::TableSizeMismatch)?;
    let bits = entry >> 16;
    if bits > reader.bitpos {
        return Err(RansThreeLaneDecodeError::ReaderUnderflow);
    }
    reader.acc <<= bits;
    reader.bitpos -= bits;
    Ok((entry & 0xffff) as u16)
}

/// Decode mode-1 symbols with the 3-lane bit decoder (`0x110ef70`).
///
/// The main loop decodes groups of 12 symbols: four table-coded symbols from
/// each of three readers. Readers 0 and 2 reload by little-endian `u64` loads and
/// post-decrement their payload pointer by `(bitpos >> 3) ^ 7`; reader 1 uses
/// `rev` on the loaded word and post-increments by the same expression
/// (`0x110f030..0x110f080`). A final reload handles `count % 12` tail symbols
/// in reader order 0, 1, 2 (`0x110f1f8..0x110f380`).
pub fn rans_three_lane_decode_into(
    out: &mut [u16],
    spec: RansThreeLaneDecodeSpec<'_>,
) -> Result<(), RansThreeLaneDecodeError> {
    let table_len = 1usize
        .checked_shl(spec.log)
        .ok_or(RansThreeLaneDecodeError::TableSizeMismatch)?;
    if spec.table.len() != table_len || spec.log > 63 {
        return Err(RansThreeLaneDecodeError::TableSizeMismatch);
    }
    if spec.stride == 0 {
        return Err(RansThreeLaneDecodeError::ZeroStride);
    }
    let min_len = spec
        .count
        .checked_mul(spec.stride)
        .ok_or(RansThreeLaneDecodeError::OutputTooSmall)?;
    if out.len() < min_len {
        return Err(RansThreeLaneDecodeError::OutputTooSmall);
    }

    let mut written = 0usize;
    while spec.count - written >= 12 {
        reload_backward(spec.payload, &mut spec.readers[0])?;
        reload_forward_rev(spec.payload, &mut spec.readers[1])?;
        reload_backward(spec.payload, &mut spec.readers[2])?;
        for _ in 0..4 {
            for lane in 0..3 {
                out[written * spec.stride] =
                    take_three_lane_symbol(&mut spec.readers[lane], spec.table, spec.log)?;
                written += 1;
            }
        }
    }

    if written < spec.count {
        reload_backward(spec.payload, &mut spec.readers[0])?;
        reload_forward_rev(spec.payload, &mut spec.readers[1])?;
        reload_backward(spec.payload, &mut spec.readers[2])?;
        while written < spec.count {
            let lane = written % 3;
            out[written * spec.stride] =
                take_three_lane_symbol(&mut spec.readers[lane], spec.table, spec.log)?;
            written += 1;
        }
    }

    Ok(())
}

/// Errors from the segment dispatch wrapper (`0x110de00`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RansSegmentDispatchError {
    /// Mode 1 requires the three reverse-reader states from `x5`.
    MissingThreeLaneReaders,
    /// The observed dispatch modes are 0, 1, and 2.
    UnknownMode(u32),
    /// Mode 0 rANS decode rejected the segment.
    Decode(RansDecodeError),
    /// Mode 1 three-lane decode rejected the segment.
    ThreeLane(RansThreeLaneDecodeError),
    /// Mode 2 RLE fill rejected the segment.
    Rle(RansRleFillError),
}

/// Inputs for one already-built segment descriptor dispatched by `0x110de00`.
pub struct RansSegmentDispatchSpec<'a> {
    /// Descriptor mode at `[x3]`: 0 = rANS, 1 = `0x110ef70`, 2 = RLE fill.
    pub mode: u32,
    /// rANS table log at `[x3+4]`.
    pub log: u32,
    /// RLE value at `[x3+8]` for mode 2.
    pub value: u16,
    /// Number of symbols this dispatch writes (`w2`).
    pub count: usize,
    /// Output spacing in u16 slots (`w1`).
    pub stride: usize,
    /// Warm rANS states at `[x3+0x10..0x30]` for mode 0. Updated in place.
    pub states: &'a mut [u64; 4],
    /// rANS step table at `[x3+0x80]` for mode 0.
    pub step: &'a [u32],
    /// rANS symbol table at `[x3+0x2080]` for mode 0.
    pub sym: &'a [u16],
    /// Forward renorm bytes for mode 0.
    pub stream: &'a [u8],
    /// Payload bytes for mode 1 reader loads.
    pub payload: &'a [u8],
    /// Three mode-1 reader states from `x5`, when dispatching mode 1.
    pub three_lane_readers: Option<&'a mut [RansThreeLaneReader; 3]>,
}

/// Dispatch one built symbol segment (`0x110de00`).
///
/// Mode 0 builds the same stack output spec as `0x110de14..0x110de48`:
/// `prod=count*stride` is the caller's full u16 buffer size, while `count`
/// remains the number of symbols passed to `0x110e270`; this is what preserves
/// sibling lanes for stride-3 segments. Mode 1 maps to `0x110ef70` with the
/// reader context passed in `x5`. Mode 2 maps to the strided fill at
/// `0x110de70..0x110de78` / `0x110f930`.
pub fn rans_segment_dispatch_into(
    out: &mut [u16],
    spec: RansSegmentDispatchSpec<'_>,
) -> Result<usize, RansSegmentDispatchError> {
    match spec.mode {
        0 => {
            let (used, states) = rans_decode_into_with_states(
                out,
                RansDecodeSpec {
                    count: spec.count,
                    log: spec.log,
                    stride: spec.stride,
                    step: spec.step,
                    sym: spec.sym,
                    init_states: *spec.states,
                    stream: spec.stream,
                },
            )
            .map_err(RansSegmentDispatchError::Decode)?;
            *spec.states = states;
            Ok(used)
        }
        1 => {
            let readers = spec
                .three_lane_readers
                .ok_or(RansSegmentDispatchError::MissingThreeLaneReaders)?;
            rans_three_lane_decode_into(
                out,
                RansThreeLaneDecodeSpec {
                    count: spec.count,
                    log: spec.log,
                    stride: spec.stride,
                    table: spec.step,
                    readers,
                    payload: spec.payload,
                },
            )
            .map_err(RansSegmentDispatchError::ThreeLane)?;
            Ok(0)
        }
        2 => {
            rans_rle_fill(out, spec.value, spec.count, spec.stride)
                .map_err(RansSegmentDispatchError::Rle)?;
            Ok(0)
        }
        mode => Err(RansSegmentDispatchError::UnknownMode(mode)),
    }
}

/// Reverse bit-reader state for the vertex frequency decoder (`0x110e7b0`).
///
/// `ptr` indexes the stream; `acc` holds the next bits (MSB = next to read);
/// `bitpos` is how far into `u64_le(buf, ptr)` the window starts. After each
/// symbol the reader steps `ptr -= (bitpos>>3)^7` and `bitpos = (bitpos|0x38)-nbits`,
/// matching the game's `x2` struct at `[0]=ptr, [8]=acc, [0x10]=bitpos`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RansFreqReader {
    pub ptr: usize,
    pub acc: u64,
    pub bitpos: u32,
}

/// Parameters for one `0x110e7b0` invocation (register args at entry).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RansFreqParams {
    /// Number of frequencies to write (`w1` at entry).
    pub count: u32,
    /// Initial adaptive-width state (`w3 << 10` at `0x110e7c4`).
    pub w3_init: u32,
    /// Width-update shift (`w4` at entry; `w10 = 16 - w4`).
    pub w4: u32,
    /// rANS normalization interval `M` (`x5 >> 32`).
    pub m: u32,
    /// First-symbol frequency prediction (`x5` low half, typically `M // (count+1)`).
    pub initfreq: u32,
}

/// Result of [`rans_read_freqs`]: written frequencies plus the implicit tail symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RansFreqRead {
    pub freqs: Vec<u16>,
    /// Last written frequency (`w5` at `0x110e980`).
    pub last_freq: u16,
    /// Remaining probability mass — the `(count+1)`-th symbol's frequency (`w9`).
    pub rem: u32,
    pub reader: RansFreqReader,
}

#[inline]
fn clz64(x: u64) -> u32 {
    if x == 0 {
        64
    } else {
        x.leading_zeros()
    }
}

#[inline]
fn clz32(x: u32) -> u32 {
    if x == 0 {
        32
    } else {
        x.leading_zeros()
    }
}

/// Zigzag decode for a 32-bit raw value (`0x110e848` / `0x110e8fc`).
#[inline]
fn unzigzag32(v: u32) -> u32 {
    (v >> 1) ^ (0u32.wrapping_sub(v & 1))
}

/// Decode adaptive rANS symbol frequencies (`0x110e7b0`).
///
/// Three validated code paths (see `local-assets/re/_freqdis.txt`):
/// * slow adaptive `clz` prefix (`0x110e7f8`),
/// * `clz`-coded run length (`0x110e890` / `0x110e8e8`),
/// * fixed-width run body (`0x110e900`; top bits via `(acc>>1)>>~width`).
pub fn rans_read_freqs(buf: &[u8], reader: RansFreqReader, params: RansFreqParams) -> RansFreqRead {
    const M32: u32 = 0xffff_ffff;
    const MASK64: u64 = u64::MAX;

    let mut bitpos = reader.bitpos & M32;
    let width_mul = 16u32.wrapping_sub(params.w4);
    let mut width_state = params.w3_init << 10;
    let mut ptr = reader.ptr;
    let mut acc = reader.acc & MASK64;
    let cap_base = 0x8000u32;
    let count_bitlen_base = 0x20u32;
    let one = 1u32;

    let mut rem = params.m & M32;
    let mut freq = params.initfreq & M32;
    let mut remaining = params.count & M32;
    let mut prime = 0u32;
    let mut remaining_after_run = 0u32;
    let mut out = Vec::with_capacity(remaining as usize);
    let mut site = FreqSite::SlowClz;

    loop {
        match site {
            FreqSite::AfterRun => {
                // 0x110e7e4 — `w7 = 1<<width` primes the next slow-path symbol.
                prime = one << ((width_state >> 10) & 31);
                if remaining == 0 {
                    site = FreqSite::Return;
                } else {
                    site = FreqSite::SlowClz;
                }
            }
            FreqSite::SlowClz => {
                // 0x110e7f8
                let chunk = u64_le(buf, ptr);
                remaining = remaining.wrapping_sub(1);
                let at_end = remaining == 0;
                acc = ((chunk >> (bitpos & 63)) | acc) & MASK64;

                let width = width_state >> 10;
                let clz = clz64(acc);
                let mut nbits = width.wrapping_add(clz << 1);
                let neg_nbits = !nbits; // before `nbits += 1` at 0x110e820
                nbits = nbits.wrapping_add(1);
                let val = (acc >> (neg_nbits & 63)) & MASK64;
                let ptr_step = ((bitpos >> 3) ^ 7) & M32;
                bitpos = (bitpos | 0x38).wrapping_sub(nbits);
                acc = (acc << (nbits & 63)) & MASK64;
                ptr = ptr.wrapping_sub(ptr_step as usize);
                // `w18 = val - (1<<width) + prime` (`0x110e814`–`0x110e834`; prime from `0x110e7ec`).
                let raw = (val as u32)
                    .wrapping_sub(one << (width & 31))
                    .wrapping_add(prime);
                prime = 0;
                freq = freq.wrapping_add(unzigzag32(raw));
                rem = rem.wrapping_sub(freq);
                out.push(freq as u16);

                if at_end {
                    site = FreqSite::Return;
                    continue;
                }

                width_state = width_state.wrapping_mul(width_mul);
                let w17h = raw >> (width & 31);
                width_state = width_state.wrapping_add(
                    (cap_base.wrapping_sub(clz32(raw) << 10)).wrapping_mul(params.w4),
                );
                width_state >>= 4;
                let cap = cap_base.wrapping_sub(clz32(rem) << 10);
                if cap < width_state {
                    width_state = cap;
                }
                site = if w17h != 0 {
                    FreqSite::SlowClz
                } else {
                    FreqSite::RunLength
                };
            }
            FreqSite::RunLength => {
                // 0x110e890
                let chunk = u64_le(buf, ptr);
                ptr = ptr.wrapping_sub((((bitpos >> 3) ^ 7) & M32) as usize);
                acc = ((chunk >> (bitpos & 63)) | acc) & MASK64;
                let nbits_count = count_bitlen_base.wrapping_sub(clz32(remaining));
                let clz = clz64(acc);
                if nbits_count <= clz {
                    // 0x110e8e8 — `w18 = 0`; `w1` (remaining) is unchanged.
                    bitpos = (bitpos | 0x38).wrapping_sub(nbits_count);
                    acc = (acc << (nbits_count & 63)) & MASK64;
                    site = FreqSite::RunBody;
                } else {
                    let run_nbits = one | (clz << 1);
                    let top = (acc >> (run_nbits.wrapping_neg() & 63)) & MASK64;
                    acc = (acc << (run_nbits & 63)) & MASK64;
                    let run_len = (top as u32).wrapping_sub(1);
                    bitpos = (bitpos | 0x38).wrapping_sub(run_nbits);
                    remaining_after_run = remaining.wrapping_sub(run_len);
                    remaining = run_len;
                    site = if run_len != 0 {
                        FreqSite::RunBody
                    } else {
                        remaining = remaining_after_run;
                        FreqSite::AfterRun
                    };
                }
            }
            FreqSite::RunBody => {
                // 0x110e900
                let chunk = u64_le(buf, ptr);
                remaining = remaining.wrapping_sub(1);
                let at_end = remaining == 0;
                ptr = ptr.wrapping_sub((((bitpos >> 3) ^ 7) & M32) as usize);
                acc = ((chunk >> (bitpos & 63)) | acc) & MASK64;
                bitpos = (bitpos | 0x38) & M32;
                let width = width_state >> 10;
                width_state = width_state.wrapping_mul(width_mul);
                let top_half = (acc >> 1) & MASK64;
                acc = (acc << (width & 63)) & MASK64;
                bitpos = bitpos.wrapping_sub(width);
                let val = (top_half >> ((!width) & 63)) & MASK64;
                freq = freq.wrapping_add(unzigzag32(val as u32));
                rem = rem.wrapping_sub(freq);
                out.push(freq as u16);
                width_state = width_state.wrapping_add(
                    (cap_base.wrapping_sub(clz32(val as u32) << 10)).wrapping_mul(params.w4),
                );
                width_state >>= 4;
                let cap = cap_base.wrapping_sub(clz32(rem) << 10);
                if cap < width_state {
                    width_state = cap;
                }
                if at_end {
                    remaining = remaining_after_run;
                    site = FreqSite::AfterRun;
                } else {
                    site = FreqSite::RunBody;
                }
            }
            FreqSite::Return => break,
        }
    }

    RansFreqRead {
        freqs: out,
        last_freq: freq as u16,
        rem,
        reader: RansFreqReader { ptr, acc, bitpos },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FreqSite {
    SlowClz,
    RunLength,
    RunBody,
    AfterRun,
    Return,
}

/// Mode-0 segment table built by `0x110e540`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RansMode0TableBuild {
    pub table: RansDecodeTable,
    /// Sparse symbol IDs decoded before the frequency table.
    pub symbols: Vec<u16>,
    /// Frequencies paired with `symbols`; the last entry is the implicit tail mass.
    pub freqs: Vec<u16>,
    /// Reverse reader state after the symbol-list and frequency readers.
    pub reader: RansFreqReader,
}

/// Errors from the mode-0 segment table builder (`0x110e540`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RansMode0TableBuildError {
    /// `0x110e540` expects at least one sparse symbol.
    TableCountZero,
    /// The descriptor reserves 2048 entries for the mode-0 table (`log <= 11`).
    UnsupportedLog(u32),
    /// A sparse alphabet with more symbols than rANS states cannot assign
    /// positive frequency to every symbol.
    TableCountExceedsMass { count: u32, mass: u32 },
    /// A reverse-reader load would read outside the provided payload.
    PayloadTooSmall,
    /// Decoded frequencies did not sum to `1 << log`.
    FrequencyMassMismatch { expected: u32, actual: u64 },
}

#[inline]
fn checked_mode0_u64_le(buf: &[u8], ptr: usize) -> Result<u64, RansMode0TableBuildError> {
    let end = ptr
        .checked_add(8)
        .ok_or(RansMode0TableBuildError::PayloadTooSmall)?;
    let bytes = buf
        .get(ptr..end)
        .ok_or(RansMode0TableBuildError::PayloadTooSmall)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

#[inline]
fn checked_sub_ptr(ptr: usize, step: u32) -> Result<usize, RansMode0TableBuildError> {
    ptr.checked_sub(step as usize)
        .ok_or(RansMode0TableBuildError::PayloadTooSmall)
}

fn rans_mode0_small_symbols(
    payload: &[u8],
    reader: RansFreqReader,
    count: usize,
) -> Result<(Vec<u16>, RansFreqReader), RansMode0TableBuildError> {
    const M32: u32 = u32::MAX;
    const MASK64: u64 = u64::MAX;

    let mut ptr = reader.ptr;
    let mut acc = reader.acc;
    let mut bitpos = reader.bitpos;
    let mut width_state = 0u32;
    let mut prev_plus_one = 0u32;
    let mut symbols = Vec::with_capacity(count);

    for _ in 0..count {
        let chunk = checked_mode0_u64_le(payload, ptr)?;
        let width = width_state >> 10;
        width_state = width_state.wrapping_mul(12);
        ptr = checked_sub_ptr(ptr, (bitpos >> 3) ^ 7)?;
        acc = ((chunk >> (bitpos & 63)) | acc) & MASK64;

        let leading_zeroes = clz64(acc);
        let nbits_minus_one = width.wrapping_add(leading_zeroes << 1);
        let raw = (M32 << (width & 31)).wrapping_add((acc >> ((!nbits_minus_one) & 63)) as u32);
        acc = (acc << (nbits_minus_one.wrapping_add(1) & 63)) & MASK64;
        bitpos = (bitpos | 0x38).wrapping_sub(nbits_minus_one.wrapping_add(1));

        let symbol = raw.wrapping_add(prev_plus_one);
        symbols.push(symbol as u16);
        prev_plus_one = symbol.wrapping_add(1);

        width_state = width_state
            .wrapping_add(0x0002_0000)
            .wrapping_sub(clz32(raw).wrapping_shl(12));
        width_state >>= 4;
    }

    Ok((symbols, RansFreqReader { ptr, acc, bitpos }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode0SymbolSite {
    Slow,
    RunLength,
    RunBody,
    AfterRun,
}

fn rans_mode0_large_symbols(
    payload: &[u8],
    reader: RansFreqReader,
    count: usize,
) -> Result<(Vec<u16>, RansFreqReader), RansMode0TableBuildError> {
    const M32: u32 = u32::MAX;
    const MASK64: u64 = u64::MAX;

    let mut ptr = reader.ptr;
    let mut acc = reader.acc;
    let mut bitpos = reader.bitpos;
    let mut run_bitpos = 0u32;
    let mut width_state = 0u32;
    let mut previous_plus_one = 0u32;
    let mut remaining = count as u32;
    let mut remaining_after_run = 0u32;
    let mut prime = 0u32;
    let mut site = Mode0SymbolSite::Slow;
    let mut symbols = Vec::with_capacity(count);

    loop {
        match site {
            Mode0SymbolSite::AfterRun => {
                let width = width_state >> 10;
                prime = 1u32 << (width & 31);
                remaining = remaining_after_run;
                bitpos = run_bitpos;
                if remaining == 0 {
                    break;
                }
                site = Mode0SymbolSite::Slow;
            }
            Mode0SymbolSite::Slow => {
                let chunk = checked_mode0_u64_le(payload, ptr)?;
                remaining = remaining.wrapping_sub(1);
                let at_end = remaining == 0;
                ptr = checked_sub_ptr(ptr, (bitpos >> 3) ^ 7)?;
                acc = ((chunk >> (bitpos & 63)) | acc) & MASK64;

                let width = width_state >> 10;
                let leading_zeroes = clz64(acc);
                let nbits_minus_one = width.wrapping_add(leading_zeroes << 1);
                let base = (M32 << (width & 31)).wrapping_add(prime);
                let raw = base.wrapping_add((acc >> ((!nbits_minus_one) & 63)) as u32);
                acc = (acc << (nbits_minus_one.wrapping_add(1) & 63)) & MASK64;
                bitpos = (bitpos | 0x38).wrapping_sub(nbits_minus_one.wrapping_add(1));

                let symbol = previous_plus_one.wrapping_add(raw);
                symbols.push(symbol as u16);
                previous_plus_one = symbol.wrapping_add(1);
                if at_end {
                    run_bitpos = bitpos;
                    break;
                }

                width_state = width_state.wrapping_mul(13);
                prime = 0;
                width_state = width_state.wrapping_add(
                    (0x8000u32.wrapping_sub(clz32(raw).wrapping_shl(10))).wrapping_mul(3),
                );
                width_state >>= 4;
                site = if raw >> (width & 31) != 0 {
                    Mode0SymbolSite::Slow
                } else {
                    Mode0SymbolSite::RunLength
                };
            }
            Mode0SymbolSite::RunLength => {
                let chunk = checked_mode0_u64_le(payload, ptr)?;
                ptr = checked_sub_ptr(ptr, (bitpos >> 3) ^ 7)?;
                acc = ((chunk >> (bitpos & 63)) | acc) & MASK64;
                let nbits_count = 32u32.wrapping_sub(clz32(remaining));
                let leading_zeroes = clz64(acc);
                if nbits_count <= leading_zeroes {
                    run_bitpos = (bitpos | 0x38).wrapping_sub(nbits_count);
                    acc = (acc << (nbits_count & 63)) & MASK64;
                    remaining_after_run = 0;
                    site = Mode0SymbolSite::RunBody;
                } else {
                    let run_nbits = 1u32 | (leading_zeroes << 1);
                    let top = acc >> (run_nbits.wrapping_neg() & 63);
                    acc = (acc << (run_nbits & 63)) & MASK64;
                    let run_len = (top as u32).wrapping_sub(1);
                    run_bitpos = (bitpos | 0x38).wrapping_sub(run_nbits);
                    remaining_after_run = remaining.wrapping_sub(run_len);
                    remaining = run_len;
                    site = if run_len == 0 {
                        Mode0SymbolSite::AfterRun
                    } else {
                        Mode0SymbolSite::RunBody
                    };
                }
            }
            Mode0SymbolSite::RunBody => {
                let chunk = checked_mode0_u64_le(payload, ptr)?;
                remaining = remaining.wrapping_sub(1);
                let at_end = remaining == 0;
                ptr = checked_sub_ptr(ptr, (run_bitpos >> 3) ^ 7)?;
                acc = ((chunk >> (run_bitpos & 63)) | acc) & MASK64;
                run_bitpos |= 0x38;

                let width = width_state >> 10;
                width_state = width_state.wrapping_mul(13);
                let value = ((acc >> 1) >> ((!width) & 63)) as u32;
                acc = (acc << (width & 63)) & MASK64;
                run_bitpos = run_bitpos.wrapping_sub(width);

                let symbol = previous_plus_one.wrapping_add(value);
                symbols.push(symbol as u16);
                previous_plus_one = symbol.wrapping_add(1);

                width_state = width_state.wrapping_add(
                    (0x8000u32.wrapping_sub(clz32(value).wrapping_shl(10))).wrapping_mul(3),
                );
                width_state >>= 4;
                site = if at_end {
                    Mode0SymbolSite::AfterRun
                } else {
                    Mode0SymbolSite::RunBody
                };
            }
        }
    }

    Ok((
        symbols,
        RansFreqReader {
            ptr,
            acc,
            bitpos: run_bitpos,
        },
    ))
}

fn rans_read_freqs_checked(
    buf: &[u8],
    reader: RansFreqReader,
    params: RansFreqParams,
) -> Result<RansFreqRead, RansMode0TableBuildError> {
    const M32: u32 = 0xffff_ffff;
    const MASK64: u64 = u64::MAX;

    let mut bitpos = reader.bitpos & M32;
    let width_mul = 16u32.wrapping_sub(params.w4);
    let mut width_state = params.w3_init << 10;
    let mut ptr = reader.ptr;
    let mut acc = reader.acc & MASK64;
    let cap_base = 0x8000u32;
    let count_bitlen_base = 0x20u32;
    let one = 1u32;

    let mut rem = params.m & M32;
    let mut freq = params.initfreq & M32;
    let mut remaining = params.count & M32;
    let mut prime = 0u32;
    let mut remaining_after_run = 0u32;
    let mut out = Vec::with_capacity(remaining as usize);
    let mut site = FreqSite::SlowClz;

    loop {
        match site {
            FreqSite::AfterRun => {
                prime = one << ((width_state >> 10) & 31);
                if remaining == 0 {
                    site = FreqSite::Return;
                } else {
                    site = FreqSite::SlowClz;
                }
            }
            FreqSite::SlowClz => {
                let chunk = checked_mode0_u64_le(buf, ptr)?;
                remaining = remaining.wrapping_sub(1);
                let at_end = remaining == 0;
                acc = ((chunk >> (bitpos & 63)) | acc) & MASK64;

                let width = width_state >> 10;
                let clz = clz64(acc);
                let mut nbits = width.wrapping_add(clz << 1);
                let neg_nbits = !nbits;
                nbits = nbits.wrapping_add(1);
                let val = (acc >> (neg_nbits & 63)) & MASK64;
                ptr = checked_sub_ptr(ptr, (bitpos >> 3) ^ 7)?;
                bitpos = (bitpos | 0x38).wrapping_sub(nbits);
                acc = (acc << (nbits & 63)) & MASK64;
                let raw = (val as u32)
                    .wrapping_sub(one << (width & 31))
                    .wrapping_add(prime);
                prime = 0;
                freq = freq.wrapping_add(unzigzag32(raw));
                rem = rem.wrapping_sub(freq);
                out.push(freq as u16);

                if at_end {
                    site = FreqSite::Return;
                    continue;
                }

                width_state = width_state.wrapping_mul(width_mul);
                let raw_high = raw >> (width & 31);
                width_state = width_state.wrapping_add(
                    (cap_base.wrapping_sub(clz32(raw) << 10)).wrapping_mul(params.w4),
                );
                width_state >>= 4;
                let cap = cap_base.wrapping_sub(clz32(rem) << 10);
                if cap < width_state {
                    width_state = cap;
                }
                site = if raw_high != 0 {
                    FreqSite::SlowClz
                } else {
                    FreqSite::RunLength
                };
            }
            FreqSite::RunLength => {
                let chunk = checked_mode0_u64_le(buf, ptr)?;
                ptr = checked_sub_ptr(ptr, (bitpos >> 3) ^ 7)?;
                acc = ((chunk >> (bitpos & 63)) | acc) & MASK64;
                let nbits_count = count_bitlen_base.wrapping_sub(clz32(remaining));
                let clz = clz64(acc);
                if nbits_count <= clz {
                    bitpos = (bitpos | 0x38).wrapping_sub(nbits_count);
                    acc = (acc << (nbits_count & 63)) & MASK64;
                    site = FreqSite::RunBody;
                } else {
                    let run_nbits = one | (clz << 1);
                    let top = (acc >> (run_nbits.wrapping_neg() & 63)) & MASK64;
                    acc = (acc << (run_nbits & 63)) & MASK64;
                    let run_len = (top as u32).wrapping_sub(1);
                    bitpos = (bitpos | 0x38).wrapping_sub(run_nbits);
                    remaining_after_run = remaining.wrapping_sub(run_len);
                    remaining = run_len;
                    site = if run_len != 0 {
                        FreqSite::RunBody
                    } else {
                        remaining = remaining_after_run;
                        FreqSite::AfterRun
                    };
                }
            }
            FreqSite::RunBody => {
                let chunk = checked_mode0_u64_le(buf, ptr)?;
                remaining = remaining.wrapping_sub(1);
                let at_end = remaining == 0;
                ptr = checked_sub_ptr(ptr, (bitpos >> 3) ^ 7)?;
                acc = ((chunk >> (bitpos & 63)) | acc) & MASK64;
                bitpos = (bitpos | 0x38) & M32;
                let width = width_state >> 10;
                width_state = width_state.wrapping_mul(width_mul);
                let top_half = (acc >> 1) & MASK64;
                acc = (acc << (width & 63)) & MASK64;
                bitpos = bitpos.wrapping_sub(width);
                let val = (top_half >> ((!width) & 63)) & MASK64;
                freq = freq.wrapping_add(unzigzag32(val as u32));
                rem = rem.wrapping_sub(freq);
                out.push(freq as u16);
                width_state = width_state.wrapping_add(
                    (cap_base.wrapping_sub(clz32(val as u32) << 10)).wrapping_mul(params.w4),
                );
                width_state >>= 4;
                let cap = cap_base.wrapping_sub(clz32(rem) << 10);
                if cap < width_state {
                    width_state = cap;
                }
                if at_end {
                    remaining = remaining_after_run;
                    site = FreqSite::AfterRun;
                } else {
                    site = FreqSite::RunBody;
                }
            }
            FreqSite::Return => break,
        }
    }

    Ok(RansFreqRead {
        freqs: out,
        last_freq: freq as u16,
        rem,
        reader: RansFreqReader { ptr, acc, bitpos },
    })
}

fn rans_spread_sparse(
    log: u32,
    symbols: &[u16],
    freqs: &[u16],
) -> Result<RansDecodeTable, RansMode0TableBuildError> {
    let m = 1usize << log;
    let mut step = vec![0u32; m];
    let mut sym = vec![0u16; m];
    let mut cursor = 0usize;

    for (&symbol, &freq) in symbols.iter().zip(freqs) {
        let freq = freq as usize;
        let end =
            cursor
                .checked_add(freq)
                .ok_or(RansMode0TableBuildError::FrequencyMassMismatch {
                    expected: m as u32,
                    actual: u64::MAX,
                })?;
        if end > m {
            return Err(RansMode0TableBuildError::FrequencyMassMismatch {
                expected: m as u32,
                actual: end as u64,
            });
        }
        let high = (freq as u32) << 16;
        for slot in cursor..end {
            let low = (slot - cursor) as u32;
            sym[slot] = symbol;
            step[slot] = high | low;
        }
        cursor = end;
    }

    if cursor != m {
        return Err(RansMode0TableBuildError::FrequencyMassMismatch {
            expected: m as u32,
            actual: cursor as u64,
        });
    }

    Ok(RansDecodeTable { log, step, sym })
}

/// Build a mode-0 segment decode table (`0x110e540`).
///
/// The builder first decodes a sparse, strictly-increasing symbol list. Small
/// alphabets (`count <= 10`) use the compact loop at `0x110e578..0x110e60c`;
/// larger alphabets tail-call the related symbol-list reader at `0x110e9a0`
/// with `w4=3`. It then reads `count-1` frequencies with `0x110e7b0`, using
/// `w4=15` for small alphabets and `w4=14` for large alphabets, and spreads the
/// implicit tail mass contiguously into the mode-0 step/symbol tables.
pub fn rans_build_mode0_table(
    payload: &[u8],
    reader: RansFreqReader,
    table_count: u32,
    log: u32,
) -> Result<RansMode0TableBuild, RansMode0TableBuildError> {
    if table_count == 0 {
        return Err(RansMode0TableBuildError::TableCountZero);
    }
    if log > 11 {
        return Err(RansMode0TableBuildError::UnsupportedLog(log));
    }
    let mass = 1u32 << log;
    if table_count > mass {
        return Err(RansMode0TableBuildError::TableCountExceedsMass {
            count: table_count,
            mass,
        });
    }

    let count = table_count as usize;
    let (symbols, reader, freq_w4) = if count <= 10 {
        let (symbols, reader) = rans_mode0_small_symbols(payload, reader, count)?;
        (symbols, reader, 15)
    } else {
        let (symbols, reader) = rans_mode0_large_symbols(payload, reader, count)?;
        (symbols, reader, 14)
    };

    let (freqs, reader) = if count == 1 {
        (vec![mass as u16], reader)
    } else {
        let freq_count = table_count - 1;
        let freq_read = rans_read_freqs_checked(
            payload,
            reader,
            RansFreqParams {
                count: freq_count,
                w3_init: log.max(3) - 2,
                w4: freq_w4,
                m: mass,
                initfreq: mass / table_count,
            },
        )?;
        let mut freqs = freq_read.freqs;
        let actual = freqs.iter().map(|&f| f as u64).sum::<u64>() + freq_read.rem as u64;
        if freq_read.rem > u16::MAX as u32 || actual != mass as u64 {
            return Err(RansMode0TableBuildError::FrequencyMassMismatch {
                expected: mass,
                actual,
            });
        }
        freqs.push(freq_read.rem as u16);
        (freqs, freq_read.reader)
    };

    let table = rans_spread_sparse(log, &symbols, &freqs)?;
    Ok(RansMode0TableBuild {
        table,
        symbols,
        freqs,
        reader,
    })
}

/// Mode-1 segment table built by `0x110f3c0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RansMode1TableBuild {
    /// Packed three-lane table entries: low 16 bits = symbol, high 16 bits =
    /// number of bits consumed by `0x110ef70`.
    pub table: Vec<u32>,
    /// Reverse reader state after the table builder.
    pub reader: RansFreqReader,
}

/// Errors from the mode-1 segment table builder (`0x110f3c0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RansMode1TableBuildError {
    TableCountZero,
    UnsupportedLog(u32),
    TableCountExceedsMass { count: u32, mass: u32 },
    PayloadTooSmall,
    MalformedTable,
}

#[inline]
fn checked_mode1_u64_le(buf: &[u8], ptr: usize) -> Result<u64, RansMode1TableBuildError> {
    let end = ptr
        .checked_add(8)
        .ok_or(RansMode1TableBuildError::PayloadTooSmall)?;
    let bytes = buf
        .get(ptr..end)
        .ok_or(RansMode1TableBuildError::PayloadTooSmall)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

#[inline]
fn checked_mode1_sub_ptr(ptr: usize, step: u32) -> Result<usize, RansMode1TableBuildError> {
    ptr.checked_sub(step as usize)
        .ok_or(RansMode1TableBuildError::PayloadTooSmall)
}

#[inline]
fn mode1_low(workspace: &[u32; 0x800], index: usize) -> Result<u16, RansMode1TableBuildError> {
    workspace
        .get(index)
        .map(|entry| (entry & 0xffff) as u16)
        .ok_or(RansMode1TableBuildError::MalformedTable)
}

#[inline]
fn mode1_high(workspace: &[u32; 0x800], index: usize) -> Result<u16, RansMode1TableBuildError> {
    workspace
        .get(index)
        .map(|entry| (entry >> 16) as u16)
        .ok_or(RansMode1TableBuildError::MalformedTable)
}

#[inline]
fn mode1_store_low(
    workspace: &mut [u32; 0x800],
    index: usize,
    value: u32,
) -> Result<(), RansMode1TableBuildError> {
    let slot = workspace
        .get_mut(index)
        .ok_or(RansMode1TableBuildError::MalformedTable)?;
    *slot = (*slot & 0xffff_0000) | (value & 0xffff);
    Ok(())
}

#[inline]
fn mode1_store_high(
    workspace: &mut [u32; 0x800],
    index: usize,
    value: u32,
) -> Result<(), RansMode1TableBuildError> {
    let slot = workspace
        .get_mut(index)
        .ok_or(RansMode1TableBuildError::MalformedTable)?;
    *slot = (*slot & 0x0000_ffff) | ((value & 0xffff) << 16);
    Ok(())
}

#[inline]
fn mode1_store_pair(
    workspace: &mut [u32; 0x800],
    index: usize,
    value: u32,
) -> Result<(), RansMode1TableBuildError> {
    let end = index
        .checked_add(1)
        .ok_or(RansMode1TableBuildError::MalformedTable)?;
    if end >= workspace.len() {
        return Err(RansMode1TableBuildError::MalformedTable);
    }
    workspace[index] = value;
    workspace[end] = value;
    Ok(())
}

/// Build a mode-1 segment decode table (`0x110f3c0`).
///
/// The function first reads the number of symbols assigned to each bit length
/// into high halfwords of the scratch table (`0x110f404..0x110f4bc`), then reads
/// grouped sparse symbol IDs into low halfwords (`0x110f4c8..0x110f558`), and
/// finally expands those groups into the first `1 << log` packed entries
/// (`0x110f558..0x110f718`) consumed by the three-lane decoder.
pub fn rans_build_mode1_table(
    payload: &[u8],
    reader: RansFreqReader,
    table_count: u32,
    log: u32,
) -> Result<RansMode1TableBuild, RansMode1TableBuildError> {
    const M32: u32 = u32::MAX;
    const MASK64: u64 = u64::MAX;

    if table_count == 0 {
        return Err(RansMode1TableBuildError::TableCountZero);
    }
    if log == 0 || log > 11 {
        return Err(RansMode1TableBuildError::UnsupportedLog(log));
    }
    let mass = 1u32 << log;
    if table_count > mass {
        return Err(RansMode1TableBuildError::TableCountExceedsMass {
            count: table_count,
            mass,
        });
    }

    let mut workspace = [0u32; 0x800];
    let mut ptr = reader.ptr;
    let mut acc = reader.acc;
    let mut bitpos = reader.bitpos;
    let count_base = 0x800usize - log as usize;
    let symbol_base = 0x800usize - table_count as usize;

    let chunk = checked_mode1_u64_le(payload, ptr)?;
    ptr = checked_mode1_sub_ptr(ptr, (bitpos >> 3) ^ 7)?;
    acc = ((chunk >> (bitpos & 63)) | acc) & MASK64;
    bitpos |= 0x38;

    if log >= 2 {
        let mut count_index = count_base;
        let mut length_index = 0u32;
        let mut remaining = table_count;
        loop {
            if bitpos <= 9 {
                let chunk = checked_mode1_u64_le(payload, ptr)?;
                ptr = checked_mode1_sub_ptr(ptr, (bitpos >> 3) ^ 7)?;
                acc = ((chunk >> (bitpos & 63)) | acc) & MASK64;
                bitpos |= 0x38;
            }

            length_index = length_index.wrapping_add(1);
            let neg_remaining = remaining.wrapping_neg();
            let mask = M32 << (length_index & 31);
            let selected = if (mask as i32) > (neg_remaining as i32) {
                mask
            } else {
                neg_remaining
            };
            let leading = clz32(!selected);
            let bits = 32u32.wrapping_sub(leading);
            let value = acc >> ((leading + 32) & 63);
            acc = (acc << (bits & 63)) & MASK64;
            bitpos = bitpos.wrapping_sub(bits);
            mode1_store_high(&mut workspace, count_index, value as u32)?;
            remaining = remaining.wrapping_sub(value as u32);

            if length_index == log - 1 {
                mode1_store_high(&mut workspace, 0x7ff, remaining)?;
                break;
            }
            count_index += 1;
        }
    } else {
        mode1_store_high(&mut workspace, count_base, table_count)?;
    }

    let mut symbol_write = symbol_base;
    let mut count_read = count_base;
    let mut current_bucket = 0u32;
    let mut previous_plus_one = 0u32;
    let mut width_state = 0u32;
    let mut remaining_symbols = table_count;
    while remaining_symbols != 0 {
        if current_bucket == 0 {
            current_bucket = mode1_high(&workspace, count_read)? as u32;
            count_read += 1;
            if current_bucket == 0 {
                continue;
            }
            previous_plus_one = 0;
        }

        let chunk = checked_mode1_u64_le(payload, ptr)?;
        let width = width_state >> 10;
        width_state = width_state.wrapping_mul(12);
        remaining_symbols = remaining_symbols.wrapping_sub(1);
        current_bucket = current_bucket.wrapping_sub(1);
        ptr = checked_mode1_sub_ptr(ptr, (bitpos >> 3) ^ 7)?;
        acc = ((chunk >> (bitpos & 63)) | acc) & MASK64;
        bitpos |= 0x38;

        let leading = clz64(acc);
        let nbits_minus_one = width.wrapping_add(leading << 1);
        let raw = (M32 << (width & 31)).wrapping_add((acc >> ((!nbits_minus_one) & 63)) as u32);
        acc = (acc << (nbits_minus_one.wrapping_add(1) & 63)) & MASK64;
        bitpos = bitpos.wrapping_sub(nbits_minus_one.wrapping_add(1));

        let symbol = previous_plus_one.wrapping_add(raw);
        mode1_store_low(&mut workspace, symbol_write, symbol)?;
        symbol_write += 1;
        previous_plus_one = symbol.wrapping_add(1);
        width_state = width_state
            .wrapping_add(0x0002_0000)
            .wrapping_sub(clz32(raw).wrapping_shl(12));
        width_state >>= 4;
    }

    let mut output = 0usize;
    let mut symbol_read = symbol_base;
    if log >= 2 {
        let mut block = 1usize << (log - 1);
        let mut length = 1u32;
        let mut count_index = count_base;
        while length != log {
            let mut bucket_count = mode1_high(&workspace, count_index)? as u32;
            if bucket_count == 0 || block == 0 {
                symbol_read = symbol_read
                    .checked_add(bucket_count as usize)
                    .ok_or(RansMode1TableBuildError::MalformedTable)?;
            } else {
                let entry_high = length << 16;
                if bucket_count & 1 != 0 {
                    let entry = entry_high | mode1_low(&workspace, symbol_read)? as u32;
                    symbol_read += 1;
                    let mut fill = 0usize;
                    loop {
                        mode1_store_pair(&mut workspace, output + fill, entry)?;
                        fill += 2;
                        if fill >= block {
                            break;
                        }
                    }
                    output += fill;
                    bucket_count -= 1;
                    if bucket_count == 0 {
                        length += 1;
                        count_index += 1;
                        block >>= 1;
                        continue;
                    }
                }
                while bucket_count != 0 {
                    let entry = entry_high | mode1_low(&workspace, symbol_read)? as u32;
                    symbol_read += 1;
                    let mut fill = 0usize;
                    loop {
                        mode1_store_pair(&mut workspace, output + fill, entry)?;
                        fill += 2;
                        if fill >= block {
                            break;
                        }
                    }
                    output += fill;

                    let entry = entry_high | mode1_low(&workspace, symbol_read)? as u32;
                    symbol_read += 1;
                    let mut fill = 0usize;
                    loop {
                        mode1_store_pair(&mut workspace, output + fill, entry)?;
                        fill += 2;
                        if fill >= block {
                            break;
                        }
                    }
                    output += fill;
                    bucket_count = bucket_count.wrapping_sub(2);
                }
            }
            length += 1;
            count_index += 1;
            block >>= 1;
        }
    }

    let mut final_count = mode1_high(&workspace, 0x7ff)? as u32;
    if final_count != 0 {
        let entry_high = log << 16;
        if final_count & 1 != 0 {
            let entry = entry_high | mode1_low(&workspace, symbol_read)? as u32;
            let slot = workspace
                .get_mut(output)
                .ok_or(RansMode1TableBuildError::MalformedTable)?;
            *slot = entry;
            output += 1;
            symbol_read += 1;
            final_count -= 1;
        }
        while final_count != 0 {
            let first = entry_high | mode1_low(&workspace, symbol_read)? as u32;
            let second = entry_high | mode1_low(&workspace, symbol_read + 1)? as u32;
            let next = output
                .checked_add(1)
                .ok_or(RansMode1TableBuildError::MalformedTable)?;
            if next >= workspace.len() {
                return Err(RansMode1TableBuildError::MalformedTable);
            }
            workspace[output] = first;
            workspace[next] = second;
            output += 2;
            symbol_read += 2;
            final_count = final_count.wrapping_sub(2);
        }
    }

    let table_len = 1usize << log;
    Ok(RansMode1TableBuild {
        table: workspace[..table_len].to_vec(),
        reader: RansFreqReader { ptr, acc, bitpos },
    })
}

/// Segment descriptor built by `0x110de80`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RansBuiltSegmentDescriptor {
    pub mode: u32,
    pub log: u32,
    pub value: u16,
    /// Mode-0 step table or mode-1 packed table.
    pub step: Vec<u32>,
    /// Mode-0 symbol spread. Mode 1 stores symbols in `step` low halfwords.
    pub sym: Vec<u16>,
    /// Reverse reader state after the header and any table build.
    pub reader: RansFreqReader,
}

/// Errors from the full segment descriptor builder (`0x110de80`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RansSegmentDescriptorBuildError {
    Header(RansSegmentHeaderError),
    MissingTableCount,
    Mode0(RansMode0TableBuildError),
    Mode1(RansMode1TableBuildError),
    RleValueTooLarge(u32),
    UnknownMode(u32),
}

/// Build one segment descriptor from the reverse-bit stream (`0x110de80`).
///
/// This composes the validated header parser with the observed mode-specific
/// table builders. It intentionally does not initialize or mutate the four rANS
/// states used by mode 0 dispatch; those states live in the surrounding
/// `0x110dc30` segment loop and are passed through `0x110de00`.
pub fn rans_build_segment_descriptor(
    payload: &[u8],
    reader: RansFreqReader,
) -> Result<RansBuiltSegmentDescriptor, RansSegmentDescriptorBuildError> {
    let header = rans_read_segment_header(payload, reader)
        .map_err(RansSegmentDescriptorBuildError::Header)?;
    match header.mode {
        0 => {
            let table_count = header
                .table_count
                .ok_or(RansSegmentDescriptorBuildError::MissingTableCount)?;
            let built = rans_build_mode0_table(payload, header.reader, table_count, header.log)
                .map_err(RansSegmentDescriptorBuildError::Mode0)?;
            Ok(RansBuiltSegmentDescriptor {
                mode: 0,
                log: header.log,
                value: 0,
                step: built.table.step,
                sym: built.table.sym,
                reader: built.reader,
            })
        }
        1 => {
            let table_count = header
                .table_count
                .ok_or(RansSegmentDescriptorBuildError::MissingTableCount)?;
            let built = rans_build_mode1_table(payload, header.reader, table_count, header.log)
                .map_err(RansSegmentDescriptorBuildError::Mode1)?;
            Ok(RansBuiltSegmentDescriptor {
                mode: 1,
                log: header.log,
                value: 0,
                step: built.table,
                sym: Vec::new(),
                reader: built.reader,
            })
        }
        2 => {
            if header.value > u16::MAX as u32 {
                return Err(RansSegmentDescriptorBuildError::RleValueTooLarge(
                    header.value,
                ));
            }
            Ok(RansBuiltSegmentDescriptor {
                mode: 2,
                log: 0,
                value: header.value as u16,
                step: Vec::new(),
                sym: Vec::new(),
                reader: header.reader,
            })
        }
        mode => Err(RansSegmentDescriptorBuildError::UnknownMode(mode)),
    }
}

/// Mutable context threaded through the `0x110dc30` segment loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RansSegmentLoopContext {
    /// Primary reverse reader at `x5+0`: descriptor headers and run codes use it.
    pub reader: RansFreqReader,
    /// Extra mode-1 readers at `x5+0x18` and `x5+0x30`.
    pub mode1_extra_readers: [RansThreeLaneReader; 2],
    /// Payload-relative forward stream pointer stored at `x5+0x48`.
    pub stream_pos: usize,
    /// rANS state buffer living at descriptor workspace `x4+0x10`.
    pub state: RansStateBuffer,
}

/// Minimal `0x110d7f0` byte-group reader state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteGroupReadState {
    /// Primary reverse selector/descriptor reader at `x6+0`.
    pub reader: RansThreeLaneReader,
    /// Payload-relative forward byte stream pointer at `x6+0x48`.
    pub stream_pos: usize,
}

/// Inputs for one `0x110d7f0` byte-group reader call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteGroupReadSpec<'a> {
    /// Payload bytes addressed by both the reverse reader and forward stream.
    pub payload: &'a [u8],
    /// Register `w2`: element-size shift applied after `count * group_stride`.
    pub element_shift: u32,
    /// Register `w3`: group width multiplier.
    pub group_stride: usize,
    /// Register `w4`: number of groups to materialize.
    pub count: usize,
}

/// Bytes returned by `0x110d7f0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteGroupRead {
    /// Two-bit selector consumed from the reverse reader.
    pub selector: u8,
    /// Materialized byte stream for the caller.
    pub bytes: Vec<u8>,
}

/// Errors from the byte-group reader (`0x110d7f0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ByteGroupReadError {
    /// The reverse selector load tried to read outside the payload.
    PayloadTooSmall,
    /// `count * group_stride << element_shift` overflowed.
    OutputSizeOverflow,
    /// The direct selector's forward byte slice was truncated.
    StreamTooShort,
    /// Selectors 0, 1, and 2 require the byte/zstd segment paths not yet ported.
    UnportedSelector(u8),
}

/// Inputs for the segment loop (`0x110dc30`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RansSegmentLoopSpec<'a> {
    /// Byte count passed in `w1`; the loop writes `byte_count / 2` u16 symbols.
    pub byte_count: usize,
    /// Interleaved lane count / dispatch stride (`w2`).
    pub lanes: usize,
    /// Segment run granularity as `log2(segment_size)` (`w3`).
    pub segment_log: u32,
    /// Payload bytes addressed by all reverse readers and the forward rANS stream.
    pub payload: &'a [u8],
}

/// Errors from the segment loop (`0x110dc30`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RansSegmentLoopError {
    /// `w1` must describe whole u16 slots.
    OddByteCount,
    /// `w2 == 0` would make the dispatch stride zero.
    ZeroLaneCount,
    /// The observed caller keeps this at or below 10; larger values risk overflow.
    UnsupportedSegmentLog(u32),
    /// The byte count must divide evenly into the interleaved lanes.
    UnevenLaneSlots { slots: usize, lanes: usize },
    /// Caller output must include the logical slots plus `lanes-1` padding slots.
    OutputTooSmall,
    /// The run-code reverse reader tried to load outside the payload.
    PayloadTooSmall,
    /// A malformed all-zero run prefix would require more than 64 bits.
    RunCodeTooLong,
    /// Run/count arithmetic overflowed.
    RunCountOverflow,
    /// The shared forward stream pointer was outside the payload.
    StreamPointerOutOfBounds,
    /// Descriptor build rejected the segment header/table.
    Descriptor(RansSegmentDescriptorBuildError),
    /// Mode 1 has not been observed inside `0x110dc30`; guard until captured.
    UnobservedMode1Segment,
    /// The already-ported dispatch wrapper rejected the segment.
    Dispatch(RansSegmentDispatchError),
}

fn read_segment_loop_run_code(
    payload: &[u8],
    reader: &mut RansFreqReader,
) -> Result<usize, RansSegmentLoopError> {
    let bitpos = reader.bitpos;
    let ptr_step = ((bitpos >> 3) ^ 7) as usize;
    let ptr = reader
        .ptr
        .checked_sub(ptr_step)
        .ok_or(RansSegmentLoopError::PayloadTooSmall)?;
    let bits = (checked_header_u64_le(payload, reader.ptr)
        .map_err(|_| RansSegmentLoopError::PayloadTooSmall)?
        >> (bitpos & 63))
        | reader.acc;
    let run_bits = 1 + 2 * clz64(bits);
    if run_bits > 64 {
        return Err(RansSegmentLoopError::RunCodeTooLong);
    }
    let run = (bits >> (64 - run_bits)) as usize;
    reader.ptr = ptr;
    reader.acc = if run_bits == 64 { 0 } else { bits << run_bits };
    reader.bitpos = (bitpos | 0x38).wrapping_sub(run_bits);
    Ok(run)
}

fn checked_byte_group_u64_le(buf: &[u8], ptr: usize) -> Result<u64, ByteGroupReadError> {
    let end = ptr
        .checked_add(8)
        .ok_or(ByteGroupReadError::PayloadTooSmall)?;
    let bytes = buf
        .get(ptr..end)
        .ok_or(ByteGroupReadError::PayloadTooSmall)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn take_byte_group_selector(
    payload: &[u8],
    reader: &mut RansThreeLaneReader,
) -> Result<u8, ByteGroupReadError> {
    let bitpos = reader.bitpos;
    let step = ((bitpos >> 3) ^ 7) as usize;
    let ptr = reader
        .ptr
        .checked_sub(step)
        .ok_or(ByteGroupReadError::PayloadTooSmall)?;
    let bits = checked_byte_group_u64_le(payload, reader.ptr)?
        .checked_shr(bitpos & 63)
        .unwrap_or(0)
        | reader.acc;
    reader.ptr = ptr;
    reader.acc = bits << 2;
    reader.bitpos = (bitpos | 0x38).wrapping_sub(2);
    Ok((bits >> 62) as u8)
}

/// Read one byte-group stream (`0x110d7f0`).
///
/// This chunk ports the observed selector-3 branch only: the common selector
/// prologue consumes two reverse-reader bits (`0x110d808..0x110d854`), then
/// selector 3 returns the current forward stream slice and advances
/// `x6+0x48` by `(w4 * w3) << w2` (`0x110da00..0x110dab8`). Selectors 0, 1,
/// and 2 are reached by the current fixtures but route through the unported
/// byte/zstd segment loops, so they are typed errors until their replay lands.
pub fn byte_group_read(
    state: &mut ByteGroupReadState,
    spec: ByteGroupReadSpec<'_>,
) -> Result<ByteGroupRead, ByteGroupReadError> {
    let group_bytes = spec
        .count
        .checked_mul(spec.group_stride)
        .ok_or(ByteGroupReadError::OutputSizeOverflow)?;
    let out_len = group_bytes
        .checked_shl(spec.element_shift)
        .ok_or(ByteGroupReadError::OutputSizeOverflow)?;
    let selector = take_byte_group_selector(spec.payload, &mut state.reader)?;
    match selector {
        3 => {
            let end = state
                .stream_pos
                .checked_add(out_len)
                .ok_or(ByteGroupReadError::StreamTooShort)?;
            let bytes = spec
                .payload
                .get(state.stream_pos..end)
                .ok_or(ByteGroupReadError::StreamTooShort)?
                .to_vec();
            state.stream_pos = end;
            Ok(ByteGroupRead { selector, bytes })
        }
        selector => Err(ByteGroupReadError::UnportedSelector(selector)),
    }
}

fn ceil_div_segment(value: usize, segment_mask: usize, segment_log: u32) -> Option<usize> {
    value.checked_add(segment_mask).map(|v| v >> segment_log)
}

/// Decode one `0x110dc30` segment loop into an interleaved u16 output buffer.
///
/// The loop first builds a descriptor (`0x110dc90..0x110dc98`), then reads a
/// CLZ-prefixed run count from the shared reverse reader (`0x110dc9c..0x110dcdc`).
/// The run is measured in `1 << segment_log` symbol chunks and may cross lane
/// boundaries: `0x110dd1c..0x110dd24` either advances the lane or carries the
/// current descriptor into the next slice. Dispatch itself is delegated to the
/// already-validated `0x110de00` wrapper.
///
/// The observed enumerate-all population is one Bass call with mode 0 + mode 2
/// descriptors. Mode 1 is guarded here because its extra-reader threading inside
/// this loop has not been observed, even though `0x110de00` mode 1 is ported.
pub fn rans_segment_loop_into(
    out: &mut [u16],
    context: &mut RansSegmentLoopContext,
    spec: RansSegmentLoopSpec<'_>,
) -> Result<usize, RansSegmentLoopError> {
    if spec.byte_count & 1 != 0 {
        return Err(RansSegmentLoopError::OddByteCount);
    }
    if spec.lanes == 0 {
        return Err(RansSegmentLoopError::ZeroLaneCount);
    }
    if spec.segment_log > 30 {
        return Err(RansSegmentLoopError::UnsupportedSegmentLog(
            spec.segment_log,
        ));
    }

    let logical_slots = spec.byte_count >> 1;
    if !logical_slots.is_multiple_of(spec.lanes) {
        return Err(RansSegmentLoopError::UnevenLaneSlots {
            slots: logical_slots,
            lanes: spec.lanes,
        });
    }
    let padded_slots = logical_slots
        .checked_add(spec.lanes - 1)
        .ok_or(RansSegmentLoopError::OutputTooSmall)?;
    if out.len() < padded_slots {
        return Err(RansSegmentLoopError::OutputTooSmall);
    }

    let symbols_per_lane = logical_slots / spec.lanes;
    let segment_size = 1usize << spec.segment_log;
    let segment_mask = segment_size - 1;
    let mut lane = 0usize;
    let mut lane_offset = 0usize;
    let mut dispatch_count = 0usize;

    while lane < spec.lanes {
        let descriptor = rans_build_segment_descriptor(spec.payload, context.reader)
            .map_err(RansSegmentLoopError::Descriptor)?;
        context.reader = descriptor.reader;
        let mut run_segments = read_segment_loop_run_code(spec.payload, &mut context.reader)?;

        if descriptor.mode == 1 {
            return Err(RansSegmentLoopError::UnobservedMode1Segment);
        }

        loop {
            let remaining = symbols_per_lane
                .checked_sub(lane_offset)
                .ok_or(RansSegmentLoopError::RunCountOverflow)?;
            let run_symbols = run_segments
                .checked_mul(segment_size)
                .ok_or(RansSegmentLoopError::RunCountOverflow)?;
            let finish_segments = ceil_div_segment(remaining, segment_mask, spec.segment_log)
                .ok_or(RansSegmentLoopError::RunCountOverflow)?;
            let finishes_lane = run_segments >= finish_segments;
            let count = if finishes_lane {
                remaining
            } else {
                run_symbols
            };
            let out_start = lane_offset
                .checked_mul(spec.lanes)
                .and_then(|v| v.checked_add(lane))
                .ok_or(RansSegmentLoopError::OutputTooSmall)?;
            let dispatch_len = count
                .checked_mul(spec.lanes)
                .ok_or(RansSegmentLoopError::OutputTooSmall)?;
            let out_end = out_start
                .checked_add(dispatch_len)
                .ok_or(RansSegmentLoopError::OutputTooSmall)?;
            let out_window = out
                .get_mut(out_start..out_end)
                .ok_or(RansSegmentLoopError::OutputTooSmall)?;

            match descriptor.mode {
                0 => {
                    let stream = spec
                        .payload
                        .get(context.stream_pos..)
                        .ok_or(RansSegmentLoopError::StreamPointerOutOfBounds)?;
                    let used = rans_segment_dispatch_into(
                        out_window,
                        RansSegmentDispatchSpec {
                            mode: descriptor.mode,
                            log: descriptor.log,
                            value: descriptor.value,
                            count,
                            stride: spec.lanes,
                            states: &mut context.state.states,
                            step: &descriptor.step,
                            sym: &descriptor.sym,
                            stream,
                            payload: spec.payload,
                            three_lane_readers: None,
                        },
                    )
                    .map_err(RansSegmentLoopError::Dispatch)?;
                    context.stream_pos = context
                        .stream_pos
                        .checked_add(used)
                        .ok_or(RansSegmentLoopError::StreamPointerOutOfBounds)?;
                }
                2 => {
                    rans_segment_dispatch_into(
                        out_window,
                        RansSegmentDispatchSpec {
                            mode: descriptor.mode,
                            log: descriptor.log,
                            value: descriptor.value,
                            count,
                            stride: spec.lanes,
                            states: &mut context.state.states,
                            step: &descriptor.step,
                            sym: &descriptor.sym,
                            stream: &[],
                            payload: spec.payload,
                            three_lane_readers: None,
                        },
                    )
                    .map_err(RansSegmentLoopError::Dispatch)?;
                }
                mode => {
                    return Err(RansSegmentLoopError::Dispatch(
                        RansSegmentDispatchError::UnknownMode(mode),
                    ));
                }
            }

            dispatch_count += 1;
            let consumed_segments = ceil_div_segment(count, segment_mask, spec.segment_log)
                .ok_or(RansSegmentLoopError::RunCountOverflow)?;
            if finishes_lane {
                lane += 1;
                lane_offset = 0;
            } else {
                lane_offset = lane_offset
                    .checked_add(run_symbols)
                    .ok_or(RansSegmentLoopError::RunCountOverflow)?;
            }
            run_segments = run_segments
                .checked_sub(consumed_segments)
                .ok_or(RansSegmentLoopError::RunCountOverflow)?;
            if run_segments == 0 {
                break;
            }
        }
    }

    Ok(dispatch_count)
}

/// Result of the `0x110d360` width combiner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WidthCombinerResult {
    /// `w0` on return: sum of the first stream's decoded widths.
    pub ret: u32,
    /// Bytes consumed from the three stream pointers stored at `x2`.
    pub consumed: [usize; 3],
}

/// Inputs for the 3-stream width combiner (`0x110d360`).
pub struct WidthCombinerSpec<'a> {
    /// Number of 8-byte records to write (`w1`).
    pub count: usize,
    /// Stride multiplier (`w4`).
    pub stride: u32,
    /// High-group shift for third-stream special codes (`w5`).
    pub shift: u32,
    /// Attribute byte width added to the second stream (`w6`).
    pub attr_width: u32,
    /// Vertex/count limit used by the tail clamp (`w7`).
    pub limit: u32,
    /// Payload bytes addressed by the reversed forward bit reader.
    pub payload: &'a [u8],
    /// First byte stream (`x2+0`).
    pub stream0: &'a [u8],
    /// Second byte stream (`x2+8`).
    pub stream1: &'a [u8],
    /// Third little-endian u16 stream (`x2+0x10`).
    pub stream2: &'a [u8],
    /// Reversed forward bit reader at `x3`.
    pub reader: &'a mut RansThreeLaneReader,
}

/// Errors from the 3-stream width combiner (`0x110d360`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WidthCombinerError {
    /// The captured population has `count >= 2`; the tail-only branch is guarded.
    UnobservedTailOnlyCount(usize),
    /// Output must contain `count` records.
    OutputTooSmall,
    /// The reversed forward bit reader tried to load outside the payload.
    PayloadTooSmall,
    /// A stream ended before the disassembly would read from it.
    StreamTooShort { stream: u8 },
    /// Byte expansion code was outside the observed 0x10..0x1f table.
    ExpansionCodeTooLarge(u8),
    /// Reader bit arithmetic underflowed.
    ReaderUnderflow,
    /// A small third-stream history reference pointed before the output history.
    HistoryOutOfBounds,
    /// Shift/count arithmetic overflowed.
    ArithmeticOverflow,
}

const WIDTH_EXPAND_TABLE: [(u32, u32); 16] = [
    (1, 0),
    (1, 2),
    (1, 4),
    (1, 6),
    (2, 8),
    (2, 12),
    (3, 16),
    (3, 24),
    (4, 32),
    (5, 48),
    (6, 80),
    (7, 144),
    (8, 272),
    (9, 528),
    (10, 1040),
    (11, 2064),
];

fn checked_width_u64_le(buf: &[u8], ptr: usize) -> Result<u64, WidthCombinerError> {
    let end = ptr
        .checked_add(8)
        .ok_or(WidthCombinerError::PayloadTooSmall)?;
    let bytes = buf
        .get(ptr..end)
        .ok_or(WidthCombinerError::PayloadTooSmall)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn reload_width_reader(
    payload: &[u8],
    reader: &mut RansThreeLaneReader,
) -> Result<(), WidthCombinerError> {
    let bitpos = reader.bitpos;
    let word = checked_width_u64_le(payload, reader.ptr)?.swap_bytes();
    let step = ((bitpos >> 3) ^ 7) as usize;
    reader.ptr = reader
        .ptr
        .checked_add(step)
        .ok_or(WidthCombinerError::PayloadTooSmall)?;
    reader.acc |= word >> (bitpos & 63);
    reader.bitpos = bitpos | 0x38;
    Ok(())
}

fn take_width_bits(
    payload: &[u8],
    reader: &mut RansThreeLaneReader,
    bits: u32,
) -> Result<u32, WidthCombinerError> {
    let bitpos = reader
        .bitpos
        .checked_sub(bits)
        .ok_or(WidthCombinerError::ReaderUnderflow)?;
    let extra = if bits == 0 {
        0
    } else {
        reader.acc >> (64 - bits)
    };
    let shifted = if bits == 64 { 0 } else { reader.acc << bits };
    let word = checked_width_u64_le(payload, reader.ptr)?.swap_bytes();
    let step = ((bitpos >> 3) ^ 7) as usize;
    reader.ptr = reader
        .ptr
        .checked_add(step)
        .ok_or(WidthCombinerError::PayloadTooSmall)?;
    reader.acc = shifted | (word >> (bitpos & 63));
    reader.bitpos = bitpos | 0x38;
    Ok(extra as u32)
}

fn decode_width_byte(
    payload: &[u8],
    reader: &mut RansThreeLaneReader,
    byte: u8,
) -> Result<u32, WidthCombinerError> {
    if byte < 0x10 {
        return Ok(byte as u32);
    }
    let (bits, base) = *WIDTH_EXPAND_TABLE
        .get((byte - 0x10) as usize)
        .ok_or(WidthCombinerError::ExpansionCodeTooLarge(byte))?;
    take_width_bits(payload, reader, bits).map(|extra| base + extra + 0x10)
}

fn read_width_u8(stream: &[u8], pos: &mut usize, stream_id: u8) -> Result<u8, WidthCombinerError> {
    let byte = stream
        .get(*pos)
        .copied()
        .ok_or(WidthCombinerError::StreamTooShort { stream: stream_id })?;
    *pos += 1;
    Ok(byte)
}

fn read_width_u16(
    stream: &[u8],
    pos: &mut usize,
    stream_id: u8,
) -> Result<u16, WidthCombinerError> {
    let end = pos
        .checked_add(2)
        .ok_or(WidthCombinerError::StreamTooShort { stream: stream_id })?;
    let bytes = stream
        .get(*pos..end)
        .ok_or(WidthCombinerError::StreamTooShort { stream: stream_id })?;
    *pos = end;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn decode_width_third_special(
    payload: &[u8],
    reader: &mut RansThreeLaneReader,
    code: u16,
    stride: u32,
    shift: u32,
) -> Result<u32, WidthCombinerError> {
    let raw = (code - 3) as u32;
    let bits = raw & 0x1f;
    let high = raw >> 5;
    let extra = take_width_bits(payload, reader, bits)?;
    let low = extra
        .checked_add((1u32 << bits) - 1)
        .ok_or(WidthCombinerError::ArithmeticOverflow)?;
    let scaled_low = low
        .checked_mul(stride)
        .ok_or(WidthCombinerError::ArithmeticOverflow)?;
    let scaled_high = high
        .checked_shl(shift)
        .ok_or(WidthCombinerError::ArithmeticOverflow)?;
    scaled_low
        .checked_add(scaled_high)
        .ok_or(WidthCombinerError::ArithmeticOverflow)
}

fn width_history_value(
    out: &[[u32; 2]],
    history_index: i64,
    index: u32,
) -> Result<u32, WidthCombinerError> {
    let source = history_index
        .checked_sub(index as i64)
        .ok_or(WidthCombinerError::HistoryOutOfBounds)?;
    if source < 0 {
        return Err(WidthCombinerError::HistoryOutOfBounds);
    }
    out.get(source as usize)
        .map(|record| record[1])
        .ok_or(WidthCombinerError::HistoryOutOfBounds)
}

/// Combine three width streams into `count` records (`0x110d360`).
///
/// Stream 0 and stream 1 store inline values for bytes below `0x10`; larger
/// bytes index the table at `0x2cf69fc`, then pull MSB-first bits from the
/// reversed forward reader (`0x110d498..0x110d4d4` and `0x110d4e4..0x110d520`).
/// Stream 2 either references output history for codes `0..=2` or expands a
/// special code via `(code-3)&0x1f` reader bits plus `(code-3)>>5` shifted by
/// `w5` (`0x110d3d0..0x110d420`, mirrored in the tail at `0x110d574..0x110d5cc`).
pub fn width_combiner_into(
    out: &mut [[u32; 2]],
    spec: WidthCombinerSpec<'_>,
) -> Result<WidthCombinerResult, WidthCombinerError> {
    if spec.count < 2 {
        return Err(WidthCombinerError::UnobservedTailOnlyCount(spec.count));
    }
    if out.len() < spec.count {
        return Err(WidthCombinerError::OutputTooSmall);
    }

    reload_width_reader(spec.payload, spec.reader)?;

    let mut pos0 = 0usize;
    let mut pos1 = 0usize;
    let mut pos2 = 0usize;
    let mut sum_first = 0u32;
    let mut sum_width = 0u32;
    let mut zero_history = 0i64;
    let mut history_index = -1i64;

    for output_index in 0..(spec.count - 1) {
        let first_raw = read_width_u8(spec.stream0, &mut pos0, 0)?;
        let first = decode_width_byte(spec.payload, spec.reader, first_raw)?;
        let second_raw = read_width_u8(spec.stream1, &mut pos1, 1)?;
        let second = decode_width_byte(spec.payload, spec.reader, second_raw)?;
        let third_code = read_width_u16(spec.stream2, &mut pos2, 2)?;
        let (third, advance) = if third_code > 2 {
            (
                decode_width_third_special(
                    spec.payload,
                    spec.reader,
                    third_code,
                    spec.stride,
                    spec.shift,
                )?,
                zero_history + 1,
            )
        } else {
            let index = third_code as u32 + u32::from(first == 0);
            let third = width_history_value(out, history_index, index)?;
            if index == 0 {
                zero_history += 1;
                (third, 0)
            } else {
                let advance = zero_history + 1;
                zero_history = 0;
                (third, advance)
            }
        };
        if third_code > 2 {
            zero_history = 0;
        }

        let second_width = second
            .checked_add(spec.attr_width)
            .ok_or(WidthCombinerError::ArithmeticOverflow)?;
        sum_first = sum_first
            .checked_add(first)
            .ok_or(WidthCombinerError::ArithmeticOverflow)?;
        sum_width = sum_width
            .checked_add(first)
            .and_then(|v| v.checked_add(second_width))
            .ok_or(WidthCombinerError::ArithmeticOverflow)?;
        out[output_index] = [first | (second_width << 16), third];
        history_index = history_index
            .checked_add(advance)
            .ok_or(WidthCombinerError::ArithmeticOverflow)?;
    }

    let first_raw = read_width_u8(spec.stream0, &mut pos0, 0)?;
    let first = decode_width_byte(spec.payload, spec.reader, first_raw)?;
    let used_width = sum_width
        .checked_add(first)
        .ok_or(WidthCombinerError::ArithmeticOverflow)?;
    let (remaining_limit, third) = if spec.limit <= used_width {
        (0, 0)
    } else {
        let remaining = spec.limit - used_width;
        let third_code = read_width_u16(spec.stream2, &mut pos2, 2)?;
        let third = if third_code > 2 {
            decode_width_third_special(
                spec.payload,
                spec.reader,
                third_code,
                spec.stride,
                spec.shift,
            )?
        } else {
            let index = third_code as u32 + u32::from(first == 0);
            width_history_value(out, history_index, index)?
        };
        (remaining, third)
    };

    out[spec.count - 1] = [first | (remaining_limit << 16), third];
    let ret = sum_first
        .checked_add(first)
        .ok_or(WidthCombinerError::ArithmeticOverflow)?;
    Ok(WidthCombinerResult {
        ret,
        consumed: [pos0, pos1, pos2],
    })
}

/// One run/copy record consumed by the `0x10fc5e0` byte-copy transform tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransformTailRecord {
    /// Number of literal bytes copied from source stream 0.
    pub literal_count: u16,
    /// Number of bytes copied from earlier output at `back_distance`.
    pub copy_count: u16,
    /// Byte distance from the current output position to the copy source.
    pub back_distance: usize,
}

/// Inputs for the single-byte transform tail (`0x10fc5e0`).
pub struct TransformTailCopy1Spec<'a> {
    /// Entry high byte (`entry >> 24`): byte distance between consecutive vertices.
    pub output_stride: usize,
    /// Block index at `[x0+0xa0]`, folded into the initial output position.
    pub block_index: usize,
    /// Per-entry output byte offset from `[x0 + current*4 + 0x64]`.
    pub out_offset: usize,
    /// Run/copy records at `x2`.
    pub records: &'a [TransformTailRecord],
    /// Source stream pointer at `[x4]`.
    pub source: &'a [u8],
}

/// Inputs for the two-byte transform tail (`0x10fc680`).
pub struct TransformTailCopy2Spec<'a> {
    /// Entry high byte (`entry >> 24`): byte distance between consecutive vertices.
    pub output_stride: usize,
    /// Block index at `[x0+0xa0]`, folded into the initial output position.
    pub block_index: usize,
    /// Per-entry output byte offset from `[x0 + current*4 + 0x64]`.
    pub out_offset: usize,
    /// Run/copy records at `x2`.
    pub records: &'a [TransformTailRecord],
    /// Source stream pointer at `[x4]`.
    pub source: &'a [u8],
}

/// Inputs for the four-byte transform tail (`0x10fc7d0`).
pub struct TransformTailCopy4Spec<'a> {
    /// Entry high byte (`entry >> 24`): byte distance between consecutive vertices.
    pub output_stride: usize,
    /// Block index at `[x0+0xa0]`, folded into the initial output position.
    pub block_index: usize,
    /// Per-entry output byte offset from `[x0 + current*4 + 0x64]`.
    pub out_offset: usize,
    /// Run/copy records at `x2`.
    pub records: &'a [TransformTailRecord],
    /// Source stream pointer at `[x4]`.
    pub source: &'a [u8],
}

/// Errors from fixed-width transform copy tails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransformTailCopyError {
    ZeroStride,
    OutputTooSmall,
    SourceTooSmall,
    CopyBeforeOutput,
    ArithmeticOverflow,
    UnobservedRecordShape,
}

struct TransformTailCopyUnitsSpec<'a> {
    output_stride: usize,
    block_index: usize,
    out_offset: usize,
    records: &'a [TransformTailRecord],
    source: &'a [u8],
    unit_size: usize,
    allow_zero_literal: bool,
    allow_zero_copy: bool,
}

/// Apply the observed single-byte copy transform tail (`0x10fc5e0`).
///
/// Each record first writes `literal_count` bytes from source stream 0, stepping
/// the output cursor by `entry >> 24`; if the high halfword is non-zero, it then
/// copies that many bytes from `back_distance` bytes behind the current cursor.
/// The copy distance is in bytes, not vertex slots (`0x10fc610..0x10fc664`).
pub fn transform_tail_copy1_into(
    out: &mut [u8],
    spec: TransformTailCopy1Spec<'_>,
) -> Result<usize, TransformTailCopyError> {
    transform_tail_copy_units_into(
        out,
        TransformTailCopyUnitsSpec {
            output_stride: spec.output_stride,
            block_index: spec.block_index,
            out_offset: spec.out_offset,
            records: spec.records,
            source: spec.source,
            unit_size: 1,
            allow_zero_literal: true,
            allow_zero_copy: true,
        },
    )
}

/// Apply the observed two-byte copy transform tail (`0x10fc680`).
///
/// This is the `ldrh`/`strh` sibling of `0x10fc5e0`: literals and copies move
/// two bytes per record unit, while cursor advance and back-distance remain
/// byte counts (`0x10fc6c0..0x10fc6fc`). The sole observed call has non-zero
/// literal and copy counts in every record, so zero-count branch shapes are
/// rejected until captured.
pub fn transform_tail_copy2_into(
    out: &mut [u8],
    spec: TransformTailCopy2Spec<'_>,
) -> Result<usize, TransformTailCopyError> {
    transform_tail_copy_units_into(
        out,
        TransformTailCopyUnitsSpec {
            output_stride: spec.output_stride,
            block_index: spec.block_index,
            out_offset: spec.out_offset,
            records: spec.records,
            source: spec.source,
            unit_size: 2,
            allow_zero_literal: false,
            allow_zero_copy: false,
        },
    )
}

/// Apply the observed four-byte copy transform tail (`0x10fc7d0`).
///
/// This is the `ldr`/`str` sibling of `0x10fc5e0`: literals and copies move
/// four bytes per record unit, while cursor advance and back-distance remain
/// byte counts (`0x10fc810..0x10fc84c`).
pub fn transform_tail_copy4_into(
    out: &mut [u8],
    spec: TransformTailCopy4Spec<'_>,
) -> Result<usize, TransformTailCopyError> {
    transform_tail_copy_units_into(
        out,
        TransformTailCopyUnitsSpec {
            output_stride: spec.output_stride,
            block_index: spec.block_index,
            out_offset: spec.out_offset,
            records: spec.records,
            source: spec.source,
            unit_size: 4,
            allow_zero_literal: true,
            allow_zero_copy: true,
        },
    )
}

/// Inputs for the two-byte delta-match transform tail (`0x10fbcc0`).
pub struct TransformTailDelta2Spec<'a> {
    /// Entry high byte (`entry >> 24`): byte distance between consecutive vertices.
    pub output_stride: usize,
    /// Block index at `[x0+0xa0]`, folded into the initial output position.
    pub block_index: usize,
    /// Per-entry output byte offset from `[x0 + current*4 + 0x64]`.
    pub out_offset: usize,
    /// Run/copy records at `x2`.
    pub records: &'a [TransformTailRecord],
    /// Match table at `[x0+0x10]`, indexed by emitted vertex.
    pub matches: &'a [u32],
    /// Direct literal source stream at `[x4]`.
    pub source0: &'a [u8],
    /// Direct literal second-byte stream at `[x4+8]`.
    pub source1: &'a [u8],
    /// Matched delta stream at `[x4+0x10]`.
    pub source2: &'a [u8],
}

/// Inputs for the three-byte delta-match transform tail (`0x10fbdc0`).
pub struct TransformTailDelta3Spec<'a> {
    /// Entry high byte (`entry >> 24`): byte distance between consecutive vertices.
    pub output_stride: usize,
    /// Block index at `[x0+0xa0]`, folded into the initial output position.
    pub block_index: usize,
    /// Per-entry output byte offset from `[x0 + current*4 + 0x64]`.
    pub out_offset: usize,
    /// Run/copy records at `x2`.
    pub records: &'a [TransformTailRecord],
    /// Match table at `[x0+0x10]`, indexed by emitted vertex.
    pub matches: &'a [u32],
    /// Direct literal source stream at `[x4]`.
    pub source0: &'a [u8],
    /// Direct literal bytes 1 and 2 stream at `[x4+8]`.
    pub source1: &'a [u8],
    /// Matched delta stream at `[x4+0x10]`.
    pub source2: &'a [u8],
}

/// Source and table consumption from a delta-match transform tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransformTailDeltaUsage {
    pub source0: usize,
    pub source1: usize,
    pub source2: usize,
    pub match_entries: usize,
}

/// Errors from delta-match transform tails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransformTailDeltaError {
    ZeroStride,
    OutputTooSmall,
    Source0TooSmall,
    Source1TooSmall,
    Source2TooSmall,
    MatchTableTooSmall,
    MatchBeforeOutput,
    CopyBeforeOutput,
    ArithmeticOverflow,
}

/// Apply the observed two-byte delta-match transform tail (`0x10fbcc0`).
///
/// Direct literals use source streams 0 and 1: byte 1 is copied from stream 1,
/// and byte 0 is `source0 - byte1 - 1`. Matched literals use the match table's
/// `entry >> 3` distance in vertices, add two source-2 deltas to the earlier
/// output, and then advance the same strided cursor (`0x10fbd04..0x10fbd70`).
pub fn transform_tail_delta2_into(
    out: &mut [u8],
    spec: TransformTailDelta2Spec<'_>,
) -> Result<TransformTailDeltaUsage, TransformTailDeltaError> {
    if spec.output_stride == 0 {
        return Err(TransformTailDeltaError::ZeroStride);
    }
    let mut cursor = spec
        .block_index
        .checked_mul(spec.output_stride)
        .and_then(|offset| offset.checked_add(spec.out_offset))
        .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
    let mut match_index = 0usize;
    let mut source0_pos = 0usize;
    let mut source1_pos = 0usize;
    let mut source2_pos = 0usize;
    let mut chunk = [0u8; 2];

    for record in spec.records {
        for _ in 0..record.literal_count {
            let match_entry = *spec
                .matches
                .get(match_index)
                .ok_or(TransformTailDeltaError::MatchTableTooSmall)?;
            let cursor_end = cursor
                .checked_add(2)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            let (first, second) = if match_entry == 0 {
                let second = *spec
                    .source1
                    .get(source1_pos)
                    .ok_or(TransformTailDeltaError::Source1TooSmall)?;
                let first_raw = *spec
                    .source0
                    .get(source0_pos)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                source0_pos += 1;
                source1_pos += 1;
                (first_raw.wrapping_sub(second).wrapping_sub(1), second)
            } else {
                let match_units = (match_entry >> 3) as usize;
                let match_distance = match_units
                    .checked_mul(spec.output_stride)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let source = cursor
                    .checked_sub(match_distance)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                let source_end = source
                    .checked_add(2)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let base = out
                    .get(source..source_end)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                let source2_end = source2_pos
                    .checked_add(2)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let delta = spec
                    .source2
                    .get(source2_pos..source2_end)
                    .ok_or(TransformTailDeltaError::Source2TooSmall)?;
                source2_pos = source2_end;
                (
                    delta[0].wrapping_add(base[0]),
                    delta[1].wrapping_add(base[1]),
                )
            };
            let slot = out
                .get_mut(cursor..cursor_end)
                .ok_or(TransformTailDeltaError::OutputTooSmall)?;
            slot[0] = first;
            slot[1] = second;
            cursor = cursor
                .checked_add(spec.output_stride)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            match_index += 1;
        }

        for _ in 0..record.copy_count {
            if record.back_distance == 0 {
                return Err(TransformTailDeltaError::CopyBeforeOutput);
            }
            let source = cursor
                .checked_sub(record.back_distance)
                .ok_or(TransformTailDeltaError::CopyBeforeOutput)?;
            let source_end = source
                .checked_add(2)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            let cursor_end = cursor
                .checked_add(2)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            let value = out
                .get(source..source_end)
                .ok_or(TransformTailDeltaError::CopyBeforeOutput)?;
            chunk.copy_from_slice(value);
            let slot = out
                .get_mut(cursor..cursor_end)
                .ok_or(TransformTailDeltaError::OutputTooSmall)?;
            slot.copy_from_slice(&chunk);
            cursor = cursor
                .checked_add(spec.output_stride)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
        }

        match_index = match_index
            .checked_add(usize::from(record.copy_count))
            .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
        if match_index > spec.matches.len() {
            return Err(TransformTailDeltaError::MatchTableTooSmall);
        }
    }

    Ok(TransformTailDeltaUsage {
        source0: source0_pos,
        source1: source1_pos,
        source2: source2_pos,
        match_entries: match_index,
    })
}

/// Apply the observed three-byte delta-match transform tail (`0x10fbdc0`).
///
/// Direct literals use source streams 0 and 1: bytes 1 and 2 are copied from
/// source1, and byte 0 is `source0 - byte1 - byte2 - 1`. Matched literals use
/// the match table's `entry >> 3` distance in vertices, add three source-2
/// deltas to earlier output, and then advance the same strided cursor
/// (`0x10fbe04..0x10fbec0`).
pub fn transform_tail_delta3_into(
    out: &mut [u8],
    spec: TransformTailDelta3Spec<'_>,
) -> Result<TransformTailDeltaUsage, TransformTailDeltaError> {
    if spec.output_stride == 0 {
        return Err(TransformTailDeltaError::ZeroStride);
    }
    let mut cursor = spec
        .block_index
        .checked_mul(spec.output_stride)
        .and_then(|offset| offset.checked_add(spec.out_offset))
        .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
    let mut match_index = 0usize;
    let mut source0_pos = 0usize;
    let mut source1_pos = 0usize;
    let mut source2_pos = 0usize;
    let mut chunk = [0u8; 3];

    for record in spec.records {
        for _ in 0..record.literal_count {
            let match_entry = *spec
                .matches
                .get(match_index)
                .ok_or(TransformTailDeltaError::MatchTableTooSmall)?;
            let cursor_end = cursor
                .checked_add(3)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            let (first, second, third) = if match_entry == 0 {
                let source1_end = source1_pos
                    .checked_add(2)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let pair = spec
                    .source1
                    .get(source1_pos..source1_end)
                    .ok_or(TransformTailDeltaError::Source1TooSmall)?;
                let first_raw = *spec
                    .source0
                    .get(source0_pos)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                source0_pos += 1;
                source1_pos = source1_end;
                (
                    first_raw
                        .wrapping_sub(pair[0])
                        .wrapping_sub(pair[1])
                        .wrapping_sub(1),
                    pair[0],
                    pair[1],
                )
            } else {
                let match_units = (match_entry >> 3) as usize;
                let match_distance = match_units
                    .checked_mul(spec.output_stride)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let source = cursor
                    .checked_sub(match_distance)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                let source_end = source
                    .checked_add(3)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let base = out
                    .get(source..source_end)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                let source2_end = source2_pos
                    .checked_add(3)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let delta = spec
                    .source2
                    .get(source2_pos..source2_end)
                    .ok_or(TransformTailDeltaError::Source2TooSmall)?;
                source2_pos = source2_end;
                (
                    delta[0].wrapping_add(base[0]),
                    delta[1].wrapping_add(base[1]),
                    delta[2].wrapping_add(base[2]),
                )
            };
            let slot = out
                .get_mut(cursor..cursor_end)
                .ok_or(TransformTailDeltaError::OutputTooSmall)?;
            slot[0] = first;
            slot[1] = second;
            slot[2] = third;
            cursor = cursor
                .checked_add(spec.output_stride)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            match_index += 1;
        }

        for _ in 0..record.copy_count {
            if record.back_distance == 0 {
                return Err(TransformTailDeltaError::CopyBeforeOutput);
            }
            let source = cursor
                .checked_sub(record.back_distance)
                .ok_or(TransformTailDeltaError::CopyBeforeOutput)?;
            let source_end = source
                .checked_add(3)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            let cursor_end = cursor
                .checked_add(3)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            let value = out
                .get(source..source_end)
                .ok_or(TransformTailDeltaError::CopyBeforeOutput)?;
            chunk.copy_from_slice(value);
            let slot = out
                .get_mut(cursor..cursor_end)
                .ok_or(TransformTailDeltaError::OutputTooSmall)?;
            slot.copy_from_slice(&chunk);
            cursor = cursor
                .checked_add(spec.output_stride)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
        }

        match_index = match_index
            .checked_add(usize::from(record.copy_count))
            .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
        if match_index > spec.matches.len() {
            return Err(TransformTailDeltaError::MatchTableTooSmall);
        }
    }

    Ok(TransformTailDeltaUsage {
        source0: source0_pos,
        source1: source1_pos,
        source2: source2_pos,
        match_entries: match_index,
    })
}

fn transform_tail_copy_units_into(
    out: &mut [u8],
    spec: TransformTailCopyUnitsSpec<'_>,
) -> Result<usize, TransformTailCopyError> {
    if spec.output_stride == 0 {
        return Err(TransformTailCopyError::ZeroStride);
    }
    let mut cursor = spec
        .block_index
        .checked_mul(spec.output_stride)
        .and_then(|offset| offset.checked_add(spec.out_offset))
        .ok_or(TransformTailCopyError::ArithmeticOverflow)?;
    let mut source_pos = 0usize;
    let mut chunk = [0u8; 8];

    for record in spec.records {
        if (!spec.allow_zero_literal && record.literal_count == 0)
            || (!spec.allow_zero_copy && record.copy_count == 0)
        {
            return Err(TransformTailCopyError::UnobservedRecordShape);
        }

        for _ in 0..record.literal_count {
            let source_end = source_pos
                .checked_add(spec.unit_size)
                .ok_or(TransformTailCopyError::ArithmeticOverflow)?;
            let cursor_end = cursor
                .checked_add(spec.unit_size)
                .ok_or(TransformTailCopyError::ArithmeticOverflow)?;
            let value = spec
                .source
                .get(source_pos..source_end)
                .ok_or(TransformTailCopyError::SourceTooSmall)?;
            let slot = out
                .get_mut(cursor..cursor_end)
                .ok_or(TransformTailCopyError::OutputTooSmall)?;
            slot.copy_from_slice(value);
            source_pos = source_end;
            cursor = cursor
                .checked_add(spec.output_stride)
                .ok_or(TransformTailCopyError::ArithmeticOverflow)?;
        }

        for _ in 0..record.copy_count {
            if record.back_distance == 0 {
                return Err(TransformTailCopyError::CopyBeforeOutput);
            }
            let source = cursor
                .checked_sub(record.back_distance)
                .ok_or(TransformTailCopyError::CopyBeforeOutput)?;
            let source_end = source
                .checked_add(spec.unit_size)
                .ok_or(TransformTailCopyError::ArithmeticOverflow)?;
            let cursor_end = cursor
                .checked_add(spec.unit_size)
                .ok_or(TransformTailCopyError::ArithmeticOverflow)?;
            let value = out
                .get(source..source_end)
                .ok_or(TransformTailCopyError::CopyBeforeOutput)?;
            chunk[..spec.unit_size].copy_from_slice(value);
            let slot = out
                .get_mut(cursor..cursor_end)
                .ok_or(TransformTailCopyError::OutputTooSmall)?;
            slot.copy_from_slice(&chunk[..spec.unit_size]);
            cursor = cursor
                .checked_add(spec.output_stride)
                .ok_or(TransformTailCopyError::ArithmeticOverflow)?;
        }
    }

    Ok(source_pos)
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
    section: &super::MeshSection,
    payload: &[u8],
) -> Result<Vec<u8>, super::McError> {
    let map_z = |e: zstd_pure::ZstdError| super::McError::MeshFraming(format!("window zstd: {e}"));
    let map_m =
        |e: meshopt::MeshoptError| super::McError::MeshFraming(format!("index decode: {e}"));

    let sub_a = section.first_chunk.sub_a_size as usize;
    let align_a = (section.align_a as usize).max(1);
    let (t0, t1, pos) = parse_super_block_trailer(payload);
    if (t0, t1) != (0, 0) {
        return Err(super::McError::MeshFraming(format!(
            "unexpected super-block trailer ({t0},{t1}); multi-super-block not yet supported"
        )));
    }
    let mut fwd = ForwardReader::new(payload, pos);
    let _w27 = fwd.varint();
    let hdr = parse_sub_block_header(&mut fwd)
        .ok_or_else(|| super::McError::MeshFraming("empty first sub-block".into()))?;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn forward(bytes: &[u8]) -> ForwardReader<'_> {
        ForwardReader::new(bytes, 0)
    }

    #[test]
    fn forward_varint_msb_first() {
        // 0x89 0x3a -> (0x09 << 7) | 0x3a = 1210 (a real Bear window srcsize).
        assert_eq!(forward(&[0x89, 0x3a]).varint(), 1210);
        // 0x99 0x7f -> (0x19 << 7) | 0x7f = 3327 (Bear vertex count / w8).
        assert_eq!(forward(&[0x99, 0x7f]).varint(), 3327);
        // single byte < 128.
        assert_eq!(forward(&[0x48]).varint(), 0x48);
        // 0xb3 0x4e -> (0x33 << 7) | 0x4e = 6606 (Bear DESC#1.d).
        assert_eq!(forward(&[0xb3, 0x4e]).varint(), 6606);
    }

    #[test]
    fn leb_lsb_trailer() {
        assert_eq!(read_leb_lsb(&[0x00, 0x00, 0x0e], 0), (0, 1));
        let (a, p) = read_leb_lsb(&[0x00, 0x00, 0x0e], 0);
        let (b, _) = read_leb_lsb(&[0x00, 0x00, 0x0e], p);
        assert_eq!((a, b), (0, 0));
    }

    /// Load a committed `.mc` fixture, returning `None` if absent (so the suite
    /// stays green where fixtures aren't checked out).
    fn fixture(name: &str) -> Option<Vec<u8>> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/fixtures/mc/{name}"));
        std::fs::read(path).ok()
    }

    /// End-to-end: from a real Bear `.mc`, reproduce the first index sub-mesh
    /// byte-exact via the clean-room transport (no emulator, no oracle file).
    #[test]
    fn bear_first_subblock_indices_match_oracle() {
        let Some(bytes) = fixture("Animal_Bear.Bear.bfres.mc") else {
            eprintln!("skipping: Bear fixture absent");
            return;
        };
        let mc = crate::mc::read_mc(&bytes).unwrap();
        let section = crate::mc::read_mesh_section(&mc)
            .unwrap()
            .expect("mesh section");
        let stream = mc.compressed_stream();
        let payload = &stream[section.payload_offset..];

        // Framing scalars (validated against the decoder).
        assert_eq!(parse_super_block_trailer(payload).0, 0);
        let (t0, t1, pos) = parse_super_block_trailer(payload);
        assert_eq!((t0, t1), (0, 0));
        let mut fwd = ForwardReader::new(payload, pos);
        assert_eq!(fwd.varint(), 14, "w27 sub-block count");
        let hdr = parse_sub_block_header(&mut fwd).expect("header");
        assert_eq!(
            (hdr.count, hdr.a, hdr.b, hdr.c, hdr.d, hdr.e, hdr.f),
            (2, 1, 0, 1, 6606, 0, 6606)
        );

        // State-0 table builder cursor transition (the hard, validated piece).
        let sub_a = section.first_chunk.sub_a_size as usize;
        let tb = state0_table_builder(payload, fwd.pos, sub_a - 8, 0, 0, 7);
        assert_eq!(tb.fwd, 15, "forward cursor after table builder");
        assert_eq!(tb.rev_ptr, sub_a - 8 - 18, "reverse-A ptr (P+32807)");
        assert_eq!(tb.rev_bitpos, 50, "reverse-A bit position");
        assert_eq!((tb.w8, tb.symbols, tb.dir_bit), (3327, 8, 1));
        // Canonical-Huffman table values (golden, from the oracle/emulator).
        assert_eq!(
            tb.entries,
            [
                0x0c00100b, 0x0c000803, 0x0c000803, 0x10000a13, 0x1000100a, 0x1000100a, 0x10000803,
                0x10000801
            ]
        );
        assert_eq!(tb.offsets, [0, 6, 9, 39928, 39932, 39936, 39940, 39943]);
        assert_eq!(tb.cols, [0, 6, 9, 0, 4, 8, 12, 15]);
        assert_eq!(tb.longs, [131340, 458768]);
        assert_eq!(tb.byte_group_total, 93160);
        assert_eq!(tb.max_prod, 48);

        // Window decode (zstd code stream + raw data stream) + index decode of
        // the whole first sub-block (idx#1 6606 + pad + idx#2 1662 = 99.3% of
        // Bear's 16664-byte index buffer; the rest follows the vertex states).
        let bufa = decode_first_subblock_indices(&section, payload).unwrap();
        assert_eq!(bufa.len(), 16540);
        // Golden bytes from the mesh-codec-output oracle (Bear bufA).
        assert_eq!(
            &bufa[..16],
            &[0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7, 0]
        );
        // idx#1 tail, the align_a zero pad, then idx#2 head + tail.
        assert_eq!(
            &bufa[13200..13212],
            &[0x83, 0x08, 0x7f, 0x08, 0x82, 0x08, 0x83, 0x08, 0x82, 0x08, 0x80, 0x08]
        );
        assert_eq!(&bufa[13212..13216], &[0, 0, 0, 0], "align_a pad");
        assert_eq!(&bufa[13216..13224], &[0, 0, 1, 0, 2, 0, 3, 0]);
        assert_eq!(
            &bufa[16528..16540],
            &[0x76, 0x04, 0x78, 0x04, 0x7a, 0x04, 0x76, 0x04, 0x7a, 0x04, 0x79, 0x04]
        );
    }

    /// Contiguous spread for Bear's first rANS segment (M=64, log=6).
    ///
    /// Provenance: `spread_ref.py` / `vtxgt/rans/{step,sym}.bin` from `trace_rans.py`
    /// (Animal_Bear first `0x110e270` call). Freqs inferred from the spread map:
    /// `[5,1,1,0,1,0,1,1,0,1,3,6,13,23,8]` (symbols 3/5/8 unused). Rules out
    /// FSE-style scatter and off-by-one slot indexing within each symbol run.
    #[test]
    fn rans_spread_bear_first_rans_m64() {
        const FREQS: [u16; 15] = [5, 1, 1, 0, 1, 0, 1, 1, 0, 1, 3, 6, 13, 23, 8];
        const STEP: [u32; 64] = [
            327680, 327681, 327682, 327683, 327684, 65536, 65536, 65536, 65536, 65536, 65536,
            196608, 196609, 196610, 393216, 393217, 393218, 393219, 393220, 393221, 851968, 851969,
            851970, 851971, 851972, 851973, 851974, 851975, 851976, 851977, 851978, 851979, 851980,
            1507328, 1507329, 1507330, 1507331, 1507332, 1507333, 1507334, 1507335, 1507336,
            1507337, 1507338, 1507339, 1507340, 1507341, 1507342, 1507343, 1507344, 1507345,
            1507346, 1507347, 1507348, 1507349, 1507350, 524288, 524289, 524290, 524291, 524292,
            524293, 524294, 524295,
        ];
        const SYM: [u16; 64] = [
            0, 0, 0, 0, 0, 1, 2, 4, 6, 7, 9, 10, 10, 10, 11, 11, 11, 11, 11, 11, 12, 12, 12, 12,
            12, 12, 12, 12, 12, 12, 12, 12, 12, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
            13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 14, 14, 14, 14, 14, 14, 14, 14,
        ];
        let t = rans_spread(6, &FREQS);
        assert_eq!(t.step, STEP);
        assert_eq!(t.sym, SYM);
    }

    /// Warm main-loop slice of the four-state rANS init (`0x110dfa0`).
    ///
    /// Provenance: `capture_init6.py` / `verify_init_invariant.py` on `0x110dfa0`
    /// return at `0x110e1b8` (Animal_Bear, the segment at stream `P+8044`,
    /// `prod=228`, `log=5`, already-loaded states). The `(log, freqs)` are this
    /// segment's own freq-reader+spread output — NOT a fixed table: across the 3
    /// fixtures `log` ranges 3..11 and the freqs differ every segment (see
    /// `capture_init_all.py`). `[28,3,1]` is simply Bear's data here. Rules out
    /// treating init as decode-only renorm and using log=6 / the M=64 decode table.
    #[test]
    fn rans_init_states_bear_first_rans() {
        const INIT_FREQS: [u16; 3] = [28, 3, 1];
        const ST_IN: [u64; 4] = [0x15601103de, 0x7a4056e3de, 0x4939330469c, 0x1136b5c57093e];
        const ST_OUT: [u64; 4] = [
            0x1670c7fb0e5cc107,
            0x80581303,
            0x0e1e9623a87cf343,
            0x01321a08545304,
        ];
        let init_stream =
            hex_bytes("1c6c79053929c95b0ce6a98f0bb0c472ab757821cbb49d0d44d69beb2784028b");
        let t = rans_spread(5, &INIT_FREQS);
        let r = rans_init_states(&t, &init_stream, 228, 1, ST_IN).unwrap();
        assert_eq!(r.states, ST_OUT);
        assert_eq!(r.stream_used, 24);

        let mut bad_stream = init_stream.clone();
        bad_stream[0] ^= 1;
        assert_ne!(
            rans_init_states(&t, &bad_stream, 228, 1, ST_IN)
                .unwrap()
                .states,
            ST_OUT
        );
        assert!(matches!(
            rans_init_states(&t, &init_stream[..8], 228, 1, ST_IN),
            Err(RansInitError::StreamTooShort)
        ));
        assert!(matches!(
            rans_init_states(&t, &init_stream, 3, 1, ST_IN),
            Err(RansInitError::ProdTooSmall)
        ));
        assert!(matches!(
            rans_init_states(&t, &init_stream, 229, 1, ST_IN),
            Err(RansInitError::UnsupportedProdTail)
        ));
    }

    /// Cold-start + continuation for the generic four-state init (`0x110dfa0`).
    ///
    /// Provenance: `capture_init_all.py` + replay prototype on Animal_Bear
    /// stream `P+1312`, calls 0 and 1. Call 0 has `flag=0`, so the branch at
    /// `0x110dfc0` takes `0x110e1bc`, loads four seed states from the stream,
    /// sets `flag |= 0xf`, and then runs the main loop. Call 1 reuses the same
    /// stream descriptor, entering with `[x2+12] == 135` and writing back 187.
    /// This rules out both the warm-only shortcut and resetting the forward
    /// cursor to zero on continuation.
    #[test]
    fn rans_init_states_cold_start_and_shared_cursor_bear() {
        const COLD_FREQS: [u16; 5] = [95, 408, 7, 1, 1];
        const CONT_FREQS: [u16; 2] = [3, 29];
        const AFTER_COLD: [u64; 4] = [
            0x1bb7813ea643,
            0x0e82d56be41e2018,
            0x074b790c100e3297,
            0x6e08d7b8e2e,
        ];
        const AFTER_CONT: [u64; 4] = [
            0x4c075b8b626,
            0x027cdfde572a0f44,
            0x211ec0c84c35ac,
            0x1f2795027,
        ];
        let stream = hex_bytes(
            "8456b469510786ef7a6cd10c5407d3936e8849a3d90517c4fdb100186c6808c6\
             efdb29f2eb95f3c58808a258f1e4fb4cae50cdf6e3fc8a8e058f6afb2b90c85c\
             1fb1c369c65d11d916ee3b455c7e514e12136f4802a124282e1525441af02b4e\
             96c03a1724a554aa4ec34ff2b85f9845879b2612b4052877d15b42436042d3a\
             3eb34cf9faed765e95ae1ad8053d4b3883b8ab07455a19b5dd00fcbf44ce28ce\
             4011185d7efb208e226b25d215d61630ba4da975ec185be977e44d63162358fbd",
        );

        let cold_table = rans_spread(9, &COLD_FREQS);
        let cont_table = rans_spread(5, &CONT_FREQS);
        let mut state = RansStateBuffer::cold();
        let mut cursor = RansStreamCursor::default();

        let cold =
            rans_init_states_with_cursor(&cold_table, &stream, 1024, 1, &mut state, &mut cursor)
                .unwrap();
        assert_eq!(cold.states, AFTER_COLD);
        assert_eq!(cold.flag, 0xf);
        assert_eq!((cold.stream_used, cold.stream_offset), (135, 135));
        assert_eq!(state.flag, 0xf);
        assert_eq!(cursor.offset, 135);

        let cont =
            rans_init_states_with_cursor(&cont_table, &stream, 1024, 1, &mut state, &mut cursor)
                .unwrap();
        assert_eq!(cont.states, AFTER_CONT);
        assert_eq!((cont.stream_used, cont.stream_offset), (52, 187));
        assert_eq!(cursor.offset, 187);

        let warm_only = rans_init_states(&cold_table, &stream, 1024, 1, [0; 4])
            .unwrap()
            .states;
        assert_ne!(warm_only, AFTER_COLD);

        let mut reset_state = RansStateBuffer::warm(AFTER_COLD);
        let mut reset_cursor = RansStreamCursor::default();
        let reset = rans_init_states_with_cursor(
            &cont_table,
            &stream,
            1024,
            1,
            &mut reset_state,
            &mut reset_cursor,
        )
        .unwrap()
        .states;
        assert_ne!(reset, AFTER_CONT);
    }

    /// Second-model cold-start coverage for the generic init (`0x110dfa0`).
    ///
    /// Provenance: `capture_init_all.py` + `verify_init_invariant.py` over all
    /// fixtures, with the minimal inline bytes dumped by
    /// `capture_init_bass_golden.py` to `local-assets/re/init_bass_p394_golden.json`.
    /// Animal_Bass call 0 enters at stream `P+394` with `flag=0`, `log=7`,
    /// `prod=568`, and freqs `[6,118,3,1]`. This cross-model cold call rules out
    /// the warm-only implementation independently of the Bear continuation test.
    #[test]
    fn rans_init_states_cold_start_bass_p394() {
        const COLD_FREQS: [u16; 4] = [6, 118, 3, 1];
        const AFTER_COLD: [u64; 4] = [0x3901a31f71085, 0x4552634d0a, 0x2e32d8bbfce, 0x186072316a8];
        const WARM_ONLY_ZERO_STATES: [u64; 4] = [
            0x6911728ba,
            0x6d5b56bbf6335,
            0x8f75efe56b1a856,
            0x1ea79cf8bde9ddb,
        ];
        let stream = hex_bytes(
            "06b1d9f57282a6854e36f8f582058fa2643c3f96dbfe7fed52044497928f64d\
             6e86b8418cf7502f4d95c243c19f0e372b0e6680ab7e79cdd9e7f8a31da5742a7",
        );
        let table = rans_spread(7, &COLD_FREQS);
        let mut state = RansStateBuffer::cold();
        let mut cursor = RansStreamCursor::default();

        let cold =
            rans_init_states_with_cursor(&table, &stream, 568, 1, &mut state, &mut cursor).unwrap();
        assert_eq!(cold.states, AFTER_COLD);
        assert_eq!(cold.flag, 0xf);
        assert_eq!((cold.stream_used, cold.stream_offset), (58, 58));
        assert_eq!(state.flag, 0xf);
        assert_eq!(cursor.offset, 58);

        let warm_only = rans_init_states(&table, &stream, 568, 1, [0; 4])
            .unwrap()
            .states;
        assert_eq!(warm_only, WARM_ONLY_ZERO_STATES);
        assert_ne!(warm_only, AFTER_COLD);
    }

    #[test]
    fn rans_init_states_rejects_truncated_cold_loader() {
        let table = rans_spread(9, &[95, 408, 7, 1, 1]);
        let mut state = RansStateBuffer::cold();
        let mut cursor = RansStreamCursor::default();
        assert!(matches!(
            rans_init_states_with_cursor(&table, &[0x84, 0x56], 1024, 1, &mut state, &mut cursor),
            Err(RansInitError::StreamTooShort)
        ));
    }

    /// Spread → init (`0x110dfa0`) → decode: Bear first rANS without hardcoded states.
    ///
    /// Provenance: `trace_rans.py` decode oracle; init from `capture_init6.py`.
    #[test]
    fn rans_spread_init_then_decode_bear_first_rans() {
        const DEC_FREQS: [u16; 15] = [5, 1, 1, 0, 1, 0, 1, 1, 0, 1, 3, 6, 13, 23, 8];
        const INIT_FREQS: [u16; 3] = [28, 3, 1];
        const ST_IN: [u64; 4] = [0x15601103de, 0x7a4056e3de, 0x4939330469c, 0x1136b5c57093e];
        let init_stream =
            hex_bytes("1c6c79053929c95b0ce6a98f0bb0c472ab757821cbb49d0d44d69beb2784028b");
        let decode_stream = hex_bytes(
            "44d69beb2784028b6a39382a036f90a250ebc749203fa34e0d60353e5071548d51aa7a26\
             943ad95a422eea145dab83d860ba542ed7bf85ec1c78e11fedddfb9ceaf8b9031988e12f",
        );
        let init_tbl = rans_spread(5, &INIT_FREQS);
        let states = rans_init_states(&init_tbl, &init_stream, 228, 1, ST_IN)
            .unwrap()
            .states;
        let dec_tbl = rans_spread(6, &DEC_FREQS);
        let out = rans_decode(RansDecodeSpec {
            count: 228,
            log: 6,
            stride: 1,
            step: &dec_tbl.step,
            sym: &dec_tbl.sym,
            init_states: states,
            stream: &decode_stream,
        })
        .unwrap();
        assert_eq!(out.len(), 228);
        assert_eq!(
            &out[..24],
            &[4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 10, 4, 2, 12, 13, 12, 10, 13, 10]
        );
        assert_eq!(&out[220..], &[14, 13, 13, 14, 14, 13, 14, 13]);
    }

    /// The rANS decoder reproduces a real decoded symbol stream (the first
    /// vertex-coder rANS call of Animal_Bear), validated against the emulator.
    #[test]
    fn rans_decode_matches_oracle() {
        // Decode table (step[64] = (freq<<16)|low, sym[64] spread map), log2(M)=6.
        const STEP: [u32; 64] = [
            327680, 327681, 327682, 327683, 327684, 65536, 65536, 65536, 65536, 65536, 65536,
            196608, 196609, 196610, 393216, 393217, 393218, 393219, 393220, 393221, 851968, 851969,
            851970, 851971, 851972, 851973, 851974, 851975, 851976, 851977, 851978, 851979, 851980,
            1507328, 1507329, 1507330, 1507331, 1507332, 1507333, 1507334, 1507335, 1507336,
            1507337, 1507338, 1507339, 1507340, 1507341, 1507342, 1507343, 1507344, 1507345,
            1507346, 1507347, 1507348, 1507349, 1507350, 524288, 524289, 524290, 524291, 524292,
            524293, 524294, 524295,
        ];
        const SYM: [u16; 64] = [
            0, 0, 0, 0, 0, 1, 2, 4, 6, 7, 9, 10, 10, 10, 11, 11, 11, 11, 11, 11, 12, 12, 12, 12,
            12, 12, 12, 12, 12, 12, 12, 12, 12, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
            13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 14, 14, 14, 14, 14, 14, 14, 14,
        ];
        let states = [
            0x1670c7fb0e5cc107u64,
            0x80581303,
            0x0e1e9623a87cf343,
            0x01321a08545304,
        ];
        let stream = hex_bytes(
            "44d69beb2784028b6a39382a036f90a250ebc749203fa34e0d60353e5071548d51aa7a26\
             943ad95a422eea145dab83d860ba542ed7bf85ec1c78e11fedddfb9ceaf8b9031988e12f",
        );
        let out = rans_decode(RansDecodeSpec {
            count: 228,
            log: 6,
            stride: 1,
            step: &STEP,
            sym: &SYM,
            init_states: states,
            stream: &stream,
        })
        .unwrap();
        assert_eq!(out.len(), 228);
        assert_eq!(
            &out[..24],
            &[4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 10, 4, 2, 12, 13, 12, 10, 13, 10]
        );
        assert_eq!(&out[220..], &[14, 13, 13, 14, 14, 13, 14, 13]);
        assert_eq!(out.iter().map(|&s| s as u32).sum::<u32>(), 2565);
    }

    /// Discriminating tail decode: `count % 4 != 0`, so the leftover symbols
    /// exercise the tail loop (`0x110e410`). The tail must continue lanes 0,1,…
    /// (tail symbol `k` from `states[k]`), NOT decode every leftover from
    /// `states[0]`.
    ///
    /// Provenance: `capture_decode_tail_golden.py`, Animal_Bear's 4th `0x110e270`
    /// call (`count=142`, `tail=2`, `log=6`, `stride=1`). With the wrong
    /// `states[0]`-only tail the last symbol decodes to `12`; the emulator (and
    /// the `states[k]` rule, confirmed by the `str x17,[x0],#8` post-increment)
    /// gives `14`. Also cross-checked against Animal_Bass call #1 (`count=194`,
    /// `tail=2`) in `audit_decode_spread.py`.
    #[test]
    fn rans_decode_tail_continues_lanes() {
        const FREQS: [u16; 15] = [1, 0, 0, 0, 1, 1, 1, 1, 1, 1, 2, 3, 13, 21, 18];
        let states = [0x1bdb9fbf46u64, 0xbccd27a202, 0x424e240141, 0x13610be603];
        let stream = hex_bytes(
            "c9f18c17d9d5062f2be62f960821c609377ce810a26db5967caa56af741a01f3\
             e7238315b6ebcb1ce861fedaff21bcbd",
        );
        let t = rans_spread(6, &FREQS);
        let out = rans_decode(RansDecodeSpec {
            count: 142,
            log: 6,
            stride: 1,
            step: &t.step,
            sym: &t.sym,
            init_states: states,
            stream: &stream,
        })
        .unwrap();
        assert_eq!(out.len(), 142);
        assert_eq!(&out[..8], &[9, 5, 4, 6, 11, 10, 8, 13]);
        // The two-symbol tail: lanes 0 and 1 continue -> [14, 14]. The
        // `states[0]`-only shortcut would yield [14, 12] here.
        assert_eq!(&out[138..], &[12, 13, 14, 14]);
    }

    /// Stride-3 output layout for `0x110e270`.
    ///
    /// Provenance: `capture_decode_stride3.py` on Animal_Bass call 2
    /// (`prod=960`, decoded `w2=320`, `stride=3`, `log=5`). The wrapper at
    /// `0x110de14..0x110de48` stores the product at `x1+8` but passes `w2` at
    /// `x1+0xc` to `0x110e270`; the decode loop stores symbol `i` at
    /// `out[i*stride]` (`strh w22,[x11]`, then `add x11,x11,x20` where
    /// `x20 = 4*stride*2`). Sibling lanes are not touched by this call. This
    /// rules out both the old count-sized buffer and a dense stride-1 writer.
    #[test]
    fn rans_decode_stride3_writes_lane_slots() {
        const STEP: [u32; 32] = [
            1966080, 1966081, 1966082, 1966083, 1966084, 1966085, 1966086, 1966087, 1966088,
            1966089, 1966090, 1966091, 1966092, 1966093, 1966094, 1966095, 1966096, 1966097,
            1966098, 1966099, 1966100, 1966101, 1966102, 1966103, 1966104, 1966105, 1966106,
            1966107, 1966108, 1966109, 65536, 65536,
        ];
        const SYM: [u16; 32] = [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 2, 1022,
        ];
        let states = [0x68007d0ef80f, 0x674a999ea5a, 0x647f7484a3f513e, 0xd7a40fe0];
        let expected_lane = hex_u16s(
            "00000000020000000000000000000000000000000000000000000000000000000000000000000000\
             0000000000000000000000000000000000000000fe030000fe030000fe0300000000000000000000\
             00000000fe03fe030000000000000000020002000200000000000200000000000200000002000000\
             02000000020000000000000000000000000000000000000000000000000000000000000000000000\
             00000000000000000000000000000000000000000000000000000000000000000000000000000000\
             00000000000000000000000000000000000000000000000000000000000000000000000000000000\
             00000000000000000000000000000000000000000000000000000000000000000000000000000000\
             00000000000000000000000000000000000000000000000000000000000000000000000000000000\
             00000000000000000000000000000000000000000000000000000000000000000000000000000000\
             00000000000000000000000000000000000000000000000000000000000000000000000000000000\
             00000000020000000000000000000000000000000000000000000000000000000000000000000000\
             00000000000000000000000000000000000000000000000000000000000000000000000000000000\
             00000000000000000000000000000000000000000000000000000000000000000000000000000000\
             00000000000000000000000000000000000000000000000000000000000000000000000000000000\
             fe0300000000000000000000000000000000000000000000fe030000000000000200000000000000\
             02000000020000000000000000000000000000000000000000000000000000000000000000000000",
        );
        assert_eq!(expected_lane.len(), 320);
        let stream = hex_bytes("87e2163f7eff1a365ef3a7f6a5e841a3");
        let spec = RansDecodeSpec {
            count: 320,
            log: 5,
            stride: 3,
            step: &STEP,
            sym: &SYM,
            init_states: states,
            stream: &stream,
        };

        let mut out = vec![0xbeefu16; 960];
        let used = rans_decode_into(&mut out, spec).unwrap();
        assert_eq!(used, 16);
        for (i, &expected) in expected_lane.iter().enumerate() {
            assert_eq!(out[i * 3], expected, "lane symbol {i}");
        }
        assert!(out
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 3 != 0)
            .all(|(_, &v)| v == 0xbeef));

        let fresh = rans_decode(spec).unwrap();
        assert_eq!(fresh.len(), 960);
        for (i, &expected) in expected_lane.iter().enumerate() {
            assert_eq!(fresh[i * 3], expected, "fresh lane symbol {i}");
        }
        assert!(fresh
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 3 != 0)
            .all(|(_, &v)| v == 0));

        let mut too_small = vec![0u16; 320];
        assert_eq!(
            rans_decode_into(&mut too_small, spec),
            Err(RansDecodeError::OutputTooSmall)
        );
    }

    /// RLE fill helper used by the segment dispatch (`0x110f930`).
    ///
    /// Provenance: `capture_rle_fill.py` over Bear/Bass/Dragonfly. Observed
    /// calls are Bass `value=0,count=2,stride=3`, Bass `value=0,count=322,stride=3`
    /// twice, and Dragonfly `value=11,count=3,stride=1`. The strided Bass case
    /// rules out a dense fill that would overwrite sibling lanes.
    #[test]
    fn rans_rle_fill_matches_observed_stride_and_dense_cases() {
        let mut bass = vec![0xbeefu16; 322 * 3];
        rans_rle_fill(&mut bass, 0, 322, 3).unwrap();
        for i in 0..322 {
            assert_eq!(bass[i * 3], 0);
        }
        assert!(bass
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 3 != 0)
            .all(|(_, &v)| v == 0xbeef));

        let mut dragonfly = vec![0u16; 3];
        rans_rle_fill(&mut dragonfly, 11, 3, 1).unwrap();
        assert_eq!(dragonfly, [11, 11, 11]);

        let mut too_small = vec![0u16; 322];
        assert_eq!(
            rans_rle_fill(&mut too_small, 0, 322, 3),
            Err(RansRleFillError::OutputTooSmall)
        );
        assert_eq!(
            rans_rle_fill(&mut bass, 0, 1, 0),
            Err(RansRleFillError::ZeroStride)
        );
    }

    /// Short segment-header form for mode 0 (`0x110de80`).
    ///
    /// Provenance: `capture_segment_dispatch.py`, Animal_Bear table-build 1.
    /// The top bit is clear and the 4-bit class is nonzero, so
    /// `0x110deb4..0x110df54` yields mode 0, log 9, and table count 5.
    #[test]
    fn rans_segment_header_short_mode0() {
        let payload = hex_bytes("1117eda742422e81");
        let header = rans_read_segment_header(
            &payload,
            RansFreqReader {
                ptr: 0,
                acc: 0x227f1b04e40cc5e4,
                bitpos: 61,
            },
        )
        .unwrap();
        assert_eq!(
            header,
            RansSegmentHeader {
                mode: 0,
                log: 9,
                table_count: Some(5),
                value: 0,
                reader: RansFreqReader {
                    ptr: 0,
                    acc: 0xfc6c139033179000,
                    bitpos: 51,
                },
            }
        );
    }

    /// Long segment-header form for mode 1 (`0x110de80`).
    ///
    /// Provenance: `capture_segment_dispatch.py`, Animal_Bear table-build 0.
    /// This covers the long path at `0x110dec4..0x110df60`; the `csel ... eq`
    /// polarity after `tst` is the discriminating rule.
    #[test]
    fn rans_segment_header_long_mode1() {
        let payload = hex_bytes("5555f9abcff355b5");
        let header = rans_read_segment_header(
            &payload,
            RansFreqReader {
                ptr: 0,
                acc: 0x0c736b6abdf6deec,
                bitpos: 58,
            },
        )
        .unwrap();
        assert_eq!(
            header,
            RansSegmentHeader {
                mode: 1,
                log: 1,
                table_count: Some(2),
                value: 0,
                reader: RansFreqReader {
                    ptr: 0,
                    acc: 0xcdadaaf7db7bb400,
                    bitpos: 48,
                },
            }
        );
    }

    /// Long segment-header form selecting the mid-width count.
    ///
    /// Provenance: `capture_segment_dispatch.py`, Animal_Bear table-build 6.
    /// This covers the `high_bits`/`wide_bits` branch where the second `tst`
    /// finds a clear top bit and selects the 12-bit form.
    #[test]
    fn rans_segment_header_long_mid_width() {
        let payload = hex_bytes("00a03c61c56d6014");
        let header = rans_read_segment_header(
            &payload,
            RansFreqReader {
                ptr: 0,
                acc: 0xbf044239f0e66c14,
                bitpos: 56,
            },
        )
        .unwrap();
        assert_eq!(
            header,
            RansSegmentHeader {
                mode: 0,
                log: 8,
                table_count: Some(248),
                value: 0,
                reader: RansFreqReader {
                    ptr: 0,
                    acc: 0x8473e1ccd8280000,
                    bitpos: 39,
                },
            }
        );
    }

    /// Long segment-header form selecting the widest observed count.
    ///
    /// Provenance: `capture_segment_dispatch.py`, Animal_Bear table-build 26.
    /// This covers the `high_bits`/`wider_bits` branch where the long path
    /// consumes 19 header bits before reading mode/log.
    #[test]
    fn rans_segment_header_long_wide_width() {
        let payload = hex_bytes("a814d2b6402520a7");
        let header = rans_read_segment_header(
            &payload,
            RansFreqReader {
                ptr: 0,
                acc: 0xbf401a00400000f4,
                bitpos: 59,
            },
        )
        .unwrap();
        assert_eq!(
            header,
            RansSegmentHeader {
                mode: 1,
                log: 10,
                table_count: Some(760),
                value: 0,
                reader: RansFreqReader {
                    ptr: 0,
                    acc: 0x00400000f4000000,
                    bitpos: 35,
                },
            }
        );
    }

    /// RLE segment-header value form for mode 2 (`0x110de80`).
    ///
    /// Provenance: `capture_segment_dispatch.py`, Animal_Dragonfly table-build
    /// 18. The short class is zero, so `0x110df64..0x110df98` decodes the value
    /// varint and returns without a table build.
    #[test]
    fn rans_segment_header_rle_value127() {
        let payload = hex_bytes("f1a106940000623a");
        let header = rans_read_segment_header(
            &payload,
            RansFreqReader {
                ptr: 0,
                acc: 0x03fbfd0221c04704,
                bitpos: 59,
            },
        )
        .unwrap();
        assert_eq!(
            header,
            RansSegmentHeader {
                mode: 2,
                log: 0,
                table_count: None,
                value: 127,
                reader: RansFreqReader {
                    ptr: 0,
                    acc: 0x7fa0443808e0e000,
                    bitpos: 46,
                },
            }
        );
        assert_eq!(
            rans_read_segment_header(
                &payload[..7],
                RansFreqReader {
                    ptr: 0,
                    acc: 0x03fbfd0221c04704,
                    bitpos: 59,
                },
            ),
            Err(RansSegmentHeaderError::PayloadTooSmall)
        );
    }

    /// Mode-0 table builder (`0x110e540`) small-symbol branch.
    ///
    /// Provenance: `capture_segment_dispatch.py`, Animal_Bear table-build 2,
    /// replayed by `verify_mode0_table_builder.py`. This covers the
    /// `count <= 10` symbol-list loop at `0x110e578..0x110e60c` and the
    /// `w4=15` frequency-reader call.
    #[test]
    fn rans_mode0_table_builder_small_branch() {
        const STEP: [u32; 32] = [
            196608, 196609, 196610, 1900544, 1900545, 1900546, 1900547, 1900548, 1900549, 1900550,
            1900551, 1900552, 1900553, 1900554, 1900555, 1900556, 1900557, 1900558, 1900559,
            1900560, 1900561, 1900562, 1900563, 1900564, 1900565, 1900566, 1900567, 1900568,
            1900569, 1900570, 1900571, 1900572,
        ];
        const SYM: [u16; 32] = [
            0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
            1, 1, 1,
        ];
        let payload = hex_bytes("cb8b88ff4f53860f38");
        let built = rans_build_mode0_table(
            &payload,
            RansFreqReader {
                ptr: 1,
                acc: 0xc84854fda2e22400,
                bitpos: 51,
            },
            2,
            5,
        )
        .unwrap();
        assert_eq!(built.symbols, [0, 1]);
        assert_eq!(built.freqs, [3, 29]);
        assert_eq!(built.table.step, STEP);
        assert_eq!(built.table.sym, SYM);
        assert_eq!(
            built.reader,
            RansFreqReader {
                ptr: 0,
                acc: 0x2153f68b889c0700,
                bitpos: 49,
            }
        );

        assert_eq!(
            rans_build_mode0_table(
                &payload[..8],
                RansFreqReader {
                    ptr: 1,
                    acc: 0xc84854fda2e22400,
                    bitpos: 51,
                },
                2,
                5,
            ),
            Err(RansMode0TableBuildError::PayloadTooSmall)
        );
    }

    /// Mode-0 table builder (`0x110e540`) large-symbol branch.
    ///
    /// Provenance: `capture_segment_dispatch.py`, Animal_Bear table-build 25,
    /// replayed by `verify_mode0_table_builder.py`. This covers the
    /// `count > 10` call to `0x110e9a0`, the `w4=14` frequency-reader call, and
    /// the contiguous sparse spread into the descriptor's step/symbol tables.
    #[test]
    fn rans_mode0_table_builder_large_branch() {
        const STEP: [u32; 64] = [
            327680, 327681, 327682, 327683, 327684, 65536, 65536, 65536, 65536, 65536, 65536,
            196608, 196609, 196610, 393216, 393217, 393218, 393219, 393220, 393221, 851968, 851969,
            851970, 851971, 851972, 851973, 851974, 851975, 851976, 851977, 851978, 851979, 851980,
            1507328, 1507329, 1507330, 1507331, 1507332, 1507333, 1507334, 1507335, 1507336,
            1507337, 1507338, 1507339, 1507340, 1507341, 1507342, 1507343, 1507344, 1507345,
            1507346, 1507347, 1507348, 1507349, 1507350, 524288, 524289, 524290, 524291, 524292,
            524293, 524294, 524295,
        ];
        const SYM: [u16; 64] = [
            0, 0, 0, 0, 0, 1, 2, 4, 6, 7, 9, 10, 10, 10, 11, 11, 11, 11, 11, 11, 12, 12, 12, 12,
            12, 12, 12, 12, 12, 12, 12, 12, 12, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
            13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 14, 14, 14, 14, 14, 14, 14, 14,
        ];
        let payload = hex_bytes("d2b6402520a707000002d000faed3d27");
        let built = rans_build_mode0_table(
            &payload,
            RansFreqReader {
                ptr: 8,
                acc: 0xbaf109e148a24c00,
                bitpos: 47,
            },
            12,
            6,
        )
        .unwrap();
        assert_eq!(built.symbols, [0, 1, 2, 4, 6, 7, 9, 10, 11, 12, 13, 14]);
        assert_eq!(built.freqs, [5, 1, 1, 1, 1, 1, 1, 3, 6, 13, 23, 8]);
        assert_eq!(built.table.step, STEP);
        assert_eq!(built.table.sym, SYM);
        assert_eq!(
            built.reader,
            RansFreqReader {
                ptr: 0,
                acc: 0xf7b7e80340080000,
                bitpos: 54,
            }
        );

        assert_eq!(
            rans_build_mode0_table(&payload, built.reader, 0, 6),
            Err(RansMode0TableBuildError::TableCountZero)
        );
        assert_eq!(
            rans_build_mode0_table(&payload, built.reader, 12, 12),
            Err(RansMode0TableBuildError::UnsupportedLog(12))
        );
        assert_eq!(
            rans_build_mode0_table(&payload, built.reader, 65, 6),
            Err(RansMode0TableBuildError::TableCountExceedsMass {
                count: 65,
                mass: 64
            })
        );
    }

    /// Mode-1 table builder (`0x110f3c0`) special `log < 2` path.
    ///
    /// Provenance: `capture_segment_dispatch.py`, Animal_Bear table-build 0,
    /// replayed by `verify_mode1_table_builder.py`. This covers
    /// `0x110f3d8` branching to `0x110f498`, then the final single-bit table
    /// expansion consumed by `0x110ef70`.
    #[test]
    fn rans_mode1_table_builder_log1() {
        let payload = hex_bytes("55c55555f9abcff355b5");
        let built = rans_build_mode1_table(
            &payload,
            RansFreqReader {
                ptr: 2,
                acc: 0xcdadaaf7db7bb400,
                bitpos: 48,
            },
            2,
            1,
        )
        .unwrap();
        assert_eq!(built.table, [65536, 65537]);
        assert_eq!(
            built.reader,
            RansFreqReader {
                ptr: 0,
                acc: 0x36b6abdf6deed556,
                bitpos: 62,
            }
        );
        assert_eq!(
            rans_build_mode1_table(
                &payload[..9],
                RansFreqReader {
                    ptr: 2,
                    acc: 0xcdadaaf7db7bb400,
                    bitpos: 48,
                },
                2,
                1,
            ),
            Err(RansMode1TableBuildError::PayloadTooSmall)
        );
    }

    /// Mode-1 table builder (`0x110f3c0`) general prefix-table expansion.
    ///
    /// Provenance: `capture_segment_dispatch.py`, Animal_Bass table-build 7,
    /// replayed by `verify_mode1_table_builder.py`. This covers the
    /// length-count reader, grouped sparse-symbol reader, and replicated table
    /// expansion at `0x110f558..0x110f718`.
    #[test]
    fn rans_mode1_table_builder_log4() {
        const TABLE: [u32; 16] = [
            65547, 65547, 65547, 65547, 65547, 65547, 65547, 65547, 262144, 262145, 262148, 262149,
            262151, 262152, 262153, 262154,
        ];
        let payload = hex_bytes("cd090074f80f006a8a3fe021");
        let built = rans_build_mode1_table(
            &payload,
            RansFreqReader {
                ptr: 4,
                acc: 0x806575ea0e731000,
                bitpos: 53,
            },
            9,
            4,
        )
        .unwrap();
        assert_eq!(built.table, TABLE);
        assert_eq!(
            built.reader,
            RansFreqReader {
                ptr: 0,
                acc: 0xa839cc443c07f14c,
                bitpos: 59,
            }
        );
        assert_eq!(
            rans_build_mode1_table(&payload, built.reader, 0, 4),
            Err(RansMode1TableBuildError::TableCountZero)
        );
        assert_eq!(
            rans_build_mode1_table(&payload, built.reader, 9, 0),
            Err(RansMode1TableBuildError::UnsupportedLog(0))
        );
        assert_eq!(
            rans_build_mode1_table(&payload, built.reader, 17, 4),
            Err(RansMode1TableBuildError::TableCountExceedsMass {
                count: 17,
                mass: 16
            })
        );
    }

    /// Segment descriptor builder (`0x110de80`) feeding mode-0 dispatch (`0x110de00`).
    ///
    /// Provenance: `capture_segment_dispatch.py`, Animal_Bear table-build 25 +
    /// dispatch 3. The descriptor is now built from the reverse stream
    /// (`count=12,log=6`) and then passed to the mode-0 dispatch wrapper. This
    /// proves the header/table builder output satisfies the existing rANS
    /// dispatch contract.
    #[test]
    fn rans_segment_dispatch_mode0_updates_output_cursor_and_states() {
        const FINAL_STATES: [u64; 4] = [0x3e69fd3c25, 0x1eb070dcc1aa, 0x7af49bc9b0fd, 0x14d2c33bcc];
        let descriptor_payload = hex_bytes("d2b6402520a707000002d000faed3d27");
        let descriptor = rans_build_segment_descriptor(
            &descriptor_payload,
            RansFreqReader {
                ptr: 8,
                acc: 0x59aebc4278522890,
                bitpos: 57,
            },
        )
        .unwrap();
        assert_eq!(descriptor.mode, 0);
        assert_eq!(descriptor.log, 6);
        assert_eq!(
            descriptor.reader,
            RansFreqReader {
                ptr: 0,
                acc: 0xf7b7e80340080000,
                bitpos: 54,
            }
        );
        let mut states = [
            0x1670c7fb0e5cc107u64,
            0x80581303,
            0x0e1e9623a87cf343,
            0x01321a08545304,
        ];
        let stream = hex_bytes(
            "44d69beb2784028b6a39382a036f90a250ebc749203fa34e0d60353e5071548d51aa7a26\
             943ad95a422eea145dab83d860ba542ed7bf85ec1c78e11fedddfb9ceaf8b9031988e12f",
        );
        let mut out = vec![0xbeefu16; 228];

        let used = rans_segment_dispatch_into(
            &mut out,
            RansSegmentDispatchSpec {
                mode: descriptor.mode,
                log: descriptor.log,
                value: descriptor.value,
                count: 228,
                stride: 1,
                states: &mut states,
                step: &descriptor.step,
                sym: &descriptor.sym,
                stream: &stream,
                payload: &[],
                three_lane_readers: None,
            },
        )
        .unwrap();

        assert_eq!(used, 68);
        assert_eq!(states, FINAL_STATES);
        assert_eq!(
            &out[..24],
            &[4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 10, 4, 2, 12, 13, 12, 10, 13, 10]
        );
        assert_eq!(&out[220..], &[14, 13, 13, 14, 14, 13, 14, 13]);
        assert_eq!(out.iter().map(|&s| s as u32).sum::<u32>(), 2565);
    }

    /// Segment descriptor builder (`0x110de80`) feeding mode-2 RLE dispatch.
    ///
    /// Provenance: `capture_segment_dispatch.py`, Animal_Dragonfly table-build
    /// 14 + dispatch 5: `mode=2,value=11,count=3,stride=1`.
    #[test]
    fn rans_segment_dispatch_mode2_rle_fills_dense_segment() {
        let descriptor_payload = hex_bytes("d244781d50f180ec");
        let descriptor = rans_build_segment_descriptor(
            &descriptor_payload,
            RansFreqReader {
                ptr: 0,
                acc: 0x005f479f43193ebc,
                bitpos: 59,
            },
        )
        .unwrap();
        assert_eq!(descriptor.mode, 2);
        assert_eq!(descriptor.value, 11);
        assert_eq!(
            descriptor.reader,
            RansFreqReader {
                ptr: 0,
                acc: 0xe8f3e86327d7a000,
                bitpos: 46,
            }
        );
        let mut states = [0u64; 4];
        let mut out = [6u16, 6, 120];
        let used = rans_segment_dispatch_into(
            &mut out,
            RansSegmentDispatchSpec {
                mode: descriptor.mode,
                log: descriptor.log,
                value: descriptor.value,
                count: 3,
                stride: 1,
                states: &mut states,
                step: &descriptor.step,
                sym: &descriptor.sym,
                stream: &[],
                payload: &[],
                three_lane_readers: None,
            },
        )
        .unwrap();
        assert_eq!(used, 0);
        assert_eq!(out, [11, 11, 11]);
    }

    #[test]
    fn rans_segment_dispatch_mode1_requires_reader_state() {
        let mut states = [0u64; 4];
        let mut out = [0u16; 1];
        assert_eq!(
            rans_segment_dispatch_into(
                &mut out,
                RansSegmentDispatchSpec {
                    mode: 1,
                    log: 4,
                    value: 0,
                    count: 1,
                    stride: 1,
                    states: &mut states,
                    step: &[],
                    sym: &[],
                    stream: &[],
                    payload: &[],
                    three_lane_readers: None,
                },
            ),
            Err(RansSegmentDispatchError::MissingThreeLaneReaders)
        );
    }

    /// Three-lane mode-1 decoder (`0x110ef70`) main loop (`count >= 12`).
    ///
    /// Provenance: `capture_segment_dispatch.py`, Animal_Bass dispatch 13
    /// (`mode=1,log=3,count=12,stride=1`). This covers the group-of-12 path at
    /// `0x110f030..0x110f1b4`, including reader 1's `rev` load and forward
    /// pointer movement. It rules out treating all three readers as the same
    /// backwards little-endian reader.
    #[test]
    fn rans_three_lane_decode_bass_count12_main_loop() {
        const TABLE: [u32; 8] = [
            0x0002_0000,
            0x0002_0000,
            0x0003_0001,
            0x0003_0004,
            0x0003_0007,
            0x0003_0009,
            0x0003_000a,
            0x0003_000b,
        ];
        let payload = sparse_payload(
            834,
            &[
                (1, "c1927cb097255b04"),
                (826, "c1d38107e0871e88"),
                (814, "80a21d403e668f86"),
            ],
        );
        let mut readers = [
            RansThreeLaneReader {
                ptr: 1,
                acc: 0x84b8f4310101be08,
                bitpos: 55,
            },
            RansThreeLaneReader {
                ptr: 826,
                acc: 0x668f867594cc9ac0,
                bitpos: 56,
            },
            RansThreeLaneReader {
                ptr: 814,
                acc: 0x0e9e0cd664a3ac00,
                bitpos: 53,
            },
        ];
        let mut out = [0u16, 2, 6464, 0, 8, 2, 6400, 0, 0, 3, 6384, 0];
        rans_three_lane_decode_into(
            &mut out,
            RansThreeLaneDecodeSpec {
                count: 12,
                log: 3,
                stride: 1,
                table: &TABLE,
                readers: &mut readers,
                payload: &payload,
            },
        )
        .unwrap();

        assert_eq!(out, [7, 4, 0, 0, 0, 0, 7, 10, 11, 9, 7, 1]);
        assert_eq!(
            readers,
            [
                RansThreeLaneReader {
                    ptr: 0,
                    acc: 0xc7a188080df04000,
                    bitpos: 52,
                },
                RansThreeLaneReader {
                    ptr: 826,
                    acc: 0x7c33aca664d60800,
                    bitpos: 45,
                },
                RansThreeLaneReader {
                    ptr: 813,
                    acc: 0x783359928eb0d000,
                    bitpos: 51,
                },
            ]
        );
    }

    /// Segment descriptor builder feeding mode-1 tail dispatch (`count < 12`).
    ///
    /// Provenance: `capture_segment_dispatch.py`, Animal_Dragonfly table-build
    /// 15 + dispatch 6 (`mode=1,log=1,count=2,stride=1`). This exercises the
    /// tail path at `0x110f1f8..0x110f380`, with the table built from the
    /// reverse stream instead of hardcoded.
    #[test]
    fn rans_segment_dispatch_mode1_three_lane_tail() {
        let descriptor_payload = hex_bytes("5b22b1399b96d244781d");
        let descriptor = rans_build_segment_descriptor(
            &descriptor_payload,
            RansFreqReader {
                ptr: 2,
                acc: 0x0c64faf64078a80c,
                bitpos: 57,
            },
        )
        .unwrap();
        assert_eq!(descriptor.mode, 1);
        assert_eq!(descriptor.log, 1);
        assert_eq!(descriptor.step, [0x0001_0000, 0x0001_0004]);
        assert_eq!(
            descriptor.reader,
            RansFreqReader {
                ptr: 0,
                acc: 0xfaf64078a80ebc20,
                bitpos: 57,
            }
        );
        let payload = sparse_payload(
            463,
            &[
                (0, "5b22b1399b96d244"),
                (431, "63ffffe917bfb0c8"),
                (455, "20f89bc3f9ff5f37"),
            ],
        );
        let mut states = [0u64; 4];
        let mut readers = [
            RansThreeLaneReader {
                ptr: 0,
                acc: 0xfaf64078a80ebc20,
                bitpos: 57,
            },
            RansThreeLaneReader {
                ptr: 431,
                acc: 0x226000043fffcc00,
                bitpos: 51,
            },
            RansThreeLaneReader {
                ptr: 455,
                acc: 0x910a0801fffb9a00,
                bitpos: 49,
            },
        ];
        let mut out = [0u16; 2];
        let used = rans_segment_dispatch_into(
            &mut out,
            RansSegmentDispatchSpec {
                mode: descriptor.mode,
                log: descriptor.log,
                value: descriptor.value,
                count: 2,
                stride: 1,
                states: &mut states,
                step: &descriptor.step,
                sym: &[],
                stream: &[],
                payload: &payload,
                three_lane_readers: Some(&mut readers),
            },
        )
        .unwrap();

        assert_eq!(used, 0);
        assert_eq!(out, [4, 0]);
        assert_eq!(
            readers,
            [
                RansThreeLaneReader {
                    ptr: 0,
                    acc: 0xf5ec80f1501d7844,
                    bitpos: 56,
                },
                RansThreeLaneReader {
                    ptr: 432,
                    acc: 0x44c000087fff98fe,
                    bitpos: 58,
                },
                RansThreeLaneReader {
                    ptr: 454,
                    acc: 0x910a0801fffb9baf,
                    bitpos: 57,
                },
            ]
        );

        let mut short_readers = readers;
        let mut short_out = [0u16; 2];
        assert_eq!(
            rans_three_lane_decode_into(
                &mut short_out,
                RansThreeLaneDecodeSpec {
                    count: 2,
                    log: 1,
                    stride: 1,
                    table: &descriptor.step,
                    readers: &mut short_readers,
                    payload: &[0; 7],
                },
            ),
            Err(RansThreeLaneDecodeError::PayloadTooSmall)
        );
    }

    /// Segment loop (`0x110dc30`) over the complete observed population.
    ///
    /// Provenance: `capture_segment_loop.py`, Animal_Bass loop 0. The
    /// enumerate-all population is exactly one loop call across Bear/Bass/
    /// Dragonfly: `byte_count=1932,lanes=3,segment_log=6`, dispatching one
    /// mode-0 segment followed by three mode-2 RLE segments. This covers the
    /// descriptor-to-dispatch pipeline and the subtle run carry across lanes
    /// (`0x110dd1c..0x110dd24`): the zero-valued RLE descriptor first finishes
    /// the last two symbols of lane 0, then carries into lanes 1 and 2.
    #[test]
    fn rans_segment_loop_bass_mode0_then_rle_lanes() {
        const BEFORE: &[(usize, u16)] = &[
            (62, 65535),
            (63, 65535),
            (64, 1),
            (69, 65535),
            (70, 1),
            (77, 65535),
            (78, 65535),
            (79, 1),
            (93, 65535),
            (94, 1),
            (116, 65535),
        ];
        const EXPECTED: &[(usize, u16)] = &[
            (6, 2),
            (90, 1022),
            (96, 1022),
            (102, 1022),
            (126, 1022),
            (129, 1022),
            (144, 2),
            (147, 2),
            (150, 2),
            (159, 2),
            (168, 2),
            (174, 2),
            (180, 2),
            (186, 2),
            (606, 2),
            (840, 1022),
            (876, 1022),
            (888, 2),
            (900, 2),
            (906, 2),
        ];
        let payload = sparse_payload(
            6225,
            &[
                (2018, "87e2163f7eff1a365ef3a7f6a5e841a3"),
                (6210, "1946484f6815d7c8e2d3909ede0000"),
            ],
        );
        let mut out = vec![0u16; 968];
        for &(idx, value) in BEFORE {
            out[idx] = value;
        }
        let mut context = RansSegmentLoopContext {
            reader: RansFreqReader {
                ptr: 6217,
                acc: 0x116801fe180f4a00,
                bitpos: 55,
            },
            mode1_extra_readers: [
                RansThreeLaneReader {
                    ptr: 6923,
                    acc: 0x78c45f1a02887500,
                    bitpos: 62,
                },
                RansThreeLaneReader {
                    ptr: 6928,
                    acc: 0xcb087f456207a1f8,
                    bitpos: 58,
                },
            ],
            stream_pos: 2018,
            state: RansStateBuffer {
                states: [0x68007d0ef80f, 0x674a999ea5a, 0x647f7484a3f513e, 0xd7a40fe0],
                flag: 0xf,
            },
        };

        let dispatches = rans_segment_loop_into(
            &mut out,
            &mut context,
            RansSegmentLoopSpec {
                byte_count: 1932,
                lanes: 3,
                segment_log: 6,
                payload: &payload,
            },
        )
        .unwrap();

        let mut expected = vec![0u16; 968];
        for &(idx, value) in EXPECTED {
            expected[idx] = value;
        }
        assert_eq!(dispatches, 4);
        assert_eq!(out, expected);
        assert_eq!(
            context,
            RansSegmentLoopContext {
                reader: RansFreqReader {
                    ptr: 6208,
                    acc: 0xe9e90d3e2c8d7100,
                    bitpos: 52,
                },
                mode1_extra_readers: [
                    RansThreeLaneReader {
                        ptr: 6923,
                        acc: 0x78c45f1a02887500,
                        bitpos: 62,
                    },
                    RansThreeLaneReader {
                        ptr: 6928,
                        acc: 0xcb087f456207a1f8,
                        bitpos: 58,
                    },
                ],
                stream_pos: 2034,
                state: RansStateBuffer {
                    states: [0xff653ce9, 0x2b0d9366535bcb9, 0x839f26fdba, 0xa88726296addc,],
                    flag: 0xf,
                },
            }
        );
        assert_eq!(out.iter().map(|&v| v as u32).sum::<u32>(), 7180);
    }

    #[test]
    fn rans_segment_loop_rejects_unobserved_mode1_and_bad_bounds() {
        let mode1_payload = sparse_payload(10, &[(0, "5b22b1399b96d244781d")]);
        let mut context = RansSegmentLoopContext {
            reader: RansFreqReader {
                ptr: 2,
                acc: 0x0c64faf64078a80c,
                bitpos: 57,
            },
            mode1_extra_readers: [
                RansThreeLaneReader {
                    ptr: 0,
                    acc: 0,
                    bitpos: 0,
                },
                RansThreeLaneReader {
                    ptr: 0,
                    acc: 0,
                    bitpos: 0,
                },
            ],
            stream_pos: 0,
            state: RansStateBuffer::warm([0; 4]),
        };
        let mut out = [0u16; 1];
        assert_eq!(
            rans_segment_loop_into(
                &mut out,
                &mut context,
                RansSegmentLoopSpec {
                    byte_count: 2,
                    lanes: 1,
                    segment_log: 0,
                    payload: &mode1_payload,
                },
            ),
            Err(RansSegmentLoopError::UnobservedMode1Segment)
        );

        let mut context = RansSegmentLoopContext {
            reader: RansFreqReader {
                ptr: 0,
                acc: 0,
                bitpos: 0,
            },
            mode1_extra_readers: [
                RansThreeLaneReader {
                    ptr: 0,
                    acc: 0,
                    bitpos: 0,
                },
                RansThreeLaneReader {
                    ptr: 0,
                    acc: 0,
                    bitpos: 0,
                },
            ],
            stream_pos: 0,
            state: RansStateBuffer::warm([0; 4]),
        };
        assert_eq!(
            rans_segment_loop_into(
                &mut out,
                &mut context,
                RansSegmentLoopSpec {
                    byte_count: 3,
                    lanes: 1,
                    segment_log: 0,
                    payload: &[],
                },
            ),
            Err(RansSegmentLoopError::OddByteCount)
        );
        assert_eq!(
            rans_segment_loop_into(
                &mut out,
                &mut context,
                RansSegmentLoopSpec {
                    byte_count: 2,
                    lanes: 0,
                    segment_log: 0,
                    payload: &[],
                },
            ),
            Err(RansSegmentLoopError::ZeroLaneCount)
        );
        assert_eq!(
            rans_segment_loop_into(
                &mut [],
                &mut context,
                RansSegmentLoopSpec {
                    byte_count: 2,
                    lanes: 1,
                    segment_log: 0,
                    payload: &[],
                },
            ),
            Err(RansSegmentLoopError::OutputTooSmall)
        );
        assert_eq!(
            rans_segment_loop_into(
                &mut out,
                &mut context,
                RansSegmentLoopSpec {
                    byte_count: 2,
                    lanes: 1,
                    segment_log: 0,
                    payload: &[],
                },
            ),
            Err(RansSegmentLoopError::Descriptor(
                RansSegmentDescriptorBuildError::Header(RansSegmentHeaderError::PayloadTooSmall)
            ))
        );
    }

    /// Byte-group reader (`0x110d7f0`) selector-3 direct-forward branch.
    ///
    /// Provenance: `capture_byte_group_reader.py`, Animal_Bass call 16:
    /// selector 3, `w2=0,w3=1,w4=3,w5=0`, forward stream bytes `1b001b`.
    /// The payload is relocated to a compact fixture-free buffer while keeping
    /// the observed combined selector window and resulting reader writeback.
    #[test]
    fn byte_group_reader_bass_selector3_direct_slice() {
        let payload = sparse_payload(11, &[(0, "0000000000000000"), (8, "1b001b")]);
        let mut state = ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 0,
                acc: 0xff0e_8001_39a1_a067,
                bitpos: 59,
            },
            stream_pos: 8,
        };

        let read = byte_group_read(
            &mut state,
            ByteGroupReadSpec {
                payload: &payload,
                element_shift: 0,
                group_stride: 1,
                count: 3,
            },
        )
        .unwrap();

        assert_eq!(
            read,
            ByteGroupRead {
                selector: 3,
                bytes: hex_bytes("1b001b"),
            }
        );
        assert_eq!(
            state,
            ByteGroupReadState {
                reader: RansThreeLaneReader {
                    ptr: 0,
                    acc: 0xfc3a_0004_e686_819c,
                    bitpos: 57,
                },
                stream_pos: 11,
            }
        );
    }

    #[test]
    fn byte_group_reader_rejects_unported_selectors_and_bad_bounds() {
        for selector in 0..=2u64 {
            let payload = sparse_payload(8, &[(0, "0000000000000000")]);
            let mut state = ByteGroupReadState {
                reader: RansThreeLaneReader {
                    ptr: 0,
                    acc: selector << 62,
                    bitpos: 59,
                },
                stream_pos: 0,
            };
            assert_eq!(
                byte_group_read(
                    &mut state,
                    ByteGroupReadSpec {
                        payload: &payload,
                        element_shift: 0,
                        group_stride: 1,
                        count: 1,
                    },
                ),
                Err(ByteGroupReadError::UnportedSelector(selector as u8))
            );
        }

        let mut state = ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 0,
                acc: 3 << 62,
                bitpos: 59,
            },
            stream_pos: 8,
        };
        assert_eq!(
            byte_group_read(
                &mut state,
                ByteGroupReadSpec {
                    payload: &[0; 7],
                    element_shift: 0,
                    group_stride: 1,
                    count: 1,
                },
            ),
            Err(ByteGroupReadError::PayloadTooSmall)
        );

        let payload = sparse_payload(10, &[(0, "0000000000000000"), (8, "1b00")]);
        let mut state = ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 0,
                acc: 3 << 62,
                bitpos: 59,
            },
            stream_pos: 8,
        };
        assert_eq!(
            byte_group_read(
                &mut state,
                ByteGroupReadSpec {
                    payload: &payload,
                    element_shift: 0,
                    group_stride: 1,
                    count: 3,
                },
            ),
            Err(ByteGroupReadError::StreamTooShort)
        );

        let payload = sparse_payload(8, &[(0, "0000000000000000")]);
        let mut state = ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 0,
                acc: 3 << 62,
                bitpos: 59,
            },
            stream_pos: 0,
        };
        assert_eq!(
            byte_group_read(
                &mut state,
                ByteGroupReadSpec {
                    payload: &payload,
                    element_shift: 0,
                    group_stride: 2,
                    count: usize::MAX,
                },
            ),
            Err(ByteGroupReadError::OutputSizeOverflow)
        );
    }

    /// Width combiner (`0x110d360`) with expanded stream bytes, history refs,
    /// special third-stream codes, and a non-clamped tail.
    ///
    /// Provenance: `capture_width_combiner.py`, Animal_Bear call 7. This is the
    /// compact discriminating call: `count=10,stride=16,shift=0,attr_width=3`,
    /// first-stream expansion, seven second-stream expansions, one history
    /// reference, nine special third-stream codes, and tail high half `699`.
    #[test]
    fn width_combiner_bear_expanded_history_tail_limit() {
        let payload = sparse_payload(
            28357,
            &[(
                28328,
                "a5f1856977d98c4c451d3db51f0c79400101013c00140a000c00822d14",
            )],
        );
        let stream0 = hex_bytes("16001700010000000000");
        let stream1 = hex_bytes("0f17191d1c1d1c0f19");
        let stream2 = hex_bytes("08000800090001000b000d000d000b0009000e00");
        let mut reader = RansThreeLaneReader {
            ptr: 28328,
            acc: 0,
            bitpos: 0,
        };
        let mut out = hex_width_records(
            "0700020010000000040002001000000001000200100000000100020010000000\
             0100020010000000010002001000000001000200100000000100020010000000\
             01000200100000000100020010000000",
        );

        let result = width_combiner_into(
            &mut out,
            WidthCombinerSpec {
                count: 10,
                stride: 16,
                shift: 0,
                attr_width: 3,
                limit: 3327,
                payload: &payload,
                stream0: &stream0,
                stream1: &stream1,
                stream2: &stream2,
                reader: &mut reader,
            },
        )
        .unwrap();

        assert_eq!(
            out,
            hex_width_records(
                "250012004002000000003200000300002c004800900500000000de0240020000\
                 01000f02501c000000006f023051000000009701a07d000000001200001500\
                 0000006100500400000000bb02409e0000"
            )
        );
        assert_eq!(
            result,
            WidthCombinerResult {
                ret: 82,
                consumed: [10, 9, 20],
            }
        );
        assert_eq!(
            reader,
            RansThreeLaneReader {
                ptr: 28351,
                acc: 0x0004_0404_f000_5028,
                bitpos: 62,
            }
        );
    }

    /// Width combiner (`0x110d360`) clamped-tail branch.
    ///
    /// Provenance: `capture_width_combiner.py`, Animal_Dragonfly call 2:
    /// `count=2,stride=20,shift=2,attr_width=1,limit=523`. The tail's
    /// `limit - (sum_width + first)` is non-positive, so the final record's
    /// high half and second word are both zero.
    #[test]
    fn width_combiner_dragonfly_tail_clamps() {
        let payload = sparse_payload(1207, &[(1190, "1c20c064a5a1c25aa7991d081e287cf867")]);
        let stream0 = hex_bytes("1a1c");
        let stream1 = hex_bytes("00");
        let stream2 = hex_bytes("0700");
        let mut reader = RansThreeLaneReader {
            ptr: 1190,
            acc: 0,
            bitpos: 0,
        };
        let mut out = hex_width_records("0100b3010a00000001004d00f8020000");

        let result = width_combiner_into(
            &mut out,
            WidthCombinerSpec {
                count: 2,
                stride: 20,
                shift: 2,
                attr_width: 1,
                limit: 523,
                payload: &payload,
                stream0: &stream0,
                stream1: &stream1,
                stream2: &stream2,
                reader: &mut reader,
            },
        )
        .unwrap();

        assert_eq!(out, hex_width_records("670001002c010000a301000000000000"));
        assert_eq!(
            result,
            WidthCombinerResult {
                ret: 522,
                consumed: [2, 1, 2],
            }
        );
        assert_eq!(
            reader,
            RansThreeLaneReader {
                ptr: 1200,
                acc: 0x0192_9687_096a_9e64,
                bitpos: 62,
            }
        );
    }

    #[test]
    fn width_combiner_rejects_unobserved_and_malformed_inputs() {
        let mut reader = RansThreeLaneReader {
            ptr: 0,
            acc: 0,
            bitpos: 0,
        };
        let mut one = [[0u32; 2]; 1];
        assert_eq!(
            width_combiner_into(
                &mut one,
                WidthCombinerSpec {
                    count: 1,
                    stride: 1,
                    shift: 0,
                    attr_width: 1,
                    limit: 1,
                    payload: &[0; 8],
                    stream0: &[0],
                    stream1: &[],
                    stream2: &[0, 0],
                    reader: &mut reader,
                },
            ),
            Err(WidthCombinerError::UnobservedTailOnlyCount(1))
        );

        let mut reader = RansThreeLaneReader {
            ptr: 0,
            acc: 0,
            bitpos: 0,
        };
        assert_eq!(
            width_combiner_into(
                &mut one,
                WidthCombinerSpec {
                    count: 2,
                    stride: 1,
                    shift: 0,
                    attr_width: 1,
                    limit: 1,
                    payload: &[0; 8],
                    stream0: &[0, 0],
                    stream1: &[0],
                    stream2: &[3, 0, 3, 0],
                    reader: &mut reader,
                },
            ),
            Err(WidthCombinerError::OutputTooSmall)
        );

        let mut two = [[0u32; 2]; 2];
        let mut reader = RansThreeLaneReader {
            ptr: 0,
            acc: 0,
            bitpos: 0,
        };
        assert_eq!(
            width_combiner_into(
                &mut two,
                WidthCombinerSpec {
                    count: 2,
                    stride: 1,
                    shift: 0,
                    attr_width: 1,
                    limit: 1,
                    payload: &[],
                    stream0: &[0, 0],
                    stream1: &[0],
                    stream2: &[3, 0, 3, 0],
                    reader: &mut reader,
                },
            ),
            Err(WidthCombinerError::PayloadTooSmall)
        );

        let mut reader = RansThreeLaneReader {
            ptr: 0,
            acc: 0,
            bitpos: 0,
        };
        assert_eq!(
            width_combiner_into(
                &mut two,
                WidthCombinerSpec {
                    count: 2,
                    stride: 1,
                    shift: 0,
                    attr_width: 1,
                    limit: 10,
                    payload: &[0; 8],
                    stream0: &[1, 1],
                    stream1: &[0],
                    stream2: &[0, 0],
                    reader: &mut reader,
                },
            ),
            Err(WidthCombinerError::HistoryOutOfBounds)
        );
    }

    /// Transform tail `0x10fc5e0`: literal bytes plus overlapping copy-back.
    ///
    /// Provenance: `capture_transform_tails.py`, Animal_Bear `0x10fc5e0` call,
    /// first two records from entry `0x10000801`: `(37,18,576)` then
    /// `(0,50,768)`. This compact slice keeps the observed stride-16 cursor and
    /// byte-distance copy units that `verify_transform_tail_copy1.py` replays
    /// over the full 3-call population.
    #[test]
    fn transform_tail_copy1_bear_literal_and_copy_back() {
        let source =
            hex_bytes("7f7f7f7f7f7f7f7f7f8181817f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f");
        let records = [
            TransformTailRecord {
                literal_count: 37,
                copy_count: 18,
                back_distance: 576,
            },
            TransformTailRecord {
                literal_count: 0,
                copy_count: 50,
                back_distance: 768,
            },
        ];
        let expected_lane = hex_bytes(
            "7f7f7f7f7f7f7f7f7f8181817f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f\
             7f7f7f7f7f7f7f7f7f7f8181817f7f7f7f7f7f7f7f7f8181817f7f7f7f7f7f7f7f7f7f\
             7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f8181817f7f7f7f7f7f7f7f7f",
        );
        let mut out = vec![0xee; expected_lane.len() * 16];

        let consumed = transform_tail_copy1_into(
            &mut out,
            TransformTailCopy1Spec {
                output_stride: 16,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &source,
            },
        )
        .unwrap();

        assert_eq!(consumed, 37);
        for (index, &expected) in expected_lane.iter().enumerate() {
            assert_eq!(out[index * 16], expected);
        }
        for (index, &byte) in out.iter().enumerate() {
            if index % 16 != 0 {
                assert_eq!(byte, 0xee, "non-lane byte {index} changed");
            }
        }
        assert_eq!(out[1], 0xee, "rules out a contiguous cursor");
        assert_eq!(out[37 * 16], source[1], "copy-back distance is in bytes");
        assert_eq!(out[55 * 16], source[7], "zero-literal record copies only");
    }

    /// Transform tail `0x10fc680`: two-byte literals plus copy-back.
    ///
    /// Provenance: `capture_transform_tails.py`, Animal_Dragonfly
    /// `0x10fc680` call, all three records from entry `0x0a000802`:
    /// `(1,435,10)`, `(1,77,760)`, `(1,8,100)`. This covers the full
    /// observed population for the two-byte copy tail and keeps the stride-10
    /// byte-distance copy units replayed by `verify_transform_tail_copy2.py`.
    #[test]
    fn transform_tail_copy2_dragonfly_two_byte_runs() {
        let source = hex_bytes("ff007f807f80");
        let records = [
            TransformTailRecord {
                literal_count: 1,
                copy_count: 435,
                back_distance: 10,
            },
            TransformTailRecord {
                literal_count: 1,
                copy_count: 77,
                back_distance: 760,
            },
            TransformTailRecord {
                literal_count: 1,
                copy_count: 8,
                back_distance: 100,
            },
        ];
        let expected_lane = hex_bytes(concat!(
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff007f80",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff007f80",
            "ff007f80ff00ff00ff00ff00ff00ff00ff007f80",
        ));
        let mut out = vec![0xee; (expected_lane.len() / 2) * 10];

        let consumed = transform_tail_copy2_into(
            &mut out,
            TransformTailCopy2Spec {
                output_stride: 10,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &source,
            },
        )
        .unwrap();

        assert_eq!(consumed, 6);
        for (unit_index, expected) in expected_lane.chunks_exact(2).enumerate() {
            let base = unit_index * 10;
            assert_eq!(&out[base..base + 2], expected);
        }
        for (index, &byte) in out.iter().enumerate() {
            if index % 10 >= 2 {
                assert_eq!(byte, 0xee, "non-lane byte {index} changed");
            }
        }
        assert_eq!(&out[10..12], &source[0..2], "rules out a contiguous cursor");
        assert_eq!(
            &out[436 * 10..436 * 10 + 2],
            &source[2..4],
            "second literal follows a long copy run"
        );
        assert_eq!(
            &out[512 * 10..512 * 10 + 2],
            &source[2..4],
            "copy-back distance is in bytes across prior literals"
        );
    }

    /// Transform tail `0x10fc7d0`: four-byte literals plus copy-back.
    ///
    /// Provenance: `capture_transform_tails.py`, Animal_Bear `0x10fc7d0` call,
    /// first record from entry `0x1000100a`: `(101,2,1616)`. The replay script
    /// covers the full 2-call population; this compact golden keeps the
    /// observed stride-16 cursor and byte-distance copy units fixture-free.
    #[test]
    fn transform_tail_copy4_bear_u32_runs() {
        let source = hex_bytes(concat!(
            "3b064795f0051595bf055795f9021688a6033888df0323872307b08ede07e78e7307778ea07fd736f77f913833800638",
            "4a75cc0b6f746907cf71010d9229a03a132bd13f022daa3a3c1774076e15e80162134f07a43fb316b941bf1b12438917",
            "ad0e22417e0d023cab0ba23f815191035c4fd8015650b7067a81fa0c7b821012f3831c0cd46c1e030a6f78087c6faf01",
            "3a0c046f3b0b556a630a796d9b605507025f3602f65d7005b50952f66704bcf7cf0f71fad02d1d0fcb348a117730290f",
            "dff8009fadf8be9e63f8f09ec1f6ec96faf60198a7f7de9703f8a09a98f7109b54f8d89afe8305383b8490389184d636",
            "5f8c2b03858e79071a8f0a04bef9b1027dfbe9072efdc702f7d1b203aed33709d0d5c903b39de10eee9bd40a5f9a1010",
            "3ef12d705ff213747af4416faab1440f1eb2520a2bb0430cfe88840c6189a312468bce0d6ba1750612a05f01309fc405",
            "a2fe704c6cfdf247fffba54e19b83b040db7010175b52006caece3f933f82ef7e5f2c4f571cce81991c7b21af5cec01a",
            "b806389451060395b7063a95b8063894d505d094",
        ));
        let records = [TransformTailRecord {
            literal_count: 101,
            copy_count: 2,
            back_distance: 1616,
        }];
        let expected_lane = hex_bytes(concat!(
            "3b064795f0051595bf055795f9021688a6033888df0323872307b08ede07e78e7307778ea07fd736f77f913833800638",
            "4a75cc0b6f746907cf71010d9229a03a132bd13f022daa3a3c1774076e15e80162134f07a43fb316b941bf1b12438917",
            "ad0e22417e0d023cab0ba23f815191035c4fd8015650b7067a81fa0c7b821012f3831c0cd46c1e030a6f78087c6faf01",
            "3a0c046f3b0b556a630a796d9b605507025f3602f65d7005b50952f66704bcf7cf0f71fad02d1d0fcb348a117730290f",
            "dff8009fadf8be9e63f8f09ec1f6ec96faf60198a7f7de9703f8a09a98f7109b54f8d89afe8305383b8490389184d636",
            "5f8c2b03858e79071a8f0a04bef9b1027dfbe9072efdc702f7d1b203aed33709d0d5c903b39de10eee9bd40a5f9a1010",
            "3ef12d705ff213747af4416faab1440f1eb2520a2bb0430cfe88840c6189a312468bce0d6ba1750612a05f01309fc405",
            "a2fe704c6cfdf247fffba54e19b83b040db7010175b52006caece3f933f82ef7e5f2c4f571cce81991c7b21af5cec01a",
            "b806389451060395b7063a95b8063894d505d0943b064795f0051595",
        ));
        let mut out = vec![0xee; (expected_lane.len() / 4) * 16];

        let consumed = transform_tail_copy4_into(
            &mut out,
            TransformTailCopy4Spec {
                output_stride: 16,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &source,
            },
        )
        .unwrap();

        assert_eq!(consumed, 404);
        for (unit_index, expected) in expected_lane.chunks_exact(4).enumerate() {
            let base = unit_index * 16;
            assert_eq!(&out[base..base + 4], expected);
        }
        for (index, &byte) in out.iter().enumerate() {
            if index % 16 >= 4 {
                assert_eq!(byte, 0xee, "non-lane byte {index} changed");
            }
        }
        assert_eq!(&out[101 * 16..101 * 16 + 4], &source[0..4]);
        assert_eq!(&out[102 * 16..102 * 16 + 4], &source[4..8]);
    }

    #[test]
    fn transform_tail_copy4_allows_observed_zero_literal_and_zero_copy() {
        let zero_literal = [TransformTailRecord {
            literal_count: 0,
            copy_count: 2,
            back_distance: 5440,
        }];
        let mut out = vec![0xee; 5460];
        out[0..4].copy_from_slice(&[0x30, 0x2a, 0x03, 0xdd]);
        out[16..20].copy_from_slice(&[0x61, 0x1f, 0x3c, 0xd7]);
        let consumed = transform_tail_copy4_into(
            &mut out,
            TransformTailCopy4Spec {
                output_stride: 16,
                block_index: 0,
                out_offset: 5440,
                records: &zero_literal,
                source: &[],
            },
        )
        .unwrap();
        assert_eq!(consumed, 0);
        assert_eq!(&out[5440..5444], &[0x30, 0x2a, 0x03, 0xdd]);
        assert_eq!(&out[5456..5460], &[0x61, 0x1f, 0x3c, 0xd7]);

        let zero_copy = [TransformTailRecord {
            literal_count: 10,
            copy_count: 0,
            back_distance: 0,
        }];
        let source = hex_bytes(
            "0a22dae92211dcd3462878d7f0382deca11d35bccb3153c77e33ccd50b4b0fe61b3850c3b251aed4",
        );
        let mut out = vec![0xee; 10 * 16];
        let consumed = transform_tail_copy4_into(
            &mut out,
            TransformTailCopy4Spec {
                output_stride: 16,
                block_index: 0,
                out_offset: 0,
                records: &zero_copy,
                source: &source,
            },
        )
        .unwrap();
        assert_eq!(consumed, 40);
        for (unit_index, expected) in source.chunks_exact(4).enumerate() {
            let base = unit_index * 16;
            assert_eq!(&out[base..base + 4], expected);
        }
    }

    /// Transform tail `0x10fbcc0`: two-byte direct and matched deltas.
    ///
    /// Provenance: `capture_transform_tails.py`, Animal_Bass `0x10fbcc0` call,
    /// first record from entry `0x0a000802`: `(276,6,20)`. This covers direct
    /// literals, match-table literals, and the copy loop; the replay script
    /// covers the full 1-call population including the later zero-count records.
    #[test]
    fn transform_tail_delta2_bass_direct_match_and_copy() {
        let source0 = hex_bytes(concat!(
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000000000",
        ));
        let source1 = hex_bytes(concat!(
            "000000000000000000000000000000000000000000cc000000000000b2a6000000000000000000000000000000000000",
            "00000000000000000000cc00cc00000000000000000000000000000000000000000000000000000000001a001a80bfbf",
            "1a801a000000000000001a0000008000bf00bf000000000000001a1a000000000000cc",
        ));
        let source2 = hex_bytes(concat!(
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "0000",
        ));
        let matches = hex_u32_words(concat!(
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000003100000031000000000000003000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000006101000000000000",
            "790100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000009901000091010000a10100009901000099010000990100009901000089000000810000009100000089000000",
            "890000008900000081000000910000008900000089000000390200003902000039020000590200006902000081020000",
            "810200007901000088010000990100008801000089010000880100005202000000000000000000000000000000000000",
            "000000005800000058000000290200005002000060020000290200002802000028020000890200005002000060020000",
            "89020000300000008802000098000000fb020000a1000000a0000000a1000000a1000000a00000009002000010030000",
            "110300001003000031030000400300005903000059030000a102000099020000a8020000a002000000040000a0020000",
            "18040000a0020000a1020000e80200000000000048000000000000000000000000000000080300000000000000000000",
            "000000000000000000000000280300000000000000000000b10400000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000300400002001000000000000000000004004000000000000b8010000f8010000a8010000b8010000",
            "b0010000b1010000b001000020020000b0010000b1010000b1010000b0010000b101000038020000b1010000b1010000",
            "60060000b1010000b1010000b1010000b1010000b1010000b1010000b1010000b1010000b1010000b1010000b1010000",
            "b1010000b0010000b1010000b1010000b1010000b1010000b0010000b1010000b1010000b1010000b1010000b1010000",
            "b1010000b1010000b1010000b1010000b1010000b0010000b0010000b101000040030000a9010000a801000048030000",
            "a8010000a801000000000000500000006800000000000000000000000000000000000000980000000000000000000000",
            "c00000000000000000000000e0000000f000000000000000000000000000000000000000000000003801000000000000",
            "000000000000000068010000000000007001000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000",
        ));
        let records = [TransformTailRecord {
            literal_count: 276,
            copy_count: 6,
            back_distance: 20,
        }];
        let expected_lane = hex_bytes(concat!(
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff0033ccff00ff00",
            "ff00ff00ff00ff004db259a6ff00ff00ff00ff004db2ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff0033ccff00",
            "33ccff00ff00ff00ff00ff00ff004db2ff00ff00ff00ff00ff004db259a6ff00ff00ff0033ccff0033ccff00ff0033cc",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00e51aff00e51a7f8040bf40bfe51a7f80e51aff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00e51aff00e51a7f8040bf40bfe51a7f80e51aff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00e51aff00e51aff00ff00ff007f807f80ff0040bf40bfff0040bf40bf7f80ff00ff00ff00ff00ff00e51aff00",
            "ff00e51aff00e51aff00ff00ff00ff00ff00ff00ff0033ccff0033ccff0033ccff0033cc",
        ));
        let mut out = vec![0xee; (expected_lane.len() / 2) * 10];

        let usage = transform_tail_delta2_into(
            &mut out,
            TransformTailDelta2Spec {
                output_stride: 10,
                block_index: 0,
                out_offset: 0,
                records: &records,
                matches: &matches,
                source0: &source0,
                source1: &source1,
                source2: &source2,
            },
        )
        .unwrap();

        assert_eq!(
            usage,
            TransformTailDeltaUsage {
                source0: 131,
                source1: 131,
                source2: 290,
                match_entries: 282,
            }
        );
        for (unit_index, expected) in expected_lane.chunks_exact(2).enumerate() {
            let base = unit_index * 10;
            assert_eq!(&out[base..base + 2], expected);
        }
        for (index, &byte) in out.iter().enumerate() {
            if index % 10 >= 2 {
                assert_eq!(byte, 0xee, "non-lane byte {index} changed");
            }
        }
        assert_eq!(&out[0..2], &[0xff, 0x00], "direct literal uses minus one");
        assert_eq!(&out[34 * 10..34 * 10 + 2], &[0x4d, 0xb2]);
        assert_eq!(&out[281 * 10..281 * 10 + 2], &[0x33, 0xcc]);
    }

    /// Transform tail `0x10fbdc0`: three-byte direct and matched deltas.
    ///
    /// Provenance: `capture_transform_tails.py`, Animal_Bear `0x10fbdc0` call,
    /// first two records from entry `0x0c000803`: `(1,41,12)` and
    /// `(52,87,12)`. This covers direct literals, match-table literals, and
    /// the copy loop; the replay script covers the full 1-call population.
    #[test]
    fn transform_tail_delta3_bear_direct_match_and_copy() {
        let source0 = hex_bytes("00000000000000");
        let source1 = hex_bytes("0000990000003300000000000000");
        let source2 = hex_bytes(concat!(
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        ));
        let matches = hex_u32_words(concat!(
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "700100008001000091010000710100008101000091010000710100008101000091010000710100008101000091010000",
            "710100008101000091010000710100008101000091010000710100008101000091010000710100008101000091010000",
            "710100008101000091010000710100008101000091010000710100008101000091010000710100008101000091010000",
            "710100008101000091010000710100008101000091010000710100008101000091010000710100008101000091010000",
            "000000000000000000000000180000000000000000000000000000002900000039000000490000003000000010000000",
            "390000003800000000000000000000000000000000000000000000000000000000000000000000000000000028000000",
            "280000000000000000000000000000000000000000000000000000002800000000000000300000000000000000000000",
            "000000000000000000000000280000000000000030000000000000000000000000000000000000000000000028000000",
            "000000003000000000000000000000000000000000000000000000002800000000000000300000000000000000000000",
            "000000000000000000000000180000000000000030000000000000000000000000000000000000001800000000000000",
            "000000003800000000000000000000000000000000000000000000001800000000000000380000000000000028040000",
            "48040000",
        ));
        let records = [
            TransformTailRecord {
                literal_count: 1,
                copy_count: 41,
                back_distance: 12,
            },
            TransformTailRecord {
                literal_count: 52,
                copy_count: 87,
                back_distance: 12,
            },
        ];
        let expected_lane = hex_bytes(concat!(
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000669900ff0000cc3300ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000cc3300ff0000669900ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000",
        ));
        let mut out = vec![0xee; (expected_lane.len() / 3) * 12];

        let usage = transform_tail_delta3_into(
            &mut out,
            TransformTailDelta3Spec {
                output_stride: 12,
                block_index: 0,
                out_offset: 0,
                records: &records,
                matches: &matches,
                source0: &source0,
                source1: &source1,
                source2: &source2,
            },
        )
        .unwrap();

        assert_eq!(
            usage,
            TransformTailDeltaUsage {
                source0: 7,
                source1: 14,
                source2: 138,
                match_entries: 181,
            }
        );
        for (unit_index, expected) in expected_lane.chunks_exact(3).enumerate() {
            let base = unit_index * 12;
            assert_eq!(&out[base..base + 3], expected);
        }
        for (index, &byte) in out.iter().enumerate() {
            if index % 12 >= 3 {
                assert_eq!(byte, 0xee, "non-lane byte {index} changed");
            }
        }
        assert_eq!(&out[0..3], &[0xff, 0x00, 0x00]);
        assert_eq!(&out[42 * 12..42 * 12 + 3], &[0x66, 0x99, 0x00]);
        assert_eq!(&out[48 * 12..48 * 12 + 3], &[0xff, 0x00, 0x00]);
    }

    #[test]
    fn transform_tail_copy1_rejects_malformed_inputs() {
        let records = [TransformTailRecord {
            literal_count: 1,
            copy_count: 0,
            back_distance: 0,
        }];
        let mut out = [0u8; 1];
        assert_eq!(
            transform_tail_copy1_into(
                &mut out,
                TransformTailCopy1Spec {
                    output_stride: 0,
                    block_index: 0,
                    out_offset: 0,
                    records: &records,
                    source: &[1],
                },
            ),
            Err(TransformTailCopyError::ZeroStride)
        );
        assert_eq!(
            transform_tail_copy1_into(
                &mut out,
                TransformTailCopy1Spec {
                    output_stride: 1,
                    block_index: 0,
                    out_offset: 0,
                    records: &records,
                    source: &[],
                },
            ),
            Err(TransformTailCopyError::SourceTooSmall)
        );

        let mut empty = [];
        assert_eq!(
            transform_tail_copy1_into(
                &mut empty,
                TransformTailCopy1Spec {
                    output_stride: 1,
                    block_index: 0,
                    out_offset: 0,
                    records: &records,
                    source: &[1],
                },
            ),
            Err(TransformTailCopyError::OutputTooSmall)
        );

        let copy_first = [TransformTailRecord {
            literal_count: 0,
            copy_count: 1,
            back_distance: 1,
        }];
        assert_eq!(
            transform_tail_copy1_into(
                &mut out,
                TransformTailCopy1Spec {
                    output_stride: 1,
                    block_index: 0,
                    out_offset: 0,
                    records: &copy_first,
                    source: &[],
                },
            ),
            Err(TransformTailCopyError::CopyBeforeOutput)
        );
    }

    #[test]
    fn transform_tail_copy2_rejects_unobserved_and_malformed_inputs() {
        let records = [TransformTailRecord {
            literal_count: 1,
            copy_count: 1,
            back_distance: 2,
        }];
        let mut out = [0u8; 4];
        assert_eq!(
            transform_tail_copy2_into(
                &mut out,
                TransformTailCopy2Spec {
                    output_stride: 0,
                    block_index: 0,
                    out_offset: 0,
                    records: &records,
                    source: &[1, 2],
                },
            ),
            Err(TransformTailCopyError::ZeroStride)
        );
        assert_eq!(
            transform_tail_copy2_into(
                &mut out,
                TransformTailCopy2Spec {
                    output_stride: 2,
                    block_index: 0,
                    out_offset: 0,
                    records: &records,
                    source: &[1],
                },
            ),
            Err(TransformTailCopyError::SourceTooSmall)
        );

        let mut short_out = [0u8; 1];
        assert_eq!(
            transform_tail_copy2_into(
                &mut short_out,
                TransformTailCopy2Spec {
                    output_stride: 2,
                    block_index: 0,
                    out_offset: 0,
                    records: &records,
                    source: &[1, 2],
                },
            ),
            Err(TransformTailCopyError::OutputTooSmall)
        );

        let copy_before = [TransformTailRecord {
            literal_count: 1,
            copy_count: 1,
            back_distance: 4,
        }];
        assert_eq!(
            transform_tail_copy2_into(
                &mut out,
                TransformTailCopy2Spec {
                    output_stride: 2,
                    block_index: 0,
                    out_offset: 0,
                    records: &copy_before,
                    source: &[1, 2],
                },
            ),
            Err(TransformTailCopyError::CopyBeforeOutput)
        );

        let zero_literal = [TransformTailRecord {
            literal_count: 0,
            copy_count: 1,
            back_distance: 2,
        }];
        assert_eq!(
            transform_tail_copy2_into(
                &mut out,
                TransformTailCopy2Spec {
                    output_stride: 2,
                    block_index: 0,
                    out_offset: 0,
                    records: &zero_literal,
                    source: &[1, 2],
                },
            ),
            Err(TransformTailCopyError::UnobservedRecordShape)
        );

        let zero_copy = [TransformTailRecord {
            literal_count: 1,
            copy_count: 0,
            back_distance: 0,
        }];
        assert_eq!(
            transform_tail_copy2_into(
                &mut out,
                TransformTailCopy2Spec {
                    output_stride: 2,
                    block_index: 0,
                    out_offset: 0,
                    records: &zero_copy,
                    source: &[1, 2],
                },
            ),
            Err(TransformTailCopyError::UnobservedRecordShape)
        );
    }

    #[test]
    fn transform_tail_copy4_rejects_malformed_inputs() {
        let records = [TransformTailRecord {
            literal_count: 1,
            copy_count: 1,
            back_distance: 4,
        }];
        let mut out = [0u8; 8];
        assert_eq!(
            transform_tail_copy4_into(
                &mut out,
                TransformTailCopy4Spec {
                    output_stride: 0,
                    block_index: 0,
                    out_offset: 0,
                    records: &records,
                    source: &[1, 2, 3, 4],
                },
            ),
            Err(TransformTailCopyError::ZeroStride)
        );
        assert_eq!(
            transform_tail_copy4_into(
                &mut out,
                TransformTailCopy4Spec {
                    output_stride: 4,
                    block_index: 0,
                    out_offset: 0,
                    records: &records,
                    source: &[1, 2, 3],
                },
            ),
            Err(TransformTailCopyError::SourceTooSmall)
        );

        let mut short_out = [0u8; 3];
        assert_eq!(
            transform_tail_copy4_into(
                &mut short_out,
                TransformTailCopy4Spec {
                    output_stride: 4,
                    block_index: 0,
                    out_offset: 0,
                    records: &records,
                    source: &[1, 2, 3, 4],
                },
            ),
            Err(TransformTailCopyError::OutputTooSmall)
        );

        let copy_before = [TransformTailRecord {
            literal_count: 1,
            copy_count: 1,
            back_distance: 8,
        }];
        assert_eq!(
            transform_tail_copy4_into(
                &mut out,
                TransformTailCopy4Spec {
                    output_stride: 4,
                    block_index: 0,
                    out_offset: 0,
                    records: &copy_before,
                    source: &[1, 2, 3, 4],
                },
            ),
            Err(TransformTailCopyError::CopyBeforeOutput)
        );
    }

    #[test]
    fn transform_tail_delta2_allows_observed_zero_literal_and_zero_copy() {
        let zero_literal = [TransformTailRecord {
            literal_count: 0,
            copy_count: 7,
            back_distance: 1780,
        }];
        let mut out = vec![0xee; 1842];
        for unit in 0..7 {
            let base = unit * 10;
            out[base] = unit as u8;
            out[base + 1] = 0x80 | unit as u8;
        }
        let usage = transform_tail_delta2_into(
            &mut out,
            TransformTailDelta2Spec {
                output_stride: 10,
                block_index: 0,
                out_offset: 1780,
                records: &zero_literal,
                matches: &[0; 7],
                source0: &[],
                source1: &[],
                source2: &[],
            },
        )
        .unwrap();
        assert_eq!(
            usage,
            TransformTailDeltaUsage {
                source0: 0,
                source1: 0,
                source2: 0,
                match_entries: 7,
            }
        );
        for unit in 0..7 {
            let base = 1780 + unit * 10;
            assert_eq!(&out[base..base + 2], &[unit as u8, 0x80 | unit as u8]);
        }

        let zero_copy = [TransformTailRecord {
            literal_count: 1,
            copy_count: 0,
            back_distance: 0,
        }];
        let mut out = vec![0xee; 10];
        let usage = transform_tail_delta2_into(
            &mut out,
            TransformTailDelta2Spec {
                output_stride: 10,
                block_index: 0,
                out_offset: 0,
                records: &zero_copy,
                matches: &[0],
                source0: &[0],
                source1: &[0],
                source2: &[],
            },
        )
        .unwrap();
        assert_eq!(
            usage,
            TransformTailDeltaUsage {
                source0: 1,
                source1: 1,
                source2: 0,
                match_entries: 1,
            }
        );
        assert_eq!(&out[0..2], &[0xff, 0x00]);
    }

    #[test]
    fn transform_tail_delta2_rejects_malformed_inputs() {
        let direct = [TransformTailRecord {
            literal_count: 1,
            copy_count: 0,
            back_distance: 0,
        }];
        let mut out = [0u8; 2];
        assert_eq!(
            transform_tail_delta2_into(
                &mut out,
                TransformTailDelta2Spec {
                    output_stride: 0,
                    block_index: 0,
                    out_offset: 0,
                    records: &direct,
                    matches: &[0],
                    source0: &[0],
                    source1: &[0],
                    source2: &[],
                },
            ),
            Err(TransformTailDeltaError::ZeroStride)
        );
        assert_eq!(
            transform_tail_delta2_into(
                &mut out,
                TransformTailDelta2Spec {
                    output_stride: 2,
                    block_index: 0,
                    out_offset: 0,
                    records: &direct,
                    matches: &[],
                    source0: &[0],
                    source1: &[0],
                    source2: &[],
                },
            ),
            Err(TransformTailDeltaError::MatchTableTooSmall)
        );
        assert_eq!(
            transform_tail_delta2_into(
                &mut out,
                TransformTailDelta2Spec {
                    output_stride: 2,
                    block_index: 0,
                    out_offset: 0,
                    records: &direct,
                    matches: &[0],
                    source0: &[],
                    source1: &[0],
                    source2: &[],
                },
            ),
            Err(TransformTailDeltaError::Source0TooSmall)
        );
        assert_eq!(
            transform_tail_delta2_into(
                &mut out,
                TransformTailDelta2Spec {
                    output_stride: 2,
                    block_index: 0,
                    out_offset: 0,
                    records: &direct,
                    matches: &[0],
                    source0: &[0],
                    source1: &[],
                    source2: &[],
                },
            ),
            Err(TransformTailDeltaError::Source1TooSmall)
        );

        let matched = [TransformTailRecord {
            literal_count: 1,
            copy_count: 0,
            back_distance: 0,
        }];
        assert_eq!(
            transform_tail_delta2_into(
                &mut out,
                TransformTailDelta2Spec {
                    output_stride: 2,
                    block_index: 0,
                    out_offset: 0,
                    records: &matched,
                    matches: &[8],
                    source0: &[],
                    source1: &[],
                    source2: &[0, 0],
                },
            ),
            Err(TransformTailDeltaError::MatchBeforeOutput)
        );

        let mut matched_out = [0u8; 12];
        assert_eq!(
            transform_tail_delta2_into(
                &mut matched_out,
                TransformTailDelta2Spec {
                    output_stride: 10,
                    block_index: 0,
                    out_offset: 10,
                    records: &matched,
                    matches: &[8],
                    source0: &[],
                    source1: &[],
                    source2: &[0],
                },
            ),
            Err(TransformTailDeltaError::Source2TooSmall)
        );

        let mut short_out = [0u8; 1];
        assert_eq!(
            transform_tail_delta2_into(
                &mut short_out,
                TransformTailDelta2Spec {
                    output_stride: 2,
                    block_index: 0,
                    out_offset: 0,
                    records: &direct,
                    matches: &[0],
                    source0: &[0],
                    source1: &[0],
                    source2: &[],
                },
            ),
            Err(TransformTailDeltaError::OutputTooSmall)
        );

        let copy_first = [TransformTailRecord {
            literal_count: 0,
            copy_count: 1,
            back_distance: 1,
        }];
        assert_eq!(
            transform_tail_delta2_into(
                &mut out,
                TransformTailDelta2Spec {
                    output_stride: 2,
                    block_index: 0,
                    out_offset: 0,
                    records: &copy_first,
                    matches: &[0],
                    source0: &[],
                    source1: &[],
                    source2: &[],
                },
            ),
            Err(TransformTailDeltaError::CopyBeforeOutput)
        );
    }

    #[test]
    fn transform_tail_delta3_allows_observed_zero_literal_and_zero_copy() {
        let zero_literal = [TransformTailRecord {
            literal_count: 0,
            copy_count: 4,
            back_distance: 600,
        }];
        let mut out = vec![0xee; 639];
        for unit in 0..4 {
            let base = unit * 12;
            out[base] = unit as u8;
            out[base + 1] = 0x40 | unit as u8;
            out[base + 2] = 0x80 | unit as u8;
        }
        let usage = transform_tail_delta3_into(
            &mut out,
            TransformTailDelta3Spec {
                output_stride: 12,
                block_index: 0,
                out_offset: 600,
                records: &zero_literal,
                matches: &[0; 4],
                source0: &[],
                source1: &[],
                source2: &[],
            },
        )
        .unwrap();
        assert_eq!(
            usage,
            TransformTailDeltaUsage {
                source0: 0,
                source1: 0,
                source2: 0,
                match_entries: 4,
            }
        );
        for unit in 0..4 {
            let base = 600 + unit * 12;
            assert_eq!(
                &out[base..base + 3],
                &[unit as u8, 0x40 | unit as u8, 0x80 | unit as u8]
            );
        }

        let zero_copy = [TransformTailRecord {
            literal_count: 1,
            copy_count: 0,
            back_distance: 0,
        }];
        let mut out = vec![0xee; 12];
        let usage = transform_tail_delta3_into(
            &mut out,
            TransformTailDelta3Spec {
                output_stride: 12,
                block_index: 0,
                out_offset: 0,
                records: &zero_copy,
                matches: &[0],
                source0: &[0],
                source1: &[0, 0],
                source2: &[],
            },
        )
        .unwrap();
        assert_eq!(
            usage,
            TransformTailDeltaUsage {
                source0: 1,
                source1: 2,
                source2: 0,
                match_entries: 1,
            }
        );
        assert_eq!(&out[0..3], &[0xff, 0x00, 0x00]);
    }

    #[test]
    fn transform_tail_delta3_rejects_malformed_inputs() {
        let direct = [TransformTailRecord {
            literal_count: 1,
            copy_count: 0,
            back_distance: 0,
        }];
        let mut out = [0u8; 3];
        assert_eq!(
            transform_tail_delta3_into(
                &mut out,
                TransformTailDelta3Spec {
                    output_stride: 0,
                    block_index: 0,
                    out_offset: 0,
                    records: &direct,
                    matches: &[0],
                    source0: &[0],
                    source1: &[0, 0],
                    source2: &[],
                },
            ),
            Err(TransformTailDeltaError::ZeroStride)
        );
        assert_eq!(
            transform_tail_delta3_into(
                &mut out,
                TransformTailDelta3Spec {
                    output_stride: 3,
                    block_index: 0,
                    out_offset: 0,
                    records: &direct,
                    matches: &[],
                    source0: &[0],
                    source1: &[0, 0],
                    source2: &[],
                },
            ),
            Err(TransformTailDeltaError::MatchTableTooSmall)
        );
        assert_eq!(
            transform_tail_delta3_into(
                &mut out,
                TransformTailDelta3Spec {
                    output_stride: 3,
                    block_index: 0,
                    out_offset: 0,
                    records: &direct,
                    matches: &[0],
                    source0: &[],
                    source1: &[0, 0],
                    source2: &[],
                },
            ),
            Err(TransformTailDeltaError::Source0TooSmall)
        );
        assert_eq!(
            transform_tail_delta3_into(
                &mut out,
                TransformTailDelta3Spec {
                    output_stride: 3,
                    block_index: 0,
                    out_offset: 0,
                    records: &direct,
                    matches: &[0],
                    source0: &[0],
                    source1: &[0],
                    source2: &[],
                },
            ),
            Err(TransformTailDeltaError::Source1TooSmall)
        );

        let matched = [TransformTailRecord {
            literal_count: 1,
            copy_count: 0,
            back_distance: 0,
        }];
        assert_eq!(
            transform_tail_delta3_into(
                &mut out,
                TransformTailDelta3Spec {
                    output_stride: 3,
                    block_index: 0,
                    out_offset: 0,
                    records: &matched,
                    matches: &[8],
                    source0: &[],
                    source1: &[],
                    source2: &[0, 0, 0],
                },
            ),
            Err(TransformTailDeltaError::MatchBeforeOutput)
        );

        let mut matched_out = [0u8; 15];
        assert_eq!(
            transform_tail_delta3_into(
                &mut matched_out,
                TransformTailDelta3Spec {
                    output_stride: 12,
                    block_index: 0,
                    out_offset: 12,
                    records: &matched,
                    matches: &[8],
                    source0: &[],
                    source1: &[],
                    source2: &[0, 0],
                },
            ),
            Err(TransformTailDeltaError::Source2TooSmall)
        );

        let mut short_out = [0u8; 2];
        assert_eq!(
            transform_tail_delta3_into(
                &mut short_out,
                TransformTailDelta3Spec {
                    output_stride: 3,
                    block_index: 0,
                    out_offset: 0,
                    records: &direct,
                    matches: &[0],
                    source0: &[0],
                    source1: &[0, 0],
                    source2: &[],
                },
            ),
            Err(TransformTailDeltaError::OutputTooSmall)
        );

        let copy_first = [TransformTailRecord {
            literal_count: 0,
            copy_count: 1,
            back_distance: 1,
        }];
        assert_eq!(
            transform_tail_delta3_into(
                &mut out,
                TransformTailDelta3Spec {
                    output_stride: 3,
                    block_index: 0,
                    out_offset: 0,
                    records: &copy_first,
                    matches: &[0],
                    source0: &[],
                    source1: &[],
                    source2: &[],
                },
            ),
            Err(TransformTailDeltaError::CopyBeforeOutput)
        );
    }

    fn hex_bytes(s: &str) -> Vec<u8> {
        let h: Vec<u8> = s.bytes().filter(|b| b.is_ascii_hexdigit()).collect();
        h.chunks(2)
            .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
            .collect()
    }

    fn hex_u32_words(s: &str) -> Vec<u32> {
        hex_bytes(s)
            .chunks_exact(4)
            .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect()
    }

    fn sparse_payload(len: usize, chunks: &[(usize, &str)]) -> Vec<u8> {
        let mut payload = vec![0u8; len];
        for &(offset, hex) in chunks {
            let bytes = hex_bytes(hex);
            payload[offset..offset + bytes.len()].copy_from_slice(&bytes);
        }
        payload
    }

    fn hex_u16s(s: &str) -> Vec<u16> {
        let bytes = hex_bytes(s);
        assert_eq!(bytes.len() % 2, 0);
        bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect()
    }

    fn hex_width_records(s: &str) -> Vec<[u32; 2]> {
        let bytes = hex_bytes(s);
        assert_eq!(bytes.len() % 8, 0);
        bytes
            .chunks_exact(8)
            .map(|c| {
                [
                    u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                    u32::from_le_bytes([c[4], c[5], c[6], c[7]]),
                ]
            })
            .collect()
    }

    /// Bear freq call #0 (`trace_freq_all.py` / `freq_golden.py` bear_call0): slow
    /// path only, M=512 → [95,408,7,1]+rem=1. Rules out `~nbits` after `nbits+=1`
    /// and wrong ptr step without `^7`.
    #[test]
    fn rans_read_freqs_bear_call0() {
        const WIN: [u8; 13] = [
            0x88, 0xff, 0x4f, 0x53, 0x86, 0x0f, 0x38, 0x11, 0x17, 0xed, 0xa7, 0x42, 0x42,
        ];
        let r = rans_read_freqs(
            &WIN,
            RansFreqReader {
                ptr: 5,
                acc: 0x8d82720662f204b8,
                bitpos: 62,
            },
            RansFreqParams {
                count: 4,
                w3_init: 7,
                w4: 15,
                m: 512,
                initfreq: 102,
            },
        );
        assert_eq!(r.freqs, [95, 408, 7, 1]);
        assert_eq!(r.rem, 1);
        assert_eq!(r.reader.ptr, 0);
        assert_eq!(r.reader.acc, 0x204b9090a9fb45c0);
        assert_eq!(r.reader.bitpos, 58);
    }

    /// Bear freq call #2 (`freq_golden.py` bear_call2_allpaths): exercises slow,
    /// run-length (`0x110e890`/`0x110e8e8`), and run-body (`0x110e900`) paths.
    #[test]
    fn rans_read_freqs_bear_call2_allpaths() {
        const WIN: [u8; 14] = [
            0xd9, 0x5d, 0xec, 0x75, 0x69, 0x8b, 0x11, 0x68, 0x2b, 0x87, 0xcb, 0x8b, 0x88, 0xff,
        ];
        let r = rans_read_freqs(
            &WIN,
            RansFreqReader {
                ptr: 6,
                acc: 0x45c44e03e194d3f8,
                bitpos: 58,
            },
            RansFreqParams {
                count: 5,
                w3_init: 7,
                w4: 15,
                m: 512,
                initfreq: 85,
            },
        );
        assert_eq!(r.freqs, [9, 496, 3, 2, 1]);
        assert_eq!(r.rem, 1);
        assert_eq!(r.reader.ptr, 0);
        assert_eq!(r.reader.acc, 0x34fff888bcb872b6);
        assert_eq!(r.reader.bitpos, 60);
    }

    /// Animal_Bass call #21 (`freq_golden.py` bass_call21): second model, M=128.
    #[test]
    fn rans_read_freqs_bass_call21() {
        const WIN: [u8; 10] = [0xd2, 0xa7, 0x9c, 0x93, 0xcf, 0xb3, 0xe0, 0x9b, 0x61, 0x8b];
        let r = rans_read_freqs(
            &WIN,
            RansFreqReader {
                ptr: 2,
                acc: 0x531000ed76178088,
                bitpos: 60,
            },
            RansFreqParams {
                count: 3,
                w3_init: 5,
                w4: 15,
                m: 128,
                initfreq: 32,
            },
        );
        assert_eq!(r.freqs, [6, 118, 3]);
        assert_eq!(r.rem, 1);
        assert_eq!(r.reader.ptr, 0);
        assert_eq!(r.reader.acc, 0x76178088b619b000);
        assert_eq!(r.reader.bitpos, 44);
    }

    /// The raw window primitive copies exactly `srcsize` bytes after the var-int.
    #[test]
    fn raw_window_copies_srcsize_bytes() {
        // srcsize var-int 0x04, then 6 payload bytes; only 4 are the window.
        let payload = [0x04u8, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
        let (out, pos) = decode_raw_window(&payload, 0);
        assert_eq!(out, [0xaa, 0xbb, 0xcc, 0xdd]);
        assert_eq!(pos, 5);
    }
}
