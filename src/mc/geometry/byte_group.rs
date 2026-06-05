use crate::zstd_pure;

use super::rans::*;
/// Mutable `0x110d7f0` byte-group reader state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteGroupReadState {
    /// Primary reverse selector/descriptor reader at `x6+0`.
    pub reader: RansThreeLaneReader,
    /// Extra mode-1 readers at `x6+0x18` and `x6+0x30`.
    pub mode1_extra_readers: [RansThreeLaneReader; 2],
    /// Payload-relative forward byte stream pointer at `x6+0x48`.
    pub stream_pos: usize,
    /// rANS state buffer living in the descriptor workspace used by selector 0.
    pub segment_state: RansStateBuffer,
    /// Selector-2 zstd history window rooted at the caller's `[x0+8]` buffer.
    pub selector2_history: Vec<u8>,
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
    /// Reserved selector value.
    UnportedSelector(u8),
    /// Selectors 0, 1, and 2 have only observed byte and u16 element shifts.
    UnsupportedElementShift(u32),
    /// Selector 1's multi-window split at 0x80000 groups has not been observed.
    UnobservedSelector1LargeWindow { group_symbols: usize },
    /// Selector 1 descriptor build rejected the stream.
    SegmentDescriptor(RansSegmentDescriptorBuildError),
    /// Selector 1 byte dispatch rejected the stream.
    SegmentDispatchBytes(RansSegmentDispatchBytesError),
    /// Selector 1 u16 dispatch rejected the stream.
    SegmentDispatch(RansSegmentDispatchError),
    /// Selector 2's multi-window loop above 0x20000 output bytes is unobserved.
    UnobservedSelector2MultiWindow { byte_count: usize },
    /// Selector 2's 0x80000 history wrap is unobserved.
    UnobservedSelector2HistoryWrap {
        history_len: usize,
        byte_count: usize,
    },
    /// Selector 2 raw-copy windows have not been observed in current fixtures.
    UnobservedSelector2RawWindow,
    /// Selector 2's forward window-size varint exceeded the u32-shaped encoding.
    Selector2VarintTooLong,
    /// Selector 2 zstd block decode failed.
    Selector2ZstdDecode,
    /// Selector 2 zstd block regenerated a different size than the caller asked.
    Selector2OutputSizeMismatch { expected: usize, actual: usize },
    /// Selector 0 byte segment loop rejected the stream.
    ByteSegmentLoop(RansByteSegmentLoopError),
    /// Selector 0 u16 segment loop rejected the stream.
    SegmentLoop(RansSegmentLoopError),
}

