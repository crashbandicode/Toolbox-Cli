# AGENTSSUMMARY.md

Living context document for AI agents working on Toolbox-Cli. Update the
**Session log** at the bottom whenever you finish a meaningful chunk of
work. Keep this file concise — link to commits / files instead of pasting
long output.

## Project

Pure-Rust **library + CLI** (crate `nx-layout-toolbox`, lib
`nx_layout_toolbox`) for editing Nintendo Switch layout/texture assets.
Produces byte-identical round-trips of BFLYT v8/v9, BFLAN, BNTX, and
SARC (plus DDS texture interchange), and opens compressed TotK/BOTW
assets in-tool (**zstd + the TotK dictionaries, Yaz0/Yaz1**). Validated on
Smash Ultimate **and** Tears of the Kingdom. Used by the SGPO project to
apply custom face-button skins. The CLI is behind a default `cli` feature;
`default-features = false` gives just the format library (no clap/anyhow).
NB: SARC read **and** write are now native (the third-party `sarc` crate was
dropped — it pinned an ancient C libzstd that conflicted with modern zstd).

**Direction:** evolving from a Smash-specific layout tool into a
general-purpose Switch-modding toolkit (live roadmap in `todo.md`).

- Repo: https://github.com/crashbandicode/Toolbox-Cli
- License: MIT (no GPL deps)
- Inspired by KillzXGaming/Switch-Toolbox (GPL); no upstream code copied.

## Build & test

```bash
cargo build           # dev (also compiles vendored libzstd via zstd-sys)
cargo build --release # release (static-links Intel ISPC, ~3 min from clean)
cargo test            # ~85 tests across 24 integration binaries + lib unit
                      # tests (compression + sarc) + 1 doctest
                      # (many skip cleanly when tests/fixtures/ is absent)
```

## Architecture

```
src/
├── lib.rs              Library entry point (modules below)
├── main.rs             CLI binary; thin wrapper over verbs::dispatch
├── error.rs            Unified high-level Error / Result (thiserror)
├── bflyt/
│   ├── sections.rs     Type definitions (incl. PaneKind::Opaque, trailing_sections)
│   ├── read.rs         Parser (malformed mat1 via flags_untrusted; opaque/unknown sections)
│   ├── write.rs        Writer (byte-identical round-trip)
│   └── ops.rs          Mutation ops (clone_pane, add_material_from_template, set_pane…)
├── bntx/
│   ├── mod.rs          BntxFile, Texture, AppendTextureSpec, append/remove
│   ├── read.rs         Full-fidelity parser (str pool, dict, RLT, BRTD)
│   ├── write.rs        Writer with canonical/preserved RLT modes
│   ├── decode.rs       Deswizzle + decode (texture2ddecoder; BCn/ASTC/R8/R8G8/RGBA8/BGRA8) → RGBA, applies channel-swizzle
│   ├── pipeline.rs     PNG/DDS import (BC7 default or RGBA8 via ImportTextureFormat), format-preserving replace, DDS export
│   └── dict_builder.rs Patricia-trie builder for _DIC
├── bflan.rs            BFLAN parse/write (verbatim sections, byte-identical) + pat1/pai1 inspect
├── byml/               BYML (binary YAML) read + verbatim round-trip; decoded value tree
│   ├── mod.rs          Byml value enum, BymlDocument, node-type constants
│   ├── read.rs         Parser (both endians, v1..=7; bounds-checked, depth-guarded)
│   ├── write.rs        Verbatim writer (raw passthrough; canonical writer TBD)
│   └── error.rs        BymlError (offset / node-type / index context)
├── compression/
│   ├── mod.rs          Codec detect + decompress/compress entry points (Cow passthrough)
│   ├── zstd.rs         libzstd wrapper + pure-Rust frame-header dict-id parser
│   ├── yaz0.rs         Pure-Rust Yaz0/Yaz1 decode + encode (hash-chain LZ)
│   └── dict.rs         DictRegistry: ZsDic.pack loader, id-keyed (zs=1, bcett=2, pack=3)
├── texpipe.rs          PNG → BC7/BC1/BC3/BC4/BC5 (intel_tex_2) → Tegra swizzle
├── dds.rs              DDS (DX10) read/write; DXGI↔TextureFormat; interchange
├── sarc/               Native SARC read+write (no `sarc` crate); crate-extractable
│   ├── mod.rs          Public API, ArcFile/ArcEntry/UnpackedFile, format constants
│   ├── read.rs         Reader (header/SFAT/SFNT, bounds-checked) — pure std
│   ├── write.rs        Per-file-alignment writer + file_alignment — pure std
│   ├── error.rs        SarcError (std + thiserror only; no walkdir)
│   └── fsutil.rs       pack_directory / unpack_to_dir (the only walkdir/fs users)
├── layout.rs           apply_manifest / validate_manifest / apply_manifest_to_arc
├── diff.rs             Structured BFLYT+BNTX before/after diff (name-keyed)
├── audit.rs            Recursive unsupported/suspicious-structure scan → JSON
│                       (compression-aware: inflates .zs/.szs, then recurses)
├── manifest.rs         SGPO skin manifest schema (serde)
└── verbs/              One file per CLI verb (~35 verbs)
```

## Round-trip status (as of commit c6159ec)

- **BFLYT**: 508/508 Smash byte-identical, **plus 373/373 TotK** (Boot +
  Common + Title `.blarc`) after the cross-game robustness pass
  (unknown sections → opaque; `scr1/ali1/spi1`/unknown in-tree → opaque
  *pane nodes* so `pas1`/`pae1` nesting round-trips; post-tree `usd1`
  after `cnt1` → trailing section).
- **BFLAN**: 7616/7616 byte-identical (5838 Smash + 1778 TotK). Verb
  `bflan-roundtrip-test` mirrors the BFLYT/BNTX ones.
- **BNTX**: Smash (`0x00040000`) **and TotK (`0x00040100`)** now both
  round-trip byte-identically — the TotK Title `__Combined.bntx` (225
  textures, ASTC_4x4_SRGB + R8G8 + BC1/BC4/BC5/RGBA8) is byte-identical,
  joining the Smash fixtures. The one tolerated diff remains the C#
  Switch-Toolbox `sgpo_one_pane_png_proof__Combined.bntx` (verbose-RLT).
  Surface formats now cover BC1–BC7, R8/R8G8/R8G8B8A8/B8G8R8A8 (`0x0c01`),
  and the full **ASTC LDR family** (4x4–12x12, UNORM+SRGB); ASTC + the
  low-bpp formats are **decode/round-trip only** (no encoder). The TotK
  container is structurally identical to `0x00040000` — only the version
  field + format set differ. Note: HDR's recolored `info_melee` B8G8R8A8
  pack parses + decodes but does **not** round-trip byte-identically
  (a C#-tool non-uniform BRTI spacing, same known-gap class as
  `sgpo_one_pane_png_proof`); its B8G8R8A8 handling is covered by a
  semantic write→re-parse test instead.
- **SARC**: native reader + custom writer re-pack `info_melee.layout.arc`
  at ~2.16 MB (was 4.7 MB via a naive 0x2000 writer), all 344 entries
  byte-identical, per-file alignment correct.
- **BYML** (binary YAML): reader decodes the full value tree across **both
  endians** and versions `1..=7`; `write_byml` re-emits the bytes captured at
  parse time, so an unmodified document is byte-identical by construction
  (the same discipline the compression layer uses). Verified on real TotK
  assets: uncompressed `CookingTable.bgyml` (LE v7), compressed RSDB
  `ActorInfo`/`Challenge` (`.byml.zs`, LE) and `GameDataList` (`.byml.zs`,
  **big-endian** — a TotK quirk the auto-detect handles) — **~3.3M nodes
  total, all byte-identical**, zero unknown-type/truncation errors. The
  from-scratch canonical writer + structural diff are the next batch.
- **Compression**: zstd decode is **byte-identical to Python 3.14's
  `compression.zstd`** on real TotK `Boot.blarc.zs` (id-1 dict) and
  `AI.Global…pack.zs` (id-3 dict, 3.9 MB → 25 MB). Containers can't be
  re-encoded byte-identically (different encoder), so the discipline here
  is `decompress(compress(x)) == x` (verified for zstd+dict and Yaz0) plus
  byte-identical **inner-format** round-trips after inflation.
