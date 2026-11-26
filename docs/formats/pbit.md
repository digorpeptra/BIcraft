# PBIT and thin PBIX: what BIcraft writes (M3)

Implementation: `crates/pbixwriter` (`write_pbit`, `write_thin_pbix`, their `*_documents`
variants, `check_package`). The loader rules, encodings and part table come from
[pbix-package.md](pbix-package.md) (Power BI Desktop 2.158.1304.0). This note
records the choices made on top of them, and what was confirmed by comparing our output with
Desktop-saved files.

Sources: black-box tests against Power BI Desktop 2.158 (structural comparison with Desktop-saved
files, and our files opened in Desktop) and the packaging facts of pbix-package.md; detailed
provenance is kept privately by the maintainers.

Used here:
- `pbix-package.md` (part table, encodings, loader rules, Desktop 2.158.1304.0), including the
  shapes of the `Connections`, `Settings` and `Metadata` containers recorded below.
- A structural comparison (part names by category, content types, encodings, compression,
  `Version`, part order; never content) with a PBIX and a PBIT saved by Desktop 2.158 from the same
  report. These files are local test data and are not in the repository.

## Parts written

### PBIT (`write_pbit`)

| # | Part | Encoding | Content type | Notes |
|---|---|---|---|---|
| 1 | `Version` | UTF-16LE, no BOM | `""` | `1.32` (PBIR report); see "Version" below |
| 2 | `[Content_Types].xml` | UTF-8 **with** BOM | n/a | see below |
| 3 | `DataModelSchema` | UTF-16LE, no BOM | `""` | `bicraft_tmsl::write`, converted to CRLF line endings with no trailing newline, which is how Desktop stores it |
| 4 | `DiagramLayout` | UTF-16LE, no BOM | `""` | one empty diagram (`version` `1.1.0`, no nodes); Desktop lays the tables out |
| 5 | `Settings` | UTF-16LE, no BOM | `application/json` | `{"Version":4,"ReportSettings":{},"QueriesSettings":{"TypeDetectionEnabled":true,"RelationshipImportEnabled":true,"Version":"2.130.0.0"}}`. The `QueriesSettings.Version` is **required** (see below) |
| 6 | `Metadata` | UTF-16LE, no BOM | `application/json` | `{"Version":5,"AutoCreatedRelationships":[]}` |
| 7 | `Report/StaticResources/…` | bytes verbatim | `""` (by extension) | from the report |
| 8 | `Report/definition/…` | UTF-8, no BOM | `""` (by extension) | PBIR documents from `bicraft_pbir::write_documents`, order: `version.json`, `report.json`, `reportExtensions.json`, `pages/pages.json`, pages (each `page.json` with its visuals, in the order given), bookmarks |

This is exactly Desktop's PBIT part order, minus `SecurityBindings` (last in Desktop's file).

All parts are Deflate-compressed. Only `DataModel` would be stored, and a PBIT has none.

Not written: `DataModel` (PBIT has none), `Connections` (a template has no external connection),
`SecurityBindings` (below), `DataMashup`/`UnappliedChanges` (queries live in the model's M
partitions and expressions for a V3 model), `DiagramState`, `Report/Layout` (PBIR is used instead),
`docProps/custom.xml`.

A model with an empty name gets a deterministic GUID-shaped name (FNV-1a of its TMSL), because
Desktop always names the database in `DataModelSchema` (Desktop itself uses a GUID).

### Conversions: `write_pbit_with` and `write_pbix` (M3.3)

Both take the report as `ReportParts` and carried-over parts as `Extras`:

- `ReportParts::Pbir(docs)`: as above (`definition/` plus `StaticResources/` and `CustomVisuals/`
  files); `Version` 1.32.
- `ReportParts::Legacy { layout, resources }`: the layout JSON becomes `Report/Layout`
  (UTF-16LE, no BOM, deflated, after the resources), no `Report/definition/`; `Version` stays the
  1.28 baseline. Only `StaticResources/…` and `CustomVisuals/…` resources are accepted.
- `Extras::diagram_layout`: the source's model diagram (PBIX `DiagramLayout`, PBIP
  `diagramLayout.json`) instead of the empty one; it must be a JSON object.

