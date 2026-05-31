//! AAMP serialization.
//!
//! [`write_aamp`] is **verbatim**: it returns the bytes captured at read time,
//! so an *unmodified* document round-trips byte-identically — the same
//! discipline [`crate::byml`] / [`crate::msbt`] use. A from-scratch canonical
//! writer (for mutated / synthesized trees, re-emitting the header + node
//! arrays + de-duplicated data/string sections) is the next stage.

use super::AampDocument;

/// Serialize `doc` back to AAMP bytes verbatim (byte-identical for an
/// unmodified [`AampDocument`]).
pub fn write_aamp(doc: &AampDocument) -> Vec<u8> {
    doc.raw.clone()
}
