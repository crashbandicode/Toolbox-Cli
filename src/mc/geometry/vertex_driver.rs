use super::byte_group::*;
use super::rans::RansThreeLaneReader;
use super::transform_tails::*;
use super::transport::TableBuild;
/// One payload window located by the CP5d vertex/index kernel path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VertexKernelWindow {
    /// Reverse-reader flag passed to `0x11109e0`: observed `0` = zstd block,
    /// `1` = raw copy.
    pub flag: u8,
    /// Payload-relative start of the window bytes after the forward varint.
    pub src_start: usize,
    /// Window byte count from the forward varint.
    pub src_size: usize,
    /// Forward stream position after this window.
    pub next_stream_pos: usize,
}

/// Continuation header parsed at `0x10f9838..0x10f9918` when the first kernel
/// leaf did not finish the observed index sub-block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VertexKernelContinuation {
    /// Low nibble of the continuation byte (`w27`).
    pub mode: u8,
    /// High nibble of the continuation byte (`w22`).
    pub kind: u8,
    /// First forward varint (`w20`).
    pub repeat: u32,
    /// Second forward varint (`w28`), the next leaf count.
    pub count: u32,
    /// Third forward varint (`w4`), the next leaf's current cursor.
    pub current: u32,
}

/// Result of replaying the observed CP5d pre-state-4 kernel/control-bit path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VertexKernelState4Entry {
    /// Expanded control bits consumed from the shared reverse reader.
    pub bits: Vec<u8>,
    /// Code stream window (`0x10fae60`, first call).
    pub code_window: VertexKernelWindow,
    /// Data stream window (`0x10fae60`, second call).
    pub data_window: VertexKernelWindow,
    /// Optional second-submesh continuation header.
    pub continuation: Option<VertexKernelContinuation>,
    /// Reader state at the `0x11104d0` state-4 setup entry.
    pub reader: RansThreeLaneReader,
    /// Forward stream position at the `0x11104d0` state-4 setup entry.
    pub stream_pos: usize,
}

/// Errors from the observed CP5d kernel/control-bit transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VertexKernelStateError {
    /// A reverse-reader load ran outside the payload.
    PayloadTooSmall,
    /// A forward varint or window body ran outside the payload.
    StreamTooShort,
    /// A forward varint exceeded the observed u32-shaped encoding.
    VarintTooLong,
    /// Pointer or size arithmetic overflowed.
    ArithmeticOverflow,
    /// The pre-kernel decision bit at `0x10f90d4` took the unobserved scratch path.
    UnobservedDecisionBit(u8),
    /// Current captures cover first-sub-block counts 1 and 2 only.
    UnobservedSubmeshCount(usize),
    /// The first kernel leaf must request the observed data-window unary code.
    UnobservedFirstLeafUnary(u32),
    /// The continuation leaf must take the observed no-new-window unary code.
    UnobservedContinuationUnary(u32),
    /// A window flag disagreed with the observed zstd/raw structure.
    UnobservedWindowFlag { window: &'static str, flag: u8 },
    /// The second-submesh continuation selected an unobserved leaf.
    UnobservedContinuationModeKind { mode: u8, kind: u8 },
}

fn checked_vertex_kernel_u64_le(payload: &[u8], ptr: usize) -> Result<u64, VertexKernelStateError> {
    let end = ptr
        .checked_add(8)
        .ok_or(VertexKernelStateError::PayloadTooSmall)?;
    let bytes = payload
        .get(ptr..end)
        .ok_or(VertexKernelStateError::PayloadTooSmall)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn take_vertex_kernel_decision_bit(
    payload: &[u8],
    reader: &mut RansThreeLaneReader,
    reverse_mode: u32,
) -> Result<u8, VertexKernelStateError> {
    let bitpos = reader.bitpos;
    let step = ((bitpos >> 3) ^ 7) as usize;
    let word = checked_vertex_kernel_u64_le(payload, reader.ptr)?;
    let (word, ptr) = if reverse_mode == 1 {
        (
            word.swap_bytes(),
            reader
                .ptr
                .checked_add(step)
                .ok_or(VertexKernelStateError::PayloadTooSmall)?,
        )
    } else {
        (
            word,
            reader
                .ptr
                .checked_sub(step)
                .ok_or(VertexKernelStateError::PayloadTooSmall)?,
        )
    };
    let bits = (word >> (bitpos & 63)) | reader.acc;
    reader.ptr = ptr;
    reader.acc = bits << 1;
    reader.bitpos = (bitpos | 0x38).wrapping_sub(1);
    Ok((bits >> 63) as u8)
}

fn take_vertex_kernel_bit(
    payload: &[u8],
    reader: &mut RansThreeLaneReader,
) -> Result<u8, VertexKernelStateError> {
    let bitpos = reader.bitpos;
    let step = ((bitpos >> 3) ^ 7) as usize;
    let ptr = reader
        .ptr
        .checked_sub(step)
        .ok_or(VertexKernelStateError::PayloadTooSmall)?;
    let bits = (checked_vertex_kernel_u64_le(payload, reader.ptr)? >> (bitpos & 63)) | reader.acc;
    reader.ptr = ptr;
    reader.acc = bits << 1;
    reader.bitpos = (bitpos | 0x38).wrapping_sub(1);
    Ok((bits >> 63) as u8)
}

fn take_vertex_kernel_unary(
    payload: &[u8],
    reader: &mut RansThreeLaneReader,
) -> Result<u32, VertexKernelStateError> {
    let bitpos = reader.bitpos;
    let step = ((bitpos >> 3) ^ 7) as usize;
    let ptr = reader
        .ptr
        .checked_sub(step)
        .ok_or(VertexKernelStateError::PayloadTooSmall)?;
    let bits = (checked_vertex_kernel_u64_le(payload, reader.ptr)? >> (bitpos & 63)) | reader.acc;
    let leading_zeroes = bits.leading_zeros();
    let consumed = leading_zeroes
        .checked_add(1)
        .ok_or(VertexKernelStateError::ArithmeticOverflow)?;
    if consumed > 64 {
        return Err(VertexKernelStateError::PayloadTooSmall);
    }
    reader.ptr = ptr;
    reader.acc = if consumed == 64 { 0 } else { bits << consumed };
    reader.bitpos = (bitpos | 0x38).wrapping_sub(consumed);
    Ok(leading_zeroes)
}

fn read_vertex_kernel_varint(
    payload: &[u8],
    mut pos: usize,
) -> Result<(u32, usize), VertexKernelStateError> {
    let mut value = 0u32;
    for _ in 0..5 {
        let byte = payload
            .get(pos)
            .copied()
            .ok_or(VertexKernelStateError::StreamTooShort)?;
        pos += 1;
        value = value
            .checked_shl(7)
            .and_then(|v| v.checked_add((byte & 0x7f) as u32))
            .ok_or(VertexKernelStateError::VarintTooLong)?;
        if byte & 0x80 == 0 {
            return Ok((value, pos));
        }
    }
    Err(VertexKernelStateError::VarintTooLong)
}

fn read_vertex_kernel_window(
    payload: &[u8],
    state: &mut ByteGroupReadState,
    bits: &mut Vec<u8>,
    name: &'static str,
    expected_flag: u8,
) -> Result<VertexKernelWindow, VertexKernelStateError> {
    let flag = take_vertex_kernel_bit(payload, &mut state.reader)?;
    bits.push(flag);
    if flag != expected_flag {
        return Err(VertexKernelStateError::UnobservedWindowFlag { window: name, flag });
    }
    let (src_size, src_start) = read_vertex_kernel_varint(payload, state.stream_pos)?;
    let src_size = src_size as usize;
    let next_stream_pos = src_start
        .checked_add(src_size)
        .ok_or(VertexKernelStateError::StreamTooShort)?;
    payload
        .get(src_start..next_stream_pos)
        .ok_or(VertexKernelStateError::StreamTooShort)?;
    state.stream_pos = next_stream_pos;
    Ok(VertexKernelWindow {
        flag,
        src_start,
        src_size,
        next_stream_pos,
    })
}

fn push_vertex_kernel_unary_bits(bits: &mut Vec<u8>, leading_zeroes: u32) {
    bits.extend(std::iter::repeat_n(0, leading_zeroes as usize));
    bits.push(1);
}

/// Replay the observed CP5d transition from the state-0 table reader into
/// state 4's `0x11104d0` setup entry.
///
/// This is the cursor/state portion of `0x10f90d4..0x10f91d0` plus the
/// `0x10fa980` leaf decisions needed before state 4. It deliberately stops at
/// the input contract of the already-ported `vertex_match_table`; it does not
/// claim to port the full leaf output transforms. Observed first sub-blocks
/// have one (Dragonfly) or two (Bear/Bass) index submeshes: the first consumes a
/// zstd code window, a unary `01` code, and a raw data window; the optional
/// continuation consumes `mode=1,kind=0` plus three forward varints, then a
/// unary `1` code.
pub fn vertex_kernel_state4_entry(
    payload: &[u8],
    state: &mut ByteGroupReadState,
    submesh_count: usize,
    reverse_mode: u32,
) -> Result<VertexKernelState4Entry, VertexKernelStateError> {
    if !(1..=2).contains(&submesh_count) {
        return Err(VertexKernelStateError::UnobservedSubmeshCount(
            submesh_count,
        ));
    }

    let mut bits = Vec::new();
    let decision = take_vertex_kernel_decision_bit(payload, &mut state.reader, reverse_mode)?;
    bits.push(decision);
    if decision != 0 {
        return Err(VertexKernelStateError::UnobservedDecisionBit(decision));
    }

    let code_window = read_vertex_kernel_window(payload, state, &mut bits, "code", 0)?;
    let first_unary = take_vertex_kernel_unary(payload, &mut state.reader)?;
    push_vertex_kernel_unary_bits(&mut bits, first_unary);
    if first_unary != 1 {
        return Err(VertexKernelStateError::UnobservedFirstLeafUnary(
            first_unary,
        ));
    }
    let data_window = read_vertex_kernel_window(payload, state, &mut bits, "data", 1)?;

    let continuation = if submesh_count == 2 {
        let header = payload
            .get(state.stream_pos)
            .copied()
            .ok_or(VertexKernelStateError::StreamTooShort)?;
        state.stream_pos += 1;
        let mode = header & 0x0f;
        let kind = header >> 4;
        if mode != 1 || kind >= 2 {
            return Err(VertexKernelStateError::UnobservedContinuationModeKind { mode, kind });
        }
        let (repeat, pos) = read_vertex_kernel_varint(payload, state.stream_pos)?;
        state.stream_pos = pos;
        let (count, pos) = read_vertex_kernel_varint(payload, state.stream_pos)?;
        state.stream_pos = pos;
        let (current, pos) = read_vertex_kernel_varint(payload, state.stream_pos)?;
        state.stream_pos = pos;

        let continuation_unary = take_vertex_kernel_unary(payload, &mut state.reader)?;
        push_vertex_kernel_unary_bits(&mut bits, continuation_unary);
        if continuation_unary != 0 {
            return Err(VertexKernelStateError::UnobservedContinuationUnary(
                continuation_unary,
            ));
        }

        Some(VertexKernelContinuation {
            mode,
            kind,
            repeat,
            count,
            current,
        })
    } else {
        None
    };

    Ok(VertexKernelState4Entry {
        bits,
        code_window,
        data_window,
        continuation,
        reader: state.reader,
        stream_pos: state.stream_pos,
    })
}

/// Mutable state for the vertex match-table builder (`0x11106d0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VertexMatchTableState {
    /// Wrapped base value stored at `ctx+0x148`.
    pub base: u32,
    /// Current wrap limit stored at `ctx+0x14c`.
    pub limit: u32,
    /// History-ring mask stored at `ctx+0x150`.
    pub mask: u32,
}

