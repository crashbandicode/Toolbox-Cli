# Toolbox-Cli — CLI verb trust matrix

Reliability/support classification for every `nx-layout-toolbox` CLI verb. This
is the source of truth for *how much each verb is proven*, separate from the
feature/roadmap status in `AGENTSSUMMARY.md` / `todo.md`. Update it whenever a
verb gains (or loses) test coverage.

## Trust tiers

- **Trusted** — broad real-corpus coverage across applicable games/formats,
  byte-identical round-trip where that's the contract, mutation invariants
  where applicable, malformed-input tests, fixture-free CI tests, and clean
  `cargo test` + `cargo clippy --all-targets` + `cargo build --no-default-features`.
- **Validated** — tested on real fixtures *and* has fixture-free unit tests, but
  corpus breadth or mutation coverage isn't yet enough for Trusted.
- **Experimental** — works on known fixtures but needs more corpus coverage,
  negative testing, or mutation invariants.
- **Inspect-only** — safe for reading/inspection; no mutation/write guarantee.
- **Lossless / not byte-identical** — semantically lossless but expected to
  differ at the byte/container level (recompression, canonical rewriting).

A verb is **not** Trusted just because `cargo test` passes — it must meet the
tier definition (see the per-verb "→ Trusted" column).

## Output contracts (what "correct" means per verb)

- **byte-identical** — `write(read(x)) == x` for an unmodified document.
- **semantic** — `read(write_canonical(read(x)))` equals `read(x)` (a
  from-scratch writer; byte layout is writer-specific by contract).
- **inspect** — read-only; produces a report, never writes the source.
- **mutate** — changes exactly one logical value/structure; unrelated
  entries/sections/opaque bytes stay stable (proved by a diff-shape test).
- **lossless-recompress** — `decompress(compress(x)) == x`; the compressed
  container is *not* byte-identical to the game's encoder (different encoder).

## Coverage legend

`corpus` = exercised on real game fixtures; `unit` = fixture-free unit test;
`neg` = malformed-input/typed-error test; `mut-diff` = mutation diff-shape /
semantic-diff test. ✓ = present, ~ = partial, ✗ = absent, n/a = not applicable.

## Validation status (last full run)

`cargo test` = **133 lib unit + 224 total across all binaries, 0 failures**;
`cargo clippy --all-targets` = clean; `cargo build --no-default-features` = ok.
The hardening pass landed: fixture-free malformed-input tests for every parser,
mutation diff-shape tests for the setters, canonical-writer idempotency tests,
and the `corpus-audit` breadth tool. Tiers below reflect that coverage. Scope
caveats are explicit (e.g. byml = TotK, aamp = BOTW, msbt = TotK v3 only).

---

## BFLYT (Cafe Layout v8/v9) — `src/bflyt`

Corpus: 881 BFLYT (508 Smash + 373 TotK) byte-identical; v8/v9; opaque/unknown
sections retained verbatim. Writer rebuilds all section sizes/offsets.

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `bflyt-inspect` | R | inspect | ✓ / ✓ / ✓ / n/a | Inspect-only | (parser is corpus-trusted; inspect = read-only) |
| `bflyt-roundtrip-test` | R | byte-identical | ✓(881) / ✓ / ✓ / n/a | **Trusted** | — (881 byte-identical + negatives + typed errors) |
| `bflyt-section-diff`, `bflyt-mat1-diff` | R | inspect (diagnostic) | ✓ / ✗ / ✓(via parser) / n/a | Inspect-only | diagnostic-only; low priority |
| `bflyt-add-texture-ref`, `bflyt-add-material`, `mat-rename`, `pane-set`, `pane-clone` | W | mutate | ✓ / ✓ / ✓ / ~ | Validated | per-op diff-shape + broader corpus mutation |
| `pane-remove`, `pane-move`, `pane-rename`, `pane-copy` | W | mutate | ~ / ✓(ops) / ✓(guards) / ~ | Validated | diff-shape (only target subtree/groups change) |
| `bflyt-prune`, `bflyt-repair` | W | mutate | ~ / ✓(repair) / ✓ / ~ | Validated | repaired-vs-original diff-shape on corpus |
| `bflyt-set-text`, `bflyt-set-window` | W | mutate | ~ / ✓ / ✓(rejects) / ~ | Validated | diff-shape: only the txt1/wnd1 field changed |

