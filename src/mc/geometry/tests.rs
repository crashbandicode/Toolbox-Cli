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
    let section = crate::mc::read_mesh_section(&mc)
        .unwrap()
        .expect("mesh section");
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
    let tb = state0_table_builder(payload, fwd.pos, sub_a - 8, 0, 0, 7);
    assert_eq!(tb.fwd, 15, "forward cursor after table builder");
    assert_eq!(tb.rev_ptr, sub_a - 8 - 18, "reverse-A ptr (P+32807)");
    assert_eq!(tb.rev_bitpos, 50, "reverse-A bit position");
    assert_eq!(
        (tb.w8, tb.symbols, tb.branch_bit, tb.dir_bit),
        (3327, 8, 0, 1)
    );
    // Canonical-Huffman table values (golden, from the oracle/emulator).
    assert_eq!(
        tb.entries,
        [
            0x0c00100b, 0x0c000803, 0x0c000803, 0x10000a13, 0x1000100a, 0x1000100a, 0x10000803,
            0x10000801
        ]
    );
    assert_eq!(tb.offsets, [0, 6, 9, 39928, 39932, 39936, 39940, 39943]);
    assert_eq!(tb.cols, [0, 6, 9, 0, 4, 8, 12, 15]);
    assert_eq!(tb.longs, [131340, 458768]);
    assert_eq!(tb.byte_group_total, 93160);
    assert_eq!(tb.max_prod, 48);

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

/// Contiguous spread for Bear's first rANS segment (M=64, log=6).
///
/// Provenance: `spread_ref.py` / `vtxgt/rans/{step,sym}.bin` from `trace_rans.py`
/// (Animal_Bear first `0x110e270` call). Freqs inferred from the spread map:
/// `[5,1,1,0,1,0,1,1,0,1,3,6,13,23,8]` (symbols 3/5/8 unused). Rules out
/// FSE-style scatter and off-by-one slot indexing within each symbol run.
#[test]
fn rans_spread_bear_first_rans_m64() {
    const FREQS: [u16; 15] = [5, 1, 1, 0, 1, 0, 1, 1, 0, 1, 3, 6, 13, 23, 8];
    const STEP: [u32; 64] = [
        327680, 327681, 327682, 327683, 327684, 65536, 65536, 65536, 65536, 65536, 65536, 196608,
        196609, 196610, 393216, 393217, 393218, 393219, 393220, 393221, 851968, 851969, 851970,
        851971, 851972, 851973, 851974, 851975, 851976, 851977, 851978, 851979, 851980, 1507328,
        1507329, 1507330, 1507331, 1507332, 1507333, 1507334, 1507335, 1507336, 1507337, 1507338,
        1507339, 1507340, 1507341, 1507342, 1507343, 1507344, 1507345, 1507346, 1507347, 1507348,
        1507349, 1507350, 524288, 524289, 524290, 524291, 524292, 524293, 524294, 524295,
    ];
    const SYM: [u16; 64] = [
        0, 0, 0, 0, 0, 1, 2, 4, 6, 7, 9, 10, 10, 10, 11, 11, 11, 11, 11, 11, 12, 12, 12, 12, 12,
        12, 12, 12, 12, 12, 12, 12, 12, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
        13, 13, 13, 13, 13, 13, 13, 13, 14, 14, 14, 14, 14, 14, 14, 14,
    ];
    let t = rans_spread(6, &FREQS);
    assert_eq!(t.step, STEP);
    assert_eq!(t.sym, SYM);
}

/// Warm main-loop slice of the four-state rANS init (`0x110dfa0`).
///
/// Provenance: `capture_init6.py` / `verify_init_invariant.py` on `0x110dfa0`
/// return at `0x110e1b8` (Animal_Bear, the segment at stream `P+8044`,
/// `prod=228`, `log=5`, already-loaded states). The `(log, freqs)` are this
/// segment's own freq-reader+spread output — NOT a fixed table: across the 3
/// fixtures `log` ranges 3..11 and the freqs differ every segment (see
/// `capture_init_all.py`). `[28,3,1]` is simply Bear's data here. Rules out
/// treating init as decode-only renorm and using log=6 / the M=64 decode table.
#[test]
fn rans_init_states_bear_first_rans() {
    const INIT_FREQS: [u16; 3] = [28, 3, 1];
    const ST_IN: [u64; 4] = [0x15601103de, 0x7a4056e3de, 0x4939330469c, 0x1136b5c57093e];
    const ST_OUT: [u64; 4] = [
        0x1670c7fb0e5cc107,
        0x80581303,
        0x0e1e9623a87cf343,
        0x01321a08545304,
    ];
    let expected_bytes = hex_bytes(
        "0101010101010101010101000000000001000000000000000000000000000000\
             0000000000010000000000000001000001000000000000000000000000000100\
             0000000000000000010000000000000000000000000000000000020000000100\
             0000010000020000020001020002020202000000000000000000000000000000\
             0000000000000000000000000000000000000100000000000000000000000000\
             0000000200000000000000000001000000000000000000000000000000000000\
             0000000000000000000000000000000000000000000000000000000000000000\
             00000000",
    );
    let init_stream = hex_bytes("1c6c79053929c95b0ce6a98f0bb0c472ab757821cbb49d0d44d69beb2784028b");
    let t = rans_spread(5, &INIT_FREQS);
    let r = rans_init_states(&t, &init_stream, 228, 1, ST_IN).unwrap();
    assert_eq!(r.states, ST_OUT);
    assert_eq!(r.stream_used, 24);

    let mut out = vec![0u8; 228];
    let r = rans_decode_bytes_into(&mut out, &t, &init_stream, 228, 1, ST_IN).unwrap();
    assert_eq!(out, expected_bytes);
    assert_eq!(r.states, ST_OUT);
    assert_eq!(r.stream_used, 24);

    let mut bad_stream = init_stream.clone();
    bad_stream[0] ^= 1;
    assert_ne!(
        rans_init_states(&t, &bad_stream, 228, 1, ST_IN)
            .unwrap()
            .states,
        ST_OUT
    );
    assert!(matches!(
        rans_init_states(&t, &init_stream[..8], 228, 1, ST_IN),
        Err(RansInitError::StreamTooShort)
    ));
    assert!(matches!(
        rans_init_states(&t, &init_stream, 3, 1, ST_IN),
        Err(RansInitError::ProdTooSmall)
    ));
    assert!(matches!(
        rans_decode_bytes_into(&mut [0u8; 228], &t, &init_stream, 228, 0, ST_IN),
        Err(RansInitError::ZeroStride)
    ));
    assert!(matches!(
        rans_decode_bytes_into(&mut [0u8; 227], &t, &init_stream, 228, 1, ST_IN),
        Err(RansInitError::OutputTooSmall)
    ));
}

/// Byte-output tail coverage for `0x110dfa0`.
///
/// Provenance: `capture_rans_byte_decode.py`, Animal_Bass call 1:
/// `log=3,count=30,stride=1`, warm states, no renorm bytes consumed. The
/// `count & 3 == 2` tail writes lanes 0 and 1 after seven full groups.
#[test]
fn rans_decode_bytes_bass_tail_outputs_symbols() {
    let table = RansDecodeTable {
        log: 3,
        step: vec![
            458752, 458753, 458754, 458755, 458756, 458757, 458758, 65536,
        ],
        sym: vec![0, 0, 0, 0, 0, 0, 0, 1],
    };
    let states = [1002867111956613, 297734982922, 3174744965070, 1675156985512];
    let mut out = vec![0xff; 30];

    let result = rans_decode_bytes_into(&mut out, &table, &[], 30, 1, states).unwrap();

    assert_eq!(
        out,
        hex_bytes("000000000000000001000000000000000000000000010000000000000000")
    );
    assert_eq!(
        result,
        RansInitResult {
            states: [49227725862390, 14614913526, 1246709343325, 657827286397],
            flag: 0xf,
            stream_offset: 0,
            stream_used: 0,
        }
    );
}

/// Byte three-lane mode-1 decoder (`0x110eb50`) main loop plus scalar tail.
///
/// Provenance: `capture_segment_dispatch_byte.py`, Animal_Dragonfly dispatch
/// call 61, replayed by `verify_rans_byte_three_lane.py`: `count=21`,
/// `log=3`, stride 1. The first 12 symbols take the unrolled group path and
/// the remaining 9 symbols take the tail path.
#[test]
fn rans_three_lane_decode_bytes_dragonfly_main_and_tail() {
    let payload = sparse_payload(
        4622,
        &[
            (4277, "3281377012ded167"),
            (4584, "21fffe63ffffe917"),
            (4614, "c3f9ff5f37f7ff03"),
        ],
    );
    let table = [65536, 65536, 65536, 65536, 196609, 196611, 196612, 196613];
    let mut readers = [
        RansThreeLaneReader {
            ptr: 4277,
            acc: 10054460316793406170,
            bitpos: 61,
        },
        RansThreeLaneReader {
            ptr: 4584,
            acc: 5195271318892707844,
            bitpos: 59,
        },
        RansThreeLaneReader {
            ptr: 4614,
            acc: 8940803800127113216,
            bitpos: 57,
        },
    ];
    let mut out = hex_bytes("9e009e0108014b022400e401000049010000200000");

    rans_three_lane_decode_bytes_into(
        &mut out,
        RansByteThreeLaneDecodeSpec {
            count: 21,
            log: 3,
            stride: 1,
            table: &table,
            readers: &mut readers,
            payload: &payload,
        },
    )
    .unwrap();

    assert_eq!(out, hex_bytes("010000000105030104040000000000000000010000"));
    assert_eq!(
        readers,
        [
            RansThreeLaneReader {
                ptr: 4276,
                acc: 5706504233741557568,
                bitpos: 54
            },
            RansThreeLaneReader {
                ptr: 4585,
                acc: 14591074635563934200,
                bitpos: 56
            },
            RansThreeLaneReader {
                ptr: 4613,
                acc: 11596061540452667384,
                bitpos: 54
            },
        ]
    );

    assert_eq!(
        rans_three_lane_decode_bytes_into(
            &mut [0u8; 21],
            RansByteThreeLaneDecodeSpec {
                count: 21,
                log: 4,
                stride: 1,
                table: &table,
                readers: &mut readers,
                payload: &payload,
            },
        ),
        Err(RansThreeLaneDecodeError::TableSizeMismatch)
    );
    assert_eq!(
        rans_three_lane_decode_bytes_into(
            &mut [0u8; 21],
            RansByteThreeLaneDecodeSpec {
                count: 21,
                log: 3,
                stride: 0,
                table: &table,
                readers: &mut readers,
                payload: &payload,
            },
        ),
        Err(RansThreeLaneDecodeError::ZeroStride)
    );
    assert_eq!(
        rans_three_lane_decode_bytes_into(
            &mut [0u8; 20],
            RansByteThreeLaneDecodeSpec {
                count: 21,
                log: 3,
                stride: 1,
                table: &table,
                readers: &mut readers,
                payload: &payload,
            },
        ),
        Err(RansThreeLaneDecodeError::OutputTooSmall)
    );
    let mut truncated_readers = [
        RansThreeLaneReader {
            ptr: 4277,
            acc: 10054460316793406170,
            bitpos: 61,
        },
        RansThreeLaneReader {
            ptr: 4584,
            acc: 5195271318892707844,
            bitpos: 59,
        },
        RansThreeLaneReader {
            ptr: 4614,
            acc: 8940803800127113216,
            bitpos: 57,
        },
    ];
    assert_eq!(
        rans_three_lane_decode_bytes_into(
            &mut [0u8; 21],
            RansByteThreeLaneDecodeSpec {
                count: 21,
                log: 3,
                stride: 1,
                table: &table,
                readers: &mut truncated_readers,
                payload: &payload[..4280],
            },
        ),
        Err(RansThreeLaneDecodeError::PayloadTooSmall)
    );
}

/// Byte segment dispatch (`0x110dd80`) mode 0 feeding byte rANS (`0x110dfa0`).
///
/// Provenance: `capture_segment_dispatch_byte.py`, Animal_Bass call 1:
/// `mode=0,log=3,count=30,stride=1`, warm states, no renorm bytes consumed.
/// This covers the dispatcher's mode-0 branch and the `count & 3 == 2` tail.
#[test]
fn rans_segment_dispatch_bytes_mode0_tail_updates_output_cursor_and_states() {
    let mut state =
        RansStateBuffer::warm([1002867111956613, 297734982922, 3174744965070, 1675156985512]);
    let mut cursor = RansStreamCursor::default();
    let step = [
        458752, 458753, 458754, 458755, 458756, 458757, 458758, 65536,
    ];
    let sym = [0, 0, 0, 0, 0, 0, 0, 1];
    let mut out = vec![0xff; 30];

    let used = rans_segment_dispatch_bytes_into(
        &mut out,
        RansSegmentDispatchBytesSpec {
            mode: 0,
            log: 3,
            value: 0,
            count: 30,
            stride: 1,
            state: &mut state,
            step: &step,
            sym: &sym,
            stream: &[],
            payload: &[],
            cursor: &mut cursor,
            three_lane_readers: None,
        },
    )
    .unwrap();

    assert_eq!(used, 0);
    assert_eq!(cursor.offset, 0);
    assert_eq!(
        state,
        RansStateBuffer::warm([49227725862390, 14614913526, 1246709343325, 657827286397])
    );
    assert_eq!(
        out,
        hex_bytes("000000000000000001000000000000000000000000010000000000000000")
    );
}

/// Byte segment dispatch (`0x110dd80`) mode 2 feeding byte RLE (`0x110f800`).
///
/// Provenance: `capture_segment_dispatch_byte.py`, Animal_Dragonfly call 5:
/// `mode=2,value=1,count=3,stride=1`, whose byte fill overwrites `000001`
/// with `010101`.
#[test]
fn rans_segment_dispatch_bytes_mode2_rle_fills_dense_segment() {
    let mut state = RansStateBuffer::warm([
        290362338331826,
        324891473402,
        3329202948120618,
        1082470406632,
    ]);
    let mut cursor = RansStreamCursor::default();
    let mut out = hex_bytes("000001");

    let used = rans_segment_dispatch_bytes_into(
        &mut out,
        RansSegmentDispatchBytesSpec {
            mode: 2,
            log: 2,
            value: 1,
            count: 3,
            stride: 1,
            state: &mut state,
            step: &[],
            sym: &[],
            stream: &[],
            payload: &[],
            cursor: &mut cursor,
            three_lane_readers: None,
        },
    )
    .unwrap();

    assert_eq!(used, 0);
    assert_eq!(cursor.offset, 0);
    assert_eq!(
        state,
        RansStateBuffer::warm([
            290362338331826,
            324891473402,
            3329202948120618,
            1082470406632
        ])
    );
    assert_eq!(out, hex_bytes("010101"));
}

#[test]
fn rans_segment_dispatch_bytes_mode1_three_lane_bytes() {
    let payload = sparse_payload(
        4622,
        &[
            (4277, "3281377012ded167"),
            (4584, "21fffe63ffffe917"),
            (4614, "c3f9ff5f37f7ff03"),
        ],
    );
    let table = [65536, 65536, 65536, 65536, 196609, 196611, 196612, 196613];
    let mut readers = [
        RansThreeLaneReader {
            ptr: 4277,
            acc: 10054460316793406170,
            bitpos: 61,
        },
        RansThreeLaneReader {
            ptr: 4584,
            acc: 5195271318892707844,
            bitpos: 59,
        },
        RansThreeLaneReader {
            ptr: 4614,
            acc: 8940803800127113216,
            bitpos: 57,
        },
    ];
    let mut state = RansStateBuffer::warm([0x8000_0000; 4]);
    let mut cursor = RansStreamCursor::default();
    let mut out = hex_bytes("9e009e0108014b022400e401000049010000200000");

    let used = rans_segment_dispatch_bytes_into(
        &mut out,
        RansSegmentDispatchBytesSpec {
            mode: 1,
            log: 3,
            value: 0,
            count: 21,
            stride: 1,
            state: &mut state,
            step: &table,
            sym: &[],
            stream: &[],
            payload: &payload,
            cursor: &mut cursor,
            three_lane_readers: Some(&mut readers),
        },
    )
    .unwrap();

    assert_eq!(used, 0);
    assert_eq!(cursor.offset, 0);
    assert_eq!(out, hex_bytes("010000000105030104040000000000000000010000"));
    assert_eq!(
        readers,
        [
            RansThreeLaneReader {
                ptr: 4276,
                acc: 5706504233741557568,
                bitpos: 54
            },
            RansThreeLaneReader {
                ptr: 4585,
                acc: 14591074635563934200,
                bitpos: 56
            },
            RansThreeLaneReader {
                ptr: 4613,
                acc: 11596061540452667384,
                bitpos: 54
            },
        ]
    );
}

#[test]
fn rans_segment_dispatch_bytes_rejects_unported_and_bad_bounds() {
    let mut state = RansStateBuffer::warm([0x8000_0000; 4]);
    let mut cursor = RansStreamCursor::default();
    let mut out = [0u8; 4];
    assert_eq!(
        rans_segment_dispatch_bytes_into(
            &mut out,
            RansSegmentDispatchBytesSpec {
                mode: 1,
                log: 1,
                value: 0,
                count: 4,
                stride: 1,
                state: &mut state,
                step: &[],
                sym: &[],
                stream: &[],
                payload: &[],
                cursor: &mut cursor,
                three_lane_readers: None,
            },
        ),
        Err(RansSegmentDispatchBytesError::MissingThreeLaneReaders)
    );

    let mut state = RansStateBuffer::warm([0x8000_0000; 4]);
    let mut cursor = RansStreamCursor::default();
    assert_eq!(
        rans_segment_dispatch_bytes_into(
            &mut out,
            RansSegmentDispatchBytesSpec {
                mode: 9,
                log: 1,
                value: 0,
                count: 4,
                stride: 1,
                state: &mut state,
                step: &[],
                sym: &[],
                stream: &[],
                payload: &[],
                cursor: &mut cursor,
                three_lane_readers: None,
            },
        ),
        Err(RansSegmentDispatchBytesError::UnknownMode(9))
    );

    let mut state = RansStateBuffer::warm([0x8000_0000; 4]);
    let mut cursor = RansStreamCursor::default();
    assert_eq!(
        rans_segment_dispatch_bytes_into(
            &mut out,
            RansSegmentDispatchBytesSpec {
                mode: 2,
                log: 0,
                value: 1,
                count: 4,
                stride: 0,
                state: &mut state,
                step: &[],
                sym: &[],
                stream: &[],
                payload: &[],
                cursor: &mut cursor,
                three_lane_readers: None,
            },
        ),
        Err(RansSegmentDispatchBytesError::Rle(
            RansByteRleFillError::ZeroStride
        ))
    );

    let mut state = RansStateBuffer::warm([0x8000_0000; 4]);
    let mut cursor = RansStreamCursor::default();
    assert_eq!(
        rans_segment_dispatch_bytes_into(
            &mut out[..2],
            RansSegmentDispatchBytesSpec {
                mode: 2,
                log: 0,
                value: 1,
                count: 3,
                stride: 1,
                state: &mut state,
                step: &[],
                sym: &[],
                stream: &[],
                payload: &[],
                cursor: &mut cursor,
                three_lane_readers: None,
            },
        ),
        Err(RansSegmentDispatchBytesError::Rle(
            RansByteRleFillError::OutputTooSmall
        ))
    );

    let step = [65536, 65536];
    let sym = [0, 1];
    #[cfg(target_pointer_width = "64")]
    {
        let mut state = RansStateBuffer::warm([0x8000_0000; 4]);
        let mut cursor = RansStreamCursor::default();
        let large_count = (u32::MAX as usize) + 1;
        assert_eq!(
            rans_segment_dispatch_bytes_into(
                &mut out,
                RansSegmentDispatchBytesSpec {
                    mode: 0,
                    log: 1,
                    value: 0,
                    count: large_count,
                    stride: 1,
                    state: &mut state,
                    step: &step,
                    sym: &sym,
                    stream: &[],
                    payload: &[],
                    cursor: &mut cursor,
                    three_lane_readers: None,
                },
            ),
            Err(RansSegmentDispatchBytesError::CountTooLarge(large_count))
        );
    }

    let mut state = RansStateBuffer::warm([0x8000_0000; 4]);
    let mut cursor = RansStreamCursor::default();
    assert_eq!(
        rans_segment_dispatch_bytes_into(
            &mut out[..3],
            RansSegmentDispatchBytesSpec {
                mode: 0,
                log: 1,
                value: 0,
                count: 4,
                stride: 1,
                state: &mut state,
                step: &step,
                sym: &sym,
                stream: &[],
                payload: &[],
                cursor: &mut cursor,
                three_lane_readers: None,
            },
        ),
        Err(RansSegmentDispatchBytesError::Decode(
            RansInitError::OutputTooSmall
        ))
    );
}

/// Cold-start + continuation for the generic four-state init (`0x110dfa0`).
///
/// Provenance: `capture_init_all.py` + replay prototype on Animal_Bear
/// stream `P+1312`, calls 0 and 1. Call 0 has `flag=0`, so the branch at
/// `0x110dfc0` takes `0x110e1bc`, loads four seed states from the stream,
/// sets `flag |= 0xf`, and then runs the main loop. Call 1 reuses the same
/// stream descriptor, entering with `[x2+12] == 135` and writing back 187.
/// This rules out both the warm-only shortcut and resetting the forward
/// cursor to zero on continuation.
#[test]
fn rans_init_states_cold_start_and_shared_cursor_bear() {
    const COLD_FREQS: [u16; 5] = [95, 408, 7, 1, 1];
    const CONT_FREQS: [u16; 2] = [3, 29];
    const AFTER_COLD: [u64; 4] = [
        0x1bb7813ea643,
        0x0e82d56be41e2018,
        0x074b790c100e3297,
        0x6e08d7b8e2e,
    ];
    const AFTER_CONT: [u64; 4] = [
        0x4c075b8b626,
        0x027cdfde572a0f44,
        0x211ec0c84c35ac,
        0x1f2795027,
    ];
    let stream = hex_bytes(
        "8456b469510786ef7a6cd10c5407d3936e8849a3d90517c4fdb100186c6808c6\
             efdb29f2eb95f3c58808a258f1e4fb4cae50cdf6e3fc8a8e058f6afb2b90c85c\
             1fb1c369c65d11d916ee3b455c7e514e12136f4802a124282e1525441af02b4e\
             96c03a1724a554aa4ec34ff2b85f9845879b2612b4052877d15b42436042d3a\
             3eb34cf9faed765e95ae1ad8053d4b3883b8ab07455a19b5dd00fcbf44ce28ce\
             4011185d7efb208e226b25d215d61630ba4da975ec185be977e44d63162358fbd",
    );

    let cold_table = rans_spread(9, &COLD_FREQS);
    let cont_table = rans_spread(5, &CONT_FREQS);
    let mut state = RansStateBuffer::cold();
    let mut cursor = RansStreamCursor::default();

    let cold = rans_init_states_with_cursor(&cold_table, &stream, 1024, 1, &mut state, &mut cursor)
        .unwrap();
    assert_eq!(cold.states, AFTER_COLD);
    assert_eq!(cold.flag, 0xf);
    assert_eq!((cold.stream_used, cold.stream_offset), (135, 135));
    assert_eq!(state.flag, 0xf);
    assert_eq!(cursor.offset, 135);

    let cont = rans_init_states_with_cursor(&cont_table, &stream, 1024, 1, &mut state, &mut cursor)
        .unwrap();
    assert_eq!(cont.states, AFTER_CONT);
    assert_eq!((cont.stream_used, cont.stream_offset), (52, 187));
    assert_eq!(cursor.offset, 187);

    let warm_only = rans_init_states(&cold_table, &stream, 1024, 1, [0; 4])
        .unwrap()
        .states;
    assert_ne!(warm_only, AFTER_COLD);

    let mut reset_state = RansStateBuffer::warm(AFTER_COLD);
    let mut reset_cursor = RansStreamCursor::default();
    let reset = rans_init_states_with_cursor(
        &cont_table,
        &stream,
        1024,
        1,
        &mut reset_state,
        &mut reset_cursor,
    )
    .unwrap()
    .states;
    assert_ne!(reset, AFTER_CONT);
}

/// Second-model cold-start coverage for the generic init (`0x110dfa0`).
///
/// Provenance: `capture_init_all.py` + `verify_init_invariant.py` over all
/// fixtures, with the minimal inline bytes dumped by
/// `capture_init_bass_golden.py` to `local-assets/re/init_bass_p394_golden.json`.
/// Animal_Bass call 0 enters at stream `P+394` with `flag=0`, `log=7`,
/// `prod=568`, and freqs `[6,118,3,1]`. This cross-model cold call rules out
/// the warm-only implementation independently of the Bear continuation test.
#[test]
fn rans_init_states_cold_start_bass_p394() {
    const COLD_FREQS: [u16; 4] = [6, 118, 3, 1];
    const AFTER_COLD: [u64; 4] = [0x3901a31f71085, 0x4552634d0a, 0x2e32d8bbfce, 0x186072316a8];
    const WARM_ONLY_ZERO_STATES: [u64; 4] = [
        0x6911728ba,
        0x6d5b56bbf6335,
        0x8f75efe56b1a856,
        0x1ea79cf8bde9ddb,
    ];
    let stream = hex_bytes(
        "06b1d9f57282a6854e36f8f582058fa2643c3f96dbfe7fed52044497928f64d\
             6e86b8418cf7502f4d95c243c19f0e372b0e6680ab7e79cdd9e7f8a31da5742a7",
    );
    let table = rans_spread(7, &COLD_FREQS);
    let mut state = RansStateBuffer::cold();
    let mut cursor = RansStreamCursor::default();

    let cold =
        rans_init_states_with_cursor(&table, &stream, 568, 1, &mut state, &mut cursor).unwrap();
    assert_eq!(cold.states, AFTER_COLD);
    assert_eq!(cold.flag, 0xf);
    assert_eq!((cold.stream_used, cold.stream_offset), (58, 58));
    assert_eq!(state.flag, 0xf);
    assert_eq!(cursor.offset, 58);

    let warm_only = rans_init_states(&table, &stream, 568, 1, [0; 4])
        .unwrap()
        .states;
    assert_eq!(warm_only, WARM_ONLY_ZERO_STATES);
    assert_ne!(warm_only, AFTER_COLD);
}

#[test]
fn rans_init_states_rejects_truncated_cold_loader() {
    let table = rans_spread(9, &[95, 408, 7, 1, 1]);
    let mut state = RansStateBuffer::cold();
    let mut cursor = RansStreamCursor::default();
    assert!(matches!(
        rans_init_states_with_cursor(&table, &[0x84, 0x56], 1024, 1, &mut state, &mut cursor),
        Err(RansInitError::StreamTooShort)
    ));
}

/// Spread → init (`0x110dfa0`) → decode: Bear first rANS without hardcoded states.
///
/// Provenance: `trace_rans.py` decode oracle; init from `capture_init6.py`.
#[test]
fn rans_spread_init_then_decode_bear_first_rans() {
    const DEC_FREQS: [u16; 15] = [5, 1, 1, 0, 1, 0, 1, 1, 0, 1, 3, 6, 13, 23, 8];
    const INIT_FREQS: [u16; 3] = [28, 3, 1];
    const ST_IN: [u64; 4] = [0x15601103de, 0x7a4056e3de, 0x4939330469c, 0x1136b5c57093e];
    let init_stream = hex_bytes("1c6c79053929c95b0ce6a98f0bb0c472ab757821cbb49d0d44d69beb2784028b");
    let decode_stream = hex_bytes(
        "44d69beb2784028b6a39382a036f90a250ebc749203fa34e0d60353e5071548d51aa7a26\
             943ad95a422eea145dab83d860ba542ed7bf85ec1c78e11fedddfb9ceaf8b9031988e12f",
    );
    let init_tbl = rans_spread(5, &INIT_FREQS);
    let states = rans_init_states(&init_tbl, &init_stream, 228, 1, ST_IN)
        .unwrap()
        .states;
    let dec_tbl = rans_spread(6, &DEC_FREQS);
    let out = rans_decode(RansDecodeSpec {
        count: 228,
        log: 6,
        stride: 1,
        step: &dec_tbl.step,
        sym: &dec_tbl.sym,
        init_states: states,
        stream: &decode_stream,
    })
    .unwrap();
    assert_eq!(out.len(), 228);
    assert_eq!(
        &out[..24],
        &[4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 10, 4, 2, 12, 13, 12, 10, 13, 10]
    );
    assert_eq!(&out[220..], &[14, 13, 13, 14, 14, 13, 14, 13]);
}

/// The rANS decoder reproduces a real decoded symbol stream (the first
/// vertex-coder rANS call of Animal_Bear), validated against the emulator.
#[test]
fn rans_decode_matches_oracle() {
    // Decode table (step[64] = (freq<<16)|low, sym[64] spread map), log2(M)=6.
    const STEP: [u32; 64] = [
        327680, 327681, 327682, 327683, 327684, 65536, 65536, 65536, 65536, 65536, 65536, 196608,
        196609, 196610, 393216, 393217, 393218, 393219, 393220, 393221, 851968, 851969, 851970,
        851971, 851972, 851973, 851974, 851975, 851976, 851977, 851978, 851979, 851980, 1507328,
        1507329, 1507330, 1507331, 1507332, 1507333, 1507334, 1507335, 1507336, 1507337, 1507338,
        1507339, 1507340, 1507341, 1507342, 1507343, 1507344, 1507345, 1507346, 1507347, 1507348,
        1507349, 1507350, 524288, 524289, 524290, 524291, 524292, 524293, 524294, 524295,
    ];
    const SYM: [u16; 64] = [
        0, 0, 0, 0, 0, 1, 2, 4, 6, 7, 9, 10, 10, 10, 11, 11, 11, 11, 11, 11, 12, 12, 12, 12, 12,
        12, 12, 12, 12, 12, 12, 12, 12, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
        13, 13, 13, 13, 13, 13, 13, 13, 14, 14, 14, 14, 14, 14, 14, 14,
    ];
    let states = [
        0x1670c7fb0e5cc107u64,
        0x80581303,
        0x0e1e9623a87cf343,
        0x01321a08545304,
    ];
    let stream = hex_bytes(
        "44d69beb2784028b6a39382a036f90a250ebc749203fa34e0d60353e5071548d51aa7a26\
             943ad95a422eea145dab83d860ba542ed7bf85ec1c78e11fedddfb9ceaf8b9031988e12f",
    );
    let out = rans_decode(RansDecodeSpec {
        count: 228,
        log: 6,
        stride: 1,
        step: &STEP,
        sym: &SYM,
        init_states: states,
        stream: &stream,
    })
    .unwrap();
    assert_eq!(out.len(), 228);
    assert_eq!(
        &out[..24],
        &[4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 10, 4, 2, 12, 13, 12, 10, 13, 10]
    );
    assert_eq!(&out[220..], &[14, 13, 13, 14, 14, 13, 14, 13]);
    assert_eq!(out.iter().map(|&s| s as u32).sum::<u32>(), 2565);
}

/// Discriminating tail decode: `count % 4 != 0`, so the leftover symbols
/// exercise the tail loop (`0x110e410`). The tail must continue lanes 0,1,…
/// (tail symbol `k` from `states[k]`), NOT decode every leftover from
/// `states[0]`.
///
/// Provenance: `capture_decode_tail_golden.py`, Animal_Bear's 4th `0x110e270`
/// call (`count=142`, `tail=2`, `log=6`, `stride=1`). With the wrong
/// `states[0]`-only tail the last symbol decodes to `12`; the emulator (and
/// the `states[k]` rule, confirmed by the `str x17,[x0],#8` post-increment)
/// gives `14`. Also cross-checked against Animal_Bass call #1 (`count=194`,
/// `tail=2`) in `audit_decode_spread.py`.
#[test]
fn rans_decode_tail_continues_lanes() {
    const FREQS: [u16; 15] = [1, 0, 0, 0, 1, 1, 1, 1, 1, 1, 2, 3, 13, 21, 18];
    let states = [0x1bdb9fbf46u64, 0xbccd27a202, 0x424e240141, 0x13610be603];
    let stream = hex_bytes(
        "c9f18c17d9d5062f2be62f960821c609377ce810a26db5967caa56af741a01f3\
             e7238315b6ebcb1ce861fedaff21bcbd",
    );
    let t = rans_spread(6, &FREQS);
    let out = rans_decode(RansDecodeSpec {
        count: 142,
        log: 6,
        stride: 1,
        step: &t.step,
        sym: &t.sym,
        init_states: states,
        stream: &stream,
    })
    .unwrap();
    assert_eq!(out.len(), 142);
    assert_eq!(&out[..8], &[9, 5, 4, 6, 11, 10, 8, 13]);
    // The two-symbol tail: lanes 0 and 1 continue -> [14, 14]. The
    // `states[0]`-only shortcut would yield [14, 12] here.
    assert_eq!(&out[138..], &[12, 13, 14, 14]);
}

/// Stride-3 output layout for `0x110e270`.
///
/// Provenance: `capture_decode_stride3.py` on Animal_Bass call 2
/// (`prod=960`, decoded `w2=320`, `stride=3`, `log=5`). The wrapper at
/// `0x110de14..0x110de48` stores the product at `x1+8` but passes `w2` at
/// `x1+0xc` to `0x110e270`; the decode loop stores symbol `i` at
/// `out[i*stride]` (`strh w22,[x11]`, then `add x11,x11,x20` where
/// `x20 = 4*stride*2`). Sibling lanes are not touched by this call. This
/// rules out both the old count-sized buffer and a dense stride-1 writer.
#[test]
fn rans_decode_stride3_writes_lane_slots() {
    const STEP: [u32; 32] = [
        1966080, 1966081, 1966082, 1966083, 1966084, 1966085, 1966086, 1966087, 1966088, 1966089,
        1966090, 1966091, 1966092, 1966093, 1966094, 1966095, 1966096, 1966097, 1966098, 1966099,
        1966100, 1966101, 1966102, 1966103, 1966104, 1966105, 1966106, 1966107, 1966108, 1966109,
        65536, 65536,
    ];
    const SYM: [u16; 32] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        2, 1022,
    ];
    let states = [0x68007d0ef80f, 0x674a999ea5a, 0x647f7484a3f513e, 0xd7a40fe0];
    let expected_lane = hex_u16s(
        "00000000020000000000000000000000000000000000000000000000000000000000000000000000\
             0000000000000000000000000000000000000000fe030000fe030000fe0300000000000000000000\
             00000000fe03fe030000000000000000020002000200000000000200000000000200000002000000\
             02000000020000000000000000000000000000000000000000000000000000000000000000000000\
             00000000000000000000000000000000000000000000000000000000000000000000000000000000\
             00000000000000000000000000000000000000000000000000000000000000000000000000000000\
             00000000000000000000000000000000000000000000000000000000000000000000000000000000\
             00000000000000000000000000000000000000000000000000000000000000000000000000000000\
             00000000000000000000000000000000000000000000000000000000000000000000000000000000\
             00000000000000000000000000000000000000000000000000000000000000000000000000000000\
             00000000020000000000000000000000000000000000000000000000000000000000000000000000\
             00000000000000000000000000000000000000000000000000000000000000000000000000000000\
             00000000000000000000000000000000000000000000000000000000000000000000000000000000\
             00000000000000000000000000000000000000000000000000000000000000000000000000000000\
             fe0300000000000000000000000000000000000000000000fe030000000000000200000000000000\
             02000000020000000000000000000000000000000000000000000000000000000000000000000000",
    );
    assert_eq!(expected_lane.len(), 320);
    let stream = hex_bytes("87e2163f7eff1a365ef3a7f6a5e841a3");
    let spec = RansDecodeSpec {
        count: 320,
        log: 5,
        stride: 3,
        step: &STEP,
        sym: &SYM,
        init_states: states,
        stream: &stream,
    };

    let mut out = vec![0xbeefu16; 960];
    let used = rans_decode_into(&mut out, spec).unwrap();
    assert_eq!(used, 16);
    for (i, &expected) in expected_lane.iter().enumerate() {
        assert_eq!(out[i * 3], expected, "lane symbol {i}");
    }
    assert!(out
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 3 != 0)
        .all(|(_, &v)| v == 0xbeef));

    let fresh = rans_decode(spec).unwrap();
    assert_eq!(fresh.len(), 960);
    for (i, &expected) in expected_lane.iter().enumerate() {
        assert_eq!(fresh[i * 3], expected, "fresh lane symbol {i}");
    }
    assert!(fresh
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 3 != 0)
        .all(|(_, &v)| v == 0));

    let mut too_small = vec![0u16; 320];
    assert_eq!(
        rans_decode_into(&mut too_small, spec),
        Err(RansDecodeError::OutputTooSmall)
    );
}

/// RLE fill helper used by the segment dispatch (`0x110f930`).
///
/// Provenance: `capture_rle_fill.py` over Bear/Bass/Dragonfly. Observed
/// calls are Bass `value=0,count=2,stride=3`, Bass `value=0,count=322,stride=3`
/// twice, and Dragonfly `value=11,count=3,stride=1`. The strided Bass case
/// rules out a dense fill that would overwrite sibling lanes.
#[test]
fn rans_rle_fill_matches_observed_stride_and_dense_cases() {
    let mut bass = vec![0xbeefu16; 322 * 3];
    rans_rle_fill(&mut bass, 0, 322, 3).unwrap();
    for i in 0..322 {
        assert_eq!(bass[i * 3], 0);
    }
    assert!(bass
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 3 != 0)
        .all(|(_, &v)| v == 0xbeef));

    let mut dragonfly = vec![0u16; 3];
    rans_rle_fill(&mut dragonfly, 11, 3, 1).unwrap();
    assert_eq!(dragonfly, [11, 11, 11]);

    let mut too_small = vec![0u16; 322];
    assert_eq!(
        rans_rle_fill(&mut too_small, 0, 322, 3),
        Err(RansRleFillError::OutputTooSmall)
    );
    assert_eq!(
        rans_rle_fill(&mut bass, 0, 1, 0),
        Err(RansRleFillError::ZeroStride)
    );
}

/// Short segment-header form for mode 0 (`0x110de80`).
///
/// Provenance: `capture_segment_dispatch.py`, Animal_Bear table-build 1.
/// The top bit is clear and the 4-bit class is nonzero, so
/// `0x110deb4..0x110df54` yields mode 0, log 9, and table count 5.
#[test]
fn rans_segment_header_short_mode0() {
    let payload = hex_bytes("1117eda742422e81");
    let header = rans_read_segment_header(
        &payload,
        RansFreqReader {
            ptr: 0,
            acc: 0x227f1b04e40cc5e4,
            bitpos: 61,
        },
    )
    .unwrap();
    assert_eq!(
        header,
        RansSegmentHeader {
            mode: 0,
            log: 9,
            table_count: Some(5),
            value: 0,
            reader: RansFreqReader {
                ptr: 0,
                acc: 0xfc6c139033179000,
                bitpos: 51,
            },
        }
    );
}

/// Long segment-header form for mode 1 (`0x110de80`).
///
/// Provenance: `capture_segment_dispatch.py`, Animal_Bear table-build 0.
/// This covers the long path at `0x110dec4..0x110df60`; the `csel ... eq`
/// polarity after `tst` is the discriminating rule.
#[test]
fn rans_segment_header_long_mode1() {
    let payload = hex_bytes("5555f9abcff355b5");
    let header = rans_read_segment_header(
        &payload,
        RansFreqReader {
            ptr: 0,
            acc: 0x0c736b6abdf6deec,
            bitpos: 58,
        },
    )
    .unwrap();
    assert_eq!(
        header,
        RansSegmentHeader {
            mode: 1,
            log: 1,
            table_count: Some(2),
            value: 0,
            reader: RansFreqReader {
                ptr: 0,
                acc: 0xcdadaaf7db7bb400,
                bitpos: 48,
            },
        }
    );
}

/// Long segment-header form selecting the mid-width count.
///
/// Provenance: `capture_segment_dispatch.py`, Animal_Bear table-build 6.
/// This covers the `high_bits`/`wide_bits` branch where the second `tst`
/// finds a clear top bit and selects the 12-bit form.
#[test]
fn rans_segment_header_long_mid_width() {
    let payload = hex_bytes("00a03c61c56d6014");
    let header = rans_read_segment_header(
        &payload,
        RansFreqReader {
            ptr: 0,
            acc: 0xbf044239f0e66c14,
            bitpos: 56,
        },
    )
    .unwrap();
    assert_eq!(
        header,
        RansSegmentHeader {
            mode: 0,
            log: 8,
            table_count: Some(248),
            value: 0,
            reader: RansFreqReader {
                ptr: 0,
                acc: 0x8473e1ccd8280000,
                bitpos: 39,
            },
        }
    );
}

/// Long segment-header low-count form still takes mode/log from `bits << 9`.
///
/// Provenance: `capture_vertex_writer_loop.py`, Animal_Bear current 0,
/// writer-source 0, descriptor 1. This rules out reading mode/log from the
/// earlier `bits << 5` stage, which mis-parses the same header as mode 0,
/// log 5, table count 63.
#[test]
fn rans_segment_header_long_low_count_uses_high_bits() {
    let payload = hex_bytes("00c6fd3b903638d907");
    let header = rans_read_segment_header(
        &payload,
        RansFreqReader {
            ptr: 1,
            acc: 0xf16007396341c038,
            bitpos: 53,
        },
    )
    .unwrap();
    assert_eq!(
        header,
        RansSegmentHeader {
            mode: 1,
            log: 8,
            table_count: Some(63),
            value: 0,
            reader: RansFreqReader {
                ptr: 0,
                acc: 0x01ce58d0700f8000,
                bitpos: 47,
            },
        }
    );
}

/// Long segment-header form selecting the widest observed count.
///
/// Provenance: `capture_segment_dispatch.py`, Animal_Bear table-build 26.
/// This covers the `high_bits`/`wider_bits` branch where the long path
/// consumes 19 header bits before reading mode/log.
#[test]
fn rans_segment_header_long_wide_width() {
    let payload = hex_bytes("a814d2b6402520a7");
    let header = rans_read_segment_header(
        &payload,
        RansFreqReader {
            ptr: 0,
            acc: 0xbf401a00400000f4,
            bitpos: 59,
        },
    )
    .unwrap();
    assert_eq!(
        header,
        RansSegmentHeader {
            mode: 1,
            log: 10,
            table_count: Some(760),
            value: 0,
            reader: RansFreqReader {
                ptr: 0,
                acc: 0x00400000f4000000,
                bitpos: 35,
            },
        }
    );
}

/// RLE segment-header value form for mode 2 (`0x110de80`).
///
/// Provenance: `capture_segment_dispatch.py`, Animal_Dragonfly table-build
/// 18. The short class is zero, so `0x110df64..0x110df98` decodes the value
/// varint and returns without a table build.
#[test]
fn rans_segment_header_rle_value127() {
    let payload = hex_bytes("f1a106940000623a");
    let header = rans_read_segment_header(
        &payload,
        RansFreqReader {
            ptr: 0,
            acc: 0x03fbfd0221c04704,
            bitpos: 59,
        },
    )
    .unwrap();
    assert_eq!(
        header,
        RansSegmentHeader {
            mode: 2,
            log: 0,
            table_count: None,
            value: 127,
            reader: RansFreqReader {
                ptr: 0,
                acc: 0x7fa0443808e0e000,
                bitpos: 46,
            },
        }
    );
    assert_eq!(
        rans_read_segment_header(
            &payload[..7],
            RansFreqReader {
                ptr: 0,
                acc: 0x03fbfd0221c04704,
                bitpos: 59,
            },
        ),
        Err(RansSegmentHeaderError::PayloadTooSmall)
    );
}

/// Mode-0 table builder (`0x110e540`) small-symbol branch.
///
/// Provenance: `capture_segment_dispatch.py`, Animal_Bear table-build 2,
/// replayed by `verify_mode0_table_builder.py`. This covers the
/// `count <= 10` symbol-list loop at `0x110e578..0x110e60c` and the
/// `w4=15` frequency-reader call.
#[test]
fn rans_mode0_table_builder_small_branch() {
    const STEP: [u32; 32] = [
        196608, 196609, 196610, 1900544, 1900545, 1900546, 1900547, 1900548, 1900549, 1900550,
        1900551, 1900552, 1900553, 1900554, 1900555, 1900556, 1900557, 1900558, 1900559, 1900560,
        1900561, 1900562, 1900563, 1900564, 1900565, 1900566, 1900567, 1900568, 1900569, 1900570,
        1900571, 1900572,
    ];
    const SYM: [u16; 32] = [
        0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
        1, 1,
    ];
    let payload = hex_bytes("cb8b88ff4f53860f38");
    let built = rans_build_mode0_table(
        &payload,
        RansFreqReader {
            ptr: 1,
            acc: 0xc84854fda2e22400,
            bitpos: 51,
        },
        2,
        5,
    )
    .unwrap();
    assert_eq!(built.symbols, [0, 1]);
    assert_eq!(built.freqs, [3, 29]);
    assert_eq!(built.table.step, STEP);
    assert_eq!(built.table.sym, SYM);
    assert_eq!(
        built.reader,
        RansFreqReader {
            ptr: 0,
            acc: 0x2153f68b889c0700,
            bitpos: 49,
        }
    );

    assert_eq!(
        rans_build_mode0_table(
            &payload[..8],
            RansFreqReader {
                ptr: 1,
                acc: 0xc84854fda2e22400,
                bitpos: 51,
            },
            2,
            5,
        ),
        Err(RansMode0TableBuildError::PayloadTooSmall)
    );
}

/// Mode-0 table builder (`0x110e540`) large-symbol branch.
///
/// Provenance: `capture_segment_dispatch.py`, Animal_Bear table-build 25,
/// replayed by `verify_mode0_table_builder.py`. This covers the
/// `count > 10` call to `0x110e9a0`, the `w4=14` frequency-reader call, and
/// the contiguous sparse spread into the descriptor's step/symbol tables.
#[test]
fn rans_mode0_table_builder_large_branch() {
    const STEP: [u32; 64] = [
        327680, 327681, 327682, 327683, 327684, 65536, 65536, 65536, 65536, 65536, 65536, 196608,
        196609, 196610, 393216, 393217, 393218, 393219, 393220, 393221, 851968, 851969, 851970,
        851971, 851972, 851973, 851974, 851975, 851976, 851977, 851978, 851979, 851980, 1507328,
        1507329, 1507330, 1507331, 1507332, 1507333, 1507334, 1507335, 1507336, 1507337, 1507338,
        1507339, 1507340, 1507341, 1507342, 1507343, 1507344, 1507345, 1507346, 1507347, 1507348,
        1507349, 1507350, 524288, 524289, 524290, 524291, 524292, 524293, 524294, 524295,
    ];
    const SYM: [u16; 64] = [
        0, 0, 0, 0, 0, 1, 2, 4, 6, 7, 9, 10, 10, 10, 11, 11, 11, 11, 11, 11, 12, 12, 12, 12, 12,
        12, 12, 12, 12, 12, 12, 12, 12, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
        13, 13, 13, 13, 13, 13, 13, 13, 14, 14, 14, 14, 14, 14, 14, 14,
    ];
    let payload = hex_bytes("d2b6402520a707000002d000faed3d27");
    let built = rans_build_mode0_table(
        &payload,
        RansFreqReader {
            ptr: 8,
            acc: 0xbaf109e148a24c00,
            bitpos: 47,
        },
        12,
        6,
    )
    .unwrap();
    assert_eq!(built.symbols, [0, 1, 2, 4, 6, 7, 9, 10, 11, 12, 13, 14]);
    assert_eq!(built.freqs, [5, 1, 1, 1, 1, 1, 1, 3, 6, 13, 23, 8]);
    assert_eq!(built.table.step, STEP);
    assert_eq!(built.table.sym, SYM);
    assert_eq!(
        built.reader,
        RansFreqReader {
            ptr: 0,
            acc: 0xf7b7e80340080000,
            bitpos: 54,
        }
    );

    assert_eq!(
        rans_build_mode0_table(&payload, built.reader, 0, 6),
        Err(RansMode0TableBuildError::TableCountZero)
    );
    assert_eq!(
        rans_build_mode0_table(&payload, built.reader, 12, 12),
        Err(RansMode0TableBuildError::UnsupportedLog(12))
    );
    assert_eq!(
        rans_build_mode0_table(&payload, built.reader, 65, 6),
        Err(RansMode0TableBuildError::TableCountExceedsMass {
            count: 65,
            mass: 64
        })
    );
}

/// Dense mode-0 table builder with a stale run-remainder discriminator.
///
/// Provenance: `capture_vertex_writer_loop.py`, Animal_Bear current 0,
/// writer-source 0, descriptor 2. This covers a 244-of-256 sparse table; a
/// reader that misses `mov w18, wzr` at `0x110e8e8` carries an old
/// post-run count into the next run body and rejects this as overfull mass.
#[test]
fn rans_mode0_table_builder_dense_count_resets_run_remainder() {
    let payload = hex_bytes(
        "c1e6220cc4f5f1430f8832ce2439be8085c2e282850c2c1c272e2c5bb80612\
             f0e20b9bcf7a37666cde",
    );
    let header = rans_read_segment_header(
        &payload,
        RansFreqReader {
            ptr: 33,
            acc: 0x9f0446609f0c0000,
            bitpos: 44,
        },
    )
    .unwrap();
    assert_eq!(
        header,
        RansSegmentHeader {
            mode: 0,
            log: 8,
            table_count: Some(244),
            value: 0,
            reader: RansFreqReader {
                ptr: 31,
                acc: 0x8cc13e1bcd8c0000,
                bitpos: 43,
            },
        }
    );

    let built = rans_build_mode0_table(&payload, header.reader, 244, 8).unwrap();
    assert_eq!(built.symbols.len(), 244);
    assert_eq!(built.freqs.len(), 244);
    assert_eq!(
        built.freqs.iter().map(|&freq| freq as u32).sum::<u32>(),
        256
    );
    assert_eq!(
        built.reader,
        RansFreqReader {
            ptr: 0,
            acc: 0x5f1c9267194407a1,
            bitpos: 57,
        }
    );
}

/// Mode-1 table builder (`0x110f3c0`) special `log < 2` path.
///
/// Provenance: `capture_segment_dispatch.py`, Animal_Bear table-build 0,
/// replayed by `verify_mode1_table_builder.py`. This covers
/// `0x110f3d8` branching to `0x110f498`, then the final single-bit table
/// expansion consumed by `0x110ef70`.
#[test]
fn rans_mode1_table_builder_log1() {
    let payload = hex_bytes("55c55555f9abcff355b5");
    let built = rans_build_mode1_table(
        &payload,
        RansFreqReader {
            ptr: 2,
            acc: 0xcdadaaf7db7bb400,
            bitpos: 48,
        },
        2,
        1,
    )
    .unwrap();
    assert_eq!(built.table, [65536, 65537]);
    assert_eq!(
        built.reader,
        RansFreqReader {
            ptr: 0,
            acc: 0x36b6abdf6deed556,
            bitpos: 62,
        }
    );
    assert_eq!(
        rans_build_mode1_table(
            &payload[..9],
            RansFreqReader {
                ptr: 2,
                acc: 0xcdadaaf7db7bb400,
                bitpos: 48,
            },
            2,
            1,
        ),
        Err(RansMode1TableBuildError::PayloadTooSmall)
    );
}

/// Mode-1 table builder (`0x110f3c0`) general prefix-table expansion.
///
/// Provenance: `capture_segment_dispatch.py`, Animal_Bass table-build 7,
/// replayed by `verify_mode1_table_builder.py`. This covers the
/// length-count reader, grouped sparse-symbol reader, and replicated table
/// expansion at `0x110f558..0x110f718`.
#[test]
fn rans_mode1_table_builder_log4() {
    const TABLE: [u32; 16] = [
        65547, 65547, 65547, 65547, 65547, 65547, 65547, 65547, 262144, 262145, 262148, 262149,
        262151, 262152, 262153, 262154,
    ];
    let payload = hex_bytes("cd090074f80f006a8a3fe021");
    let built = rans_build_mode1_table(
        &payload,
        RansFreqReader {
            ptr: 4,
            acc: 0x806575ea0e731000,
            bitpos: 53,
        },
        9,
        4,
    )
    .unwrap();
    assert_eq!(built.table, TABLE);
    assert_eq!(
        built.reader,
        RansFreqReader {
            ptr: 0,
            acc: 0xa839cc443c07f14c,
            bitpos: 59,
        }
    );
    assert_eq!(
        rans_build_mode1_table(&payload, built.reader, 0, 4),
        Err(RansMode1TableBuildError::TableCountZero)
    );
    assert_eq!(
        rans_build_mode1_table(&payload, built.reader, 9, 0),
        Err(RansMode1TableBuildError::UnsupportedLog(0))
    );
    assert_eq!(
        rans_build_mode1_table(&payload, built.reader, 17, 4),
        Err(RansMode1TableBuildError::TableCountExceedsMass {
            count: 17,
            mass: 16
        })
    );
}

/// Segment descriptor builder (`0x110de80`) feeding mode-0 dispatch (`0x110de00`).
///
/// Provenance: `capture_segment_dispatch.py`, Animal_Bear table-build 25 +
/// dispatch 3. The descriptor is now built from the reverse stream
/// (`count=12,log=6`) and then passed to the mode-0 dispatch wrapper. This
/// proves the header/table builder output satisfies the existing rANS
/// dispatch contract.
#[test]
fn rans_segment_dispatch_mode0_updates_output_cursor_and_states() {
    const FINAL_STATES: [u64; 4] = [0x3e69fd3c25, 0x1eb070dcc1aa, 0x7af49bc9b0fd, 0x14d2c33bcc];
    let descriptor_payload = hex_bytes("d2b6402520a707000002d000faed3d27");
    let descriptor = rans_build_segment_descriptor(
        &descriptor_payload,
        RansFreqReader {
            ptr: 8,
            acc: 0x59aebc4278522890,
            bitpos: 57,
        },
    )
    .unwrap();
    assert_eq!(descriptor.mode, 0);
    assert_eq!(descriptor.log, 6);
    assert_eq!(
        descriptor.reader,
        RansFreqReader {
            ptr: 0,
            acc: 0xf7b7e80340080000,
            bitpos: 54,
        }
    );
    let mut states = [
        0x1670c7fb0e5cc107u64,
        0x80581303,
        0x0e1e9623a87cf343,
        0x01321a08545304,
    ];
    let stream = hex_bytes(
        "44d69beb2784028b6a39382a036f90a250ebc749203fa34e0d60353e5071548d51aa7a26\
             943ad95a422eea145dab83d860ba542ed7bf85ec1c78e11fedddfb9ceaf8b9031988e12f",
    );
    let mut out = vec![0xbeefu16; 228];

    let used = rans_segment_dispatch_into(
        &mut out,
        RansSegmentDispatchSpec {
            mode: descriptor.mode,
            log: descriptor.log,
            value: descriptor.value,
            count: 228,
            stride: 1,
            states: &mut states,
            step: &descriptor.step,
            sym: &descriptor.sym,
            stream: &stream,
            payload: &[],
            three_lane_readers: None,
        },
    )
    .unwrap();

    assert_eq!(used, 68);
    assert_eq!(states, FINAL_STATES);
    assert_eq!(
        &out[..24],
        &[4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 10, 4, 2, 12, 13, 12, 10, 13, 10]
    );
    assert_eq!(&out[220..], &[14, 13, 13, 14, 14, 13, 14, 13]);
    assert_eq!(out.iter().map(|&s| s as u32).sum::<u32>(), 2565);
}

/// Segment descriptor builder (`0x110de80`) feeding mode-2 RLE dispatch.
///
/// Provenance: `capture_segment_dispatch.py`, Animal_Dragonfly table-build
/// 14 + dispatch 5: `mode=2,value=11,count=3,stride=1`.
#[test]
fn rans_segment_dispatch_mode2_rle_fills_dense_segment() {
    let descriptor_payload = hex_bytes("d244781d50f180ec");
    let descriptor = rans_build_segment_descriptor(
        &descriptor_payload,
        RansFreqReader {
            ptr: 0,
            acc: 0x005f479f43193ebc,
            bitpos: 59,
        },
    )
    .unwrap();
    assert_eq!(descriptor.mode, 2);
    assert_eq!(descriptor.value, 11);
    assert_eq!(
        descriptor.reader,
        RansFreqReader {
            ptr: 0,
            acc: 0xe8f3e86327d7a000,
            bitpos: 46,
        }
    );
    let mut states = [0u64; 4];
    let mut out = [6u16, 6, 120];
    let used = rans_segment_dispatch_into(
        &mut out,
        RansSegmentDispatchSpec {
            mode: descriptor.mode,
            log: descriptor.log,
            value: descriptor.value,
            count: 3,
            stride: 1,
            states: &mut states,
            step: &descriptor.step,
            sym: &descriptor.sym,
            stream: &[],
            payload: &[],
            three_lane_readers: None,
        },
    )
    .unwrap();
    assert_eq!(used, 0);
    assert_eq!(out, [11, 11, 11]);
}

#[test]
fn rans_segment_dispatch_mode1_requires_reader_state() {
    let mut states = [0u64; 4];
    let mut out = [0u16; 1];
    assert_eq!(
        rans_segment_dispatch_into(
            &mut out,
            RansSegmentDispatchSpec {
                mode: 1,
                log: 4,
                value: 0,
                count: 1,
                stride: 1,
                states: &mut states,
                step: &[],
                sym: &[],
                stream: &[],
                payload: &[],
                three_lane_readers: None,
            },
        ),
        Err(RansSegmentDispatchError::MissingThreeLaneReaders)
    );
}

/// Three-lane mode-1 decoder (`0x110ef70`) main loop (`count >= 12`).
///
/// Provenance: `capture_segment_dispatch.py`, Animal_Bass dispatch 13
/// (`mode=1,log=3,count=12,stride=1`). This covers the group-of-12 path at
/// `0x110f030..0x110f1b4`, including reader 1's `rev` load and forward
/// pointer movement. It rules out treating all three readers as the same
/// backwards little-endian reader.
#[test]
fn rans_three_lane_decode_bass_count12_main_loop() {
    const TABLE: [u32; 8] = [
        0x0002_0000,
        0x0002_0000,
        0x0003_0001,
        0x0003_0004,
        0x0003_0007,
        0x0003_0009,
        0x0003_000a,
        0x0003_000b,
    ];
    let payload = sparse_payload(
        834,
        &[
            (1, "c1927cb097255b04"),
            (826, "c1d38107e0871e88"),
            (814, "80a21d403e668f86"),
        ],
    );
    let mut readers = [
        RansThreeLaneReader {
            ptr: 1,
            acc: 0x84b8f4310101be08,
            bitpos: 55,
        },
        RansThreeLaneReader {
            ptr: 826,
            acc: 0x668f867594cc9ac0,
            bitpos: 56,
        },
        RansThreeLaneReader {
            ptr: 814,
            acc: 0x0e9e0cd664a3ac00,
            bitpos: 53,
        },
    ];
    let mut out = [0u16, 2, 6464, 0, 8, 2, 6400, 0, 0, 3, 6384, 0];
    rans_three_lane_decode_into(
        &mut out,
        RansThreeLaneDecodeSpec {
            count: 12,
            log: 3,
            stride: 1,
            table: &TABLE,
            readers: &mut readers,
            payload: &payload,
        },
    )
    .unwrap();

    assert_eq!(out, [7, 4, 0, 0, 0, 0, 7, 10, 11, 9, 7, 1]);
    assert_eq!(
        readers,
        [
            RansThreeLaneReader {
                ptr: 0,
                acc: 0xc7a188080df04000,
                bitpos: 52,
            },
            RansThreeLaneReader {
                ptr: 826,
                acc: 0x7c33aca664d60800,
                bitpos: 45,
            },
            RansThreeLaneReader {
                ptr: 813,
                acc: 0x783359928eb0d000,
                bitpos: 51,
            },
        ]
    );
}

/// Segment descriptor builder feeding mode-1 tail dispatch (`count < 12`).
///
/// Provenance: `capture_segment_dispatch.py`, Animal_Dragonfly table-build
/// 15 + dispatch 6 (`mode=1,log=1,count=2,stride=1`). This exercises the
/// tail path at `0x110f1f8..0x110f380`, with the table built from the
/// reverse stream instead of hardcoded.
#[test]
fn rans_segment_dispatch_mode1_three_lane_tail() {
    let descriptor_payload = hex_bytes("5b22b1399b96d244781d");
    let descriptor = rans_build_segment_descriptor(
        &descriptor_payload,
        RansFreqReader {
            ptr: 2,
            acc: 0x0c64faf64078a80c,
            bitpos: 57,
        },
    )
    .unwrap();
    assert_eq!(descriptor.mode, 1);
    assert_eq!(descriptor.log, 1);
    assert_eq!(descriptor.step, [0x0001_0000, 0x0001_0004]);
    assert_eq!(
        descriptor.reader,
        RansFreqReader {
            ptr: 0,
            acc: 0xfaf64078a80ebc20,
            bitpos: 57,
        }
    );
    let payload = sparse_payload(
        463,
        &[
            (0, "5b22b1399b96d244"),
            (431, "63ffffe917bfb0c8"),
            (455, "20f89bc3f9ff5f37"),
        ],
    );
    let mut states = [0u64; 4];
    let mut readers = [
        RansThreeLaneReader {
            ptr: 0,
            acc: 0xfaf64078a80ebc20,
            bitpos: 57,
        },
        RansThreeLaneReader {
            ptr: 431,
            acc: 0x226000043fffcc00,
            bitpos: 51,
        },
        RansThreeLaneReader {
            ptr: 455,
            acc: 0x910a0801fffb9a00,
            bitpos: 49,
        },
    ];
    let mut out = [0u16; 2];
    let used = rans_segment_dispatch_into(
        &mut out,
        RansSegmentDispatchSpec {
            mode: descriptor.mode,
            log: descriptor.log,
            value: descriptor.value,
            count: 2,
            stride: 1,
            states: &mut states,
            step: &descriptor.step,
            sym: &[],
            stream: &[],
            payload: &payload,
            three_lane_readers: Some(&mut readers),
        },
    )
    .unwrap();

    assert_eq!(used, 0);
    assert_eq!(out, [4, 0]);
    assert_eq!(
        readers,
        [
            RansThreeLaneReader {
                ptr: 0,
                acc: 0xf5ec80f1501d7844,
                bitpos: 56,
            },
            RansThreeLaneReader {
                ptr: 432,
                acc: 0x44c000087fff98fe,
                bitpos: 58,
            },
            RansThreeLaneReader {
                ptr: 454,
                acc: 0x910a0801fffb9baf,
                bitpos: 57,
            },
        ]
    );

    let mut short_readers = readers;
    let mut short_out = [0u16; 2];
    assert_eq!(
        rans_three_lane_decode_into(
            &mut short_out,
            RansThreeLaneDecodeSpec {
                count: 2,
                log: 1,
                stride: 1,
                table: &descriptor.step,
                readers: &mut short_readers,
                payload: &[0; 7],
            },
        ),
        Err(RansThreeLaneDecodeError::PayloadTooSmall)
    );
}

/// Segment loop (`0x110dc30`) over the complete observed population.
///
/// Provenance: `capture_segment_loop.py`, Animal_Bass loop 0. The
/// enumerate-all population is exactly one loop call across Bear/Bass/
/// Dragonfly: `byte_count=1932,lanes=3,segment_log=6`, dispatching one
/// mode-0 segment followed by three mode-2 RLE segments. This covers the
/// descriptor-to-dispatch pipeline and the subtle run carry across lanes
/// (`0x110dd1c..0x110dd24`): the zero-valued RLE descriptor first finishes
/// the last two symbols of lane 0, then carries into lanes 1 and 2.
#[test]
fn rans_segment_loop_bass_mode0_then_rle_lanes() {
    const BEFORE: &[(usize, u16)] = &[
        (62, 65535),
        (63, 65535),
        (64, 1),
        (69, 65535),
        (70, 1),
        (77, 65535),
        (78, 65535),
        (79, 1),
        (93, 65535),
        (94, 1),
        (116, 65535),
    ];
    const EXPECTED: &[(usize, u16)] = &[
        (6, 2),
        (90, 1022),
        (96, 1022),
        (102, 1022),
        (126, 1022),
        (129, 1022),
        (144, 2),
        (147, 2),
        (150, 2),
        (159, 2),
        (168, 2),
        (174, 2),
        (180, 2),
        (186, 2),
        (606, 2),
        (840, 1022),
        (876, 1022),
        (888, 2),
        (900, 2),
        (906, 2),
    ];
    let payload = sparse_payload(
        6225,
        &[
            (2018, "87e2163f7eff1a365ef3a7f6a5e841a3"),
            (6210, "1946484f6815d7c8e2d3909ede0000"),
        ],
    );
    let mut out = vec![0u16; 968];
    for &(idx, value) in BEFORE {
        out[idx] = value;
    }
    let mut context = RansSegmentLoopContext {
        reader: RansFreqReader {
            ptr: 6217,
            acc: 0x116801fe180f4a00,
            bitpos: 55,
        },
        mode1_extra_readers: [
            RansThreeLaneReader {
                ptr: 6923,
                acc: 0x78c45f1a02887500,
                bitpos: 62,
            },
            RansThreeLaneReader {
                ptr: 6928,
                acc: 0xcb087f456207a1f8,
                bitpos: 58,
            },
        ],
        stream_pos: 2018,
        state: RansStateBuffer {
            states: [0x68007d0ef80f, 0x674a999ea5a, 0x647f7484a3f513e, 0xd7a40fe0],
            flag: 0xf,
        },
    };

    let dispatches = rans_segment_loop_into(
        &mut out,
        &mut context,
        RansSegmentLoopSpec {
            byte_count: 1932,
            lanes: 3,
            segment_log: 6,
            payload: &payload,
        },
    )
    .unwrap();

    let mut expected = vec![0u16; 968];
    for &(idx, value) in EXPECTED {
        expected[idx] = value;
    }
    assert_eq!(dispatches, 4);
    assert_eq!(out, expected);
    assert_eq!(
        context,
        RansSegmentLoopContext {
            reader: RansFreqReader {
                ptr: 6208,
                acc: 0xe9e90d3e2c8d7100,
                bitpos: 52,
            },
            mode1_extra_readers: [
                RansThreeLaneReader {
                    ptr: 6923,
                    acc: 0x78c45f1a02887500,
                    bitpos: 62,
                },
                RansThreeLaneReader {
                    ptr: 6928,
                    acc: 0xcb087f456207a1f8,
                    bitpos: 58,
                },
            ],
            stream_pos: 2034,
            state: RansStateBuffer {
                states: [0xff653ce9, 0x2b0d9366535bcb9, 0x839f26fdba, 0xa88726296addc,],
                flag: 0xf,
            },
        }
    );
    assert_eq!(out.iter().map(|&v| v as u32).sum::<u32>(), 7180);
}

/// U16 segment loop (`0x110dc30`) mode-1 three-lane dispatches.
///
/// Provenance: `find_phase1_segment_loop_mode1.py` found
/// Animal_Bull.Bull.bfres.mc; `verify_segment_loop.py` replays that targeted
/// capture 1/1 with three mode-1 dispatches. The zero-filled output is enough
/// because the observed schedule writes every logical slot and only leaves the
/// padded slot untouched.
#[test]
fn rans_segment_loop_bull_mode1_three_lane_dispatches() {
    let payload = sparse_payload(
        31614,
        &[
            (
                24763,
                concat!(
                    "48aca2a13a880f03b9d60390b6e3811fe711b4a057ff5b74f5130e5fdfb0eac2fd73a0944c0ca1703dba7ac07ab5440d",
                    "33105ad04f2e15b1afbb03bac2dff662daca899ad5ba57b3c0c509791aeffc97d032522e5d3e98c379b7c7e1c182dbb6",
                    "8047237afa4b0d3ec0a3395e8afd1c249c515931923931622917293afa6bbc2f278f43771f4fc19a12971e07061809a0",
                    "1510b413e5d379e5a5094ddbdade9ffab108d9d2678bf8b1c2a8ec8d1d411708118547cf4f32b0484e46882968ca1205",
                    "dd5337b33c888b6bcecebef498c235a672a24c2f05e6ee3905d2dc2ab70b3e356680b36940ab55c58016eac38463188f",
                    "205f1940be779714011a4c0400c15aa74a8e1b00180240b6d1da553d9106b8b00000beb4b52595d6bfb88110ffa8cdc4",
                    "28cb55965d5252ffbfa6a52db55fea6ecf114b27ebb4e47491d192bc49f1a79bc8e2d6132242e2446b4445e2b412dd8c",
                    "ca27daaf692a4b3ca9f49d9ea84cd2ebdd7d5737557e8e2c2f6e6b3a79d5924a6b9dea3df127c492dcf41259b5aaf189",
                    "2b19cd3fe5dbbbed9abfaaaf4444974ae4f3a5a3d292ffae4a5a967295933f917a89d17242eae9ad499abaaab67e9296",
                    "bf4a1bcd6c958013f9033c000400802c8fcbd9c99494091561ae6ef0710178a7b8a5f1e7d1450505b27100c00f50a75e",
                    "1d6fabd681645486fefcc499de90a76408208b253fcdf230c429c70ac1ecfe3846e56dcf0549c27143b22c61d2fb3831",
                    "27a48d51b928207d4aeb74eafabe31445574c6a8d0e8e42169e3be93521d7a47b6e9a2bb2722448f883619b37dfbaa99",
                    "59325aec133cc384650ed5565475228977138a13b44c88be4e722d0b2c1622dda80b42895d3d8a6892483f8280a332d4",
                    "016e0800018666de",
                ),
            ),
            (
                28818,
                concat!(
                    "7ca2b9e18af2fc5b3262698acf180414bf478a29cd9b6bc23cfca8a751d0d77cc2fd371469acfdf1ea0178c1379b8000",
                    "07401f2dab924f8f7c2d0d1fe123496f12691d7925f359bc66f6e12390984bfe93603e61adb14fd0a00fd7b9b0db2c29",
                    "8fba44980ca000ec0c804b37805a80f077b45ab3ad59b91ec871bfe1a60cb0ec5a9779800002a261e40781bc2ef7543b",
                    "811ed2f88168d4055600be95e7009e0178f90552ac3859f592708b3b2be5f2ef3dfa4a8c15944754038cc1bf8fb04db3",
                    "bf71f89c5d920e70761f624fd6e9479ec9423c03907e23eea00d15de4642115932fff02e1b205be41020cfb723a58533",
                    "d3447b562b4e5cfff37be58649b13a99b4d6787e60c6eb3e27369ad9b1187b0964aa17a58eaa75c5a23609bf078d2224",
                    "2627e59ebb549a9346fd1cb767d82c7c0dbb79bf2aba1728a1c4580dd6e3c925bd59d705ac6c05cd599b0610f0afdec8",
                    "46d781b72eb8169b122e382089ff83c5781d1c1407fe0075007f",
                ),
            ),
            (
                29729,
                concat!(
                    "e8039803c07f49a0a180ff7ff01c90467f7cf60deecc8f41be38c21b93b76127cabe04a9f7bb48de7b553b2b21c687f9",
                    "1abd3b69916a5e1e98431da29ffd32e3b183d10beec83827699f1f07c9f9788c98c8731e2a31c2e478932038c724daeb",
                    "6fc273e8eb1dd2cf57fddc3a9114d2a8e4befd2b7f7bdf4987d812978def7abb4e2e8c444c9d868b20a96060bd8f3fbc",
                    "f589050d5a31a9ab976dc253957d49a0810cddb468e4ca41a84a131f034d8dbc30a29d212ab07a23a0748a47a81275ad",
                    "ac310a98831e6ee9f68aeab2d6c2c594db8bf21ca985ac0efcb4d5d8cd6b6702662207285107bcf8866e19301ea083d9",
                    "e1a78403b32c07aeceec467094a7339af47d48a740c21e00dcdab30e359476b16a940070b806ccb00b4928d7af0b1721",
                    "f9f1c73e0c04b676b204ba8898783a42162e633323237517691d05064bc1bc09ea704d6c5d0d9e8ebc4e65d5be8eea5d",
                    "897da1398f7aefb089f27fbd0ef0e7121342f14acd4d4469df66b7df4deb5e",
                ),
            ),
        ],
    );
    let mut out = vec![0u16; 999];
    let mut context = RansSegmentLoopContext {
        reader: RansFreqReader {
            ptr: 25379,
            acc: 0xcd0c_0200_10dc_03a8,
            bitpos: 55,
        },
        mode1_extra_readers: [
            RansThreeLaneReader {
                ptr: 28826,
                acc: 0xa2b9_e18a_f2fc_5b00,
                bitpos: 56,
            },
            RansThreeLaneReader {
                ptr: 30080,
                acc: 0xeeb4_ddfb_766d_f694,
                bitpos: 60,
            },
        ],
        stream_pos: 3717,
        state: RansStateBuffer {
            states: [
                0x2d810dc4c,
                0xd9e836ca330e3,
                0x15e7563e602c,
                0x4f35d818d38a1,
            ],
            flag: 0xf,
        },
    };

    let dispatches = rans_segment_loop_into(
        &mut out,
        &mut context,
        RansSegmentLoopSpec {
            byte_count: 1996,
            lanes: 2,
            segment_log: 6,
            payload: &payload,
        },
    )
    .unwrap();

    let expected = hex_u16s(concat!(
        "02014702a701e20213036d03a502c000f301e503df018a00ec004101a002a0028602c8026401ce021b01680255033102",
        "000001022703aa02ab000e03b4032e0256030a035d005002ba004903a1034a02400340037a007e028001af027101a002",
        "f0018d03fc004302e501a0002a01a0028501f1007300370198006400fc03c5000600c2025e0374008b033001f8027702",
        "7202da002103350210027c00e7003802db0032020000020200000202d901d303e201ce03c7012f03ae01f102fc004602",
        "990019024d0322027d01170189013301af01f200ca015b00e201a200f0012a00fd01d7030b014c02f00172006d01f702",
        "7401ce009d004200b7001a010500ca0001004e026b0338004b031a017c02bd00b302ba02090257001c0337020000fe01",
        "0000fc013a018f0186014701da015303ca01220300000502000009027301a702ad003b020000060200003f02fa010400",
        "d3017c03ba014d03e601e403fd0360022302f1033702a8002d022a00bf024f0004002901ad024f002c002100e3031600",
        "56011800fb013200d801a90355027d03f3032d03f001a7035a01b803fc01100019035c0320027c030c029d03d7026703",
        "bd021800a10277036d01b603330322032f01f003d00024032c016c0341016100c401a7002302f6034002ee032c021a00",
        "240281038e02d6028c01ef020000c70200006202b5002c0200006d023f03490200007302f502960200002202cc029602",
        "d7026a02000002029502c302800233030902cc035c02e9021302b003110240006302d702f8024a02000001020f026500",
        "5a021a010c02a10364022e01f102b1012a03cf010000ff010000fb0197007602a600de010000fc010000ff010000f501",
        "d20078010000f701e901be030000fb0100009003f901cd030000fe0100000302fe01f3030000ff0100000202ff01f103",
        "00000102a400ef00eb0094025a014a0000002f0300003400510093000000990000002301dc000001c700fb03aa015600",
        "d9017003ee010200cc013200bf012500ce010f00d3013a00f601a603eb015200f801bb03af018f039401c902c1011203",
        "d3005802000011020000170200000a024403da0115020200510212010102070031026d008202ef024102de036702dc02",
        "2d02a50346022a032e02db038802f20263036503fe00eb020000f8000000e501480187010000f6015001770134019701",
        "0000fd01ac010e0192013b01f2017300f1015d00e9015800470312031801c10213016702560113034101150321019503",
        "c5014203e6016803a301de037e015b00d5019300ff01f403e5018400e901ff036b015201a40122010902ae03ec018303",
        "2c014c03b902cb03d7018503eb016000dc018b00d3017600b401c000b2006a0100007b012002dc033602b30002022500",
        "02001f01fe021301f70259000902a6030d010d010703be03d301a700c102bf020802b103d400cc03d5009702e4016d03",
        "fa004e00ee01a003e401e803fd010c00dc01d203d501d6030200350297010400670211005201b903fe038d03a702bf03",
        "43015c03bf025203d1005903e300a7001e0121000c0351031202cc03c702b700c7022000af018f008901b200f601fc03",
        "a5008b0150017501f503c5010000de010000f0012300f3017403e1010000ed01000065013f00ec0101005901f503bb00",
        "0000cf0032003701e003da001b00b6010000290183010f013b00bb007b0133008b011f033601ab036f0003018b000901",
        "b1014e002901c4002001f20070004401c6032101d30001016b0343016003410162031200ad01fc008701ba02ff009f01",
        "cf00310200000602000015023e0196020000fb016c0161010000d501e7019a0300006501000047020000010200000e02",
        "c5011c0300000502c103be001a00cc008403eb003700dc00a0029602a803120200000e02a801160323019f0003014a02",
        "1300da017303d0018e016203cd0148035601b402000044020e01ea020000a1023a011b03a60099020000860200000902",
        "1401b202ec003e02b801ff021b015b02e0016900b601d403dd01a903d6013e007a0100038d017e03f601b7037f01ae02",
        "3a0216033202340346021b0392028103ca03c4036e033d030000fb03e503dc03b800c403aa038a0000002c00f900ed03",
        "8c00d700a300c9016601e900dc01c903ca013d0098011801e4017603fb01c703d50140031501ac02ba014903eb014900",
        "ec017600e701d1039a00e7024003d10294012f0134038602c100d1019c00e6014300fa010000fc01d70329022e002402",
        "cf0353022e0058020701b6010000ff010000f001c101f4006e015e01620153010000ca01a0011a012a018d010000ed01",
        "8f013a012e018c010000fe013300c3010000c0012700e4010000d8017c01fe0065003801ab01bd0376019303cc00c102",
        "62006003000038010000a202000074036f019f02b801e2039a0123007e012e00b90060000d01e700e30363010200df02",
        "d80165004601f602c801ca03a6002b029c01d202220165028201d9003e012d01b201640042025203a8004903fb01c403",
        "6301c8030000ff01c501ec009d0295006e0217003502770315012d01c8019f03ee018303dc01b9009e01f1028d002302",
        "8f02a402fc01dc03ad00b900e60010009700e301000002027d026f0039027f038b02a303c0006e03f5019c03d601d003",
        "31035e022c028303500135004e01f202630195002d02e803d102bd03cc0291014c03c8003802b6036c0239010000ff01",
        "94013901c20178034802f00381011900ef025a000000c901b1024c030000",
    ));
    assert_eq!(dispatches, 3);
    assert_eq!(out, expected);
    assert_eq!(
        context,
        RansSegmentLoopContext {
            reader: RansFreqReader {
                ptr: 24767,
                acc: 0x7e07_8eda_4000_0000,
                bitpos: 30,
            },
            mode1_extra_readers: [
                RansThreeLaneReader {
                    ptr: 29169,
                    acc: 0xc704_113f_f078_0000,
                    bitpos: 43,
                },
                RansThreeLaneReader {
                    ptr: 29733,
                    acc: 0xfe8d_2039_e0fc_0000,
                    bitpos: 39,
                },
            ],
            stream_pos: 3717,
            state: RansStateBuffer {
                states: [
                    0x2d810dc4c,
                    0xd9e836ca330e3,
                    0x15e7563e602c,
                    0x4f35d818d38a1,
                ],
                flag: 0xf,
            },
        }
    );
    assert_eq!(out.iter().map(|&v| v as u32).sum::<u32>(), 460720);
}

/// Byte segment loop (`0x110dae0`) mode-2 RLE descriptor.
///
/// Provenance: `capture_phase1_byte_segment_mode2.py`,
/// Animal_Boar.Boar.bfres.mc reaches byte-loop mode 2 in 6 dispatches across
/// 4 loop calls; `verify_byte_segment_loop.py` replays the Boar capture 6/6.
/// This compact golden uses the observed mode-2 value header from
/// `capture_segment_dispatch.py` and verifies the loop dispatches it through
/// byte RLE without advancing the forward stream or rANS state.
#[test]
fn rans_segment_loop_bytes_mode2_rle_fills_lane() {
    let payload = sparse_payload(10, &[(2, "f1a106940000623a")]);
    let mut out = vec![0xee; 5];
    let mut context = RansSegmentLoopContext {
        reader: RansFreqReader {
            ptr: 2,
            acc: 0x03fbfd0221c04704,
            bitpos: 59,
        },
        mode1_extra_readers: [zero_three_lane_reader(); 2],
        stream_pos: 123,
        state: RansStateBuffer::warm([1, 2, 3, 4]),
    };

    let dispatches = rans_segment_loop_bytes_into(
        &mut out,
        &mut context,
        RansByteSegmentLoopSpec {
            byte_count: 5,
            lanes: 1,
            segment_log: 1,
            payload: &payload,
        },
    )
    .unwrap();

    assert_eq!(dispatches, 1);
    assert_eq!(out, vec![0x7f; 5]);
    assert_eq!(
        context,
        RansSegmentLoopContext {
            reader: RansFreqReader {
                ptr: 0,
                acc: 0xfd02_21c0_4707_4c40,
                bitpos: 59,
            },
            mode1_extra_readers: [zero_three_lane_reader(); 2],
            stream_pos: 123,
            state: RansStateBuffer::warm([1, 2, 3, 4]),
        }
    );
}

#[test]
fn rans_segment_loop_rejects_bad_bounds() {
    let mut context = RansSegmentLoopContext {
        reader: RansFreqReader {
            ptr: 0,
            acc: 0,
            bitpos: 0,
        },
        mode1_extra_readers: [
            RansThreeLaneReader {
                ptr: 0,
                acc: 0,
                bitpos: 0,
            },
            RansThreeLaneReader {
                ptr: 0,
                acc: 0,
                bitpos: 0,
            },
        ],
        stream_pos: 0,
        state: RansStateBuffer::warm([0; 4]),
    };
    let mut out = [0u16; 1];
    assert_eq!(
        rans_segment_loop_into(
            &mut out,
            &mut context,
            RansSegmentLoopSpec {
                byte_count: 3,
                lanes: 1,
                segment_log: 0,
                payload: &[],
            },
        ),
        Err(RansSegmentLoopError::OddByteCount)
    );
    assert_eq!(
        rans_segment_loop_into(
            &mut out,
            &mut context,
            RansSegmentLoopSpec {
                byte_count: 2,
                lanes: 0,
                segment_log: 0,
                payload: &[],
            },
        ),
        Err(RansSegmentLoopError::ZeroLaneCount)
    );
    assert_eq!(
        rans_segment_loop_into(
            &mut [],
            &mut context,
            RansSegmentLoopSpec {
                byte_count: 2,
                lanes: 1,
                segment_log: 0,
                payload: &[],
            },
        ),
        Err(RansSegmentLoopError::OutputTooSmall)
    );
    assert_eq!(
        rans_segment_loop_into(
            &mut out,
            &mut context,
            RansSegmentLoopSpec {
                byte_count: 2,
                lanes: 1,
                segment_log: 0,
                payload: &[],
            },
        ),
        Err(RansSegmentLoopError::Descriptor(
            RansSegmentDescriptorBuildError::Header(RansSegmentHeaderError::PayloadTooSmall)
        ))
    );
}

fn zero_three_lane_reader() -> RansThreeLaneReader {
    RansThreeLaneReader {
        ptr: 0,
        acc: 0,
        bitpos: 0,
    }
}

fn byte_group_state(reader: RansThreeLaneReader, stream_pos: usize) -> ByteGroupReadState {
    ByteGroupReadState {
        reader,
        mode1_extra_readers: [zero_three_lane_reader(); 2],
        stream_pos,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    }
}

/// Byte-group reader (`0x110d7f0`) selector-0 byte segment loop.
///
/// Provenance: refreshed `capture_byte_group_reader.py`, Animal_Bass call 0:
/// selector 0, `w2=0,w3=1,w4=236,w5=0`. This routes through the byte
/// segment loop `0x110dae0` and byte mode-1 dispatch `0x110eb50`.
#[test]
fn byte_group_reader_bass_selector0_byte_segment_loop() {
    let payload = sparse_payload(
        7167,
        &[
            (6649, "b3e09b618b087861d70e0031f5671407c03c1f59ff57e0"),
            (6692, "40f23f3a3f97be760006aa0005b206605938e456b6e94ae554"),
            (7142, "4a19dff61d148c57320adc6b30030078009cfc64ffa8029480"),
        ],
    );
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 6664,
            acc: 520920709971982558,
            bitpos: 60,
        },
        mode1_extra_readers: [
            RansThreeLaneReader {
                ptr: 6692,
                acc: 0,
                bitpos: 0,
            },
            RansThreeLaneReader {
                ptr: 7159,
                acc: 0,
                bitpos: 0,
            },
        ],
        stream_pos: 394,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    };

    let read = byte_group_read(
        &mut state,
        ByteGroupReadSpec {
            payload: &payload,
            element_shift: 0,
            group_stride: 1,
            count: 236,
        },
    )
    .unwrap();

    assert_eq!(read.selector, 0);
    assert_eq!(
            read.bytes,
            hex_bytes(
                "000100010001010101010101010101010101010101020300010001000100010001000101000001010000010000010000\
                 010000010000010000000001000100010001010100010001000101010001000101000001000001010001010101010101\
                 010101010101010100010101000100000101010001000100010100010001010000010100000000010001010101010101\
                 010101010101000100000100010001010001010101000001010000010100000101000101000000000000000000000000\
                 0000000000000000000000000000000000000000000001010101010101010001000100000000000100010000",
            )
        );
    assert_eq!(
        state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 6649,
                acc: 5079870232897353232,
                bitpos: 52,
            },
            mode1_extra_readers: [
                RansThreeLaneReader {
                    ptr: 6709,
                    acc: 11529315240777192320,
                    bitpos: 52,
                },
                RansThreeLaneReader {
                    ptr: 7142,
                    acc: 1795329482843296,
                    bitpos: 55,
                },
            ],
            stream_pos: 394,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        }
    );
}

/// Byte-group reader (`0x110d7f0`) selector-1 byte dispatch.
///
/// Provenance: refreshed `capture_byte_group_reader.py`, Animal_Bass call 24:
/// selector 1, `w2=0,w3=1,w4=35,w5=0`. This routes through one
/// `0x110de80` descriptor and byte mode-0 dispatch `0x110dd80`, advancing
/// the forward stream cursor by four bytes.
#[test]
fn byte_group_reader_bass_selector1_byte_dispatch() {
    let payload = sparse_payload(
        6512,
        &[
            (1918, "ea6bed014cb2007ea542a0d3f92b2868"),
            (
                6480,
                "0010c0400002c068de7b004aa9472f322b83a1a8a6e821829438040092e64e7d",
            ),
        ],
    );
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 6490,
            acc: 4917109273557729280,
            bitpos: 43,
        },
        mode1_extra_readers: [
            RansThreeLaneReader {
                ptr: 6791,
                acc: 5341445456360767488,
                bitpos: 47,
            },
            RansThreeLaneReader {
                ptr: 7062,
                acc: 10022179518769823744,
                bitpos: 48,
            },
        ],
        stream_pos: 1918,
        segment_state: RansStateBuffer {
            states: [
                106104675681684,
                92019277541,
                847353644991333,
                4038367482700755552,
            ],
            flag: 15,
        },
        selector2_history: Vec::new(),
    };

    let read = byte_group_read(
        &mut state,
        ByteGroupReadSpec {
            payload: &payload,
            element_shift: 0,
            group_stride: 1,
            count: 35,
        },
    )
    .unwrap();

    assert_eq!(read.selector, 1);
    assert_eq!(
        read.bytes,
        hex_bytes("0000000000000000000000010000000001000000000300010000000101000000000000")
    );
    assert_eq!(
        state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 6485,
                acc: 5855530908632649536,
                bitpos: 55,
            },
            mode1_extra_readers: [
                RansThreeLaneReader {
                    ptr: 6791,
                    acc: 5341445456360767488,
                    bitpos: 47,
                },
                RansThreeLaneReader {
                    ptr: 7062,
                    acc: 10022179518769823744,
                    bitpos: 48,
                },
            ],
            stream_pos: 1922,
            segment_state: RansStateBuffer {
                states: [
                    221301007954,
                    4945825627247054004,
                    63623282601795,
                    1871725651352470,
                ],
                flag: 15,
            },
            selector2_history: Vec::new(),
        }
    );
}

/// Byte-group reader (`0x110d7f0`) selector-1 u16 dispatch.
///
/// Provenance: refreshed `capture_byte_group_reader.py`, Animal_Bass call 6:
/// selector 1, `w2=1,w3=1,w4=30,w5=0`. This routes through one
/// `0x110de80` descriptor and u16 mode-0 dispatch `0x110de00`, then returns
/// little-endian bytes.
#[test]
fn byte_group_reader_bass_selector1_u16_dispatch() {
    let payload = sparse_payload(
        6568,
        &[
            (
                550,
                "2d0b7de52ebe2a5d4b452e4d10f75029bab99b570751286acd531c0000f47e66",
            ),
            (
                6536,
                "3fe02162ce41bdae0c90a23efb03dcf4038e46eb56f137c3843c42ff7fedcf12",
            ),
        ],
    );
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 6546,
            acc: 5490872100723124416,
            bitpos: 52,
        },
        mode1_extra_readers: [
            RansThreeLaneReader {
                ptr: 6785,
                acc: 14801326531105989520,
                bitpos: 59,
            },
            RansThreeLaneReader {
                ptr: 7069,
                acc: 11236589065793971168,
                bitpos: 53,
            },
        ],
        stream_pos: 550,
        segment_state: RansStateBuffer {
            states: [49227725862390, 14614913526, 1246709343325, 657827286397],
            flag: 15,
        },
        selector2_history: Vec::new(),
    };

    let read = byte_group_read(
        &mut state,
        ByteGroupReadSpec {
            payload: &payload,
            element_shift: 1,
            group_stride: 1,
            count: 30,
        },
    )
    .unwrap();

    assert_eq!(read.selector, 1);
    assert_eq!(
            read.bytes,
            hex_bytes(
                "0a000a000a000a000a000a000a000a000a000a000a000b000b000700050007000700060007000a000b00080008000800050006000600060007000900"
            )
        );
    assert_eq!(
        state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 6541,
                acc: 15954232304648028280,
                bitpos: 59,
            },
            mode1_extra_readers: [
                RansThreeLaneReader {
                    ptr: 6785,
                    acc: 14801326531105989520,
                    bitpos: 59,
                },
                RansThreeLaneReader {
                    ptr: 7069,
                    acc: 11236589065793971168,
                    bitpos: 53,
                },
            ],
            stream_pos: 566,
            segment_state: RansStateBuffer {
                states: [
                    100818382515160028,
                    29931342885566,
                    20426085882968341,
                    10777842260191934,
                ],
                flag: 15,
            },
            selector2_history: Vec::new(),
        }
    );
}

/// Byte-group reader (`0x110d7f0`) selector-2 zstd window.
///
/// Provenance: refreshed `capture_byte_group_reader.py`, Animal_Dragonfly
/// call 2: selector 2, `w2=0,w3=1,w4=560,w5=0`, zstd flag 0, empty
/// history, and forward stream cursor `157 -> 474`. This covers the
/// observed single-window `0x1110a60` zstd branch and appends the decoded
/// bytes to the selector-2 history buffer rooted at caller `[x0+8]`.
#[test]
fn byte_group_reader_dragonfly_selector2_zstd_window() {
    let payload = sparse_payload(
            4488,
            &[
                (
                    157,
                    "823b461c460a1038f806592d677b431a430042003f009d6905bd43a712d2f1406d50201390440703604bf92c775b47906a1b1435880dca8ec35a9e6298d51835bdb26d124da3adce72141ad464a6c0cb542a213397242d4946102e74d8488d793d4dddadadfe37588dc588960f255dcc0528e3dae68eb2cfff139b1d8cd3464db22a65f7d860c9ea08e9929d73af13bedc94561384e4922958e90289cfff07406de6e053184ad3a2b8f9785ec7f9fbb8d1ec5962961750ab7a9b21265e1ff509188c6aa65d67af974fbda6a91eb5bbb775c13fff022184fb160c89d92a92aa745097e0d77eccdeaaf822225114bcc6bd4060471e7b50effae7a216c5c2fc338d930365b16543f1f8f3dc2266a591e49ed01afd49cfc2c7a0be3144b4020c00962e820c256acef5729bced04a0631a4f05002a15c952e8eb20033317003",
                ),
                (
                    4464,
                    "f866efff43fe001000d08c502d459eff00a075fd4ef5fc2a",
                ),
            ],
        );
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 4471,
            acc: 11383669668276287488,
            bitpos: 47,
        },
        mode1_extra_readers: [
            RansThreeLaneReader {
                ptr: 4524,
                acc: 4679714676352931668,
                bitpos: 58,
            },
            RansThreeLaneReader {
                ptr: 4673,
                acc: 11559655046995041492,
                bitpos: 58,
            },
        ],
        stream_pos: 157,
        segment_state: RansStateBuffer {
            states: [
                290362338331826,
                324891473402,
                3329202948120618,
                1082470406632,
            ],
            flag: 15,
        },
        selector2_history: Vec::new(),
    };

    let read = byte_group_read(
        &mut state,
        ByteGroupReadSpec {
            payload: &payload,
            element_shift: 0,
            group_stride: 1,
            count: 560,
        },
    )
    .unwrap();
    let expected = hex_bytes(
            "000102020202020202020202020205030c01040202020d1002101204080d02020206030e0b10070502141113000c0301\
             02041302111406030606030808060507021312070004050a040f0010050a0413100b0115041314080407080311120900\
             0a140f0202020202090c0209030e0202020202020202020202130b000815020d1002140e04140a110c02141502110a0c\
             020508020d0d13110c0c04021617021416151515080215020d0710080212001216150803100308020603061407000a14\
             02040f181818180714020b181513041119020b0a151211030c0b1602130f1211001008001618181014031414140c0316\
             110808130c0012070b111717171116041717171705181802181817110314041414140319181002031316101602161114\
             020202020202020202020202020202020202020202020202141514061310060e090c020202020205030c180319180318\
             020202020205030c02020202020503190601060601060601060601060601130414100410090408040601040704061904\
             19180418060104070408060104070406020202020202020202180003180218001806181a1a0313050e0d021817151214\
             0317091618181807080c110b190e001603171216170407030c18180b14130d16150c01130512080b041516071819181a\
             0e120412110509180118000c02181802181802180217041a1a0319151618031816100e1419180115080d0a1615060408\
             1a1a0f190f11181a101418120816140318180a05140b070c10030c1a1112080c",
        );

    assert_eq!(
        read,
        ByteGroupRead {
            selector: 2,
            bytes: expected.clone(),
        }
    );
    assert_eq!(
        state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 4469,
                acc: 17282381051372102738,
                bitpos: 60,
            },
            mode1_extra_readers: [
                RansThreeLaneReader {
                    ptr: 4524,
                    acc: 4679714676352931668,
                    bitpos: 58,
                },
                RansThreeLaneReader {
                    ptr: 4673,
                    acc: 11559655046995041492,
                    bitpos: 58,
                },
            ],
            stream_pos: 474,
            segment_state: RansStateBuffer {
                states: [
                    290362338331826,
                    324891473402,
                    3329202948120618,
                    1082470406632,
                ],
                flag: 15,
            },
            selector2_history: expected,
        }
    );
}

/// Selector-2 zstd windows need the caller's prior selector-2 history.
///
/// Provenance: `capture_vertex_writer_loop.py`, Animal_Bear current 1,
/// source 0, with the minimal 1243-byte suffix from the earlier selector-2
/// outputs in `byte_group_reader_capture.py`. This rules out starting the
/// writer loop with an empty selector-2 history.
#[test]
fn selector2_zstd_window_allows_existing_history() {
    let history = hex_bytes(
            "110202020f040e0202020202020301080f011202020210120202170208160d1002020205040401011c1c0202021b1b020202030402020306030615131b1b1b1b0202020202030602020202020306020202020202020202020201050a070a0d1001040b0408070a010411011402021818020202181802090c0205080209040b1004070a0502020306021a1a020219190508021a1a020202021d1d0308021a19020202021b1c06040202020202021919020202020207061d1d0202020202020219190202020201031b1b0218180202020201031b1b181702021b1b020504040202020202021c1c1c1c1c18161c1b161c1c1401071a030202090f191b1b010805181a041b021b021b02021c0a1c020202021c11161c1c1c0205080202000104020f061c0e1b1b1c12140202011c1c1c1c01021c061d1d1c02021c1c0202031b1b001b021b150b1c081c0a1c1c1b1b1b1b0202020202020306020202121402020d100215160104100d011616160214150202160214000b0f1b011c1c04010e181800180300041b1c180804040700070c04031b001b031b02021b1c1c1b1b1b1b1b011c1c1a021d1d011b1c1c0208031d1d04021c1c0406031c1c0508040104040104021b1b0603011d1d020401060104021b1b020803041c1c061b1b020803041c1c06021b1b1c1c00060104040108010111011c1c1a1a0713111a1a1c1c1a1a1a1a1a190e0b14020b18021a0418011116151d1e1c1c100d00031c1c1d1c1c0a1c0102001819020214140415141a19151c1c0b0e1a041b1c1c08190b14151b1a1c161c1c04050d021c081c1c1c1c131c021c1c1c021b1b021c021c1c031c07071212131c1c031c1a1c1916181c1c1517141c1c1c1d0608071d1c0f01140614151c1c171802181c1c01010c1c1c191c1c1313150d02080d1b011b14120401010301041c1b0103171c1c19191e1d1c1c021a1c14191c1c0618121b1b021c1c0b0712041b1b1a0106011c1c1c031c1c1a1a1a020202000202090c1b011b191a1b01051a161a19021a1a1b1a1a1b1c1b1d1d081d1d1c141601151a191a1c1c0610060a090c150015141a1918071a0201041a16191a13141a1c1c1b1b1c181c141a191b1c0c111c0c020f1c1c1a1a1c0204091c1b021c1c011b1d181d1d021c191c021c1d190312011b1b02181d1d191b1c1c041c1c1501161c1c0e0f1c1902021c04011c1c02171c1c15161a1c011b021c14050904051a041b1b1b1c1c071c1c131b031b1b1d1c1c1d1d1c0c0f1b151b1d1d1d1d1b1a1a1a1c1c1c1b1c1a1b15160702000918180101171b1b1b0c1c1b1d1e1c1d1b1c1c1b181c1c1c011c1a1a0a0e0f1a0b14141a190417171a1b181818181c1c1c191c1b1c181b1b1b041b1c1d1b1c1c1c181c09091c18190d1c1c101c110c1b1b1c1901191c1c1d1b1c1d1e1d1b1d1a021a09021b0d1d051d1a1b011a011905011a1a0c1a1a1917161a1a1614141a1a1a1a1819191b1d1c1c1a1a04150b031a18021c1c1c1c18181a19191c1c1c141506041c1c1c191c01141c191a1b19061c1c1b1819140215191a1a1a19040d181901011b1b1b1a17021c1c18141a071a0d000002020000050000050000050000030205010301001a08020801101a000301020201021a0100011a02011c000000010000031d101c0100000118000001181a0009000003000800060000030007000003000700090203000014000f1819000408011a030109101a1b191a",
        );
    let payload = sparse_payload(7746, &[(7452, "822452cc1e0b309afabb3920f09eb01647fad33c22febf83ee0cc7e0a160d0603343927e2dd51af761746bbe92d020003c013f70a588cd4c4dbde68ba2d82508a27079fa5dafe7e871188c20089dc6123e7db95f771b467373372fb52608022304a6314e5bb02bcfef5a6bd76afef629fdfa679e5961666684509224bbab940a67a8e08c8692340730083221621d00b3a603f006e1de3e3b2aabbfb612e8c0268674638903d9bdd4fc455678044c6a5c2a7b439adacb77c1d1e2ed198da87e65c4e5b3ed233481994224506b30ba044652abaf1a96a605ebf9579d8cc00e61e0acbed96c5b9050a5e0134cc1d4b1dc81f1d58e28f869dc197ec96e0ebf9b9f7ba651ae4b4a9135aebef4ab226a28b9390b8d1453bc4ef00eab7a35571cd33db98aa1962ca214")]);
    let expected = hex_bytes(
            "0f0f0f0e0e0e1b1b1b0b0b0b1519191c1c1c151c151c1c1c1b1b1b151515001515151915151515020808020808020802020902020d0d020d020d0e0d020d0e0102080102080102090102090e0f0f0e140e1414140e0e0e0e14140e0e0e020d0d020202020d0d0208020e0e0e0001150015190e0f0f0e0f0f0e0f0e0e140e0e130e0d0e0e0e0e0e0d0e0f0e0f0e15191c1c1c1c0506070506060506070606060506050e0f140e140e0e140e0e14140e140e0e14141414141516161516151516151516151617171617161717171617161617171616160d0e0f0102010001020102090001150102010001020102020102150102010102020102010102150102151a1b1a1a1b1b1a1b1b15191a15191515191c0a0b0b0a0b0b001519191919151919151919191a1a1519191a1a1a15191915191a0015191519150015191717171717171718171718180e0f0f090a0a090a0a090a09090a0a0a0a0a0a0a0a0a0a0a090a0a0a0a0a02090201020d0202020202020015000015001111110e11110e0e0e1111110202020d0e0e020d0e0e13130e13130e0e0e13131302080902090902090a02090902090902090a0809090909090208090809090808080e110e0f140f0e0f140f140f0f0f0f0e0f0e02030202030d0203030102010204050d0e0e02040d00011515161500011517181715161715161c161718181818161716161716151c1c0f14140e0f1402090d0f140f02090a11111115191a151915191a1b191a190015190102090015160405040507050405040104050a0c0a01090a0a0b0a010202020402020304020404020809010209",
        );

    assert_eq!(history.len(), 1243);
    assert_eq!(expected.len(), 591);
    assert!(decode_selector2_zstd_window(&payload, 7452, expected.len(), &[]).is_err());
    let (decoded, stream_pos) =
        decode_selector2_zstd_window(&payload, 7452, expected.len(), &history).unwrap();
    assert_eq!(stream_pos, 7746);
    assert_eq!(decoded, expected);
}

/// Byte-group reader (`0x110d7f0`) selector-3 direct-forward branch.
///
/// Provenance: `capture_byte_group_reader.py`, Animal_Bass call 16:
/// selector 3, `w2=0,w3=1,w4=3,w5=0`, forward stream bytes `1b001b`.
/// The payload is relocated to a compact fixture-free buffer while keeping
/// the observed combined selector window and resulting reader writeback.
#[test]
fn byte_group_reader_bass_selector3_direct_slice() {
    let payload = sparse_payload(11, &[(0, "0000000000000000"), (8, "1b001b")]);
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 0xff0e_8001_39a1_a067,
            bitpos: 59,
        },
        8,
    );

    let read = byte_group_read(
        &mut state,
        ByteGroupReadSpec {
            payload: &payload,
            element_shift: 0,
            group_stride: 1,
            count: 3,
        },
    )
    .unwrap();

    assert_eq!(
        read,
        ByteGroupRead {
            selector: 3,
            bytes: hex_bytes("1b001b"),
        }
    );
    assert_eq!(
        state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 0,
                acc: 0xfc3a_0004_e686_819c,
                bitpos: 57,
            },
            mode1_extra_readers: [zero_three_lane_reader(); 2],
            stream_pos: 11,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        }
    );
}

#[test]
fn byte_group_reader_rejects_unported_selectors_and_bad_bounds() {
    let payload = sparse_payload(8, &[(0, "0000000000000000")]);
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 2 << 62,
            bitpos: 59,
        },
        0,
    );
    assert_eq!(
        byte_group_read(
            &mut state,
            ByteGroupReadSpec {
                payload: &payload,
                element_shift: 2,
                group_stride: 1,
                count: 1,
            },
        ),
        Err(ByteGroupReadError::UnsupportedElementShift(2))
    );

    let payload = sparse_payload(8, &[(0, "0000000000000000")]);
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 0,
            bitpos: 59,
        },
        0,
    );
    assert_eq!(
        byte_group_read(
            &mut state,
            ByteGroupReadSpec {
                payload: &payload,
                element_shift: 2,
                group_stride: 1,
                count: 1,
            },
        ),
        Err(ByteGroupReadError::UnsupportedElementShift(2))
    );

    let payload = sparse_payload(8, &[(0, "0000000000000000")]);
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 1 << 62,
            bitpos: 59,
        },
        0,
    );
    assert_eq!(
        byte_group_read(
            &mut state,
            ByteGroupReadSpec {
                payload: &payload,
                element_shift: 2,
                group_stride: 1,
                count: 1,
            },
        ),
        Err(ByteGroupReadError::UnsupportedElementShift(2))
    );

    let payload = sparse_payload(8, &[(0, "0000000000000000")]);
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 1 << 62,
            bitpos: 59,
        },
        0,
    );
    assert_eq!(
        byte_group_read(
            &mut state,
            ByteGroupReadSpec {
                payload: &payload,
                element_shift: 0,
                group_stride: 1,
                count: 0x80000,
            },
        ),
        Err(ByteGroupReadError::UnobservedSelector1LargeWindow {
            group_symbols: 0x80000,
        })
    );

    let payload = sparse_payload(8, &[(0, "0000000000000000")]);
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 2 << 62,
            bitpos: 59,
        },
        0,
    );
    assert_eq!(
        byte_group_read(
            &mut state,
            ByteGroupReadSpec {
                payload: &payload,
                element_shift: 0,
                group_stride: 1,
                count: 0x20001,
            },
        ),
        Err(ByteGroupReadError::UnobservedSelector2MultiWindow {
            byte_count: 0x20001,
        })
    );

    let payload = sparse_payload(8, &[(0, "0000000000000000")]);
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 5 << 61,
            bitpos: 59,
        },
        0,
    );
    assert_eq!(
        byte_group_read(
            &mut state,
            ByteGroupReadSpec {
                payload: &payload,
                element_shift: 0,
                group_stride: 1,
                count: 1,
            },
        ),
        Err(ByteGroupReadError::UnobservedSelector2RawWindow)
    );

    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 3 << 62,
            bitpos: 59,
        },
        8,
    );
    assert_eq!(
        byte_group_read(
            &mut state,
            ByteGroupReadSpec {
                payload: &[0; 7],
                element_shift: 0,
                group_stride: 1,
                count: 1,
            },
        ),
        Err(ByteGroupReadError::PayloadTooSmall)
    );

    let payload = sparse_payload(10, &[(0, "0000000000000000"), (8, "1b00")]);
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 3 << 62,
            bitpos: 59,
        },
        8,
    );
    assert_eq!(
        byte_group_read(
            &mut state,
            ByteGroupReadSpec {
                payload: &payload,
                element_shift: 0,
                group_stride: 1,
                count: 3,
            },
        ),
        Err(ByteGroupReadError::StreamTooShort)
    );

    let payload = sparse_payload(8, &[(0, "0000000000000000")]);
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 3 << 62,
            bitpos: 59,
        },
        0,
    );
    assert_eq!(
        byte_group_read(
            &mut state,
            ByteGroupReadSpec {
                payload: &payload,
                element_shift: 0,
                group_stride: 2,
                count: usize::MAX,
            },
        ),
        Err(ByteGroupReadError::OutputSizeOverflow)
    );
}

/// Byte-group transform wrapper (`0x10fb2e0`) active mode-1 path.
///
/// Provenance: refreshed `capture_byte_group_transform.py`,
/// Animal_Dragonfly call 1. This compact call has four direct selector-3
/// `0x110d7f0` streams (`first=3,second=2,third=2,tail=3`) and then runs
/// `0x110d360` for three records, covering the wrapper's forward varints,
/// reverse count-bit consumption, direct tail bitstream slop, and table
/// entry class derivation from `0xa000802`.
#[test]
fn byte_group_transform_dragonfly_active_direct_streams() {
    let payload = sparse_payload(
        4480,
        &[
            (
                1149,
                "0303011a021c0104000600427e801103031c190400090006009028d6ff00",
            ),
            (4464, "f866efff43fe001000d0"),
        ],
    );
    let mut transform_state = ByteGroupTransformState {
        mode: 1,
        count_bits: 191,
        record_count: 0,
        second_count: 0,
        third_count: 0,
        tail_count: 0,
    };
    let mut byte_state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 4466,
            acc: 18419235586919153664,
            bitpos: 48,
        },
        mode1_extra_readers: [zero_three_lane_reader(); 2],
        stream_pos: 1149,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    };

    let result = byte_group_transform(
        &mut transform_state,
        &mut byte_state,
        ByteGroupTransformSpec {
            payload: &payload,
            table_entry: ByteGroupTransformTableEntry { raw: 0x0a00_0802 },
            limit: 523,
        },
    )
    .unwrap();

    assert_eq!(
        result,
        ByteGroupTransformResult {
            ret: 162,
            records: hex_width_records("010065010a0000009f0004005a0000000200000000000000"),
        }
    );
    assert_eq!(
        transform_state,
        ByteGroupTransformState {
            mode: 1,
            count_bits: 95,
            record_count: 3,
            second_count: 2,
            third_count: 2,
            tail_count: 3,
        }
    );
    assert_eq!(
        byte_state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 4464,
                acc: 4362398837027897376,
                bitpos: 55,
            },
            mode1_extra_readers: [zero_three_lane_reader(); 2],
            stream_pos: 1163,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        }
    );
}

/// Byte-group transform wrapper (`0x10fb2e0`) early even-count path.
///
/// Provenance: refreshed `capture_byte_group_transform.py`,
/// Animal_Dragonfly call 0. `count_bits=382` has low bit 0, so
/// `0x10fb31c..0x10fb328` returns one `[limit, 0]` record without touching
/// the byte-group reader.
#[test]
fn byte_group_transform_dragonfly_even_count_early_record() {
    let mut transform_state = ByteGroupTransformState {
        mode: 1,
        count_bits: 382,
        record_count: 0,
        second_count: 0,
        third_count: 0,
        tail_count: 0,
    };
    let initial_byte_state = byte_group_state(zero_three_lane_reader(), 0);
    let mut byte_state = initial_byte_state.clone();

    let result = byte_group_transform(
        &mut transform_state,
        &mut byte_state,
        ByteGroupTransformSpec {
            payload: &[],
            table_entry: ByteGroupTransformTableEntry { raw: 0x0a00_100b },
            limit: 523,
        },
    )
    .unwrap();

    assert_eq!(
        result,
        ByteGroupTransformResult {
            ret: 523,
            records: vec![[523, 0]],
        }
    );
    assert_eq!(
        transform_state,
        ByteGroupTransformState {
            mode: 1,
            count_bits: 191,
            record_count: 1,
            second_count: 0,
            third_count: 0,
            tail_count: 0,
        }
    );
    assert_eq!(byte_state, initial_byte_state);
}

/// Vertex attribute driver setup (`0x10fafe0`) plus observed wrapper steps
/// from the per-attribute loop (`0x10f924c..0x10f92b8`).
///
/// Provenance: `capture_vertex_driver_setup.py` and
/// `capture_byte_group_transform.py`, Animal_Dragonfly. The setup capture
/// enumerates all fixtures: Bear/Bass use `w3=8,count_bits=255`; Dragonfly
/// uses `w3=9,count_bits=382`; all set mode 1. `verify_vertex_driver_setup.py`
/// replays 3/3 setup calls, and `verify_vertex_attribute_driver.py` replays
/// 25/25 attribute-loop wrapper calls across Bear/Bass/Dragonfly.
#[test]
fn vertex_attribute_driver_dragonfly_setup_and_steps() {
    let payload = hex_bytes(concat!(
            "00000c010101845200840b0a001400392402f0fefcfcf0fefe20fe00fe1000fef010f010f020fc00f020f0200010f000",
            "10f0000008002b09669a4e3184b3b3b35c98457319ab2910021520032030020204203002020202100360020406100384",
            "30810a4407526548d4e399bb07cedced5dfa99be051fd846a635667359460f3d0a2d2407b874ac18455211e59fef27dd",
            "9c6083ec4d0e185b120f0b61b9823b461c460a1038f806592d677b431a430042003f009d6905bd43a712d2f1406d5020",
            "1390440703604bf92c775b47906a1b1435880dca8ec35a9e6298d51835bdb26d124da3adce72141ad464a6c0cb542a21",
            "3397242d4946102e74d8488d793d4dddadadfe37588dc588960f255dcc0528e3dae68eb2cfff139b1d8cd3464db22a65",
            "f7d860c9ea08e9929d73af13bedc94561384e4922958e90289cfff07406de6e053184ad3a2b8f9785ec7f9fbb8d1ec59",
            "62961750ab7a9b21265e1ff509188c6aa65d67af974fbda6a91eb5bbb775c13fff022184fb160c89d92a92aa745097e0",
            "d77eccdeaaf822225114bcc6bd4060471e7b50effae7a216c5c2fc338d930365b16543f1f8f3dc2266a591e49ed01afd",
            "49cfc2c7a0be3144b4020c00962e820c256acef5729bced04a0631a4f05002a15c952e8eb20033317003624f1323c968",
            "5822e90852e271ae9e523c1a38f0a1a1097bb3a4e89e5f33f464104d3082a090006e636580af1e03a91dc26c09676fcf",
            "3a40c3f1e650234c4184963c30d2650000502bf1a80000a4292bab1124072cb2a90000262b0cb40000422c05b4b31e17",
            "2ca5b38e241f2b5623000024296d21e725e32b14a800001e2c5a2800006c2e6120a9221c2ec220ad24cf2d82a7b723a5",
            "2d69a90000742ce3b200006e2c0cb11524512c2cac0000372beaad0000c92c90b30000bc2c0faa00007a2d02aa000038",
            "2e7aa77f24c52a59276625c12bb426d523b329d02a0000102ac3270000392ca92472226c2b352ce622302cc42a6d219f",
            "2c942b0000092d012700805a2d3929b123d22c07270000ad286b2b0080342da92a211e1f2da22aa426852c6c2aca234c",
            "2bdb292b19522d3a29ca27e92b9529ed271f2c5d28ef23392d8f2af126dd2aca28b322562dc4280080872c0e2c0080b3",
            "2b8a2c9ba4dd2aca2808a6112d92280000352d2dacb626f125109d2321cd9f6328cf24222546a07927a92812a9c120f8",
            "9da61997252e28aaa96c28ad1828a2d21c8e2a1b25069a9f29f4260c2918261bac931e3c2aaf20da9844295123bb26cf",
            "2baeae92224c2afb9c751e5c2945106219772a9321a51ec32ae91b7e20282b21a19f26f22b41ac7e1e761de2a9c6249b",
            "2ba2ac2a25d22b9aab2b25ac26a1198e25b22833a82023192141a2f022d594102322243718dea180210b19e5a1f92045",
            "2763abc81e438cfea2e5213f254aab441e5a254cab129cc52dada90fb4322eebac0eb4312e9812a21fbe2b0cb1992063",
            "2c23ae0000662de9ad6a9ed52dd5a5bcac042ea0b00000bc2aacb30000c62a0cb1aa1ded2d20a0a833082f332c0d34f3",
            "2e00804d1ee22d99a8bd308a2e51ab0db4fe2e0080a8b3132f332caa9df82d20a04d9eed2d99a8bdb0942e51ab030301",
            "1a021c0104000600427e801103031c190400090006009028d6ff007f807f8002031a1c0007001c20c064a5a1c25aa799",
            "1d081e287cf86722d136dca4e94a9e0626d329f808163d54d718975e11a86392712f7abbc07b16111b0710000d030300",
            "03061905020309020b180b0100117430ab8c86c090633daa8d107aef353a584f49af8df526eea804990ae6e6085c6e0a",
            "32013df2f109f502d0d6fd0a40fbb0116eddb9e9603ed2fd5dfdf5d77ae652edcade20ff1cfa21eb3112b1f8eb2cc900",
            "1a477b0637e330f2ff0b57f7f3f625f86bffa7eabe1797f711e096bb69039bfab1ea3408a3f253fb0000880e2209e52e",
            "5807d7f948f76dfa41f85fe6c1ff290ae20bc5f55af4d237000050dbeafcc4f9d936c34ae803bb0bc3fca8f61f0cfcfa",
            "b90b5effb71029faf647424d7e0ebcfc3ef83d023ef87e0a83eeb50acd0f48f9fbeba3f08d29ee1b140059723ff12290",
            "0c1370716eff1e95f31124f7c80f66970d00fdff7fed64000c00993fd3f1b6c44c005001cdf3c925233b052a0000f3c6",
            "96494f8abd2f9ffa9901e8f29fc170fe4f958128560a5c03e93f12002aa36a3f2b41abf7ae0242344ec13eec63998d29",
            "073dbae252fdbecbd5be55084f077eb7882cd886abcaae719c24d81e92501891e2ee9e4989c3f9b41906cf729cabe4a5",
            "098ce6d1140f8f191c8bc44f0e64fa609b6f7cc35a18fd626cfc412a4244d22588344191f465d241f97598ce04f840fd",
            "7478e15e16e437bbeae99dffa745d72ede75c67c897204a156f29fda04061c151914554843b2a90065331ce2de1a03f0",
            "f503f740076003e89aebfacb004001f9052aa1631252f4c701220e2915af01add88f1bc002690b12113dff0c14bc0bdd",
            "020cfb480b2a005d1b19030a2c25faaaeed1f92e0744055d048505cdfcca0b4bf5a784940a94031c0262fa53f1440364",
            "f606f6b000cedf45061504bb04fe033cfa9411c6fa06f9d5ff86082908e9c2a206752e5ef14404fafd35ba3ee8c1082e",
            "fc5906dafd38fb9ef89fff52f910fdecf466d7dee7a9fe25f7b0fa77feb0faccf89cf3d8f5b209b902f6f453f48974f2",
            "3812006c1cad0388e419e0c2fe2c005418750365e9e7427011a200b4e5b2fb8dfe0df86d24bdff2fe88dfbddfda8f50a",
            "16c8ff49e5b2fbcbfdf2028be6320212e9aa0267e6f3c459fd30002619ea0330e83bf8c0ff6928ec0639e027019ea93b",
            "06efff1b0fae034ff2c7f8ecff58f0d1060500370e8a0328f3dbf83000f4c0df7a9819209091dcb0fb9b5672a028fc5b",
            "1fc0ff3bf61000347248172e544f068a011a27d9ff7d04a59d3493b9e2d9fa6f1e3572a80b38bf870c312af8bf38e158",
            "023113056fbc0c6e016912a4c08607c425003897becb2ea455d20017e1ff25637baaee256be7feea122257841d894ccb",
            "184251a692351be59e75276a82fa0a008d9c1c25de636eec216fb1f75774e6fc00966af3f24279f201b19921ddaa9a51",
            "de2826e443f36483ed4667d5efd8d91cbcbf2d4c3c462cfdf2c72469f20dc3520f20c7e3fb4ae80afd82d435ed9dca5e",
            "f5c8bddeed1bbb4bd52fb970d538422212bab457e86b3e0aaff333afb9433da3b03a2a57c3732930c46a2047cd880919",
            "e4f90fa8dd57104addd35f10ec29096b3a00fbaa45b6502dfb9c1605d7091a97d3d7f695c53ee523c3bdddfbe3de7cb3",
            "024b806e042b88db0292d4d0daa8cde6d2d910e0e1e01fe71de45bea4eee8df4c79a97efef8bb50811744bf74ba728e1",
            "d57725fda3835c02db941901d25bab2dc1e71f384f66df228f817de8bf7a4c5d7ca4fdd91e521a7e2bcb120d9b7d1200",
            "b9f1d0f738eac1f71cf1cff7fed342fcbccc42fca6390c3c08c7134308280b3a08bc244bf7372e4ef750254cf7bf41e4",
            "05534ad205c1a305630865204af7023cf005fef24405d1ea570515f3450559fa320506f8def73cdb43fcf9e13f00bcda",
            "3700fbe13e002ee946e0e45500bedd4a00ace4540007ec5f00bc655104f248192188428427e37129f8b617f60234d02b",
            "421e204c3864e9fb2858e3062f6335a20aaeddb13400e2dfb4c731c22a27e1aee5d8159a0f50f8c3e6fb02a7eaeee5d1",
            "ea3965691030f2c524fb9516deab5f620a0eb7e7debf90340551e8a3ad16eddea855a09ef511c1e4d41b618bd8229c92",
            "13a98b1a034185b41052224fcbb57f92fbcc2fd5bd2e6afdfa772640604863a72306b02ed7b6451d41dab762cf5aa8d1",
            "8880f04225414a6a2c8d01e0529216e40142d009014245424f04824840155228915285360e206edaf2f764cf92b26d72",
            "9355eb5c2aaedc846c4104b7c93e205331b0639df0c78aa272b1c558daa198fa68bbceb63354cf9a3d61959244b75876",
            "3586024419757afa03cb25c4e6424c1cc72bb844f378abf913b06a9304ee0d3cd8853ffa6ce42f9ce9c5000dd640dabf",
            "edd7a3717488d685ea15eb65d1ecb48e2a6bfc02130103021c04050100dd0081008100ca8f0c960c9309813a6f7c063c",
            "6f664b7b07397082fb820ba13fb1632e70bd683f687f003b650b837f007fff0d82810083fc4fab7eff0a821b6ea83eb8",
            "5e7f027f003e6b3271b164b96281fda148b5648825880ba1532f09ce1b2f06a9b2eefda74e3cee7ffd43027d1739f765",
            "12f820f40bf7d04fda21d95ba8f005be31d80dd514d20eae07aa51edf6cf09c53f820a820dc01ae1fac308e4f332ebff",
            "1810d1001933ed0a827eff7f000dfc0d09edf960533f6d4367f2aa08b509810686dea3f9812078376ff770fdb5fdf7e5",
            "a200d4004bee45e63f2409fcd4f539086cf442008100b5009ae9840a821a971883f587f7870bf70e4008f4fd8af98b20",
            "86eff7f3f6d8a82a8bff8f078171e379dc7ff8ef842297ff8303f7033906419ec7eb86ee8578da7ffa7ef47ef37ff876",
            "fd1c3ee0035040bafaa806b039810688e883027df84c9b67b868bb7df37efc810281fb8fda81028fe182f1a8f8cb259b",
            "e581038fdf81ffad07b42094045d266b004dd58e00bff7a539bb0dacf0bc11ba372df93323b426df371be717e3e339fb",
            "d86bf26dea4c1fbc47b230be2632fb6fed67f782ff81fa8100a23bbb0bbd0eed3cf340f9f406f508d9f23e51ff41b74b",
            "e5a123a614b2203064d11bba1e81000e82ce8b7dfc76e009bdefcb484734e92d8d43960690ae0eb929b428217a247a0c",
            "7e1e70bc60b162bf32f107dd1f2a6d1f71b766af1dd111d515d0aee6a39be0f4d000ccadfb05560c593a63167c197c38",
            "72fb7a087b396c1c37d10a23381c79e06c1f79027fbf6c0f7eece9f1e7ab064de9fb05f7084ac603dbf7f05bd30ff007",
            "fa74e775e5278b3670750b760d7dfb7c0c309e315774e671d7cf539a21e55ff430ad05003406ab3b9e0da81784388e1a",
            "84fb883794078718c320c1cdf21c871f87e09401810e82bf94ed14acf8f2164d15f8f4fcf74a3bf61102265b2d07060f",
            "10731a2776741c369077f476f67e0930637ef831ab6e2c721df1fef101f100f987018496f597f50f88f007f315f309f3",
            "09fe060c15f9ff0ce7daf8ec07e9eb310e2de6320b051e315cfd73e477132c2c58097cf76f003900611069017f056d23",
            "76ff6e002ce47799fb94c082008200bac13b9c58a774ce5fac73cc9bc5f08fec8694bfb79b7df67ef578d85daa7bef70",
            "c4982c81fd82f788ef8208990a8102992599058b014bd632f867f948186be6f400f4fbf4fef501f503ee00e098eeee00",
            "0900fcfffefefffbee070407ec00ee000409ecf000e614f9fcdfeddf07e4e118e7ff82f8817fff0a826efb66f4fb830c",
            "85078285fcb9a7f88183fd810082fd3171bc622c76ee7d82038206b1639b460e827dfcf3a07efd851e9cf6870ac44528",
            "8a0d8257a649a0eaf4a9f3c2ebbfe8a8fb81fd91f788f2810082f483f088dcb529331abd0df20d81f881f782fd82fd82",
            "fdeffeeffdeffeefffef039bec0999cffbf1af5eb05fb578d44fe1f3e08f23b7a330b5196d65a5b2e90822df3462e565",
            "0ec85378dfee23d3fc099435aaf8fbd7514b6fd90841ba564c65bbf8d5e076c7630252e64c4935b80bd5ad94e30d48b6",
            "9e6b0d4d64cce5670c3df311e85ff9c4eab05defbd339c145e62cbc2c791a8b0ca4e179b6df48390c780c72c28e4e8ad",
            "e3e545ba9cbaa9b7c661c16cbca62131445e70bc0947f70203011b1c0400090042026011f1a106940000623a38020e11",
            "e8df1fe8f19df28043fd003620c8dfebf1fffff7fbfd0400d0498e215b22b1399b96d244781d50f180ecf5c918fa3cfa",
            "020800746031e1849d08707fdc2700844968a34642adb3c54afa840cc561804508d710e02bb984607340850da007534c",
            "0f4fdf0455190820d5ef4801844b22c243c28c08b077414d064425fe63d004c2495f626764eec63de033052602461064",
            "225039200a3281377012ded1675b70d764cc1371712927697ef23f8029e482c1c9d3318482ad09371fc0aa9e05622981",
            "042b92cd17363ba66723af33c0c6e8ce7ce7ef298cdabe0ccf34ca7a7174a7227c596b1ed88092f7d281280731aacf5f",
            "c0c5af5083bd907e2e49b6a8ee7d3ba544ad64659414335b945182d2266142edbb49ab8eb2610461aec4be476a2d23e4",
            "5806a2a80a73c44cbb46993d8b44a0115708b7148a623121047d3faedb020d20119c36c384c6008bc90efc07781b4404",
            "f866efff43fe001000d08c502d459eff00a075fd4ef5fc2a23d1aa9e4345a9750c8d73a0831cea552f4bf6a10ef64a24",
            "222ae75fd103c6bed0d8cb5d5966d02043f76610b5a488f7e6a3dea353ec341174e46de67055e1b88f1fa4d596778df9",
            "9b778f7a72b778705609392724fa9020ea40ca7de913000021fffe63ffffe917bfb0c80c03a68f00947be0e071da0087",
            "df002620f89bc3f9ff5f37f7ff031014225f3b28f8ee18139e0b245791681048a6b373a9908555eb47fd558be89f64d7",
            "750fbe7ce6ff078197fc1c2e0ece2947abff427a0d8ddbec5873c3081494b081aafa1c551502",
        ));
    let table = TableBuild {
        fwd: 0,
        rev_ptr: 0,
        rev_acc: 0,
        rev_bitpos: 0,
        w8: 523,
        symbols: 9,
        branch_bit: 0,
        dir_bit: 1,
        entries: vec![
            0x0a00_100b,
            0x0a00_0802,
            0x0a00_0802,
            0x1400_0a13,
            0x1400_100a,
            0x1400_100a,
            0x1400_0803,
            0x1400_0801,
            0x1400_0804,
        ],
        offsets: vec![0, 6, 8, 5232, 5236, 5240, 5244, 5247, 5248],
        cols: vec![0, 6, 8, 0, 4, 8, 12, 15, 16],
        longs: vec![131082, 524308],
        byte_group_total: 15696,
        max_prod: 48,
    };
    let mut state = VertexAttributeDriverState {
        current_attribute: 0,
        processed_vertices: 0,
        vertex_count: 523,
        block_limit: 32768,
        transform_state: ByteGroupTransformState {
            mode: 2,
            count_bits: 0,
            record_count: 0,
            second_count: 0,
            third_count: 0,
            tail_count: 0,
        },
        byte_state: ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 4469,
                acc: 13789291984359756104,
                bitpos: 58,
            },
            mode1_extra_readers: [
                RansThreeLaneReader {
                    ptr: 4524,
                    acc: 4679714676352931668,
                    bitpos: 58,
                },
                RansThreeLaneReader {
                    ptr: 4673,
                    acc: 11559655046995041492,
                    bitpos: 58,
                },
            ],
            stream_pos: 542,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        },
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
        ByteGroupTransformState {
            mode: 1,
            count_bits: 382,
            record_count: 0,
            second_count: 0,
            third_count: 0,
            tail_count: 0,
        }
    );
    assert_eq!(
        state.byte_state.reader,
        RansThreeLaneReader {
            ptr: 4469,
            acc: 8475775596583267328,
            bitpos: 48,
        }
    );

    let first = vertex_attribute_driver_step(&mut state, &table, &payload).unwrap();
    assert_eq!(state.current_attribute, 1);
    assert_eq!(
        first,
        VertexAttributeTransform {
            index: 0,
            table_entry: ByteGroupTransformTableEntry { raw: 0x0a00_100b },
            out_offset: 0,
            column: 0,
            limit: 523,
            ret: 523,
            records: hex_width_records("0b02000000000000"),
        }
    );
    assert_eq!(
        state.transform_state,
        ByteGroupTransformState {
            mode: 1,
            count_bits: 191,
            record_count: 1,
            second_count: 0,
            third_count: 0,
            tail_count: 0,
        }
    );
    assert_eq!(
        state.byte_state.reader,
        RansThreeLaneReader {
            ptr: 4469,
            acc: 8475775596583267328,
            bitpos: 48,
        }
    );

    let mut second_state = VertexAttributeDriverState {
        current_attribute: 1,
        processed_vertices: 0,
        vertex_count: 523,
        block_limit: 32768,
        transform_state: ByteGroupTransformState {
            mode: 1,
            count_bits: 191,
            record_count: 0,
            second_count: 0,
            third_count: 0,
            tail_count: 0,
        },
        byte_state: ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 4466,
                acc: 18419235586919153664,
                bitpos: 48,
            },
            mode1_extra_readers: [zero_three_lane_reader(); 2],
            stream_pos: 1149,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        },
    };
    let second = vertex_attribute_driver_step(&mut second_state, &table, &payload).unwrap();
    assert_eq!(second_state.current_attribute, 2);
    assert_eq!(
        second,
        VertexAttributeTransform {
            index: 1,
            table_entry: ByteGroupTransformTableEntry { raw: 0x0a00_0802 },
            out_offset: 6,
            column: 6,
            limit: 523,
            ret: 162,
            records: hex_width_records("010065010a0000009f0004005a0000000200000000000000"),
        }
    );
    assert_eq!(
        second_state.transform_state,
        ByteGroupTransformState {
            mode: 1,
            count_bits: 95,
            record_count: 3,
            second_count: 2,
            third_count: 2,
            tail_count: 3,
        }
    );
    assert_eq!(
        second_state.byte_state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 4464,
                acc: 4362398837027897376,
                bitpos: 55,
            },
            mode1_extra_readers: [zero_three_lane_reader(); 2],
            stream_pos: 1163,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        }
    );
    assert_eq!(
        vertex_attribute_driver_step(&mut state, &table, &payload),
        Err(VertexAttributeDriverError::ByteGroupTransform {
            index: 1,
            error: ByteGroupTransformError::UnobservedZeroTailBitstream,
        })
    );
}

/// Interstage source setup/materialization (`0x10f92c8..0x10f9394`).
///
/// Provenance: `capture_vertex_interstage.py`, Animal_Dragonfly current 2:
/// dispatch 16 calls setup `0x10fc4b0`, descriptor `(w2=0,w3=2,w4=3)`,
/// then one selector-3 `0x110d7f0` direct source `ff007f807f80`.
/// The same capture enumerates 25/25 interstage calls across
/// Bear/Bass/Dragonfly, replayed by `verify_vertex_interstage.py`.
#[test]
fn vertex_attribute_interstage_dragonfly_copy2_source() {
    let payload = sparse_payload(
        4470,
        &[
            (1177, "ff007f807f8002031a1c0007001c20c0"),
            (4454, "008bc90efc07781b4404f866efff43fe"),
        ],
    );
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 4454,
            acc: 2449949072564560396,
            bitpos: 57,
        },
        mode1_extra_readers: [
            RansThreeLaneReader {
                ptr: 4525,
                acc: 7776887029535970912,
                bitpos: 52,
            },
            RansThreeLaneReader {
                ptr: 4671,
                acc: 1333770381586800016,
                bitpos: 59,
            },
        ],
        stream_pos: 1177,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    };

    let interstage = vertex_attribute_interstage_sources(
        &mut state,
        &payload,
        ByteGroupTransformTableEntry { raw: 0x0a00_0802 },
        3,
    )
    .unwrap();

    assert_eq!(
        interstage,
        VertexAttributeInterstage {
            dispatch: 16,
            writer: VertexAttributeWriterTarget::Copy2,
            descriptors: vec![VertexAttributeSourceDescriptor {
                element_shift: 0,
                group_stride: 2,
                count: 3,
            }],
            sources: vec![VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("ff007f807f80"),
            }],
        }
    );
    assert_eq!(
        state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 4453,
                acc: 18442072214514965368,
                bitpos: 56,
            },
            mode1_extra_readers: [
                RansThreeLaneReader {
                    ptr: 4525,
                    acc: 7776887029535970912,
                    bitpos: 52,
                },
                RansThreeLaneReader {
                    ptr: 4671,
                    acc: 1333770381586800016,
                    bitpos: 59,
                },
            ],
            stream_pos: 1183,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        }
    );
}

/// Interstage source setup/materialization for dispatch 9.
///
/// Provenance: `capture_phase1_dispatch_9_interstage.py`,
/// Animal_Boar.Boar current 2: dispatch 9 selects setup `0x10fb730`,
/// reads one split varint, returns three descriptors, and maps to writer
/// `0x10fbee0`.
#[test]
fn vertex_attribute_interstage_dispatch9_delta4_sources() {
    let mut payload = vec![0u8; 19];
    payload[10] = 1;
    payload[11] = 0xaa;
    payload[12..15].copy_from_slice(&[0xbb, 0xcc, 0xdd]);
    payload[15..19].copy_from_slice(&[1, 2, 3, 4]);
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 2,
            acc: (9u64 << 57) | (3u64 << 55) | (3u64 << 53) | (3u64 << 51),
            bitpos: 57,
        },
        mode1_extra_readers: [zero_three_lane_reader(); 2],
        stream_pos: 10,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    };

    let interstage = vertex_attribute_interstage_sources(
        &mut state,
        &payload,
        ByteGroupTransformTableEntry { raw: 0x0e00_0804 },
        2,
    )
    .unwrap();

    assert_eq!(
        interstage,
        VertexAttributeInterstage {
            dispatch: 9,
            writer: VertexAttributeWriterTarget::Delta4,
            descriptors: vec![
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 1,
                    count: 1,
                },
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 3,
                    count: 1,
                },
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 4,
                    count: 1,
                },
            ],
            sources: vec![
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("aa"),
                },
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("bbccdd"),
                },
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("01020304"),
                },
            ],
        }
    );
    assert_eq!(
        state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 0,
                acc: 0,
                bitpos: 60,
            },
            mode1_extra_readers: [zero_three_lane_reader(); 2],
            stream_pos: 19,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        }
    );
}

/// Interstage source setup/materialization for dispatch 20.
///
/// Provenance: `capture_phase1_dispatch_20_interstage.py`,
/// DgnObj_Fire_WallBeam_A_09 current 2: dispatch 20 selects setup
/// `0x10fc4b0`, descriptor `(w2=0,w3=8,w4=460)`, and writer `0x10fc920`.
#[test]
fn vertex_attribute_interstage_dispatch20_copy8_source() {
    let mut payload = vec![0u8; 17];
    payload[9..17].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 1,
            acc: (20u64 << 57) | (3u64 << 55),
            bitpos: 57,
        },
        mode1_extra_readers: [zero_three_lane_reader(); 2],
        stream_pos: 9,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    };

    let interstage = vertex_attribute_interstage_sources(
        &mut state,
        &payload,
        ByteGroupTransformTableEntry { raw: 0x1c00_2012 },
        1,
    )
    .unwrap();

    assert_eq!(
        interstage,
        VertexAttributeInterstage {
            dispatch: 20,
            writer: VertexAttributeWriterTarget::Copy8,
            descriptors: vec![VertexAttributeSourceDescriptor {
                element_shift: 0,
                group_stride: 8,
                count: 1,
            }],
            sources: vec![VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("0102030405060708"),
            }],
        }
    );
    assert_eq!(
        state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 0,
                acc: 0,
                bitpos: 56,
            },
            mode1_extra_readers: [zero_three_lane_reader(); 2],
            stream_pos: 17,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        }
    );
}

/// Interstage source setup/materialization for dispatch 17.
///
/// Provenance: `capture_phase1_dispatch_17_interstage.py`,
/// CaveObj_WaterFallBottom_A_02 current 4: dispatch 17 selects setup
/// `0x10fc4b0`, descriptor `(w2=0,w3=3,w4=43)`, and writer `0x10fc720`.
#[test]
fn vertex_attribute_interstage_dispatch17_copy3_source() {
    let mut payload = vec![0u8; 12];
    payload[9..12].copy_from_slice(&[1, 2, 3]);
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 1,
            acc: (17u64 << 57) | (3u64 << 55),
            bitpos: 57,
        },
        mode1_extra_readers: [zero_three_lane_reader(); 2],
        stream_pos: 9,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    };

    let interstage = vertex_attribute_interstage_sources(
        &mut state,
        &payload,
        ByteGroupTransformTableEntry { raw: 0x0c00_0803 },
        1,
    )
    .unwrap();

    assert_eq!(
        interstage,
        VertexAttributeInterstage {
            dispatch: 17,
            writer: VertexAttributeWriterTarget::Copy3,
            descriptors: vec![VertexAttributeSourceDescriptor {
                element_shift: 0,
                group_stride: 3,
                count: 1,
            }],
            sources: vec![VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("010203"),
            }],
        }
    );
    assert_eq!(
        state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 0,
                acc: 0,
                bitpos: 56,
            },
            mode1_extra_readers: [zero_three_lane_reader(); 2],
            stream_pos: 12,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        }
    );
}

/// Interstage source setup/materialization for dispatch 19.
///
/// Provenance: `phase1_direction_zero_writer_loop_capture.json` from
/// `capture_vertex_writer_loop.py`, Animal_Shell_B.Shell_B row 0: dispatch 19
/// selects setup `0x10fc4b0`, descriptor `(w2=0,w3=6,w4=25)`, and writer
/// `0x10fc870`. This is the DirectionZero-only six-byte fixed-copy target.
#[test]
fn vertex_attribute_interstage_dispatch19_copy6_source() {
    let mut payload = vec![0u8; 21];
    payload[9..21].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 1,
            acc: (19u64 << 57) | (3u64 << 55),
            bitpos: 57,
        },
        mode1_extra_readers: [zero_three_lane_reader(); 2],
        stream_pos: 9,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    };

    let interstage = vertex_attribute_interstage_sources(
        &mut state,
        &payload,
        ByteGroupTransformTableEntry { raw: 0x0600_100b },
        2,
    )
    .unwrap();

    assert_eq!(
        interstage,
        VertexAttributeInterstage {
            dispatch: 19,
            writer: VertexAttributeWriterTarget::Copy6,
            descriptors: vec![VertexAttributeSourceDescriptor {
                element_shift: 0,
                group_stride: 6,
                count: 2,
            }],
            sources: vec![VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("0102030405060708090a0b0c"),
            }],
        }
    );
    assert_eq!(
        state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 0,
                acc: 0,
                bitpos: 56,
            },
            mode1_extra_readers: [zero_three_lane_reader(); 2],
            stream_pos: 21,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        }
    );
}

/// Interstage source setup/materialization for dispatch 29.
///
/// Provenance: `capture_phase1_dispatch_29_interstage.py`,
/// Animal_Crab.Crab current 1: dispatch 29 selects setup `0x10fc4e0`, reads
/// one split varint, and maps to writer `0x10fdb30`.
#[test]
fn vertex_attribute_interstage_dispatch29_delta1_direct_sources() {
    let mut payload = vec![0u8; 12];
    payload[9] = 1;
    payload[10] = 0xaa;
    payload[11] = 0xbb;
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 1,
            acc: (29u64 << 57) | (3u64 << 55) | (3u64 << 53),
            bitpos: 57,
        },
        mode1_extra_readers: [zero_three_lane_reader(); 2],
        stream_pos: 9,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    };

    let interstage = vertex_attribute_interstage_sources(
        &mut state,
        &payload,
        ByteGroupTransformTableEntry { raw: 0x0800_0801 },
        2,
    )
    .unwrap();

    assert_eq!(
        interstage,
        VertexAttributeInterstage {
            dispatch: 29,
            writer: VertexAttributeWriterTarget::Delta1Direct,
            descriptors: vec![
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 1,
                    count: 1,
                },
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 1,
                    count: 1,
                },
            ],
            sources: vec![
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("aa"),
                },
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("bb"),
                },
            ],
        }
    );
    assert_eq!(
        state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 0,
                acc: 0,
                bitpos: 54,
            },
            mode1_extra_readers: [zero_three_lane_reader(); 2],
            stream_pos: 12,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        }
    );
}

/// Interstage source setup/materialization for dispatch 35.
///
/// Provenance: `capture_transform_tail_10fdfe0.py`, Animal_Bass_Boneless
/// current 2: dispatch 35 selects setup `0x10fc4e0`, reads one split varint,
/// and maps to writer `0x10fdfe0` with two same-width source descriptors.
#[test]
fn vertex_attribute_interstage_dispatch35_u16x2_direct_delta_sources() {
    let mut payload = vec![0u8; 18];
    payload[9] = 1;
    payload[10..14].copy_from_slice(&[1, 2, 3, 4]);
    payload[14..18].copy_from_slice(&[5, 6, 7, 8]);
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 1,
            acc: (35u64 << 57) | (3u64 << 55) | (3u64 << 53),
            bitpos: 57,
        },
        mode1_extra_readers: [zero_three_lane_reader(); 2],
        stream_pos: 9,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    };

    let interstage = vertex_attribute_interstage_sources(
        &mut state,
        &payload,
        ByteGroupTransformTableEntry { raw: 0x1000_100a },
        2,
    )
    .unwrap();

    assert_eq!(
        interstage,
        VertexAttributeInterstage {
            dispatch: 35,
            writer: VertexAttributeWriterTarget::U16x2DirectDelta,
            descriptors: vec![
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 4,
                    count: 1,
                },
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 4,
                    count: 1,
                },
            ],
            sources: vec![
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("01020304"),
                },
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("05060708"),
                },
            ],
        }
    );
    assert_eq!(
        state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 0,
                acc: 0,
                bitpos: 54,
            },
            mode1_extra_readers: [zero_three_lane_reader(); 2],
            stream_pos: 18,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        }
    );
}

/// Interstage source setup/materialization for dispatch 39.
///
/// Provenance: `capture_phase1_dispatch_39_interstage.py`,
/// DgnObj_ElectricStoneWall_A_01 current 2: dispatch 39 selects setup
/// `0x10fc4e0`, reads one split varint, and maps to writer `0x10fe4d0`
/// with two same-width source descriptors.
#[test]
fn vertex_attribute_interstage_dispatch39_u32x2_delta_sources() {
    let mut payload = vec![0u8; 26];
    payload[9] = 1;
    payload[10..18].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
    payload[18..26].copy_from_slice(&[9, 10, 11, 12, 13, 14, 15, 16]);
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 1,
            acc: (39u64 << 57) | (3u64 << 55) | (3u64 << 53),
            bitpos: 57,
        },
        mode1_extra_readers: [zero_three_lane_reader(); 2],
        stream_pos: 9,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    };

    let interstage = vertex_attribute_interstage_sources(
        &mut state,
        &payload,
        ByteGroupTransformTableEntry { raw: 0x1400_2012 },
        2,
    )
    .unwrap();

    assert_eq!(
        interstage,
        VertexAttributeInterstage {
            dispatch: 39,
            writer: VertexAttributeWriterTarget::U32x2Delta,
            descriptors: vec![
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 8,
                    count: 1,
                },
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 8,
                    count: 1,
                },
            ],
            sources: vec![
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("0102030405060708"),
                },
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("090a0b0c0d0e0f10"),
                },
            ],
        }
    );
    assert_eq!(
        state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 0,
                acc: 0,
                bitpos: 54,
            },
            mode1_extra_readers: [zero_three_lane_reader(); 2],
            stream_pos: 26,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        }
    );
}

/// Interstage source setup/materialization for dispatch 46.
///
/// Provenance: `capture_phase1_dispatch_46_interstage.py`,
/// Animal_Grasshopper.Grasshopper current 4: dispatch 46 selects setup
/// `0x10fc4e0`, reads one split varint, and maps to writer `0x10ffdb0`.
#[test]
fn vertex_attribute_interstage_dispatch46_i8x3_direct_delta_sources() {
    let mut payload = vec![0u8; 16];
    payload[9] = 1;
    payload[10..13].copy_from_slice(&[1, 2, 3]);
    payload[13..16].copy_from_slice(&[4, 5, 6]);
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 1,
            acc: (46u64 << 57) | (3u64 << 55) | (3u64 << 53),
            bitpos: 57,
        },
        mode1_extra_readers: [zero_three_lane_reader(); 2],
        stream_pos: 9,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    };

    let interstage = vertex_attribute_interstage_sources(
        &mut state,
        &payload,
        ByteGroupTransformTableEntry { raw: 0x1000_0803 },
        2,
    )
    .unwrap();

    assert_eq!(
        interstage,
        VertexAttributeInterstage {
            dispatch: 46,
            writer: VertexAttributeWriterTarget::I8x3DirectDelta,
            descriptors: vec![
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 3,
                    count: 1,
                },
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 3,
                    count: 1,
                },
            ],
            sources: vec![
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("010203"),
                },
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("040506"),
                },
            ],
        }
    );
    assert_eq!(
        state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 0,
                acc: 0,
                bitpos: 54,
            },
            mode1_extra_readers: [zero_three_lane_reader(); 2],
            stream_pos: 16,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        }
    );
}

/// Interstage source setup/materialization for dispatch 61.
///
/// Provenance: `capture_phase1_dispatch_61_interstage.py`,
/// Animal_Boar.Boar current 7: dispatch 61 selects setup `0x11010b0`,
/// returns one descriptor, and maps to writer `0x1101230`.
#[test]
fn vertex_attribute_interstage_dispatch61_u8_previous_delta_source() {
    let mut payload = vec![0u8; 10];
    payload[9] = 0x7f;
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 1,
            acc: (61u64 << 57) | (3u64 << 55),
            bitpos: 57,
        },
        mode1_extra_readers: [zero_three_lane_reader(); 2],
        stream_pos: 9,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    };

    let interstage = vertex_attribute_interstage_sources(
        &mut state,
        &payload,
        ByteGroupTransformTableEntry { raw: 0x1000_0801 },
        1,
    )
    .unwrap();

    assert_eq!(
        interstage,
        VertexAttributeInterstage {
            dispatch: 61,
            writer: VertexAttributeWriterTarget::U8PreviousDelta,
            descriptors: vec![VertexAttributeSourceDescriptor {
                element_shift: 0,
                group_stride: 1,
                count: 1,
            }],
            sources: vec![VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("7f"),
            }],
        }
    );
    assert_eq!(
        state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 0,
                acc: 0,
                bitpos: 56,
            },
            mode1_extra_readers: [zero_three_lane_reader(); 2],
            stream_pos: 10,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        }
    );
}

/// Interstage source setup/materialization for dispatch 63.
///
/// Provenance: `capture_phase1_dispatch_63_interstage.py`,
/// CaveObj_WaterFall_A_04 current 4: dispatch 63 selects setup `0x11010b0`,
/// returns one descriptor, and maps to writer `0x1101410`.
#[test]
fn vertex_attribute_interstage_dispatch63_u8x3_previous_delta_source() {
    let mut payload = vec![0u8; 12];
    payload[9..12].copy_from_slice(&[1, 2, 3]);
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 1,
            acc: (63u64 << 57) | (3u64 << 55),
            bitpos: 57,
        },
        mode1_extra_readers: [zero_three_lane_reader(); 2],
        stream_pos: 9,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    };

    let interstage = vertex_attribute_interstage_sources(
        &mut state,
        &payload,
        ByteGroupTransformTableEntry { raw: 0x0c00_0803 },
        1,
    )
    .unwrap();

    assert_eq!(
        interstage,
        VertexAttributeInterstage {
            dispatch: 63,
            writer: VertexAttributeWriterTarget::U8x3PreviousDelta,
            descriptors: vec![VertexAttributeSourceDescriptor {
                element_shift: 0,
                group_stride: 3,
                count: 1,
            }],
            sources: vec![VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("010203"),
            }],
        }
    );
    assert_eq!(
        state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 0,
                acc: 0,
                bitpos: 56,
            },
            mode1_extra_readers: [zero_three_lane_reader(); 2],
            stream_pos: 12,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        }
    );
}

/// Interstage source setup/materialization for dispatch 67.
///
/// Provenance: `capture_phase1_dispatch_67_interstage.py`,
/// Animal_Boar.Boar current 4: dispatch 67 selects setup `0x11010b0`,
/// returns one descriptor, and maps to writer `0x1101850`.
#[test]
fn vertex_attribute_interstage_dispatch67_u16x2_previous_delta_source() {
    let mut payload = vec![0u8; 13];
    payload[9..13].copy_from_slice(&[1, 2, 3, 4]);
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 1,
            acc: (67u64 << 57) | (3u64 << 55),
            bitpos: 57,
        },
        mode1_extra_readers: [zero_three_lane_reader(); 2],
        stream_pos: 9,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    };

    let interstage = vertex_attribute_interstage_sources(
        &mut state,
        &payload,
        ByteGroupTransformTableEntry { raw: 0x1000_100a },
        1,
    )
    .unwrap();

    assert_eq!(
        interstage,
        VertexAttributeInterstage {
            dispatch: 67,
            writer: VertexAttributeWriterTarget::U16x2PreviousDelta,
            descriptors: vec![VertexAttributeSourceDescriptor {
                element_shift: 1,
                group_stride: 2,
                count: 1,
            }],
            sources: vec![VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("01020304"),
            }],
        }
    );
    assert_eq!(
        state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 0,
                acc: 0,
                bitpos: 56,
            },
            mode1_extra_readers: [zero_three_lane_reader(); 2],
            stream_pos: 13,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        }
    );
}

/// Interstage source setup/materialization for dispatch 77.
///
/// Provenance: `capture_phase1_dispatch_77_interstage.py`,
/// Animal_Cassowary.Cassowary current 1: dispatch 77 selects setup
/// `0x11010e0`, reads one split varint, and maps to writer `0x1103530`.
#[test]
fn vertex_attribute_interstage_dispatch77_u8x3_delta_sources() {
    let mut payload = vec![0u8; 16];
    payload[9] = 1;
    payload[10..13].copy_from_slice(&[1, 2, 3]);
    payload[13..16].copy_from_slice(&[4, 5, 6]);
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 1,
            acc: (77u64 << 57) | (3u64 << 55) | (3u64 << 53),
            bitpos: 57,
        },
        mode1_extra_readers: [zero_three_lane_reader(); 2],
        stream_pos: 9,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    };

    let interstage = vertex_attribute_interstage_sources(
        &mut state,
        &payload,
        ByteGroupTransformTableEntry { raw: 0x0c00_0803 },
        2,
    )
    .unwrap();

    assert_eq!(
        interstage,
        VertexAttributeInterstage {
            dispatch: 77,
            writer: VertexAttributeWriterTarget::U8x3Delta,
            descriptors: vec![
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 3,
                    count: 1,
                },
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 3,
                    count: 1,
                },
            ],
            sources: vec![
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("010203"),
                },
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("040506"),
                },
            ],
        }
    );
    assert_eq!(
        state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 0,
                acc: 0,
                bitpos: 54,
            },
            mode1_extra_readers: [zero_three_lane_reader(); 2],
            stream_pos: 16,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        }
    );
}

/// Interstage source setup/materialization for dispatch 79.
///
/// Provenance: `capture_phase1_dispatch_79_interstage.py`,
/// Item_Ore_L_Dummy current 1: dispatch 79 selects setup `0x11010e0`, reads
/// one split varint, and maps to writer `0x1103840`.
#[test]
fn vertex_attribute_interstage_dispatch79_pack10x3_previous_delta_sources() {
    let mut payload = vec![0u8; 22];
    payload[9] = 1;
    payload[10..16].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
    payload[16..22].copy_from_slice(&[7, 8, 9, 10, 11, 12]);
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 1,
            acc: (79u64 << 57) | (3u64 << 55) | (3u64 << 53),
            bitpos: 57,
        },
        mode1_extra_readers: [zero_three_lane_reader(); 2],
        stream_pos: 9,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    };

    let interstage = vertex_attribute_interstage_sources(
        &mut state,
        &payload,
        ByteGroupTransformTableEntry { raw: 0x1800_0a13 },
        2,
    )
    .unwrap();

    assert_eq!(
        interstage,
        VertexAttributeInterstage {
            dispatch: 79,
            writer: VertexAttributeWriterTarget::Pack10x3PreviousDelta,
            descriptors: vec![
                VertexAttributeSourceDescriptor {
                    element_shift: 1,
                    group_stride: 3,
                    count: 1,
                },
                VertexAttributeSourceDescriptor {
                    element_shift: 1,
                    group_stride: 3,
                    count: 1,
                },
            ],
            sources: vec![
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("010203040506"),
                },
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("0708090a0b0c"),
                },
            ],
        }
    );
    assert_eq!(
        state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 0,
                acc: 0,
                bitpos: 54,
            },
            mode1_extra_readers: [zero_three_lane_reader(); 2],
            stream_pos: 22,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        }
    );
}

/// Interstage source setup/materialization for dispatch 108.
///
/// Provenance: `phase1_direction_zero_writer_loop_capture.json`,
/// Armor_009 and Animal_Shell_B `0x110aba0` rows: dispatch 108 selects setup
/// `0x110aa00`, maps to writer `0x110aba0`, and uses the rounded 10-bit
/// element shift for the first two normal-vector sources.
#[test]
fn vertex_attribute_interstage_dispatch108_pack10x3_normal_sources() {
    let mut payload = vec![0u8; 16];
    payload[9..13].copy_from_slice(&[1, 2, 3, 4]);
    payload[13..15].copy_from_slice(&[5, 6]);
    payload[15] = 7;
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 2,
            acc: (108u64 << 57) | (3u64 << 55) | (3u64 << 53) | (3u64 << 51),
            bitpos: 57,
        },
        mode1_extra_readers: [zero_three_lane_reader(); 2],
        stream_pos: 9,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    };

    let interstage = vertex_attribute_interstage_sources(
        &mut state,
        &payload,
        ByteGroupTransformTableEntry { raw: 0x0c00_0a13 },
        1,
    )
    .unwrap();

    assert_eq!(
        interstage,
        VertexAttributeInterstage {
            dispatch: 108,
            writer: VertexAttributeWriterTarget::Pack10x3Normal,
            descriptors: vec![
                VertexAttributeSourceDescriptor {
                    element_shift: 1,
                    group_stride: 2,
                    count: 1,
                },
                VertexAttributeSourceDescriptor {
                    element_shift: 1,
                    group_stride: 1,
                    count: 1,
                },
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 1,
                    count: 1,
                },
            ],
            sources: vec![
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("01020304"),
                },
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("0506"),
                },
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("07"),
                },
            ],
        }
    );
}

/// Interstage source setup/materialization for dispatch 92.
///
/// Provenance: `phase1_direction_zero_writer_loop_capture.json`,
/// Armor_009 `0x1106250` row: setup `0x11071c0` reads two split varints and
/// derives five streams from `wrapper_ret * group_width`.
#[test]
fn vertex_attribute_interstage_dispatch92_f16x3_predict_sources() {
    let mut payload = vec![0u8; 35];
    payload[9] = 4;
    payload[10] = 5;
    payload[11..17].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
    payload[17..21].copy_from_slice(&[7, 8, 9, 10]);
    payload[21..23].copy_from_slice(&[11, 12]);
    payload[23..33].copy_from_slice(&[13, 14, 15, 16, 17, 18, 19, 20, 21, 22]);
    payload[33..35].copy_from_slice(&[23, 24]);
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 2,
            acc: (92u64 << 57)
                | (3u64 << 55)
                | (3u64 << 53)
                | (3u64 << 51)
                | (3u64 << 49)
                | (3u64 << 47),
            bitpos: 57,
        },
        mode1_extra_readers: [zero_three_lane_reader(); 2],
        stream_pos: 9,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    };

    let interstage = vertex_attribute_interstage_sources(
        &mut state,
        &payload,
        ByteGroupTransformTableEntry { raw: 0x0800_100b },
        2,
    )
    .unwrap();

    assert_eq!(
        interstage,
        VertexAttributeInterstage {
            dispatch: 92,
            writer: VertexAttributeWriterTarget::F16x3Predict,
            descriptors: vec![
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 1,
                    count: 6,
                },
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 1,
                    count: 4,
                },
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 1,
                    count: 2,
                },
                VertexAttributeSourceDescriptor {
                    element_shift: 1,
                    group_stride: 1,
                    count: 5,
                },
                VertexAttributeSourceDescriptor {
                    element_shift: 1,
                    group_stride: 1,
                    count: 1,
                },
            ],
            sources: vec![
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("010203040506"),
                },
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("0708090a"),
                },
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("0b0c"),
                },
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("0d0e0f10111213141516"),
                },
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("1718"),
                },
            ],
        }
    );
}

/// Interstage source setup/materialization for dispatch 110.
///
/// Provenance: `capture_phase1_dispatch_110_interstage.py`,
/// Animal_Fish_Cave.Fish_Cave current 4 and 5: dispatch 110 selects setup
/// `0x110aa40`, reads one split varint, and maps to writer `0x110ae30` with
/// four byte-sized normal-delta source descriptors.
#[test]
fn vertex_attribute_interstage_dispatch110_i8x3_normal_delta_sources() {
    let mut payload = vec![0u8; 17];
    payload[9] = 1;
    payload[10..12].copy_from_slice(&[1, 2]);
    payload[12] = 3;
    payload[13] = 4;
    payload[14..17].copy_from_slice(&[5, 6, 7]);
    let mut state = ByteGroupReadState {
        reader: RansThreeLaneReader {
            ptr: 2,
            acc: (110u64 << 57) | (3u64 << 55) | (3u64 << 53) | (3u64 << 51) | (3u64 << 49),
            bitpos: 57,
        },
        mode1_extra_readers: [zero_three_lane_reader(); 2],
        stream_pos: 9,
        segment_state: RansStateBuffer::cold(),
        selector2_history: Vec::new(),
    };

    let interstage = vertex_attribute_interstage_sources(
        &mut state,
        &payload,
        ByteGroupTransformTableEntry { raw: 0x0c00_0803 },
        2,
    )
    .unwrap();

    assert_eq!(
        interstage,
        VertexAttributeInterstage {
            dispatch: 110,
            writer: VertexAttributeWriterTarget::I8x3NormalDelta,
            descriptors: vec![
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 2,
                    count: 1,
                },
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 1,
                    count: 1,
                },
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 1,
                    count: 1,
                },
                VertexAttributeSourceDescriptor {
                    element_shift: 0,
                    group_stride: 3,
                    count: 1,
                },
            ],
            sources: vec![
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("0102"),
                },
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("03"),
                },
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("04"),
                },
                VertexAttributeSource {
                    selector: 3,
                    bytes: hex_bytes("050607"),
                },
            ],
        }
    );
    assert_eq!(
        state,
        ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 0,
                acc: 0x4000,
                bitpos: 58,
            },
            mode1_extra_readers: [zero_three_lane_reader(); 2],
            stream_pos: 17,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        }
    );
}

/// Writer-table dispatch (`0x10f93d8`) into the already-ported `0x10fc680`.
///
/// Provenance: `capture_vertex_interstage.py` plus
/// `capture_transform_tails.py`, Animal_Dragonfly current 2. The wrapper
/// records are `(1,435,10)`, `(1,77,760)`, `(1,8,100)` and the interstage
/// materializes the single source stream `ff007f807f80`.
#[test]
fn vertex_attribute_writer_dispatch_dragonfly_copy2() {
    let transform = VertexAttributeTransform {
        index: 2,
        table_entry: ByteGroupTransformTableEntry { raw: 0x0a00_0802 },
        out_offset: 8,
        column: 8,
        limit: 523,
        ret: 3,
        records: hex_width_records("0100b3010a00000001004d00f80200000100080064000000"),
    };
    let interstage = VertexAttributeInterstage {
        dispatch: 16,
        writer: VertexAttributeWriterTarget::Copy2,
        descriptors: vec![VertexAttributeSourceDescriptor {
            element_shift: 0,
            group_stride: 2,
            count: 3,
        }],
        sources: vec![VertexAttributeSource {
            selector: 3,
            bytes: hex_bytes("ff007f807f80"),
        }],
    };
    let mut out = vec![0xee; 5230];

    let usage = vertex_attribute_apply_writer(
        &mut out,
        VertexAttributeWriterCall {
            transform: &transform,
            interstage: &interstage,
            matches: &[],
            aux_table: &[],
            block_index: 0,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        VertexAttributeWriterUsage {
            sources: [6, 0, 0, 0, 0],
            match_entries: 0,
            aux_entries: 0,
        }
    );
    assert_eq!(&out[8..10], &hex_bytes("ff00"));
    assert_eq!(&out[18..20], &hex_bytes("ff00"));
    assert_eq!(&out[4368..4370], &hex_bytes("7f80"));
    assert_eq!(&out[5148..5150], &hex_bytes("7f80"));
    assert_eq!(out[7], 0xee, "out_offset is byte-based, not contiguous");
}

/// Writer-table dispatch (`0x10f93d8`) into `0x10fc920`.
#[test]
fn vertex_attribute_writer_dispatch_copy8() {
    let transform = VertexAttributeTransform {
        index: 2,
        table_entry: ByteGroupTransformTableEntry { raw: 0x1c00_2012 },
        out_offset: 0,
        column: 0,
        limit: 19,
        ret: 19,
        records: vec![[0x0001_0012, 280]],
    };
    let interstage = VertexAttributeInterstage {
        dispatch: 20,
        writer: VertexAttributeWriterTarget::Copy8,
        descriptors: Vec::new(),
        sources: vec![VertexAttributeSource {
            selector: 3,
            bytes: hex_bytes(concat!(
                "1b0c22bf78e748bf9a9919bf8c6144bf6fcf18bf28e846bf",
                "9a9919bfd814b63d9a9919bf30de7c3dc6ee22bf70308a3d",
                "d0cccc3eb0b02a3fd0cccc3e661a323fa0fce03e20bb2f3f",
                "7d434441829855c0d1884341e28f55c02d944441400454c0",
                "3333b33f80a4833ebcb0b53ff0a6833e3333b33ffcb86a3e",
                "d0cccc3e7af05a3f5856e43eb062513fd0cccc3e4eb4503f",
            )),
        }],
    };
    let mut out = vec![0xee; 19 * 28];

    let usage = vertex_attribute_apply_writer(
        &mut out,
        VertexAttributeWriterCall {
            transform: &transform,
            interstage: &interstage,
            matches: &[],
            aux_table: &[],
            block_index: 0,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        VertexAttributeWriterUsage {
            sources: [144, 0, 0, 0, 0],
            match_entries: 0,
            aux_entries: 0,
        }
    );
    assert_eq!(&out[0..8], &hex_bytes("1b0c22bf78e748bf"));
    assert_eq!(&out[18 * 28..18 * 28 + 8], &hex_bytes("a0fce03e20bb2f3f"));
    assert_eq!(out[8], 0xee, "writer must keep the table stride");
}

/// Writer-table dispatch (`0x10f93d8`) into `0x10fdb30`.
#[test]
fn vertex_attribute_writer_dispatch_delta1_direct() {
    let transform = VertexAttributeTransform {
        index: 0,
        table_entry: ByteGroupTransformTableEntry { raw: 0x0800_0801 },
        out_offset: 0,
        column: 0,
        limit: 4,
        ret: 2,
        records: vec![[0x0001_0003, 8]],
    };
    let interstage = VertexAttributeInterstage {
        dispatch: 29,
        writer: VertexAttributeWriterTarget::Delta1Direct,
        descriptors: Vec::new(),
        sources: vec![
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("1001"),
            },
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("05"),
            },
        ],
    };
    let mut out = vec![0xee; 25];

    let usage = vertex_attribute_apply_writer(
        &mut out,
        VertexAttributeWriterCall {
            transform: &transform,
            interstage: &interstage,
            matches: &[0, 8, 0, 0],
            aux_table: &[],
            block_index: 0,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        VertexAttributeWriterUsage {
            sources: [2, 1, 0, 0, 0],
            match_entries: 4,
            aux_entries: 0,
        }
    );
    assert_eq!(out[0], 0x10);
    assert_eq!(out[8], 0x15);
    assert_eq!(out[16], 0x01);
    assert_eq!(out[24], 0x01);
    assert_eq!(out[1], 0xee, "writer must keep the table stride");
}

/// Writer-table dispatch (`0x10f93d8`) into `0x10fbee0`.
///
/// This wires dispatch 9's three-source interstage to the four-byte
/// direct/match/copy writer. The first row exercises the direct
/// `source0 - byte1 - byte2 - byte3 - 1` lane-0 rule, the second uses
/// `match_entry >> 3` as the prior-row distance, and the third is copied by
/// the record tail.
#[test]
fn vertex_attribute_writer_dispatch_delta4() {
    let transform = VertexAttributeTransform {
        index: 0,
        table_entry: ByteGroupTransformTableEntry { raw: 0x0e00_0804 },
        out_offset: 0,
        column: 0,
        limit: 3,
        ret: 1,
        records: vec![[0x0001_0002, 14]],
    };
    let interstage = VertexAttributeInterstage {
        dispatch: 9,
        writer: VertexAttributeWriterTarget::Delta4,
        descriptors: Vec::new(),
        sources: vec![
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("0b"),
            },
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("020304"),
            },
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("01010101"),
            },
        ],
    };
    let mut out = vec![0xee; 32];

    let usage = vertex_attribute_apply_writer(
        &mut out,
        VertexAttributeWriterCall {
            transform: &transform,
            interstage: &interstage,
            matches: &[0, 8, 0],
            aux_table: &[],
            block_index: 0,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        VertexAttributeWriterUsage {
            sources: [1, 3, 4, 0, 0],
            match_entries: 3,
            aux_entries: 0,
        }
    );
    assert_eq!(&out[0..4], &hex_bytes("01020304"));
    assert_eq!(&out[14..18], &hex_bytes("02030405"));
    assert_eq!(&out[28..32], &hex_bytes("02030405"));
    assert_eq!(out[4], 0xee, "writer must keep the table stride");
}

/// Writer-table dispatch (`0x10f93d8`) into `0x10fdfe0`.
///
/// This wires the dispatch-level `VertexAttributeWriterCall` to the
/// two-u16 direct/delta tail, covering direct, matched, and copy rows with the
/// table metadata shape selected by dispatch 35.
#[test]
fn vertex_attribute_writer_dispatch_u16x2_direct_delta() {
    let transform = VertexAttributeTransform {
        index: 0,
        table_entry: ByteGroupTransformTableEntry { raw: 0x1000_100a },
        out_offset: 0,
        column: 0,
        limit: 3,
        ret: 2,
        records: vec![[0x0001_0002, 16]],
    };
    let interstage = VertexAttributeInterstage {
        dispatch: 35,
        writer: VertexAttributeWriterTarget::U16x2DirectDelta,
        descriptors: Vec::new(),
        sources: vec![
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("01000200"),
            },
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("01000200"),
            },
        ],
    };
    let mut out = vec![0xee; 36];

    let usage = vertex_attribute_apply_writer(
        &mut out,
        VertexAttributeWriterCall {
            transform: &transform,
            interstage: &interstage,
            matches: &[0, 8, 0],
            aux_table: &[],
            block_index: 0,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        VertexAttributeWriterUsage {
            sources: [4, 4, 0, 0, 0],
            match_entries: 3,
            aux_entries: 0,
        }
    );
    assert_eq!(&out[0..4], &hex_bytes("01000200"));
    assert_eq!(&out[16..20], &hex_bytes("02000400"));
    assert_eq!(&out[32..36], &hex_bytes("02000400"));
    assert_eq!(out[4], 0xee, "writer must keep the table stride");
}

/// Writer-table dispatch (`0x10f93d8`) into `0x10ffdb0`.
#[test]
fn vertex_attribute_writer_dispatch_i8x3_direct_delta() {
    let transform = VertexAttributeTransform {
        index: 0,
        table_entry: ByteGroupTransformTableEntry { raw: 0x1000_0803 },
        out_offset: 0,
        column: 0,
        limit: 3,
        ret: 2,
        records: vec![[0x0000_0003, 0]],
    };
    let interstage = VertexAttributeInterstage {
        dispatch: 46,
        writer: VertexAttributeWriterTarget::I8x3DirectDelta,
        descriptors: Vec::new(),
        sources: vec![
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("102030010203"),
            },
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("010203"),
            },
        ],
    };
    let mut out = vec![0xee; 35];

    let usage = vertex_attribute_apply_writer(
        &mut out,
        VertexAttributeWriterCall {
            transform: &transform,
            interstage: &interstage,
            matches: &[0, 9, 0],
            aux_table: &[],
            block_index: 0,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        VertexAttributeWriterUsage {
            sources: [6, 3, 0, 0, 0],
            match_entries: 3,
            aux_entries: 0,
        }
    );
    assert_eq!(&out[0..3], &hex_bytes("102030"));
    assert_eq!(&out[16..19], &hex_bytes("f12233"));
    assert_eq!(&out[32..35], &hex_bytes("010203"));
    assert_eq!(out[3], 0xee, "writer must keep the table stride");
}

/// Writer-table dispatch (`0x10f93d8`) into `0x1101850`.
#[test]
fn vertex_attribute_writer_dispatch_u16x2_previous_delta() {
    let transform = VertexAttributeTransform {
        index: 0,
        table_entry: ByteGroupTransformTableEntry { raw: 0x1000_100a },
        out_offset: 0,
        column: 0,
        limit: 3,
        ret: 1,
        records: vec![[0x0001_0002, 16]],
    };
    let interstage = VertexAttributeInterstage {
        dispatch: 67,
        writer: VertexAttributeWriterTarget::U16x2PreviousDelta,
        descriptors: Vec::new(),
        sources: vec![VertexAttributeSource {
            selector: 3,
            bytes: hex_bytes("0100020003000400"),
        }],
    };
    let mut out = vec![0xee; 36];

    let usage = vertex_attribute_apply_writer(
        &mut out,
        VertexAttributeWriterCall {
            transform: &transform,
            interstage: &interstage,
            matches: &[0xdead_beef, 0x0102_0304, 0x5566_7788],
            aux_table: &[],
            block_index: 0,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        VertexAttributeWriterUsage {
            sources: [8, 0, 0, 0, 0],
            match_entries: 0,
            aux_entries: 0,
        }
    );
    assert_eq!(&out[0..4], &hex_bytes("01000200"));
    assert_eq!(&out[16..20], &hex_bytes("04000600"));
    assert_eq!(&out[32..36], &hex_bytes("04000600"));
    assert_eq!(out[4], 0xee, "writer must keep the table stride");
}

/// Writer-table dispatch (`0x10f93d8`) into `0x10fe4d0`.
///
/// This wires dispatch 39's two-source interstage to the two-u32
/// direct/matched/copy writer.
#[test]
fn vertex_attribute_writer_dispatch_u32x2_delta() {
    let transform = VertexAttributeTransform {
        index: 0,
        table_entry: ByteGroupTransformTableEntry { raw: 0x1400_2012 },
        out_offset: 0,
        column: 0,
        limit: 3,
        ret: 1,
        records: vec![[0x0001_0002, 20]],
    };
    let interstage = VertexAttributeInterstage {
        dispatch: 39,
        writer: VertexAttributeWriterTarget::U32x2Delta,
        descriptors: Vec::new(),
        sources: vec![
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("0100000002000000"),
            },
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("0300000004000000"),
            },
        ],
    };
    let mut out = vec![0xee; 48];

    let usage = vertex_attribute_apply_writer(
        &mut out,
        VertexAttributeWriterCall {
            transform: &transform,
            interstage: &interstage,
            matches: &[0, 8, 0],
            aux_table: &[],
            block_index: 0,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        VertexAttributeWriterUsage {
            sources: [8, 8, 0, 0, 0],
            match_entries: 3,
            aux_entries: 0,
        }
    );
    assert_eq!(&out[0..8], &hex_bytes("0100000002000000"));
    assert_eq!(&out[20..28], &hex_bytes("0400000006000000"));
    assert_eq!(&out[40..48], &hex_bytes("0400000006000000"));
    assert_eq!(out[8], 0xee, "writer must keep the table stride");
}

/// Writer-table dispatch (`0x10f93d8`) into `0x1101230`.
#[test]
fn vertex_attribute_writer_dispatch_u8_previous_delta() {
    let transform = VertexAttributeTransform {
        index: 0,
        table_entry: ByteGroupTransformTableEntry { raw: 0x1000_0801 },
        out_offset: 0,
        column: 0,
        limit: 4,
        ret: 1,
        records: vec![[0x0001_0003, 16]],
    };
    let interstage = VertexAttributeInterstage {
        dispatch: 61,
        writer: VertexAttributeWriterTarget::U8PreviousDelta,
        descriptors: Vec::new(),
        sources: vec![VertexAttributeSource {
            selector: 3,
            bytes: hex_bytes("7f01ff"),
        }],
    };
    let mut out = vec![0xee; 49];

    let usage = vertex_attribute_apply_writer(
        &mut out,
        VertexAttributeWriterCall {
            transform: &transform,
            interstage: &interstage,
            matches: &[],
            aux_table: &[],
            block_index: 0,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        VertexAttributeWriterUsage {
            sources: [3, 0, 0, 0, 0],
            match_entries: 0,
            aux_entries: 0,
        }
    );
    assert_eq!(out[0], 0x7f);
    assert_eq!(out[16], 0x80);
    assert_eq!(out[32], 0x7f);
    assert_eq!(out[48], 0x7f);
    assert_eq!(out[1], 0xee, "writer must keep the table stride");
}

/// Writer-table dispatch (`0x10f93d8`) into `0x1101410`.
#[test]
fn vertex_attribute_writer_dispatch_u8x3_previous_delta() {
    let transform = VertexAttributeTransform {
        index: 0,
        table_entry: ByteGroupTransformTableEntry { raw: 0x1000_0803 },
        out_offset: 0,
        column: 0,
        limit: 4,
        ret: 1,
        records: vec![[0x0001_0003, 16]],
    };
    let interstage = VertexAttributeInterstage {
        dispatch: 63,
        writer: VertexAttributeWriterTarget::U8x3PreviousDelta,
        descriptors: Vec::new(),
        sources: vec![VertexAttributeSource {
            selector: 3,
            bytes: hex_bytes("0102030100ff000000"),
        }],
    };
    let mut out = vec![0xee; 51];

    let usage = vertex_attribute_apply_writer(
        &mut out,
        VertexAttributeWriterCall {
            transform: &transform,
            interstage: &interstage,
            matches: &[],
            aux_table: &[],
            block_index: 0,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        VertexAttributeWriterUsage {
            sources: [9, 0, 0, 0, 0],
            match_entries: 0,
            aux_entries: 0,
        }
    );
    assert_eq!(&out[0..3], &hex_bytes("010203"));
    assert_eq!(&out[16..19], &hex_bytes("020202"));
    assert_eq!(&out[32..35], &hex_bytes("020202"));
    assert_eq!(&out[48..51], &hex_bytes("020202"));
    assert_eq!(out[3], 0xee, "writer must keep the table stride");
}

/// Writer-table dispatch (`0x10f93d8`) into `0x1103530`.
#[test]
fn vertex_attribute_writer_dispatch_u8x3_delta() {
    let transform = VertexAttributeTransform {
        index: 0,
        table_entry: ByteGroupTransformTableEntry { raw: 0x0c00_0803 },
        out_offset: 0,
        column: 0,
        limit: 4,
        ret: 2,
        records: vec![[0x0001_0003, 12]],
    };
    let interstage = VertexAttributeInterstage {
        dispatch: 77,
        writer: VertexAttributeWriterTarget::U8x3Delta,
        descriptors: Vec::new(),
        sources: vec![
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("010203010101"),
            },
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("010000"),
            },
        ],
    };
    let mut out = vec![0xee; 39];

    let usage = vertex_attribute_apply_writer(
        &mut out,
        VertexAttributeWriterCall {
            transform: &transform,
            interstage: &interstage,
            matches: &[0, 0, 8, 0],
            aux_table: &[],
            block_index: 0,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        VertexAttributeWriterUsage {
            sources: [6, 3, 0, 0, 0],
            match_entries: 4,
            aux_entries: 0,
        }
    );
    assert_eq!(&out[0..3], &hex_bytes("010203"));
    assert_eq!(&out[12..15], &hex_bytes("020304"));
    assert_eq!(&out[24..27], &hex_bytes("030304"));
    assert_eq!(&out[36..39], &hex_bytes("030304"));
    assert_eq!(out[3], 0xee, "writer must keep the table stride");
}

/// Writer-table dispatch (`0x10f93d8`) into `0x1103840`.
#[test]
fn vertex_attribute_writer_dispatch_pack10x3_previous_delta() {
    let transform = VertexAttributeTransform {
        index: 0,
        table_entry: ByteGroupTransformTableEntry { raw: 0x1000_0a13 },
        out_offset: 0,
        column: 0,
        limit: 4,
        ret: 2,
        records: vec![[0x0001_0003, 16]],
    };
    let interstage = VertexAttributeInterstage {
        dispatch: 79,
        writer: VertexAttributeWriterTarget::Pack10x3PreviousDelta,
        descriptors: Vec::new(),
        sources: vec![
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("010002000300010000000000"),
            },
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("010001000100"),
            },
        ],
    };
    let mut out = vec![0xee; 52];

    let usage = vertex_attribute_apply_writer(
        &mut out,
        VertexAttributeWriterCall {
            transform: &transform,
            interstage: &interstage,
            matches: &[0, 0, 8, 0],
            aux_table: &[],
            block_index: 0,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        VertexAttributeWriterUsage {
            sources: [12, 6, 0, 0, 0],
            match_entries: 4,
            aux_entries: 0,
        }
    );
    assert_eq!(&out[0..4], &hex_bytes("01083000"));
    assert_eq!(&out[16..20], &hex_bytes("02083000"));
    assert_eq!(&out[32..36], &hex_bytes("030c4000"));
    assert_eq!(&out[48..52], &hex_bytes("030c4000"));
    assert_eq!(out[4], 0xee, "writer must keep the table stride");
}

/// Writer-table dispatch (`0x10f93d8`) into `0x110ae30`.
#[test]
fn vertex_attribute_writer_dispatch_i8x3_normal_delta() {
    let transform = VertexAttributeTransform {
        index: 0,
        table_entry: ByteGroupTransformTableEntry { raw: 0x0c00_0803 },
        out_offset: 0,
        column: 0,
        limit: 1,
        ret: 1,
        records: vec![[1, 0]],
    };
    let interstage = VertexAttributeInterstage {
        dispatch: 110,
        writer: VertexAttributeWriterTarget::I8x3NormalDelta,
        descriptors: Vec::new(),
        sources: vec![
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("04f8"),
            },
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("00"),
            },
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("00"),
            },
            VertexAttributeSource {
                selector: 3,
                bytes: Vec::new(),
            },
        ],
    };
    let mut out = vec![0xee; 12];

    let usage = vertex_attribute_apply_writer(
        &mut out,
        VertexAttributeWriterCall {
            transform: &transform,
            interstage: &interstage,
            matches: &[0],
            aux_table: &[],
            block_index: 0,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        VertexAttributeWriterUsage {
            sources: [2, 1, 1, 0, 0],
            match_entries: 1,
            aux_entries: 0,
        }
    );
    assert_eq!(&out[0..3], &hex_bytes("04f87f"));
    assert_eq!(out[3], 0xee, "writer must keep the table stride");
}

/// Writer-table dispatch (`0x10f93d8`) into `0x110aba0`.
#[test]
fn vertex_attribute_writer_dispatch_pack10x3_normal() {
    let transform = VertexAttributeTransform {
        index: 0,
        table_entry: ByteGroupTransformTableEntry { raw: 0x0c00_0a13 },
        out_offset: 0,
        column: 0,
        limit: 3,
        ret: 1,
        records: vec![[0x0002_0001, 12]],
    };
    let interstage = VertexAttributeInterstage {
        dispatch: 108,
        writer: VertexAttributeWriterTarget::Pack10x3Normal,
        descriptors: Vec::new(),
        sources: vec![
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("8b039103"),
            },
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("0000"),
            },
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("00"),
            },
        ],
    };
    let mut out = vec![0xee; 28];

    let usage = vertex_attribute_apply_writer(
        &mut out,
        VertexAttributeWriterCall {
            transform: &transform,
            interstage: &interstage,
            matches: &[],
            aux_table: &[],
            block_index: 0,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        VertexAttributeWriterUsage {
            sources: [4, 2, 1, 0, 0],
            match_entries: 3,
            aux_entries: 0,
        }
    );
    assert_eq!(&out[0..4], &hex_bytes("8b475e1e"));
    assert_eq!(&out[12..16], &hex_bytes("8b475e1e"));
    assert_eq!(&out[24..28], &hex_bytes("8b475e1e"));
    assert_eq!(out[4], 0xee, "writer must keep the table stride");
}

/// Writer-table dispatch (`0x10f93d8`) into `0x1106250`.
#[test]
fn vertex_attribute_writer_dispatch_f16x3_predict() {
    let transform = VertexAttributeTransform {
        index: 0,
        table_entry: ByteGroupTransformTableEntry { raw: 0x0800_100b },
        out_offset: 0,
        column: 0,
        limit: 3,
        ret: 2,
        records: vec![[0x0001_0002, 8]],
    };
    let interstage = VertexAttributeInterstage {
        dispatch: 92,
        writer: VertexAttributeWriterTarget::F16x3Predict,
        descriptors: Vec::new(),
        sources: vec![
            VertexAttributeSource {
                selector: 3,
                bytes: vec![0; 6],
            },
            VertexAttributeSource {
                selector: 3,
                bytes: vec![0; 6],
            },
            VertexAttributeSource {
                selector: 3,
                bytes: Vec::new(),
            },
            VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("020004000600020002000200"),
            },
            VertexAttributeSource {
                selector: 3,
                bytes: Vec::new(),
            },
        ],
    };
    let aux_table = [0u64; 2];
    let mut out = vec![0xee; 24];

    let usage = vertex_attribute_apply_writer(
        &mut out,
        VertexAttributeWriterCall {
            transform: &transform,
            interstage: &interstage,
            matches: &[],
            aux_table: &aux_table,
            block_index: 0,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        VertexAttributeWriterUsage {
            sources: [6, 6, 0, 12, 0],
            match_entries: 0,
            aux_entries: 2,
        }
    );
    assert_eq!(&out[0..6], &hex_bytes("010002000300"));
    assert_eq!(&out[8..14], &hex_bytes("020003000400"));
    assert_eq!(&out[16..22], &hex_bytes("020003000400"));
    assert_eq!(out[6], 0xee, "writer must keep the table stride");
}

/// Composed per-attribute writer-loop step (`0x10f924c..0x10f93d8`).
///
/// Provenance: compact Animal_Dragonfly current-2 slice from
/// `capture_vertex_interstage.py` plus the already-validated copy2 writer
/// path. This fixture-free golden drives `vertex_attribute_driver_step`,
/// `vertex_attribute_interstage_sources`, and `vertex_attribute_apply_writer`
/// together so the writer receives the shared `ctx+0x228` match table.
#[test]
fn vertex_attribute_writer_loop_step_dragonfly_copy2() {
    let payload = sparse_payload(
        4470,
        &[
            (
                1160,
                "427e801103031c190400090006009028d6ff007f807f8002031a1c000700",
            ),
            (4450, "36c384c6008bc90efc07781b4404f866efff"),
        ],
    );
    let table = TableBuild {
        fwd: 0,
        rev_ptr: 0,
        rev_acc: 0,
        rev_bitpos: 0,
        w8: 523,
        symbols: 1,
        branch_bit: 0,
        dir_bit: 1,
        entries: vec![0x0a00_0802],
        offsets: vec![8],
        cols: vec![8],
        longs: vec![131082, 524308],
        byte_group_total: 15696,
        max_prod: 2,
    };
    let mut state = VertexAttributeDriverState {
        current_attribute: 0,
        processed_vertices: 0,
        vertex_count: 523,
        block_limit: 523,
        transform_state: ByteGroupTransformState {
            mode: 1,
            count_bits: 95,
            record_count: 3,
            second_count: 2,
            third_count: 2,
            tail_count: 3,
        },
        byte_state: ByteGroupReadState {
            reader: RansThreeLaneReader {
                ptr: 4458,
                acc: 2306402145233977344,
                bitpos: 47,
            },
            mode1_extra_readers: [
                RansThreeLaneReader {
                    ptr: 4525,
                    acc: 7776887029535970912,
                    bitpos: 52,
                },
                RansThreeLaneReader {
                    ptr: 4671,
                    acc: 1333770381586800016,
                    bitpos: 59,
                },
            ],
            stream_pos: 1164,
            segment_state: RansStateBuffer::cold(),
            selector2_history: Vec::new(),
        },
    };
    let mut out = vec![0xee; 5230];
    let matches = [0u32; 523];

    let step = vertex_attribute_writer_loop_step(
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
    .unwrap();

    assert_eq!(state.current_attribute, 1);
    assert_eq!(
        step.transform,
        VertexAttributeTransform {
            index: 0,
            table_entry: ByteGroupTransformTableEntry { raw: 0x0a00_0802 },
            out_offset: 8,
            column: 8,
            limit: 523,
            ret: 3,
            records: hex_width_records("0100b3010a00000001004d00f80200000100080064000000"),
        }
    );
    assert_eq!(
        step.interstage,
        VertexAttributeInterstage {
            dispatch: 16,
            writer: VertexAttributeWriterTarget::Copy2,
            descriptors: vec![VertexAttributeSourceDescriptor {
                element_shift: 0,
                group_stride: 2,
                count: 3,
            }],
            sources: vec![VertexAttributeSource {
                selector: 3,
                bytes: hex_bytes("ff007f807f80"),
            }],
        }
    );
    assert_eq!(
        step.usage,
        VertexAttributeWriterUsage {
            sources: [6, 0, 0, 0, 0],
            match_entries: 0,
            aux_entries: 0,
        }
    );
    assert_eq!(&out[8..10], &hex_bytes("ff00"));
    assert_eq!(&out[18..20], &hex_bytes("ff00"));
    assert_eq!(&out[4368..4370], &hex_bytes("7f80"));
    assert_eq!(&out[5148..5150], &hex_bytes("7f80"));
    assert_eq!(out[7], 0xee);
}

/// Writer-table dispatch (`0x10f93d8`) into `0x10fc720`.
///
/// Provenance: `capture_phase1_dispatch_17_interstage.py` plus
/// `capture_phase1_transform_tail_10fc720.py`, dispatch 17 maps through the
/// one-source setup family to the three-byte fixed-copy writer.
#[test]
fn vertex_attribute_writer_dispatch_copy3() {
    let transform = VertexAttributeTransform {
        index: 0,
        table_entry: ByteGroupTransformTableEntry { raw: 0x0c00_0803 },
        out_offset: 0,
        column: 0,
        limit: 4,
        ret: 1,
        records: vec![[0x0001_0003, 12]],
    };
    let interstage = VertexAttributeInterstage {
        dispatch: 17,
        writer: VertexAttributeWriterTarget::Copy3,
        descriptors: vec![VertexAttributeSourceDescriptor {
            element_shift: 0,
            group_stride: 3,
            count: 3,
        }],
        sources: vec![VertexAttributeSource {
            selector: 3,
            bytes: hex_bytes("010203040506070809"),
        }],
    };
    let mut out = vec![0xee; 39];

    let usage = vertex_attribute_apply_writer(
        &mut out,
        VertexAttributeWriterCall {
            transform: &transform,
            interstage: &interstage,
            matches: &[],
            aux_table: &[],
            block_index: 0,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        VertexAttributeWriterUsage {
            sources: [9, 0, 0, 0, 0],
            match_entries: 0,
            aux_entries: 0,
        }
    );
    assert_eq!(&out[0..3], &hex_bytes("010203"));
    assert_eq!(&out[12..15], &hex_bytes("040506"));
    assert_eq!(&out[24..27], &hex_bytes("070809"));
    assert_eq!(&out[36..39], &hex_bytes("070809"));
    assert_eq!(out[3], 0xee, "writer must keep the table stride");
}

/// Writer-table dispatch (`0x10f93d8`) into `0x10fc870`.
///
/// Provenance: `phase1_direction_zero_writer_loop_capture.json` from
/// `capture_vertex_writer_loop.py`, Animal_Shell_B.Shell_B row 0. Dispatch 19
/// maps through setup `0x10fc4b0` to the DirectionZero-only six-byte fixed-copy
/// writer. The compact golden keeps the first three records, including the
/// zero-literal `(0,1,144)` record; `verify_transform_tail_10fc870.py` replays
/// the full observed population 1/1.
#[test]
fn vertex_attribute_writer_dispatch_shell_copy6() {
    let transform = VertexAttributeTransform {
        index: 0,
        table_entry: ByteGroupTransformTableEntry { raw: 0x0600_100b },
        out_offset: 0,
        column: 0,
        limit: 150,
        ret: 25,
        records: hex_width_records("120001001e000000040001007e0000000000010090000000"),
    };
    let interstage = VertexAttributeInterstage {
        dispatch: 19,
        writer: VertexAttributeWriterTarget::Copy6,
        descriptors: vec![VertexAttributeSourceDescriptor {
            element_shift: 0,
            group_stride: 6,
            count: 25,
        }],
        sources: vec![VertexAttributeSource {
            selector: 3,
            bytes: hex_bytes(concat!(
                "efa5af290923eda90029462b33ab872889a47daeb5245c27ccb0b221f52f78ac43264c3046b0228bb93342ab6223ed32",
                "68a465241e34f6aa361cfc34722ac51fbd354f2a8427a6349fa06633a4b2bd25a43449b4f829323377b242a57e31cdb0",
                "e328c230a1b058a8a32ea4adea2b1334cbb15b28022dafae692bca3126b1efa451236ba6",
            )),
        }],
    };
    let mut out = vec![0; 150];

    let usage = vertex_attribute_apply_writer(
        &mut out,
        VertexAttributeWriterCall {
            transform: &transform,
            interstage: &interstage,
            matches: &[],
            aux_table: &[],
            block_index: 0,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        VertexAttributeWriterUsage {
            sources: [132, 0, 0, 0, 0],
            match_entries: 0,
            aux_entries: 0,
        }
    );
    assert_eq!(
        out,
        hex_bytes(concat!(
            "efa5af290923eda90029462b33ab872889a47daeb5245c27ccb0b221f52f78ac43264c3046b0228bb93342ab6223ed32",
            "68a465241e34f6aa361cfc34722ac51fbd354f2a8427a6349fa06633a4b2bd25a43449b4f829323377b242a57e31cdb0",
            "e328c230a1b058a8a32ea4adbd25a43449b4ea2b1334cbb15b28022dafae692bca3126b1efa451236ba633ab872889a4",
            "efa5af290923",
        ))
    );
}

/// CP5d pre-state-4 kernel/control-bit transition.
///
/// Provenance: `capture_kernel_state_machine.py`, Animal_Bear first
/// `0x10f90d4 -> 0x11104d0` path. `verify_kernel_state_machine.py` replays
/// Bear/Bass/Dragonfly 3/3 from payload bytes and proves Bear's control
/// sequence is `0,0,0,1,1,1`: decision bit, zstd-window flag, unary `01`,
/// raw-window flag, continuation unary `1`. This rules out the tempting
/// "skip six bits" shortcut because the forward cursor must also parse the
/// `0x10f983c/0x10f9870/0x10f98d8/0x10f9918` continuation values.
#[test]
fn vertex_kernel_state4_entry_bear_control_bits() {
    let payload = sparse_payload(
        32817,
        &[
            (15, "893a96"),
            (1227, "48"),
            (1300, "01018c7e9104"),
            (32805, "5555f9abcff355b57bdbf7aa"),
        ],
    );
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 32807,
            acc: 0x1d0c_736b_6abd_f000,
            bitpos: 50,
        },
        15,
    );

    let entry = vertex_kernel_state4_entry(&payload, &mut state, 2, u32::MAX).unwrap();

    assert_eq!(entry.bits, vec![0, 0, 0, 1, 1, 1]);
    assert_eq!(
        entry.code_window,
        VertexKernelWindow {
            flag: 0,
            src_start: 17,
            src_size: 1210,
            next_stream_pos: 1227,
        }
    );
    assert_eq!(
        entry.data_window,
        VertexKernelWindow {
            flag: 1,
            src_start: 1228,
            src_size: 72,
            next_stream_pos: 1300,
        }
    );
    assert_eq!(
        entry.continuation,
        Some(VertexKernelContinuation {
            mode: 1,
            kind: 0,
            repeat: 1,
            count: 1662,
            current: 2180,
        })
    );
    assert_eq!(
        (entry.reader, entry.stream_pos),
        (
            RansThreeLaneReader {
                ptr: 32805,
                acc: 0x431c_dada_af7d_b7ba,
                bitpos: 60,
            },
            1306,
        )
    );
    assert_eq!(state.reader, entry.reader);
    assert_eq!(state.stream_pos, entry.stream_pos);

    let mut skip_only_state = byte_group_state(
        RansThreeLaneReader {
            ptr: 32807,
            acc: 0x1d0c_736b_6abd_f000,
            bitpos: 50,
        },
        1306,
    );
    assert_ne!(skip_only_state.reader, entry.reader);
    skip_only_state.stream_pos = entry.stream_pos;
    assert_ne!(skip_only_state.reader, entry.reader);
}

/// P1-HEAD-1 state-3 index leaf before the first state-4 entry.
///
/// Provenance: `capture_phase1_decision_bit.py`, `Animal_Bee.Bee.bfres.mc`.
/// The trace reaches `0x10f9690 -> 0x10fa980` once from
/// `0x10f8b60..0x10f8b94` before the `0x10f90d4` decision site. The first leaf
/// decodes the zstd code window at `P+14` (regenerated size 203), consumes unary
/// `1`, then `0x10f8c58..0x10f8c1c` consumes a raw index-data window from
/// `P+122..P+706`. The next state-0 table lands exactly on the captured
/// decision reader, and the carried code quota means the state-4 leaf starts at
/// the unary read instead of refilling the code window. This rules out both the
/// old "read a decision bit immediately after the first table" cut and the
/// tempting "always refill the code window at state 4" cut.
#[test]
fn vertex_kernel_state4_entry_bee_skips_index_leaf() {
    let payload = sparse_payload(
        2458,
        &[
            (
                13,
                concat!(
                    "6a9206141ac0251d9bfe7fd56ffa4d7f9b56dd94c010401a310d48f2965b06bd950c",
                    "ae95c2f5d672a06b54f3e211818c832e6eb8fff87a0f7a0b90226a5863f26c806b",
                    "f33227367dc445823a8aa815c6f9ede10509005917125208829c39a67480022ce",
                    "3563680bd110e0b",
                ),
            ),
            (120, "844814"),
            (706, "010101836900814908000c0002701481"),
            (2427, "f729ad91824c13102dc09a6c422f42130e72a8"),
        ],
    );
    let first_header = SubBlockHeader {
        count: 1,
        a: 1,
        b: 0,
        c: 1,
        d: 120,
        e: 0,
        f: 120,
    };
    let first_table = TableBuild {
        fwd: 13,
        rev_ptr: 2438,
        rev_acc: 0x8cbd_83e4_965e_a000,
        rev_bitpos: 46,
        w8: 56,
        symbols: 4,
        branch_bit: 1,
        dir_bit: 0,
        entries: Vec::new(),
        offsets: Vec::new(),
        cols: Vec::new(),
        longs: Vec::new(),
        byte_group_total: 0,
        max_prod: 0,
    };
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 2438,
            acc: 0x465e_c1f2_4b2f_5000,
            bitpos: 47,
        },
        13,
    );

    let entry = vertex_kernel_state4_entry_from_table(
        &payload,
        &mut state,
        VertexKernelState4EntrySpec {
            first_header,
            first_table: &first_table,
            remaining_subblocks: 11,
            reverse_mode: u32::MAX,
        },
    )
    .unwrap();

    assert_eq!(entry.bits, vec![0, 1, 0, 0, 0, 1, 1]);
    assert_eq!(entry.skipped_index_leaves.len(), 1);
    assert_eq!(
        entry.skipped_index_leaves[0].code_window,
        VertexKernelWindow {
            flag: 0,
            src_start: 14,
            src_size: 106,
            next_stream_pos: 120,
        }
    );
    assert_eq!(entry.skipped_index_leaves[0].code_regenerated_size, 203);
    assert_eq!(entry.skipped_index_leaves[0].unary, 0);
    assert_eq!(
        entry.skipped_index_leaves[0].data_window,
        VertexKernelWindow {
            flag: 0,
            src_start: 122,
            src_size: 584,
            next_stream_pos: 706,
        }
    );
    assert_eq!(
        entry.data_window,
        VertexKernelWindow {
            flag: 1,
            src_start: 719,
            src_size: 2,
            next_stream_pos: 721,
        }
    );
    assert_eq!(
        (entry.reader, entry.stream_pos),
        (
            RansThreeLaneReader {
                ptr: 2427,
                acc: 0x422f_426c_9ac0_2d10,
                bitpos: 56,
            },
            721,
        )
    );
    assert_eq!(state.reader, entry.reader);
    assert_eq!(state.stream_pos, entry.stream_pos);
}

/// P1-HEAD-1 branch-bit/direction-bit split at `0x10f900c..0x10f9028`.
///
/// Provenance: `capture_phase1_index_submesh_count2.py`,
/// `AsbObj_AssassinParts.AsbObj_Assassin_BaloonkeyPlate_01.bfres.mc`. The
/// first state-0 table is `branch_bit=0,direction=0`: the disassembly consumes
/// the direction bit at `0x10f9010..0x10f9024`, then reaches the `0x10f90d4`
/// decision site. This rules out the old shortcut that treated any direction-0
/// table as the Bee-shaped state-3 skip branch.
#[test]
fn vertex_kernel_state4_entry_count2_direction_zero_uses_direct_branch() {
    let payload = sparse_payload(8870, &[(15, "814d"), (8858, "3719762b25536d228a68872e")]);
    let first_header = SubBlockHeader {
        count: 2,
        a: 1,
        b: 0,
        c: 1,
        d: 3456,
        e: 0,
        f: 3456,
    };
    let first_table = TableBuild {
        fwd: 15,
        rev_ptr: 8860,
        rev_acc: 0x3fe8_88fa_2e87_6000,
        rev_bitpos: 48,
        w8: 2330,
        symbols: 6,
        branch_bit: 0,
        dir_bit: 0,
        entries: Vec::new(),
        offsets: Vec::new(),
        cols: Vec::new(),
        longs: Vec::new(),
        byte_group_total: 0,
        max_prod: 0,
    };
    let spec = VertexKernelState4EntrySpec {
        first_header,
        first_table: &first_table,
        remaining_subblocks: 19,
        reverse_mode: u32::MAX,
    };

    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 0,
            bitpos: 0,
        },
        0,
    );
    assert_eq!(
        vertex_kernel_state4_entry_from_table(&payload, &mut state, spec),
        Err(VertexKernelStateError::UnobservedContinuationModeKind { mode: 0, kind: 0 })
    );

    let mut old_cut_table = first_table.clone();
    old_cut_table.branch_bit = 1;
    let mut old_cut_state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 0,
            bitpos: 0,
        },
        0,
    );
    assert_eq!(
        vertex_kernel_state4_entry_from_table(
            &payload,
            &mut old_cut_state,
            VertexKernelState4EntrySpec {
                first_header,
                first_table: &old_cut_table,
                remaining_subblocks: 19,
                reverse_mode: u32::MAX,
            },
        ),
        Err(VertexKernelStateError::UnobservedIndexSubmeshCount(2))
    );
}

/// P1-HEAD-1 direct `DecisionBit(1)` path at `0x10f90d4..0x10f9128`.
///
/// Provenance: `capture_phase1_decision_bit1.py`,
/// `Armor_005.Armor_005_Head.bfres.mc`. The decision bit is 1, so
/// `0x10f9114..0x10f9124` allocates scratch for the first
/// `0x10f9690 -> 0x10fa980` call. The scratch pointer is stored at
/// `ctx+0x220` and later freed; the reader/cursor transition into
/// `0x11104d0` still follows the same code-window, unary, and data-window
/// sequence as the decision-0 fixtures.
#[test]
fn vertex_kernel_state4_entry_decision_bit1_direct_scratch_cursor() {
    let payload = sparse_payload(
        17522,
        &[(15, "883e"), (1103, "4e"), (17512, "bca0e3fff09f01f0c73f")],
    );
    let first_header = SubBlockHeader {
        count: 1,
        a: 1,
        b: 0,
        c: 1,
        d: 4935,
        e: 0,
        f: 4935,
    };
    let first_table = TableBuild {
        fwd: 15,
        rev_ptr: 17514,
        rev_acc: 0x9a18_e037_816b_c000,
        rev_bitpos: 50,
        w8: 1532,
        symbols: 8,
        branch_bit: 0,
        dir_bit: 1,
        entries: Vec::new(),
        offsets: Vec::new(),
        cols: Vec::new(),
        longs: Vec::new(),
        byte_group_total: 0,
        max_prod: 0,
    };
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 0,
            bitpos: 0,
        },
        0,
    );

    let entry = vertex_kernel_state4_entry_from_table(
        &payload,
        &mut state,
        VertexKernelState4EntrySpec {
            first_header,
            first_table: &first_table,
            remaining_subblocks: 23,
            reverse_mode: u32::MAX,
        },
    )
    .unwrap();

    assert_eq!(entry.bits, vec![1, 0, 0, 1, 1]);
    assert_eq!(
        entry.code_window,
        VertexKernelWindow {
            flag: 0,
            src_start: 17,
            src_size: 1086,
            next_stream_pos: 1103,
        }
    );
    assert_eq!(
        entry.data_window,
        VertexKernelWindow {
            flag: 1,
            src_start: 1104,
            src_size: 78,
            next_stream_pos: 1182,
        }
    );
    assert_eq!(
        (entry.reader, entry.stream_pos),
        (
            RansThreeLaneReader {
                ptr: 17512,
                acc: 0x431c_06f0_2d79_fe3e,
                bitpos: 61,
            },
            1182,
        )
    );
    assert_eq!(state.reader, entry.reader);
    assert_eq!(state.stream_pos, entry.stream_pos);
}

/// P1-HEAD-2 direct data-window flag 0 at `0x10fae60 -> 0x10fab08`.
///
/// Provenance: `capture_phase1_data_window_flag0.py`,
/// `Animal_Crab.Crab_Boneless.bfres.mc`. The first direct state-4 leaf calls
/// the helper twice: code flag 0 decodes `P+17..P+377`, unary `01` follows,
/// then data flag 0 decodes `P+378..P+426` before the count-2 continuation
/// header at `P+426`. This rules out the old "direct data is always raw flag
/// 1" guard and a wrong skip-only implementation that would accept malformed
/// zstd data without validating the helper body.
#[test]
fn vertex_kernel_state4_entry_data_flag0_zstd_window() {
    let payload = sparse_payload(
        9329,
        &[
            (
                15,
                concat!(
                    "82683619482ca0254907c020929c024a50217070020b0c204000636cc23efccf8c0fffff2da7dadfdade51c612d208d9",
                    "3b053c003b0040005e022cb041ffa4440f82948a3369a4ac172f57094bff780a0870165a86d252b65f4e4e43b04448fb",
                    "9cda8a8a69ca4473a172cc6ebf6a47c478656b0c2c0f1441a15c96b5e7d0db1111b7387ca5e34413537fd57065677934",
                    "14eae4893848c50096de8df6ca25b27f356a6892723565cfc8dd8acb2b6942099aa3b48cc17e3f324c4a542e6b448d6a",
                    "90325cb062797148231eff54d31e9515e88b321c0e29404fef50dc32a3d02238e4502f071070221ee6d0876a8fc30345",
                    "a38bf5d158645816534bd76ee54aecebd10f6912e73131ea44952b73436ea44e4ebdd2e8315e89679c8481e6f1cf4705",
                    "7a5c7af4201c201048476ed40331c5b715eeb70b8d8339af9b5847081507cfe4e2f0b06437d0711029c379040b86c33b",
                    "f6dcc0a0964e3eace2d809afe7137c853221c127000d632cc504303402020302030506fad2020507f810891e04020745",
                    "030205050604060302050303020202030400ee80848aa10b1c00840101833c8449",
                ),
            ),
            (9319, "cbd2d8201676e26fc171"),
        ],
    );
    let first_header = SubBlockHeader {
        count: 2,
        a: 1,
        b: 0,
        c: 1,
        d: 1794,
        e: 0,
        f: 1794,
    };
    let first_table = TableBuild {
        fwd: 15,
        rev_ptr: 9321,
        rev_acc: 0x1500_0264_2fcd_7000,
        rev_bitpos: 48,
        w8: 971,
        symbols: 4,
        branch_bit: 0,
        dir_bit: 1,
        entries: Vec::new(),
        offsets: Vec::new(),
        cols: Vec::new(),
        longs: Vec::new(),
        byte_group_total: 0,
        max_prod: 0,
    };
    let spec = VertexKernelState4EntrySpec {
        first_header,
        first_table: &first_table,
        remaining_subblocks: 8,
        reverse_mode: u32::MAX,
    };
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 0,
            bitpos: 0,
        },
        0,
    );

    let entry = vertex_kernel_state4_entry_from_table(&payload, &mut state, spec).unwrap();

    assert_eq!(entry.bits, vec![0, 0, 0, 1, 0, 1]);
    assert_eq!(
        entry.code_window,
        VertexKernelWindow {
            flag: 0,
            src_start: 17,
            src_size: 360,
            next_stream_pos: 377,
        }
    );
    assert_eq!(
        entry.data_window,
        VertexKernelWindow {
            flag: 0,
            src_start: 378,
            src_size: 48,
            next_stream_pos: 426,
        }
    );
    assert_eq!(
        entry.continuation,
        Some(VertexKernelContinuation {
            mode: 1,
            kind: 0,
            repeat: 1,
            count: 444,
            current: 585,
        })
    );
    assert_eq!(
        (entry.reader, entry.stream_pos),
        (
            RansThreeLaneReader {
                ptr: 9319,
                acc: 0x4000_990b_f35c_705a,
                bitpos: 58,
            },
            432,
        )
    );
    assert_eq!(state.reader, entry.reader);
    assert_eq!(state.stream_pos, entry.stream_pos);

    let mut malformed = payload;
    malformed[377] = 0;
    let mut malformed_state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 0,
            bitpos: 0,
        },
        0,
    );
    assert_eq!(
        vertex_kernel_state4_entry_from_table(
            &malformed,
            &mut malformed_state,
            VertexKernelState4EntrySpec {
                first_header,
                first_table: &first_table,
                remaining_subblocks: 8,
                reverse_mode: u32::MAX,
            },
        ),
        Err(VertexKernelStateError::WindowDecodeFailed("data"))
    );
}

/// P1-HEAD-4 direct first-leaf unary 0 with one submesh.
///
/// Provenance: `capture_phase1_first_unary0.py`,
/// `Animal_Fairy.Fairy.bfres.mc`. The first `0x10fa980` leaf decodes only the
/// zstd code helper, then unary code `1` means no data helper is consumed before
/// `0x11104d0`. This rules out treating unary 0 as an unobserved error for
/// count-1 direct entries.
#[test]
fn vertex_kernel_state4_entry_unary0_count1_has_no_data_window() {
    let payload = sparse_payload(
        337,
        &[
            (13, "1858f0001010f02000f000000004003cb3883963258c063704"),
            (327, "00ad6bd3ae0f30b2394d"),
        ],
    );
    let first_header = SubBlockHeader {
        count: 1,
        a: 1,
        b: 0,
        c: 1,
        d: 102,
        e: 0,
        f: 102,
    };
    let first_table = TableBuild {
        fwd: 13,
        rev_ptr: 329,
        rev_acc: 0x2801_d000_a6a4_4000,
        rev_bitpos: 48,
        w8: 42,
        symbols: 4,
        branch_bit: 0,
        dir_bit: 1,
        entries: Vec::new(),
        offsets: Vec::new(),
        cols: Vec::new(),
        longs: Vec::new(),
        byte_group_total: 0,
        max_prod: 0,
    };
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 0,
            bitpos: 0,
        },
        0,
    );

    let entry = vertex_kernel_state4_entry_from_table(
        &payload,
        &mut state,
        VertexKernelState4EntrySpec {
            first_header,
            first_table: &first_table,
            remaining_subblocks: 3,
            reverse_mode: u32::MAX,
        },
    )
    .unwrap();

    assert_eq!(entry.bits, vec![0, 0, 1]);
    assert_eq!(
        entry.code_window,
        VertexKernelWindow {
            flag: 0,
            src_start: 14,
            src_size: 24,
            next_stream_pos: 38,
        }
    );
    assert_eq!(
        entry.data_window,
        VertexKernelWindow {
            flag: 0,
            src_start: 38,
            src_size: 0,
            next_stream_pos: 38,
        }
    );
    assert_eq!(entry.continuation, None);
    assert_eq!(
        (entry.reader, entry.stream_pos),
        (
            RansThreeLaneReader {
                ptr: 327,
                acc: 0x400e_8005_3522_69cc,
                bitpos: 61,
            },
            38,
        )
    );
}

/// P1-HEAD-4 direct first-leaf unary 0 with count-2 continuation data.
///
/// Provenance: `capture_phase1_first_unary0.py`,
/// `Animal_Beetle.Beetle.bfres.mc`. The first leaf consumes no data helper;
/// `0x10f983c..0x10f9918` parses the second leaf header, continuation unary
/// `01` requests the raw data helper, and only then does the path reach
/// `0x11104d0`.
#[test]
fn vertex_kernel_state4_entry_unary0_count2_data_moves_to_continuation() {
    let payload = sparse_payload(
        7312,
        &[
            (
                15,
                concat!(
                    "816f06102c2590354907bbf5aaeaff0735f6dcf39e7dd0d67fe803c4622ac7f7feb6b5fdb0dedf92b6770a1f00230023",
                    "00ecb77e7da95d1e46b0022ed16be0c971dcb91fb70a00065060ac950ee3f73eb51f27c60011f6d1220c28612dcda5e5",
                    "4b2b613045bbb09305832cb1d8e3e28fbd9e01c92261235524a3cddae31aa1d40ac7d47ad22271c9d4dc1e1c37a1a044",
                    "973a6a2c94d87c7beeebb78f5da3eff99d6fbc018eaaa0227230dac408842ba2db9b744c5a1900480d00c1802a506e5d",
                    "e182cb39021848099333ee73264307680916dc04b7998a97a92cc3561c904393c162682e76e6ca5339c822e250b73538",
                    "23010181468202020210",
                ),
            ),
            (7302, "400b3a4b6a37220030f1"),
        ],
    );
    let first_header = SubBlockHeader {
        count: 2,
        a: 1,
        b: 0,
        c: 1,
        d: 840,
        e: 0,
        f: 840,
    };
    let first_table = TableBuild {
        fwd: 15,
        rev_ptr: 7304,
        rev_acc: 0x2d0c_7e28_9ebf_f000,
        rev_bitpos: 50,
        w8: 417,
        symbols: 4,
        branch_bit: 0,
        dir_bit: 1,
        entries: Vec::new(),
        offsets: Vec::new(),
        cols: Vec::new(),
        longs: Vec::new(),
        byte_group_total: 0,
        max_prod: 0,
    };
    let spec = VertexKernelState4EntrySpec {
        first_header,
        first_table: &first_table,
        remaining_subblocks: 7,
        reverse_mode: u32::MAX,
    };
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 0,
            bitpos: 0,
        },
        0,
    );

    let entry = vertex_kernel_state4_entry_from_table(&payload, &mut state, spec).unwrap();

    assert_eq!(entry.bits, vec![0, 0, 1, 0, 1, 1]);
    assert_eq!(
        entry.code_window,
        VertexKernelWindow {
            flag: 0,
            src_start: 17,
            src_size: 239,
            next_stream_pos: 256,
        }
    );
    assert_eq!(
        entry.data_window,
        VertexKernelWindow {
            flag: 1,
            src_start: 263,
            src_size: 2,
            next_stream_pos: 265,
        }
    );
    assert_eq!(
        entry.continuation,
        Some(VertexKernelContinuation {
            mode: 1,
            kind: 0,
            repeat: 1,
            count: 198,
            current: 258,
        })
    );
    assert_eq!(
        (entry.reader, entry.stream_pos),
        (
            RansThreeLaneReader {
                ptr: 7302,
                acc: 0x431f_8a27_afff_1300,
                bitpos: 60,
            },
            265,
        )
    );

    let mut malformed = payload;
    malformed[262] = 0xff;
    let mut malformed_state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 0,
            bitpos: 0,
        },
        0,
    );
    assert_eq!(
        vertex_kernel_state4_entry_from_table(
            &malformed,
            &mut malformed_state,
            VertexKernelState4EntrySpec {
                first_header,
                first_table: &first_table,
                remaining_subblocks: 7,
                reverse_mode: u32::MAX,
            },
        ),
        Err(VertexKernelStateError::StreamTooShort)
    );
}

/// Direction-zero branch-clear tables run the first kernel leaf but do not
/// immediately enter `0x11104d0`.
///
/// Provenance: `capture_phase1_decision_bit1.py`,
/// `Armor_009.Armor_009_Head.bfres.mc`. The first leaf is otherwise shaped like
/// the direct state4 fixtures, but `ctx+0x118 == 0` sends the disassembly from
/// `0x10f9180..0x10f9184` to the non-state4 path at `0x10f9220`.
#[test]
fn vertex_kernel_state4_entry_direction_zero_direct_path_is_guarded() {
    let payload = sparse_payload(
        23846,
        &[(15, "895d"), (1262, "50"), (23836, "8000bdade870dbbaa87c")],
    );
    let first_header = SubBlockHeader {
        count: 1,
        a: 1,
        b: 0,
        c: 1,
        d: 1767,
        e: 0,
        f: 1767,
    };
    let first_table = TableBuild {
        fwd: 15,
        rev_ptr: 23838,
        rev_acc: 0x9bb2_cb50_254c_7000,
        rev_bitpos: 48,
        w8: 584,
        symbols: 6,
        branch_bit: 0,
        dir_bit: 0,
        entries: Vec::new(),
        offsets: Vec::new(),
        cols: Vec::new(),
        longs: Vec::new(),
        byte_group_total: 0,
        max_prod: 0,
    };
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 0,
            bitpos: 0,
        },
        0,
    );

    assert_eq!(
        vertex_kernel_state4_entry_from_table(
            &payload,
            &mut state,
            VertexKernelState4EntrySpec {
                first_header,
                first_table: &first_table,
                remaining_subblocks: 42,
                reverse_mode: u32::MAX,
            },
        ),
        Err(VertexKernelStateError::UnobservedDirectionZeroDirectPath)
    );
}

#[test]
fn vertex_kernel_state4_entry_rejects_malformed_inputs() {
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 1 << 63,
            bitpos: 63,
        },
        0,
    );
    assert_eq!(
        vertex_kernel_state4_entry(&[0; 8], &mut state, 1, u32::MAX),
        Err(VertexKernelStateError::PayloadTooSmall)
    );

    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 32807,
            acc: 0x1d0c_736b_6abd_f000,
            bitpos: 50,
        },
        15,
    );
    assert_eq!(
        vertex_kernel_state4_entry(&[0; 8], &mut state, 1, u32::MAX),
        Err(VertexKernelStateError::PayloadTooSmall)
    );

    let payload = sparse_payload(
        32817,
        &[
            (15, "893a96"),
            (1227, "48"),
            (1300, "22018c7e9104"),
            (32805, "5555f9abcff355b57bdbf7aa"),
        ],
    );
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 32807,
            acc: 0x1d0c_736b_6abd_f000,
            bitpos: 50,
        },
        15,
    );
    assert_eq!(
        vertex_kernel_state4_entry(&payload, &mut state, 2, u32::MAX),
        Err(VertexKernelStateError::UnobservedContinuationModeKind { mode: 2, kind: 2 })
    );

    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 32807,
            acc: 0x1d0c_736b_6abd_f000,
            bitpos: 50,
        },
        15,
    );
    assert_eq!(
        vertex_kernel_state4_entry(&payload, &mut state, 3, u32::MAX),
        Err(VertexKernelStateError::UnobservedSubmeshCount(3))
    );
}

/// Vertex match-table builder (`0x11106d0`).
///
/// Provenance: `capture_vertex_match_table.py`, Animal_Dragonfly. This is
/// the state-4 builder that materializes `ctx+0x228` before the state-5
/// writer loop; `verify_vertex_match_table.py` replays the whole observed
/// Bear/Bass/Dragonfly population 3/3. The golden covers sparse output
/// deltas, ring wraps that consume stream 0, extended-byte bit reads from
/// stream 3, and zero-filled gaps. It rules out a capture-seeded writer loop
/// that imports `match_hex` instead of deriving it from the four setup
/// streams.
#[test]
fn vertex_match_table_dragonfly_from_setup_streams() {
    let stream0 = hex_bytes(concat!(
            "0000000100000001000100000100000001000100010000000100000000000001000100010000000100010001000000010000",
            "0001000100010001000100010001000100010000000100010000010001010001000101000100010100010001010001000101",
            "0101010001000101010101000100010101000100010101000100010001000100010001010000",
        ));
    let stream1 = hex_bytes(concat!(
            "0101010001010100010001010001010101010100010001010001010101010101000101010101010101010101010101010101",
            "0101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010100",
            "0100010001010101010101000100010001010100010101010101010101000101010101010100010100010101000101000101",
            "0101010101000101000101010101010101000101010100010101010101010101010101010101010101010101010101010101",
            "0101010101010101010001010101010101010100010101010101010101010101010101010101010101010101010101010101",
            "0101010101010101010101010101010101010101010101010101010101010101010101010101000100010100010001010001",
            "0001010001000101000100010101010101010101010100010001010101010101010101010001000101010101000100010101",
            "0101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101",
            "0001000100010001000101010101010101010101010101010101010101010101010101010101010101010101010101010101",
            "0101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101",
            "0101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101",
            "01010101010601010101",
        ));
    let stream2 = hex_bytes(concat!(
            "000102020202020202020202020205030c01040202020d1002101204080d02020206030e0b10070502141113000c03010204",
            "1302111406030606030808060507021312070004050a040f0010050a0413100b01150413140804070803111209000a140f02",
            "02020202090c0209030e0202020202020202020202130b000815020d1002140e04140a110c02141502110a0c020508020d0d",
            "13110c0c04021617021416151515080215020d0710080212001216150803100308020603061407000a1402040f1818181807",
            "14020b181513041119020b0a151211030c0b1602130f1211001008001618181014031414140c0316110808130c0012070b11",
            "1717171116041717171705181802181817110314041414140319181002031316101602161114020202020202020202020202",
            "020202020202020202020202141514061310060e090c020202020205030c180319180318020202020205030c020202020205",
            "0319060106060106060106060106060113041410041009040804060104070406190419180418060104070408060104070406",
            "020202020202020202180003180218001806181a1a0313050e0d0218171512140317091618181807080c110b190e00160317",
            "1216170407030c18180b14130d16150c01130512080b041516071819181a0e120412110509180118000c0218180218180218",
            "0217041a1a0319151618031816100e1419180115080d0a16150604081a1a0f190f11181a101418120816140318180a05140b",
            "070c10030c1a1112080c",
        ));
    let stream3 = hex_bytes(concat!(
            "624f1323c9685822e90852e271ae9e523c1a38f0a1a1097bb3a4e89e5f33f464104d3082a090006e636580af1e03a91dc26c",
            "09676fcf3a40c3f1e650234c4184963c30d2650000502bf1a80000a4292bab112407",
        ));
    let mut state = VertexMatchTableState {
        base: 0,
        limit: 0,
        mask: 0x3ff,
    };

    let matches = vertex_match_table(VertexMatchTableSpec {
        count: 523,
        processed_vertices: 0,
        counts: [138, 560, 560, 68],
        state: &mut state,
        stream0: &stream0,
        stream1: &stream1,
        stream2: &stream2,
        stream3: &stream3,
    })
    .unwrap();

    assert_eq!(
            matches,
            hex_u32_words(concat!(
                "0000000000000000000000000000000000000000000000000000000000000000000000000000000018000000280000000000",
                "0000200000000000000000000000400000000000000010000000980000009000000080000000000000000000000000000000",
                "c000000000000000b80000000000000040000000500000000000000000000000b8000000780000008800000008000000b000",
                "0000c0000000d800000010000000200000005000000000000000d80000005800000061010000b00000006801000061010000",
                "b80000006901000038010000390100001000000020000000b80000007000000059010000a000000008000000400000004000",
                "00002800000079010000c901000008000000a8000000b800000098000000a80000005801000028000000f800000058010000",
                "980000005000000048000000100000009000000088000000a80000001000000020000000b801000070000000000200000800",
                "0000180000009800000000000000000000000000000018000000000000000000000028000000200000001800000000000000",
                "0000000000000000000000000000000000000000000000006000000050000000080000008800000028000000000000005800",
                "000000000000800000006800000060000000a8000000200000001800000000000000a800000000000000a000000058000000",
                "00000000200000000000000040000000300000002800000060000000200000003000000000000000b8000000000000006800",
                "0000a8000000400000003000000060000000f8000000300000000000000060000000a0000000580000000000000098000000",
                "080000001800000068000000f9010000f101000001020000c1010000d1010000c901000060010000c1010000a0000000a001",
                "0000480000005800000008000000e001000028000000d0000000c10100006000000000000000100000000000000088000000",
                "6000000048000000480000003000000040000000e1010000c800000020000000e00000000000000049010000100000000902",
                "0000180000001000000050000000010200001000000000000000e101000018000000e1010000200000000800000018000000",
                "a1010000080000005001000028000000b000000020010000380000006800000030000000d000000088010000300000001801",
                "0000c8000000c8000000c00000005000000030000000e9020000080000009800000028000000180000007800000020000000",
                "a8000000280000000001000098000000b001000080000000c005000080060000580600000000000008050000180000002805",
                "000018000000280000006003000050030000c80300008000000070000000000200002003000028000000a804000068000000",
                "8805000078060000e00500005805000028060000d8060000380000003000000030000000b0000000b0050000000000000000",
                "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
                "0000000048000000000000005000000048000000000000005000000048000000000000005000000000000000000000000000",
                "000000000000180000002800000000000000f00000000001000000000000f000000000010000000000000000000000000000",
                "0000000018000000280000000000000000000000000000000000000018000000280000008101000079010000890100008101",
                "0000790100008901000081010000790100008901000081010000790100008901000081010000790100008901000048000000",
                "4000000089010000480000004000000089010000480000004000000089010000810100007901000089010000810100002000",
                "00001800000081010000f8000000f000000081010000f8000000f00000008101000079010000890100008101000020000000",
                "180000008101000079010000890100008101000020000000180000000000000000000000000000000000000000000000800a",
                "000008000000200400002100000000000000c00a0000080000004900000020000000d90a0000280400009003000078040000",
                "98030000080a00003009000010000000800900007808000038040000300400007806000020000000b8070000280000007805",
                "000088060000a0040000c8060000100000005000000010000000580000001800000058000000e8070000b106000008000000",
                "7000000008050000890600000008000058000000100000004800000058080000380800001800000070000000280800006808",
                "000040000000600000004008000040080000e007000060090000e80700002806000030080000100600002800000038000000",
                "1800000030000000b0060000e80600002000000018070000280000004802000000000000680200006802000000000000e00a",
                "0000300c00000000000040020000100b00004802000008000000280000000000000098020000180000001800000018000000",
                "a80200008802000088020000200000008002000090020000a0020000000300001003000098000000c00b0000100000005800",
                "000030000000200300002800000098000000e80000005000000080000000b000000028000000c8060000880d0000780e0000",
                "e80c0000f0060000600c0000a80d0000f006000080000000c800000010070000a80000005007000018070000500000000001",
                "0000b802000090020000600200005802000038080000f8090000d0070000d007000018000000580300008808000048030000",
                "10090000b8080000e80800004002000028080000680200001809000080020000000000000000000000000000000000000000",
                "0000d904000071020000f1040000e9040000a10200000000000000000000000000000000000000000000",
            ))
        );
    assert_eq!(
        state,
        VertexMatchTableState {
            base: 134,
            limit: 138,
            mask: 0x3ff,
        }
    );
}

#[test]
fn vertex_match_table_rejects_malformed_inputs() {
    let mut state = VertexMatchTableState {
        base: 0,
        limit: 0,
        mask: 0x3ff,
    };
    assert_eq!(
        vertex_match_table(VertexMatchTableSpec {
            count: 1,
            processed_vertices: 0,
            counts: [0, 1, 1, 0],
            state: &mut state,
            stream0: &[],
            stream1: &[1],
            stream2: &[0],
            stream3: &[],
        }),
        Err(VertexMatchTableError::StreamTooShort { stream: 0 })
    );

    let mut state = VertexMatchTableState {
        base: 0,
        limit: 1,
        mask: 0x3ff,
    };
    assert_eq!(
        vertex_match_table(VertexMatchTableSpec {
            count: 1,
            processed_vertices: 0,
            counts: [0, 1, 1, 0],
            state: &mut state,
            stream0: &[],
            stream1: &[2],
            stream2: &[0],
            stream3: &[],
        }),
        Err(VertexMatchTableError::MatchIndexOutOfBounds { index: 1, count: 1 })
    );

    let mut state = VertexMatchTableState {
        base: 0,
        limit: 1,
        mask: 0x3ff,
    };
    assert_eq!(
        vertex_match_table(VertexMatchTableSpec {
            count: 2,
            processed_vertices: 0,
            counts: [0, 1, 1, 0],
            state: &mut state,
            stream0: &[],
            stream1: &[0x10],
            stream2: &[0],
            stream3: &[0; 7],
        }),
        Err(VertexMatchTableError::BitstreamTooShort)
    );

    let mut state = VertexMatchTableState {
        base: 0,
        limit: 0,
        mask: u32::MAX,
    };
    assert_eq!(
        vertex_match_table(VertexMatchTableSpec {
            count: 1,
            processed_vertices: 0,
            counts: [0, 0, 0, 0],
            state: &mut state,
            stream0: &[],
            stream1: &[],
            stream2: &[],
            stream3: &[],
        }),
        Err(VertexMatchTableError::HistoryTooLarge(u32::MAX))
    );
}

#[test]
fn vertex_attribute_driver_rejects_unobserved_and_malformed_inputs() {
    let mut transform_state = ByteGroupTransformState {
        mode: 2,
        count_bits: 0,
        record_count: 0,
        second_count: 0,
        third_count: 0,
        tail_count: 0,
    };
    let mut byte_state = byte_group_state(zero_three_lane_reader(), 0);
    assert_eq!(
        vertex_attribute_driver_setup(&mut transform_state, &mut byte_state, &[], 0),
        Err(VertexAttributeDriverError::UnobservedTableCount(0))
    );

    let payload = sparse_payload(8, &[(0, "0000000000000000")]);
    let mut byte_state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 1 << 63,
            bitpos: 63,
        },
        0,
    );
    assert_eq!(
        vertex_attribute_driver_setup(&mut transform_state, &mut byte_state, &payload, 1),
        Err(VertexAttributeDriverError::UnobservedSetupMode(0))
    );

    let table = TableBuild {
        fwd: 0,
        rev_ptr: 0,
        rev_acc: 0,
        rev_bitpos: 0,
        w8: 1,
        symbols: 2,
        branch_bit: 0,
        dir_bit: 1,
        entries: vec![0x0a00_0802],
        offsets: vec![0],
        cols: vec![0],
        longs: Vec::new(),
        byte_group_total: 0,
        max_prod: 0,
    };
    let mut state = VertexAttributeDriverState {
        current_attribute: 0,
        processed_vertices: 0,
        vertex_count: 1,
        block_limit: 1,
        transform_state,
        byte_state: byte_group_state(zero_three_lane_reader(), 0),
    };
    assert_eq!(
        vertex_attribute_driver_step(&mut state, &table, &[]),
        Err(VertexAttributeDriverError::TableShapeMismatch {
            symbols: 2,
            entries: 1,
            offsets: 1,
            cols: 1,
        })
    );

    let mut table = table;
    table.symbols = 1;
    let mut state = VertexAttributeDriverState {
        current_attribute: 1,
        processed_vertices: 0,
        vertex_count: 1,
        block_limit: 1,
        transform_state,
        byte_state: byte_group_state(zero_three_lane_reader(), 0),
    };
    assert_eq!(
        vertex_attribute_driver_step(&mut state, &table, &[]),
        Err(VertexAttributeDriverError::NoAttributesRemaining {
            current: 1,
            total: 1,
        })
    );

    let mut state = VertexAttributeDriverState {
        current_attribute: 2,
        processed_vertices: 0,
        vertex_count: 1,
        block_limit: 1,
        transform_state,
        byte_state: byte_group_state(zero_three_lane_reader(), 0),
    };
    assert_eq!(
        vertex_attribute_driver_step(&mut state, &table, &[]),
        Err(VertexAttributeDriverError::CurrentAttributeOutOfRange {
            current: 2,
            total: 1,
        })
    );

    let mut state = VertexAttributeDriverState {
        current_attribute: 0,
        processed_vertices: 1,
        vertex_count: 2,
        block_limit: 2,
        transform_state,
        byte_state: byte_group_state(zero_three_lane_reader(), 0),
    };
    assert_eq!(
        vertex_attribute_driver_step(&mut state, &table, &[]),
        Err(VertexAttributeDriverError::UnobservedNonzeroProcessedVertices(1))
    );

    let mut state = VertexAttributeDriverState {
        current_attribute: 0,
        processed_vertices: 0,
        vertex_count: 2,
        block_limit: 1,
        transform_state,
        byte_state: byte_group_state(zero_three_lane_reader(), 0),
    };
    assert_eq!(
        vertex_attribute_driver_step(&mut state, &table, &[]),
        Err(VertexAttributeDriverError::UnobservedPartialVertexBlock {
            remaining: 2,
            block_limit: 1,
        })
    );
}

#[test]
fn vertex_attribute_interstage_rejects_unobserved_and_malformed_inputs() {
    let valid_entry = ByteGroupTransformTableEntry { raw: 0x0a00_0802 };
    let mut state = byte_group_state(zero_three_lane_reader(), 0);
    assert_eq!(
        vertex_attribute_interstage_sources(&mut state, &[], valid_entry, 1),
        Err(VertexAttributeInterstageError::PayloadTooSmall)
    );

    let payload = sparse_payload(8, &[(0, "0000000000000000")]);
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 0,
            bitpos: 57,
        },
        0,
    );
    assert_eq!(
        vertex_attribute_interstage_sources(&mut state, &payload, valid_entry, 1),
        Err(VertexAttributeInterstageError::UnobservedDispatch(0))
    );

    let payload = sparse_payload(4470, &[(4454, "008bc90efc07781b4404f866efff43fe")]);
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 4454,
            acc: 2449949072564560396,
            bitpos: 57,
        },
        1177,
    );
    assert_eq!(
        vertex_attribute_interstage_sources(
            &mut state,
            &payload,
            ByteGroupTransformTableEntry { raw: 0x0a00_0800 },
            3,
        ),
        Err(VertexAttributeInterstageError::UnobservedZeroTableGroupWidth)
    );

    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 4454,
            acc: 2449949072564560396,
            bitpos: 57,
        },
        1177,
    );
    assert_eq!(
        vertex_attribute_interstage_sources(&mut state, &payload, valid_entry, 0),
        Err(VertexAttributeInterstageError::UnobservedZeroSourceCount {
            dispatch: 16,
            index: 0,
        })
    );

    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 4454,
            acc: 2449949072564560396,
            bitpos: 57,
        },
        5000,
    );
    assert_eq!(
        vertex_attribute_interstage_sources(&mut state, &payload, valid_entry, 3),
        Err(VertexAttributeInterstageError::ByteGroupRead {
            index: 0,
            error: ByteGroupReadError::StreamTooShort,
        })
    );

    let payload = vec![0x80; 8];
    let mut state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 30u64 << 57,
            bitpos: 57,
        },
        0,
    );
    assert_eq!(
        vertex_attribute_interstage_sources(&mut state, &payload, valid_entry, 3),
        Err(VertexAttributeInterstageError::VarintTooLong)
    );
}

#[test]
fn vertex_attribute_writer_dispatch_rejects_malformed_inputs() {
    let transform = VertexAttributeTransform {
        index: 0,
        table_entry: ByteGroupTransformTableEntry { raw: 0x0a00_0802 },
        out_offset: 0,
        column: 0,
        limit: 1,
        ret: 1,
        records: vec![[1, 0]],
    };
    let mut out = vec![0u8; 16];
    let missing_source = VertexAttributeInterstage {
        dispatch: 16,
        writer: VertexAttributeWriterTarget::Copy2,
        descriptors: Vec::new(),
        sources: Vec::new(),
    };
    assert_eq!(
        vertex_attribute_apply_writer(
            &mut out,
            VertexAttributeWriterCall {
                transform: &transform,
                interstage: &missing_source,
                matches: &[],
                aux_table: &[],
                block_index: 0,
            },
        ),
        Err(VertexAttributeWriterError::MissingSource {
            target: VertexAttributeWriterTarget::Copy2,
            index: 0,
        })
    );

    let missing_second_source = VertexAttributeInterstage {
        dispatch: 76,
        writer: VertexAttributeWriterTarget::U8x2Delta,
        descriptors: Vec::new(),
        sources: vec![VertexAttributeSource {
            selector: 3,
            bytes: vec![0, 0],
        }],
    };
    assert_eq!(
        vertex_attribute_apply_writer(
            &mut out,
            VertexAttributeWriterCall {
                transform: &transform,
                interstage: &missing_second_source,
                matches: &[],
                aux_table: &[],
                block_index: 0,
            },
        ),
        Err(VertexAttributeWriterError::MissingSource {
            target: VertexAttributeWriterTarget::U8x2Delta,
            index: 1,
        })
    );

    let delta = VertexAttributeInterstage {
        dispatch: 30,
        writer: VertexAttributeWriterTarget::Delta2Direct,
        descriptors: Vec::new(),
        sources: vec![
            VertexAttributeSource {
                selector: 3,
                bytes: vec![1, 2],
            },
            VertexAttributeSource {
                selector: 3,
                bytes: vec![3, 4],
            },
        ],
    };
    assert_eq!(
        vertex_attribute_apply_writer(
            &mut out,
            VertexAttributeWriterCall {
                transform: &transform,
                interstage: &delta,
                matches: &[],
                aux_table: &[],
                block_index: 0,
            },
        ),
        Err(VertexAttributeWriterError::Delta(
            TransformTailDeltaError::MatchTableTooSmall
        ))
    );

    let table = TableBuild {
        fwd: 0,
        rev_ptr: 0,
        rev_acc: 0,
        rev_bitpos: 0,
        w8: 2,
        symbols: 1,
        branch_bit: 0,
        dir_bit: 1,
        entries: vec![0x0a00_0802],
        offsets: vec![0],
        cols: vec![0],
        longs: Vec::new(),
        byte_group_total: 0,
        max_prod: 0,
    };
    let mut state = VertexAttributeDriverState {
        current_attribute: 0,
        processed_vertices: 0,
        vertex_count: 2,
        block_limit: 2,
        transform_state: ByteGroupTransformState {
            mode: 1,
            count_bits: 0,
            record_count: 0,
            second_count: 0,
            third_count: 0,
            tail_count: 0,
        },
        byte_state: byte_group_state(zero_three_lane_reader(), 0),
    };
    assert_eq!(
        vertex_attribute_writer_loop_step(
            &mut out,
            &mut state,
            &table,
            &[],
            VertexAttributeWriterTable {
                matches: &[0],
                aux_table: &[],
                block_index: 0,
            },
        ),
        Err(VertexAttributeWriterLoopError::MatchTableTooSmall {
            expected: 2,
            actual: 1,
        })
    );
}

#[test]
fn byte_group_transform_rejects_unobserved_and_malformed_inputs() {
    let valid_table = ByteGroupTransformTableEntry { raw: 0x0a00_0802 };
    let mut transform_state = ByteGroupTransformState {
        mode: 0,
        count_bits: 1,
        record_count: 0,
        second_count: 0,
        third_count: 0,
        tail_count: 0,
    };
    let mut byte_state = byte_group_state(zero_three_lane_reader(), 0);
    assert_eq!(
        byte_group_transform(
            &mut transform_state,
            &mut byte_state,
            ByteGroupTransformSpec {
                payload: &[],
                table_entry: valid_table,
                limit: 1,
            },
        ),
        Err(ByteGroupTransformError::UnobservedMode(0))
    );

    let mut transform_state = ByteGroupTransformState {
        mode: 1,
        count_bits: 1,
        record_count: 0,
        second_count: 0,
        third_count: 0,
        tail_count: 0,
    };
    let mut byte_state = byte_group_state(zero_three_lane_reader(), 0);
    assert_eq!(
        byte_group_transform(
            &mut transform_state,
            &mut byte_state,
            ByteGroupTransformSpec {
                payload: &[],
                table_entry: ByteGroupTransformTableEntry { raw: 0 },
                limit: 1,
            },
        ),
        Err(ByteGroupTransformError::UnobservedZeroTableByteCount)
    );

    let payload = sparse_payload(8, &[(0, "8080808080000000")]);
    let mut transform_state = ByteGroupTransformState {
        mode: 1,
        count_bits: 1,
        record_count: 0,
        second_count: 0,
        third_count: 0,
        tail_count: 0,
    };
    let mut byte_state = byte_group_state(zero_three_lane_reader(), 0);
    assert_eq!(
        byte_group_transform(
            &mut transform_state,
            &mut byte_state,
            ByteGroupTransformSpec {
                payload: &payload,
                table_entry: valid_table,
                limit: 1,
            },
        ),
        Err(ByteGroupTransformError::VarintTooLong)
    );

    let payload = sparse_payload(8, &[(0, "0101000000000000")]);
    let mut transform_state = ByteGroupTransformState {
        mode: 1,
        count_bits: 1,
        record_count: 0,
        second_count: 0,
        third_count: 0,
        tail_count: 0,
    };
    let mut byte_state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 0,
            bitpos: 59,
        },
        0,
    );
    assert_eq!(
        byte_group_transform(
            &mut transform_state,
            &mut byte_state,
            ByteGroupTransformSpec {
                payload: &payload,
                table_entry: valid_table,
                limit: 1,
            },
        ),
        Err(ByteGroupTransformError::UnobservedShortActiveCount(1))
    );

    let payload = sparse_payload(8, &[(0, "0200000000000000")]);
    let mut transform_state = ByteGroupTransformState {
        mode: 1,
        count_bits: 1,
        record_count: 0,
        second_count: 0,
        third_count: 0,
        tail_count: 0,
    };
    let mut byte_state = byte_group_state(
        RansThreeLaneReader {
            ptr: 0,
            acc: 0,
            bitpos: 59,
        },
        0,
    );
    assert_eq!(
        byte_group_transform(
            &mut transform_state,
            &mut byte_state,
            ByteGroupTransformSpec {
                payload: &payload,
                table_entry: valid_table,
                limit: 1,
            },
        ),
        Err(ByteGroupTransformError::UnobservedZeroTailBitstream)
    );

    let payload = sparse_payload(48, &[(16, "0201aabbccddeeff1122")]);
    let mut transform_state = ByteGroupTransformState {
        mode: 1,
        count_bits: 1,
        record_count: 0,
        second_count: 0,
        third_count: 0,
        tail_count: 0,
    };
    let mut byte_state = byte_group_state(
        RansThreeLaneReader {
            ptr: 1,
            acc: (0b0_11_11_11_01u64) << (64 - 9),
            bitpos: 59,
        },
        16,
    );
    assert_eq!(
        byte_group_transform(
            &mut transform_state,
            &mut byte_state,
            ByteGroupTransformSpec {
                payload: &payload,
                table_entry: valid_table,
                limit: 32,
            },
        ),
        Err(ByteGroupTransformError::UnobservedTailSelector(1))
    );
}

/// Width combiner (`0x110d360`) with expanded stream bytes, history refs,
/// special third-stream codes, and a non-clamped tail.
///
/// Provenance: `capture_width_combiner.py`, Animal_Bear call 7. This is the
/// compact discriminating call: `count=10,stride=16,shift=0,attr_width=3`,
/// first-stream expansion, seven second-stream expansions, one history
/// reference, nine special third-stream codes, and tail high half `699`.
#[test]
fn width_combiner_bear_expanded_history_tail_limit() {
    let payload = sparse_payload(
        28357,
        &[(
            28328,
            "a5f1856977d98c4c451d3db51f0c79400101013c00140a000c00822d14",
        )],
    );
    let stream0 = hex_bytes("16001700010000000000");
    let stream1 = hex_bytes("0f17191d1c1d1c0f19");
    let stream2 = hex_bytes("08000800090001000b000d000d000b0009000e00");
    let mut reader = RansThreeLaneReader {
        ptr: 28328,
        acc: 0,
        bitpos: 0,
    };
    let mut out = hex_width_records(
        "0700020010000000040002001000000001000200100000000100020010000000\
             0100020010000000010002001000000001000200100000000100020010000000\
             01000200100000000100020010000000",
    );

    let result = width_combiner_into(
        &mut out,
        WidthCombinerSpec {
            count: 10,
            stride: 16,
            shift: 0,
            attr_width: 3,
            limit: 3327,
            payload: &payload,
            stream0: &stream0,
            stream1: &stream1,
            stream2: &stream2,
            reader: &mut reader,
        },
    )
    .unwrap();

    assert_eq!(
        out,
        hex_width_records(
            "250012004002000000003200000300002c004800900500000000de0240020000\
                 01000f02501c000000006f023051000000009701a07d000000001200001500\
                 0000006100500400000000bb02409e0000"
        )
    );
    assert_eq!(
        result,
        WidthCombinerResult {
            ret: 82,
            consumed: [10, 9, 20],
        }
    );
    assert_eq!(
        reader,
        RansThreeLaneReader {
            ptr: 28351,
            acc: 0x0004_0404_f000_5028,
            bitpos: 62,
        }
    );
}

/// Width combiner (`0x110d360`) clamped-tail branch.
///
/// Provenance: `capture_width_combiner.py`, Animal_Dragonfly call 2:
/// `count=2,stride=20,shift=2,attr_width=1,limit=523`. The tail's
/// `limit - (sum_width + first)` is non-positive, so the final record's
/// high half and second word are both zero.
#[test]
fn width_combiner_dragonfly_tail_clamps() {
    let payload = sparse_payload(1207, &[(1190, "1c20c064a5a1c25aa7991d081e287cf867")]);
    let stream0 = hex_bytes("1a1c");
    let stream1 = hex_bytes("00");
    let stream2 = hex_bytes("0700");
    let mut reader = RansThreeLaneReader {
        ptr: 1190,
        acc: 0,
        bitpos: 0,
    };
    let mut out = hex_width_records("0100b3010a00000001004d00f8020000");

    let result = width_combiner_into(
        &mut out,
        WidthCombinerSpec {
            count: 2,
            stride: 20,
            shift: 2,
            attr_width: 1,
            limit: 523,
            payload: &payload,
            stream0: &stream0,
            stream1: &stream1,
            stream2: &stream2,
            reader: &mut reader,
        },
    )
    .unwrap();

    assert_eq!(out, hex_width_records("670001002c010000a301000000000000"));
    assert_eq!(
        result,
        WidthCombinerResult {
            ret: 522,
            consumed: [2, 1, 2],
        }
    );
    assert_eq!(
        reader,
        RansThreeLaneReader {
            ptr: 1200,
            acc: 0x0192_9687_096a_9e64,
            bitpos: 62,
        }
    );
}

#[test]
fn width_combiner_rejects_unobserved_and_malformed_inputs() {
    let mut reader = RansThreeLaneReader {
        ptr: 0,
        acc: 0,
        bitpos: 0,
    };
    let mut one = [[0u32; 2]; 1];
    assert_eq!(
        width_combiner_into(
            &mut one,
            WidthCombinerSpec {
                count: 1,
                stride: 1,
                shift: 0,
                attr_width: 1,
                limit: 1,
                payload: &[0; 8],
                stream0: &[0],
                stream1: &[],
                stream2: &[0, 0],
                reader: &mut reader,
            },
        ),
        Err(WidthCombinerError::UnobservedTailOnlyCount(1))
    );

    let mut reader = RansThreeLaneReader {
        ptr: 0,
        acc: 0,
        bitpos: 0,
    };
    assert_eq!(
        width_combiner_into(
            &mut one,
            WidthCombinerSpec {
                count: 2,
                stride: 1,
                shift: 0,
                attr_width: 1,
                limit: 1,
                payload: &[0; 8],
                stream0: &[0, 0],
                stream1: &[0],
                stream2: &[3, 0, 3, 0],
                reader: &mut reader,
            },
        ),
        Err(WidthCombinerError::OutputTooSmall)
    );

    let mut two = [[0u32; 2]; 2];
    let mut reader = RansThreeLaneReader {
        ptr: 0,
        acc: 0,
        bitpos: 0,
    };
    assert_eq!(
        width_combiner_into(
            &mut two,
            WidthCombinerSpec {
                count: 2,
                stride: 1,
                shift: 0,
                attr_width: 1,
                limit: 1,
                payload: &[],
                stream0: &[0, 0],
                stream1: &[0],
                stream2: &[3, 0, 3, 0],
                reader: &mut reader,
            },
        ),
        Err(WidthCombinerError::PayloadTooSmall)
    );

    let mut reader = RansThreeLaneReader {
        ptr: 0,
        acc: 0,
        bitpos: 0,
    };
    assert_eq!(
        width_combiner_into(
            &mut two,
            WidthCombinerSpec {
                count: 2,
                stride: 1,
                shift: 0,
                attr_width: 1,
                limit: 10,
                payload: &[0; 8],
                stream0: &[1, 1],
                stream1: &[0],
                stream2: &[0, 0],
                reader: &mut reader,
            },
        ),
        Err(WidthCombinerError::HistoryOutOfBounds)
    );
}

/// Transform tail `0x10fc5e0`: literal bytes plus overlapping copy-back.
///
/// Provenance: `capture_transform_tails.py`, Animal_Bear `0x10fc5e0` call,
/// first two records from entry `0x10000801`: `(37,18,576)` then
/// `(0,50,768)`. This compact slice keeps the observed stride-16 cursor and
/// byte-distance copy units that `verify_transform_tail_copy1.py` replays
/// over the full 3-call population.
#[test]
fn transform_tail_copy1_bear_literal_and_copy_back() {
    let source =
        hex_bytes("7f7f7f7f7f7f7f7f7f8181817f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f");
    let records = [
        TransformTailRecord {
            literal_count: 37,
            copy_count: 18,
            back_distance: 576,
        },
        TransformTailRecord {
            literal_count: 0,
            copy_count: 50,
            back_distance: 768,
        },
    ];
    let expected_lane = hex_bytes(
        "7f7f7f7f7f7f7f7f7f8181817f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f\
             7f7f7f7f7f7f7f7f7f7f8181817f7f7f7f7f7f7f7f7f8181817f7f7f7f7f7f7f7f7f7f\
             7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f8181817f7f7f7f7f7f7f7f7f",
    );
    let mut out = vec![0xee; expected_lane.len() * 16];

    let consumed = transform_tail_copy1_into(
        &mut out,
        TransformTailCopy1Spec {
            output_stride: 16,
            block_index: 0,
            out_offset: 0,
            records: &records,
            source: &source,
        },
    )
    .unwrap();

    assert_eq!(consumed, 37);
    for (index, &expected) in expected_lane.iter().enumerate() {
        assert_eq!(out[index * 16], expected);
    }
    for (index, &byte) in out.iter().enumerate() {
        if index % 16 != 0 {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
    assert_eq!(out[1], 0xee, "rules out a contiguous cursor");
    assert_eq!(out[37 * 16], source[1], "copy-back distance is in bytes");
    assert_eq!(out[55 * 16], source[7], "zero-literal record copies only");
}

/// Transform tail `0x10fc680`: two-byte literals plus copy-back.
///
/// Provenance: `capture_transform_tails.py`, Animal_Dragonfly
/// `0x10fc680` call, all three records from entry `0x0a000802`:
/// `(1,435,10)`, `(1,77,760)`, `(1,8,100)`. This covers the full
/// observed population for the two-byte copy tail and keeps the stride-10
/// byte-distance copy units replayed by `verify_transform_tail_copy2.py`.
#[test]
fn transform_tail_copy2_dragonfly_two_byte_runs() {
    let source = hex_bytes("ff007f807f80");
    let records = [
        TransformTailRecord {
            literal_count: 1,
            copy_count: 435,
            back_distance: 10,
        },
        TransformTailRecord {
            literal_count: 1,
            copy_count: 77,
            back_distance: 760,
        },
        TransformTailRecord {
            literal_count: 1,
            copy_count: 8,
            back_distance: 100,
        },
    ];
    let expected_lane = hex_bytes(concat!(
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff007f80",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
        "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff007f80",
        "ff007f80ff00ff00ff00ff00ff00ff00ff007f80",
    ));
    let mut out = vec![0xee; (expected_lane.len() / 2) * 10];

    let consumed = transform_tail_copy2_into(
        &mut out,
        TransformTailCopy2Spec {
            output_stride: 10,
            block_index: 0,
            out_offset: 0,
            records: &records,
            source: &source,
        },
    )
    .unwrap();

    assert_eq!(consumed, 6);
    for (unit_index, expected) in expected_lane.chunks_exact(2).enumerate() {
        let base = unit_index * 10;
        assert_eq!(&out[base..base + 2], expected);
    }
    for (index, &byte) in out.iter().enumerate() {
        if index % 10 >= 2 {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
    assert_eq!(&out[10..12], &source[0..2], "rules out a contiguous cursor");
    assert_eq!(
        &out[436 * 10..436 * 10 + 2],
        &source[2..4],
        "second literal follows a long copy run"
    );
    assert_eq!(
        &out[512 * 10..512 * 10 + 2],
        &source[2..4],
        "copy-back distance is in bytes across prior literals"
    );
}

/// Transform tail `0x10fc720`: three-byte literals plus copy-back.
///
/// Provenance: `capture_phase1_transform_tail_10fc720.py`,
/// Horse_Link_Mane_GrabbedModel_L current 2, full call from entry
/// `0x0c000803`: records `(1,64,12)`, `(1,5,24)`, `(1,5,84)`,
/// `(0,5,96)`, `(0,6,120)`, and `(0,9,288)`. This covers direct literals,
/// byte-distance copy units, and observed zero-literal records;
/// `verify_transform_tail_10fc720.py` replays the 7-call population.
#[test]
fn transform_tail_copy3_horse_zero_literal_copy_runs() {
    let source = hex_bytes("ff00002eb71aff0000");
    let records = [
        TransformTailRecord {
            literal_count: 1,
            copy_count: 64,
            back_distance: 12,
        },
        TransformTailRecord {
            literal_count: 1,
            copy_count: 5,
            back_distance: 24,
        },
        TransformTailRecord {
            literal_count: 1,
            copy_count: 5,
            back_distance: 84,
        },
        TransformTailRecord {
            literal_count: 0,
            copy_count: 5,
            back_distance: 96,
        },
        TransformTailRecord {
            literal_count: 0,
            copy_count: 6,
            back_distance: 120,
        },
        TransformTailRecord {
            literal_count: 0,
            copy_count: 9,
            back_distance: 288,
        },
    ];
    let expected_lane = hex_bytes(concat!(
        "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
        "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
        "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
        "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
        "ff00002eb71aff00002eb71aff00002eb71aff0000ff00002eb71aff00002eb71aff00002eb71a2eb71aff0000ff0000",
        "2eb71aff00002eb71aff00002eb71aff00002eb71a2eb71aff00002eb71aff00002eb71aff00002eb71aff0000ff0000",
        "2eb71a",
    ));
    let mut out = vec![0xee; 9 + 96 * 12 + 3];

    let consumed = transform_tail_copy3_into(
        &mut out,
        TransformTailCopy3Spec {
            output_stride: 12,
            block_index: 0,
            out_offset: 9,
            records: &records,
            source: &source,
        },
    )
    .unwrap();

    assert_eq!(consumed, 9);
    assert_eq!(expected_lane.len(), 97 * 3);
    for (unit_index, expected) in expected_lane.chunks_exact(3).enumerate() {
        let base = 9 + unit_index * 12;
        assert_eq!(&out[base..base + 3], expected, "unit {unit_index}");
    }
    for (index, &byte) in out.iter().enumerate() {
        let is_lane = (9..9 + 97 * 12).contains(&index) && (index - 9) % 12 < 3;
        if !is_lane {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
    assert_eq!(&out[21..24], &source[0..3], "copy distance is in bytes");
    assert_eq!(
        &out[9 + 65 * 12..9 + 65 * 12 + 3],
        &source[3..6],
        "second literal follows a long copy run"
    );
}

/// Transform tail `0x10fc720`: observed zero-copy record.
///
/// Provenance: `capture_phase1_transform_tail_10fc720.py`,
/// DgnObj_ElectricStoneBoard_A_01 current 5, full call from entry
/// `0x18000803`. The final record `(15,0,0)` takes the `b.lo` skip-copy
/// branch at `0x10fc788..0x10fc7c0`.
#[test]
fn transform_tail_copy3_electric_board_zero_copy_record() {
    let source = hex_bytes(concat!(
        "810000810500820f0081000081000081ff0081010081000081040081010081fe0083ea0489d300f886ddfafa81eeaea1",
        "fefa81e4e08803fd8102fa814b67037f01007e10031f7bfa772dfd01017f0efd7e00fc7fff017f0c027e00007f00017f",
        "00fe7fffff7f04fd7f03017ffe017f00007f00027f00007fed127ce6f27c00007f00007f7ffef51684f179ddf02988fe",
        "7ffffd7ff70413830a177cef7f02fb772dfd6847ee7c180700fe810be584fdf98101f082179cb505ee820a8dcb0682ef",
        "00f98100018100008100fe81010081df0f86dced8800008100008101008100ff810d098220d78c000081010881348c04",
        "378e067fff067f00037ef1ff4595fc82030d85ee1a98cc34b9573b8413158100f78100f68100fd81fd008100078100fe",
        "81000781fdf78100068200f2810a0082f4f188fed68104f8e1b2a1f7e784a503a781000081000181000081ff03840de6",
        "90fbc486fe248a102ccfe88dca168f8a08d1a6ffa78bff30d21d73a60359c4df6b",
    ));
    let records = [
        TransformTailRecord {
            literal_count: 6,
            copy_count: 2,
            back_distance: 48,
        },
        TransformTailRecord {
            literal_count: 4,
            copy_count: 2,
            back_distance: 120,
        },
        TransformTailRecord {
            literal_count: 65,
            copy_count: 2,
            back_distance: 72,
        },
        TransformTailRecord {
            literal_count: 33,
            copy_count: 2,
            back_distance: 24,
        },
        TransformTailRecord {
            literal_count: 15,
            copy_count: 0,
            back_distance: 0,
        },
    ];
    let expected_lane = hex_bytes(concat!(
        "810000810500820f0081000081000081ff0081000081ff0081010081000081040081010081ff0081010081fe0083ea04",
        "89d300f886ddfafa81eeaea1fefa81e4e08803fd8102fa814b67037f01007e10031f7bfa772dfd01017f0efd7e00fc7f",
        "ff017f0c027e00007f00017f00fe7fffff7f04fd7f03017ffe017f00007f00027f00007fed127ce6f27c00007f00007f",
        "7ffef51684f179ddf02988fe7ffffd7ff70413830a177cef7f02fb772dfd6847ee7c180700fe810be584fdf98101f082",
        "179cb505ee820a8dcb0682ef00f98100018100008100fe81010081df0f86dced8800008100008101008100ff81000081",
        "0100810d098220d78c000081010881348c04378e067fff067f00037ef1ff4595fc82030d85ee1a98cc34b9573b841315",
        "8100f78100f68100fd81fd008100078100fe81000781fdf78100068200f2810a0082f4f188fed68104f8e1b2a1f7e784",
        "a503a781000081000081000081000181000081ff03840de690fbc486fe248a102ccfe88dca168f8a08d1a6ffa78bff30",
        "d21d73a60359c4df6b",
    ));
    let mut out = vec![0xee; 808 + 130 * 24 + 3];

    let consumed = transform_tail_copy3_into(
        &mut out,
        TransformTailCopy3Spec {
            output_stride: 24,
            block_index: 0,
            out_offset: 808,
            records: &records,
            source: &source,
        },
    )
    .unwrap();

    assert_eq!(consumed, 369);
    assert_eq!(expected_lane.len(), 131 * 3);
    for (unit_index, expected) in expected_lane.chunks_exact(3).enumerate() {
        let base = 808 + unit_index * 24;
        assert_eq!(&out[base..base + 3], expected, "unit {unit_index}");
    }
    assert_eq!(
        &out[808 + 116 * 24..808 + 116 * 24 + 3],
        &source[108 * 3..108 * 3 + 3],
        "zero-copy record still writes trailing literals"
    );
}

/// Transform tail `0x11033e0`: two-byte seed/previous/matched delta.
///
/// Provenance: `capture_transform_tail_11033e0.py`, Animal_Bass current 1:
/// one observed call at entry `0x0a000802`, `bufB+6`, stride 10.
/// `verify_transform_tail_11033e0.py` replays 1/1 captured call with
/// source0=208, source1=382, and 559 match-table entries consumed.
#[test]
fn transform_tail_u8x2_delta_bass_seed_previous_match_and_copy() {
    let records = [TransformTailRecord {
        literal_count: 4,
        copy_count: 13,
        back_distance: 10,
    }];
    let matches = [0u32; 17];
    let source0 = hex_bytes("0000000000000101");
    let mut out = vec![0xee; 180];

    let usage = transform_tail_u8x2_delta_into(
        &mut out,
        TransformTailU8x2DeltaSpec {
            output_stride: 10,
            block_index: 0,
            out_offset: 6,
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &[],
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 8,
            source1: 0,
            source2: 0,
            match_entries: 17,
        }
    );
    let expected_pairs =
        hex_bytes("00000000000001010101010101010101010101010101010101010101010101010101");
    for (index, expected) in expected_pairs.chunks_exact(2).enumerate() {
        assert_eq!(&out[6 + index * 10..8 + index * 10], expected);
    }
    assert_eq!(out[8], 0xee, "rules out contiguous two-byte writes");

    let records = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let matches = [8u32];
    let mut out = vec![0xee; 30];
    out[6] = 10;
    out[7] = 20;
    let usage = transform_tail_u8x2_delta_into(
        &mut out,
        TransformTailU8x2DeltaSpec {
            output_stride: 10,
            block_index: 0,
            out_offset: 16,
            records: &records,
            matches: &matches,
            source0: &[],
            source1: &[1, 2],
        },
    )
    .unwrap();
    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 0,
            source1: 2,
            source2: 0,
            match_entries: 1,
        }
    );
    assert_eq!(&out[16..18], &[11, 22]);
}

/// Transform tail `0x10fdb30`: one-byte direct/matched delta.
///
/// Provenance: `capture_phase1_transform_tail_10fdb30.py`,
/// Animal_Crab.Crab current 1, records 0 through 3 rebased from output offset
/// 6 to zero. `verify_transform_tail_10fdb30.py` replays the full observed
/// population 1/1.
#[test]
fn transform_tail_delta1_direct_crab_first_matched_prefix() {
    let records = [
        TransformTailRecord {
            literal_count: 1,
            copy_count: 17,
            back_distance: 8,
        },
        TransformTailRecord {
            literal_count: 1,
            copy_count: 85,
            back_distance: 8,
        },
        TransformTailRecord {
            literal_count: 1,
            copy_count: 7,
            back_distance: 8,
        },
        TransformTailRecord {
            literal_count: 1,
            copy_count: 10,
            back_distance: 832,
        },
    ];
    let mut matches = vec![0u32; 123];
    matches[112] = 840;
    let source0 = hex_bytes("040306");
    let source1 = hex_bytes("00");
    let expected_slots = hex_bytes(concat!(
        "0404040404040404040404040404040404040303030303030303030303030303",
        "0303030303030303030303030303030303030303030303030303030303030303",
        "0303030303030303030303030303030303030303030303030303030303030303",
        "030303030303030306060606060606060404040404040404040403",
    ));
    let mut out = vec![0xee; 123 * 8];

    let usage = transform_tail_delta1_direct_into(
        &mut out,
        TransformTailDelta1DirectSpec {
            output_stride: 8,
            block_index: 0,
            out_offset: 0,
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 3,
            source1: 1,
            source2: 0,
            match_entries: 123,
        }
    );
    assert_eq!(expected_slots.len(), 123);
    for (unit_index, &expected) in expected_slots.iter().enumerate() {
        assert_eq!(out[unit_index * 8], expected, "unit {unit_index}");
    }
    for (index, &byte) in out.iter().enumerate() {
        if index % 8 != 0 {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
}

/// Transform tail `0x10ffdb0`: three-byte signed direct/matched delta.
///
/// Provenance: `capture_phase1_transform_tail_10ffdb0.py`,
/// Animal_Grasshopper.Grasshopper current 4, prefix through the first sign-bit
/// matched literal rebased from output offset 2120 to zero.
/// `verify_transform_tail_10ffdb0.py` replays the full observed population 1/1.
#[test]
fn transform_tail_i8x3_direct_delta_grasshopper_sign_prefix() {
    let records = [TransformTailRecord {
        literal_count: 37,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut matches = vec![0u32; 37];
    for (index, value) in [
        (18, 32),
        (19, 48),
        (21, 80),
        (22, 96),
        (24, 56),
        (25, 72),
        (27, 176),
        (28, 192),
        (30, 224),
        (31, 240),
        (33, 200),
        (34, 216),
        (36, 273),
    ] {
        matches[index] = value;
    }
    let source0 = hex_bytes(concat!(
        "93cb2793cb278dd723e6037ce6037cba286236ae5036ae50e6f07be3e779e3e7790aef7e",
        "51d7582cc66836f372329f41329f41500162e9e18791c91a3ca6be35118edae18b4aa8ca",
    ));
    let source1 =
        hex_bytes("97fe1bbd1b1f7c09a5aee2a191f6230a077d9fff4d35160b520568471664830b3d14fa7a000000");
    let expected_slots = hex_bytes(concat!(
        "93cb2793cb278dd723e6037ce6037cba286236ae5036ae50e6f07be3e779e3e7790aef7e",
        "51d7582cc66836f372329f41329f41500162cdf18de9e187e9e18786f82391c91a91c91a",
        "e1f7853ca6be3ca6be5927af1b198735118edfdc8bdae18bdae18b69fbb84aa8ca4aa8ca",
        "73d723",
    ));
    let mut out = vec![0xee; 37 * 16];

    let usage = transform_tail_i8x3_direct_delta_into(
        &mut out,
        TransformTailI8x3DirectDeltaSpec {
            output_stride: 16,
            block_index: 0,
            out_offset: 0,
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 72,
            source1: 39,
            source2: 0,
            match_entries: 37,
        }
    );
    assert_eq!(expected_slots.len(), 37 * 3);
    for (unit_index, expected) in expected_slots.chunks_exact(3).enumerate() {
        let base = unit_index * 16;
        assert_eq!(&out[base..base + 3], expected, "unit {unit_index}");
    }
    for (index, &byte) in out.iter().enumerate() {
        if index % 16 >= 3 {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
}

/// Transform tail `0x1101230`: one-byte seed/previous delta.
///
/// Provenance: `capture_phase1_transform_tail_1101230.py`,
/// Animal_Boar.Boar current 7, first record `(25,26,384)` rebased from output
/// offset 23567 to zero. `verify_transform_tail_1101230.py` replays the full
/// observed population 1/1.
#[test]
fn transform_tail_u8_previous_delta_boar_first_record() {
    let records = [TransformTailRecord {
        literal_count: 25,
        copy_count: 26,
        back_distance: 384,
    }];
    let source0 = hex_bytes("7f0000020000fe000000000000000000000000000000000000");
    let expected_slots = hex_bytes(
        "7f7f7f8181817f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f8181817f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f",
    );
    let mut out = vec![0xee; 51 * 16];

    let usage = transform_tail_u8_previous_delta_into(
        &mut out,
        TransformTailU8PreviousDeltaSpec {
            output_stride: 16,
            block_index: 0,
            out_offset: 0,
            records: &records,
            source0: &source0,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 25,
            source1: 0,
            source2: 0,
            match_entries: 0,
        }
    );
    assert_eq!(expected_slots.len(), 51);
    for (unit_index, &expected) in expected_slots.iter().enumerate() {
        assert_eq!(out[unit_index * 16], expected, "unit {unit_index}");
    }
    for (index, &byte) in out.iter().enumerate() {
        if index % 16 != 0 {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
}

/// Transform tail `0x1101410`: three-byte seed/previous delta.
///
/// Provenance: `capture_phase1_transform_tail_1101410.py`,
/// CaveObj_WaterFall_A.CaveObj_WaterFall_A_04 current 4, first record
/// `(85,2,20)` rebased from output offset 1668 to zero.
/// `verify_transform_tail_1101410.py` replays the sampled population 7/7.
#[test]
fn transform_tail_u8x3_previous_delta_cave_waterfall_first_record() {
    let records = [TransformTailRecord {
        literal_count: 85,
        copy_count: 2,
        back_distance: 20,
    }];
    let source0 = hex_bytes(concat!(
        "7e13fefd08f401020b02f2fb01f208fe11f102ef0f00fe00ff0bf701f70900ff000002ffff07f2ff03fb02f70f00fe",
        "04fe02ed0001fcfffdfd030012fe00f1fefff9040014fe00f3ff00fcfe00f9ff00fd05001afc00eafd00f6fe00fb",
        "020006030008020008020009000000010012ff00f101000d000002000002ff000dff0008ff0002fc0010fe0005ff00",
        "030400f50000010200f70100fffe00050400effe000bff000401fffd0201f1ff0008fffc030304e80000090000f5",
        "fffd120103ef00ff08fff806fffd060208f20004fa00fc03fff609fffc03fffafe0207fcfdf2fa030eff0110ffff",
        "f0010000fefdf2030416fd00090000fc00000100000200",
    ));
    let expected_slots = hex_bytes(concat!(
        "7e13fe7b1bf27c1dfd7e0ff87f01007d12f17f01007fff007e0af77f01007f00007f02ff7e09f17d0cec7f03",
        "fb7f01ff7d03ec7d04e87c01e57f01f77d01e87b00e17f00f57d00e87c00e47a00dd7900da7e00f47a00de",
        "7700d47500cf7700d57a00dd7c00e57e00ee7e00ee7f00007e00f17f00fe7f00007f00027e000f7d00177c",
        "001978002976002e7500317900267900277b001e7c001d7a00227e00117c001c7b00207cff1d7e000e7d0016",
        "7cfc197f00017f000a7f00ff7efd117f00007fff087ef70e7df4147ffc067f00007ffc037ef20c7dee0f7ce8",
        "0d7eef097be1037eef027fff017eef027eef007be1037ff7007f00007ffc007ffd007fff007fff007fff00",
    ));
    let mut out = vec![0xee; 87 * 20];

    let usage = transform_tail_u8x3_previous_delta_into(
        &mut out,
        TransformTailU8x3PreviousDeltaSpec {
            output_stride: 20,
            block_index: 0,
            out_offset: 0,
            records: &records,
            source0: &source0,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 255,
            source1: 0,
            source2: 0,
            match_entries: 0,
        }
    );
    assert_eq!(expected_slots.len(), 87 * 3);
    for (unit_index, expected) in expected_slots.chunks_exact(3).enumerate() {
        let base = unit_index * 20;
        assert_eq!(&out[base..base + 3], expected, "unit {unit_index}");
    }
    assert_eq!(
        &out[85 * 20..85 * 20 + 3],
        &out[84 * 20..84 * 20 + 3],
        "first copy uses the previous row by byte back-distance"
    );
    assert_eq!(
        &out[86 * 20..86 * 20 + 3],
        &out[85 * 20..85 * 20 + 3],
        "second copy chains through the copied row"
    );
    for (index, &byte) in out.iter().enumerate() {
        if index % 20 >= 3 {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
}

/// Transform tail `0x1103530`: three-byte seed/previous/matched delta.
///
/// Provenance: `capture_phase1_transform_tail_1103530.py`,
/// Animal_Cassowary.Cassowary current 1, first two records
/// `(5,2,24)` and `(18,2,84)`. `verify_transform_tail_1103530.py` replays
/// the full observed population 1/1.
#[test]
fn transform_tail_u8x3_delta_cassowary_first_records() {
    let records = [
        TransformTailRecord {
            literal_count: 5,
            copy_count: 2,
            back_distance: 24,
        },
        TransformTailRecord {
            literal_count: 18,
            copy_count: 2,
            back_distance: 84,
        },
    ];
    let mut matches = [0u32; 27];
    matches[18] = 56;
    matches[22] = 16;
    let source0 = hex_bytes(concat!(
        "0b131af5f8f90b0807f500f900f8000000000000000b0ff80000000000000cfd0c000000000000eff0f10000000201",
        "0000000000000ff5f1000000f100f90f",
    ));
    let source1 = hex_bytes("000000000000");
    let expected_slots = hex_bytes(concat!(
        "0b131a000b130b131a001313000b13001313000b13000b13000b130b1a0b0b1a0b0b1a0b171717171717171717",
        "0607080607080808080b1a0b0b1a0b0b1a1a000b1a0b1a1a0b1a0b0b131a0b1a0b0b1a0b",
    ));
    let mut out = vec![0xee; 6 + 27 * 12];

    let usage = transform_tail_u8x3_delta_into(
        &mut out,
        TransformTailU8x3DeltaSpec {
            output_stride: 12,
            block_index: 0,
            out_offset: 6,
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 63,
            source1: 6,
            source2: 0,
            match_entries: 27,
        }
    );
    assert_eq!(expected_slots.len(), 27 * 3);
    for (unit_index, expected) in expected_slots.chunks_exact(3).enumerate() {
        let base = 6 + unit_index * 12;
        assert_eq!(&out[base..base + 3], expected, "unit {unit_index}");
    }
    for (index, &byte) in out.iter().enumerate() {
        let is_lane = (6..6 + 27 * 12).contains(&index) && (index - 6) % 12 < 3;
        if !is_lane {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
}

/// Transform tail `0x10fc7d0`: four-byte literals plus copy-back.
///
/// Provenance: `capture_transform_tails.py`, Animal_Bear `0x10fc7d0` call,
/// first record from entry `0x1000100a`: `(101,2,1616)`. The replay script
/// covers the full 2-call population; this compact golden keeps the
/// observed stride-16 cursor and byte-distance copy units fixture-free.
#[test]
fn transform_tail_copy4_bear_u32_runs() {
    let source = hex_bytes(concat!(
            "3b064795f0051595bf055795f9021688a6033888df0323872307b08ede07e78e7307778ea07fd736f77f913833800638",
            "4a75cc0b6f746907cf71010d9229a03a132bd13f022daa3a3c1774076e15e80162134f07a43fb316b941bf1b12438917",
            "ad0e22417e0d023cab0ba23f815191035c4fd8015650b7067a81fa0c7b821012f3831c0cd46c1e030a6f78087c6faf01",
            "3a0c046f3b0b556a630a796d9b605507025f3602f65d7005b50952f66704bcf7cf0f71fad02d1d0fcb348a117730290f",
            "dff8009fadf8be9e63f8f09ec1f6ec96faf60198a7f7de9703f8a09a98f7109b54f8d89afe8305383b8490389184d636",
            "5f8c2b03858e79071a8f0a04bef9b1027dfbe9072efdc702f7d1b203aed33709d0d5c903b39de10eee9bd40a5f9a1010",
            "3ef12d705ff213747af4416faab1440f1eb2520a2bb0430cfe88840c6189a312468bce0d6ba1750612a05f01309fc405",
            "a2fe704c6cfdf247fffba54e19b83b040db7010175b52006caece3f933f82ef7e5f2c4f571cce81991c7b21af5cec01a",
            "b806389451060395b7063a95b8063894d505d094",
        ));
    let records = [TransformTailRecord {
        literal_count: 101,
        copy_count: 2,
        back_distance: 1616,
    }];
    let expected_lane = hex_bytes(concat!(
            "3b064795f0051595bf055795f9021688a6033888df0323872307b08ede07e78e7307778ea07fd736f77f913833800638",
            "4a75cc0b6f746907cf71010d9229a03a132bd13f022daa3a3c1774076e15e80162134f07a43fb316b941bf1b12438917",
            "ad0e22417e0d023cab0ba23f815191035c4fd8015650b7067a81fa0c7b821012f3831c0cd46c1e030a6f78087c6faf01",
            "3a0c046f3b0b556a630a796d9b605507025f3602f65d7005b50952f66704bcf7cf0f71fad02d1d0fcb348a117730290f",
            "dff8009fadf8be9e63f8f09ec1f6ec96faf60198a7f7de9703f8a09a98f7109b54f8d89afe8305383b8490389184d636",
            "5f8c2b03858e79071a8f0a04bef9b1027dfbe9072efdc702f7d1b203aed33709d0d5c903b39de10eee9bd40a5f9a1010",
            "3ef12d705ff213747af4416faab1440f1eb2520a2bb0430cfe88840c6189a312468bce0d6ba1750612a05f01309fc405",
            "a2fe704c6cfdf247fffba54e19b83b040db7010175b52006caece3f933f82ef7e5f2c4f571cce81991c7b21af5cec01a",
            "b806389451060395b7063a95b8063894d505d0943b064795f0051595",
        ));
    let mut out = vec![0xee; (expected_lane.len() / 4) * 16];

    let consumed = transform_tail_copy4_into(
        &mut out,
        TransformTailCopy4Spec {
            output_stride: 16,
            block_index: 0,
            out_offset: 0,
            records: &records,
            source: &source,
        },
    )
    .unwrap();

    assert_eq!(consumed, 404);
    for (unit_index, expected) in expected_lane.chunks_exact(4).enumerate() {
        let base = unit_index * 16;
        assert_eq!(&out[base..base + 4], expected);
    }
    for (index, &byte) in out.iter().enumerate() {
        if index % 16 >= 4 {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
    assert_eq!(&out[101 * 16..101 * 16 + 4], &source[0..4]);
    assert_eq!(&out[102 * 16..102 * 16 + 4], &source[4..8]);
}

#[test]
fn transform_tail_copy4_allows_observed_zero_literal_and_zero_copy() {
    let zero_literal = [TransformTailRecord {
        literal_count: 0,
        copy_count: 2,
        back_distance: 5440,
    }];
    let mut out = vec![0xee; 5460];
    out[0..4].copy_from_slice(&[0x30, 0x2a, 0x03, 0xdd]);
    out[16..20].copy_from_slice(&[0x61, 0x1f, 0x3c, 0xd7]);
    let consumed = transform_tail_copy4_into(
        &mut out,
        TransformTailCopy4Spec {
            output_stride: 16,
            block_index: 0,
            out_offset: 5440,
            records: &zero_literal,
            source: &[],
        },
    )
    .unwrap();
    assert_eq!(consumed, 0);
    assert_eq!(&out[5440..5444], &[0x30, 0x2a, 0x03, 0xdd]);
    assert_eq!(&out[5456..5460], &[0x61, 0x1f, 0x3c, 0xd7]);

    let zero_copy = [TransformTailRecord {
        literal_count: 10,
        copy_count: 0,
        back_distance: 0,
    }];
    let source = hex_bytes(
        "0a22dae92211dcd3462878d7f0382deca11d35bccb3153c77e33ccd50b4b0fe61b3850c3b251aed4",
    );
    let mut out = vec![0xee; 10 * 16];
    let consumed = transform_tail_copy4_into(
        &mut out,
        TransformTailCopy4Spec {
            output_stride: 16,
            block_index: 0,
            out_offset: 0,
            records: &zero_copy,
            source: &source,
        },
    )
    .unwrap();
    assert_eq!(consumed, 40);
    for (unit_index, expected) in source.chunks_exact(4).enumerate() {
        let base = unit_index * 16;
        assert_eq!(&out[base..base + 4], expected);
    }
}

/// Transform tail `0x10fc870`: six-byte literals plus copy-back.
///
/// Provenance: `phase1_direction_zero_writer_loop_capture.json` from
/// `capture_vertex_writer_loop.py`, Animal_Shell_B.Shell_B row 0, first three
/// records for writer `0x10fc870`: `(18,1,30)`, `(4,1,126)`, `(0,1,144)`.
/// The zero-literal third record is observed in DirectionZero; zero-copy is not.
/// `verify_transform_tail_10fc870.py` replays the full observed population 1/1.
#[test]
fn transform_tail_copy6_shell_direction_zero_writer() {
    let source = hex_bytes(concat!(
        "efa5af290923eda90029462b33ab872889a47daeb5245c27ccb0b221f52f78ac43264c3046b0228bb93342ab6223ed32",
        "68a465241e34f6aa361cfc34722ac51fbd354f2a8427a6349fa06633a4b2bd25a43449b4f829323377b242a57e31cdb0",
        "e328c230a1b058a8a32ea4adea2b1334cbb15b28022dafae692bca3126b1efa451236ba6",
    ));
    let records = [
        TransformTailRecord {
            literal_count: 18,
            copy_count: 1,
            back_distance: 30,
        },
        TransformTailRecord {
            literal_count: 4,
            copy_count: 1,
            back_distance: 126,
        },
        TransformTailRecord {
            literal_count: 0,
            copy_count: 1,
            back_distance: 144,
        },
    ];
    let expected = hex_bytes(concat!(
        "efa5af290923eda90029462b33ab872889a47daeb5245c27ccb0b221f52f78ac43264c3046b0228bb93342ab6223ed32",
        "68a465241e34f6aa361cfc34722ac51fbd354f2a8427a6349fa06633a4b2bd25a43449b4f829323377b242a57e31cdb0",
        "e328c230a1b058a8a32ea4adbd25a43449b4ea2b1334cbb15b28022dafae692bca3126b1efa451236ba633ab872889a4",
        "efa5af290923",
    ));
    let mut out = vec![0; expected.len()];

    let consumed = transform_tail_copy6_into(
        &mut out,
        TransformTailCopy6Spec {
            output_stride: 6,
            block_index: 0,
            out_offset: 0,
            records: &records,
            source: &source,
        },
    )
    .unwrap();

    assert_eq!(consumed, 132);
    assert_eq!(out, expected);
    assert_eq!(
        &out[108..114],
        &out[78..84],
        "first copy uses byte distance"
    );
    assert_eq!(
        &out[144..150],
        &out[0..6],
        "zero-literal copy remains active"
    );
}

/// Transform tail `0x10fc920`: eight-byte literals plus copy-back.
///
/// Provenance: `capture_phase1_transform_tail_10fc920.py`,
/// DgnObj_Fire_WallBeam_A_09 current 2, first record from entry `0x1c002012`:
/// `(18,1,280)`. `verify_transform_tail_10fc920.py` replays the full observed
/// population 1/1.
#[test]
fn transform_tail_copy8_wallbeam_first_record() {
    let source = hex_bytes(concat!(
        "1b0c22bf78e748bf9a9919bf8c6144bf6fcf18bf28e846bf",
        "9a9919bfd814b63d9a9919bf30de7c3dc6ee22bf70308a3d",
        "d0cccc3eb0b02a3fd0cccc3e661a323fa0fce03e20bb2f3f",
        "7d434441829855c0d1884341e28f55c02d944441400454c0",
        "3333b33f80a4833ebcb0b53ff0a6833e3333b33ffcb86a3e",
        "d0cccc3e7af05a3f5856e43eb062513fd0cccc3e4eb4503f",
    ));
    let records = [TransformTailRecord {
        literal_count: 18,
        copy_count: 1,
        back_distance: 280,
    }];
    let expected_slots = hex_bytes(concat!(
        "1b0c22bf78e748bf9a9919bf8c6144bf6fcf18bf28e846bf",
        "9a9919bfd814b63d9a9919bf30de7c3dc6ee22bf70308a3d",
        "d0cccc3eb0b02a3fd0cccc3e661a323fa0fce03e20bb2f3f",
        "7d434441829855c0d1884341e28f55c02d944441400454c0",
        "3333b33f80a4833ebcb0b53ff0a6833e3333b33ffcb86a3e",
        "d0cccc3e7af05a3f5856e43eb062513fd0cccc3e4eb4503f",
        "a0fce03e20bb2f3f",
    ));
    let mut out = vec![0xee; 19 * 28];

    let consumed = transform_tail_copy8_into(
        &mut out,
        TransformTailCopy8Spec {
            output_stride: 28,
            block_index: 0,
            out_offset: 0,
            records: &records,
            source: &source,
        },
    )
    .unwrap();

    assert_eq!(consumed, 18 * 8);
    assert_eq!(expected_slots.len(), 19 * 8);
    for (unit_index, expected) in expected_slots.chunks_exact(8).enumerate() {
        let base = unit_index * 28;
        assert_eq!(&out[base..base + 8], expected, "unit {unit_index}");
    }
    for (index, &byte) in out.iter().enumerate() {
        if index % 28 >= 8 {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
    assert_eq!(
        &out[18 * 28..18 * 28 + 8],
        &source[8 * 8..9 * 8],
        "copy-back distance is in bytes"
    );
}

/// Transform tail `0x10fbcc0`: two-byte direct and matched deltas.
///
/// Provenance: `capture_transform_tails.py`, Animal_Bass `0x10fbcc0` call,
/// first record from entry `0x0a000802`: `(276,6,20)`. This covers direct
/// literals, match-table literals, and the copy loop; the replay script
/// covers the full 1-call population including the later zero-count records.
#[test]
fn transform_tail_delta2_bass_direct_match_and_copy() {
    let source0 = hex_bytes(concat!(
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000000000",
        ));
    let source1 = hex_bytes(concat!(
            "000000000000000000000000000000000000000000cc000000000000b2a6000000000000000000000000000000000000",
            "00000000000000000000cc00cc00000000000000000000000000000000000000000000000000000000001a001a80bfbf",
            "1a801a000000000000001a0000008000bf00bf000000000000001a1a000000000000cc",
        ));
    let source2 = hex_bytes(concat!(
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "0000",
        ));
    let matches = hex_u32_words(concat!(
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000003100000031000000000000003000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000006101000000000000",
            "790100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000009901000091010000a10100009901000099010000990100009901000089000000810000009100000089000000",
            "890000008900000081000000910000008900000089000000390200003902000039020000590200006902000081020000",
            "810200007901000088010000990100008801000089010000880100005202000000000000000000000000000000000000",
            "000000005800000058000000290200005002000060020000290200002802000028020000890200005002000060020000",
            "89020000300000008802000098000000fb020000a1000000a0000000a1000000a1000000a00000009002000010030000",
            "110300001003000031030000400300005903000059030000a102000099020000a8020000a002000000040000a0020000",
            "18040000a0020000a1020000e80200000000000048000000000000000000000000000000080300000000000000000000",
            "000000000000000000000000280300000000000000000000b10400000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000300400002001000000000000000000004004000000000000b8010000f8010000a8010000b8010000",
            "b0010000b1010000b001000020020000b0010000b1010000b1010000b0010000b101000038020000b1010000b1010000",
            "60060000b1010000b1010000b1010000b1010000b1010000b1010000b1010000b1010000b1010000b1010000b1010000",
            "b1010000b0010000b1010000b1010000b1010000b1010000b0010000b1010000b1010000b1010000b1010000b1010000",
            "b1010000b1010000b1010000b1010000b1010000b0010000b0010000b101000040030000a9010000a801000048030000",
            "a8010000a801000000000000500000006800000000000000000000000000000000000000980000000000000000000000",
            "c00000000000000000000000e0000000f000000000000000000000000000000000000000000000003801000000000000",
            "000000000000000068010000000000007001000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000",
        ));
    let records = [TransformTailRecord {
        literal_count: 276,
        copy_count: 6,
        back_distance: 20,
    }];
    let expected_lane = hex_bytes(concat!(
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff0033ccff00ff00",
            "ff00ff00ff00ff004db259a6ff00ff00ff00ff004db2ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff0033ccff00",
            "33ccff00ff00ff00ff00ff00ff004db2ff00ff00ff00ff00ff004db259a6ff00ff00ff0033ccff0033ccff00ff0033cc",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00e51aff00e51a7f8040bf40bfe51a7f80e51aff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00ff00ff00ff00ff00ff00e51aff00e51a7f8040bf40bfe51a7f80e51aff00ff00ff00ff00ff00ff00ff00ff00",
            "ff00ff00e51aff00e51aff00ff00ff007f807f80ff0040bf40bfff0040bf40bf7f80ff00ff00ff00ff00ff00e51aff00",
            "ff00e51aff00e51aff00ff00ff00ff00ff00ff00ff0033ccff0033ccff0033ccff0033cc",
        ));
    let mut out = vec![0xee; (expected_lane.len() / 2) * 10];

    let usage = transform_tail_delta2_into(
        &mut out,
        TransformTailDelta2Spec {
            output_stride: 10,
            block_index: 0,
            out_offset: 0,
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
            source2: &source2,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 131,
            source1: 131,
            source2: 290,
            match_entries: 282,
        }
    );
    for (unit_index, expected) in expected_lane.chunks_exact(2).enumerate() {
        let base = unit_index * 10;
        assert_eq!(&out[base..base + 2], expected);
    }
    for (index, &byte) in out.iter().enumerate() {
        if index % 10 >= 2 {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
    assert_eq!(&out[0..2], &[0xff, 0x00], "direct literal uses minus one");
    assert_eq!(&out[34 * 10..34 * 10 + 2], &[0x4d, 0xb2]);
    assert_eq!(&out[281 * 10..281 * 10 + 2], &[0x33, 0xcc]);
}

/// Transform tail `0x10fdc00`: two-byte direct literals, matched deltas,
/// and copy runs.
///
/// Provenance: refreshed `capture_transform_tails.py`, Animal_Dragonfly
/// `0x10fdc00` call, entry `0x0a000802`, records
/// `(1,357,10)`, `(159,4,90)`, and `(2,0,0)`. This is the next
/// oracle-proven writer after the early Dragonfly bufB probe: its input
/// records are exactly `0x10fb2e0` call 1 output and it writes `bufB+6`,
/// stride 10.
#[test]
fn transform_tail_delta2_direct_dragonfly_column() {
    let source0 = hex_bytes("00000101010100000000000000000001000000000000020202020000000200000002");
    let source1 = vec![0u8; 290];
    let matches = hex_u32_words(concat!(
            "000000000000000000000000000000000000000000000000000000000000000000000000000000001800000028000000",
            "000000002000000000000000000000004000000000000000100000009800000090000000800000000000000000000000",
            "00000000c000000000000000b80000000000000040000000500000000000000000000000b80000007800000088000000",
            "08000000b0000000c0000000d800000010000000200000005000000000000000d80000005800000061010000b0000000",
            "6801000061010000b80000006901000038010000390100001000000020000000b80000007000000059010000a0000000",
            "0800000040000000400000002800000079010000c901000008000000a8000000b800000098000000a800000058010000",
            "28000000f800000058010000980000005000000048000000100000009000000088000000a80000001000000020000000",
            "b80100007000000000020000080000001800000098000000000000000000000000000000180000000000000000000000",
            "280000002000000018000000000000000000000000000000000000000000000000000000000000006000000050000000",
            "080000008800000028000000000000005800000000000000800000006800000060000000a80000002000000018000000",
            "00000000a800000000000000a00000005800000000000000200000000000000040000000300000002800000060000000",
            "200000003000000000000000b80000000000000068000000a8000000400000003000000060000000f800000030000000",
            "0000000060000000a0000000580000000000000098000000080000001800000068000000f9010000f101000001020000",
            "c1010000d1010000c901000060010000c1010000a0000000a0010000480000005800000008000000e001000028000000",
            "d0000000c101000060000000000000001000000000000000880000006000000048000000480000003000000040000000",
            "e1010000c800000020000000e00000000000000049010000100000000902000018000000100000005000000001020000",
            "1000000000000000e101000018000000e1010000200000000800000018000000a1010000080000005001000028000000",
            "b000000020010000380000006800000030000000d0000000880100003000000018010000c8000000c8000000c0000000",
            "5000000030000000e9020000080000009800000028000000180000007800000020000000a80000002800000000010000",
            "98000000b001000080000000c00500008006000058060000000000000805000018000000280500001800000028000000",
            "6003000050030000c80300008000000070000000000200002003000028000000a8040000680000008805000078060000",
            "e00500005805000028060000d8060000380000003000000030000000b0000000b0050000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "480000000000000050000000480000000000000050000000480000000000000050000000000000000000000000000000",
            "00000000180000002800000000000000f00000000001000000000000f000000000010000000000000000000000000000",
            "000000001800000028000000000000000000000000000000000000001800000028000000810100007901000089010000",
            "810100007901000089010000810100007901000089010000810100007901000089010000810100007901000089010000",
            "480000004000000089010000480000004000000089010000480000004000000089010000810100007901000089010000",
            "81010000200000001800000081010000f8000000f000000081010000f8000000f0000000810100007901000089010000",
            "810100002000000018000000810100007901000089010000810100002000000018000000000000000000000000000000",
            "0000000000000000800a000008000000200400002100000000000000c00a0000080000004900000020000000d90a0000",
            "28040000900300007804000098030000080a000030090000100000008009000078080000380400003004000078060000",
            "20000000b8070000280000007805000088060000a0040000c80600001000000050000000100000005800000018000000",
            "58000000e8070000b1060000080000007000000008050000890600000008000058000000100000004800000058080000",
            "380800001800000070000000280800006808000040000000600000004008000040080000e007000060090000e8070000",
            "28060000300800001006000028000000380000001800000030000000b0060000e8060000200000001807000028000000",
            "4802000000000000680200006802000000000000e00a0000300c00000000000040020000100b00004802000008000000",
            "280000000000000098020000180000001800000018000000a80200008802000088020000200000008002000090020000",
            "a0020000000300001003000098000000c00b0000100000005800000030000000200300002800000098000000e8000000",
            "5000000080000000b000000028000000c8060000880d0000780e0000e80c0000f0060000600c0000a80d0000f0060000",
            "80000000c800000010070000a800000050070000180700005000000000010000b8020000900200006002000058020000",
            "38080000f8090000d0070000d00700001800000058030000880800004803000010090000b8080000e808000040020000",
            "280800006802000018090000800200000000000000000000000000000000000000000000d904000071020000f1040000",
            "e9040000a10200000000000000000000000000000000000000000000",
        ));
    let records = [
        TransformTailRecord {
            literal_count: 1,
            copy_count: 357,
            back_distance: 10,
        },
        TransformTailRecord {
            literal_count: 159,
            copy_count: 4,
            back_distance: 90,
        },
        TransformTailRecord {
            literal_count: 2,
            copy_count: 0,
            back_distance: 0,
        },
    ];
    let mut out = vec![0xee; 5246];

    let usage = transform_tail_delta2_direct_into(
        &mut out,
        TransformTailDelta2DirectSpec {
            output_stride: 10,
            block_index: 0,
            out_offset: 6,
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 34,
            source1: 290,
            source2: 0,
            match_entries: 523,
        }
    );
    let expected_nonzero = [
        (358usize, [1, 1]),
        (359, [1, 1]),
        (432, [1, 1]),
        (435, [1, 1]),
        (436, [0, 1]),
        (509, [2, 2]),
        (510, [2, 2]),
        (512, [0, 2]),
        (513, [1, 1]),
        (514, [0, 1]),
        (516, [1, 1]),
        (518, [2, 2]),
        (519, [2, 2]),
        (522, [0, 2]),
    ];
    for unit_index in 0..523 {
        let expected = expected_nonzero
            .iter()
            .find(|(index, _)| *index == unit_index)
            .map(|(_, bytes)| *bytes)
            .unwrap_or([0, 0]);
        let base = 6 + unit_index * 10;
        assert_eq!(&out[base..base + 2], &expected);
    }
    for (index, &byte) in out.iter().enumerate() {
        if !(6..=7).contains(&(index % 10)) {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
}

/// Transform tail `0x10fdcf0`: three-byte direct literals, matched deltas,
/// and copy runs.
///
/// Provenance: refreshed `capture_transform_tails.py`, Animal_Bear
/// `0x10fdcf0` call, first seven records from entry `0x0c000803`. This
/// covers direct literals, non-zero match-table routing, and copy runs; the
/// replay script covers the full 1-call population.
#[test]
fn transform_tail_delta3_direct_bear_column_prefix() {
    let source0 = hex_bytes("0f0f0f0e0e0e1b1b1b0b0b0b1519191c1c1c151c151c1c1c");
    let source1 =
        hex_bytes("000000000000000000000000000000000000000000000000000000000000000000000000fdfdfd");
    let matches = hex_u32_words(concat!(
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "700100008001000091010000710100008101000091010000710100008101000091010000710100008101000091010000",
            "710100008101000091010000710100008101000091010000710100008101000091010000710100008101000091010000",
            "710100008101000091010000",
        ));
    let records = [
        TransformTailRecord {
            literal_count: 1,
            copy_count: 2,
            back_distance: 12,
        },
        TransformTailRecord {
            literal_count: 1,
            copy_count: 5,
            back_distance: 12,
        },
        TransformTailRecord {
            literal_count: 0,
            copy_count: 3,
            back_distance: 108,
        },
        TransformTailRecord {
            literal_count: 1,
            copy_count: 14,
            back_distance: 12,
        },
        TransformTailRecord {
            literal_count: 1,
            copy_count: 14,
            back_distance: 12,
        },
        TransformTailRecord {
            literal_count: 4,
            copy_count: 2,
            back_distance: 12,
        },
        TransformTailRecord {
            literal_count: 13,
            copy_count: 14,
            back_distance: 12,
        },
    ];
    let expected_lane = hex_bytes(concat!(
            "0f0f0f0f0f0f0f0f0f0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0f0f0f0f0f0f0f0f0f1b1b1b1b1b1b1b1b1b",
            "1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b0b0b0b0b0b0b0b0b0b",
            "0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b1519191c1c1c151c15",
            "1c1c1c1c1c1c1c1c1c0f0f0f0f0f0f0f0f0f0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0f0f0f0f0f0f0f0f0f",
            "181818181818181818181818181818181818181818181818181818181818181818181818181818181818181818",
        ));
    let mut out = vec![0xee; 912];

    let usage = transform_tail_delta3_direct_into(
        &mut out,
        TransformTailDelta3DirectSpec {
            output_stride: 12,
            block_index: 0,
            out_offset: 6,
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 24,
            source1: 39,
            source2: 0,
            match_entries: 75,
        }
    );
    for (unit_index, expected) in expected_lane.chunks_exact(3).enumerate() {
        let base = 6 + unit_index * 12;
        assert_eq!(&out[base..base + 3], expected);
    }
    for (index, &byte) in out.iter().enumerate() {
        if !(6..=8).contains(&(index % 12)) {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
}

/// Transform tail `0x10fde00`: four-byte direct literals, matched deltas,
/// and copy runs.
///
/// Provenance: `capture_transform_tail_10fde00.py`, Animal_Dragonfly
/// current 8 `0x10fde00` call, entry `0x14000804`, records
/// `(1,356,20)` and `(162,4,1740)`. This full one-call population covers
/// the direct literal branch, non-zero match-table routing, and both copy
/// run distances.
#[test]
fn transform_tail_delta4_direct_dragonfly_column() {
    let source0 = hex_bytes(
        "ffffffffffffff00ffffff8effffff8effffffffffffffffffffffffffffff00ffffff8e\
             ffffffffffffffffffffff00ffffff8effffff8effffff00ffffff8effffff8e",
    );
    let source1 = vec![0u8; 584];
    let matches = hex_u32_words(concat!(
            "000000000000000000000000000000000000000000000000000000000000000000000000000000001800000028000000",
            "000000002000000000000000000000004000000000000000100000009800000090000000800000000000000000000000",
            "00000000c000000000000000b80000000000000040000000500000000000000000000000b80000007800000088000000",
            "08000000b0000000c0000000d800000010000000200000005000000000000000d80000005800000061010000b0000000",
            "6801000061010000b80000006901000038010000390100001000000020000000b80000007000000059010000a0000000",
            "0800000040000000400000002800000079010000c901000008000000a8000000b800000098000000a800000058010000",
            "28000000f800000058010000980000005000000048000000100000009000000088000000a80000001000000020000000",
            "b80100007000000000020000080000001800000098000000000000000000000000000000180000000000000000000000",
            "280000002000000018000000000000000000000000000000000000000000000000000000000000006000000050000000",
            "080000008800000028000000000000005800000000000000800000006800000060000000a80000002000000018000000",
            "00000000a800000000000000a00000005800000000000000200000000000000040000000300000002800000060000000",
            "200000003000000000000000b80000000000000068000000a8000000400000003000000060000000f800000030000000",
            "0000000060000000a0000000580000000000000098000000080000001800000068000000f9010000f101000001020000",
            "c1010000d1010000c901000060010000c1010000a0000000a0010000480000005800000008000000e001000028000000",
            "d0000000c101000060000000000000001000000000000000880000006000000048000000480000003000000040000000",
            "e1010000c800000020000000e00000000000000049010000100000000902000018000000100000005000000001020000",
            "1000000000000000e101000018000000e1010000200000000800000018000000a1010000080000005001000028000000",
            "b000000020010000380000006800000030000000d0000000880100003000000018010000c8000000c8000000c0000000",
            "5000000030000000e9020000080000009800000028000000180000007800000020000000a80000002800000000010000",
            "98000000b001000080000000c00500008006000058060000000000000805000018000000280500001800000028000000",
            "6003000050030000c80300008000000070000000000200002003000028000000a8040000680000008805000078060000",
            "e00500005805000028060000d8060000380000003000000030000000b0000000b0050000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "480000000000000050000000480000000000000050000000480000000000000050000000000000000000000000000000",
            "00000000180000002800000000000000f00000000001000000000000f000000000010000000000000000000000000000",
            "000000001800000028000000000000000000000000000000000000001800000028000000810100007901000089010000",
            "810100007901000089010000810100007901000089010000810100007901000089010000810100007901000089010000",
            "480000004000000089010000480000004000000089010000480000004000000089010000810100007901000089010000",
            "81010000200000001800000081010000f8000000f000000081010000f8000000f0000000810100007901000089010000",
            "810100002000000018000000810100007901000089010000810100002000000018000000000000000000000000000000",
            "0000000000000000800a000008000000200400002100000000000000c00a0000080000004900000020000000d90a0000",
            "28040000900300007804000098030000080a000030090000100000008009000078080000380400003004000078060000",
            "20000000b8070000280000007805000088060000a0040000c80600001000000050000000100000005800000018000000",
            "58000000e8070000b1060000080000007000000008050000890600000008000058000000100000004800000058080000",
            "380800001800000070000000280800006808000040000000600000004008000040080000e007000060090000e8070000",
            "28060000300800001006000028000000380000001800000030000000b0060000e8060000200000001807000028000000",
            "4802000000000000680200006802000000000000e00a0000300c00000000000040020000100b00004802000008000000",
            "280000000000000098020000180000001800000018000000a80200008802000088020000200000008002000090020000",
            "a0020000000300001003000098000000c00b0000100000005800000030000000200300002800000098000000e8000000",
            "5000000080000000b000000028000000c8060000880d0000780e0000e80c0000f0060000600c0000a80d0000f0060000",
            "80000000c800000010070000a800000050070000180700005000000000010000b8020000900200006002000058020000",
            "38080000f8090000d0070000d00700001800000058030000880800004803000010090000b8080000e808000040020000",
            "280800006802000018090000800200000000000000000000000000000000000000000000d904000071020000f1040000",
            "e9040000a10200000000000000000000000000000000000000000000",
        ));
    let expected_lane = hex_bytes(concat!(
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff00ffffff8effffff8e",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffff8effffff00ffffff00ffffff8effffff8effffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "ffffffffffffffffffffffffffffffffffffff00ffffff8effffff8effffff00ffffff8effffff8effffff8effffff00",
            "ffffff8effffff00ffffff8effffff8effffff00ffffff00ffffff8e",
        ));
    let records = [
        TransformTailRecord {
            literal_count: 1,
            copy_count: 356,
            back_distance: 20,
        },
        TransformTailRecord {
            literal_count: 162,
            copy_count: 4,
            back_distance: 1740,
        },
    ];
    let mut out = vec![0xee; 15696];

    let usage = transform_tail_delta4_direct_into(
        &mut out,
        TransformTailDelta4DirectSpec {
            output_stride: 20,
            block_index: 0,
            out_offset: 5248,
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 68,
            source1: 584,
            source2: 0,
            match_entries: 523,
        }
    );
    assert_eq!(expected_lane.len(), 523 * 4);
    for (unit_index, expected) in expected_lane.chunks_exact(4).enumerate() {
        let base = 5248 + unit_index * 20;
        assert_eq!(&out[base..base + 4], expected, "unit {unit_index}");
    }
    for (index, &byte) in out.iter().enumerate() {
        let is_lane = (5248..5248 + 523 * 20).contains(&index) && (index - 5248) % 20 < 4;
        if !is_lane {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
}

/// Transform tail `0x10fdfe0`: four-byte direct literals, matched two-u16
/// deltas, and copy runs.
///
/// Provenance: `capture_transform_tail_10fdfe0.py`, Animal_Bass_Boneless
/// current 2 `0x10fdfe0` call, first record `(52,2,416)`. This compact
/// prefix covers the direct literal branch, non-zero match-table routing as
/// little-endian u16 deltas, and a copy run; the replay script covers the
/// captured call 1/1 and rejects byte-wise deltas plus `0x1103ab0`'s
/// previous-row zero-match behavior.
#[test]
fn transform_tail_u16x2_direct_delta_bass_boneless_first_record() {
    let source0 = hex_bytes(concat!(
        "be271260a0328d5fdd1c96605bb3a00c86aeda0578ccda05dceada0587ea28147afcda0550f8620c32af8bd448a9a6ec",
        "e39503d23094caf50d7e93dbea76c9f62c6ed7e1f217ebafd8148ba09538fea450378e8ce85d6b9ffd5f1492b0bb0cc6",
        "5cc234b59fd128da95d9d9b45ff887ec68ec80b4a7f9c7769ad3378c22bc68a45cc234b5",
    ));
    let source1 = hex_bytes(concat!(
        "0000000000000000000000005ca515b49eae80b500000000000000000000000000000000000000000000000000000000",
        "00000000000000000000000000000000000000000000000000000000",
    ));
    let matches = hex_u32_words(concat!(
        "0000000000000000000000001a0000001200000022000000000000000000000030000000000000005000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000010000000",
        "2800000000000000480000000000000042000000420000005a0000006a00000000000000880000003800000018000000",
        "5a00000038000000820000005000000000000000d2000000",
    ));
    let expected_lane = hex_bytes(concat!(
        "be271260a0328d5fdd1c9660be271260dd1c9660a0328d5f5bb3a00c86aeda0539c2ab1478ccda055cd69215dceada05",
        "87ea28147afcda0550f8620c32af8bd448a9a6ece39503d23094caf50d7e93dbea76c9f62c6ed7e1f217ebafd8148ba0",
        "9538fea450378e8ce85d6b9ffd5f1492b0bb0cc65cc234b59fd128da95d9d9b45ff887ec68ec80b4a7f9c77668ec80b4",
        "95d9d9b49ad3378c5cc234b522bc68a45ff887ec68ec80b495d9d9b49fd128da5cc234b5b0bb0cc622bc68a45cc234b5",
        "9ad3378c95d9d9b4a7f9c77668ec80b4e85d6b9ffd5f1492",
    ));
    let records = [TransformTailRecord {
        literal_count: 52,
        copy_count: 2,
        back_distance: 416,
    }];
    let mut out = vec![0xee; 4168];

    let usage = transform_tail_u16x2_direct_delta_into(
        &mut out,
        TransformTailU16x2DirectDeltaSpec {
            output_stride: 16,
            block_index: 0,
            out_offset: 3300,
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 132,
            source1: 76,
            source2: 0,
            match_entries: 54,
        }
    );
    assert_eq!(expected_lane.len(), 54 * 4);
    for (unit_index, expected) in expected_lane.chunks_exact(4).enumerate() {
        let base = 3300 + unit_index * 16;
        assert_eq!(&out[base..base + 4], expected, "unit {unit_index}");
    }
    for (index, &byte) in out.iter().enumerate() {
        let is_lane = (3300..3300 + 54 * 16).contains(&index) && (index - 3300) % 16 < 4;
        if !is_lane {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
}

/// Transform tail `0x10fe4d0`: eight-byte direct literals, matched two-u32
/// deltas, and copy runs.
///
/// Provenance: `capture_phase1_transform_tail_10fe4d0.py`,
/// DgnObj_ElectricStoneWall_A_01 current 2 `0x10fe4d0` call, first two
/// records `(18,1,20)` and `(0,1,60)`. This compact prefix covers direct
/// literals, one non-zero match-table row, a normal copy run, and an observed
/// zero-literal copy record; `verify_transform_tail_10fe4d0.py` replays the
/// full 8-call population.
#[test]
fn transform_tail_u32x2_delta_dgn_wall_prefix() {
    let source0 = hex_bytes(concat!(
        "421b193f608e3d3e88761e3f88d14b3ef6ca1d3f447d3c3eb2648d3f9e80d33e046d8a3fd43ad93e8b378c3f3886e43e",
        "8b66543f88cd553f53c35d3fc7f14f3f5feb553f22f4513f974e753f06ad523f3f5b753fa8ae523fe878053fa8af9b3f",
        "188c023f3e949b3f9ae6043f9a5c9d3f6454a43d2f98793f6076773c6dfc793f2e57b83dab8c7d3f",
    ));
    let source1 = hex_bytes("0000000000000000");
    let matches = hex_u32_words(concat!(
        "000000000000000000000000000000000000000000000000000000000000000000000000000000001800000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000",
    ));
    let records = [
        TransformTailRecord {
            literal_count: 18,
            copy_count: 1,
            back_distance: 20,
        },
        TransformTailRecord {
            literal_count: 0,
            copy_count: 1,
            back_distance: 60,
        },
    ];
    let expected_lane = hex_bytes(concat!(
        "421b193f608e3d3e88761e3f88d14b3ef6ca1d3f447d3c3eb2648d3f9e80d33e046d8a3fd43ad93e8b378c3f3886e43e",
        "8b66543f88cd553f53c35d3fc7f14f3f5feb553f22f4513f974e753f06ad523f53c35d3fc7f14f3f3f5b753fa8ae523f",
        "e878053fa8af9b3f188c023f3e949b3f9ae6043f9a5c9d3f6454a43d2f98793f6076773c6dfc793f2e57b83dab8c7d3f",
        "2e57b83dab8c7d3f6076773c6dfc793f",
    ));
    let units = expected_lane.len() / 8;
    let mut out = vec![0xee; 7604 + (units - 1) * 20 + 8];

    let usage = transform_tail_u32x2_delta_into(
        &mut out,
        TransformTailU32x2DeltaSpec {
            output_stride: 20,
            block_index: 0,
            out_offset: 7604,
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 136,
            source1: 8,
            source2: 0,
            match_entries: 20,
        }
    );
    assert_eq!(matches.len(), 20);
    assert_eq!(expected_lane.len(), 20 * 8);
    for (unit_index, expected) in expected_lane.chunks_exact(8).enumerate() {
        let base = 7604 + unit_index * 20;
        assert_eq!(&out[base..base + 8], expected, "unit {unit_index}");
    }
    for (index, &byte) in out.iter().enumerate() {
        let in_lane = if index >= 7604 {
            let lane_offset = index - 7604;
            lane_offset / 20 < units && lane_offset % 20 < 8
        } else {
            false
        };
        if !in_lane {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
}

#[test]
fn transform_tail_u32x2_delta_allows_observed_zero_copy() {
    let records = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = vec![0xee; 8];

    let usage = transform_tail_u32x2_delta_into(
        &mut out,
        TransformTailU32x2DeltaSpec {
            output_stride: 8,
            block_index: 0,
            out_offset: 0,
            records: &records,
            matches: &[0],
            source0: &hex_bytes("0100000002000000"),
            source1: &[],
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 8,
            source1: 0,
            source2: 0,
            match_entries: 1,
        }
    );
    assert_eq!(&out, &hex_bytes("0100000002000000"));
}

/// Transform tail `0x1101850`: first two-u16 row is a direct seed, later
/// literals delta from the previous row, and copy runs use record byte distance.
///
/// Provenance: `capture_phase1_transform_tail_1101850.py`,
/// Animal_Boar.Boar current 4 `0x1101850` call, first record
/// `(107,6,144)`. `verify_transform_tail_1101850.py` replays the full
/// observed population 1/1.
#[test]
fn transform_tail_u16x2_previous_delta_boar_first_record() {
    let source0 = hex_bytes(concat!(
        "6f9ebfe3cc01fdfaf70103053dfc4406c30300003dfc44068e0575eef70103053efc880cc203bcf9cb01b9f4f7010305",
        "7cf8cb123dfc0000cc010305b905fdfa09fe0305f701baf4c203bcf9000044060000430609fe0305b905fdfa0afe",
        "0305b805fdfa0afe0305f601baf43efc00000000bcf9c20300000000bcf93efc00000afefdfac2030000c96948dd",
        "35fe020509fefefac203bcf972fa460b0afefefac203bcf90000bcf9c20300007cf84406c20379f3c203000035fefd",
        "fa47fa460b0000bdf9f601fdfa48fa460b0000bdf9f601fdfa47fa0305f701fdfa47fa0305f701fdfacb01460b",
        "3efc000000004406c2030000c30300003dfc44063efc0000f70102058e05fefaf60102053efc0000f4ecb7eaee02",
        "7cfc9ef524023a021efe5dfb23ff0b0356ffa506f5fc17f9620176fe9efedffa19148df8a0fe620adbfdd8f743005",
        "dfb23ff0b0356ffa506f5fc17f9620176fe9efeae18d4078df8a0fe610adbfdd9f743005cfb23ff0b0356ffa506",
        "f5fc18f9620176fe9efec8f58ffbef027cfc9ef524023a021efe5cfb23ff0c0356ffa506f5fc17f9620176fe",
        "9efe9f08d4078df8a0fe620adcfd",
    ));
    let expected_slots = hex_bytes(concat!(
        "6f9ebfe33ba0bcde32a2bfe36f9e03ea32a203ea6f9e47f0fda3bcdef4a5bfe332a247f0f4a503eabfa7bcdeb6a9",
        "bfe332a28af66f9e8af63ba08dfbf4a58af6fda38dfbf4a547f0b6a903eab6a947f0b6a98af6bfa78dfb78ad8a",
        "f682ab8dfb3ab18af644af8dfb3ab147f078ad47f078ad03ea3ab103ea3ab1bfe378adbfe382abbcde44afbcde",
        "0d1904bc421706c14b1504bc0d19c0b57f1306c1891104bc4b15c0b54b157caf0d197caf8911c0b54b1539a90",
        "d1939a9421736a489117caf891139a97f1336a4c70d7cafc70d39a9bd0f36a4040a39a9fb0b36a4420639a93",
        "90836a4040a7caf42067caf4206c0b5040ac0b5c70dc0b5040a04bc420604bc390806c1c70d04bcbd0f06c1",
        "fb0b06c1eff8bdabddfb39a87bf15daab5f37ba812ef9ea71df2f4a6c2f8e9a3d9f14ba54ff0e9a32eeb",
        "02b8bbe3a2b61dee7db4f5e5c0b452e1e3b35de439b302eb2eb019e490b18fe22eb03dfb02b8caf3a2b6",
        "2bfe7db404f6c0b460f1e3b36bf439b310fb2eb028f490b19ef22eb066e8bdab55eb39a8f3e05daa2de",
        "37ba889de9ea795e1f4a63ae8e9a351e14ba5c7dfe9a366e8bdabf3e05daa55eb39a82de37ba889de9",
        "ea795e1f4a63ae8e9a351e14ba5c7dfe9a3",
    ));
    let records = [TransformTailRecord {
        literal_count: 107,
        copy_count: 6,
        back_distance: 144,
    }];
    let mut out = vec![0xee; 53776 + 113 * 16];

    let usage = transform_tail_u16x2_previous_delta_into(
        &mut out,
        TransformTailU16x2PreviousDeltaSpec {
            output_stride: 16,
            block_index: 0,
            out_offset: 53776,
            records: &records,
            source0: &source0,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 428,
            source1: 0,
            source2: 0,
            match_entries: 0,
        }
    );
    assert_eq!(expected_slots.len(), 113 * 4);
    for (unit_index, expected) in expected_slots.chunks_exact(4).enumerate() {
        let base = 53776 + unit_index * 16;
        assert_eq!(&out[base..base + 4], expected, "unit {unit_index}");
    }
    for (index, &byte) in out.iter().enumerate() {
        let is_lane = (53776..53776 + 113 * 16).contains(&index) && (index - 53776) % 16 < 4;
        if !is_lane {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
}

/// Transform tail `0x1100c90`: direct three-u16 rows, signed matched
/// deltas, and copy runs.
///
/// Provenance: `capture_transform_tail_1100c90.py`, Animal_Bear
/// `0x1100c90` call, first real record from entry `0x0c00100b`:
/// `(101,2,1212)`. This covers direct source0 rows, non-zero match-table
/// rows with sign bits 0..2 (`0x1100d20`, `0x1100d28`, `0x1100d34`), and
/// byte-distance copy rows; `verify_transform_tail_1100c90.py` replays the
/// full observed population 3/3.
#[test]
fn transform_tail_u16x3_delta_bear_first_record() {
    let source0 = hex_bytes(concat!(
        "b0a0b33cc3400000a53cc2400000a73cbb4012abea3c8640fdabcf3c8f4087acb53c784072a8b03cb940",
        "23a98e3cb3406eaaa83cb740c5ac9d3ca64053aba03cbd4023ac903cba4097b82d2b23baf1b800006d",
        "b933b92d2b93baa9b72f2acfb930b80000d8b887b82f2ad7b925b6c32acfb9f3b60000c6b8b7b7",
        "c32aceb9ddb48a290cba5fb5000005b92db68a29c0b9b5b3a329abba21b40000aab9f5b4472a43",
        "ba69b81928a43ba8b80000093ce2b8312a2a3b88b70e2ad73b00b80000663c42b80e2aa33b28b6d829",
        "023c7eb60000903c58b7d829dc3beab48428a93b98b500007e3c00b684280d3cbab35728353b",
        "43b400001f3cd4b42827a83ba2b4163c16bec3b4ae3c60be7daf463bc5bd59adfd383dbea",
        "0afcb39c6bd9daf8538f9bd3fa9973cb44071a4a93cc340aea7ba3cc3400000973cc240",
    ));
    let source1 = vec![0u8; 294];
    let matches = hex_u32_words(concat!(
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "700100008001000091010000710100008101000091010000710100008101000091010000710100008101000091010000",
            "710100008101000091010000710100008101000091010000710100008101000091010000710100008101000091010000",
            "710100008101000091010000710100008101000091010000710100008101000091010000710100008101000091010000",
            "710100008101000091010000710100008101000091010000710100008101000091010000710100008101000091010000",
            "00000000000000000000000018000000000000000000000000000000",
        ));
    let expected_lane = hex_bytes(concat!(
        "b0a0b33cc3400000a53cc2400000a73cbb4012abea3c8640fdabcf3c8f4087acb53c784072a8b03cb940",
        "23a98e3cb3406eaaa83cb740c5ac9d3ca64053aba03cbd4023ac903cba4097b82d2b23baf1b800006d",
        "b933b92d2b93baa9b72f2acfb930b80000d8b887b82f2ad7b925b6c32acfb9f3b60000c6b8b7b7",
        "c32aceb9ddb48a290cba5fb5000005b92db68a29c0b9b5b3a329abba21b40000aab9f5b4472a43",
        "ba69b81928a43ba8b80000093ce2b8312a2a3b88b70e2ad73b00b80000663c42b80e2aa33b28b6d829",
        "023c7eb60000903c58b7d829dc3beab48428a93b98b500007e3c00b684280d3cbab35728353b",
        "43b400001f3cd4b42827a83ba2b4163c16bec3b4ae3c60be7daf463bc5bd59adfd383dbea",
        "0afcb39c6bd9daf8538f9bd0000a73cbb400000a53cc240b020b33cc340872cb53c7840fd2bcf3c",
        "8f40122bea3c86406e2aa83cb74023298e3cb3407228b03cb940232c903cba40532ba03cbd40c52c",
        "9d3ca64033392d2b93baf13800006db997382d2b23ba87382f2ad7b930380000d8b8a9372f2a",
        "cfb9b737c32aceb9f3360000c6b82536c32acfb92d368a29c0b95f35000005b9dd348a290cbaf534",
        "472a43ba21340000aab9b533a329abbae238312a2a3ba8380000093c69381928a43b42380e2aa33b",
        "00380000663c88370e2ad73b5837d829dc3b7e360000903c2836d829023c003684280d3c98350000",
        "7e3cea348428a93bd4342827a83b433400001f3cba335728353b7d2f463bc5bdc334ae3c60bea2",
        "34163c16be9d2f8538f9bda02fcb39c6bd592dfd383dbe3fa9973cb44071a4a93cc340aea7ba3c",
        "c3403fa9973cb4400000973cc240b0a0b33cc3400000a53cc240",
    ));
    let records = [TransformTailRecord {
        literal_count: 101,
        copy_count: 2,
        back_distance: 1212,
    }];
    let mut out = vec![0xee; 1236];

    let usage = transform_tail_u16x3_delta_into(
        &mut out,
        TransformTailU16x3DeltaSpec {
            output_stride: 12,
            block_index: 0,
            out_offset: 0,
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 312,
            source1: 294,
            source2: 0,
            match_entries: 103,
        }
    );
    for (unit_index, expected) in expected_lane.chunks_exact(6).enumerate() {
        let base = unit_index * 12;
        assert_eq!(&out[base..base + 6], expected);
    }
    for (index, &byte) in out.iter().enumerate() {
        if index % 12 >= 6 {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
}

/// Transform tail `0x1103ab0`: first zero-match row is a seed, later
/// zero-match rows delta from the previous row, and non-zero matches delta
/// from the match-table look-back.
///
/// Provenance: `capture_transform_tail_1103ab0.py`, Animal_Dragonfly
/// current-4 `0x1103ab0` call, first real record `(218,3,1380)`. This
/// compact prefix covers the seed special case, zero-match previous-row
/// deltas, non-zero match deltas, and copy rows; the replay script covers
/// the full observed population 4/4.
#[test]
fn transform_tail_u16x2_delta_dragonfly_first_record() {
    let source0 = hex_bytes(concat!(
            "49af8df526eea804990ae6e6085c6e0a32013df2f109f502d0d6fd0a40fbb0116eddb9e9603ed2fd5dfdf5d77ae652ed",
            "cade20ff1cfa21eb3112b1f8eb2cc9001a477b0637e330f2ff0b57f7f3f625f86bffa7eabe1797f711e096bb69039bfa",
            "b1ea3408a3f253fb0000880e2209e52e5807d7f948f76dfa41f85fe6c1ff290ae20bc5f55af4d237000050dbeafcc4f9",
            "d936c34ae803bb0bc3fca8f61f0cfcfab90b5effb71029faf647424d7e0ebcfc3ef83d023ef87e0a83eeb50acd0f48f9",
        ));
    let source1 = hex_bytes(concat!(
            "000000000000000000000000000000000000000073099aff980c35050000000000000000000000000000000000000000",
            "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000008ef67000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000980c3505720990ff00000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000d3ed45bf49ec73ca0000000000000000",
            "0000d46508015c620000000059fea40b0000000000000000000000000000000000000000000000000000000000000000",
            "2b26e5300000000000000000000000000000000000000000000000000000000000000000000000005129a9289a382821",
            "be4e7f675653454600000000000000000000000066c7d8de000000000000000000000000ad4a49260000000000000000",
            "00000000000000000000000000000000000000000000000000000000000000000000ffff000000000000000059fea40b",
            "dbeea12100000000000000000000000000000000fcfffdff000000000000000000000000a7015cf40000000000000000",
            "0000000000000000000000002b26e5305129a928073bf231000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000005653464600000000",
            "000000009c382921000000000000000000000000000000000000000064c7d7de00000000000000000000000000000000",
            "000000009c382921",
        ));
    let matches = hex_u32_words(concat!(
            "000000000000000000000000000000000000000000000000000000000000000000000000000000001800000028000000",
            "000000002000000000000000000000004000000000000000100000009800000090000000800000000000000000000000",
            "00000000c000000000000000b80000000000000040000000500000000000000000000000b80000007800000088000000",
            "08000000b0000000c0000000d800000010000000200000005000000000000000d80000005800000061010000b0000000",
            "6801000061010000b80000006901000038010000390100001000000020000000b80000007000000059010000a0000000",
            "0800000040000000400000002800000079010000c901000008000000a8000000b800000098000000a800000058010000",
            "28000000f800000058010000980000005000000048000000100000009000000088000000a80000001000000020000000",
            "b80100007000000000020000080000001800000098000000000000000000000000000000180000000000000000000000",
            "280000002000000018000000000000000000000000000000000000000000000000000000000000006000000050000000",
            "080000008800000028000000000000005800000000000000800000006800000060000000a80000002000000018000000",
            "00000000a800000000000000a00000005800000000000000200000000000000040000000300000002800000060000000",
            "200000003000000000000000b80000000000000068000000a8000000400000003000000060000000f800000030000000",
            "0000000060000000a0000000580000000000000098000000080000001800000068000000f9010000f101000001020000",
            "c1010000d1010000c901000060010000c1010000a0000000a0010000480000005800000008000000e001000028000000",
            "d0000000c101000060000000000000001000000000000000880000006000000048000000480000003000000040000000",
            "e1010000c800000020000000e00000000000000049010000100000000902000018000000100000005000000001020000",
            "1000000000000000e101000018000000e1010000200000000800000018000000a1010000080000005001000028000000",
            "b000000020010000380000006800000030000000d0000000880100003000000018010000c8000000c8000000c0000000",
            "5000000030000000e90200000800000098000000",
        ));
    let expected_lane = hex_bytes(concat!(
            "49af8df56f9d35fa08a81be1100489eb4205c6dd330fbbe003e6b8eb43e168fdb1be21e711fdf3e443e168fd03e6b8eb",
            "60e3adc311fdf3e48be345d255c265d1b1be21e7cdb842d2b1be21e7bcb827f5a0b450e6330fbbe064216cd94f4e35da",
            "6995b0e06f9d35faa68065ec4205c6dd41111dd5330fbbe0a0b450e693ab75defeaa1cc943e168fdbcb827f5b1be21e7",
            "b1be21e755c265d18be345d260e3adc38be345d255c265d1feaa1cc9bcc2b3c0cdb842d24aaf97f508a81be16f9d35fa",
            "100489eb330fbbe04205c6dd03e6b8eb11fdf3e48be345d211fdf3e403e6b8eb43e168fdbcc2b3c055c265d160e3adc3",
            "60e3adc38be345d211fdf3e455c265d1cdb842d2b1be21e7b1be21e7a0b450e6bcb827f54205c6dd330fbbe041111dd5",
            "a0b450e6feaa1cc993ab75de43e168fdb1be21e7bcb827f5b1be21e78be345d255c265d160e3adc355c265d18be345d2",
            "330fbbe041111dd564216cd964216cd941111dd5330fbbe044ef519cadf2ec965edd209f17dd965bbacfe956bacf7165",
            "f6de5f6117dd965bbacf7165dcd8569434e02d8e7cd79a88bdcff96e7ecf227960dbe76ebacfb9a6bacfbdbc1fdef2bd",
            "1fdef2bdb7dbc4aabacfb9a6bacf09827cd79a8866d45e82bacf7165bdcff96e60dbe76ef6de5f61bacf716560dbe76e",
            "3912aab95f0612bf470acdca7ecf227966d45e8229d10679470acdca6616c9c53912aab97ecf227929d1067960dbe76e",
            "3912aab96616c9c51f2227c5cd0043b184116cab000d86a3b42ddec8b62e2cb51f2227c57ecf2279bacf098266d45e82",
            "5c1ca0cf6616c9c5470acdca84116cab022028a8d61b4f9fd61b4f9f022028a8b62e2cb544ef519c5edd209fadf2ec96",
            "34e02d8edcd856947cd79a88bdcff96e60dbe66e7ecf2279bacfb9a6b7dbc4aa1fdef2bd1fdef2bdbacfbdbcbacfb9a6",
            "bacf098262d45b827cd79a88bacfd78a7cd79a88bacf1893dcd856945edd209fbacfb9a6bacf09827cd79a88bacfd78a",
            "3912aab95f0612bfcd0043b1b42ddec8371c93d35c1ca0cf371c93d3470acdca5c1ca0cf470acdca3912aab96616c9c5",
            "3912aab90622f2b21f2227c53912aab984116cab0622f2b20622f2b284116cab022028a8022028a8b62e2cb50622f2b2",
            "cd0043b1fe0c84a384116cab1f2227c5b62e2cb5b42ddec87ecf227962d45b82bacf09825c1ca0cf470acdca6616c9c5",
            "84116cabfe0c84a3d61b4f9fd61b4f9f022028a8",
        ));
    let records = [TransformTailRecord {
        literal_count: 218,
        copy_count: 3,
        back_distance: 1380,
    }];
    let mut out = vec![0xee; 4420];

    let usage = transform_tail_u16x2_delta_into(
        &mut out,
        TransformTailU16x2DeltaSpec {
            output_stride: 20,
            block_index: 0,
            out_offset: 0,
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 192,
            source1: 680,
            source2: 0,
            match_entries: 221,
        }
    );
    for (unit_index, expected) in expected_lane.chunks_exact(4).enumerate() {
        let base = unit_index * 20;
        assert_eq!(&out[base..base + 4], expected, "unit {unit_index}");
    }
    for (index, &byte) in out.iter().enumerate() {
        if index % 20 >= 4 {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
}

/// Transform tail `0x110aac0`: signed i8 x/y direct rows, sqrt-derived z
/// byte, source2 sign byte, and copy rows.
///
/// Provenance: `capture_transform_tail_110aac0.py`, Animal_Bear
/// `0x110aac0` call, first real record `(7,2,16)`. This compact prefix
/// covers direct reconstruction, a negative z sign byte, and copy rows; the
/// replay script covers the full observed population 3/3.
#[test]
fn transform_tail_i8x2_normal_bear_first_record() {
    let source0 = hex_bytes("bc1eb4f6c222e38de08def89ca94");
    let source1 = hex_bytes("000001ff0101ff");
    let source2 = hex_bytes("00000000000001");
    let expected_lane = hex_bytes("bc1e67b4f665c2226ae38d2ce08d2cef892aca94daca94daca94da");
    let records = [TransformTailRecord {
        literal_count: 7,
        copy_count: 2,
        back_distance: 16,
    }];
    let mut out = vec![0xee; 144];

    let usage = transform_tail_i8x2_normal_into(
        &mut out,
        TransformTailI8x2NormalSpec {
            output_stride: 16,
            block_index: 0,
            out_offset: 0,
            records: &records,
            source0: &source0,
            source1: &source1,
            source2: &source2,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 14,
            source1: 7,
            source2: 7,
            match_entries: 9,
        }
    );
    for (unit_index, expected) in expected_lane.chunks_exact(3).enumerate() {
        let base = unit_index * 16;
        assert_eq!(&out[base..base + 3], expected, "unit {unit_index}");
    }
    for (index, &byte) in out.iter().enumerate() {
        if index % 16 >= 3 {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
}

/// Transform tail `0x110aba0`: packed signed 10-bit x/y direct rows,
/// sqrt-derived z lane, source2 sign byte, copy rows, and a zero-literal record.
///
/// Provenance: `phase1_direction_zero_writer_loop_capture.json`,
/// Armor_009.Armor_009_Head `0x110aba0` row, first eight records
/// `(1,2,12)`, `(1,2,12)`, `(1,2,12)`, `(0,3,72)`, then four `(1,3,12)`.
/// `verify_transform_tail_110aba0.py` replays the full observed population 2/2.
#[test]
fn transform_tail_pack10x3_normal_direction_zero_writer() {
    let source0 = hex_bytes("8b039103bd0300008b036f000400730098031d00bb00250220025c03");
    let source1 = hex_bytes("0000000000000000000002000200");
    let source2 = hex_bytes("00000000000001");
    let expected_lane = hex_bytes(concat!(
        "8b475e1e8b475e1e8b475e1ebd03b01fbd03b01fbd03b01f8bbf511e",
        "8bbf511e8bbf511ebd03b01fbd03b01fbd03b01f04cc211f04cc211f",
        "04cc211f04cc211f9877301f9877301f9877301f9877301fbb949801",
        "bb949801bb949801bb94980120720d3c20720d3c20720d3c20720d3c",
    ));
    let records = [
        TransformTailRecord {
            literal_count: 1,
            copy_count: 2,
            back_distance: 12,
        },
        TransformTailRecord {
            literal_count: 1,
            copy_count: 2,
            back_distance: 12,
        },
        TransformTailRecord {
            literal_count: 1,
            copy_count: 2,
            back_distance: 12,
        },
        TransformTailRecord {
            literal_count: 0,
            copy_count: 3,
            back_distance: 72,
        },
        TransformTailRecord {
            literal_count: 1,
            copy_count: 3,
            back_distance: 12,
        },
        TransformTailRecord {
            literal_count: 1,
            copy_count: 3,
            back_distance: 12,
        },
        TransformTailRecord {
            literal_count: 1,
            copy_count: 3,
            back_distance: 12,
        },
        TransformTailRecord {
            literal_count: 1,
            copy_count: 3,
            back_distance: 12,
        },
    ];
    let mut out = vec![0xee; 28 * 12];

    let usage = transform_tail_pack10x3_normal_into(
        &mut out,
        TransformTailPack10x3NormalSpec {
            output_stride: 12,
            block_index: 0,
            out_offset: 0,
            records: &records,
            source0: &source0,
            source1: &source1,
            source2: &source2,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailPack10Usage {
            source0: 28,
            source1: 14,
            source2: 7,
            source3: 0,
            match_entries: 28,
        }
    );
    for (unit_index, expected) in expected_lane.chunks_exact(4).enumerate() {
        let base = unit_index * 12;
        assert_eq!(&out[base..base + 4], expected, "unit {unit_index}");
    }
    for (index, &byte) in out.iter().enumerate() {
        if index % 12 >= 4 {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
}

/// Transform tail `0x1106250`: f16x3 rows predicted from zero, previous row,
/// and the aux table, then updated through helper `0x110c110`.
///
/// Provenance: `phase1_direction_zero_writer_loop_capture.json`, Armor_009
/// `0x1106250` row, first seven records. `verify_transform_tail_1106250.py`
/// replays the full observed population 1/1 and this compact prefix covers
/// aux/previous/zero prediction, mantissa-delta, mantissa-direct, and copy.
#[test]
fn transform_tail_f16x3_predict_direction_zero_writer() {
    let records = hex_width_records(concat!(
        "0300010018000000010001002000000000000100180000000000010028000000",
        "010001001800000000000100100000000a00010080000000",
    ));
    let records: Vec<_> = records
        .iter()
        .map(|record| TransformTailRecord {
            literal_count: (record[0] & 0xffff) as u16,
            copy_count: (record[0] >> 16) as u16,
            back_distance: record[1] as usize,
        })
        .collect();
    let source0 = hex_bytes(
        "000000000000000000000000000000000000000100000000000000000000000000000000000000000000000100",
    );
    let source1 = hex_bytes(
        "0b080b001f00000100001e0000010000000001011f1c1f0104010000000104011f1d1f0103010001000000",
    );
    let source2 = hex_bytes("031e");
    let source3 =
        hex_bytes("64013000340032005c000c00330031000701a8012300c600b5013800a4002c0004014400");
    let source4 = hex_bytes(concat!(
        "24016301ca03740163014d00050091027f00b500cf017f00860354028000b400",
        "a000750060009c01b9008c0357004d00500096027401",
    ));
    let aux_table = hex_u64_words(concat!(
        "0000000000000000000000000000000000008000000800000000000000000000",
        "0000000000000000000080000008000000000000000000000000000000000000",
        "0000800000080000000000000000000000000000000000000000800000080000",
        "0000000000000000000000000000000000008000000800000300400000100000",
        "0000000000000000000000000000000000008000000800000300400000100000",
        "00000000000000000000000000000000",
    ));
    let expected_rows = hex_bytes(concat!(
        "242d6321ca2fd62d741de22ff02d6321fb2f242d6321ca2f522d4d18d02f",
        "d62d741de22f242d6321ca2ff02d6321fb2fd62d0524e22f242d6321ca2f",
        "d62d0524e22f522dd924d02f91327fb0b530cf2d7fa0862f543280b0b430",
        "b82db4a3a02fa03075b060309c2db9a48c2f57304db050305c2d96a6822f",
        "de2d741da42fd62d741de22f",
    ));
    let mut out = vec![0xee; 22 * 8];

    let usage = transform_tail_f16x3_predict_into(
        &mut out,
        TransformTailF16x3PredictSpec {
            output_stride: 8,
            block_index: 0,
            out_offset: 0,
            records: &records,
            aux_table: &aux_table,
            source0: &source0,
            source1: &source1,
            source2: &source2,
            source3: &source3,
            source4: &source4,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailF16x3PredictUsage {
            source0: 45,
            source1: 43,
            source2: 2,
            source3: 36,
            source4: 54,
            aux_entries: 21,
        }
    );
    for (unit_index, expected) in expected_rows.chunks_exact(6).enumerate() {
        let base = unit_index * 8;
        assert_eq!(&out[base..base + 6], expected, "unit {unit_index}");
    }
    for (index, &byte) in out.iter().enumerate() {
        if index % 8 >= 6 {
            assert_eq!(byte, 0xee, "padding byte {index} changed");
        }
    }
}

/// Transform tail `0x110ae30`: signed i8 x/y direct rows, sqrt-derived z,
/// byte-wise matched deltas, and match-bit sign toggles.
///
/// Provenance: `capture_phase1_transform_tail_110ae30.py`,
/// Animal_Fish_Cave.Fish_Cave second `0x110ae30` call, entry `0x0c000803`,
/// record `(92,0,0)`. `verify_transform_tail_110ae30.py` replays the full
/// observed population 2/2.
#[test]
fn transform_tail_i8x3_normal_delta_fish_cave_second_call() {
    let source0 = hex_bytes(concat!(
        "04f8a7d604e797e4f5d987e596eb8ce085ec90d783ef87df97e495ec8ce28fdc85e40110fd2749185d19fa106914740d",
        "7a0f731266186517561a5e196a157a107413651999e19de689de93eaf610efd702f588e296049b0487ff8900940381fa",
        "87fe81f981fa89ed8cef9af38fee9af28def54127808730f78085413730d",
    ));
    let source1 = hex_bytes(concat!(
        "000001000000ffff00fefefeff00ff0002000000ff00ff01feffff01000000010100ff010001000000ff000000000009",
        "0006060001ff0000ffff00ff000001",
    ));
    let source2 = hex_bytes(concat!(
        "000000000000000000000000010101010101010101010101010101010100000000000101010101010100010101010101",
        "010001000000010101000000010101",
    ));
    let source3 = hex_bytes(concat!(
        "00000000000200000600001000000000000000ff0000fffc00000000fff200fffc0000000001000001ff0002010000ee",
        "0002000000feff02000000fc000000000000000000000000000000000000000000000000000000",
    ));
    let matches = hex_u32_words(concat!(
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "5c0000006800000060000000000000006000000064000000000000000000000064000000000000006400000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "600000005c00000068000000000000006000000000000000640000000000000000000000640000000000000064000000",
        "000000000000000000000000000000000000000000000000000000002400000018000000440000000000000028000000",
        "640000004800000000000000000000000000000000000000000000000000000000000000000000000000000044000000",
        "540000004c00000044000000540000004400000054000000440000005400000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000",
    ));
    let expected_lane = hex_bytes(concat!(
        "04f87fa7d65004e77d97e442f5d97887e51c96eb428ce02885ec1890d72a83ef0d87df12a7d6b004f88104e78397e4bf",
        "f5d98887e5e495ecbf8ce2d785ece88fdcd383eef385e4ef011082fd278749189b5d19aefa10826914bc740dcd7a0fe2",
        "7312ce6618b96517b6561aa6010f7e491865fd26795e1952fa0f7e6a1543740d337a1020741331661947651949561b59",
        "99e1bd9de6b489dee493eac2f61082efd78902f58193ec3fefd7779de84c88e21c02f57f98e343f6107e9604ba9b04b3",
        "87ffd98900d49403bd81faf787fed981f90681fafa9b044d96044687ff2794034389002c87fe2781fa0981fa0681f9fa",
        "89ed288cef329af34a8feec99af2b68defce54125d780829730f337808d75413a3730dcb",
    ));
    let records = [TransformTailRecord {
        literal_count: 92,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = vec![0xee; 24503];

    let usage = transform_tail_i8x3_normal_delta_into(
        &mut out,
        TransformTailI8x3NormalDeltaSpec {
            output_stride: 12,
            block_index: 0,
            out_offset: 23408,
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
            source2: &source2,
            source3: &source3,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailI8x3NormalDeltaUsage {
            source0: 126,
            source1: 63,
            source2: 63,
            source3: 87,
            match_entries: 92,
        }
    );
    for (unit_index, expected) in expected_lane.chunks_exact(3).enumerate() {
        let base = 23408 + unit_index * 12;
        assert_eq!(&out[base..base + 3], expected, "unit {unit_index}");
    }
    for (index, &byte) in out[23408..].iter().enumerate() {
        if index % 12 >= 3 {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
}

/// Transform tail `0x1103840`: packed 10-10-10 seed row, previous-row
/// deltas, matched look-backs, and copy rows.
///
/// Provenance: `capture_phase1_transform_tail_1103840.py`,
/// Item_Ore_L.Item_Ore_L_Dummy current-1 `0x1103840` call, first record
/// `(32,1,96)`. `verify_transform_tail_1103840.py` replays the full
/// observed population 1/1.
#[test]
fn transform_tail_pack10x3_previous_delta_ore_first_record() {
    let source0 = hex_bytes(concat!(
        "6e008003e201c30322001200cb03d403f803ea0124013e02160065031000ea03d8035d006a0025005802be03d503a700",
        "a000be03fb025403550062010500cd03fc03fc03210014001702d803f4017203f1026403a7007a01a100f7013903a701",
        "cf037403b000f903e4037a0019009d030b00e90394003d001f006003e603090017004f0033009203f503990112038f03",
        "07000600a303f903fa035400fd030b008e030400f50373001b000c008b03",
    ));
    let source1 = hex_bytes("000000000000000000000000000000000000");
    let matches = hex_u32_words(concat!(
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000004800000058000000000000001800000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000",
    ));
    let expected_slots = hex_bytes(concat!(
        "6e002e1e31884e1ffcdbcd1ee669a202fcfdaf03e65d7f0950f2ff2e0e466f39ae3e1e2902923f3f07c6fe3e034a3",
        "f001aa87e1f8c6fba153354c01f2a3a3d3a0e466f3950f2ff2e1fc2fd390e466f3907d60e01204abd01099a8f05281",
        "aed033176dd0864be2b08fd071801041c483bfd078800fa336839fe07980019344839fd071801",
    ));
    let records = [TransformTailRecord {
        literal_count: 32,
        copy_count: 1,
        back_distance: 96,
    }];
    let mut out = vec![0xee; 33 * 16];

    let usage = transform_tail_pack10x3_previous_delta_into(
        &mut out,
        TransformTailPack10x3PreviousDeltaSpec {
            output_stride: 16,
            block_index: 0,
            out_offset: 0,
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 174,
            source1: 18,
            source2: 0,
            match_entries: 33,
        }
    );
    assert_eq!(expected_slots.len(), 33 * 4);
    for (unit_index, expected) in expected_slots.chunks_exact(4).enumerate() {
        let base = unit_index * 16;
        assert_eq!(&out[base..base + 4], expected, "unit {unit_index}");
    }
    assert_eq!(
        &out[32 * 16..32 * 16 + 4],
        &out[26 * 16..26 * 16 + 4],
        "copy back-distance is in bytes"
    );
    for (index, &byte) in out.iter().enumerate() {
        if index % 16 >= 4 {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
}

/// Transform tail `0x110afb0`: packed 10-10-10 direct rows, sqrt-derived
/// third component, signed matched deltas, and copy runs.
///
/// Provenance: `capture_transform_tail_110afb0.py`, Animal_Dragonfly
/// `0x110afb0` full call, entry `0x14000a13`, records `(103,1,300)` and
/// `(419,0,0)`. This is the next target named by the Dragonfly sparse bufB
/// oracle after `0x1100c90`; `verify_transform_tail_110afb0.py` replays the
/// full observed population 3/3.
#[test]
fn transform_tail_pack10x3_delta_dragonfly_column() {
    let source0 = hex_bytes(concat!(
            "0000020200000202c10142030000ec0200000601f9012200af01470300001002ba011f030000db030000d90152014401",
            "8e013a011c010d010000ff010000fe01fb013700000007020000fe01000078010000e1000000ef011d01b50227019903",
            "9601da0200006502000014007001ef0399018303ea00e20000009e01a1002001000021020000c9011a00b6017f013a00",
            "a80190033200fa01a9017e03d40122009e009e0108014b022400e401000049010000200000000102b40267010000f501",
            "b901fe02d501420302017502d5013803d90151033f01d002ab01c300a701ad005d010300a301da03a001cd0360013d03",
            "f801fb03f201c4039e01fa0258016a01a801de00b7010601e0015603e3015b0352012a039001e7039c016d0056013d00",
            "1800d5012500d501e50190003202cb003d00b2010500ba01d5017a003602df000b00ff010d00ff010d00ff01fe01e003",
            "fb01f6030000f8010900fe010c00ff010000060200000102e003fe01e003fe01df03fe01df03fe01dc03fe012100fe01",
            "2000fe012000fe012100fe012400fe01",
        ));
    let source1 = hex_bytes(concat!(
            "0600fb0301000000000003000000ff030000000000000000ff03000011000500fc030300060000000000020001000000",
            "ff03000000000000000000000000000001000000000001000000fb03000001000000fb03ff0300000000010001000000",
            "f703fe0300000100ff0300000000ff030000000001000000010001000000ff030000040002000100000000000100ff03",
            "00000200fe03ff0300000000ff030100080004000000060001000000f7030b0002000e000e0011000c000a0008000c00",
            "11000e000a000800",
        ));
    let source2 = hex_bytes(concat!(
            "000001010101000001000000010100010101010101010101000101000000010100000000000100010001010000010100",
            "000001000001000000000000000000010001010101000000000000000000000001000001010101000100010101010001",
            "01010100",
        ));
    let source3 = sparse_payload(2532, &[(1020, "ff03")]);
    let matches = hex_u32_words(concat!(
            "000000000000000000000000000000000000000000000000000000000000000000000000000000001800000028000000",
            "000000002000000000000000000000004000000000000000100000009800000090000000800000000000000000000000",
            "00000000c000000000000000b80000000000000040000000500000000000000000000000b80000007800000088000000",
            "08000000b0000000c0000000d800000010000000200000005000000000000000d80000005800000061010000b0000000",
            "6801000061010000b80000006901000038010000390100001000000020000000b80000007000000059010000a0000000",
            "0800000040000000400000002800000079010000c901000008000000a8000000b800000098000000a800000058010000",
            "28000000f800000058010000980000005000000048000000100000009000000088000000a80000001000000020000000",
            "b80100007000000000020000080000001800000098000000000000000000000000000000180000000000000000000000",
            "280000002000000018000000000000000000000000000000000000000000000000000000000000006000000050000000",
            "080000008800000028000000000000005800000000000000800000006800000060000000a80000002000000018000000",
            "00000000a800000000000000a00000005800000000000000200000000000000040000000300000002800000060000000",
            "200000003000000000000000b80000000000000068000000a8000000400000003000000060000000f800000030000000",
            "0000000060000000a0000000580000000000000098000000080000001800000068000000f9010000f101000001020000",
            "c1010000d1010000c901000060010000c1010000a0000000a0010000480000005800000008000000e001000028000000",
            "d0000000c101000060000000000000001000000000000000880000006000000048000000480000003000000040000000",
            "e1010000c800000020000000e00000000000000049010000100000000902000018000000100000005000000001020000",
            "1000000000000000e101000018000000e1010000200000000800000018000000a1010000080000005001000028000000",
            "b000000020010000380000006800000030000000d0000000880100003000000018010000c8000000c8000000c0000000",
            "5000000030000000e9020000080000009800000028000000180000007800000020000000a80000002800000000010000",
            "98000000b001000080000000c00500008006000058060000000000000805000018000000280500001800000028000000",
            "6003000050030000c80300008000000070000000000200002003000028000000a8040000680000008805000078060000",
            "e00500005805000028060000d8060000380000003000000030000000b0000000b0050000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "480000000000000050000000480000000000000050000000480000000000000050000000000000000000000000000000",
            "00000000180000002800000000000000f00000000001000000000000f000000000010000000000000000000000000000",
            "000000001800000028000000000000000000000000000000000000001800000028000000810100007901000089010000",
            "810100007901000089010000810100007901000089010000810100007901000089010000810100007901000089010000",
            "480000004000000089010000480000004000000089010000480000004000000089010000810100007901000089010000",
            "81010000200000001800000081010000f8000000f000000081010000f8000000f0000000810100007901000089010000",
            "810100002000000018000000810100007901000089010000810100002000000018000000000000000000000000000000",
            "0000000000000000800a000008000000200400002100000000000000c00a0000080000004900000020000000d90a0000",
            "28040000900300007804000098030000080a000030090000100000008009000078080000380400003004000078060000",
            "20000000b8070000280000007805000088060000a0040000c80600001000000050000000100000005800000018000000",
            "58000000e8070000b1060000080000007000000008050000890600000008000058000000100000004800000058080000",
            "380800001800000070000000280800006808000040000000600000004008000040080000e007000060090000e8070000",
            "28060000300800001006000028000000380000001800000030000000b0060000e8060000200000001807000028000000",
            "4802000000000000680200006802000000000000e00a0000300c00000000000040020000100b00004802000008000000",
            "280000000000000098020000180000001800000018000000a80200008802000088020000200000008002000090020000",
            "a0020000000300001003000098000000c00b0000100000005800000030000000200300002800000098000000e8000000",
            "5000000080000000b000000028000000c8060000880d0000780e0000e80c0000f0060000600c0000a80d0000f0060000",
            "80000000c800000010070000a800000050070000180700005000000000010000b8020000900200006002000058020000",
            "38080000f8090000d0070000d00700001800000058030000880800004803000010090000b8080000e808000040020000",
            "280800006802000018090000800200000000000000000000000000000000000000000000d904000071020000f1040000",
            "e9040000a10200000000000000000000000000000000000000000000",
        ));
    let expected_lane = hex_bytes(concat!(
            "000868020008b801c1096d3600b02b2500189424f989703baf1dbd0c0040a807ba7d5c38006cef1f0040a807af1dbd0c",
            "0064170c006cef1f5211d50c8ee9143cba7d5c381c35742bba7d5c3800086802c1096d36f989703b00fc170100f8b73d",
            "fbdd403e0008b801001cf83a0018942400f8a73df989703bc1096d3600e0652a008453230040a80700086802ba7d5c38",
            "ba7d5c388ee9143c5211d50c0064170c5211d50c8ee9143c0084532300bcf7371c35742b000868023f0a6d360008b801",
            "00b02b25078a703b00189424511ebd0c006cef1fae12d50c006cef1f511ebd0c0040a80700bcf73772ea143c0064170c",
            "0064170cae12d50c006cef1f72ea143ce436742b467e5c38467e5c383f0a6d360008680200189424078a703b00f8a73d",
            "3f0a6d360084532300e0652a0040a807467e5c3800086802467e5c38ae12d50c72ea143c0064170c72ea143cae12d50c",
            "f989703b00f8a73d00fc170100fc170100f8a73d078a703b1dd56a2f2765ce2696692b061dd56a2f0094092d00501020",
            "2765ce261dd56a2f0050102070bd2f16990d8e11ea88a3180078462d00f8a73da180a4270084380b0094092d1dd56a2f",
            "1dd56a2f96692b060084380b0024570eea88a3181ad86610005010200078462da180a4272765ce2600501020a180a427",
            "7fe9e014990d8e11a8416e1000f8a73d1ad8661032e8273da8416e10a9f9cd0f7fe9e01400f8a73d32e8273da180a427",
            "7fe9e014a9f9cd0fd4895033ea88a3189e78e60f1ad866102765ce26a180a427d489503300f8a73d0024570e1ad86610",
            "082d093fa9f9cd0fa8416e109e78e60f2490173632e8273d32e8273d24901736a180a427e3d66a2f6a6a2b06d966ce26",
            "670e8e1190be2f16168ba3180078462d5f83a42700f8a73d0084380b6a6a2b06e3d66a2fe3d66a2f0094092d0084380b",
            "0024570ee6db6610168ba31800247518168ba3180080e01f90be2f166a6a2b060084380b0024570e168ba31800247518",
            "81eae014670e8e11168ba318d966ce260004f83ff82e093f0004f83f58426e10f82e093f58426e1081eae01457facd0f",
            "81eae014b49eb5362c8a503381eae014627be60fb49eb536b49eb536627be60fdc931736dc9317365f83a427b49eb536",
            "168ba318e6db6610627be60f2c8a50335f83a427d966ce2600f8a73de6db66100024570ef82e093f58426e1057facd0f",
            "627be60fe6db6610cdeb273dcdeb273ddc931736627be60fcdeb273d5f83a427dc931736168ba318627be60f81eae014",
            "d966ce266a6a2b0658426e100008b801fbdd403ec1096d3600d4570600e0652ac1096d363f0a6d3600e0652a00d45706",
            "7fe9e014ea88a318990d8e1181eae01458426e10670e8e11a8416e10990d8e1170bd2f16c1096d36008453231c35742b",
            "e436742b72ea143c00bcf737ba7d5c38c1096d361c35742be436742b3f0a6d36467e5c38b9f90b00d5095d0402d5c933",
            "d5e13c02d9451d053f41db2fab0da30ca7b5420e5d0d5017a3692f12a0355f1260f5bc13f8ed5f05f2113f069ee91b09",
            "ab0da30c58a95539a7b5420ea3692f12a879330ba0355f12f8ed5f05b719c43ff2113f06e0593d3de36d6d3e52a92c2c",
            "909ddf13e36d6d3ee0593d3d9cb5b111d5095d04b9f90b0056f56017d9451d05d5e13c021854970c2554970ce5416204",
            "322ef3042554970c1854970c3dc8761005e80610d5e9110a367e930205e806103dc8761047fa0b00fed6c9332b0a5d04",
            "2be23c02c142db2f27461d05550ea30ca30e501759b6420e5d6a2f12a0f6bc1360365f1208ee5f0562ea1b090e123f06",
            "550ea30c59b6420ea8aa55395d6a2f1260365f12587a330b08ee5f050e123f06491ac43f205a3d3daeaa2c2c1d6e6d3e",
            "709edf13205a3d3d1d6e6d3e64b6b11147fa0b002b0a5d04aaf660172be23c0227461d05e857970c1b426204db57970c",
            "ce2df304e857970cdb57970cc3cb76102bea110afbeb0610ca7d9302c3cb7610fbeb06100bfc873f0dfc47000dfc0700",
            "fe81af3ffbd90f3c001cf83a001cf83afbdd403e05da0f3c00e0c73a00f8b73d00f8b73d0282af3f00e0c73a05de403e",
            "00d457063f0a6d360008b801467e5c38511ebd0cae12d50c511ebd0c0040a80796692b06a8416e1070bd2f160084380b",
            "96692b062765ce26a8416e100024570e00247518ea88a3180080e01fea88a31870bd2f16ea88a3180084380b70bd2f16",
            "2765ce26082d093f0004f83f0004f83fa8416e107fe9e0144c9db5369e78e60fea88a3189e78e60f082d093fd4895033",
            "a9f9cd0f082d093f2765ce2632e8273d1ad866109e78e60f4c9db53624901736a180a427e3d66a2f005010200094092d",
            "5f83a4270078462dd966ce2600501020e3d66a2fd966ce265f83a427f82e093f2c8a5033d966ce26b49eb5365f83a427",
            "0dfc070009f8a73e0bfc873f0dfc47000cfcb700078a703b00b02b250018783b00f8b73d00fc17010282af3f0282af3f",
            "0018783b0004e800001cf83a0018783b0004e800001cf83a05da0f3c00e0c73a05de403e001cf83a0008b8013f0a6d36",
            "00d45706fbd90f3cfe81af3f00f8b73df989703b00f8b73d00e0c73afbd90f3cfbdd403ef989703b0018783b00b02b25",
            "fe81af3f001cf83a0004e8000018783bba7d5c385211d50caf1dbd0c006cef1f00bcf7370064170c8ee9143c1c35742b",
            "fbdd403e00d45706c1096d3600e0c73a00845323e436742b00bcf7373f0a6d367fe9e014d48950334c9db536a180a427",
            "6a6a2b0690be2f1658426e10670e8e1190be2f160080e01f168ba3180084380b00f8a73dcdeb273de6db66105f83a427",
            "81eae0142c8a503357facd0ff82e093fe0fb273fe0fbf73edffb473fdffb673fdcfb8700f3ff4700f4ffb700f5ff873f",
            "f3ff0700f7fba73e21f8473f20f8f73e20f8273f21f8673f24f88700",
        ));
    let records = [
        TransformTailRecord {
            literal_count: 103,
            copy_count: 1,
            back_distance: 300,
        },
        TransformTailRecord {
            literal_count: 419,
            copy_count: 0,
            back_distance: 0,
        },
    ];
    let mut out = vec![0xee; 10460];

    let usage = transform_tail_pack10x3_delta_into(
        &mut out,
        TransformTailPack10x3DeltaSpec {
            output_stride: 20,
            block_index: 0,
            out_offset: 0,
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
            source2: &source2,
            source3: &source3,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailPack10Usage {
            source0: 400,
            source1: 200,
            source2: 100,
            source3: 2532,
            match_entries: 523,
        }
    );
    for (unit_index, expected) in expected_lane.chunks_exact(4).enumerate() {
        let base = unit_index * 20;
        assert_eq!(&out[base..base + 4], expected);
    }
    for (index, &byte) in out.iter().enumerate() {
        if index % 20 >= 4 {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
}

/// Transform tail `0x10fbdc0`: three-byte direct and matched deltas.
///
/// Provenance: `capture_transform_tails.py`, Animal_Bear `0x10fbdc0` call,
/// first two records from entry `0x0c000803`: `(1,41,12)` and
/// `(52,87,12)`. This covers direct literals, match-table literals, and
/// the copy loop; the replay script covers the full 1-call population.
#[test]
fn transform_tail_delta3_bear_direct_match_and_copy() {
    let source0 = hex_bytes("00000000000000");
    let source1 = hex_bytes("0000990000003300000000000000");
    let source2 = hex_bytes(concat!(
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        ));
    let matches = hex_u32_words(concat!(
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "700100008001000091010000710100008101000091010000710100008101000091010000710100008101000091010000",
            "710100008101000091010000710100008101000091010000710100008101000091010000710100008101000091010000",
            "710100008101000091010000710100008101000091010000710100008101000091010000710100008101000091010000",
            "710100008101000091010000710100008101000091010000710100008101000091010000710100008101000091010000",
            "000000000000000000000000180000000000000000000000000000002900000039000000490000003000000010000000",
            "390000003800000000000000000000000000000000000000000000000000000000000000000000000000000028000000",
            "280000000000000000000000000000000000000000000000000000002800000000000000300000000000000000000000",
            "000000000000000000000000280000000000000030000000000000000000000000000000000000000000000028000000",
            "000000003000000000000000000000000000000000000000000000002800000000000000300000000000000000000000",
            "000000000000000000000000180000000000000030000000000000000000000000000000000000001800000000000000",
            "000000003800000000000000000000000000000000000000000000001800000000000000380000000000000028040000",
            "48040000",
        ));
    let records = [
        TransformTailRecord {
            literal_count: 1,
            copy_count: 41,
            back_distance: 12,
        },
        TransformTailRecord {
            literal_count: 52,
            copy_count: 87,
            back_distance: 12,
        },
    ];
    let expected_lane = hex_bytes(concat!(
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000669900ff0000cc3300ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000cc3300ff0000669900ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
            "ff0000ff0000ff0000ff0000ff0000",
        ));
    let mut out = vec![0xee; (expected_lane.len() / 3) * 12];

    let usage = transform_tail_delta3_into(
        &mut out,
        TransformTailDelta3Spec {
            output_stride: 12,
            block_index: 0,
            out_offset: 0,
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
            source2: &source2,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 7,
            source1: 14,
            source2: 138,
            match_entries: 181,
        }
    );
    for (unit_index, expected) in expected_lane.chunks_exact(3).enumerate() {
        let base = unit_index * 12;
        assert_eq!(&out[base..base + 3], expected);
    }
    for (index, &byte) in out.iter().enumerate() {
        if index % 12 >= 3 {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
    assert_eq!(&out[0..3], &[0xff, 0x00, 0x00]);
    assert_eq!(&out[42 * 12..42 * 12 + 3], &[0x66, 0x99, 0x00]);
    assert_eq!(&out[48 * 12..48 * 12 + 3], &[0xff, 0x00, 0x00]);
}

/// Transform tail `0x10fbee0`: four-byte direct literals, matched deltas,
/// and copy runs.
///
/// Provenance: `capture_phase1_transform_tail_10fbee0.py`, compact
/// Armor_020.Armor_020_Head current 2 prefix from the first dispatch-9 call,
/// entry `0x0e000804`, records `(1,7,14)`, `(5,3,70)`, `(1,11,14)`,
/// `(1,25,14)`, `(12,4,56)`, `(7,2,938)`, and `(0,2,924)`. The full replay
/// script covers all 9 captured calls and this prefix is the fixture-free
/// discriminator for the direct lane-0 minus-one rule, match distance
/// `entry >> 3`, and copy-run match-index advance.
#[test]
fn transform_tail_delta4_armor_prefix() {
    let source0 = hex_bytes("0000000000000000000000000000000000000000000000000000");
    let source1 = hex_bytes(concat!(
        "616626654d00654d00994000a63300994000000000b233000d1a1a0d1a1ab23300",
        "b23300145200145200b23300cb1a000d57290d5729cb1a00724000724000bf2600",
        "bf26009940008c40008c4000",
    ));
    let source2 = hex_bytes("00000000");
    let matches = hex_u32_words(concat!(
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000006800000060000000800000000000000070000000a80000000000000098000000",
        "b80000000000000000000000a00000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000",
    ));
    let records = [
        TransformTailRecord {
            literal_count: 1,
            copy_count: 7,
            back_distance: 14,
        },
        TransformTailRecord {
            literal_count: 5,
            copy_count: 3,
            back_distance: 70,
        },
        TransformTailRecord {
            literal_count: 1,
            copy_count: 11,
            back_distance: 14,
        },
        TransformTailRecord {
            literal_count: 1,
            copy_count: 25,
            back_distance: 14,
        },
        TransformTailRecord {
            literal_count: 12,
            copy_count: 4,
            back_distance: 56,
        },
        TransformTailRecord {
            literal_count: 7,
            copy_count: 2,
            back_distance: 938,
        },
        TransformTailRecord {
            literal_count: 0,
            copy_count: 2,
            back_distance: 924,
        },
    ];
    let expected_lane = hex_bytes(concat!(
        "12616626126166261261662612616626126166261261662612616626126166264d654d00",
        "4d654d00",
        "2699400026a63300269940004d654d004d654d002699400012616626126166261261662612616626",
        "1261662612616626126166261261662612616626126166261261662612616626ff000000ff000000",
        "ff000000ff000000ff000000ff000000ff000000ff000000ff000000ff000000ff000000ff000000",
        "ff000000ff000000ff000000ff000000ff000000ff000000ff000000ff000000ff000000ff000000",
        "ff000000ff000000ff000000ff0000001ab23300be0d1a1abe0d1a1a1ab233001ab2330099145200",
        "991452001ab233001acb1a00720d5729720d57291acb1a001acb1a00720d5729720d57291acb1a00",
        "4d7240004d7240001abf26001abf260026994000338c4000338c40002699400026a633004d654d00",
        "4d654d00",
    ));
    let units = expected_lane.len() / 4;
    let mut out = vec![0xee; 10 + (units - 1) * 14 + 4];

    let usage = transform_tail_delta4_into(
        &mut out,
        TransformTailDelta4Spec {
            output_stride: 14,
            block_index: 0,
            out_offset: 10,
            records: &records,
            matches: &matches,
            source0: &source0,
            source1: &source1,
            source2: &source2,
        },
    )
    .unwrap();

    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 26,
            source1: 78,
            source2: 4,
            match_entries: 81,
        }
    );
    assert_eq!(matches.len(), 81);
    for (unit_index, expected) in expected_lane.chunks_exact(4).enumerate() {
        let base = 10 + unit_index * 14;
        assert_eq!(&out[base..base + 4], expected);
    }
    for (index, &byte) in out.iter().enumerate() {
        let in_lane = index >= 10 && (index - 10) % 14 < 4;
        if !in_lane {
            assert_eq!(byte, 0xee, "non-lane byte {index} changed");
        }
    }
    assert_eq!(&out[10..14], &hex_bytes("12616626"));
}

#[test]
fn transform_tail_copy1_rejects_malformed_inputs() {
    let records = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 1];
    assert_eq!(
        transform_tail_copy1_into(
            &mut out,
            TransformTailCopy1Spec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &[1],
            },
        ),
        Err(TransformTailCopyError::ZeroStride)
    );
    assert_eq!(
        transform_tail_copy1_into(
            &mut out,
            TransformTailCopy1Spec {
                output_stride: 1,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &[],
            },
        ),
        Err(TransformTailCopyError::SourceTooSmall)
    );

    let mut empty = [];
    assert_eq!(
        transform_tail_copy1_into(
            &mut empty,
            TransformTailCopy1Spec {
                output_stride: 1,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &[1],
            },
        ),
        Err(TransformTailCopyError::OutputTooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 1,
    }];
    assert_eq!(
        transform_tail_copy1_into(
            &mut out,
            TransformTailCopy1Spec {
                output_stride: 1,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                source: &[],
            },
        ),
        Err(TransformTailCopyError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_copy2_rejects_unobserved_and_malformed_inputs() {
    let records = [TransformTailRecord {
        literal_count: 1,
        copy_count: 1,
        back_distance: 2,
    }];
    let mut out = [0u8; 4];
    assert_eq!(
        transform_tail_copy2_into(
            &mut out,
            TransformTailCopy2Spec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &[1, 2],
            },
        ),
        Err(TransformTailCopyError::ZeroStride)
    );
    assert_eq!(
        transform_tail_copy2_into(
            &mut out,
            TransformTailCopy2Spec {
                output_stride: 2,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &[1],
            },
        ),
        Err(TransformTailCopyError::SourceTooSmall)
    );

    let mut short_out = [0u8; 1];
    assert_eq!(
        transform_tail_copy2_into(
            &mut short_out,
            TransformTailCopy2Spec {
                output_stride: 2,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &[1, 2],
            },
        ),
        Err(TransformTailCopyError::OutputTooSmall)
    );

    let copy_before = [TransformTailRecord {
        literal_count: 1,
        copy_count: 1,
        back_distance: 4,
    }];
    assert_eq!(
        transform_tail_copy2_into(
            &mut out,
            TransformTailCopy2Spec {
                output_stride: 2,
                block_index: 0,
                out_offset: 0,
                records: &copy_before,
                source: &[1, 2],
            },
        ),
        Err(TransformTailCopyError::CopyBeforeOutput)
    );

    let zero_literal = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 2,
    }];
    assert_eq!(
        transform_tail_copy2_into(
            &mut out,
            TransformTailCopy2Spec {
                output_stride: 2,
                block_index: 0,
                out_offset: 0,
                records: &zero_literal,
                source: &[1, 2],
            },
        ),
        Err(TransformTailCopyError::UnobservedRecordShape)
    );

    let zero_copy = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    assert_eq!(
        transform_tail_copy2_into(
            &mut out,
            TransformTailCopy2Spec {
                output_stride: 2,
                block_index: 0,
                out_offset: 0,
                records: &zero_copy,
                source: &[1, 2],
            },
        ),
        Err(TransformTailCopyError::UnobservedRecordShape)
    );
}

#[test]
fn transform_tail_copy3_rejects_malformed_inputs() {
    let records = [TransformTailRecord {
        literal_count: 1,
        copy_count: 1,
        back_distance: 3,
    }];
    let mut out = [0u8; 6];
    assert_eq!(
        transform_tail_copy3_into(
            &mut out,
            TransformTailCopy3Spec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &[1, 2, 3],
            },
        ),
        Err(TransformTailCopyError::ZeroStride)
    );
    assert_eq!(
        transform_tail_copy3_into(
            &mut out,
            TransformTailCopy3Spec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &[1, 2],
            },
        ),
        Err(TransformTailCopyError::SourceTooSmall)
    );

    let mut short_out = [0u8; 2];
    assert_eq!(
        transform_tail_copy3_into(
            &mut short_out,
            TransformTailCopy3Spec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &[1, 2, 3],
            },
        ),
        Err(TransformTailCopyError::OutputTooSmall)
    );

    let copy_before = [TransformTailRecord {
        literal_count: 1,
        copy_count: 1,
        back_distance: 6,
    }];
    assert_eq!(
        transform_tail_copy3_into(
            &mut out,
            TransformTailCopy3Spec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &copy_before,
                source: &[1, 2, 3],
            },
        ),
        Err(TransformTailCopyError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_copy4_rejects_malformed_inputs() {
    let records = [TransformTailRecord {
        literal_count: 1,
        copy_count: 1,
        back_distance: 4,
    }];
    let mut out = [0u8; 8];
    assert_eq!(
        transform_tail_copy4_into(
            &mut out,
            TransformTailCopy4Spec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &[1, 2, 3, 4],
            },
        ),
        Err(TransformTailCopyError::ZeroStride)
    );
    assert_eq!(
        transform_tail_copy4_into(
            &mut out,
            TransformTailCopy4Spec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &[1, 2, 3],
            },
        ),
        Err(TransformTailCopyError::SourceTooSmall)
    );

    let mut short_out = [0u8; 3];
    assert_eq!(
        transform_tail_copy4_into(
            &mut short_out,
            TransformTailCopy4Spec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &[1, 2, 3, 4],
            },
        ),
        Err(TransformTailCopyError::OutputTooSmall)
    );

    let copy_before = [TransformTailRecord {
        literal_count: 1,
        copy_count: 1,
        back_distance: 8,
    }];
    assert_eq!(
        transform_tail_copy4_into(
            &mut out,
            TransformTailCopy4Spec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &copy_before,
                source: &[1, 2, 3, 4],
            },
        ),
        Err(TransformTailCopyError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_copy6_rejects_unobserved_and_malformed_inputs() {
    let records = [TransformTailRecord {
        literal_count: 1,
        copy_count: 1,
        back_distance: 6,
    }];
    let mut out = [0u8; 12];
    assert_eq!(
        transform_tail_copy6_into(
            &mut out,
            TransformTailCopy6Spec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &[1, 2, 3, 4, 5, 6],
            },
        ),
        Err(TransformTailCopyError::ZeroStride)
    );
    assert_eq!(
        transform_tail_copy6_into(
            &mut out,
            TransformTailCopy6Spec {
                output_stride: 6,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &[1, 2, 3, 4, 5],
            },
        ),
        Err(TransformTailCopyError::SourceTooSmall)
    );

    let mut short_out = [0u8; 5];
    assert_eq!(
        transform_tail_copy6_into(
            &mut short_out,
            TransformTailCopy6Spec {
                output_stride: 6,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &[1, 2, 3, 4, 5, 6],
            },
        ),
        Err(TransformTailCopyError::OutputTooSmall)
    );

    let copy_before = [TransformTailRecord {
        literal_count: 1,
        copy_count: 1,
        back_distance: 12,
    }];
    assert_eq!(
        transform_tail_copy6_into(
            &mut out,
            TransformTailCopy6Spec {
                output_stride: 6,
                block_index: 0,
                out_offset: 0,
                records: &copy_before,
                source: &[1, 2, 3, 4, 5, 6],
            },
        ),
        Err(TransformTailCopyError::CopyBeforeOutput)
    );

    let zero_copy = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    assert_eq!(
        transform_tail_copy6_into(
            &mut out,
            TransformTailCopy6Spec {
                output_stride: 6,
                block_index: 0,
                out_offset: 0,
                records: &zero_copy,
                source: &[1, 2, 3, 4, 5, 6],
            },
        ),
        Err(TransformTailCopyError::UnobservedRecordShape)
    );
}

#[test]
fn transform_tail_copy8_rejects_malformed_inputs() {
    let records = [TransformTailRecord {
        literal_count: 1,
        copy_count: 1,
        back_distance: 28,
    }];
    let mut out = [0u8; 36];
    assert_eq!(
        transform_tail_copy8_into(
            &mut out,
            TransformTailCopy8Spec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &[1; 8],
            },
        ),
        Err(TransformTailCopyError::ZeroStride)
    );
    assert_eq!(
        transform_tail_copy8_into(
            &mut out,
            TransformTailCopy8Spec {
                output_stride: 28,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &[1; 7],
            },
        ),
        Err(TransformTailCopyError::SourceTooSmall)
    );

    let mut short_out = [0u8; 7];
    assert_eq!(
        transform_tail_copy8_into(
            &mut short_out,
            TransformTailCopy8Spec {
                output_stride: 28,
                block_index: 0,
                out_offset: 0,
                records: &records,
                source: &[1; 8],
            },
        ),
        Err(TransformTailCopyError::OutputTooSmall)
    );

    let copy_before = [TransformTailRecord {
        literal_count: 1,
        copy_count: 1,
        back_distance: 56,
    }];
    assert_eq!(
        transform_tail_copy8_into(
            &mut out,
            TransformTailCopy8Spec {
                output_stride: 28,
                block_index: 0,
                out_offset: 0,
                records: &copy_before,
                source: &[1; 8],
            },
        ),
        Err(TransformTailCopyError::CopyBeforeOutput)
    );

    let zero_copy = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    assert_eq!(
        transform_tail_copy8_into(
            &mut out,
            TransformTailCopy8Spec {
                output_stride: 28,
                block_index: 0,
                out_offset: 0,
                records: &zero_copy,
                source: &[1; 8],
            },
        ),
        Err(TransformTailCopyError::UnobservedRecordShape)
    );
}

#[test]
fn transform_tail_delta2_allows_observed_zero_literal_and_zero_copy() {
    let zero_literal = [TransformTailRecord {
        literal_count: 0,
        copy_count: 7,
        back_distance: 1780,
    }];
    let mut out = vec![0xee; 1842];
    for unit in 0..7 {
        let base = unit * 10;
        out[base] = unit as u8;
        out[base + 1] = 0x80 | unit as u8;
    }
    let usage = transform_tail_delta2_into(
        &mut out,
        TransformTailDelta2Spec {
            output_stride: 10,
            block_index: 0,
            out_offset: 1780,
            records: &zero_literal,
            matches: &[0; 7],
            source0: &[],
            source1: &[],
            source2: &[],
        },
    )
    .unwrap();
    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 0,
            source1: 0,
            source2: 0,
            match_entries: 7,
        }
    );
    for unit in 0..7 {
        let base = 1780 + unit * 10;
        assert_eq!(&out[base..base + 2], &[unit as u8, 0x80 | unit as u8]);
    }

    let zero_copy = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = vec![0xee; 10];
    let usage = transform_tail_delta2_into(
        &mut out,
        TransformTailDelta2Spec {
            output_stride: 10,
            block_index: 0,
            out_offset: 0,
            records: &zero_copy,
            matches: &[0],
            source0: &[0],
            source1: &[0],
            source2: &[],
        },
    )
    .unwrap();
    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 1,
            source1: 1,
            source2: 0,
            match_entries: 1,
        }
    );
    assert_eq!(&out[0..2], &[0xff, 0x00]);
}

#[test]
fn transform_tail_delta2_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 2];
    assert_eq!(
        transform_tail_delta2_into(
            &mut out,
            TransformTailDelta2Spec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0],
                source1: &[0],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_delta2_into(
            &mut out,
            TransformTailDelta2Spec {
                output_stride: 2,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[],
                source0: &[0],
                source1: &[0],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::MatchTableTooSmall)
    );
    assert_eq!(
        transform_tail_delta2_into(
            &mut out,
            TransformTailDelta2Spec {
                output_stride: 2,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[],
                source1: &[0],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );
    assert_eq!(
        transform_tail_delta2_into(
            &mut out,
            TransformTailDelta2Spec {
                output_stride: 2,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0],
                source1: &[],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::Source1TooSmall)
    );

    let matched = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    assert_eq!(
        transform_tail_delta2_into(
            &mut out,
            TransformTailDelta2Spec {
                output_stride: 2,
                block_index: 0,
                out_offset: 0,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[],
                source2: &[0, 0],
            },
        ),
        Err(TransformTailDeltaError::MatchBeforeOutput)
    );

    let mut matched_out = [0u8; 12];
    assert_eq!(
        transform_tail_delta2_into(
            &mut matched_out,
            TransformTailDelta2Spec {
                output_stride: 10,
                block_index: 0,
                out_offset: 10,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[],
                source2: &[0],
            },
        ),
        Err(TransformTailDeltaError::Source2TooSmall)
    );

    let mut short_out = [0u8; 1];
    assert_eq!(
        transform_tail_delta2_into(
            &mut short_out,
            TransformTailDelta2Spec {
                output_stride: 2,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0],
                source1: &[0],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 1,
    }];
    assert_eq!(
        transform_tail_delta2_into(
            &mut out,
            TransformTailDelta2Spec {
                output_stride: 2,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                matches: &[0],
                source0: &[],
                source1: &[],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_delta2_direct_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 2];
    assert_eq!(
        transform_tail_delta2_direct_into(
            &mut out,
            TransformTailDelta2DirectSpec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0, 0],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_delta2_direct_into(
            &mut out,
            TransformTailDelta2DirectSpec {
                output_stride: 2,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[],
                source0: &[0, 0],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::MatchTableTooSmall)
    );
    assert_eq!(
        transform_tail_delta2_direct_into(
            &mut out,
            TransformTailDelta2DirectSpec {
                output_stride: 2,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );

    let matched = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    assert_eq!(
        transform_tail_delta2_direct_into(
            &mut out,
            TransformTailDelta2DirectSpec {
                output_stride: 2,
                block_index: 0,
                out_offset: 0,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0, 0],
            },
        ),
        Err(TransformTailDeltaError::MatchBeforeOutput)
    );

    let mut matched_out = [0u8; 12];
    assert_eq!(
        transform_tail_delta2_direct_into(
            &mut matched_out,
            TransformTailDelta2DirectSpec {
                output_stride: 10,
                block_index: 0,
                out_offset: 10,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0],
            },
        ),
        Err(TransformTailDeltaError::Source1TooSmall)
    );

    let mut short_out = [0u8; 1];
    assert_eq!(
        transform_tail_delta2_direct_into(
            &mut short_out,
            TransformTailDelta2DirectSpec {
                output_stride: 2,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0, 0],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 1,
    }];
    assert_eq!(
        transform_tail_delta2_direct_into(
            &mut out,
            TransformTailDelta2DirectSpec {
                output_stride: 2,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                matches: &[0],
                source0: &[],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_delta3_direct_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 3];
    assert_eq!(
        transform_tail_delta3_direct_into(
            &mut out,
            TransformTailDelta3DirectSpec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0, 0, 0],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_delta3_direct_into(
            &mut out,
            TransformTailDelta3DirectSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[],
                source0: &[0, 0, 0],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::MatchTableTooSmall)
    );
    assert_eq!(
        transform_tail_delta3_direct_into(
            &mut out,
            TransformTailDelta3DirectSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0, 0],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );

    let matched = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    assert_eq!(
        transform_tail_delta3_direct_into(
            &mut out,
            TransformTailDelta3DirectSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0, 0, 0],
            },
        ),
        Err(TransformTailDeltaError::MatchBeforeOutput)
    );

    let mut matched_out = [0u8; 15];
    assert_eq!(
        transform_tail_delta3_direct_into(
            &mut matched_out,
            TransformTailDelta3DirectSpec {
                output_stride: 12,
                block_index: 0,
                out_offset: 12,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0, 0],
            },
        ),
        Err(TransformTailDeltaError::Source1TooSmall)
    );

    let mut short_out = [0u8; 2];
    assert_eq!(
        transform_tail_delta3_direct_into(
            &mut short_out,
            TransformTailDelta3DirectSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0, 0, 0],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 1,
    }];
    assert_eq!(
        transform_tail_delta3_direct_into(
            &mut out,
            TransformTailDelta3DirectSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                matches: &[0],
                source0: &[],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_delta4_direct_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 4];
    assert_eq!(
        transform_tail_delta4_direct_into(
            &mut out,
            TransformTailDelta4DirectSpec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0, 0, 0, 0],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_delta4_direct_into(
            &mut out,
            TransformTailDelta4DirectSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[],
                source0: &[0, 0, 0, 0],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::MatchTableTooSmall)
    );
    assert_eq!(
        transform_tail_delta4_direct_into(
            &mut out,
            TransformTailDelta4DirectSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0, 0, 0],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );

    let matched = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    assert_eq!(
        transform_tail_delta4_direct_into(
            &mut out,
            TransformTailDelta4DirectSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0, 0, 0, 0],
            },
        ),
        Err(TransformTailDeltaError::MatchBeforeOutput)
    );

    let mut matched_out = [0u8; 24];
    assert_eq!(
        transform_tail_delta4_direct_into(
            &mut matched_out,
            TransformTailDelta4DirectSpec {
                output_stride: 20,
                block_index: 0,
                out_offset: 20,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0, 0, 0],
            },
        ),
        Err(TransformTailDeltaError::Source1TooSmall)
    );

    let mut short_out = [0u8; 3];
    assert_eq!(
        transform_tail_delta4_direct_into(
            &mut short_out,
            TransformTailDelta4DirectSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0, 0, 0, 0],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 1,
    }];
    assert_eq!(
        transform_tail_delta4_direct_into(
            &mut out,
            TransformTailDelta4DirectSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                matches: &[0],
                source0: &[],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_u8x2_delta_rejects_malformed_inputs() {
    let records = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let matches = [0u32];
    let mut out = vec![0u8; 20];
    assert_eq!(
        transform_tail_u8x2_delta_into(
            &mut out,
            TransformTailU8x2DeltaSpec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &records,
                matches: &matches,
                source0: &[0, 0],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_u8x2_delta_into(
            &mut out,
            TransformTailU8x2DeltaSpec {
                output_stride: 10,
                block_index: 0,
                out_offset: 0,
                records: &records,
                matches: &[],
                source0: &[0, 0],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::MatchTableTooSmall)
    );
    assert_eq!(
        transform_tail_u8x2_delta_into(
            &mut out,
            TransformTailU8x2DeltaSpec {
                output_stride: 10,
                block_index: 0,
                out_offset: 0,
                records: &records,
                matches: &matches,
                source0: &[0],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );

    let matches = [8u32];
    assert_eq!(
        transform_tail_u8x2_delta_into(
            &mut out,
            TransformTailU8x2DeltaSpec {
                output_stride: 10,
                block_index: 0,
                out_offset: 6,
                records: &records,
                matches: &matches,
                source0: &[],
                source1: &[1, 2],
            },
        ),
        Err(TransformTailDeltaError::MatchBeforeOutput)
    );

    out[6] = 10;
    out[7] = 20;
    assert_eq!(
        transform_tail_u8x2_delta_into(
            &mut out,
            TransformTailU8x2DeltaSpec {
                output_stride: 10,
                block_index: 0,
                out_offset: 16,
                records: &records,
                matches: &matches,
                source0: &[],
                source1: &[1],
            },
        ),
        Err(TransformTailDeltaError::Source1TooSmall)
    );

    let copy_records = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 10,
    }];
    assert_eq!(
        transform_tail_u8x2_delta_into(
            &mut out,
            TransformTailU8x2DeltaSpec {
                output_stride: 10,
                block_index: 0,
                out_offset: 6,
                records: &copy_records,
                matches: &matches,
                source0: &[],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_u16x3_delta_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 6];
    assert_eq!(
        transform_tail_u16x3_delta_into(
            &mut out,
            TransformTailU16x3DeltaSpec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 6],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_u16x3_delta_into(
            &mut out,
            TransformTailU16x3DeltaSpec {
                output_stride: 6,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[],
                source0: &[0; 6],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::MatchTableTooSmall)
    );
    assert_eq!(
        transform_tail_u16x3_delta_into(
            &mut out,
            TransformTailU16x3DeltaSpec {
                output_stride: 6,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 5],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );

    let matched = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    assert_eq!(
        transform_tail_u16x3_delta_into(
            &mut out,
            TransformTailU16x3DeltaSpec {
                output_stride: 6,
                block_index: 0,
                out_offset: 0,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0; 6],
            },
        ),
        Err(TransformTailDeltaError::MatchBeforeOutput)
    );

    let mut matched_out = [0u8; 16];
    assert_eq!(
        transform_tail_u16x3_delta_into(
            &mut matched_out,
            TransformTailU16x3DeltaSpec {
                output_stride: 10,
                block_index: 0,
                out_offset: 10,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0; 5],
            },
        ),
        Err(TransformTailDeltaError::Source1TooSmall)
    );

    let mut short_out = [0u8; 5];
    assert_eq!(
        transform_tail_u16x3_delta_into(
            &mut short_out,
            TransformTailU16x3DeltaSpec {
                output_stride: 6,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 6],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 1,
    }];
    assert_eq!(
        transform_tail_u16x3_delta_into(
            &mut out,
            TransformTailU16x3DeltaSpec {
                output_stride: 6,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                matches: &[0],
                source0: &[],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_pack10x3_delta_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 4];
    assert_eq!(
        transform_tail_pack10x3_delta_into(
            &mut out,
            TransformTailPack10x3DeltaSpec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 4],
                source1: &[0; 2],
                source2: &[0],
                source3: &[],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_pack10x3_delta_into(
            &mut out,
            TransformTailPack10x3DeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[],
                source0: &[0; 4],
                source1: &[0; 2],
                source2: &[0],
                source3: &[],
            },
        ),
        Err(TransformTailDeltaError::MatchTableTooSmall)
    );
    assert_eq!(
        transform_tail_pack10x3_delta_into(
            &mut out,
            TransformTailPack10x3DeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 3],
                source1: &[0; 2],
                source2: &[0],
                source3: &[],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );
    assert_eq!(
        transform_tail_pack10x3_delta_into(
            &mut out,
            TransformTailPack10x3DeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 4],
                source1: &[0; 1],
                source2: &[0],
                source3: &[],
            },
        ),
        Err(TransformTailDeltaError::Source1TooSmall)
    );
    assert_eq!(
        transform_tail_pack10x3_delta_into(
            &mut out,
            TransformTailPack10x3DeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 4],
                source1: &[0; 2],
                source2: &[],
                source3: &[],
            },
        ),
        Err(TransformTailDeltaError::Source2TooSmall)
    );

    let matched = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    assert_eq!(
        transform_tail_pack10x3_delta_into(
            &mut out,
            TransformTailPack10x3DeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[],
                source2: &[],
                source3: &[0; 6],
            },
        ),
        Err(TransformTailDeltaError::MatchBeforeOutput)
    );

    let mut matched_out = [0u8; 24];
    assert_eq!(
        transform_tail_pack10x3_delta_into(
            &mut matched_out,
            TransformTailPack10x3DeltaSpec {
                output_stride: 20,
                block_index: 0,
                out_offset: 20,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[],
                source2: &[],
                source3: &[0; 5],
            },
        ),
        Err(TransformTailDeltaError::Source3TooSmall)
    );

    let mut short_out = [0u8; 3];
    assert_eq!(
        transform_tail_pack10x3_delta_into(
            &mut short_out,
            TransformTailPack10x3DeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 4],
                source1: &[0; 2],
                source2: &[0],
                source3: &[],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 1,
    }];
    assert_eq!(
        transform_tail_pack10x3_delta_into(
            &mut out,
            TransformTailPack10x3DeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                matches: &[0],
                source0: &[],
                source1: &[],
                source2: &[],
                source3: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_pack10x3_normal_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 4];
    assert_eq!(
        transform_tail_pack10x3_normal_into(
            &mut out,
            TransformTailPack10x3NormalSpec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                source0: &[0; 4],
                source1: &[0; 2],
                source2: &[0],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_pack10x3_normal_into(
            &mut out,
            TransformTailPack10x3NormalSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                source0: &[0; 3],
                source1: &[0; 2],
                source2: &[0],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );
    assert_eq!(
        transform_tail_pack10x3_normal_into(
            &mut out,
            TransformTailPack10x3NormalSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                source0: &[0; 4],
                source1: &[0; 1],
                source2: &[0],
            },
        ),
        Err(TransformTailDeltaError::Source1TooSmall)
    );
    assert_eq!(
        transform_tail_pack10x3_normal_into(
            &mut out,
            TransformTailPack10x3NormalSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                source0: &[0; 4],
                source1: &[0; 2],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::Source2TooSmall)
    );

    let mut short_out = [0u8; 3];
    assert_eq!(
        transform_tail_pack10x3_normal_into(
            &mut short_out,
            TransformTailPack10x3NormalSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                source0: &[0; 4],
                source1: &[0; 2],
                source2: &[0],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 1,
    }];
    assert_eq!(
        transform_tail_pack10x3_normal_into(
            &mut out,
            TransformTailPack10x3NormalSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                source0: &[],
                source1: &[],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_f16x3_predict_rejects_malformed_inputs() {
    let direct_then_copy = [TransformTailRecord {
        literal_count: 1,
        copy_count: 1,
        back_distance: 8,
    }];
    let zero_copy = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 8,
    }];
    let aux = [0u64];
    let source0 = [0u8; 3];
    let source1 = [0u8; 3];
    let source2 = [];
    let source3 = [0u8; 6];
    let source4 = [];
    let run = |out: &mut [u8],
               output_stride,
               records: &[TransformTailRecord],
               aux_table: &[u64],
               source0: &[u8],
               source1: &[u8],
               source2: &[u8],
               source3: &[u8],
               source4: &[u8]| {
        transform_tail_f16x3_predict_into(
            out,
            TransformTailF16x3PredictSpec {
                output_stride,
                block_index: 0,
                out_offset: 0,
                records,
                aux_table,
                source0,
                source1,
                source2,
                source3,
                source4,
            },
        )
    };

    let mut out = [0u8; 16];
    assert_eq!(
        run(
            &mut out,
            0,
            &direct_then_copy,
            &aux,
            &source0,
            &source1,
            &source2,
            &source3,
            &source4,
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        run(
            &mut out,
            8,
            &direct_then_copy,
            &[],
            &source0,
            &source1,
            &source2,
            &source3,
            &source4,
        ),
        Err(TransformTailDeltaError::AuxTableTooSmall)
    );
    assert_eq!(
        run(
            &mut out,
            8,
            &direct_then_copy,
            &aux,
            &[0; 2],
            &source1,
            &source2,
            &source3,
            &source4,
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );
    assert_eq!(
        run(
            &mut out,
            8,
            &direct_then_copy,
            &aux,
            &source0,
            &[0; 2],
            &source2,
            &source3,
            &source4,
        ),
        Err(TransformTailDeltaError::Source1TooSmall)
    );
    assert_eq!(
        run(
            &mut out,
            8,
            &direct_then_copy,
            &aux,
            &[1, 0, 0],
            &source1,
            &[],
            &source3,
            &source4,
        ),
        Err(TransformTailDeltaError::Source2TooSmall)
    );
    assert_eq!(
        run(
            &mut out,
            8,
            &direct_then_copy,
            &aux,
            &source0,
            &source1,
            &source2,
            &[0; 5],
            &source4,
        ),
        Err(TransformTailDeltaError::Source3TooSmall)
    );
    assert_eq!(
        run(
            &mut out,
            8,
            &direct_then_copy,
            &aux,
            &[1, 0, 0],
            &source1,
            &[0],
            &source3,
            &[],
        ),
        Err(TransformTailDeltaError::Source4TooSmall)
    );

    let mut short_out = [0u8; 5];
    assert_eq!(
        run(
            &mut short_out,
            8,
            &direct_then_copy,
            &aux,
            &source0,
            &source1,
            &source2,
            &source3,
            &source4,
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let far_aux = [1u64];
    assert_eq!(
        run(
            &mut out,
            8,
            &direct_then_copy,
            &far_aux,
            &source0,
            &source1,
            &source2,
            &source3,
            &source4,
        ),
        Err(TransformTailDeltaError::PredictorBeforeOutput)
    );
    assert_eq!(
        run(&mut out, 8, &copy_first, &[], &[], &[], &[], &[], &[],),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
    assert_eq!(
        run(&mut out, 8, &zero_copy, &aux, &source0, &source1, &source2, &source3, &source4,),
        Err(TransformTailDeltaError::UnobservedRecordShape)
    );
}

#[test]
fn transform_tail_pack10x3_previous_delta_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 4];
    assert_eq!(
        transform_tail_pack10x3_previous_delta_into(
            &mut out,
            TransformTailPack10x3PreviousDeltaSpec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 6],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_pack10x3_previous_delta_into(
            &mut out,
            TransformTailPack10x3PreviousDeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[],
                source0: &[0; 6],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::MatchTableTooSmall)
    );
    assert_eq!(
        transform_tail_pack10x3_previous_delta_into(
            &mut out,
            TransformTailPack10x3PreviousDeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 5],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );

    let mut short_out = [0u8; 3];
    assert_eq!(
        transform_tail_pack10x3_previous_delta_into(
            &mut short_out,
            TransformTailPack10x3PreviousDeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 6],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let matched = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    assert_eq!(
        transform_tail_pack10x3_previous_delta_into(
            &mut out,
            TransformTailPack10x3PreviousDeltaSpec {
                output_stride: 16,
                block_index: 0,
                out_offset: 0,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0; 6],
            },
        ),
        Err(TransformTailDeltaError::MatchBeforeOutput)
    );

    let mut matched_out = [0u8; 20];
    assert_eq!(
        transform_tail_pack10x3_previous_delta_into(
            &mut matched_out,
            TransformTailPack10x3PreviousDeltaSpec {
                output_stride: 16,
                block_index: 0,
                out_offset: 16,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0; 5],
            },
        ),
        Err(TransformTailDeltaError::Source1TooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 16,
    }];
    assert_eq!(
        transform_tail_pack10x3_previous_delta_into(
            &mut out,
            TransformTailPack10x3PreviousDeltaSpec {
                output_stride: 16,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                matches: &[0],
                source0: &[],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_u8x3_delta_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 3];
    assert_eq!(
        transform_tail_u8x3_delta_into(
            &mut out,
            TransformTailU8x3DeltaSpec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 3],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_u8x3_delta_into(
            &mut out,
            TransformTailU8x3DeltaSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[],
                source0: &[0; 3],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::MatchTableTooSmall)
    );
    assert_eq!(
        transform_tail_u8x3_delta_into(
            &mut out,
            TransformTailU8x3DeltaSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 2],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );

    let matched = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    assert_eq!(
        transform_tail_u8x3_delta_into(
            &mut out,
            TransformTailU8x3DeltaSpec {
                output_stride: 12,
                block_index: 0,
                out_offset: 0,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0; 3],
            },
        ),
        Err(TransformTailDeltaError::MatchBeforeOutput)
    );

    let mut matched_out = [0u8; 15];
    assert_eq!(
        transform_tail_u8x3_delta_into(
            &mut matched_out,
            TransformTailU8x3DeltaSpec {
                output_stride: 12,
                block_index: 0,
                out_offset: 12,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0; 2],
            },
        ),
        Err(TransformTailDeltaError::Source1TooSmall)
    );

    let mut short_out = [0u8; 2];
    assert_eq!(
        transform_tail_u8x3_delta_into(
            &mut short_out,
            TransformTailU8x3DeltaSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 3],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 3,
    }];
    assert_eq!(
        transform_tail_u8x3_delta_into(
            &mut out,
            TransformTailU8x3DeltaSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                matches: &[0],
                source0: &[],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_u16x2_direct_delta_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 4];
    assert_eq!(
        transform_tail_u16x2_direct_delta_into(
            &mut out,
            TransformTailU16x2DirectDeltaSpec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 4],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_u16x2_direct_delta_into(
            &mut out,
            TransformTailU16x2DirectDeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[],
                source0: &[0; 4],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::MatchTableTooSmall)
    );
    assert_eq!(
        transform_tail_u16x2_direct_delta_into(
            &mut out,
            TransformTailU16x2DirectDeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 3],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );

    let matched = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    assert_eq!(
        transform_tail_u16x2_direct_delta_into(
            &mut out,
            TransformTailU16x2DirectDeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0; 4],
            },
        ),
        Err(TransformTailDeltaError::MatchBeforeOutput)
    );

    let mut matched_out = [0u8; 24];
    assert_eq!(
        transform_tail_u16x2_direct_delta_into(
            &mut matched_out,
            TransformTailU16x2DirectDeltaSpec {
                output_stride: 20,
                block_index: 0,
                out_offset: 20,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0; 3],
            },
        ),
        Err(TransformTailDeltaError::Source1TooSmall)
    );

    let mut short_out = [0u8; 3];
    assert_eq!(
        transform_tail_u16x2_direct_delta_into(
            &mut short_out,
            TransformTailU16x2DirectDeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 4],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 1,
    }];
    assert_eq!(
        transform_tail_u16x2_direct_delta_into(
            &mut out,
            TransformTailU16x2DirectDeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                matches: &[0],
                source0: &[],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_delta1_direct_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 1];
    assert_eq!(
        transform_tail_delta1_direct_into(
            &mut out,
            TransformTailDelta1DirectSpec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_delta1_direct_into(
            &mut out,
            TransformTailDelta1DirectSpec {
                output_stride: 1,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[],
                source0: &[0],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::MatchTableTooSmall)
    );
    assert_eq!(
        transform_tail_delta1_direct_into(
            &mut out,
            TransformTailDelta1DirectSpec {
                output_stride: 1,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );

    let matched = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    assert_eq!(
        transform_tail_delta1_direct_into(
            &mut out,
            TransformTailDelta1DirectSpec {
                output_stride: 8,
                block_index: 0,
                out_offset: 0,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0],
            },
        ),
        Err(TransformTailDeltaError::MatchBeforeOutput)
    );

    let mut matched_out = [0u8; 9];
    assert_eq!(
        transform_tail_delta1_direct_into(
            &mut matched_out,
            TransformTailDelta1DirectSpec {
                output_stride: 8,
                block_index: 0,
                out_offset: 8,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::Source1TooSmall)
    );

    let mut short_out = [0u8; 0];
    assert_eq!(
        transform_tail_delta1_direct_into(
            &mut short_out,
            TransformTailDelta1DirectSpec {
                output_stride: 1,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 1,
    }];
    assert_eq!(
        transform_tail_delta1_direct_into(
            &mut out,
            TransformTailDelta1DirectSpec {
                output_stride: 1,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                matches: &[0],
                source0: &[],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_i8x3_direct_delta_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 3];
    assert_eq!(
        transform_tail_i8x3_direct_delta_into(
            &mut out,
            TransformTailI8x3DirectDeltaSpec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 3],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_i8x3_direct_delta_into(
            &mut out,
            TransformTailI8x3DirectDeltaSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[],
                source0: &[0; 3],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::MatchTableTooSmall)
    );
    assert_eq!(
        transform_tail_i8x3_direct_delta_into(
            &mut out,
            TransformTailI8x3DirectDeltaSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 2],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );

    let matched = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    assert_eq!(
        transform_tail_i8x3_direct_delta_into(
            &mut out,
            TransformTailI8x3DirectDeltaSpec {
                output_stride: 16,
                block_index: 0,
                out_offset: 0,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0; 3],
            },
        ),
        Err(TransformTailDeltaError::MatchBeforeOutput)
    );

    let mut matched_out = [0u8; 19];
    assert_eq!(
        transform_tail_i8x3_direct_delta_into(
            &mut matched_out,
            TransformTailI8x3DirectDeltaSpec {
                output_stride: 16,
                block_index: 0,
                out_offset: 16,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0; 2],
            },
        ),
        Err(TransformTailDeltaError::Source1TooSmall)
    );

    let mut short_out = [0u8; 2];
    assert_eq!(
        transform_tail_i8x3_direct_delta_into(
            &mut short_out,
            TransformTailI8x3DirectDeltaSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 3],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let copy_record = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 16,
    }];
    assert_eq!(
        transform_tail_i8x3_direct_delta_into(
            &mut out,
            TransformTailI8x3DirectDeltaSpec {
                output_stride: 16,
                block_index: 0,
                out_offset: 0,
                records: &copy_record,
                matches: &[0],
                source0: &[],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::UnobservedRecordShape)
    );
}

#[test]
fn transform_tail_u8_previous_delta_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 1];
    assert_eq!(
        transform_tail_u8_previous_delta_into(
            &mut out,
            TransformTailU8PreviousDeltaSpec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                source0: &[0],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_u8_previous_delta_into(
            &mut out,
            TransformTailU8PreviousDeltaSpec {
                output_stride: 1,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                source0: &[],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );

    let mut short_out = [0u8; 0];
    assert_eq!(
        transform_tail_u8_previous_delta_into(
            &mut short_out,
            TransformTailU8PreviousDeltaSpec {
                output_stride: 1,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                source0: &[0],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 1,
    }];
    assert_eq!(
        transform_tail_u8_previous_delta_into(
            &mut out,
            TransformTailU8PreviousDeltaSpec {
                output_stride: 1,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                source0: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_u8x3_previous_delta_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 3];
    assert_eq!(
        transform_tail_u8x3_previous_delta_into(
            &mut out,
            TransformTailU8x3PreviousDeltaSpec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                source0: &[0; 3],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_u8x3_previous_delta_into(
            &mut out,
            TransformTailU8x3PreviousDeltaSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                source0: &[0; 2],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );

    let mut short_out = [0u8; 2];
    assert_eq!(
        transform_tail_u8x3_previous_delta_into(
            &mut short_out,
            TransformTailU8x3PreviousDeltaSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                source0: &[0; 3],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let two_literals = [TransformTailRecord {
        literal_count: 2,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut short_second_out = [0u8; 5];
    assert_eq!(
        transform_tail_u8x3_previous_delta_into(
            &mut short_second_out,
            TransformTailU8x3PreviousDeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &two_literals,
                source0: &[0; 6],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 3,
    }];
    assert_eq!(
        transform_tail_u8x3_previous_delta_into(
            &mut out,
            TransformTailU8x3PreviousDeltaSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                source0: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_u16x2_previous_delta_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 4];
    assert_eq!(
        transform_tail_u16x2_previous_delta_into(
            &mut out,
            TransformTailU16x2PreviousDeltaSpec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                source0: &[0; 4],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_u16x2_previous_delta_into(
            &mut out,
            TransformTailU16x2PreviousDeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                source0: &[0; 3],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );

    let mut short_out = [0u8; 3];
    assert_eq!(
        transform_tail_u16x2_previous_delta_into(
            &mut short_out,
            TransformTailU16x2PreviousDeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                source0: &[0; 4],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 4,
    }];
    assert_eq!(
        transform_tail_u16x2_previous_delta_into(
            &mut out,
            TransformTailU16x2PreviousDeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                source0: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_u16x2_delta_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 4];
    assert_eq!(
        transform_tail_u16x2_delta_into(
            &mut out,
            TransformTailU16x2DeltaSpec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 4],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_u16x2_delta_into(
            &mut out,
            TransformTailU16x2DeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[],
                source0: &[0; 4],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::MatchTableTooSmall)
    );
    assert_eq!(
        transform_tail_u16x2_delta_into(
            &mut out,
            TransformTailU16x2DeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 3],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );

    let two_zero_matches = [TransformTailRecord {
        literal_count: 2,
        copy_count: 0,
        back_distance: 0,
    }];
    assert_eq!(
        transform_tail_u16x2_delta_into(
            &mut out,
            TransformTailU16x2DeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &two_zero_matches,
                matches: &[0, 0],
                source0: &[0; 7],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );

    let matched = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    assert_eq!(
        transform_tail_u16x2_delta_into(
            &mut out,
            TransformTailU16x2DeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0; 4],
            },
        ),
        Err(TransformTailDeltaError::MatchBeforeOutput)
    );

    let mut matched_out = [0u8; 24];
    assert_eq!(
        transform_tail_u16x2_delta_into(
            &mut matched_out,
            TransformTailU16x2DeltaSpec {
                output_stride: 20,
                block_index: 0,
                out_offset: 20,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0; 3],
            },
        ),
        Err(TransformTailDeltaError::Source1TooSmall)
    );

    let mut short_out = [0u8; 3];
    assert_eq!(
        transform_tail_u16x2_delta_into(
            &mut short_out,
            TransformTailU16x2DeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 4],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 1,
    }];
    assert_eq!(
        transform_tail_u16x2_delta_into(
            &mut out,
            TransformTailU16x2DeltaSpec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                matches: &[0],
                source0: &[],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_u32x2_delta_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 8];
    assert_eq!(
        transform_tail_u32x2_delta_into(
            &mut out,
            TransformTailU32x2DeltaSpec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 8],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_u32x2_delta_into(
            &mut out,
            TransformTailU32x2DeltaSpec {
                output_stride: 8,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[],
                source0: &[0; 8],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::MatchTableTooSmall)
    );
    assert_eq!(
        transform_tail_u32x2_delta_into(
            &mut out,
            TransformTailU32x2DeltaSpec {
                output_stride: 8,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 7],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );

    let matched = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    assert_eq!(
        transform_tail_u32x2_delta_into(
            &mut out,
            TransformTailU32x2DeltaSpec {
                output_stride: 8,
                block_index: 0,
                out_offset: 0,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0; 8],
            },
        ),
        Err(TransformTailDeltaError::MatchBeforeOutput)
    );

    let mut matched_out = [0u8; 28];
    assert_eq!(
        transform_tail_u32x2_delta_into(
            &mut matched_out,
            TransformTailU32x2DeltaSpec {
                output_stride: 20,
                block_index: 0,
                out_offset: 20,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[0; 7],
            },
        ),
        Err(TransformTailDeltaError::Source1TooSmall)
    );

    let mut short_out = [0u8; 7];
    assert_eq!(
        transform_tail_u32x2_delta_into(
            &mut short_out,
            TransformTailU32x2DeltaSpec {
                output_stride: 8,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 8],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 1,
    }];
    assert_eq!(
        transform_tail_u32x2_delta_into(
            &mut out,
            TransformTailU32x2DeltaSpec {
                output_stride: 8,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                matches: &[0],
                source0: &[],
                source1: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_i8x2_normal_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 3];
    assert_eq!(
        transform_tail_i8x2_normal_into(
            &mut out,
            TransformTailI8x2NormalSpec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                source0: &[0; 2],
                source1: &[0],
                source2: &[0],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_i8x2_normal_into(
            &mut out,
            TransformTailI8x2NormalSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                source0: &[0; 1],
                source1: &[0],
                source2: &[0],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );
    assert_eq!(
        transform_tail_i8x2_normal_into(
            &mut out,
            TransformTailI8x2NormalSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                source0: &[0; 2],
                source1: &[],
                source2: &[0],
            },
        ),
        Err(TransformTailDeltaError::Source1TooSmall)
    );
    assert_eq!(
        transform_tail_i8x2_normal_into(
            &mut out,
            TransformTailI8x2NormalSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                source0: &[0; 2],
                source1: &[0],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::Source2TooSmall)
    );

    let mut short_out = [0u8; 2];
    assert_eq!(
        transform_tail_i8x2_normal_into(
            &mut short_out,
            TransformTailI8x2NormalSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                source0: &[0; 2],
                source1: &[0],
                source2: &[0],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 1,
    }];
    assert_eq!(
        transform_tail_i8x2_normal_into(
            &mut out,
            TransformTailI8x2NormalSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                source0: &[],
                source1: &[],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_i8x3_normal_delta_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 3];
    assert_eq!(
        transform_tail_i8x3_normal_delta_into(
            &mut out,
            TransformTailI8x3NormalDeltaSpec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 2],
                source1: &[0],
                source2: &[0],
                source3: &[],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_i8x3_normal_delta_into(
            &mut out,
            TransformTailI8x3NormalDeltaSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[],
                source0: &[0; 2],
                source1: &[0],
                source2: &[0],
                source3: &[],
            },
        ),
        Err(TransformTailDeltaError::MatchTableTooSmall)
    );
    assert_eq!(
        transform_tail_i8x3_normal_delta_into(
            &mut out,
            TransformTailI8x3NormalDeltaSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 1],
                source1: &[0],
                source2: &[0],
                source3: &[],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );
    assert_eq!(
        transform_tail_i8x3_normal_delta_into(
            &mut out,
            TransformTailI8x3NormalDeltaSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 2],
                source1: &[],
                source2: &[0],
                source3: &[],
            },
        ),
        Err(TransformTailDeltaError::Source1TooSmall)
    );
    assert_eq!(
        transform_tail_i8x3_normal_delta_into(
            &mut out,
            TransformTailI8x3NormalDeltaSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 2],
                source1: &[0],
                source2: &[],
                source3: &[],
            },
        ),
        Err(TransformTailDeltaError::Source2TooSmall)
    );

    let matched = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    assert_eq!(
        transform_tail_i8x3_normal_delta_into(
            &mut out,
            TransformTailI8x3NormalDeltaSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[],
                source2: &[],
                source3: &[0; 3],
            },
        ),
        Err(TransformTailDeltaError::MatchBeforeOutput)
    );

    let mut matched_out = [0u8; 15];
    assert_eq!(
        transform_tail_i8x3_normal_delta_into(
            &mut matched_out,
            TransformTailI8x3NormalDeltaSpec {
                output_stride: 12,
                block_index: 0,
                out_offset: 12,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[],
                source2: &[],
                source3: &[0; 2],
            },
        ),
        Err(TransformTailDeltaError::Source3TooSmall)
    );

    let mut short_out = [0u8; 2];
    assert_eq!(
        transform_tail_i8x3_normal_delta_into(
            &mut short_out,
            TransformTailI8x3NormalDeltaSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0; 2],
                source1: &[0],
                source2: &[0],
                source3: &[],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 1,
    }];
    assert_eq!(
        transform_tail_i8x3_normal_delta_into(
            &mut out,
            TransformTailI8x3NormalDeltaSpec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                matches: &[0],
                source0: &[],
                source1: &[],
                source2: &[],
                source3: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_delta3_allows_observed_zero_literal_and_zero_copy() {
    let zero_literal = [TransformTailRecord {
        literal_count: 0,
        copy_count: 4,
        back_distance: 600,
    }];
    let mut out = vec![0xee; 639];
    for unit in 0..4 {
        let base = unit * 12;
        out[base] = unit as u8;
        out[base + 1] = 0x40 | unit as u8;
        out[base + 2] = 0x80 | unit as u8;
    }
    let usage = transform_tail_delta3_into(
        &mut out,
        TransformTailDelta3Spec {
            output_stride: 12,
            block_index: 0,
            out_offset: 600,
            records: &zero_literal,
            matches: &[0; 4],
            source0: &[],
            source1: &[],
            source2: &[],
        },
    )
    .unwrap();
    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 0,
            source1: 0,
            source2: 0,
            match_entries: 4,
        }
    );
    for unit in 0..4 {
        let base = 600 + unit * 12;
        assert_eq!(
            &out[base..base + 3],
            &[unit as u8, 0x40 | unit as u8, 0x80 | unit as u8]
        );
    }

    let zero_copy = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = vec![0xee; 12];
    let usage = transform_tail_delta3_into(
        &mut out,
        TransformTailDelta3Spec {
            output_stride: 12,
            block_index: 0,
            out_offset: 0,
            records: &zero_copy,
            matches: &[0],
            source0: &[0],
            source1: &[0, 0],
            source2: &[],
        },
    )
    .unwrap();
    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 1,
            source1: 2,
            source2: 0,
            match_entries: 1,
        }
    );
    assert_eq!(&out[0..3], &[0xff, 0x00, 0x00]);
}

#[test]
fn transform_tail_delta3_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 3];
    assert_eq!(
        transform_tail_delta3_into(
            &mut out,
            TransformTailDelta3Spec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0],
                source1: &[0, 0],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_delta3_into(
            &mut out,
            TransformTailDelta3Spec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[],
                source0: &[0],
                source1: &[0, 0],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::MatchTableTooSmall)
    );
    assert_eq!(
        transform_tail_delta3_into(
            &mut out,
            TransformTailDelta3Spec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[],
                source1: &[0, 0],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );
    assert_eq!(
        transform_tail_delta3_into(
            &mut out,
            TransformTailDelta3Spec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0],
                source1: &[0],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::Source1TooSmall)
    );

    let matched = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    assert_eq!(
        transform_tail_delta3_into(
            &mut out,
            TransformTailDelta3Spec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[],
                source2: &[0, 0, 0],
            },
        ),
        Err(TransformTailDeltaError::MatchBeforeOutput)
    );

    let mut matched_out = [0u8; 15];
    assert_eq!(
        transform_tail_delta3_into(
            &mut matched_out,
            TransformTailDelta3Spec {
                output_stride: 12,
                block_index: 0,
                out_offset: 12,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[],
                source2: &[0, 0],
            },
        ),
        Err(TransformTailDeltaError::Source2TooSmall)
    );

    let mut short_out = [0u8; 2];
    assert_eq!(
        transform_tail_delta3_into(
            &mut short_out,
            TransformTailDelta3Spec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0],
                source1: &[0, 0],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 1,
    }];
    assert_eq!(
        transform_tail_delta3_into(
            &mut out,
            TransformTailDelta3Spec {
                output_stride: 3,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                matches: &[0],
                source0: &[],
                source1: &[],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

#[test]
fn transform_tail_delta4_allows_observed_zero_literal_and_zero_copy() {
    let zero_literal = [TransformTailRecord {
        literal_count: 0,
        copy_count: 2,
        back_distance: 56,
    }];
    let mut out = vec![0xee; 74];
    out[0..4].copy_from_slice(&[1, 2, 3, 4]);
    out[14..18].copy_from_slice(&[5, 6, 7, 8]);
    let usage = transform_tail_delta4_into(
        &mut out,
        TransformTailDelta4Spec {
            output_stride: 14,
            block_index: 0,
            out_offset: 56,
            records: &zero_literal,
            matches: &[0; 2],
            source0: &[],
            source1: &[],
            source2: &[],
        },
    )
    .unwrap();
    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 0,
            source1: 0,
            source2: 0,
            match_entries: 2,
        }
    );
    assert_eq!(&out[56..60], &[1, 2, 3, 4]);
    assert_eq!(&out[70..74], &[5, 6, 7, 8]);

    let zero_copy = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = vec![0xee; 4];
    let usage = transform_tail_delta4_into(
        &mut out,
        TransformTailDelta4Spec {
            output_stride: 4,
            block_index: 0,
            out_offset: 0,
            records: &zero_copy,
            matches: &[0],
            source0: &[0x0b],
            source1: &[2, 3, 4],
            source2: &[],
        },
    )
    .unwrap();
    assert_eq!(
        usage,
        TransformTailDeltaUsage {
            source0: 1,
            source1: 3,
            source2: 0,
            match_entries: 1,
        }
    );
    assert_eq!(&out[0..4], &[1, 2, 3, 4]);
}

#[test]
fn transform_tail_delta4_rejects_malformed_inputs() {
    let direct = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    let mut out = [0u8; 4];
    assert_eq!(
        transform_tail_delta4_into(
            &mut out,
            TransformTailDelta4Spec {
                output_stride: 0,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0],
                source1: &[0, 0, 0],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::ZeroStride)
    );
    assert_eq!(
        transform_tail_delta4_into(
            &mut out,
            TransformTailDelta4Spec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[],
                source0: &[0],
                source1: &[0, 0, 0],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::MatchTableTooSmall)
    );
    assert_eq!(
        transform_tail_delta4_into(
            &mut out,
            TransformTailDelta4Spec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[],
                source1: &[0, 0, 0],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::Source0TooSmall)
    );
    assert_eq!(
        transform_tail_delta4_into(
            &mut out,
            TransformTailDelta4Spec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0],
                source1: &[0, 0],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::Source1TooSmall)
    );

    let matched = [TransformTailRecord {
        literal_count: 1,
        copy_count: 0,
        back_distance: 0,
    }];
    assert_eq!(
        transform_tail_delta4_into(
            &mut out,
            TransformTailDelta4Spec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[],
                source2: &[0, 0, 0, 0],
            },
        ),
        Err(TransformTailDeltaError::MatchBeforeOutput)
    );

    let mut matched_out = [0u8; 18];
    assert_eq!(
        transform_tail_delta4_into(
            &mut matched_out,
            TransformTailDelta4Spec {
                output_stride: 14,
                block_index: 0,
                out_offset: 14,
                records: &matched,
                matches: &[8],
                source0: &[],
                source1: &[],
                source2: &[0, 0, 0],
            },
        ),
        Err(TransformTailDeltaError::Source2TooSmall)
    );

    let mut short_out = [0u8; 3];
    assert_eq!(
        transform_tail_delta4_into(
            &mut short_out,
            TransformTailDelta4Spec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &direct,
                matches: &[0],
                source0: &[0],
                source1: &[0, 0, 0],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::OutputTooSmall)
    );

    let copy_first = [TransformTailRecord {
        literal_count: 0,
        copy_count: 1,
        back_distance: 1,
    }];
    assert_eq!(
        transform_tail_delta4_into(
            &mut out,
            TransformTailDelta4Spec {
                output_stride: 4,
                block_index: 0,
                out_offset: 0,
                records: &copy_first,
                matches: &[0],
                source0: &[],
                source1: &[],
                source2: &[],
            },
        ),
        Err(TransformTailDeltaError::CopyBeforeOutput)
    );
}

fn hex_bytes(s: &str) -> Vec<u8> {
    let h: Vec<u8> = s.bytes().filter(|b| b.is_ascii_hexdigit()).collect();
    h.chunks(2)
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
        .collect()
}

fn hex_u32_words(s: &str) -> Vec<u32> {
    hex_bytes(s)
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

fn hex_u64_words(s: &str) -> Vec<u64> {
    hex_bytes(s)
        .chunks_exact(8)
        .map(|chunk| {
            u64::from_le_bytes([
                chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
            ])
        })
        .collect()
}

fn sparse_payload(len: usize, chunks: &[(usize, &str)]) -> Vec<u8> {
    let mut payload = vec![0u8; len];
    for &(offset, hex) in chunks {
        let bytes = hex_bytes(hex);
        payload[offset..offset + bytes.len()].copy_from_slice(&bytes);
    }
    payload
}

fn hex_u16s(s: &str) -> Vec<u16> {
    let bytes = hex_bytes(s);
    assert_eq!(bytes.len() % 2, 0);
    bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

fn hex_width_records(s: &str) -> Vec<[u32; 2]> {
    let bytes = hex_bytes(s);
    assert_eq!(bytes.len() % 8, 0);
    bytes
        .chunks_exact(8)
        .map(|c| {
            [
                u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                u32::from_le_bytes([c[4], c[5], c[6], c[7]]),
            ]
        })
        .collect()
}

/// Bear freq call #0 (`trace_freq_all.py` / `freq_golden.py` bear_call0): slow
/// path only, M=512 → [95,408,7,1]+rem=1. Rules out `~nbits` after `nbits+=1`
/// and wrong ptr step without `^7`.
#[test]
fn rans_read_freqs_bear_call0() {
    const WIN: [u8; 13] = [
        0x88, 0xff, 0x4f, 0x53, 0x86, 0x0f, 0x38, 0x11, 0x17, 0xed, 0xa7, 0x42, 0x42,
    ];
    let r = rans_read_freqs(
        &WIN,
        RansFreqReader {
            ptr: 5,
            acc: 0x8d82720662f204b8,
            bitpos: 62,
        },
        RansFreqParams {
            count: 4,
            w3_init: 7,
            w4: 15,
            m: 512,
            initfreq: 102,
        },
    );
    assert_eq!(r.freqs, [95, 408, 7, 1]);
    assert_eq!(r.rem, 1);
    assert_eq!(r.reader.ptr, 0);
    assert_eq!(r.reader.acc, 0x204b9090a9fb45c0);
    assert_eq!(r.reader.bitpos, 58);
}

/// Bear freq call #2 (`freq_golden.py` bear_call2_allpaths): exercises slow,
/// run-length (`0x110e890`/`0x110e8e8`), and run-body (`0x110e900`) paths.
#[test]
fn rans_read_freqs_bear_call2_allpaths() {
    const WIN: [u8; 14] = [
        0xd9, 0x5d, 0xec, 0x75, 0x69, 0x8b, 0x11, 0x68, 0x2b, 0x87, 0xcb, 0x8b, 0x88, 0xff,
    ];
    let r = rans_read_freqs(
        &WIN,
        RansFreqReader {
            ptr: 6,
            acc: 0x45c44e03e194d3f8,
            bitpos: 58,
        },
        RansFreqParams {
            count: 5,
            w3_init: 7,
            w4: 15,
            m: 512,
            initfreq: 85,
        },
    );
    assert_eq!(r.freqs, [9, 496, 3, 2, 1]);
    assert_eq!(r.rem, 1);
    assert_eq!(r.reader.ptr, 0);
    assert_eq!(r.reader.acc, 0x34fff888bcb872b6);
    assert_eq!(r.reader.bitpos, 60);
}

/// Animal_Bass call #21 (`freq_golden.py` bass_call21): second model, M=128.
#[test]
fn rans_read_freqs_bass_call21() {
    const WIN: [u8; 10] = [0xd2, 0xa7, 0x9c, 0x93, 0xcf, 0xb3, 0xe0, 0x9b, 0x61, 0x8b];
    let r = rans_read_freqs(
        &WIN,
        RansFreqReader {
            ptr: 2,
            acc: 0x531000ed76178088,
            bitpos: 60,
        },
        RansFreqParams {
            count: 3,
            w3_init: 5,
            w4: 15,
            m: 128,
            initfreq: 32,
        },
    );
    assert_eq!(r.freqs, [6, 118, 3]);
    assert_eq!(r.rem, 1);
    assert_eq!(r.reader.ptr, 0);
    assert_eq!(r.reader.acc, 0x76178088b619b000);
    assert_eq!(r.reader.bitpos, 44);
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

/// Provenance: synthetic zstd block generated with libzstd 1.5.7 using a
/// raw-content dictionary `abcdefghijklmnop`, then stripped to one block
/// body. The sequence emits no literals and copies 16 bytes from history,
/// matching the MeshCodec state-2 tail contract at `0x11109e0`/`0x5ffb30`.
#[test]
fn zstd_window_can_copy_from_preloaded_history() {
    const HISTORY: &[u8] = b"abcdefghijklmnop";
    const PAYLOAD: [u8; 7] = [0x06, 0x00, 0x01, 0x00, 0x83, 0x4c, 0x20];
    let (out, pos) = decode_zstd_window_with_history(&PAYLOAD, 0, HISTORY).unwrap();
    assert_eq!(out, HISTORY);
    assert_eq!(pos, PAYLOAD.len());
    assert!(matches!(
        decode_zstd_window(&PAYLOAD, 0),
        Err(zstd_pure::ZstdError::OffsetTooLarge { .. })
    ));
}

#[test]
fn zstd_window_rejects_truncated_body() {
    let payload = [0x06u8, 0x00, 0x01];
    match decode_zstd_window(&payload, 0) {
        Err(zstd_pure::ZstdError::Truncated {
            what: "zstd window body",
            needed: 4,
        }) => {}
        other => panic!("expected truncated zstd window body, got {other:?}"),
    }
}
