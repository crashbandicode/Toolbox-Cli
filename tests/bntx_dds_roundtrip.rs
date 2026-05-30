//! DDS interchange invariants over `tests/fixtures/bntx/`.
//!
//! For each surface format present in the corpus, picks a 2D single-mip
//! texture and verifies the full DDS pipeline:
//!
//! 1. `export_texture_dds` produces a DDS whose metadata matches the
//!    BNTX texture and whose payload equals a fresh deswizzle.
//! 2. `Dds::write` → `Dds::read` round-trips byte-for-byte (lossless
//!    DDS serialization).
//! 3. `replace_with_dds` (preserving the texture's block height) keeps
//!    the texture's format/dims/mips/image_size, leaves the file size
//!    and every other texture untouched, and re-exports to the *same*
//!    linear payload (swizzle∘deswizzle is identity on the linear data).
//! 4. `import_dds` (as a new texture) re-exports to the same linear
//!    payload and metadata.
//!
//! Skipped when `tests/fixtures/bntx/` is absent.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use nx_layout_toolbox::bntx::decode::deswizzle_texture;
use nx_layout_toolbox::bntx::pipeline::{export_texture_dds, import_dds, replace_with_dds};
use nx_layout_toolbox::bntx::{read_bntx, write_bntx};
use nx_layout_toolbox::dds::Dds;

#[test]
fn dds_export_import_replace_invariants() {
    let dir = Path::new("tests/fixtures/bntx");
    if !dir.exists() {
        eprintln!("skipping DDS round-trip test (no fixtures at {})", dir.display());
        return;
    }

    let mut covered: BTreeSet<&'static str> = BTreeSet::new();
    let mut paths: Vec<_> = fs::read_dir(dir)
        .expect("read fixtures dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("bntx"))
        .collect();
    paths.sort();

    for path in &paths {
        let bytes = fs::read(path).expect("read fixture");
        let parsed = read_bntx(&bytes).unwrap_or_else(|e| panic!("{}: parse: {e}", path.display()));

        for (idx, tex) in parsed.textures.iter().enumerate() {
            if tex.dim != 2 || tex.mips_count != 1 || tex.array_len != 1 {
                continue;
            }
            let fmt = tex.format.name();
            if covered.contains(fmt) {
                continue;
            }
            let name = tex.name(&parsed).to_string();

            // (1) Export.
            let dds = export_texture_dds(&parsed, &name).expect("export dds");
            assert_eq!(dds.format, tex.format);
            assert_eq!((dds.width, dds.height), (tex.width, tex.height));
            assert_eq!(dds.mip_count, 1);
            assert_eq!(dds.array_count, 1);
            assert!(!dds.is_cube);
            let fresh = deswizzle_texture(&parsed, tex).expect("deswizzle");
            assert_eq!(dds.data, fresh.linear, "export payload must equal a deswizzle");

            // (2) DDS serialize -> parse round-trips losslessly.
            let serialized = dds.write();
            let reparsed_dds = Dds::read(&serialized).expect("re-read dds");
            assert_eq!(reparsed_dds, dds, "DDS write/read must round-trip");

            // (3) Replace in place from the (re-parsed) DDS.
            let others: Vec<(usize, Vec<u8>)> = parsed
                .textures
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != idx)
                .map(|(i, t)| (i, t.pixel_data(&parsed.brtd).to_vec()))
                .collect();
            let exp_image_size = tex.image_size;
            let exp_offset = tex.data_offset_in_brtd;

            let mut replaced = parsed.clone();
            replace_with_dds(&mut replaced, &name, &reparsed_dds).expect("replace_with_dds");
            let written = write_bntx(&replaced).expect("write replaced");
            assert_eq!(written.len(), bytes.len(), "replace must preserve file size");

            let re = read_bntx(&written).expect("re-parse replaced");
            let rt = &re.textures[idx];
            assert_eq!(rt.format, tex.format);
            assert_eq!((rt.width, rt.height), (tex.width, tex.height));
            assert_eq!(rt.mips_count, 1);
            assert_eq!(rt.image_size, exp_image_size);
            assert_eq!(rt.data_offset_in_brtd, exp_offset);
            // Re-export: the linear payload must come back identical
            // (swizzle∘deswizzle is identity on the linear data).
            let dds_after = export_texture_dds(&re, &name).expect("re-export after replace");
            assert_eq!(dds_after.data, dds.data, "replace round-trip changed linear payload");
            for (i, orig) in &others {
                assert_eq!(
                    re.textures[*i].pixel_data(&re.brtd),
                    &orig[..],
                    "{}: replace touched another texture (#{i})",
                    path.display()
                );
            }

            // (4) Import as a new texture, then re-export.
            let mut imported = parsed.clone();
            let new_name = format!("tex_dds_import_{fmt}");
            import_dds(&mut imported, &new_name, &reparsed_dds, None).expect("import_dds");
            let written2 = write_bntx(&imported).expect("write imported");
            let re2 = read_bntx(&written2).expect("re-parse imported");
            let new_idx = re2
                .texture_index_by_name(&new_name)
                .expect("imported texture present");
            let nt = &re2.textures[new_idx];
            assert_eq!(nt.format, tex.format);
            assert_eq!((nt.width, nt.height), (tex.width, tex.height));
            assert_eq!(nt.mips_count, 1);
            let dds_imported = export_texture_dds(&re2, &new_name).expect("export imported");
            assert_eq!(
                dds_imported.data, dds.data,
                "import round-trip changed linear payload"
            );
            // The original texture must be unchanged by the append.
            assert_eq!(
                re2.textures[idx].pixel_data(&re2.brtd),
                parsed.textures[idx].pixel_data(&parsed.brtd),
                "import (append) disturbed the source texture"
            );

            covered.insert(fmt);
            println!("OK: DDS round-trip for {fmt} '{name}' ({}x{})", tex.width, tex.height);
        }
    }

    for required in ["BC1_UNORM_SRGB", "BC4_UNORM", "BC5_UNORM", "BC7_UNORM_SRGB"] {
        assert!(
            covered.contains(required),
            "expected DDS round-trip coverage for {required}; covered={covered:?}"
        );
    }
    println!("OK: DDS interchange invariants verified for {covered:?}");
}
