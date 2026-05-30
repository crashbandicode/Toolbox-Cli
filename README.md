# nx-layout-toolbox

Pure-Rust **library and CLI** for editing Nintendo Switch UI assets — BFLYT
(Cafe Layout v8/v9), BNTX (texture container), and SARC archives. End-to-end
pipeline: take an unpacked game layout + a JSON manifest + a folder of
PNGs and produce a modified, deployable layout.

> Crate name: `nx-layout-toolbox` (binary + library `nx_layout_toolbox`).
> The source repository is still named `Toolbox-Cli`.

**Inspired by** [KillzXGaming/Switch-Toolbox](https://github.com/KillzXGaming/Switch-Toolbox)
(GPL-3.0, archived). All format parsers, writers, and the Patricia-trie
dict builder here are original implementations informed by public format
documentation; no upstream code is copied or linked. This project is
licensed independently under the [MIT License](LICENSE).

## Status

Read **and write** for the supported formats are working and validated
against real Smash Ultimate assets (byte-identical round-trip corpus).

| Area | Status |
|---|---|
| BFLYT v8 / v9 read+write | **Byte-identical round-trip** on 508/508 BFLYT across game UIs + HDR/training-modpack community mods (up to 30 KB / 287 sections / 68 materials, incl. v9 material extensions). Strict on unknown sections (TotK `ctl1` not yet handled). |
| BFLAN read+write | **Byte-identical round-trip** on 5838/5838 BFLAN; `pat1`/`pai1` decoded for inspect |
| BNTX read+write | **Byte-identical round-trip** (5/6 fixtures; the 6th is a C#-tool verbose-RLT output, tolerated). Version `0x00040000` only |
| BNTX `_DIC` Patricia-trie builder | **Validated** — rebuilds the trie in texture (BRTI) order; routes all lookups; survives append/remove |
| BNTX → PNG / DDS export | **Working** — deswizzle + decode (BC1–BC7, R8G8B8A8) honoring the channel-swizzle; DDS (DX10) interchange |
| PNG → BC1/BC3/BC4/BC5/BC7 → Tegra swizzle | **Working** — [`intel_tex_2`](https://crates.io/crates/intel_tex_2) (Intel ISPC) + [`tegra_swizzle`](https://crates.io/crates/tegra_swizzle); multi-mip + cube; auto-pads non-4-aligned dims |
| BNTX texture append / remove / replace | **Working** — append (2D/cube/multi-mip), remove, format-preserving in-place replace from PNG or DDS |
| BFLYT mutation (add texture ref / material / pane / set transform / clone pane) | **Working** |
| SARC unpack / pack | **Working** — reads via [`sarc`](https://crates.io/crates/sarc); **custom writer** assigns per-file alignment (no `0x2000`-everywhere bloat) and preserves hash-only entries |
| `layout-apply-manifest` / `layout-apply-arc` | **Working** — full SGPO face-skin workflow end-to-end on an unpacked dir or directly on a packed `layout.arc` |
| `layout-diff` / `layout-audit` | **Working** — structured BFLYT+BNTX diff; recursive unsupported/suspicious-structure scan to JSON |
| `layout-validate-manifest` | **Working** — read-only verifier; cross-validated against this CLI and the upstream C# Switch-Toolbox |

## Build

```bash
cargo build --release
# ./target/release/nx-layout-toolbox.exe
```

Requires Rust 1.96+. The release build statically links Intel ISPC for
BC7 (via `intel_tex_2`), adding ~9 MB to the binary. Prebuilt BC7 binaries
ship for x86_64 Linux, Windows, and macOS, so no ISPC/libclang toolchain is
needed to build.

## Use as a library

The crate is also a library. Depend on it with the CLI machinery (`clap` +
`anyhow`) disabled when you only need the format API:

```toml
[dependencies]
nx-layout-toolbox = { version = "0.1", default-features = false }
```

```rust
use nx_layout_toolbox::prelude::*;
use std::path::Path;

let mut bntx = read_bntx(&std::fs::read("__Combined.bntx")?)?;
let opts = ImportOptions { quality: Bc7Quality::Fast, ..Default::default() };
import_png_file(&mut bntx, "tex_button_a", Path::new("a.png"), &opts)?;
std::fs::write("__Combined.bntx", write_bntx(&bntx)?)?;
```

The default `cli` feature builds the `nx-layout-toolbox` binary; library
consumers use `default-features = false`. High-level building blocks live in
`bflyt` (mutation ops on `BFLYT`), `bflan`, `bntx` (+ `bntx::pipeline` for
PNG/DDS import/replace and `bntx::decode` for export), `texpipe`, `dds`,
`sarc` (incl. the custom writer), `manifest`, `layout` (`apply_manifest` /
`validate_manifest` / `apply_manifest_to_arc`), `diff`, and `audit`.

## End-to-end SGPO workflow

```bash
# Unpack the original layout archive.
nx-layout-toolbox sarc-unpack -i info_melee.arc -o unpacked/

# Apply the SGPO skin manifest: encodes 4 face-button PNGs to BC7,
# appends them to BNTX, then adds the matching BFLYT panes/materials.
nx-layout-toolbox layout-apply-manifest \
  --layout-dir unpacked/ \
  --manifest skin_manifest.json \
  --skin-dir my_skin_pngs/ \
  --quality fast

# Verify the result matches the manifest (4/4 elements should pass).
nx-layout-toolbox layout-validate-manifest \
  --layout-dir unpacked/ \
  --manifest skin_manifest.json

# Repack into a deployable SARC.
nx-layout-toolbox sarc-pack -i unpacked/ -o info_melee_modded.arc
```

## Verbs

Read-only / inspect:

```text
bflyt-inspect             JSON or human-readable snapshot of a BFLYT
bflan-inspect             JSON snapshot of a BFLAN (sections + pat1/pai1)
bntx-inspect              JSON or human-readable snapshot of a BNTX
bntx-export-png           Deswizzle + decode one texture to a PNG
bntx-export-all           Export every texture in a BNTX to PNGs
bntx-export-dds           Export one texture to a DDS (DX10) file
layout-diff               Structured before/after diff of two layout.arc
layout-audit              Recursive scan for unsupported/suspicious structures (JSON)
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
bntx-import-png           Encode a PNG to BC7 + swizzle, append to BNTX
bntx-replace-png          Re-encode a PNG over an existing texture (format-preserving)
bntx-remove-texture       Remove a named texture (shrinks string pool / dict / BRTD)
bntx-import-dds           Swizzle a DDS surface and append as a new texture
bntx-replace-dds          Splice a DDS surface over an existing texture in place
layout-apply-manifest     End-to-end on an unpacked dir: PNGs + manifest -> BFLYT + BNTX
layout-apply-arc          Same, directly on a packed layout.arc (unpack/apply/validate/repack)
sarc-pack                 Pack a directory into a SARC archive
```

Internal/debug (used to develop and validate the writers; preserved
because they're useful when extending format support):

```text
bflyt-roundtrip-test      Read a BFLYT, write it back, byte-diff
bflyt-section-diff        Per-section size diff vs. the original
bflyt-mat1-diff           Per-material size diff vs. the original
bntx-roundtrip-test       Read a BNTX, write it back, byte-diff
bntx-dict-test            Rebuild the _DIC Patricia trie and verify lookups
bntx-rlt-dump             Dump the _RLT relocation-table layout
bntx-layout-dump          Dump per-texture data offsets/alignment in BRTD
```

Run `nx-layout-toolbox <verb> --help` for the per-verb option list.

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
├── error.rs           Unified high-level Error / Result
├── bflyt/             BFLYT v8/v9 parser/writer + mutation ops (ops.rs)
│   ├── sections.rs    Type definitions
│   ├── read.rs        Parser
│   └── write.rs       Writer (byte-identical round-trip)
├── bflan.rs           BFLAN parser/writer (verbatim sections) + pat1/pai1 inspect
├── bntx/              BNTX parser/writer
│   ├── mod.rs         BntxFile + Texture types; append/remove
│   ├── read.rs        Full-fidelity parser
│   ├── write.rs       Writer (byte-identical round-trip)
│   ├── decode.rs      Deswizzle + decode (texture2ddecoder) -> RGBA
│   ├── pipeline.rs    PNG/DDS import + format-preserving replace + DDS export
│   └── dict_builder.rs  Patricia-trie builder for the _DIC section
├── texpipe.rs         PNG -> RGBA8 -> BC1/BC3/BC4/BC5/BC7 -> Tegra swizzle
├── dds.rs             DDS (DX10) read/write; DXGI <-> TextureFormat
├── sarc.rs            SARC read (sarc crate) + custom per-file-alignment writer
├── manifest.rs        SGPO skin manifest schema (serde)
├── layout.rs          apply_manifest / validate_manifest / apply_manifest_to_arc
├── diff.rs            Structured BFLYT+BNTX before/after diff
├── audit.rs           Recursive unsupported/suspicious-structure scan -> JSON
└── verbs/             One module per CLI verb
```

## Dependencies

All MIT or MIT/Apache-2.0:

- [`clap`](https://crates.io/crates/clap) — CLI parsing
- [`binrw`](https://crates.io/crates/binrw), [`byteorder`](https://crates.io/crates/byteorder) — binary IO helpers
- [`serde`](https://crates.io/crates/serde), [`serde_json`](https://crates.io/crates/serde_json) — JSON
- [`image`](https://crates.io/crates/image) — PNG/JPG/BMP decoding
- [`intel_tex_2`](https://crates.io/crates/intel_tex_2) — BCn encoder via Intel ISPC
- [`texture2ddecoder`](https://crates.io/crates/texture2ddecoder) — BCn decoder (for PNG/DDS export)
- [`tegra_swizzle`](https://crates.io/crates/tegra_swizzle) — Tegra X1 block-linear swizzle
- [`sarc`](https://crates.io/crates/sarc) — SARC archive **reading** (writing uses a custom per-file-alignment writer in `sarc.rs`)
- [`anyhow`](https://crates.io/crates/anyhow), [`thiserror`](https://crates.io/crates/thiserror) — errors
- [`walkdir`](https://crates.io/crates/walkdir) — directory traversal

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
- BNTX support targets version `0x00040000` (Smash-era). TotK BNTX
  (`0x00040100`) and ASTC formats are not yet handled.
- BNTX append supports 2D, cube, and multi-mip; PNG import re-encodes to
  BC7, while in-place replace preserves the existing format
  (BC1/BC3/BC4/BC5/BC7). BC2 and BC6 have no encoder.
- BNTX texture data alignment defaults to 0x200 in `bntx-import-png` /
  `layout-apply-manifest` (good to ~256x256). Use `--align 0x1000` for
  512x512+ textures.
- The custom SARC writer derives per-file alignment from content (BNTX/
  BNSH on 0x1000, layout files at the 8-byte minimum), so repacked
  archives are close to the original size rather than padded to 0x2000.
  It does not deduplicate identical files.
- v9 BFLYTs include an undocumented material extension on some materials;
  it's captured verbatim for round-trip and reproduced when cloning.

## Round-trip test corpus

The BFLYT writer is validated against 508 BFLYT across real Smash
Ultimate UI archives plus HDR / training-modpack community mods; the
BFLAN writer against 5838 BFLAN; the BNTX writer against the game
`__Combined.bntx` files; and the custom SARC writer against a full
`layout.arc` repack. Tests live in `tests/` and are skipped when the
(gitignored) `tests/fixtures/` corpus is absent. To reproduce on your
own copy:

```bash
nx-layout-toolbox sarc-unpack -i layout.arc -o unpacked/
for f in unpacked/blyt/*.bflyt; do
  nx-layout-toolbox bflyt-roundtrip-test -i "$f"
done
nx-layout-toolbox bntx-roundtrip-test -i unpacked/timg/__Combined.bntx
```
