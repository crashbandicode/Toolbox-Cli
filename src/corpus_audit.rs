//! `corpus-audit`: measure real-world parser/round-trip confidence per format
//! *without committing game assets*.
//!
//! It walks files (and recurses into SARC archives, inflating compressed
//! entries), dispatches each by content magic, runs the **safest applicable
//! operation** for that format, and tallies the outcome. The result serializes
//! to a JSON manifest summarizing, per format: how many files were seen, how
//! many round-trip byte-identically, how many are semantically lossless, how
//! many only inspect-parse, how many are an expected-unsupported variant, and
//! how many fail unexpectedly (with the failing paths + typed error). This is
//! the breadth gate a verb needs to graduate from *Validated* to *Trusted* in
//! `TRUST_MATRIX.md` — run it locally over a romfs; nothing is written.

use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;

use crate::compression::{self, DictRegistry};

/// A format the audit can classify + exercise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Byml,
    Msbt,
    Sarc,
    Bntx,
    Restbl,
    Aamp,
    Bfres,
    Bflyt,
    Bflan,
}

impl Format {
    /// The lowercase key used on the CLI and in the JSON (`"byml"`, …).
    pub fn key(self) -> &'static str {
        match self {
            Format::Byml => "byml",
            Format::Msbt => "msbt",
            Format::Sarc => "sarc",
            Format::Bntx => "bntx",
            Format::Restbl => "restbl",
            Format::Aamp => "aamp",
            Format::Bfres => "bfres",
            Format::Bflyt => "bflyt",
            Format::Bflan => "bflan",
        }
    }

    /// Parse a CLI key (case-insensitive) into a [`Format`].
    pub fn from_key(s: &str) -> Option<Format> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "byml" | "byaml" | "bgyml" => Format::Byml,
            "msbt" => Format::Msbt,
            "sarc" | "pack" | "arc" => Format::Sarc,
            "bntx" => Format::Bntx,
            "restbl" | "rsizetable" => Format::Restbl,
            "aamp" => Format::Aamp,
            "bfres" => Format::Bfres,
            "bflyt" => Format::Bflyt,
            "bflan" => Format::Bflan,
            _ => return None,
        })
    }

    /// All formats the audit understands.
    pub fn all() -> [Format; 9] {
        [
            Format::Byml,
            Format::Msbt,
            Format::Sarc,
            Format::Bntx,
            Format::Restbl,
            Format::Aamp,
            Format::Bfres,
            Format::Bflyt,
            Format::Bflan,
        ]
    }

    /// The typed error name reported for this format's failures.
    fn error_name(self) -> &'static str {
        match self {
            Format::Byml => "BymlError",
            Format::Msbt => "MsbtError",
            Format::Sarc => "SarcError",
            Format::Bntx => "BntxError",
            Format::Restbl => "RestblError",
            Format::Aamp => "AampError",
            Format::Bfres => "BfresError",
            Format::Bflyt => "BflytError",
            Format::Bflan => "Error",
        }
    }
}

/// The result of auditing one file with one format's safest operation.
#[derive(Debug, Clone)]
pub enum Outcome {
    /// `write(read(x)) == x` (the byte-identical contract).
    ByteIdentical,
    /// Re-read of a canonical/from-scratch write matches the tree (not
    /// byte-identical by contract).
    Semantic,
    /// Parsed/inspected OK; no round-trip was attempted/expected.
    InspectOk,
    /// A recognized but deliberately-unsupported variant (e.g. MeshCodec).
    ExpectedUnsupported(String),
    /// An unexpected failure (typed error from the parser/writer).
    Failed { error_type: String, message: String },
}

/// One failure (or expected-unsupported) record in the JSON.
#[derive(Debug, Clone, Serialize)]
pub struct FailureEntry {
    pub path: String,
    pub operation: String,
    pub error_type: String,
    pub message: String,
    /// `true` = a known/expected unsupported variant; `false` = unexpected.
    pub expected: bool,
}

