//! Ignored Phase 1 MeshCodec model-corpus decode sweep.
//!
//! This is a robustness/structural gate, not a byte-exact oracle beyond the
//! three `mc_vertex_decode_oracle` fixtures. Point `MC_MODEL_CORPUS` at a TotK
//! `romfs/Model` directory and run with:
//!
//! `cargo test --test mc_vertex_decode_sweep -- --ignored --nocapture`

use std::{
    collections::BTreeMap,
    env, fs,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
};

use nx_layout_toolbox::{
    mc::{
        extract,
        geometry::{
            byte_group_read, decode_raw_window, decode_zstd_window,
            decode_zstd_window_with_history, parse_sub_block_header, parse_super_block_trailer,
            state0_table_builder, vertex_attribute_driver_setup, vertex_attribute_writer_loop_step,
            vertex_kernel_state4_entry_from_table, vertex_match_table, ByteGroupReadSpec,
            ByteGroupReadState, ByteGroupTransformState, ForwardReader, RansStateBuffer,
            RansThreeLaneReader, TableBuild, VertexAttributeDriverError,
            VertexAttributeDriverState, VertexAttributeWriterLoopError, VertexAttributeWriterTable,
            VertexKernelState4EntrySpec, VertexMatchTableSpec, VertexMatchTableState,
        },
        read_mc, read_mesh_section, MeshSection,
    },
    meshopt,
};

const MESH_INFO_LEN: usize = 0x120;

#[test]
#[ignore = "requires MC_MODEL_CORPUS pointing at TotK romfs/Model"]
fn model_corpus_decode_is_structurally_consistent() {
    let Some(root) = env::var_os("MC_MODEL_CORPUS").map(PathBuf::from) else {
        eprintln!("skipping (set MC_MODEL_CORPUS to a TotK romfs/Model directory)");
        return;
    };
    if !root.is_dir() {
        panic!("MC_MODEL_CORPUS is not a directory: {}", root.display());
    }

    let mut seen = 0usize;
    let mut no_mesh = 0usize;
    let mut clean = 0usize;
    let mut failures = Vec::new();
    let mut failure_classes = BTreeMap::<String, usize>::new();

    for entry in walkdir::WalkDir::new(&root).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !is_bfres_mc(path) {
            continue;
        }
        seen += 1;
        let result = panic::catch_unwind(AssertUnwindSafe(|| sweep_one(path)));
        match result {
            Ok(Ok(OneResult::NoMesh)) => {
                no_mesh += 1;
            }
            Ok(Ok(OneResult::Decoded)) => {
                clean += 1;
            }
            Ok(Err(error)) => {
                *failure_classes.entry(classify_failure(&error)).or_default() += 1;
                failures.push(format!("{}: {error}", path.display()));
            }
            Err(_) => {
                *failure_classes.entry("panic".to_string()).or_default() += 1;
                failures.push(format!("{}: panic", path.display()));
            }
        }
        if seen.is_multiple_of(1000) {
            eprintln!(
                "  ... scanned {seen} .bfres.mc files, no-mesh={no_mesh}, clean={clean}, failures={}",
                failures.len()
            );
        }
    }

    eprintln!(
        "MeshCodec model sweep: seen={seen}, no-mesh={no_mesh}, decoded-clean={clean}, failures={}",
        failures.len()
    );
    if !failures.is_empty() {
        eprintln!("MeshCodec failure classes:");
        for (class, count) in sorted_failure_classes(&failure_classes) {
            eprintln!("  {class} -> {count}");
        }
        for failure in failures.iter().take(40) {
            eprintln!("FAIL: {failure}");
        }
        panic!(
            "MeshCodec model sweep found {} failure(s); first failure: {}",
            failures.len(),
            failures[0]
        );
    }
}

