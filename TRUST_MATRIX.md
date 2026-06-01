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

`cargo test` = **111 lib unit + all integration, 0 failures**;
`cargo clippy --all-targets` = clean; `cargo build --no-default-features` = ok.
(This matrix is being raised toward Trusted by the in-progress hardening pass;
the coverage columns reflect state at the start of that pass and are updated as
tests land.)

---

## BFLYT (Cafe Layout v8/v9) — `src/bflyt`

Corpus: 881 BFLYT (508 Smash + 373 TotK) byte-identical; v8/v9; opaque/unknown
sections retained verbatim. Writer rebuilds all section sizes/offsets.

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `bflyt-inspect` | R | inspect | ✓ / ~ / ✗ / n/a | Inspect-only | fixture-free bad-magic/truncated test |
| `bflyt-roundtrip-test` | R | byte-identical | ✓ / ✓(synthesis,flags) / ✗ / n/a | Validated | fixture-free malformed test (then Trusted) |
| `bflyt-section-diff`, `bflyt-mat1-diff` | R | inspect (diagnostic) | ✓ / ✗ / ✗ / n/a | Inspect-only | diagnostic-only; low priority |
| `bflyt-add-texture-ref`, `bflyt-add-material`, `mat-rename`, `pane-set`, `pane-clone` | W | mutate | ✓ / ✓ / ~ / ~ | Validated | per-op diff-shape + broader corpus mutation |
| `pane-remove`, `pane-move`, `pane-rename`, `pane-copy` | W | mutate | ~ / ✓(ops) / ✓(guards) / ~ | Validated | diff-shape (only target subtree/groups change) |
| `bflyt-prune`, `bflyt-repair` | W | mutate | ~ / ✓(repair) / ✓ / ~ | Validated | repaired-vs-original diff-shape on corpus |
| `bflyt-set-text`, `bflyt-set-window` | W | mutate | ~ / ✓ / ✓(rejects) / ~ | Validated | diff-shape: only the txt1/wnd1 field changed |

## BFLAN (Cafe Layout Animation) — `src/bflan.rs`

Corpus: 7616 BFLAN (5838 Smash + 1778 TotK) byte-identical; pat1/pai1 decoded.

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `bflan-inspect` | R | inspect | ✓ / ✗ / ✗ / n/a | Inspect-only | fixture-free malformed test |
| `bflan-roundtrip-test` | R | byte-identical | ✓ / ✗ / ✗ / n/a | Validated | fixture-free malformed test (then Trusted) |

## BNTX (texture container) — `src/bntx`

