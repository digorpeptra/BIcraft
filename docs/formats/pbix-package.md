# PBIX / PBIT / PBIP packaging (Power BI Desktop 2.158.1304.0)

| | |
|---|---|
| Target | Power BI Desktop, Microsoft Store edition |
| Version | 2.158.1304.0 (oracle cases of [packaging-verification.md](packaging-verification.md): 2.159.883.0) |
| Date | 2026-10-13, black-box checks through 2026-10-28 |

Sources: black-box tests against Power BI Desktop (files Desktop writes, and our own files opened
in Desktop: §2 "Copilot containers", §8–11 and the oracle cases of
[packaging-verification.md](packaging-verification.md)), the Open Packaging Conventions (ECMA-376
Part 2), Microsoft's public JSON schemas
(<https://github.com/microsoft/json-schemas/tree/main/fabric>), its Learn pages for PBIP and
templates, [MS-QDEFF] for `/DataMashup`, and .NET's public `System.IO.Packaging` API used as an
oracle (§11); detailed provenance is kept privately by the maintainers.

Labels used below:
- **Observed**: seen in files Desktop saved (OBS in packaging-verification.md) or in Desktop's
  answers to our own files (the ORA-* cases, G26 runs in §2 and §8–11).
- **Public**: stated by a public specification, schema or Learn page.
- **Inferred**: a conclusion drawn from the above.
- **Unknown**: not settled; BIcraft does not rely on it, or keeps such parts byte-identical.

Fact ids (PK-*, P-*) and oracle cases (ORA-x) are those of
[packaging-verification.md](packaging-verification.md).

---

## 1. Container: OPC package, not a plain zip

| Finding | Label |
|---|---|
| PBIX and PBIT are Open Packaging Conventions (OPC) packages: a zip with a `[Content_Types].xml` stream (PK-OPC, PK-CT). | Observed, Public (OPC) |
| Desktop's files hold no `_rels/` part (OBS). | Observed |
| A malformed `[Content_Types].xml` (duplicate entries, schema violations) or a missing content type makes Desktop refuse the file or lose the part (§11, `[Content_Types].xml`). | Observed |
| Part names are relative URIs with a leading `/`, compared without case (PK-CASE). Folder-like content is stored as one part per file, `<folder part>/<relative path>` (OBS). | Public (OPC), Observed |
| Zip layout, `[Content_Types].xml` variants, Zip64, data descriptors and entry order, checked on Desktop 2.158: §11. | Observed |

BIcraft treats a folder part (for example `/Report/definition`) as every part whose name starts
with `<folder part>/`; that is our reading of the files Desktop writes.

### Compression per part

| Part | Desktop writes | Label |
|---|---|---|
| `/DataModel` | stored (no compression) | Observed (PK-COMP) |
| every other part | Deflate | Observed (PK-COMP) |

Desktop also opens a deflated `/DataModel` (ORA-d02) and a package where every part is stored
(ORA-z01). BIcraft writes `/DataModel` stored, as Desktop does.

---

## 2. Part table (PBIX and PBIT)

Parts BIcraft knows. Only `/Version` is required (PK-VER-REQ); `/Settings`, `/Metadata` and
`/DiagramLayout` may be absent (PK-OPTIONAL).

| Part name / path | Kind | Evidence | Notes |
|---|---|---|---|
| `/Version` | single | OBS, ORA-v* | required |
| `/DataModel` | single | OBS | PBIX only; stored |
| `/DataModelSchema` | single | OBS | PBIT only |
| `/DiagramLayout` | single | OBS | |
| `/Settings` | single | OBS, ORA-s* | |
| `/Metadata` | single | OBS, ORA-m* | |
| `/SecurityBindings` | single | OBS, §9 | optional |
| `/Report/definition` | folder | OBS | PBIR inside PBIX |
| `/Report/StaticResources` | folder | OBS | |
| `/Report/Layout` | single | G26 (§9, §10: legacy packages open) | legacy report JSON |
| `/Report/CustomVisuals` | folder | G26 (§11 "Custom visuals") | one folder per custom visual |
| `/Copilot`, `/VerifiedAnswers` | folder | G26 (Copilot check below) | |
| `/docProps/custom.xml` | single | ORA-c01, ORA-c02 | at most 2 MiB (PK-CUSTOM) |
| `/DataMashup` | single | [MS-QDEFF]; ORA-x* | |
| `/UnappliedChanges` | single | ORA-x04 | |
| `/Connections` | single | not seen in a Desktop file (PK-CONN) | written by BIcraft only for a live-connected PBIX (§4) |
| `/Report/MobileState`, `/Report/LinguisticSchema`, `/DiagramState` | single | not seen in a Desktop file (PK-ENC-MINOR) | kept byte-identical, never written |
| `/DAXQueries`, `/TMDLScripts` | folder | the project folders of the same name (L-REPORT, L-MODEL); not seen in a Desktop package | kept byte-identical, never written |

Other parts, and any part a later Desktop adds, are kept byte-identical with their compression
flag and position (§9, §11).

### Folder layouts

| Folder part | Layout | Label |
|---|---|---|
| `/Report/CustomVisuals` | `<visualGuid>/package.json`, `<visualGuid>/resources/<file>`; the `CustomVisualMetadata` item path is relative to `<visualGuid>/resources/` | Observed (§11, "Custom visuals") |
| `/Report/StaticResources` | one part per resource, `RegisteredResources/<name>`, names percent-encoded as OPC requires (PK-PCT, ORA-n01) | Observed, Public (OPC) |
| `/Report/definition` | the PBIR files of [pbir.md](pbir.md): `version.json`, `report.json`, `pages/…`, `bookmarks/…` | Observed, Public (schemas) |
| `/Copilot`, `/VerifiedAnswers` | containers with one `version.json` each; layout in "Copilot containers" below | Observed (G26), Public (schemas) |

### Copilot containers

Each container has a `version.json` file (`{"version": "<x.y.z>"}`, schema `item/version`
1.0.0). The JSON files and their limits are Microsoft's public schemas
(`fabric/item/semanticModel/copilot/…` in microsoft/json-schemas); the layout and the encodings
are from the Desktop check below.

The same layout is used in a PBIX (parts under `/Copilot/`) and in a PBIP
(`<Name>.SemanticModel/Copilot/`); only the text encoding differs: UTF-16LE without BOM in the
package, UTF-8 without BOM in the project (PK-COPILOT-ENC).

| File (relative to `Copilot/`) | Content | Limits |
|---|---|---|
| `version.json` | container 1.0.0, `{"version"}` | |
| `settings.json` | schema `item/semanticModel/copilot/settings` 1.0.0: `indexingEnabled` (bool, optional) | |
| `schema.json` | schema `item/semanticModel/copilot/schema` 1.0.0: `tables[]`: `id`, `name` (required), `visibility`, `synonyms[]`, `columns[]` (same fields plus `index`), `measures[]`, `hierarchies[]` (with `levels[]`) | from the schema: 16 000 tables; 15 999 columns, measures, hierarchies per table and levels per hierarchy; names and ids 512 chars; 1 024 synonyms of 1 024 chars |
| `examplePrompts.json` | schema `item/semanticModel/copilot/examplePrompts` 1.0.0: `prompts[]` (strings) | from the schema: 10 000 prompts of 500 chars |
| `Instructions/version.json` | `{"version"}` | |
| `Instructions/instructions.md` | text file: the AI instructions, Markdown | 15 000 chars (checked: 15 001 refused, below) |
| `VerifiedAnswers/version.json` | verified answers container: `{"version"}` | |
| `VerifiedAnswers/definitions/<name>.json` | one verified answer per file | Unknown: no Desktop-written one seen |
| `VerifiedAnswers/ResourcePackages/…` | raw files (bytes) | |

- JSON files carry `$schema` with the URI and version above
  (`https://developer.microsoft.com/json-schemas/fabric/…/<version>/schema.json`).
- `/VerifiedAnswers/` at the package root next to a `/Copilot/` container is refused (below).
- BIcraft writes a package with a Copilot container with `/Version` 1.36 and one with root
  verified answers only with 1.34; Desktop does not require either (it opens 1.28, below;
  PK-VER-COPILOT). In a project, the folder is read with `definition.pbism` 6.0 (below).
- `/Report/LinguisticSchema` is a separate, report-side part; in a project the linguistic
  metadata lives in the model (TOM `Culture.LinguisticMetadata`, in TMDL cultures).

**Desktop check (2.158.1304.0, 2026-10-14).** A Desktop-saved local sample (its
`SecurityBindings` removed, `[Content_Types].xml` given `json` / `md` defaults) with Copilot
parts added; "opens" means the model loaded in the engine (tables counted through Tabular
Editor), "refused" means Desktop came up on an untitled file:

| Variant | Result |
|---|---|
| valid container (`version.json`, `settings.json`, `examplePrompts.json`, `Instructions/version.json`, `Instructions/instructions.md`; UTF-16LE), `/Version` 1.36 | opens |
| same, `/Version` 1.28 | opens |
| `settings.json` not JSON, `/Version` 1.36 or 1.28 | refused: Desktop parses the container whatever `/Version` says |
| the same files UTF-8 instead of UTF-16LE | refused: package text is UTF-16LE |
| `instructions.md` of 15 001 characters | refused |
| root `/VerifiedAnswers/version.json` next to `/Copilot/` | refused |
| PBIP (`<Name>.SemanticModel/Copilot/`, UTF-8) with `definition.pbism` 4.2, `settings.json` not JSON | opens: the folder is ignored |
| same with `definition.pbism` 6.0 | refused; with valid files it opens: the folder is read from 6.0 (5.0 not tried) |
| the valid PBIX converted by BIcraft to PBIP (`bicraft convert`, TMDL, `definition.pbism` raised to 6.0) | opens; with its `settings.json` then broken, refused (the folder is read) |
| that PBIP converted back to PBIX by BIcraft (`/Version` 1.36) | opens |

**bicraft.** The container is not modeled: a PBIX/PBIT save keeps its parts byte-identical; a
conversion moves the files between `/Copilot/…`, `/VerifiedAnswers/…` and
`<Name>.SemanticModel/Copilot/…`, `VerifiedAnswers/…`, re-encoding the `.json` and `.md` files
outside `ResourcePackages/` between UTF-16LE and UTF-8 (`bicraft_opc::is_container_text`;
other files and text that does not decode keep their bytes) and raising `/Version` to 1.36 (1.34
for root verified answers only) in a package, or `definition.pbism` to 6.0 in a project (the
rest of the file untouched; `bicraft_pbip::write_copilot_files`); `file.inspect` reports `details.copilot` (files, sizes and
counts: instruction length, schema tables / columns / measures, example prompts, verified
answers, `indexingEnabled`; never the text).

### PBIX vs PBIT

A **PBIT** has `/DataModelSchema` (the model as TMSL JSON) and no `/DataModel`; a **PBIX** has
`/DataModel` and no `/DataModelSchema` (OBS; L-PBIT: templates hold no data). Desktop refuses a
package with both (PK-DM-XOR, ORA-d01). Every other part is shared between the two. In Desktop's
files `/DataModel` is the last zip entry (OBS, PK-ORDER); Desktop opens any order (ORA-d03).

### Mashup: `/DataMashup` and `/UnappliedChanges`

| Finding | Label |
|---|---|
| The local Desktop 2.158 files (V3 models) hold neither `/DataMashup` nor `/UnappliedChanges` (OBS); the queries are in the model's M expressions and partitions. | Observed |
| `/DataMashup` starts with a little-endian Int32 version 0, then length-prefixed blocks ([MS-QDEFF]). Desktop refused a `/DataMashup` with another version (ORA-x01), but also the control with version 0 and empty blocks (ORA-x03). | Public; Observed |
| A package holding both `/DataMashup` and `/UnappliedChanges` was refused (ORA-x02); the control x03 shows the oracle does not isolate the rule (PK-MASHUP). | Observed (not isolated) |

BIcraft never writes either part and keeps them byte-identical.

---

## 3. Text encoding of each part

Text parts are **UTF-16LE without BOM** except where listed otherwise; binary parts are stored
verbatim.

| Part | Encoding | Content type | Evidence |
|---|---|---|---|
| `/Version` | UTF-16LE, no BOM, no newline | `""` | OBS; ORA-v05 (UTF-8), ORA-v08 (CRLF) refused (PK-VER-ENC) |
| `/DataModelSchema` | UTF-16LE, CRLF line ends, no trailing newline | `""` | OBS (PK-DMS); ORA-e01 (UTF-8) refused |
| `/DiagramLayout` | UTF-16LE | `""` | OBS; ORA-e03 (UTF-8) refused |
| `/Settings` | UTF-16LE | `application/json` | OBS; ORA-s08 (UTF-8) refused |
| `/Metadata` | UTF-16LE | `application/json` | OBS |
| `/Report/Layout` | UTF-16LE | `""` | G26 (legacy packages written that way open; PK-ENC-LAYOUT) |
| `/Copilot/*`, `/VerifiedAnswers/*` text | UTF-16LE | `""` | G26 (§2; UTF-8 refused) |
| `/Report/definition/*` (PBIR in PBIX) | **UTF-8**, no BOM | `""` | OBS; ORA-r01 (BOM), ORA-r02 (UTF-16LE) refused (PK-ENC-PBIR) |
| `/DataModel` | binary (engine output) | `""` | OBS |
| `/Report/StaticResources/*`, `/Report/CustomVisuals/*` | raw bytes | `""` | OBS, G26 |
| `/SecurityBindings` | binary | `""` | OBS |
| `/Connections` | UTF-8 JSON as BIcraft writes it | `""` | Unknown: no Desktop file seen (PK-CONN) |
| `/Report/MobileState`, `/Report/LinguisticSchema`, `/DiagramState`, `/UnappliedChanges` | Unknown (PK-ENC-MINOR) | | kept byte-identical |

A BOM: before `/Version` or `/Settings` Desktop refuses the file, before `/DataModelSchema` it
opens (§11); a UTF-8 BOM in a PBIR part is refused (ORA-r01). Other parts and other BOMs were not
tried. BIcraft never writes a BOM.

### `[Content_Types].xml` consequences

Since almost all content types are the empty string, Desktop's `[Content_Types].xml` maps nearly
every part to an empty content type and `/Settings` and `/Metadata` to `application/json`
(PK-CT). The exact shape (one `Default` per extension, `Override`s for extension-less parts) and
what the OPC layer refuses or silently ignores are in §11.

---

## 4. The `/Version` part

| Finding | Label |
|---|---|
| Content: `1.<minor>`, e.g. `1.28`, UTF-16LE, no BOM, no trailing newline (PK-VER-ENC). | Observed |
| `1.36` opens; `1.37`, `1.32.0` and ` 1.32` are refused (ORA-v01–v04). Minors below 1.28 were not opened one by one (PK-VER-SET). | Observed |
| Desktop writes 1.28 with a legacy report and 1.32 with a PBIR report (OBS, PK-VER); a PBIR package marked 1.28 opens (ORA-v07). | Observed |
| With Copilot content, `/Version` 1.28 and 1.36 both open (§2). | Observed |

BIcraft writes 1.28 (legacy report) or 1.32 (PBIR), as Desktop does; 1.29 for a live-connected
report (unverified: PK-VER-EXT); 1.34 or 1.36 with verified answers or Copilot content (§2). What
the other minors mark is Unknown.

Inner versions of the JSON parts:

| Part | Rule | Evidence |
|---|---|---|
| `/Settings` | a top-level `Version` of at most 4; missing or 5 is refused | OBS (4); ORA-s01, ORA-s09 (PK-SETTINGS) |
| `/Settings` `QueriesSettings.Version` | required; at least 1.0.3207.0, not from the future; two components are enough | ORA-s02–s06 (PK-QSV) |
| `/Metadata` | a top-level `Version` of at most 5; 6 is refused | OBS (5); ORA-m01 (PK-METADATA) |
| `/Connections` | BIcraft writes `Version` 4 for a live connection | Unknown (PK-CONN) |
| `/UnappliedChanges`, `/SecurityBindings` | | Unknown |

---

## 5. `/DataModel` and the Analysis Services boundary

| Finding | Label |
|---|---|
| The payload is the engine's backup format (ABF) with XPress9 compression; see `[MS-XLDM]`, `abf.md` and `xpress9.md`. | Public spec, project notes |
| A project's `.pbi/cache.abf` has the same format (P-CACHE). | Observed, Public (L-MODEL) |

For PBIT, the boundary is `/DataModelSchema`: the TMSL database JSON, stored as UTF-16LE text
(OBS).

---

## 6. PBIP (Power BI Project) layout

PBIP text files are **UTF-8 without BOM** (L-PBIP; a BOM in a PBIR or TMDL file is refused,
ORA-j09, ORA-j10; P-UTF8). Desktop writes CRLF; LF opens too (P-EOL). The JSON files follow
Microsoft's public schemas (`fabric/pbip/…`, `fabric/item/report/…`, `fabric/item/semanticModel/…`
and `fabric/gitIntegration/platformProperties` in microsoft/json-schemas); member lists below are
the schemas'.