fn classify_failure(error: &str) -> String {
    if error.starts_with("unexpected super-block trailer") {
        return "unexpected super-block trailer".to_string();
    }
    if error.contains("FMSH sub-stream sizes") {
        return "FMSH sub-stream size mismatch".to_string();
    }
    if error.contains("history-backed zstd tail at") {
        return "history-backed zstd tail decode error".to_string();
    }
    for marker in [
        "UnobservedDispatch",
        "UnobservedDecisionBit",
        "UnobservedFirstLeafUnary",
        "UnobservedContinuationUnary",
        "UnobservedSubmeshCount",
    ] {
        if let Some(class) = extract_balanced(error, marker, '(', ')') {
            return class;
        }
    }
    for marker in ["UnobservedWindowFlag", "UnobservedContinuationModeKind"] {
        if let Some(class) = extract_balanced(error, marker, '{', '}') {
            return class;
        }
    }
    for marker in [
        "UnobservedSelector2RawWindow",
        "UnobservedSelector2MultiWindow",
        "UnobservedSelector2HistoryWrap",
        "UnobservedSelector1LargeWindow",
        "UnobservedTailSelector",
        "UnobservedTailOnlyCount",
        "UnobservedShortActiveCount",
        "UnobservedZeroTailBitstream",
        "UnobservedZeroTableByteCount",
        "UnobservedZeroTableGroupWidth",
        "UnobservedZeroSetupCountBits",
    ] {
        if error.contains(marker) {
            return marker.to_string();
        }
    }
    if error.contains("ByteSegmentLoop(UnobservedMode2Segment)") {
        return "ByteSegmentLoop(UnobservedMode2Segment)".to_string();
    }
    if error.contains("SegmentLoop(UnobservedMode1Segment)") {
        return "SegmentLoop(UnobservedMode1Segment)".to_string();
    }
    if error.contains("FrequencyMassMismatch") {
        return with_loop_context(error, "FrequencyMassMismatch");
    }
    if error.contains("TableCountExceedsMass") {
        return with_loop_context(error, "TableCountExceedsMass");
    }
    if error.contains("RleValueTooLarge") {
        return with_loop_context(error, "RleValueTooLarge");
    }
    if let Some(class) = extract_balanced(error, "UnsupportedLog", '(', ')') {
        return with_loop_context(error, &class);
    }
    if error.contains("MalformedTable") {
        return with_loop_context(error, "MalformedTable");
    }
    if error.contains("PayloadTooSmall") {
        return with_loop_context(error, "PayloadTooSmall");
    }
    if let Some(_class) = extract_balanced(error, "ExpansionCodeTooLarge", '(', ')') {
        return "ExpansionCodeTooLarge".to_string();
    }
    if error.contains("MatchIndexOutOfBounds") {
        return "MatchIndexOutOfBounds".to_string();
    }
    if error.contains("SplitExceedsWrapperReturn") {
        return "SplitExceedsWrapperReturn".to_string();
    }
    if error.contains("WidthCombinerUnusedInput") {
        return "WidthCombinerUnusedInput".to_string();
    }
    for marker in [
        "Selector2ZstdDecode",
        "Selector2VarintTooLong",
        "Selector2OutputSizeMismatch",
        "StreamTooShort",
        "OutputTooSmall",
        "ArithmeticOverflow",
        "HistoryOutOfBounds",
        "VarintTooLong",
        "Copy(UnobservedRecordShape)",
        "Dispatch(Decode(ProdTooSmall))",
    ] {
        if error.contains(marker) {
            return marker.to_string();
        }
    }
    for marker in [
        "ByteSegmentLoop",
        "SegmentLoop",
        "SegmentDispatchBytes",
        "SegmentDispatch",
        "ByteGroupRead",
        "WidthCombiner",
    ] {
        if let Some(class) = extract_balanced(error, marker, '(', ')') {
            return class;
        }
    }
    if error.contains("tail fills through") {
        return "state-2 zstd tail length mismatch".to_string();
    }
    if error.contains("index decode count") {
        return if error.contains("must be a multiple of 3") {
            "index decode count not multiple of three".to_string()
        } else if error.contains("Truncated") {
            "index decode truncated stream".to_string()
        } else {
            "index decode error".to_string()
        };
    }
    error.to_string()
}