pub(super) fn checked_byte_group_u64_le(buf: &[u8], ptr: usize) -> Result<u64, ByteGroupReadError> {
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

fn byte_group_segment_log(group_bytes: usize) -> Result<u32, ByteGroupReadError> {
    let rounded = group_bytes
        .checked_add(15)
        .ok_or(ByteGroupReadError::OutputSizeOverflow)?
        >> 4;
    let ceil_log = if rounded <= 1 {
        0
    } else {
        usize::BITS - (rounded - 1).leading_zeros()
    };
    Ok(if group_bytes >= 0x4000 {
        10
    } else {
        ceil_log.max(4)
    })
}

fn byte_group_context_from_state(state: &ByteGroupReadState) -> RansSegmentLoopContext {
    RansSegmentLoopContext {
        reader: freq_reader_from_three(state.reader),
        mode1_extra_readers: state.mode1_extra_readers,
        stream_pos: state.stream_pos,
        state: state.segment_state,
    }
}

fn write_byte_group_context(state: &mut ByteGroupReadState, context: RansSegmentLoopContext) {
    state.reader = three_reader_from_freq(context.reader);
    state.mode1_extra_readers = context.mode1_extra_readers;
    state.stream_pos = context.stream_pos;
    state.segment_state = context.state;
}

fn dispatch_selector1_bytes(
    out: &mut [u8],
    context: &mut RansSegmentLoopContext,
    descriptor: &RansBuiltSegmentDescriptor,
    payload: &[u8],
) -> Result<(), ByteGroupReadError> {
    match descriptor.mode {
        0 => {
            let mut cursor = RansStreamCursor {
                offset: context.stream_pos,
            };
            rans_segment_dispatch_bytes_into(
                out,
                RansSegmentDispatchBytesSpec {
                    mode: descriptor.mode,
                    log: descriptor.log,
                    value: descriptor.value as u32,
                    count: out.len(),
                    stride: 1,
                    state: &mut context.state,
                    step: &descriptor.step,
                    sym: &descriptor.sym,
                    stream: payload,
                    payload,
                    cursor: &mut cursor,
                    three_lane_readers: None,
                },
            )
            .map_err(ByteGroupReadError::SegmentDispatchBytes)?;
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
                out,
                RansSegmentDispatchBytesSpec {
                    mode: descriptor.mode,
                    log: descriptor.log,
                    value: descriptor.value as u32,
                    count: out.len(),
                    stride: 1,
                    state: &mut context.state,
                    step: &descriptor.step,
                    sym: &descriptor.sym,
                    stream: &[],
                    payload,
                    cursor: &mut cursor,
                    three_lane_readers: Some(&mut readers),
                },
            )
            .map_err(ByteGroupReadError::SegmentDispatchBytes)?;
            context.reader = freq_reader_from_three(readers[0]);
            context.mode1_extra_readers = [readers[1], readers[2]];
        }
        2 => {
            let mut cursor = RansStreamCursor::default();
            rans_segment_dispatch_bytes_into(
                out,
                RansSegmentDispatchBytesSpec {
                    mode: descriptor.mode,
                    log: descriptor.log,
                    value: descriptor.value as u32,
                    count: out.len(),
                    stride: 1,
                    state: &mut context.state,
                    step: &descriptor.step,
                    sym: &descriptor.sym,
                    stream: &[],
                    payload,
                    cursor: &mut cursor,
                    three_lane_readers: None,
                },
            )
            .map_err(ByteGroupReadError::SegmentDispatchBytes)?;
        }
        mode => {
            return Err(ByteGroupReadError::SegmentDispatchBytes(
                RansSegmentDispatchBytesError::UnknownMode(mode),
            ));
        }
    }

    Ok(())
}

fn dispatch_selector1_u16(
    out: &mut [u16],
    context: &mut RansSegmentLoopContext,
    descriptor: &RansBuiltSegmentDescriptor,
    payload: &[u8],
) -> Result<(), ByteGroupReadError> {
    match descriptor.mode {
        0 => {
            let stream = payload
                .get(context.stream_pos..)
                .ok_or(ByteGroupReadError::StreamTooShort)?;
            let used = rans_segment_dispatch_into(
                out,
                RansSegmentDispatchSpec {
                    mode: descriptor.mode,
                    log: descriptor.log,
                    value: descriptor.value,
                    count: out.len(),
                    stride: 1,
                    states: &mut context.state.states,
                    step: &descriptor.step,
                    sym: &descriptor.sym,
                    stream,
                    payload,
                    three_lane_readers: None,
                },
            )
            .map_err(ByteGroupReadError::SegmentDispatch)?;
            context.stream_pos = context
                .stream_pos
                .checked_add(used)
                .ok_or(ByteGroupReadError::StreamTooShort)?;
        }
        1 => {
            let mut readers = [
                three_reader_from_freq(context.reader),
                context.mode1_extra_readers[0],
                context.mode1_extra_readers[1],
            ];
            rans_segment_dispatch_into(
                out,
                RansSegmentDispatchSpec {
                    mode: descriptor.mode,
                    log: descriptor.log,
                    value: descriptor.value,
                    count: out.len(),
                    stride: 1,
                    states: &mut context.state.states,
                    step: &descriptor.step,
                    sym: &descriptor.sym,
                    stream: &[],
                    payload,
                    three_lane_readers: Some(&mut readers),
                },
            )
            .map_err(ByteGroupReadError::SegmentDispatch)?;
            context.reader = freq_reader_from_three(readers[0]);
            context.mode1_extra_readers = [readers[1], readers[2]];
        }
        2 => {
            rans_segment_dispatch_into(
                out,
                RansSegmentDispatchSpec {
                    mode: descriptor.mode,
                    log: descriptor.log,
                    value: descriptor.value,
                    count: out.len(),
                    stride: 1,
                    states: &mut context.state.states,
                    step: &descriptor.step,
                    sym: &descriptor.sym,
                    stream: &[],
                    payload,
                    three_lane_readers: None,
                },
            )
            .map_err(ByteGroupReadError::SegmentDispatch)?;
        }
        mode => {
            return Err(ByteGroupReadError::SegmentDispatch(
                RansSegmentDispatchError::UnknownMode(mode),
            ));
        }
    }

    Ok(())
}

