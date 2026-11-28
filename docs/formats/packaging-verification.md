# Packaging facts: verification from public and black-box evidence

The oracle cases ran on Power BI Desktop 2.159.883.0.

This note checks every PBIX / PBIT / PBIP packaging fact the code relies on, from
[pbix-package.md](pbix-package.md), [pbit.md](pbit.md), [pbip.md](pbip.md) and
[pbir.md](pbir.md), against public documentation and black-box tests (the sources below),
and records the evidence. It does not rewrite the code. Where a fact turned out wrong,
the code was fixed with a regression test. Where no source settles it, the code was made tolerant or
the gap is listed.

## Sources

| Id | Source |
|---|---|
| **L-PBIP** | Microsoft Learn, *Power BI Desktop projects (PBIP)*, <https://learn.microsoft.com/power-bi/developer/projects/projects-overview> (page dated 2026-09-23) |
| **L-REPORT** | Microsoft Learn, *Power BI Desktop project report folder*, <https://learn.microsoft.com/power-bi/developer/projects/projects-report> (2026-09-23) |
| **L-MODEL** | Microsoft Learn, *Power BI Desktop project semantic model folder*, <https://learn.microsoft.com/power-bi/developer/projects/projects-dataset> (2026-09-23) |
| **L-PBIT** | Microsoft Learn, *Create and use report templates in Power BI Desktop*, <https://learn.microsoft.com/power-bi/create-reports/desktop-templates> |
| **L-GIT** | Microsoft Learn (Fabric), *Git source code format*, <https://learn.microsoft.com/fabric/cicd/git-integration/source-code-format> |
| **OPC** | ECMA-376 Part 2 / ISO/IEC 29500-2, Open Packaging Conventions: part names, part-name equivalence (ASCII case-insensitive), percent-encoding of part names, the Content Types stream (`Default` / `Override`) |
| **SCHEMAS** | Microsoft's JSON schemas, <https://github.com/microsoft/json-schemas/tree/main/fabric>, vendored under `crates/pbir/schemas/` |
| **OBS** | Structure of files Desktop saved. Two PBIX files and two PBIT files saved by Desktop 2.158.1304.0, held locally and never committed. Only their structure was read, with `cargo xtask pkg-oracle observe`, which prints no content. |
| **ORA-x** | Oracle case `x`. A BIcraft-generated file with one fact changed (`cargo xtask pkg-oracle mutate`) was opened in Desktop 2.159.883.0 by `tools/packaging-oracle/run.ps1`. **accept** means the report page appeared and its card showed the value Desktop's engine computed. **refuse** means the page never appeared: Desktop fell back to an empty report, usually with an error dialog. |
| **G26** | The black-box runs recorded in pbix-package.md §9–11, pbit.md and pbip.md: Desktop opening our files and variants of them, and .NET's public `System.IO.Packaging` API used as an oracle. |

Base files for the ORA cases are made by `bicraft-cli run` from the commands in the oracle steps
below: a model with one table `T`, three rows, a measure `Total` = 742, and a report with one
page `Probe` holding a card bound to `Total`. They are saved as PBIX and PBIT, then converted to
a TMDL / PBIR project with its cache.

## Summary

49 facts. 42 verified. 2 contradicted and fixed: PK-VER-PBIR (`check_package` was stricter than
Desktop) and P-NAME (Desktop drops PBIR objects whose names BIcraft accepted). 3 unverifiable:
PK-CONN, PK-VER-EXT and PK-ENC-MINOR. 2 verified in part, with the rest unverifiable but harmless:
PK-VER-SET and PK-MASHUP. Also fixed along the way: a `check_package` gap (PK-ENC-16), a wrong
zip-layout statement (PK-ZIP), the `definition.pbir` upgrade value (P-PBIR-VER), and the project
reader, which now refuses `model.bim` next to TMDL instead of silently picking `model.bim` (P-BOTH).

## Fact table

