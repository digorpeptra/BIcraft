# PBIP projects: what `project.new` writes (Power BI Desktop 2.158.1304.0)

Implemented by `bicraft_pbip::create_project` / `create_project_with_ids` and `save_model`
(`crates/pbip/src/create.rs`). The loader rules this relies on are in
[pbix-package.md, section 6](pbix-package.md#6-pbip-power-bi-project-layout).

Sources: Microsoft's public JSON schemas and Learn pages on PBIP, black-box tests against Power BI
Desktop 2.157 and 2.158, and the packaging facts of pbix-package.md; detailed provenance is kept
privately by the maintainers.

## Sources

| Source | Used for |
|---|---|
| `pbix-package.md` §6 (Desktop 2.158.1304.0) | File inventory, `version` values and what they gate, `$schema` matching rule (major version must be known), UTF-8 without BOM, `.gitignore` lines |
| Microsoft's public JSON schemas, <https://github.com/microsoft/json-schemas/tree/main/fabric> (fetched 2026-10-13; not vendored) | Required properties and the latest schema version of every file; release dates in each `CHANGELOG.md` |
| A project saved by Power BI Desktop 2.157 on the development machine (not committed) | What Desktop actually writes today: property order, 2-space indentation, which files carry `$schema`, PBIR schema versions, report `settings` |

## Files created

For `create_project(parent, "Sales", model, format)`:

| Path (under `parent`) | Content |
|---|---|
| `Sales.pbip` | `$schema` pbipProperties 1.0.0, `version` `1.0`, `artifacts: [{report: {path: "Sales.Report"}}]`, `settings: {enableAutoRecovery: true}` |
| `.gitignore` | `**/.pbi/localSettings.json` and `**/.pbi/cache.abf`. Created when absent; when present, only the missing lines are appended. |
| `Sales.SemanticModel/definition.pbism` | `$schema` semanticModel/definitionProperties 1.0.0, `version` `4.2`, `settings: {}` |
| `Sales.SemanticModel/.platform` | `$schema` platformProperties 2.0.0, `metadata: {type: "SemanticModel", displayName: "Sales"}`, `config: {version: "2.0", logicalId: <GUID>}` |
| `Sales.SemanticModel/definition/` | TMDL, from `bicraft_tmdl::write_folder` (format `Tmdl`) |
| `Sales.SemanticModel/model.bim` | TMSL, from `bicraft_tmsl::write` (format `Tmsl`); never both |
| `Sales.Report/definition.pbir` | `$schema` report/definitionProperties 2.0.0, `version` `4.0`, `datasetReference: {byPath: {path: "../Sales.SemanticModel"}}` |
| `Sales.Report/.platform` | as above with `type: "Report"` and its own GUID |
| `Sales.Report/definition/version.json` | `$schema` versionMetadata 1.0.0, `version` `2.0.0` |
| `Sales.Report/definition/report.json` | `$schema` report 3.3.0, `themeCollection: {}`, `settings` as Desktop writes them for a new report |
| `Sales.Report/definition/pages/pages.json` | `$schema` pagesMetadata 1.1.0, `pageOrder: [<page>]`, `activePageName: <page>` |
| `Sales.Report/definition/pages/<page>/page.json` | `$schema` page 2.1.0, `name`, `displayName: "Page 1"`, `displayOption: "FitToPage"`, `height: 720`, `width: 1280` |

`<page>` is 20 random hex digits (Desktop's page-name shape); logical ids are random version-4
GUIDs drawn from `std` entropy (`RandomState` keys, clock, process id, counter: unique, not
cryptographic). `create_project_with_ids` takes the three ids from the caller, so the output is
byte-for-byte deterministic.

No `.pbi/` folder is written: `localSettings.json`, `editorSettings.json` and `cache.abf` are
local state that Desktop creates on first open.

## Versions chosen, and why

| File | Choice | Why |
|---|---|---|
| `.pbip` | schema 1.0.0, `version` 1.0 | Only version that exists. |
| `definition.pbism` | schema 1.0.0, `version` 4.2 | Desktop 2.157 writes 4.2. TMDL is read from 4.0 (Learn; 3.0 also opens, ORA-j05); 6.0 also reads the `Copilot/` folder (pbix-package.md §2), so BIcraft raises it only when it writes one. Harmless for `model.bim`, because a `definition/` folder is not created then. |
| `definition.pbir` | schema 2.0.0, `version` 4.0 | Desktop writes exactly this. PBIR `definition/` is read from 4.0 (Learn; 3.0 also opens, ORA-j01). |
| `.platform` | schema 2.0.0, `config.version` 2.0 | What Desktop 2.157 writes; 2.1.0 only adds the optional `sensitivityLabelId`. |
| PBIR `version.json` | schema 1.0.0, `version` 2.0.0 | What Desktop writes. |
| `report.json` | schema 3.3.0 | Latest published (May 2026) and what Desktop 2.157 writes. Requires only `$schema` and `themeCollection`. |
| `pages.json` | schema 1.1.0 | Latest (May 2026), what Desktop writes. |
| `page.json` | schema 2.1.0 | Latest (March 2026), what Desktop writes. Requires `$schema`, `name` (≤ 50 chars), `displayName`, `displayOption`. |

Desktop opens a newer minor version of a schema major it knows (ORA-j12), so these choices also open in a slightly older Desktop as long as the majors match.

All these values live in one table, `crates/versions/src/lib.rs` (`bicraft-versions`), which the
writers use. `cargo xtask schema-watch` compares that table with the schemas Microsoft publishes
(see [../schema-watch.md](../schema-watch.md)).

## Deliberate differences from Desktop's own output

- **LF line endings.** Desktop on Windows writes CRLF. Every reader (JSON, TMDL) is line-ending
  agnostic, and LF keeps output byte-identical on Windows and Linux. No trailing newline, as
  Desktop.
- **`$schema` on `.pbip` and `definition.pbism`.** Desktop 2.157 omits it there (V1 form);
  the newer `$schema` form is accepted by the loader and required by the public schemas.
- **No base theme.** Desktop references `StaticResources/SharedResources/BaseThemes/<theme>.json`,
  a Microsoft asset we must not ship, so `themeCollection` is empty and the report uses the
  default theme. The schema allows this.
- **1280 × 720 page** (Desktop's 16:9 default).

## Safety rules

- The name must be non-empty, ≤ 128 bytes, without `/ \ : * ? " < > |`, control characters or
  `..`, must not start with a space or end with a space or dot, and must not be a Windows device
  name (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`, with or without an extension).
- Creation fails with `AlreadyExists` if `parent` holds `<name>.pbip`, `.SemanticModel`,
  `.Dataset` or `.Report` under **any letter case**, so a project made on Linux stays valid on
  case-insensitive Windows. Files are opened with `create_new`; on any failure, everything this
  call created is removed again (nothing pre-existing is touched).
- `locate` prefers the items named like the `.pbip` file (or item folder) it is given, so several
  projects can share one folder.
- `save_model` writes only the semantic model, in the project's current format: `model.bim` is
  replaced through a temporary sibling file and a rename; TMDL goes through
  `bicraft_tmdl::write_folder`. The report folder and the metadata files are never touched.

## Saving a report (`read_report` / `save_report`, `crates/pbip/src/report.rs`)

- `read_report` reads PBIR (`definition/`, through `bicraft_pbir::read_dir`) or the legacy
  `report.json` (UTF-8 or UTF-16, through `bicraft_layout::read_layout`), as `locate` classified
  the report folder.
- `save_report` always writes PBIR through `bicraft_pbir::write_dir`. A legacy (or empty) report
  folder is converted, in this order: `definition.pbir` is read and checked first; `definition/` is
  written; the write is checked (`definition/report.json` must exist, else the legacy file is kept
  and the call fails); `definition.pbir` `version` is raised to `4.0` when it is below 3.0 (Desktop
  reads `definition/` from 3.0 on, see pbix-package.md §6; Microsoft documents 4.0 for a
  `definition/` folder), other properties kept in order; finally `report.json` is removed
  (PBIR and `report.json` together are an error for Desktop). `mobileState.json` is left alone.
- `blank_report(page)` is the report `create_project` writes, in memory; the engine's
  `report.new` uses it.

## Data cache: `<Name>.SemanticModel/.pbi/cache.abf`

What Desktop keeps next to a project so it reopens with data. Compared on a project and a PBIX
that Desktop 2.158.1304.0 saved from the same model (local sample, held locally and not
committed: only structure is recorded here; tool: `bicraft_xpress9::inspect_datamodel` and `bicraft_abf`):

| Property | `cache.abf` | PBIX `/DataModel` |
|---|---|---|
| Framing | XPress9, **single-threaded** layout: the 102-byte banner, then (u32 uncompressed, u32 compressed, payload) chunks (`xpress9.md` §6) | same |
| Banner bytes | identical | identical |
| Encoder parameters | window 2^22, MTF 4, pointer min match 4, MTF min match 2, from the block headers (re-encoding with the `EncoderParams::power_bi_desktop` preset reproduces the bytes) | same |
| Decompressed | an ABF backup ([MS-XLDM]), same size and page count | same |
| ABF files | same file set (310 here) | |

Inside the two backups:

| ABF file | Same bytes? |
|---|---|
| Column storage: every `.idf`, `.idfmeta`, `.dictionary`, `.hidx` | yes |
| `ADDITIONAL_LOG`, `PARTITIONS` | yes |
| `metadata.sqlitedb` | no (same model, different ids and timestamps) |
| `LOG` (backup log), `0.CryptKey.bin`, the database `.db.xml` | no: each save is a new database (new id, so a different database folder name) |

So the cache is **compressed, in exactly the PBIX `DataModel` format**, and the two are
interchangeable as far as the format goes: `bicraft_pbix::read_data_model` /
`write_data_model` read and write both. The conversions rely on this (see below and the Desktop
check in `pbix-package.md` §10, where Desktop opens a project whose `cache.abf` is a PBIX's
`DataModel`).

## Conversions (`file.save_as` / `file.convert`, `crates/engine/src/convert.rs`)

A project written from another kind of file gets, besides what `create_project` writes:

- **`.pbi/cache.abf`** from the source's data: a PBIX's `DataModel` part as is (with the session's
  metadata edits written in through `write_data_model` when there are some), or another project's
  cache. `cache: false` leaves it out; a source with no data (PBIT, `model.bim`, new model) gets
  none, and Desktop opens the project without data until it is refreshed.
- **`diagramLayout.json`** from the PBIX/PBIT `DiagramLayout` part (UTF-16LE there, UTF-8 here).
- **A legacy report stays legacy** unless `report_format: "pbir"`: `write_legacy_report` writes the
  layout as `report.json` (UTF-8 without BOM), copies `StaticResources/` and `CustomVisuals/`, and
  removes the blank PBIR `definition/` folder (Desktop refuses a folder holding both).
  `definition.pbir` keeps version 4.0: with no `definition/` folder Desktop reads `report.json`.
- The report's raw `StaticResources/` and `CustomVisuals/` files; PBIX part names are
  percent-decoded into file names.

The other way, a project becomes a PBIX only when it has a cache: its bytes become the
`DataModel` part. A project model read from TMDL does not record three things the cache's model
has, which are taken from the cache before comparing (none is user metadata): the database and
model object names, the engine-internal `RowNumber-…` columns, and the letter case of a few
property names (only letter case differs). If the project model then still differs
in a way only new VertiPaq data could follow (a new column, a changed data type…), the conversion
is refused and names the differences: Desktop must refresh and save the project first.

## Desktop answers (2026-10-14)

Settled with Power BI Desktop 2.158.1304.0 on projects `project.new` wrote (opened, saved with the
Save button, files compared):

1. An empty `themeCollection` opens, and Desktop saves it back unchanged.
2. On save, Desktop rewrites `.pbip`, `definition.pbism` **and** `definition.pbir` in the V1 form:
   same properties, no `$schema`, CRLF. BIcraft keeps writing `$schema` (accepted, and required
   by the public schemas); a project Desktop saved simply loses it.
3. A `definition.pbir` that declares the `definitionProperties` 1.0.0 `$schema` with `version` 4.0
   opens (PBIR `definition/` read); `version` decides, not the `$schema` minor/major. On save it
   becomes V1 like the others.

## Public and black-box evidence

The project layout, versions and encodings used here rest on Microsoft Learn (PBIP, report folder,
semantic model folder) and on Desktop runs; see [packaging-verification.md](packaging-verification.md)
(P-* facts). The legacy-to-PBIR save raises `definition.pbir` to `4.0`, the version Microsoft
documents for a `definition/` folder (P-PBIR-VER). Desktop 2.158 also reads
PBIR at 3.0.

A semantic model folder holding both `model.bim` and a TMDL `definition/` folder is now refused
on read (`Error::BothModelFormats`), as Desktop refuses it (P-BOTH); `model.bim` used to win.