fn take_byte_group_window_flag(
    payload: &[u8],
    reader: &mut RansThreeLaneReader,
) -> Result<u8, ByteGroupReadError> {
    let bitpos = reader.bitpos;
    let ptr_step = ((bitpos >> 3) ^ 7) as usize;
    let ptr = reader
        .ptr
        .checked_sub(ptr_step)
        .ok_or(ByteGroupReadError::PayloadTooSmall)?;
    let bits = (checked_byte_group_u64_le(payload, reader.ptr)? >> (bitpos & 63)) | reader.acc;
    let flag = (bits >> 63) as u8;
    reader.ptr = ptr;
    reader.acc = bits << 1;
    reader.bitpos = (bitpos | 0x38).wrapping_sub(1);
    Ok(flag)
}

fn read_selector2_window_size(
    payload: &[u8],
    mut pos: usize,
) -> Result<(usize, usize), ByteGroupReadError> {
    let mut value = 0usize;
    for _ in 0..5 {
        let byte = payload
            .get(pos)
            .copied()
            .ok_or(ByteGroupReadError::StreamTooShort)?;
        pos += 1;
        value = value
            .checked_shl(7)
            .and_then(|v| v.checked_add((byte & 0x7f) as usize))
            .ok_or(ByteGroupReadError::Selector2VarintTooLong)?;
        if byte & 0x80 == 0 {
            return Ok((value, pos));
        }
    }

    Err(ByteGroupReadError::Selector2VarintTooLong)
}

pub(super) fn decode_selector2_zstd_window(
    payload: &[u8],
    stream_pos: usize,
    byte_count: usize,
    history: &[u8],
) -> Result<(Vec<u8>, usize), ByteGroupReadError> {
    let (src_size, src_start) = read_selector2_window_size(payload, stream_pos)?;
    let src_end = src_start
        .checked_add(src_size)
        .ok_or(ByteGroupReadError::StreamTooShort)?;
    let block = payload
        .get(src_start..src_end)
        .ok_or(ByteGroupReadError::StreamTooShort)?;
    let mut state = zstd_pure::block::BlockState {
        out: history.to_vec(),
        dict_len: history.len(),
        max_output: byte_count,
        huff: None,
        seq: zstd_pure::sequences::SeqTables::default(),
        rep: [1, 4, 8],
    };
    state
        .decode_compressed(block)
        .map_err(|_| ByteGroupReadError::Selector2ZstdDecode)?;
    let decoded = state.out.split_off(history.len());
    if decoded.len() != byte_count {
        return Err(ByteGroupReadError::Selector2OutputSizeMismatch {
            expected: byte_count,
            actual: decoded.len(),
        });
    }
    Ok((decoded, src_end))
}

fn append_byte_group_history(
    state: &mut ByteGroupReadState,
    bytes: &[u8],
) -> Result<(), ByteGroupReadError> {
    let history_len = state.selector2_history.len();
    match history_len.checked_add(bytes.len()) {
        Some(end) if end <= 0x80000 => {
            state.selector2_history.extend_from_slice(bytes);
            Ok(())
        }
        _ => Err(ByteGroupReadError::UnobservedSelector2HistoryWrap {
            history_len,
            byte_count: bytes.len(),
        }),
    }
}