Status:
- **verified**: a source above agrees with what the code does.
- **contradicted**: a source above disagrees; the fix is named.
- **unverifiable**: no source above settles it.
- **tolerant**: unverifiable, but the code no longer depends on it.

### Container (PBIX / PBIT)

| Id | Fact (our words) | Code | Evidence | Status |
|---|---|---|---|---|
| PK-OPC | A PBIX/PBIT is an OPC zip. A part that no `[Content_Types].xml` entry covers is invisible to Desktop. Duplicate entries or a schema violation make the file unreadable. | `pbixwriter/src/content_types.rs`, `check.rs` | OPC; G26 (`[Content_Types].xml` variants) | verified |
| PK-CASE | Part names are compared case-insensitively. | `check.rs` (duplicates), `pbixwriter/src/names.rs` | OPC (part-name equivalence) | verified |
| PK-PCT | Part-name characters outside the RFC 3986 path set are percent-encoded (`a b.png` → `a%20b.png`). | `names::encode_part_name`, `pbix/src/report.rs` | OPC; ORA-n01 accept | verified |
| PK-CT | `[Content_Types].xml` as Desktop writes it: UTF-8 with BOM, XML declaration, one line, `<X … />` elements. One `Default` per extension, in first-use order, with an empty type. One `Override` per extension-less part. `application/json` for `Settings` and `Metadata`, `""` for the other parts. | `content_types.rs` | OBS (all four files) | verified |
| PK-ORDER | `Version` is the first entry and `[Content_Types].xml` the second. Desktop puts `DataModel` last but accepts any order. | `pbixwriter/src/lib.rs` `finish` | OBS; ORA-d03 accept; G26 (reverse order) | verified |
| PK-COMP | `DataModel` is stored; every other part is deflated. Desktop also accepts a deflated `DataModel` and an all-stored package. | `pbixwriter/src/lib.rs`, `pbix/src/report.rs` | OBS; ORA-d02 accept; ORA-z01 accept | verified |
| PK-ZIP | Desktop's zip entries: made by 4.5 / MS-DOS, needed 2.0 (1.0 when stored), flags 0, no data descriptor, no Zip64 on small files, and a **28-byte local extra field** (tag `0xA220`, padding) with no central extra. BIcraft writes no extra field; Desktop accepts that. | `opc/src/zipwrite.rs` | OBS; ORA-p00/p01 accept | verified. **Doc corrected**: pbix-package.md §11 said "no extra field". |
| PK-ZIP64 | Desktop reads Zip64 records and data descriptors. | `zipwrite.rs` | G26 (forced Zip64, data descriptors) | verified |
| PK-EXT | `.pbix`, `.pbit`, `.zip` and `.tmp` files open. | none (docs) | G26 | verified |

### `/Version`

| Id | Fact | Code | Evidence | Status |
|---|---|---|---|---|
| PK-VER-REQ | `/Version` is required. | `check.rs` | ORA-v06 refuse | verified |
| PK-VER-ENC | UTF-16LE, no BOM, no newline. | `pbixwriter/src/lib.rs` `text_part`, `check.rs` | OBS; ORA-v05 (UTF-8) refuse; ORA-v08 (CRLF) refuse; G26 (BOM) refuse | verified |
| PK-VER-SET | An exact `1.<minor>` string up to 1.36. 1.36 opens; 1.37, `1.32.0` and ` 1.32` are refused. | `check.rs` `accepted_version`, `MAX_MINOR` | ORA-v01 accept, ORA-v02/v03/v04 refuse | verified at the boundaries. Each minor below 1.28 and `0.1` were not opened one by one: **unverifiable**, and harmless because BIcraft writes 1.28–1.36 only. |
| PK-VER | BIcraft writes 1.28 with a legacy report and 1.32 with PBIR, as Desktop does. | `pbixwriter/src/lib.rs` `version` | OBS (Desktop wrote 1.32 for PBIR); G26 (legacy 1.28 file opens) | verified |
| PK-VER-PBIR | Is `/Version` ≥ 1.32 required when the package holds PBIR? **No**: Desktop opens a PBIR package marked 1.28. | `check.rs` (rule removed) | ORA-v07 accept | **contradicted**. Fix: `check_package` no longer reports it. Test `pbir_with_version_1_28_passes_check_package`. |
| PK-VER-EXT | 1.29 for a live-connected (thin) report. | `pbixwriter/src/lib.rs` `version(true, …)` | none: opening a thin report needs a Power BI service sign-in, which these tests do not use | **unverifiable**. Harmless: a thin PBIX with PBIR is 1.32 anyway (PK-VER-SET). |
| PK-VER-COPILOT | 1.36 with Copilot content, 1.34 with root verified answers only. | `pbixwriter/src/lib.rs` `copilot_parts` | G26 (§2 Copilot check: opens with 1.36 *and* with 1.28) | **tolerant**: Desktop reads the container whatever the version, so raising it is never required. 1.34 for root verified answers was not opened. |