### File inventory

| Path | Required | Content | Evidence |
|---|---|---|---|
| `<Name>.pbip` | yes (entry point) | `$schema`, `version`, `artifacts[]` each with `report.path` (relative), `settings.enableAutoRecovery`; schema `pbip/pbipProperties` 1.0.0 | L-PBIP, schema |
| `.gitignore` (created when absent) | no | `**/.pbi/localSettings.json` and `**/.pbi/cache.abf` | L-PBIP (P-LAYOUT) |
| `<Name>.Report/` | yes | | L-PBIP |
| `<Name>.Report/definition.pbir` | **yes** | `$schema`, `version`, `datasetReference` with `byPath.path` (relative) or `byConnection` (`connectionString`; schema 1.0.0 also `pbiServiceModelId`, `pbiModelVirtualServerName`, `pbiModelDatabaseName`, `name`, `connectionType`). Both refused (ORA-j15), absolute `byPath` refused (ORA-j14) | L-REPORT, schemas (P-BYPATH) |
| `<Name>.Report/.platform` | no | `$schema`, `metadata` {`type`, `displayName`, `description`, `sensitivityLabelId`}, `config` {`version`, `logicalId`}; type `Report`. Next to the older `item.config.json` + `item.metadata.json` pair it is refused (ORA-j18) | L-GIT, schema (P-PLATFORM) |
| `<Name>.Report/report.json` | one of the two report formats | legacy report JSON ("PBIR-Legacy") | L-REPORT |
| `<Name>.Report/definition/` | one of the two report formats | PBIR folder ([pbir.md](pbir.md)) | L-REPORT |
| `<Name>.Report/semanticModelDiagramLayout.json` | no | report-side diagram layout | L-REPORT |
| `<Name>.Report/StaticResources/`, `CustomVisuals/` | no | raw files, relative paths preserved | L-REPORT |
| `<Name>.Report/.pbi/localSettings.json` | no | `$schema`, `remoteArtifacts[].reportId`, `securityBindingsSignature`; schema `item/report/localSettings` 1.0.0 | L-REPORT, schema |
| `<Name>.SemanticModel/` (older suffix `.Dataset`) | with `byPath` | | L-MODEL; ORA-j19 (P-DATASET) |
| `<Name>.SemanticModel/definition.pbism` | **yes** | `$schema`, `version`, `settings` {`qnaEnabled`, `qnaLsdlSharingPermissions`} | L-MODEL, schema |
| `<Name>.SemanticModel/model.bim` | one of TMSL / TMDL | TMSL JSON | L-MODEL |
| `<Name>.SemanticModel/definition/*.tmdl` | one of TMSL / TMDL | TMDL files | L-MODEL |
| `<Name>.SemanticModel/modelReference.json` | live/remote model alternative | `$schema`, `connectionString`, `isMultiDimentional` (sic), `connectionType`; schema `item/semanticModel/modelReference` 2.0.0 | schema |
| `<Name>.SemanticModel/diagramLayout.json` | no | | L-MODEL |
| `<Name>.SemanticModel/.platform` | no | type `SemanticModel` | L-GIT |
| `<Name>.SemanticModel/DAXQueries/`, `TMDLScripts/` | no | `.dax` / `.tmdl` files | L-MODEL |
| `<Name>.SemanticModel/Copilot/` | no | the Copilot container, UTF-8 (§2); read only with `definition.pbism` 6.0 | Observed (§2, ORA-j20; P-COPILOT) |
| `<Name>.SemanticModel/.pbi/localSettings.json` | no | `$schema`, `userConsent` {`compositeModel`, `dynamicQueryParameters`, `qnAForLiveConnect`}, `remoteArtifacts[].datasetId`, `securityBindingsSignature`, `remoteModelingObjectId`; schema `item/semanticModel/localSettings` 1.1.0 | L-MODEL, schema |
| `<Name>.SemanticModel/.pbi/editorSettings.json` | no | `showHiddenFields`, `autodetectRelationships`, `parallelQueryLoading`, `typeDetectionEnabled`, `relationshipImportEnabled`, `relationshipRefreshEnabled`, `runBackgroundAnalysis`, `shouldNotifyUserOfNameConflictResolution`; schema `item/semanticModel/editorSettings` 1.0.0 | L-MODEL, schema |
| `<Name>.SemanticModel/.pbi/unappliedChanges.json` | no | `conceptualSchemaSettings`, `queries`, `queryGroups`, `culture`, `firewallEnabled`; schema `item/semanticModel/unappliedChanges` 1.0.0 | schema |
| `<Name>.SemanticModel/.pbi/cache.abf` | no | the engine data cache (same format as PBIX `/DataModel`); a project opens without it | L-MODEL (P-CACHE) |

