# AGENTS.md — canonical agent operating bar (Toolbox-Cli)

This file is the **canonical, version-controlled contract** for AI agents working
in this repo. It is auto-loaded by agent tooling, so it is present in every
session regardless of what any handoff says. It is the source of truth for the
**METHOD**, **QUALITY BAR**, **PRE-COMMIT CHECKLIST**, **OVERNIGHT EXECUTION
MODEL**, **CONSTRAINTS**, **WHEN TO HAND OFF**, and the **HANDOFF PROTOCOL**.

Rules for this file:

- **Strengthen-only.** Refine to make the bar stricter or clearer; never weaken or
  delete a rule.
- **Do not summarize it elsewhere.** A handoff or session doc may *reference* this
  file, but must never replace its sections with a compressed paraphrase. (A
  summarized handoff once dropped the no-bare-`#N` rule and a violation slipped
  through — that is the failure mode this file exists to prevent.)
- **Per-session STATUS / CHECKPOINT PLAN / progress ledger live elsewhere**
  (`AGENTSSUMMARY.md` + the session handoff), not here. This file is rules, not state.

The work this bar governs: finishing the TotK **MeshCodec** vertex-geometry codec
in `src/mc/geometry.rs` — a clean-room (MIT, no GPL) port of functions reverse-
engineered from the game executable, validated byte-exact against an emulator
oracle. See `local-assets/re/FINDINGS.md` (gitignored) for the RE map.

---

## METHOD — how to produce commit-able work (the gate below checks the result)
- Anchor on ground truth: dump the function's real I/O from the emulator
  (`emu.py` + a tracer) BEFORE writing Rust. You are reproducing observed bytes,
  not inventing an algorithm.
- ENUMERATE THE WHOLE INPUT SPACE FIRST. Before porting a function at 0xADDR, hook
  it and dump EVERY invocation across ALL fixtures (not one). Tabulate every value
  that varies: count, log, stride, M, flag, count%4, prod, and any byte the disasm
  branches on. That table is the spec for your golden set — you reproduce a
  population, not a sample. Identify every DATA-DEPENDENT branch (b.eq/b.lo/cbz/tbz
  on a loaded/derived value); each must be exercised by a golden OR be a typed
  error/guard. Adversarial self-check, answered in the commit: "which input value
  would make my simplified version diverge from the disasm?" — then go capture it.
  Write the dump to a gitignored file and reference it (compaction-safe).
- Prototype first in Python, validate it byte-exact against the dump, and only
  then port to Rust. Port the *validated* algorithm; don't debug in Rust.
- Work one micro-step at a time and diff intermediates against ground truth
  (per-symbol / per-field), not just the final result.
- Map your implementation to the disassembly: each non-obvious step should
  correspond to instructions at a known address; cite them.
- DERIVE rules from the structure/invariant. If you can only fit a constant to one
  input, you are not done — get another trace or single-step the emulator. A fudge
  constant is a defect even when a test passes.
- **CODE QUALITY & ABSTRACTION (Elevate to Idiomatic Rust):** While your logic must
  be byte-exact and mapped to the disassembly, your final Rust code MUST NOT read
  like decompiled assembly.
  - *Semantic Naming:* Rename all raw assembly registers (e.g., `w15`, `x16`, `w1`, `x8`)
    to meaningful domain variables that describe their purpose (e.g., `width`,
    `accumulator`, `remaining`, `ptr`).
  - *Idiomatic Abstractions:* Encapsulate low-level pointer and bit math into
    dedicated helper structs or methods (e.g., a custom `RevReader` struct with clean
    `refill()`, `clz()`, and `take()` methods) rather than scattering inline shifts
    and cursor updates.
  - *Intent over Instruction:* When disassembly uses obscure bitwise tricks to avoid
    branching (e.g., a 4-byte chunked loop that is really "append N bytes big-endian",
    or a scalar-tail-plus-unroll that is really a strided fill), understand *why* and
    write the idiomatic Rust equivalent. Write for human maintainers, not the compiler.
  - *High-Level Control Flow:* Use expressive control flow. If implementing a state
    machine, use an `enum` with descriptive variant names (e.g., `Site::RunBody`)
    rather than matching on raw hex memory addresses or mimicking `goto` labels.

## QUALITY BAR — acceptance gate for EVERY commit
The bar is set by the existing commits `2e9e463` (`rans_decode_matches_oracle`),
`8ea1d16` (`rans_read_freqs_*`), `61a1793` (`rans_spread_*`), `0381d9d`
(`rans_decode_tail_continues_lanes`), `e63c071`
(`rans_init_states_cold_start_and_shared_cursor_bear`), and `9175847`
(`rans_decode_stride3_writes_lane_slots`) — study them; they are the exemplars
(enumerate-all + replay-all N/N + a discriminating fixture-free golden that names
the wrong impls it rules out + typed-error guards for unobserved paths +
derive-from-structure citations). Reproducing one trace is NECESSARY but NOT
SUFFICIENT. Every commit MUST satisfy ALL of:

1. **FIXTURE-FREE golden test for each newly ported function.** It must pass in a
   clean checkout with NO `local-assets/` and NO `tests/fixtures/` present —
   hardcode the minimal golden input bytes + expected output INLINE, exactly like
   `2e9e463` hardcodes STEP/SYM/states/stream. In a comment, record PROVENANCE:
   which tracer + source (model/offset) produced the golden, so it is
   reproducible. A FIXTURE-GATED test (one that early-returns when the `.mc` is
   absent) silently no-ops in CI and does NOT count as validation for new core
   logic — add one only as a BONUS on top of the fixture-free test.

2. **NO speculative / unvalidated code paths.** Commit ONLY branches a test
   exercises against ground truth. If a disassembled branch is not yet reached by
   a validated trace, do NOT ship a best-guess implementation "to keep it useful
   later" — leave a precise `// TODO(0xADDR, FINDINGS N): ...` or an explicit
   typed error / `unreachable!` with a comment. No fudge constants to line up one
   trace.

3. **REPLAY-ALL, not just ≥2.** Write a reference replay of your ported algorithm
   (same steps as the Rust) and run it against EVERY captured invocation across all
   fixtures. Commit only when it reproduces N/N. For any call it does NOT reproduce,
   either (a) port that path with its own golden, or (b) reject it with a typed
   error + a precise `TODO(0xADDR)` + a test proving rejection — never ship it
   silently. The commit MUST include the parameter-coverage table (observed values
   of each branch-selecting parameter, and which golden covers each), and at least
   one DISCRIMINATING input on which a plausible-but-wrong impl (off-by-one, unmasked
   shift, wrong width, a constant fit to one trace, `state[0]` vs `state[k]`,
   warm-only, reset-cursor, dense-vs-strided) visibly fails — named in the message.
   SINGLE-TRACE is allowed ONLY if you PROVE the population is 1.

4. **Defensive Parsing & Error Boundaries:** Never trust the bitstream. All decoders
   and table builders MUST return `Result` or `Option` and gracefully reject malformed
   data (overfull mass, out-of-bounds indices, truncated streams, zero stride,
   undersized output). No `unwrap()`/`panic!`/sole-`debug_assert!` on data-driven
   invariants; use `checked_*` for index/size math. Write explicit tests that feed
   invalid inputs and prove safe rejection.

5. **Pipeline Verification:** When porting a function that feeds an already-ported
   function (a table builder feeding a decoder; a segment loop feeding freq→spread→
   init→decode/RLE; a width combiner feeding the transform), write an integration
   test wiring them so the new output exactly satisfies the existing input contract.

6. **FULL green sign-off, RESTATED VERBATIM** in the commit message AND the
   AGENTSSUMMARY entry, with ACTUAL counts:
   `All green: N lib unit (incl. M mc::geometry) + all integration; clippy
   --all-targets clean; --no-default-features builds.` Run all three at the
   CHECKPOINT boundary (per the execution model) and restate; per-chunk commits may
   cite the sample + the last full-suite counts, but must not claim a full-suite run
   they didn't do.

7. **COMMIT MESSAGE in the prior multi-paragraph form** (see `8ea1d16`/`e63c071`/`9175847`):
   (a) what was ported + the function address; (b) how it works, mapping logic to
   disasm addresses; (c) the subtlety/gotcha that cost a wrong first cut; (d) the
   explicit "Validated byte-exact against X" (and what was NOT) + the green line.
   NEVER write a bare `#<digit>` token (GitHub auto-links it to a bogus issue) —
   write "FINDINGS 8" not "FINDINGS #8", and asm immediates as `0x8`/`+8`/`lsl x8, 1`,
   never `,#8` or `lsl x8,#1`. (A `#1` from `lsl x8,#1` slipped through once — scan
   your message for `#<digit>` before committing.)

8. **CLEAN DIFF, one logical change per commit.** Run `cargo fmt` as its own step;
   do NOT bundle rustfmt reflow of untouched lines into a feature commit.

9. **Honesty — claims match evidence.** "byte-exact"/"validated"/"done" may not
   exceed what a test demonstrates. Scope docs to exactly the validated slice. Under-
   claim rather than over-claim.

10. **Domain correctness checklist:** confirm integer widths (u32 vs u64),
    wrapping/masking, signed vs unsigned shifts (zigzag, `sar` vs `lsr`), endianness
    (cold-loader varint is big-endian append; u32 renorm is little-endian; output
    slots are u16), branch POLARITY (`bics`+`b.ne` reads backwards easily), bit order
    (lowest-set-bit lane = `x & -x`, `trailing_zeros`), product-vs-decoded-count and
    stride-in-u16-slots, and MSB-first reader conventions match the ARM semantics —
    these pass one trace and fail the next.

