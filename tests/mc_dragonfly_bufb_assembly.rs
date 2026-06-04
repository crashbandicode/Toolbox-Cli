//! Ignored Dragonfly bufB assembly oracle.
//!
//! Requires gitignored captures under `local-assets/re/`. This is intentionally
//! not part of normal CI; run it locally after porting state-5 writers.

use std::{fs, path::Path};

use nx_layout_toolbox::mc::geometry::{
    transform_tail_copy1_into, transform_tail_copy2_into, transform_tail_delta2_direct_into,
    transform_tail_delta4_direct_into, transform_tail_i8x2_normal_into,
    transform_tail_pack10x3_delta_into, transform_tail_u16x2_delta_into,
    transform_tail_u16x3_delta_into, TransformTailCopy1Spec, TransformTailCopy2Spec,
    TransformTailDelta2DirectSpec, TransformTailDelta4DirectSpec, TransformTailI8x2NormalSpec,
    TransformTailPack10x3DeltaSpec, TransformTailRecord, TransformTailU16x2DeltaSpec,
    TransformTailU16x3DeltaSpec,
};
use serde_json::Value;

#[test]
#[ignore = "requires gitignored Dragonfly oracle captures in local-assets/re"]
fn dragonfly_bufb_assembly_matches_oracle() {
    let byte_group = dragonfly_row(&json_file("byte_group_transform_capture.json"));
    let tails = dragonfly_row(&json_file("transform_tails_capture.json"));
    let u16x3 =
        dragonfly_row(&json_file("transform_tail_1100c90_capture.json"))["calls"][0].clone();
    let pack10 =
        dragonfly_row(&json_file("transform_tail_110afb0_capture.json"))["calls"][0].clone();
    let u16x2_row = dragonfly_row(&json_file("transform_tail_1103ab0_capture.json"));
    let i8x2 = dragonfly_row(&json_file("transform_tail_110aac0_capture.json"))["calls"][0].clone();
    let delta4 =
        dragonfly_row(&json_file("transform_tail_10fde00_capture.json"))["calls"][0].clone();
    let oracle = hex_bytes(str_field(&delta4, "bufb_after_hex"));
    let mut bufb = vec![0u8; oracle.len()];
    let mut writers = vec![None; oracle.len()];

    assert_eq!(
        early_record_hex(&byte_group["calls"][0]),
        str_field(&u16x3, "records_hex")
    );
    run_u16x3(&mut bufb, &mut writers, &u16x3);

    let direct2 = call_by_fn(&tails, "0x10fdc00");
    assert_eq!(
        byte_group_records_hex(&byte_group["calls"][1]),
        str_field(&direct2, "records_hex")
    );
    run_direct2(&mut bufb, &mut writers, &direct2);

    let copy2 = call_by_fn(&tails, "0x10fc680");
    assert_eq!(
        byte_group_records_hex(&byte_group["calls"][2]),
        str_field(&copy2, "records_hex")
    );
    run_copy2(&mut bufb, &mut writers, &copy2);

    assert_eq!(
        byte_group_records_hex(&byte_group["calls"][3]),
        str_field(&pack10, "records_hex")
    );
    run_pack10(&mut bufb, &mut writers, &pack10);

    for current in [4usize, 5] {
        let call = call_by_current(&u16x2_row, current);
        assert_eq!(
            byte_group_records_hex(&byte_group["calls"][current]),
            str_field(&call, "records_hex")
        );
        run_u16x2(&mut bufb, &mut writers, &call);
    }

    assert_eq!(
        byte_group_records_hex(&byte_group["calls"][6]),
        str_field(&i8x2, "records_hex")
    );
    run_i8x2(&mut bufb, &mut writers, &i8x2);

    let copy1 = call_by_fn(&tails, "0x10fc5e0");
    assert_eq!(
        early_record_hex(&byte_group["calls"][7]),
        str_field(&copy1, "records_hex")
    );
    run_copy1(&mut bufb, &mut writers, &copy1);

    assert_eq!(
        byte_group_records_hex(&byte_group["calls"][8]),
        str_field(&delta4, "records_hex")
    );
    run_direct4(&mut bufb, &mut writers, &delta4);

    if let Some(offset) = first_diff(&bufb, &oracle) {
        let writer = writers[offset].unwrap_or_else(|| fallback_writer_for_offset(offset));
        eprintln!(
            "first-diff offset {} (0x{:x}) writer {}: got 0x{:02x}, oracle 0x{:02x}",
            offset, offset, writer, bufb[offset], oracle[offset]
        );
        panic!(
            "Dragonfly bufB mismatch at offset {} (0x{:x}), writer {}",
            offset, offset, writer
        );
    }

    let touched = writers.iter().filter(|writer| writer.is_some()).count();
    eprintln!(
        "Dragonfly bufB assembly matched oracle: {touched}/{} writer bytes",
        oracle.len()
    );
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

fn dragonfly_row(models: &Value) -> Value {
    models
        .as_array()
        .and_then(|rows| {
            rows.iter().find(|row| {
                row["model"]
                    .as_str()
                    .is_some_and(|model| model.starts_with("Animal_Dragonfly"))
            })
        })
        .unwrap_or_else(|| panic!("Animal_Dragonfly row missing"))
        .clone()
}

fn call_by_fn(row: &Value, addr: &str) -> Value {
    row["calls"]
        .as_array()
        .and_then(|calls| calls.iter().find(|call| call["fn"].as_str() == Some(addr)))
        .unwrap_or_else(|| panic!("{addr} call missing"))
        .clone()
}

fn call_by_current(row: &Value, current: usize) -> Value {
    row["calls"]
        .as_array()
        .and_then(|calls| {
            calls
                .iter()
                .find(|call| call["current"].as_u64() == Some(current as u64))
        })
        .unwrap_or_else(|| panic!("current {current} call missing"))
        .clone()
}

fn str_field<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} string missing"))
}

