# Legacy report layout (public sources and black-box tests)

The legacy report format is one JSON document: the `Report/Layout` part of a PBIX/PBIT (UTF-16LE,
no BOM) or `report.json` in a PBIP report folder (UTF-8). Power BI Desktop still opens it and,
when the PBIR format is on, upgrades it to a PBIR `definition/` folder on save. This note describes
the document and that upgrade in our own words. `bicraft-layout` (`crates/layout-bb`) implements it.

Each fact carries its source id from [`legacy-layout-bb.sources.md`](legacy-layout-bb.sources.md):
**SCH** the public PBIR schemas, **TU** powerbi-visuals-utils-typeutils (MIT), **LO-case** a
Desktop conversion of a generated legacy project (2026-10-15, Desktop 2.158.1304.0 by date), **LX-case** our own Desktop 2.159.883.0 runs (2026-10-28), **API** the
callers of the crate. *Inferred* marks a rule we chose without direct evidence; **Unknown** marks
what we have not observed.

## 1. Document shape

Several values are JSON documents serialized into strings ("slots"); the reader parses them, the
writer serializes them back as strings (LO-all):

| Object | Slots |
|---|---|
| root | `config`, `filters` |
| `pods[]` | `config`, `parameters` |
| `sections[]` | `config`, `filters` |
| `sections[].visualContainers[]` | `config`, `filters`, `query`, `dataTransforms` |

Root keys seen (LO-all): `id`, `resourcePackages`, `sections`, `config`, `layoutOptimization`,
`filters`, `pods`, `publicCustomVisuals`. Root `config` keys: `version`, `themeCollection`,
`activeSectionIndex`, `linguisticSchemaSyncVersion`, `objects`, `settings`,
`defaultDrillFilterOtherVisuals`, `filterSortOrder`, `bookmarks`, `modelExtensions`.

A section: `name`, `displayName`, `filters`, `ordinal`, `visualContainers`, `config`,
`displayOption`, `width`, `height`. Its `config`: `objects`, `relationships`, `visibility`, `type`,
`filterSortOrder`.