Corpus: Smash `0x00040000` + TotK `0x00040100` byte-identical; BC1–BC7 + ASTC
family + R8/R8G8/B8G8R8A8 decode. Known non-byte-identical: `sgpo_one_pane_png_proof`
(verbose RLT), HDR `info_melee` B8G8R8A8 (C#-tool BRTI spacing) — documented.

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `bntx-inspect` | R | inspect | ✓ / ✓(fmt codes) / ✗ / n/a | Validated | fixture-free malformed test |
| `bntx-roundtrip-test` | R | byte-identical (documented exceptions) | ✓ / ✓ / ✗ / n/a | Validated | fixture-free malformed test |
| `bntx-export-png`, `bntx-export-all`, `bntx-export-dds` | R | inspect (decode→file) | ✓ / ✓ / ✗ / n/a | Validated | malformed test; export does not mutate |
| `bntx-import-png`, `bntx-import-dds` | W | mutate (append) | ✓ / ✓ / ~ / ~ | Validated | metadata-preservation + encodable-format negative |
| `bntx-replace-png`, `bntx-replace-dds` | W | mutate (in-place) | ✓ / ✓ / ~ / ✓(format-preserving) | Validated | explicit metadata-unchanged diff + size-mismatch neg |
| `bntx-remove-texture` | W | mutate | ✓ / ✓ / ✓(missing-name) | n/a | Validated | others-unchanged diff (have) + corpus |
| `bntx-dict-test`, `bntx-rlt-dump`, `bntx-layout-dump` | R | inspect (diagnostic) | ✓ / ✓(dict) / ✗ / n/a | Inspect-only | diagnostic-only |

## BYML (binary YAML) — `src/byml`

Corpus: real TotK assets, both endians, v1..=7, ~3.3M nodes byte-identical;
canonical writer semantically lossless (≤12.7 MB).

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `byml-inspect` | R | inspect | ✓ / ✓ / ✓ / n/a | Validated | broad BOTW+TotK corpus-audit |
| `byml-roundtrip-test` | R | byte-identical | ✓ / ✓ / ✓ / n/a | Validated | broad corpus-audit (both games) → Trusted |
| `byml-diff` | R | inspect | ✓ / ✓ / ✓ / n/a | Validated | corpus-audit; already strong |
| `byml-set` | W | mutate→semantic | ✓ / ✓ / ✓ / ✓(exactly-one-diff) | Validated | corpus-audit + repeated-write stability (then Trusted) |

## MSBT (LibMessageStudio message) — `src/msbt`

Corpus: 1510 USen + 1510 JPja TotK `.msbt` byte-identical; v3 LE/UTF-16 only.
Canonical writer semantically lossless (byte-identical on local fixtures).

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `msbt-inspect` | R | inspect | ✓ / ✓ / ✓ / n/a | Validated | broad corpus-audit |
| `msbt-roundtrip-test` | R | byte-identical | ✓ / ✓ / ✓ / n/a | Validated | BOTW/non-v3 either round-trip or fail as unsupported |
| `msbt-export-json` | R | inspect (→JSON) | ✓ / ~ / ✗ / n/a | Validated | malformed test |
| `msbt-import-json` | W | mutate→semantic | ~ / ~ / ✗ / ✗ | Experimental | mutation diff-shape (unrelated labels/opaque unchanged) |

## RESTBL (Resource Size Table) — `src/restbl.rs`

Corpus: TotK `RESTBL` v1 (379,715 entries) byte-identical, both 1.2.1/1.4.3.
BOTW `RSTB` (older magic) NOT supported.

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `restbl-inspect` | R | inspect | ✓ / ✓ / ✓ / n/a | Validated | broad corpus-audit |
| `restbl-roundtrip-test` | R | byte-identical | ✓ / ✓ / ✓ / n/a | Validated | BOTW `RSTB` coverage or explicit unsupported |
| `restbl-set` | W | mutate | ~ / ✓ / ✓ / ~ | Validated | unrelated-entries-unchanged diff (add) + BOTW RSTB |

## AAMP (binary parameter archive, BOTW) — `src/aamp`

Corpus: 418 real BOTW files byte-identical; canonical writer semantically
lossless on all 418; v2 LE/UTF-8.

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `aamp-inspect` | R | inspect | ✓ / ✓ / ✗ / n/a | Validated | broad corpus-audit; default name table |
| `aamp-roundtrip-test` | R | byte-identical / semantic | ✓ / ✓ / ✓ / n/a | Validated | corpus-audit (then Trusted) |
| `aamp-set` | W | mutate→semantic | ✓ / ✓ / ✓ / ~ | Validated | exactly-one-diff on more fixtures + repeated-write |

## BFRES (`FRES`, BOTW/TotK 3D resource) — `src/bfres`

Corpus: 424 files (BOTW v5 `.sbfres`, TotK v10 `.bfres.zs` + decompressed
models) byte-identical (verbatim, inspect-only parser). MeshCodec `.mc` not
decompressed in-tool (see `local-assets/re/FINDINGS.md`).

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `bfres-inspect` | R | inspect | ✓ / ✓ / ✓ / n/a | Inspect-only | broad corpus-audit (both games) |
| `bfres-roundtrip-test` | R | byte-identical (verbatim) | ✓ / ✓ / ✓ / n/a | Validated | corpus-audit → Trusted |

## NSO (Switch executable) — `src/nso.rs`

Byte-exact LZ4 segment inflate vs a Python-lz4 oracle on the real 35 MB TotK
`main`. Read-only (writes segment dumps; never mutates the NSO).

| Verb | Kind | Contract | corpus / unit / neg / mut-diff | Tier | → Trusted |
|---|---|---|---|---|---|
| `nso-extract` | R | inspect (decompress→files) | ✓ / ✓ / ✓ / n/a | Inspect-only | multi-module corpus (subsdk/rtld) |

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
| `corpus-audit` | R | inspect (measure) | (Phase 4) | Experimental | aggregation unit tests + real-romfs run |

---

## Summary of what the hardening pass adds

1. Fixture-free **malformed-input** tests for parsers missing them (`bntx`,
   `bflyt`, `bflan`) so every parser has typed-error negative coverage on CI.
2. **Mutation diff-shape** tests for `msbt-import-json` and `restbl-set`
   (proving unrelated labels/messages/entries stay byte-stable), complementing
   the existing `byml-set` / `aamp-set` diff tests.
3. **Canonical-writer stability** (idempotency) tests for `byml` / `msbt` /
   `aamp` (`read→canonical→read→canonical` is stable).
4. The **`corpus-audit`** verb to measure real-corpus confidence (per-format
   byte-identical / semantic / inspect / expected-unsupported / unexpected-fail
   tallies → JSON), the breadth gate most verbs need for Trusted.

A verb reaches **Trusted** only when: it's in this matrix; has fixture-free unit
tests; has negative malformed-input tests where applicable; has real fixture or
`corpus-audit` coverage; has an explicit contract; mutating verbs have
diff-shape/semantic-diff tests; unsupported variants fail loudly with typed
errors; and the latest `cargo test` / `clippy --all-targets` /
`build --no-default-features` are clean.