/// Inputs for the state-4 vertex match-table builder (`0x11106d0`).
pub struct VertexMatchTableSpec<'a> {
    /// Number of output match words (`w1`, current vertex block size).
    pub count: usize,
    /// Already-processed vertices added to emitted match distances (`w6`).
    pub processed_vertices: u32,
    /// Descriptor counts written by `0x11104d0`.
    pub counts: [usize; 4],
    /// Builder state at `ctx+0x148`, updated on success.
    pub state: &'a mut VertexMatchTableState,
    /// First setup stream (`x2[0]`), consumed only when the ring wraps.
    pub stream0: &'a [u8],
    /// Sparse output-position deltas (`x2[1]`).
    pub stream1: &'a [u8],
    /// Ring-position deltas (`x2[2]`).
    pub stream2: &'a [u8],
    /// Extended-byte bitstream (`x2[3]`).
    pub stream3: &'a [u8],
}

/// Errors from the vertex match-table builder (`0x11106d0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VertexMatchTableError {
    /// A setup stream ended before the disassembly would read it.
    StreamTooShort { stream: u8 },
    /// The extended-byte bitstream could not supply an 8-byte reversed load.
    BitstreamTooShort,
    /// Byte expansion code was outside the observed 0x10..0x1f table.
    ExpansionCodeTooLarge(u8),
    /// The history mask would allocate an unreasonable or overflowing ring.
    HistoryTooLarge(u32),
    /// A sparse match-table index points outside the output block.
    MatchIndexOutOfBounds { index: usize, count: usize },
    /// Index arithmetic overflowed.
    ArithmeticOverflow,
}

fn read_match_u8(
    stream: &[u8],
    pos: &mut usize,
    stream_id: u8,
) -> Result<u8, VertexMatchTableError> {
    let byte = stream
        .get(*pos)
        .copied()
        .ok_or(VertexMatchTableError::StreamTooShort { stream: stream_id })?;
    *pos += 1;
    Ok(byte)
}

