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

/// Inputs for the one-byte seed/previous-delta transform tail (`0x1101230`).
pub struct TransformTailU8PreviousDeltaSpec<'a> {
    /// Entry high byte (`entry >> 24`): byte distance between consecutive vertices.
    pub output_stride: usize,
    /// Block index at `[x0+0xa0]`, folded into the initial output position.
    pub block_index: usize,
    /// Per-entry output byte offset from `[x0 + current*4 + 0x64]`.
    pub out_offset: usize,
    /// Run/copy records at `x2`.
    pub records: &'a [TransformTailRecord],
    /// Seed and previous-row delta stream at `[x4]`.
    pub source0: &'a [u8],
}

/// Inputs for the three-byte seed/previous-delta transform tail (`0x1101410`).
pub struct TransformTailU8x3PreviousDeltaSpec<'a> {
    /// Entry high byte (`entry >> 24`): byte distance between consecutive vertices.
    pub output_stride: usize,
    /// Block index at `[x0+0xa0]`, folded into the initial output position.
    pub block_index: usize,
    /// Per-entry output byte offset from `[x0 + current*4 + 0x64]`.
    pub out_offset: usize,
    /// Run/copy records at `x2`.
    pub records: &'a [TransformTailRecord],
    /// Seed and previous-row delta stream at `[x4]`.
    pub source0: &'a [u8],
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

/// Inputs for the three-byte transform tail (`0x10fc720`).
pub struct TransformTailCopy3Spec<'a> {
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

/// Inputs for the six-byte transform tail (`0x10fc870`).
pub struct TransformTailCopy6Spec<'a> {
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

/// Inputs for the eight-byte transform tail (`0x10fc920`).
pub struct TransformTailCopy8Spec<'a> {
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

