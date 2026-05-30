//! Recursive layout audit: scan a directory (or archive) for BFLYT/BNTX
//! files and report unsupported or suspicious structures.
//!
//! "Suspicious" means structures we round-trip but can't fully vouch for:
//! BFLYT materials whose flag bits disagreed with their byte budget
//! (`flags_untrusted`, the malformed-mat1 recovery), materials carrying
//! undocumented v9 extension bytes (`trailing`), and v9 layouts.
//! "Unsupported" means a parser rejected the file outright (unknown
//! section, unsupported version, unknown BNTX surface format). The result
//! serializes to JSON.

use std::path::Path;

use serde::Serialize;
use walkdir::WalkDir;

use crate::bflan::read_bflan;
use crate::bflyt::read_bflyt;
use crate::bntx::read_bntx;
use crate::error::{Error, Result};

/// Aggregate counts across an audited tree.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AuditTotals {
    pub bflyt_scanned: usize,
    pub bflyt_failed: usize,
    pub bflyt_v9: usize,
    pub bflyt_with_untrusted_mat: usize,
    pub bflyt_with_v9_mat_extension: usize,
    pub untrusted_materials: usize,
    pub v9_extension_materials: usize,
    pub bntx_scanned: usize,
    pub bntx_failed: usize,
    pub bntx_unsupported_format: usize,
    pub bflan_scanned: usize,
    pub bflan_failed: usize,
    pub bflan_truncated_section: usize,
    pub arc_scanned: usize,
    pub arc_failed: usize,
    pub other_files: usize,
}

/// Per-file audit, recorded only when there is something to report (a
/// finding or a parse failure).
#[derive(Debug, Clone, Serialize)]
pub struct FileAudit {
    pub path: String,
    pub kind: String,
    pub ok: bool,
    pub findings: Vec<String>,
    pub error: Option<String>,
}

/// Full audit report.
#[derive(Debug, Clone, Serialize)]
pub struct AuditReport {
    pub root: String,
    pub totals: AuditTotals,
    /// Files with at least one finding or a parse failure.
    pub files: Vec<FileAudit>,
}

/// Audit a path: a single file, or a directory walked recursively. SARC
/// archives encountered are unpacked and their entries audited too.
pub fn audit_path(root: &Path) -> Result<AuditReport> {
    let mut totals = AuditTotals::default();
    let mut files = Vec::new();

    if root.is_file() {
        let bytes = std::fs::read(root)?;
        audit_entry(&root.display().to_string(), &bytes, &mut totals, &mut files);
    } else if root.is_dir() {
        for entry in WalkDir::new(root).sort_by_file_name() {
            let entry =
                entry.map_err(|e| Error::Other(format!("walking {}: {e}", root.display())))?;
            if !entry.file_type().is_file() {
                continue;
            }
            // Decide by extension before reading: skip the (typically
            // thousands of) non-layout files without slurping their bytes.
            if !is_auditable(entry.path()) {
                totals.other_files += 1;
                continue;
            }
            let bytes = std::fs::read(entry.path())?;
            let rel = entry
                .path()
                .strip_prefix(root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");
            audit_entry(&rel, &bytes, &mut totals, &mut files);
        }
    } else {
        return Err(Error::Other(format!("path not found: {}", root.display())));
    }

    Ok(AuditReport {
        root: root.display().to_string(),
        totals,
        files,
    })
}

/// True if a path's extension is one the auditor parses (so the walker
/// can skip reading everything else).
fn is_auditable(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => {
            let ext = ext.to_ascii_lowercase();
            matches!(ext.as_str(), "bflyt" | "bntx" | "bflan" | "arc" | "szs")
        }
        None => false,
    }
}

/// Dispatch one in-memory file by extension.
fn audit_entry(path: &str, bytes: &[u8], totals: &mut AuditTotals, files: &mut Vec<FileAudit>) {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".bflyt") {
        audit_bflyt(path, bytes, totals, files);
    } else if lower.ends_with(".bntx") {
        audit_bntx(path, bytes, totals, files);
    } else if lower.ends_with(".bflan") {
        audit_bflan(path, bytes, totals, files);
    } else if lower.ends_with(".arc") || lower.ends_with(".szs") {
        audit_arc(path, bytes, totals, files);
    } else {
        totals.other_files += 1;
    }
}

