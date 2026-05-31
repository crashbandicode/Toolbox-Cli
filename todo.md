# Toolbox-Cli — follow-up TODO

Backlog captured 2026-05-29 after the 7-item handoff + corner-case review.
Ordered roughly by value. Items marked done were completed in the same
session immediately after this file was written.

## In progress / next

- [x] **MSBT inspect + round-trip + JSON export/import (Stages A+B).** Roadmap
  item (text/message format), done + committed. New `src/msbt/`
  (`mod`/`read`/`write`/`error`, typed `MsbtError`): `MsgStdBn` header (endian
  via BOM, encoding, version, section count), generic section walk with
  structural decode of `LBL1` (label→message-index hash table) and `TXT2`
  (UTF-16 messages + a tag-aware chunk decoder: `0x000E` open / `0x000F` close,
  literal `\n`/`\t` preserved); other sections retained opaque.
  **Stage A** — `write_msbt` re-emits captured bytes **verbatim** →
  byte-identical round-trip (verified on **all 1510 USen + 1510 JPja** TotK
  `Mals` `.msbt`). **Stage B** — `write_msbt_canonical` rebuilds from the
  decoded sections (re-encodes `LBL1` via the verified LMS hash
  `h=h*0x492+byte` into the original bucket count, `TXT2` from messages,
  opaque verbatim); semantically lossless (and byte-identical on all local
  fixtures). `Message::from_chunks` (inverse of the decoder) +
  `set_message_by_label` mutation. Verbs `msbt-inspect`, `msbt-roundtrip-test`,
  `msbt-export-json` (label→chunks JSON, tags as hex), `msbt-import-json`
  (overlay edits by label → canonical write). Tests: `tests/msbt_roundtrip.rs`
  (corpus verbatim + canonical semantic round-trip + pinned structure) + 8
  fixture-free unit tests; CLI export→edit→import verified end-to-end
  (byte-identical no-edit rebuild; a text edit propagates). Fixtures
  gitignored under `tests/fixtures/msbt/`.
  *Follow-up (not done):* BOTW/older MSBT versions (only TotK v3 LE/UTF-16
  fixtures locally); `ATR1`/`TSY1` structural decode (retained opaque today).
- [x] **BYAML/BYML inspect + round-trip + diff.** Roadmap item #2 (done,
  committed). New `src/byml/` (read/write/diff/mod/error): `Byml` value tree
  + `BymlDocument`, both endians + versions `1..=7`, bounds-checked +
  depth-guarded parser. **Stage A** — `write_byml` re-emits the captured bytes
  verbatim → **byte-identical round-trip** for unchanged docs; verbs
  `byml-inspect` (`--json`/`--max-depth`, inflates `.byml.zs` via
  `--dict`/`--romfs`) + `byml-roundtrip-test`. Verified on real TotK assets
  (`CookingTable.bgyml` LE, RSDB `Challenge`/`ActorInfo` LE, `GameDataList`
  **big-endian** — ~3.3M nodes, all byte-identical). **Stage B** — from-scratch
  `write_byml_canonical` (sorted/deduped tables, BFS node layout, back-patched
  offsets), **semantically lossless** across the corpus (`read(write(x)) ==
  read(x)`, both endians, ≤12.7 MB); `diff_byml` + `byml-diff` (path-keyed
  structural diff, `--json`). Tests: `tests/byml_roundtrip.rs` (+ canonical
  semantic round-trip), `tests/byml_diff.rs`, 10 fixture-free unit tests.
  Fixtures gitignored under `tests/fixtures/byml/`.
  *Follow-up (not done):* `byml-set` mutation by path (edit a leaf / add /
  remove, then `write_byml_canonical`).
- [x] **RSTB/RESTBL read + update.** Roadmap item #3 (done, committed). New
  `src/restbl.rs` (typed `RestblError`): TotK `RESTBL` v1 — 22-byte header +
  CRC table (`{hash,size}` sorted by hash) + 160-byte-name collision table
  (sorted by name). `write_restbl` is **byte-identical** (verified on the real
  379,715-entry `ResourceSizeTable.Product.121`/`.143`). Native standard
  CRC-32 (checked vs `0xCBF43926`); `get`/`set`/`insert` by hash / name /
  resource path. Verbs `restbl-inspect` (`--json`, `--lookup`/`--hash`,
  inflates `.rsizetable.zs`), `restbl-roundtrip-test`, `restbl-set`
  (`--path`/`--hash`/`--name` + `--size` + `--insert`). Tests:
  `tests/restbl_roundtrip.rs` + 6 fixture-free unit tests. Fixtures gitignored
  under `tests/fixtures/restbl/`.
  *Follow-up (not done):* BOTW `RSTB` (older magic — no version /
  `string_block_size`, 128-byte names); a `restbl-update-dir` that scans a mod
  folder and bumps every changed resource's size.
