//! MSBT (LibMessageStudio binary message) read + write.
//!
//! MSBT (`MsgStdBn`) is Nintendo's binary message container — the localized
//! text format for menus, dialogue, item names, etc. On Switch it ships inside
//! the `Mals/*.sarc.zs` archives (a zstd SARC of `.msbt` files). A 0x20-byte
//! header (magic, byte-order mark, encoding, version, section count, file size)
//! is followed by a sequence of 0x10-byte-headed sections padded with `0xAB` to
//! a 16-byte boundary. The two sections that carry the actual data are:
//!
//! - **`LBL1`** — a label hash table mapping a string *label* (e.g.
//!   `Talk_0001`) to an index into the text section.
//! - **`TXT2`** — the messages themselves: a `u32` count, a table of `u32`
//!   offsets, then NUL-terminated strings in the file's [`Encoding`] (UTF-16LE
//!   for Tears of the Kingdom) with inline control tags.
//!
//! Other sections (`ATR1` attributes, `NLI1`, `TSY1`, `ATO1`, …) are retained
//! opaquely — they are reproduced verbatim by the writer but not separately
//! decoded.
//!
//! ## Round-trip discipline
//!
//! Like [`crate::byml`], the on-disk bytes depend on writer-specific choices
//! (hash-group count, section ordering, `0xAB` padding), so [`read_msbt`]
//! retains the original bytes and [`write_msbt`] re-emits them **verbatim** —
//! byte-identical by construction for an unmodified document. A from-scratch
//! canonical writer (for edited message trees) is a follow-up.

mod error;
mod read;
mod write;

pub use error::{MsbtError, Result};
pub use read::read_msbt;
pub use write::{write_msbt, write_msbt_canonical};

/// MSBT section header magic + size fields occupy a fixed 0x10 bytes; the
/// section body follows immediately after.
pub(crate) const SECTION_HEADER_LEN: usize = 0x10;
/// The fixed file header length.
pub(crate) const HEADER_LEN: usize = 0x20;
/// Sections are padded to this boundary with [`PAD_BYTE`].
pub(crate) const SECTION_ALIGN: usize = 0x10;
/// MSBT pads section tails with this byte (not 0x00). Used by the tests now
/// and by the canonical writer (a follow-up); the verbatim writer needs no
/// padding logic.
#[allow(dead_code)]
pub(crate) const PAD_BYTE: u8 = 0xAB;

/// Control-tag markers embedded in [`Encoding::Utf16`] text. `0x000E` opens a
/// tag (group, type, size, payload); `0x000F` closes one (group, type).
pub(crate) const TAG_OPEN: u16 = 0x000E;
pub(crate) const TAG_CLOSE: u16 = 0x000F;

/// Text encoding used by the [`TXT2`](SectionData::Text) strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// UTF-8 (`0`).
    Utf8,
    /// UTF-16 (`1`) — the common Switch case.
    Utf16,
    /// UTF-32 (`2`).
    Utf32,
}

impl Encoding {
    /// Decode the header's encoding byte.
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0 => Ok(Encoding::Utf8),
            1 => Ok(Encoding::Utf16),
            2 => Ok(Encoding::Utf32),
            other => Err(MsbtError::UnsupportedEncoding(other)),
        }
    }

    /// The on-disk encoding byte.
    pub fn to_u8(self) -> u8 {
        match self {
            Encoding::Utf8 => 0,
            Encoding::Utf16 => 1,
            Encoding::Utf32 => 2,
        }
    }

    /// A short label for diagnostics.
    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf16 => "UTF-16",
            Encoding::Utf32 => "UTF-32",
        }
    }
}

/// A single `LBL1` label: its ASCII name and the [`TXT2`](SectionData::Text)
/// message index it points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    /// The label name (e.g. `Talk_Greeting_01`).
    pub name: String,
    /// Index into the text section's message list.
    pub index: u32,
}

/// A decoded MSBT message — a sequence of plain-text runs and control tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// The exact encoded bytes of this entry (the slice between this message's
    /// offset and the next, including the NUL terminator). Retained so the
    /// message survives round-trip regardless of how well the tag decoder
    /// understands its contents.
    pub raw: Vec<u8>,
}

/// One chunk of a decoded [`Message`]: either literal text or a control tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextChunk {
    /// A run of literal text (newlines/tabs included verbatim).
    Text(String),
    /// An opening control tag (`0x000E`): a `group`/`type` pair plus an opaque
    /// payload (often a `u16`-prefixed UTF-16 argument).
    Tag { group: u16, ty: u16, data: Vec<u8> },
    /// A closing control tag (`0x000F`): a `group`/`type` pair.
    TagClose { group: u16, ty: u16 },
}