/// Apply the observed three-byte copy transform tail (`0x10fc720`).
///
/// This is the `ldrh`+`ldrb` / `strh`+`strb` sibling of `0x10fc5e0`:
/// literals and copies move three bytes per record unit, while cursor advance
/// and back-distance remain byte counts (`0x10fc760..0x10fc7bc`). The captured
/// population exercises both zero-literal and zero-copy records.
pub fn transform_tail_copy3_into(
    out: &mut [u8],
    spec: TransformTailCopy3Spec<'_>,
) -> Result<usize, TransformTailCopyError> {
    transform_tail_copy_units_into(
        out,
        TransformTailCopyUnitsSpec {
            output_stride: spec.output_stride,
            block_index: spec.block_index,
            out_offset: spec.out_offset,
            records: spec.records,
            source: spec.source,
            unit_size: 3,
            allow_zero_literal: true,
            allow_zero_copy: true,
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

/// Apply the observed six-byte copy transform tail (`0x10fc870`).
///
/// This is the `ldr`+`ldrh` / `str`+`strh` sibling of `0x10fc5e0`:
/// literals and copies move six bytes per record unit, while cursor advance
/// and back-distance remain byte counts (`0x10fc8b0..0x10fc90c`). The captured
/// DirectionZero population exercises zero-literal records but not zero-copy
/// records, so zero-copy shapes remain guarded.
pub fn transform_tail_copy6_into(
    out: &mut [u8],
    spec: TransformTailCopy6Spec<'_>,
) -> Result<usize, TransformTailCopyError> {
    transform_tail_copy_units_into(
        out,
        TransformTailCopyUnitsSpec {
            output_stride: spec.output_stride,
            block_index: spec.block_index,
            out_offset: spec.out_offset,
            records: spec.records,
            source: spec.source,
            unit_size: 6,
            allow_zero_literal: true,
            allow_zero_copy: false,
        },
    )
}

/// Apply the observed eight-byte copy transform tail (`0x10fc920`).
///
/// This is the `ldp`/`stp` sibling of `0x10fc5e0`: literals and copies move
/// two u32 words per record unit, while cursor advance and back-distance remain
/// byte counts (`0x10fc960..0x10fc9a8`). The captured population exercises
/// zero-literal records, but not zero-copy records, so zero-copy shapes are
/// rejected until captured.
pub fn transform_tail_copy8_into(
    out: &mut [u8],
    spec: TransformTailCopy8Spec<'_>,
) -> Result<usize, TransformTailCopyError> {
    transform_tail_copy_units_into(
        out,
        TransformTailCopyUnitsSpec {
            output_stride: spec.output_stride,
            block_index: spec.block_index,
            out_offset: spec.out_offset,
            records: spec.records,
            source: spec.source,
            unit_size: 8,
            allow_zero_literal: true,
            allow_zero_copy: false,
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

/// Inputs for the four-byte delta-match transform tail (`0x10fbee0`).
pub struct TransformTailDelta4Spec<'a> {
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
    /// Direct literal bytes 1, 2, and 3 stream at `[x4+8]`.
    pub source1: &'a [u8],
    /// Matched delta stream at `[x4+0x10]`.
    pub source2: &'a [u8],
}

/// Inputs for the one-byte direct/delta transform tail (`0x10fdb30`).
pub struct TransformTailDelta1DirectSpec<'a> {
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

/// Inputs for the three-byte signed direct/delta transform tail (`0x10ffdb0`).
pub struct TransformTailI8x3DirectDeltaSpec<'a> {
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

/// Inputs for the two-u32 direct/matched delta transform tail (`0x10fe4d0`).
pub struct TransformTailU32x2DeltaSpec<'a> {
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
    /// Direct two-u32 literal source stream at `[x4]`.
    pub source0: &'a [u8],
    /// Matched two-u32 delta stream at `[x4+8]`.
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

/// Inputs for the packed 10-10-10 normal transform tail (`0x110aba0`).
pub struct TransformTailPack10x3NormalSpec<'a> {
    /// Entry high byte (`entry >> 24`): byte distance between consecutive vertices.
    pub output_stride: usize,
    /// Block index at `[x0+0xa0]`, folded into the initial output position.
    pub block_index: usize,
    /// Per-entry output byte offset from `[x0 + current*4 + 0x64]`.
    pub out_offset: usize,
    /// Run/copy records at `x2`.
    pub records: &'a [TransformTailRecord],
    /// Direct source stream with two raw 10-bit components per row at `[x4]`.
    pub source0: &'a [u8],
    /// Direct third-component delta stream at `[x4+8]`.
    pub source1: &'a [u8],
    /// Direct third-component sign-byte stream at `[x4+0x10]`.
    pub source2: &'a [u8],
}

/// Inputs for the f16x3 predictor transform tail (`0x1106250`).
pub struct TransformTailF16x3PredictSpec<'a> {
    /// Entry high byte (`entry >> 24`): byte distance between consecutive vertices.
    pub output_stride: usize,
    /// Block index at `[x0+0xa0]`, folded into the initial output position.
    pub block_index: usize,
    /// Per-entry output byte offset from `[x0 + current*4 + 0x64]`.
    pub out_offset: usize,
    /// Run/copy records at `x2`.
    pub records: &'a [TransformTailRecord],
    /// Predictor table at `[x0+8]`, indexed by emitted vertex.
    pub aux_table: &'a [u64],
    /// Per-lane flag stream at `[x4]`.
    pub source0: &'a [u8],
    /// Exponent-delta stream for zero flags at `[x4+8]`.
    pub source1: &'a [u8],
    /// Exponent-delta stream for non-zero flags at `[x4+0x10]`.
    pub source2: &'a [u8],
    /// Zigzag mantissa deltas for unchanged sign/exponent lanes at `[x4+0x18]`.
    pub source3: &'a [u8],
    /// Direct mantissas for changed sign/exponent lanes at `[x4+0x20]`.
    pub source4: &'a [u8],
}

/// Inputs for the two-u16 f16x3 reference predictor transform tail (`0x1108550`).
pub struct TransformTailU16x2F16x3PredictSpec<'a> {
    /// Entry high byte (`entry >> 24`): byte distance between consecutive vertices.
    pub output_stride: usize,
    /// Block index at `[x0+0xa0]`, folded into the initial output position.
    pub block_index: usize,
    /// Per-entry output byte offset from `[x0 + current*4 + 0x64]`.
    pub out_offset: usize,
    /// Referenced attribute high byte selected by the writer-local five-bit reader.
    pub reference_output_stride: usize,
    /// Referenced attribute byte offset selected by the same reader.
    pub reference_out_offset: usize,
    /// Run/copy records at `x2`.
    pub records: &'a [TransformTailRecord],
    /// Predictor table at `[x0+8]`, indexed by emitted vertex.
    pub aux_table: &'a [u64],
    /// Seed row and zero-aux previous-row u16 delta stream at `[x4]`.
    pub source0: &'a [u8],
    /// Base two-u16 rows for non-zero aux literals at `[x4+8]`.
    pub source1: &'a [u8],
    /// Non-zero aux orientation byte stream at `[x4+0x10]`.
    pub source2: &'a [u8],
}

/// Inputs for the packed 10-10-10 seed/previous/matched delta tail (`0x1103840`).
pub struct TransformTailPack10x3PreviousDeltaSpec<'a> {
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

/// Source and aux-table consumption from the f16x3 predictor tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransformTailF16x3PredictUsage {
    pub source0: usize,
    pub source1: usize,
    pub source2: usize,
    pub source3: usize,
    pub source4: usize,
    pub aux_entries: usize,
}

/// Source and aux-table consumption from the two-u16 f16x3 reference predictor tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransformTailU16x2F16x3PredictUsage {
    pub source0: usize,
    pub source1: usize,
    pub source2: usize,
    pub aux_entries: usize,
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
    Source4TooSmall,
    MatchTableTooSmall,
    AuxTableTooSmall,
    MatchBeforeOutput,
    PredictorBeforeOutput,
    CopyBeforeOutput,
    ArithmeticOverflow,
    UnobservedRecordShape,
    UnobservedPredictorSignZero,
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
fn pack10_lane_delta_component(previous: u32, delta: u16) -> u32 {
    previous.wrapping_add(u32::from(delta)) & 0x03ff
}

#[inline]
fn pack10x3_from_lanes(lane0: u32, lane1: u32, lane2: u32) -> u32 {
    (lane0 & 0x03ff) | ((lane1 & 0x03ff) << 10) | ((lane2 & 0x03ff) << 20)
}

#[inline]
fn pack10x3_direct(raw0: u16, raw1: u16, raw2: u16) -> u32 {
    u32::from(raw0) | (u32::from(raw1) << 10) | (u32::from(raw2) << 20)
}

#[inline]
fn pack10x3_reconstructed_normal(raw_x: u16, raw_y: u16, z_delta: u16, z_sign: u8) -> u32 {
    const MAX_COMPONENT_SQUARED: i64 = 0x3fc01;

    let x = sign_extend_10(raw_x);
    let y = sign_extend_10(raw_y);
    let remaining = MAX_COMPONENT_SQUARED
        .checked_sub(x * x)
        .and_then(|value| value.checked_sub(y * y))
        .unwrap_or(-1)
        .max(0);
    let z = (remaining as f32).sqrt().round() as u32;
    let z = z.wrapping_add(u32::from(z_delta));
    let z = if z_sign == 1 { 0u32.wrapping_sub(z) } else { z } & 0x03ff;
    (z << 20) | (u32::from(raw_y) << 10) | u32::from(raw_x)
}

#[inline]
fn round_shift_to_even(value: u32, shift: u32) -> u32 {
    if shift == 0 {
        return value;
    }
    let keep = value >> shift;
    let halfway = 1u32 << (shift - 1);
    let dropped = value & ((1u32 << shift) - 1);
    if dropped > halfway || (dropped == halfway && keep & 1 != 0) {
        keep + 1
    } else {
        keep
    }
}

#[inline]
fn half_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits & 0x8000) << 16;
    let exponent = u32::from((bits >> 10) & 0x1f);
    let mantissa = u32::from(bits & 0x03ff);
    let out = match exponent {
        0 if mantissa == 0 => sign,
        0 => {
            let mut mant = mantissa;
            let mut exp = -14i32;
            while mant & 0x0400 == 0 {
                mant <<= 1;
                exp -= 1;
            }
            mant &= 0x03ff;
            sign | (((exp + 127) as u32) << 23) | (mant << 13)
        }
        0x1f => sign | 0x7f80_0000 | (mantissa << 13),
        _ => sign | ((exponent + 112) << 23) | (mantissa << 13),
    };
    f32::from_bits(out)
}