fn usize_field(value: &Value, key: &str) -> usize {
    value[key]
        .as_u64()
        .unwrap_or_else(|| panic!("{key} integer missing")) as usize
}

fn source(value: &Value, index: usize) -> Vec<u8> {
    hex_bytes(
        value["source_hex"]
            .as_array()
            .and_then(|sources| sources.get(index))
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("source_hex[{index}] missing")),
    )
}

fn records(value: &Value) -> Vec<TransformTailRecord> {
    records_from_hex(str_field(value, "records_hex"))
}

fn matches(value: &Value) -> Vec<u32> {
    hex_bytes(str_field(value, "match_hex"))
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap()))
        .collect()
}

fn records_from_hex(hex: &str) -> Vec<TransformTailRecord> {
    hex_bytes(hex)
        .chunks_exact(8)
        .map(|chunk| {
            let word = u32::from_le_bytes(chunk[0..4].try_into().unwrap());
            let back_distance = u32::from_le_bytes(chunk[4..8].try_into().unwrap()) as usize;
            TransformTailRecord {
                literal_count: (word & 0xffff) as u16,
                copy_count: (word >> 16) as u16,
                back_distance,
            }
        })
        .collect()
}

fn byte_group_records_hex(call: &Value) -> &str {
    call["d360"][0]["out_after_hex"]
        .as_str()
        .unwrap_or_else(|| panic!("byte-group d360 output missing"))
}