- **SGPO end-to-end**: layout-apply-manifest / -arc + validate pass 4/4
  elements on a fresh `info_melee` archive.

## Tests

- **Library unit tests** (`cargo test --lib`, 43): the `verbs`
  `--texture-format` alias/`--srgb` resolver, plus `compression::yaz0`
  (encode→decode lossless on empty/short/RLE/pseudo-random/text + Yaz1
  magic + truncation rejection), `compression::zstd` (plain + raw-dict
  round-trip + frame-header dict-id parsing on hand-built headers matching
  the real ZsDic/blarc/pack descriptors), `compression::dict` (embedded-id
  parse + registry keying), `compression` (codec detect, Cow passthrough,
  high-level zstd/Yaz0 round-trips, missing-dict error), `sarc` (native
  reader↔writer round-trip incl. hash-only entries, big-endian, garbage
  rejection), `bntx` (surface-format code round-trip across **every**
  format incl. the full ASTC family, pinned codes for R8/R8G8/BGRA8/ASTC,
  ASTC block geometry + 16-byte block size), `dds` (DXGI round-trip
  for the new formats + canonical ASTC DXGI codes), and `byml` (a
  hand-built minimal little-endian array decodes to the right inline
  scalars + writes back verbatim; bad-magic / too-small / truncated-node
  rejection). These run with no fixtures — the format-code tests are the
  correctness net for the ASTC family / BYML reader we can't fully
  fixture-cover on CI.
- `tests/compression_fixtures.rs` — fixture-gated (skips without
  `tests/fixtures/totk/compression/ZsDic.pack.zs`): loads the 3 TotK
  dictionaries (ids {1,2,3}), decompresses each local `.blarc.zs` to a SARC
  and round-trips every inner BFLYT/BFLAN **byte-identically**, and proves
  `decompress(compress(x)) == x` for zstd-with-dict (frame advertises id 1)
  and Yaz0.
- `tests/byml_roundtrip.rs` — fixture-gated (skips without
  `tests/fixtures/byml/`). Walks the BYML corpus (inflating `.byml.zs` via
  the local ZsDic), and for each fixture asserts the parser decodes the
  whole tree and `write_byml` reproduces the (decompressed) input
  **byte-identically**. Pins decoded structure on the uncompressed
  `CookingTable.bgyml` (v7, LE; `RecipeList`=158, `SingleRecipeList`=15,
  `SystemData` 11 entries incl. known string values) and asserts
  `GameDataList.Product.110` parses as **big-endian** v7 — coverage across
  both endians + compressed/uncompressed.
- `tests/sarc_writer.rs` — round-trips `info_melee.layout.arc` through
  `read_arc` → `write_arc` (now exercising the **native reader** too): all
  344 files byte-identical, re-readable, output stays ~2.16 MB (not the old
  4.7 MB), and every entry sits on its required alignment with BNTX/BNSH on
  0x1000.
- `tests/bntx_cube_mip_decode.rs` — appends a 3-mip 2D texture and a
  6-face / 3-mip cube to a real BNTX, then verifies mip 0/1/2 dims halve,
  cube layer 0/5 + a deep middle-face mip decode, out-of-range mip/layer
  error cleanly, and both round-trip through DDS (export→serialize→parse→
  replace→re-export preserves the linear payload + metadata). Covers the
  `mip>0` / `layer>0` paths the single-mip-2D fixtures don't reach.
- `tests/bflan_roundtrip.rs` — walks `tests/fixtures/` recursively and
  round-trips every `.bflan` (**7616** in our setup: 5838 Smash + 1778
  TotK) byte-identically, and asserts the pat1 + pai1 inspect decoders
  run across the whole corpus. Caught + handled the HDR stage-select
  files whose final `pai1` section is truncated below its declared size.
- `tests/layout_audit.rs` — pins the `training-modpack` unpacked archive
  audit exactly (19 BFLYT all v9, 2 with v9-extension mats / 8 mats, 1
  BNTX, 157 BFLAN, 0 failures) and asserts the full `unpacked/` tree
  audit (451 BFLYT all parse + all v9; 2 BFLYT / 42 materials flagged
  `flags_untrusted`; 32 BFLYT / 174 materials with v9 extension bytes; 31
  BNTX, **all parse** now that B8G8R8A8 `0x0c01` is supported — HDR's
  recolored info_melee no longer fails; 5838 BFLAN, 0 failed, 12 with
  a truncated final section). A third case audits `archives/` (6 packed
  `layout.arc`) to cover the in-memory unpack→recurse path (95 bflyt / 6
  bntx / 1306 bflan reached inside, all parse).
- `tests/layout_diff.rs` — diffs original `info_melee` vs the generated
  SGPO fixture: pins 25 BFLYT panes added (1 `sgpo_root` pan1 under
  RootPane + 24 pic1 markers under sgpo_root), nothing removed/changed,
  BNTX unchanged. Checks reverse-diff (25 removed) and that a self-diff
  is empty.
- `tests/layout_apply_arc.rs` — applies a 2-element in-code manifest
  (panes cloned from stock `set_rep_stock_01` under `RootPane`) to
  `info_melee_original.layout.arc` via `apply_manifest_to_arc`; asserts
  both elements validate, the 344-entry count is preserved, the
  repacked archive re-opens + re-validates, only the BFLYT/BNTX entries
  changed (all others byte-identical), and a `skip_existing` re-run is a
  no-op.
- `tests/bntx_dds_roundtrip.rs` — per surface format in the corpus,
  exports a texture to DDS (DX10), asserts payload == a fresh deswizzle,
  asserts `Dds::write`/`read` round-trips, then `replace_with_dds`
  (preserves format/dims/mips/image_size, file size, other textures, and
  re-exports the identical linear payload) and `import_dds` (new texture
  re-exports the identical payload). Covers BC1/BC4/BC5/BC7.
- `tests/bntx_replace_format_preserving.rs` — walks `tests/fixtures/bntx/`
  and, for each surface format present, replaces one 2D single-mip
  texture of that format with a procedural image, asserting format /
  dims / mip / image_size / data_offset are preserved, other textures
  are byte-identical, file size is unchanged, and the target bytes
  actually changed. Requires BC1/BC4/BC5/BC7 coverage (BC7 both UNORM +
  SRGB seen).
