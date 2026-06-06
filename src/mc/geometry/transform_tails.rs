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

/// Inputs for the two-byte direct/delta transform tail (`0x10fdc00`).
pub struct TransformTailDelta2DirectSpec<'a> {
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
    /// Matched delta stream at `[x4+8]`.
    pub source1: &'a [u8],
}

/// Inputs for the three-byte direct/delta transform tail (`0x10fdcf0`).
pub struct TransformTailDelta3DirectSpec<'a> {
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
    /// Matched delta stream at `[x4+8]`.
    pub source1: &'a [u8],
}

/// Inputs for the four-byte direct/delta transform tail (`0x10fde00`).
pub struct TransformTailDelta4DirectSpec<'a> {
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
    /// Matched delta stream at `[x4+8]`.
    pub source1: &'a [u8],
}

/// Inputs for the two-byte previous/matched delta transform tail (`0x11033e0`).
pub struct TransformTailU8x2DeltaSpec<'a> {
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
    /// Seed and zero-match previous-row delta stream at `[x4]`.
    pub source0: &'a [u8],
    /// Non-zero match delta stream at `[x4+8]`.
    pub source1: &'a [u8],
}

/// Inputs for the three-byte previous/matched delta transform tail (`0x1103530`).
pub struct TransformTailU8x3DeltaSpec<'a> {
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
    /// Seed and zero-match previous-row delta stream at `[x4]`.
    pub source0: &'a [u8],
    /// Non-zero match delta stream at `[x4+8]`.
    pub source1: &'a [u8],
}

/// Inputs for the three-u16 direct/signed-delta transform tail (`0x1100c90`).
pub struct TransformTailU16x3DeltaSpec<'a> {
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
    /// Direct three-u16 literal source stream at `[x4]`.
    pub source0: &'a [u8],
    /// Matched three-u16 delta stream at `[x4+8]`.
    pub source1: &'a [u8],
}

/// Inputs for the two-u16 direct/matched delta transform tail (`0x10fdfe0`).
pub struct TransformTailU16x2DirectDeltaSpec<'a> {
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
    /// Direct two-u16 literal source stream at `[x4]`.
    pub source0: &'a [u8],
    /// Matched two-u16 delta stream at `[x4+8]`.
    pub source1: &'a [u8],
}

/// Inputs for the two-u16 seed/previous delta transform tail (`0x1101850`).
pub struct TransformTailU16x2PreviousDeltaSpec<'a> {
    /// Entry high byte (`entry >> 24`): byte distance between consecutive vertices.
    pub output_stride: usize,
    /// Block index at `[x0+0xa0]`, folded into the initial output position.
    pub block_index: usize,
    /// Per-entry output byte offset from `[x0 + current*4 + 0x64]`.
    pub out_offset: usize,
    /// Run/copy records at `x2`.
    pub records: &'a [TransformTailRecord],
    /// Seed row and previous-row u16 delta stream at `[x4]`.
    pub source0: &'a [u8],
}

/// Inputs for the packed 10-10-10 direct/delta transform tail (`0x110afb0`).
pub struct TransformTailPack10x3DeltaSpec<'a> {
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
    /// Direct source stream with two raw 10-bit components per row at `[x4]`.
    pub source0: &'a [u8],
    /// Direct third-component delta stream at `[x4+8]`.
    pub source1: &'a [u8],
    /// Direct third-component sign-byte stream at `[x4+0x10]`.
    pub source2: &'a [u8],
    /// Matched three-u16 packed-lane delta stream at `[x4+0x18]`.
    pub source3: &'a [u8],
}

/// Inputs for the two-u16 seed/previous/matched delta transform tail (`0x1103ab0`).
pub struct TransformTailU16x2DeltaSpec<'a> {
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
    /// Seed row and zero-match previous-row delta stream at `[x4]`.
    pub source0: &'a [u8],
    /// Non-zero match delta stream at `[x4+8]`.
    pub source1: &'a [u8],
}

/// Inputs for the three-byte i8/i8/sqrt transform tail (`0x110aac0`).
pub struct TransformTailI8x2NormalSpec<'a> {
    /// Entry high byte (`entry >> 24`): byte distance between consecutive vertices.
    pub output_stride: usize,
    /// Block index at `[x0+0xa0]`, folded into the initial output position.
    pub block_index: usize,
    /// Per-entry output byte offset from `[x0 + current*4 + 0x64]`.
    pub out_offset: usize,
    /// Run/copy records at `x2`.
    pub records: &'a [TransformTailRecord],
    /// Direct signed i8 x/y source stream at `[x4]`.
    pub source0: &'a [u8],
    /// Direct z adjustment stream at `[x4+8]`.
    pub source1: &'a [u8],
    /// Direct z sign-byte stream at `[x4+0x10]`.
    pub source2: &'a [u8],
}

/// Inputs for the three-byte i8/i8/sqrt direct/matched delta tail (`0x110ae30`).
pub struct TransformTailI8x3NormalDeltaSpec<'a> {
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
    /// Direct signed i8 x/y source stream at `[x4]`.
    pub source0: &'a [u8],
    /// Direct z adjustment stream at `[x4+8]`.
    pub source1: &'a [u8],
    /// Direct z sign-byte stream at `[x4+0x10]`.
    pub source2: &'a [u8],
    /// Matched three-byte delta stream at `[x4+0x18]`.
    pub source3: &'a [u8],
}

/// Source and table consumption from a delta-match transform tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransformTailDeltaUsage {
    pub source0: usize,
    pub source1: usize,
    pub source2: usize,
    pub match_entries: usize,
}

/// Source and table consumption from a packed 10-10-10 transform tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransformTailPack10Usage {
    pub source0: usize,
    pub source1: usize,
    pub source2: usize,
    pub source3: usize,
    pub match_entries: usize,
}

/// Source and table consumption from the three-byte normal delta tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransformTailI8x3NormalDeltaUsage {
    pub source0: usize,
    pub source1: usize,
    pub source2: usize,
    pub source3: usize,
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
    Source3TooSmall,
    MatchTableTooSmall,
    MatchBeforeOutput,
    CopyBeforeOutput,
    ArithmeticOverflow,
}

