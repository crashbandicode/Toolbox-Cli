# AGENTSSUMMARY.md

Living context document for AI agents working on Toolbox-Cli. Update the
**Session log** at the bottom whenever you finish a meaningful chunk of
work. Keep this file concise — link to commits / files instead of pasting
long output.

## Project

Pure-Rust CLI for editing Nintendo Switch UI assets used by Smash
Ultimate (and other Switch games). Produces byte-identical round-trips
of BFLYT v8/v9, BNTX, and SARC files. Used by the SGPO project to apply
custom face-button skins.

- Repo: https://github.com/crashbandicode/Toolbox-Cli
- License: MIT (no GPL deps)
- Inspired by KillzXGaming/Switch-Toolbox (GPL); no upstream code copied.

## Build & test

```bash
cargo build           # dev
cargo build --release # release (static-links Intel ISPC, ~3 min from clean)
cargo test            # all 38+ tests across 11 binaries (debug; one test is debug-only)
```

## Architecture

```
src/
├── lib.rs              Library entry point (modules below)
├── main.rs             CLI binary; thin wrapper over verbs::dispatch
├── bflyt/
│   ├── sections.rs     Type definitions
│   ├── read.rs         Parser (handles malformed mat1 via flags_untrusted)
│   └── write.rs        Writer (byte-identical round-trip)
├── bntx/
│   ├── mod.rs          BntxFile, Texture, AppendTextureSpec, append_texture
│   ├── read.rs         Full-fidelity parser (str pool, dict, RLT, BRTD)
│   ├── write.rs        Writer with canonical/preserved RLT modes
│   └── dict_builder.rs Patricia-trie builder for _DIC
├── texpipe.rs          PNG → BC7 (intel_tex_2) → Tegra swizzle (tegra_swizzle)
├── manifest.rs         SGPO skin manifest schema (serde)
└── verbs/              One file per CLI verb
```

## Round-trip status (commit 0208194)

- **BFLYT**: 508/508 byte-identical (4 game UI archives + 28 HDR mod
  archives + training-modpack + installed_sgpo_layout).
- **BNTX**: 5/6 byte-identical. The 6th
  (`sgpo_one_pane_png_proof__Combined.bntx`) is a C# Switch-Toolbox
  output with a 1040-entry verbose RLT vs Nintendo's 8-entry compact
  RLT — both are functionally valid; the test tolerates this.
- **SGPO end-to-end**: layout-apply-manifest + layout-validate-manifest
  pass 4/4 elements on a fresh `info_melee` archive.

## Tests

- `tests/bflyt_synthesis.rs` — 2 synthetic-layout round-trip tests.
- `tests/bflyt_real_fixtures.rs` — walks every `*.bflyt` under
  `tests/fixtures/` recursively (508 files in our setup).
- `tests/bntx_real_fixtures.rs` — walks `tests/fixtures/bntx/`,
  tolerates the known sgpo_one_pane_png_proof RLT diff.
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

## Conventions

- **Errors**: parse errors carry section-index, material-index, or pane
  context. Add similar context when extending. Look at how
  `read_mat1` / `read_bflyt` wrap inner errors with `map_err(context)`.
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

## TODO (live, ordered)

All seven handoff items resolved this session. Next-up candidates if
new work is requested:

- v9 BFLYT 60-byte material extension decode (currently captured
  verbatim in `Material::trailing`).
- GitHub Actions CI workflow for `cargo build` + `cargo test --release`
  (gated on whether fixtures should ship to CI).
- In-game runtime validation on Switch hardware (requires hardware).

Resolved this session:

1. ~~`bntx-replace-png` verb~~ — done (commit pending).
2. ~~`bntx-remove-texture` verb~~ — done (commit pending).
3. ~~`tests/texpipe_round_trip.rs`~~ — done (commit pending).
4. ~~`flags_untrusted` guardrail~~ — done (commit pending).
5. ~~Exhaustive `prt1`/`wnd1` round-trip tests~~ — done (commit pending).
6. ~~BNTX dict 10k-name stress test~~ — done (commit pending).
7. ~~`tests/texpipe_cube_and_mip.rs`~~ — done (commit pending).

## Session log

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