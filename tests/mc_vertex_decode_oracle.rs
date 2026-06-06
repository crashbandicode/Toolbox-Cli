//! Ignored CP5-int vertex decode oracle.
//!
//! Requires gitignored TotK `.mc` fixtures and RE captures under
//! `local-assets/re`. This is the convergence gate for wiring the already
//! ported state-0 -> state-4 -> match-table -> state-5 writer pieces from raw
//! payload bytes.

use std::{fs, path::Path};

use nx_layout_toolbox::mc::{
    extract,
    geometry::{
        byte_group_read, decode_raw_window, decode_zstd_window, decode_zstd_window_with_history,
        parse_sub_block_header, parse_super_block_trailer, state0_table_builder,
        vertex_attribute_driver_setup, vertex_attribute_writer_loop_step,
        vertex_kernel_state4_entry_from_table, vertex_match_table, ByteGroupReadSpec,
        ByteGroupReadState, ByteGroupTransformState, ForwardReader, RansStateBuffer,
        RansThreeLaneReader, TableBuild, VertexAttributeDriverError, VertexAttributeDriverState,
        VertexAttributeWriterLoopError, VertexAttributeWriterLoopStep, VertexAttributeWriterTable,
        VertexAttributeWriterTarget, VertexKernelState4EntrySpec, VertexMatchTableSpec,
        VertexMatchTableState,
    },
    read_mc, read_mesh_section, MeshSection,
};
use nx_layout_toolbox::meshopt;
use serde_json::Value;

const MESH_INFO_LEN: usize = 0x120;

struct FixtureCase {
    label: &'static str,
    model: &'static str,
    fixture: &'static str,
    tail: Option<TailWindow>,
}

#[derive(Clone, Copy)]
struct TailWindow {
    fwd_pos: usize,
    next_pos: usize,
}

const FIXTURES: &[FixtureCase] = &[
    FixtureCase {
        label: "Bear",
        model: "Animal_Bear.Bear.bfres.mc",
        fixture: "tests/fixtures/mc/Animal_Bear.Bear.bfres.mc",
        tail: Some(TailWindow {
            fwd_pos: 28354,
            next_pos: 28657,
        }),
    },
    FixtureCase {
        label: "Bass",
        model: "Animal_Bass.Bass.bfres.mc",
        fixture: "tests/fixtures/mc/Animal_Bass.Bass.bfres.mc",
        tail: Some(TailWindow {
            fwd_pos: 5763,
            next_pos: 6102,
        }),
    },
    FixtureCase {
        label: "Dragonfly",
        model: "Animal_Dragonfly.Dragonfly.bfres.mc",
        fixture: "tests/fixtures/mc/Animal_Dragonfly.Dragonfly.bfres.mc",
        tail: None,
    },
];

#[test]
#[ignore = "requires gitignored MeshCodec oracle captures in local-assets/re"]
fn bufgroups_from_payload_report_first_diff_for_all_fixtures() {
    for case in FIXTURES {
        assert_byte_group_from_payload_matches(case);
    }
}

#[test]
#[ignore = "requires gitignored MeshCodec oracle captures in local-assets/re"]
fn assembled_mesh_layout_from_payload_matches_oracle_for_all_fixtures() {
    for case in FIXTURES {
        assert_assembled_mesh_layout_matches(case);
    }
}

struct CaseInput {
    mc: nx_layout_toolbox::mc::McFile,
    section: MeshSection,
    payload: Vec<u8>,
    oracle_buf_b: Vec<u8>,
}

struct VertexBufBDecode {
    bytes: Vec<u8>,
    byte_group_len: usize,
    after_vertex: ByteGroupReadState,
}

struct IndexDecodeState {
    buf_a: Vec<u8>,
    code: Vec<u8>,
    data: Vec<u8>,
    code_off: usize,
    data_off: usize,
}

fn assert_byte_group_from_payload_matches(case: &FixtureCase) {
    let input = load_case_input(case);
    let decoded =
        decode_buf_b_from_payload(case, &input.section, &input.payload, &input.oracle_buf_b);
    eprintln!(
        "{} from-payload full bufB matched oracle: {} byte-group + {} tail = {}/{} bytes",
        case.label,
        decoded.byte_group_len,
        decoded.bytes.len() - decoded.byte_group_len,
        decoded.bytes.len(),
        input.oracle_buf_b.len()
    );
}