/// Per-format tally.
#[derive(Debug, Default, Clone, Serialize)]
pub struct FormatStats {
    pub files_seen: u64,
    pub files_attempted: u64,
    pub roundtrip_byte_identical: u64,
    pub semantic_roundtrip_ok: u64,
    pub inspect_ok: u64,
    pub failed: u64,
    pub skipped: u64,
    pub expected_unsupported: u64,
    pub versions: BTreeSet<String>,
    pub endianness: BTreeSet<String>,
    pub encodings: BTreeSet<String>,
    pub failures: Vec<FailureEntry>,
}

/// Metadata collected from a parsed file (whatever applies to the format).
#[derive(Debug, Default, Clone)]
pub struct Meta {
    pub version: Option<String>,
    pub endianness: Option<String>,
    pub encoding: Option<String>,
}

impl FormatStats {
    fn record(
        &mut self,
        format: Format,
        path: &str,
        operation: &str,
        outcome: Outcome,
        meta: Meta,
    ) {
        self.files_seen += 1;
        if let Some(v) = meta.version {
            self.versions.insert(v);
        }
        if let Some(e) = meta.endianness {
            self.endianness.insert(e);
        }
        if let Some(e) = meta.encoding {
            self.encodings.insert(e);
        }
        match outcome {
            Outcome::ByteIdentical => {
                self.files_attempted += 1;
                self.roundtrip_byte_identical += 1;
            }
            Outcome::Semantic => {
                self.files_attempted += 1;
                self.semantic_roundtrip_ok += 1;
            }
            Outcome::InspectOk => {
                self.files_attempted += 1;
                self.inspect_ok += 1;
            }
            Outcome::ExpectedUnsupported(reason) => {
                self.expected_unsupported += 1;
                self.skipped += 1;
                self.failures.push(FailureEntry {
                    path: path.to_string(),
                    operation: operation.to_string(),
                    error_type: "unsupported".to_string(),
                    message: reason,
                    expected: true,
                });
            }
            Outcome::Failed {
                error_type,
                message,
            } => {
                self.files_attempted += 1;
                self.failed += 1;
                self.failures.push(FailureEntry {
                    path: path.to_string(),
                    operation: operation.to_string(),
                    error_type: if error_type.is_empty() {
                        format.error_name().to_string()
                    } else {
                        error_type
                    },
                    message,
                    expected: false,
                });
            }
        }
    }
}

/// Audit configuration.
#[derive(Debug, Clone)]
pub struct AuditConfig {
    /// Formats to record (recursion into SARC happens regardless).
    pub formats: Vec<Format>,
    /// Max SARC-recursion depth (a compressed pack → SARC → inner files is
    /// depth 1; nested archives go deeper).
    pub max_depth: usize,
}

impl Default for AuditConfig {
    fn default() -> Self {
        AuditConfig {
            formats: Format::all().to_vec(),
            max_depth: 4,
        }
    }
}

/// The full audit report (serializes to the JSON manifest).
#[derive(Debug, Clone, Serialize)]
pub struct AuditReport {
    pub tool_version: String,
    pub git_commit: String,
    pub game: String,
    pub input_root: String,
    pub started_at: String,
    pub finished_at: String,
    pub files_scanned: u64,
    pub decompress_failed: u64,
    pub unclassified: u64,
    pub formats: std::collections::BTreeMap<String, FormatStats>,
}

/// Mutable audit accumulator.
pub struct Auditor<'a> {
    cfg: &'a AuditConfig,
    dicts: &'a DictRegistry,
    stats: std::collections::BTreeMap<String, FormatStats>,
    pub files_scanned: u64,
    pub decompress_failed: u64,
    pub unclassified: u64,
}

impl<'a> Auditor<'a> {
    pub fn new(cfg: &'a AuditConfig, dicts: &'a DictRegistry) -> Self {
        let mut stats = std::collections::BTreeMap::new();
        for f in &cfg.formats {
            stats.insert(f.key().to_string(), FormatStats::default());
        }
        Auditor {
            cfg,
            dicts,
            stats,
            files_scanned: 0,
            decompress_failed: 0,
            unclassified: 0,
        }
    }

    fn wants(&self, f: Format) -> bool {
        self.cfg.formats.contains(&f)
    }

