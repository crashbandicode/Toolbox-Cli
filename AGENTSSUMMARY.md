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
cargo build           # dev (zstd is pure-Rust zstd-pure; libzstd/zstd-sys only builds for tests)
cargo build --release # release (static-links Intel ISPC, ~3 min from clean)
cargo test            # ~95 tests across 26 integration binaries + lib unit
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
│   ├── ops.rs          Mutation ops (clone/set/remove/move/rename/copy-subtree pane, set-text/-window, add_material…)
│   └── repair.rs       Cleanup (prune unused materials/textures, clamp dangling refs, dedupe pane names)
├── bntx/
│   ├── mod.rs          BntxFile, Texture, AppendTextureSpec, append/remove
│   ├── read.rs         Full-fidelity parser (str pool, dict, RLT, BRTD)
│   ├── write.rs        Writer with canonical/preserved RLT modes
│   ├── decode.rs       Deswizzle + decode (texture2ddecoder; BCn/ASTC/R8/R8G8/RGBA8/BGRA8) → RGBA, applies channel-swizzle
│   ├── pipeline.rs     PNG/DDS import (BC7 default or RGBA8 via ImportTextureFormat), format-preserving replace, DDS export
│   └── dict_builder.rs Patricia-trie builder for _DIC
├── bflan.rs            BFLAN parse/write (verbatim sections, byte-identical) + pat1/pai1 inspect
├── byml/               BYML (binary YAML) read + round-trip + diff + scalar edit
│   ├── mod.rs          Byml value enum, BymlDocument, node-type constants
│   ├── read.rs         Parser (both endians, v1..=7; bounds-checked, depth-guarded)
│   ├── write.rs        Verbatim writer + from-scratch canonical writer
│   ├── diff.rs         Path-keyed structural diff of two Byml trees
│   ├── edit.rs         Scalar mutation by path (set_by_path + ScalarType) for byml-set
│   └── error.rs        BymlError (offset / node-type / index / path-edit context)
├── compression/
│   ├── mod.rs          Codec detect + decompress/compress entry points (Cow passthrough)
│   ├── zstd.rs         libzstd wrapper + pure-Rust frame-header dict-id parser
│   ├── yaz0.rs         Pure-Rust Yaz0/Yaz1 decode + encode (hash-chain LZ)
│   └── dict.rs         DictRegistry: ZsDic.pack loader, id-keyed (zs=1, bcett=2, pack=3)
├── texpipe.rs          PNG → BC7/BC1/BC3/BC4/BC5 (intel_tex_2) → Tegra swizzle
├── dds.rs              DDS (DX10) read/write; DXGI↔TextureFormat; interchange
├── restbl.rs           RESTBL (Resource Size Table) read/write (byte-identical) + CRC-32 lookup/update
├── msbt/               MSBT (LibMessageStudio message) read + verbatim round-trip
│   ├── mod.rs          MsbtDocument, Section/SectionData, Label, Message + UTF-16 tag-aware chunk decoder
│   ├── read.rs         Parser (header + section walk; LBL1 hash table + TXT2 offsets, bounds-checked)
│   ├── write.rs        Verbatim writer (byte-identical round-trip)
│   └── error.rs        MsbtError (offset / section / index context)
├── aamp/               AAMP (BOTW binary parameter archive) read + round-trip + canonical + set
│   ├── mod.rs          ParameterList/Object/Parameter, Value enum, ParamType (0..20), AampDocument
│   ├── read.rs         Offset-driven parser (header → root list → list/object/param tree)
│   ├── write.rs        Verbatim writer + from-scratch canonical writer (semantically lossless)
│   ├── edit.rs         set_by_path (type-preserving scalar mutation by name/hash path)
│   └── error.rs        AampError (offset / type / edit context)
├── bfres/              BFRES (FRES, BOTW/TotK 3D-resource container) inspect + verbatim round-trip
│   ├── mod.rs          BfresDocument, DetectedBlock, version constants, embedded-BNTX accessor
│   ├── read.rs         Header decode (magic/version/BOM/name/size/RLT) + sub-block magic scan
│   ├── write.rs        Verbatim writer (byte-identical round-trip)
│   └── error.rs        BfresError (offset / magic / BOM context)
├── (zstd_pure)         → external pure-Rust `zstd-pure` crate (RFC 8478 decode + encode,
│                         dictionaries, magicless frames, block API). No libzstd at runtime;
│                         re-exported as `nx_layout_toolbox::zstd_pure`. libzstd (`zstd`) is a
│                         dev-only test oracle. (Was an in-tree decoder; lifted out + completed.)
├── nso.rs              NSO (Switch exefs/main) read + LZ4 segment inflate (read_nso, NsoError)
│                       — for inspecting executable contents (e.g. the MeshCodec dict)
├── mc/                 MC/MCPK (TotK MeshCodec) container: inspect + verbatim round-trip + extract/repack
│   ├── mod.rs          McpkHeader, McFile, header/size-descriptor decode
│   ├── read.rs         MCPK header parser (magic/flags/reserved/size descriptor)
│   ├── write.rs        Verbatim writer (byte-identical no-op round-trip)
│   ├── codec.rs        Magicless-zstd extract + repack (pure zstd-pure decode/encode; no dict)
│   ├── mesh.rs         FMSH mesh-section framing parser (has-mesh flag, header, chunk, sizes)
│   ├── geometry.rs     FMSH geometry transport: fwd/reverse readers, super-block/sub-block header, state-0 canonical-Huffman table builder, zstd-block + raw windows, first-sub-block index decode (bufA 99.3%), vertex rANS decode loop + freq reader (`0x110e7b0`) + contiguous spread (`0x110e6f8`); 4-state init + transform TODO
│   └── error.rs        McError (magic/flags/reserved/size/zstd/mesh-framing context)
├── meshopt/            Pure-Rust meshoptimizer 0.15 codec (reference + encoder; MeshCodec uses a custom entropy backend)
│   ├── mod.rs          Public encode/decode_{vertex_buffer,index_buffer,index_sequence} + decode_index_buffer_split + read_indices
│   ├── vertex.rs       Vertex codec (0xa0): byte-group planes, zigzag deltas, tail
│   ├── index.rs        Index buffer (0xe0 v0/v1, vertex/edge FIFOs) + index sequence (0xd0) + split-(code,data)-stream decode (the MeshCodec form; `_split_used` returns consumed counts for multi-sub-mesh chaining)
│   └── error.rs        MeshoptError (std + thiserror only)
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
├── corpus_audit.rs     Multi-format real-corpus confidence audit → JSON
│                       (magic-dispatch + safest op per format; recurses SARC)
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
  total, all byte-identical**, zero unknown-type/truncation errors. A
  from-scratch **canonical writer** (`write_byml_canonical`, for mutated /
  synthesized trees) is **semantically lossless** across the whole corpus
  (`read(write(x)) == read(x)`, both endians, up to 12.7 MB / 1.4M nodes;
  it even reproduces the exact byte length on several files, though
  byte-identity is not its contract). `diff_byml` / `byml-diff` give a
  path-keyed structural diff (matching hashes by key, arrays by index) — on
  real `ActorInfo.121`→`.143` it surfaces +8212/−8249/~79401 changes
  (added actors, heap-size tweaks, an f32 precision shift). `set_by_path`
  (`byml-set`) edits a scalar leaf addressed by a diff-style path
  (`/RecipeList/0/ResultActorName`) — type-preserving by default, `--type` to
  override the kind or promote a `null` — then canonical-writes; verified on
  real `CookingTable` (a one-leaf edit yields **exactly one** structural diff,
  the rest of the tree untouched).
- **RESTBL** (Resource Size Table): TotK `RESTBL` v1 (`System/Resource/
  ResourceSizeTable.Product.NNN.rsizetable.zs`). A fixed deterministic layout
  (22-byte header + CRC table `{hash,size}` sorted by hash + a 160-byte-name
  collision/overflow table sorted by name), so `write_restbl` is
  **byte-identical** — verified on the real 379,715-entry / 3.04 MB tables
  (both 1.2.1 and 1.4.3). Standard CRC-32 (verified against the `0xCBF43926`
  check value); `get`/`set`/`insert` by hash / name / resource path (binary
  search), so a mod can update a resource's reserved size (`restbl-set`) —
  the thing needed to repack without crashing. BOTW's older `RSTB` magic
  (128-byte names, no version field) is a follow-up (no fixtures locally).
- **MSBT** (LibMessageStudio message): TotK `Mals/*.sarc.zs` ships a SARC of
  `.msbt` text files (`MsgStdBn`; LE / UTF-16 / v3 / `LBL1`+`TXT2`). The reader
  decodes the header, the `LBL1` label→message-index hash table, and the
  `TXT2` UTF-16 messages (a tag-aware chunk decoder splits literal text from
  the `0x000E` open / `0x000F` close control tags; `\n`/`\t` stay literal);
  other section magics (`ATR1`/`NLI1`/`TSY1`/…) are retained opaque. `write_msbt`
  re-emits the bytes captured at parse time, so an unmodified file is
  **byte-identical** by construction — verified on **all 1510 USen + 1510
  JPja** `Mals` `.msbt` (every one byte-identical, zero parse errors), so the
  parser provably walks every section/label/message. A from-scratch
  `write_msbt_canonical` rebuilds an edited document (re-encodes `LBL1` via the
  verified LMS hash `h=h*0x492+byte` into the original bucket count, `TXT2` from
  the messages, opaque sections verbatim); it is **semantically lossless** and
  in fact byte-identical on every local fixture. `msbt-export-json` /
  `msbt-import-json` give a translation-editing workflow (label→message JSON,
  control tags preserved as hex; import overlays edits by label then
  canonical-writes) — verified end-to-end (byte-identical no-edit rebuild; an
  edit propagates).
- **AAMP** (binary resource parameter archive): BOTW's `agl::utl::Parameter`
  container (`.bxml`/`.bgparamlist`/`.baiprog`/`.bphysics`/…), packed inside
  `Actor/Pack/*.sbactorpack` (Yaz0 SARC). TotK replaced AAMP with BYML, so
  fixtures come from a **BOTW** dump. The reader decodes the v2 header (LE /
  UTF-8) and the offset-driven tree (root Parameter IO → lists → objects →
  parameters), all 21 `ParameterType`s (keys kept as CRC-32 hashes).
  `write_aamp` re-emits the captured bytes → **byte-identical** for an
  unmodified file — verified on **418** real BOTW AAMP files (Link / Guardian /
  Lizalfos / Gerudo, weapons / armor / animals / objects / treasure / items),
  every one byte-identical, zero parse errors. `write_aamp_canonical` rebuilds
  from the decoded tree (sections header → lists [BFS] → objects → params →
  data → de-duplicated strings, 4-aligned because offsets are stored `/4`);
  **semantically lossless** across all 418 files (re-parses to the same tree;
  not byte-identical by contract). `set_by_path` (`aamp-set`) edits a parameter
  by a `/<lists…>/<object>/<param>` name path (CRC-32-matched; `0x…` = raw
  hash), type-preserving, then canonical-writes. Verbs `aamp-inspect`
  (`--json`, `--names` to resolve hashes), `aamp-roundtrip-test` (`--canonical`),
  `aamp-set`. *Follow-ups:* a name table for readable inspect by default;
  decoding curve control points (kept as raw bytes today); add/remove params.
- **BFRES** (`FRES`, Binary caFe RESource): Nintendo's 3D-resource container —
  models (`FMDL`), skeletal/material/visibility/scene animations, embedded
  textures (a `BNTX` block), the shared `_STR`/`_DIC`/`_RLT` tables. BOTW ships
  it as `Model/*.sbfres` (Yaz0, **v `0x00050003`**); TotK as `Model/*.bfres.zs`
  (plain zstd, **v `0x000A0000`**) + `Model/*.bfres.mc` (MeshCodec — see the gap
  table). Both little-endian on Switch (BOM-detected). The reader decodes the
  header (magic, version, endianness, embedded file name, file size,
  relocation-table offset) and structurally scans the well-known sub-block
  magics; like BNTX/AAMP, the byte layout is offset/relocation-heavy so the
  parser is **inspect-only** and `write_bfres` re-emits the captured bytes →
  **byte-identical** for an unmodified document. Verified across **424** real
  files (BOTW v5 `.sbfres`, TotK v10 `.bfres.zs`, and the decompressed v10 model
  corpus), 0 parse errors. A BOTW `.Tex.bfres` embeds a full BNTX; `bfres-inspect`
  surfaces it via the existing BNTX reader (bytes bounded by the BNTX's own
  `file_size`) — e.g. `Animal_Bass.Tex` reports its 8 textures (`Bass_Alb`
  BC1_UNORM_SRGB 128×128 mips=8, …). Verbs `bfres-inspect` (`--json`) +
  `bfres-roundtrip-test`. *Follow-ups:* decode the model/animation sub-resources
  (FMDL/FSKA/…) beyond the structural scan; MeshCodec `.mc` decompression (a
  dedicated RE effort — see the gap table).
- **Compression**: zstd decode is **byte-identical to Python 3.14's
  `compression.zstd`** on real TotK `Boot.blarc.zs` (id-1 dict) and
  `AI.Global…pack.zs` (id-3 dict, 3.9 MB → 25 MB). Containers can't be
  re-encoded byte-identically (different encoder), so the discipline here
  is `decompress(compress(x)) == x` (verified for zstd+dict and Yaz0) plus
  byte-identical **inner-format** round-trips after inflation.
- **SGPO end-to-end**: layout-apply-manifest / -arc + validate pass 4/4
  elements on a fresh `info_melee` archive.

## Tests

- **Library unit tests** (`cargo test --lib`, 133): the `verbs`
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
  rejection;   the **canonical writer** round-trips a tree of every node kind
  in both endians + rejects a scalar root; the **diff** detects
  add/remove/change + nested-path / type changes; **`set_by_path`** sets a
  nested hash / array leaf type-preserving, via a `--type` override, and from
  hex `u32`, and rejects every bad path/value (unknown key, index out of range,
  descend-through-scalar, container target, unparseable value/type)), and
  `restbl` (CRC-32
  `0xCBF43926` check value; build → byte-identical write → read; get/set/
  insert by hash / name / path keeps the tables sorted), and `msbt` (a
  hand-built minimal LE/UTF-16 file with a 2-label `LBL1` + 2-message `TXT2`
  — one message carrying a `0x000E` tag — decodes to the right
  labels/messages/chunks and writes back verbatim; bad-magic / bad-BOM /
  too-small / section-overrun rejection; the **canonical writer** semantic
  round-trips the minimal file, `from_chunks` inverts the decoder, and
  `set_message_by_label` edits a message then canonical-writes a re-readable
  file). These run with no
  fixtures — the format-code tests are the
  correctness net for the ASTC family / BYML reader we can't fully
  fixture-cover on CI. Plus `bflyt::ops` (pane remove/move/rename/
  copy-subtree with cycle + collision guards, a mutate→write→read structural
  round-trip, set-text read-back + write/read, set-window border edits) and
  `bflyt::repair` (material/texture prune index-remap, duplicate-name dedupe,
  dangling-ref clamp, full `repair()`, prt1-skip). Plus `aamp::read` (hand-built
  minimal AAMP decode + verbatim round-trip; bad-magic/version/too-small
  rejection), `aamp::write` (canonical semantic round-trip of a nested doc), and
  `aamp::edit` (scalar/string/color set, nested-list descent, hex-hash segments,
  every error path). Plus `bfres::read` (hand-built minimal `FRES` header decode
  + verbatim round-trip; big-endian BOM detection; bad-magic/BOM/too-small
  rejection; structural block scan). Plus `nso::read` (NSO0 header parse with
  uncompressed segments; an LZ4-compressed segment round-trips via `lz4_flex`;
  bad-magic / too-small / segment-overrun rejection).
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
  both endians + compressed/uncompressed. A second test runs
  `write_byml_canonical` over the whole corpus and asserts the result
  re-parses to the same tree (semantic round-trip, both endians, ≤12.7 MB).