fn checked_match_u64_le(buf: &[u8], ptr: usize) -> Result<u64, VertexMatchTableError> {
    let end = ptr
        .checked_add(8)
        .ok_or(VertexMatchTableError::BitstreamTooShort)?;
    let bytes = buf
        .get(ptr..end)
        .ok_or(VertexMatchTableError::BitstreamTooShort)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn decode_match_table_byte(
    bitstream: &[u8],
    reader: &mut RansThreeLaneReader,
    byte: u8,
) -> Result<u32, VertexMatchTableError> {
    if byte < 0x10 {
        return Ok(byte as u32);
    }
    let (bits, base) = *WIDTH_EXPAND_TABLE
        .get((byte - 0x10) as usize)
        .ok_or(VertexMatchTableError::ExpansionCodeTooLarge(byte))?;
    let word = checked_match_u64_le(bitstream, reader.ptr)?.swap_bytes();
    let bitpos =
        i32::try_from(reader.bitpos).map_err(|_| VertexMatchTableError::ArithmeticOverflow)?;
    let step = (bitpos >> 3) ^ 7;
    if step < 0 {
        return Err(VertexMatchTableError::ArithmeticOverflow);
    }
    reader.ptr = reader
        .ptr
        .checked_add(step as usize)
        .ok_or(VertexMatchTableError::BitstreamTooShort)?;
    let acc = (word >> (reader.bitpos & 63)) | reader.acc;
    let extra = if bits == 0 { 0 } else { acc >> (64 - bits) };
    reader.acc = if bits == 64 { 0 } else { acc << bits };
    reader.bitpos = (reader.bitpos | 0x38)
        .checked_sub(bits)
        .ok_or(VertexMatchTableError::ArithmeticOverflow)?;
    Ok(base + extra as u32 + 0x10)
}

/// Build the writer match table produced by `0x11106d0`.
///
/// State 4 (`0x10f9158..0x10f9220`) first calls `0x11104d0` to materialize four
/// byte-group streams. `0x11106d0` then walks streams 1/2 as sparse deltas,
/// uses stream 3 as a reversed extended-byte bitstream, and emits one u32 match
/// word per vertex at `ctx+0x228`. The history ring stores negative
/// `(vertex_index << 3)` distances, so later occurrences become the writer
/// match-table lookback values.
pub fn vertex_match_table(
    spec: VertexMatchTableSpec<'_>,
) -> Result<Vec<u32>, VertexMatchTableError> {
    let history_len = spec
        .state
        .mask
        .checked_add(1)
        .ok_or(VertexMatchTableError::HistoryTooLarge(spec.state.mask))?;
    if history_len > 0x10000 {
        return Err(VertexMatchTableError::HistoryTooLarge(spec.state.mask));
    }
    let mut history = vec![0u32; history_len as usize];
    let mut out = vec![0u32; spec.count];
    let table_count = spec.counts[2];
    if table_count == 0 {
        return Ok(out);
    }

    let mut pos0 = 0usize;
    let mut pos1 = 0usize;
    let mut pos2 = 0usize;
    let mut last_index = -1i64;
    let mut reader = RansThreeLaneReader {
        ptr: 0,
        acc: 0,
        bitpos: 0,
    };
    let mut base = spec.state.base;
    let mut limit = spec.state.limit;

    for _ in 0..table_count {
        let first_raw = read_match_u8(spec.stream1, &mut pos1, 1)?;
        let first = decode_match_table_byte(spec.stream3, &mut reader, first_raw)?;
        let second_raw = read_match_u8(spec.stream2, &mut pos2, 2)?;
        let second = decode_match_table_byte(spec.stream3, &mut reader, second_raw)?;

        last_index = last_index
            .checked_add(i64::from(first))
            .ok_or(VertexMatchTableError::ArithmeticOverflow)?;
        if last_index < 0 {
            return Err(VertexMatchTableError::ArithmeticOverflow);
        }

        let wrap_limit = limit.wrapping_add(1);
        let sign_mask = if second & 1 == 0 { 0 } else { u32::MAX };
        let signed_delta = sign_mask ^ (second >> 1);
        let mut candidate = base.wrapping_add(signed_delta);
        if (candidate as i32) < 0 {
            candidate = candidate.wrapping_add(wrap_limit);
        }
        let wrapped = if (candidate as i32) > (limit as i32) {
            wrap_limit
        } else {
            0
        };
        base = candidate.wrapping_sub(wrapped);

        let output_index =
            usize::try_from(last_index).map_err(|_| VertexMatchTableError::ArithmeticOverflow)?;
        let absolute_index = (last_index as u32).wrapping_add(spec.processed_vertices);
        let history_index = (base & spec.state.mask) as usize;
        let history_slot = history
            .get_mut(history_index)
            .ok_or(VertexMatchTableError::HistoryTooLarge(spec.state.mask))?;

        if base == limit {
            let low_bits = read_match_u8(spec.stream0, &mut pos0, 0)?;
            limit = wrap_limit;
            let distance = absolute_index
                .checked_shl(3)
                .ok_or(VertexMatchTableError::ArithmeticOverflow)?;
            *history_slot = 0u32.wrapping_sub(distance) | low_bits as u32;
            continue;
        }

        if output_index >= spec.count {
            return Err(VertexMatchTableError::MatchIndexOutOfBounds {
                index: output_index,
                count: spec.count,
            });
        }
        let distance = absolute_index
            .checked_shl(3)
            .ok_or(VertexMatchTableError::ArithmeticOverflow)?;
        out[output_index] = history_slot.wrapping_add(distance);
        *history_slot = 0u32.wrapping_sub(distance);
    }

    spec.state.base = base;
    spec.state.limit = limit;
    Ok(out)
}

/// Mutable state for the observed vertex attribute driver loop (`0x10f924c`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VertexAttributeDriverState {
    /// Driver cursor stored at `ctx+0x218`.
    pub current_attribute: usize,
    /// Vertex count already consumed by prior blocks (`ctx+0x108`).
    pub processed_vertices: u32,
    /// Total vertex count for the current sub-block (`ctx+0x10c`).
    pub vertex_count: u32,
    /// Per-pass block limit (`ctx+0x110`), compared against remaining vertices.
    pub block_limit: u32,
    /// The byte-group transform state rooted at `ctx+0x70`.
    pub transform_state: ByteGroupTransformState,
    /// The shared byte-group reader rooted at the driver stack frame.
    pub byte_state: ByteGroupReadState,
}

/// One per-attribute output from the vertex driver before writer dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VertexAttributeTransform {
    /// Attribute/table index selected from `ctx+0x218`.
    pub index: usize,
    /// Packed table entry from `ctx+0x240[index]`.
    pub table_entry: ByteGroupTransformTableEntry,
    /// Destination byte offset from `ctx+0x27c[index]`.
    pub out_offset: u32,
    /// Attribute column offset from `ctx+0x310[index]`.
    pub column: u8,
    /// Limit passed as `w6` to `0x10fb2e0`.
    pub limit: u32,
    /// Return value from `0x10fb2e0`.
    pub ret: u32,
    /// Width records returned through the wrapper out pointer/count pair.
    pub records: Vec<[u32; 2]>,
}

/// One source descriptor written by the `0x10f9314..0x10f9360` setup tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VertexAttributeSourceDescriptor {
    /// Register `w2` for `0x110d7f0`: element-size shift.
    pub element_shift: u32,
    /// Register `w3` for `0x110d7f0`: byte/element group stride.
    pub group_stride: usize,
    /// Register `w4` for `0x110d7f0`: group count.
    pub count: usize,
}

/// Writer target selected by the interstage dispatch table at `0x39ba570`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VertexAttributeWriterTarget {
    /// `0x10fc5e0`.
    Copy1,
    /// `0x10fc680`.
    Copy2,
    /// `0x10fc7d0`.
    Copy4,
    /// `0x10fbcc0`.
    Delta2,
    /// `0x10fbdc0`.
    Delta3,
    /// `0x10fdc00`.
    Delta2Direct,
    /// `0x10fdcf0`.
    Delta3Direct,
    /// `0x10fde00`.
    Delta4Direct,
    /// `0x1100c90`.
    U16x3Delta,
    /// `0x10fdfe0`.
    U16x2DirectDelta,
    /// `0x1101850`.
    U16x2PreviousDelta,
    /// `0x11033e0`.
    U8x2Delta,
    /// `0x1103ab0`.
    U16x2Delta,
    /// `0x110aac0`.
    I8x2Normal,
    /// `0x110ae30`.
    I8x3NormalDelta,
    /// `0x110afb0`.
    Pack10x3Delta,
}

/// One materialized source stream for a vertex writer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VertexAttributeSource {
    /// Two-bit selector consumed by `0x110d7f0`.
    pub selector: u8,
    /// Bytes returned to the writer source table.
    pub bytes: Vec<u8>,
}