### Text encodings and JSON parts

| Id | Fact | Code | Evidence | Status |
|---|---|---|---|---|
| PK-ENC-16 | `DataModelSchema`, `DiagramLayout`, `Settings` and `Metadata` are UTF-16LE without BOM. In UTF-8, each of `Settings`, `DataModelSchema` and `DiagramLayout` is refused. | `pbixwriter/src/lib.rs`, `check.rs` | OBS; ORA-s08/e01/e03 refuse | verified. **Fix**: UTF-8 bytes of even length decode as wrong UTF-16LE, so `check_package` now also requires `DataModelSchema` and `DiagramLayout` to parse as JSON (test `check_package_agrees_with_desktop`). |
| PK-ENC-LAYOUT | `Report/Layout` is UTF-16LE. | `pbixwriter/src/lib.rs`, `pbix/src/report.rs` | G26 (legacy layout packages written that way open) | verified |
| PK-ENC-PBIR | PBIR parts inside a package are UTF-8 without BOM. A BOM is refused, and so is UTF-16LE. Desktop writes them compact, but indented JSON opens too. | `pbixwriter`, `check.rs`, `pbix/src/report.rs` | OBS; ORA-r01/r02 refuse; ORA-r04 accept | verified |
| PK-DMS | `DataModelSchema` uses CRLF line ends, with no trailing newline. | `pbixwriter/src/lib.rs` `data_model_schema` | OBS | verified |
| PK-ENC-MINOR | `Report/MobileState`, `Report/LinguisticSchema`, `DiagramState` and `UnappliedChanges` are UTF-16LE. | none (BIcraft never writes them, and `check_package` no longer checks their encoding) | none: no Desktop file at hand has them | **unverifiable**. BIcraft keeps such parts byte-identical. |
| PK-COPILOT-ENC | Copilot / verified-answers text is UTF-16LE in a package and UTF-8 in a project. | `opc::is_container_text`, `pbip::write_copilot_files` | G26 (§2: UTF-8 in a package refused) | verified |
| PK-CONN | `Connections` as BIcraft writes it for a thin report: UTF-8 JSON, `Version` 4, `pbiServiceXmlaStyleLive`. | `pbixwriter/src/lib.rs` `connections_json` (`check_package` no longer checks the part) | none: no Desktop file at hand has the part, and a thin report needs a service sign-in | **unverifiable**. Listed for the lead (below). |
| PK-SETTINGS | `Settings` needs a top-level `Version` of at most 4. | `check.rs` `inner_version` | OBS (4); ORA-s01 (5) refuse; ORA-s09 (missing) refuse | verified |
| PK-QSV | `Settings.QueriesSettings.Version` is required, must be at least 1.0.3207.0 and must not be from the future. Two components are enough. BIcraft writes 2.130.0.0; Desktop 2.158 wrote 2.158.928.0. | `check.rs` `mashup_version`, `bicraft_versions::MASHUP_PACKAGE_VERSION` | ORA-s02 (missing) refuse; ORA-s03 (1.0.3206.0) refuse; ORA-s04 (1.0.3207.0) accept; ORA-s05 (99.0.0.0) refuse; ORA-s06 (`2.130`) accept; OBS | verified, including the exact lower boundary |
| PK-METADATA | `Metadata` needs a `Version` of at most 5. | `check.rs` | OBS (5); ORA-m01 (6) refuse | verified |
| PK-OPTIONAL | `Settings`, `Metadata` and `DiagramLayout` may be absent. | none (BIcraft always writes them) | ORA-s07/m02/e02 accept | verified |