## BFLAN (Cafe Layout Animation) — `src/bflan.rs`

Corpus: 7616 BFLAN (5838 Smash + 1778 TotK) byte-identical; pat1/pai1 decoded.

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `bflan-inspect` | R | inspect | ✓ / ✗ / ✓ / n/a | Inspect-only | typed `BflanError` (uses crate `Error::Other` today) |
| `bflan-roundtrip-test` | R | byte-identical | ✓(7616) / ✗ / ✓ / n/a | Validated | typed `BflanError` (currently crate `Error::Other`) → Trusted |

## BNTX (texture container) — `src/bntx`

Corpus: Smash `0x00040000` + TotK `0x00040100` byte-identical; BC1–BC7 + ASTC
family + R8/R8G8/B8G8R8A8 decode. Known non-byte-identical: `sgpo_one_pane_png_proof`
(verbose RLT), HDR `info_melee` B8G8R8A8 (C#-tool BRTI spacing) — documented.

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `bntx-inspect` | R | inspect | ✓ / ✓(fmt codes) / ✓ / n/a | Inspect-only | (parser corpus-trusted; inspect = read-only) |
| `bntx-roundtrip-test` | R | byte-identical (documented C#-tool exceptions) | ✓ / ✓ / ✓ / n/a | **Trusted** | — (Smash+TotK byte-identical; sgpo/HDR exceptions documented) |
| `bntx-export-png`, `bntx-export-all`, `bntx-export-dds` | R | inspect (decode→file) | ✓ / ✓ / ✓ / n/a | Validated | export does not mutate; decode corpus-broad |
| `bntx-import-png`, `bntx-import-dds` | W | mutate (append) | ✓ / ✓ / ~ / ~ | Validated | metadata-preservation + encodable-format negative |
| `bntx-replace-png`, `bntx-replace-dds` | W | mutate (in-place) | ✓ / ✓ / ~ / ✓(format-preserving) | Validated | explicit metadata-unchanged diff + size-mismatch neg |
| `bntx-remove-texture` | W | mutate | ✓ / ✓ / ✓(missing-name) | n/a | Validated | others-unchanged diff (have) + corpus |
| `bntx-dict-test`, `bntx-rlt-dump`, `bntx-layout-dump` | R | inspect (diagnostic) | ✓ / ✓(dict) / ✗ / n/a | Inspect-only | diagnostic-only |

## BYML (binary YAML) — `src/byml`

Corpus: real TotK assets, both endians, v1..=7, ~3.3M nodes byte-identical;
canonical writer semantically lossless (≤12.7 MB).

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `byml-inspect` | R | inspect | ✓ / ✓ / ✓ / n/a | Inspect-only | (parser corpus-trusted on TotK; read-only) |
| `byml-roundtrip-test` | R | byte-identical | ✓(~3.3M nodes, both endians) / ✓ / ✓ / n/a | **Trusted** (TotK) | — (broad TotK corpus byte-identical; BOTW uses AAMP) |
| `byml-diff` | R | inspect | ✓ / ✓ / ✓ / n/a | Validated | corpus-audit; already strong |
| `byml-set` | W | mutate→semantic | ✓ / ✓ / ✓ / ✓(exactly-one-diff) | **Trusted** | — (exactly-one-diff invariant + canonical idempotency + negatives) |

## MSBT (LibMessageStudio message) — `src/msbt`

Corpus: 1510 USen + 1510 JPja TotK `.msbt` byte-identical; v3 LE/UTF-16 only.
Canonical writer semantically lossless (byte-identical on local fixtures).

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `msbt-inspect` | R | inspect | ✓ / ✓ / ✓ / n/a | Inspect-only | (parser corpus-trusted on TotK v3; read-only) |
| `msbt-roundtrip-test` | R | byte-identical | ✓(3020, v3) / ✓ / ✓ / n/a | Validated | BOTW/non-v3 either round-trip or fail as unsupported |
| `msbt-export-json` | R | inspect (→JSON) | ✓ / ~ / ✓(via parser) / n/a | Validated | dedicated JSON-shape negative test |
| `msbt-import-json` | W | mutate→semantic | ~ / ✓ / ✓ / ✓(unrelated unchanged) | Validated | real-fixture end-to-end mut-diff + version breadth |

## RESTBL (Resource Size Table) — `src/restbl.rs`

Corpus: TotK `RESTBL` v1 (379,715 entries) byte-identical, both 1.2.1/1.4.3.
BOTW `RSTB` (older magic) NOT supported.

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `restbl-inspect` | R | inspect | ✓ / ✓ / ✓ / n/a | Inspect-only | (parser corpus-trusted on TotK v1; read-only) |
| `restbl-roundtrip-test` | R | byte-identical | ✓(379k) / ✓ / ✓ / n/a | Validated | BOTW `RSTB` coverage or explicit unsupported |
| `restbl-set` | W | mutate | ~ / ✓ / ✓ / ✓(only-target-changes) | Validated | BOTW `RSTB` variant coverage (then Trusted) |
| `restbl-update-dir` | W | mutate (grow-only) | ✓(real table) / ✓ / ~ / ✓(only-grow) | Validated | per-format overhead formulas + in-game verify (over-allocation is safe today) |

## AAMP (binary parameter archive, BOTW) — `src/aamp`

Corpus: 418 real BOTW files byte-identical; canonical writer semantically
lossless on all 418; v2 LE/UTF-8.

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `aamp-inspect` | R | inspect | ✓ / ✓ / ✓(via parser) / n/a | Inspect-only | default name table (cosmetic); read-only |
| `aamp-roundtrip-test` | R | byte-identical / semantic | ✓(418) / ✓ / ✓ / n/a | **Trusted** (BOTW) | — (418 byte-identical + canonical semantic + idempotency) |
| `aamp-set` | W | mutate→semantic | ✓ / ✓ / ✓ / ~ | Validated | exactly-one-diff test (byml-set style) → Trusted |

## BFRES (`FRES`, BOTW/TotK 3D resource) — `src/bfres`

Corpus: 424 files (BOTW v5 `.sbfres`, TotK v10 `.bfres.zs` + decompressed
models) byte-identical (verbatim, inspect-only parser). MeshCodec `.mc` not
decompressed in-tool (see `local-assets/re/FINDINGS.md`).

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `bfres-inspect` | R | inspect | ✓ / ✓ / ✓ / n/a | Inspect-only | (parser corpus-trusted, both games; read-only) |
| `bfres-roundtrip-test` | R | byte-identical (verbatim) | ✓(424, v5+v10) / ✓ / ✓ / n/a | **Trusted** | — (424 byte-identical across both games + negatives) |

## NSO (Switch executable) — `src/nso.rs`

Byte-exact LZ4 segment inflate vs a Python-lz4 oracle on the real 35 MB TotK
`main`. Read-only (writes segment dumps; never mutates the NSO).

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `nso-extract` | R | inspect (decompress→files) | ✓ / ✓ / ✓ / n/a | Inspect-only | multi-module corpus (subsdk/rtld) |

## MC / MCPK (TotK MeshCodec) — `src/mc`

A model `.mc` = `[BFRES frame: magicless zstd, no dict] + [mesh vertex/index
buffers: a CUSTOM MeshCodec encoding, NOT zstd]`. `mc-extract` decodes the first
frame = the BFRES **structure** (byte-identical to the reference decompressor's
BFRES portion); it does **not** decode the geometry (custom mesh codec, unsolved).
`mc-repack` re-encodes the BFRES and preserves the original mesh tail verbatim
(edited structure + original geometry; same-BFRES-size edits only).
**In-game acceptance of repacked `.mc` is untestable here (no hardware).**

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `mc-inspect` | R | inspect | ✓(12,395) / ✓ / ✓ / n/a | Inspect-only | (header decode corpus-trusted; read-only) |
| `mc-inspect --mesh` | R | inspect (FMSH framing) | ✓(3) / ✓ / ✓ / n/a | Inspect-only | FMSH header/chunk/sizes parsed + verified vs oracle on 3 fixtures; geometry NOT decoded (custom entropy codec) |
| `mc-roundtrip-test` | R | byte-identical (verbatim) | ✓(12,395) / ✓ / ✓ / n/a | **Trusted** | — (all 12,395 `.mc` parse + verbatim round-trip) |
| `mc-extract` | R | inspect (decompress BFRES structure) | ✓(3 `.mc` vs oracle + 12,395-payload round-trip vs libzstd, pure `zstd-pure`) / ✓ / ✓ / n/a | Validated | mesh-geometry decode (custom codec); then full-model Trusted |
| `mc-repack` | W | mutate (BFRES re-encode + mesh tail preserved; NOT byte-identical) | ✓(self-RT + tail-preserve) / ✓ / ✓(resize-guard) / ✓(extract∘repack=id) | Experimental | in-game acceptance (no hardware) + geometry-edit support |

## meshopt (meshoptimizer 0.15 codec) — `src/meshopt`

Clean-room (MIT) port of the stock meshoptimizer 0.15 vertex (`0xa0`) /
index-buffer (`0xe0` v0/v1) / index-sequence (`0xd0`) codecs. The exe links
`NintendoWare_Meshoptimizer_For_MeshCodec-0_15_0`, but TotK's actual mesh decode
uses a **custom entropy backend** (`clz`-based bitstream; see FINDINGS), so this
module is a faithful **reference codec + encoder foundation**, not a drop-in
decoder for the game's streams. Library only (no CLI verb yet). Encode + decode
are mutual inverses; std + `thiserror` only.

| API | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `encode/decode_vertex_buffer` | lib | lossless round-trip | ~ / ✓ / ✓ / n/a | Validated | end-to-end vs the `mesh-codec-output` oracle (needs the Nintendo streaming framing) |
| `encode/decode_index_buffer` | lib | lossless round-trip | ~ / ✓ / ✓ / n/a | Validated | same (oracle end-to-end) |
| `encode/decode_index_sequence` | lib | lossless round-trip | ~ / ✓ / ✓ / n/a | Validated | same (oracle end-to-end) |

`unit` = exact-format vectors (anchored to the meshopt 0.15 byte layout) +
synthetic multi-block/grid/random round-trips; `corpus (~)` = `decode(encode(x))
== x` verified locally on the oracle's decoded vertex/index buffers for all 3
`tests/fixtures/mc` models (real TotK bytes; not a committed fixture). Reaching
Trusted requires decoding the **real game-encoded** streams (i.e. solving the
Nintendo streaming framing) and matching the oracle end-to-end.

## SARC archive — `src/sarc`

Native reader + per-file-alignment writer; `info_melee.layout.arc` (344 entries)
byte-identical round-trip; 14 unit tests incl. malformed inputs.

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `sarc-unpack` | R | inspect (extract) | ✓ / ✓ / ✓ / n/a | Validated | corpus-audit (TotK packs) |
| `sarc-pack` | W | byte-identical (re-pack) | ✓ / ✓ / ✓ / ~ | Validated | round-trip diff on a TotK pack corpus |

## Compression — `src/compression`

zstd decode byte-identical to Python 3.14 `compression.zstd`; native Yaz0/Yaz1.
Recompression is lossless, NOT container-byte-identical (different encoder).

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `decompress` | R | inspect (decompressed bytes identical) | ✓ / ✓ / ✓(missing-dict) / n/a | Validated | corpus-audit over romfs |
| `compress` | W | lossless-recompress (NOT byte-identical) | ✓ / ✓ / ~ / n/a | Lossless / not byte-identical | round-trip on more codecs/levels |
| `archive-extract` | R | inspect (extract+inflate) | ✓ / ~ / ~ / n/a | Validated | path-traversal + malformed-entry negatives |

## Layout / SGPO orchestration — `src/layout.rs`, `src/diff.rs`, `src/audit.rs`

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `layout-apply-arc` | W | mutate (only target entries change) | ✓ / ✓ / ~ / ✓(others byte-identical) | Validated | real SGPO skin end-to-end |
| `layout-apply-manifest` | W | mutate | ✓ / ✓ / ~ / ~ | Validated | real SGPO skin + diff-shape |
| `layout-validate-manifest` | R | inspect (validate) | ✓ / ✓ / n/a / n/a | Validated | corpus |
| `layout-diff` | R | inspect | ✓ / ✓ / n/a / n/a | Validated | wnd1/prt1 binding diff |
| `layout-audit` | R | inspect (scan) | ✓ / ✓ / n/a / n/a | Validated | superseded-by `corpus-audit` for breadth |
| `corpus-audit` | R | inspect (measure) | ~ / ✓(6) / ✓ / n/a | Validated | recorded BOTW + TotK real-romfs runs (local) |
| `nso-extract` | R | inspect (decompress→files) | ✓ / ✓ / ✓ / n/a | Inspect-only | multi-module corpus (subsdk/rtld); read-only |

---

## What the hardening pass landed (Phases 2–5)

1. Fixture-free **malformed-input** tests for the parsers that lacked them
   (`bntx`, `bflyt`, `bflan`); the rest (`byml`/`msbt`/`aamp`/`restbl`/`sarc`/
   `nso`/`bfres`) already had typed-error negative coverage on CI.
2. **Mutation diff-shape** tests for `msbt-import-json` (unrelated messages +
   labels + section set byte-stable) and `restbl-set` (only the target entry's
   size changes; name table + ordering stable; a miss is a no-op) — joining the
   existing `byml-set` (exactly-one-diff) / `aamp-set` tests.
3. **Canonical-writer idempotency** tests for `byml` (both endians), `msbt`,
   and `aamp` (`read→canonical→read→canonical` is byte-stable).
4. The **`corpus-audit`** verb + module (per-format byte-identical / semantic /
   inspect / expected-unsupported / unexpected-fail tally → JSON), the breadth
   gate verbs need for Trusted.

### Promoted to Trusted this pass
`bflyt-roundtrip-test`, `bntx-roundtrip-test` (documented C#-tool exceptions),
`byml-roundtrip-test` (TotK), `byml-set`, `aamp-roundtrip-test` (BOTW),
`bfres-roundtrip-test` — each has broad real-corpus byte-identical coverage,
fixture-free unit tests, malformed-input negatives, an explicit contract, typed
errors, and clean `cargo test` / `clippy --all-targets` /
`build --no-default-features`.

### Still Validated (concrete gap to Trusted)
- `bflan-roundtrip-test`/`-inspect`: add a typed `BflanError` (uses the crate
  `Error::Other` today).
- `msbt-roundtrip-test`/`-import-json`: BOTW / non-v3 variants must either
  round-trip or fail as explicitly unsupported; add a real-fixture end-to-end
  import mut-diff.
- `restbl-roundtrip-test`/`-set`: BOTW `RSTB` (older magic) coverage.
- `aamp-set`: an exactly-one-diff test (byml-set style).
- `corpus-audit`: record BOTW + TotK real-romfs runs (local-only).
- Inspect verbs stay **Inspect-only** (no write contract); their parsers are
  corpus-trusted.

A verb reaches **Trusted** only when: it's in this matrix; has fixture-free unit
tests; has negative malformed-input tests where applicable; has real fixture or
`corpus-audit` coverage; has an explicit contract; mutating verbs have
diff-shape/semantic-diff tests; unsupported variants fail loudly with typed
errors; and the latest `cargo test` / `clippy --all-targets` /
`build --no-default-features` are clean.