/// Read one byte-group stream (`0x110d7f0`).
///
/// The common selector prologue consumes two reverse-reader bits
/// (`0x110d808..0x110d854`). Selector 0 allocates a segment-decoded stream:
/// byte elements (`w2 == 0`) route through `0x110dae0`, while halfword elements
/// (`w2 == 1`) route through `0x110dc30` and are returned little-endian.
/// Selector 1 builds one descriptor via `0x110de80` and dispatches one window
/// through `0x110dd80`/`0x110de00`; the unobserved large-window split remains
/// guarded.
/// Selector 2 follows the observed single zstd-window path through
/// `0x1110cc0`/`0x1110a60`; raw windows, multi-window output, and history wrap
/// remain guarded until captured.
/// Selector 3 returns the current forward stream slice and advances `x6+0x48`
/// by `(w4 * w3) << w2` (`0x110da00..0x110dab8`).
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
        0 => {
            let segment_log = byte_group_segment_log(group_bytes)?;
            let mut context = byte_group_context_from_state(state);
            let bytes = match spec.element_shift {
                0 => {
                    let padded_len = out_len
                        .checked_add(spec.group_stride.saturating_sub(1))
                        .ok_or(ByteGroupReadError::OutputSizeOverflow)?;
                    let mut out = vec![0u8; padded_len];
                    rans_segment_loop_bytes_into(
                        &mut out,
                        &mut context,
                        RansByteSegmentLoopSpec {
                            byte_count: out_len,
                            lanes: spec.group_stride,
                            segment_log,
                            payload: spec.payload,
                        },
                    )
                    .map_err(ByteGroupReadError::ByteSegmentLoop)?;
                    out.truncate(out_len);
                    out
                }
                1 => {
                    let logical_slots = out_len >> 1;
                    let padded_slots = logical_slots
                        .checked_add(spec.group_stride.saturating_sub(1))
                        .ok_or(ByteGroupReadError::OutputSizeOverflow)?;
                    let mut out = vec![0u16; padded_slots];
                    rans_segment_loop_into(
                        &mut out,
                        &mut context,
                        RansSegmentLoopSpec {
                            byte_count: out_len,
                            lanes: spec.group_stride,
                            segment_log,
                            payload: spec.payload,
                        },
                    )
                    .map_err(ByteGroupReadError::SegmentLoop)?;
                    let mut bytes = Vec::with_capacity(out_len);
                    for &symbol in out.iter().take(logical_slots) {
                        bytes.extend_from_slice(&symbol.to_le_bytes());
                    }
                    bytes
                }
                shift => return Err(ByteGroupReadError::UnsupportedElementShift(shift)),
            };
            write_byte_group_context(state, context);
            Ok(ByteGroupRead { selector, bytes })
        }
        1 => {
            if spec.element_shift > 1 {
                return Err(ByteGroupReadError::UnsupportedElementShift(
                    spec.element_shift,
                ));
            }
            if group_bytes >= 0x80000 {
                return Err(ByteGroupReadError::UnobservedSelector1LargeWindow {
                    group_symbols: group_bytes,
                });
            }

            let mut context = byte_group_context_from_state(state);
            let descriptor = rans_build_segment_descriptor(spec.payload, context.reader)
                .map_err(ByteGroupReadError::SegmentDescriptor)?;
            context.reader = descriptor.reader;

            let bytes = if spec.element_shift == 0 {
                let mut out = vec![0u8; group_bytes];
                dispatch_selector1_bytes(&mut out, &mut context, &descriptor, spec.payload)?;
                out
            } else {
                let mut out = vec![0u16; group_bytes];
                dispatch_selector1_u16(&mut out, &mut context, &descriptor, spec.payload)?;
                let mut bytes = Vec::with_capacity(out_len);
                for symbol in out {
                    bytes.extend_from_slice(&symbol.to_le_bytes());
                }
                bytes
            };

            write_byte_group_context(state, context);
            Ok(ByteGroupRead { selector, bytes })
        }
        2 => {
            if spec.element_shift > 1 {
                return Err(ByteGroupReadError::UnsupportedElementShift(
                    spec.element_shift,
                ));
            }
            if out_len > 0x20000 {
                return Err(ByteGroupReadError::UnobservedSelector2MultiWindow {
                    byte_count: out_len,
                });
            }

            let history_len = state.selector2_history.len();
            match history_len.checked_add(out_len) {
                Some(end) if end <= 0x80000 => {}
                _ => {
                    return Err(ByteGroupReadError::UnobservedSelector2HistoryWrap {
                        history_len,
                        byte_count: out_len,
                    });
                }
            }

            let flag = take_byte_group_window_flag(spec.payload, &mut state.reader)?;
            if flag != 0 {
                return Err(ByteGroupReadError::UnobservedSelector2RawWindow);
            }

            let (bytes, stream_pos) = decode_selector2_zstd_window(
                spec.payload,
                state.stream_pos,
                out_len,
                &state.selector2_history,
            )?;
            state.stream_pos = stream_pos;
            append_byte_group_history(state, &bytes)?;
            Ok(ByteGroupRead { selector, bytes })
        }
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