### `$schema` URIs and versions

| Rule | Evidence |
|---|---|
| Schema URIs have the shape `https://developer.microsoft.com/json-schemas/fabric/<key>/<major>.<minor>.<patch>/schema.json`. | schemas |
| Desktop refuses an unknown major (ORA-j11, ORA-j13) and opens a newer minor (ORA-j12). | Observed (P-SCHEMA) |
| `.pbip` and `definition.pbism` without `$schema` (the form Desktop writes) open (ORA-j17). | Observed (P-V1) |

Schema keys and the latest published versions (microsoft/json-schemas, 2026-11-25):

| File | Schema key | Latest version |
|---|---|---|
| `.pbip` | `pbip/pbipProperties` | 1.0.0 |
| `definition.pbir` | `item/report/definitionProperties` | 2.0.0 (1.0.0 also published) |
| `definition.pbism` | `item/semanticModel/definitionProperties` | 1.0.0 |
| `.platform` | `gitIntegration/platformProperties` | 2.1.0 |
| report `.pbi/localSettings.json` | `item/report/localSettings` | 1.0.0 |
| model `.pbi/localSettings.json` | `item/semanticModel/localSettings` | 1.1.0 |
| `.pbi/editorSettings.json` | `item/semanticModel/editorSettings` | 1.0.0 |
| `modelReference.json` | `item/semanticModel/modelReference` | 2.0.0 |
| `.pbi/unappliedChanges.json` | `item/semanticModel/unappliedChanges` | 1.0.0 |
| container `version.json` (Copilot, verified answers) | `item/version` | 1.0.0 |
| PBIR files | `item/report/definition/<file>/<version>` | per file |

`version` property values:

| File | Values | Evidence |
|---|---|---|
| `definition.pbir` | `1.0` (legacy `report.json` only), `4.0` (PBIR `definition/` read); `3.0` with `definition/` also opens | L-REPORT; ORA-j01, j03, j04 (P-PBIR-VER) |
| `definition.pbism` | `1.0` (TMSL only), from `4.0` TMDL read (3.0 with TMDL also opens); Desktop 2.157 writes 4.2; `6.0` reads the Copilot folder | L-MODEL; ORA-j05, j06; §2 (P-PBISM-VER, P-COPILOT) |
| `.pbip` | `1.0` | L-PBIP |

### Report and model formats

| Rule | Evidence |
|---|---|
| `report.json` next to a PBIR `definition/` folder is refused (ORA-j08); so is `model.bim` next to TMDL (ORA-j07). | Observed (P-BOTH) |
| PBIR page, visual and bookmark names are letters, digits, `_` and `-`; Desktop drops other names (ORA-j21–j23). | L-REPORT; Observed (P-NAME) |
| Whether Desktop writes PBIR or legacy, TMDL or TMSL, depends on its options (the PBIR and TMDL preview features, L-REPORT, L-MODEL); their defaults per release were not determined. | Public; Unknown |

---

## 7. Package rules (writer checklist)

Rules Desktop's answers establish (`check_package`, `crates/pbixwriter/src/check.rs`, reports each):

| # | Rule | Evidence |
|---|---|---|
| 1 | The file must be a valid OPC package; every part needs a content type (§11). | G26, PK-OPC |
| 2 | No `/Version.txt` part (refused); an unrelated `.txt` part opens. Other `<part>.txt` siblings were not tried. | ORA-k01, ORA-k02 (PK-TXT) |
| 3 | `/Version` must exist and hold an accepted value (§4). | ORA-v* |
| 4 | `/DataModel` and `/DataModelSchema` must not both exist. | ORA-d01 |
| 5 | `/Report/Layout` must not coexist with `/Report/definition/`. | ORA-l01 |
| 6 | `/docProps/custom.xml`, if present, at most 2 MiB. | ORA-c01, ORA-c02 |
| 7 | `/DataMashup` starts with Int32 0; not together with `/UnappliedChanges`. | [MS-QDEFF]; ORA-x02 (not isolated) |
| 8 | `/Settings` and `/Metadata` parse as UTF-16LE JSON with an inner version not above 4 and 5; `/Settings` `QueriesSettings.Version` as in §4; `/DataModelSchema` and `/DiagramLayout` are UTF-16LE JSON. | ORA-s*, ORA-m01, ORA-e01, ORA-e03 |
| 9 | PBIR parts are UTF-8 without BOM and valid JSON, with `version.json`, `report.json` and at least one page. | ORA-r01, ORA-r02; L-REPORT (PK-PBIR-PAGE) |
| 10 | Files named `.pbix`, `.pbit`, `.zip` and `.tmp` open. | G26 (PK-EXT) |

`/SecurityBindings` is optional, and a stale one makes Desktop refuse the file (§9, PK-SECBIND).
Its content is Unknown; a writer should omit the part, and BIcraft drops it on any report change.

### Minimal writer recipe (Inferred from the above)

1. Write an OPC package (valid `[Content_Types].xml`; content type `""` for all parts except `application/json` for `/Settings` and `/Metadata`).
2. Write `/Version` = `1.28` (legacy report) or `1.32` (PBIR) as UTF-16LE, no BOM, no newline.
3. Write `/Report/Layout` (UTF-16LE) **or** `/Report/definition/*` (UTF-8, no BOM), never both.
4. PBIX: `/DataModel` stored uncompressed. PBIT: `/DataModelSchema` (UTF-16LE TMSL), no `/DataModel`.
5. Omit `/SecurityBindings`, `/DataMashup` and `/UnappliedChanges`.
6. Do not create a `/Version.txt` part.

---

## 8. Saving model edits into an existing package (BIcraft)

`bicraft_pbix::write_model(&package, &model)` writes an edited model back into the package it
was read from; the engine's `file.save` / `file.save_as` use it. Recipe:

| Package | What changes | How |
|---|---|---|
| PBIT (`/DataModelSchema`) | the TMSL text | `bicraft_tmsl::write(model)`, encoded in the part's original encoding (UTF-16LE for Desktop files) |
| PBIX (`/DataModel`) | `metadata.sqlitedb` inside the backup | decompress (XPress9) → parse the ABF → `bicraft_metadata::write_sqlitedb` (apply the metadata-only diff, rewrite only the changed SQLite tables) → `AbfWriter::replace_metadata_sqlitedb` → `finish` → `compress_datamodel(abf, Some(&original_framing))` (section "Writing the DataModel framing" of `xpress9.md`) → decompress again and compare with the written ABF |

- Every other part keeps its name, position, bytes and compression flag. `/DataModel` is
  written **stored** (section 1). A `/DataModel` that was a bare ABF stays a bare ABF.
- Supported PBIX edits are those `apply_model` writes (`metadata-sqlitedb.md`, "Writing"):
  measures (add, remove, move, expression, format string, display folder, description, hidden),
  descriptions and display properties of tables and columns, annotations, roles and their RLS
  filters, shared expression text. `write_model` refuses anything that needs VertiPaq data
  with `Error::UnsupportedChange`; the engine sends those saves through the structural writer
  below.

### Structural saves (`bicraft_pbix::structural`, engine `pbix_struct.rs`)

`DataModelEditor::open(part)` decompresses `/DataModel` once; `changes(model)` diffs the edited
model against the stored one: tables and columns added, removed or **renamed** (a different
name with the same lineage tag), tables whose partitions changed (**replaced** data),
relationships added or removed (by name) and user hierarchies added, removed or restructured
(levels changed). Without such changes `write` is the metadata-only
path above. Otherwise the caller supplies the values of every new table and new column
(`StructuralData`) and `write`:

1. encodes each new column with `bicraft-vertipaq` (`EncodingRequest::Auto`, or hash
   encoding for the "one" end of a relationship; strings equivalent under the model collation
   share the first spelling, as the engine's dictionary does), reads the result back to get
   every row's data id (and checks it decodes to the given values), builds its `H$` attribute
   hierarchy (with its sort-by column when it has one) and the table's row-number column; a
   new column of a table of several partitions (incremental-refresh policy ranges, hybrid
   tables) is then encoded once per partition with the whole column's dictionary order or
   value parameters (one segment file pair per partition, RLE statistics summed; engine-loaded,
   `vertipaq-large.md` §5); a new or replaced table of several partitions
   (`write_partitioned`, with the rows of each partition) gets the same per-partition segments,
   its row numbers continuing across partitions;
2. builds the `R$` index of each new relationship from the new values or from the stored
   columns, decoded from the backup: many-to-one (the "one" side must hold no blank and no
   duplicate under the collation), one-to-one (both sides unique, one index per direction) and
   many-to-many (no index: the engine stores none; blanks and duplicates on both sides);
   and the `U$` node table of each new user hierarchy (levels' data ids and attribute-hierarchy
   order from the new values, else from the stored columns and their `H$` tables);
3. writes the rows with `bicraft_metadata::write_structural_sqlitedb_renamed_at`: renames are
   applied to the base rows first (`Table.Name`, `Column.ExplicitName` only: storage names,
   folders and files keep the old name next to the object ID), then the structural edit;
4. a replaced table is written in two stages: first without it (and the relationships, role
   permissions and perspective memberships that touch it), then with it back, same name,
   lineage tags, measures, hierarchies, calculated columns and perspective memberships, new
   data, relationships rebuilt;
5. applies the files to remove and add to the ABF (`AbfWriter`), re-compresses like the
   original part and reads it back.

The engine (`file.save` / `file.save_as` on a PBIX) gathers the values: `m` partitions run in
BIcraft's Power Query evaluator with the model's culture and shared queries, reading files only
under the session's opt-in `data_root` (`data-sources.md`, *In the engine*; `table.import` /
`table.set_data` write CSV data as a `#table` literal, so the table stays refreshable in
Desktop), and text is stored as the engine stores it (`data-sources.md`, *Stored text*);
calculated tables load through the DAX literal loaders or, otherwise,
the viewer's DAX evaluator in an empty filter context (a calculated table without declared
columns gets them from the result, as `calculatedTableColumn`s), calculated columns are
evaluated row by row with that row as row context (in passes, so they may depend on each
other and on new calculated tables). A table of several `m` partitions runs each partition's
query on its own (policy-range and other partition kinds are refused). A data column added to
an existing import table reads its values from the table's `m` partitions run again: every row
of the result is matched to a stored row through the columns already stored (in order when
they agree, else by value), so the values follow the engine's row order; refused when the query
no longer returns the stored rows (then `table.set_data`). Anything the evaluator does not
support is refused with its reason. Nothing is written when anything is refused.
- An unchanged model rewrites nothing in `metadata.sqlitedb` and re-packs the ABF byte for
  byte; only the XPress9 session signature and block header CRCs of `/DataModel` differ from
  Desktop's file (checked on a local Desktop sample: same part size, same envelope and block
  headers).