`write_pbix(data_model, report, extras)` writes a PBIX **with data**: the `DataModel` part is the
given bytes (an XPress9-framed ABF, e.g. a PBIP `.pbi/cache.abf`, which has the same format, see
`pbip.md` "Data cache"), **stored** (not deflated) as Desktop writes it, followed by
`DiagramLayout`, `Settings`, `Metadata` and the report, in the same encodings as a PBIT. The
backup itself is not inspected by the writer; the engine reads the written package back before
it keeps it. Every package still goes through `check_package`.

`CustomVisuals/…` files (a custom visual's package) are now accepted next to a PBIR report too,
as `Report/CustomVisuals/…`.

### Thin PBIX (`write_thin_pbix`)

`Version` (`1.32`), `[Content_Types].xml`, `Connections`, `Settings`, `Metadata`,
`Report/StaticResources/…`, `Report/definition/…`. No `DataModel`, `DataModelSchema` or
`DiagramLayout`.

`Version`: an external dataset needs 1.29 and PBIR needs 1.32, so the result is 1.32.

## `[Content_Types].xml`

Desktop's files show the layout .NET's public `System.IO.Packaging` writes, and BIcraft reproduces
it byte for byte (the
rebuilt template's file is identical to Desktop's except for the `SecurityBindings` override):

- UTF-8 with BOM; `<?xml version="1.0" encoding="utf-8"?>` then a single-line `<Types>` element in
  the OPC namespace, with each element written as `<X … />` (a space before `/>`).
- `Default` elements come first, one per file extension, in the order each extension first
  appears among the parts. Each takes the content type of that first part (`json` → `""`,
  `png` → `""`).
- `Override` elements follow, in part order. One is written for every extension-less part, and for
  a part whose content type differs from its extension's `Default`.
- `[Content_Types].xml` is the second zip entry, after `Version`. There is no `_rels/` because
  there are no relationships.

`check_package` reads the file back and requires a content type for every part: a part that no
entry covers is invisible to Desktop (PK-OPC).

## Part names

Part names are zip entry names without the leading `/`. Like System.IO.Packaging, BIcraft
percent-encodes any byte outside the RFC 3986 path characters, so `My Logo.png` is stored as
`My%20Logo.png`. Report document paths must be relative and must not contain backslashes, empty
segments, `.`/`..` segments, segments ending in `.`, or control characters. Two paths that differ
only in case are refused, because OPC names ignore case. Only `definition/` and
`StaticResources/` documents are accepted, and at least one `definition/` document is required.

Readers handing these names to `bicraft_pbir::read_documents` should percent-decode them. The
Desktop files compared so far contain no escaped names.

## Version choice

`Version` follows what Desktop writes (pbix-package.md §4): 1.28 with a legacy report, 1.32 with
PBIR `Report/definition/`. Desktop 2.158 wrote `1.32` for both the PBIX and the PBIT with a PBIR
report. A thin PBIX gets 1.29 (Unknown: not verified, PK-VER-EXT).

## `Connections` (thin PBIX)

UTF-8 without BOM, content type `""`, compact JSON (no whitespace):

```json
{"Version":4,"Connections":[{"Name":"EntityDataSource","ConnectionString":"Data Source=powerbi://api.powerbi.com/v1.0/myorg/<Workspace>;Initial Catalog=<Model>","ConnectionType":"pbiServiceXmlaStyleLive"}]}
```

- Members: `Version`, then `Connections`, each with `Name`, `ConnectionString` and
  `ConnectionType`. This is what BIcraft writes; no Desktop-saved `Connections` part was at hand
  to compare (packaging-verification.md PK-CONN), so the member set and `Version` 4 are
  Unknown (not verified). A package with this part is read by Desktop 2.159.1231.0 (open
  question 1): with `analysisServicesDatabaseLive` and an unreachable server Desktop shows its
  "cannot connect" dialog; with `pbiServiceXmlaStyleLive` and a `powerbi://` source, signed
  out, it fails (G12).
- The connection-string values follow OLE DB quoting. A value containing `;`, a quote character,
  or leading or trailing whitespace is wrapped in double quotes, with inner double quotes doubled.
  The workspace connection must start with `powerbi://`, and the model name must not be empty.

## `Settings`, `Metadata`, `DiagramLayout`

### `QueriesSettings.Version` is required (Desktop check, 2026-10-13)

The first PBITs BIcraft wrote had no `QueriesSettings.Version`. Desktop 2.158 refused them with
"cannot open the model: this file is damaged or was created by an unrecognized version of Power BI
Desktop" and loaded no tables.

The cause was found by bisecting in Desktop. Each test package was a Desktop-saved PBIT with one
part replaced by BIcraft's (all without `SecurityBindings`):

| Part replaced | Result |
|---|---|
| `DataModelSchema` | opens |
| `DiagramLayout` | opens |
| `Settings` | rejected |

The rule Desktop applies to `QueriesSettings.Version`:
- it is rejected when missing (ORA-s02) or below `1.0.3207.0` (ORA-s03 refuses `1.0.3206.0`,
  ORA-s04 accepts `1.0.3207.0`); whether any other value that is not a version is refused was
  not tested;
- it is also rejected as a "future" version when newer than the running Desktop (99.0.0.0 is
  refused, ORA-s05; how "newer" is compared is Unknown);
- Desktop-saved files carry a value close to Desktop's own release (2.158.928.0 from Desktop
  2.158, PK-QSV).

