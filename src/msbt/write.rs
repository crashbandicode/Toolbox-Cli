//! MSBT writer.
//!
//! Stage A is the verbatim writer: [`write_msbt`] re-emits the bytes captured
//! at parse time, so an unmodified document round-trips **byte-identically** by
//! construction (the same discipline [`crate::byml::write_byml`] uses). A
//! from-scratch canonical writer that rebuilds LBL1/TXT2 from an edited tree is
//! a follow-up.

use super::error::Result;
use super::MsbtDocument;

/// Serialize an MSBT document.
///
/// Returns the original bytes verbatim — byte-identical to the input for an
/// unmodified document.
pub fn write_msbt(doc: &MsbtDocument) -> Result<Vec<u8>> {
    Ok(doc.raw.clone())
}