- `tests/byml_diff.rs` — fixture-gated. Self-diff of a real file is empty; a
  mutated clone of `CookingTable` (add a root key, change one nested string,
  remove one nested key) yields exactly those three diff entries at the
  expected paths; and (when both present) `ActorInfo.121`↔`.143` differ with
  a mirror-image reverse diff.
- `tests/byml_set.rs` — fixture-gated. Edits a real `CookingTable` scalar via
  `set_by_path` (a type-preserving string, a type-preserving numeric, and a
  `--type` override string→u32), canonical-writes, re-reads, and asserts the
  result differs from the original by **exactly one** structural diff at the
  targeted path — the core safety property that a single edit doesn't perturb
  the rest of the tree.
- `tests/msbt_roundtrip.rs` — fixture-gated (skips without
  `tests/fixtures/msbt/`). Reads every `.msbt`, decodes each message's chunks,
  and asserts `write_msbt` reproduces the input **byte-identically**; pins the
  structure of `Info_BuildHouse` (4 labels / 4 messages, `Name` →
  "Home on Arrange") and `Npc` (labels == messages, every label index
  resolves). Locally the corpus is the 4 sampled USen files; the full 1510
  USen + 1510 JPja corpus was round-tripped via `msbt-roundtrip-test` (all
  byte-identical). A   `canonical_writer_semantic_round_trips_corpus` test runs
  `read → write_msbt_canonical → read` over the fixtures (labels/messages/
  entries preserved; also reports the byte-identical count — 4/4 locally).
- `tests/aamp_roundtrip.rs` — fixture-gated (skips without
  `tests/fixtures/aamp/`, 32 BOTW files across 18 extensions). Asserts every
  fixture `write_aamp`-round-trips **byte-identically**; a
  `canonical_writer_semantic_round_trips_corpus` test runs `read →
  write_aamp_canonical → read` (same tree); pins `Weapon_Sword_001`
  `.bxml`/`.bphysics` counts `(1,3,38)`/`(14,23,231)`; and edits a real Int
  param via `set_by_path` (hex-hash path) → canonical-write → re-read. The full
  418-file BOTW sweep (verbatim + `--canonical`) was run via
  `aamp-roundtrip-test`.
- `tests/bfres_roundtrip.rs` — fixture-gated (skips without
  `tests/fixtures/bfres/`, gitignored game data: decompressed BOTW v5 `.sbfres`
  + TotK v10 models/animations). Asserts every `.bfres` fixture `write_bfres`-
  round-trips **byte-identically** and that the corpus spans both games (a v5 +
  a v10); a `surfaces_embedded_bntx` test parses each `.Tex.bfres`'s embedded
  BNTX via `read_bntx` (every texture name resolves, dims > 0); and pins
  `Animal_Bass` (v5, `FMDL`×2) + `Animal_Bass.Tex` (8 embedded textures). The
  full 424-file sweep (BOTW `.sbfres` + TotK `.bfres.zs` + decompressed v10
  models) was run via `bfres-roundtrip-test` (0 parse errors).
- `tests/restbl_roundtrip.rs` — fixture-gated. Inflates the real TotK
  `ResourceSizeTable.Product.{121,143}.…rsizetable.zs` and asserts each
  re-serializes **byte-identically** with sorted CRC + name tables; pins the
  1.2.1 counts (379,715 CRC + 32 name) and known lookups
  (`MainField_U_30_50.bkres` = 64416 via the collision table); and inserts a
  new resource into the real table, checking the +8-byte growth, sortedness,
  and a write→read resolve.
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
- `tests/bflyt_pane_ops.rs` — fixture-gated real-bytes guard for the BFLYT
  editing ops: picks a leaf pane in a real layout and rename/copy/remove +
  write + re-parse it; repairs a real BFLYT and asserts every material→texture
  ref is in range with no duplicate pane names; and edits the first real
  simple `txt1` pane's text and reads it back. (The exhaustive logic lives in
  the `bflyt::ops` / `bflyt::repair` unit tests.)
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
| BYML canonical (from-scratch) writer | Resolved | `write_byml_canonical` emits sorted/deduped string tables + BFS node layout with back-patched offsets; **semantically lossless** on the real corpus (both endians, ≤12.7 MB). Not byte-identical to Nintendo by contract (writer-specific layout), though it matches several files' exact size. `byml-diff` + `byml-set` (scalar mutation-by-path → canonical write) added. Add/remove-by-path remains a follow-up. |
| BOTW `RSTB` (older magic) | Not implemented | Only TotK `RESTBL` v1 is implemented (read/write/update byte-identical, real fixtures). BOTW's `RSTB` header differs (no version / `string_block_size`; 128-byte names) — a follow-up when BOTW fixtures are available. |
| BFRES model/animation sub-resources | Inspect-only | `read_bfres` decodes the header + scans sub-block magics; FMDL/FSKA/vertex/material payloads aren't decoded. Verbatim round-trip is byte-identical; full decode is a follow-up. |
| TotK MeshCodec `.mc` (`MCPK`) — BFRES structure | Resolved | A model `.mc` = `[BFRES frame: magicless zstd, NO dict] + [mesh vertex/index buffers: a CUSTOM MeshCodec encoding, NOT zstd]`. `mc-extract` decodes the first frame = the BFRES **structure** (FMDL/FSKL/FVTX-defs/FSHP/FMAT/_STR/_RLT; complete + valid BFRES, ~17 KB for Bear), **byte-identical to the reference decompressor's BFRES portion** (3 real `.mc` fixtures vs oracle + 12,395-payload round-trip cross-checked against libzstd). Decoded with the pure-Rust `zstd-pure` codec (**no libzstd at runtime**); the decoder ignores the frames' advisory dict-id when no dictionary is supplied. `src/mc/`. |
| TotK MeshCodec `.mc` — mesh geometry decode/encode | Not implemented | The trailing vertex/index buffers use a **custom MeshCodec codec** (the state machine at `main` `0x6c6da0`/`0x5ffb90`; not zstd) — the genuinely hard, community-decodes-only-via-game-code part. `mc-extract` does NOT decode it (so the extracted BFRES has structure, not geometry). `mc-repack` **preserves the original mesh tail verbatim** → edited structure + original geometry; same-BFRES-size edits only (`--allow-resize` to force); geometry editing unsupported. *Untestable here:* in-game acceptance (no hardware). |
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
current work. Status snapshot: `origin/main` was at **c08f765** (AAMP docs) at
the start of this session — all BYML / RESTBL / MSBT / BFLYT-ops / AAMP work is
pushed. This session adds **BFRES inspect-only** (header inspect + verbatim
byte-identical round-trip; embedded-BNTX surfacing) on top — see the top Session
log entry. **Ask the user before pushing.** (MeshCodec `.mc` decompression was
investigated and deferred to a user-approved deep-RE follow-up — see the gap
table + the session log.)

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
- ✅ **BYAML/BYML** (this session): read + `byml-inspect` + verbatim
  byte-identical round-trip (686ad53) **plus** the from-scratch canonical
  writer (semantically lossless) + `byml-diff` (aa7d166). Scalar
  mutation-by-path (`byml-set`) added this session; add/remove-by-path remains
  a follow-up.
- ✅ **RSTB/RESTBL** (commit 71fe57c): TotK `RESTBL` v1 read/write
  (byte-identical) + CRC-32 path lookup + size update/insert; verbs
  `restbl-inspect`/`restbl-roundtrip-test`/`restbl-set`. BOTW `RSTB` (older
  magic) is a follow-up.
- ✅ **MSBT (Stages A+B)** (this session, committed/unpushed): `src/msbt/`
  read + verbatim byte-identical round-trip (all 1510 USen + 1510 JPja `Mals`
  `.msbt`) + a semantically-lossless canonical writer (LMS-hash `LBL1`
  rebuild) + `Message::from_chunks`/`set_message_by_label` mutation. Verbs
  `msbt-inspect`/`msbt-roundtrip-test`/`msbt-export-json`/`msbt-import-json`
  (translation-editing workflow). BOTW/older versions + `ATR1`/`TSY1`
  structural decode are follow-ups.
- ✅ **`byml-set`** (this session, committed): BYML scalar mutation-by-path —
  new `src/byml/edit.rs` (`set_by_path` + `ScalarType` + `SetReport`,
  type-preserving by default or `--type` override / `null` promotion; refuses
  to clobber containers/binary or descend through scalars) → re-serialize with
  `write_byml_canonical`. Verb `byml-set` (path/value/`--type`, inflates
  `.byml.zs`, writes uncompressed). 11 unit + 3 fixture-gated tests
  (`tests/byml_set.rs`: a real `CookingTable` edit = exactly one structural
  diff). Add/remove-by-path is the remaining BYML follow-up.
- ✅ **BFLYT advanced pane mutations** (this session, committed `a50ae76` +
  `d75960a`): `remove`/`move`/`rename`/`copy-subtree` pane (group-ref-aware,
  cycle/collision-guarded) + `set-window` (wnd1 borders) + `set-text`/`pane_text`
  (txt1 single-string layout, UTF-16LE; rejects text-id/per-char/line-width
  panes). Verbs `pane-remove`/`pane-move`/`pane-rename`/`pane-copy`/
  `bflyt-set-window`/`bflyt-set-text`.
- ✅ **BFLYT prune + repair** (this session, committed `582e493`):
  `src/bflyt/repair.rs` — prune unused materials/textures (index remap), clamp
  dangling texture refs, dedupe duplicate pane names, `repair()` →
  `RepairReport`. Verbs `bflyt-prune` + `bflyt-repair` (`--dry-run`). Material
  pruning skips when prt1 property data is present.
- ✅ **AAMP (BOTW binary parameter archive)** (this session, committed
  `ef11b4c` / `6ad1ee6` / `cf9f257`): `src/aamp/` read + verbatim byte-identical
  round-trip (**418** real BOTW files) + from-scratch canonical writer
  (semantically lossless on all 418) + `set_by_path` type-preserving scalar
  mutation. Verbs `aamp-inspect` / `aamp-roundtrip-test` / `aamp-set`. Fixtures
  (32 files across 18 extensions, from a BOTW dump) under the gitignored
  `tests/fixtures/aamp/`.
- ✅ **BFRES inspect-only** (this session): `src/bfres/` header inspect
  (magic/version/BOM/name/size/RLT + sub-block magic scan) + verbatim
  byte-identical round-trip across **424** real files (BOTW v5 `.sbfres`, TotK
  v10 `.bfres.zs`, decompressed v10 models); `bfres-inspect` surfaces a BOTW
  `.Tex.bfres`'s embedded BNTX via the existing reader. Verbs `bfres-inspect` /
  `bfres-roundtrip-test`.
- ▶️ **Next** (see `todo.md`): **MeshCodec `.mc` deep RE** (user-approved:
  NSO0+LZ4 Rust port → locate raw dict + framing in `exefs/main` → validate vs
  the decompressed-output oracle) and/or **BFRES sub-resource decode**
  (FMDL/FSKA) → **project workflow** (project-init/audit/apply/build + cached
  corpus audit). (Smaller follow-ups: AAMP name table + curve decode +
  add/remove; layout.arc `layout-repair`; ASTC/low-bpp **encode**; BYML
  add/remove-by-path; BOTW `RSTB`; MSBT `ATR1`/older versions.)

Standing backlog (no owner):

- v9 BFLYT 60-byte material extension decode (captured verbatim today).
- GitHub Actions CI workflow (gated on shipping fixtures to CI).
- In-game runtime validation on Switch hardware (requires hardware).

## Session log

### 2026-06-04 - MeshCodec B2-CP4 chunk three-u16 signed-delta writer
**Committed:** B2-CP4 three-u16 signed-delta writer chunk landed in `1d19d70`,
porting `0x1100c90` in `src/mc/geometry.rs` as
`transform_tail_u16x3_delta_into`. Durable local evidence:
`capture_transform_tail_1100c90.py` enumerated **3** observed calls
(Bear/Bass/Dragonfly current 0), and `verify_transform_tail_1100c90.py`
replayed **3/3**. Coverage totals are direct literals 1193, matched literals
3067, copy units 149, sign flips 881, match entries 4409, source0 7158 bytes,
and source1 18402 bytes. Discriminators: no-sign-flip fails 3/3 and byte-delta
fails 3/3. Rust coverage: fixture-free Bear first-record golden plus malformed
guards; `cargo test --lib transform_tail_u16x3_delta` passed 2 tests,
`cargo test --lib transform_tail` passed 19 tests, and `cargo build` passed.
Updated Dragonfly sparse bufB probe now matches **5753/5753** touched oracle
bytes; full bufB first diff moved to byte 5233, which maps to Dragonfly current
3 at `bufB+5232`, target `0x110afb0`. Next after commit: enumerate/replay
`0x110afb0` before porting any other writer target.

### 2026-06-04 - MeshCodec B2-CP4 state-5 writer target capture
**Captured:** `local-assets/re/capture_state5_writer_targets.py` now records the
actual state-5 indirect writer target for every observed table column across
Bear, Bass, and Dragonfly. This confirms the early bufB/oracle plan: the updated
Dragonfly sparse probe includes the newly ported `0x10fdc00` writer and matches
**2615/2615** touched oracle bytes, but full bufB still fails first at byte 2.
That byte belongs to Dragonfly current 0, target `0x1100c90`, `bufB+0`, stride
10, record count 1, source count 2. Other observed unported targets are
`0x1103ab0`, `0x110aac0`, `0x110afb0`, `0x10fde00`, and Bass-only
`0x11033e0`, but they should wait until the oracle diff proves them needed.
Next: enumerate/replay `0x1100c90` across the observed calls and port it only
after the Python replay is byte-exact.

### 2026-06-04 - MeshCodec B2-CP4 chunk Bear direct/delta writer
**Committed:** B2-CP4 three-byte direct/delta writer chunk landed in `04c09d0`,
porting the now-proven `0x10fdcf0` writer in `src/mc/geometry.rs` as
`transform_tail_delta3_direct_into`. It is the Bear table-entry-1 sibling of
`0x10fdc00`: direct literals copy three-byte source0 units, non-zero match
entries add source1 deltas to prior strided output, and copy runs clone previous
output by byte distance. Durable local evidence:
`verify_transform_tail_delta3_direct.py` replayed **1/1** Bear call
with direct literals 197, matched literals 1185, copy units 1945, match entries
3327, source0 591 bytes, source1 3555 bytes; wrong delta-match direct shape and
no-match-delta both fail 1/1. Rust coverage:
`cargo test --lib transform_tail_delta3_direct` passed 2 tests,
`cargo test --lib transform_tail_delta` passed 10 tests, and `cargo build`
passed. Next after commit: update/rerun the early bufB sparse assembly probe
with the direct-delta writers included, then port only the next writer the diff
actually proves.

