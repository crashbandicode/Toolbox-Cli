# Toolbox-Cli

Pure-Rust CLI for inspecting Nintendo Switch UI assets — BFLYT (Cafe
Layout v8/v9), BNTX (texture container), and SARC archives.

**Inspired by** [KillzXGaming/Switch-Toolbox](https://github.com/KillzXGaming/Switch-Toolbox)
(GPL-3.0, archived). All format parsers in this project are original
implementations informed by public format documentation; no upstream code
is copied or linked. This project is licensed independently under the
[MIT License](LICENSE).

## Status

| Area | Status |
|---|---|
| BFLYT v8 / v9 parser + writer | **Working** — round-trip is byte-identical against every BFLYT in a real Smash Ultimate `layout.arc` (25/25 files including the 30 KB `info_melee_lct_player_00.bflyt` with 287 sections, 68 materials, and v9-specific material extensions) |
| BNTX parser | **Working** — decodes 206 textures from a real `__Combined.bntx` (1.7 MB) with correct format codes (BC1/3/4/5/7, R8G8B8A8) and channel swizzles |
| BNTX writer | **Stub** — returns an explicit error. Adding new textures programmatically requires this; tracked in TODOs |
| Texture pipeline (PNG → BC7 → swizzled BNTX) | **Stub** — depends on BNTX writer; `intel_tex_2` and `tegra_swizzle` are wired into Cargo.toml ready for use |
| SARC unpack | **Working** — uses [`sarc`](https://crates.io/crates/sarc) by jam1garner |
| SARC pack | **Working** — output size differs from the original because the `sarc` crate doesn't deduplicate identical files; functionally equivalent |
| `layout-validate-manifest` | **Working** — read-only verifier for SGPO skin manifests; cross-validated against a layout produced by the C# CLI (passes 4/4) and the unmodified original (correctly fails 0/4) |
| BFLYT mutation verbs (`bflyt-add-texture-ref`, `bflyt-add-material`, `mat-rename`, `pane-clone`, `pane-set`) | **Working** — replicated the SGPO 4-button `proof-one-pane` workflow in pure Rust against `info_melee.bflyt`; the Rust-built layout passes all BFLYT-side manifest checks (pane present, parent correct, material binding correct, texture-by-name in txl1). Only the "texture in BNTX" check fails — see "Limitations" below |

The intent is to land BNTX writing and the texture pipeline in a follow-up.
The current build covers the **inspection** and **validation** workflows
that the [SGPO skin converter](https://github.com/intpa/smash-gamepad-overlay)
needs to verify a generated layout against its manifest.

## Build

```bash
cargo build --release
# ./target/release/toolbox-cli.exe
```

Requires Rust 1.74+ (uses `let-else` and other recent features). The
release build statically links Intel ISPC for BC7 (via `intel_tex_2`),
adding ~9 MB to the binary.

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
sarc-pack                 Pack a directory into a SARC archive
```

Internal/debug (used to develop and validate the writer):

```text
bflyt-roundtrip-test      Read a BFLYT and write it back; reports byte diffs
bflyt-section-diff        Per-section size diff vs. the original
bflyt-mat1-diff           Per-material size diff vs. the original
```

Run `toolbox-cli <verb> --help` for the per-verb option list.

### `bflyt-inspect --json`

Emits a structured document with the following shape:

```json
{
  "path": "info_melee.bflyt",
  "file_size": 9068,
  "endian": "little",
  "version": "9.0.0.0",
  "section_kinds": [
    {"kind": "lyt1", "present": true},
    {"kind": "txl1", "count": 8},
    {"kind": "fnl1", "count": 1},
    {"kind": "mat1", "count": 24}
  ],
  "texture_list": [{"index": 0, "name": "..."}, ...],
  "fonts": ["nintendo64"],
  "materials": [{
    "index": 0,
    "name": "...",
    "white_color": [255, 255, 255, 255],
    "black_color": [0, 0, 0, 0],
    "texture_refs": [{"slot": 0, "texture_index": 0, "texture_name": "...",
                      "wrap_s": 0, "wrap_t": 0}]
  }, ...],
  "panes": [{
    "kind": "pic1",
    "name": "...",
    "parent": "RootPane",
    "visible": true,
    "alpha": 255,
    "translate": [x, y, z],
    "scale": [sx, sy],
    "size": [w, h],
    "material_index": 1,
    "material_name": "..."
  }, ...],
  "counts": {"panes": 40, "materials": 24, "textures": 8}
}
```

### `bntx-inspect --json`

```json
{
  "path": "__Combined.bntx",
  "file_size": 1749120,
  "name": "...",
  "texture_count": 206,
  "textures": [{
    "name": "com_eff_aura_03^t",
    "width": 256,
    "height": 256,
    "depth": 1,
    "mip_count": 1,
    "array_count": 1,
    "format": "BC5_UNORM",
    "channels": ["Red", "Red", "Red", "Green"],
    "has_alpha": false
  }, ...]
}
```

## Architecture

```text
src/
├── main.rs            CLI entry point + clap-based dispatch
├── bflyt/             BFLYT v8/v9 parser/writer
│   ├── sections.rs    Type definitions (BasePane, Material, etc.)
│   ├── read.rs        Parser; produces a tree from the flat section list
│   └── write.rs       Writer (partial)
├── bntx/              BNTX parser
│   ├── mod.rs         BNTX/Texture types, format enum
│   ├── read.rs        BRTI/BRTD parser
│   └── write.rs       Writer stub
├── manifest.rs        SGPO skin manifest schema (serde)
├── texpipe.rs         PNG → BC7 → swizzle pipeline (stub)
└── verbs/             One file per verb
    ├── bflyt_inspect.rs
    ├── bflyt_roundtrip_test.rs
    ├── bntx_inspect.rs
    ├── sarc_pack.rs
    └── sarc_unpack.rs
```

## Dependencies

All MIT or MIT/Apache-2.0:

- [`clap`](https://crates.io/crates/clap) — CLI parsing
- [`binrw`](https://crates.io/crates/binrw), [`byteorder`](https://crates.io/crates/byteorder) — binary IO helpers
- [`serde`](https://crates.io/crates/serde), [`serde_json`](https://crates.io/crates/serde_json) — JSON output and manifest parsing
- [`image`](https://crates.io/crates/image) — PNG/BMP/JPG decoding
- [`intel_tex_2`](https://crates.io/crates/intel_tex_2) — BC7 encoder via Intel ISPC (used by the texture pipeline once BNTX writing lands)
- [`tegra_swizzle`](https://crates.io/crates/tegra_swizzle) — Tegra X1 block swizzle
- [`sarc`](https://crates.io/crates/sarc) — SARC archive read/write
- [`anyhow`](https://crates.io/crates/anyhow), [`thiserror`](https://crates.io/crates/thiserror) — error handling
- [`walkdir`](https://crates.io/crates/walkdir) — directory traversal for `sarc-pack`

## Format references

- [Switch-Toolbox source](https://github.com/KillzXGaming/Switch-Toolbox) (GPL-3.0) — used as reading material
- [nintendo-formats.com / BFLYT](https://nintendo-formats.com/libs/nw/bflyt.html)
- [FuryBaguette / SwitchLayoutEditor](https://github.com/FuryBaguette/SwitchLayoutEditor)
- [mk8.tockdom.com / BFLYT](http://mk8.tockdom.com/wiki/)
- [`jam1garner/bntx`](https://github.com/jam1garner/bntx) — BRTI/BRTD layout reference (MIT)
- [`ultimate-research/bflyt-rs`](https://github.com/ultimate-research/bflyt-rs) — pane tree and pas1/pae1 semantics reference (MIT)

## License

[MIT License](LICENSE). Switch-Toolbox is GPL-3.0; this project does not
link against any GPL-3.0 binary or copy any GPL-3.0 source code.

## Limitations / non-goals

- Only Switch BFLYT v8 and v9 are supported. Wii U BFLYT (v5) and 3DS
  BCLYT/BRLYT are out of scope.
- v9 BFLYTs include an undocumented 60-byte material extension on some
  materials (gated by flag bit 19). We capture it verbatim for round-trip
  preservation but don't decode the field. This means tools building
  materials from scratch in Rust can't yet produce the v9 extension; the
  workaround is to clone a template material that already has it.
- BNTX texture-data round-trip is partial: dimensions, formats, and
  channels are captured, but the raw image bytes are not yet re-emitted
  by the writer. This blocks adding new textures programmatically; the
  reader half is functional.
- SARC packing doesn't deduplicate identical files (the upstream `sarc`
  crate doesn't surface dedup), so output sizes will be larger than what
  Switch Toolbox produces. The packed file is still valid.

## Round-trip test corpus

The BFLYT writer is validated against every BFLYT in a real Smash
Ultimate `layout.arc` (25 files, ranging from 9 KB to 30 KB, with section
counts from 68 to 287). The full corpus passes a byte-identical
`bflyt-roundtrip-test`. To reproduce on your own copy:

```bash
toolbox-cli sarc-unpack -i layout.arc -o unpacked/
for f in unpacked/blyt/*.bflyt; do
  toolbox-cli bflyt-roundtrip-test -i "$f"
done
```