impl Message {
    /// Decode this message into text runs + control tags for the given
    /// encoding/endianness. UTF-16 is fully tag-aware; other encodings return
    /// a single best-effort [`TextChunk::Text`] chunk.
    pub fn chunks(&self, encoding: Encoding, big_endian: bool) -> Vec<TextChunk> {
        match encoding {
            Encoding::Utf16 => decode_utf16_chunks(&self.raw, big_endian),
            Encoding::Utf8 => {
                let s = String::from_utf8_lossy(trim_terminator_u8(&self.raw)).into_owned();
                if s.is_empty() {
                    Vec::new()
                } else {
                    vec![TextChunk::Text(s)]
                }
            }
            Encoding::Utf32 => vec![TextChunk::Text(format!(
                "<utf-32 {} bytes>",
                self.raw.len()
            ))],
        }
    }

    /// Build a message from text chunks for the given encoding/endianness —
    /// the inverse of [`Message::chunks`]. UTF-16 is fully tag-aware (re-emits
    /// `0x000E`/`0x000F` markers + a NUL terminator); for UTF-8 the text chunks
    /// are concatenated (tag payloads appended raw) with a NUL terminator.
    pub fn from_chunks(chunks: &[TextChunk], encoding: Encoding, big_endian: bool) -> Message {
        let raw = match encoding {
            Encoding::Utf16 => encode_utf16_chunks(chunks, big_endian),
            Encoding::Utf8 => {
                let mut v = Vec::new();
                for c in chunks {
                    match c {
                        TextChunk::Text(t) => v.extend_from_slice(t.as_bytes()),
                        TextChunk::Tag { data, .. } => v.extend_from_slice(data),
                        TextChunk::TagClose { .. } => {}
                    }
                }
                v.push(0);
                v
            }
            Encoding::Utf32 => Vec::new(),
        };
        Message { raw }
    }

    /// A human-readable rendering of the message: literal text with control
    /// tags shown as `[tag g=.. t=.. n=..]` / `[/tag g=.. t=..]`. Lossy — for
    /// display only (a reversible JSON codec is a follow-up).
    pub fn to_display(&self, encoding: Encoding, big_endian: bool) -> String {
        let mut out = String::new();
        for chunk in self.chunks(encoding, big_endian) {
            match chunk {
                TextChunk::Text(t) => out.push_str(&t),
                TextChunk::Tag { group, ty, data } => {
                    out.push_str(&format!("[tag g={group} t={ty} n={}]", data.len()))
                }
                TextChunk::TagClose { group, ty } => {
                    out.push_str(&format!("[/tag g={group} t={ty}]"))
                }
            }
        }
        out
    }
}

/// Decoded payload of an MSBT section.
#[derive(Debug, Clone)]
pub enum SectionData {
    /// `LBL1` — label → message-index table.
    Labels(Vec<Label>),
    /// `TXT2` — the message list.
    Text(Vec<Message>),
    /// Any other section, retained as raw body bytes (re-emitted verbatim).
    Opaque(Vec<u8>),
}

/// A parsed MSBT section: its 4-byte magic and decoded payload.
#[derive(Debug, Clone)]
pub struct Section {
    /// The 4-byte section magic (`LBL1`, `TXT2`, `ATR1`, …).
    pub magic: [u8; 4],
    /// The decoded payload.
    pub data: SectionData,
}

impl Section {
    /// The magic as a UTF-8 string (lossy) for diagnostics.
    pub fn magic_str(&self) -> String {
        String::from_utf8_lossy(&self.magic).into_owned()
    }
}

/// A parsed MSBT document.
///
/// Retains the original bytes (`raw`) so an unmodified document re-emits
/// byte-identically via [`write_msbt`].
#[derive(Debug, Clone)]
pub struct MsbtDocument {
    /// `true` for big-endian (`FEFF`), `false` for little-endian (`FFFE`).
    pub big_endian: bool,
    /// Text encoding of the `TXT2` strings.
    pub encoding: Encoding,
    /// Format version (3 for Tears of the Kingdom).
    pub version: u8,
    /// Sections in file order.
    pub sections: Vec<Section>,
    /// The `LBL1` hash-bucket count captured at read time (0 if no `LBL1`).
    /// The canonical writer rehashes labels into this many buckets so the
    /// rebuilt table matches the game's lookup convention.
    pub(crate) lbl1_groups: u32,
    /// The original file bytes, used for the verbatim [`write_msbt`] path.
    pub(crate) raw: Vec<u8>,
}

impl MsbtDocument {
    /// The original bytes captured at parse time.
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// The labels from the first `LBL1` section, if present.
    pub fn labels(&self) -> Option<&[Label]> {
        self.sections.iter().find_map(|s| match &s.data {
            SectionData::Labels(l) => Some(l.as_slice()),
            _ => None,
        })
    }

    /// The messages from the first `TXT2` section, if present.
    pub fn messages(&self) -> Option<&[Message]> {
        self.sections.iter().find_map(|s| match &s.data {
            SectionData::Text(t) => Some(t.as_slice()),
            _ => None,
        })
    }