### 2026-06-04 - MeshCodec B2-CP4 chunk Dragonfly direct/delta writer
**Committed:** B2-CP4 direct/delta writer chunk landed in `c81468b`. After the
Dragonfly bufB probe, expanded `capture_transform_tails.py` to hook all 32
state-5 writer-table entries. The refreshed capture found Dragonfly uses
`0x10fdc00` for table entry 1 before the already ported copy2/copy1 tails; Bear
also proves `0x10fdcf0` for table entry 1, while Bass adds no new writer.
Ported `0x10fdc00` in
`src/mc/geometry.rs` as `transform_tail_delta2_direct_into`: direct literals
copy two-byte source0 units, non-zero match entries add source1 deltas to prior
strided output, and copy runs clone previous output by byte distance. Durable
local evidence: `verify_transform_tail_delta2_direct.py` replayed **1/1**
Dragonfly call with direct literals 17, matched literals 145, copy units 361,
match entries 523, source0 34 bytes, source1 290 bytes; wrong delta-match
direct shape and no-match-delta both fail 1/1. Rust coverage:
`cargo test --lib transform_tail_delta2_direct` passed 2 tests,
`cargo test --lib transform_tail_delta` passed 8 tests, and `cargo build`
passed. Next: commit this chunk, then port the now-proven `0x10fdcf0`
three-byte direct/delta writer before chasing any unobserved table entries.

### 2026-06-04 - MeshCodec B2-CP4 early Dragonfly bufB probe
**Probe complete:** after `98334d2`, stopped adding leaves and ran the requested
early bufB/oracle slice through `local-assets/re/probe_bufb_dragonfly.py`.
Dragonfly table entry 2 has `0x10fb2e0` call 2 records matching the downstream
`0x10fc680` copy2 tail exactly; replay touches **1046** bufB byte positions at
`bufB+8`, stride 10, and all touched bytes match the final oracle. Dragonfly
table entry 7 has the early wrapper record `[523, 0]` matching the downstream
`0x10fc5e0` copy1 tail exactly; replay touches **523** positions at
`bufB+5247`, stride 20, and all touched bytes match oracle. Sparse assembly
with just those two already-ported tails matches **1569/1569** touched oracle
bytes but full bufB still fails at byte 2. The diff points to the inline/simple
state-5 transform columns after `0x10fb2e0` as the next proven-needed path, not
`mode_other`, `mode3`, or additional non-observed transform leaves.

### 2026-06-04 - MeshCodec B2-CP4 chunk byte-group transform wrapper
**Committed:** B2-CP4 transform-wrapper chunk landed in `98334d2`, porting the
observed `0x10fb2e0` paths in `src/mc/geometry.rs`. The wrapper now handles the
even-count early return and the observed mode-1 active path: two forward
varints, one reverse count bit, three decoded byte-group streams, one direct
tail stream, and `0x110d360` record assembly. Durable local evidence:
`capture_byte_group_transform.py` enumerated Bear 8 + Bass 8 + Dragonfly 9 =
**25** calls; `verify_byte_group_transform.py` replayed **25/25**. Branch
coverage is active `(4 x 0x110d7f0, 1 x 0x110d360)` count 23 and early count
2; active tails are all direct selector 3. Discriminators: early-as-active
fails 2/25, `third_count = first_count` fails 18/23 active calls, and
`third_count = first_count - 1` fails 5/23 active calls. Rust coverage:
fixture-free Animal_Dragonfly active and early goldens plus defensive guards;
`cargo test --lib byte_group_transform` passed 3 tests,
`cargo test --lib byte_group_reader` passed 6 tests,
`cargo test --lib width_combiner` passed 3 tests, and `cargo build` passed.
Last checkpoint baseline remains: All green: **195 lib unit** (incl. **49**
`mc::geometry`) + all integration; clippy `--all-targets` clean;
`--no-default-features` builds. Next after commit: per user instruction, stop
adding leaves and attempt early bufB assembly plus decode-vs-oracle on one
fixture/region/attribute before porting more transform paths.

### 2026-06-04 - MeshCodec B2-CP4 chunk byte-group selector 2
**Committed:** B2-CP4 selector-2 byte-group chunk landed in `5a858dc`,
porting the observed single zstd-window branch of `0x110d7f0`. Selector 2 now
consumes the `0x1110a60` window flag, decodes one zstd block-content window
from the forward stream using the caller selector-2 history as dictionary
content, advances `x6+0x48`, and appends the regenerated bytes to the history
rooted at caller `[x0+8]`. Durable local evidence:
`verify_byte_group_reader_selector2.py` replayed selector 2 **8/8** across the
refreshed Bear/Bass/Dragonfly capture. Selector coverage remains 0 = 5, 1 = 84,
2 = 8, 3 = 62; selector-2 params `(w2,w3,w5)` are `(0,1,0)` count 3,
`(0,3,8)` count 1, `(0,4,8)` count 2, `(1,2,8)` count 2. All observed windows
have zstd flag 0; raw flag 1, output >0x20000, and 0x80000 history wrap remain
typed errors. Discriminators: no-output fails 8/8 and no-history fails 1/8,
proving the dictionary history is required. Rust coverage:
`cargo test --lib byte_group_reader` passed **6** tests, including the
fixture-free Animal_Dragonfly selector-2 zstd golden and selector-2 defensive
guards; `cargo build` passed. Last checkpoint baseline remains: All green:
**195 lib unit** (incl. **49** `mc::geometry`) + all integration; clippy
`--all-targets` clean; `--no-default-features` builds. Next: `0x110d7f0`
replays all observed selector populations; continue B2-CP4 with the
`0x10fb2e0` / caller-side transform wrapper unless a full checkpoint gate is
due.

### 2026-06-03 - MeshCodec B2-CP4 chunk byte-group selector 1
**Committed:** B2-CP4 selector-1 byte-group chunk landed in `1db9848`,
porting the observed single-window selector-1 branch of `0x110d7f0`.
Selector 1 now builds one descriptor through `0x110de80` and dispatches through
`0x110dd80` for byte elements or `0x110de00` for halfword elements, returning
halfwords little-endian. Durable local evidence:
`verify_byte_group_reader_selector1.py` replayed **84/84** selector-1 calls
from refreshed `byte_group_reader_capture.json`; selector coverage remains
0 = 5, 1 = 84, 2 = 8, 3 = 62. Selector-1 descriptor coverage is byte mode 0 =
21, byte mode 1 = 24, byte mode 2 = 10, u16 mode 0 = 11, u16 mode 1 = 17, and
u16 mode 2 = 1. No-output failed 76/84, and no stream-cursor writeback failed
28/28 stream-advancing calls. The large selector-1 split at `w4*w3 >= 0x80000`
is still guarded because max observed `w4*w3` is 13884. Rust coverage:
fixture-free Animal_Bass call 24 covers byte mode 0 with a four-byte stream
advance; Animal_Bass call 6 covers u16 mode 0 with little-endian serialization
and a 16-byte stream advance. Per-chunk sample green:
`python local-assets/re/verify_byte_group_reader_selector1.py` (84/84),
`cargo test --lib byte_group_reader` (5 passed),
`cargo test --lib rans_segment_dispatch_bytes` (4 passed),
`cargo test --lib rans_segment_dispatch` (8 passed), and `cargo build`.
Last checkpoint baseline remains: All green: **195 lib unit** (incl. **49**
`mc::geometry`) + all integration; clippy `--all-targets` clean;
`--no-default-features` builds. Next: selector 2 (`0x1110cc0`/`0x1110a60`)
remains the only guarded `0x110d7f0` selector in the refreshed capture, unless
the next independent CP4 step can wire selector 0/1/3 while keeping selector 2
guarded.

### 2026-06-03 - MeshCodec B2-CP4 chunk byte-group selector 0
**Committed:** B2-CP4 selector-0 byte-group chunk landed in `1610408`,
porting `0x110dae0` as `geometry::rans_segment_loop_bytes_into` and wiring
selector 0 of `0x110d7f0` through the byte and u16 segment loops. Durable local
evidence: `verify_byte_segment_loop.py` replayed **4/4** byte-loop calls across
Bear/Bass/Dragonfly for output bytes, schedules, primary reader, mode-1 extra
readers, rANS state, and stream cursor; dispatch coverage inside those loops is
mode 0 = 7, mode 1 = 6, mode 2 = 0, with mode 2 typed-error guarded until
captured. Refreshed `capture_byte_group_reader.py` still has **159** calls with
selector coverage 0 = 5, 1 = 84, 2 = 8, 3 = 62; `verify_byte_group_reader_selector0.py`
replayed selector 0 **5/5** for byte and halfword paths, and no-output failed
5/5. Rust coverage: fixture-free Animal_Bass selector-0 call 0
(`w2=0,w3=1,w4=236,w5=0`) covers the byte segment loop plus byte mode-1
dispatch; guard tests reject selectors 1/2, selector-0 element shift 2,
truncated selector loads, direct-stream underflow, and output-size overflow.
Per-chunk sample green: `python local-assets/re/verify_byte_segment_loop.py`
(4/4), `python local-assets/re/verify_byte_group_reader_selector0.py` (5/5),
`cargo test --lib byte_group_reader` (3 passed),
`cargo test --lib rans_segment_dispatch_bytes` (4 passed), and `cargo build`.
Last checkpoint baseline remains: All green: **195 lib unit** (incl. **49**
`mc::geometry`) + all integration; clippy `--all-targets` clean;
`--no-default-features` builds. Next: continue `0x110d7f0` with selector 1 or
selector 2 windowed/zstd paths, or wire selector 0/3 into the `0x10fb2e0`
transform wrapper if that is the next independent CP4 step.

### 2026-06-03 - MeshCodec B2-CP4 chunk byte three-lane decode
**Committed:** B2-CP4 byte three-lane chunk landed in `425532c`, porting
`0x110eb50` as `geometry::rans_three_lane_decode_bytes_into` and wiring
`geometry::rans_segment_dispatch_bytes_into` mode 1 through it. Durable local
evidence: `verify_rans_byte_three_lane.py` replayed **30/30** byte mode-1 calls
from `segment_dispatch_byte_capture.json` for output bytes and all three reader
writebacks. Coverage includes `count % 12` values 0/1/2/4/6/7/8/9/10/11, logs
1 through 9 as observed, stride 1 and stride 6. Updated
`verify_segment_dispatch_byte.py` now replays the full byte dispatch population:
mode0 **28/28**, mode1 **30/30**, mode2 **10/10**. Rust coverage: fixture-free
Animal_Dragonfly dispatch call 61 (`count=21,log=3,stride=1`) covers the
12-symbol main loop plus 9-symbol tail, the same golden is wired through byte
dispatch mode 1, and defensive tests reject missing readers, table mismatch,
zero stride, undersized output, and truncated payload. Per-chunk sample green:
`cargo test --lib rans_three_lane_decode_bytes` (1 passed),
`cargo test --lib rans_segment_dispatch_bytes` (4 passed),
`cargo test --lib rans_three_lane` (2 passed), and `cargo build`. Last checkpoint
baseline remains: All green: **195 lib unit** (incl. **49** `mc::geometry`) +
all integration; clippy `--all-targets` clean; `--no-default-features` builds.
Next: return to `0x110d7f0` selector 0/1 byte segment paths; `0x110dd80` now has
all observed dispatch modes available.

### 2026-06-03 - MeshCodec B2-CP4 chunk byte segment dispatch
**Committed:** B2-CP4 byte segment dispatch chunk landed in `5501c6f`, porting
the observed mode-0 and mode-2 branches of `0x110dd80` as
`geometry::rans_segment_dispatch_bytes_into`. Durable local evidence:
`capture_segment_dispatch_byte.py` enumerated **68** calls across
Bear/Bass/Dragonfly: mode 0 = 28, mode 1 = 30, and mode 2 = 10.
`verify_segment_dispatch_byte.py` replayed mode 0 **28/28** for output bytes,
rANS states, flags, stream consumption, and stream offsets, replayed mode 2
**10/10**, and counted the 30 mode-1 calls as guarded for the separate
`0x110eb50` byte three-lane decoder. Rust coverage: fixture-free Animal_Bass
mode-0 tail golden (`count=30,count&3=2`), Animal_Dragonfly mode-2 fill golden
(`value=1,count=3,stride=1`), plus rejection for mode 1, unknown modes, zero
stride, undersized output, and counts too large for the `w2` byte-rANS argument.
Per-chunk sample green: `python local-assets/re/verify_segment_dispatch_byte.py`
(mode0 28/28, mode2 10/10, mode1 guarded 30),
`cargo test --lib rans_segment_dispatch_bytes` (3 passed),
`cargo test --lib rans_decode_bytes` (1 passed),
`cargo test --lib rans_init_states` (4 passed), and `cargo build`. Last
checkpoint baseline remains: All green: **195 lib unit** (incl. **49**
`mc::geometry`) + all integration; clippy `--all-targets` clean;
`--no-default-features` builds. Next: continue `0x110d7f0` selector 0 or 1
byte/zstd segment paths, using the byte segment dispatcher for rANS/RLE branches
while keeping `0x110eb50` guarded until replayed.

### 2026-06-03 - MeshCodec B2-CP4 chunk byte-output rANS
**Committed:** B2-CP4 byte-output rANS chunk landed in `9525c5b`, porting the
emitted-byte side of `0x110dfa0` as
`geometry::rans_decode_bytes_into_with_cursor` and
`geometry::rans_decode_bytes_into`. Durable local evidence:
`capture_rans_byte_decode.py` enumerated **28** calls across
Bear/Bass/Dragonfly; `verify_rans_byte_decode.py` replayed **28/28** for output
bytes, states, flags, stream consumption, and stream offsets. Coverage includes
`count % 4` values 0/1/2/3, stride 1 and stride 6, plus cold and warm state
buffers; the unobserved `count < 4` scalar path is still typed-error guarded.
Rust coverage: existing Bear `0x110dfa0` state golden now also asserts emitted
bytes, Animal_Bass call 1 covers the observed tail (`count=30,count&3=2`), and
defensive tests reject zero stride, undersized output, truncated stream data,
and scalar count. Per-chunk sample green: `cargo test --lib rans_init_states`
(4 passed), `cargo test --lib rans_decode_bytes` (1 passed), and `cargo build`.
Last checkpoint baseline remains: All green: **195 lib unit** (incl. **49**
`mc::geometry`) + all integration; clippy `--all-targets` clean;
`--no-default-features` builds. Next: use the byte-output primitive to port
`0x110dd80`, then continue `0x110d7f0` selectors 0/1.

### 2026-06-03 - MeshCodec B2-CP4 chunk byte-group direct selector
**Committed:** B2-CP4 byte-group reader slice landed in `150057c`, porting the
observed selector-3 branch of `0x110d7f0` as `geometry::byte_group_read`.
Durable local evidence: `capture_byte_group_reader.py` enumerated **159**
`0x110d7f0` calls across Bear/Bass/Dragonfly; selector coverage is selector 0 =
5, selector 1 = 84, selector 2 = 8, and selector 3 = 62.
`verify_byte_group_reader_selector3.py` replayed selector 3 **62/62** for output
bytes plus reverse-reader and forward-stream writeback. Selectors 0/1/2 are
typed-error guarded until their byte/zstd segment paths are ported. Rust
coverage: fixture-free Animal_Bass call 16 selector-3 golden, plus rejection for
guarded selectors, truncated selector loads, direct-stream underflow, and output
size overflow. Per-chunk sample green: `cargo test --lib byte_group_reader` (2
passed) and `cargo build`. Last checkpoint baseline remains: All green: **195
lib unit** (incl. **49** `mc::geometry`) + all integration; clippy
`--all-targets` clean; `--no-default-features` builds. Next: continue
`0x110d7f0` with selector 0, 1, or 2, then wire the byte-group reader into
`0x10fb2e0`.