    /// Walk one path (file or directory) and audit every file under it.
    pub fn audit_path(&mut self, root: &Path) {
        if root.is_file() {
            self.audit_one_path(root, root);
        } else if root.is_dir() {
            for entry in walkdir::WalkDir::new(root)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if entry.file_type().is_file() {
                    self.audit_one_path(root, entry.path());
                }
            }
        }
    }

    fn rel(root: &Path, p: &Path) -> String {
        p.strip_prefix(root)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn audit_one_path(&mut self, root: &Path, p: &Path) {
        let Ok(bytes) = std::fs::read(p) else {
            return;
        };
        self.files_scanned += 1;
        let rel = Self::rel(root, p);
        self.audit_bytes(&rel, &bytes, 0);
    }

    /// Audit a single file's bytes: inflate, classify, run the format op, and
    /// (for SARC) recurse into entries.
    pub fn audit_bytes(&mut self, rel: &str, raw: &[u8], depth: usize) {
        // Inflate zstd/Yaz0; passthrough for already-decompressed data.
        let data = match compression::decompress(raw, self.dicts) {
            Ok(cow) => cow.into_owned(),
            Err(_) => {
                self.decompress_failed += 1;
                return;
            }
        };
        let Some(format) = classify_magic(&data) else {
            self.unclassified += 1;
            return;
        };

        if format == Format::Sarc {
            if self.wants(Format::Sarc) {
                let (outcome, meta) = audit_sarc(&data);
                self.stats.get_mut("sarc").unwrap().record(
                    Format::Sarc,
                    rel,
                    "read_arc/write_arc",
                    outcome,
                    meta,
                );
            }
            // Recurse into entries regardless of whether sarc is recorded, so
            // inner byml/msbt/bntx/... get audited.
            if depth < self.cfg.max_depth {
                if let Ok(arc) = crate::sarc::read_arc(&data) {
                    for e in &arc.files {
                        let name = e.name.clone().unwrap_or_else(|| "<hash-only>".to_string());
                        let inner_rel = format!("{rel}!/{name}");
                        self.audit_bytes(&inner_rel, &e.data, depth + 1);
                    }
                }
            }
            return;
        }

        if !self.wants(format) {
            return;
        }
        let (operation, outcome, meta) = audit_format(format, &data);
        self.stats
            .get_mut(format.key())
            .unwrap()
            .record(format, rel, operation, outcome, meta);
    }

    /// Consume the accumulator into the per-format map.
    pub fn into_stats(self) -> std::collections::BTreeMap<String, FormatStats> {
        self.stats
    }
}

/// Classify a (decompressed) buffer by its content magic.
pub fn classify_magic(data: &[u8]) -> Option<Format> {
    if data.len() < 8 {
        return None;
    }
    let m4 = &data[0..4];
    if m4 == b"FRES" || m4 == b"MCPK" {
        return Some(Format::Bfres);
    }
    if &data[0..8] == b"MsgStdBn" {
        return Some(Format::Msbt);
    }
    if m4 == b"AAMP" {
        return Some(Format::Aamp);
    }
    if &data[0..6] == b"RESTBL" {
        return Some(Format::Restbl);
    }
    if m4 == b"SARC" {
        return Some(Format::Sarc);
    }
    if m4 == b"FLYT" {
        return Some(Format::Bflyt);
    }
    if m4 == b"FLAN" {
        return Some(Format::Bflan);
    }
    if m4 == b"BNTX" {
        return Some(Format::Bntx);
    }
    // BYML: "YB" (LE) / "BY" (BE) + a plausible version (1..=7).
    if &data[0..2] == b"YB" || &data[0..2] == b"BY" {
        let be = &data[0..2] == b"BY";
        let ver = if be {
            u16::from_be_bytes([data[2], data[3]])
        } else {
            u16::from_le_bytes([data[2], data[3]])
        };
        if (1..=7).contains(&ver) {
            return Some(Format::Byml);
        }
    }
    None
}

fn failed<E: std::fmt::Display>(e: E) -> Outcome {
    Outcome::Failed {
        error_type: String::new(),
        message: e.to_string(),
    }
}

