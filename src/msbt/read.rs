//! MSBT parser: header → section walk → LBL1 / TXT2 decode.
//!
//! All reads are bounds-checked and report the failing offset. The whole file
//! is retained for the verbatim [`write_msbt`](super::write_msbt) round-trip;
//! decoding LBL1 + TXT2 here proves the parser walks the entire document.

use super::error::{MsbtError, Result};
use super::*;

/// Parse an MSBT document, retaining the original bytes for the verbatim
/// round-trip.
pub fn read_msbt(data: &[u8]) -> Result<MsbtDocument> {
    if data.len() < HEADER_LEN {
        return Err(MsbtError::TooSmall(data.len()));
    }
    if &data[0..8] != b"MsgStdBn" {
        let mut magic = [0u8; 8];
        magic.copy_from_slice(&data[0..8]);
        return Err(MsbtError::BadMagic(magic));
    }
    let bom = [data[8], data[9]];
    let big_endian = match bom {
        [0xFE, 0xFF] => true,
        [0xFF, 0xFE] => false,
        _ => return Err(MsbtError::BadBom(bom)),
    };
    let encoding = Encoding::from_u8(data[0x0C])?;
    let version = data[0x0D];
    let section_count = read_u16(data, 0x0E, big_endian)? as usize;

    let mut sections = Vec::with_capacity(section_count);
    let mut lbl1_groups = 0u32;
    let mut off = HEADER_LEN;
    for index in 0..section_count {
        if off + SECTION_HEADER_LEN > data.len() {
            return Err(MsbtError::Truncated {
                offset: off,
                need: SECTION_HEADER_LEN,
                len: data.len(),
            });
        }
        let mut magic = [0u8; 4];
        magic.copy_from_slice(&data[off..off + 4]);
        let size = read_u32(data, off + 4, big_endian)? as usize;
        let body = off + SECTION_HEADER_LEN;
        if body + size > data.len() {
            return Err(MsbtError::SectionOutOfRange {
                magic: String::from_utf8_lossy(&magic).into_owned(),
                index,
                offset: off,
                size,
                len: data.len(),
            });
        }
        let body_bytes = &data[body..body + size];
        let payload = match &magic {
            b"LBL1" => {
                let (labels, groups) = read_lbl1(body_bytes, big_endian)?;
                if lbl1_groups == 0 {
                    lbl1_groups = groups;
                }
                SectionData::Labels(labels)
            }
            b"TXT2" => SectionData::Text(read_txt2(body_bytes, big_endian)?),
            _ => SectionData::Opaque(body_bytes.to_vec()),
        };
        sections.push(Section {
            magic,
            data: payload,
        });

        // Advance past the body and its 0xAB padding to the next 16-byte
        // boundary.
        off = align_up(body + size, SECTION_ALIGN);
    }

    Ok(MsbtDocument {
        big_endian,
        encoding,
        version,
        sections,
        lbl1_groups,
        raw: data.to_vec(),
    })
}

/// Decode an `LBL1` section body: a hash table of `ngroups` buckets, each
/// `{count, offset}`, with the label entries (`len`-prefixed ASCII name +
/// `u32` message index) living at the bucket offsets (relative to the section
/// body start).
fn read_lbl1(body: &[u8], big_endian: bool) -> Result<(Vec<Label>, u32)> {
    if body.len() < 4 {
        return Err(MsbtError::Truncated {
            offset: 0,
            need: 4,
            len: body.len(),
        });
    }
    let ngroups = read_u32(body, 0, big_endian)? as usize;
    let mut labels = Vec::new();
    for g in 0..ngroups {
        let head = 4 + g * 8;
        let count = read_u32(body, head, big_endian)? as usize;
        let mut p = read_u32(body, head + 4, big_endian)? as usize;
        for _ in 0..count {
            if p >= body.len() {
                return Err(MsbtError::OffsetOutOfRange {
                    section: "LBL1",
                    index: labels.len(),
                    offset: p,
                    size: body.len(),
                });
            }
            let name_len = body[p] as usize;
            let name_start = p + 1;
            let idx_start = name_start + name_len;
            if idx_start + 4 > body.len() {
                return Err(MsbtError::OffsetOutOfRange {
                    section: "LBL1",
                    index: labels.len(),
                    offset: p,
                    size: body.len(),
                });
            }
            let name = std::str::from_utf8(&body[name_start..idx_start])
                .map_err(|source| MsbtError::NonUtf8 {
                    offset: name_start,
                    source,
                })?
                .to_string();
            let index = read_u32(body, idx_start, big_endian)?;
            labels.push(Label { name, index });
            p = idx_start + 4;
        }
    }
    Ok((labels, ngroups as u32))
}

