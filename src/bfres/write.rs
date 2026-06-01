//! BFRES writer.
//!
//! BFRES, like [`crate::bntx`], is an offset-and-relocation-heavy container
//! whose exact byte layout is writer-specific. Since this module decodes only
//! the header + a structural scan (not the model/animation payloads), the
//! writer re-emits the bytes captured at parse time **verbatim** — so an
//! unmodified document round-trips byte-identically by construction. This is
//! the same discipline used by [`crate::byml`] / [`crate::msbt`] /
//! [`crate::aamp`] for their verbatim paths.

use super::BfresDocument;

/// Serialize a BFRES document. Because the parser is inspect-only, this returns
/// the original bytes verbatim — byte-identical for an unmodified document.
pub fn write_bfres(doc: &BfresDocument) -> Vec<u8> {
    doc.raw.clone()
}