- `tests/bntx_export_png.rs` — decodes every texture (mip 0) in every
  `tests/fixtures/bntx/` file, asserts decoded dims == BNTX metadata +
  RGBA byte count, asserts the corpus covers BC1/BC4/BC5/BC7, and pins
  channel-swizzle application (textures with `One,One,One,*` RGB swizzle
  must decode to white RGB). 989 textures / 7 fixtures in our setup
  (incl. the TotK file's ASTC_4x4_SRGB + R8G8 textures).
- `tests/bflyt_synthesis.rs` — 2 synthetic-layout round-trip tests.
- `tests/bflyt_real_fixtures.rs` — walks every `*.bflyt` under
  `tests/fixtures/` recursively (**881** in our setup: 508 Smash +
  373 TotK Boot/Common/Title), all byte-identical.
- `tests/bntx_real_fixtures.rs` — walks `tests/fixtures/bntx/`,
  tolerates the known sgpo_one_pane_png_proof RLT diff. Now also covers
  the TotK `totk_title__Combined.bntx` (v`0x00040100`, byte-identical).
- `tests/bntx_import_format.rs` — PNG-import format selection
  (fixture-gated): appends a generated image as `Rgba8`/`Rgba8Srgb` and
  asserts the re-read texture is `R8G8B8A8_UNORM`/`_SRGB` with the **exact
  source dims** (no block padding), the default BC7 path is unchanged
  (`BC7_UNORM`, padded to the 4-grid), and `apply_manifest_to_arc` with
  `texture_format = Rgba8` imports the manifest PNG as RGBA8 + validates.
- `tests/bntx_totk_formats.rs` — TotK/new-format coverage (fixture-gated):
  the TotK Title `__Combined.bntx` round-trips **byte-identically**
  (asserts version `0x00040100` + presence of ASTC_4x4_SRGB & R8G8), and
  **all 225 textures decode** (mip 0) to the right dims. A second test
  takes HDR's `info_melee` B8G8R8A8 (`0x0c01`) pack through a *semantic*
  round-trip (write→re-parse → every texture's format/dims/mips/pixels
  intact) + decode-all (it isn't byte-identical: C#-tool BRTI spacing).
- `tests/bntx_dict_edge.rs` — 10 Patricia-trie edge cases (empty,
  prefix, non-ASCII, last-bit-only, 64-key power-of-two).
- `tests/bntx_replace_in_place.rs` — 2 tests pinning the
  `bntx-replace-png` invariants: same-size splice preserves layout +
  other textures, identity-splice is byte-identical.
- `tests/bntx_remove_texture.rs` — 5 tests for `BntxFile::remove_texture`
  / `bntx-remove-texture`: remove first/middle/last preserves all
  others' pixel data and metadata, missing-name errors cleanly,
  remove + re-append produces a still-valid BNTX with the same name.
- `tests/texpipe_round_trip.rs` — full PNG → BC7 → Tegra-swizzle →
  Tegra-deswizzle → BC7-decode (`texture2ddecoder`) round-trip across
  every `tests/fixtures/png-test-images/rgba_alpha_*.png` fixture.
  Bounded per-channel mean (≤12) and peak (≤80) error to catch
  axis transposition / byte-order / block_height_log2 mismatches
  without false-failing on BC7's intrinsic lossy quantization.
- `tests/bflyt_flags_untrusted.rs` — 6 tests for the `flags_untrusted`
  guardrail: `assert_flags_trusted` ok on trusted / err on untrusted,
  `clear_untrusted_flag` round-trip, untrusted-but-consistent material
  writes cleanly, mutated-without-clear panics writer's `debug_assert!`
  in dev builds, mutate→clear→write succeeds.
- `tests/bflyt_prt1_wnd1_round_trip.rs` — 3 focused tests that walk the
  fixture corpus to find the most-complex `wnd1` (highest frame_count
  + tex_coord count) and `prt1` (highest property_count + raw bytes),
  then round-trip the BFLYT containing each and assert pane-internal
  details survive bit-for-bit. Plus a coverage assertion that the
  fixture set actually contains non-trivial examples of each.
- `tests/bntx_dict_stress.rs` — 4 stress tests for the Patricia-trie
  builder at scale: N=10,000 names under three distributions
  (sequential hex, heavy shared prefix, long shared prefix + short
  unique suffix), and N=25,000 with a soft 30s sanity budget against
  catastrophic regression. Each test prints insert/lookup timings.
- `tests/texpipe_cube_and_mip.rs` — 3 round-trip tests exercising the
  multi-mip 2D path (`compress_image_bc7_with_mips`, 4-mip chain),
  the cube-map path (`compress_cube_bc7`, 6 faces × 1 mip), and the
  combined cube + multi-mip path (6 × 3). Each verifies the linear-
  size accounting matches `bc7_mip_size_bytes`'s per-level math, then
  decodes mip 0 / face 0 mip 0 / face 5 mip 0 and asserts within the
  same per-channel error budget as the single-mip test.
- `tests/bntx_dict_parallel_order.rs` — 1 test pinning the invariant that
  the rebuilt `_DIC` trie node order matches the BRTI/texture order
  (regression guard for the in-game `_DIC` ordering bug).
- `tests/bntx_rlt_large.rs` — 2 tests for the canonical RLT builder at
  >255 textures (one-pointer-per-struct path).

## Conventions

- **Errors**: parse errors carry section-index, material-index, or pane
  context. Add similar context when extending. Look at how
  `read_mat1` / `read_bflyt` wrap inner errors with `map_err(context)`.
  Per-format modules expose their own typed error enums (`BflytError`,
  `BntxError`, `SarcError`) that convert into the crate-level `Error` via
  `#[from]`; new format modules should follow the same pattern.
- **Verbatim preservation**: When reading a structure we don't fully
  decode, capture it as opaque bytes (`trailing`, `opaque_sections`,
  `parts.raw_property_data`, `text.trailing`) and re-emit verbatim.
  This is how we hit byte-identical round-trip on real-world malformed
  inputs.
- **Mutations**: BFLYT and BNTX struct counts must agree with their
  encoded flags / RLT. The writer detects mismatches and either
  recomputes (BFLYT `rebuild_flags`) or rebuilds the canonical layout
  (BNTX `relocation_table_dirty` → `build_canonical_reloc_table`). When
  adding new mutation paths, mirror this pattern.
- **Comments**: explain *why* and *what's non-obvious*, not what the
  code already says. Reference specific fixture filenames when a fix
  was driven by a real-world case.
- **CLI verbs**: one file per verb under `src/verbs/`, with an `Args`
  struct (clap derive) and a `pub fn run(args: Args) -> Result<ExitCode>`
  entry point. Wire up in `src/verbs/mod.rs`.

## Known gaps

| Item | Severity | Notes |
|---|---|---|
| `sgpo_one_pane_png_proof.bntx` 8KB RLT diff | Low | C# tool's verbose RLT. Both layouts valid. |
| TotK BNTX (v`0x00040100`) + ASTC / low-bpp formats | Resolved | Read/write/decode for `0x00040100`, full ASTC LDR family (4x4–12x12), R8/R8G8, B8G8R8A8 (`0x0c01`). TotK `__Combined.bntx` round-trips byte-identically. Decode/round-trip only (no ASTC/low-bpp **encoder** yet) — see `todo.md`. |
| HDR `info_melee` B8G8R8A8 pack not byte-identical | Low | C#-tool non-uniform BRTI spacing (same class as `sgpo_one_pane_png_proof`). Parses + decodes; semantic round-trip tested. |
| In-tool ZSTD+dict / Yaz0 decompression | Resolved | `compression` module: zstd (+ TotK dicts via `ZsDic.pack.zs`) decode **byte-identical to Python 3.14 `compression.zstd`**; native Yaz0/Yaz1; verbs `decompress`/`compress`/`archive-extract`; compression-aware `layout-audit --dict/--romfs`. Re-compression is lossless, not container-identical (expected). |
| BYML canonical (from-scratch) writer | In progress | Stage A is read + inspect + **verbatim** round-trip (byte-identical for unchanged docs, both endians, v1..=7). The from-scratch canonical writer (for mutated/synthesized trees), semantic round-trip, and `byml-diff` are Stage B. |
| In-game runtime validation on Switch hardware | High value | Untestable without hardware. |
| v9 BFLYT 60-byte material extension (flag bit 19) | Low | Captured verbatim; can't construct from scratch (unspec'd). User accepted this gap. |
| `flags_untrusted` materials can't safely re-encode after sub-section count changes | Resolved in TODO #4 | `Material::assert_flags_trusted()` + `clear_untrusted_flag()` API; writer `debug_assert!` catches misuse via `original_section_size` snapshot. |
| `prt1` / `wnd1` round-trip not exhaustively unit-tested | Resolved in TODO #5 | `tests/bflyt_prt1_wnd1_round_trip.rs` discovers and round-trips the most-complex example of each plus pane-internal field-by-field comparison. Coverage check asserts non-trivial examples exist. |
| BNTX dict insertion at N ≥ 10,000 untested | Resolved in TODO #6 | `tests/bntx_dict_stress.rs` covers 10k under three distributions and 25k as a scale-headroom check. Current numbers: ~3-10 ms total insertion, ~100 ns avg lookup. |
| Cube-map / multi-mip integration tests | Resolved in TODO #7 | `tests/texpipe_cube_and_mip.rs` covers multi-mip 2D, cube single-mip, and cube + multi-mip combined; each verifies layout accounting + decode round-trip on the levels we have a cheap reference for. |
| No GitHub Actions CI | Low | Add when remote CI infra needed. |

## Workflow rules for agents

1. **Don't commit or push without explicit user OK.** Stage changes,
   show them, and wait.
2. **Update this file** at the end of each meaningful work batch — add
   an entry under "Session log" with a short description and the commit
   hash.
3. **Run `cargo test` before declaring a task complete.** If tests
   touch real fixtures, also run a representative manual command
   (e.g., `bntx-roundtrip-test`).
4. **Add fixture-driven tests when fixing real-world bugs.** When a
   community-mod file exposes a parser bug, add a focused test with
   that file's signature so future regressions fail loudly.

## TODO / roadmap

The **live, prioritized backlog lives in `todo.md`** — read it first for
current work. Status snapshot: `origin/main` is at **bff4757** (a doc-sync on
top of **14e5991**, the PNG-import RGBA8 format option), itself on top of
**f46788c** (BNTX `0x00040100` + ASTC/low-bpp) and the earlier compression
(bd454c7) + SARC-hardening (a36c07c) commits — all pushed. The current
session adds BYML Stage A (read + inspect + verbatim round-trip) — see the
top Session log entry:

- ✅ **Prior 7-item handoff** (bntx export-png/all, format-preserving
  replace, DDS export/import/replace, layout-apply-arc, layout-diff,
  layout-audit, BFLAN inspect + roundtrip) — done (commit d958a13),
  plus post-review hardening (`bntx_cube_mip_decode` for mip>0/layer>0 +
  DDS; archive-recursion audit test).
- ✅ **Custom SARC writer** (per-file alignment, no 0x2000 bloat).
- ✅ **Doc refresh** + **BFLYT cross-game robustness (TotK)** + **TotK
  fixtures & `bflan-roundtrip-test` verb** (commits e19d7bc, c6159ec).
- ✅ **Compression module (zstd + dict + Yaz0) + recursive archives**
  (commit bd454c7): native SARC reader (dropped the `sarc` crate); `zstd
  0.13`; `compression::{mod,zstd,yaz0,dict}`; verbs `decompress`/`compress`/
  `archive-extract`; compression-aware `layout-audit`. Decode byte-identical
  to Python on real TotK `.blarc.zs`/`.pack.zs`.
- ✅ **SARC crate-ready hardening** (commit a36c07c): `src/sarc.rs` → `src/sarc/`
  (`mod`/`read`/`write`/`error`/`fsutil`); typed `SarcError` (wired into the
  crate `Error` via `#[from]`); std-only codec core with the `walkdir`/`fs`
  helpers isolated in `fsutil`; 14 original unit tests (LE/BE round-trip,
  alignment derivation, hash-only ordering, empty/single/large, malformed
  inputs, pseudo-random property round-trip). Ready to lift into `nx-sarc`.
- ✅ **BNTX `0x00040100` + full ASTC family + R8/R8G8/B8G8R8A8** (this
  session, uncommitted): version gate + 28 ASTC variants (`AstcBlock`) +
  R8/R8G8/BGRA8; byte-identical round-trip on a real 225-texture TotK
  `__Combined.bntx`; decode for ASTC (all footprints), R8/R8G8, BGRA8;
  decode/round-trip only (no encoder). `0x0c01` now parses so `layout-audit`
  reports 0 unsupported BNTX. `filename_offset` writer now locates the
  container name by value (fixes non-slot-1 string pools).
- ▶️ **In progress: BYAML/BYML.** Stage A (read + `byml-inspect` +
  verbatim byte-identical round-trip + fixture/unit tests) is done this
  session; Stage B (from-scratch canonical writer + semantic round-trip +
  `byml-diff`) is next. Then (see `todo.md`): RSTB/RESTBL → MSBT → AAMP →
  BFLYT advanced mutations → layout-repair → BFRES inspect-only → project
  workflow. (ASTC/low-bpp **encode** is a smaller follow-up under Hardening.)

Standing backlog (no owner):

- v9 BFLYT 60-byte material extension decode (captured verbatim today).
- GitHub Actions CI workflow (gated on shipping fixtures to CI).
- In-game runtime validation on Switch hardware (requires hardware).

## Session log

### 2026-05-31 — BYML (binary YAML) read + inspect + verbatim round-trip (Stage A)
Roadmap item #2, Stage A. BYML is the most-edited Switch data format (game
parameters, RSDB actor/resource databases, cooking/recipe tables, event data,
GameDataList, …). Done in the disciplined order: read/inspect + byte-identical
round-trip FIRST; the from-scratch canonical writer + diff are Stage B (same
session).