/// Run the safest applicable operation for `format` on `data`, returning the
/// operation label, the outcome, and any collected metadata.
fn audit_format(format: Format, data: &[u8]) -> (&'static str, Outcome, Meta) {
    match format {
        Format::Byml => audit_byml(data),
        Format::Msbt => audit_msbt(data),
        Format::Bntx => audit_bntx(data),
        Format::Restbl => ("read/write", audit_restbl(data), Meta::default()),
        Format::Aamp => audit_aamp(data),
        Format::Bfres => audit_bfres(data),
        Format::Bflyt => audit_bflyt(data),
        Format::Bflan => ("read/write", audit_bflan(data), Meta::default()),
        Format::Sarc => ("read_arc", Outcome::InspectOk, Meta::default()),
    }
}

fn audit_byml(data: &[u8]) -> (&'static str, Outcome, Meta) {
    match crate::byml::read_byml(data) {
        Ok(doc) => {
            let meta = Meta {
                version: Some(format!("v{}", doc.version)),
                endianness: Some(endian(doc.big_endian)),
                ..Default::default()
            };
            let out = match crate::byml::write_byml(&doc) {
                Ok(b) if b == data => Outcome::ByteIdentical,
                _ => {
                    match crate::byml::write_byml_canonical(doc.version, doc.big_endian, &doc.root)
                    {
                        Ok(c) => match crate::byml::read_byml(&c) {
                            Ok(d2) if d2.root == doc.root => Outcome::Semantic,
                            _ => Outcome::InspectOk,
                        },
                        Err(_) => Outcome::InspectOk,
                    }
                }
            };
            ("read/write", out, meta)
        }
        Err(e) => ("read", failed(e), Meta::default()),
    }
}

fn audit_msbt(data: &[u8]) -> (&'static str, Outcome, Meta) {
    match crate::msbt::read_msbt(data) {
        Ok(doc) => {
            let meta = Meta {
                version: Some(format!("v{}", doc.version)),
                endianness: Some(endian(doc.big_endian)),
                encoding: Some(format!("{:?}", doc.encoding)),
            };
            let out = match crate::msbt::write_msbt(&doc) {
                Ok(b) if b == data => Outcome::ByteIdentical,
                _ => match crate::msbt::write_msbt_canonical(&doc) {
                    Ok(c) => match crate::msbt::read_msbt(&c) {
                        Ok(d2) if d2.messages() == doc.messages() => Outcome::Semantic,
                        _ => Outcome::InspectOk,
                    },
                    Err(_) => Outcome::InspectOk,
                },
            };
            ("read/write", out, meta)
        }
        Err(e) => ("read", failed(e), Meta::default()),
    }
}

fn audit_bntx(data: &[u8]) -> (&'static str, Outcome, Meta) {
    match crate::bntx::read_bntx(data) {
        Ok(f) => {
            let meta = Meta {
                version: Some(format!("0x{:08x}", f.header.version)),
                endianness: Some("little".to_string()),
                ..Default::default()
            };
            let out = match crate::bntx::write_bntx(&f) {
                Ok(b) if b == data => Outcome::ByteIdentical,
                Ok(_) => Outcome::InspectOk, // parsed; known non-byte-identical tools
                Err(e) => failed(e),
            };
            ("read/write", out, meta)
        }
        Err(e) => ("read", failed(e), Meta::default()),
    }
}

fn audit_restbl(data: &[u8]) -> Outcome {
    match crate::restbl::read_restbl(data) {
        Ok(r) => match crate::restbl::write_restbl(&r) {
            Ok(b) if b == data => Outcome::ByteIdentical,
            Ok(_) => Outcome::InspectOk,
            Err(e) => failed(e),
        },
        Err(e) => failed(e),
    }
}

fn audit_aamp(data: &[u8]) -> (&'static str, Outcome, Meta) {
    match crate::aamp::read_aamp(data) {
        Ok(doc) => {
            let meta = Meta {
                version: Some(format!("pio{}", doc.pio_version)),
                endianness: Some(endian(doc.big_endian)),
                ..Default::default()
            };
            let out = if crate::aamp::write_aamp(&doc) == data {
                Outcome::ByteIdentical
            } else {
                Outcome::InspectOk
            };
            ("read/write", out, meta)
        }
        Err(e) => ("read", failed(e), Meta::default()),
    }
}