pub(super) const WIDTH_EXPAND_TABLE: [(u32, u32); 16] = [
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

/// Mutable state for the byte-group transform wrapper (`0x10fb2e0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteGroupTransformState {
    /// Descriptor mode at `x0+0xc`; only mode 1 is observed in current fixtures.
    pub mode: u32,
    /// Bit-count field at `x0+0x10`; the wrapper shifts it right on every call.
    pub count_bits: u32,
    /// Current record count mirrored at `x0+8`/`x0+0x14`.
    pub record_count: u32,
    /// Second byte-stream count stored at `x0+0x18`.
    pub second_count: u32,
    /// Halfword stream count stored at `x0+0x1c`.
    pub third_count: u32,
    /// Width-bitstream byte count stored at `x0+0x20`.
    pub tail_count: u32,
}

/// One packed table entry consumed by `0x10fb2e0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteGroupTransformTableEntry {
    /// Raw `ctx+0x240` entry built by `0x10f8d20`.
    pub raw: u32,
}

impl ByteGroupTransformTableEntry {
    pub(super) fn byte_count(self) -> u32 {
        (self.raw >> 8) & 0xff
    }

    pub(super) fn group_width(self) -> u32 {
        self.raw & 7
    }

    pub(super) fn width_stride(self) -> u32 {
        self.raw >> 24
    }

    pub(super) fn shift(self) -> u32 {
        (self.raw >> 3) & 3
    }

    pub(super) fn width_class(self) -> Result<u32, ByteGroupTransformError> {
        let byte_count = self.byte_count();
        let group_width = self.group_width();
        if byte_count == 0 {
            return Err(ByteGroupTransformError::UnobservedZeroTableByteCount);
        }
        if group_width == 0 {
            return Err(ByteGroupTransformError::UnobservedZeroTableGroupWidth);
        }
        let rounded_bytes = byte_count
            .checked_add(7)
            .ok_or(ByteGroupTransformError::ArithmeticOverflow)?
            >> 3;
        let product = group_width
            .checked_mul(rounded_bytes)
            .ok_or(ByteGroupTransformError::ArithmeticOverflow)?;
        Ok(if product <= 2 {
            3
        } else if product <= 5 {
            2
        } else {
            1
        })
    }
}

/// Inputs for one byte-group transform-wrapper call (`0x10fb2e0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteGroupTransformSpec<'a> {
    /// Payload bytes addressed by byte-group readers and direct bitstreams.
    pub payload: &'a [u8],
    /// Current packed table entry selected from `ctx+0x240`.
    pub table_entry: ByteGroupTransformTableEntry,
    /// Vertex/count limit passed in `w6`.
    pub limit: u32,
}

/// Result of one byte-group transform-wrapper call (`0x10fb2e0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteGroupTransformResult {
    /// `w0` on return: sum of first-stream widths, or `limit` for the early path.
    pub ret: u32,
    /// Records returned through the caller's `x1`/`x2` out pointer/count pair.
    pub records: Vec<[u32; 2]>,
}