/// Source setup/materialization result before the writer call at `0x10f93d8`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VertexAttributeInterstage {
    /// Seven-bit dispatch key read at `0x10f92c8..0x10f9310`.
    pub dispatch: u8,
    /// Writer target selected by the same dispatch.
    pub writer: VertexAttributeWriterTarget,
    /// Descriptor array passed to the source materialization loop.
    pub descriptors: Vec<VertexAttributeSourceDescriptor>,
    /// Materialized source streams in descriptor order.
    pub sources: Vec<VertexAttributeSource>,
}

/// Errors from the observed interstage source setup/materialization path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VertexAttributeInterstageError {
    /// The shared reverse reader tried to load outside the payload.
    PayloadTooSmall,
    /// A setup varint or direct source slice ran past the payload.
    StreamTooShort,
    /// A setup varint exceeded the observed u32-shaped encoding.
    VarintTooLong,
    /// Descriptor arithmetic overflowed.
    ArithmeticOverflow,
    /// The dispatch key has not been observed in the captured population.
    UnobservedDispatch(u8),
    /// Table entries with zero byte count would produce unobserved descriptors.
    UnobservedZeroTableByteCount,
    /// Table entries with zero group width would produce zero-stride sources.
    UnobservedZeroTableGroupWidth,
    /// A descriptor with zero `w3` has not been observed.
    UnobservedZeroDescriptorStride { dispatch: u8, index: usize },
    /// A descriptor with zero `w4` would take the unobserved null-source branch.
    UnobservedZeroSourceCount { dispatch: u8, index: usize },
    /// The setup split varint exceeded the wrapper return.
    SplitExceedsWrapperReturn {
        dispatch: u8,
        split: usize,
        wrapper_ret: u32,
    },
    /// Dispatch 110 has only been validated for byte-sized source components.
    UnobservedNormalElementShift { dispatch: u8, element_shift: u32 },
    /// Nested `0x110d7f0` source read rejected its stream.
    ByteGroupRead {
        index: usize,
        error: ByteGroupReadError,
    },
}

/// Inputs for the writer-table call at `0x10f93d8`.
pub struct VertexAttributeWriterCall<'a> {
    /// Wrapper output for the current attribute.
    pub transform: &'a VertexAttributeTransform,
    /// Source streams and target metadata from `vertex_attribute_interstage_sources`.
    pub interstage: &'a VertexAttributeInterstage,
    /// Match table read through the writer-table state at `x0+0x10`.
    pub matches: &'a [u32],
    /// Block index stored at `[x0+0xa0]`.
    pub block_index: usize,
}

/// Writer-table state rooted at `ctx+0x218`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VertexAttributeWriterTable<'a> {
    /// Match words read by writer targets through `x0+0x10` (`ctx+0x228`).
    pub matches: &'a [u32],
    /// Block index stored at writer-table `[x0+0xa0]`.
    pub block_index: usize,
}

/// Source/table consumption reported by the selected writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VertexAttributeWriterUsage {
    /// Bytes consumed from up to four source streams.
    pub sources: [usize; 4],
    /// Match-table entries consumed by delta writers.
    pub match_entries: usize,
}

/// Errors from the writer-table dispatch at `0x10f93d8`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VertexAttributeWriterError {
    /// The selected writer needed a source stream that setup did not produce.
    MissingSource {
        target: VertexAttributeWriterTarget,
        index: usize,
    },
    /// Fixed-width copy writer rejected its inputs.
    Copy(TransformTailCopyError),
    /// Delta/match writer rejected its inputs.
    Delta(TransformTailDeltaError),
}

/// One completed per-attribute writer-loop step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VertexAttributeWriterLoopStep {
    /// Wrapper records returned by `0x10fb2e0`.
    pub transform: VertexAttributeTransform,
    /// Dispatch/source materialization from `0x10f92c8..0x10f9394`.
    pub interstage: VertexAttributeInterstage,
    /// Source and match-table consumption from the selected writer.
    pub usage: VertexAttributeWriterUsage,
}

/// Errors from the composed per-attribute writer loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VertexAttributeWriterLoopError {
    /// The writer table at `x0+0x10` must cover the full vertex block.
    MatchTableTooSmall { expected: usize, actual: usize },
    /// The `0x10fb2e0` wrapper step rejected its stream or table state.
    Driver(VertexAttributeDriverError),
    /// The setup/source-materialization stage rejected its stream.
    Interstage {
        index: usize,
        error: VertexAttributeInterstageError,
    },
    /// The selected writer rejected its input contract.
    Writer {
        index: usize,
        target: VertexAttributeWriterTarget,
        error: VertexAttributeWriterError,
    },
}

/// Errors from the observed vertex attribute driver setup/loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VertexAttributeDriverError {
    /// The setup bit count has only been observed for non-empty small tables.
    UnobservedTableCount(usize),
    /// The setup reverse reader tried to load outside the payload.
    PayloadTooSmall,
    /// Bit/counter arithmetic overflowed.
    ArithmeticOverflow,
    /// `0x10fafe0` count bits were zero, which would take an unobserved state path.
    UnobservedZeroSetupCountBits,
    /// `0x10fafe0` mode 0 preloads four streams; current captures all return in mode 1.
    UnobservedSetupMode(u32),
    /// The table arrays do not agree with `ctx+0x2c4`.
    TableShapeMismatch {
        symbols: usize,
        entries: usize,
        offsets: usize,
        cols: usize,
    },
    /// The driver cursor was outside the table.
    CurrentAttributeOutOfRange { current: usize, total: usize },
    /// The caller asked for another step after the table was exhausted.
    NoAttributesRemaining { current: usize, total: usize },
    /// Only the first vertex block (`ctx+0x108 == 0`) has been captured.
    UnobservedNonzeroProcessedVertices(u32),
    /// Current fixtures have a single block where `ctx+0x110 >= remaining`.
    UnobservedPartialVertexBlock { remaining: u32, block_limit: u32 },
    /// Zero-size vertex blocks have not been observed.
    UnobservedZeroVertexLimit,
    /// Nested byte-group transform rejected a stream.
    ByteGroupTransform {
        index: usize,
        error: ByteGroupTransformError,
    },
    /// A zero wrapper return jumps to an unported driver path at `0x10f9410`.
    UnobservedZeroTransformReturn { index: usize },
}

/// Observed port of `0x10fafe0` for vertex byte-group transform setup.
///
/// The initializer consumes `table_count` reverse-reader bits into
/// `count_bits` (`0x10fb018..0x10fb034`), then consumes one more bit into
/// `mode` (`0x10fb040..0x10fb058`). Current fixtures all set mode 1, so the
/// stream-preload branch for mode 0 remains guarded.
pub fn vertex_attribute_driver_setup(
    state: &mut ByteGroupTransformState,
    byte_state: &mut ByteGroupReadState,
    payload: &[u8],
    table_count: usize,
) -> Result<(), VertexAttributeDriverError> {
    if table_count == 0 || table_count > 32 {
        return Err(VertexAttributeDriverError::UnobservedTableCount(
            table_count,
        ));
    }

    let reader = &mut byte_state.reader;
    let bitpos = reader.bitpos;
    let ptr_step = ((bitpos >> 3) ^ 7) as usize;
    let next_ptr = reader
        .ptr
        .checked_sub(ptr_step)
        .ok_or(VertexAttributeDriverError::PayloadTooSmall)?;
    let bits = (checked_byte_group_u64_le(payload, reader.ptr)
        .map_err(|_| VertexAttributeDriverError::PayloadTooSmall)?
        >> (bitpos & 63))
        | reader.acc;

    let normalized_bitpos = bitpos | 0x38;
    let table_count_u32 = table_count as u32;
    let count_bits = (bits >> (64 - table_count_u32)) as u32;
    let acc_after_count = bits
        .checked_shl(table_count_u32)
        .ok_or(VertexAttributeDriverError::ArithmeticOverflow)?;
    let bitpos_after_count = normalized_bitpos
        .checked_sub(table_count_u32)
        .ok_or(VertexAttributeDriverError::ArithmeticOverflow)?;
    if count_bits == 0 {
        return Err(VertexAttributeDriverError::UnobservedZeroSetupCountBits);
    }

    let mode = (acc_after_count >> 63) as u32;
    state.count_bits = count_bits;
    state.mode = mode;
    reader.ptr = next_ptr;
    reader.acc = acc_after_count
        .checked_shl(1)
        .ok_or(VertexAttributeDriverError::ArithmeticOverflow)?;
    reader.bitpos = bitpos_after_count
        .checked_sub(1)
        .ok_or(VertexAttributeDriverError::ArithmeticOverflow)?;

    if mode != 1 {
        return Err(VertexAttributeDriverError::UnobservedSetupMode(mode));
    }

    Ok(())
}