11. **STOP is a valid — and preferred — outcome over a fake commit.** If you cannot
    meet this bar, do NOT commit: keep the work in the gitignored Python prototype,
    document the exact blocker + ground truth in FINDINGS + the AGENTSSUMMARY ledger,
    and (overnight mode) move to the next independent chunk. No commit beats a bad one.

12. **DERIVE-FROM-STRUCTURE PROOF.** Cite the specific instruction(s) that establish
    each non-obvious rule (the `str x17,[x0],#8` post-increment proving the decode
    tail continues lanes; the `bics w10,w9`+`b.ne` proving the init nibble polarity;
    `sxtw`+`lsl #1` proving stride is in u16 slots; product at `x1+8` vs decoded `w2`
    at `x1+0xc`), not merely "it matches the trace."

It is BETTER to commit a smaller, fully-validated, fixture-free-tested piece than a
larger piece with an untested branch. Split commits accordingly.

WHY (cautionary precedent — do not repeat): pieces that passed on a single lucky
trace were wrong on unseen inputs. `rans_init_states` (0x110dfa0): the golden was a
WARM buffer on an offset-0 stream, missing the cold-start state loader (0x110e1bc —
polarity even documented backwards) and the shared forward stream cursor (closed by
`e63c071`). `rans_decode` (0x110e270): the golden had count%4 == 0, so the tail loop
(0x110e410) never ran (closed by `0381d9d`); and it allocated only `count` slots so
the stride-3 case would have indexed out of bounds (closed by `9175847`).
Enumerate-all + replay-all catches all of these — the failing inputs live in the
same fixtures.

