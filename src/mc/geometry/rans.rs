use super::transport::u64_le;
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

/// Errors from the byte-output four-lane rANS decoder (`0x110dfa0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RansInitError {
    /// `prod < 4` hits the scalar path (`0x110e140`) — not yet traced for commit.
    ProdTooSmall,
    /// `step`/`sym` length must equal `1 << table.log`.
    TableSizeMismatch,
    /// A zero stride would repeatedly overwrite the same output slot.
    ZeroStride,
    /// `count * stride` overflowed or does not fit the caller-provided output.
    OutputTooSmall,
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
/// `count < 4` (`0x110e140`). The `count & 3` tail at `0x110e128` is observed
/// and covered.
pub fn rans_init_states_with_cursor(
    table: &RansDecodeTable,
    stream: &[u8],
    prod: u32,
    stride: usize,
    state: &mut RansStateBuffer,
    cursor: &mut RansStreamCursor,
) -> Result<RansInitResult, RansInitError> {
    rans_byte_decode_core(
        None,
        table.log,
        &table.step,
        &table.sym,
        stream,
        prod,
        stride,
        state,
        cursor,
    )
}

/// Decode byte symbols with the generic `0x110dfa0` primitive and write them to
/// `out[i * stride]`.
pub fn rans_decode_bytes_into_with_cursor(
    out: &mut [u8],
    table: &RansDecodeTable,
    stream: &[u8],
    count: u32,
    stride: usize,
    state: &mut RansStateBuffer,
    cursor: &mut RansStreamCursor,
) -> Result<RansInitResult, RansInitError> {
    rans_byte_decode_core(
        Some(out),
        table.log,
        &table.step,
        &table.sym,
        stream,
        count,
        stride,
        state,
        cursor,
    )
}