### Loader rules

| Id | Fact | Code | Evidence | Status |
|---|---|---|---|---|
| PK-DM-XOR | `DataModel` and `DataModelSchema` together are refused. A PBIT has the schema; a PBIX has the data. | `check.rs`, `pbix/src/lib.rs` `inventory` | ORA-d01 refuse; L-PBIT ("templates don't contain any data") | verified |
| PK-LAYOUT-XOR | `Report/Layout` next to `Report/definition/` is refused. | `check.rs` | ORA-l01 refuse | verified |
| PK-TXT | A `Version.txt` part is refused; an unrelated `.txt` part is not. | `check.rs` (`Version.txt` only) | ORA-k01 refuse; ORA-k02 accept | verified (one sibling tried) |
| PK-CUSTOM | `docProps/custom.xml` over 2 MiB is refused; a small one opens. | `check.rs` | ORA-c02 refuse; ORA-c01 accept | verified |
| PK-MASHUP | A `DataMashup` that does not start with Int32 0 is refused. `DataMashup` plus `UnappliedChanges` is refused. | `check.rs` | [MS-QDEFF] (public Power Query document format: the leading version field is 0); ORA-x01 and ORA-x02 refuse, but so does the control ORA-x03 (an all-zero `DataMashup` alone), so the oracle does not isolate either rule; ORA-x04 (`UnappliedChanges` alone) partial | Int32 0: verified (public spec). Exclusivity: **unverifiable**, and harmless: BIcraft never writes either part and keeps them byte-identical. |
| PK-PBIR-PAGE | A PBIR report needs `version.json`, `report.json` and at least one page. | `check.rs`, `pbir` validator | L-REPORT (required files) | verified (ORA-r03 is inconclusive: with no page the oracle cannot see a page either way) |
| PK-SECBIND | `SecurityBindings` is optional, and a stale one makes Desktop refuse the file, so report saves drop it. | `pbix/src/report.rs` | G26 (§9 finding) | verified |
| PK-CV | A custom visual's resource item path is relative to `CustomVisuals/<guid>/resources/`. | `pbix/src/report.rs`, `engine/src/custom_visuals.rs`, pbir validator | G26 (§11 "Custom visuals", Desktop runs with a visual of our own) | verified |

### PBIP projects