#[inline]
fn read_transform_tail_u16(buf: &[u8], offset: usize) -> Option<u16> {
    let bytes = buf.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

#[inline]
fn write_transform_tail_u16(buf: &mut [u8], offset: usize, value: u16) {
    buf[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

#[inline]
fn read_transform_tail_u32(buf: &[u8], offset: usize) -> Option<u32> {
    let bytes = buf.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[inline]
fn write_transform_tail_u32(buf: &mut [u8], offset: usize, value: u32) {
    buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[inline]
fn sign_extend_10(value: u16) -> i64 {
    let lane = i64::from(value & 0x03ff);
    if lane & 0x0200 != 0 {
        lane - 0x0400
    } else {
        lane
    }
}

#[inline]
fn sign_extend_i8(value: u8) -> i64 {
    i64::from(value as i8)
}

#[inline]
fn pack10_matched_component(previous: u32, delta: u16, sign_bit: u32) -> u32 {
    let sign_mask = if sign_bit != 0 { u32::MAX } else { 0 };
    (previous ^ sign_mask)
        .wrapping_add(u32::from(delta))
        .wrapping_add(sign_bit)
        & 0x03ff
}

#[inline]
fn transform_tail_delta_cursor_init(
    block_index: usize,
    output_stride: usize,
    out_offset: usize,
) -> Result<usize, TransformTailDeltaError> {
    if output_stride == 0 {
        return Err(TransformTailDeltaError::ZeroStride);
    }
    block_index
        .checked_mul(output_stride)
        .and_then(|offset| offset.checked_add(out_offset))
        .ok_or(TransformTailDeltaError::ArithmeticOverflow)
}

fn copy_run_units(
    out: &mut [u8],
    cursor: &mut usize,
    output_stride: usize,
    unit_size: usize,
    back_distance: usize,
    copy_count: u16,
) -> Result<(), TransformTailDeltaError> {
    let mut chunk = [0u8; 8];
    let scratch = chunk
        .get_mut(..unit_size)
        .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;

    for _ in 0..copy_count {
        if back_distance == 0 {
            return Err(TransformTailDeltaError::CopyBeforeOutput);
        }
        let source = cursor
            .checked_sub(back_distance)
            .ok_or(TransformTailDeltaError::CopyBeforeOutput)?;
        let source_end = source
            .checked_add(unit_size)
            .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
        let cursor_end = cursor
            .checked_add(unit_size)
            .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
        let value = out
            .get(source..source_end)
            .ok_or(TransformTailDeltaError::CopyBeforeOutput)?;
        scratch.copy_from_slice(value);
        let slot = out
            .get_mut(*cursor..cursor_end)
            .ok_or(TransformTailDeltaError::OutputTooSmall)?;
        slot.copy_from_slice(scratch);
        *cursor = cursor
            .checked_add(output_stride)
            .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
    }

    Ok(())
}

/// Apply the observed two-byte direct/delta transform tail (`0x10fdc00`).
///
/// Direct literals copy two bytes from source stream 0. Matched literals use the
/// match table's `entry >> 3` distance in vertices, add two source-1 deltas to
/// earlier output bytes, then advance the same strided cursor
/// (`0x10fdc30..0x10fdc9c`). Copy runs clone prior output bytes by the record's
/// byte distance (`0x10fdca0..0x10fdcd4`).
pub fn transform_tail_delta2_direct_into(
    out: &mut [u8],
    spec: TransformTailDelta2DirectSpec<'_>,
) -> Result<TransformTailDeltaUsage, TransformTailDeltaError> {
    let mut cursor =
        transform_tail_delta_cursor_init(spec.block_index, spec.output_stride, spec.out_offset)?;
    let mut match_index = 0usize;
    let mut source0_pos = 0usize;
    let mut source1_pos = 0usize;

    for record in spec.records {
        for _ in 0..record.literal_count {
            let match_entry = *spec
                .matches
                .get(match_index)
                .ok_or(TransformTailDeltaError::MatchTableTooSmall)?;
            let cursor_end = cursor
                .checked_add(2)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            if match_entry == 0 {
                let source0_end = source0_pos
                    .checked_add(2)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let bytes = spec
                    .source0
                    .get(source0_pos..source0_end)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                let slot = out
                    .get_mut(cursor..cursor_end)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                slot.copy_from_slice(bytes);
                source0_pos = source0_end;
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
                let base0 = base[0];
                let base1 = base[1];
                let source1_end = source1_pos
                    .checked_add(2)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let delta = spec
                    .source1
                    .get(source1_pos..source1_end)
                    .ok_or(TransformTailDeltaError::Source1TooSmall)?;
                let slot = out
                    .get_mut(cursor..cursor_end)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                slot[0] = delta[0].wrapping_add(base0);
                slot[1] = delta[1].wrapping_add(base1);
                source1_pos = source1_end;
            }
            cursor = cursor
                .checked_add(spec.output_stride)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            match_index += 1;
        }

        copy_run_units(
            out,
            &mut cursor,
            spec.output_stride,
            2,
            record.back_distance,
            record.copy_count,
        )?;

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
        source2: 0,
        match_entries: match_index,
    })
}

/// Apply the observed two-u16 seed/previous/matched delta tail (`0x1103ab0`).
///
/// The first emitted zero-match row copies four source0 bytes directly
/// (`0x1103ba4..0x1103bb0`). Later zero-match rows add two source0 u16 deltas
/// to the immediately previous row (`0x1103b5c..0x1103b90`). Non-zero match rows
/// use `(match >> 3) * stride` as the look-back and add two source1 u16 deltas
/// (`0x1103b04..0x1103b44`). Copy runs clone four bytes by byte distance
/// (`0x1103bc0..0x1103be4`).
pub fn transform_tail_u16x2_delta_into(
    out: &mut [u8],
    spec: TransformTailU16x2DeltaSpec<'_>,
) -> Result<TransformTailDeltaUsage, TransformTailDeltaError> {
    let mut cursor =
        transform_tail_delta_cursor_init(spec.block_index, spec.output_stride, spec.out_offset)?;
    let mut match_index = 0usize;
    let mut source0_pos = 0usize;
    let mut source1_pos = 0usize;

    for record in spec.records {
        for _ in 0..record.literal_count {
            let match_entry = *spec
                .matches
                .get(match_index)
                .ok_or(TransformTailDeltaError::MatchTableTooSmall)?;
            let cursor_end = cursor
                .checked_add(4)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            if match_entry != 0 {
                let match_units = (match_entry >> 3) as usize;
                let match_distance = match_units
                    .checked_mul(spec.output_stride)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let source = cursor
                    .checked_sub(match_distance)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                let source_end = source
                    .checked_add(4)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                if out.get(source..source_end).is_none() {
                    return Err(TransformTailDeltaError::MatchBeforeOutput);
                }
                let source1_end = source1_pos
                    .checked_add(4)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let deltas = spec
                    .source1
                    .get(source1_pos..source1_end)
                    .ok_or(TransformTailDeltaError::Source1TooSmall)?;
                let first = read_transform_tail_u16(out, source)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?
                    .wrapping_add(
                        read_transform_tail_u16(deltas, 0)
                            .ok_or(TransformTailDeltaError::Source1TooSmall)?,
                    );
                let second = read_transform_tail_u16(out, source + 2)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?
                    .wrapping_add(
                        read_transform_tail_u16(deltas, 2)
                            .ok_or(TransformTailDeltaError::Source1TooSmall)?,
                    );
                let slot = out
                    .get_mut(cursor..cursor_end)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                write_transform_tail_u16(slot, 0, first);
                write_transform_tail_u16(slot, 2, second);
                source1_pos = source1_end;
            } else {
                let source0_end = source0_pos
                    .checked_add(4)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let bytes = spec
                    .source0
                    .get(source0_pos..source0_end)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                if match_index == 0 {
                    let slot = out
                        .get_mut(cursor..cursor_end)
                        .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                    slot.copy_from_slice(bytes);
                } else {
                    let source = cursor
                        .checked_sub(spec.output_stride)
                        .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                    let source_end = source
                        .checked_add(4)
                        .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                    if out.get(source..source_end).is_none() {
                        return Err(TransformTailDeltaError::MatchBeforeOutput);
                    }
                    let first = read_transform_tail_u16(out, source)
                        .ok_or(TransformTailDeltaError::MatchBeforeOutput)?
                        .wrapping_add(
                            read_transform_tail_u16(bytes, 0)
                                .ok_or(TransformTailDeltaError::Source0TooSmall)?,
                        );
                    let second = read_transform_tail_u16(out, source + 2)
                        .ok_or(TransformTailDeltaError::MatchBeforeOutput)?
                        .wrapping_add(
                            read_transform_tail_u16(bytes, 2)
                                .ok_or(TransformTailDeltaError::Source0TooSmall)?,
                        );
                    let slot = out
                        .get_mut(cursor..cursor_end)
                        .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                    write_transform_tail_u16(slot, 0, first);
                    write_transform_tail_u16(slot, 2, second);
                }
                source0_pos = source0_end;
            }
            cursor = cursor
                .checked_add(spec.output_stride)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            match_index += 1;
        }

        copy_run_units(
            out,
            &mut cursor,
            spec.output_stride,
            4,
            record.back_distance,
            record.copy_count,
        )?;

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
        source2: 0,
        match_entries: match_index,
    })
}

/// Apply the observed two-u16 seed/previous delta tail (`0x1101850`).
///
/// The first emitted literal copies four source0 bytes directly
/// (`0x11018e0..0x11018e4`). Later literals add two source0 u16 deltas to the
/// immediately previous output row (`0x11018a4..0x11018c4`). Copy runs clone
/// four bytes by the record's byte distance (`0x1101920..0x110192c`). The match
/// table at `[x0+0x10]` is not loaded by this writer.
pub fn transform_tail_u16x2_previous_delta_into(
    out: &mut [u8],
    spec: TransformTailU16x2PreviousDeltaSpec<'_>,
) -> Result<TransformTailDeltaUsage, TransformTailDeltaError> {
    let mut cursor =
        transform_tail_delta_cursor_init(spec.block_index, spec.output_stride, spec.out_offset)?;
    let mut written = 0usize;
    let mut source0_pos = 0usize;

    for record in spec.records {
        for _ in 0..record.literal_count {
            let cursor_end = cursor
                .checked_add(4)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            let source0_end = source0_pos
                .checked_add(4)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            let bytes = spec
                .source0
                .get(source0_pos..source0_end)
                .ok_or(TransformTailDeltaError::Source0TooSmall)?;
            if written == 0 {
                let slot = out
                    .get_mut(cursor..cursor_end)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                slot.copy_from_slice(bytes);
            } else {
                let source = cursor
                    .checked_sub(spec.output_stride)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                let source_end = source
                    .checked_add(4)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                if out.get(source..source_end).is_none() {
                    return Err(TransformTailDeltaError::MatchBeforeOutput);
                }
                let first = read_transform_tail_u16(out, source)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?
                    .wrapping_add(
                        read_transform_tail_u16(bytes, 0)
                            .ok_or(TransformTailDeltaError::Source0TooSmall)?,
                    );
                let second = read_transform_tail_u16(out, source + 2)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?
                    .wrapping_add(
                        read_transform_tail_u16(bytes, 2)
                            .ok_or(TransformTailDeltaError::Source0TooSmall)?,
                    );
                let slot = out
                    .get_mut(cursor..cursor_end)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                write_transform_tail_u16(slot, 0, first);
                write_transform_tail_u16(slot, 2, second);
            }
            source0_pos = source0_end;
            cursor = cursor
                .checked_add(spec.output_stride)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            written = written
                .checked_add(1)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
        }

        copy_run_units(
            out,
            &mut cursor,
            spec.output_stride,
            4,
            record.back_distance,
            record.copy_count,
        )?;

        written = written
            .checked_add(usize::from(record.copy_count))
            .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
    }

    Ok(TransformTailDeltaUsage {
        source0: source0_pos,
        source1: 0,
        source2: 0,
        match_entries: 0,
    })
}

#[inline]
fn byte_matched_component(previous: u8, delta: u8, sign_bit: u32) -> u8 {
    let sign_mask = if sign_bit != 0 { u8::MAX } else { 0 };
    (previous ^ sign_mask)
        .wrapping_add(delta)
        .wrapping_add(sign_bit as u8)
}

/// Apply the observed two-u16 direct/matched delta tail (`0x10fdfe0`).
///
/// Zero-match literals copy four source0 bytes directly
/// (`0x10fe020..0x10fe030`). Non-zero match rows use
/// `(match >> 3) * stride` as the look-back and add two little-endian u16
/// deltas from source1 (`0x10fe038..0x10fe078`). Copy runs clone four bytes
/// by byte distance (`0x10fe080..0x10fe0b4`).
pub fn transform_tail_u16x2_direct_delta_into(
    out: &mut [u8],
    spec: TransformTailU16x2DirectDeltaSpec<'_>,
) -> Result<TransformTailDeltaUsage, TransformTailDeltaError> {
    let mut cursor =
        transform_tail_delta_cursor_init(spec.block_index, spec.output_stride, spec.out_offset)?;
    let mut match_index = 0usize;
    let mut source0_pos = 0usize;
    let mut source1_pos = 0usize;

    for record in spec.records {
        for _ in 0..record.literal_count {
            let match_entry = *spec
                .matches
                .get(match_index)
                .ok_or(TransformTailDeltaError::MatchTableTooSmall)?;
            let cursor_end = cursor
                .checked_add(4)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            if match_entry == 0 {
                let source0_end = source0_pos
                    .checked_add(4)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let bytes = spec
                    .source0
                    .get(source0_pos..source0_end)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                let slot = out
                    .get_mut(cursor..cursor_end)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                slot.copy_from_slice(bytes);
                source0_pos = source0_end;
            } else {
                let match_units = (match_entry >> 3) as usize;
                let match_distance = match_units
                    .checked_mul(spec.output_stride)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let source = cursor
                    .checked_sub(match_distance)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                let source_end = source
                    .checked_add(4)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                if out.get(source..source_end).is_none() {
                    return Err(TransformTailDeltaError::MatchBeforeOutput);
                }
                let source1_end = source1_pos
                    .checked_add(4)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let deltas = spec
                    .source1
                    .get(source1_pos..source1_end)
                    .ok_or(TransformTailDeltaError::Source1TooSmall)?;
                let first = read_transform_tail_u16(out, source)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?
                    .wrapping_add(
                        read_transform_tail_u16(deltas, 0)
                            .ok_or(TransformTailDeltaError::Source1TooSmall)?,
                    );
                let second = read_transform_tail_u16(out, source + 2)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?
                    .wrapping_add(
                        read_transform_tail_u16(deltas, 2)
                            .ok_or(TransformTailDeltaError::Source1TooSmall)?,
                    );
                let slot = out
                    .get_mut(cursor..cursor_end)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                write_transform_tail_u16(slot, 0, first);
                write_transform_tail_u16(slot, 2, second);
                source1_pos = source1_end;
            }
            cursor = cursor
                .checked_add(spec.output_stride)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            match_index = match_index
                .checked_add(1)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
        }

        copy_run_units(
            out,
            &mut cursor,
            spec.output_stride,
            4,
            record.back_distance,
            record.copy_count,
        )?;

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
        source2: 0,
        match_entries: match_index,
    })
}

/// Apply the observed three-byte i8/i8/sqrt transform tail (`0x110aac0`).
///
/// Direct literals copy two signed i8 components from source0, reconstruct the
/// third byte as `round(sqrt(127^2 - x^2 - y^2))`, add source1, and apply the
/// source2 sign byte (`0x110ab00..0x110ab4c`). Copy runs clone three bytes by
/// the record back-distance in bytes (`0x110ab60..0x110ab8c`).
pub fn transform_tail_i8x2_normal_into(
    out: &mut [u8],
    spec: TransformTailI8x2NormalSpec<'_>,
) -> Result<TransformTailDeltaUsage, TransformTailDeltaError> {
    const MAX_COMPONENT_SQUARED: i64 = 0x3f01;

    let mut cursor =
        transform_tail_delta_cursor_init(spec.block_index, spec.output_stride, spec.out_offset)?;
    let mut source0_pos = 0usize;
    let mut source1_pos = 0usize;
    let mut source2_pos = 0usize;
    let mut written = 0usize;

    for record in spec.records {
        for _ in 0..record.literal_count {
            let cursor_end = cursor
                .checked_add(3)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            let source0_end = source0_pos
                .checked_add(2)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            let source1_end = source1_pos
                .checked_add(1)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            let source2_end = source2_pos
                .checked_add(1)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            let xy = spec
                .source0
                .get(source0_pos..source0_end)
                .ok_or(TransformTailDeltaError::Source0TooSmall)?;
            let z_delta = *spec
                .source1
                .get(source1_pos)
                .filter(|_| source1_end <= spec.source1.len())
                .ok_or(TransformTailDeltaError::Source1TooSmall)?;
            let z_sign = *spec
                .source2
                .get(source2_pos)
                .filter(|_| source2_end <= spec.source2.len())
                .ok_or(TransformTailDeltaError::Source2TooSmall)?;
            let x = sign_extend_i8(xy[0]);
            let y = sign_extend_i8(xy[1]);
            let remaining = MAX_COMPONENT_SQUARED
                .checked_sub(x * x)
                .and_then(|value| value.checked_sub(y * y))
                .unwrap_or(-1)
                .max(0);
            // Game code uses single-precision `fsqrt s0` + `frinti s0` at 0x110ab38..0x110ab3c.
            let z = (remaining as f32).sqrt().round() as u32;
            let z = z.wrapping_add(u32::from(z_delta));
            let z = if z_sign == 1 { 0u32.wrapping_sub(z) } else { z } as u8;
            let slot = out
                .get_mut(cursor..cursor_end)
                .ok_or(TransformTailDeltaError::OutputTooSmall)?;
            slot[0] = xy[0];
            slot[1] = xy[1];
            slot[2] = z;
            source0_pos = source0_end;
            source1_pos = source1_end;
            source2_pos = source2_end;
            written += 1;
            cursor = cursor
                .checked_add(spec.output_stride)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
        }

        copy_run_units(
            out,
            &mut cursor,
            spec.output_stride,
            3,
            record.back_distance,
            record.copy_count,
        )?;
        written = written
            .checked_add(usize::from(record.copy_count))
            .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
    }

    Ok(TransformTailDeltaUsage {
        source0: source0_pos,
        source1: source1_pos,
        source2: source2_pos,
        match_entries: written,
    })
}

/// Apply the observed three-byte i8/i8/sqrt direct/matched delta tail (`0x110ae30`).
///
/// Direct literals match the `0x110aac0` normal reconstruction: two signed i8
/// components from source0, `round(sqrt(127^2 - x^2 - y^2))`, source1 z
/// adjustment, and source2 sign byte (`0x110aed0..0x110af2c`). Matched literals
/// use `(match >> 3) * stride` as a three-byte look-back, toggle each previous
/// byte from match bits 0..2, add source3 byte deltas, and write three bytes
/// (`0x110af34..0x110afa0`). Copy runs clone three bytes by record byte
/// distance (`0x110ae84..0x110aeb4`).
pub fn transform_tail_i8x3_normal_delta_into(
    out: &mut [u8],
    spec: TransformTailI8x3NormalDeltaSpec<'_>,
) -> Result<TransformTailI8x3NormalDeltaUsage, TransformTailDeltaError> {
    const MAX_COMPONENT_SQUARED: i64 = 0x3f01;

    let mut cursor =
        transform_tail_delta_cursor_init(spec.block_index, spec.output_stride, spec.out_offset)?;
    let mut match_index = 0usize;
    let mut source0_pos = 0usize;
    let mut source1_pos = 0usize;
    let mut source2_pos = 0usize;
    let mut source3_pos = 0usize;

    for record in spec.records {
        for _ in 0..record.literal_count {
            let match_entry = *spec
                .matches
                .get(match_index)
                .ok_or(TransformTailDeltaError::MatchTableTooSmall)?;
            let cursor_end = cursor
                .checked_add(3)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            if match_entry == 0 {
                let source0_end = source0_pos
                    .checked_add(2)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let source1_end = source1_pos
                    .checked_add(1)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let source2_end = source2_pos
                    .checked_add(1)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let xy = spec
                    .source0
                    .get(source0_pos..source0_end)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                let z_delta = *spec
                    .source1
                    .get(source1_pos)
                    .filter(|_| source1_end <= spec.source1.len())
                    .ok_or(TransformTailDeltaError::Source1TooSmall)?;
                let z_sign = *spec
                    .source2
                    .get(source2_pos)
                    .filter(|_| source2_end <= spec.source2.len())
                    .ok_or(TransformTailDeltaError::Source2TooSmall)?;
                let x = sign_extend_i8(xy[0]);
                let y = sign_extend_i8(xy[1]);
                let remaining = MAX_COMPONENT_SQUARED
                    .checked_sub(x * x)
                    .and_then(|value| value.checked_sub(y * y))
                    .unwrap_or(-1)
                    .max(0);
                // Game code uses single-precision `fsqrt s0` + `frinti s0` at 0x110af04..0x110af08.
                let z = (remaining as f32).sqrt().round() as u32;
                let z = z.wrapping_add(u32::from(z_delta));
                let z = if z_sign == 1 { 0u32.wrapping_sub(z) } else { z } as u8;
                let slot = out
                    .get_mut(cursor..cursor_end)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                slot[0] = xy[0];
                slot[1] = xy[1];
                slot[2] = z;
                source0_pos = source0_end;
                source1_pos = source1_end;
                source2_pos = source2_end;
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
                let previous = {
                    let bytes = out
                        .get(source..source_end)
                        .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                    [bytes[0], bytes[1], bytes[2]]
                };
                let source3_end = source3_pos
                    .checked_add(3)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let deltas = spec
                    .source3
                    .get(source3_pos..source3_end)
                    .ok_or(TransformTailDeltaError::Source3TooSmall)?;
                let slot = out
                    .get_mut(cursor..cursor_end)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                slot[0] = byte_matched_component(previous[0], deltas[0], match_entry & 1);
                slot[1] = byte_matched_component(previous[1], deltas[1], (match_entry >> 1) & 1);
                slot[2] = byte_matched_component(previous[2], deltas[2], (match_entry >> 2) & 1);
                source3_pos = source3_end;
            }
            cursor = cursor
                .checked_add(spec.output_stride)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            match_index += 1;
        }

        copy_run_units(
            out,
            &mut cursor,
            spec.output_stride,
            3,
            record.back_distance,
            record.copy_count,
        )?;

        match_index = match_index
            .checked_add(usize::from(record.copy_count))
            .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
        if match_index > spec.matches.len() {
            return Err(TransformTailDeltaError::MatchTableTooSmall);
        }
    }

    Ok(TransformTailI8x3NormalDeltaUsage {
        source0: source0_pos,
        source1: source1_pos,
        source2: source2_pos,
        source3: source3_pos,
        match_entries: match_index,
    })
}

/// Apply the observed packed 10-10-10 direct/delta transform tail (`0x110afb0`).
///
/// Direct literals read two signed 10-bit components from source0, reconstruct
/// the third as `round(sqrt(511^2 - x^2 - y^2))`, add source1, apply the source2
/// sign byte, and pack the result (`0x110b004..0x110b058`). Matched literals use
/// `(match >> 3) * stride` as a packed-row look-back, toggle each 10-bit lane
/// from match bits 0..2, add three source3 u16 deltas, and repack
/// (`0x110b068..0x110b0dc`). Copy runs clone four packed bytes by byte distance
/// (`0x110b0f4..0x110b118`).
pub fn transform_tail_pack10x3_delta_into(
    out: &mut [u8],
    spec: TransformTailPack10x3DeltaSpec<'_>,
) -> Result<TransformTailPack10Usage, TransformTailDeltaError> {
    const MAX_COMPONENT_SQUARED: i64 = 0x3fc01;

    let mut cursor =
        transform_tail_delta_cursor_init(spec.block_index, spec.output_stride, spec.out_offset)?;
    let mut match_index = 0usize;
    let mut source0_pos = 0usize;
    let mut source1_pos = 0usize;
    let mut source2_pos = 0usize;
    let mut source3_pos = 0usize;

    for record in spec.records {
        for _ in 0..record.literal_count {
            let match_entry = *spec
                .matches
                .get(match_index)
                .ok_or(TransformTailDeltaError::MatchTableTooSmall)?;
            let cursor_end = cursor
                .checked_add(4)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            if match_entry == 0 {
                let source0_end = source0_pos
                    .checked_add(4)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let source1_end = source1_pos
                    .checked_add(2)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let source2_end = source2_pos
                    .checked_add(1)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let raw_components = spec
                    .source0
                    .get(source0_pos..source0_end)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                let z_delta = read_transform_tail_u16(spec.source1, source1_pos)
                    .filter(|_| source1_end <= spec.source1.len())
                    .ok_or(TransformTailDeltaError::Source1TooSmall)?;
                let z_sign = *spec
                    .source2
                    .get(source2_pos)
                    .filter(|_| source2_end <= spec.source2.len())
                    .ok_or(TransformTailDeltaError::Source2TooSmall)?;
                let raw_x = u16::from_le_bytes([raw_components[0], raw_components[1]]);
                let raw_y = u16::from_le_bytes([raw_components[2], raw_components[3]]);
                let x = sign_extend_10(raw_x);
                let y = sign_extend_10(raw_y);
                let remaining = MAX_COMPONENT_SQUARED
                    .checked_sub(x * x)
                    .and_then(|value| value.checked_sub(y * y))
                    .unwrap_or(-1)
                    .max(0);
                // Game code uses single-precision `fsqrt s0` + `frinti s0` at 0x110b034..0x110b038.
                let z = (remaining as f32).sqrt().round() as u32;
                let z = z.wrapping_add(u32::from(z_delta));
                let z = if z_sign == 1 { 0u32.wrapping_sub(z) } else { z } & 0x03ff;
                let packed = (z << 20) | (u32::from(raw_y) << 10) | u32::from(raw_x);
                let slot = out
                    .get_mut(cursor..cursor_end)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                write_transform_tail_u32(slot, 0, packed);
                source0_pos = source0_end;
                source1_pos = source1_end;
                source2_pos = source2_end;
            } else {
                let match_units = (match_entry >> 3) as usize;
                let match_distance = match_units
                    .checked_mul(spec.output_stride)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let source = cursor
                    .checked_sub(match_distance)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                let source_end = source
                    .checked_add(4)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let previous = read_transform_tail_u32(out, source)
                    .filter(|_| source_end <= out.len())
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                let source3_end = source3_pos
                    .checked_add(6)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let deltas = spec
                    .source3
                    .get(source3_pos..source3_end)
                    .ok_or(TransformTailDeltaError::Source3TooSmall)?;
                let lane0 = pack10_matched_component(
                    previous & 0x03ff,
                    read_transform_tail_u16(deltas, 0)
                        .ok_or(TransformTailDeltaError::Source3TooSmall)?,
                    match_entry & 1,
                );
                let lane1 = pack10_matched_component(
                    (previous >> 10) & 0x03ff,
                    read_transform_tail_u16(deltas, 2)
                        .ok_or(TransformTailDeltaError::Source3TooSmall)?,
                    (match_entry >> 1) & 1,
                );
                let lane2 = pack10_matched_component(
                    (previous >> 20) & 0x03ff,
                    read_transform_tail_u16(deltas, 4)
                        .ok_or(TransformTailDeltaError::Source3TooSmall)?,
                    (match_entry >> 2) & 1,
                );
                let slot = out
                    .get_mut(cursor..cursor_end)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                write_transform_tail_u32(slot, 0, lane0 | (lane1 << 10) | (lane2 << 20));
                source3_pos = source3_end;
            }
            cursor = cursor
                .checked_add(spec.output_stride)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            match_index += 1;
        }

        copy_run_units(
            out,
            &mut cursor,
            spec.output_stride,
            4,
            record.back_distance,
            record.copy_count,
        )?;

        match_index = match_index
            .checked_add(usize::from(record.copy_count))
            .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
        if match_index > spec.matches.len() {
            return Err(TransformTailDeltaError::MatchTableTooSmall);
        }
    }

    Ok(TransformTailPack10Usage {
        source0: source0_pos,
        source1: source1_pos,
        source2: source2_pos,
        source3: source3_pos,
        match_entries: match_index,
    })
}

fn read_transform_tail_u8x2_at(
    out: &[u8],
    cursor: usize,
) -> Result<[u8; 2], TransformTailDeltaError> {
    let start = cursor
        .checked_sub(1)
        .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
    let end = cursor
        .checked_add(1)
        .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
    let bytes = out
        .get(start..end)
        .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
    Ok([bytes[0], bytes[1]])
}

fn write_transform_tail_u8x2_at(
    out: &mut [u8],
    cursor: usize,
    bytes: [u8; 2],
) -> Result<(), TransformTailDeltaError> {
    let start = cursor
        .checked_sub(1)
        .ok_or(TransformTailDeltaError::OutputTooSmall)?;
    let end = cursor
        .checked_add(1)
        .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
    let slot = out
        .get_mut(start..end)
        .ok_or(TransformTailDeltaError::OutputTooSmall)?;
    slot.copy_from_slice(&bytes);
    Ok(())
}

fn read_transform_tail_u8x3_at(
    out: &[u8],
    cursor: usize,
) -> Result<[u8; 3], TransformTailDeltaError> {
    let end = cursor
        .checked_add(3)
        .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
    let bytes = out
        .get(cursor..end)
        .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
    Ok([bytes[0], bytes[1], bytes[2]])
}

fn write_transform_tail_u8x3_at(
    out: &mut [u8],
    cursor: usize,
    bytes: [u8; 3],
) -> Result<(), TransformTailDeltaError> {
    let end = cursor
        .checked_add(3)
        .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
    let slot = out
        .get_mut(cursor..end)
        .ok_or(TransformTailDeltaError::OutputTooSmall)?;
    slot.copy_from_slice(&bytes);
    Ok(())
}

/// Apply the observed two-byte previous/matched delta transform tail (`0x11033e0`).
///
/// The first zero-match literal seeds two bytes from source0 (`0x11034d4..0x11034e0`).
/// Later zero-match literals add source0 deltas to the previous row
/// (`0x1103488..0x11034c8`), while non-zero matches add source1 deltas to the
/// matched row selected by `(match >> 3) * stride` (`0x1103434..0x1103478`).
/// Copy runs clone two bytes by record byte distance (`0x11034f0..0x1103514`).
pub fn transform_tail_u8x2_delta_into(
    out: &mut [u8],
    spec: TransformTailU8x2DeltaSpec<'_>,
) -> Result<TransformTailDeltaUsage, TransformTailDeltaError> {
    let mut cursor =
        transform_tail_delta_cursor_init(spec.block_index, spec.output_stride, spec.out_offset)?
            .checked_add(1)
            .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
    let mut match_index = 0usize;
    let mut source0_pos = 0usize;
    let mut source1_pos = 0usize;

    for record in spec.records {
        for _ in 0..record.literal_count {
            let match_entry = *spec
                .matches
                .get(match_index)
                .ok_or(TransformTailDeltaError::MatchTableTooSmall)?;
            let bytes = if match_entry != 0 {
                let match_units = (match_entry >> 3) as usize;
                let match_distance = match_units
                    .checked_mul(spec.output_stride)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let source = cursor
                    .checked_sub(match_distance)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                let previous = read_transform_tail_u8x2_at(out, source)?;
                let source1_end = source1_pos
                    .checked_add(2)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let deltas = spec
                    .source1
                    .get(source1_pos..source1_end)
                    .ok_or(TransformTailDeltaError::Source1TooSmall)?;
                source1_pos = source1_end;
                [
                    previous[0].wrapping_add(deltas[0]),
                    previous[1].wrapping_add(deltas[1]),
                ]
            } else if match_index == 0 {
                let source0_end = source0_pos
                    .checked_add(2)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let bytes = spec
                    .source0
                    .get(source0_pos..source0_end)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                source0_pos = source0_end;
                [bytes[0], bytes[1]]
            } else {
                let source = cursor
                    .checked_sub(spec.output_stride)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                let previous = read_transform_tail_u8x2_at(out, source)?;
                let source0_end = source0_pos
                    .checked_add(2)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let deltas = spec
                    .source0
                    .get(source0_pos..source0_end)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                source0_pos = source0_end;
                [
                    previous[0].wrapping_add(deltas[0]),
                    previous[1].wrapping_add(deltas[1]),
                ]
            };
            write_transform_tail_u8x2_at(out, cursor, bytes)?;
            cursor = cursor
                .checked_add(spec.output_stride)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            match_index += 1;
        }

        for _ in 0..record.copy_count {
            let source = cursor
                .checked_sub(record.back_distance)
                .ok_or(TransformTailDeltaError::CopyBeforeOutput)?;
            let bytes = read_transform_tail_u8x2_at(out, source)
                .map_err(|_| TransformTailDeltaError::CopyBeforeOutput)?;
            write_transform_tail_u8x2_at(out, cursor, bytes)?;
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
        source2: 0,
        match_entries: match_index,
    })
}

/// Apply the observed three-byte previous/matched delta transform tail (`0x1103530`).
///
/// The first zero-match literal seeds three source0 bytes (`0x1103690..0x11036a0`).
/// Later zero-match literals add source0 deltas to the previous row
/// (`0x1103638..0x1103684`), while non-zero matches add source1 deltas to the
/// matched row selected by `(match >> 3) * stride` (`0x11035d0..0x1103624`).
/// Copy runs clone three bytes by record byte distance (`0x1103594..0x11035ac`).
pub fn transform_tail_u8x3_delta_into(
    out: &mut [u8],
    spec: TransformTailU8x3DeltaSpec<'_>,
) -> Result<TransformTailDeltaUsage, TransformTailDeltaError> {
    let mut cursor =
        transform_tail_delta_cursor_init(spec.block_index, spec.output_stride, spec.out_offset)?;
    let mut match_index = 0usize;
    let mut source0_pos = 0usize;
    let mut source1_pos = 0usize;

    for record in spec.records {
        for _ in 0..record.literal_count {
            let match_entry = *spec
                .matches
                .get(match_index)
                .ok_or(TransformTailDeltaError::MatchTableTooSmall)?;
            let bytes = if match_entry != 0 {
                let match_units = (match_entry >> 3) as usize;
                let match_distance = match_units
                    .checked_mul(spec.output_stride)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let source = cursor
                    .checked_sub(match_distance)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                let previous = read_transform_tail_u8x3_at(out, source)?;
                let source1_end = source1_pos
                    .checked_add(3)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let deltas = spec
                    .source1
                    .get(source1_pos..source1_end)
                    .ok_or(TransformTailDeltaError::Source1TooSmall)?;
                source1_pos = source1_end;
                [
                    previous[0].wrapping_add(deltas[0]),
                    previous[1].wrapping_add(deltas[1]),
                    previous[2].wrapping_add(deltas[2]),
                ]
            } else if match_index == 0 {
                let source0_end = source0_pos
                    .checked_add(3)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let bytes = spec
                    .source0
                    .get(source0_pos..source0_end)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                source0_pos = source0_end;
                [bytes[0], bytes[1], bytes[2]]
            } else {
                let source = cursor
                    .checked_sub(spec.output_stride)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                let previous = read_transform_tail_u8x3_at(out, source)?;
                let source0_end = source0_pos
                    .checked_add(3)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let deltas = spec
                    .source0
                    .get(source0_pos..source0_end)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                source0_pos = source0_end;
                [
                    previous[0].wrapping_add(deltas[0]),
                    previous[1].wrapping_add(deltas[1]),
                    previous[2].wrapping_add(deltas[2]),
                ]
            };
            write_transform_tail_u8x3_at(out, cursor, bytes)?;
            cursor = cursor
                .checked_add(spec.output_stride)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            match_index += 1;
        }

        for _ in 0..record.copy_count {
            let source = cursor
                .checked_sub(record.back_distance)
                .ok_or(TransformTailDeltaError::CopyBeforeOutput)?;
            let bytes = read_transform_tail_u8x3_at(out, source)
                .map_err(|_| TransformTailDeltaError::CopyBeforeOutput)?;
            write_transform_tail_u8x3_at(out, cursor, bytes)?;
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
        source2: 0,
        match_entries: match_index,
    })
}

/// Apply the observed three-u16 direct/signed-delta transform tail (`0x1100c90`).
///
/// Direct literals copy six source-0 bytes to the strided output
/// (`0x1100cd0..0x1100ce0`). Matched literals use `(match >> 3) * stride` as the
/// look-back distance, flip the high bit of each prior u16 from match bits 0..2,
/// then add three source-1 u16 deltas (`0x1100cfc..0x1100d54`). Copy runs clone
/// six bytes from the record's byte back-distance (`0x1100d70..0x1100d9c`).
pub fn transform_tail_u16x3_delta_into(
    out: &mut [u8],
    spec: TransformTailU16x3DeltaSpec<'_>,
) -> Result<TransformTailDeltaUsage, TransformTailDeltaError> {
    let mut cursor =
        transform_tail_delta_cursor_init(spec.block_index, spec.output_stride, spec.out_offset)?;
    let mut match_index = 0usize;
    let mut source0_pos = 0usize;
    let mut source1_pos = 0usize;

    for record in spec.records {
        for _ in 0..record.literal_count {
            let match_entry = *spec
                .matches
                .get(match_index)
                .ok_or(TransformTailDeltaError::MatchTableTooSmall)?;
            let cursor_end = cursor
                .checked_add(6)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            if match_entry == 0 {
                let source0_end = source0_pos
                    .checked_add(6)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let bytes = spec
                    .source0
                    .get(source0_pos..source0_end)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                let slot = out
                    .get_mut(cursor..cursor_end)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                slot.copy_from_slice(bytes);
                source0_pos = source0_end;
            } else {
                let match_units = (match_entry >> 3) as usize;
                let match_distance = match_units
                    .checked_mul(spec.output_stride)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let source = cursor
                    .checked_sub(match_distance)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                let source_end = source
                    .checked_add(6)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                if out.get(source..source_end).is_none() {
                    return Err(TransformTailDeltaError::MatchBeforeOutput);
                }
                let source1_end = source1_pos
                    .checked_add(6)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let deltas = spec
                    .source1
                    .get(source1_pos..source1_end)
                    .ok_or(TransformTailDeltaError::Source1TooSmall)?;
                let mut values = [0u16; 3];
                for (lane, value) in values.iter_mut().enumerate() {
                    let source_offset = source
                        .checked_add(lane * 2)
                        .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                    let delta_offset = lane * 2;
                    let sign_bit = if (match_entry >> lane) & 1 != 0 {
                        0x8000
                    } else {
                        0
                    };
                    let previous = read_transform_tail_u16(out, source_offset)
                        .ok_or(TransformTailDeltaError::MatchBeforeOutput)?
                        ^ sign_bit;
                    let delta = read_transform_tail_u16(deltas, delta_offset)
                        .ok_or(TransformTailDeltaError::Source1TooSmall)?;
                    *value = previous.wrapping_add(delta);
                }
                let slot = out
                    .get_mut(cursor..cursor_end)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                for (lane, value) in values.into_iter().enumerate() {
                    write_transform_tail_u16(slot, lane * 2, value);
                }
                source1_pos = source1_end;
            }
            cursor = cursor
                .checked_add(spec.output_stride)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            match_index += 1;
        }

        copy_run_units(
            out,
            &mut cursor,
            spec.output_stride,
            6,
            record.back_distance,
            record.copy_count,
        )?;

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
        source2: 0,
        match_entries: match_index,
    })
}

/// Apply the observed three-byte direct/delta transform tail (`0x10fdcf0`).
///
/// Direct literals copy three bytes from source stream 0. Matched literals use
/// the match table's `entry >> 3` distance in vertices, add three source-1
/// deltas to earlier output bytes, then advance the same strided cursor
/// (`0x10fdd20..0x10fdda8`). Copy runs clone prior output bytes by the record's
/// byte distance (`0x10fddac..0x10fdde4`).
pub fn transform_tail_delta3_direct_into(
    out: &mut [u8],
    spec: TransformTailDelta3DirectSpec<'_>,
) -> Result<TransformTailDeltaUsage, TransformTailDeltaError> {
    let mut cursor =
        transform_tail_delta_cursor_init(spec.block_index, spec.output_stride, spec.out_offset)?;
    let mut match_index = 0usize;
    let mut source0_pos = 0usize;
    let mut source1_pos = 0usize;

    for record in spec.records {
        for _ in 0..record.literal_count {
            let match_entry = *spec
                .matches
                .get(match_index)
                .ok_or(TransformTailDeltaError::MatchTableTooSmall)?;
            let cursor_end = cursor
                .checked_add(3)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            if match_entry == 0 {
                let source0_end = source0_pos
                    .checked_add(3)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let bytes = spec
                    .source0
                    .get(source0_pos..source0_end)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                let slot = out
                    .get_mut(cursor..cursor_end)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                slot.copy_from_slice(bytes);
                source0_pos = source0_end;
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
                let base0 = base[0];
                let base1 = base[1];
                let base2 = base[2];
                let source1_end = source1_pos
                    .checked_add(3)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let delta = spec
                    .source1
                    .get(source1_pos..source1_end)
                    .ok_or(TransformTailDeltaError::Source1TooSmall)?;
                let slot = out
                    .get_mut(cursor..cursor_end)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                slot[0] = delta[0].wrapping_add(base0);
                slot[1] = delta[1].wrapping_add(base1);
                slot[2] = delta[2].wrapping_add(base2);
                source1_pos = source1_end;
            }
            cursor = cursor
                .checked_add(spec.output_stride)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            match_index += 1;
        }

        copy_run_units(
            out,
            &mut cursor,
            spec.output_stride,
            3,
            record.back_distance,
            record.copy_count,
        )?;

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
        source2: 0,
        match_entries: match_index,
    })
}

