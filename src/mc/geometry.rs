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
//! the rANS table build (`0x110de80` spread + state init), and the kernel
//! (`0x10fa980`) + vertex byte-group transform (`0x10fb2e0`). Tracked in
//! `local-assets/re/FINDINGS.md`.

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
/// Validated byte-exact against the decoder's dumped I/O. The table build,
/// 3-lane variant (`0x110ef70`), and RLE fill (`0x110f930`) remain.
pub fn rans_decode(
    count: usize,
    log: u32,
    stride: usize,
    step: &[u32],
    sym: &[u16],
    init_states: [u64; 4],
    stream: &[u8],
) -> Vec<u16> {
    let mask = (1u64 << log) - 1;
    let mut states = init_states;
    let mut spos = 0usize;
    let mut out = vec![0u16; count];

    let decode_lane = |st: u64, spos: &mut usize| -> (u16, u64) {
        let idx = (st & mask) as usize;
        let s = sym[idx];
        let e = step[idx];
        let mut ns = (st >> log) * (e >> 16) as u64 + (e & 0xffff) as u64;
        if ns >> 31 == 0 {
            if let Some(b) = stream.get(*spos..*spos + 4) {
                ns = (ns << 32) | u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u64;
                *spos += 4;
            }
        }
        (s, ns)
    };

    let iters = count / 4;
    for it in 0..iters {
        let base = it * 4 * stride;
        for lane in 0..4 {
            let (s, ns) = decode_lane(states[lane], &mut spos);
            out[base + lane * stride] = s;
            states[lane] = ns;
        }
    }
    for k in 0..(count & 3) {
        let (s, ns) = decode_lane(states[0], &mut spos);
        out[iters * 4 * stride + k * stride] = s;
        states[0] = ns;
    }
    out
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
    /// Remaining probability mass -- the `(count+1)`-th symbol's frequency (`w9`).
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
                // 0x110e7e4 -- `w7 = 1<<width` primes the next slow-path symbol.
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
                // `w18 = val - (1<<width) + prime` (`0x110e814`-`0x110e834`).
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
                    // 0x110e8e8 -- `w18 = 0`; `w1` (remaining) is unchanged.
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
    let map_m = |e: meshopt::MeshoptError| super::McError::MeshFraming(format!("index decode: {e}"));

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
        let (out, cu, du) =
            meshopt::decode_index_buffer_split_used(count, 2, &code[code_off..], &data[data_off..], 0)
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
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/mc/{name}"));
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
        let section = crate::mc::read_mesh_section(&mc).unwrap().expect("mesh section");
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
            [0x0c00100b, 0x0c000803, 0x0c000803, 0x10000a13, 0x1000100a, 0x1000100a, 0x10000803, 0x10000801]
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

    /// The rANS decoder reproduces a real decoded symbol stream (the first
    /// vertex-coder rANS call of Animal_Bear), validated against the emulator.
    #[test]
    fn rans_decode_matches_oracle() {
        // Decode table (step[64] = (freq<<16)|low, sym[64] spread map), log2(M)=6.
        const STEP: [u32; 64] = [
            327680, 327681, 327682, 327683, 327684, 65536, 65536, 65536, 65536, 65536, 65536,
            196608, 196609, 196610, 393216, 393217, 393218, 393219, 393220, 393221, 851968, 851969,
            851970, 851971, 851972, 851973, 851974, 851975, 851976, 851977, 851978, 851979, 851980,
            1507328, 1507329, 1507330, 1507331, 1507332, 1507333, 1507334, 1507335, 1507336, 1507337,
            1507338, 1507339, 1507340, 1507341, 1507342, 1507343, 1507344, 1507345, 1507346, 1507347,
            1507348, 1507349, 1507350, 524288, 524289, 524290, 524291, 524292, 524293, 524294, 524295,
        ];
        const SYM: [u16; 64] = [
            0, 0, 0, 0, 0, 1, 2, 4, 6, 7, 9, 10, 10, 10, 11, 11, 11, 11, 11, 11, 12, 12, 12, 12, 12,
            12, 12, 12, 12, 12, 12, 12, 12, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
            13, 13, 13, 13, 13, 13, 13, 13, 13, 14, 14, 14, 14, 14, 14, 14, 14,
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
        let out = rans_decode(228, 6, 1, &STEP, &SYM, states, &stream);
        assert_eq!(out.len(), 228);
        assert_eq!(
            &out[..24],
            &[4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 10, 4, 2, 12, 13, 12, 10, 13, 10]
        );
        assert_eq!(&out[220..], &[14, 13, 13, 14, 14, 13, 14, 13]);
        assert_eq!(out.iter().map(|&s| s as u32).sum::<u32>(), 2565);
    }

    fn hex_bytes(s: &str) -> Vec<u8> {
        let h: Vec<u8> = s.bytes().filter(|b| b.is_ascii_hexdigit()).collect();
        h.chunks(2)
            .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
            .collect()
    }

    /// Bear freq call #0 (`trace_freq_all.py` / `freq_golden.py` bear_call0): slow
    /// path only, M=512 -> [95,408,7,1]+rem=1. Rules out `~nbits` after `nbits+=1`
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