#[inline]
fn f32_to_half(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x007f_ffff;

    if exponent == 0xff {
        let payload = if mantissa == 0 {
            0
        } else {
            ((mantissa >> 13) as u16).max(1)
        };
        return sign | 0x7c00 | payload;
    }
    if exponent == 0 {
        return sign;
    }

    let mut half_exponent = exponent - 127 + 15;
    if half_exponent >= 0x1f {
        return sign | 0x7c00;
    }
    if half_exponent <= 0 {
        if half_exponent < -10 {
            return sign;
        }
        let rounded = round_shift_to_even(mantissa | 0x0080_0000, (14 - half_exponent) as u32);
        return sign | rounded.min(0x0400) as u16;
    }

    let rounded = round_shift_to_even(mantissa, 13);
    if rounded == 0x0400 {
        half_exponent += 1;
        if half_exponent >= 0x1f {
            return sign | 0x7c00;
        }
        return sign | ((half_exponent as u16) << 10);
    }

    sign | ((half_exponent as u16) << 10) | rounded as u16
}

#[inline]
fn f16_predict_lane(older: u16, left: u16, right: u16) -> u16 {
    f32_to_half(half_to_f32(left) + half_to_f32(right) - half_to_f32(older))
}

#[inline]
fn f16x3_component(predicted: u16, flag: u8, exponent_delta: u8, mantissa_word: u16) -> u16 {
    let sign = (predicted & 0x8000) ^ (u16::from(flag) << 15);
    let exponent = predicted.wrapping_add(u16::from(exponent_delta) << 10) & 0x7c00;
    let mantissa = if flag == 0 && exponent_delta == 0 {
        predicted.wrapping_add(zigzag10_u16(mantissa_word)) & 0x03ff
    } else {
        mantissa_word
    };
    sign | exponent | mantissa
}

#[inline]
fn zigzag10_u16(value: u16) -> u16 {
    let magnitude = value >> 1;
    if value & 1 == 0 {
        magnitude
    } else {
        0u16.wrapping_sub(magnitude).wrapping_sub(1)
    }
}

fn f16x3_row_at(out: &[u8], offset: usize) -> Result<[u16; 3], TransformTailDeltaError> {
    let end = offset
        .checked_add(6)
        .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
    let row = out
        .get(offset..end)
        .ok_or(TransformTailDeltaError::PredictorBeforeOutput)?;
    Ok([
        read_transform_tail_u16(row, 0).ok_or(TransformTailDeltaError::PredictorBeforeOutput)?,
        read_transform_tail_u16(row, 2).ok_or(TransformTailDeltaError::PredictorBeforeOutput)?,
        read_transform_tail_u16(row, 4).ok_or(TransformTailDeltaError::PredictorBeforeOutput)?,
    ])
}

fn f16x3_predictor_row(
    out: &[u8],
    cursor: usize,
    output_stride: usize,
    emitted: usize,
    aux_entry: u64,
) -> Result<[u16; 3], TransformTailDeltaError> {
    if aux_entry & 0x3f_ffff != 0 {
        let distance0 = (aux_entry & 0x3f_ffff) as usize;
        let distance1 = ((aux_entry >> 22) & 0x1f_ffff) as usize;
        let distance2 = (aux_entry >> 43) as usize;
        let row_offset = |distance: usize| {
            distance
                .checked_mul(output_stride)
                .and_then(|bytes| cursor.checked_sub(bytes))
                .ok_or(TransformTailDeltaError::PredictorBeforeOutput)
        };
        let older = f16x3_row_at(out, row_offset(distance0)?)?;
        let left = f16x3_row_at(out, row_offset(distance1)?)?;
        let right = f16x3_row_at(out, row_offset(distance2)?)?;
        return Ok([
            f16_predict_lane(older[0], left[0], right[0]),
            f16_predict_lane(older[1], left[1], right[1]),
            f16_predict_lane(older[2], left[2], right[2]),
        ]);
    }

    if emitted == 0 {
        Ok([0, 0, 0])
    } else {
        let previous = cursor
            .checked_sub(output_stride)
            .ok_or(TransformTailDeltaError::PredictorBeforeOutput)?;
        f16x3_row_at(out, previous)
    }
}

fn f16x3_row_as_f32(out: &[u8], offset: usize) -> Result<[f32; 3], TransformTailDeltaError> {
    let row = f16x3_row_at(out, offset)?;
    Ok([
        half_to_f32(row[0]),
        half_to_f32(row[1]),
        half_to_f32(row[2]),
    ])
}

fn u16x2_row_as_f32(out: &[u8], offset: usize) -> Result<[f32; 2], TransformTailDeltaError> {
    let end = offset
        .checked_add(4)
        .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
    let row = out
        .get(offset..end)
        .ok_or(TransformTailDeltaError::PredictorBeforeOutput)?;
    Ok([
        f32::from(
            read_transform_tail_u16(row, 0)
                .ok_or(TransformTailDeltaError::PredictorBeforeOutput)?,
        ),
        f32::from(
            read_transform_tail_u16(row, 2)
                .ok_or(TransformTailDeltaError::PredictorBeforeOutput)?,
        ),
    ])
}

