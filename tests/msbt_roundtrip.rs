//! Fixture-gated tests for the MSBT reader + verbatim round-trip against real
//! Tears of the Kingdom message files. Skipped unless `tests/fixtures/msbt/`
//! exists with `.msbt` files — the fixtures are gitignored game data (extract
//! them with `archive-extract` on a `Mals/*.sarc.zs`).
//!
//! Every fixture must decode fully (header + LBL1 + TXT2, every label and
//! message) and re-emit byte-identically, and every message must decode to
//! text chunks without panicking. A couple of files have pinned label/message
//! counts so a structural regression fails loudly.

use std::path::{Path, PathBuf};

use nx_layout_toolbox::msbt::{read_msbt, write_msbt, write_msbt_canonical, Encoding};

fn msbt_dir() -> &'static Path {
    Path::new("tests/fixtures/msbt")
}

fn msbt_fixtures() -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(msbt_dir()) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|n| n.to_str())
                .map(|n| n.eq_ignore_ascii_case("msbt"))
                .unwrap_or(false)
        })
        .collect();
    paths.sort();
    paths
}

#[test]
fn msbt_corpus_round_trips_byte_identically() {
    let fixtures = msbt_fixtures();
    if fixtures.is_empty() {
        eprintln!("skipping: no fixtures under {}", msbt_dir().display());
        return;
    }

    let mut total_labels = 0usize;
    let mut total_messages = 0usize;
    for path in &fixtures {
        let original = std::fs::read(path).expect("read fixture");
        let doc = read_msbt(&original)
            .unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()));

        // Decode every message's chunks to prove the tag decoder walks them
        // without panicking (the verbatim writer is unaffected either way).
        if let Some(messages) = doc.messages() {
            for m in messages {
                let _ = m.chunks(doc.encoding, doc.big_endian);
            }
            total_messages += messages.len();
        }
        total_labels += doc.labels().map(|l| l.len()).unwrap_or(0);

        let written = write_msbt(&doc).expect("write");
        assert_eq!(
            written,
            original,
            "round-trip not byte-identical for {}",
            path.display()
        );
    }

    eprintln!(
        "MSBT corpus: {} file(s), {} label(s), {} message(s) — all byte-identical",
        fixtures.len(),
        total_labels,
        total_messages
    );
}

#[test]
fn canonical_writer_semantic_round_trips_corpus() {
    let fixtures = msbt_fixtures();
    if fixtures.is_empty() {
        eprintln!("skipping: no fixtures under {}", msbt_dir().display());
        return;
    }

    let mut byte_identical = 0usize;
    for path in &fixtures {
        let original = std::fs::read(path).expect("read fixture");
        let doc = read_msbt(&original).expect("parse");

        let rebuilt = write_msbt_canonical(&doc)
            .unwrap_or_else(|e| panic!("canonical write {}: {e}", path.display()));
        let doc2 = read_msbt(&rebuilt)
            .unwrap_or_else(|e| panic!("re-parse canonical {}: {e}", path.display()));

        // Semantic round-trip: labels + messages + label->message pairing match.
        assert_eq!(doc2.labels(), doc.labels(), "labels differ for {}", path.display());
        assert_eq!(
            doc2.messages(),
            doc.messages(),
            "messages differ for {}",
            path.display()
        );
        assert_eq!(doc2.entries(), doc.entries(), "entries differ for {}", path.display());

        if rebuilt == original {
            byte_identical += 1;
        }
    }
    eprintln!(
        "MSBT canonical writer: {}/{} fixture(s) also byte-identical",
        byte_identical,
        fixtures.len()
    );
}

/// Pin the structure of a couple of known fixtures so a parser regression
/// (off-by-one in the LBL1 hash walk or the TXT2 offset table) fails loudly.
/// Both are USen TotK files; skipped individually if absent.
#[test]
fn pins_known_fixture_structure() {
    // Info_BuildHouse: tiny ChallengeMsg file — 4 labels / 4 messages.
    if let Ok(bytes) = std::fs::read(msbt_dir().join("Info_BuildHouse.msbt")) {
        let doc = read_msbt(&bytes).expect("parse Info_BuildHouse");
        assert!(!doc.big_endian);
        assert_eq!(doc.encoding, Encoding::Utf16);
        assert_eq!(doc.version, 3);
        assert_eq!(doc.labels().unwrap().len(), 4);
        assert_eq!(doc.messages().unwrap().len(), 4);
        // The "Name" label resolves to the title string.
        let name = doc
            .entries()
            .into_iter()
            .find(|(l, _)| *l == "Name")
            .map(|(_, m)| m.to_display(doc.encoding, doc.big_endian));
        assert_eq!(name.as_deref(), Some("Home on Arrange"));
    }

    // Npc: larger ActorMsg file — labels and messages match in count.
    if let Ok(bytes) = std::fs::read(msbt_dir().join("Npc.msbt")) {
        let doc = read_msbt(&bytes).expect("parse Npc");
        let labels = doc.labels().unwrap().len();
        let messages = doc.messages().unwrap().len();
        assert_eq!(labels, messages, "Npc label/message count mismatch");
        assert!(labels > 100, "expected many Npc labels, got {labels}");
        // Every label index must resolve to a real message.
        assert_eq!(doc.entries().len(), labels);
    }
}