/// Decode a `TXT2` section body: a `u32` count, a table of `u32` offsets
/// (relative to the section body start), then the messages (the bytes between
/// consecutive offsets, including the terminator).
fn read_txt2(body: &[u8], big_endian: bool) -> Result<Vec<Message>> {
    if body.len() < 4 {
        return Err(MsbtError::Truncated {
            offset: 0,
            need: 4,
            len: body.len(),
        });
    }
    let count = read_u32(body, 0, big_endian)? as usize;
    let mut offsets = Vec::with_capacity(count + 1);
    for i in 0..count {
        offsets.push(read_u32(body, 4 + i * 4, big_endian)? as usize);
    }
    offsets.push(body.len());

    let mut messages = Vec::with_capacity(count);
    for i in 0..count {
        let start = offsets[i];
        let end = offsets[i + 1];
        if start > end || end > body.len() {
            return Err(MsbtError::OffsetOutOfRange {
                section: "TXT2",
                index: i,
                offset: start,
                size: body.len(),
            });
        }
        messages.push(Message {
            raw: body[start..end].to_vec(),
        });
    }
    Ok(messages)
}

/// Round `x` up to the next multiple of `align` (a power of two).
fn align_up(x: usize, align: usize) -> usize {
    (x + align - 1) & !(align - 1)
}

fn read_u16(data: &[u8], off: usize, big_endian: bool) -> Result<u16> {
    match off.checked_add(2) {
        Some(e) if e <= data.len() => {
            let b = [data[off], data[off + 1]];
            Ok(if big_endian {
                u16::from_be_bytes(b)
            } else {
                u16::from_le_bytes(b)
            })
        }
        _ => Err(MsbtError::Truncated {
            offset: off,
            need: 2,
            len: data.len(),
        }),
    }
}