BIcraft writes `2.130.0.0` (`MASHUP_PACKAGE_VERSION`). That is older than any Desktop that reads
PBIR inside a PBIX (Version 1.32), so it is never a future version, and it is above the lowest
accepted value. The exact choice is an assumption; any engine version between `1.0.3207.0` and
that of the oldest targeted Desktop should work. `check_package` now flags a missing, unparsable
or too-old value.

With the fix, Desktop 2.158 loads every table, measure and relationship of both test files: the
Demo PBIT (4/3/2) and the real template rebuilt through the writer (8/25/4). Both have no
`SecurityBindings`, an empty `DiagramLayout`, and zip headers written by the `zip` crate.

### Shapes

- `Settings`: `Version` must be at most 4. The other members are
  `ReportSettings` (flags such as `IsRelationshipAutodetectionEnabled` and `ShowHiddenFields`) and
  `QueriesSettings` (`TypeDetectionEnabled`, `RelationshipImportEnabled`,
  `RelationshipRefreshEnabled`, `RunBackgroundAnalysis`, and `Version`, which is required: see
  above). Desktop-saved files omit false and null members.
  BIcraft writes the new-file query defaults and `Version` `2.130.0.0`.
- `Metadata`: `Version` must be at most 5 (PK-METADATA). BIcraft writes only `Version` and an
  empty `AutoCreatedRelationships`; Desktop opens the file.
- `DiagramLayout` has the same top-level shape as Desktop's: `version`, `diagrams[]` with
  `ordinal`, `scrollPosition`, `nodes`, `name`, `zoomValue` and four boolean flags, then
  `selectedDiagram` and `defaultDiagram`. Desktop localizes the diagram name; BIcraft uses
  `All tables`.

## `SecurityBindings`: omitted (confirmed harmless)

Desktop writes `SecurityBindings` even in a template (it was present in the Desktop-saved PBIT).
BIcraft omits it, for two reasons:
- The part is optional. Only `Version` is required (pbix-package.md §2).
- Its content is Unknown, and a stale one makes Desktop refuse the file (below).

Desktop check (2.158): a Desktop-saved PBIT with `SecurityBindings` and its content-type
override removed opens normally, and so do BIcraft's PBITs.

The reverse also holds: a package whose report parts were changed while `SecurityBindings` was
kept is refused by Desktop ("damaged or created by an unrecognized version"). Saving report edits
into an existing PBIX/PBIT therefore removes the part (`pbix-package.md` §9).

## Loader-rule validation (`check_package`)

`check_package(&Package) -> Vec<String>` runs on every package before the writer returns it, and
works on any package. Its messages name parts and rules, never content. It checks:

- OPC structure:
  - part names are unique, ignoring case;
  - part names are well formed;
  - `[Content_Types].xml` is present, parses, and gives every part a content type.
- `Version`:
  - it exists;
  - it is UTF-16LE without BOM;
  - it is exactly `1.<minor>` up to `1.36` (no minimum for PBIR: Desktop opens 1.28, ORA-v07).
- Forbidden parts:
  - no `Version.txt` (ORA-k01; other siblings were not tried);
  - not both `DataModel` and `DataModelSchema`;
  - not both `Report/Layout` and `Report/definition/…`;
  - not both `DataMashup` and `UnappliedChanges`.