## PRE-COMMIT CHECKLIST (tick all, in the commit message)
- [ ] enumerate-all dump run (written to a gitignored file); parameter-coverage table in the message
- [ ] reference replay reproduces N/N invocations (state N) OR unreproduced paths are typed-error + TODO + negative test
- [ ] fixture-free golden test added, with a provenance comment
- [ ] every data-dependent disasm branch is covered by a golden or guarded
- [ ] a discriminating input is tested; the wrong impls it rules out are named
- [ ] each non-obvious rule cites the instruction that proves it (derive-from-structure)
- [ ] defensive error boundaries tested (rejects malformed/truncated/zero-stride/undersized data)
- [ ] pipeline verification test added (if feeding into existing functions)
- [ ] domain checklist (#10) reviewed (widths, shifts, endianness, branch polarity, bit order, stride units)
- [ ] adversarial "which input breaks my version?" answered in the message
- [ ] per-chunk: sample tests green; per-checkpoint: full `cargo test` + `clippy --all-targets` + `--no-default-features`, counts restated verbatim
- [ ] diff is one logical change; `cargo fmt` committed separately
- [ ] claims match evidence (no over-claiming); scanned message for bare `#<digit>` (none)
- [ ] AGENTSSUMMARY ledger line appended (commit hash, green counts, next action)

## OVERNIGHT EXECUTION MODEL (autonomous, multi-commit, unattended runs)
When a handoff says to run multiple checkpoints in one unattended session,
optimize for DURABLE PROGRESS that survives automatic context compaction, because
your context window is NOT durable — only git + the docs + the gitignored RE
prototypes are. Rules:

1. **Chunk = one ported function = one commit.** Never batch two functions into
   one commit. A CHECKPOINT is a group of related chunks named by the handoff.
2. **Commit after every validated chunk, immediately.** A commit is the only thing
   guaranteed to survive compaction. Never leave validated work uncommitted across
   a chunk boundary.
3. **Write state to disk, not the context window.** Put enumerate-all dumps,
   disasm, and prototypes in gitignored `local-assets/re/` files and REFERENCE
   them. After each chunk AND each checkpoint, append a one-line ledger to the
   `AGENTSSUMMARY.md` session log: what landed (commit hash), green counts, and the
   very next action. This is how you re-orient after a compaction.
4. **Re-orient after any compaction by reading, in order:** `git log --oneline -12`,
   the top `AGENTSSUMMARY.md` session entry (your ledger), the latest `FINDINGS.md`
   UPDATE, and the handoff's CHECKPOINT PLAN. Then resume at the first unchecked chunk.
5. **Test cadence — sample per chunk, full suite per checkpoint:**
   - After each CHUNK: a REPRESENTATIVE sample covering what changed — the new
     fixture-free golden + its directly-related tests, e.g.
     `cargo test --lib mc::geometry::<new_test>` (and the pipeline test it feeds).
     A quick `cargo build` is fine. Keep it < a few seconds; do NOT run the full
     suite or the `#[ignore]`d corpus sweep per chunk.
   - At each CHECKPOINT boundary: run the FULL gate — `cargo test`, `cargo clippy
     --all-targets`, `cargo build --no-default-features` — and restate the verbatim
     green line. Only here.
6. **Don't get stuck.** If a chunk can't meet the bar after a genuine effort, do NOT
   commit a guess (QUALITY BAR item 11). Record the blocker + ground truth in
   `FINDINGS.md` and the ledger, keep the prototype gitignored, and MOVE ON to the
   next INDEPENDENT chunk/checkpoint. Only stop entirely if all remaining checkpoints
   depend on the blocked one. Always leave the tree green and committed.
7. **Stay green at all times.** Never commit a chunk that breaks the sample tests.
   If the end-of-checkpoint full suite reveals a regression, fix it (or revert the
   offending chunk) before proceeding.

## CONSTRAINTS / env
- MIT only; runtime deps pure-Rust (`zstd-pure` + std + thiserror); libzstd (`zstd`)
  is dev-only (test oracle). `meshopt` / `zstd_pure` consumers stay std+thiserror.
- Windows PowerShell: `;` not `&&`; one logical op per call; commit via
  `local-assets/re/msg.txt` + `git commit -F`. Build commit messages encoding-safe
  (e.g. the `fix_head_msg.py` pattern) and scan for bare `#<digit>` before committing.
- **In VCS (commit these):** `AGENTS.md`, `CURSOR.md` (the agent contract), and
  `AGENTSSUMMARY.md` (status/ledger). These are excluded from the published crate
  via `Cargo.toml` `exclude`, but are tracked in git.
- **Never commit:** `local-assets/`, `Switch-Toolbox/`, `tests/fixtures/`, the
  gitignored `docs/` debug scripts, `.cursor` index files, and the user's GLOBAL
  personal config `CLAUDE.md` (it lives at the user's home, not in this repo — do
  not copy it in). A repo-local `CLAUDE.md` is unnecessary; `AGENTS.md` is canonical.
- Do not push without explicit permission; never force-push (a one-time message-only
  history rewrite to remove bogus `#N` autolinks was done once, with approval — do
  not repeat it without an explicit request).
- Timings: emulator runs take a few seconds each; full `cargo test` ~1-2 min, clippy
  ~30s, `--no-default-features` build ~30s-2min — budget these at checkpoint boundaries.

## WHEN TO HAND OFF AGAIN
- Hand off at a clean fully-green committed checkpoint. Each commit must clear the
  QUALITY BAR (or precisely guard what it can't reach).
- In STATUS, mark which checkpoints are DONE (with commit hashes), the replay N/N
  counts, that tests are fixture-free, and which checkpoint/chunk is NEXT. Record
  blockers (with exact ground truth) in FINDINGS + the AGENTSSUMMARY ledger.
- Write the handoff per the HANDOFF PROTOCOL below.

## HANDOFF PROTOCOL
- Output the handoff as a SINGLE copyable fenced markdown code block (open with
  three backticks then the word markdown; close with three backticks).
- The handoff REWRITES to current reality: the title, the HEAD/push/working-tree
  line, STATUS, the CHECKPOINT PLAN (mark done/next), the green counts, and Begin.
- SELF-CONTAINED & VERBATIM IS NON-NEGOTIABLE: a fresh agent may have ONLY the
  handoff. Paste the canonical sections (METHOD, QUALITY BAR, PRE-COMMIT CHECKLIST,
  OVERNIGHT EXECUTION MODEL, CONSTRAINTS, WHEN TO HAND OFF, this PROTOCOL) IN FULL,
  or — when this `AGENTS.md` is known to be auto-loaded — state explicitly that the
  bar is in `AGENTS.md`/`CURSOR.md` and STILL include it in full as belt-and-suspenders.
  NEVER replace a canonical section with a summary, an abbreviation, or a pointer
  like "carry forward the previous rules." If you catch yourself writing "carry
  forward the previous..." or compressing a rule, STOP and paste the actual section.
  Before finishing, verify ALL canonical sections are present in full.
- CARRY FORWARD VERBATIM (strengthen-only): everything in this `AGENTS.md`. Refine
  only to strengthen; never weaken. This keeps the bar self-propagating across
  agents and models.

## RE TOOLING (living list — current set in the latest handoff + `local-assets/re/`)
The enumerate-all / replay-all harnesses (`capture_*`, `verify_*`, `confirm_*`),
tracers, Python ports/validators, `disasm.py`, the `emu.py` oracle, and the `vtxgt/`
ground-truth dumps all live gitignored under `local-assets/re/`. The set grows each
session; the latest handoff's RE TOOLING section enumerates the current files. COPY
the enumerate-all + replay-all pattern for every new function you port.