fn read_u32(data: &[u8], off: usize, big_endian: bool) -> Result<u32> {
    match off.checked_add(4) {
        Some(e) if e <= data.len() => {
            let b = [data[off], data[off + 1], data[off + 2], data[off + 3]];
            Ok(if big_endian {
                u32::from_be_bytes(b)
            } else {
                u32::from_le_bytes(b)
            })
        }
        _ => Err(MsbtError::Truncated {
            offset: off,
            need: 4,
            len: data.len(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-built minimal little-endian UTF-16 MSBT with one LBL1 (a single
    /// group holding two labels) and a TXT2 of two messages — one of which
    /// carries a `0x000E` control tag. Fixture-free so CI (no game assets) has
    /// a correctness net for the reader + the tag decoder.
    fn minimal_msbt() -> Vec<u8> {
        fn pad_to(b: &mut Vec<u8>, align: usize) {
            while !b.len().is_multiple_of(align) {
                b.push(PAD_BYTE);
            }
        }
        fn utf16(s: &str) -> Vec<u8> {
            let mut v = Vec::new();
            for u in s.encode_utf16() {
                v.extend_from_slice(&u.to_le_bytes());
            }
            v
        }

        // --- TXT2 body ---
        // msg 0: "Hi" + NUL
        let mut m0 = utf16("Hi");
        m0.extend_from_slice(&0u16.to_le_bytes());
        // msg 1: "A" + tag(group=1,type=2,data="!") + NUL
        let mut m1 = utf16("A");
        m1.extend_from_slice(&TAG_OPEN.to_le_bytes());
        m1.extend_from_slice(&1u16.to_le_bytes()); // group
        m1.extend_from_slice(&2u16.to_le_bytes()); // type
        let tag_data = utf16("!");
        m1.extend_from_slice(&(tag_data.len() as u16).to_le_bytes());
        m1.extend_from_slice(&tag_data);
        m1.extend_from_slice(&0u16.to_le_bytes());

        let mut txt2 = Vec::new();
        txt2.extend_from_slice(&2u32.to_le_bytes()); // count
        let table = 4 + 2 * 4; // count + two offsets
        txt2.extend_from_slice(&((table) as u32).to_le_bytes()); // off[0]
        txt2.extend_from_slice(&((table + m0.len()) as u32).to_le_bytes()); // off[1]
        txt2.extend_from_slice(&m0);
        txt2.extend_from_slice(&m1);

        // --- LBL1 body ---
        // one group, two labels: "Greeting"->0, "Reply"->1
        let mut lbl1 = Vec::new();
        lbl1.extend_from_slice(&1u32.to_le_bytes()); // ngroups
        lbl1.extend_from_slice(&2u32.to_le_bytes()); // group 0 count
        lbl1.extend_from_slice(&12u32.to_le_bytes()); // group 0 offset (4 + 8)
        // entries at offset 12
        for (name, idx) in [("Greeting", 0u32), ("Reply", 1u32)] {
            lbl1.push(name.len() as u8);
            lbl1.extend_from_slice(name.as_bytes());
            lbl1.extend_from_slice(&idx.to_le_bytes());
        }

        // --- assemble ---
        let mut b = Vec::new();
        b.extend_from_slice(b"MsgStdBn");
        b.extend_from_slice(&[0xFF, 0xFE]); // LE BOM
        b.extend_from_slice(&0u16.to_le_bytes()); // unknown
        b.push(1); // encoding = UTF-16
        b.push(3); // version
        b.extend_from_slice(&2u16.to_le_bytes()); // section count
        b.extend_from_slice(&0u16.to_le_bytes()); // unknown
        let fsize_pos = b.len();
        b.extend_from_slice(&0u32.to_le_bytes()); // file size (patched below)
        b.extend_from_slice(&[0u8; 10]); // padding
        assert_eq!(b.len(), HEADER_LEN);

        for (magic, body) in [(b"LBL1", &lbl1), (b"TXT2", &txt2)] {
            b.extend_from_slice(magic);
            b.extend_from_slice(&(body.len() as u32).to_le_bytes());
            b.extend_from_slice(&[0u8; 8]); // padding
            b.extend_from_slice(body);
            pad_to(&mut b, SECTION_ALIGN);
        }

        let fsize = b.len() as u32;
        b[fsize_pos..fsize_pos + 4].copy_from_slice(&fsize.to_le_bytes());
        b
    }

    #[test]
    fn parses_minimal() {
        let bytes = minimal_msbt();
        let doc = read_msbt(&bytes).expect("parse");
        assert_eq!(doc.version, 3);
        assert!(!doc.big_endian);
        assert_eq!(doc.encoding, Encoding::Utf16);
        assert_eq!(doc.sections.len(), 2);

        let labels = doc.labels().expect("labels");
        assert_eq!(labels.len(), 2);
        let messages = doc.messages().expect("messages");
        assert_eq!(messages.len(), 2);

        // entries() pairs label -> message, sorted by label name.
        let entries = doc.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, "Greeting");
        assert_eq!(entries[1].0, "Reply");

        // Message 0 decodes to plain text.
        assert_eq!(
            messages[0].chunks(Encoding::Utf16, false),
            vec![TextChunk::Text("Hi".into())]
        );
        // Message 1 decodes to text + a control tag.
        assert_eq!(
            messages[1].chunks(Encoding::Utf16, false),
            vec![
                TextChunk::Text("A".into()),
                TextChunk::Tag {
                    group: 1,
                    ty: 2,
                    data: "!".encode_utf16().flat_map(u16::to_le_bytes).collect(),
                },
            ]
        );

        // Verbatim writer reproduces the input byte-for-byte.
        assert_eq!(write_msbt(&doc).unwrap(), bytes);
    }

    #[test]
    fn canonical_writer_semantic_round_trips() {
        let bytes = minimal_msbt();
        let doc = read_msbt(&bytes).expect("parse");
        // The canonical writer rebuilds from the decoded sections; re-reading
        // it must yield the same labels + messages (semantic round-trip), even
        // though the bytes need not match the input.
        let rebuilt = write_msbt_canonical(&doc).expect("canonical write");
        let doc2 = read_msbt(&rebuilt).expect("re-parse canonical");
        assert_eq!(doc2.labels(), doc.labels());
        assert_eq!(doc2.messages(), doc.messages());
        assert_eq!(doc2.entries(), doc.entries());
        // Single group in the source -> the rebuilt file preserves the count.
        assert_eq!(doc2.lbl1_groups, doc.lbl1_groups);
    }

    #[test]
    fn rejects_too_small() {
        assert!(matches!(read_msbt(&[0u8; 8]), Err(MsbtError::TooSmall(8))));
    }

    #[test]
    fn rejects_bad_magic() {
        let mut b = vec![0u8; HEADER_LEN];
        b[0..8].copy_from_slice(b"NOTMSBT!");
        assert!(matches!(read_msbt(&b), Err(MsbtError::BadMagic(_))));
    }

    #[test]
    fn rejects_bad_bom() {
        let mut b = vec![0u8; HEADER_LEN];
        b[0..8].copy_from_slice(b"MsgStdBn");
        b[8] = 0x12;
        b[9] = 0x34;
        assert!(matches!(read_msbt(&b), Err(MsbtError::BadBom(_))));
    }

    #[test]
    fn rejects_section_out_of_range() {
        let mut b = minimal_msbt();
        // Corrupt the first section's size to overrun the file.
        b[HEADER_LEN + 4..HEADER_LEN + 8].copy_from_slice(&0xFFFF_u32.to_le_bytes());
        assert!(matches!(
            read_msbt(&b),
            Err(MsbtError::SectionOutOfRange { .. })
        ));
    }
}