**Format (verified on real TotK bytes).** 16-byte header (`YB` LE / `BY` BE,
version `u16`, hash-key-table + string-table + root offsets) → two `0xc2`
string tables → a recursive tagged-union node tree. Node tags: containers
`0xc0` array / `0xc1` hash; inline scalars `0xd0` bool / `0xd1` s32 / `0xd2`
f32 / `0xd3` u32 / `0xa0` string-index / `0xff` null; offset-referenced
`0xd4` s64 / `0xd5` u64 / `0xd6` f64 / `0xa1` binary. Node header packs a type
byte + 24-bit count (count endian-aware); hash entries mirror it (24-bit key
index + type byte + 4-byte value).

**New `src/byml/`.** `read.rs` — bounds-checked, depth-guarded parser, both
endians, v1..=7; `mod.rs` — `Byml` value enum (distinct s32/u32/f32/s64/u64/
f64 so widths round-trip) + `BymlDocument { version, big_endian, root, raw }`;
`write.rs` — verbatim writer (returns the bytes captured at read time →
byte-identical for unchanged docs); `error.rs` — typed `BymlError` (offset /
node-type / index context), wired into the crate `Error` via `#[from]`. Verbs
`byml-inspect` (`--json`/`--max-depth`; transparently inflates `.byml.zs` via
`--dict`/`--romfs`) and `byml-roundtrip-test`.

**Validation (real fixtures).** Round-trips **byte-identically**, ~3.3M nodes
total, zero unknown-type/truncation errors: `CookingTable.bgyml` (LE v7,
uncompressed, 3981 nodes), RSDB `Challenge`/`ActorInfo` (`.byml.zs`, LE v7,
26.8k / 398k nodes), and `GameDataList.Product.110`/`.140` (`.byml.zs`,
**big-endian** v7, 1.43M nodes each — a TotK quirk the magic-based auto-detect
handles, exercising every BE code path on real data).

**Tests/docs.** `tests/byml_roundtrip.rs` (fixture-gated: byte-identical walk +
pinned `CookingTable` structure + `GameDataList` big-endian assertion); 4 new
fixture-free lib unit tests in `byml::read` (hand-built minimal LE array +
bad-magic/too-small/truncated-node rejection). `cargo test` green (43 lib +
all integration); `clippy --all-targets` clean; `--no-default-features`
builds. Fixtures copied into the gitignored `tests/fixtures/byml/`.

### 2026-05-31 — PNG import: selectable BC7 / uncompressed RGBA8
Lets SGPO generate sharper UI skins: BC7's block quantization softens small
text/letters, so an uncompressed RGBA8 import path was added (BC7 stays the
default — unchanged). Committed + pushed this session (14e5991).

- New `pipeline::ImportTextureFormat { Bc7, Rgba8, Rgba8Srgb }` +
  `ImportOptions::texture_format` (default `Bc7`). `import_image` branches:
  BC7 keeps the in-game-validated `compress_image_bc7[_with_mips]` path
  **byte-for-byte**; RGBA8 routes through `compress_image_to_format`
  (no compression, `--quality` ignored). `import_cube_png_files` errors on
  a non-BC7 request (cube is BC7-only; the feature is 2D).
- New generic `AppendTextureSpec::texture_2d_with_mips(format, …)`;
  `bc7_2d_with_mips` now delegates to it (identical BC7 specs). Defaults
  are format-agnostic — verified against real BRTI headers: every Smash
  texture (any format) uses `flags=1`, every TotK texture `flags=9`, i.e.
  the `flags` byte tracks the game/tool, not the surface format, so the
  Smash-default `1` is correct for SGPO appends.
- `ApplyOptions::texture_format` threads the choice through
  `apply_manifest` / `apply_manifest_to_arc`. CLI: `--texture-format`
  (`bc7`|`rgba8`|`rgba8-srgb`, plus aliases `bc7-unorm`/`bc7-srgb`/
  `r8g8b8a8`/`r8g8b8a8-srgb`/…) on `bntx-import-png`,
  `layout-apply-manifest`, `layout-apply-arc`; default `bc7`. A shared
  `verbs::parse_import_texture_format` resolves the flag (+ `--srgb`).