fn with_loop_context(error: &str, leaf: &str) -> String {
    if error.contains("ByteSegmentLoop(") {
        format!("ByteSegmentLoop({leaf})")
    } else if error.contains("SegmentLoop(") {
        format!("SegmentLoop({leaf})")
    } else if error.contains("ByteGroupRead(") {
        format!("ByteGroupRead({leaf})")
    } else if error.contains("SegmentDescriptor(") {
        format!("SegmentDescriptor({leaf})")
    } else {
        leaf.to_string()
    }
}

fn extract_balanced(error: &str, marker: &str, open: char, close: char) -> Option<String> {
    let marker_start = error.find(marker)?;
    let after_marker = &error[marker_start + marker.len()..];
    let open_relative = after_marker.find(open)?;
    let open_index = marker_start + marker.len() + open_relative;
    let mut depth = 0usize;
    for (relative, ch) in error[open_index..].char_indices() {
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                let end = open_index + relative + ch.len_utf8();
                return Some(error[marker_start..end].to_string());
            }
        }
    }
    None
}

fn sorted_failure_classes(classes: &BTreeMap<String, usize>) -> Vec<(&str, usize)> {
    let mut sorted = classes
        .iter()
        .map(|(class, count)| (class.as_str(), *count))
        .collect::<Vec<_>>();
    sorted.sort_by(|(left_class, left_count), (right_class, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left_class.cmp(right_class))
    });
    sorted
}

#[test]
fn failure_classification_groups_common_phase1_gaps() {
    assert_eq!(
        classify_failure(
            "state-5 writer loop: Interstage { index: 2, error: UnobservedDispatch(35) }"
        ),
        "UnobservedDispatch(35)"
    );
    assert_eq!(
        classify_failure(
            "0x10f90d4 state-4 entry: UnobservedWindowFlag { window: \"data\", flag: 0 }"
        ),
        "UnobservedWindowFlag { window: \"data\", flag: 0 }"
    );
    assert_eq!(
        classify_failure(
            "state-5 writer loop: Interstage { index: 1, error: ByteGroupRead { index: 1, error: ByteSegmentLoop(UnobservedMode2Segment) } }"
        ),
        "ByteSegmentLoop(UnobservedMode2Segment)"
    );
    assert_eq!(
        classify_failure(
            "index decode count 68: Invalid(\"index_count 68 must be a multiple of 3\")"
        ),
        "index decode count not multiple of three"
    );
    assert_eq!(
        classify_failure("unexpected super-block trailer (170097,64152)"),
        "unexpected super-block trailer"
    );
    assert_eq!(
        classify_failure("0x11104d0 stream 2: ByteSegmentLoop(Descriptor(Mode0(FrequencyMassMismatch { expected: 1024, actual: 525312 })))"),
        "ByteSegmentLoop(FrequencyMassMismatch)"
    );
    assert_eq!(
        classify_failure("0x11104d0 stream 0: SegmentDescriptor(Mode1(TableCountExceedsMass { count: 64, mass: 16 }))"),
        "SegmentDescriptor(TableCountExceedsMass)"
    );
    assert_eq!(
        classify_failure("0x11106d0 match table: ExpansionCodeTooLarge(108)"),
        "ExpansionCodeTooLarge"
    );
    assert_eq!(
        classify_failure(
            "history-backed zstd tail at P+10018: Truncated { what: \"literals section\", needed: 83 }"
        ),
        "history-backed zstd tail decode error"
    );
    assert_eq!(
        classify_failure("state-5 writer loop: Interstage { index: 2, error: SplitExceedsWrapperReturn { dispatch: 81, split: 9605, wrapper_ret: 112 } }"),
        "SplitExceedsWrapperReturn"
    );

    let mut counts = BTreeMap::new();
    counts.insert("z".to_string(), 1);
    counts.insert("a".to_string(), 2);
    counts.insert("b".to_string(), 2);
    assert_eq!(
        sorted_failure_classes(&counts),
        vec![("a", 2), ("b", 2), ("z", 1)]
    );
}