fn assert_assembled_mesh_layout_matches(case: &FixtureCase) {
    let input = load_case_input(case);
    let decoded_b =
        decode_buf_b_from_payload(case, &input.section, &input.payload, &input.oracle_buf_b);
    let mut index_state = decode_index_streams(&input.section, &input.payload)
        .unwrap_or_else(|error| panic!("{} index stream decode failed: {error}", case.label));
    let next_fwd = append_later_index_subblocks(
        case,
        &input.section,
        &input.payload,
        &decoded_b.after_vertex,
        &mut index_state,
    )
    .unwrap_or_else(|error| panic!("{} later index decode failed: {error}", case.label));

    if let Some(tail) = case.tail {
        assert_eq!(next_fwd, tail.fwd_pos, "{} state-2 tail cursor", case.label);
    }

    let buf_a_size = input.section.buf_a_size as usize;
    assert!(
        index_state.buf_a.len() <= buf_a_size,
        "{} decoded bufA {} exceeds section size {buf_a_size}",
        case.label,
        index_state.buf_a.len()
    );
    index_state.buf_a.resize(buf_a_size, 0);

    let oracle_path = Path::new("local-assets")
        .join("mesh-codec-output")
        .join(case.model.strip_suffix(".mc").expect("oracle model suffix"));
    let oracle =
        fs::read(&oracle_path).unwrap_or_else(|e| panic!("read {}: {e}", oracle_path.display()));
    let bfres = extract(&input.mc).expect("extract leading BFRES");
    let real_size = bfres.len();
    assert_eq!(
        &oracle[..real_size],
        bfres.as_slice(),
        "{} leading BFRES prefix",
        case.label
    );

    let mesh_info = mesh_info_header(real_size, input.mc.decompressed_size());
    let info_start = real_size;
    let info_end = info_start + MESH_INFO_LEN;
    assert_eq!(
        &oracle[info_start..info_end],
        mesh_info.as_slice(),
        "{} mesh info header",
        case.label
    );

    let buf_a_start = info_end;
    let buf_b_start = align_up(buf_a_start + buf_a_size, input.section.align_b as usize);
    let mut assembled = Vec::with_capacity(input.mc.decompressed_size());
    assembled.extend_from_slice(&bfres);
    assembled.extend_from_slice(&mesh_info);
    assembled.extend_from_slice(&index_state.buf_a);
    assembled.resize(buf_b_start, 0);
    assembled.extend_from_slice(&decoded_b.bytes);
    assembled.resize(input.mc.decompressed_size(), 0);

    let first = first_diff(&assembled, &oracle);
    if let Some(offset) = first {
        eprintln!(
            "{} assembled layout first-diff offset {offset} (0x{offset:x}): got 0x{:02x}, oracle 0x{:02x}",
            case.label, assembled[offset], oracle[offset]
        );
        panic!(
            "{} assembled [info][bufA][bufB]+pad mismatch at offset {offset} (0x{offset:x})",
            case.label
        );
    }

    eprintln!(
        "{} from-payload assembled layout matched oracle: {} bytes",
        case.label,
        assembled.len()
    );
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
    let table = state0_table_builder(payload, fwd.pos, sub_a.wrapping_sub(8), 0, 0, 0);
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
    case: &FixtureCase,
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

    eprintln!(
        "{} later index sub-block matched local decode: count={} cursor P+{}",
        case.label, header.count, body_fwd.pos
    );
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

fn mesh_info_header(real_size: usize, decompressed_size: usize) -> [u8; MESH_INFO_LEN] {
    let mut info = [0u8; MESH_INFO_LEN];
    let buf_a_offset = real_size + MESH_INFO_LEN;
    info[0..4].copy_from_slice(&(buf_a_offset as u32).to_le_bytes());
    info[4..8].copy_from_slice(&(decompressed_size as u32).to_le_bytes());
    info
}

fn align_up(value: usize, align: usize) -> usize {
    let align = align.max(1);
    let rem = value % align;
    if rem == 0 {
        value
    } else {
        value + align - rem
    }
}

fn load_case_input(case: &FixtureCase) -> CaseInput {
    let bytes = fs::read(case.fixture).unwrap_or_else(|e| {
        panic!(
            "read {}: {e}; run ignored test only with local fixtures present",
            case.fixture
        )
    });
    let mc = read_mc(&bytes).expect("parse .mc");
    let section = read_mesh_section(&mc)
        .expect("parse FMSH")
        .expect("mesh section");
    let stream = mc.compressed_stream();
    let payload = stream
        .get(section.payload_offset..)
        .expect("FMSH payload offset");

    let writer_json = json_file("vertex_writer_loop_capture.json");
    let writer_capture = model_row(&writer_json, case.model);
    let oracle = hex_bytes(str_field(writer_capture, "bufB_hex"));
    assert_eq!(
        payload,
        hex_bytes(str_field(writer_capture, "payload_hex")).as_slice(),
        "{} fixture payload must match the captured oracle",
        case.label
    );
    let payload = payload.to_vec();

    CaseInput {
        mc,
        section,
        payload,
        oracle_buf_b: oracle,
    }
}

fn decode_buf_b_from_payload(
    case: &FixtureCase,
    section: &MeshSection,
    payload: &[u8],
    oracle: &[u8],
) -> VertexBufBDecode {
    let writer_json = json_file("vertex_writer_loop_capture.json");
    let writer_capture = model_row(&writer_json, case.model);
    assert_eq!(
        payload,
        hex_bytes(str_field(writer_capture, "payload_hex")).as_slice(),
        "{} fixture payload must match the captured oracle",
        case.label
    );

    let match_json = json_file("vertex_match_table_capture.json");
    let match_capture = model_row(&match_json, case.model);
    let match_row = &match_capture["rows"][0];
    let driver_json = json_file("vertex_driver_setup_capture.json");
    let driver_capture = model_row(&driver_json, case.model);

    let (header, table, mut byte_state) =
        state4_entry_from_payload(payload, section.first_chunk.sub_a_size as usize);
    assert!(header.count > 0, "{} first sub-block count", case.label);
    assert_table_matches_capture(&table, &driver_capture["table"]);

    let setup = state4_setup_streams(&mut byte_state, payload).expect("0x11104d0 setup streams");
    assert_setup_matches_capture(&setup, match_row);

    let mut match_state = VertexMatchTableState {
        base: 0,
        limit: 0,
        mask: observed_match_history_mask(table.w8).expect("match history mask"),
    };
    assert_eq!(
        match_state.mask,
        u32_field(&match_row["state_before"], "mask"),
        "derived match history mask"
    );
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
    .expect("0x11106d0 match table");
    assert_eq!(matches, u32_words(str_field(match_row, "out_after_hex")));
    assert_eq!(
        (match_state.base, match_state.limit, match_state.mask),
        (
            u32_field(&match_row["state_after"], "base"),
            u32_field(&match_row["state_after"], "limit"),
            u32_field(&match_row["state_after"], "mask"),
        )
    );

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
    .expect("0x10fafe0 setup");

    let mut out = vec![0u8; oracle.len()];
    let mut writers = vec![None; oracle.len()];
    let mut steps = Vec::new();
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
            Ok(step) => {
                mark_written(&mut writers, &step, 0);
                steps.push(step);
            }
            Err(VertexAttributeWriterLoopError::Driver(
                VertexAttributeDriverError::NoAttributesRemaining { .. },
            )) => break,
            Err(error) => panic!("state-5 writer loop failed before diff: {error:?}"),
        }
    }

    let byte_group_len = table.byte_group_total as usize;
    assert!(
        byte_group_len <= oracle.len(),
        "byte-group region {byte_group_len} exceeds oracle {}",
        oracle.len()
    );
    let first = first_diff(&out[..byte_group_len], &oracle[..byte_group_len]);
    if let Some(offset) = first {
        let addr = writers[offset].unwrap_or_else(|| fallback_writer_for_offset(offset, &steps));
        eprintln!(
            "{} from-payload bufB first-diff offset {offset} (0x{offset:x}) \
             responsible {addr}: got 0x{:02x}, oracle 0x{:02x}",
            case.label, out[offset], oracle[offset]
        );
        panic!(
            "{} from-payload bufB mismatch at offset {offset} (0x{offset:x}), responsible {addr}",
            case.label
        );
    }

    eprintln!(
        "{} from-payload bufB byte-group region matched oracle: {byte_group_len}/{} bytes",
        case.label,
        oracle.len()
    );

    if let Some(tail) = case.tail {
        let expected_tail = &oracle[byte_group_len..];
        let (decoded_tail, next_pos) =
            decode_zstd_window_with_history(payload, tail.fwd_pos, &out[..byte_group_len])
                .unwrap_or_else(|error| {
                    panic!(
                        "{} history-backed zstd tail at P+{} failed: {error:?}",
                        case.label, tail.fwd_pos
                    )
                });
        assert_eq!(next_pos, tail.next_pos, "{} zstd tail cursor", case.label);
        assert_eq!(
            decoded_tail, expected_tail,
            "{} zstd tail bytes",
            case.label
        );
        out[byte_group_len..byte_group_len + decoded_tail.len()].copy_from_slice(&decoded_tail);
        eprintln!(
            "{} from-payload bufB zstd tail matched oracle: {}/{} bytes",
            case.label,
            decoded_tail.len(),
            expected_tail.len()
        );
    } else {
        assert_eq!(
            byte_group_len,
            oracle.len(),
            "{} should not have a direct zstd tail",
            case.label
        );
    }

    let first = first_diff(&out, oracle);
    if let Some(offset) = first {
        let addr = writers[offset].unwrap_or_else(|| fallback_writer_for_offset(offset, &steps));
        eprintln!(
            "{} from-payload full bufB first-diff offset {offset} (0x{offset:x}) \
             responsible {addr}: got 0x{:02x}, oracle 0x{:02x}",
            case.label, out[offset], oracle[offset]
        );
        panic!(
            "{} from-payload full bufB mismatch at offset {offset} (0x{offset:x}), responsible {addr}",
            case.label
        );
    }

    VertexBufBDecode {
        bytes: out,
        byte_group_len,
        after_vertex: driver.byte_state,
    }
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
) -> (
    nx_layout_toolbox::mc::geometry::SubBlockHeader,
    TableBuild,
    ByteGroupReadState,
) {
    let (trailer0, trailer1, pos) = parse_super_block_trailer(payload);
    assert_eq!((trailer0, trailer1), (0, 0), "single super-block trailer");
    let mut forward = ForwardReader::new(payload, pos);
    let sub_block_count = forward.varint();
    let header = parse_sub_block_header(&mut forward).expect("first sub-block header");
    let table = state0_table_builder(payload, forward.pos, sub_a_size - 8, 0, 0, 7);
    // The mode-1 auxiliary reverse readers are the two remaining substream
    // tails: captured first-call seeds are Bear 32833/37919, Bass 6692/7159,
    // and Dragonfly 4511/4686, matching sub_a_size and payload.len() - 8.
    let sub_b_tail = payload
        .len()
        .checked_sub(8)
        .expect("payload shorter than reverse-B seed");
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
    .expect("0x10f90d4 -> 0x11104d0 state-4 entry");
    assert_eq!(byte_state.reader, entry.reader);
    assert_eq!(byte_state.stream_pos, entry.stream_pos);
    (header, table, byte_state)
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
            if read.selector != 3 {
                return Err(format!(
                    "0x11104d0 stream 3 expected direct selector 3, got {}",
                    read.selector
                ));
            }
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

fn assert_setup_matches_capture(setup: &State4Setup, row: &Value) {
    let expected_counts = usize_array4(&row["counts"]);
    assert_eq!(setup.counts, expected_counts, "0x11104d0 counts");
    let streams = row["stream_hex"].as_array().expect("stream_hex array");
    for (index, stream) in streams.iter().enumerate().take(4) {
        let expected = hex_bytes(stream.as_str().expect("stream hex"));
        if index == 3 {
            assert_eq!(setup.stream3_with_slop, expected, "setup stream 3 + slop");
            assert_eq!(
                setup.streams[3],
                expected[..setup.counts[3]].to_vec(),
                "setup stream 3 logical bytes"
            );
        } else {
            assert_eq!(setup.streams[index], expected, "setup stream {index}");
        }
    }
}

fn assert_table_matches_capture(table: &TableBuild, value: &Value) {
    assert_eq!(table.w8, u32_field(value, "vertex_count"));
    assert_eq!(table.symbols, u32_field(value, "count"));
    assert_eq!(table.entries, u32_vec(&value["entries"]));
    assert_eq!(table.offsets, u32_vec(&value["offsets"]));
    assert_eq!(table.cols, u8_vec(&value["cols"]));
    assert_eq!(table.byte_group_total, u32_field(value, "byte_group_total"));
    assert_eq!(table.max_prod, u32_field(value, "max_prod"));
}

fn observed_match_history_mask(count: u32) -> Option<u32> {
    count.checked_next_power_of_two()?.checked_sub(1)
}

fn mark_written(
    writers: &mut [Option<&'static str>],
    step: &VertexAttributeWriterLoopStep,
    block_index: usize,
) {
    let addr = target_addr(step.interstage.writer);
    let unit_size = target_unit_size(step.interstage.writer);
    let stride = (step.transform.table_entry.raw >> 24) as usize;
    let mut cursor = step.transform.out_offset as usize + block_index * stride;
    for record in &step.transform.records {
        let literal_count = (record[0] & 0xffff) as usize;
        let copy_count = (record[0] >> 16) as usize;
        for _ in 0..literal_count {
            mark_unit(writers, cursor, unit_size, addr);
            cursor += stride;
        }
        for _ in 0..copy_count {
            mark_unit(writers, cursor, unit_size, addr);
            cursor += stride;
        }
    }
}

fn mark_unit(
    writers: &mut [Option<&'static str>],
    cursor: usize,
    unit_size: usize,
    addr: &'static str,
) {
    for slot in writers.iter_mut().skip(cursor).take(unit_size) {
        *slot = Some(addr);
    }
}

fn fallback_writer_for_offset(
    offset: usize,
    steps: &[VertexAttributeWriterLoopStep],
) -> &'static str {
    for step in steps {
        let stride = (step.transform.table_entry.raw >> 24) as usize;
        let unit_size = target_unit_size(step.interstage.writer);
        let out_offset = step.transform.out_offset as usize;
        if stride != 0 && offset >= out_offset {
            let column = (offset - out_offset) % stride;
            if column < unit_size {
                return target_addr(step.interstage.writer);
            }
        }
    }
    "unwritten/0x10f924c"
}

fn target_unit_size(target: VertexAttributeWriterTarget) -> usize {
    match target {
        VertexAttributeWriterTarget::Copy1 => 1,
        VertexAttributeWriterTarget::Copy2 => 2,
        VertexAttributeWriterTarget::Copy3 => 3,
        VertexAttributeWriterTarget::Copy4 => 4,
        VertexAttributeWriterTarget::Copy8 => 8,
        VertexAttributeWriterTarget::U8PreviousDelta => 1,
        VertexAttributeWriterTarget::U8x3PreviousDelta => 3,
        VertexAttributeWriterTarget::Delta2 => 2,
        VertexAttributeWriterTarget::Delta3 => 3,
        VertexAttributeWriterTarget::Delta4 => 4,
        VertexAttributeWriterTarget::Delta1Direct => 1,
        VertexAttributeWriterTarget::Delta2Direct => 2,
        VertexAttributeWriterTarget::Delta3Direct => 3,
        VertexAttributeWriterTarget::Delta4Direct => 4,
        VertexAttributeWriterTarget::I8x3DirectDelta => 3,
        VertexAttributeWriterTarget::U16x3Delta => 6,
        VertexAttributeWriterTarget::U16x2DirectDelta => 4,
        VertexAttributeWriterTarget::U32x2Delta => 8,
        VertexAttributeWriterTarget::U16x2PreviousDelta => 4,
        VertexAttributeWriterTarget::U8x2Delta => 2,
        VertexAttributeWriterTarget::U8x3Delta => 3,
        VertexAttributeWriterTarget::Pack10x3PreviousDelta => 4,
        VertexAttributeWriterTarget::U16x2Delta => 4,
        VertexAttributeWriterTarget::I8x2Normal => 3,
        VertexAttributeWriterTarget::I8x3NormalDelta => 3,
        VertexAttributeWriterTarget::Pack10x3Delta => 4,
    }
}

fn target_addr(target: VertexAttributeWriterTarget) -> &'static str {
    match target {
        VertexAttributeWriterTarget::Copy1 => "0x10fc5e0",
        VertexAttributeWriterTarget::Copy2 => "0x10fc680",
        VertexAttributeWriterTarget::Copy3 => "0x10fc720",
        VertexAttributeWriterTarget::Copy4 => "0x10fc7d0",
        VertexAttributeWriterTarget::Copy8 => "0x10fc920",
        VertexAttributeWriterTarget::U8PreviousDelta => "0x1101230",
        VertexAttributeWriterTarget::U8x3PreviousDelta => "0x1101410",
        VertexAttributeWriterTarget::Delta2 => "0x10fbcc0",
        VertexAttributeWriterTarget::Delta3 => "0x10fbdc0",
        VertexAttributeWriterTarget::Delta4 => "0x10fbee0",
        VertexAttributeWriterTarget::Delta1Direct => "0x10fdb30",
        VertexAttributeWriterTarget::Delta2Direct => "0x10fdc00",
        VertexAttributeWriterTarget::Delta3Direct => "0x10fdcf0",
        VertexAttributeWriterTarget::Delta4Direct => "0x10fde00",
        VertexAttributeWriterTarget::I8x3DirectDelta => "0x10ffdb0",
        VertexAttributeWriterTarget::U16x3Delta => "0x1100c90",
        VertexAttributeWriterTarget::U16x2DirectDelta => "0x10fdfe0",
        VertexAttributeWriterTarget::U32x2Delta => "0x10fe4d0",
        VertexAttributeWriterTarget::U16x2PreviousDelta => "0x1101850",
        VertexAttributeWriterTarget::U8x2Delta => "0x11033e0",
        VertexAttributeWriterTarget::U8x3Delta => "0x1103530",
        VertexAttributeWriterTarget::Pack10x3PreviousDelta => "0x1103840",
        VertexAttributeWriterTarget::U16x2Delta => "0x1103ab0",
        VertexAttributeWriterTarget::I8x2Normal => "0x110aac0",
        VertexAttributeWriterTarget::I8x3NormalDelta => "0x110ae30",
        VertexAttributeWriterTarget::Pack10x3Delta => "0x110afb0",
    }
}

fn zero_reader_at(ptr: usize) -> RansThreeLaneReader {
    RansThreeLaneReader {
        ptr,
        acc: 0,
        bitpos: 0,
    }
}

fn json_file(name: &str) -> Value {
    let path = Path::new("local-assets").join("re").join(name);
    serde_json::from_str(&fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "read {}: {e}; run ignored test only with local oracle captures present",
            path.display()
        )
    }))
    .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn model_row<'a>(value: &'a Value, model: &str) -> &'a Value {
    value
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["model"].as_str() == Some(model)))
        .unwrap_or_else(|| panic!("{model} row missing"))
}