- [x] **Doc scan/refresh** — README, `lib.rs` rustdoc, AGENTSSUMMARY
  brought up to date with the current verb/format/test set.
- [x] **BFLYT cross-game robustness** — unknown sections no longer fatal;
  in-tree unknown/`scr1`/`ali1`/`spi1` → `PaneKind::Opaque` pane nodes;
  post-tree `usd1`→trailing. TotK Boot/Common/Title BFLYT now 373/373
  byte-identical; Smash still 508/508.
- [x] **BNTX version `0x00040100` + ASTC/low-bpp formats** (TotK). Done
  (uncommitted): version gate accepts `0x00040000`+`0x00040100` (identical
  container layout); added the **full ASTC LDR family** (4x4–12x12,
  UNORM+SRGB via an `AstcBlock` sub-enum), `R8`/`R8G8`, and `B8G8R8A8`
  (`0x0c01`). Byte-identical round-trip on a real 225-texture TotK
  `__Combined.bntx`; decode for ASTC (all footprints) + R8/R8G8/BGRA8
  (all 225 export to PNG). **Decode/round-trip only — no encoder** (see
  the ASTC-encode follow-up below). `layout-audit` now reports 0
  unsupported BNTX (HDR's `0x0c01` parses). Codes verified vs real data +
  public BNTX research. Fixtures: `tests/fixtures/bntx/totk_title__Combined.bntx`
  (gitignored).
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
- [x] **Make SARC a crate-ready module.** Done: `src/sarc.rs` → `src/sarc/`
  (`mod`/`read`/`write`/`error`/`fsutil`); typed `SarcError` matching
  `BflytError`/`BntxError`, wired into the crate `Error` via `#[from]`;
  std-only codec core with the `walkdir`/`std::fs` helpers isolated in
  `fsutil` (future optional `fs` feature) so it can be lifted into a
  standalone `nx-sarc` crate. 14 original unit tests (LE/BE round-trip,
  alignment derivation incl. BNTX/BNSH→0x1000 & nested→0x2000, hash-only
  ordering stability, empty/single/2000-entry, malformed inputs: bad
  magic/BOM/missing-SFAT/node-OOB, pseudo-random property round-trip);
  edge-case checklist informed by the MIT `jam1garner/sarc` crate (credited
  in a comment) — no verbatim copying, no GPL, no committed fixtures.
  *Remaining for the actual crate split (deferred):* move to its own
  `Cargo.toml`/repo, gate `fsutil` behind an `fs` feature, and consider a
  hand-rolled (thiserror-free) `Display` for a truly std-only core.
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
- [x] **Support BNTX surface format `0x00000C01`.** Done (uncommitted):
  it's **B8G8R8A8** (32bpp, identity channel-swizzle — *not* the 16bpp
  R5G6B5 guessed here), decoded by swapping R↔B back to RGBA. HDR's
  recolored `info_melee` now parses + decodes (audit reports it clean).
- [x] **Uncompressed RGBA8 PNG import** (SGPO sharper-text option). Done
  (committed, unpushed): `ImportTextureFormat { Bc7, Rgba8, Rgba8Srgb }` +
  `ImportOptions/ApplyOptions::texture_format`; `--texture-format`
  (`bc7`/`rgba8`/`rgba8-srgb` + aliases) on `bntx-import-png` /
  `layout-apply-manifest` / `layout-apply-arc`; BC7 default unchanged
  byte-for-byte. New `AppendTextureSpec::texture_2d_with_mips`. 2D only
  (cube stays BC7). `R8G8B8A8` is now fully editable (import + in-place
  replace).
- [ ] **ASTC + R8/R8G8/B8G8R8A8 *encode*.** ASTC needs a new MIT encoder
  crate (none wired — `intel_tex_2` is BCn-only). The single-channel
  `R8`/two-channel `R8G8` and the byte-swapped `B8G8R8A8` are trivial
  channel-packs but still gated off in `texpipe::format_is_encodable` /
  `encode_mip_blocks` (only `R8G8B8A8` is wired). Add the rest when TotK
  *texture editing* (vs inspect/export) needs them.
- [ ] **Confirm the non-4x4 ASTC footprints on real data.** Only
  ASTC_4x4 (SRGB) appears in our local TotK fixtures; 5x5/6x6/8x8/etc.
  decode is generic + unit-tested for codes, but unverified against real
  pixels. Grab a TotK file that uses a larger footprint and pixel-check.
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