| Id | Fact | Code | Evidence | Status |
|---|---|---|---|---|
| P-LAYOUT | `<Name>.pbip`, `<Name>.Report/`, `<Name>.SemanticModel/`, and a `.gitignore` holding `**/.pbi/localSettings.json` and `**/.pbi/cache.abf`, created only when absent. | `pbip/src/create.rs` | L-PBIP | verified |
| P-UTF8 | Project files are UTF-8 without BOM. A BOM in a PBIR file is refused. | `pbip`, `pbir` writers | L-PBIP ("UTF-8 without BOM"); ORA-j09 (BOM in `page.json`) refuse; ORA-j10 (BOM in a `.tmdl` file) refuse | verified |
| P-EOL | Desktop writes CRLF; BIcraft writes LF; both open. | `pbip`, `pbir`, `tmdl` writers | L-PBIP (Desktop uses CRLF); ORA-j00 (LF) accept; ORA-j16 (CRLF) accept | verified |
| P-PBIR-VER | `definition.pbir` `version` 1.0 means a legacy `report.json` only; from 4.0, PBIR `definition/` is read. | `pbip/src/report.rs` (upgrade), `bicraft_versions::PBIR_VERSION` | L-REPORT (version table); ORA-j01 (3.0) accept; ORA-j02 (3.0, no `.platform`) accept; ORA-j03 (4.0, no `.platform`) accept; ORA-j04 (1.0 with `definition/`) refuse | verified. **Changed**: the legacy-to-PBIR upgrade wrote 3.0 when there was no `.platform`. It now always writes 4.0, the documented value (test `definition_upgrade_is_always_4_0`). 3.0 opens too (j01). |
| P-PBISM-VER | `definition.pbism` `version` 1.0 means TMSL only; from 4.0, TMDL is read. BIcraft writes 4.2, as Desktop does. | `bicraft_versions::PBISM_VERSION` | L-MODEL (version table); ORA-j05 (3.0 with TMDL) accept; ORA-j06 (1.0 with TMDL) refuse | verified (4.2 ≥ 4.0) |
| P-BOTH | `model.bim` together with TMDL, and `report.json` together with PBIR, are errors. BIcraft never writes both, and the legacy-to-PBIR save removes `report.json`. The project reader used to take `model.bim` silently when both model forms were present; it now refuses the project, as Desktop does (`Error::BothModelFormats`, test `model_bim_next_to_tmdl_is_refused`). | `pbip/src/lib.rs`, `pbip/src/report.rs` | L-REPORT / L-MODEL ("replaces"); ORA-j07 (`model.bim` + TMDL) refuse; ORA-j08 (`report.json` + PBIR) refuse | verified |
| P-BYPATH | `byPath` is relative, with `/` separators; absolute paths are unsupported. | `pbip/src/create.rs` | L-REPORT; ORA-j14 (absolute) refuse; ORA-j15 (byPath and byConnection together) refuse | verified |
| P-SCHEMA | Each file declares its `$schema`. Desktop rejects an unknown major and accepts a newer minor. | `bicraft_versions`, pbir validator | SCHEMAS; L-REPORT; ORA-j11 (`definition.pbir` major 9) refuse; ORA-j12 (minor 2.9.0) accept; ORA-j13 (`page.json` major 9) refuse | verified |
| P-V1 | `.pbip` and `definition.pbism` without `$schema` (the form Desktop writes) open. | readers | G26 (pbip.md "Desktop answers"); ORA-j17 accept | verified |
| P-PLATFORM | `.platform` (`metadata.type`, `displayName`, `config.version` 2.0, `logicalId`) is optional for Desktop. It may not coexist with the V1 pair `item.metadata.json` / `item.config.json`. | `pbip/src/create.rs` | L-GIT; ORA-j03 (no `.platform`) accept; ORA-j18 (`.platform` plus the V1 pair) refuse | verified |
| P-DATASET | The legacy semantic-model folder suffix is `.Dataset`. | `pbip/src/lib.rs` (`locate`) | L-REPORT example (`../Sales.Dataset`); ORA-j19 accept | verified |
| P-COPILOT | `Copilot/` is read only with `definition.pbism` 6.0. | `bicraft_versions::PBISM_COPILOT_VERSION` | G26 (§2 runs); ORA-j20 (4.2, broken `Copilot/settings.json`) accept | verified |
| P-CACHE | `.pbi/cache.abf` has the PBIX `DataModel` format, and a project opens without it (no data). | `pbip/src/extras.rs`, engine conversions | L-MODEL (cache.abf, open without it); G26 (pbip.md "Data cache", §10 conversions) | verified |
| P-NAME | PBIR page, visual and bookmark names, and their folders, are letters, digits, underscores and hyphens only; Desktop ignores other names. BIcraft accepted any file-safe name, such as `pg.1` or `pg 1`. | `pbir/src/paths.rs` `check_object_name`, `validate.rs` | L-REPORT ("PBIR naming convention"); ORA-j21 (`pg-1_A`) accept; ORA-j22 (`pg.1`) and ORA-j23 (`pg 1`) refuse: the page is dropped | **contradicted**. Fix: the writer refuses such names and the validator reports them (tests `object_names_follow_the_pbir_convention`, `object_names_must_be_word_characters_or_hyphens`). |