/// Observed port of one `0x10f924c` per-attribute driver step through wrapper output.
///
/// This stops before the setup-dispatch/source-materialization and writer tables
/// at `0x10f9314..0x10f93d8`; it returns the width records those later stages
/// consume. Current captures cover the single-block path where `ctx+0x108 == 0`
/// and `ctx+0x110` is at least the vertex count.
pub fn vertex_attribute_driver_step(
    state: &mut VertexAttributeDriverState,
    table: &TableBuild,
    payload: &[u8],
) -> Result<VertexAttributeTransform, VertexAttributeDriverError> {
    let table_len = table.symbols as usize;
    if table.entries.len() != table_len
        || table.offsets.len() != table_len
        || table.cols.len() != table_len
    {
        return Err(VertexAttributeDriverError::TableShapeMismatch {
            symbols: table_len,
            entries: table.entries.len(),
            offsets: table.offsets.len(),
            cols: table.cols.len(),
        });
    }
    if state.current_attribute > table_len {
        return Err(VertexAttributeDriverError::CurrentAttributeOutOfRange {
            current: state.current_attribute,
            total: table_len,
        });
    }
    if state.current_attribute == table_len {
        return Err(VertexAttributeDriverError::NoAttributesRemaining {
            current: state.current_attribute,
            total: table_len,
        });
    }
    if state.processed_vertices != 0 {
        return Err(
            VertexAttributeDriverError::UnobservedNonzeroProcessedVertices(
                state.processed_vertices,
            ),
        );
    }

    let remaining = state
        .vertex_count
        .checked_sub(state.processed_vertices)
        .ok_or(VertexAttributeDriverError::ArithmeticOverflow)?;
    if remaining == 0 {
        return Err(VertexAttributeDriverError::UnobservedZeroVertexLimit);
    }
    if state.block_limit < remaining {
        return Err(VertexAttributeDriverError::UnobservedPartialVertexBlock {
            remaining,
            block_limit: state.block_limit,
        });
    }

    let index = state.current_attribute;
    let table_entry = ByteGroupTransformTableEntry {
        raw: table.entries[index],
    };
    let result = byte_group_transform(
        &mut state.transform_state,
        &mut state.byte_state,
        ByteGroupTransformSpec {
            payload,
            table_entry,
            limit: remaining,
        },
    )
    .map_err(|error| VertexAttributeDriverError::ByteGroupTransform { index, error })?;
    if result.ret == 0 {
        return Err(VertexAttributeDriverError::UnobservedZeroTransformReturn { index });
    }
    state.current_attribute += 1;

    Ok(VertexAttributeTransform {
        index,
        table_entry,
        out_offset: table.offsets[index],
        column: table.cols[index],
        limit: remaining,
        ret: result.ret,
        records: result.records,
    })
}