### 2026-06-03 - MeshCodec B2-CP4 checkpoint primitive transform tails
**Committed:** B2-CP4 observed primitive transform-tail helpers are green through
`bbf4225` (latest ledger), with code chunks `bbc0cf7` (`0x10fc5e0` byte copy),
`0ac04ab` (`0x10fc680` halfword copy), `7206bcc` (`0x10fc7d0` word copy),
`0cf6c03` (`0x10fbcc0` two-byte delta-match), and `6a74462`
(`0x10fbdc0` three-byte delta-match). Durable local evidence:
`capture_transform_tails.py` now records match tables and still finds the same
8 transform-tail calls across Bear/Bass/Dragonfly with matching final buffers;
the replay scripts reproduce `0x10fc5e0` **3/3**, `0x10fc680` **1/1**,
`0x10fc7d0` **2/2**, `0x10fbcc0` **1/1**, and `0x10fbdc0` **1/1**. Full
checkpoint gate run: All green: **195 lib unit** (incl. **49** `mc::geometry`)
+ all integration; clippy `--all-targets` clean; `--no-default-features`
builds. Next: continue B2-CP4 by wiring the validated primitive tails into the
observed `0x10fb2e0` transform wrapper, or follow the handoff's next checkpoint
if it names a different independent chunk.

### 2026-06-03 - MeshCodec B2-CP4 chunk three-byte delta-match tail
**Committed:** B2-CP4 chunk landed in `6a74462`, porting the observed
`0x10fbdc0` three-byte delta-match tail as
`geometry::transform_tail_delta3_into`. Durable local evidence:
`verify_transform_tail_delta3.py` replayed the Bear `0x10fbdc0` call **1/1**.
Coverage: direct literals 483, match literals 1910, copy units 934,
zero-literal records 22, zero-copy records 1, and 3327 match-table entries
consumed. Rust coverage: fixture-free Bear golden for first two entry
`0x0c000803` records covering direct/match/copy behavior, observed
zero-literal and zero-copy branch tests, plus defensive rejection for zero
stride, output underflow, all source underflows, match-table underflow,
match-before-output, and copy-before-output. Per-chunk sample green:
`cargo test --lib transform_tail` (13 passed) and `cargo build`. Last
checkpoint baseline remains: All green: **182 lib unit** (incl. **36**
`mc::geometry`) + all integration; clippy `--all-targets` clean;
`--no-default-features` builds. B2-CP4 primitive observed tail coverage is now
complete for the current Bear/Bass/Dragonfly fixtures; next: run the B2-CP4
full checkpoint gate and record counts.

### 2026-06-03 - MeshCodec B2-CP4 chunk two-byte delta-match tail
**Committed:** B2-CP4 chunk landed in `0cf6c03`, porting the observed
`0x10fbcc0` two-byte delta-match tail as
`geometry::transform_tail_delta2_into`. Refreshed local evidence:
`capture_transform_tails.py` now records the match table and still finds the
same 8 transform-tail calls with matching final buffers; `verify_transform_tail_delta2.py`
replayed the Bass `0x10fbcc0` call **1/1**. Coverage: direct literals 214,
match literals 332, copy units 13, zero-literal records 1, zero-copy records 1,
and 559 match-table entries consumed. Rust coverage: fixture-free Bass golden
for first entry `0x0a000802` record covering direct/match/copy behavior,
observed zero-literal and zero-copy branch tests, plus defensive rejection for
zero stride, output underflow, all source underflows, match-table underflow,
match-before-output, and copy-before-output. Per-chunk sample green:
`cargo test --lib transform_tail` (10 passed) and `cargo build`. Last
checkpoint baseline remains: All green: **182 lib unit** (incl. **36**
`mc::geometry`) + all integration; clippy `--all-targets` clean;
`--no-default-features` builds. Next: continue B2-CP4 with observed
`0x10fbdc0` three-byte delta-match tail.

### 2026-06-03 - MeshCodec B2-CP4 chunk four-byte transform tail
**Committed:** B2-CP4 chunk landed in `7206bcc`, porting the observed
`0x10fc7d0` four-byte transform tail as
`geometry::transform_tail_copy4_into`. It uses the shared fixed-width copy
routine with four-byte literal/copy units while keeping stride and
back-distance in bytes. Durable local evidence: `capture_transform_tails.py`
found 2 calls to this address (Bear and Bass, both stride 16), and
`verify_transform_tail_copy4.py` replayed **2/2**. Coverage: record counts
{31,143}, 150 literal runs, 24 zero-literal runs, 172 copy runs, 2 zero-copy
runs, 3516 literal units, and 370 copy units. Rust coverage: fixture-free Bear
golden for first entry `0x1000100a` record, observed zero-literal and zero-copy
branch tests, plus defensive rejection for zero stride, source underflow,
output underflow, and copy-before-output. Per-chunk sample green:
`cargo test --lib transform_tail_copy` (7 passed) and `cargo build`. Last
checkpoint baseline remains: All green: **182 lib unit** (incl. **36**
`mc::geometry`) + all integration; clippy `--all-targets` clean;
`--no-default-features` builds. Next: continue B2-CP4 with observed
delta-match tails `0x10fbcc0` and `0x10fbdc0`.

### 2026-06-03 - MeshCodec B2-CP4 chunk two-byte transform tail
**Committed:** B2-CP4 chunk landed in `0ac04ab`, porting the observed
`0x10fc680` two-byte transform tail as
`geometry::transform_tail_copy2_into`. It shares the fixed-width copy routine
with the `0x10fc5e0` byte tail but uses two-byte literal/copy units while
keeping stride and back-distance in bytes. Durable local evidence:
`capture_transform_tails.py` found exactly 1 call to this address in
Dragonfly, and `verify_transform_tail_copy2.py` replayed it **1/1**.
Coverage: stride {10}, block index {0}, 3 records, 3 literal runs, 0
zero-literal runs, 3 copy runs, 0 zero-copy runs, 3 literal units, 520 copy
units, and 6 source bytes. Rust coverage: fixture-free Dragonfly golden for
the full entry `0x0a000802` record population plus defensive rejection for
zero stride, source underflow, output underflow, copy-before-output, and the
unobserved zero-literal or zero-copy record shapes. Per-chunk sample green:
`cargo test --lib transform_tail_copy` (4 passed) and `cargo build`. Last
checkpoint baseline remains: All green: **182 lib unit** (incl. **36**
`mc::geometry`) + all integration; clippy `--all-targets` clean;
`--no-default-features` builds. Next: continue B2-CP4 with `0x10fc7d0`
four-byte copy tail or the delta-match tails.

### 2026-06-03 - MeshCodec B2-CP4 chunk single-byte transform tail
**Committed:** B2-CP4 chunk landed in `bbc0cf7`, porting the observed
`0x10fc5e0` single-byte transform tail as
`geometry::transform_tail_copy1_into`. Durable local evidence:
`capture_transform_tails.py` found 8 transform-tail calls total across
Bear/Bass/Dragonfly and `verify_transform_tail_copy1.py` replayed the
`0x10fc5e0` subset **3/3**. Parameter coverage for this tail: strides
{16,20}, block index {0}, record counts {1,10,12}, 14 literal runs, 9
zero-literal runs, 22 copy runs, 686 literal bytes, and 3723 copy bytes.
Rust coverage: fixture-free Bear golden for the first two entry
`0x10000801` records plus defensive rejection for zero stride, source
underflow, output underflow, and copy-before-output. Per-chunk sample green:
`cargo test --lib transform_tail_copy1` (2 passed) and `cargo build`. Last
checkpoint baseline remains: All green: **182 lib unit** (incl. **36**
`mc::geometry`) + all integration; clippy `--all-targets` clean;
`--no-default-features` builds. Next: continue B2-CP4 with another observed
transform tail, likely `0x10fc7d0` or `0x10fc680`.

### 2026-06-03 - MeshCodec B2-CP3 checkpoint width combiner
**Committed:** B2-CP3 landed in `f3e4673`, porting the `0x110d360`
3-stream width combiner as `geometry::width_combiner_into`. It decodes
the three `0x110d7f0`-produced width streams into `[u32; 2]` records,
threads the reversed forward bit reader, returns the game's `w0` sum, and
reports exact stream byte consumption. Durable local evidence:
`capture_width_combiner.py` enumerated **23** calls across Bear/Bass/
Dragonfly; `verify_width_combiner.py` replays **23/23** final records,
reader states, return values, and consumed stream lengths. Covered
branches include first/second inline and expanded bytes, third-stream
history and special codes, clamped and non-clamped tails; the unobserved
tail-only `count < 2` path is guarded. Fixture-free Rust goldens cover
Bear call 7 and Dragonfly call 2 plus malformed bounds. All green:
**182 lib unit** (incl. **36** `mc::geometry`) + all integration; clippy
`--all-targets` clean; `--no-default-features` builds. Next: B2-CP4
kernel transform `0x10f9690` -> `0x10fa980`/`0x10fab60`/`0x10facf0`.

### 2026-06-03 - MeshCodec B2-CP2 checkpoint segment loop
**Committed:** B2-CP2 landed in `52664f8`, porting the `0x110dc30`
segment loop as `geometry::rans_segment_loop_into`. It threads the shared
reverse reader, forward rANS stream pointer, descriptor-workspace states, and
lane-interleaved output while composing the B2-CP1 `0x110de80` descriptor
builder and `0x110de00` dispatch wrapper. Durable local evidence:
`capture_segment_loop.py` enumerated the full local population as exactly one
loop call (Bear 0 / Bass 1 / Dragonfly 0), and `verify_segment_loop.py`
replays **1/1** final output/context/state/schedule with dispatch modes
{0:1, 2:3}. Mode 1 inside this loop is guarded as unobserved. Fixture-free
Rust tests cover the Bass full 968-slot padded output plus malformed bounds.
All green: **179 lib unit** (incl. **33** `mc::geometry`) + all integration;
clippy `--all-targets` clean; `--no-default-features` builds. Next: B2-CP3
3-stream width combiner `0x110d360`.

### 2026-06-03 - MeshCodec B2-CP1 checkpoint green
**Committed:** B2-CP1 checkpoint is now fully green after cleanup commit
`bda3c17` removed the redundant `u32` casts that made clippy warn in
`rans_read_segment_header`. The completed B2-CP1 chain is `e290e74`
(`0x110de00` dispatch), `b5ed5bc` (`0x110ef70` three-lane decode),
`43eafb0` (`0x110de80` header), `bb864aa` (`0x110e540` mode-0 table),
`c2dc036` (`0x110f3c0` mode-1 table), `279511b` (`0x110de80`
descriptor builder), plus `bda3c17` checkpoint cleanup.
Durable local evidence remains gitignored under `local-assets/re/`:
`verify_segment_descriptor_builder.py` replays **99/99** descriptors
with mode counts {0:40, 1:47, 2:12}; `verify_mode0_table_builder.py`
replays **40/40**; `verify_mode1_table_builder.py` replays **47/47**;
`verify_segment_dispatch.py` replays mode 0 **12/12**, mode 1 **17/17**,
and mode 2 **4/4**. All green: **177 lib unit** (incl. **31**
`mc::geometry`) + all integration; clippy `--all-targets` clean;
`--no-default-features` builds. Next: B2-CP2 segment loop `0x110dc30`.

### 2026-06-03 - MeshCodec B2-CP1 chunk segment descriptor builder
**Committed:** B2-CP1 chunk 1f wires the parsed segment header and mode-specific
table builders into `geometry::rans_build_segment_descriptor`, the composed
`0x110de80` descriptor builder. It returns mode/log/value, mode-0 step+sym
tables or mode-1 packed entries, and the advanced reverse reader state. It does
not initialize mode-0 rANS states; those remain owned by the surrounding segment
loop (`0x110dc30`). Durable local evidence: new gitignored
`verify_segment_descriptor_builder.py` replays **99/99** table builds from
`segment_dispatch_capture.json` with mode counts {0:40, 1:47, 2:12}. The
fixture-free dispatch tests now feed descriptors built from the reverse stream
into mode 0, mode 1, and mode 2 dispatch paths. Next: run the B2-CP1 checkpoint
full gate, append the checkpoint ledger, then proceed to B2-CP2 (`0x110dc30`
segment loop).
Sample green: **31 `mc::geometry` lib unit**; `verify_segment_descriptor_builder.py`
and `verify_segment_dispatch.py` green. Last checkpoint full-suite baseline
remains: All green: **163 lib unit** (incl. **17** `mc::geometry`) + all
integration; clippy `--all-targets` clean; `--no-default-features` builds.

### 2026-06-03 - MeshCodec B2-CP1 chunk mode-1 table builder
**Committed:** B2-CP1 chunk 1e ports the mode-1 segment table builder
`0x110f3c0` as `geometry::rans_build_mode1_table`. It builds the packed
three-lane table consumed by `0x110ef70`: high 16 bits are bits consumed and low
16 bits are the emitted symbol. Durable local evidence: new gitignored
`verify_mode1_table_builder.py` replays **47/47** captured mode-1 table builds
from `segment_dispatch_capture.json`; branch coverage is log<2 special path
9/47 and log>=2 general path 38/47, with observed logs 1..11. Fixture-free
tests cover the log=1 special expansion, a log=4 general prefix table, exact
packed entries, advanced reader state, truncated payload rejection, zero count,
unsupported log, and count greater than mass. Next: wire
`rans_read_segment_header` + `rans_build_mode0_table` + `rans_build_mode1_table`
into a full segment descriptor builder and run the B2-CP1 checkpoint full gate.
Sample green: **31 `mc::geometry` lib unit**; `verify_mode1_table_builder.py`
and `verify_segment_dispatch.py` green. Last checkpoint full-suite baseline
remains: All green: **163 lib unit** (incl. **17** `mc::geometry`) + all
integration; clippy `--all-targets` clean; `--no-default-features` builds.

### 2026-06-03 - MeshCodec B2-CP1 chunk mode-0 table builder
**Committed:** B2-CP1 chunk 1d ports the mode-0 segment table builder
`0x110e540` as `geometry::rans_build_mode0_table`. It consumes the parsed
header's table count/log plus the reverse reader state, decodes the sparse
symbol list (`count <= 10` loop or the `0x110e9a0` large-list path), reads
`count-1` frequencies with the validated `0x110e7b0` formula, appends the
implicit tail mass, and builds the sparse contiguous step/symbol table consumed
by `0x110de00` mode 0. Durable local evidence: new gitignored
`verify_mode0_table_builder.py` replays **40/40** captured mode-0 table builds
from `segment_dispatch_capture.json`; branch coverage is small-symbol 27/40 and
large-symbol 13/40. Fixture-free tests cover both branches, exact table output,
advanced reader state, truncated payload rejection, zero count, unsupported log,
and count greater than mass. Next: port/connect the mode-1 table builder
`0x110f3c0`, then wire header plus table builders into a descriptor builder and
run the B2-CP1 checkpoint full gate.
Sample green: **29 `mc::geometry` lib unit**; `verify_mode0_table_builder.py`
and `verify_segment_dispatch.py` green. Last checkpoint full-suite baseline
remains: All green: **163 lib unit** (incl. **17** `mc::geometry`) + all
integration; clippy `--all-targets` clean; `--no-default-features` builds.

### 2026-06-03 - MeshCodec B2-CP1 chunk segment header parser
**Committed:** B2-CP1 chunk 1c ports the header parser portion of `0x110de80`
as `geometry::rans_read_segment_header`. It consumes the reverse-bit segment
header and returns mode, log, table-count for modes 0/1, RLE value for mode 2,
and the advanced reader state. Durable local evidence: refreshed
`capture_segment_dispatch.py` now records 87 non-RLE header snapshots
(`0x110df48`) plus the 12 RLE headers in the build rows; new
`verify_segment_header.py` replays the parser **99/99** over all table-build
headers with mode counts {0:40, 1:47, 2:12}. Fixture-free tests cover the short
non-RLE form, all three observed long-form count-width branches, the mode-2 RLE
value form, and truncated payload rejection. The long-form gotcha is the
`csel ... eq` polarity after `tst`: eq means the tested top bit is clear. Next:
port/connect the
mode-specific table builders after this header (`0x110e540` for mode 0 and
`0x110f3c0` for mode 1) so segment descriptors no longer come from capture
fixtures, then run the B2-CP1 checkpoint full gate.
Sample green: **27 `mc::geometry` lib unit**; `verify_segment_header.py` and
`verify_segment_dispatch.py` green. Last checkpoint full-suite baseline remains:
All green: **163 lib unit** (incl. **17** `mc::geometry`) + all integration;
clippy `--all-targets` clean; `--no-default-features` builds.

