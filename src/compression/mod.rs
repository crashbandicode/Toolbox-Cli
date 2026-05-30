//! Compression for Switch modding assets.
//!
//! Two codecs cover the real game data:
//! - **zstd** (with TotK dictionaries) — `.zs`, `.pack.zs`, `.blarc.zs`, …
//!   Backed by the vendored libzstd (`zstd` crate; BSD-3, GPL-free) plus a
//!   pure-Rust frame-header parser ([`zstd::frame_dictionary_id`]) so we can
//!   pick the right dictionary without decompressing first.
//! - **Yaz0/Yaz1** (`.szs`) — implemented natively in [`yaz0`].
//!
//! [`detect`] sniffs the codec from the leading magic; [`decompress`]
//! transparently inflates a buffer (selecting the zstd dictionary by the
//! frame's id from a [`DictRegistry`]) and returns uncompressed input
//! untouched. The byte-for-byte round-trip discipline the rest of the crate
//! holds does **not** apply to the compressed container (re-compression
//! can't reproduce the original encoder's bytes); the invariant here is
//! `decompress(compress(x)) == x`, and unmodified files are passed through
//! verbatim at the archive layer.

pub mod dict;
pub mod yaz0;
pub mod zstd;

use std::borrow::Cow;

use crate::error::{Error, Result};

pub use dict::{dict_id, DictRegistry};

/// Compression codec identified from a buffer's leading magic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// zstd frame (`28 B5 2F FD`).
    Zstd,
    /// Yaz0 container (`.szs`).
    Yaz0,
    /// Yaz1 container (rare variant of Yaz0).
    Yaz1,
    /// No recognized compression magic.
    None,
}

impl Codec {
    /// True for everything but [`Codec::None`].
    pub fn is_compressed(self) -> bool {
        !matches!(self, Codec::None)
    }

    /// A short label for diagnostics / CLI output.
    pub fn label(self) -> &'static str {
        match self {
            Codec::Zstd => "zstd",
            Codec::Yaz0 => "Yaz0",
            Codec::Yaz1 => "Yaz1",
            Codec::None => "none",
        }
    }
}

/// Detect the compression codec from `bytes`' leading magic.
pub fn detect(bytes: &[u8]) -> Codec {
    if zstd::is_zstd(bytes) {
        return Codec::Zstd;
    }
    if bytes.len() >= 4 {
        match &bytes[0..4] {
            b"Yaz0" => return Codec::Yaz0,
            b"Yaz1" => return Codec::Yaz1,
            _ => {}
        }
    }
    Codec::None
}

/// Decompress `bytes` if it carries a known compression magic, selecting the
/// right zstd dictionary by the frame's `Dictionary_ID`. Uncompressed input
/// is returned borrowed (no copy), so this is cheap to call speculatively on
/// every file while walking an archive tree.
pub fn decompress<'a>(bytes: &'a [u8], dicts: &DictRegistry) -> Result<Cow<'a, [u8]>> {
    match detect(bytes) {
        Codec::None => Ok(Cow::Borrowed(bytes)),
        Codec::Yaz0 | Codec::Yaz1 => Ok(Cow::Owned(yaz0::decompress(bytes)?)),
        Codec::Zstd => {
            let id = zstd::frame_dictionary_id(bytes)?;
            let dict = if id == 0 {
                None
            } else {
                Some(dicts.get(id).ok_or_else(|| {
                    Error::Compression(format!(
                        "zstd frame needs dictionary id {id}, which isn't loaded (have {:?}). \
                         Supply TotK's ZsDic.pack.zs (e.g. --dict).",
                        dicts.ids()
                    ))
                })?)
            };
            Ok(Cow::Owned(zstd::decompress(bytes, dict)?))
        }
    }
}