enum OneResult {
    NoMesh,
    Decoded,
}

struct VertexBufBDecode {
    bytes: Vec<u8>,
    after_vertex: ByteGroupReadState,
}

struct IndexDecodeState {
    buf_a: Vec<u8>,
    code: Vec<u8>,
    data: Vec<u8>,
    code_off: usize,
    data_off: usize,
}

fn sweep_one(path: &Path) -> Result<OneResult, String> {
    let bytes = fs::read(path).map_err(|error| format!("read: {error}"))?;
    let mc = read_mc(&bytes).map_err(|error| format!("read_mc: {error}"))?;
    let Some(section) = read_mesh_section(&mc).map_err(|error| format!("FMSH: {error}"))? else {
        return Ok(OneResult::NoMesh);
    };
    if section.first_chunk.kind != 2 {
        return Err(format!(
            "unsupported first chunk kind {}",
            section.first_chunk.kind
        ));
    }
    let stream = mc.compressed_stream();
    let payload_end = section
        .payload_offset
        .checked_add(section.compressed_size as usize)
        .ok_or_else(|| "payload offset overflow".to_string())?;
    let payload = stream
        .get(section.payload_offset..payload_end)
        .ok_or_else(|| "FMSH payload out of stream bounds".to_string())?;

    let decoded_b = decode_buf_b_from_payload(&section, payload)?;
    if decoded_b.bytes.len() != section.buf_b_size as usize {
        return Err(format!(
            "bufB decoded length {} != FMSH buf_b_size {}",
            decoded_b.bytes.len(),
            section.buf_b_size
        ));
    }

    let mut index_state = decode_index_streams(&section, payload)?;
    let _next_fwd =
        append_later_index_subblocks(&section, payload, &decoded_b.after_vertex, &mut index_state)?;
    if index_state.buf_a.len() > section.buf_a_size as usize {
        return Err(format!(
            "decoded bufA {} exceeds FMSH buf_a_size {}",
            index_state.buf_a.len(),
            section.buf_a_size
        ));
    }
    if index_state.code_off > index_state.code.len()
        || index_state.data_off > index_state.data.len()
    {
        return Err("index stream cursor exceeded decoded stream length".to_string());
    }

    let bfres = extract(&mc).map_err(|error| format!("extract BFRES: {error}"))?;
    let info_start = bfres.len();
    let buf_a_start = info_start
        .checked_add(MESH_INFO_LEN)
        .ok_or_else(|| "mesh info offset overflow".to_string())?;
    let buf_b_start = align_up(
        buf_a_start
            .checked_add(section.buf_a_size as usize)
            .ok_or_else(|| "bufA end overflow".to_string())?,
        section.align_b as usize,
    )?;
    let decoded_end = buf_b_start
        .checked_add(section.buf_b_size as usize)
        .ok_or_else(|| "bufB end overflow".to_string())?;
    if decoded_end > mc.decompressed_size() {
        return Err(format!(
            "assembled end {decoded_end} exceeds MCPK capacity {}",
            mc.decompressed_size()
        ));
    }
    if !mc
        .decompressed_size()
        .is_multiple_of(1usize << mc.header.alignment_shift())
    {
        return Err(format!(
            "MCPK decompressed size {} is not aligned to 1<<{}",
            mc.decompressed_size(),
            mc.header.alignment_shift()
        ));
    }
    Ok(OneResult::Decoded)
}