/// Errors from the byte-group transform wrapper (`0x10fb2e0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ByteGroupTransformError {
    /// The wrapper only observed descriptor mode 1.
    UnobservedMode(u32),
    /// Active mode observed `record_count >= 2`.
    UnobservedShortActiveCount(u32),
    /// Active mode observed a non-empty fourth stream for width bits.
    UnobservedZeroTailBitstream,
    /// The fourth stream is always direct selector 3 in current captures.
    UnobservedTailSelector(u8),
    /// Packed table entries should have non-zero byte counts.
    UnobservedZeroTableByteCount,
    /// Packed table entries should have non-zero group widths.
    UnobservedZeroTableGroupWidth,
    /// Forward varint read exceeded the observed u32-shaped encoding.
    VarintTooLong,
    /// The shared payload ended before a forward varint or bitstream slop read.
    StreamTooShort,
    /// The reverse count-bit reader tried to load outside the payload.
    PayloadTooSmall,
    /// Count/table arithmetic overflowed.
    ArithmeticOverflow,
    /// Nested byte-group reader rejected a stream.
    ByteGroupRead(ByteGroupReadError),
    /// Nested width combiner rejected the decoded streams.
    WidthCombiner(WidthCombinerError),
    /// `0x110d360` did not consume the three logical byte-group streams exactly.
    WidthCombinerUnusedInput {
        expected: [usize; 3],
        actual: [usize; 3],
    },
}

