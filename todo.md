# Toolbox-Cli — follow-up TODO

Backlog captured 2026-05-29 after the 7-item handoff + corner-case review.
Ordered roughly by value. Items marked done were completed in the same
session immediately after this file was written.

## In progress / next

- [x] **Doc scan/refresh** — README, `lib.rs` rustdoc, AGENTSSUMMARY
  brought up to date with the current verb/format/test set.
- [x] **BFLYT cross-game robustness** — unknown sections no longer fatal;
  in-tree unknown/`scr1`/`ali1`/`spi1` → `PaneKind::Opaque` pane nodes;
  post-tree `usd1`→trailing. TotK Boot/Common/Title BFLYT now 373/373
  byte-identical; Smash still 508/508.
- [ ] **BNTX version `0x00040100` + ASTC/low-bpp formats** (TotK). The
  other half of the robustness pass; gated behind whether TotK *texture*
  editing matters. ASTC4x4 + R8/R8G8 + the `0x0c01` family seen in the
  audit.
- [x] **Compression module (zstd+dict, yaz0)** + recursive archive
  (`.blarc.zs`/`.pack.zs`/`.szs`) so TotK assets open in-tool. Done:
  native SARC reader (dropped the `sarc` crate, which pinned an ancient C
  libzstd 1.4.4); `zstd 0.13` (vendored libzstd, BSD); `compression::{mod,
  zstd,yaz0,dict}` (pure-Rust Yaz0 + frame-header dict-id parser); verbs
  `decompress`/`compress`/`archive-extract`; compression-aware
  `layout-audit --dict/--romfs`. Decode is byte-identical to Python 3.14
  `compression.zstd` on real `Boot.blarc.zs` (id 1) and `…pack.zs` (id 3);
  `decompress(compress(x)) == x` for zstd+dict and Yaz0. Local fixtures:
  `tests/fixtures/totk/compression/` (gitignored).
- [ ] **Make SARC a crate-ready module** (dedicated follow-up to the native
  reader added above; do *after* the compression batch is green+committed).
  Promote `src/sarc.rs` → `src/sarc/` (`mod.rs`/`read.rs`/`write.rs`/
  `error.rs`); add a typed `SarcError` to match `BflytError`/`BntxError`;
  keep the codec core **zero-dependency** (std only) and isolate the
  `walkdir`/`std::fs` helpers behind a clear boundary (future optional `fs`
  feature) so it can be extracted as a standalone `nx-sarc` crate later.
  Add comprehensive **original** tests (LE+BE round-trip, hash-only entries,
  alignment derivation incl. BNTX/BNSH→0x1000 & nested→0x2000, subdir names,
  4-byte name padding, hash-order stability/collisions, empty/single/large
  >1000 entries, malformed inputs: bad magic/truncation/OOB ranges/missing
  SFNT/non-UTF8 name, property round-trip). Tests authored from the public
  spec + our round-trip discipline; edge-case checklist informed by the MIT
  `jam1garner/sarc` crate (credit in a comment) — **no** verbatim copying,
  **no** GPL/Switch-Toolbox, **no** committed game fixtures.
- [x] **Adopt TotK fixtures** (bflyt/bflan) + `bflan-roundtrip-test` verb.
  Local gates now cover Smash + TotK: 881 BFLYT, 7616 BFLAN — all
  byte-identical. (Fixtures gitignored under `tests/fixtures/totk/`.)
- [x] **Custom SARC writer with per-file alignment.** Replace the `sarc`
  crate's writer (which pads every entry to `0x2000`, bloating
  `info_melee` 2.1 MB → 4.7 MB). Derive each file's alignment from
  content via the `nn::util::BinaryFileHeader` convention (BOM at `0x0C`
  → `1 << byte[0x0E]`; verified: BNTX & BNSH want `0x1000`, FLYT/FLAN/
  info want the minimum). Route `write_arc` + `pack_directory` through
  it. Bonus: correctly preserves multiple hash-only (unnamed) entries
  that the `sarc` crate writer collapsed.

## Hardening (small, no new fixtures needed)

- [ ] **`--channel-swizzle` flag for `bntx-import-dds`.** DDS carries no
  channel-swizzle; new imports currently default to identity
  (`R,G,B,A`). Let callers set e.g. `One,One,One,Red` for a BC4 alpha
  mask so an imported texture renders as intended in-game.
- [ ] **BGRA mask handling for legacy (non-DX10) DDS read.** Today a
  legacy uncompressed DDS is assumed RGBA; a BGRA-masked file would have
  its channels swapped. Parse `ddspf` R/G/B/A masks and reorder.
- [ ] **Support BNTX surface format `0x00000C01`.** HDR's recolored
  `info_melee` texture pack uses it (currently rejected; audit flags
  it). Identify the format (looks 16bpp, likely R5G6B5 / RGB5A1) and add
  decode (+ encode if feasible).
- [ ] **`layout-diff`: compare `wnd1`/`prt1` material bindings**, not
  just `pic1`/`txt1`; handle duplicate pane names (current name-keyed map
  collapses them).

## Coverage (needs assets / hardware)

- [ ] **Real cube / multi-mip / BC2 / BC6 / R8G8B8A8 BNTX fixtures.**
  Decode + DDS paths for these are currently covered only synthetically
  (cube/mip) or not at all (BC2/BC6/RGBA on real data). A stage/skybox
  `layout.arc` would exercise real cube + multi-mip.
- [ ] **Real SGPO end-to-end `layout-apply-arc`.** Test used a synthetic
  2-pane manifest cloning `set_rep_stock_01` under `RootPane`. Run the
  real skin (face PNGs + manifest targeting a layout with `sgpo_root`).
- [ ] **In-game / emulator validation.** Load a `layout-apply-arc`
  output and a format-preserving `bntx-replace-png` output on hardware
  to confirm rendering. Untestable in this repo.

## Infra (pre-existing backlog)

- [ ] v9 BFLYT 60-byte material extension decode (captured verbatim).
- [ ] GitHub Actions CI (`cargo build` + `cargo test --release`).