fn audit_bfres(data: &[u8]) -> (&'static str, Outcome, Meta) {
    if data.get(0..4) == Some(b"MCPK".as_slice()) {
        return (
            "decompress",
            Outcome::ExpectedUnsupported("MeshCodec (.mc) decompression not implemented".into()),
            Meta::default(),
        );
    }
    match crate::bfres::read_bfres(data) {
        Ok(doc) => {
            let meta = Meta {
                version: Some(doc.version_label()),
                endianness: Some(endian(doc.big_endian)),
                ..Default::default()
            };
            let out = if crate::bfres::write_bfres(&doc) == data {
                Outcome::ByteIdentical
            } else {
                Outcome::InspectOk
            };
            ("read/write", out, meta)
        }
        Err(e) => ("read", failed(e), Meta::default()),
    }
}

fn audit_bflyt(data: &[u8]) -> (&'static str, Outcome, Meta) {
    match crate::bflyt::read_bflyt(data) {
        Ok(doc) => {
            let meta = Meta {
                version: Some(format!("0x{:08x}", doc.version)),
                endianness: Some("little".to_string()),
                ..Default::default()
            };
            let out = match crate::bflyt::write_bflyt(&doc) {
                Ok(b) if b == data => Outcome::ByteIdentical,
                Ok(_) => Outcome::InspectOk,
                Err(e) => failed(e),
            };
            ("read/write", out, meta)
        }
        Err(e) => ("read", failed(e), Meta::default()),
    }
}

fn audit_bflan(data: &[u8]) -> Outcome {
    match crate::bflan::read_bflan(data) {
        Ok(doc) => match crate::bflan::write_bflan(&doc) {
            Ok(b) if b == data => Outcome::ByteIdentical,
            Ok(_) => Outcome::InspectOk,
            Err(e) => failed(e),
        },
        Err(e) => failed(e),
    }
}

fn audit_sarc(data: &[u8]) -> (Outcome, Meta) {
    let meta = Meta {
        endianness: crate::sarc::read_arc(data)
            .ok()
            .map(|a| endian(a.big_endian)),
        ..Default::default()
    };
    match crate::sarc::read_arc(data) {
        Ok(arc) => match crate::sarc::write_arc(&arc) {
            Ok(b) if b == data => (Outcome::ByteIdentical, meta),
            Ok(_) => (Outcome::InspectOk, meta), // re-pack is canonical, not always byte-identical
            Err(e) => (failed(e), meta),
        },
        Err(e) => (failed(e), meta),
    }
}

fn endian(big: bool) -> String {
    if big { "big" } else { "little" }.to_string()
}