fn checked_predictor_lookback(
    cursor: usize,
    distance: usize,
    stride: usize,
) -> Result<usize, TransformTailDeltaError> {
    if distance == 0 {
        return Err(TransformTailDeltaError::PredictorBeforeOutput);
    }
    distance
        .checked_mul(stride)
        .and_then(|bytes| cursor.checked_sub(bytes))
        .ok_or(TransformTailDeltaError::PredictorBeforeOutput)
}

fn round_fcvtns_i32(value: f32) -> i32 {
    value.round_ties_even() as i32
}

struct U16x2F16x3PredictorDeltaSpec {
    cursor: usize,
    output_stride: usize,
    reference_cursor: usize,
    reference_stride: usize,
    distance_a: usize,
    distance_b: usize,
    sign_byte: u8,
}

fn u16x2_f16x3_predictor_delta(
    out: &[u8],
    spec: U16x2F16x3PredictorDeltaSpec,
) -> Result<[i32; 2], TransformTailDeltaError> {
    if spec.sign_byte == 0 {
        return Err(TransformTailDeltaError::UnobservedPredictorSignZero);
    }

    let reference_a = checked_predictor_lookback(
        spec.reference_cursor,
        spec.distance_a,
        spec.reference_stride,
    )?;
    let reference_b = checked_predictor_lookback(
        spec.reference_cursor,
        spec.distance_b,
        spec.reference_stride,
    )?;
    let current_a = checked_predictor_lookback(spec.cursor, spec.distance_a, spec.output_stride)?;
    let current_b = checked_predictor_lookback(spec.cursor, spec.distance_b, spec.output_stride)?;

    let anchor = f16x3_row_as_f32(out, reference_a)?;
    let other = f16x3_row_as_f32(out, reference_b)?;
    let current = f16x3_row_as_f32(out, spec.reference_cursor)?;
    let anchor_uv = u16x2_row_as_f32(out, current_a)?;
    let other_uv = u16x2_row_as_f32(out, current_b)?;

    let reference_delta = [
        other[0] - anchor[0],
        other[1] - anchor[1],
        other[2] - anchor[2],
    ];
    let current_delta = [
        current[0] - anchor[0],
        current[1] - anchor[1],
        current[2] - anchor[2],
    ];
    let denominator = (reference_delta[0] * reference_delta[0]
        + reference_delta[1] * reference_delta[1]
        + reference_delta[2] * reference_delta[2])
        .max(f32::from_bits(0x3580_0000));
    let scale = (reference_delta[0] * current_delta[0]
        + reference_delta[1] * current_delta[1]
        + reference_delta[2] * current_delta[2])
        / denominator;

    let projected = [
        anchor[0] + reference_delta[0] * scale,
        anchor[1] + reference_delta[1] * scale,
        anchor[2] + reference_delta[2] * scale,
    ];
    let interpolated_uv = [
        anchor_uv[0] + (other_uv[0] - anchor_uv[0]) * scale,
        anchor_uv[1] + (other_uv[1] - anchor_uv[1]) * scale,
    ];
    let projected_delta = [
        projected[0] - anchor[0],
        projected[1] - anchor[1],
        projected[2] - anchor[2],
    ];
    let current_len = current_delta[0] * current_delta[0]
        + current_delta[1] * current_delta[1]
        + current_delta[2] * current_delta[2];
    let projected_len = projected_delta[0] * projected_delta[0]
        + projected_delta[1] * projected_delta[1]
        + projected_delta[2] * projected_delta[2];
    let radius = ((current_len - projected_len) / denominator)
        .max(0.0)
        .sqrt();
    let uv_delta = [other_uv[0] - anchor_uv[0], other_uv[1] - anchor_uv[1]];
    let predicted = [
        interpolated_uv[0] + uv_delta[1] * radius,
        interpolated_uv[1] - uv_delta[0] * radius,
    ];

    Ok([
        round_fcvtns_i32(predicted[0]),
        round_fcvtns_i32(predicted[1]),
    ])
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

/// Apply the observed one-byte direct/delta transform tail (`0x10fdb30`).
///
/// Zero-match literals copy one source-0 byte directly (`0x10fdb70..0x10fdb80`).
/// Matched literals use the match table's `entry >> 3` distance in vertices,
/// add one source-1 delta to the earlier output byte, then advance the same
/// strided cursor (`0x10fdb88..0x10fdbb8`). Copy runs clone prior output bytes
/// by the record's byte distance (`0x10fdbc4..0x10fdbe8`).
pub fn transform_tail_delta1_direct_into(
    out: &mut [u8],
    spec: TransformTailDelta1DirectSpec<'_>,
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
            if match_entry == 0 {
                let byte = *spec
                    .source0
                    .get(source0_pos)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                let slot = out
                    .get_mut(cursor)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                *slot = byte;
                source0_pos = source0_pos
                    .checked_add(1)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            } else {
                let match_units = (match_entry >> 3) as usize;
                let match_distance = match_units
                    .checked_mul(spec.output_stride)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let source = cursor
                    .checked_sub(match_distance)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                let base = *out
                    .get(source)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                let delta = *spec
                    .source1
                    .get(source1_pos)
                    .ok_or(TransformTailDeltaError::Source1TooSmall)?;
                let slot = out
                    .get_mut(cursor)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                *slot = delta.wrapping_add(base);
                source1_pos = source1_pos
                    .checked_add(1)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
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
            1,
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

/// Apply the observed three-byte signed direct/delta transform tail (`0x10ffdb0`).
///
/// Zero-match literals copy three source-0 bytes directly
/// (`0x10ffdf0..0x10ffe0c`). Matched literals use the match table's
/// `entry >> 3` distance in vertices, add three source-1 deltas to earlier
/// output bytes, and apply the low three match bits as per-lane sign flips
/// (`0x10ffe14..0x10ffe98`). Copy runs exist in the disassembly
/// (`0x10ffea4..0x10ffed4`) but were not reached by the captured population, so
/// this port rejects nonzero copy counts until that branch has ground truth.
pub fn transform_tail_i8x3_direct_delta_into(
    out: &mut [u8],
    spec: TransformTailI8x3DirectDeltaSpec<'_>,
) -> Result<TransformTailDeltaUsage, TransformTailDeltaError> {
    if spec.records.iter().any(|record| record.copy_count != 0) {
        return Err(TransformTailDeltaError::UnobservedRecordShape);
    }

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
            if match_entry == 0 {
                let source0_end = source0_pos
                    .checked_add(3)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let bytes = spec
                    .source0
                    .get(source0_pos..source0_end)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                write_transform_tail_u8x3_at(out, cursor, [bytes[0], bytes[1], bytes[2]])?;
                source0_pos = source0_end;
            } else {
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
                let bytes = [
                    byte_matched_component(previous[0], deltas[0], match_entry & 1),
                    byte_matched_component(previous[1], deltas[1], (match_entry >> 1) & 1),
                    byte_matched_component(previous[2], deltas[2], (match_entry >> 2) & 1),
                ];
                write_transform_tail_u8x3_at(out, cursor, bytes)?;
                source1_pos = source1_end;
            }
            cursor = cursor
                .checked_add(spec.output_stride)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            match_index += 1;
        }
    }

    Ok(TransformTailDeltaUsage {
        source0: source0_pos,
        source1: source1_pos,
        source2: 0,
        match_entries: match_index,
    })
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

/// Apply the observed one-byte seed/previous delta tail (`0x1101230`).
///
/// The first emitted literal copies one source0 byte directly
/// (`0x11012a8..0x11012b8`). Later literals add source0 byte deltas to the
/// immediately previous output row (`0x1101280..0x110129c`). Copy runs clone one
/// byte by the record's byte distance (`0x11012cc..0x11012f4`). The match table
/// at `[x0+0x10]` is not loaded by this writer.
pub fn transform_tail_u8_previous_delta_into(
    out: &mut [u8],
    spec: TransformTailU8PreviousDeltaSpec<'_>,
) -> Result<TransformTailDeltaUsage, TransformTailDeltaError> {
    let mut cursor =
        transform_tail_delta_cursor_init(spec.block_index, spec.output_stride, spec.out_offset)?;
    let mut written = 0usize;
    let mut source0_pos = 0usize;

    for record in spec.records {
        for _ in 0..record.literal_count {
            let delta = *spec
                .source0
                .get(source0_pos)
                .ok_or(TransformTailDeltaError::Source0TooSmall)?;
            if written == 0 {
                let slot = out
                    .get_mut(cursor)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                *slot = delta;
            } else {
                let source = cursor
                    .checked_sub(spec.output_stride)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                let previous = *out
                    .get(source)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                let slot = out
                    .get_mut(cursor)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                *slot = previous.wrapping_add(delta);
            }
            source0_pos = source0_pos
                .checked_add(1)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
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
            1,
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

/// Apply the observed three-byte seed/previous delta tail (`0x1101410`).
///
/// The first emitted literal copies three source0 bytes directly
/// (`0x11014b0..0x11014bc`). Later literals add three source0 byte deltas to
/// the immediately previous output row (`0x1101460..0x1101494`). Copy runs
/// clone three bytes by the record byte distance (`0x11014e0..0x1101510`).
/// The match table at `[x0+0x10]` is not loaded by this writer.
pub fn transform_tail_u8x3_previous_delta_into(
    out: &mut [u8],
    spec: TransformTailU8x3PreviousDeltaSpec<'_>,
) -> Result<TransformTailDeltaUsage, TransformTailDeltaError> {
    let mut cursor =
        transform_tail_delta_cursor_init(spec.block_index, spec.output_stride, spec.out_offset)?;
    let mut written = 0usize;
    let mut source0_pos = 0usize;

    for record in spec.records {
        for _ in 0..record.literal_count {
            let cursor_end = cursor
                .checked_add(3)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            let source0_end = source0_pos
                .checked_add(3)
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
                    .checked_add(3)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                if out.get(source..source_end).is_none() {
                    return Err(TransformTailDeltaError::MatchBeforeOutput);
                }
                let previous = [
                    *out.get(source)
                        .ok_or(TransformTailDeltaError::MatchBeforeOutput)?,
                    *out.get(source + 1)
                        .ok_or(TransformTailDeltaError::MatchBeforeOutput)?,
                    *out.get(source + 2)
                        .ok_or(TransformTailDeltaError::MatchBeforeOutput)?,
                ];
                let slot = out
                    .get_mut(cursor..cursor_end)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                for lane in 0..3 {
                    slot[lane] = previous[lane].wrapping_add(bytes[lane]);
                }
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

/// Apply the observed two-u32 direct/matched delta tail (`0x10fe4d0`).
///
/// Zero-match literals copy eight source0 bytes directly
/// (`0x10fe510..0x10fe51c`). Non-zero match rows use
/// `(match >> 3) * stride` as the look-back and add two little-endian u32
/// deltas from source1 (`0x10fe528..0x10fe54c`). Copy runs clone eight bytes
/// by byte distance (`0x10fe560..0x10fe598`).
pub fn transform_tail_u32x2_delta_into(
    out: &mut [u8],
    spec: TransformTailU32x2DeltaSpec<'_>,
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
                .checked_add(8)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            if match_entry == 0 {
                let source0_end = source0_pos
                    .checked_add(8)
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
                    .checked_add(8)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                if out.get(source..source_end).is_none() {
                    return Err(TransformTailDeltaError::MatchBeforeOutput);
                }
                let source1_end = source1_pos
                    .checked_add(8)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let deltas = spec
                    .source1
                    .get(source1_pos..source1_end)
                    .ok_or(TransformTailDeltaError::Source1TooSmall)?;
                let first = read_transform_tail_u32(out, source)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?
                    .wrapping_add(
                        read_transform_tail_u32(deltas, 0)
                            .ok_or(TransformTailDeltaError::Source1TooSmall)?,
                    );
                let second = read_transform_tail_u32(out, source + 4)
                    .ok_or(TransformTailDeltaError::MatchBeforeOutput)?
                    .wrapping_add(
                        read_transform_tail_u32(deltas, 4)
                            .ok_or(TransformTailDeltaError::Source1TooSmall)?,
                    );
                let slot = out
                    .get_mut(cursor..cursor_end)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                write_transform_tail_u32(slot, 0, first);
                write_transform_tail_u32(slot, 4, second);
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
            8,
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

/// Apply the observed packed 10-10-10 normal transform tail (`0x110aba0`).
///
/// Direct literals read two signed 10-bit components from source0, reconstruct
/// the third as `round(sqrt(511^2 - x^2 - y^2))`, add source1, apply the source2
/// sign byte, and pack the row (`0x110abe8..0x110ac34` and
/// `0x110ac50..0x110acf4`). Copy runs clone four packed bytes by record byte
/// distance (`0x110ad08..0x110ad24`).
pub fn transform_tail_pack10x3_normal_into(
    out: &mut [u8],
    spec: TransformTailPack10x3NormalSpec<'_>,
) -> Result<TransformTailPack10Usage, TransformTailDeltaError> {
    let mut cursor =
        transform_tail_delta_cursor_init(spec.block_index, spec.output_stride, spec.out_offset)?;
    let mut source0_pos = 0usize;
    let mut source1_pos = 0usize;
    let mut source2_pos = 0usize;
    let mut written = 0usize;

    for record in spec.records {
        for _ in 0..record.literal_count {
            let cursor_end = cursor
                .checked_add(4)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
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
            // Game code uses single-precision `fsqrt s0` + `frinti s0` at
            // 0x110ac14..0x110ac18 and 0x110aca0..0x110acb8.
            let packed = pack10x3_reconstructed_normal(raw_x, raw_y, z_delta, z_sign);
            let slot = out
                .get_mut(cursor..cursor_end)
                .ok_or(TransformTailDeltaError::OutputTooSmall)?;
            write_transform_tail_u32(slot, 0, packed);
            source0_pos = source0_end;
            source1_pos = source1_end;
            source2_pos = source2_end;
            written = written
                .checked_add(1)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            cursor = cursor
                .checked_add(spec.output_stride)
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

    Ok(TransformTailPack10Usage {
        source0: source0_pos,
        source1: source1_pos,
        source2: source2_pos,
        source3: 0,
        match_entries: written,
    })
}

/// Apply the observed f16x3 predictor transform tail (`0x1106250`).
///
/// Literal rows choose a half-float predictor from the aux table at `[x0+8]`:
/// non-zero low 22 bits load three prior rows and compute `row1 + row2 - row0`
/// in f32 before narrowing with `fcvtn` (`0x1106320..0x1106358`), zero aux on
/// the first emitted row seeds zero (`0x11063a8..0x11063b0`), and later zero aux
/// reuses the previous row (`0x110638c..0x11063a4`). Helper `0x110c110`
/// consumes five source streams to update each f16 component, while copy runs
/// clone the six written bytes by record byte distance (`0x11062c0..0x1106304`).
pub fn transform_tail_f16x3_predict_into(
    out: &mut [u8],
    spec: TransformTailF16x3PredictSpec<'_>,
) -> Result<TransformTailF16x3PredictUsage, TransformTailDeltaError> {
    let mut cursor =
        transform_tail_delta_cursor_init(spec.block_index, spec.output_stride, spec.out_offset)?;
    let mut source0_pos = 0usize;
    let mut source1_pos = 0usize;
    let mut source2_pos = 0usize;
    let mut source3_pos = 0usize;
    let mut source4_pos = 0usize;
    let mut emitted = 0usize;
    let mut aux_entries = 0usize;

    for record in spec.records {
        if record.copy_count == 0 {
            return Err(TransformTailDeltaError::UnobservedRecordShape);
        }

        for _ in 0..record.literal_count {
            let aux_entry = *spec
                .aux_table
                .get(emitted)
                .ok_or(TransformTailDeltaError::AuxTableTooSmall)?;
            aux_entries = aux_entries.max(
                emitted
                    .checked_add(1)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?,
            );
            let predicted =
                f16x3_predictor_row(out, cursor, spec.output_stride, emitted, aux_entry)?;
            let cursor_end = cursor
                .checked_add(6)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            let slot = out
                .get_mut(cursor..cursor_end)
                .ok_or(TransformTailDeltaError::OutputTooSmall)?;

            for (lane, &predicted_lane) in predicted.iter().enumerate() {
                let flag = *spec
                    .source0
                    .get(source0_pos)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                source0_pos = source0_pos
                    .checked_add(1)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let exponent_delta = if flag == 0 {
                    let value = *spec
                        .source1
                        .get(source1_pos)
                        .ok_or(TransformTailDeltaError::Source1TooSmall)?;
                    source1_pos = source1_pos
                        .checked_add(1)
                        .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                    value
                } else {
                    let value = *spec
                        .source2
                        .get(source2_pos)
                        .ok_or(TransformTailDeltaError::Source2TooSmall)?;
                    source2_pos = source2_pos
                        .checked_add(1)
                        .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                    value
                };
                let mantissa_word = if flag == 0 && exponent_delta == 0 {
                    let value = read_transform_tail_u16(spec.source3, source3_pos)
                        .ok_or(TransformTailDeltaError::Source3TooSmall)?;
                    source3_pos = source3_pos
                        .checked_add(2)
                        .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                    value
                } else {
                    let value = read_transform_tail_u16(spec.source4, source4_pos)
                        .ok_or(TransformTailDeltaError::Source4TooSmall)?;
                    source4_pos = source4_pos
                        .checked_add(2)
                        .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                    value
                };
                write_transform_tail_u16(
                    slot,
                    lane * 2,
                    f16x3_component(predicted_lane, flag, exponent_delta, mantissa_word),
                );
            }

            emitted = emitted
                .checked_add(1)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            cursor = cursor
                .checked_add(spec.output_stride)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
        }

        copy_run_units(
            out,
            &mut cursor,
            spec.output_stride,
            6,
            record.back_distance,
            record.copy_count,
        )?;
        emitted = emitted
            .checked_add(usize::from(record.copy_count))
            .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
    }

    Ok(TransformTailF16x3PredictUsage {
        source0: source0_pos,
        source1: source1_pos,
        source2: source2_pos,
        source3: source3_pos,
        source4: source4_pos,
        aux_entries,
    })
}

/// Apply the observed two-u16 f16x3 reference predictor tail (`0x1108550`).
///
/// Dispatch setup `0x1108200` supplies three streams: source0 for zero-aux
/// seed/previous deltas, source1 for non-zero-aux base rows, and source2 for
/// the non-zero orientation byte. The writer-local five-bit selector chooses a
/// prior f16x3 attribute; the observed helper slot 3 (`0x11094b0`) projects the
/// current f16x3 row onto two earlier f16x3 rows, interpolates their already
/// written u16x2 rows, and returns two `fcvtns` deltas
/// (`0x11094b0..0x1109638`). Copy runs clone four bytes by record distance
/// (`0x1108660..0x1108670`).
pub fn transform_tail_u16x2_f16x3_predict_into(
    out: &mut [u8],
    spec: TransformTailU16x2F16x3PredictSpec<'_>,
) -> Result<TransformTailU16x2F16x3PredictUsage, TransformTailDeltaError> {
    let mut cursor =
        transform_tail_delta_cursor_init(spec.block_index, spec.output_stride, spec.out_offset)?;
    let reference_base = transform_tail_delta_cursor_init(
        spec.block_index,
        spec.reference_output_stride,
        spec.reference_out_offset,
    )?;
    let mut source0_pos = 0usize;
    let mut source1_pos = 0usize;
    let mut source2_pos = 0usize;
    let mut emitted = 0usize;
    let mut aux_entries = 0usize;

    for record in spec.records {
        if record.literal_count == 0 {
            return Err(TransformTailDeltaError::UnobservedRecordShape);
        }

        for _ in 0..record.literal_count {
            let aux_entry = *spec
                .aux_table
                .get(emitted)
                .ok_or(TransformTailDeltaError::AuxTableTooSmall)?;
            aux_entries = aux_entries.max(
                emitted
                    .checked_add(1)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?,
            );
            let cursor_end = cursor
                .checked_add(4)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;

            if aux_entry != 0 {
                let source1_end = source1_pos
                    .checked_add(4)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let base0 = read_transform_tail_u16(spec.source1, source1_pos)
                    .filter(|_| source1_end <= spec.source1.len())
                    .ok_or(TransformTailDeltaError::Source1TooSmall)?;
                let source1_second = source1_pos
                    .checked_add(2)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let base1 = read_transform_tail_u16(spec.source1, source1_second)
                    .filter(|_| source1_end <= spec.source1.len())
                    .ok_or(TransformTailDeltaError::Source1TooSmall)?;
                let sign_byte = *spec
                    .source2
                    .get(source2_pos)
                    .ok_or(TransformTailDeltaError::Source2TooSmall)?;
                let reference_cursor = emitted
                    .checked_mul(spec.reference_output_stride)
                    .and_then(|offset| reference_base.checked_add(offset))
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let predicted = u16x2_f16x3_predictor_delta(
                    out,
                    U16x2F16x3PredictorDeltaSpec {
                        cursor,
                        output_stride: spec.output_stride,
                        reference_cursor,
                        reference_stride: spec.reference_output_stride,
                        distance_a: ((aux_entry >> 22) & 0x1f_ffff) as usize,
                        distance_b: (aux_entry >> 43) as usize,
                        sign_byte,
                    },
                )?;
                let slot = out
                    .get_mut(cursor..cursor_end)
                    .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                write_transform_tail_u16(slot, 0, base0.wrapping_add(predicted[0] as u16));
                write_transform_tail_u16(slot, 2, base1.wrapping_add(predicted[1] as u16));
                source1_pos = source1_end;
                source2_pos = source2_pos
                    .checked_add(1)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            } else {
                let source0_end = source0_pos
                    .checked_add(4)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let source0 = spec
                    .source0
                    .get(source0_pos..source0_end)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                if emitted == 0 {
                    let seed = read_transform_tail_u32(source0, 0)
                        .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                    let slot = out
                        .get_mut(cursor..cursor_end)
                        .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                    write_transform_tail_u32(slot, 0, seed);
                } else {
                    let previous = cursor
                        .checked_sub(spec.output_stride)
                        .ok_or(TransformTailDeltaError::PredictorBeforeOutput)?;
                    let previous0 = read_transform_tail_u16(out, previous)
                        .ok_or(TransformTailDeltaError::PredictorBeforeOutput)?;
                    let previous_second = previous
                        .checked_add(2)
                        .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                    let previous1 = read_transform_tail_u16(out, previous_second)
                        .ok_or(TransformTailDeltaError::PredictorBeforeOutput)?;
                    let delta0 = read_transform_tail_u16(source0, 0)
                        .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                    let delta1 = read_transform_tail_u16(source0, 2)
                        .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                    let slot = out
                        .get_mut(cursor..cursor_end)
                        .ok_or(TransformTailDeltaError::OutputTooSmall)?;
                    write_transform_tail_u16(slot, 0, previous0.wrapping_add(delta0));
                    write_transform_tail_u16(slot, 2, previous1.wrapping_add(delta1));
                }
                source0_pos = source0_end;
            }

            emitted = emitted
                .checked_add(1)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            cursor = cursor
                .checked_add(spec.output_stride)
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
        emitted = emitted
            .checked_add(usize::from(record.copy_count))
            .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
    }

    Ok(TransformTailU16x2F16x3PredictUsage {
        source0: source0_pos,
        source1: source1_pos,
        source2: source2_pos,
        aux_entries,
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

/// Apply the observed packed 10-10-10 seed/previous/matched delta tail (`0x1103840`).
///
/// The first zero-match literal packs three source0 u16 lanes directly
/// (`0x1103978..0x110398c`). Later zero-match literals add source0 u16 deltas
/// to the previous packed row (`0x1103928..0x1103970`). Non-zero match literals
/// add source1 u16 deltas to the matched packed row selected by `(match >> 3) *
/// stride` (`0x11038d0..0x110391c`). Copy runs clone one packed u32, masked to
/// 30 bits, by the record byte back-distance (`0x1103880..0x11038b8`).
pub fn transform_tail_pack10x3_previous_delta_into(
    out: &mut [u8],
    spec: TransformTailPack10x3PreviousDeltaSpec<'_>,
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
            let packed = if match_entry == 0 {
                let source0_end = source0_pos
                    .checked_add(6)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let deltas = spec
                    .source0
                    .get(source0_pos..source0_end)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                let lane0 = read_transform_tail_u16(deltas, 0)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                let lane1 = read_transform_tail_u16(deltas, 2)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                let lane2 = read_transform_tail_u16(deltas, 4)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                source0_pos = source0_end;
                if match_index == 0 {
                    pack10x3_direct(lane0, lane1, lane2)
                } else {
                    let previous = cursor
                        .checked_sub(spec.output_stride)
                        .and_then(|source| read_transform_tail_u32(out, source))
                        .ok_or(TransformTailDeltaError::MatchBeforeOutput)?;
                    pack10x3_from_lanes(
                        pack10_lane_delta_component(previous & 0x03ff, lane0),
                        pack10_lane_delta_component((previous >> 10) & 0x03ff, lane1),
                        pack10_lane_delta_component((previous >> 20) & 0x03ff, lane2),
                    )
                }
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
                let source1_end = source1_pos
                    .checked_add(6)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let deltas = spec
                    .source1
                    .get(source1_pos..source1_end)
                    .ok_or(TransformTailDeltaError::Source1TooSmall)?;
                source1_pos = source1_end;
                pack10x3_from_lanes(
                    pack10_lane_delta_component(
                        previous & 0x03ff,
                        read_transform_tail_u16(deltas, 0)
                            .ok_or(TransformTailDeltaError::Source1TooSmall)?,
                    ),
                    pack10_lane_delta_component(
                        (previous >> 10) & 0x03ff,
                        read_transform_tail_u16(deltas, 2)
                            .ok_or(TransformTailDeltaError::Source1TooSmall)?,
                    ),
                    pack10_lane_delta_component(
                        (previous >> 20) & 0x03ff,
                        read_transform_tail_u16(deltas, 4)
                            .ok_or(TransformTailDeltaError::Source1TooSmall)?,
                    ),
                )
            };
            let slot = out
                .get_mut(cursor..cursor_end)
                .ok_or(TransformTailDeltaError::OutputTooSmall)?;
            write_transform_tail_u32(slot, 0, packed);
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
                .checked_add(4)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            let cursor_end = cursor
                .checked_add(4)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            let packed = read_transform_tail_u32(out, source)
                .filter(|_| source_end <= out.len())
                .ok_or(TransformTailDeltaError::CopyBeforeOutput)?
                & 0x3fff_ffff;
            let slot = out
                .get_mut(cursor..cursor_end)
                .ok_or(TransformTailDeltaError::OutputTooSmall)?;
            write_transform_tail_u32(slot, 0, packed);
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
                // Game code uses single-precision `fsqrt s0` + `frinti s0` at 0x110b034..0x110b038.
                let packed = pack10x3_reconstructed_normal(raw_x, raw_y, z_delta, z_sign);
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

/// Apply the observed four-byte delta-match transform tail (`0x10fbee0`).
///
/// Direct literals use source streams 0 and 1: bytes 1, 2, and 3 are copied
/// from source1, and byte 0 is `source0 - byte1 - byte2 - byte3 - 1`.
/// Matched literals use the match table's `entry >> 3` distance in vertices,
/// add four source-2 deltas to earlier output bytes, and then advance the same
/// strided cursor (`0x10fbf30..0x10fbfdc`). Copy runs clone four bytes by the
/// record's byte distance (`0x10fbfe4..0x10fc014`).
pub fn transform_tail_delta4_into(
    out: &mut [u8],
    spec: TransformTailDelta4Spec<'_>,
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
                .checked_add(4)
                .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
            let value = if match_entry == 0 {
                let source1_end = source1_pos
                    .checked_add(3)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let tail = spec
                    .source1
                    .get(source1_pos..source1_end)
                    .ok_or(TransformTailDeltaError::Source1TooSmall)?;
                let first_raw = *spec
                    .source0
                    .get(source0_pos)
                    .ok_or(TransformTailDeltaError::Source0TooSmall)?;
                source0_pos += 1;
                source1_pos = source1_end;
                [
                    first_raw
                        .wrapping_sub(tail[0])
                        .wrapping_sub(tail[1])
                        .wrapping_sub(tail[2])
                        .wrapping_sub(1),
                    tail[0],
                    tail[1],
                    tail[2],
                ]
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
                let source2_end = source2_pos
                    .checked_add(4)
                    .ok_or(TransformTailDeltaError::ArithmeticOverflow)?;
                let delta = spec
                    .source2
                    .get(source2_pos..source2_end)
                    .ok_or(TransformTailDeltaError::Source2TooSmall)?;
                source2_pos = source2_end;
                [
                    delta[0].wrapping_add(base[0]),
                    delta[1].wrapping_add(base[1]),
                    delta[2].wrapping_add(base[2]),
                    delta[3].wrapping_add(base[3]),
                ]
            };
            let slot = out
                .get_mut(cursor..cursor_end)
                .ok_or(TransformTailDeltaError::OutputTooSmall)?;
            slot.copy_from_slice(&value);
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
