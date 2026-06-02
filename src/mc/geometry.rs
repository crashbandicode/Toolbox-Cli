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
//! Threading the reverse-A reader through the per-window raw/zstd flag bits and
//! the kernel (`0x10fa980`), and the custom **vertex** byte-group coder
//! (`0x10fb2e0` + the transpose/delta transform). Tracked in
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
/// updated cursor positions plus the parsed scalars. (The table *values* drive
/// the vertex coder; for the index path only the cursor advance matters, and is
/// validated byte-exact against the decoder.)
#[derive(Debug, Clone, Copy)]
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
}

/// Port of state 0's continuation (`0x10f8cc8..0x10f9028`): read the forward
/// var-int `w8`, then walk the canonical-Huffman table description (a 4-bit
/// symbol count from reverse-A, then `symbols` entries of 11 reverse-A bits +
/// conditional forward byte-pairs), then one direction bit. Returns the advanced
/// cursors (the table contents themselves are not materialised here — that is
/// the vertex coder's job).
///
/// `rev_ptr` is the reverse-A pointer (payload offset; the decoder seeds it at
/// `sub_a_size - 8`), `rev_acc`/`rev_bitpos` its accumulator/bit position.
pub fn state0_table_builder(
    payload: &[u8],
    fwd: usize,
    rev_ptr: usize,
    rev_acc: u64,
    rev_bitpos: u32,
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

    if symbols != 0 {
        x12 += 2; // initial 2 forward bytes (0x10f8d38)
        for _ in 0..symbols {
            x14 |= u64_le(payload, x11) >> w10;
            let w19 = x14 >> 0x3e; // top 2 bits
            if (w19 >> 1) & 1 != 0 {
                x12 += 2; // "long" symbol: 2 forward bytes
            }
            let pre = w10;
            w10 = (w10 | 0x38).wrapping_sub(11);
            x14 <<= 11;
            x11 = x11.wrapping_sub(((pre >> 3) ^ 7) as usize);
        }
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

    let tb = state0_table_builder(payload, fwd.pos, sub_a.wrapping_sub(8), 0, 0);

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
        let tb = state0_table_builder(payload, fwd.pos, sub_a - 8, 0, 0);
        assert_eq!(tb.fwd, 15, "forward cursor after table builder");
        assert_eq!(tb.rev_ptr, sub_a - 8 - 18, "reverse-A ptr (P+32807)");
        assert_eq!(tb.rev_bitpos, 50, "reverse-A bit position");
        assert_eq!((tb.w8, tb.symbols, tb.dir_bit), (3327, 8, 1));

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