    /// Replace the message a label points at. Returns `false` if the label is
    /// unknown or its index is out of range (no change made). Serialize with
    /// [`write_msbt_canonical`] afterward.
    pub fn set_message_by_label(&mut self, label: &str, message: Message) -> bool {
        let Some(index) = self.labels().and_then(|ls| {
            ls.iter()
                .find(|l| l.name == label)
                .map(|l| l.index as usize)
        }) else {
            return false;
        };
        for s in &mut self.sections {
            if let SectionData::Text(messages) = &mut s.data {
                if index < messages.len() {
                    messages[index] = message;
                    return true;
                }
            }
        }
        false
    }

    /// Pair each label with the message it points at, sorted by label name.
    /// Labels whose index is out of range are skipped.
    pub fn entries(&self) -> Vec<(&str, &Message)> {
        let (Some(labels), Some(messages)) = (self.labels(), self.messages()) else {
            return Vec::new();
        };
        let mut out: Vec<(&str, &Message)> = labels
            .iter()
            .filter_map(|l| messages.get(l.index as usize).map(|m| (l.name.as_str(), m)))
            .collect();
        out.sort_by(|a, b| a.0.cmp(b.0));
        out
    }
}

/// Decode a UTF-16 message body into text runs + control tags. `0x000E` opens
/// a tag (group, type, size-in-bytes, payload); `0x000F` closes one; a trailing
/// `0x0000` terminator ends the message. Malformed trailing tags are tolerated
/// (decode stops cleanly) — the verbatim writer is unaffected.
fn decode_utf16_chunks(raw: &[u8], big_endian: bool) -> Vec<TextChunk> {
    let cu = |o: usize| -> u16 {
        let b = [raw[o], raw[o + 1]];
        if big_endian {
            u16::from_be_bytes(b)
        } else {
            u16::from_le_bytes(b)
        }
    };

    let mut chunks = Vec::new();
    let mut text: Vec<u16> = Vec::new();
    let mut i = 0usize;

    let flush = |text: &mut Vec<u16>, chunks: &mut Vec<TextChunk>| {
        if !text.is_empty() {
            chunks.push(TextChunk::Text(String::from_utf16_lossy(text)));
            text.clear();
        }
    };

    while i + 2 <= raw.len() {
        let c = cu(i);
        if c == 0 {
            // NUL terminator: end of message.
            break;
        }
        match c {
            TAG_OPEN => {
                if i + 8 > raw.len() {
                    break;
                }
                let group = cu(i + 2);
                let ty = cu(i + 4);
                let size = cu(i + 6) as usize;
                let data_start = i + 8;
                if data_start + size > raw.len() {
                    break;
                }
                flush(&mut text, &mut chunks);
                chunks.push(TextChunk::Tag {
                    group,
                    ty,
                    data: raw[data_start..data_start + size].to_vec(),
                });
                i = data_start + size;
            }
            TAG_CLOSE => {
                if i + 6 > raw.len() {
                    break;
                }
                flush(&mut text, &mut chunks);
                chunks.push(TextChunk::TagClose {
                    group: cu(i + 2),
                    ty: cu(i + 4),
                });
                i += 6;
            }
            other => {
                text.push(other);
                i += 2;
            }
        }
    }
    flush(&mut text, &mut chunks);
    chunks
}

/// Encode text chunks back into a UTF-16 message body (the inverse of
/// [`decode_utf16_chunks`]): text runs as code units, `0x000E` open tags
/// (group/type/size/payload) and `0x000F` close tags (group/type), then a
/// `0x0000` terminator.
fn encode_utf16_chunks(chunks: &[TextChunk], big_endian: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let push = |u: u16, out: &mut Vec<u8>| {
        out.extend_from_slice(&if big_endian {
            u.to_be_bytes()
        } else {
            u.to_le_bytes()
        });
    };
    for chunk in chunks {
        match chunk {
            TextChunk::Text(t) => {
                for u in t.encode_utf16() {
                    push(u, &mut out);
                }
            }
            TextChunk::Tag { group, ty, data } => {
                push(TAG_OPEN, &mut out);
                push(*group, &mut out);
                push(*ty, &mut out);
                push(data.len() as u16, &mut out);
                out.extend_from_slice(data);
            }
            TextChunk::TagClose { group, ty } => {
                push(TAG_CLOSE, &mut out);
                push(*group, &mut out);
                push(*ty, &mut out);
            }
        }
    }
    push(0, &mut out); // NUL terminator
    out
}

/// The LMS label hash: `h = h*0x492 + byte` (wrapping u32) over the label's
/// ASCII bytes. The bucket is `hash % group_count`. Verified against the whole
/// TotK `Mals` corpus (every label lands in the bucket it is stored under).
pub(crate) fn label_hash(name: &str) -> u32 {
    let mut h: u32 = 0;
    for b in name.bytes() {
        h = h.wrapping_mul(0x492).wrapping_add(b as u32);
    }
    h
}

/// Drop a trailing single-byte NUL terminator (UTF-8 messages).
fn trim_terminator_u8(raw: &[u8]) -> &[u8] {
    match raw.last() {
        Some(0) => &raw[..raw.len() - 1],
        _ => raw,
    }
}