- Tests: `tests/bntx_import_format.rs` (rgba8/rgba8-srgb append as
  `R8G8B8A8_UNORM`/`_SRGB`, exact source dims, write→read-back, BC7 default
  unchanged; plus `apply_manifest_to_arc` with `texture_format = Rgba8`
  imports + validates) and a `verbs` unit test pinning the
  `--texture-format` alias/`--srgb` resolution. `cargo test` green
  (39 lib + all integration); `clippy --all-targets` clean;
  `--no-default-features` builds.

### 2026-05-31 — BNTX 0x00040100 + full ASTC family + R8/R8G8/B8G8R8A8
Roadmap item #1. Unlocks TotK textures (and HDR's B8G8R8A8 Smash mods).
Committed + pushed this session. Done in the disciplined staged order:
read/inspect + byte-identical round-trip FIRST, then decode, encode
deferred.

**Findings (real fixtures).** Extracted a TotK `__Combined.bntx` via
`archive-extract` on `Title*.blarc.zs`: version `0x00040100` is
**structurally identical** to `0x00040000` (same `0x198` info-ptr offset,
`0x150` memory pool, uniform `0x2a8` BRTI stride — all section offsets land
where the writer computes them). New surface formats confirmed by
byte-math (swizzled `image_size` matches the computed block layout):
`0x2d06` = ASTC_4x4_SRGB (swizzled `10240 = 320×32`), `0x0901` = R8G8
(`8192 = 128×64`, swizzle `[R,R,R,G]`). HDR's `0x0c01` measured as
B8G8R8A8 (32bpp identity-swizzle, *not* the 16bpp the old TODO guessed).
ASTC family codes (`0x2D` 4x4 … `0x3A` 12x12) cross-checked against public
BNTX research + ARM's 14-footprint ordering (no GPL code consulted).

**Stage A — parse + round-trip.** `read.rs` accepts `0x00040000` **and**
`0x00040100`. New `AstcBlock` enum (14 footprints; `index()` is the single
ordering source for the surface-format high byte `0x2D+idx` and the DXGI
code `134+idx*4`) + `TextureFormat::Astc { block, srgb }`, plus flat
`R8Unorm`/`R8G8Unorm`/`Bgra8Unorm`/`Bgra8UnormSrgb`. Wired
`to/from_surface_format`, `block_dim`, `block_size` (ASTC=16), `has_alpha`,
`name`, and the `dds.rs` DXGI map. **Byte-identical round-trip on the real
225-texture TotK `__Combined.bntx`** (verified via `bntx-roundtrip-test`
and a fixture test). Also fixed a latent writer bug: `filename_offset`
hard-assumed the container name was `strings[1]`; it now locates the name
by value (byte-identical for standard files, correct for HDR's reordered
pool).

**Stage B — decode.** `decode.rs`: `block_dim_for` derives arbitrary
footprints from `TextureFormat::block_dim`; ASTC dispatches to
`texture2ddecoder::decode_astc(.., bw, bh, ..)`; R8/R8G8/B8G8R8A8 expand
straight to RGBA (BGRA swaps R↔B). **All 225 TotK textures export to PNG**
and three spot-checks render as real images (ASTC bonus icon, R8G8 "x2"
text, BGRA meter). Channel-swizzle still applied on top.

**Stage C — encode deferred.** ASTC + the low-bpp formats are
non-encodable (clear `texpipe` error; `format_is_encodable` excludes them).