fn read_byte_group_transform_varint(
    payload: &[u8],
    stream_pos: &mut usize,
) -> Result<usize, ByteGroupTransformError> {
    let mut value = 0usize;
    for _ in 0..5 {
        let byte = payload
            .get(*stream_pos)
            .copied()
            .ok_or(ByteGroupTransformError::StreamTooShort)?;
        *stream_pos += 1;
        value = value
            .checked_shl(7)
            .and_then(|v| v.checked_add((byte & 0x7f) as usize))
            .ok_or(ByteGroupTransformError::VarintTooLong)?;
        if value > u32::MAX as usize {
            return Err(ByteGroupTransformError::VarintTooLong);
        }
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(ByteGroupTransformError::VarintTooLong)
}

fn take_byte_group_transform_count_bit(
    payload: &[u8],
    reader: &mut RansThreeLaneReader,
) -> Result<u8, ByteGroupTransformError> {
    let bitpos = reader.bitpos;
    let ptr_step = ((bitpos >> 3) ^ 7) as usize;
    let ptr = reader
        .ptr
        .checked_sub(ptr_step)
        .ok_or(ByteGroupTransformError::PayloadTooSmall)?;
    let bits = (checked_byte_group_u64_le(payload, reader.ptr)
        .map_err(|_| ByteGroupTransformError::PayloadTooSmall)?
        >> (bitpos & 63))
        | reader.acc;
    let flag = (bits >> 63) as u8;
    reader.ptr = ptr;
    reader.acc = bits << 1;
    reader.bitpos = (bitpos | 0x38).wrapping_sub(1);
    Ok(flag)
}

fn read_transform_byte_group(
    byte_state: &mut ByteGroupReadState,
    payload: &[u8],
    element_shift: u32,
    count: usize,
) -> Result<ByteGroupRead, ByteGroupTransformError> {
    byte_group_read(
        byte_state,
        ByteGroupReadSpec {
            payload,
            element_shift,
            group_stride: 1,
            count,
        },
    )
    .map_err(ByteGroupTransformError::ByteGroupRead)
}

fn peek_byte_group_transform_selector(
    payload: &[u8],
    reader: &RansThreeLaneReader,
) -> Result<u8, ByteGroupTransformError> {
    let bitpos = reader.bitpos;
    let bits = checked_byte_group_u64_le(payload, reader.ptr)
        .map_err(|_| ByteGroupTransformError::PayloadTooSmall)?
        .checked_shr(bitpos & 63)
        .unwrap_or(0)
        | reader.acc;
    Ok((bits >> 62) as u8)
}

/// Port of the observed byte-group transform wrapper (`0x10fb2e0`).
///
/// The wrapper first shifts the state count field (`0x10fb2fc..0x10fb318`).
/// Even counts take the captured early path, returning one `[limit, 0]` record.
/// Odd counts in mode 1 read two forward varints, consume one reverse count bit,
/// materialize three logical byte streams plus one direct bitstream through
/// `0x110d7f0`, then run `0x110d360` over a fresh record buffer.
pub fn byte_group_transform(
    state: &mut ByteGroupTransformState,
    byte_state: &mut ByteGroupReadState,
    spec: ByteGroupTransformSpec<'_>,
) -> Result<ByteGroupTransformResult, ByteGroupTransformError> {
    let count_bits = state.count_bits;
    state.count_bits >>= 1;
    if count_bits & 1 == 0 {
        state.record_count = 1;
        state.second_count = 0;
        state.third_count = 0;
        state.tail_count = 0;
        return Ok(ByteGroupTransformResult {
            ret: spec.limit,
            records: vec![[spec.limit, 0]],
        });
    }

    if state.mode != 1 {
        return Err(ByteGroupTransformError::UnobservedMode(state.mode));
    }
    let width_class = spec.table_entry.width_class()?;

    let first_count = read_byte_group_transform_varint(spec.payload, &mut byte_state.stream_pos)?;
    let tail_count = read_byte_group_transform_varint(spec.payload, &mut byte_state.stream_pos)?;
    let high_count_bit =
        take_byte_group_transform_count_bit(spec.payload, &mut byte_state.reader)? as usize;
    if first_count < 2 {
        return Err(ByteGroupTransformError::UnobservedShortActiveCount(
            first_count as u32,
        ));
    }
    if tail_count == 0 {
        return Err(ByteGroupTransformError::UnobservedZeroTailBitstream);
    }

    let second_count = first_count
        .checked_sub(1)
        .ok_or(ByteGroupTransformError::ArithmeticOverflow)?;
    let third_count = first_count
        .checked_sub(high_count_bit)
        .ok_or(ByteGroupTransformError::ArithmeticOverflow)?;

    state.record_count = first_count as u32;
    state.second_count = second_count as u32;
    state.third_count = third_count as u32;
    state.tail_count = tail_count as u32;

    let stream0 = read_transform_byte_group(byte_state, spec.payload, 0, first_count)?;
    let stream1 = read_transform_byte_group(byte_state, spec.payload, 0, second_count)?;
    let stream2 = read_transform_byte_group(byte_state, spec.payload, 1, third_count)?;

    let tail_stream_start = byte_state.stream_pos;
    let tail_selector = peek_byte_group_transform_selector(spec.payload, &byte_state.reader)?;
    if tail_selector != 3 {
        return Err(ByteGroupTransformError::UnobservedTailSelector(
            tail_selector,
        ));
    }
    let _bitstream = read_transform_byte_group(byte_state, spec.payload, 0, tail_count)?;
    let bitstream_end = tail_stream_start
        .checked_add(tail_count)
        .and_then(|v| v.checked_add(16))
        .ok_or(ByteGroupTransformError::ArithmeticOverflow)?;
    let bitstream_payload = spec
        .payload
        .get(tail_stream_start..bitstream_end)
        .ok_or(ByteGroupTransformError::StreamTooShort)?;

    let mut records = vec![[0u32; 2]; first_count];
    let mut width_reader = RansThreeLaneReader {
        ptr: 0,
        acc: 0,
        bitpos: 0,
    };
    let width = width_combiner_into(
        &mut records,
        WidthCombinerSpec {
            count: first_count,
            stride: spec.table_entry.width_stride(),
            shift: spec.table_entry.shift(),
            attr_width: width_class,
            limit: spec.limit,
            payload: bitstream_payload,
            stream0: &stream0.bytes,
            stream1: &stream1.bytes,
            stream2: &stream2.bytes,
            reader: &mut width_reader,
        },
    )
    .map_err(ByteGroupTransformError::WidthCombiner)?;
    let expected_consumed = [
        stream0.bytes.len(),
        stream1.bytes.len(),
        stream2.bytes.len(),
    ];
    if width.consumed != expected_consumed {
        return Err(ByteGroupTransformError::WidthCombinerUnusedInput {
            expected: expected_consumed,
            actual: width.consumed,
        });
    }

    Ok(ByteGroupTransformResult {
        ret: width.ret,
        records,
    })
}