- The engine replaces the file atomically (write a `<name>.bicraft-tmp` sibling, then rename)
  and `file.save` keeps the previous file as `<name>.bak` unless `backup: false`. Errors leave
  the file untouched. `file.save_as` to another kind of file converts (§10). The model is
  written only when it has unsaved edits: a save whose only edits are in the report keeps
  `/DataModel` as it was (§9).

Desktop 2.158 opens the rewritten files (identity, added measure, changed format string,
added RLS role) and shows the edits (M5.1, 2026-10-13; open question 11). The structural
leftovers (many-to-many, one-to-one and both-direction relationships, hierarchies, tables of
several partitions, perspective memberships of replaced tables, a calculated table's column
removed) are engine-checked during development (`metadata-sqlitedb.md`, "Engine check of
the structural writer").

---

## 9. Saving report edits into an existing package (BIcraft)

`bicraft_pbix::write_report(&package, &report)` writes an edited report back into the package it
was read from, **in the form the package holds**; the engine's `file.save` / `file.save_as` call
it when the report has unsaved edits (and `write_model` when the model has; either, both or none).

| Package report | How it is written |
|---|---|
| PBIR (`/Report/definition/…`) | `bicraft_pbir::write_documents`, then each document under `definition/` is re-serialized **without indentation** (Desktop's PBIX documents are compact JSON, unlike the indented PBIP files). A document whose JSON value equals the part's keeps the part's bytes, so untouched pages and visuals are byte-identical. Parts under `/Report/definition/` the report no longer writes (removed pages, visuals, bookmarks) are removed. |
| Legacy (`/Report/Layout`) | `bicraft_layout::write_layout` patches the document (`legacy-layout-bb.md`), re-encoded in the part's encoding (UTF-16LE without BOM). Never converted to PBIR. |
| both forms, `/Report/StaticResources/…` | follow `Report::resources`: changed bytes rewritten, new resources added (part names percent-encoded like System.IO.Packaging), a part the report no longer lists removed. |

- Every other part keeps its name, bytes, compression flag and position. New parts are inserted
  after the last existing `/Report/` part, deflated. With no model edit, `/DataModel` (or
  `/DataModelSchema`) is not touched at all: no XPress9 decompression or re-compression.
- `[Content_Types].xml` changes only when needed: a `Default Extension="…" ContentType=""` for a
  new extension (after the last `Default`), an `Override` for a new extension-less part, and the
  `Override` of each removed part dropped. Otherwise its bytes are kept.
- **`/SecurityBindings` is removed whenever a report part changes** (and its `Override`). See
  the finding below.
- The engine reads the written report (and model, when written) back before replacing the file;
  any error leaves the file as it was.

### Finding: Desktop rejects a package whose report parts no longer match `/SecurityBindings`

Desktop 2.158.1304.0, 2026-10-13, local PBIR sample (PBIX with `/SecurityBindings`). Packages
rewritten with .NET's public `System.IO.Packaging` API, so the zip
writer is not a variable:

| Package | Desktop |
|---|---|
| sample re-zipped, no change | opens |
| `pages/pages.json` `activePageName` changed, `/SecurityBindings` kept | **refused**: "this file is damaged or was created by an unrecognized version of Power BI Desktop" |
| same change, `/SecurityBindings` (and its `Override`) deleted | opens, on the new active page |

So the "byte-identical PBIX with only the active page changed" that Desktop refused failed the
`/SecurityBindings` check: the part depends on the report parts, which no longer matched. It is optional: Desktop
opens the file without it and writes a new one on its next save. BIcraft therefore drops it on
any report change (`security_bindings_removed: true` and a warning in the save result). The zip
writer played no part (the sample re-zipped by another writer, unchanged, opens; so do
BIcraft's identity saves). Inferred, not tested: an edit of `/Connections` or `/Settings` would
need the same treatment; BIcraft does not edit them.

### Desktop check of report saves

Desktop 2.158.1304.0, 2026-10-13, local sample PBIX and PBIT (PBIR) and a legacy-layout PBIX built
from the sample for the test; each opened with the model loaded (8 tables, 25 measures, 4
relationships) and the edit visible on the opened page:

| Case (bicraft commands, then `file.save_as`) | Desktop |
|---|---|
| identity save (nothing edited): every part keeps its bytes, `/SecurityBindings` kept | opens |
| `page.set_active` | opens on that page |
| `visual.set_position` (move + resize a card) | opens; the card is at its new place |
| `visual.add` (card bound to a measure) + `visual.set_title` | opens; the titled card shows its value |
| `page.add` + `visual.add` on it, `page.remove` of another page | opens on the new page, card computed |
| `report.apply` (a page with a card and a two-field table) | opens; both visuals computed |
| `measure.add` + `visual.add` binding the new measure (model and report in one save) | opens; the engine lists the new measure (26 instead of 25) |
| legacy `Report/Layout` package (built for the test), then `visual.set_position`, `visual.add` (column chart, two fields) + `visual.set_title`, `page.add`, `page.set_active` | the base file and the edited one open; the edited one shows the titled chart on the chosen page |
| M2.5 edits on a new page of the PBIR sample: single-series column color, average line, y axis from zero, table gradient, phone layouts (`mobile.json`), an interaction rule, a tooltip page and a drillthrough page (with back button), a report-page tooltip, `theme.set_colors` | opens on the new page; colors, line, axis and gradient as set; both new pages listed (`report.validate`: no issue before the save) |
| PBIT: `visual.set_position` + `page.set_active` | the template opens on that page with the card moved (the data visuals of this template show errors until refreshed, as the unedited template does) |

Timing (release build, local 280 KB sample, `report_only_save_timing`): building the saved package
after a visual move took 60 ms when the model was rewritten (what every package save did before)
and 27 ms with `/DataModel` kept; the difference grows with the model, since XPress9 compresses
at 5–25 MB/s (`benchmarks.md`).

## 10. Conversions between PBIX, PBIT and PBIP (BIcraft, M3.3)

`file.save_as` to another kind of file and `file.convert` (`crates/engine/src/convert.rs`) go
through the model and report: PBIT and PBIX packages are built by `bicraft_pbixwriter`
(`write_pbit_with`, `write_pbix`, see `pbit.md`), projects by `bicraft_pbip` (`create_project`,
then the report, `diagramLayout.json` and `.pbi/cache.abf`, see `pbip.md`). A PBIX is only built
from data that exists: an open PBIX's `DataModel` or a project's `.pbi/cache.abf`, which has the
same format (`pbip.md`, "Data cache").

Two model spellings mattered for Desktop (found by this check):

- The `metadata.sqlitedb` reader spelled a column's `IsAvailableInMDX` as `isAvailableInMDX`;
  Desktop 2.158 refuses that in TMSL ("unrecognized JSON property") and TMDL. The reader now
  produces TOM's `isAvailableInMdx` (the PBIX writers still accept the old spelling on input).
- The TMDL writer put a model property it has no rank for (`valueFilterBehavior`) after the
  nested `dataAccessOptions` block; Desktop refuses that ("invalid indentation", `model.tmdl`).
  Unranked scalar properties now come first.

### Desktop check (Desktop 2.158.1304.0, 2026-10-14)

Copies of a local PBIX (PBIR report, 8 tables, 25 measures, 4 relationships, 10 pages) and of
its PBIT, plus a legacy variant of the PBIX (the PBIR report replaced by a one-page
`Report/Layout` of our own), were converted with `bicraft convert` and each result opened in
Desktop; the model was counted through Tabular Editor on Desktop's engine. The source PBIX gives
8 tables, 25 measures, 4 relationships.

| Conversion | Report | Desktop |
|---|---|---|
| PBIX → PBIT | PBIR | opens; 8 / 25 / 4 |
| PBIX → PBIP (TMDL, with cache) | PBIR | opens with data; 8 / 25 / 4 |
| PBIX → PBIP (TMSL, with cache) | PBIR | opens; 8 / 25 / 4 |
| PBIT → PBIP (TMDL, no cache) | PBIR | opens; 8 / 25 / 4 |
| PBIP (TMDL) → PBIT | PBIR | opens; 8 / 25 / 4 |
| PBIP (TMDL, cache) → PBIX | PBIR | opens; 8 / 25 / 4 |
| PBIP TMDL → PBIP TMSL (cache copied) | PBIR | opens; 8 / 25 / 4 |
| PBIP (TMSL, cache) → PBIX | PBIR | opens; 8 / 25 / 4 |
| legacy PBIX → PBIP (TMDL) | legacy `report.json`, `definition.pbir` 4.0, no `definition/` | opens; 8 / 25 / 4 |
| legacy PBIX → PBIT | legacy `Report/Layout`, `Version` 1.28 | opens; 8 / 25 / 4 |
| legacy PBIX → PBIP, `report_format: pbir` | PBIR (converted) | opens; 8 / 25 / 4 |
| legacy PBIP → PBIX | legacy `Report/Layout` | opens; 8 / 25 / 4 |
| legacy PBIP → PBIT, `report_format: pbir` | PBIR (converted) | opens; 8 / 25 / 4 |

Before the two spelling fixes above, the PBIT and both PBIP projects made from the PBIX were
refused. A PBIX's `DataModel` copied as a project's `cache.abf` (a different database id from
the one Desktop would write) is accepted. PBIT → PBIX, PBIP without `cache.abf` → PBIX and a
new model or `model.bim` → PBIX build the `DataModel` from scratch (below).

### Built from scratch (M5, 2026-10-14)

Without a source `DataModel`, `bicraft_pbixwriter::write_pbix` gets one built by
`crate::pbix_new` / `bicraft_pbix::fresh` ([`datamodel-fresh.md`](datamodel-fresh.md)): an
empty database image, then every table added by the structural writer (§8). Desktop 2.158
(local check, synthetic models) opened, with the model loaded in its engine: a new model with one
M literal table and a measure (1 table, 1 measure); a `model.bim` with four tables (M literal, CSV
under `data_root`, `CALENDAR` date table, `FILTER` calculated table), a calculated column, two
relationships, three measures, an RLS role and a parameter (4 / 3 / 2); a PBIT and a PBIP
without cache, each with an imported CSV table, a calculated column and a role. Before the
database key file was written (`datamodel-fresh.md`, "Database key") every such file was
refused. Second round (queries through Desktop's own engine, window captured):

| File | Desktop |
|---|---|
| the `model.bim` star with a user hierarchy and a report of our own (card, table, column chart) | opens; the card shows 8.75, the table the per-product totals, the chart the two months; DAX queries return the engine-oracle values; the hierarchy (2 levels) and the role are listed; a full refresh runs the CSV query again and every value is unchanged |
| a table written empty (`empty_tables`, an `Sql.Database` source) next to an M literal table | opens; the empty table counts no row, the other 2; a refresh attempt waits on the unreachable server (expected for that source) |
| PBIT → PBIX, PBIP without cache → PBIX | open; the role and its filter are there, the table and its calculated column hold the expected values |

## 11. Package-level checks against Desktop 2.158 (G26)

Desktop 2.158.1304.0, 2026-10-14. Two oracles:

- **System.IO.Packaging**: `System.IO.Packaging.Package.Open(stream, Open, Read)` from Windows
  PowerShell 5.1 (.NET Framework's public OPC API), then every part read. This answers what the OPC layer accepts.
- **Desktop**: the file opened in Desktop; accepted means the window shows the file and the
  engine holds the model (counted through Tabular Editor on Desktop's engine), refused means
  Desktop falls back to an empty untitled report with no model.

Inputs: a PBIT of our own (`model.new` + `table.import` + a card, built by BIcraft: 1 table,
1 measure) and copies of a local Desktop-saved PBIX (8 tables, 25 measures, 4 relationships; not
committed), each changed at the package level only.

### Zip layout

What Desktop's own files look like (the local PBIX and PBIT saved by 2.158; observed): every
entry has "version made by" 4.5 on MS-DOS (`0x002D`), "version needed" 2.0 (deflated) or 1.0
(the stored `/DataModel`), general-purpose flags 0, a 28-byte local extra field (tag `0xA220`,
padding) and no central extra field, no data descriptor, no
Zip64 record, no archive comment, local times as timestamps. `/Version` is the first entry and
`[Content_Types].xml` the second; `/DataModel` (written by the engine into the closed file) is
the last.

| Variant | System.IO.Packaging | Desktop |
|---|---|---|
| re-zipped by Python's `zipfile` (made by 2.0, Unix host, 1980 timestamps) | opens | opens (8 / 25 / 4) |
| **forced Zip64** (BIcraft `WriteOptions::force_zip64`: Zip64 extra field with both sizes in every local header, sizes and offset in every central header, Zip64 end record and locator, `0xFFFF` / `0xFFFFFFFF` in the classic end record) — PBIX and PBIT | opens | opens (8 / 25 / 4; the PBIT loads its table) |
| data descriptors (flag bit 3, sizes 0 in the local headers, as streaming writers emit) | opens | opens (8 / 25 / 4) |
| entries in reverse order (`/DataModel` first, `/Version` last); `[Content_Types].xml` last; `/Version` last | open | reverse order: opens (8 / 25 / 4) |
| every part stored; `/DataModel` deflated | open | not run |
| the BIcraft writer's default layout (below), PBIX and PBIT | opens | opens (8 / 25 / 4; the PBIT loads its table) |

So Desktop's reader takes any entry order, data descriptors and Zip64, including Zip64 on small
files. A `/DataModel` over 4 GiB (or a package of more than 65,535 parts) needs Zip64; whether
Desktop writes it the same way could not be checked here (no such file was
made on this machine: disk and memory are too tight), so it is Inferred from the reader accepting
every Zip64 record.

**BIcraft's writer** (`bicraft_opc::Package::write`, `crates/opc/src/zipwrite.rs`) lays
entries out like Desktop's files, without their padding extra field: made by 4.5 / MS-DOS, needed 2.0 or 1.0 (4.5 for a Zip64 entry), flags
0 (bit 11 only for a non-ASCII name), no extra field, no data descriptor, a fixed 1980-01-01
timestamp (byte-stable output). Zip64 is written only where a value needs it: an entry whose
size reaches `0xFFFFFFFF` gets the Zip64 extra field with **both** sizes in its local header
(APPNOTE 4.5.3) and in its central header; an entry whose offset reaches `0xFFFFFFFF` gets the
offset in its central Zip64 field; 65,535 entries or more, or a central directory past 4 GiB,
add the Zip64 end record and locator. The previous writer (the `zip` crate) put an offset-only
Zip64 field in the *local* header of a small entry stored after 4 GiB, which APPNOTE does not
allow. Reading stays with the `zip` crate, which handles Zip64 and data descriptors; the size
caps (§ F4 of `fuzzing.md`) apply to the sizes a Zip64 field declares too. Tests: layout field
by field, forced Zip64 round trip, 70,000 parts, a hand-made data-descriptor archive, lying Zip64
records (`crates/opc/src/package.rs`).

### Packages over 4 GiB (BIcraft)

A `/DataModel` over 4 GiB is held neither in memory nor decompressed:

- **Deferred parts.** `Package::open` (any source that can be opened again,
  `bicraft_opc::ArchiveSource`; a file through `FileSource`) does not read an entry whose
  declared size is over `ReadLimits::max_part_bytes` (4 GiB): the part records the offset of its
  compressed data, both sizes and the CRC (`Part::deferred`) and counts nothing toward the
  in-memory total. Reading from bytes (`read_bytes`) still refuses such a part. `Part::len`,
  `Part::reader` (decompressed stream) and `Part::seekable` (stored parts) give access to it.
- **Writing.** The writer copies a deferred part's compressed bytes from the source in 1 MiB
  pieces, with the recorded CRC and sizes, so nothing is decompressed or recompressed; the byte
  count must match and a stored part's CRC is recomputed on the way (a source that changed is an
  error, never a corrupt archive). `FileSource` also refuses a file whose length or modification
  time changed since it was opened. Zip64 fields are written where the values need them: both
  sizes for the large entry, the offset for every entry after it, the Zip64 end record when the
  central directory starts past 4 GiB. The engine writes saves straight to a temporary sibling
  (no archive in memory), renames it over the target, then points the deferred parts at the new
  file (`Package::rebind_deferred`).
- **Reading the model.** Only `metadata.sqlitedb` is needed (`bicraft_pbix::large`). The part
  is decoded as a stream (`bicraft_xpress9::stream_datamodel`: one chunk at a time, the
  decoder keeping its own 4 MiB history window; sizes per chunk capped at 256 MiB) and only four
  ranges are kept: the header page, the virtual directory with the 64 MiB before it (where the
  BackupLog usually is), the BackupLog if it was not in that window, and `metadata.sqlitedb`
  (`bicraft_abf::streamed_header` / `streamed_log_range` / `streamed_index`; each range at
  most 512 MiB). Each range is one pass that stops as soon as it is complete, so a model is
  usually read in two passes over the part. A bare ABF part (no XPress9 framing) that is stored
  is read by seeking. The same path serves an in-memory part whose backup declares more than
  `DataModelLimits::max_bytes` (4 GiB) but stays within `max_ratio` of the part.
- **Limits.** Model edits cannot be saved into such a package (`Error::DataModelTooLargeToWrite`:
  the backup would have to be rewritten); report edits can, and the part is copied through. The
  data preview and conversions that need the `DataModel` bytes (to PBIT/PBIP `cache.abf`) refuse
  it with a message.
- **Tests, without 4 GiB of memory or disk.** Archives held sparsely (zero runs not stored): a
  5 GiB stored part between two small ones written, read back deferred, written again byte for
  byte, with the Zip64 fields checked (`crates/opc/tests/large.rs`); a bare 5 GiB backup whose
  BackupLog and directory sit past 4 GiB, read through seeking; deferred and streamed reads of
  XPress9 parts equal to the in-memory reader (`crates/pbix/tests/large_model.rs`); the
  streaming decoder against the whole-part decoder, single- and multithreaded layouts
  (`crates/xpress9/src/stream.rs`).

What Desktop itself writes for a backup over 4 GiB was not observed: a
model that large cannot be built within this machine's memory.

### `[Content_Types].xml`

Desktop's files (observed, 2.158): UTF-8 with BOM, one line, `Default` elements first (one per
extension, in the order of the first part with it, all with `ContentType=""`), then `Override`
elements for extension-less parts in part order: `/Version` `""`, `/DataModelSchema` `""`,
`/DiagramLayout` `""`, `/Settings` and `/Metadata` `application/json`, `/SecurityBindings` `""`,
`/DataModel` `""`. A package with PBIR and images has exactly `Default json` and `Default png`.
BIcraft's writers emit the same shape (`crates/pbixwriter/src/content_types.rs`); `write_report`
adds a `Default` for a new extension and an `Override` for a new extension-less part (§9). A
custom visual adds only `.json` files, so it needs no new entry.

| Variant | System.IO.Packaging | Desktop |
|---|---|---|
| no BOM; line breaks and indentation between elements | open | not run (System.IO.Packaging is the parser) |
| only `Override` elements, one per part, no `Default` | opens | opens (8 / 25 / 4) |
| unused extra `Default` (`xml`, `pbiviz`) and an `Override` for a part that does not exist | opens | not run |
| `Override`/`Extension` names in another case (`/VERSION`, `JSON`) | opens (names and extensions ignore case) | not run |
| `Extension=".json"` (leading dot) | opens, `.json` parts still found | not run |
| every `""` replaced by `application/octet-stream` | opens | opens (8 / 25 / 4) |
| `/Settings` and `/Metadata` given `""` | opens | not run |
| `application/json; charset=utf-16` | opens (parameters allowed) | not run |
| **the `Default` for `json` removed** | opens, but **every `.json` part is hidden** (13 of 143 parts listed) | **refused** (the PBIR report is gone) |
| the `Override` for `/Version` removed | `/Version` hidden → missing | refused (Inferred: the required part is missing) |
| no `[Content_Types].xml` | every part hidden | refused (Inferred) |
| two `Default`s for `json` (also `json` + `JSON`); two `Override`s for one part (also another case) | **throws** (duplicate key) | refused |
| another namespace on `<Types>`; an unknown element; an unknown attribute; malformed XML | throws (schema / XML error) | refused (Inferred: §7 rule 1) |
| a content type that is not `type/subtype` (`json`) | throws | refused (Inferred) |
| an `Override` of `/[Content_Types].xml` | throws (invalid part URI) | refused (Inferred) |

So a part that no content type covers is **silently dropped** by the OPC layer (no error); a
duplicate or a schema violation makes the whole file unreadable. `check_package`
(`crates/pbixwriter/src/check.rs`) reports both: a part without a content type ("Desktop would
not see the part") and every System.IO.Packaging refusal above (`content_types::strict_issues`). Desktop's
own files pass.

### BOM in text parts, file extensions

| Variant | Desktop |
|---|---|
| UTF-16LE BOM (`FF FE`) before `/Version` | refused (the version string no longer matches, §4) |
| UTF-16LE BOM before `/Settings` | refused |
| UTF-16LE BOM before `/DataModelSchema` (PBIT) | opens, model loaded |
| the PBIT copied to `.zip` and `.tmp`, opened through the Store alias `PBIDesktopStore.exe <file>` | both open, model loaded |

So a BOM is tolerated in some JSON parts and fatal in others; BIcraft never writes one (its
decoder accepts one on read). The `.zip` and `.tmp` extensions are accepted on open, as are
`.pbix` and `.pbit` (§7 rule 10).

### Custom visuals

Desktop 2.158 loads a custom visual from a resource package of type `CustomVisual` whose
`CustomVisualMetadata` item names the visual's `.pbiviz.json`; visuals without that item are
refused as legacy. For an AppSource visual the package is implied: name = the visual's guid, one
item `{name: guid, path: guid + ".pbiviz.json", type: CustomVisualMetadata}`. The file side comes
from §2 (`<guid>/package.json`, `<guid>/resources/<file>`). Checked on Desktop with a visual of
our own, built by hand from the public `.pbiviz` format (a `package.json` manifest and `resources/<guid>.pbiviz.json` holding the
`visual` block, `apiVersion` 5.3.0, capabilities with one measure role, and `content` with a small
JavaScript plugin, CSS and a 20×20 PNG icon), embedded in our PBIT:

| Layout | Desktop |
|---|---|
| `Report/CustomVisuals/<guid>/package.json` + `…/resources/<guid>.pbiviz.json`; `report.json` `resourcePackages` += `{name: <guid>, type: CustomVisual, items: [{name: <guid>, path: "<guid>.pbiviz.json", type: CustomVisualMetadata}]}`; a `visual.json` with `visualType` = `<guid>` and a measure projection | opens; the visual's icon appears in the Visualizations pane and its container sits at its position with its title (its sandboxed content is not in window captures) |
| same, item `path` = `resources/<guid>.pbiviz.json` | opens; visual **not** registered (no icon, nothing drawn) |
| same layout, `.pbiviz.json` file missing | opens; visual not registered |

Power BI Desktop 2.159.1231.0 (2026-11-23), saving a PBIP that holds our visual, writes each
`CustomVisualMetadata` item's `path` as its `name`: an item `{name: <guid>, path:
"<guid>.pbiviz.json"}` is saved as `{name: <guid>, path: <guid>}` and the visual is not registered
when the project is reopened, while `{name: "<guid>.pbiviz.json", path: "<guid>.pbiviz.json"}`
keeps its path. BIcraft writes the item with its name equal to its path.
| `publicCustomVisuals: [<guid>]` only (no package) | opens (AppSource visuals are fetched online) |
| the first layout opened in BIcraft, the visual moved (`visual.set_position`), saved as PBIT | every part but that `visual.json` byte-identical and in place; Desktop opens it, visual registered at its new position |

So: **an item path is relative to `CustomVisuals/<package name>/resources/`**. In the PBIR
`report.json` the package and item types are strings (`CustomVisual`, `CustomVisualMetadata`; their form in
a legacy `Report/Layout` was not checked). `package.json` is the
`.pbiviz` manifest (`visual`, `author`, `resources[{resourceId, sourceType: 5, file}]`,
`metadata.pbivizjson.resourceId`).

BIcraft (`bicraft_pbix::read_report`) now attaches the `Report/CustomVisuals/…` parts to the
report as resources in both report forms (PBIP already did), so they convert to PBIP and back;
report saves keep them byte for byte and in place (test
`custom_visuals_and_unknown_parts_survive_report_saves`), and the PBIR validator checks that each
`CustomVisual` item's file exists under `resources/`. `report.custom_visuals` (read-only) lists
embedded visuals (guid, display name, version, API version, author, data roles, privileges, file
sizes, items and the file each resolves to, users, issues), folders under `CustomVisuals/` that no
package registers, AppSource and organization visuals. `visual.import_custom` writes this layout
from a `.pbiviz` file and `visual.remove_custom` removes it ([pbiviz.md](pbiviz.md)).

### Copilot and other newer parts

The Copilot container is now settled (§2, "Copilot containers": layout, encoding and read
rules checked on Desktop with files of our own). A fresh
Desktop save with Copilot, DAX query view or TMDL view content could still not be produced here:
the session has no interactive desktop for UI automation. What is known: the 2.158 part table and
their encodings (§2, §3: `/Copilot/…`, `/VerifiedAnswers/…`, `/Copilot/VerifiedAnswers/…`,
`/DAXQueries/…`, `/TMDLScripts/…` as UTF-16LE containers; `/Report/definition/reportExtensions.json`
as UTF-8 PBIR), and that the local 2.158-saved PBIX and PBIT (V3 model, PBIR report) hold only
`Version`, `[Content_Types].xml`, `DiagramLayout`, `Settings`, `Metadata`,
`Report/StaticResources/…`, `Report/definition/…`, `SecurityBindings` and `DataModel` /
`DataModelSchema`: no `/DataMashup`, `/Connections` or `/UnappliedChanges`, consistent with
the finding that current Desktop saves never write `/DataMashup` (§2). BIcraft keeps every part
it does not model — these and any future one — byte-identical, with its compression flag and
position, through model saves and report saves (test above; `[Content_Types].xml` is only touched for added or removed parts).

## Open questions

1. ~~**`[Content_Types].xml` exact shape.**~~ Settled (§11): Desktop writes one `Default` per extension and an `Override` per extension-less part; System.IO.Packaging accepts `Override`-only files and extra entries, hides parts no entry covers, and refuses duplicates and schema violations.
2. ~~**BOM tolerance.**~~ Settled for the parts tried (§11): fatal before `/Version` and `/Settings`, tolerated before `/DataModelSchema`. Other parts untested; never write one.
3. **`/DataMashup` in current files.** The local 2.158-saved PBIX and PBIT carry none (V3 model), consistent with §2. A non-V3 model saved by 2.158 was not available to check.
4. **`/DataModel` payload.** The ABF layout and XPress9 framing belong to the Analysis Services engine, not to the packaging layer: see `[MS-XLDM]`, `abf.md` and `xpress9.md`.
5. ~~**Zip-level constraints.**~~ Settled (§11): Zip64 (forced on a small file), data descriptors and reversed entry order are all accepted. BIcraft reads and writes packages over 4 GiB without holding them in memory (§11, "Packages over 4 GiB"). Not checked: what the engine itself writes for a `/DataModel` over 4 GiB.
6. ~~**Custom visual sub-part nesting.**~~ Settled (§11): item paths resolve under `CustomVisuals/<guid>/resources/`.
7. ~~**Copilot / verified-answers containers.**~~ Settled (§2, "Copilot containers"): file layout, schemas and limits from Microsoft's public Copilot JSON schemas; UTF-16LE in a package and the read rules (any `/Version`; `definition.pbism` 6.0 in a project) checked on Desktop. Not seen: a Desktop-written verified answer definition (its JSON shape).
8. **PBIP `version` vs `$schema` precedence.** Which of the two Desktop relies on when a file carries both, and whether newer releases write only `$schema`, was not determined.
9. **Preview option defaults.** Whether the PBIR and TMDL preview options are on by default in a given release was not determined.
10. ~~**`.zip` / `.tmp` extensions.**~~ Settled (§11): a package named `.zip` or `.tmp` opens.
11. ~~**Desktop acceptance of metadata-only saves**~~ **Settled** (M5.1, 2026-10-13): Desktop 2.158 opens such files with the edit (added measure, format change, RLS role); the restore needed nothing beyond the `metadata.sqlitedb` rows BIcraft writes. Original question (section 8): does Desktop open a PBIX whose `/DataModel` was re-packed and re-compressed by BIcraft, with an added measure, a changed format string or an added RLS role, and show the edit? Also whether the restore needs anything beyond the `metadata.sqlitedb` rows (`DBPROPERTIES` markers, the `Model` row's times, measure `DataType`).

## Public and black-box evidence

Every packaging fact above that the code relies on points to public or black-box evidence in
[packaging-verification.md](packaging-verification.md): fact ids PK-* (container, `/Version`,
encodings, JSON parts, loader rules) and P-* (projects), each with its source (Microsoft Learn,
ECMA-376 Part 2, the vendored schemas, the structure of Desktop-saved files, and the
`tools/packaging-oracle` Desktop cases). Facts that check settled:

- §11 "Zip layout": Desktop's local headers carry a 28-byte padding field (tag `0xA220`); the
  central headers carry none. BIcraft writes none, and Desktop accepts that (PK-ZIP).
- §7 and the writer recipe: PBIR in a package does not need `/Version` ≥ 1.32. Desktop opens 1.28
  (PK-VER-PBIR). BIcraft still writes 1.32, as Desktop does.
- `DataModelSchema` and `DiagramLayout` in UTF-8 are refused, like `Settings` (PK-ENC-16).
  `Settings`, `Metadata` and `DiagramLayout` may be absent (PK-OPTIONAL). A deflated `DataModel`
  opens (PK-COMP).
- `Settings` `QueriesSettings.Version`: 1.0.3207.0 is the exact lowest accepted value (PK-QSV).