**Tests/docs.** New `tests/bntx_totk_formats.rs` (TotK byte-identical
round-trip + decode-all; HDR B8G8R8A8 semantic round-trip + decode-all);
4 new lib unit tests pin the surface-format/DXGI code maps across the
**whole** ASTC family (the correctness net we can't fully fixture-cover);
`tests/layout_audit.rs` updated (HDR `0x0c01` now parses → 0 unsupported
BNTX). Copied the TotK BNTX into `tests/fixtures/bntx/` (gitignored) so the
existing round-trip + export tests exercise it too. `cargo test` green
(38 lib + all integration); `clippy --all-targets` clean;
`--no-default-features` builds. **Known gap:** HDR `info_melee` isn't
byte-identical (C#-tool non-uniform BRTI spacing, same class as
`sgpo_one_pane_png_proof`).

### 2026-05-30 — SARC crate-ready hardening (module split + SarcError)
Follow-up to the compression batch (same session). Restructured the native
SARC code toward a future standalone `nx-sarc` crate, without behavior
changes (round-trip + real-data CLI output identical to before).

- `src/sarc.rs` → `src/sarc/`: `mod.rs` (public API, `ArcFile`/`ArcEntry`/
  `UnpackedFile`, format constants), `read.rs` + `write.rs` (the codec core,
  **pure std**), `error.rs` (typed `SarcError`, `std + thiserror` only — no
  `walkdir`), `fsutil.rs` (the only `walkdir`/`std::fs` user: directory
  pack/unpack; a future optional `fs` feature).
- **Typed errors.** Replaced the stringly `Error::Sarc(String)` with
  `Error::Sarc(#[from] SarcError)`; `SarcError` has structured parser
  variants (offsets, node index, byte ranges) matching the
  `BflytError`/`BntxError` convention. All callers were `?`/`.map_err`/
  `.expect` and needed no changes beyond the `#[from]` conversion.
- **Tests (14, original).** Authored from the format spec + our round-trip
  discipline; malformed-input checklist informed by the MIT `jam1garner/sarc`
  crate (credited in a comment) — no verbatim copying, no GPL, no committed
  fixtures. Covers LE/BE round-trip, alignment derivation (BNTX/BNSH→0x1000,
  nested→0x2000, Yaz0→0x80, exponent clamp), hash-only ordering stability,
  empty/single/2000-entry archives, bad magic/BOM/missing-SFAT/node-OOB, and
  a pseudo-random property round-trip. `cargo test` green (34 lib unit tests
  total); `clippy --all-targets` clean; `--no-default-features` builds.

### 2026-05-30 — Compression module (zstd+dict, Yaz0) + native SARC reader
Roadmap item #1. Lets the tool open real TotK/BOTW dumps in-process.

**Dependency surgery.** Adding modern `zstd 0.13` collided with the
third-party `sarc` crate, which transitively pinned an ancient C libzstd
1.4.4 (`zstd 0.5` → `zstd-sys 1.4.x`; Cargo forbids two `links = "zstd"`).
Resolved by writing a **native SARC reader** (`sarc::parse_sarc`: header +
SFAT + SFNT, bounds-checked, LE/BE, hash-only entries) so the `sarc` crate
could be dropped entirely — we already owned the writer. `read_arc`/`unpack`
now route through it; `tests/sarc_writer.rs` exercises the reader on a real
`layout.arc`, plus 3 new lib unit tests. (This also removed a stale C libzstd
from the tree. A dedicated "SARC crate-ready" follow-up is queued in
`todo.md`: module split + `SarcError` + comprehensive tests, toward a future
standalone `nx-sarc` crate.)

**`compression` module.** `zstd 0.13` (vendored libzstd; MIT wrapper, libzstd
under BSD-3 — GPLv2 grant not taken, so GPL-free). New module:
- `zstd.rs` — wrapper over libzstd plus a **pure-Rust frame-header parser**
  (`frame_dictionary_id`) that reads the referenced `Dictionary_ID` without
  decompressing (handles the single-segment / window-descriptor / 1–4-byte
  id cases; matches the real `0xA0`/`0x61`/`0x81` TotK descriptors).
- `yaz0.rs` — **pure-Rust** Yaz0/Yaz1 decode (byte-exact) + encode
  (hash-chained greedy LZ, 0x1000 window, lengths to 0x111). From the public
  spec; no GPL consulted.
- `dict.rs` — `DictRegistry` keyed by each dictionary's embedded id;
  `from_zsdic_pack` decompresses TotK's `ZsDic.pack.zs` (plain zstd) → SARC →
  registers `zs`(1)/`bcett`(2)/`pack`(3).
- `mod.rs` — `Codec` detect + `decompress` (Cow; picks the dict by frame id,
  passes uncompressed input through borrowed) + `compress_zstd`/`compress_yaz0`.

**Verbs + audit.** `decompress`, `compress` (`--format zstd|yaz0`, `--level`,
`--dict-id`), and `archive-extract` (recursively inflate + unpack, path-
traversal-guarded). `layout-audit` is now compression-aware (`--dict`/
`--romfs`): it transparently inflates `.zs`/`.szs` and recurses, with a
content-magic dispatch fallback for inflated bodies; added `compressed_*`
counters. A shared `verbs::load_dict_registry` backs all four.

**Validation (real ROMFS, gold standard).** Our zstd decode is
**SHA-256-identical to Python 3.14 `compression.zstd`** on `Boot.blarc.zs`
(id-1 dict, 6179→20608) and `AI.Global…pack.zs` (id-3 dict, 3.9 MB→25 MB).
`archive-extract` of `ZsDic.pack.zs` yields the 3 dicts (131072 B each).
`layout-audit --romfs` on a `.blarc.zs` inflates → unpacks → audits 2 BFLYT
+ 1 BFLAN (parse OK) and flags the inner BNTX as `0x00040100` (the TotK
texture gap, todo #2). Lossless round-trips: zstd+dict (20608→6171→20608)
and Yaz0 (→7241→) both byte-exact. `cargo test` green (incl. fixture-gated
`tests/compression_fixtures.rs`); `cargo clippy --all-targets` clean.

### 2026-05-30 — Doc refresh + BFLYT cross-game robustness + TotK gates
Three batches toward "general Switch modding tool" (commits e19d7bc,
c6159ec):

**Doc scan/fix.** README was stale (pre-handoff): refreshed the status
table, verb list (export/DDS/replace/remove/bflan/diff/audit/apply-arc),
architecture tree, dependencies (texture2ddecoder; custom SARC writer),
limitations (multi-mip/cube, RLT hardened, alignment fixed), and the
test-corpus counts. Fixed `lib.rs` rustdoc module list (added
bflan/dds/diff/audit) and the stale `(commit 0208194)` round-trip-status
header.

**BFLYT robustness (TotK).** The parser hard-failed on TotK's `ctl1`
section and several TotK pane-nesting shapes. Fixes:
- Unknown sections are no longer fatal. File-level ones (before the pane
  tree, e.g. `ctl1` between `mat1` and the first pane) → file-level
  `OpaqueSection` re-emitted before the root pane.
- In-tree unknown/`scr1`/`ali1`/`spi1` sections → new `PaneKind::Opaque`
  **pane nodes** carrying verbatim bytes. They were previously flattened
  to anchored sections, which unbalanced `pas1`/`pae1` and dropped
  sections when a real pane nested under them (`pan1 pas1 ali1 pas1 …`).
- A `usd1` after the pane/group tree + `cnt1` (TotK `Pa*` layouts end
  `… gre1 cnt1 usd1`) → `BFLYT.trailing_sections`, re-emitted last.

Result: **0 → 373/373 TotK Boot/Common/Title BFLYT byte-identical**, and
Smash stays **508/508** (the changes are byte-identical for Smash too —
opaque panes emit the same magic sequence).

**TotK fixtures + `bflan-roundtrip-test` verb.** Added the
`bflan-roundtrip-test` verb (mirrors bflyt/bntx). Verified BFLAN is
already cross-game: all 1778 TotK BFLAN round-trip byte-identical.
Adopted the TotK Boot/Common/Title bflyt+bflan as local (gitignored)
fixtures under `tests/fixtures/totk/`, so the recursive round-trip gates
now cover both games: **881 BFLYT** and **7616 BFLAN**, all
byte-identical. All tests pass; clippy clean. Decompressed TotK assets
live in `%TEMP%\totk_probe` (Python 3.14 stdlib `compression.zstd` + the
`zs.zsdic` extracted from `Pack/ZsDic.pack.zs` via our own `sarc-unpack`);
the source dump is at the Eden RomFS path.

### 2026-05-29 — Custom SARC writer (per-file alignment)
Replaced the `sarc` crate's writer (we still use its reader) with a
native `sarc::write_sarc` that gives each file the alignment it actually
needs instead of padding everything to 0x2000. Alignment is derived from
content via the `nn::util::BinaryFileHeader` convention — BOM at 0x0C →
`1 << byte[0x0E]` — verified against the fixtures (BNTX & BNSH report
0x1000; FLYT/FLAN/`info` have no BOM there → 8-byte minimum); nested
SARC → 0x2000, Yaz0 → 0x80; clamped to [0x8, 0x2000]. `write_arc` and
`pack_directory` now route through it. Result: repacking
`info_melee.layout.arc` is **2.16 MB again (2166040 → 2161600)** instead
of 4.7 MB, and `layout-apply-arc` grows the file by ~4 KB (the two new
textures) rather than doubling it. Bonus: the native writer preserves
multiple hash-only (unnamed) entries that the crate writer collapsed via
a hash-keyed map. `tests/sarc_writer.rs` round-trips the arc (all 344
files byte-identical, re-readable), asserts every entry sits on its
required alignment, and that BNTX/BNSH land on 0x1000. Follow-up backlog
captured in `todo.md`. All tests pass; clippy clean.

### 2026-05-29 — BFLAN roundtrip + inspect (handoff #7)
New `src/bflan.rs`: BFLAN shares BFLYT's container shape (0x14 `FLAN`
header + `magic + u32 size` sections). We capture each section's bytes
verbatim (with its on-disk `declared_size`) so `write_bflan` reproduces
a **byte-identical** file, and decode `pat1` (animation name, frame
range, child-binding, group bindings) and `pai1` (frame size, loop,
texture list, entry name/target/tag-count) read-only for inspect. Verb
`bflan-inspect` (text + `--json`). `tests/bflan_roundtrip.rs` round-
trips all 5838 fixtures byte-identically and exercises both decoders.
Real-world quirk handled: 12 HDR stage-select animations declare a
`pai1` size a few bytes past EOF — we clamp the captured payload to the
bytes present while preserving the declared size field, so the file
re-emits exactly (the writer would otherwise shrink the size field).
Also extended `layout-audit` to scan `.bflan` (counts + a
"truncated final section" finding); audit test updated accordingly.
All tests pass; clippy clean.

### 2026-05-29 — layout-audit (handoff #6)
New `src/audit.rs` recursively walks a directory (or single file /
archive — SARC entries are unpacked + audited too) and reports
unsupported/suspicious structures: BFLYT parse failures, v9 layouts,
materials flagged `flags_untrusted` (malformed-mat1 recovery), materials
carrying undocumented v9 extension bytes, and BNTX parse failures
(incl. unsupported surface formats). Aggregate `AuditTotals` + per-file
findings serialize to JSON. The walker checks extensions *before*
reading so the thousands of non-layout files in an unpacked archive are
skipped (full `unpacked/` scan dropped 34s → <1s). Verb `layout-audit
-p <path> [--json] [--fail-on-error]`. `tests/layout_audit.rs` pins the
counts (training-modpack exact + full-unpacked detection of HDR's
unsupported BNTX format `0x00000c01` and 42 untrusted materials). All
tests pass; clippy clean.

### 2026-05-29 — layout-diff (handoff #5)
New `src/diff.rs` produces a structured before/after diff of a layout's
BFLYT + BNTX, matching panes/materials/textures by **name** (stable
across index shifts): txl1 refs, materials (colors + bound texture
names), and panes (kind/parent/transform/size/alpha/visible/material)
added/removed/changed; BNTX textures (dims/format/mips/array + a pixel-
data-changed flag) added/removed/changed. Serializes to JSON. Verb
`layout-diff --old --new [--json]` diffs two `layout.arc` files.
`tests/layout_diff.rs` pins the original-info_melee → generated-SGPO
diff at exactly 25 added panes (sgpo_root + 24 markers, BNTX unchanged),
verifies the reverse diff flips them to removals, and that self-diffs
are empty. All tests pass; clippy clean.

### 2026-05-29 — layout-apply-arc end-to-end (handoff #4)
Added `layout::apply_manifest_to_arc`: unpack a packed `layout.arc` in
memory, apply an SGPO manifest to the contained BFLYT+BNTX, validate,
and re-pack **every** entry into a new archive. To do this losslessly,
extracted in-memory cores `apply_manifest_in_memory` /
`validate_manifest_in_memory` (the on-disk `apply_manifest` /
`validate_manifest` now wrap them) and added `sarc::read_arc` /
`write_arc` + `ArcFile`/`ArcEntry` that preserve **all** entries
(named and hash-only) through a round-trip — so editing two files never
drops the other 342. Verb `layout-apply-arc` wraps it (reports
applied/skipped + validation, exits non-zero on validation failure
unless `--allow-invalid`). `tests/layout_apply_arc.rs` proves the full
pipeline on `info_melee_original.layout.arc` (2 elements, 344 entries
preserved, only BFLYT/BNTX changed, re-open re-validates, idempotent
re-run). NOTE (superseded by the custom-SARC-writer entry below): at
first this used the `sarc` crate writer, which padded every entry to
0x2000 and bloated the repack 2.1MB → 4.7MB. All tests pass; clippy
clean.

### 2026-05-29 — DDS interchange (handoff #3)
New `src/dds.rs`: a focused DDS reader/writer. We always **write** the
DX10 extended header (exact DXGI format incl. sRGB round-trips) and
**read** both DX10 and the common legacy FourCCs (DXT1/3/5, ATI1/BC4U,
ATI2/BC5U, 32-bit RGBA) for interop with texconv/GIMP/Switch-Toolbox.
The DDS payload is the tightly-packed linear surface (layer-major, then
mip) — exactly what `tegra_swizzle` deswizzle emits / swizzle consumes,
so BNTX↔DDS is just (de)swizzle + header. Added BNTX glue in
`bntx::pipeline`: `export_texture_dds` (deswizzle → Dds),
`import_dds` (swizzle → append new texture, canonical block height
inferred), `replace_with_dds` (re-tile with the texture's stored block
height → in-place splice, structural-change-free). Three thin verbs
wrap them. `tests/bntx_dds_roundtrip.rs` proves the export→serialize→
parse→replace/import→re-export invariants per format (the linear
payload survives swizzle∘deswizzle identically; metadata + file size +
other textures are preserved). All tests pass; clippy clean.

### 2026-05-29 — Format-preserving bntx-replace-png (handoff #2)
`bntx::pipeline::replace_texture` no longer hard-codes BC7. It now
re-encodes the source over an existing texture **in the texture's own
surface format**: added `texpipe::compress_image_to_format` (a
format-parameterized encoder over `intel_tex_2` bc1/bc3/bc4/bc5/bc7 +
raw RGBA, with `format_is_encodable` gating BC2/BC6 out) and a
channel-swizzle *inverter* (`invert_channel_swizzle` /
`remap_image_for_format`) so the source's channels are routed into the
right block channels (a BC4 alpha mask `One,One,One,Red` takes the PNG
alpha; BC5 `Red,Red,Red,Green` takes R + alpha). The re-encode is tiled
with the texture's stored block height (`size_range`) so the swizzled
length matches the slot and the splice stays structural-change-free.
Source dims are validated against the texture's *logical* size up front
(the encoder pads to the block grid internally, e.g. a 5x5 BC1 → 2x2
blocks). `tests/bntx_replace_format_preserving.rs` exercises one replace
per format across the corpus (BC1/BC4/BC5/BC7). All 43 tests pass;
clippy clean.

### 2026-05-29 — BNTX→PNG export (handoff #1)
Added the decode counterpart to `texpipe`: `src/bntx/decode.rs`
deswizzles a texture's block-linear data (driven by the stored
`size_range` block height so it exactly inverts the on-disk tiling),
decodes via the pure-Rust MIT/Apache `texture2ddecoder` (BC1-BC7 +
R8G8B8A8), and applies the texture's `channel_swizzle` so exported
pixels match what the GPU samples (BC4 alpha masks `One,One,One,Red`
→ white-with-alpha; BC5 `Red,Red,Red,Green` → grayscale-with-alpha).
`texture2ddecoder` moved from dev- to regular dependency. Two verbs:
`bntx-export-png` (one named texture, `--mip`/`--layer`/`--raw`) and
`bntx-export-all` (every texture → `<name>.png` in a dir, `--keep-going`).
`tests/bntx_export_png.rs` decodes all 764 textures across 6 fixtures,
asserts dims/byte-count vs metadata, format coverage (BC1/BC4/BC5/BC7),
and channel-swizzle correctness (148 white-mask textures verified). All
green.