#[allow(clippy::too_many_arguments)]
fn rans_byte_decode_core(
    mut out: Option<&mut [u8]>,
    log: u32,
    step: &[u32],
    sym: &[u16],
    stream: &[u8],
    count: u32,
    stride: usize,
    state: &mut RansStateBuffer,
    cursor: &mut RansStreamCursor,
) -> Result<RansInitResult, RansInitError> {
    let m = 1usize
        .checked_shl(log)
        .ok_or(RansInitError::TableSizeMismatch)?;
    if step.len() != m || sym.len() != m {
        return Err(RansInitError::TableSizeMismatch);
    }
    if count < 4 {
        return Err(RansInitError::ProdTooSmall);
    }
    if stride == 0 {
        return Err(RansInitError::ZeroStride);
    }
    if let Some(buf) = out.as_deref() {
        let min_len = (count as usize)
            .checked_mul(stride)
            .ok_or(RansInitError::OutputTooSmall)?;
        if buf.len() < min_len {
            return Err(RansInitError::OutputTooSmall);
        }
    }

    let start_offset = cursor.offset;
    if state.flag & 0xf != 0xf {
        load_cold_rans_states(stream, state, cursor)?;
    }

    let mask = (1u64 << log) - 1;

    // The disassembly decodes groups of four lanes and then a scalar tail. A
    // single round-robin loop is equivalent because a lane is not read again
    // until the next four-symbol group.
    for output_index in 0..count as usize {
        let lane = output_index & 3;
        let lane_state = &mut state.states[lane];
        let idx = (*lane_state & mask) as usize;
        let entry = step[idx];
        let shifted = *lane_state >> log;
        if let Some(buf) = out.as_deref_mut() {
            buf[output_index * stride] = sym[idx] as u8;
        }
        *lane_state = shifted * (entry >> 16) as u64 + (entry & 0xffff) as u64;
        if *lane_state >> 31 == 0 {
            let word = read_stream_u32(stream, cursor)?;
            *lane_state = (*lane_state << 32) | word as u64;
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

/// Decode byte symbols with a warm state buffer.
pub fn rans_decode_bytes_into(
    out: &mut [u8],
    table: &RansDecodeTable,
    stream: &[u8],
    count: u32,
    stride: usize,
    states_in: [u64; 4],
) -> Result<RansInitResult, RansInitError> {
    let mut state = RansStateBuffer::warm(states_in);
    let mut cursor = RansStreamCursor::default();
    rans_decode_bytes_into_with_cursor(out, table, stream, count, stride, &mut state, &mut cursor)
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

/// Errors from the byte segment RLE fill helper (`0x110f800`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RansByteRleFillError {
    /// A zero stride would repeatedly overwrite the same output slot.
    ZeroStride,
    /// `count * stride` overflowed or does not fit the caller-provided output.
    OutputTooSmall,
}

/// Fill `count` bytes at `out[i * stride]` with `value` (`0x110f800`).
///
/// The byte sibling of `0x110f930` stores with `strb`, so only the low byte of
/// the descriptor value reaches memory. `stride` is in byte slots here; the
/// disassembly increments `x0` by `sxtw x8,w3` without the u16 `lsl`.
pub fn rans_rle_fill_bytes(
    out: &mut [u8],
    value: u8,
    count: usize,
    stride: usize,
) -> Result<(), RansByteRleFillError> {
    if stride == 0 {
        return Err(RansByteRleFillError::ZeroStride);
    }
    let min_len = count
        .checked_mul(stride)
        .ok_or(RansByteRleFillError::OutputTooSmall)?;
    if out.len() < min_len {
        return Err(RansByteRleFillError::OutputTooSmall);
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
            // 0x110df28 selects x14 (`bits << 9`) on the low-count path; using
            // x10 from the earlier `bits << 5` stage reads mode/log four bits early.
            shifted = high_bits;
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

/// Inputs for `0x110eb50`, the byte-output mode-1 3-lane segment decoder.
pub struct RansByteThreeLaneDecodeSpec<'a> {
    /// Number of bytes this call writes (`w2`).
    pub count: usize,
    /// `log2` table selector width (`w5`).
    pub log: u32,
    /// Output spacing in byte slots (`w1`).
    pub stride: usize,
    /// Packed decode table. Low 16 bits are the byte symbol; high 16 bits are bits consumed.
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

/// Decode byte mode-1 symbols with the 3-lane bit decoder (`0x110eb50`).
///
/// This is the byte-output sibling of `0x110ef70`: the reader reload order and
/// table entry format are the same, but symbols are stored with `strb`
/// (`0x110ec50`, `0x110ec78`, `0x110ec84`, and the tail at `0x110ef0c`).
pub fn rans_three_lane_decode_bytes_into(
    out: &mut [u8],
    spec: RansByteThreeLaneDecodeSpec<'_>,
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
                    take_three_lane_symbol(&mut spec.readers[lane], spec.table, spec.log)? as u8;
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
                take_three_lane_symbol(&mut spec.readers[lane], spec.table, spec.log)? as u8;
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

/// Errors from the byte segment dispatch wrapper (`0x110dd80`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RansSegmentDispatchBytesError {
    /// Mode 1 requires the three reverse-reader states from `x5`.
    MissingThreeLaneReaders,
    /// The observed dispatch modes are 0, 1, and 2.
    UnknownMode(u32),
    /// `count` must fit the `w2` argument consumed by `0x110dfa0`.
    CountTooLarge(usize),
    /// Mode 0 byte rANS decode rejected the segment.
    Decode(RansInitError),
    /// Mode 1 byte three-lane decode rejected the segment.
    ThreeLane(RansThreeLaneDecodeError),
    /// Mode 2 byte RLE fill rejected the segment.
    Rle(RansByteRleFillError),
}

/// Inputs for one byte segment descriptor dispatched by `0x110dd80`.
pub struct RansSegmentDispatchBytesSpec<'a> {
    /// Descriptor mode at `[x3]`: 0 = byte rANS, 1 = `0x110eb50`, 2 = RLE fill.
    pub mode: u32,
    /// rANS table log at `[x3+4]`.
    pub log: u32,
    /// RLE value at `[x3+8]` for mode 2.
    pub value: u32,
    /// Number of bytes this dispatch writes (`w2`).
    pub count: usize,
    /// Output spacing in byte slots (`w1`).
    pub stride: usize,
    /// Four byte-rANS states at `[x3+0x10..0x30]` for mode 0. Updated in place.
    pub state: &'a mut RansStateBuffer,
    /// rANS step table at `[x3+0x80]` for mode 0.
    pub step: &'a [u32],
    /// rANS symbol table at `[x3+0x2080]` for mode 0.
    pub sym: &'a [u16],
    /// Forward renorm bytes for mode 0.
    pub stream: &'a [u8],
    /// Payload bytes for mode 1 reader loads.
    pub payload: &'a [u8],
    /// Shared forward stream cursor (`[x4+12]`) for mode 0.
    pub cursor: &'a mut RansStreamCursor,
    /// Three mode-1 reader states from `x5`, when dispatching mode 1.
    pub three_lane_readers: Option<&'a mut [RansThreeLaneReader; 3]>,
}

/// Dispatch one built byte segment (`0x110dd80`).
///
/// Mode 0 calls the same byte-output rANS primitive as `0x110dfa0`, updating the
/// descriptor states and the shared stream cursor. Mode 1 maps to the byte
/// three-lane decoder at `0x110eb50`. Mode 2 maps to `0x110f800`, a byte
/// strided fill.
pub fn rans_segment_dispatch_bytes_into(
    out: &mut [u8],
    spec: RansSegmentDispatchBytesSpec<'_>,
) -> Result<usize, RansSegmentDispatchBytesError> {
    match spec.mode {
        0 => {
            let count = u32::try_from(spec.count)
                .map_err(|_| RansSegmentDispatchBytesError::CountTooLarge(spec.count))?;
            let result = rans_byte_decode_core(
                Some(out),
                spec.log,
                spec.step,
                spec.sym,
                spec.stream,
                count,
                spec.stride,
                spec.state,
                spec.cursor,
            )
            .map_err(RansSegmentDispatchBytesError::Decode)?;
            Ok(result.stream_used)
        }
        1 => {
            let readers = spec
                .three_lane_readers
                .ok_or(RansSegmentDispatchBytesError::MissingThreeLaneReaders)?;
            rans_three_lane_decode_bytes_into(
                out,
                RansByteThreeLaneDecodeSpec {
                    count: spec.count,
                    log: spec.log,
                    stride: spec.stride,
                    table: spec.step,
                    readers,
                    payload: spec.payload,
                },
            )
            .map_err(RansSegmentDispatchBytesError::ThreeLane)?;
            Ok(0)
        }
        2 => {
            rans_rle_fill_bytes(out, spec.value as u8, spec.count, spec.stride)
                .map_err(RansSegmentDispatchBytesError::Rle)?;
            Ok(0)
        }
        mode => Err(RansSegmentDispatchBytesError::UnknownMode(mode)),
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
                    remaining_after_run = 0;
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
                    // 0x110e8e8 clears the saved post-run count; otherwise a
                    // previous run can leak into dense mode-0 frequency tables.
                    remaining_after_run = 0;
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

/// Inputs for the byte segment loop (`0x110dae0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RansByteSegmentLoopSpec<'a> {
    /// Number of bytes to write (`w1`).
    pub byte_count: usize,
    /// Interleaved lane count / dispatch stride (`w2`).
    pub lanes: usize,
    /// Segment run granularity as `log2(segment_size)` (`w3`).
    pub segment_log: u32,
    /// Payload bytes addressed by all reverse readers and byte dispatch streams.
    pub payload: &'a [u8],
}

/// Errors from the byte segment loop (`0x110dae0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RansByteSegmentLoopError {
    /// `w2 == 0` would make the dispatch stride zero.
    ZeroLaneCount,
    /// The observed caller keeps this at or below 10; larger values risk overflow.
    UnsupportedSegmentLog(u32),
    /// The byte count must divide evenly into the interleaved lanes.
    UnevenLaneBytes { bytes: usize, lanes: usize },
    /// Caller output must include the logical bytes plus `lanes-1` padding bytes.
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
    /// Mode 2 has not been observed inside `0x110dae0`; guard until captured.
    UnobservedMode2Segment,
    /// The byte dispatch wrapper rejected the segment.
    Dispatch(RansSegmentDispatchBytesError),
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

fn read_byte_segment_loop_run_code(
    payload: &[u8],
    reader: &mut RansFreqReader,
) -> Result<usize, RansByteSegmentLoopError> {
    let bitpos = reader.bitpos;
    let ptr_step = ((bitpos >> 3) ^ 7) as usize;
    let ptr = reader
        .ptr
        .checked_sub(ptr_step)
        .ok_or(RansByteSegmentLoopError::PayloadTooSmall)?;
    let bits = (checked_header_u64_le(payload, reader.ptr)
        .map_err(|_| RansByteSegmentLoopError::PayloadTooSmall)?
        >> (bitpos & 63))
        | reader.acc;
    let run_bits = 1 + 2 * clz64(bits);
    if run_bits > 64 {
        return Err(RansByteSegmentLoopError::RunCodeTooLong);
    }
    let run = (bits >> (64 - run_bits)) as usize;
    reader.ptr = ptr;
    reader.acc = if run_bits == 64 { 0 } else { bits << run_bits };
    reader.bitpos = (bitpos | 0x38).wrapping_sub(run_bits);
    Ok(run)
}

pub(super) fn freq_reader_from_three(reader: RansThreeLaneReader) -> RansFreqReader {
    RansFreqReader {
        ptr: reader.ptr,
        acc: reader.acc,
        bitpos: reader.bitpos,
    }
}

pub(super) fn three_reader_from_freq(reader: RansFreqReader) -> RansThreeLaneReader {
    RansThreeLaneReader {
        ptr: reader.ptr,
        acc: reader.acc,
        bitpos: reader.bitpos,
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

/// Decode one `0x110dae0` byte segment loop into an interleaved byte buffer.
///
/// This is the byte-output sibling of `0x110dc30`: it uses the same descriptor
/// builder and CLZ-prefixed run codes, but dispatches through `0x110dd80` and
/// computes output offsets in byte slots (`add x0,x23,w9,sxtw` at
/// `0x110dbb8`) rather than u16 slots.
pub fn rans_segment_loop_bytes_into(
    out: &mut [u8],
    context: &mut RansSegmentLoopContext,
    spec: RansByteSegmentLoopSpec<'_>,
) -> Result<usize, RansByteSegmentLoopError> {
    if spec.lanes == 0 {
        return Err(RansByteSegmentLoopError::ZeroLaneCount);
    }
    if spec.segment_log > 30 {
        return Err(RansByteSegmentLoopError::UnsupportedSegmentLog(
            spec.segment_log,
        ));
    }
    if !spec.byte_count.is_multiple_of(spec.lanes) {
        return Err(RansByteSegmentLoopError::UnevenLaneBytes {
            bytes: spec.byte_count,
            lanes: spec.lanes,
        });
    }
    let padded_bytes = spec
        .byte_count
        .checked_add(spec.lanes - 1)
        .ok_or(RansByteSegmentLoopError::OutputTooSmall)?;
    if out.len() < padded_bytes {
        return Err(RansByteSegmentLoopError::OutputTooSmall);
    }

    let bytes_per_lane = spec.byte_count / spec.lanes;
    let segment_size = 1usize << spec.segment_log;
    let segment_mask = segment_size - 1;
    let mut lane = 0usize;
    let mut lane_offset = 0usize;
    let mut dispatch_count = 0usize;

    while lane < spec.lanes {
        let descriptor = rans_build_segment_descriptor(spec.payload, context.reader)
            .map_err(RansByteSegmentLoopError::Descriptor)?;
        context.reader = descriptor.reader;
        let mut run_segments = read_byte_segment_loop_run_code(spec.payload, &mut context.reader)?;

        loop {
            let remaining = bytes_per_lane
                .checked_sub(lane_offset)
                .ok_or(RansByteSegmentLoopError::RunCountOverflow)?;
            let run_bytes = run_segments
                .checked_mul(segment_size)
                .ok_or(RansByteSegmentLoopError::RunCountOverflow)?;
            let finish_segments = ceil_div_segment(remaining, segment_mask, spec.segment_log)
                .ok_or(RansByteSegmentLoopError::RunCountOverflow)?;
            let finishes_lane = run_segments >= finish_segments;
            let count = if finishes_lane { remaining } else { run_bytes };
            let out_start = lane_offset
                .checked_mul(spec.lanes)
                .and_then(|v| v.checked_add(lane))
                .ok_or(RansByteSegmentLoopError::OutputTooSmall)?;
            let dispatch_len = count
                .checked_mul(spec.lanes)
                .ok_or(RansByteSegmentLoopError::OutputTooSmall)?;
            let out_end = out_start
                .checked_add(dispatch_len)
                .ok_or(RansByteSegmentLoopError::OutputTooSmall)?;
            let out_window = out
                .get_mut(out_start..out_end)
                .ok_or(RansByteSegmentLoopError::OutputTooSmall)?;

            match descriptor.mode {
                0 => {
                    let mut cursor = RansStreamCursor {
                        offset: context.stream_pos,
                    };
                    rans_segment_dispatch_bytes_into(
                        out_window,
                        RansSegmentDispatchBytesSpec {
                            mode: descriptor.mode,
                            log: descriptor.log,
                            value: descriptor.value as u32,
                            count,
                            stride: spec.lanes,
                            state: &mut context.state,
                            step: &descriptor.step,
                            sym: &descriptor.sym,
                            stream: spec.payload,
                            payload: spec.payload,
                            cursor: &mut cursor,
                            three_lane_readers: None,
                        },
                    )
                    .map_err(RansByteSegmentLoopError::Dispatch)?;
                    context.stream_pos = cursor.offset;
                }
                1 => {
                    let mut readers = [
                        three_reader_from_freq(context.reader),
                        context.mode1_extra_readers[0],
                        context.mode1_extra_readers[1],
                    ];
                    let mut cursor = RansStreamCursor::default();
                    rans_segment_dispatch_bytes_into(
                        out_window,
                        RansSegmentDispatchBytesSpec {
                            mode: descriptor.mode,
                            log: descriptor.log,
                            value: descriptor.value as u32,
                            count,
                            stride: spec.lanes,
                            state: &mut context.state,
                            step: &descriptor.step,
                            sym: &descriptor.sym,
                            stream: &[],
                            payload: spec.payload,
                            cursor: &mut cursor,
                            three_lane_readers: Some(&mut readers),
                        },
                    )
                    .map_err(RansByteSegmentLoopError::Dispatch)?;
                    context.reader = freq_reader_from_three(readers[0]);
                    context.mode1_extra_readers = [readers[1], readers[2]];
                }
                2 => return Err(RansByteSegmentLoopError::UnobservedMode2Segment),
                mode => {
                    return Err(RansByteSegmentLoopError::Dispatch(
                        RansSegmentDispatchBytesError::UnknownMode(mode),
                    ));
                }
            }

            dispatch_count += 1;
            let consumed_segments = ceil_div_segment(count, segment_mask, spec.segment_log)
                .ok_or(RansByteSegmentLoopError::RunCountOverflow)?;
            if finishes_lane {
                lane += 1;
                lane_offset = 0;
            } else {
                lane_offset = lane_offset
                    .checked_add(run_bytes)
                    .ok_or(RansByteSegmentLoopError::RunCountOverflow)?;
            }
            run_segments = run_segments
                .checked_sub(consumed_segments)
                .ok_or(RansByteSegmentLoopError::RunCountOverflow)?;
            if run_segments == 0 {
                break;
            }
        }
    }

    Ok(dispatch_count)
}
