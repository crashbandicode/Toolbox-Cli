//! zstd dictionary registry for TotK `.zs` assets.
//!
//! TotK ships three dictionaries inside `Pack/ZsDic.pack.zs` (a plain zstd
//! frame wrapping a SARC of `*.zsdic` files): `zs.zsdic` (id 1), `bcett`
//! (id 2), `pack.zsdic` (id 3). Each compressed `.zs` frame names the
//! dictionary id it needs in its header, so we key dictionaries by their
//! embedded id and look them up at decode time.

use std::collections::HashMap;
use std::path::Path;

use crate::error::{Error, Result};

/// zstd dictionary magic, little-endian `0xEC30A437`.
const DICT_MAGIC: [u8; 4] = [0x37, 0xA4, 0x30, 0xEC];

/// Read the `Dictionary_ID` embedded in a *formatted* zstd dictionary
/// (the `0xEC30A437` magic followed by a 4-byte little-endian id). Returns
/// an error for raw-content dictionaries (which carry no id) or non-dicts.
pub fn dict_id(raw: &[u8]) -> Result<u32> {
    if raw.len() < 8 || raw[0..4] != DICT_MAGIC {
        return Err(Error::Compression(
            "not a formatted zstd dictionary (missing 0xEC30A437 magic)".into(),
        ));
    }
    Ok(u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]))
}

/// A set of zstd dictionaries keyed by their embedded `Dictionary_ID`, used
/// to decompress (and re-compress) TotK `.zs` assets.
#[derive(Debug, Clone, Default)]
pub struct DictRegistry {
    by_id: HashMap<u32, Vec<u8>>,
}

impl DictRegistry {
    /// An empty registry (sufficient for plain, dictionary-less frames).
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// The dictionary bytes registered under `id`, if any.
    pub fn get(&self, id: u32) -> Option<&[u8]> {
        self.by_id.get(&id).map(Vec::as_slice)
    }

    /// All registered dictionary ids, sorted (for diagnostics).
    pub fn ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.by_id.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// Register a formatted dictionary under its embedded id. Returns the id.
    pub fn add_dict(&mut self, raw: &[u8]) -> Result<u32> {
        let id = dict_id(raw)?;
        self.by_id.insert(id, raw.to_vec());
        Ok(id)
    }

    /// Build a registry from TotK's `ZsDic.pack.zs` (or an already-extracted
    /// `ZsDic.pack`). The pack is a plain (dictionary-less) zstd frame
    /// wrapping a SARC of `*.zsdic` entries; we decompress, unpack, and
    /// register every dictionary by its embedded id.
    pub fn from_zsdic_pack(packed: &[u8]) -> Result<Self> {
        let raw: std::borrow::Cow<[u8]> = if super::zstd::is_zstd(packed) {
            std::borrow::Cow::Owned(super::zstd::decompress(packed, None)?)
        } else {
            std::borrow::Cow::Borrowed(packed)
        };
        let entries = crate::sarc::unpack(&raw)
            .map_err(|e| Error::Compression(format!("ZsDic pack is not a SARC: {e}")))?;
        let mut reg = Self::new();
        for f in &entries {
            if f.name.to_ascii_lowercase().ends_with(".zsdic") {
                reg.add_dict(&f.data)?;
            }
        }
        if reg.is_empty() {
            return Err(Error::Compression(format!(
                "ZsDic pack contained no .zsdic entries (unpacked {} files)",
                entries.len()
            )));
        }
        Ok(reg)
    }

    /// Load every `*.zsdic` file directly from a directory.
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let mut reg = Self::new();
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let is_zsdic = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("zsdic"))
                .unwrap_or(false);
            if is_zsdic {
                reg.add_dict(&std::fs::read(&path)?)?;
            }
        }
        if reg.is_empty() {
            return Err(Error::Compression(format!(
                "no .zsdic files found in {}",
                dir.display()
            )));
        }
        Ok(reg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal byte buffer that looks like a formatted dictionary with the
    /// given id (magic + id + filler). Enough to exercise id parsing and the
    /// registry; real decode is covered by the fixture-gated TotK tests.
    fn fake_dict(id: u32) -> Vec<u8> {
        let mut v = DICT_MAGIC.to_vec();
        v.extend_from_slice(&id.to_le_bytes());
        v.extend_from_slice(&[0xCDu8; 32]);
        v
    }

    #[test]
    fn reads_embedded_id() {
        assert_eq!(dict_id(&fake_dict(1)).unwrap(), 1);
        assert_eq!(dict_id(&fake_dict(3)).unwrap(), 3);
        assert_eq!(dict_id(&fake_dict(0x1234_5678)).unwrap(), 0x1234_5678);
    }

    #[test]
    fn rejects_non_dictionary() {
        assert!(dict_id(b"SARC....").is_err());
        assert!(dict_id(&[]).is_err());
        // Raw-content dictionaries (no magic) have no id.
        assert!(dict_id(&[0u8; 64]).is_err());
    }

    #[test]
    fn registry_keys_by_id() {
        let mut reg = DictRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.add_dict(&fake_dict(1)).unwrap(), 1);
        assert_eq!(reg.add_dict(&fake_dict(3)).unwrap(), 3);
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.ids(), vec![1, 3]);
        assert!(reg.get(1).is_some());
        assert!(reg.get(2).is_none());
        assert_eq!(&reg.get(3).unwrap()[0..4], &DICT_MAGIC);
    }
}