- Part content:
  - `docProps/custom.xml` is at most 2 MiB;
  - `DataMashup` starts with Int32 0;
  - `Version`, `DataModelSchema`, `DiagramLayout`, `Settings`, `Metadata` and `Report/Layout` are
    UTF-16LE without BOM, and `DataModelSchema` and `DiagramLayout` are JSON;
  - `Settings` (inner `Version` at most 4) and `Metadata` (at most 5) are UTF-16LE JSON;
  - `Settings` `QueriesSettings.Version` is at least 1.0.3207.0 (PK-QSV).
- Not checked, because no black-box run settled them: the content of `Connections` and
  `UnappliedChanges`, and the encoding of `DiagramState`, `Report/MobileState` and
  `Report/LinguisticSchema` (PK-CONN, PK-ENC-MINOR).
- PBIR documents:
  - UTF-8 without BOM, and valid JSON;
  - `version.json` and `report.json` are present;
  - there is at least one `pages/<page>/page.json`.

Both Desktop-saved files pass `check_package` with no violation.

## Comparison with Desktop-saved files

The env-gated tests in `crates/pbixwriter/tests/real_data.rs` (`BICRAFT_TESTDATA`) feed the
model (from `DataModelSchema`) and the PBIR documents of a Desktop-saved PBIT back through
`write_pbit_documents`. The rebuilt package has the same structure as Desktop's file: same part
categories and order, content types, encodings, compression and `Version`, and an identical
`[Content_Types].xml`. The only difference is the deliberately omitted `SecurityBindings`. The
model also round-trips. The parts shared with the Desktop-saved PBIX match in the same way.

Desktop also writes `Report/StaticResources/SharedResources/BaseThemes/<theme>.json`, Microsoft's
base theme. BIcraft never ships that file (asset policy). It only writes the resources the report
model carries.

Zip level: `bicraft-opc` writes entries like those of Desktop-saved files (version made by 45 /
MS-DOS, needed 20 or 10, flags 0, no data descriptor; Zip64 only where needed,
`pbix-package.md` §11). The differences:

| Field | Desktop-saved files | BIcraft |
|---|---|---|
| entry timestamps | save time | fixed 1980-01-01 00:00 (deterministic output) |
| local extra field | a 28-byte padding field (tag `0xA220`) | none; Desktop opens such files (PK-ZIP) |

## Open questions

Closed by Desktop checks: a missing `SecurityBindings` is fine, and so is an empty `DiagramLayout`.
Zip entries with Unix host / version 20 / 1980 timestamps are also accepted (a Desktop-saved PBIT
re-zipped that way opens).

1. **Thin PBIX connection string.** Desktop's own `definition.pbir` `byConnection` strings also
   carry `integrated security=ClaimsToken` and `semanticmodelid=…`.
   Whether the minimal `Data Source=…;Initial Catalog=…` form is enough for Desktop to connect
   needs a check against a published model. Seen on Power BI Desktop 2.159.1231.0 (2026-11-23),
   not signed in: with `pbiServiceXmlaStyleLive` and a `powerbi://` data source, Desktop shows a
   null-reference error and opens a blank untitled report; with `analysisServicesDatabaseLive`
   and an unreachable server, it shows its "cannot connect" dialog (Retry, Edit, Cancel), so the
   rest of the package is read. The open signed in, against a published model, is not pursued,
   by the maintainers' decision (2026-11-24, docs/gaps.md G12): BIcraft reads a thin report's
   report and reports its live connection (`bicraft_pbix::live_connection`: the type, `Data
   Source` and `Initial Catalog` only).
2. **Escaped part names.** No Desktop file seen so far has a percent-escaped name. Confirm the
   escaping against a report whose image file name contains a space.
3. **`QueriesSettings.Version` value.** Check `2.130.0.0` against an older Desktop, and decide whether to
   follow the targeted Desktop release instead. A thin PBIX has been opened only signed out (1.).

## Public and black-box evidence

The parts, encodings, content types, `Version` choice and `check_package` rules of this note
rest on public and black-box sources in
[packaging-verification.md](packaging-verification.md) (PK-* facts). The `QueriesSettings.Version`
bounds are black-box: 1.0.3206.0 is refused, 1.0.3207.0 and `2.130` are accepted, and a
future version is refused (PK-QSV). Still unverifiable: the thin PBIX `Connections` part
(PK-CONN, PK-VER-EXT).