fn decode_buf_b_from_payload(
    section: &MeshSection,
    payload: &[u8],
) -> Result<VertexBufBDecode, String> {
    let (_header, table, mut byte_state) =
        state4_entry_from_payload(payload, section.first_chunk.sub_a_size as usize)?;
    let setup = state4_setup_streams(&mut byte_state, payload)?;
    let mut match_state = VertexMatchTableState {
        base: 0,
        limit: 0,
        mask: observed_match_history_mask(table.w8)
            .ok_or_else(|| format!("vertex count {} cannot derive match history mask", table.w8))?,
    };
    let matches = vertex_match_table(VertexMatchTableSpec {
        count: table.w8 as usize,
        processed_vertices: 0,
        counts: setup.counts,
        state: &mut match_state,
        stream0: &setup.streams[0],
        stream1: &setup.streams[1],
        stream2: &setup.streams[2],
        stream3: &setup.stream3_with_slop,
    })
    .map_err(|error| format!("0x11106d0 match table: {error:?}"))?;

    let mut driver = VertexAttributeDriverState {
        current_attribute: 0,
        processed_vertices: 0,
        vertex_count: table.w8,
        block_limit: 0x8000,
        transform_state: ByteGroupTransformState {
            mode: 2,
            count_bits: 0,
            record_count: 0,
            second_count: 0,
            third_count: 0,
            tail_count: 0,
        },
        byte_state,
    };
    vertex_attribute_driver_setup(
        &mut driver.transform_state,
        &mut driver.byte_state,
        payload,
        table.entries.len(),
    )
    .map_err(|error| format!("0x10fafe0 setup: {error:?}"))?;

    let mut out = vec![0u8; section.buf_b_size as usize];
    loop {
        match vertex_attribute_writer_loop_step(
            &mut out,
            &mut driver,
            &table,
            payload,
            VertexAttributeWriterTable {
                matches: &matches,
                block_index: 0,
            },
        ) {
            Ok(_) => {}
            Err(VertexAttributeWriterLoopError::Driver(
                VertexAttributeDriverError::NoAttributesRemaining { .. },
            )) => break,
            Err(error) => return Err(format!("state-5 writer loop: {error:?}")),
        }
    }

    let byte_group_len = table.byte_group_total as usize;
    if byte_group_len > out.len() {
        return Err(format!(
            "byte-group length {byte_group_len} exceeds bufB {}",
            out.len()
        ));
    }
    if byte_group_len < out.len() {
        let mut index_state = decode_index_streams(section, payload)?;
        let tail_fwd =
            append_later_index_subblocks(section, payload, &driver.byte_state, &mut index_state)?;
        let (decoded_tail, _next_pos) =
            decode_zstd_window_with_history(payload, tail_fwd, &out[..byte_group_len])
                .map_err(|error| format!("history-backed zstd tail at P+{tail_fwd}: {error:?}"))?;
        let tail_end = byte_group_len
            .checked_add(decoded_tail.len())
            .ok_or_else(|| "tail length overflow".to_string())?;
        if tail_end != out.len() {
            return Err(format!(
                "tail fills through {tail_end}, expected bufB {}",
                out.len()
            ));
        }
        out[byte_group_len..].copy_from_slice(&decoded_tail);
    }

    Ok(VertexBufBDecode {
        bytes: out,
        after_vertex: driver.byte_state,
    })
}

#[derive(Debug)]
struct State4Setup {
    counts: [usize; 4],
    streams: [Vec<u8>; 4],
    stream3_with_slop: Vec<u8>,
}

fn state4_entry_from_payload(
    payload: &[u8],
    sub_a_size: usize,
) -> Result<
    (
        nx_layout_toolbox::mc::geometry::SubBlockHeader,
        TableBuild,
        ByteGroupReadState,
    ),
    String,