/// Compress `payload` as a single zstd frame at `level`, optionally with a
/// registered dictionary id (the frame embeds that id so the game and our
/// own [`decompress`] can reselect it). `Some(0)`/`None` means no dictionary.
pub fn compress_zstd(
    payload: &[u8],
    dicts: &DictRegistry,
    dictionary_id: Option<u32>,
    level: i32,
) -> Result<Vec<u8>> {
    let dict = match dictionary_id {
        None | Some(0) => None,
        Some(id) => Some(dicts.get(id).ok_or_else(|| {
            Error::Compression(format!(
                "dictionary id {id} not loaded (have {:?})",
                dicts.ids()
            ))
        })?),
    };
    zstd::compress(payload, dict, level)
}

/// Compress `payload` into a Yaz0 stream (lossless; ratio is secondary).
pub fn compress_yaz0(payload: &[u8]) -> Vec<u8> {
    yaz0::compress(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_codecs() {
        assert_eq!(detect(&[0x28, 0xB5, 0x2F, 0xFD, 0x00]), Codec::Zstd);
        assert_eq!(detect(b"Yaz0\0\0\0\0"), Codec::Yaz0);
        assert_eq!(detect(b"Yaz1\0\0\0\0"), Codec::Yaz1);
        assert_eq!(detect(b"SARC...."), Codec::None);
        assert_eq!(detect(b""), Codec::None);
        assert!(Codec::Zstd.is_compressed());
        assert!(!Codec::None.is_compressed());
    }

    #[test]
    fn decompress_passes_through_uncompressed() {
        let dicts = DictRegistry::new();
        let plain = b"SARC and friends, not compressed";
        let out = decompress(plain, &dicts).unwrap();
        assert!(matches!(out, Cow::Borrowed(_)), "uncompressed should not copy");
        assert_eq!(&*out, plain);
    }

    #[test]
    fn zstd_plain_round_trip_via_high_level() {
        let dicts = DictRegistry::new();
        let data = b"high-level compress/decompress round trip".repeat(20);
        let packed = compress_zstd(&data, &dicts, None, 3).unwrap();
        assert_eq!(detect(&packed), Codec::Zstd);
        assert_eq!(&*decompress(&packed, &dicts).unwrap(), &data[..]);
    }

    #[test]
    fn yaz0_round_trip_via_high_level() {
        let dicts = DictRegistry::new();
        let data = b"Yaz0 high-level round trip ".repeat(30);
        let packed = compress_yaz0(&data);
        assert_eq!(detect(&packed), Codec::Yaz0);
        assert_eq!(&*decompress(&packed, &dicts).unwrap(), &data[..]);
    }

    #[test]
    fn zstd_needs_dictionary_error_is_clear() {
        // Build a frame that claims dict id 1 but supply an empty registry.
        let dicts_with = {
            let mut r = DictRegistry::new();
            // raw-content dict won't embed id 1, so craft via compress with a
            // raw dict and then assert the *missing-dict* path instead.
            r.add_dict(&{
                let mut d = vec![0x37, 0xA4, 0x30, 0xEC];
                d.extend_from_slice(&1u32.to_le_bytes());
                d.extend_from_slice(&[0u8; 16]);
                d
            })
            .unwrap();
            r
        };
        // A hand-built minimal zstd header advertising dict id 1 (descriptor
        // 0x61 = single-segment + 1-byte dict id) — decode must complain
        // about the missing dictionary when the registry lacks id 1.
        let frame = [0x28u8, 0xB5, 0x2F, 0xFD, 0x61, 0x01, 0x00];
        let empty = DictRegistry::new();
        let err = decompress(&frame, &empty).unwrap_err().to_string();
        assert!(err.contains("dictionary id 1"), "got: {err}");
        // With id 1 present we get past dictionary selection (the body is
        // bogus, so it still errors, but not on the missing-dict path).
        let err2 = decompress(&frame, &dicts_with).unwrap_err().to_string();
        assert!(!err2.contains("isn't loaded"), "got: {err2}");
    }
}
