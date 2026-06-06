//! Ignored CP5b vertex writer-loop replay.
//!
//! Requires gitignored captures under `local-assets/re`. This validates the
//! composed Rust loop against the emulator-captured writer inputs for every
//! observed Bear/Bass/Dragonfly attribute.

use std::{collections::BTreeMap, fs, path::Path};

use nx_layout_toolbox::mc::geometry::{
    vertex_attribute_apply_writer, vertex_attribute_driver_setup, vertex_attribute_driver_step,
    vertex_attribute_interstage_sources, vertex_attribute_writer_loop_step, ByteGroupReadState,
    ByteGroupTransformState, RansStateBuffer, RansThreeLaneReader, TableBuild,
    VertexAttributeDriverState, VertexAttributeWriterCall, VertexAttributeWriterLoopError,
    VertexAttributeWriterTable, VertexAttributeWriterTarget,
};
use serde_json::Value;

#[test]
#[ignore = "requires gitignored CP5b vertex writer-loop captures in local-assets/re"]
fn vertex_writer_loop_replays_captured_inputs() {
    let setup_json = json_file("vertex_driver_setup_capture.json");
    let setups = rows_by_model(&setup_json);
    let byte_group_json = json_file("byte_group_reader_capture.json");
    let byte_groups = rows_by_model(&byte_group_json);
    let loops = json_file("vertex_writer_loop_capture.json");
    let mut total = 0usize;

    for loop_model in loops.as_array().expect("loop capture array") {
        let model = str_field(loop_model, "model");
        let setup = setups
            .get(model)
            .unwrap_or_else(|| panic!("{model} setup missing"));
        let setup_call = &setup["setup"][0];
        let table_json = &setup["table"];
        let payload = hex_bytes(str_field(loop_model, "payload_hex"));
        let table = table_build(table_json);
        let mut state = VertexAttributeDriverState {
            current_attribute: usize_field(table_json, "current"),
            processed_vertices: u32_field(table_json, "processed"),
            vertex_count: u32_field(table_json, "vertex_count"),
            block_limit: u32_field(table_json, "block_limit"),
            transform_state: transform_state(&setup_call["state_before"]),
            byte_state: byte_state(&setup_call["reader_before"]),
        };

        vertex_attribute_driver_setup(
            &mut state.transform_state,
            &mut state.byte_state,
            &payload,
            table.entries.len(),
        )
        .unwrap();
        assert_eq!(
            state.transform_state,
            transform_state(&setup_call["state_after"])
        );
        assert_eq!(state.byte_state, byte_state(&setup_call["reader_after"]));
        let byte_group_row = byte_groups
            .get(model)
            .unwrap_or_else(|| panic!("{model} byte-group capture missing"));
        state.byte_state.selector2_history = selector2_history_before(
            byte_group_row,
            ptr_rel(&setup_call["reader_before"]["stream"]),
        );

        let rows = loop_model["rows"].as_array().expect("loop rows");
        state.byte_state.segment_state = segment_state_before_step(&rows[0]);
        let matches = matches_from_hex(str_field(&rows[0], "match_hex"));
        let mut out = vec![0u8; hex_bytes(str_field(loop_model, "bufB_hex")).len()];

        for expected in rows {
            assert_eq!(
                state.byte_state.segment_state,
                segment_state_before_step(expected),
                "{model} current {} segment before step",
                usize_field(expected, "current")
            );
            let transform = vertex_attribute_driver_step(&mut state, &table, &payload).unwrap();
            assert_byte_reader_state(
                &state.byte_state,
                &expected["reader_before_dispatch"],
                model,
            );
            assert_eq!(
                state.byte_state.segment_state,
                segment_state_before_interstage(expected),
                "{model} current {} segment before interstage",
                transform.index
            );
            let interstage = vertex_attribute_interstage_sources(
                &mut state.byte_state,
                &payload,
                transform.table_entry,
                transform.ret,
            )
            .unwrap();
            let _usage = vertex_attribute_apply_writer(
                &mut out,
                VertexAttributeWriterCall {
                    transform: &transform,
                    interstage: &interstage,
                    matches: &matches,
                    aux_table: &[],
                    block_index: usize_field(expected, "block_index"),
                },
            )
            .unwrap();

            assert_eq!(transform.index, usize_field(expected, "current"));
            assert_eq!(
                transform.records.len(),
                usize_field(expected, "record_count")
            );
            assert_eq!(
                records_hex(&transform.records),
                str_field(expected, "records_hex")
            );
            assert_eq!(
                target_addr(interstage.writer),
                str_field(expected, "writer_target")
            );
            assert_eq!(
                interstage.sources.len(),
                usize_field(expected, "source_count")
            );
            assert_eq!(
                str_field(expected, "match_hex"),
                str_field(&rows[0], "match_hex")
            );

            for (index, source) in interstage.sources.iter().enumerate() {
                let expected_source = source_bytes(expected, index, source.bytes.len());
                assert_eq!(
                    source.bytes, expected_source,
                    "{model} current {} source {index}",
                    transform.index
                );
            }
            assert_byte_reader_state(&state.byte_state, &expected["reader_before_writer"], model);
            assert_eq!(
                state.byte_state.segment_state,
                segment_state_after_step(expected),
                "{model} current {} segment after step",
                transform.index
            );

            let after = hex_bytes(str_field(expected, "bufb_after_hex"));
            assert_bytes_eq(
                &out[..after.len()],
                after.as_slice(),
                &format!("{model} current {} bufB after", transform.index),
            );
            total += 1;
        }

        let err = vertex_attribute_writer_loop_step(
            &mut out,
            &mut state,
            &table,
            &payload,
            VertexAttributeWriterTable {
                matches: &matches,
                aux_table: &[],
                block_index: 0,
            },
        )
        .unwrap_err();
        assert!(matches!(
            err,
            VertexAttributeWriterLoopError::Driver(
                nx_layout_toolbox::mc::geometry::VertexAttributeDriverError::NoAttributesRemaining {
                    ..
                }
            )
        ));
    }

    assert_eq!(total, 25);
}

