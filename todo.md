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
- [ ] **Compression module (zstd+dict, yaz0)** + recursive archive
  (`.blarc.zs`/`.pack.zs`/`.szs`) so TotK assets open in-tool (today they
  need external decompression — Python 3.14 `compression.zstd` works).
- [ ] **Adopt TotK fixtures** (bflyt/bflan) + `bflan-roundtrip-test` verb;
  extend the roundtrip/audit gates to Smash + TotK.
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
