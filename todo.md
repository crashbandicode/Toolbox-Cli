# Toolbox-Cli — follow-up TODO

Backlog captured 2026-05-29 after the 7-item handoff + corner-case review.
Ordered roughly by value. Items marked done were completed in the same
session immediately after this file was written.

## In progress / next

- [x] **BFLYT advanced pane mutations.** Roadmap #3 (done, committed a50ae76 +
  d75960a). `src/bflyt/ops.rs`: `remove_pane` (drop subtree + scrub groups),
  `move_pane` (reparent, cycle-guarded), `rename_pane` (+ group-ref update),
  `copy_subtree` (deep copy with suffixed unique names), `set_window` (wnd1
  stretch / frame-size borders), `set_text`/`pane_text` (txt1 single-string
  layout, UTF-16LE; rejects text-id / per-char-transform / line-width panes).
  Verbs `pane-remove` / `pane-move` / `pane-rename` / `pane-copy` /
  `bflyt-set-window` / `bflyt-set-text`. 14 fixture-free unit tests + the
  fixture-gated `tests/bflyt_pane_ops.rs`.
- [x] **BFLYT prune + repair.** Roadmap #4 (done, committed 582e493).
  `src/bflyt/repair.rs`: `prune_unused_materials` / `prune_unused_textures`
  (index remap), `fix_dangling_texture_refs` (clamp into range),
  `dedupe_pane_names`, `repair(prune_materials)` → `RepairReport`. Verbs
  `bflyt-prune` (`--materials` / `--textures` / `--force`) + `bflyt-repair`
  (`--prune-materials` / `--dry-run`). Material pruning is skipped (and flagged)
  when prt1 property data is present (opaque refs). 8 fixture-free unit tests +
  a fixture-gated repair round-trip.
  *Follow-up (not done):* a layout.arc-level `layout-repair` wrapper that
  repairs every BFLYT inside a packed archive; `layout-diff` comparing
  `wnd1`/`prt1` material bindings.
- [x] **BYML `byml-set` (scalar mutation-by-path).** Done (committed). New
  `src/byml/edit.rs`: `set_by_path(root, path, raw, ty)` edits one scalar leaf
  by a `byml-diff`-style path (`/RecipeList/0/ResultActorName`), type-preserving
  by default or `--type` (`bool`/`s32`/`u32`/`f32`/`s64`/`u64`/`f64`/`string`/
  `null`) override / `null` promotion; refuses to clobber containers/binary or
  descend through scalars (`u32`/`u64` accept `0x` hex). Then
  `write_byml_canonical` (semantically lossless). `ScalarType` + `SetReport` +
  8 `BymlError` edit variants, exported from `byml` + prelude. Verb `byml-set`
  (`-i`/`-o`/`--path`/`--value`/`--type`, inflates `.byml.zs`, writes
  uncompressed). Tests: 11 fixture-free unit + `tests/byml_set.rs` (3
  fixture-gated — a real `CookingTable` edit = exactly one structural diff).
  *Follow-up (not done):* **add/remove-by-path** (create a new hash key / append
  an array element / delete a node).
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
  *Follow-up:* scalar `byml-set` (mutation-by-path → `write_byml_canonical`) is
  **done** (see the dedicated item above); add/remove-by-path is still pending.
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
- [x] **AAMP (binary parameter archive) — DONE.** BOTW was dumped, so AAMP is
  implemented (committed `ef11b4c` / `6ad1ee6` / `cf9f257`). New `src/aamp/`
  (`mod`/`read`/`write`/`edit`/`error`, typed `AampError`): offset-driven v2
  parser (root Parameter IO → list 0xC / object 0x8 / parameter 0x8, all 21
  `ParameterType`s, CRC-32 keys); `write_aamp` verbatim **byte-identical** (418
  real BOTW files); from-scratch `write_aamp_canonical` (sections header →
  lists [BFS] → objects → params → data → dedup'd strings, 4-aligned; semantic
  round-trip, lossless on all 418); `set_by_path` type-preserving scalar
  mutation by a `/<lists…>/<object>/<param>` path (CRC-32-matched; `0x…` = raw
  hash). Verbs `aamp-inspect` (`--json` / `--names`), `aamp-roundtrip-test`
  (`--canonical`), `aamp-set`. Fixtures (32 files / 18 extensions, from a BOTW
  dump) under the gitignored `tests/fixtures/aamp/`.
  *Follow-up (not done):* a default BOTW name table for readable inspect (CRC-32
  keys shown as hex unless `--names` given); decode curve control points (kept
  as raw bytes today); AAMP add/remove params (only scalar set so far).
- [x] **BFRES (FRES) inspect-only — DONE.** Roadmap #2. New `src/bfres/`
  (`mod`/`read`/`write`/`error`, typed `BfresError`): header decode (magic /
  version / BOM-endianness / embedded file name / file size / relocation-table
  offset), consistent across **BOTW v5 `0x00050003`** + **TotK v10 `0x000A0000`**
  (both LE), plus a structural scan of the well-known sub-block magics
  (FMDL/FSKA/FMAA/FSHP/FMAT/FVTX/FSKL/BNTX/`_STR`/`_DIC`/`_RLT`). `write_bfres`
  re-emits captured bytes → **byte-identical** (inspect-only parser; offsets
  not rebuilt). Verified across **424** real files (BOTW `.sbfres`, TotK
  `.bfres.zs`, decompressed v10 models), 0 errors. `bfres-inspect` surfaces a
  BOTW `.Tex.bfres`'s embedded BNTX via `read_bntx` (bytes bounded by the BNTX's
  own `file_size`). Verbs `bfres-inspect` (`--json`) + `bfres-roundtrip-test`.
  Tests: `tests/bfres_roundtrip.rs` (fixture-gated) + 3 fixture-free
  `bfres::read` unit tests. Fixtures gitignored under `tests/fixtures/bfres/`.
  *Follow-up (not done):* decode the model/animation sub-resources
  (FMDL/FSKA/vertex/material/shape) beyond the magic scan.
- [ ] **TotK MeshCodec `.mc` decompression (deep RE; user-approved).** TotK
  models ship as `Model/*.bfres.mc` = MeshCodec (`MCPK`): magicless zstd needing
  a **raw-content dictionary embedded in `exefs/main`** (the NSO; *not* in
  RomFS — confirmed no dict magic / symbol / string blob; dictless decode
  fails). Custom out-of-band framing; the `FMSH` sub-section is
  community-unsolved (even reference tools emit **partial, non-editable** BFRES);
  the only complete reference is GPL. Plan: **(1) DONE** — ported **NSO0 + LZ4**
  decompress to Rust (`src/nso.rs`, MIT `lz4_flex`; verb `nso-extract`),
  byte-exact vs the Python-lz4 oracle on `main`'s text/rodata/data; the
  `MeshCodec` strings are in rodata (`0x56b44`/`0x9130c`/`0x91338`/`0x9ae345`).
  (2) disassemble `.text` around those xrefs to locate the raw dict pointer/size
  + the frame params (window log); (3) implement a magicless-zstd(+dict) decode
  in `compression` and validate **byte-exact against the 12,395-file decompressed
  oracle** the user produced (`local-assets/mesh-codec-output/`, gitignored).
  Until then, BFRES consumes already-decompressed `.mc` output (all v10; parse +
  round-trip verified). The raw dict is the user's own extracted game data —
  load it at runtime (`--mc-dict`), **never commit it**.

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
