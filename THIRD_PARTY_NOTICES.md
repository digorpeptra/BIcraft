# Third-party notices

BIcraft is licensed under MIT OR Apache-2.0. It includes material from the third parties
listed below, under their own licenses. The licence texts of the material embedded in the
binary are also under [`licenses/`](licenses/); release archives and the container image ship
that folder, and `bicraft licenses` prints this file and those texts. Every asset is listed in
[ATTRIBUTION.md](ATTRIBUTION.md).

## Microsoft XPress9 (Extensible Storage Engine)

| | |
|---|---|
| Component | XPress9 compression codec |
| Upstream | <https://github.com/microsoft/Extensible-Storage-Engine>, directory `dev/ese/src/_xpress9` |
| Commit | `7030fe7407615160e54d152e4ef704eede2fdd7e` |
| License | MIT. Copyright (c) Microsoft Corporation. |
| Used in | `crates/xpress9` (`bicraft-xpress9`) |

**What was ported.** The crate is a translation of the C sources into safe Rust:

- the decoder: `Xpress9DecLz77.c`, `Xpress9Lz77Dec.i`, `Xpress9DecHuffman.c`
- the encoder: `Xpress9EncLz77.c`, `Xpress9Lz77EncPass1.i`, `Xpress9Lz77EncPass2.i`,
  `Xpress9Lz77EncInsert.i`, `Xpress9Lookup.i`, `xpress9lookup.i2`, `xpress9lookup.i3`,
  `Xpress9EncHuffman.c`, and the hashing scheme of the SSE2 insertion path
- shared code: `Xpress9Misc.c` (header CRC, bit reversal), the constants and bit-I/O macros of
  `Xpress9Internal.h`, and the data table of `Xpress9ZobristTable.h`

Each ported Rust file starts with a header naming its upstream file and commit. The full license
text is in [`crates/xpress9/LICENSE-MIT-microsoft-xpress9`](crates/xpress9/LICENSE-MIT-microsoft-xpress9)
and reproduced here:

```text
MIT License

Copyright (c) Microsoft Corporation.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE
```

The C sources themselves are not part of this repository. They were used only on a development
machine, compiled with gcc, as a test oracle (see `crates/xpress9/README.md`).

## pbixray Kaitai format descriptions (CC0)

| | |
|---|---|
| Component | `docs/abf_xpress9.ksy` (DataModel XPress9 framing) |
| Upstream | <https://github.com/Hugoberry/pbixray/blob/main/docs/abf_xpress9.ksy> |
| License | CC0-1.0 (public domain dedication) |
| Used in | `crates/xpress9/src/datamodel.rs` |

No text or code was copied. The `DataModel` framing (banner, chunk envelopes, multithreaded
thread-distribution header) was implemented from this description, which is cited in the module.

## Microsoft PBIR JSON schemas (MIT)

| | |
|---|---|
| Component | Power BI enhanced report format (PBIR) JSON schemas: `report`, `page`, `visualContainer`, `bookmark`, `filterConfiguration`, `reportExtension`, … |
| Upstream | <https://github.com/microsoft/json-schemas>, folder `fabric/item/report/definition` |
| Commit | `49f4a76cb7a75fee2669b7cc6ba12ca4332050f7` |
| License | MIT. Copyright (c) Microsoft Corporation. |
| Used in | `crates/pbir` (`bicraft-pbir`): validation of PBIR documents against the exact `$schema` they declare |

The files are vendored unmodified under `crates/pbir/schemas/`, next to the upstream license
(`crates/pbir/schemas/LICENSE`) and a `SOURCE.txt`; see `docs/formats/pbir.md`.

## Microsoft powerbi-visuals-api capabilities schema (MIT)