fn json_file(name: &str) -> Value {
    let path = Path::new("local-assets").join("re").join(name);
    serde_json::from_str(&fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "read {}: {e}; run ignored test only with local captures present",
            path.display()
        )
    }))
    .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn rows_by_model(value: &Value) -> BTreeMap<&str, &Value> {
    value
        .as_array()
        .expect("model rows")
        .iter()
        .map(|row| (str_field(row, "model"), row))
        .collect()
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

fn usize_field(value: &Value, key: &str) -> usize {
    value[key]
        .as_u64()
        .unwrap_or_else(|| panic!("{key} integer missing")) as usize
}

fn ptr_rel(value: &Value) -> usize {
    assert_eq!(str_field(value, "space"), "payload");
    usize_field(value, "rel")
}

fn reader(value: &Value) -> RansThreeLaneReader {
    RansThreeLaneReader {
        ptr: ptr_rel(&value["ptr"]),
        acc: value["acc"].as_u64().expect("reader acc"),
        bitpos: u32_field(value, "bitpos"),
    }
}

fn byte_state(value: &Value) -> ByteGroupReadState {
    let extra = value["extra"].as_array().expect("extra readers");
    ByteGroupReadState {
        reader: reader(&value["main"]),
        mode1_extra_readers: [reader(&extra[0]), reader(&extra[1])],
        stream_pos: ptr_rel(&value["stream"]),
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    }
}

fn selector2_history_before(row: &Value, stream_limit: usize) -> Vec<u8> {
    let mut history = Vec::new();
    for call in row["calls"].as_array().expect("byte-group calls") {
        if u32_field(call, "selector") != 2 {
            continue;
        }
        let stream_after = ptr_rel(&call["reader_after"]["stream"]);
        if stream_after > stream_limit {
            break;
        }
        history.extend_from_slice(&hex_bytes(str_field(call, "out_hex")));
    }
    history
}

fn rans_state(value: &Value) -> RansStateBuffer {
    let states = value["states"].as_array().expect("rANS states");
    RansStateBuffer {
        states: [
            states[0].as_u64().expect("state 0"),
            states[1].as_u64().expect("state 1"),
            states[2].as_u64().expect("state 2"),
            states[3].as_u64().expect("state 3"),
        ],
        flag: u32_field(value, "flag"),
    }
}

fn segment_state_before_step(row: &Value) -> RansStateBuffer {
    let transform_reads = row["transform_source_reads"]
        .as_array()
        .expect("transform source reads");
    if let Some(read) = transform_reads.first() {
        return rans_state(&read["segment_state_before"]);
    }
    let source_reads = row["source_reads"].as_array().expect("source reads");
    rans_state(&source_reads[0]["segment_state_before"])
}

fn segment_state_after_step(row: &Value) -> RansStateBuffer {
    let source_reads = row["source_reads"].as_array().expect("source reads");
    rans_state(&source_reads.last().expect("last source read")["segment_state_after"])
}

fn segment_state_before_interstage(row: &Value) -> RansStateBuffer {
    let source_reads = row["source_reads"].as_array().expect("source reads");
    rans_state(&source_reads[0]["segment_state_before"])
}

fn assert_byte_reader_state(actual: &ByteGroupReadState, expected: &Value, model: &str) {
    let expected = byte_state(expected);
    assert_eq!(actual.reader, expected.reader, "{model} main reader");
    assert_eq!(
        actual.mode1_extra_readers, expected.mode1_extra_readers,
        "{model} extra readers"
    );
    assert_eq!(actual.stream_pos, expected.stream_pos, "{model} stream pos");
}

fn transform_state(value: &Value) -> ByteGroupTransformState {
    ByteGroupTransformState {
        mode: u32_field(value, "mode"),
        count_bits: u32_field(value, "count_bits"),
        record_count: u32_field(value, "record_count"),
        second_count: u32_field(value, "second_count"),
        third_count: u32_field(value, "third_count"),
        tail_count: u32_field(value, "tail_count"),
    }
}

fn table_build(value: &Value) -> TableBuild {
    TableBuild {
        fwd: 0,
        rev_ptr: 0,
        rev_acc: 0,
        rev_bitpos: 0,
        w8: u32_field(value, "vertex_count"),
        symbols: u32_field(value, "count"),
        branch_bit: 0,
        dir_bit: 1,
        entries: u32_vec(&value["entries"]),
        offsets: u32_vec(&value["offsets"]),
        cols: u8_vec(&value["cols"]),
        longs: u32_vec(&value["longs"]),
        byte_group_total: u32_field(value, "byte_group_total"),
        max_prod: u32_field(value, "max_prod"),
    }
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

fn hex_bytes(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2));
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn matches_from_hex(s: &str) -> Vec<u32> {
    hex_bytes(s)
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap()))
        .collect()
}