fn early_record_hex(call: &Value) -> String {
    let word = call["w6"]
        .as_u64()
        .unwrap_or_else(|| panic!("byte-group w6 missing")) as u32;
    let mut bytes = Vec::with_capacity(8);
    bytes.extend_from_slice(&word.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    to_hex(&bytes)
}

fn run_u16x3(bufb: &mut [u8], writers: &mut [Option<&'static str>], call: &Value) {
    let records = records(call);
    let matches = matches(call);
    let source0 = source(call, 0);
    let source1 = source(call, 1);
    transform_tail_u16x3_delta_into(
        bufb,
        TransformTailU16x3DeltaSpec {
            output_stride: usize_field(call, "output_stride"),
            block_index: usize_field(call, "block_index"),
            out_offset: usize_field(call, "out_offset"),
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
        },
    )
    .unwrap();
    mark_written(writers, call, &records, 6, "0x1100c90");
}

fn run_direct2(bufb: &mut [u8], writers: &mut [Option<&'static str>], call: &Value) {
    let records = records(call);
    let matches = matches(call);
    let source0 = source(call, 0);
    let source1 = source(call, 1);
    transform_tail_delta2_direct_into(
        bufb,
        TransformTailDelta2DirectSpec {
            output_stride: usize_field(call, "output_stride"),
            block_index: usize_field(call, "block_index"),
            out_offset: usize_field(call, "out_offset"),
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
        },
    )
    .unwrap();
    mark_written(writers, call, &records, 2, "0x10fdc00");
}

fn run_copy2(bufb: &mut [u8], writers: &mut [Option<&'static str>], call: &Value) {
    let records = records(call);
    let source0 = source(call, 0);
    transform_tail_copy2_into(
        bufb,
        TransformTailCopy2Spec {
            output_stride: usize_field(call, "output_stride"),
            block_index: usize_field(call, "block_index"),
            out_offset: usize_field(call, "out_offset"),
            records: &records,
            source: &source0,
        },
    )
    .unwrap();
    mark_written(writers, call, &records, 2, "0x10fc680");
}

fn run_pack10(bufb: &mut [u8], writers: &mut [Option<&'static str>], call: &Value) {
    let records = records(call);
    let matches = matches(call);
    let source0 = source(call, 0);
    let source1 = source(call, 1);
    let source2 = source(call, 2);
    let source3 = source(call, 3);
    transform_tail_pack10x3_delta_into(
        bufb,
        TransformTailPack10x3DeltaSpec {
            output_stride: usize_field(call, "output_stride"),
            block_index: usize_field(call, "block_index"),
            out_offset: usize_field(call, "out_offset"),
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
            source2: &source2,
            source3: &source3,
        },
    )
    .unwrap();
    mark_written(writers, call, &records, 4, "0x110afb0");
}

fn run_u16x2(bufb: &mut [u8], writers: &mut [Option<&'static str>], call: &Value) {
    let records = records(call);
    let matches = matches(call);
    let source0 = source(call, 0);
    let source1 = source(call, 1);
    transform_tail_u16x2_delta_into(
        bufb,
        TransformTailU16x2DeltaSpec {
            output_stride: usize_field(call, "output_stride"),
            block_index: usize_field(call, "block_index"),
            out_offset: usize_field(call, "out_offset"),
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
        },
    )
    .unwrap();
    mark_written(writers, call, &records, 4, "0x1103ab0");
}

fn run_i8x2(bufb: &mut [u8], writers: &mut [Option<&'static str>], call: &Value) {
    let records = records(call);
    let source0 = source(call, 0);
    let source1 = source(call, 1);
    let source2 = source(call, 2);
    transform_tail_i8x2_normal_into(
        bufb,
        TransformTailI8x2NormalSpec {
            output_stride: usize_field(call, "output_stride"),
            block_index: usize_field(call, "block_index"),
            out_offset: usize_field(call, "out_offset"),
            records: &records,
            source0: &source0,
            source1: &source1,
            source2: &source2,
        },
    )
    .unwrap();
    mark_written(writers, call, &records, 3, "0x110aac0");
}

fn run_copy1(bufb: &mut [u8], writers: &mut [Option<&'static str>], call: &Value) {
    let records = records(call);
    let source0 = source(call, 0);
    transform_tail_copy1_into(
        bufb,
        TransformTailCopy1Spec {
            output_stride: usize_field(call, "output_stride"),
            block_index: usize_field(call, "block_index"),
            out_offset: usize_field(call, "out_offset"),
            records: &records,
            source: &source0,
        },
    )
    .unwrap();
    mark_written(writers, call, &records, 1, "0x10fc5e0");
}

fn run_direct4(bufb: &mut [u8], writers: &mut [Option<&'static str>], call: &Value) {
    let records = records(call);
    let matches = matches(call);
    let source0 = source(call, 0);
    let source1 = source(call, 1);
    transform_tail_delta4_direct_into(
        bufb,
        TransformTailDelta4DirectSpec {
            output_stride: usize_field(call, "output_stride"),
            block_index: usize_field(call, "block_index"),
            out_offset: usize_field(call, "out_offset"),
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
        },
    )
    .unwrap();
    mark_written(writers, call, &records, 4, "0x10fde00");
}

fn mark_written(
    writers: &mut [Option<&'static str>],
    call: &Value,
    records: &[TransformTailRecord],
    unit_size: usize,
    addr: &'static str,
) {
    let stride = usize_field(call, "output_stride");
    let mut cursor = usize_field(call, "out_offset") + usize_field(call, "block_index") * stride;
    for record in records {
        for _ in 0..record.literal_count {
            mark_unit(writers, cursor, unit_size, addr);
            cursor += stride;
        }
        for _ in 0..record.copy_count {
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
    for writer in writers.iter_mut().skip(cursor).take(unit_size) {
        *writer = Some(addr);
    }
}

fn first_diff(a: &[u8], b: &[u8]) -> Option<usize> {
    let len = a.len().min(b.len());
    (0..len)
        .find(|&index| a[index] != b[index])
        .or_else(|| (a.len() != b.len()).then_some(len))
}

fn fallback_writer_for_offset(offset: usize) -> &'static str {
    if offset < 5230 {
        return match offset % 10 {
            0..=5 => "0x1100c90",
            6..=7 => "0x10fdc00",
            8..=9 => "0x10fc680",
            _ => "unknown",
        };
    }
    if offset >= 5232 {
        return match (offset - 5232) % 20 {
            0..=3 => "0x110afb0",
            4..=11 => "0x1103ab0",
            12..=14 => "0x110aac0",
            15 => "0x10fc5e0",
            16..=19 => "0x10fde00",
            _ => "unknown",
        };
    }
    "unknown"
}

fn hex_bytes(s: &str) -> Vec<u8> {
    assert_eq!(s.len() % 2, 0, "odd hex length");
    s.as_bytes()
        .chunks_exact(2)
        .map(|chunk| {
            let hex = std::str::from_utf8(chunk).unwrap();
            u8::from_str_radix(hex, 16).unwrap_or_else(|e| panic!("bad hex byte {hex}: {e}"))
        })
        .collect()
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
