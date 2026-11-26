# Legacy layout (public sources and black-box tests): source log

The sources of `crates/layout-bb` (package `bicraft-layout`) and
[`legacy-layout-bb.md`](legacy-layout-bb.md). Facts in the note carry one of the ids below.

Desktop was run on our files; the files it saved were read.

## Sources

| Id | Source | Licence / status | Used for |
|---|---|---|---|
| API | Call sites of `bicraft_layout::` in `crates/engine`, `crates/pbix`, `crates/pbip`, `fuzz/`, and those callers' tests | our code | the public API (functions, constants, error use) and the shapes the engine expects under the legacy keys |
| SCH | Vendored PBIR JSON schemas, `crates/pbir/schemas/` (microsoft/json-schemas, commit 49f4a76) | MIT | PBIR property names, allowed formatting objects (`report` 3.3.0 `ReportFormattingObjects`, `filterConfiguration` 1.3.0 `FilterContainerFormattingObjects`), settings list (`ExplorationSettings`), enumerations (`PageDisplayOption`, `VisualInteractionFilterType`, `BindingType`, `HowCreated`, `filterSortOrder`, `SortDirection`), `reportExtension` 1.0.0 measure shape |
| TU | microsoft/powerbi-visuals-utils-typeutils, `src/valueType.ts`, `PrimitiveType` enumeration (read 2026-10-28 on GitHub) | MIT, facts only, no code copied | numeric `dataType` of report-level measures |
| LO-* | Black-box pairs under `%BICRAFT_TESTDATA%\layout-oracle\<case>\`: `legacy\` is a generated legacy PBIP project (synthetic model `Sales`/`Regions`, no client data), `desktop\definition\` is what Power BI Desktop wrote when it upgraded that project to PBIR on save (`tools/layout-oracle/run.ps1`). The files record no Desktop version; they are dated 2026-10-15, before Desktop was updated to 2.159.883.0 on 2026-10-19, so they come from the version installed then (2.158.1304.0, inferred from the file dates). Cases: `report`, `pages`, `filters`, `visuals`, `groups`, `bookmarks`, `mobile`, `customvisual` | Desktop output, local only | the legacy structure (key names, nesting, JSON-in-string slots, numeric codes) and the legacy → PBIR mapping |
| LX-* | Our own black-box runs: variants of the LO inputs (built by a local `jq` script) upgraded by Desktop **2.159.883.0** with the same `run.ps1` | Desktop output, local only | codes and rules the LO pairs did not settle (see the note's "Unknowns") |

## Committed oracle pairs (2026-10-29)

`crates/engine/tests/fixtures/layout-oracle/{groups,visuals}`: the `groups` and `visuals` cases of
`cargo xtask layout-oracle` (our own generated legacy projects), converted by Desktop 2.159.883.0
with `tools/layout-oracle/run.ps1`. `layout_oracle` compares them on every run; Desktop 2.159
wrote `reportVersionAtImport` 2.14.0 for theme version 5.99 where 2.158 wrote 2.13.0, so the theme
table follows 2.159 and the local 2.158 pairs are compared without that value.