A visual container: `x`, `y`, `z`, `width`, `height`, `config`, `filters`, `tabOrder`, and in
files Desktop saved with data, `id`, `query` and `dataTransforms` (API: the engine's own fixtures).
Its `config`: `name`, `layouts` (`[{id, position}]`, id 0 the desktop layout, id 1 the phone
layout), then either `singleVisual` or `singleVisualGroup`, and `parentGroupName`, `howCreated`.

Numbers are written with a fraction (`10.0`) for positions and sizes; enumerations are numbers
(section 3).

## 2. Mapping to PBIR

### Report (`report.json`, `pages.json`, `version.json`)

| Legacy | PBIR | Source |
|---|---|---|
| sections sorted by `ordinal` (a missing ordinal sorts as 0 — *inferred*, consistent with LO-groups), ties in file order | `pages.json` `pageOrder` | LO-pages, LO-groups |
| `config.activeSectionIndex` (index in that order) | `activePageName` | LO-pages |
| `config.themeCollection.<theme>` `{name, version, type}` | `themeCollection.<theme>` `{name, reportVersionAtImport, type}`; `version` → `reportVersionAtImport` by table (§3) | LO-report, LO-groups |
| `resourcePackages[].resourcePackage` `{name, type, items[{type, path, name}], disabled}` | `resourcePackages[]` `{name, type, items[{name, path, type}]}`; `disabled: false` dropped | LO-report |
| `config.objects` | `objects`, keeping only what the schema allows (`outspacePane`: `expanded`, `visible`; `section`: `verticalAlignment`); an entry whose properties all go keeps `{"properties": {}}` | SCH, LO-pages |
| `config.settings` | `settings`, numeric codes spelled (§3); keys outside the schema dropped (`useNewFilterPaneExperience`); `hideVisualContainerHeader: false`, `filterPaneHiddenInEditMode: false` dropped (`true` kept), codes without a PBIR value dropped | SCH, LO-report, LX-x1 |
| `config.defaultDrillFilterOtherVisuals` | `settings.defaultDrillFilterOtherVisuals` | LO-report |
| `config.slowDataSourceSettings` (the query reduction options) | `slowDataSourceSettings`, the schema's five booleans; anything else unmapped | *inferred* (the PBIR name, SCH) |
| a `settings` object without `useEnhancedTooltips` | `settings.useEnhancedTooltips: false` added, whatever the theme version | LO-groups, LX-x1, LX-x3 |
| root `filters` | `filterConfig.filters` (filter mapping below) | LO-filters |
| `config.filterSortOrder` | `filterConfig.filterSortOrder` | LO-filters |
| `publicCustomVisuals` (non-empty) | `publicCustomVisuals` | LO-customvisual |
| `config.bookmarks` | one `bookmarks/<name>.bookmark.json` per bookmark, `bookmarks.json` `items` (`{name}`, or `{name, displayName, children: [names]}` for a group) | LO-bookmarks |
| `config.modelExtensions` | `reportExtensions.json` (below) | LO-report |
| `id`, `layoutOptimization`, `config.version`, `config.linguisticSchemaSyncVersion`, empty `publicCustomVisuals` | nothing | LO-all |
| — | `version.json` `version` `2.0.0` | LO-all |

**Bookmarks** keep their legacy content verbatim (`explorationState`, `options`, filters with
`expression` and numeric `howCreated` included); Desktop only adds `"visualContainers": {}` to each
exploration section that has none, and recomputes `explorationState.version` (LO-bookmarks).

**Report measures** (`config.modelExtensions`, an array of one extension): the extension's `name`;
each entity `{name, extends, measures}` → `{name, measures}`; each measure `{name, dataType,
expression, errorMessage: null, hidden: false, formulaOverride: null, formatInformation:
{formatString, format}}` → `{name, dataType, expression, formatString}` with `dataType` spelled by
the `PrimitiveType` table (§3) (LO-report, TU). `hidden: true`, `description`, `displayFolder`,
`dataCategory` are carried with their PBIR names (*inferred*, same names in SCH). Any other shape
is not converted (the engine reports it as dropped).

### Pages (`pages/<name>/page.json`)

| Legacy | PBIR | Source |
|---|---|---|
| `name`, `displayName`, `width`, `height` | same | LO-pages |
| `displayOption` | `displayOption` (§3); with `DeprecatedDynamic` Desktop writes no size | LO-pages |
| `config.objects` | `objects` (kept whole) | LO-pages |
| `config.relationships[]` `{source, target, type}` | `visualInteractions[]`, `type` spelled | LO-pages |
| `config.visibility: 1` | `visibility: HiddenInViewMode` | LO-pages |
| `config.type` | `type` | LO-pages |
| `filters` | `filterConfig.filters` | LO-pages, LO-filters |
| `config.filterSortOrder` | `filterConfig.filterSortOrder` | LO-filters |
| root `pods[]` whose `boundSection` is the page | `pageBinding` `{name, type, parameters, referenceScope, acceptsFilterContext}`: `parameters` parsed and kept, `config.acceptsFilterContext` and `referenceScope` spelled | LO-pages |

### Visuals (`pages/<p>/visuals/<v>/visual.json`, `mobile.json`)

| Legacy | PBIR | Source |
|---|---|---|
| `config.layouts` id 0 `position` (the container's own `x`, `y`, `z`, `width`, `height`, `tabOrder` when it has none) | `position` (integral numbers without fraction); when both exist and differ Desktop takes the layout's | LO-all, LX-x3 |
| `config.name` | `name` | LO-all |
| `config.layouts` id 1 `position` | `mobile.json` `position`; Desktop writes no phone formatting (`singleVisual.mobileObjects`, `mobileVcObjects` dropped) | LO-mobile |
| `singleVisual.visualType` | `visual.visualType` | LO-all |
| `singleVisual.projections` + `prototypeQuery` | `visual.query.queryState` (below) | LO-visuals, LO-report |
| `singleVisual.objects` | `visual.objects` | LO-visuals |
| `singleVisual.vcObjects` | `visual.visualContainerObjects` | LO-visuals, LO-pages |
| `singleVisual.expansionStates` | `visual.expansionStates`, `root.identityValues: null` dropped | LO-visuals |
| `syncGroup`, `drillFilterOtherVisuals`, `autoSelectVisualType` | same, in `visual` | LO-visuals |
| `isDrillDisabled` | `visual.query.isDrillDisabled` | LO-visuals |
| `display: {mode: "hidden"}` | `isHidden: true` | LO-visuals |
| `filterSortOrder` | `filterConfig.filterSortOrder` | LO-filters |
| container `filters` | `filterConfig.filters` | LO-filters |
| `cachedFilterDisplayItems` | nothing | LO-filters |
| `config.singleVisualGroup` `{displayName, groupMode}` | `visualGroup`, `groupMode` spelled | LO-groups |
| `config.parentGroupName` | `parentGroupName` | LO-groups |
| groups in a report whose `settings` lack `useStylableVisualContainerHeader: true` | no group containers; each member at its own position plus the positions of all its groups, without `parentGroupName` | LO-visuals; LX-g1, g2, g3, g6, g7, g9 (flattened); LO-groups, LX-g4, g5, g8 (kept) |
| `config.howCreated` | `howCreated` | LO-visuals |

**Query.** `prototypeQuery.From` declares aliases (`{Name, Entity, Schema?, Type: 0}`);
`Select` items are an expression (`Column`, `Measure`, `Aggregation`, `HierarchyLevel`, …) with
`Name` (the queryRef) and `NativeReferenceName`. Each projection `{queryRef, active?}` of each role
becomes `{field, queryRef, nativeQueryRef, …}`, where `field` is the select item's expression with
every `SourceRef: {Source: alias}` replaced by `{Schema?, Entity}` of the alias, `nativeQueryRef` is
`NativeReferenceName` (absent when the select item has none), and other projection keys
(`active`) are kept (LO-visuals, LO-report: report measures keep `Schema: "extension"`).
`columnProperties[queryRef]` adds `displayName` and `format` (from `formatString`) to the
projections of that queryRef; `showAllRoles` sets `showAll: true` on the role;
`prototypeQuery.OrderBy[] {Direction, Expression}` → `sortDefinition.sort[] {field, direction}`;
`hasDefaultSort: false` is dropped (LO-visuals); `hasDefaultSort: true` → `isDefaultSort: true`
(*inferred*). Role order is kept (Desktop writes roles alphabetically; the comparison ignores key
order).

**Drill roles.** In the built-in drill-down roles Desktop writes `active` on every projection: the
first one `true` when none was active, the others `false` (LO-visuals: slicer `Values`,
scatterChart `X`, treemap `Group`, pivotTable `Columns` added; stackedColumnChart `Category`,
pivotTable `Rows` completed with `false`; LO-filters, LO-mobile: slicer). The table in
`convert.rs` lists the observed roles; the `Category` role of the other charts is in it because
every legacy input already marked it (*inferred* same treatment). Roles of types not listed are
left alone (**Unknown**).

### Filters (report, page, visual)

| Legacy | PBIR | Source |
|---|---|---|
| `name`, `displayName`, `ordinal`, `type`, `filter`, `isHiddenInViewMode`, `isLockedInViewMode`, `restatement` | same | LO-filters, SCH |
| `expression` | `field` | LO-filters |
| `howCreated` | spelled (§3); 0 (`Auto`) → no `howCreated` | LO-filters |
| `objects` | kept, only `general.requireSingleSelect` / `isInvertedSelectionMode` | SCH, LO-report |
| `cachedDisplayNames` `[{id: {scopeId: {Comparison: {ComparisonKind: 0, Left: <field>, Right: <literal>}}}, displayName}]` | on the filter's single `Where` condition: `Annotations.filterExpressionMetadata` = `{expressions: [field], decomposedIdentities: {values: [[{"0": [literal]}] …], columns: [{value: field}]}, valueMap: [{"0": displayName} …]}` | LO-filters (1 value), LO-groups (2 values) |
| `isTransient: true` | the filter is not written | LO-report |

## 3. Codes

| Property | Codes (legacy → PBIR) | Source |
|---|---|---|
| section `displayOption` | 0 DeprecatedDynamic, 1 FitToPage, 2 FitToWidth, 3 ActualSize | LO-pages |
| section `config.visibility` | 1 HiddenInViewMode | LO-pages |
| section `config.type` | 1 Tooltip, 2 Drillthrough | LO-pages |
| `relationships[].type` | 1 DataFilter, 2 HighlightFilter, 3 NoFilter; a relationship of type 0 is not written | LO-pages, LX-x1 |
| pod `type` | 1 Drillthrough, 2 Tooltip | LO-pages |
| pod `referenceScope` | 1 CrossReport | LO-pages |
| pod `config.acceptsFilterContext` | 1 None | LO-pages |
| filter `howCreated` | 0 (none), 1 User, 2 Drill, 3 Include, 4 Exclude, 5 Drillthrough | LO-filters, LO-pages |
| `filterSortOrder` | 1 Ascending, 2 Descending, 3 Custom | LO-filters |
| `OrderBy[].Direction` | 1 Ascending, 2 Descending | LO-visuals |
| `groupMode` | 0 ScaleMode, 1 ScrollMode | LO-groups |
| resource package `type` | 1 RegisteredResources, 2 SharedResources | LO-report |
| resource item `type` | 100 Image, 201 CustomTheme, 202 BaseTheme | LO-report |
| theme `type` | 1 RegisteredResources, 2 SharedResources | LO-report |
| `settings.exportDataMode` | 1 AllowSummarized, 2 None; 0 not written | LO-report, LX-x1, LX-x2 |
| `settings.queryLimitOption` | 0 None, 3 SQLServerAS (the schema's order); 6 not written | LX-x1, LX-x2, LO-report |
| `settings.pagesPosition` | 1 Bottom; 0 not written | LO-report, LX-x1 |
| measure `dataType` | 0 Null … 3 Double … 13 Variant (`PrimitiveType`) | TU, LO-report (3) |
| theme `version` → `reportVersionAtImport` (visual, report, page) | 5.43 → 1.8.79, 2.0.79, 1.3.79; 5.61 → 1.8.97, 2.0.97, 1.3.97; 5.70 → 2.6.0, 3.1.0, 2.3.0; 5.80, 5.99 and 6.10 → 2.14.0, 3.4.0, 2.3.1 (Desktop 2.159; Desktop 2.158 wrote 2.13.0 for 5.99). From 5.80 on Desktop writes its own current versions | LX-x1, LO-report, LX-x2, LO-groups |

A code outside these tables is never guessed: the property stays in the legacy unmapped map
(section 4) and the engine lists it as dropped when it converts the report.

## 4. Design choices of `bicraft-layout` (not format facts)

- **Nothing dropped silently.** Every legacy property the reader does not map is kept in the
  `extra` map of its report, page or visual under `legacy:unmapped`, keyed by its path in the legacy
  object (`config.modelExtensions`, `filters[0]`, `config.singleVisual.columnProperties["Sales.Total"]`).
  `read_layout_with_summary` lists them; the engine names them when it converts to PBIR.
- Legacy-only state with a PBIR document of its own rides in `legacy:mobileState` (per visual) and
  `legacy:bookmarks` (the bookmark list); the engine turns them into `mobile.json` and
  `bookmarks.json`.
- Desktop's conversion-only rules (group flattening, drill `active` markers, `useEnhancedTooltips`, bookmark
  `visualContainers`, expansion `identityValues: null`) run in `strip_legacy`, so a legacy report
  read and written back stays as it was.
- **Writing back.** `write_layout` converts the edited report back object by object (the inverse
  of section 2, aliases named after the entity's first letter, `s`, `s1`, … as in LO-report), puts
  each unmapped property back at its path, and keeps the original text of every top-level key,
  section and container whose decoded content is unchanged. An unchanged report returns the
  original text. Edits with no legacy form are refused, each named: theme and resource package
  changes, new or changed page bindings, PBIR documents other than a visual's `mobile.json`, and
  any PBIR property the mapping above does not cover.

## 5. Unknowns

- Codes not observed: `queryLimitOption` 1, 2, 4, 5, page `type` 0, pod codes 0, item types
  other than 100/201/202. What `exportDataMode` 0 and `pagesPosition` 0 mean (Desktop writes
  nothing for them, LX-x1, LX-x2; they stay unmapped). Page `visibility` 0 is not written (LX-x1).
- Theme versions not in the table: the reader uses the closest lower known version (*inferred*).
  The table mixes two Desktop versions for the newest entries.
- The drill roles of visual types not in the table, and of custom visuals.