fn audit_bflyt(path: &str, bytes: &[u8], totals: &mut AuditTotals, files: &mut Vec<FileAudit>) {
    totals.bflyt_scanned += 1;
    match read_bflyt(bytes) {
        Ok(b) => {
            let mut findings = Vec::new();
            let major = (b.version >> 24) & 0xff;
            if major >= 9 {
                totals.bflyt_v9 += 1;
                findings.push(format!("BFLYT version {major}.x (v9 material extension is opaque)"));
            }
            let untrusted = b.materials.iter().filter(|m| m.flags_untrusted).count();
            if untrusted > 0 {
                totals.bflyt_with_untrusted_mat += 1;
                totals.untrusted_materials += untrusted;
                findings.push(format!(
                    "{untrusted} material(s) with untrusted flags (malformed mat1, recovered)"
                ));
            }
            let v9_ext = b.materials.iter().filter(|m| !m.trailing.is_empty()).count();
            if v9_ext > 0 {
                totals.bflyt_with_v9_mat_extension += 1;
                totals.v9_extension_materials += v9_ext;
                findings.push(format!(
                    "{v9_ext} material(s) carry undocumented extension bytes (preserved verbatim)"
                ));
            }
            if !findings.is_empty() {
                files.push(FileAudit {
                    path: path.to_string(),
                    kind: "bflyt".into(),
                    ok: true,
                    findings,
                    error: None,
                });
            }
        }
        Err(e) => {
            totals.bflyt_failed += 1;
            files.push(FileAudit {
                path: path.to_string(),
                kind: "bflyt".into(),
                ok: false,
                findings: Vec::new(),
                error: Some(e.to_string()),
            });
        }
    }
}

fn audit_bntx(path: &str, bytes: &[u8], totals: &mut AuditTotals, files: &mut Vec<FileAudit>) {
    totals.bntx_scanned += 1;
    match read_bntx(bytes) {
        Ok(_) => {}
        Err(e) => {
            totals.bntx_failed += 1;
            let msg = e.to_string();
            if msg.contains("surface format") {
                totals.bntx_unsupported_format += 1;
            }
            files.push(FileAudit {
                path: path.to_string(),
                kind: "bntx".into(),
                ok: false,
                findings: Vec::new(),
                error: Some(msg),
            });
        }
    }
}

fn audit_bflan(path: &str, bytes: &[u8], totals: &mut AuditTotals, files: &mut Vec<FileAudit>) {
    totals.bflan_scanned += 1;
    match read_bflan(bytes) {
        Ok(b) => {
            // A final section whose declared size exceeds the bytes
            // actually present (an HDR animation quirk we round-trip
            // verbatim) is worth surfacing.
            let truncated = b
                .sections
                .iter()
                .any(|s| s.declared_size as usize > s.payload.len() + 8);
            if truncated {
                totals.bflan_truncated_section += 1;
                files.push(FileAudit {
                    path: path.to_string(),
                    kind: "bflan".into(),
                    ok: true,
                    findings: vec!["final section truncated below its declared size".into()],
                    error: None,
                });
            }
        }
        Err(e) => {
            totals.bflan_failed += 1;
            files.push(FileAudit {
                path: path.to_string(),
                kind: "bflan".into(),
                ok: false,
                findings: Vec::new(),
                error: Some(e.to_string()),
            });
        }
    }
}

fn audit_arc(path: &str, bytes: &[u8], totals: &mut AuditTotals, files: &mut Vec<FileAudit>) {
    totals.arc_scanned += 1;
    match crate::sarc::unpack(bytes) {
        Ok(entries) => {
            for f in entries {
                let nested = format!("{path}!/{}", f.name);
                audit_entry(&nested, &f.data, totals, files);
            }
        }
        Err(e) => {
            totals.arc_failed += 1;
            files.push(FileAudit {
                path: path.to_string(),
                kind: "arc".into(),
                ok: false,
                findings: Vec::new(),
                error: Some(e.to_string()),
            });
        }
    }
}