### 2026-06-03 - MeshCodec B2-CP1 chunk three-lane segment decoder
**Committed:** B2-CP1 chunk 1b ports `0x110ef70` as
`geometry::rans_three_lane_decode_into` and wires mode 1 through
`geometry::rans_segment_dispatch_into`. The decoder uses three table readers:
readers 0 and 2 reload little-endian u64 words and post-decrement by
`(bitpos >> 3) ^ 7`; reader 1 applies the `rev` load and post-increments by the
same expression. Main groups decode 12 symbols (four from each reader), then the
tail reload at `0x110f1f8..0x110f380` emits `count % 12` in reader order.
Durable local evidence: refreshed `capture_segment_dispatch.py` now stores
per-model payload bytes in gitignored `segment_dispatch_capture.json`, and
`verify_segment_dispatch.py` replays all dispatch modes. Replay status:
mode 0 12/12, mode 1 17/17, mode 2 4/4. Fixture-free tests cover Animal_Bass
dispatch 13 (`count=12`, main loop, log 3) and Animal_Dragonfly dispatch 6
(`count=2`, tail, log 1), including final reader state writeback and truncated
payload rejection. Next: port/connect the `0x110de80` table-build path so the
segment descriptor is produced from the reverse header rather than supplied by a
capture, then run the B2-CP1 checkpoint full gate.
Sample green: **22 `mc::geometry` lib unit**; `verify_segment_dispatch.py` green.
Last checkpoint full-suite baseline remains: All green: **163 lib unit** (incl.
**17** `mc::geometry`) + all integration; clippy `--all-targets` clean;
`--no-default-features` builds.

### 2026-06-03 - MeshCodec B2-CP1 chunk segment dispatch wrapper
**Committed:** B2-CP1 chunk 1a ports the observed `0x110de00` segment
dispatch wrapper as `geometry::rans_segment_dispatch_into` for mode 0 rANS and
mode 2 RLE, with a typed guard for observed-but-unported mode 1 (`0x110ef70`).
Durable local evidence: `capture_segment_dispatch.py` and gitignored
`local-assets/re/segment_dispatch_capture.json`; replay script
`verify_segment_dispatch.py`. Enumerate-all across Bear/Bass/Dragonfly found 99
`0x110de80` table builds and 33 `0x110de00` dispatches. Dispatch population:
mode 0 = 12, mode 1 = 17, mode 2 = 4; logs {1,3,4,5,6,8,9,10,11}; strides {1,3};
count mod 4 covers {0,1,2,3}; RLE values {0,11}; mode 0 states are warm in the
observed dispatcher population. Replay status: mode 0 reproduces 12/12 output,
final states, and stream cursor usage; mode 2 reproduces 4/4 strided/dense fills;
mode 1 is guarded 17/17 until `0x110ef70` is ported. Fixture-free tests cover
Bear mode 0 output/cursor/final-state writeback, Dragonfly mode 2 RLE, and the
mode 1 typed guard. Next: port `0x110ef70` three-lane decoder, then remove the
guard and complete the `0x110de80` table-build plus dispatch wrapper checkpoint.
Sample green: **20 `mc::geometry` lib unit**; `verify_segment_dispatch.py` green.
Last checkpoint full-suite baseline remains: All green: **163 lib unit** (incl.
**17** `mc::geometry`) + all integration; clippy `--all-targets` clean;
`--no-default-features` builds.

### 2026-06-03 - MeshCodec CP3 chunk RLE fill
**Committed:** CP3 chunk 3b ports `0x110f930` as `geometry::rans_rle_fill`.
It fills `count` u16 symbols at `out[i*stride]`, preserving sibling lanes in
the product-sized caller buffer and returning typed errors for zero stride or
undersized output. Durable local evidence: `capture_rle_fill.py` and gitignored
`local-assets/re/rle_fill_capture.json`. Observed population: Bass
`value=0,count=2,stride=3`; Bass `value=0,count=322,stride=3` twice; Dragonfly
`value=11,count=3,stride=1`; Bear has no RLE fill calls. Fixture-free tests cover
the strided Bass fill, dense Dragonfly fill, and defensive rejection. Next: CP3
chunk 3a/3c table-build dispatch and segment loop (`0x110de80`/`0x110de00`,
`0x110dc30`) around the now-ported leaves.
All green: **163 lib unit** (incl. **17** `mc::geometry`) + all integration;
clippy `--all-targets` clean; `--no-default-features` builds.

### 2026-06-03 - MeshCodec CP2 stride rANS decode output layout
**Committed:** CP2 ports the observed stride output behavior of `0x110e270`.
`geometry::rans_decode` is now fallible and strided via `RansDecodeSpec`, with
`rans_decode_into` for preserving sibling lanes in the caller's output buffer.
The Bass stride case is fixture-free: `prod=960`, decoded `w2=320`, `stride=3`,
`log=5`; it writes symbol `i` to `out[i*3]`, consumes 16 forward renorm bytes,
and leaves the other two lanes untouched for sibling streams. Durable local
evidence: `capture_decode_stride3.py` and gitignored
`local-assets/re/decode_stride3_capture.json`. Enumerate-all found one stride>1
`0x110e270` call (Bass) and `0x110ef70` calls only with output stride 1 in the
three-fixture population. Next: CP3 per-segment orchestration
(`0x110de80`/`0x110de00`, RLE `0x110f930`, segment loop `0x110dc30`).
All green: **162 lib unit** (incl. **16** `mc::geometry`) + all integration;
clippy `--all-targets` clean; `--no-default-features` builds.

### 2026-06-03 - MeshCodec CP1 second-model rANS init cold golden
**Committed:** CP1 chunk 1a adds a second-model, fixture-free cold-start golden
for `geometry::rans_init_states_with_cursor` (`0x110dfa0`): Animal_Bass call 0
at stream `P+394`, `flag=0`, `log=7`, `prod=568`, freqs `[6,118,3,1]`.
The inline test asserts the byte-exact cold output, real cursor use
`soff 0->58`, and a warm-only zero-state discriminator that produces different
states. Durable local evidence: `capture_init_all.py`, `verify_init_invariant.py`,
and gitignored `local-assets/re/init_bass_p394_golden.json`
(`capture_init_bass_golden.py`). Replay remains **16/16** init calls across
Bear/Bass/Dragonfly; scalar `prod<4` and `prod&3` tail remain typed-error guards
because the enumerate-all population still does not hit them. Next: CP2,
`0x110ef70` / stride-3 rANS decode for Bass (`count=960`, `w2=320`, `stride=3`).
All green: **161 lib unit** (incl. **15** `mc::geometry`) + all integration;
clippy `--all-targets` clean; `--no-default-features` builds.

### 2026-06-03 — MeshCodec `0x110dfa0` generic rANS init completed
**Committed:** `geometry::rans_init_states_with_cursor` now covers the generic
four-lane rANS init primitive (`0x110dfa0`) instead of only the warm, offset-0
slice. Added `RansStateBuffer` (`x0` states + flag) and `RansStreamCursor`
(`[x2+12]`) so cold-start and continuation calls share the same semantics as
the game. The cold loader (`0x110e1bc`) is derived from `bics w10,w10,w9` +
`b.ne` at `0x110dfbc..0x110dfc4`: it runs when `flag & 0xf != 0xf`, reads
one nibble/byte-count varint per missing lane, adds `0x80000000`, stores lanes
selected by `w10 & -w10`, then sets `flag |= 0xf` at `0x110e264`. The stream
cursor writeback maps to `0x110e1a0..0x110e1a8`.

Validation: reran `capture_init_all.py` and updated the local
`verify_init_invariant.py` reference to include cold load + shared cursor;
it now replays **16/16** init calls across Bear/Bass/Dragonfly (was 10/16).
Fixture-free tests added: Bear `flag=0` cold call at `P+1312` and the
immediate continuation on the same stream (`soff 0→135→187`), with
discriminating checks that a warm-only implementation and a reset-to-zero
cursor both fail; malformed cold-loader input rejects with `StreamTooShort`.
Still guarded because not observed in the init population: scalar `prod<4`
(`0x110e140`) and `prod&3` tail (`0x110e128`). Next: segment loop
`0x110dc30`, `0x110ef70` stride-3 decode, and `0x110f930` RLE.
All green: **160 lib unit** (incl. **14** `mc::geometry`) + all integration;
clippy `--all-targets` clean; `--no-default-features` builds.

### 2026-06-02 — Pure-Rust zstd: lifted `src/zstd_pure/` out to the external `zstd-pure` crate
Replaced the in-tree, decoder-only `src/zstd_pure/` with the now-completed
external **`zstd-pure`** crate (git, pinned `fb3b07d`: decode + encode,
dictionaries, magicless frames, block API; MIT, `std`+`thiserror`). **No
libzstd / C at runtime** — `zstd` (libzstd) is demoted to a **dev-only test
oracle**. Rewired `mc::codec` (magicless BFRES extract/repack via
`decompress_magicless` / `compress(.., expect_magic=false)`), `compression::zstd`
(`.zs` + dict via `Dictionary::parse` / `decompress_with_dict` /
`compress_with_dict`; kept the hand-rolled `frame_dictionary_id`), and
`lib.rs` (`pub mod zstd_pure` → `pub use ::zstd_pure`, so `crate::zstd_pure`
paths in `mc::geometry` + tests are unchanged). The game frames' advisory
dict-id is simply ignored when no dictionary is supplied (the libzstd
streaming-decode workaround is gone). **Validation (byte-exact):** the 3 real
`.mc` fixtures decode identically with the pure path, libzstd, AND the
reference-decompressor oracle (`tests/zstd_pure_bfres.rs`); new
`tests/zstd_pure_corpus.rs` round-trips the pure codec and cross-checks libzstd
**both directions** over the full **12,395-file / 2.9 GiB** oracle corpus — all
byte-identical (heavy → `#[ignore]`d; run `--release --ignored`). Stale
libzstd / "executable dictionary" docs swept (mc/mod, compression/mod, README,
todo, TRUST_MATRIX, this file).
All green: **157 lib unit** (incl. 11 `mc::geometry`; was 167 — the ~10 in-tree
`zstd_pure` unit tests left with the deleted module, now covered by the external
crate + the corpus/BFRES integration tests) + all integration; clippy
`--all-targets` clean; `--no-default-features` builds.

### 2026-06-02 — MeshCodec vertex rANS contiguous spread ported (`0x110e6f8..0x110e7a4`)
**Committed:** `geometry::rans_spread` + `RansDecodeTable` — fills `step[M]` and
`sym[M]` after freq decode: `sym[cum..)=s`, `step=(f<<16)|(slot_in_symbol)`.
Mapped to `0x110e6f8` (pair loop when `f>=2`, `add` by `0x200000002`) and tail
at `0x110e744`. Subtlety: low 16 bits are the index within the symbol's run, not
the global table index. Fixture-free: Bear first rANS M=64 freqs
`[5,1,1,0,1,0,1,1,0,1,3,6,13,23,8]` → golden `step`/`sym` from `trace_rans.py` /
`vtxgt/rans/`; `rans_spread_then_decode_bear_first_rans` chains spread + existing
decode golden (init states still hardcoded — `0x110dfa0` blocked: init stream at
P+8044, body at P+8068, `0x110e1bc` nibble path when `ctx+0x20&0xf!=0`).
**Next:** port `0x110dfa0` 4-state init, then segment loop / 3-lane / RLE.
All green: **166 lib unit** (incl. `mc::geometry`) + all integration; clippy
`--all-targets` clean; `--no-default-features` builds.

### 2026-06-02 — MeshCodec vertex rANS frequency reader ported (`0x110e7b0`)
**Committed:** `geometry::rans_read_freqs` + `RansFreqReader` / `RansFreqParams` /
`RansFreqRead` — the adaptive `clz`-prefix frequency decoder that feeds each
per-segment rANS table build (`0x110de80`). Three validated code paths mapped to
disasm: slow adaptive loop (`0x110e7f8`), `clz`-coded run length (`0x110e890`/
`0x110e8e8`), fixed-width run body (`0x110e900`). Subtlety: slow-site top-nbits
uses `mvn w19,w3` **before** `add w3,#1`; run body uses `(acc>>1)>>~width`, not
`acc>>(64-n)`; short run-length path (`0x110e8e8`) clears `w18` but must not zero
`w1` (symbol count). Fixture-free goldens from `trace_freq_all.py` /
`freq_golden.py`: Bear call #0 M=512 `[95,408,7,1]+rem=1`; Bear call #2 (all
paths) `[9,496,3,2,1]`; Bass call #21 M=128 `[6,118,3]` — each checks freqs and
the advanced `(ptr,acc,bitpos)` cursor. **Next:** rANS spread + 4-state init
(`0x110dfa0`), segment loop, then width combiner + kernel transform.
All green: **164 lib unit** (incl. `mc::geometry`) + all integration; clippy
`--all-targets` clean; `--no-default-features` builds.

### 2026-06-01 — MeshCodec geometry transport ported to Rust (`src/mc/geometry.rs`); bufA 99.3% from scratch + vertex coder mapped
Turned the validated Python index framing into a **committable clean-room Rust
module** and mapped the vertex coder concretely. Two outcomes:

