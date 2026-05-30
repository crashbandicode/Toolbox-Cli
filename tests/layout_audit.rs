//! `layout-audit` over the unpacked fixture bundle.
//!
//! Two checks:
//! 1. A rock-stable exact pin against the single `training-modpack`
//!    unpacked archive.
//! 2. Suspicious/unsupported-structure detection across the whole
//!    `tests/fixtures/unpacked` tree: every BFLYT parses, the HDR
//!    `info_melee` BNTX is flagged as an unsupported surface format, and
//!    the HDR player layouts' malformed-mat1 materials are flagged.
//!
//! Skipped when the fixtures are absent.

use std::path::Path;

use nx_layout_toolbox::audit::audit_path;

#[test]
fn audit_training_modpack_exact_counts() {
    let dir = Path::new("tests/fixtures/unpacked/training-modpack");
    if !dir.exists() {
        eprintln!("skipping training-modpack audit (no fixture at {})", dir.display());
        return;
    }
    let report = audit_path(dir).expect("audit");
    let t = &report.totals;

    assert_eq!(t.bflyt_scanned, 19, "bflyt count");
    assert_eq!(t.bflyt_failed, 0);
    assert_eq!(t.bflyt_v9, 19, "all training-modpack BFLYTs are v9");
    assert_eq!(t.bflyt_with_untrusted_mat, 0);
    assert_eq!(t.untrusted_materials, 0);
    assert_eq!(t.bflyt_with_v9_mat_extension, 2);
    assert_eq!(t.v9_extension_materials, 8);
    assert_eq!(t.bntx_scanned, 1);
    assert_eq!(t.bntx_failed, 0);
    assert_eq!(t.bntx_unsupported_format, 0);
    assert_eq!(t.bflan_scanned, 157);
    assert_eq!(t.bflan_failed, 0);
    assert_eq!(t.bflan_truncated_section, 0);

    println!("OK: training-modpack audit pinned (19 bflyt, 2 v9-ext, 1 bntx, 157 bflan)");
}

#[test]
fn audit_full_unpacked_detects_unsupported_and_suspicious() {
    let dir = Path::new("tests/fixtures/unpacked");
    if !dir.exists() {
        eprintln!("skipping full unpacked audit (no fixture at {})", dir.display());
        return;
    }
    let report = audit_path(dir).expect("audit");
    let t = &report.totals;

    // Every BFLYT in the bundle parses; they're all Smash-Ultimate v9.
    assert_eq!(t.bflyt_failed, 0, "no BFLYT should fail to parse");
    assert_eq!(t.bflyt_scanned, 451);
    assert_eq!(t.bflyt_v9, 451);

    // Malformed-mat1 recovery (HDR player layouts).
    assert_eq!(t.bflyt_with_untrusted_mat, 2);
    assert_eq!(t.untrusted_materials, 42);

    // Undocumented v9 material extension bytes.
    assert_eq!(t.bflyt_with_v9_mat_extension, 32);
    assert_eq!(t.v9_extension_materials, 174);

    // BNTX: exactly one unsupported-format file (HDR's recolored
    // info_melee texture pack uses an R5G6B5-family format we don't model).
    assert_eq!(t.bntx_scanned, 31);
    assert_eq!(t.bntx_failed, 1);
    assert_eq!(t.bntx_unsupported_format, 1);

    // BFLAN: all 5838 parse; 12 HDR stage-select animations have a final
    // section truncated below its declared size (round-tripped verbatim).
    assert_eq!(t.bflan_scanned, 5838);
    assert_eq!(t.bflan_failed, 0);
    assert_eq!(t.bflan_truncated_section, 12);

    // The unsupported BNTX is surfaced with its format code.
    let bad_bntx = report
        .files
        .iter()
        .find(|f| f.kind == "bntx" && !f.ok)
        .expect("the failing BNTX is listed");
    let err = bad_bntx.error.as_deref().unwrap_or("");
    assert!(
        err.contains("0x00000c01"),
        "expected the unsupported surface-format code in the error, got: {err}"
    );
    assert!(bad_bntx.path.contains("info_melee"), "unexpected path: {}", bad_bntx.path);

    // The untrusted-material findings name the HDR player layouts.
    let untrusted: Vec<&str> = report
        .files
        .iter()
        .filter(|f| f.findings.iter().any(|s| s.contains("untrusted")))
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(untrusted.len(), 2, "expected two untrusted-mat BFLYTs");
    assert!(
        untrusted.iter().all(|p| p.contains("info_melee_lct_player")),
        "untrusted-mat files should be the HDR player layouts: {untrusted:?}"
    );

    println!(
        "OK: full-unpacked audit ({} bflyt, {} bntx; 1 unsupported bntx format, {} untrusted mats)",
        t.bflyt_scanned, t.bntx_scanned, t.untrusted_materials
    );
}

#[test]
fn audit_recurses_into_archives() {
    let dir = Path::new("tests/fixtures/archives");
    if !dir.exists() {
        eprintln!("skipping archive audit (no fixture at {})", dir.display());
        return;
    }
    let report = audit_path(dir).expect("audit");
    let t = &report.totals;

    // The 6 fixture layout.arc files all unpack...
    assert_eq!(t.arc_scanned, 6);
    assert_eq!(t.arc_failed, 0);
    // ...and recursion reaches the BFLYT/BNTX/BFLAN inside them.
    assert!(t.bflyt_scanned > 0, "arc recursion found no BFLYTs");
    assert!(t.bntx_scanned > 0, "arc recursion found no BNTXs");
    assert!(t.bflan_scanned > 0, "arc recursion found no BFLANs");
    // None of the packed game/SGPO archives carry the unsupported HDR
    // texture, so everything inside parses.
    assert_eq!(t.bflyt_failed, 0);
    assert_eq!(t.bntx_failed, 0);
    assert_eq!(t.bflan_failed, 0);

    println!(
        "OK: archive audit recursed into 6 arcs ({} bflyt, {} bntx, {} bflan inside)",
        t.bflyt_scanned, t.bntx_scanned, t.bflan_scanned
    );
}