### 2026-05-29 — Library-ify + crates.io prep + RLT >255 hardening
Renamed the crate to `nx-layout-toolbox` (lib `nx_layout_toolbox`, bin
`nx-layout-toolbox`). Gated the CLI behind a default `cli` feature so the
library builds with no `clap`/`anyhow` (verified `--no-default-features`).
Added a unified `nx_layout_toolbox::Error` / `Result` (thiserror) and moved
`texpipe` off `anyhow`. Extracted the reusable logic out of the CLI verbs
into the library so SGPO can import it directly:
- `sarc` module — `pack_directory`, `unpack`, `unpack_to_dir`.
- `BFLYT` methods — `add_texture_ref`, `add_material_from_template`,
  `rename_material`, `clone_pane` (`ClonePaneSpec`), `set_pane` (`PaneEdit`).
- `bntx::pipeline` — `import_image` / `import_png_file` /
  `import_cube_png_files`, `replace_texture` (`ReplaceSource`).
- `layout` — `apply_manifest` (`ApplyOptions`/`ApplyReport`),
  `validate_manifest` (`ValidateOptions`/`ValidateReport`).
All CLI verbs are now thin wrappers over these. Added a crate-level rustdoc
overview + a `prelude`, a `build.rs` that emits `-lstdc++` on linux-gnu so
downstream binaries link `intel_tex_2`'s ISPC objects without extra config,
and `[package.metadata.docs.rs]`. Phase-0 fix: `build_canonical_reloc_table`'s
texture-info-array entry used `offset_count: n as u8`, truncating for >=256
textures; it now uses one-pointer-per-struct past 255 (preserving the
in-game-verified <=255 encoding), covered by `tests/bntx_rlt_large.rs`.
`cargo clippy` is clean; `cargo publish --dry-run` packages + verify-builds;
all tests pass with and without default features. Not yet published / no GH
release workflow (those were intentionally deferred).

### 2026-05-29 — Fix `_DIC` rebuild order (in-game-validated bug)
`BntxFile::rebuild_dict` built the dictionary trie in **string-pool
order**, but `_DIC` is a *parallel array* to the BRTI/texture array: a
name lookup resolves to a node **index** and the loader reads
`texture[index - 1]`. Real Smash `__Combined.bntx` stores `_STR` in a
different order than the BRTI array, so every appended layout resolved
**206/207 existing names to the WRONG texture** — scrambling unrelated
HUD textures in-game (timer / percent / name-plate / portrait corrupted)
while the appended texture (last in both pools) looked fine. Found via
Switch/emulator testing on the SGPO project after a static byte-audit had
(wrongly) cleared the append. Fix: iterate `self.textures` (BRTI order)
in `rebuild_dict`; rebuilding the stock dict now reproduces Nintendo's
`_DIC` **byte-for-byte (0/207 entry mismatches)**, and the regenerated
227-texture SGPO layout is 227/227 parallel with stock texture bytes
intact. Added `tests/bntx_dict_parallel_order.rs` pinning the
node-order == texture-order invariant — the round-trip path never
exercised it (it emits the dict verbatim) and `bntx-dict-test` only
checked name→string_index resolution, which holds regardless of node
order. `remove_texture` shares `rebuild_dict`, so it's fixed too. All
38 tests pass + the new regression test.

### 2026-05-28 — agent-fixtures expansion (commit 0208194)
Round-trip coverage: 25 → 508 BFLYTs (4 game archives + 3 community
mods, including 28 HDR layout archives). Fixed name-slot off-by-one,
scr1/ali1/spi1 opaque preservation, pan1/bnd1/pic1 trailing-bytes
capture, malformed-mat1 defensive shrink + `flags_untrusted` flag,
filename_offset C-string semantics, principled canonical-RLT
regeneration, multi-mip + cube-map BNTX append support, hex parsing
for `--align`. Added 11 tests (synthesis 2, dict edge 10 — wait, 10 dict
edge — and bntx_real_fixtures 1, bflyt_real_fixtures expanded to walk
fixtures recursively). All 5 originally-Nintendo-produced BNTXs and
508 BFLYTs round-trip byte-identically.