## Results of the Desktop oracle

All 66 cases ran on 2026-10-28 (raw results kept locally by the maintainers). **partial** means the page appeared but
the value did not: the report was read but the model did not answer. Inconclusive by design: r03
(no page left to see) and x01-x04 (see PK-MASHUP).

| Case | Fact changed | Desktop 2.159.883.0 |
|---|---|---|
| p00 | control: BIcraft PBIT re-zipped | accept |
| p01 | control: BIcraft PBIX re-zipped | accept |
| j00 | control: BIcraft project | accept |
| v01 | /Version 1.36 is accepted | accept |
| v02 | /Version 1.37 (above the maximum) is refused | refuse |
| v03 | /Version 1.32.0 (three components) is refused | refuse |
| v04 | /Version with a leading space is refused | refuse |
| v05 | /Version in UTF-8 instead of UTF-16LE | refuse |
| v06 | /Version missing (the only required part) | refuse |
| v07 | /Version 1.28 with a PBIR report (below 1.32) | accept |
| v08 | /Version 1.32 followed by CRLF | refuse |
| k01 | a `Version.txt` sibling of a known part | refuse |
| k02 | a `foo.txt` part that is not a known part's sibling | accept |
| d01 | PBIX with both DataModel and DataModelSchema | refuse |
| d02 | PBIX with DataModel deflated instead of stored | accept |
| d03 | PBIX with DataModel as the first entry | accept |
| l01 | Report/Layout next to Report/definition/ | refuse |
| s01 | Settings Version 5 (above 4) | refuse |
| s02 | Settings without QueriesSettings.Version | refuse |
| s03 | QueriesSettings.Version 1.0.3206.0 (just below the oldest) | refuse |
| s04 | QueriesSettings.Version 1.0.3207.0 (the oldest supported) | accept |
| s05 | QueriesSettings.Version 99.0.0.0 (future) | refuse |
| s06 | QueriesSettings.Version with two components | accept |
| s07 | Settings part missing | accept |
| s08 | Settings in UTF-8 instead of UTF-16LE | refuse |
| s09 | Settings without a top-level Version | refuse |
| m01 | Metadata Version 6 (above 5) | refuse |
| m02 | Metadata part missing | accept |
| e01 | DataModelSchema in UTF-8 instead of UTF-16LE | refuse |
| e02 | DiagramLayout part missing | accept |
| e03 | DiagramLayout in UTF-8 instead of UTF-16LE | refuse |
| r01 | PBIR report.json with a UTF-8 BOM | refuse |
| r02 | PBIR version.json in UTF-16LE instead of UTF-8 | refuse |
| r03 | PBIR with no page | refuse |
| r04 | PBIR documents indented (multi-line) instead of compact | accept |
| c01 | docProps/custom.xml of 1 KiB (control) | accept |
| c02 | docProps/custom.xml over 2 MiB | refuse |
| x01 | DataMashup that does not start with Int32 0 | refuse |
| x02 | DataMashup and UnappliedChanges both present | refuse |
| n01 | an extra part with a percent-encoded name (Report/StaticResources/RegisteredResources/a%20b.png) | accept |
| z01 | every part stored (no deflate) | accept |
| j01 | definition.pbir version 3.0 with a PBIR definition/ folder | accept |
| j02 | definition.pbir version 3.0 and no .platform | accept |
| j03 | definition.pbir version 4.0 and no .platform | accept |
| j04 | definition.pbir version 1.0 with a PBIR definition/ folder | refuse |
| j05 | definition.pbism version 3.0 with a TMDL definition/ folder | accept |
| j06 | definition.pbism version 1.0 with a TMDL definition/ folder | refuse |
| j07 | model.bim next to a TMDL definition/ folder | refuse |
| j08 | report.json next to a PBIR definition/ folder | refuse |
| j09 | a PBIR page.json with a UTF-8 BOM | refuse |
| j10 | a TMDL file with a UTF-8 BOM | refuse |
| j11 | definition.pbir $schema with an unknown major (9.0.0) | refuse |
| j12 | definition.pbir $schema with a newer minor (2.9.0) | accept |
| j13 | page.json $schema with an unknown major (9.0.0) | refuse |
| j14 | definition.pbir byPath with an absolute path | refuse |
| j15 | definition.pbir with both byPath and byConnection | refuse |
| j16 | files with CRLF line endings instead of LF | accept |
| j17 | .pbip and definition.pbism without $schema (V1 form) | accept |
| j18 | report .platform plus item.config.json / item.metadata.json | refuse |
| j19 | semantic model folder with the legacy `.Dataset` suffix | accept |
| j20 | definition.pbism 4.2 with a broken Copilot/settings.json (folder ignored below 6.0) | accept |
| j21 | PBIR page name of word characters and a hyphen | accept |
| j22 | PBIR page name with a dot (not a word character or hyphen) | refuse |
| j23 | PBIR page name with a space | refuse |
| x03 | control for x01/x02: DataMashup of version 0 and four empty blocks, alone | refuse |
| x04 | control for x02: UnappliedChanges alone | partial |

