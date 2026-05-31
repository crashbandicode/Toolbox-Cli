//! MSBT writers.
//!
//! Two paths, mirroring [`crate::byml`]:
//!
//! - [`write_msbt`] re-emits the bytes captured at parse time, so an unmodified
//!   document round-trips **byte-identically** by construction.
//! - [`write_msbt_canonical`] rebuilds the file from the decoded sections
//!   (re-encoding `LBL1` from the labels and `TXT2` from the messages, copying
//!   other sections verbatim). Its guarantee is the **semantic** round-trip
//!   `read(write(x)) == read(x)` — like BYML's canonical writer, the exact byte
//!   layout is writer-specific (label bucket ordering, section padding), so it
//!   is not byte-identical to the game's encoder. Use it after editing a
//!   message or label.

use super::error::Result;
use super::{
    label_hash, Label, Message, MsbtDocument, Section, SectionData, HEADER_LEN, PAD_BYTE,
    SECTION_ALIGN, SECTION_HEADER_LEN,
};

/// Serialize an MSBT document verbatim — byte-identical to the input for an
/// unmodified document.
pub fn write_msbt(doc: &MsbtDocument) -> Result<Vec<u8>> {
    Ok(doc.raw.clone())
}

/// Rebuild an MSBT document from its decoded sections.
///
/// Re-encodes `LBL1` (rehashing labels into [`MsbtDocument::lbl1_groups`]
/// buckets) and `TXT2` (from the messages), copying any other section verbatim.
/// Semantically lossless (`read(write(x)) == read(x)`) but not byte-identical
/// to Nintendo's encoder.
pub fn write_msbt_canonical(doc: &MsbtDocument) -> Result<Vec<u8>> {
    let be = doc.big_endian;
    let mut out = Vec::new();

    // --- header (file_size back-patched once the body is laid out) ---
    out.extend_from_slice(b"MsgStdBn");
    out.extend_from_slice(if be { &[0xFE, 0xFF] } else { &[0xFF, 0xFE] });
    out.extend_from_slice(&[0, 0]); // unknown
    out.push(doc.encoding.to_u8());
    out.push(doc.version);
    out.extend_from_slice(&u16_bytes(doc.sections.len() as u16, be));
    out.extend_from_slice(&[0, 0]); // unknown
    let file_size_pos = out.len();
    out.extend_from_slice(&[0, 0, 0, 0]); // file_size placeholder
    out.extend_from_slice(&[0u8; 10]); // padding
    debug_assert_eq!(out.len(), HEADER_LEN);

    // --- sections ---
    let group_count = if doc.lbl1_groups == 0 {
        1
    } else {
        doc.lbl1_groups
    };
    for section in &doc.sections {
        let body = encode_section(section, group_count, be);
        out.extend_from_slice(&section.magic);
        out.extend_from_slice(&u32_bytes(body.len() as u32, be));
        out.extend_from_slice(&[0u8; 8]); // section-header padding
        debug_assert_eq!(out.len() % SECTION_HEADER_LEN, 0);
        out.extend_from_slice(&body);
        pad_to(&mut out, SECTION_ALIGN);
    }

    let file_size = out.len() as u32;
    out[file_size_pos..file_size_pos + 4].copy_from_slice(&u32_bytes(file_size, be));
    Ok(out)
}

/// Encode one section's body (excluding the 0x10 header and trailing padding).
fn encode_section(section: &Section, group_count: u32, be: bool) -> Vec<u8> {
    match &section.data {
        SectionData::Labels(labels) => encode_lbl1(labels, group_count, be),
        SectionData::Text(messages) => encode_txt2(messages, be),
        SectionData::Opaque(bytes) => bytes.clone(),
    }
}

/// Encode an `LBL1` body: `u32 group_count`, a `{count, offset}` bucket table,
/// then the label entries grouped by `label_hash % group_count`. Offsets are
/// relative to the section body start. Within a bucket, labels keep their input
/// order (the reader is order-insensitive, so this is a stable choice).
fn encode_lbl1(labels: &[Label], group_count: u32, be: bool) -> Vec<u8> {
    let n = group_count.max(1) as usize;
    let mut buckets: Vec<Vec<&Label>> = vec![Vec::new(); n];
    for l in labels {
        let idx = (label_hash(&l.name) % group_count.max(1)) as usize;
        buckets[idx].push(l);
    }

    // Each entry is: u8 name_len, name bytes, u32 message index.
    let entry_len = |l: &Label| 1 + l.name.len() + 4;

    let table_size = 4 + n * 8;
    let mut header = Vec::with_capacity(table_size);
    header.extend_from_slice(&u32_bytes(group_count, be));

    let mut entries = Vec::new();
    let mut offset = table_size;
    for bucket in &buckets {
        header.extend_from_slice(&u32_bytes(bucket.len() as u32, be));
        header.extend_from_slice(&u32_bytes(offset as u32, be));
        for l in bucket {
            offset += entry_len(l);
            entries.push(l.name.len() as u8);
            entries.extend_from_slice(l.name.as_bytes());
            entries.extend_from_slice(&u32_bytes(l.index, be));
        }
    }
    header.extend_from_slice(&entries);
    header
}

/// Encode a `TXT2` body: `u32 count`, a `u32` offset table (relative to the
/// body start), then each message's raw bytes (which already include the
/// terminator) concatenated.
fn encode_txt2(messages: &[Message], be: bool) -> Vec<u8> {
    let count = messages.len();
    let table_size = 4 + count * 4;
    let mut out = Vec::new();
    out.extend_from_slice(&u32_bytes(count as u32, be));

    let mut offset = table_size;
    for m in messages {
        out.extend_from_slice(&u32_bytes(offset as u32, be));
        offset += m.raw.len();
    }
    for m in messages {
        out.extend_from_slice(&m.raw);
    }
    out
}

fn pad_to(buf: &mut Vec<u8>, align: usize) {
    while !buf.len().is_multiple_of(align) {
        buf.push(PAD_BYTE);
    }
}

fn u16_bytes(v: u16, be: bool) -> [u8; 2] {
    if be {
        v.to_be_bytes()
    } else {
        v.to_le_bytes()
    }
}

fn u32_bytes(v: u32, be: bool) -> [u8; 4] {
    if be {
        v.to_be_bytes()
    } else {
        v.to_le_bytes()
    }
}