**Committed: `src/mc/geometry.rs` — the FMSH geometry transport primitives.**
Forward (MSB var-int) + reverse readers; super-block trailer; sub-block header
(`0x10f9570`); the **state-0 canonical-Huffman table-builder cursor math**
(`0x10f8d20`/reverse-A `clz` reader — the hard piece, ported byte-exact: Bear
`fwd→15`, `revA→P+32807`, `bitpos→50`, `w8=3327`, `symbols=8`, `dir=1`); and the
**window primitive**. Key correction vs the prototype: a zstd "window" is **not**
a bare literals block — it's a full zstd **block content** (literals **+
sequences** that RLE/back-ref-expand, e.g. the index code stream = a few `0xf0`
literals → 131072 bytes), so it decodes via `zstd_pure::block::BlockState::
decode_compressed` with an external `0x20000` ceiling (the decoder's `0x5ffb90`),
not `literals::decode`. Window located by a forward var-int = `srcsize`; raw vs
zstd is one reverse-A flag bit (code=zstd, data=raw). End-to-end test reproduces
**Bear `bufA[0:16540]` (99.3%) byte-exact from scratch** (super-block → table
builder → code window via `zstd_pure` → raw data window → per-sub-mesh
`meshopt::decode_index_buffer_split_used` ×2, align_a-aligned) — no emulator, no
oracle file (golden bytes hardcoded). Added `meshopt::decode_index_buffer_split_used`
(returns consumed code/data counts) so sub-meshes chain from one shared stream.
All green: **160 lib unit** + all integration; clippy `--all-targets` clean;
`--no-default-features` builds.

**RE: the custom VERTEX byte-group coder structure mapped** (`vtx_trace2.py`
ground truth). bufB = **two attribute-grouped vertex streams**: stream A (3327
verts × 12 B, attrs at cols {0,6,9} widths [6,3,3]) at `bufB[0..39928]`, stream B
(3327 × 16 B, cols {0,4,8,12,15} widths [4,4,4,3,1]) at `bufB[39928..93160]`,
then a 440-B direct window. The 8 `0x10fb2e0` calls each decode ONE attribute
(table entry; offsets in `ctx+0x27c`, cols in `ctx+0x310`) into a ws staging
buffer; the kernel transform (`0x10f9690`→`0x10fa980`) writes transposed
delta/zigzag into bufB. `0x110d7f0`'s 2-bit mode (jump `0x2cf6acc`): 0/1 =
entropy-coded symbol arrays (widths), 2 = zstd-window value stream, 3 =
direct (sentinel bytes); `0x110d360` is the 3-stream byte-group width combiner.

**Committed `0e14f86`: the canonical-Huffman table builder (`0x10f8d20`) is now
ported + validated in Rust** (`state0_table_builder` materializes the full decode
table — `ctx+0x240`/`0x27c`/`0x310`/`0x2d4` + totals — byte-exact vs the oracle for
Bear). Subtlety that cost a wrong first cut: two distinct `w19` — packing/mask use
bits 62-63, but the long/short branch tests **bit 56** and the stream-reset tests
**bit 55** (`0x10f8e00` overwrites `x19` with `x14>>55` before the branch); this
also fixed a latent cursor bug. `w13 = ctx[0x2c0]` (=7 across fixtures, alignment-
like) is a param (index path passes 0).

**Committed `87e8508`: the rANS decode loop (`0x110e270`) is ported + validated**
(`geometry::rans_decode`): `M=2^6`, decode table `step[idx]=(freq<<16)|(idx-
cumfreq)` + spread `sym[idx]`, step `state=(state>>6)*freq+low`, 32-bit forward
renorm at the 2^31 threshold, 4 interleaved states. Byte-exact vs the emulator-
dumped I/O (Bear's first rANS call: table + states + stream → all 228 symbols).
The table's spread is **contiguous** (symbol `s` owns the slot range
`[cumfreq[s], cumfreq[s]+freq[s])`), not the FSE scatter.

**Remaining vertex work:** the rANS **table build** `0x110de80`. Its **frequency
decoder `0x110e7b0` is reversed + verified per-symbol** (FINDINGS UPDATE #8):
adaptive `clz`-prefix code, `nbits = width + 2*clz + 1`, `w18 = val - (1<<width)`,
delta = `unzigzag(w18)` from the previous freq (init `M//nsym`), plus the adaptive
`width` update — all fields match the trace (Bear M=512 → `[95,408,7,1,1]`). The
only blocker for the freq port is the reverse bit reader's exact reload/step
(refill-traced in `trace_refill.py`; `rans_freq_port.py` reproduces freq[0] then
diverges). Then the contiguous spread + 4-state init + the 3-lane (`0x110ef70`)/
RLE (`0x110f930`) variants + segment loop `0x110dc30`; then the 3-stream width
combiner `0x110d360`, the kernel transform (delta/zigzag/transpose), and states
4/5/2. Disasm `_symdec.txt`/`_tblbuild.txt`; ground truth `_rans.txt`/`_freq*.txt`/
`vtxgt/`; ports `tbl_port.py`/`rans_port.py` (FINDINGS UPDATE #8).

### 2026-06-01 — MeshCodec INDEX transport framing VALIDATED (prototype) + vertex coder ground truth
Continued the Stage-1b port. Built a Python framing prototype (gitignored,
`local-assets/re/decode_proto.py`) and **validated the entire index transport
framing byte-exact vs the emulator/oracle**: super-block trailer, `w27`, the
sub-block header, the **custom-Huffman table builder `0x10f8d20` + reverse-A
`clz` bit reader** (the hard piece — cursor transition `fwd P+3→P+15`,
`revA P+32825→P+32807`, `bitpos 0→50` all exact), window location via the
forward srcsize var-ints, and the multi-sub-mesh index decode (count#1=DESC.f,
count#2+=the transform-loop var-int `v28`). Result: Bear's **`bufA[0:16540]`
(99.3%) reproduced exactly** via clean-room framing feeding
`decode_index_buffer_split`. The exact reverse-reader + table-builder algorithm
is documented in `FINDINGS.md` (UPDATE #7).

Then **mapped + captured ground truth for the remaining piece, the custom vertex
byte-group coder** (`0x110d7f0` 4-mode symbol reader + `0x10fb2e0` block decoder
+ `0x10fafe0` setup + the `0x10f8d20` table values + states 4/5/2). It's the
meshopt vertex transform with a canonical-Huffman entropy backend — ~10
interlocking functions, comparable to/larger than the index side. `vtx_probe.py`
dumped the table (count=8, the arrays), the 8 `0x10fb2e0` block-decoder calls,
the 4 width sub-decoders (streams 1267/3583/3583/1285), and saved the staging
window binaries + oracle `bufB` to `local-assets/re/vtxgt/` (bufB=93600 = 93160
byte-group-decoded + 440 direct-window tail). This is the concrete spec to port
`0x10fb2e0` next. No shippable code this session (RE/prototype/docs only; the
committed `decode_index_buffer_split` from `8daf7b8` remains the artifact);
`cargo test`/clippy unchanged + green.

### 2026-06-01 — MeshCodec transport fully mapped; INDEX geometry ported (split streams, **meshopt v0**)
Drove the emulator (`local-assets/re/emu.py`, gitignored) ground truth into a
clean-room Rust building block + a precise, validated map of the remaining work.
Built comprehensive **emulator tracers** (`trace.py`/`idx_check.py`/
`dump_vtxwin.py`, gitignored) that dump the COMPLETE primitive-op sequence the
real decoder runs: every Huff0/raw **window** (src, srcsize, regen, dst, caller),
every meshopt call (code/data/out), the state-machine entry/state, and each
sub-block descriptor.

**Committed increment — `meshopt::decode_index_buffer_split`.** The MeshCodec
index decoder `0x110c280` is `meshopt_decodeIndexBuffer` fed **split** code/data
streams (not one contiguous buffer) with the standard codeaux table. Refactored
`src/meshopt/index.rs` to share a `decode_index_core`, adding the split-stream
entry. **Correction (was wrong in prior notes):** these streams decode as meshopt
**version 0** (`fecmax=15`), NOT v1 — proved by a `0x0d`/`fec=13` code being a
FIFO read, not the v1 `last−1` code. Validated **byte-exact vs the oracle** on
every index sub-mesh of Bear (6606/1662/60) and Bass (1230/315/48/3) via the
emulator-dumped real streams; committed fixture-free tests cross-check it against
`decode_index_buffer`. Per-sub-mesh `count = 3·v` (forward var-int).

**Mapped the transport precisely** (FINDINGS UPDATE #7): the window primitive
(reverse bit reader picks raw-memmove vs zstd-`0x5ffb30`=`zstd_pure::literals`;
forward var-int = srcsize; regen ≤ 0x20000) with 3 call sites; the full Bear
window table; the ctx/workspace layout (`ctx[0xf8]`=bufB base, `ctx[0x100]`=DCtx,
desc@`ctx[0x1d0]`, state@`ctx[0x320]`, 6-state jump table `0x2cf6950`); the
single-call state-machine loop (`w27` sub-blocks; header `0x10f9570`).

**Confirmed the VERTEX path is CUSTOM** (answers the open question): no window
starts with `0xa0`, and the decoder (`0x10fae60`/`0x10fafe0` + table builder
`0x10f8d20` + symbol reader `0x110d7f0`) is a canonical-Huffman byte-group coder,
NOT stock meshopt vertex. So `src/meshopt/vertex.rs` is a reference, not a
drop-in; porting that custom entropy is the bulk of the remaining work. NEXT:
port the framing loop to drive `decode_index_buffer_split` → reproduce **bufA**
end-to-end first (fully understood), then the custom vertex coder → **bufB**.
All green: **156 lib unit** (+2 split-stream) + all integration; clippy clean;
`--no-default-features` builds.

### 2026-06-01 — MeshCodec mesh decode SOLVED via emulation (ground-truth oracle + confirmed architecture)
Major RE milestone. Built a Unicorn ARM64 harness (`local-assets/re/emu.py`,
gitignored — runs game code, not shippable) that executes the **real** mesh
decoder (`0x6c6bf0`) from the NSO and reproduces the `mesh-codec-output` oracle
**byte-perfectly** for all 3 fixtures (bufA+bufB exact). Setup: map the 3 NSO
segments, parse MOD0/`.dynamic` (MOD0 VA at `text[4]`), apply R_AARCH64_RELATIVE
relocs, resolve `.rela.plt` imports to Python sentinels (stub `memcpy`/`memmove`/
`memset` for real; `nn::util::ReferSymbol` + FPCR `mrs/msr` as no-ops); the
decoder's allocator is a self-contained bump allocator on a caller-provided
workspace (size = FMSH `+0x08`), so no alloc stub is needed.

**Architecture CONFIRMED (this is the corrected, final picture):** the geometry
codecs are **stock meshoptimizer 0.15** wrapped in a **custom Huff0-windowed
transport**. Instrumentation proved it: the index decoder `0x110c280(out, count,
&code, &data, mode=1)` gets a **code stream of all `0xf0`** + a var-int **data
stream**, with per-sub-mesh `count`s all `%3==0` (Bear 6606/1662/60/…) — textbook
`meshopt_decodeIndexBuffer` with split code/data streams. The transport
decompresses the FMSH sub_a/sub_b into workspace streams via **zstd Huff0 literals
windows** (`0x5ffb90`, = `zstd_pure::literals`; ~7 windows for Bear) under the
6-state machine (`0x10f8aa0`) + sub-block headers (`0x10f9570`, `w27`≈14 blocks).

**So the shippable pure-Rust port reuses both existing primitives** — `zstd_pure`
(Huff0 windows) + `src/meshopt` (geometry; index needs a split-(code,data)-stream
entry) — plus the transport framing/state-machine, **validated step-by-step
against `emu.py`** (which can dump every intermediate). Full RE map + the
emulator recipe in `local-assets/re/FINDINGS.md` (UPDATE #6). No code committed
this milestone (RE/validation only); the Rust transport port is the remaining
(now fully de-risked) deliverable.

### 2026-06-01 — MeshCodec: FMSH framing parser (`src/mc/mesh.rs`) + custom-entropy correction
Continued Stage 1b. Two outcomes: a committable framing parser, and an important
RE correction about the inner codec.

**RE correction (the inner entropy is CUSTOM, not stock meshopt byte-groups).**
Disassembling the actual decode path — `0x6c71e0` (super-block driver: validates
`ctx[0x38]+ctx[0x3c]`, reads two LSB-first LEB128 next-block sizes `(0,0)`=last,
hands the decoder forward+reverse cursors for sub-stream A=`payload[2..sizeA]`
and B=`payload[sizeA..]`) and the inner kernel `0x10fa980` — shows a **`clz`-based
variable-length bitstream** with a 64-bit bit reader, `×3`/`÷3` triangle counts,
and raw/zstd-block windows (`0x5ffb30`). Stock meshopt 0.15 has no `clz`/unary
(it uses fixed 0/2/4/8-bit byte groups), so `NintendoWare_Meshoptimizer_For_
MeshCodec` keeps meshopt's **geometry transforms** but swaps in a **custom
entropy backend**. Net: `src/meshopt/` is a correct reference codec + encoder,
but the full decode needs that custom backend ported (larger than last entry
implied). Doc claims softened accordingly.

**Committable increment: `src/mc/mesh.rs`** — the FMSH **framing** parser (the
verified container layer the decoder will plug into): `has_mesh_flag`
(`bfres[0xEE]&8`), `read_mesh_section` → `MeshSection`/`MeshChunk` (FMSH 34-byte
header: version, workspace hint, compressed payload size, bufA/bufB decoded
sizes, aligns; first-chunk descriptor `kind=u16&3`/`val=u16>>2` + two u24
sub-stream sizes; payload offset). Detection by FMSH magic at the 4-aligned
position after the BFRES frame; returns `None` for mesh-less resources. Surfaced
via `mc-inspect --mesh` (text + JSON). **Verified on all 3 real `.mc`** (Bass/
Bear/Dragonfly: `sub_a+sub_b==comp_sz`; `sub_a→vertex bufB`, `sub_b→index bufA`;
`kind=2`,`val=33` constant) + 3 fixture-free tests (synthetic frame+FMSH parse,
no-FMSH→None, inconsistent-sizes→typed error). `cargo test` = **154 lib unit +
all integration, 0 failures**; `clippy --all-targets` clean; `--no-default-
features` builds.

**NEXT:** port the custom entropy decoder (`0x10f8aa0` 8-state machine + kernels
`0x10fa980/ab60/acf0` + `0x10fae60`/`0x110ca30`/`0x110c280` + dual fwd/rev bit
readers + zstd-block windows) → decode sub_a→vertex / sub_b→index, apply meshopt
transforms, assemble `[info][bufA][bufB]`+pad, validate FULL decode == oracle.

### 2026-06-01 — MeshCodec mesh codec is **meshoptimizer 0.15**; built `src/meshopt/` (Stage 1b primitive)
Continuing the mesh-geometry decode. **Key discovery (corrects the Stage-1b
roadmap):** the FMSH chunk codec is **not** "the Huff0/raw/RLE primitives in
`zstd_pure`" — it is **meshoptimizer 0.15**. The executable embeds the version
string `SDK MW+Nintendo+NintendoWare_Meshoptimizer_For_MeshCodec-0_15_0-Release`
(rodata `0x3568316`). So the trailing geometry is meshopt (MIT, `zeux/
meshoptimizer`) vertex/index buffers wrapped in a **custom Nintendo streaming
container**; zstd is the *transport* for the meshopt byte-planes, not the
geometry codec.

**RE'd the full decode call graph** (in `local-assets/re/FINDINGS.md`):
`0x6c6bf0` (driver: bufA@out_dest, bufB@align_up) → `0x6c6cd0` (u16 chunk header
`type=u16&3`,`val=u16>>2`) → dispatcher `0x10f8860` (returns sizeA+sizeB, the two
u24 sub-stream lengths) → factory `0x10f8920` (type 1→`0x110bab0`, 2→`0x10f8950`,
0→`0x110b9e0`) → type-2 vtable `[0x10]`=`0x10f897c` INIT (allocates a **zstd
DCtx**, `0x276d0` ws) and `[0x18]`=`0x10f8aa0` DECODE (a streaming coroutine:
MSB-first var-int window sizes + a 64-bit bit reader + raw/zstd-block windows via
**`0x5ffb30`**, the same zstd family as `zstd_pure`, fed through meshopt transform
`0x10f9690`). Output = `[288-byte info header = [abs bufA off][capacity]][bufA=
index][bufB=vertex]` + zero pad (re-confirmed on Bear: `[17536][131072]`).

**Built `src/meshopt/`** — a clean-room, crate-extractable (std + `thiserror`
only) port of the **stock meshopt 0.15** codecs (a reference codec + encoder;
see the next session-log entry — TotK's actual decode uses a *custom* entropy
backend, so this is a foundation, not a drop-in): `vertex`
(`encode/decode_vertex_buffer`, `0xa0`: byte-group planes,
zigzag deltas, first-vertex tail), `index` (`encode/decode_index_buffer`, `0xe0`
v0/v1: vertex/edge FIFOs + codeaux table; and `encode/decode_index_sequence`,
`0xd0`), `read_indices`. **Validated:** exact-format vectors anchored to the spec
byte layout + synthetic round-trips (multi-block vertex, grid+random index both
versions, sequence) + **real-data round-trips on the oracle's decoded vertex/
index buffers for all 3 `mc` fixtures** (`decode(encode(x))==x` losslessly on
real TotK bytes; `dump_bufs.py`). `cargo test` = **151 lib unit + all integration,
0 failures**; `clippy --all-targets` clean; `--no-default-features` builds.

**NEXT (Stage 1b, next session):** crack the Nintendo streaming framing
(`0x10f8aa0`) to reconstruct the stock meshopt streams from the FMSH chunk
payload, run `src/meshopt/` on them, assemble `[info][bufA][bufB]+pad`, validate
the FULL decode == the `mesh-codec-output` oracle, sweep the 12,395 corpus; then
re-encode (Stage 2). Open Qs: `val`→vertex_size/index_size/count mapping; whether
Nintendo modified the meshopt kernels vs stock (confirm by decoding a real
reconstructed stream).

### 2026-06-01 — Pure-Rust Zstandard decoder (`src/zstd_pure/`) — MeshCodec mesh-codec Stage 1a
Start of the **MeshCodec mesh-geometry** codec (the big open item). RE of the
TotK `main` decoder showed the mesh stream is **Nintendo's own reimplementation
of Zstandard**: the state machine `0x6c6da0` ≈ `ZSTD_decompressStream` (states:
0/1 frame header, 2 block header `last|type|size`, 3/4 block decode, 5-7
finalize); `0x6c7330` checks the real zstd magic `0x28b52ffd`; `0x5ffb90` is the
literals section (`type=b0&3`, `sizeFormat=(b0>>2)&3`) calling `0x3c880`
(4-stream Huff0) / `0x3c6f0` (1-stream Huff0) with FSE-coded weights
(`0x3be80`/`0x3f1f0`, tableLog≤6). So the **entropy layer is standard zstd
Huff0/FSE** (implementable from the public **RFC 8478** — no GPL). A direct
libzstd decode of the mesh stream fails (custom outer framing), so the codec
must be ported; but the BFRES half of every `.mc` *is* standard magicless zstd,
giving a free real-data validation corpus.

**Built `src/zstd_pure/`** — a from-scratch, crate-extractable (std + `thiserror`
only) pure-Rust **zstd decoder**, per the user's ask to fill ruzstd's gaps and
be liftable into its own crate later: `bits` (libzstd-faithful reverse
`BIT_DStream` + forward reader), `xxhash` (XXH64, content checksum), `fse`
(`FSE_readNCount` + table build + 2-state `FSE_decompress`), `huff` (Huff0
weight decode [FSE/direct] + 1-/4-stream), `literals` (Raw/RLE/Compressed/
Treeless, table caching), `sequences` (LL/OF/ML predefined+RLE+FSE+Repeat modes,
repeat-offset `ZSTD_updateRep`, LZ execution), `block` + `frame` (header, block
loop, skippable frames, checksum). Public `decompress` / `decompress_capped` /
`decompress_magicless`. **Validated:** matches libzstd byte-for-byte across 4
input profiles × levels {1,3,9,19} + empty/tiny + a content-checksum frame (10
lib unit tests); and `tests/zstd_pure_bfres.rs` decodes the **real** BFRES frame
of every `tests/fixtures/mc/*.mc` identically to libzstd (bytes + consumed
length). `cargo test` = **143 lib unit + all integration, 0 failures**; `clippy
--all-targets` clean; `--no-default-features` builds.

**Mesh container framing — also RE'd this session (not yet ported; full detail
in `local-assets/re/FINDINGS.md`):** orchestrator `0x6c6334` decodes the BFRES
frame, then checks **bit 3 of BFRES byte `+0xEE`** (the has-mesh "external
flags" byte). The **FMSH header** is **34 bytes**, 4-aligned after the BFRES
(0-3 bytes of `00` pad): `'FMSH'` + u32 version + u32(?) + u32 compressed-
payload-size (`+0x0C`) + u32 buffer-A size (`+0x10`) + u32 buffer-B size
(`+0x14`) + align bytes + an 8-byte first-chunk descriptor. The decoded mesh =
`[~288-byte info header][buffer A][buffer B]` then zero-pad to capacity (proved:
Bear `mesh_out[0..8]` = `[17248+288][131072]`, and 288+16664+93600 = 110552 =
the non-pad mesh length). Mesh decode = `0x6c6bf0` → per-chunk `0x6c6cd0` (u16
header `type=u16&3`/`val=u16>>2`, two 24-bit sizes) → **polymorphic dispatcher
`0x10f8860`** (factory `0x10f8920` by type → vtable decode), almost certainly
the same Huff0/raw/RLE primitives now in `zstd_pure`. **Next (Stage 1b):**
follow the type→decoder vtables (`0x110bab0` …), decode each chunk with
`zstd_pure`, place at aligned offsets, validate the FULL decode == the
`mesh-codec-output` oracle and sweep the corpus; then re-encode via RAW blocks.

### 2026-06-01 — TotK MeshCodec (`.mc`/MCPK) extract + repack pipeline (SOLVED for models)
The user asked for a cautious, test-driven attempt at a trusted TotK model
repack pipeline (`mc-extract` → edit → `mc-repack` → `restbl-update-dir`). Done,
in the disciplined order, all green; the key technical question — *is `.mc`
decompression doable?* — is **yes** for model `.bfres.mc`.

**Phase A — MCPK container inspect + verbatim no-op round-trip (`db746df`).**
`src/mc/` (`mod`/`read`/`write`/`error`). Header verified vs the oracle on 310
files: magic `MCPK`; `+5` flags ≤1; `+6` reserved 0; `+8` size descriptor →
`(d>>5)<<(d&0xf)` = the alignment-padded decompressed size. `write_mc` verbatim.
Verbs `mc-inspect` + `mc-roundtrip-test` (`--dir`). **All 12,395 TotK `.mc` parse
+ round-trip byte-identically.**

**Phases B+C — extract + repack (`e205718`).** *Breakthrough:* the MCPK inner
stream is **plain magicless zstd needing NO dictionary** for model `.bfres.mc`
(decompressing `mc[+0xC..]` with `ZSTD_f_zstd1_magicless` reproduces the BFRES
exactly). The executable-dictionary lead (a real streaming state machine at
`main` `0x6c6da0`, custom block parse at `0x5ffb90`) is the *secondary* FMSH/
external path, NOT the main model payload. Gotcha: the frames carry an advisory
dict-id that libzstd's **one-shot** decode rejects ("Dictionary mismatch") but
the **streaming** decode tolerates (use `ZSTD_decompressStream`). `src/mc/codec.rs`
uses the existing `zstd` dep with the new `experimental` feature (for
`FrameFormat::Magicless`) — no dictionary, no new deps. **Scope correction:** a
model `.mc` = `[BFRES frame: magicless zstd] + [mesh buffers: a CUSTOM MeshCodec
encoding, NOT zstd]`. `mc-extract` decodes the first frame = the BFRES
**structure** (complete valid BFRES; geometry buffers are in the undecoded
custom tail), **byte-identical to the reference decompressor's BFRES portion**
(496 Python + 104 Rust). `mc-repack` re-encodes the BFRES and **preserves the
original mesh tail verbatim** (edited structure + original geometry; same-size
edits only, `--allow-resize` to force); `extract(repack(x))==x`. The custom mesh
codec (decode/encode) is the unsolved hard part (the community decodes it only
via the game's own code).

**Phase E — `restbl-update-dir` (`d08d456`).** Scans a mod folder, computes each
resource's decompressed size, bumps the RESTBL **growing-only** (under-allocation
crashes). RESTBL key = path with the compression ext stripped (`X.bfres.mc` →
`X.bfres`, verified); sizes carry ~1.9x BFRES overhead, so with `--romfs-base`
it scales the original entry proportionally (accurate; unchanged files keep
their size), else over-estimates conservatively. Verified on the real
379,715-entry table.

`cargo test` = **133 lib unit + 224 total across all binaries, 0 failures**;
`clippy --all-targets` clean; `--no-default-features` builds. **Untestable here:**
in-game acceptance of repacked `.mc` (no hardware). RE notes in
`local-assets/re/FINDINGS.md`.

### 2026-05-31 — Reliability hardening pass (trust matrix + invariants + negatives + corpus-audit)
A quality/reliability pass (no new format features) to make each CLI verb earn
an explicit support tier. Five phases, committed in batches; all green.

**Phase 1 — `TRUST_MATRIX.md` (new, tracked).** Inventories all 57 verbs by
format: read-only vs writing, the output contract (byte-identical / semantic /
inspect / mutate / lossless-recompress), current coverage (corpus / fixture-free
unit / negative / mutation diff-shape), a trust tier (Trusted / Validated /
Experimental / Inspect-only / Lossless-not-byte-identical), and a concrete
"to reach Trusted" checklist. **This is the source of truth for trust status.**

**Phases 2–3 — invariants + negatives (8 new lib tests).** Fixture-free
malformed-input tests for the parsers that lacked them (`bntx`/`bflyt`/`bflan`
→ typed errors, no panic). Mutation **diff-shape** tests for `msbt`
(`set_message_by_label` leaves unrelated messages/labels/sections byte-stable)
and `restbl` (`set_by_hash` changes only the target; name table + order stable;
miss = no-op) — joining the existing `byml-set` exactly-one-diff. **Canonical
idempotency** tests for `byml`/`msbt`/`aamp` (`read→canonical→read→canonical`
is byte-stable).

**Phase 4 — `corpus-audit` (new module + verb).** `src/corpus_audit.rs` walks a
romfs/root (recursing into SARC, inflating `.zs`/`.szs`), classifies by content
magic, runs the safest op per format, and tallies per-format byte-identical /
semantic / inspect / expected-unsupported / unexpected-fail → a JSON manifest
(tool_version, git_commit, ISO-8601 times, versions/endian/encoding, failures[]).
Read-only; nonzero exit on any unexpected failure. 6 fixture-free unit tests +
smoke-tested on `tests/fixtures/{bfres,restbl,aamp}` (42 files, all
byte-identical).

**Phase 5 — promotions.** **Trusted:** `bflyt-roundtrip-test`,
`bntx-roundtrip-test` (documented C#-tool exceptions), `byml-roundtrip-test`
(TotK), `byml-set`, `aamp-roundtrip-test` (BOTW), `bfres-roundtrip-test`.
**Validated** (concrete gaps noted in the matrix): `msbt-import-json` (was
Experimental), `corpus-audit`, the bflan/restbl/msbt round-trips. Inspect verbs
stay **Inspect-only** (their parsers are corpus-trusted). `cargo test` =
**125 lib unit + 212 total across all binaries, 0 failures**; `clippy
--all-targets` clean; `--no-default-features` builds. Remaining for more Trusted
verbs: typed `BflanError`; BOTW `RSTB`; MSBT BOTW/non-v3 unsupported-or-pass;
`aamp-set` exactly-one-diff; recorded real-romfs `corpus-audit` runs.

### 2026-05-31 — BFRES (FRES) inspect + verbatim round-trip; MeshCodec `.mc` investigated/deferred
Roadmap item #2 (BFRES inspect-only). Done in the disciplined order:
read/inspect + byte-identical round-trip FIRST, no mutation.

**New `src/bfres/`** (`mod`/`read`/`write`/`error`, typed `BfresError` via
`#[from]`). The `FRES` ResHeader was pinned against real bytes and is consistent
across **BOTW v5 (`0x00050003`)** and **TotK v10 (`0x000A0000`)**, little-endian
(BOM at 0x0C): `0x08` version, `0x10` fileNameOffset (→ name chars, u16 len at
-2), `0x18` relocationTableOffset (→ `_RLT`), `0x1C` fileSize. The reader decodes
those + structurally scans the well-known sub-block magics (FMDL/FSKA/FMAA/FSHP/
FMAT/FVTX/FSKL/BNTX/`_STR`/`_DIC`/`_RLT`/…; 4-byte magics ~never false-positive).
Like BNTX/AAMP the byte layout is offset/relocation-heavy, so the parser is
**inspect-only** and `write_bfres` re-emits the captured bytes → **byte-identical**
by construction. **Verified across 424 real files** (BOTW `.sbfres`, TotK
`.bfres.zs`, and the decompressed v10 model corpus), 0 parse errors / 0 diffs.
**Stage B:** a BOTW `.Tex.bfres` embeds a full BNTX; `BfresDocument::
embedded_bntx_bytes` bounds it by the BNTX's own `file_size` and `bfres-inspect`
surfaces its textures via the existing `read_bntx` (e.g. `Animal_Bass.Tex` → 8
textures: `Bass_Alb` BC1_UNORM_SRGB 128×128 mips=8, …). Verbs `bfres-inspect`
(`--json`, inflates `.sbfres`/`.bfres.zs`) + `bfres-roundtrip-test`. Tests:
`tests/bfres_roundtrip.rs` (fixture-gated: corpus byte-identical + spans both
games + embedded-BNTX parse + pinned `Animal_Bass`/`Animal_Bass.Tex`) + 3
fixture-free `bfres::read` unit tests. `cargo test` green (**108** lib unit + all
integration); `clippy --all-targets` clean; `--no-default-features` builds.

**MeshCodec `.mc` (TotK models) — investigated, deferred (user-approved deep RE).**
TotK ships models as `Model/*.bfres.mc` = MeshCodec (`MCPK`): magicless zstd that
needs a **raw-content dictionary embedded in the game executable** (the user
provided `exefs`; I LZ4-decompressed the 35 MB `NSO0` `main` and confirmed the
dict has **no zstd-dict magic, no `ZSTD`/`MeshCodec` symbol, isn't a string blob**,
and a dictless magicless decode fails). Framing is custom/out-of-band and the
`FMSH` sub-section is community-unsolved (reference tools emit **partial,
non-editable** BFRES); the only complete reference is GPL. Per the user, in-tool
`.mc` decode is being built as an ARM64-RE effort. **Phase 1 done this session:**
NSO0 + LZ4 segment decompression ported to Rust (`src/nso.rs`, MIT `lz4_flex`;
verb `nso-extract`), validated **byte-exact** against the Python-lz4 oracle on all
three `main` segments (text 45 MB / rodata 10 MB / data 5 MB); the `MeshCodec`
string lives in `.rodata` (`0x56b44` / `0x9130c` / `0x91338` / `0x9ae345`).
**Remaining:** disassemble `.text` around those xrefs to locate the raw dict
pointer/size + frame params (window log), then implement magicless-zstd(+dict)
decode in `compression` and validate against the **12,395 decompressed `.bfres`
oracle** the user produced (`local-assets/mesh-codec-output/`). BFRES already
consumes those decompressed `.mc` outputs (all v10, parse + round-trip); the raw
dict is the user's own game data — load at runtime (`--mc-dict`), never commit.

### 2026-05-31 — AAMP (BOTW binary parameter archive) read + round-trip + canonical + set
The user dumped **BOTW** (`01007EF00011E000`), unblocking AAMP (TotK has none).
Done in the disciplined order, three committed batches, all green.

**Fixtures.** BOTW actor params live in `Actor/Pack/*.sbactorpack` (Yaz0 SARC).
`archive-extract` (native Yaz0 + SARC) pulls them out as raw `AAMP` files.
Curated 32 fixtures across 18 extensions (`.bxml`/`.bgparamlist`/`.baiprog`/
`.bphysics`/`.bdmgparam`/…) into the gitignored `tests/fixtures/aamp/`.

**Stage A — read + verbatim round-trip (`ef11b4c`).** New `src/aamp/`
(`mod`/`read`/`write`/`error`, typed `AampError`). Header confirmed on real
bytes (magic `AAMP`, v2, flags 0x3 = LE+UTF-8). Offset-driven recursive parser:
root Parameter IO → list (0xC) / object (0x8) / parameter (0x8), each node's
children/data found via `/4` relative offsets; decodes all 21 `ParameterType`s
(scalars, vec2-4/color/quat, 4 string variants in the string section, buffers
[count at `data_off-4`], curves [raw]). Keys kept as CRC-32 hashes.
`write_aamp` = verbatim → **byte-identical**; verified on **418** real BOTW
files (Link/Guardian/Lizalfos/Gerudo, weapons/armor/animals/objects/treasure/
items), zero parse errors. Verbs `aamp-inspect` (`--json`, `--names` to resolve
hashes) + `aamp-roundtrip-test`.

**Stage B — canonical writer (`6ad1ee6`).** `write_aamp_canonical` rebuilds
from the decoded tree: sections header → lists (BFS, contiguous sibling runs) →
objects → params → data → de-duplicated strings, **4-aligning every data/string
entry** (offsets are `/4`, so each must land on a `/4`-encodable position).
Contract = semantic round-trip (not byte-identical: AAMP layout is
writer-specific). **Semantically lossless on all 418** files. `aamp-roundtrip-
test --canonical` sweeps it.

**Stage C — `aamp-set` (`cf9f257`).** `src/aamp/edit.rs` `set_by_path` edits a
parameter by a `/<lists…>/<object>/<param>` name path (CRC-32-matched; `0x…` =
raw hash), **type-preserving** (parses the value into the param's existing type;
strings keep their str32/64/256/ref kind; curve/buffer rejected), then
canonical-writes. Verb `aamp-set`. Verified end-to-end on a real `.bgparamlist`
(`int(20) → int(99)`, re-inspected). Shared `Value::summary` with the inspector.

Tests: `tests/aamp_roundtrip.rs` (fixture-gated: corpus verbatim byte-identical
+ canonical semantic round-trip + pinned `Weapon_Sword_001` structure + a real
Int `set_by_path` round-trip) + fixture-free unit tests in `aamp::read`/`write`/
`edit`. `cargo test` green (**105** lib unit + all integration); `clippy
--all-targets` clean; `--no-default-features` builds. **Follow-ups:** a name
table for readable inspect by default; curve control-point decode (raw today);
AAMP add/remove params.

### 2026-05-31 — BFLYT advanced pane mutations + prune/repair (roadmap #3 + #4)
Three committed batches on top of `byml-set`, all green.

**Batch A — pane structural ops (`a50ae76`).** `src/bflyt/ops.rs`:
`remove_pane` (drop a subtree + scrub the removed names from `grp1` lists,
refuses the root), `move_pane` (reparent; refuses root/self/own-descendant
cycle), `rename_pane` (rename + update `grp1` refs; rejects dup/over-length),
`copy_subtree` (deep-copy children, append a suffix to copied descendant names,
validate every result name before attaching). Verbs `pane-remove`/`pane-move`/
`pane-rename`/`pane-copy`. The writer rebuilds all sizes/offsets, so these are
pure in-memory tree edits. 10 unit tests + `tests/bflyt_pane_ops.rs`.

**Batch B — prune + repair (`582e493`).** `src/bflyt/repair.rs`:
`prune_unused_textures` (txl1 entries no material references; remap), 
`prune_unused_materials` (mat1 entries no pic1/txt1/wnd1 references; remap pane
refs), `fix_dangling_texture_refs` (clamp out-of-range/negative
material→texture indices into `[0,len)`; drop + rebuild flags when no textures —
clamping leaves counts unchanged so it's safe for `flags_untrusted` mats),
`dedupe_pane_names` (rename later dups to `name_2/_3`), and
`repair(prune_materials) → RepairReport`. Material pruning skips (and flags)
when `prt1` property data is present (it references an *external* part's mats,
but the data is opaque to us). Verbs `bflyt-prune` + `bflyt-repair`
(`--dry-run`). 8 unit tests + a fixture-gated repair round-trip.

**Batch C — set-text / set-window (`d75960a`).** `set_window(pane, WindowEdit)`
edits wnd1 stretch/frame-size borders. `set_text`/`pane_text` replace/read a
`txt1` string for the standard single-string layout (string at the canonical
`0xA8` offset, UTF-16LE + NUL, updates `text_str_bytes`/`text_buf_bytes`); panes
carrying a text id / per-character transform / line-width table are **rejected**
rather than corrupted (round-trip discipline). Verbs `bflyt-set-text` +
`bflyt-set-window`. 4 unit tests + a fixture-gated real-bytes set-text
round-trip (edits a real layout's first simple `txt1` and reads it back).

Per the user's request, **AAMP is moved to the bottom of `todo.md`** (deferred —
no dump). `cargo test` green (**96** lib unit + all integration, 34 binaries, 0
failures); `clippy --all-targets` clean; `--no-default-features` builds.
**Follow-up:** a layout.arc-level `layout-repair` wrapper (repair every BFLYT in
a packed archive); `layout-diff` of `wnd1`/`prt1` material bindings.

### 2026-05-31 — BYML `byml-set` (scalar mutation-by-path) + AAMP fixture finding
Picked up the AAMP handoff (roadmap #2). **Investigation result that reframes
AAMP:** Tears of the Kingdom does **not** use AAMP — every parameter file is
BYML (`.bgyml`). A full recursive romfs scan for ~21 AAMP extensions
(`.baiprog`/`.bphysics`/`.bgparamlist`/…) found **none**, and cracking a real
actor pack (`Pack/Actor/Accessory_Battery.pack.zs`) showed all 18 entries are
`YB`/BYML + `.ainb` (AI Node Binary) — zero AAMP. AAMP is a **BOTW-era** format;
with no AAMP fixtures locally it can't meet the project's real-bytes
round-trip bar. The full AAMP v2 spec is captured (header at 0x30; list/object/
param node layout with `>>2` relative offsets; the 21 `ParameterType`s; CRC32
keys), and the plan maps onto the BYML two-writer discipline — **queued pending
a BOTW dump (its actor packs) or a Python `oead` byte oracle.**

Pivoted to the **`byml-set`** BYML follow-up (highest TotK-editing value: TotK
params are all BYML, and the reader + canonical writer + diff already exist).
New **`src/byml/edit.rs`**: `set_by_path(root, path, raw, ty)` edits a single
scalar leaf addressed by a `byml-diff`-style path (`/RecipeList/0/ResultActorName`;
hash keys by name, arrays by index, leading slash optional). The target type is
**preserved** by default (editing an `f32` keeps it `f32`); `--type` overrides
the kind (`bool`/`s32`/`u32`/`f32`/`s64`/`u64`/`f64`/`string`/`null`) or promotes
a `null`. It **refuses** to clobber a container/binary node or descend through a
scalar, so a typo can't silently delete a subtree. `u32`/`u64` accept `0x` hex.
Then `write_byml_canonical` re-serializes (semantically lossless — re-parses to
the mutated tree, not byte-identical by contract). New `ScalarType` + `SetReport`
+ 8 new `BymlError` edit variants; exported from `byml` + the prelude.

Verb **`byml-set`** (`-i`/`-o`/`--path`/`--value`/`--type`, inflates `.byml.zs`
via `--dict`/`--romfs`, writes uncompressed). Verified end-to-end on real
`CookingTable`: `byml-set … --path /RecipeList/0/ResultActorName --value
Item_Cook_TEST` then `byml-diff` reports **+0 −0 ~1** (exactly the one leaf).
Tests: 11 fixture-free unit tests in `byml::edit` (nested/array set,
type-preserve, `--type` override, hex u32, and every rejection path) +
`tests/byml_set.rs` (3 fixture-gated: a real `CookingTable` string + numeric +
type-override edit each canonical-round-trip to **exactly one** structural diff
at the target). `cargo test` green (**74** lib unit + all integration);
`clippy --all-targets` clean; `--no-default-features` builds.
**Follow-up:** add/remove-by-path (create a new key / append / delete).

### 2026-05-31 — MSBT (LibMessageStudio message) read + round-trip + JSON edit (Stages A+B)
Roadmap item: the text/message format. TotK ships localized text in
`Mals/<lang>.Product.NNN.sarc.zs` — a zstd SARC of `.msbt` files
(`archive-extract --romfs <dump>` to get them).

**Format (verified on real bytes; 1510 USen files all uniform).** `MsgStdBn`:
0x20-byte header (magic + BOM at 0x08 picking endianness, `encoding` u8 / 0=UTF-8
1=UTF-16 2=UTF-32, `version` u8, `section_count` u16, `file_size` u32), then
sections each headed by `{magic[4], size u32, pad[8]}` and tail-padded with
`0xAB` to 0x10. TotK uses LE / UTF-16 / v3 / `LBL1`+`TXT2`. `LBL1` = a hash
table (`u32 ngroups`, `{count, offset}` buckets, then `{u8 len, ASCII name,
u32 message-index}` entries). `TXT2` = `u32 count`, a `u32` offset table, then
NUL-terminated UTF-16 messages with inline control tags: `0x000E` opens
(group/type/size/payload), `0x000F` closes (group/type); literal `\n`/`\t`
are ordinary text, not tags (confirmed by scanning the corpus — 95k newlines
vs 104k real tags).

**New `src/msbt/`** (`mod`/`read`/`write`/`error`, typed `MsbtError` via
`#[from]`). Reader is bounds-checked + reports the failing offset; it decodes
`LBL1` + `TXT2` (a tag-aware chunk decoder splits text from control tags) and
keeps other section magics opaque. `write_msbt` re-emits the bytes captured at
parse time → **byte-identical** for an unmodified file (the BYML/compression
discipline). Verbs `msbt-inspect` (`--json`/`--limit`, inflates `.msbt.zs` via
`--dict`/`--romfs`) + `msbt-roundtrip-test`.

**Validation.** **All 1510 USen + 1510 JPja** `Mals` `.msbt` round-trip
byte-identically (CJK + control tags + `é`-class UTF-16 all decode correctly).
Tests: `tests/msbt_roundtrip.rs` (fixture-gated corpus byte-identical + pinned
`Info_BuildHouse`/`Npc` structure) + 5 fixture-free unit tests (hand-built
minimal MSBT with a tagged message; bad-magic/BOM/too-small/section-overrun
rejection). Fixtures (4 sampled USen files) under the gitignored
`tests/fixtures/msbt/`.

**Stage B — canonical writer + JSON edit.** `write_msbt_canonical` rebuilds a
document from its decoded sections: `LBL1` re-encoded via the LMS label hash
(`h = h*0x492 + byte` over the ASCII bytes, bucket = `hash % group_count`;
**verified against all 47,657 corpus labels** — every one lands in its stored
bucket) into the original `lbl1_groups` count, `TXT2` from the messages, other
sections verbatim. Contract is the semantic round-trip (like BYML's), but it
is byte-identical on all 4 local fixtures. `Message::from_chunks` inverts the
UTF-16 chunk decoder; `MsbtDocument::set_message_by_label` edits a message in
place. Verbs `msbt-export-json` (label→message JSON: text runs as strings,
control tags as `{tag|close}` objects with hex payloads) and `msbt-import-json`
(overlay edits by label, then canonical-write). Verified end-to-end on real
`Info_BuildHouse.msbt`: a no-edit export→import rebuild is **byte-identical**,
and a "Home on Arrange"→"Casa Translated" edit propagates through.

`cargo test` green (63 lib unit + all integration); `clippy --all-targets`
clean; `--no-default-features` builds. **Follow-ups:** BOTW / non-v3 versions
(only TotK v3 LE/UTF-16 fixtures locally), `ATR1`/`TSY1` structural decode
(retained opaque today).

### 2026-05-31 — RESTBL (Resource Size Table) read + write + update
Roadmap item #3. Lets a mod repack BOTW/TotK without crashing: if a modified
resource exceeds its recorded size, the game faults, so the size table must be
updated. TotK ships it as `System/Resource/ResourceSizeTable.Product.NNN
.rsizetable.zs` (zstd).

**Format (verified on real bytes).** `RESTBL` v1 is a fixed, deterministic
layout: 22-byte header (magic `RESTBL` + `version` u32=1 + `string_block_size`
u32=160 + `crc_table_num` u32 + `name_table_num` u32) → CRC table
(`{hash:u32, size:u32}` × N, **sorted by hash**) → name/collision table
(`{char[160] name, size:u32}` × M, **sorted by name**). The size math is exact
(22 + 379715·8 + 32·164 = 3,042,990 = the decompressed `.121`), so the writer
is **byte-identical**.

**`src/restbl.rs`** (single-file format module, typed `RestblError` via
`#[from]`): `read_restbl`/`write_restbl` (byte-identical round-trip on the real
379,715-entry tables, both 1.2.1 + 1.4.3); native standard CRC-32 (reflected,
`0xEDB88320`; verified against the `0xCBF43926` check value); `Restbl` with
binary-search `get`/`set`/`insert` by hash, by name, and by resource path
(`crc32(path)` then name-table fallback), plus a `SetOutcome`. Verbs
`restbl-inspect` (`--json`, `--lookup <path>`/`--hash`, inflates
`.rsizetable.zs` via `--dict`/`--romfs`), `restbl-roundtrip-test`, and
`restbl-set` (update a size by `--path`/`--hash`/`--name`, `--insert` to add).

**Tests/docs.** `tests/restbl_roundtrip.rs` (fixture-gated byte-identical
round-trip on both real tables; pinned 1.2.1 counts + known lookups; an insert
into the real table checks +8-byte growth + sortedness + write→read resolve);
6 fixture-free unit tests in `restbl` (CRC-32 check value, build/parse,
get/set/insert/path outcomes, bad-input rejection). `cargo test` green (55 lib
unit + all integration); `clippy --all-targets` clean; `--no-default-features`
builds. Fixtures (`ResourceSizeTable.Product.121` + `.143`) added to the
gitignored `tests/fixtures/restbl/`. BOTW `RSTB` (older magic) deferred.

### 2026-05-31 — BYML canonical writer + structural diff (Stage B)
Roadmap item #2, Stage B (same session as Stage A 686ad53). Adds the
from-scratch writer (for mutated/synthesized trees) and a structural diff.

- `write_byml_canonical(version, big_endian, root)`: collects every hash key +
  string value into **sorted, deduped** `0xc2` tables, then lays the node tree
  out **breadth-first** with back-patched offsets (containers / 64-bit values /
  binary placed after their parent; value slots patched once their offset is
  known). Hash entries are emitted key-sorted (BYML's binary-search
  requirement), so the output re-parses to the same tree. Its contract is the
  **semantic** round-trip `read(write(x)) == read(x)`, *not* byte-identity —
  BYML byte layout is writer-specific (dedup / ordering / padding). In
  practice it reproduces the **exact byte length** of `CookingTable`,
  `Challenge`, and `ActorInfo` and is 4 bytes off `GameDataList`. New
  `BymlError::NonContainerRoot` for a scalar root.
- `diff_byml` + `byml-diff` verb: path-keyed structural diff (JSON-pointer-ish
  paths), matching hashes by key and arrays by index, classifying
  added/removed/changed with short value summaries (`s32(42)`,
  `string("foo")`, `array[3]`, …); bitwise float compare so `NaN`/`-0.0`
  don't false-positive. `--json`, `--limit`, and `--dict`/`--romfs` (inflates
  compressed input). On real `ActorInfo.121`→`.143`: +8212 −8249 ~79401
  (added actors like `Obj_DailyChallenge_00`, heap-size tweaks, an f32
  precision shift).
- Tests: `tests/byml_diff.rs` (self-diff empty; precise mutated-clone diff on
  `CookingTable`; `ActorInfo.121`↔`.143` non-empty + mirror-image reverse) and
  a `canonical_writer_semantic_round_trips` walk over the whole corpus
  (`read → write_byml_canonical → read` equals the original tree, both endians,
  up to 12.7 MB / 1.4M nodes). 6 new lib unit tests in `byml::write`/
  `byml::diff`. `cargo test` green (49 lib unit + all integration); `clippy
  --all-targets` clean; `--no-default-features` builds. Diff fixtures
  (`ActorInfo.Product.121/.143`) added to the gitignored `tests/fixtures/byml/`.

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