### 2026-05-28 — `bntx-replace-png` verb (TODO #1)
Added `src/verbs/bntx_replace_png.rs` and wired into the dispatcher.
The verb re-encodes a PNG (or 6-face cube source) to BC7+Tegra-swizzled
bytes and splices them over an existing texture's BRTD slot, leaving
the BNTX structure (string pool, dict, BRTI count, RLT) untouched —
`relocation_table_dirty` stays `false`, so the original `_RLT` is
emitted verbatim and the round-trip stays byte-identical for the
unchanged regions. Validates dimensions, mip count, cube-vs-2D, BC7
family, and swizzled byte length up-front so a mismatched source aborts
cleanly without partial mutation. sRGB-ness is preserved from the
existing texture (no accidental gamma flip). Added
`tests/bntx_replace_in_place.rs` with two tests: a same-size splice
that verifies file-size preservation + other textures untouched, and
an identity-splice that proves writing back the existing bytes yields
a byte-identical file (no implicit re-canonicalization). All 16 tests
pass; manual verification on `info_melee_original__Combined.bntx`
shows replaced file still passes `bntx-roundtrip-test`.

### 2026-05-28 — `bntx-remove-texture` verb (TODO #2)
Added `BntxFile::remove_texture(&mut self, name: &str)` library method
that drops a texture's BRTI, removes its name from the string pool
(and decrements `name_string_index` for any texture whose string sat
after it), rebuilds BRTD by laying out the remaining textures'
pixel-data slices back-to-back with each one's own alignment, rebuilds
the dict trie, and marks the RLT dirty so the writer regenerates a
canonical layout. Mirror-symmetric to `append_texture` (both grow/
shrink in the same way; the BRTD compaction path matches the append's
padding rule). Refuses to remove string-pool slots 0/1 (empty
sentinel / container name). Added `src/verbs/bntx_remove_texture.rs`
as the CLI surface. Added `tests/bntx_remove_texture.rs` with 5 tests
covering remove-first / remove-middle / remove-last (each verifying
all OTHER textures' pixel bytes + metadata are preserved through the
write→re-read cycle), missing-name error handling, and a
remove + re-append round-trip. All 21 tests pass; manual verification
shows file shrinks by exactly the freed BRTI block + BRTD slot + name
entry, and `bntx-roundtrip-test` succeeds against the post-remove
output. The chain `remove → import-png` (with the same name) also
round-trips cleanly.

### 2026-05-28 — texpipe round-trip test (TODO #3)
Added `tests/texpipe_round_trip.rs` and a new `texture2ddecoder = "0.1"`
dev-dependency (pure-Rust, MIT/Apache, no GPL). Walks every
`tests/fixtures/png-test-images/rgba_alpha_*.png` and round-trips it
through PNG → `compress_image_bc7` → `tegra_swizzle::deswizzle_surface`
→ `texture2ddecoder::decode_bc7` → BGRA→RGBA conversion → comparison
against the source. Bounds per-channel mean error (≤12) and peak error
(≤80) — loose enough to accommodate BC7's intrinsic lossy quantization
at the `Fast` quality preset, but tight enough that any axis
transposition, BGRA↔RGBA flip, or `block_height_log2` mismatch will
fail the budget by orders of magnitude (those produce mean errors
>100 on natural images). All 7 fixtures (32², 64², 100², 128×64, 256²,
512², 1024²) pass on first run. Total tests now 22 across 8 binaries.

### 2026-05-28 — `flags_untrusted` guardrail (TODO #4)
Closed the latent footgun where mutating an untrusted-mat1 material's
sub-section counts would silently emit a corrupt BFLYT (`flags_raw`
disagreeing with section bytes). Three layers of defense:
- `Material::assert_flags_trusted(&self) -> Result<(), BflytError>` —
  opt-in caller-side guard that fails on still-untrusted materials.
- `Material::clear_untrusted_flag(&mut self)` — explicit "I've
  reconciled the sub-sections, trust the in-memory state" reset that
  recomputes `flags_raw` from current counts and drops the
  `original_section_size` snapshot.
- Writer `debug_assert!` — captures `original_section_size: Option<u32>`
  at read time and verifies `Material::emit_size()` still matches when
  the writer takes the verbatim-flags_raw path. Dev builds panic
  loudly when a caller mutated counts without first clearing the flag;
  release builds fall back to the explicit `assert_flags_trusted`
  guard. Existing verbs (`mat-rename`, `bflyt-add-material`) only
  mutate values, never counts, so they remain safe even on cloned
  untrusted templates. Added `tests/bflyt_flags_untrusted.rs` with 6
  tests covering all three layers (assertion ok/err, clear-and-go,
  benign untrusted write, the dev-mode `should_panic` misuse case,
  and the mutate→clear→write recovery path). All 508 BFLYT fixtures
  still round-trip byte-identically; total tests now 28 across 9
  binaries.

### 2026-05-28 — focused prt1/wnd1 round-trip tests (TODO #5)
Added `tests/bflyt_prt1_wnd1_round_trip.rs`. Three tests:
- discover + round-trip the most-complex `wnd1` (highest
  `frame_count * 100 + tex_coord_count * 10` score) in the fixture
  corpus, plus pane-internal field comparison through a parse → write
  → parse cycle so a regression in any wnd1 sub-field (frames,
  tex_coords, stretch, frame_size) lights up directly;
- same shape for `prt1` (`property_count * 1M + raw_property_data.len`
  score), with `PartsProperty` field-by-field comparison and exact
  `raw_property_data` preservation;
- coverage assertion that the fixture set contains non-trivial
  examples of each (otherwise the targeted tests would be
  silently empty).
Discovery on the current corpus picks the 4-frame `btn_bg` wnd1 in
training-modpack's `info_training_btn0_00_item.bflyt` and the
20-property `set_parts_btn_eshop` prt1 (with 2324 bytes of
`raw_property_data`) in HDR's `main_menu.bflyt`. Coverage tally:
651 non-trivial wnd1 panes + 1300 non-trivial prt1 panes across 508
fixtures. Total tests now 31 across 9 binaries (debug; one test
debug-only).

### 2026-05-28 — BNTX dict stress tests (TODO #6)
Added `tests/bntx_dict_stress.rs` with 4 tests pushing the `_DIC`
Patricia-trie builder beyond any real-world BNTX size: three at
N=10,000 (sequential hex, heavy shared prefix, long shared prefix +
short unique suffix) and one at N=25,000 with a soft 30s budget as a
catastrophic-regression guard. All four exercise `Trie::insert` +
full lookup-sweep verification (every inserted name must resolve to
its inserted `string_index`) and print insert / lookup timings via
`println!` — visible with `cargo test -- --nocapture`. Current
numbers on dev hardware: ~3-10 ms total for 10-25k insertions, ~100
ns/lookup average. Confirms the Patricia trie has plenty of headroom
beyond the largest community-mod BNTXs we've seen (HDR's ~2k textures).
Total tests now 35 across 10 binaries (debug).

### 2026-05-28 — texpipe cube-map / multi-mip tests (TODO #7)
Added `tests/texpipe_cube_and_mip.rs` to close the previously CLI-only
coverage gap on `compress_image_bc7_with_mips` and `compress_cube_bc7`.
Three tests, all using the 64×64 rgba_alpha PNG fixture: a 4-mip 2D
encode → swizzle → deswizzle → decode mip 0 round-trip; a 6-face
cube with 1 mip per face decoding all 6 faces; and a 6-face × 3-mip
cube decoding face 0 mip 0 + face 5 mip 0 (different face indices,
same level — proves the per-face stride computation is correct).
Each test verifies `linear_size` matches the per-mip BC7 byte-count
sum (catches off-by-one in the texpipe's mip-chain build) before
deswizzling, and applies the same per-channel error budget the
single-mip test uses (mean ≤12, peak ≤80). Higher mips are not
asserted against pixel values because the texpipe runs Lanczos3
before encoding and we don't want to mirror that in-test, but the
fact that those bytes deswizzle to the expected size + decode without
error is a strong layout signal. Final state: 38 tests across 11
binaries (debug); all 7 handoff TODOs resolved.