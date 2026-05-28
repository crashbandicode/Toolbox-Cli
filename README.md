# Toolbox-Cli

Pure-Rust CLI for editing Nintendo Switch UI assets — BFLYT (Cafe
Layout v8/v9), BNTX (texture container), and SARC archives. End-to-end
pipeline: take an unpacked game layout + a JSON manifest + a folder of
PNGs and produce a modified, deployable layout.

**Inspired by** [KillzXGaming/Switch-Toolbox](https://github.com/KillzXGaming/Switch-Toolbox)
(GPL-3.0, archived). All format parsers, writers, and the Patricia-trie
dict builder here are original implementations informed by public format
documentation; no upstream code is copied or linked. This project is
licensed independently under the [MIT License](LICENSE).

## Status

Read **and write** for both formats are working and validated against
real Smash Ultimate assets.

| Area | Status |
|---|---|
| BFLYT v8 / v9 read+write | **Byte-identical round-trip** on every BFLYT in a real `layout.arc` (25/25 files, up to 30 KB / 287 sections / 68 materials, including v9-specific material extensions) |
| BNTX read+write | **Byte-identical round-trip** on a real 1.7 MB / 206-texture `__Combined.bntx` |
| BNTX `_DIC` Patricia-trie builder | **Validated** — rebuilds the existing 207-entry trie and routes 206/206 lookups; survives appending new entries |
| PNG → BC7 → Tegra swizzle | **Working** — uses [`intel_tex_2`](https://crates.io/crates/intel_tex_2) (Intel ISPC) and [`tegra_swizzle`](https://crates.io/crates/tegra_swizzle); auto-pads non-4-aligned dimensions |
| BFLYT mutation (add texture ref / material / pane / set transform / clone pane) | **Working** |
| BNTX texture append | **Working** — appends new textures to existing files with proper string pool, dict trie, BRTI block, BRTD data, and relocation-table updates |
| SARC unpack / pack | **Working** — uses [`sarc`](https://crates.io/crates/sarc) by jam1garner |
| `layout-apply-manifest` orchestrator | **Working** — full SGPO 4-button face-skin workflow runs end-to-end against real `info_melee.arc` (4/4 manifest checks pass, including BNTX texture presence) |
| `layout-validate-manifest` | **Working** — read-only verifier; cross-validated against layouts produced by both this CLI and the upstream C# Switch-Toolbox |

## Build

```bash
cargo build --release
# ./target/release/toolbox-cli.exe
```

Requires Rust 1.96+. The release build statically links Intel ISPC for
BC7 (via `intel_tex_2`), adding ~9 MB to the binary.

## End-to-end SGPO workflow

```bash
# Unpack the original layout archive.
toolbox-cli sarc-unpack -i info_melee.arc -o unpacked/

# Apply the SGPO skin manifest: encodes 4 face-button PNGs to BC7,
# appends them to BNTX, then adds the matching BFLYT panes/materials.
toolbox-cli layout-apply-manifest \
  --layout-dir unpacked/ \
  --manifest skin_manifest.json \
  --skin-dir my_skin_pngs/ \
  --quality fast

# Verify the result matches the manifest (4/4 elements should pass).
toolbox-cli layout-validate-manifest \
  --layout-dir unpacked/ \
  --manifest skin_manifest.json

# Repack into a deployable SARC.
toolbox-cli sarc-pack -i unpacked/ -o info_melee_modded.arc
```

## Verbs

Read-only:

```text
bflyt-inspect             Print a JSON or human-readable snapshot of a BFLYT
bntx-inspect              Print a JSON or human-readable snapshot of a BNTX
layout-validate-manifest  Verify an unpacked layout matches an SGPO skin manifest
sarc-unpack               Extract a SARC archive to a directory
```

Mutating:

```text
bflyt-add-texture-ref     Add a texture name to BFLYT txl1 (idempotent)
bflyt-add-material        Clone a template material; optionally bind a texture
mat-rename                Rename an existing material in mat1
pane-clone                Clone a template pane (e.g. SGPO marker) under a new name
pane-set                  Edit a pane's transform / alpha / visibility / material binding
bntx-import-png           Encode a PNG to BC7 + Tegra swizzle, append to BNTX
layout-apply-manifest     End-to-end: PNGs + manifest -> modified BFLYT + BNTX
sarc-pack                 Pack a directory into a SARC archive
```

Internal/debug (used to develop and validate the writers; preserved
because they're useful when extending the format support):

```text
bflyt-roundtrip-test      Read a BFLYT, write it back, byte-diff
bflyt-section-diff        Per-section size diff vs. the original
bflyt-mat1-diff           Per-material size diff vs. the original
bntx-roundtrip-test       Read a BNTX, write it back, byte-diff
bntx-dict-test            Rebuild the _DIC Patricia trie and verify lookups
bntx-rlt-dump             Dump the _RLT relocation-table layout
bntx-layout-dump          Dump per-texture data offsets/alignment in BRTD
```

Run `toolbox-cli <verb> --help` for the per-verb option list.

### `bflyt-inspect --json`

```json
{
  "path": "info_melee.bflyt",
  "file_size": 9068,
  "version": "9.0.0.0",
  "section_kinds": [{"kind": "lyt1", "present": true}, ...],
  "texture_list": [{"index": 0, "name": "..."}, ...],
  "fonts": ["nintendo64"],
  "materials": [{
    "index": 0, "name": "...",
    "white_color": [255, 255, 255, 255],
    "texture_refs": [{"slot": 0, "texture_index": 0, "texture_name": "...", "wrap_s": 0, "wrap_t": 0}]
  }, ...],
  "panes": [{
    "kind": "pic1", "name": "...",
    "parent": "RootPane", "visible": true, "alpha": 255,
    "translate": [x, y, z], "scale": [sx, sy], "size": [w, h],
    "material_index": 1, "material_name": "..."
  }, ...]
}
```

### `bntx-inspect --json`

```json
{
  "path": "__Combined.bntx",
  "file_size": 1749120,
  "name": "__Combined",
  "texture_count": 206,
  "textures": [{
    "name": "tex_foo",
    "width": 256, "height": 256, "depth": 1,
    "mip_count": 1, "array_count": 1,
    "format": "BC7_UNORM_SRGB",
    "channels": ["Red", "Green", "Blue", "Alpha"],
    "has_alpha": true
  }, ...]
}
```

## Architecture

```text
src/
├── lib.rs             Library entry point
├── main.rs            Binary entry; thin wrapper over verbs::dispatch
├── bflyt/             BFLYT v8/v9 parser/writer
│   ├── sections.rs    Type definitions
│   ├── read.rs        Parser
│   └── write.rs       Writer (byte-identical round-trip)
├── bntx/              BNTX parser/writer
│   ├── mod.rs         BntxFile + Texture types; append_texture
│   ├── read.rs        Full-fidelity parser
│   ├── write.rs       Writer (byte-identical round-trip)
│   └── dict_builder.rs  Patricia-trie builder for the _DIC section
├── texpipe.rs         PNG -> RGBA8 -> BC7 (intel_tex_2) -> Tegra swizzle (tegra_swizzle)
├── manifest.rs        SGPO skin manifest schema (serde)
└── verbs/             One module per CLI verb
```

## Dependencies

All MIT or MIT/Apache-2.0:

- [`clap`](https://crates.io/crates/clap) — CLI parsing
- [`binrw`](https://crates.io/crates/binrw), [`byteorder`](https://crates.io/crates/byteorder) — binary IO helpers
- [`serde`](https://crates.io/crates/serde), [`serde_json`](https://crates.io/crates/serde_json) — JSON
- [`image`](https://crates.io/crates/image) — PNG/JPG/BMP decoding
- [`intel_tex_2`](https://crates.io/crates/intel_tex_2) — BC7 encoder via Intel ISPC
- [`tegra_swizzle`](https://crates.io/crates/tegra_swizzle) — Tegra X1 block-linear swizzle
- [`sarc`](https://crates.io/crates/sarc) — SARC archive read/write
- [`anyhow`](https://crates.io/crates/anyhow), [`thiserror`](https://crates.io/crates/thiserror) — errors
- [`walkdir`](https://crates.io/crates/walkdir) — directory traversal for `sarc-pack`

## Format references

- [Switch-Toolbox source](https://github.com/KillzXGaming/Switch-Toolbox) (GPL-3.0; used as reading material only)
- [nintendo-formats.com / BFLYT](https://nintendo-formats.com/libs/nw/bflyt.html)
- [FuryBaguette / SwitchLayoutEditor](https://github.com/FuryBaguette/SwitchLayoutEditor)
- [mk8.tockdom.com / BFLYT](http://mk8.tockdom.com/wiki/)
- [`jam1garner/bntx`](https://github.com/jam1garner/bntx) — BRTI/BRTD layout reference
- [`ultimate-research/bflyt-rs`](https://github.com/ultimate-research/bflyt-rs) — pane tree and pas1/pae1 semantics

## License

[MIT License](LICENSE). Switch-Toolbox is GPL-3.0; this project does not
link against any GPL-3.0 binary or copy any GPL-3.0 source code. The
Patricia-trie insertion algorithm follows the same general approach as
Switch-Toolbox's `ResDict` (which itself implements a well-known data
structure), but the Rust code is original.

## Limitations / non-goals

- Only Switch BFLYT v8 and v9 are supported. Wii U BFLYT (v5) and 3DS
  BCLYT/BRLYT are out of scope.
- v9 BFLYTs include an undocumented 60-byte material extension on some
  materials (gated by flag bit 19). We capture it verbatim for round-trip
  preservation; cloning a template material reproduces it.
- BNTX texture data alignment defaults to 0x200 in `bntx-import-png` and
  `layout-apply-manifest` (sufficient for textures up to ~256x256). Use
  `--align 0x1000` for 512x512+ textures.
- BNTX append currently only supports 2D BC7 textures with a single mip
  level. Multi-mip and other formats can be added by extending
  `AppendTextureSpec`.
- The `_RLT` per-entry struct-count update on append is a heuristic
  matching against `textures.len() ± 1`. The current logic round-trips
  cleanly through the manifest workflow we exercised; structural
  invariants could be formalized further.
- SARC packing doesn't deduplicate identical files (the upstream `sarc`
  crate doesn't surface dedup), so output sizes are larger than what
  Switch Toolbox produces. The packed file is still valid.

## Round-trip test corpus

The BFLYT writer is validated against every BFLYT in a real Smash
Ultimate `layout.arc` (25 files, 9 KB to 30 KB, 68 to 287 sections each).
The BNTX writer is validated against the 1.7 MB `__Combined.bntx`. To
reproduce on your own copy:

```bash
toolbox-cli sarc-unpack -i layout.arc -o unpacked/
for f in unpacked/blyt/*.bflyt; do
  toolbox-cli bflyt-roundtrip-test -i "$f"
done
toolbox-cli bntx-roundtrip-test -i unpacked/timg/__Combined.bntx
```