fn source_bytes(value: &Value, index: usize, len: usize) -> Vec<u8> {
    let source_hex = value["source_hex"]
        .as_array()
        .and_then(|sources| sources.get(index))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("source_hex[{index}] missing"));
    hex_bytes(&source_hex[..len * 2])
}

fn records_hex(records: &[[u32; 2]]) -> String {
    let mut bytes = Vec::with_capacity(records.len() * 8);
    for record in records {
        bytes.extend_from_slice(&record[0].to_le_bytes());
        bytes.extend_from_slice(&record[1].to_le_bytes());
    }
    to_hex(&bytes)
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn assert_bytes_eq(actual: &[u8], expected: &[u8], label: &str) {
    if actual == expected {
        return;
    }
    let diff = actual
        .iter()
        .zip(expected)
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| actual.len().min(expected.len()));
    let start = diff.saturating_sub(16);
    let end = actual.len().min(expected.len()).min(diff + 32);
    panic!(
        "{label}: first diff {diff}; actual {}; expected {}",
        to_hex(&actual[start..end]),
        to_hex(&expected[start..end])
    );
}

fn target_addr(target: VertexAttributeWriterTarget) -> &'static str {
    match target {
        VertexAttributeWriterTarget::Copy1 => "0x10fc5e0",
        VertexAttributeWriterTarget::Copy2 => "0x10fc680",
        VertexAttributeWriterTarget::Copy3 => "0x10fc720",
        VertexAttributeWriterTarget::Copy4 => "0x10fc7d0",
        VertexAttributeWriterTarget::Copy6 => "0x10fc870",
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
        VertexAttributeWriterTarget::Pack10x3Normal => "0x110aba0",
        VertexAttributeWriterTarget::F16x3Predict => "0x1106250",
        VertexAttributeWriterTarget::I8x3NormalDelta => "0x110ae30",
        VertexAttributeWriterTarget::Pack10x3Delta => "0x110afb0",
    }
}