fn read_vertex_source_setup_varint(
    payload: &[u8],
    stream_pos: &mut usize,
) -> Result<usize, VertexAttributeInterstageError> {
    let mut value = 0usize;
    for _ in 0..5 {
        let byte = payload
            .get(*stream_pos)
            .copied()
            .ok_or(VertexAttributeInterstageError::StreamTooShort)?;
        *stream_pos += 1;
        value = value
            .checked_shl(7)
            .and_then(|v| v.checked_add((byte & 0x7f) as usize))
            .ok_or(VertexAttributeInterstageError::VarintTooLong)?;
        if value > u32::MAX as usize {
            return Err(VertexAttributeInterstageError::VarintTooLong);
        }
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(VertexAttributeInterstageError::VarintTooLong)
}

fn take_vertex_attribute_dispatch(
    payload: &[u8],
    reader: &mut RansThreeLaneReader,
) -> Result<u8, VertexAttributeInterstageError> {
    let bitpos = reader.bitpos;
    let ptr_step = ((bitpos >> 3) ^ 7) as usize;
    let ptr = reader
        .ptr
        .checked_sub(ptr_step)
        .ok_or(VertexAttributeInterstageError::PayloadTooSmall)?;
    let bits = (checked_byte_group_u64_le(payload, reader.ptr)
        .map_err(|_| VertexAttributeInterstageError::PayloadTooSmall)?
        >> (bitpos & 63))
        | reader.acc;
    let dispatch = (bits >> 57) as u8;
    reader.ptr = ptr;
    reader.acc = bits << 7;
    reader.bitpos = (bitpos | 0x38).wrapping_sub(7);
    Ok(dispatch)
}

fn vertex_entry_rounded_bytes(
    table_entry: ByteGroupTransformTableEntry,
) -> Result<usize, VertexAttributeInterstageError> {
    let byte_count = table_entry.byte_count();
    if byte_count == 0 {
        return Err(VertexAttributeInterstageError::UnobservedZeroTableByteCount);
    }
    let rounded = byte_count
        .checked_add(7)
        .ok_or(VertexAttributeInterstageError::ArithmeticOverflow)?
        >> 3;
    Ok(rounded as usize)
}

fn vertex_entry_group_width(
    table_entry: ByteGroupTransformTableEntry,
) -> Result<usize, VertexAttributeInterstageError> {
    let group_width = table_entry.group_width();
    if group_width == 0 {
        return Err(VertexAttributeInterstageError::UnobservedZeroTableGroupWidth);
    }
    Ok(group_width as usize)
}

fn vertex_source_descriptor(
    dispatch: u8,
    index: usize,
    element_shift: u32,
    group_stride: usize,
    count: usize,
) -> Result<VertexAttributeSourceDescriptor, VertexAttributeInterstageError> {
    if group_stride == 0 {
        return Err(
            VertexAttributeInterstageError::UnobservedZeroDescriptorStride { dispatch, index },
        );
    }
    if count == 0 {
        return Err(VertexAttributeInterstageError::UnobservedZeroSourceCount { dispatch, index });
    }
    Ok(VertexAttributeSourceDescriptor {
        element_shift,
        group_stride,
        count,
    })
}

fn vertex_split_remainder(
    dispatch: u8,
    wrapper_ret: u32,
    split: usize,
) -> Result<usize, VertexAttributeInterstageError> {
    let ret = wrapper_ret as usize;
    if split > ret {
        return Err(VertexAttributeInterstageError::SplitExceedsWrapperReturn {
            dispatch,
            split,
            wrapper_ret,
        });
    }
    Ok(ret - split)
}

fn vertex_attribute_source_descriptors(
    byte_state: &mut ByteGroupReadState,
    payload: &[u8],
    dispatch: u8,
    table_entry: ByteGroupTransformTableEntry,
    wrapper_ret: u32,
) -> Result<
    (
        VertexAttributeWriterTarget,
        Vec<VertexAttributeSourceDescriptor>,
    ),
    VertexAttributeInterstageError,
> {
    let rounded = vertex_entry_rounded_bytes(table_entry)?;
    let group_width = vertex_entry_group_width(table_entry)?;
    let ret = wrapper_ret as usize;
    let rounded_stride = rounded
        .checked_mul(group_width)
        .ok_or(VertexAttributeInterstageError::ArithmeticOverflow)?;
    let rounded_minus_one = rounded
        .checked_sub(1)
        .ok_or(VertexAttributeInterstageError::ArithmeticOverflow)?;

    match dispatch {
        // `0x10fc4b0`: one descriptor, no setup varint (`0x10fc4b0..0x10fc4cc`).
        15 | 16 | 18 => {
            let writer = match dispatch {
                15 => VertexAttributeWriterTarget::Copy1,
                16 => VertexAttributeWriterTarget::Copy2,
                18 => VertexAttributeWriterTarget::Copy4,
                _ => unreachable!(),
            };
            Ok((
                writer,
                vec![vertex_source_descriptor(
                    dispatch,
                    0,
                    0,
                    rounded_stride,
                    ret,
                )?],
            ))
        }
        // `0x10fc4e0`: split one varint into two same-width descriptors
        // (`0x10fc508..0x10fc534`).
        30 | 31 | 32 | 35 | 58 => {
            let split = read_vertex_source_setup_varint(payload, &mut byte_state.stream_pos)?;
            let remainder = vertex_split_remainder(dispatch, wrapper_ret, split)?;
            let writer = match dispatch {
                30 => VertexAttributeWriterTarget::Delta2Direct,
                31 => VertexAttributeWriterTarget::Delta3Direct,
                32 => VertexAttributeWriterTarget::Delta4Direct,
                35 => VertexAttributeWriterTarget::U16x2DirectDelta,
                58 => VertexAttributeWriterTarget::U16x3Delta,
                _ => unreachable!(),
            };
            Ok((
                writer,
                vec![
                    vertex_source_descriptor(dispatch, 0, 0, rounded_stride, split)?,
                    vertex_source_descriptor(dispatch, 1, 0, rounded_stride, remainder)?,
                ],
            ))
        }
        // `0x10fb730`: three descriptors, with the first two sharing the split
        // count (`0x10fb758..0x10fb794`).
        7 | 8 => {
            let split = read_vertex_source_setup_varint(payload, &mut byte_state.stream_pos)?;
            let remainder = vertex_split_remainder(dispatch, wrapper_ret, split)?;
            let group_width_minus_one = group_width
                .checked_sub(1)
                .ok_or(VertexAttributeInterstageError::ArithmeticOverflow)?;
            let writer = if dispatch == 7 {
                VertexAttributeWriterTarget::Delta2
            } else {
                VertexAttributeWriterTarget::Delta3
            };
            Ok((
                writer,
                vec![
                    vertex_source_descriptor(dispatch, 0, rounded_minus_one as u32, 1, split)?,
                    vertex_source_descriptor(
                        dispatch,
                        1,
                        rounded_minus_one as u32,
                        group_width_minus_one,
                        split,
                    )?,
                    vertex_source_descriptor(
                        dispatch,
                        2,
                        rounded_minus_one as u32,
                        group_width,
                        remainder,
                    )?,
                ],
            ))
        }
        // `0x11010b0`: one descriptor with element shift `rounded-1`
        // (`0x11010b0..0x11010cc`).
        67 => Ok((
            VertexAttributeWriterTarget::U16x2PreviousDelta,
            vec![vertex_source_descriptor(
                dispatch,
                0,
                rounded_minus_one as u32,
                group_width,
                ret,
            )?],
        )),
        // `0x11010e0`: two descriptors with element shift `rounded-1`
        // (`0x1101108..0x1101134`).
        76 | 81 => {
            let split = read_vertex_source_setup_varint(payload, &mut byte_state.stream_pos)?;
            let remainder = vertex_split_remainder(dispatch, wrapper_ret, split)?;
            let writer = if dispatch == 76 {
                VertexAttributeWriterTarget::U8x2Delta
            } else {
                VertexAttributeWriterTarget::U16x2Delta
            };
            Ok((
                writer,
                vec![
                    vertex_source_descriptor(
                        dispatch,
                        0,
                        rounded_minus_one as u32,
                        group_width,
                        split,
                    )?,
                    vertex_source_descriptor(
                        dispatch,
                        1,
                        rounded_minus_one as u32,
                        group_width,
                        remainder,
                    )?,
                ],
            ))
        }
        // `0x110aa00`: normal-vector setup, no split varint
        // (`0x110aa00..0x110aa34`).
        107 => Ok((
            VertexAttributeWriterTarget::I8x2Normal,
            vec![
                vertex_source_descriptor(dispatch, 0, rounded_minus_one as u32, 2, ret)?,
                vertex_source_descriptor(dispatch, 1, 0, 1, ret)?,
                vertex_source_descriptor(dispatch, 2, 0, 1, ret)?,
            ],
        )),
        // `0x110aa40`: normal-vector delta setup, split once and derive four sources
        // (`0x110aa68..0x110aab0`).
        110 | 111 => {
            let split = read_vertex_source_setup_varint(payload, &mut byte_state.stream_pos)?;
            let remainder = vertex_split_remainder(dispatch, wrapper_ret, split)?;
            let writer = if dispatch == 110 {
                if rounded_minus_one != 0 {
                    return Err(
                        VertexAttributeInterstageError::UnobservedNormalElementShift {
                            dispatch,
                            element_shift: rounded_minus_one as u32,
                        },
                    );
                }
                VertexAttributeWriterTarget::I8x3NormalDelta
            } else {
                VertexAttributeWriterTarget::Pack10x3Delta
            };
            Ok((
                writer,
                vec![
                    vertex_source_descriptor(dispatch, 0, rounded_minus_one as u32, 2, split)?,
                    vertex_source_descriptor(dispatch, 1, rounded_minus_one as u32, 1, split)?,
                    vertex_source_descriptor(dispatch, 2, 0, 1, split)?,
                    vertex_source_descriptor(dispatch, 3, rounded_minus_one as u32, 3, remainder)?,
                ],
            ))
        }
        dispatch => Err(VertexAttributeInterstageError::UnobservedDispatch(dispatch)),
    }
}

/// Port of the observed interstage source setup and materialization.
///
/// After `0x10fb2e0` returns a non-zero vertex count, the driver consumes a
/// seven-bit dispatch key (`0x10f92c8..0x10f9310`), calls one of the captured
/// source-setup targets through `0x39ba1e8` (`0x10f9314..0x10f9338`), and then
/// materializes each non-zero descriptor through `0x110d7f0`
/// (`0x10f9364..0x10f9394`). This deliberately stops before the writer-table
/// call at `0x10f93d8`.
pub fn vertex_attribute_interstage_sources(
    byte_state: &mut ByteGroupReadState,
    payload: &[u8],
    table_entry: ByteGroupTransformTableEntry,
    wrapper_ret: u32,
) -> Result<VertexAttributeInterstage, VertexAttributeInterstageError> {
    let dispatch = take_vertex_attribute_dispatch(payload, &mut byte_state.reader)?;
    let (writer, descriptors) = vertex_attribute_source_descriptors(
        byte_state,
        payload,
        dispatch,
        table_entry,
        wrapper_ret,
    )?;
    let mut sources = Vec::with_capacity(descriptors.len());
    for (index, descriptor) in descriptors.iter().enumerate() {
        let read = byte_group_read(
            byte_state,
            ByteGroupReadSpec {
                payload,
                element_shift: descriptor.element_shift,
                group_stride: descriptor.group_stride,
                count: descriptor.count,
            },
        )
        .map_err(|error| VertexAttributeInterstageError::ByteGroupRead { index, error })?;
        sources.push(VertexAttributeSource {
            selector: read.selector,
            bytes: read.bytes,
        });
    }
    Ok(VertexAttributeInterstage {
        dispatch,
        writer,
        descriptors,
        sources,
    })
}

fn vertex_transform_tail_records(records: &[[u32; 2]]) -> Vec<TransformTailRecord> {
    records
        .iter()
        .map(|record| TransformTailRecord {
            literal_count: (record[0] & 0xffff) as u16,
            copy_count: (record[0] >> 16) as u16,
            back_distance: record[1] as usize,
        })
        .collect()
}

fn vertex_writer_source(
    interstage: &VertexAttributeInterstage,
    index: usize,
) -> Result<&[u8], VertexAttributeWriterError> {
    interstage
        .sources
        .get(index)
        .map(|source| source.bytes.as_slice())
        .ok_or(VertexAttributeWriterError::MissingSource {
            target: interstage.writer,
            index,
        })
}

fn vertex_copy_usage(consumed: usize) -> VertexAttributeWriterUsage {
    VertexAttributeWriterUsage {
        sources: [consumed, 0, 0, 0],
        match_entries: 0,
    }
}

fn vertex_delta_usage(usage: TransformTailDeltaUsage) -> VertexAttributeWriterUsage {
    VertexAttributeWriterUsage {
        sources: [usage.source0, usage.source1, usage.source2, 0],
        match_entries: usage.match_entries,
    }
}

fn vertex_pack10_usage(usage: TransformTailPack10Usage) -> VertexAttributeWriterUsage {
    VertexAttributeWriterUsage {
        sources: [usage.source0, usage.source1, usage.source2, usage.source3],
        match_entries: usage.match_entries,
    }
}

fn vertex_i8x3_normal_delta_usage(
    usage: TransformTailI8x3NormalDeltaUsage,
) -> VertexAttributeWriterUsage {
    VertexAttributeWriterUsage {
        sources: [usage.source0, usage.source1, usage.source2, usage.source3],
        match_entries: usage.match_entries,
    }
}

/// Apply the writer target selected by the vertex interstage dispatch table.
///
/// This mirrors the indirect call through `0x39ba570` at `0x10f93d8`: the
/// current table entry supplies stride/out offset, `x2` supplies run/copy
/// records, `x4` supplies the materialized sources, and the writer-table state
/// supplies the match table through `x0+0x10`.
pub fn vertex_attribute_apply_writer(
    out: &mut [u8],
    spec: VertexAttributeWriterCall<'_>,
) -> Result<VertexAttributeWriterUsage, VertexAttributeWriterError> {
    let output_stride = spec.transform.table_entry.width_stride() as usize;
    let out_offset = spec.transform.out_offset as usize;
    let records = vertex_transform_tail_records(&spec.transform.records);

    match spec.interstage.writer {
        VertexAttributeWriterTarget::Copy1 => {
            let source = vertex_writer_source(spec.interstage, 0)?;
            transform_tail_copy1_into(
                out,
                TransformTailCopy1Spec {
                    output_stride,
                    block_index: spec.block_index,
                    out_offset,
                    records: &records,
                    source,
                },
            )
            .map(vertex_copy_usage)
            .map_err(VertexAttributeWriterError::Copy)
        }
        VertexAttributeWriterTarget::Copy2 => {
            let source = vertex_writer_source(spec.interstage, 0)?;
            transform_tail_copy2_into(
                out,
                TransformTailCopy2Spec {
                    output_stride,
                    block_index: spec.block_index,
                    out_offset,
                    records: &records,
                    source,
                },
            )
            .map(vertex_copy_usage)
            .map_err(VertexAttributeWriterError::Copy)
        }
        VertexAttributeWriterTarget::Copy4 => {
            let source = vertex_writer_source(spec.interstage, 0)?;
            transform_tail_copy4_into(
                out,
                TransformTailCopy4Spec {
                    output_stride,
                    block_index: spec.block_index,
                    out_offset,
                    records: &records,
                    source,
                },
            )
            .map(vertex_copy_usage)
            .map_err(VertexAttributeWriterError::Copy)
        }
        VertexAttributeWriterTarget::Delta2 => {
            let source0 = vertex_writer_source(spec.interstage, 0)?;
            let source1 = vertex_writer_source(spec.interstage, 1)?;
            let source2 = vertex_writer_source(spec.interstage, 2)?;
            transform_tail_delta2_into(
                out,
                TransformTailDelta2Spec {
                    output_stride,
                    block_index: spec.block_index,
                    out_offset,
                    records: &records,
                    matches: spec.matches,
                    source0,
                    source1,
                    source2,
                },
            )
            .map(vertex_delta_usage)
            .map_err(VertexAttributeWriterError::Delta)
        }
        VertexAttributeWriterTarget::Delta3 => {
            let source0 = vertex_writer_source(spec.interstage, 0)?;
            let source1 = vertex_writer_source(spec.interstage, 1)?;
            let source2 = vertex_writer_source(spec.interstage, 2)?;
            transform_tail_delta3_into(
                out,
                TransformTailDelta3Spec {
                    output_stride,
                    block_index: spec.block_index,
                    out_offset,
                    records: &records,
                    matches: spec.matches,
                    source0,
                    source1,
                    source2,
                },
            )
            .map(vertex_delta_usage)
            .map_err(VertexAttributeWriterError::Delta)
        }
        VertexAttributeWriterTarget::Delta2Direct => {
            let source0 = vertex_writer_source(spec.interstage, 0)?;
            let source1 = vertex_writer_source(spec.interstage, 1)?;
            transform_tail_delta2_direct_into(
                out,
                TransformTailDelta2DirectSpec {
                    output_stride,
                    block_index: spec.block_index,
                    out_offset,
                    records: &records,
                    matches: spec.matches,
                    source0,
                    source1,
                },
            )
            .map(vertex_delta_usage)
            .map_err(VertexAttributeWriterError::Delta)
        }
        VertexAttributeWriterTarget::Delta3Direct => {
            let source0 = vertex_writer_source(spec.interstage, 0)?;
            let source1 = vertex_writer_source(spec.interstage, 1)?;
            transform_tail_delta3_direct_into(
                out,
                TransformTailDelta3DirectSpec {
                    output_stride,
                    block_index: spec.block_index,
                    out_offset,
                    records: &records,
                    matches: spec.matches,
                    source0,
                    source1,
                },
            )
            .map(vertex_delta_usage)
            .map_err(VertexAttributeWriterError::Delta)
        }
        VertexAttributeWriterTarget::Delta4Direct => {
            let source0 = vertex_writer_source(spec.interstage, 0)?;
            let source1 = vertex_writer_source(spec.interstage, 1)?;
            transform_tail_delta4_direct_into(
                out,
                TransformTailDelta4DirectSpec {
                    output_stride,
                    block_index: spec.block_index,
                    out_offset,
                    records: &records,
                    matches: spec.matches,
                    source0,
                    source1,
                },
            )
            .map(vertex_delta_usage)
            .map_err(VertexAttributeWriterError::Delta)
        }
        VertexAttributeWriterTarget::U16x3Delta => {
            let source0 = vertex_writer_source(spec.interstage, 0)?;
            let source1 = vertex_writer_source(spec.interstage, 1)?;
            transform_tail_u16x3_delta_into(
                out,
                TransformTailU16x3DeltaSpec {
                    output_stride,
                    block_index: spec.block_index,
                    out_offset,
                    records: &records,
                    matches: spec.matches,
                    source0,
                    source1,
                },
            )
            .map(vertex_delta_usage)
            .map_err(VertexAttributeWriterError::Delta)
        }
        VertexAttributeWriterTarget::U16x2DirectDelta => {
            let source0 = vertex_writer_source(spec.interstage, 0)?;
            let source1 = vertex_writer_source(spec.interstage, 1)?;
            transform_tail_u16x2_direct_delta_into(
                out,
                TransformTailU16x2DirectDeltaSpec {
                    output_stride,
                    block_index: spec.block_index,
                    out_offset,
                    records: &records,
                    matches: spec.matches,
                    source0,
                    source1,
                },
            )
            .map(vertex_delta_usage)
            .map_err(VertexAttributeWriterError::Delta)
        }
        VertexAttributeWriterTarget::U8x2Delta => {
            let source0 = vertex_writer_source(spec.interstage, 0)?;
            let source1 = vertex_writer_source(spec.interstage, 1)?;
            transform_tail_u8x2_delta_into(
                out,
                TransformTailU8x2DeltaSpec {
                    output_stride,
                    block_index: spec.block_index,
                    out_offset,
                    records: &records,
                    matches: spec.matches,
                    source0,
                    source1,
                },
            )
            .map(vertex_delta_usage)
            .map_err(VertexAttributeWriterError::Delta)
        }
        VertexAttributeWriterTarget::U16x2Delta => {
            let source0 = vertex_writer_source(spec.interstage, 0)?;
            let source1 = vertex_writer_source(spec.interstage, 1)?;
            transform_tail_u16x2_delta_into(
                out,
                TransformTailU16x2DeltaSpec {
                    output_stride,
                    block_index: spec.block_index,
                    out_offset,
                    records: &records,
                    matches: spec.matches,
                    source0,
                    source1,
                },
            )
            .map(vertex_delta_usage)
            .map_err(VertexAttributeWriterError::Delta)
        }
        VertexAttributeWriterTarget::U16x2PreviousDelta => {
            let source0 = vertex_writer_source(spec.interstage, 0)?;
            transform_tail_u16x2_previous_delta_into(
                out,
                TransformTailU16x2PreviousDeltaSpec {
                    output_stride,
                    block_index: spec.block_index,
                    out_offset,
                    records: &records,
                    source0,
                },
            )
            .map(vertex_delta_usage)
            .map_err(VertexAttributeWriterError::Delta)
        }
        VertexAttributeWriterTarget::I8x2Normal => {
            let source0 = vertex_writer_source(spec.interstage, 0)?;
            let source1 = vertex_writer_source(spec.interstage, 1)?;
            let source2 = vertex_writer_source(spec.interstage, 2)?;
            transform_tail_i8x2_normal_into(
                out,
                TransformTailI8x2NormalSpec {
                    output_stride,
                    block_index: spec.block_index,
                    out_offset,
                    records: &records,
                    source0,
                    source1,
                    source2,
                },
            )
            .map(vertex_delta_usage)
            .map_err(VertexAttributeWriterError::Delta)
        }
        VertexAttributeWriterTarget::I8x3NormalDelta => {
            let source0 = vertex_writer_source(spec.interstage, 0)?;
            let source1 = vertex_writer_source(spec.interstage, 1)?;
            let source2 = vertex_writer_source(spec.interstage, 2)?;
            let source3 = vertex_writer_source(spec.interstage, 3)?;
            transform_tail_i8x3_normal_delta_into(
                out,
                TransformTailI8x3NormalDeltaSpec {
                    output_stride,
                    block_index: spec.block_index,
                    out_offset,
                    records: &records,
                    matches: spec.matches,
                    source0,
                    source1,
                    source2,
                    source3,
                },
            )
            .map(vertex_i8x3_normal_delta_usage)
            .map_err(VertexAttributeWriterError::Delta)
        }
        VertexAttributeWriterTarget::Pack10x3Delta => {
            let source0 = vertex_writer_source(spec.interstage, 0)?;
            let source1 = vertex_writer_source(spec.interstage, 1)?;
            let source2 = vertex_writer_source(spec.interstage, 2)?;
            let source3 = vertex_writer_source(spec.interstage, 3)?;
            transform_tail_pack10x3_delta_into(
                out,
                TransformTailPack10x3DeltaSpec {
                    output_stride,
                    block_index: spec.block_index,
                    out_offset,
                    records: &records,
                    matches: spec.matches,
                    source0,
                    source1,
                    source2,
                    source3,
                },
            )
            .map(vertex_pack10_usage)
            .map_err(VertexAttributeWriterError::Delta)
        }
    }
}

/// Run one observed `0x10f924c` attribute step through writer dispatch.
///
/// This composes the already-validated wrapper (`0x10fb2e0`), seven-bit
/// interstage/source materialization (`0x10f92c8..0x10f9394`), and writer-table
/// call (`0x10f93d8`). The caller supplies the writer table's stable match
/// slice from `ctx+0x228`; captures show it is not advanced per attribute.
pub fn vertex_attribute_writer_loop_step(
    out: &mut [u8],
    state: &mut VertexAttributeDriverState,
    table: &TableBuild,
    payload: &[u8],
    writer_table: VertexAttributeWriterTable<'_>,
) -> Result<VertexAttributeWriterLoopStep, VertexAttributeWriterLoopError> {
    let expected_matches = state.vertex_count as usize;
    if writer_table.matches.len() < expected_matches {
        return Err(VertexAttributeWriterLoopError::MatchTableTooSmall {
            expected: expected_matches,
            actual: writer_table.matches.len(),
        });
    }

    let transform = vertex_attribute_driver_step(state, table, payload)
        .map_err(VertexAttributeWriterLoopError::Driver)?;
    let index = transform.index;
    let interstage = vertex_attribute_interstage_sources(
        &mut state.byte_state,
        payload,
        transform.table_entry,
        transform.ret,
    )
    .map_err(|error| VertexAttributeWriterLoopError::Interstage { index, error })?;
    let target = interstage.writer;
    let usage = vertex_attribute_apply_writer(
        out,
        VertexAttributeWriterCall {
            transform: &transform,
            interstage: &interstage,
            matches: writer_table.matches,
            block_index: writer_table.block_index,
        },
    )
    .map_err(|error| VertexAttributeWriterLoopError::Writer {
        index,
        target,
        error,
    })?;

    Ok(VertexAttributeWriterLoopStep {
        transform,
        interstage,
        usage,
    })
}

/// Run the observed single-block vertex attribute writer loop to exhaustion.
pub fn vertex_attribute_writer_loop(
    out: &mut [u8],
    state: &mut VertexAttributeDriverState,
    table: &TableBuild,
    payload: &[u8],
    writer_table: VertexAttributeWriterTable<'_>,
) -> Result<Vec<VertexAttributeWriterLoopStep>, VertexAttributeWriterLoopError> {
    let mut steps = Vec::new();
    loop {
        match vertex_attribute_writer_loop_step(out, state, table, payload, writer_table) {
            Ok(step) => steps.push(step),
            Err(VertexAttributeWriterLoopError::Driver(
                VertexAttributeDriverError::NoAttributesRemaining { .. },
            )) => return Ok(steps),
            Err(error) => return Err(error),
        }
    }
}