/// Format a unix-epoch-seconds timestamp as an ISO-8601 UTC string
/// (`YYYY-MM-DDTHH:MM:SSZ`), dependency-free.
pub fn iso8601_utc(epoch_secs: u64) -> String {
    let days = (epoch_secs / 86_400) as i64;
    let secs_of_day = epoch_secs % 86_400;
    let (h, mi, s) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_magics() {
        let mut fres = vec![0u8; 32];
        fres[0..8].copy_from_slice(b"FRES    ");
        assert_eq!(classify_magic(&fres), Some(Format::Bfres));

        let mut mc = vec![0u8; 32];
        mc[0..4].copy_from_slice(b"MCPK");
        assert_eq!(classify_magic(&mc), Some(Format::Bfres));

        let mut msbt = vec![0u8; 32];
        msbt[0..8].copy_from_slice(b"MsgStdBn");
        assert_eq!(classify_magic(&msbt), Some(Format::Msbt));

        let mut restbl = vec![0u8; 32];
        restbl[0..6].copy_from_slice(b"RESTBL");
        assert_eq!(classify_magic(&restbl), Some(Format::Restbl));

        // BYML needs a sane version, so random "YB" data isn't misclassified.
        let mut byml = vec![0u8; 32];
        byml[0..2].copy_from_slice(b"YB");
        byml[2..4].copy_from_slice(&7u16.to_le_bytes());
        assert_eq!(classify_magic(&byml), Some(Format::Byml));
        let mut not_byml = vec![0u8; 32];
        not_byml[0..2].copy_from_slice(b"YB");
        not_byml[2..4].copy_from_slice(&9999u16.to_le_bytes());
        assert_eq!(classify_magic(&not_byml), None);

        assert_eq!(classify_magic(&[0u8; 4]), None);
        assert_eq!(classify_magic(b"NOTAMAGIC_AT_ALL"), None);
    }

    #[test]
    fn format_keys_round_trip() {
        for f in Format::all() {
            assert_eq!(Format::from_key(f.key()), Some(f));
        }
        assert_eq!(Format::from_key("nope"), None);
    }

    #[test]
    fn stats_record_counts_each_outcome() {
        let mut s = FormatStats::default();
        let m = || Meta {
            version: Some("v7".into()),
            endianness: Some("little".into()),
            ..Default::default()
        };
        s.record(Format::Byml, "a", "op", Outcome::ByteIdentical, m());
        s.record(Format::Byml, "b", "op", Outcome::Semantic, m());
        s.record(Format::Byml, "c", "op", Outcome::InspectOk, Meta::default());
        s.record(
            Format::Byml,
            "d",
            "op",
            Outcome::ExpectedUnsupported("nope".into()),
            Meta::default(),
        );
        s.record(
            Format::Byml,
            "e",
            "op",
            Outcome::Failed {
                error_type: String::new(),
                message: "boom".into(),
            },
            Meta::default(),
        );
        assert_eq!(s.files_seen, 5);
        assert_eq!(s.files_attempted, 4); // expected-unsupported isn't "attempted"
        assert_eq!(s.roundtrip_byte_identical, 1);
        assert_eq!(s.semantic_roundtrip_ok, 1);
        assert_eq!(s.inspect_ok, 1);
        assert_eq!(s.expected_unsupported, 1);
        assert_eq!(s.failed, 1);
        assert_eq!(s.failures.len(), 2); // the unsupported + the failure
                                         // The unexpected failure inherits the format's typed-error name.
        let unexpected = s.failures.iter().find(|f| !f.expected).unwrap();
        assert_eq!(unexpected.error_type, "BymlError");
        assert!(s.versions.contains("v7"));
    }

    #[test]
    fn audits_inflated_aamp_bytes_as_byte_identical() {
        // A minimal AAMP built the same way the aamp::read tests do.
        let mut b = vec![0u8; crate::aamp::HEADER_LEN];
        b[0..4].copy_from_slice(crate::aamp::AAMP_MAGIC);
        b[4..8].copy_from_slice(&2u32.to_le_bytes());
        b[8..12].copy_from_slice(&3u32.to_le_bytes());
        b[0x14..0x18].copy_from_slice(&4u32.to_le_bytes());
        b.extend_from_slice(b"xml\0");
        b.extend_from_slice(&0xAABB_CCDDu32.to_le_bytes()); // root name
        b.extend_from_slice(&0u32.to_le_bytes()); // no lists
        b.extend_from_slice(&0u32.to_le_bytes()); // no objects

        assert_eq!(classify_magic(&b), Some(Format::Aamp));
        let (_op, outcome, meta) = audit_format(Format::Aamp, &b);
        assert!(matches!(outcome, Outcome::ByteIdentical), "got {outcome:?}");
        assert_eq!(meta.endianness.as_deref(), Some("little"));
    }

    #[test]
    fn auditor_walks_in_memory_bytes() {
        let cfg = AuditConfig::default();
        let dicts = DictRegistry::new();
        let mut a = Auditor::new(&cfg, &dicts);
        // Garbage with no recognizable magic -> unclassified, no per-format hit.
        a.audit_bytes("junk.bin", b"not a known file at all....", 0);
        assert_eq!(a.unclassified, 1);
        let stats = a.into_stats();
        assert_eq!(stats["byml"].files_seen, 0);
    }

    #[test]
    fn iso8601_is_sane() {
        assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00Z");
        // 2021-01-01T00:00:00Z = 1609459200
        assert_eq!(iso8601_utc(1_609_459_200), "2021-01-01T00:00:00Z");
    }
}