fn str_field<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} string missing"))
}

fn u32_field(value: &Value, key: &str) -> u32 {
    value[key]
        .as_u64()
        .unwrap_or_else(|| panic!("{key} integer missing")) as u32
}

fn usize_array4(value: &Value) -> [usize; 4] {
    let values = value.as_array().expect("usize array");
    [
        values[0].as_u64().expect("usize") as usize,
        values[1].as_u64().expect("usize") as usize,
        values[2].as_u64().expect("usize") as usize,
        values[3].as_u64().expect("usize") as usize,
    ]
}

fn u32_vec(value: &Value) -> Vec<u32> {
    value
        .as_array()
        .expect("u32 array")
        .iter()
        .map(|v| v.as_u64().expect("u32") as u32)
        .collect()
}

fn u8_vec(value: &Value) -> Vec<u8> {
    value
        .as_array()
        .expect("u8 array")
        .iter()
        .map(|v| v.as_u64().expect("u8") as u8)
        .collect()
}

fn u32_words(hex: &str) -> Vec<u32> {
    hex_bytes(hex)
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap()))
        .collect()
}

fn hex_bytes(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "odd hex length");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn first_diff(a: &[u8], b: &[u8]) -> Option<usize> {
    let len = a.len().min(b.len());
    (0..len)
        .find(|&index| a[index] != b[index])
        .or_else(|| (a.len() != b.len()).then_some(len))
}