/// Apply the observed four-byte direct/delta transform tail (`0x10fde00`).
///
/// Direct literals copy four bytes from source stream 0 (`0x10fde40..0x10fde44`).
/// Matched literals use the match table's `entry >> 3` distance in vertices,
/// add four source-1 byte deltas to earlier output bytes, and write the four
/// result bytes (`0x10fde58..0x10fdeac`). Copy runs clone four bytes by the
/// record's byte distance (`0x10fdec8..0x10fdef4`).
pub fn transform_tail_delta4_direct_into(
    out: &mut [u8],
    spec: TransformTailDelta4DirectSpec<'_>,
) -> Result<TransformTailDeltaUsage, TransformTailDeltaError> {
    let mut cursor =
        transform_tail_delta_cursor_init(spec.block_index, spec.output_stride, spec.out_offset)?;
    let mut match_index = 0usize;
    let mut source0_pos = 0usize;
    let mut source1_pos = 0usize;

    for record in spec.records {
        for _ in 0..record.literal_count {
            let match_entry = *spec
                .matches
                .get(match_index)
                .ok_or(TransformTailDeltaError::MatchTableTooSmall)?;
            let cursor_end = cursor
                .checked_add(4)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            if match_entry == 0 {
                let source0_end = source0_pos
                    .checked_add(4)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let bytes = spec
                    .source0
                    .get(source0_pos..source0_end)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                let slot = out
                    .get_mut(cursor..cursor_end)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                slot.copy_from_slice(bytes);
                source0_pos = source0_end;
            } else {
                let match_units = (match_entry >> 3) as usize;
                let match_distance = match_units
                    .checked_mul(spec.output_stride)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let source = cursor
                    .checked_sub(match_distance)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                let source_end = source
                    .checked_add(4)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let base = out
                    .get(source..source_end)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                let base0 = base[0];
                let base1 = base[1];
                let base2 = base[2];
                let base3 = base[3];
                let source1_end = source1_pos
                    .checked_add(4)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let delta = spec
                    .source1
                    .get(source1_pos..source1_end)
                    .ok_or(TransformTailDeltaError::Source1TooSmall)?;
                let slot = out
                    .get_mut(cursor..cursor_end)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                slot[0] = delta[0].wrapping_add(base0);
                slot[1] = delta[1].wrapping_add(base1);
                slot[2] = delta[2].wrapping_add(base2);
                slot[3] = delta[3].wrapping_add(base3);
                source1_pos = source1_end;
            }
            cursor = cursor
                .checked_add(spec.output_stride)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            match_index = match_index
                .checked_add(1)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
        }

        copy_run_units(
            out,
            &mut cursor,
            spec.output_stride,
            4,
            record.back_distance,
            record.copy_count,
        )?;

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
        source2: 0,
        match_entries: match_index,
    })
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
    let mut cursor =
        transform_tail_delta_cursor_init(spec.block_index, spec.output_stride, spec.out_offset)?;
    let mut match_index = 0usize;
    let mut source0_pos = 0usize;
    let mut source1_pos = 0usize;
    let mut source2_pos = 0usize;

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

        copy_run_units(
            out,
            &mut cursor,
            spec.output_stride,
            2,
            record.back_distance,
            record.copy_count,
        )?;

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
    let mut cursor =
        transform_tail_delta_cursor_init(spec.block_index, spec.output_stride, spec.out_offset)?;
    let mut match_index = 0usize;
    let mut source0_pos = 0usize;
    let mut source1_pos = 0usize;
    let mut source2_pos = 0usize;

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

        copy_run_units(
            out,
            &mut cursor,
            spec.output_stride,
            3,
            record.back_distance,
            record.copy_count,
        )?;

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