## Oracle steps (manual; Windows; Desktop lock)

```powershell
# base files (our own): see "Sources"; $w is any work folder outside the repository
$w = "$env:TEMP\oracle-reports\pkg"
bicraft-cli run --file $w\base\cmds.json    # probe.pbix, probe.pbit
bicraft-cli convert $w\base\probe.pbix $w\base\proj\probe.pbip
cargo xtask pkg-oracle mutate $w\base\probe.pbit $w\base\probe.pbix $w\base\proj $w\m
# takes $env:BICRAFT_LOCKS_DIR\desktop per case and waits 90 s between cases
tools\packaging-oracle\run.ps1 -Cases $w\m\cases.json -Out $w\results.json [-Only v01,v02]
cargo xtask pkg-oracle observe <file.pbix | file.pbit | project folder>   # structure only
```

`cmds.json` builds the base files with these commands: `model.new` `{name: "Probe"}`;
`table.import` `{name: "T", csv: "K,V\na,100\nb,200\nc,442\n", types: {V: int64}}`;
`measure.add` `{table: "T", name: "Total", expression: "SUM(T[V])", format_string: "0"}`;
`report.new`, `page.add` `{display_name: "Probe"}`, `page.remove` `{page: "Page 1"}`,
`visual.add` (a card bound to `[Total]`); then `file.save_as` to `.pbix` and to `.pbit`.

The automated counterpart, which needs no Desktop, is in
`crates/pbixwriter/tests/verified_facts.rs`. It checks the structure of generated packages
against the verified facts, and checks that `check_package` reports every Desktop refusal above
and nothing for the accepted cases.

## Left for the lead

- **Unverifiable** (settling them would need a Power BI service sign-in, or a Desktop file
  that has these parts):
  - PK-CONN: the `Connections` part's encoding and versions, and the thin PBIX connection
    string;
  - PK-VER-EXT: `/Version` 1.29 for a thin report;
  - PK-ENC-MINOR: the encodings of `MobileState`, `LinguisticSchema`, `DiagramState` and
    `UnappliedChanges`;
  - the acceptance of each `/Version` minor below 1.28.

  BIcraft writes `Connections` only for a thin PBIX, which is not opened in Desktop yet
  (pbit.md, open question 1). The other parts are only validated, never written.
- **Tolerant**: PK-VER-COPILOT (the `/Version` raise is not needed).
- **Not done**: a fresh Desktop "Save as" (PBIX / PBIT / PBIP) driven through UI Automation. The
  structure of what Desktop writes comes from the Desktop-saved local files (OBS) and from the
  project saves of G26 (pbip.md "Desktop answers"). `tools/packaging-oracle/cdp.mjs` and
  `uia-tree.ps1` are the starting point for that step.