> {
    let (trailer0, trailer1, pos) = parse_super_block_trailer(payload);
    if (trailer0, trailer1) != (0, 0) {
        return Err(format!(
            "unexpected super-block trailer ({trailer0},{trailer1})"
        ));
    }
    let mut forward = ForwardReader::new(payload, pos);
    let sub_block_count = forward.varint();
    let header =
        parse_sub_block_header(&mut forward).ok_or_else(|| "empty first sub-block".to_string())?;
    let table = state0_table_builder(
        payload,
        forward.pos,
        sub_a_size
            .checked_sub(8)
            .ok_or_else(|| "sub-A smaller than reverse seed".to_string())?,
        0,
        0,
        7,
    );
    let sub_b_tail = payload
        .len()
        .checked_sub(8)
        .ok_or_else(|| "payload smaller than reverse-B seed".to_string())?;
    let mut byte_state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: table.rev_ptr,
            acc: table.rev_acc,
            bitpos: table.rev_bitpos,
        },
        mode1_extra_readers: [zero_reader_at(sub_a_size), zero_reader_at(sub_b_tail)],
        stream_pos: table.fwd,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    };
    let entry = vertex_kernel_state4_entry_from_table(
        payload,
        &mut byte_state,
        VertexKernelState4EntrySpec {
            first_header: header,
            first_table: &table,
            remaining_subblocks: (sub_block_count as usize).saturating_sub(1),
            reverse_mode: u32::MAX,
        },
    )
    .map_err(|error| format!("0x10f90d4 state-4 entry: {error:?}"))?;
    if byte_state.reader != entry.reader || byte_state.stream_pos != entry.stream_pos {
        return Err("state-4 entry did not update byte-state consistently".to_string());
    }
    Ok((header, table, byte_state))
}

fn state4_setup_streams(
    byte_state: &mut ByteGroupReadState,
    payload: &[u8],
) -> Result<State4Setup, String> {
    let repeated_count = read_msb_varint(payload, &mut byte_state.stream_pos)?;
    let mut counts = [0usize, repeated_count, repeated_count, 0usize];
    if repeated_count >= 1 {
        counts[0] = read_msb_varint(payload, &mut byte_state.stream_pos)?;
        counts[3] = read_msb_varint(payload, &mut byte_state.stream_pos)?;
    }

    let mut streams: [Vec<u8>; 4] = Default::default();
    let mut stream3_with_slop = Vec::new();
    for index in 0..4 {
        let count = counts[index];
        if count == 0 {
            continue;
        }
        let stream_start = byte_state.stream_pos;
        let read = byte_group_read(
            byte_state,
            ByteGroupReadSpec {
                payload,
                element_shift: 0,
                group_stride: 1,
                count,
            },
        )
        .map_err(|error| format!("0x11104d0 stream {index}: {error:?}"))?;
        if index == 3 {
            let slop_end = stream_start
                .checked_add(count)
                .and_then(|end| end.checked_add(16))
                .ok_or_else(|| "0x11104d0 stream 3 slop overflow".to_string())?;
            stream3_with_slop = payload
                .get(stream_start..slop_end)
                .ok_or_else(|| "0x11104d0 stream 3 slop exceeds payload".to_string())?
                .to_vec();
        }
        streams[index] = read.bytes;
    }

    Ok(State4Setup {
        counts,
        streams,
        stream3_with_slop,
    })
}

fn decode_index_streams(section: &MeshSection, payload: &[u8]) -> Result<IndexDecodeState, String> {
    let sub_a = section.first_chunk.sub_a_size as usize;
    let (trailer0, trailer1, pos) = parse_super_block_trailer(payload);
    if (trailer0, trailer1) != (0, 0) {
        return Err(format!(
            "unexpected super-block trailer ({trailer0},{trailer1})"
        ));
    }

    let mut fwd = ForwardReader::new(payload, pos);
    let _sub_block_count = fwd.varint();
    let header =
        parse_sub_block_header(&mut fwd).ok_or_else(|| "empty first sub-block".to_string())?;
    let table = state0_table_builder(
        payload,
        fwd.pos,
        sub_a
            .checked_sub(8)
            .ok_or_else(|| "sub-A smaller than reverse seed".to_string())?,
        0,
        0,
        0,
    );
    let (code, after_code) = decode_zstd_window(payload, table.fwd)
        .map_err(|error| format!("index code zstd window: {error:?}"))?;
    let (data, after_data) = decode_raw_window(payload, after_code);
    fwd.pos = after_data;

    let mut state = IndexDecodeState {
        buf_a: Vec::new(),
        code,
        data,
        code_off: 0,
        data_off: 0,
    };
    append_index_subblock_body(section.align_a, &header, &mut fwd, &mut state)?;
    Ok(state)
}

