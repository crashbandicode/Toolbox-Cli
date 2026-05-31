//! BYML serialization.
//!
//! Stage A is **verbatim**: [`write_byml`] returns the original bytes captured
//! at read time, so an unmodified document round-trips byte-identically — the
//! same discipline the [`crate::compression`] layer uses for unchanged files.
//! BYML's exact byte layout is writer-specific (node de-duplication, node
//! ordering, padding), so reproducing a third-party tool's bytes from a decoded
//! tree is deferred to the from-scratch canonical writer in a follow-up.

use super::error::Result;
use super::BymlDocument;

/// Serialize `doc` back to BYML bytes.
///
/// Returns the original bytes captured by [`read_byml`](super::read_byml), so
/// an unmodified document is byte-identical. (Mutated/synthesized trees will
/// use the canonical writer added in a follow-up.)
pub fn write_byml(doc: &BymlDocument) -> Result<Vec<u8>> {
    Ok(doc.raw.clone())
}