| | |
|---|---|
| Component | `schema.capabilities.json` (the JSON schema of a custom visual's `capabilities.json`) |
| Upstream | npm package `powerbi-visuals-api` 5.3.0 (<https://github.com/microsoft/powerbi-visuals-api>) |
| License | MIT. Copyright (c) Microsoft Corporation. |
| Used in | `crates/engine/tests/report/visual_runtime_tests.rs`: validation of BIcraft's visuals' capabilities (tests only; not embedded in the binary) |

The file is vendored unmodified under `crates/engine/tests/report/powerbi-visuals-api/`, next to
the upstream license (`LICENSE`) and a `SOURCE.txt`; see `docs/community-visuals.md` "Runtime".

## Microsoft Power BI report theme JSON schema (MIT)

| | |
|---|---|
| Component | `reportThemeSchema-2.159.json` (Power BI report theme JSON schema) |
| Upstream | <https://github.com/microsoft/powerbi-desktop-samples>, folder `Report Theme JSON Schema` |
| Commit | `34f78db38030195329a5ce05323251f778772fe1` |
| License | MIT. Copyright (c) Microsoft Corporation. |
| Used in | `crates/visuals` (`bicraft-visuals`): the catalog of visual formatting options; `crates/engine/src/theme_edit.rs`: validation of report themes (`theme.*` commands) |

The file is vendored unmodified under `crates/visuals/schemas/`, next to the upstream license
(`crates/visuals/schemas/LICENSE`) and a `SOURCE.txt`. It is embedded in the binary and parsed at
run time; see `docs/formats/visual-options.md`.

## Selawik font (SIL Open Font License 1.1)

| | |
|---|---|
| Component | Selawik Regular, Semibold, Bold and Light (`selawk.ttf`, `selawksb.ttf`, `selawkb.ttf`, `selawkl.ttf`) |
| Upstream | <https://github.com/microsoft/Selawik>, release 1.01 (`Selawik_Release.zip`) |
| License | SIL Open Font License 1.1. Copyright 2015, Microsoft Corporation, with Reserved Font Name Selawik. |
| Used in | `crates/render` (`bicraft-render`): the fallback font of `report.render` when Segoe UI is not installed (Linux, macOS); `crates/ui`: the UI font of the desktop and browser app and the font of the report pages they draw |

Selawik is an open-source font metric-compatible with Segoe UI, Power BI's default font. The four
files are vendored unmodified under `crates/render/fonts/` next to the full license text
(`crates/render/fonts/OFL-Selawik.txt`) and embedded in the binary. The fonts are not sold by
themselves; the Reserved Font Name is kept (the files are not modified).

PDF files written by `report.export_pdf` embed subsets of the bundled faces (Selawik and the
Noto fallback faces below) holding the glyphs the document uses, with each font's `name`
table (its copyright and licence notices) kept. The SIL OFL allows embedding fonts in
documents; the OFL FAQ treats a document embedding a font subset as a document, not a Modified Version
of the Font Software, so the subsets keep their font names. System fonts are embedded only when their `OS/2` `fsType`
allows it ([docs/render.md](docs/render.md), "PDF").

## Carlito (SIL Open Font License 1.1)

| | |
|---|---|
| Component | Glyph subsets of Carlito Regular, Bold, Italic and Bold Italic, renamed "BIcraft Metric Sans" (`crates/render/fonts/carlito/*-subset.ttf`) |
| Upstream | <https://github.com/google/fonts>, `ofl/carlito/` at commit `07ace6abab87a122865e5cb82c7540b39551edb2` (Version 1.104); upstream project <https://github.com/googlefonts/carlito> |
| License | SIL Open Font License 1.1. Copyright 2013 The Carlito Project Authors, with Reserved Font Name "Carlito". |
| Used in | `crates/render` (`bicraft-render`): the stand-in for Calibri (the same advance widths) in `report.render`, `report.export_pdf` and the report pages of the app, when no Calibri face is installed |

Each subset keeps the Latin characters listed in `crates/render/fonts/carlito/SOURCE.txt` (with
the source URLs and SHA-256 of the upstream files and of the subsets) and drops the others; they
were made by `cargo xtask font-subset carlito`. They are Modified Versions under the OFL, so
they do not use the Reserved Font Name: their `name` table calls them "BIcraft Metric Sans" and
keeps the font's copyright, version, designer and licence strings. They ship with the full
licence text (`crates/render/fonts/carlito/OFL-Carlito.txt`, the upstream `OFL.txt`
unmodified; `licenses/carlito/`) and are not sold by themselves.

## Noto Sans Symbols 2 (SIL Open Font License 1.1)

| | |
|---|---|
| Component | A glyph subset of Noto Sans Symbols 2 Regular (`crates/ui/fonts/NotoSansSymbols2-subset.ttf`) |
| Upstream | <https://github.com/notofonts/symbols>, release `NotoSansSymbols2-v2.008` (`NotoSansSymbols2-v2.008.zip`, file `NotoSansSymbols2/unhinted/ttf/NotoSansSymbols2-Regular.ttf`) |
| License | SIL Open Font License 1.1. Copyright 2022 The Noto Project Authors. No Reserved Font Name. |
| Used in | `crates/ui` (`bicraft-ui`): the symbols of the app's UI (`✓`, `✕`, `✖`, `⚠`, `●`, `▶`…), in the desktop app and in the browser app that `bicraft` serves |

The subset keeps the outlines of the characters listed in `crates/ui/fonts/SOURCE.txt` (with the
SHA-256 of the release archive, of the source font and of the subset) and drops the others; it
was made by `cargo xtask font-subset`. It is a Modified Version under the OFL: it keeps the
font's copyright and licence in its `name` table, ships with the full licence text
(`crates/ui/fonts/OFL-NotoSansSymbols2.txt`, `licenses/noto-sans-symbols2/`) and is not sold by
itself.

## Noto fallback faces of the renderer (SIL Open Font License 1.1)

| | |
|---|---|
| Component | Glyph subsets of Noto Sans Regular and Bold, Noto Sans Symbols 2, Noto Sans Math, Noto Sans Symbols and Noto Emoji (`crates/render/fonts/*-subset.ttf`) |
| Upstream | Noto Sans: <https://github.com/notofonts/latin-greek-cyrillic>, release `NotoSans-v2.015`; Noto Sans Symbols 2: <https://github.com/notofonts/symbols>, release `NotoSansSymbols2-v2.008`; Noto Sans Math: <https://github.com/notofonts/math>, release `NotoSansMath-v3.000`; Noto Sans Symbols: <https://github.com/notofonts/symbols>, release `NotoSansSymbols-v2.003`; Noto Emoji (monochrome): <https://github.com/google/fonts> `ofl/notoemoji/NotoEmoji[wght].ttf` at commit `8b0a1d0f5983c89bc2b93f1b5fb55f9e252744b5` |
| License | SIL Open Font License 1.1. Copyright 2022 The Noto Project Authors (Noto Emoji: Copyright 2013 Google LLC). No Reserved Font Name. |
| Used in | `crates/render` (`bicraft-render`): the fallback faces after Selawik for characters it lacks (Greek, Cyrillic, currency, superscripts, arrows, math, shapes, symbols, emoji) in `report.render` and the pages of the app; `crates/ui`: the same faces in the app's own text |

Each subset keeps the characters listed in `crates/render/fonts/SOURCE.txt` (with the release,
archive URL and SHA-256, the source file and its SHA-256, and the subset's SHA-256) and drops the
others; they were made by `cargo xtask font-subset bundle` (Noto Emoji's default instance, its
variation tables dropped). They are Modified Versions under the OFL: each keeps the font's
copyright and licence in its `name` table, ships with the full licence text
(`crates/render/fonts/OFL-*.txt`, the upstream `OFL.txt` files unmodified; `licenses/noto-sans/`,
`licenses/noto-sans-symbols2/`, `licenses/noto-sans-math/`, `licenses/noto-sans-symbols/`,
`licenses/noto-emoji/`) and is not sold by itself.

## DeckCraft UI code (MIT OR Apache-2.0)

| | |
|---|---|
| Component | The design tokens, widgets and icon drawing toolkit of BIcraft's UI (`crates/ui/src/theme.rs`, `widgets.rs`, `icons.rs`); the File view (Backstage: side menu, recent-file rows, place rows, task buttons) in `crates/ui/src/backstage.rs` |
| Upstream | <https://github.com/storytold/deckcraft>, `crates/ui-egui/src/` at `0ec0cc5e815caea982a7deec51bc437f3ba187ad`; `backstage.rs` and `backstage/files.rs` at `c252a681a0e219b8a12ca4568822411e90b01a39` |
| License | MIT OR Apache-2.0. Copyright (c) 2026 ArtCraft Team and the DeckCraft contributors. |
| Used in | `crates/ui`: the look of the desktop and browser app ([docs/design.md](docs/design.md)) |

DeckCraft is a sibling project under the same licences as BIcraft; its MIT licence text is in
[`crates/ui/LICENSE-MIT-deckcraft`](crates/ui/LICENSE-MIT-deckcraft) and under `licenses/deckcraft/`. Each ported file names its upstream file and commit in its
header; the code was adapted (BIcraft's accent colour, fonts and never-crash rules).

## DashCraft Power Query and Power Pivot code (MIT OR Apache-2.0, used under MIT)

| | |
|---|---|
| Component | Standard M library functions (lists, tables, numbers, binary, JSON, values and types, records, splitters) and the M formatter |
| Upstream | <https://github.com/Jxck-S/dashcraft>, `crates/pbi-m/src/stdlib/` (`list.rs`, `table.rs`, `table_ops.rs`, `table_rows.rs`, `number.rs`, `binary.rs`, `text.rs`, `value_fns.rs`, `record.rs`) and `crates/pbi-m/src/service/format.rs` |
| Commit | `0958fd82b04656ee1797101a2b4df8d114dfebd1` |
| License | MIT OR Apache-2.0, used here under MIT. Copyright (c) 2026 Jack Sweeney and the DashCraft contributors. |
| Used in | `crates/data` (`bicraft-data`): `src/m/lib/list_more.rs`, `table_more.rs`, `misc_more.rs` (ROADMAP M11.34), `src/m/format.rs` and its tests `src/m/tests_format.rs` (M11.35) |

**Power Pivot import (M11.33).** From the same commit, `crates/pbi-vertipaq/src/xpress8.rs` (the
Plain LZ77, "xpress8", [MS-XCA] decoder of Power Pivot backups) is ported into
`crates/xpress8/src/decode.rs` (`bicraft-xpress8`), and from
`crates/pbi-vertipaq/src/metadata/xml.rs` the `CREATE MEASURE` splitter, the XML name decoder and
the object ID parser into `crates/powerpivot/src/xml.rs` (`bicraft-powerpivot`), adapted to
BIcraft's never-crash rules and limits; the decoder was checked against [MS-XCA]. The encoder of
`crates/xpress8` and the rest of the Power Pivot reader are BIcraft's own, written from
[docs/formats/powerpivot.md](docs/formats/powerpivot.md). The same licence text is also in
[`crates/xpress8/LICENSE-MIT-dashcraft`](crates/xpress8/LICENSE-MIT-dashcraft).

**What was ported.** The logic of the library functions listed in
[docs/formats/data-sources.md](docs/formats/data-sources.md) *Library* › *Added in M11.34*,
rewritten over bicraft's own M values and checked against each function's Microsoft Learn page;
and the formatter's comment attachment, layout commands, emitter and Wadler-style renderer, driven
by a syntax tree that mirrors bicraft's own M grammar. DashCraft's recordings of Desktop (its
library listing and reference outputs) were not used. Each ported file names its upstream files
and commit in its header. The licence text is in
[`crates/data/LICENSE-MIT-dashcraft`](crates/data/LICENSE-MIT-dashcraft) (copied to
`licenses/dashcraft/`) and reproduced here:

```text
MIT License

Copyright (c) 2026 Jack Sweeney and the DashCraft contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## d3 chart layout algorithms (BSD-3-Clause)

| | |
|---|---|
| Component | The tick-step and nice-domain rule, the ordinal band layout and the squarified treemap layout of d3 version 3; the tick increment of d3-array |
| Upstream | <https://github.com/d3/d3> at tag `v3.5.17` (commit `9cc9a875e636a1dcf36cc1e07bdf77e1ad6e2c74`): `d3_scale_linearTickRange` (`src/scale/linear.js`), `rangeBands` (`src/scale/ordinal.js`), `squarify` and `position` of `d3.layout.treemap` (`src/layout/treemap.js`); <https://github.com/d3/d3-array> at tag `v1.2.4` (commit `fda02e084d50c880c695a3fe0e59cdf1a1ba5ea9`): `tickIncrement` (`src/ticks.js`) |
| License | BSD-3-Clause. Copyright (c) 2010-2016, Michael Bostock (d3); Copyright 2010-2016 Mike Bostock (d3-array). |
| Used in | `crates/render` (`bicraft-render`): `src/axes.rs` (`d3_step`, `d3_increment`, `Bands`) and `src/parts.rs` (`squarify`) |

Each ported function names its upstream function, file and commit in a comment; the code was
translated to safe Rust and adapted (guards against non-finite input, bounded loops). The licence texts are in
[`crates/render/LICENSE-BSD-d3`](crates/render/LICENSE-BSD-d3) and
[`crates/render/LICENSE-BSD-d3-array`](crates/render/LICENSE-BSD-d3-array) and under
`licenses/d3/`.

## powerbi-visuals-utils-typeutils (MIT, facts only)

| | |
|---|---|
| Component | The `PrimitiveType` enumeration of `src/valueType.ts` (numeric codes of value types) |
| Upstream | <https://github.com/microsoft/powerbi-visuals-utils-typeutils> |
| License | MIT. Copyright (c) Microsoft Corporation. |
| Used in | `crates/layout-bb/src/codes.rs`: the numeric `dataType` of report-level measures in the legacy layout. Only the codes are used; no code is copied. |

## Rust crates used by the renderer

`bicraft-render` uses tiny-skia (BSD-3-Clause), skrifa and read-fonts (MIT OR Apache-2.0),
harfrust (MIT; a Rust port of HarfBuzz, copyright the HarfBuzz developers), resvg/usvg
(Apache-2.0 OR MIT), zune-jpeg (MIT OR Apache-2.0 OR Zlib), gif (MIT OR Apache-2.0) and
image-webp (MIT OR Apache-2.0), all pure Rust, through Cargo.

## Crates of the desktop app

`bicraft-desktop` (`apps/bicraft-desktop`, docs/desktop.md) also links, from crates.io and
unmodified: egui, eframe, egui-wgpu, egui-winit, epaint, emath, ecolor and wgpu (MIT OR
Apache-2.0), winit and its platform crates (Apache-2.0 and MIT), accesskit (MIT OR Apache-2.0),
rfd (MIT), sys-locale (MIT OR Apache-2.0), windows-registry (MIT OR Apache-2.0, Windows only),
the same HTTPS client as the Databricks connector (below), and at build time resvg, tiny-skia and
winresource (MIT) for its icon. egui embeds its default fonts (`epaint_default_fonts`): Ubuntu
Light (Ubuntu Font Licence 1.0), Hack (MIT and Bitstream Vera licence), Noto Emoji (OFL-1.1) and
an emoji icon font (MIT and OFL-1.1), with its five-glyph `egui-icons` subset (same licence); with Selawik and the Noto Sans Symbols 2 subset (above)
they draw the app's chrome, never report pages.
`THIRD_PARTY_CRATES.md` in the Windows archive lists every crate with its licence texts, the
fonts' included.

## Microsoft Best Practice Rules for tabular models (MIT)

| | |
|---|---|
| Component | `BPARules.json`: 71 best-practice rules for tabular models (Dynamic LINQ expressions) |
| Upstream | <https://github.com/microsoft/Analysis-Services>, file `BestPracticeRules/BPARules.json` |
| Commit | `50e8ce5028dd7046e73974ec69cb79af225e41b0` (2026-01-21) |
| License | MIT. Copyright (c) 2016 Microsoft. The repository's license; the `BestPracticeRules` folder carries no other terms. |
| Used in | `crates/bpa` (`bicraft-bpa`): the default rule set of `model.analyze` / `bicraft analyze` |

The English rule file is vendored unmodified under `crates/bpa/rules/`, next to the upstream
license (`crates/bpa/rules/LICENSE`) and a `SOURCE.txt`, and embedded in the binary; the
translated rule files are not included. See `docs/formats/bpa.md`.

## Unicode CLDR locale data (Unicode License v3)

| | |
|---|---|
| Component | Unicode Common Locale Data Repository (CLDR) 48.0, as shipped in ICU 78.3 |
| Upstream | <https://cldr.unicode.org/> |
| License | Unicode License v3. Copyright © 1991-2025 Unicode, Inc. |
| Used in | `crates/locale` (`bicraft-locale`): a subset (names, separators, percent pattern, compact display units, first day of the week) generated into `src/data.rs` |

Only the needed subset is kept, as generated Rust data (`tools/locale-oracle/gen_data.mjs`); the
license text is in `crates/locale/LICENSE-UNICODE` and the provenance in
`crates/locale/SOURCE.txt`. See `docs/formats/locales.md`.

## HTTPS client of the Databricks, Web and OData connectors, TLS of the SQL Server connector

`crates/databricks`, `crates/webdata` and `crates/sqlserver` (which uses `rustls`, `ring` and
`webpki-roots` directly for TDS encryption), and so the `bicraft` binary, link these crates from
crates.io, under their own permissive licences (no source was copied); `crates/webdata` also
reads OData `$metadata` with `roxmltree` (MIT OR Apache-2.0), already linked for SVG:

| Crate | Licence |
|---|---|
| `ureq`, `ureq-proto`, `rustls-pki-types`, `utf8-zero`, `percent-encoding`, `base64` | MIT OR Apache-2.0 |
| `rustls` | Apache-2.0 OR ISC OR MIT |
| `ring` | Apache-2.0 AND ISC |
| `rustls-webpki`, `untrusted` | ISC |
| `subtle` | BSD-3-Clause |
| `webpki-roots` (Mozilla's root certificates) | CDLA-Permissive-2.0 |
| `rpassword`, `rtoolbox` (CLI masked prompt) | Apache-2.0 |

## TypeScript compiler of custom visual projects (swc)

`crates/tsbuild` (`bicraft-tsbuild`, used by `visual.build` and `visual.pack`), and so the
`bicraft` binary and the desktop app, link swc (<https://swc.rs>) from crates.io, unmodified and
under its own licence (no source was copied):

| Crate | Licence |
|---|---|
| `swc_bundler`, `swc_common`, `swc_ecma_ast`, `swc_ecma_codegen`, `swc_ecma_loader`, `swc_ecma_parser`, `swc_ecma_transforms_base`, `swc_ecma_transforms_typescript` and the swc crates they use (`swc_atoms`, `swc_visit`, `swc_ecma_utils`, `swc_ecma_visit`, `swc_ecma_transforms_optimization`, `swc_ecma_transforms_react`, `swc_graph_analyzer`, `hstr`, …) | Apache-2.0 |
| `anyhow`, `petgraph`, `stacker`, `psm`, `relative-path`, `bumpalo` and the other crates they pull in | MIT OR Apache-2.0, MIT, BSL-1.0 (`ryu`, `dragonbox_ecma`) or Unicode-3.0 (`icu_*`) |

## YAML reader of `bicraft test`

`crates/engine` (and so the `bicraft` binary) links these crates from crates.io to read
`bicraft.tests.yaml` (`docs/ci-tests.md`), under their own permissive licences (no source was
copied):

| Crate | Licence |
|---|---|
| `yaml-rust2`, `arraydeque`, `hashlink` | MIT OR Apache-2.0 |