fn append_later_index_subblocks(
    section: &MeshSection,
    payload: &[u8],
    after_vertex: &ByteGroupReadState,
    state: &mut IndexDecodeState,
) -> Result<usize, String> {
    let mut fwd = ForwardReader::new(payload, after_vertex.stream_pos);
    let Some(header) = parse_sub_block_header(&mut fwd) else {
        return Ok(fwd.pos);
    };

    let table = state0_table_builder(
        payload,
        fwd.pos,
        after_vertex.reader.ptr,
        after_vertex.reader.acc,
        after_vertex.reader.bitpos,
        0,
    );
    let mut body_fwd = ForwardReader::new(payload, table.fwd);
    append_index_subblock_body(section.align_a, &header, &mut body_fwd, state)?;
    Ok(body_fwd.pos)
}

fn append_index_subblock_body(
    align_a: u8,
    header: &nx_layout_toolbox::mc::geometry::SubBlockHeader,
    fwd: &mut ForwardReader,
    state: &mut IndexDecodeState,
) -> Result<(), String> {
    for index in 0..header.count {
        let count = if index == 0 {
            header.f
        } else {
            let _nibble = fwd
                .byte()
                .ok_or_else(|| "truncated transform-loop nibble".to_string())?;
            let _v20 = fwd.varint();
            let v28 = fwd.varint();
            let _v4 = fwd.varint();
            v28
        } as usize;
        append_index_submesh(align_a, count, state)?;
    }
    Ok(())
}

fn append_index_submesh(
    align_a: u8,
    count: usize,
    state: &mut IndexDecodeState,
) -> Result<(), String> {
    let align_a = (align_a as usize).max(1);
    while !state.buf_a.len().is_multiple_of(align_a) {
        state.buf_a.push(0);
    }

    let (out, code_used, data_used) = meshopt::decode_index_buffer_split_used(
        count,
        2,
        &state.code[state.code_off..],
        &state.data[state.data_off..],
        0,
    )
    .map_err(|error| format!("index decode count {count}: {error:?}"))?;
    state.code_off += code_used;
    state.data_off += data_used;
    state.buf_a.extend_from_slice(&out);
    Ok(())
}

fn read_msb_varint(payload: &[u8], pos: &mut usize) -> Result<usize, String> {
    let mut value = 0usize;
    for _ in 0..5 {
        let byte = payload
            .get(*pos)
            .copied()
            .ok_or_else(|| "truncated MSB varint".to_string())?;
        *pos += 1;
        value = value
            .checked_shl(7)
            .and_then(|v| v.checked_add((byte & 0x7f) as usize))
            .ok_or_else(|| "MSB varint overflow".to_string())?;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("oversized MSB varint".to_string())
}

fn observed_match_history_mask(count: u32) -> Option<u32> {
    count.checked_next_power_of_two()?.checked_sub(1)
}

fn zero_reader_at(ptr: usize) -> RansThreeLaneReader {
    RansThreeLaneReader {
        ptr,
        acc: 0,
        bitpos: 0,
    }
}

fn align_up(value: usize, align: usize) -> Result<usize, String> {
    let align = align.max(1);
    let rem = value % align;
    if rem == 0 {
        Ok(value)
    } else {
        value
            .checked_add(align - rem)
            .ok_or_else(|| "alignment overflow".to_string())
    }
}

fn is_bfres_mc(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".bfres.mc"))
}
