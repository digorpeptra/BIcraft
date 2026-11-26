# PBIR: the enhanced report format (`definition/`)

Implemented by `crates/pbir` (`bicraft_pbir::{read_documents, write_documents, read_dir,
write_dir, validate_documents, check_names}`), on top of the lossless report model in
`crates/report`. The loader rules (PBIR read from `definition.pbir` version 4.0, 3.0 also opening; UTF-8
without BOM; `$schema` major matching) are in [pbix-package.md](pbix-package.md) and
[pbip.md](pbip.md).

Sources: Microsoft's public PBIR JSON schemas (MIT), black-box tests against Power BI Desktop
2.157 and 2.158, and the packaging facts of pbix-package.md; detailed provenance is kept privately
by the maintainers.

## Sources

| Source | Used for |
|---|---|
| Microsoft's public JSON schemas, <https://github.com/microsoft/json-schemas>, `fabric/item/report/definition/`, commit `49f4a76c` (2026-10-12) | Property names, required properties, schema versions, key order of the contracts. **Vendored** under `crates/pbir/schemas/` (see Licenses). |
| Microsoft's `skills-for-fabric` notes (MIT), <https://github.com/microsoft/skills-for-fabric>, `skills/powerbi-report-cli/references/authoring/filters.md` and `slicers-part-02.md`, commit `ec8d7ec2` (2026-09-24) | The relative date bounds Desktop writes for "last N, today included" and that Top N filters are visual-level only (facts; no text copied) |
| `pbix-package.md` §2 and §6 (Desktop 2.158.1304.0) | File set of `definition/`, UTF-8 without BOM, `$schema` major-version rule |
| A report saved by Power BI Desktop (2.157/2.158 era) as PBIP and as PBIX on the development machine (held locally: not committed, not quoted) | Byte layout, key order, schema versions Desktop writes today. Only structure was looked at (key names, versions, whitespace). |

## Layout

Paths are relative to the report root (`<Name>.Report/` in a PBIP, `Report/` in a PBIX).

| Path | Model | Notes |
|---|---|---|
| `definition/version.json` | `report.version_extra` (incl. `version`) | `$schema` → `report.schemas["version"]` |
| `definition/report.json` | `theme_collection`, `filter_config`; the rest in `report.extra` | `resourcePackages`, `settings`, `objects`… stay in `extra` |
| `definition/pages/pages.json` | `pages` order (`pageOrder`), `active_page`; rest in `pages_extra` | |
| `definition/pages/<page>/page.json` | `Page` (`name`, `displayName`, `displayOption`, `height`, `width`, `visibility`, `filterConfig`); rest in `Page.extra` | folder = `name` |
| `definition/pages/<page>/visuals/<visual>/visual.json` | `Visual` (`name`, `position`, `visual.visualType`, `parentGroupName`, `isHidden`, `filterConfig`, fields from the query); `visual` object rest in `visual_extra`, top level rest in `extra` | folder = `name` |
| `definition/bookmarks/<name>.bookmark.json` | `report.bookmarks` (raw documents) | order from `bookmarks.json` |
| `definition/bookmarks/bookmarks.json`, `definition/reportExtensions.json`, `…/visuals/<v>/mobile.json`, any other file under `definition/` | `report.resources`, raw bytes, under their path | the model has no typed place for them (see "Model notes") |
| `StaticResources/**`, `CustomVisuals/**` | `report.resources`, raw bytes | |
| `definition.pbir`, `.platform`, `.pbi/**`, everything else | ignored | never written or deleted |

Pages are ordered by `pageOrder`, then any page missing from it by name. Visuals are ordered by
folder name. Bookmarks follow `bookmarks.json` (group children flattened), then the rest by name.

## Versions

`$schema` URLs are `https://developer.microsoft.com/json-schemas/fabric/item/report/definition/<key>/<x.y.z>/schema.json`.

The reader records, per document kind, the **highest** version seen in `report.schemas`
(`version`, `report`, `pages`, `page`, `visual`, `bookmark`, `bookmarks`, `mobile`,
`reportExtensions`). A page or visual whose `$schema` differs from that keeps its own `$schema`
in `extra`, so mixed-version reports round-trip exactly (Desktop leaves untouched visuals at the
version they were written with: the observed report mixes two `visualContainer` versions).

Objects created without a schema get `report.schemas[kind]`, else these defaults:

| File | Schema key | Default | Why |
|---|---|---|---|
| `version.json` | `versionMetadata` | 1.0.0 (`version` `2.0.0`) | what Desktop writes |
| `report.json` | `report` | 3.3.0 | latest, what Desktop writes |
| `pages.json` | `pagesMetadata` | 1.1.0 | latest, what Desktop writes |
| `page.json` | `page` | 2.1.0 | latest, what Desktop writes |
| `visual.json` | `visualContainer` | 2.12.0 | Desktop 2.158 writes **2.13.0**, which is not published yet (latest public: 2.12.0, Aug 2026). We write the newest public version so every file we emit validates against a published schema; Desktop accepts any minor of a major it knows. |
| `*.bookmark.json` | `bookmark` | 2.1.0 | latest public; no Desktop sample |
| `bookmarks.json` | `bookmarksMetadata` | 1.0.0 | only version |

## Byte layout

| | Desktop, PBIP | Desktop, PBIX `Report/definition/*` | BIcraft writer |
|---|---|---|---|
| Encoding | UTF-8, no BOM | UTF-8, no BOM | UTF-8, no BOM |
| Indentation | 2 spaces, `": "` | none (minified) | 2 spaces, `": "` (minified inside a PBIX/PBIT, see `pbix-package.md` §9) |
| Line endings | CRLF | — | LF (byte-stable on Windows and Linux) |
| Trailing newline | no | no | no |
| Empty array / object | `[]` / `{}` | `[]` / `{}` | `[]` / `{}` |

Numbers: typed numbers (`height`, `width`, `x`, `y`, `z`) are written without a fraction when
integral (`720`), else as the shortest round-trip decimal. Unmodelled values keep their JSON
number kind; a decimal is re-printed as its shortest round-trip form, which may differ in digits
(never in value) from a 17-digit .NET rendering.

## Key order

Unmodelled keys keep their source order. Typed keys are re-inserted by this rule (`json::merge`):
a typed key that also exists in `extra` takes that slot; otherwise it goes right before the first
`extra` key that comes later in the Desktop order below, or at the end. `$schema` is always first.
Because the order lists match what Desktop writes, a read → write of a Desktop file keeps every
key in place (checked on the real report: all 128 JSON documents re-serialize identically).

| Document | Desktop order (typed keys in bold) |
|---|---|
| `report.json` | `$schema`, **themeCollection**, **filterConfig**, objects, reportSource, publicCustomVisuals, resourcePackages, organizationCustomVisuals, annotations, dataSourceVariables, settings, slowDataSourceSettings |
| `pages.json` | `$schema`, **pageOrder**, **activePageName**, landingPageName |
| `version.json` | `$schema`, version |
| `page.json` | `$schema`, **name**, **displayName**, **displayOption**, **height**, **width**, **filterConfig**, pageBinding, objects, type, **visibility**, visualInteractions, autoPageGenerationConfig, annotations, howCreated |
| `visual.json` | `$schema`, **name**, **position**, **visual**, visualGroup, **parentGroupName**, **isHidden**, annotations, **filterConfig**, howCreated (Desktop puts `filterConfig` after `annotations`, unlike the schema's property order) |
| `position` | x, y, z, height, width, tabOrder, angle |
| `visual` | **visualType**, autoSelectVisualType, query, expansionStates, objects, visualContainerObjects, syncGroup, drillFilterOtherVisuals |

## Reader rules

- Paths may use `\` or `/`; each component must be a valid file name (no `..`, separators,
  `: * ? " < > |`, control characters, trailing dot/space, Windows device names). Duplicate paths
  (ignoring case) are an error.
- Documents must be UTF-8 JSON objects without BOM (serde_json's 128-level nesting cap applies).
- Typed properties of the wrong JSON type are errors (`name: 3`, `width: "x"`, `visual: []`).
  `isHidden` is typed only when `true` (a literal `false` stays in `extra`). Position members
  that are not numbers, and unknown position keys (e.g. `angle`), stay in `extra["position"]`
  and are merged back on write.
- Duplicate page / visual / bookmark names, and visuals whose page has no `page.json`, are
  errors (they cannot be represented losslessly).

## Fields

`Visual.fields` comes from `visual.query.queryState.<role>.projections[].field`, role by role:

| PBIR field | `FieldRef` |
|---|---|
| `{"Column": {"Expression": {"SourceRef": {"Entity": T}}, "Property": C}}` | `Column { table: T, column: C }` |
| `{"Measure": …same…}` | `Measure` |
| `{"Aggregation": {"Expression": {"Column": …}, "Function": n}}` | `Aggregation { function }` with `n` → `Sum` 0, `Avg` 1, `Count` 2 (distinct count), `Min` 3, `Max` 4, `CountNonNull` 5, `Median` 6, `StandardDeviation` 7, `Variance` 8 (codes from the public `semanticQuery` schema; unknown codes become their number as text) |
| anything else (extra keys, `Schema`, hierarchies, expressions…) | `Other { raw }` |

The raw `query` always stays in `visual_extra`. On write, if `fields` still equals what the raw
query yields, the query is written untouched. Otherwise `queryState` is rebuilt from `fields`
(roles in order of first use): an unchanged binding reuses its original projection (keeping
`queryRef`, `nativeQueryRef`, `active`, `displayName`…), its role object keeps its other keys
(`showAll`, `fieldParameters`), and other query keys (`sortDefinition`, `options`) are kept. New
projections get `queryRef` `Table.Column` / `Table.Measure` / `Sum(Table.Column)` and
`nativeQueryRef` `Column` / `Measure` / `Sum of Column`. A field bound again (another role, e.g.
a scatter chart's `Y` and `Size` both `Sum(Sales.Qty)`, or twice in one role) reuses the
`queryRef` of its first projection, so the query selects it once; only different fields with the
same default name get a numeric suffix. A suffixed copy of the same field (`Sum(Sales.Qty)1`, what
BIcraft wrote before) makes Desktop 2.158 fail the visual's query. Aggregation names are matched
case-insensitively and also accept `Average`, `DistinctCount`, `StdDev`, `Var`. No fields left:
the query is dropped when it held only `queryState`, else `queryState` becomes `{}`.

**Per-visual names and hierarchies (M11.13).** A projection's `displayName` (a non-empty string)
is the field's name in that visual ("Rename for this visual"): `FieldBinding::display_name`, read
and written on the projection (set, replaced, or removed when the name is restored; an empty
string is no rename and is kept as written). The legacy layout carries it as
`singleVisual.columnProperties[queryRef].displayName` (legacy-layout-bb.md). A whole hierarchy in a
role is one projection per level whose `field` is
`{"HierarchyLevel": {"Expression": {"Hierarchy": {"Expression": {"SourceRef": {"Entity": T}},
"Hierarchy": H}}, "Level": L}}` with `queryRef` `T.H.L` and `nativeQueryRef` `H L`; in a drill role
the levels carry `active` (the top one `true`, the others `false`). Source: the PBIR Desktop
2.159 wrote for the layout-oracle fixtures (`crates/engine/tests/fixtures/layout-oracle`, a
matrix with a two-level hierarchy on its rows).

**Show value as.** A value field shown as a percentage of a total is one projection whose
`field` divides the field by itself evaluated in a reduced scope:
`{"Arithmetic": {"Left": F, "Right": {"ScopedEval": {"Expression": F, "Scope": [...]}},
"Operator": 3}}`, where `F` is the plain field (a measure or an aggregation). The shapes are
the public `semanticQuery` schema's `QueryArithmeticExpression` (operator 3 is Divide) and
`QueryScopedEvalExpression` ("evaluates an expression in a specified scope"; the scope "can
only be Columns"); `crates/pbir/schemas/semanticQuery/1.4.0/schema.json`. BIcraft reads and
writes three scopes as `FieldBinding::show_as`: empty (percent of grand total), the plain
columns of the matrix's `Columns` role in order (percent of column total) and those of its
`Rows` role (percent of row total); any other `Arithmetic` stays an `Other` field, written as
read. A reused projection's scope is rewritten when the role's columns change. A new
projection is named `Divide(Sum(T.C), ScopedEval(Sum(T.C), [T.Col, …]))` with
`nativeQueryRef` `%GT Sum of C` (`%CT`, `%RT`), the prefixes Desktop shows; the field and its
percentage are two select items. Power BI Desktop 2.159.1231.0 (Store, 2026-11-23, black box:
a PBIP BIcraft wrote) reads both forms BIcraft writes as its own quick calculations: a matrix's
percent of column total shows each column summing to 100 %, a chart's percent of grand total
the shares of the total, and the field well names them with Desktop's localized prefix ("%TG
Total" in French), naming the quick calculation by its kind. *Not yet checked
against a file Desktop saved itself* (the operand order and `queryRef` it forms, the scope it
writes for a column total); reading accepts only the shape above, so a different Desktop form
stays raw and round-trips unchanged. The legacy layout carries the same expression in
`prototypeQuery.Select` (the `SourceRef` aliases resolved as for any field).

**Show items with no data.** The role object's `showAll: true` (`queryState.<role>.showAll`, the
public `visualConfiguration` schema's projection state; the legacy layout's
`singleVisual.showAllRoles`, legacy-layout-bb.md). It is a role's, not a field's: Desktop's
menu item is on each field, and BIcraft's field menu toggles the field's role.
`visual.show_items_no_data` sets or removes it. Desktop 2.159.1231.0 (2026-11-23) lists the
empty categories of a table's role with `showAll` BIcraft wrote, but in a chart whose value is
shown as a percentage of the grand total it leaves them out and flags the visual with a warning
icon, whose details say the two options are not compatible and ask to remove the quick
calculation or untick the option. BIcraft's preview does the same and both commands warn
(`warnings`) when they make that pair; the percentages of a column or row total with `showAll`
were not tried.

**Visual calculations.** A projection whose field is `{"NativeVisualCalculation": {"Language":
"dax", "Expression": E, "Name": N}}` (the public `semanticQuery` schema's
`QueryNativeVisualCalc`), with `queryRef` and `nativeQueryRef` `N` as BIcraft writes them; a
projection's `"hidden": true` (visualContainer schema) keeps a field in the query but off the
visual. Read as an `Other` field and written as read; the legacy layout carries both in
`prototypeQuery.Select` and its projections. Commands and evaluation:
[visual-calculations.md](visual-calculations.md).

**Filter cards.** `filterConfig.filters[]` of the report, a page or a visual: `{name, field, type,
filter?, howCreated?, isHiddenInViewMode?, isLockedInViewMode?}`. A card without `filter` has no
condition. `page.filter` / `report.filter` / `visual.filter` without a condition add one with
`type` `Categorical` (a column) or `Advanced` (a measure or an aggregation) and `howCreated`
`User` (*inferred* from the cards Desktop writes for user filters; not checked against a card
Desktop itself added from its Filters pane). `isLockedInViewMode: true` keeps a reader from
changing the card's condition, `isHiddenInViewMode: true` leaves the card out of a reader's
Filters pane (public `filterConfiguration` schema descriptions); the condition still applies.
BIcraft writes the flag `true` or `false` when it is toggled and leaves it out otherwise.

**Top N cards** exist on visuals only: Desktop's Filters pane offers Top N for visual-level
filters and not on page or report filters (Microsoft's `skills-for-fabric` notes, below;
consistent with the audit of the pane). `page.filter` and `report.filter` refuse `top`.

**Relative date and time filters.** A card with `type` `RelativeDate` (days, weeks, months,
years) or `RelativeTime` (minutes, hours) holds one `Between` on the column, its bounds built
from `Now`, `DateAdd {Expression, Amount, TimeUnit}` and `DateSpan {Expression, TimeUnit}`
(public `semanticQuery` schema; `TimeUnit` 0 day, 1 week, 2 month, 3 year, 6 minute, 7 hour). The
card keeps no other settings: the pane reads them back from the bounds. With `U` the period's
unit, `N` the count, `add(e, a, u)` and `span(e, u)`:

| Condition | LowerBound | UpperBound | Source |
|---|---|---|---|
| last N periods, today included | `span(add(add(Now, 1, 0), -N, U), 0)` | `span(Now, 0)` | public (last 10 years) |
| last N periods, today excluded | `span(add(Now, -N, U), 0)` | `span(add(Now, -1, 0), 0)` | inferred |
| next N periods, today included | `span(Now, 0)` | `span(add(add(Now, -1, 0), N, U), 0)` | inferred |
| next N periods, today excluded | `span(add(Now, 1, 0), 0)` | `span(add(Now, N, U), 0)` | inferred |
| last N calendar periods | `span(add(Now, -N, U), U)` | `span(add(Now, -1, U), U)`; today included: `span(Now, 0)` | inferred |
| next N calendar periods | `span(add(Now, 1, U), U)`; today included: `span(Now, 0)` | `span(add(Now, N, U), U)` | inferred |
| this period (day, week, month, year, minute, hour) | `span(Now, U)` | `span(Now, U)` | inferred |
| last N minutes or hours | `add(Now, -N, U)` | `Now` | inferred |
| next N minutes or hours | `Now` | `add(Now, N, U)` | inferred |

"Public" is the shape Microsoft's notes give for Desktop (both `DateSpan` at day, the period on
the inner `DateAdd`); "inferred" rows are BIcraft's, built from the same parts so that the
evaluator's reading of `Between` (a `DateSpan` bound covers its whole span: `>=` its first
instant, `<` the instant after it) gives the days the slicer's relative date shows
(`filters::relative_days`); they are not checked against files Desktop wrote. Weeks start on
Sunday. A relative time filter compares to the instant: the preview reads the clock to the
minute (`view.set_as_of` pins it). Sources: the public `filterConfiguration` 1.3.0 and
`semanticQuery` 1.4.0 schemas; `microsoft/skills-for-fabric` (MIT),
`skills/powerbi-report-cli/references/authoring/filters.md` and `slicers-part-02.md`, commit
`ec8d7ec2` (2026-09-24), read for these facts only.

## Writer rules

- `write_documents` always emits `version.json`, `report.json` (`themeCollection` `{}` when
  unset) and `pages.json` (`pageOrder` from the page list); `page.json` gets `displayOption`
  `FitToPage` when unset (the schema requires it).
- `bookmarks.json` is reconciled with the bookmark list: missing bookmarks are removed from
  `items` (empty groups dropped), new ones appended as `{"name": …}`; its original bytes are kept
  when nothing changed. It is created only when there are bookmarks.
- Kept documents under `definition/pages/<p>/[visuals/<v>/]` are dropped with their page/visual.
- Objects whose names are not valid file names are skipped by `write_documents`;
  `check_names` (and `write_dir`) reject them, as well as name collisions ignoring case and
  resource paths outside `StaticResources/`, `CustomVisuals/`, `definition/`.
- `write_dir` writes only files whose bytes changed, then deletes stale `page.json`,
  `visual.json`, `mobile.json`, `*.bookmark.json` and `bookmarks.json` under `definition/`, and
  `StaticResources/RegisteredResources` files the *previous* `report.json` registered but the
  report no longer has; then removes the folders this left empty (never `definition/pages` or
  `StaticResources/<package>`). It never touches `definition.pbir`, `.platform`, `.pbi/` or
  other files.

## Custom themes

A custom theme is three things that must agree (public `report` schema `ThemeCollection` /
`ResourcePackage`; checked against Power BI Desktop 2.158, Store edition):
`StaticResources/RegisteredResources/<file>.json` (the theme JSON), an item `{name: <file>, path:
<file>, type: "CustomTheme"}` in the `RegisteredResources` package of `resourcePackages`, and
`themeCollection.customTheme = {name: <file>, reportVersionAtImport, type:
"RegisteredResources"}`. `reportVersionAtImport` is `{visual, report, page}` (report schema 3.x) or a
version string (1.x/2.x). Desktop opens a report with a custom theme and no `baseTheme` (it uses its
default base theme). `theme_edit.rs` writes exactly this; `write_dir` deletes the file of a replaced
theme because the previous `report.json` registered it. Desktop's built-in visual styles take
precedence over the theme's `textClasses` for visual titles and axis labels (observed: a custom
`textClasses.title`/`label` alone left them unchanged, while `dataColors` and `visualStyles` applied).

BIcraft's built-in themes (`theme.set {builtin}`, View › Themes, 2026-11-19) are installed
exactly as a custom theme JSON: the file holds a `CustomTheme` named after the theme (`BIcraft
Dusk`), not a Desktop base theme, so Desktop and the service see an ordinary custom theme. The
documents (`crates/visuals/themes/`) are BIcraft's own, written to the public report theme
schema 2.157 (`name`, `dataColors`, `foreground`, `foregroundNeutralSecondary`, `background`,
`tableAccent`, the sentiment and divergent colours, `hyperlink`, four `textClasses`; the dark
ones a `visualStyles.page.*` background and outspace and a `visualStyles.*.*` background), and
validate against it without warnings. Not opened in Desktop yet.

## Chart specifics, page types, interactions, mobile layout

Sources: the public `page` 2.1.0, `visualConfiguration` 2.7.0, `visualContainerMobileState` and
`semanticQuery` schemas; the public report theme schema (object and property names per
visual); and a report written by BIcraft, opened and re-saved by Desktop 2.158.1304.0 (what it
keeps or rewrites); detailed provenance is kept privately by the maintainers. Implemented in
`crates/engine/src/visual_extras.rs`.

| Feature | Where | Shape |
|---|---|---|
| Axis range / scale | `objects.categoryAxis` (x), `valueAxis` (y) | `start`, `end`, `logAxisScale`, `gridlineShow/Color/Thickness/Style`, `labelDisplayUnits`, `labelPrecision`, `showAxisTitle`, `titleText`, `invertAxis`, x `axisType` `'Categorical'`/`'Scalar'` |
| Secondary axis | combo charts: `valueAxis.sec*` (`secShow`, `secStart`…); line/area charts: `y2Axis` (`show`, `sec*`) | no gridline / invert properties |
| Constant line | `objects.y1AxisReferenceLine` / `xAxisReferenceLine` | entry per line, `selector: {id: "1"}` (ids: max + 1 per object); `show`, `displayName`, `value` (number), `lineColor`, `style`, `width`, `position`, `dataLabelShow`… |
| Min / max / average / median / percentile | `objects.referenceLine` | `selector: {metadata: <series queryRef>, id}` (Desktop's key order); `value` = `{expr: {Aggregation: {Expression: {SelectRef: {ExpressionName: <queryRef>}}, Function: 3/4/1/6}}}` or `{expr: {Percentile: {Expression: …, K: 0.75}}}` (Desktop drops `Exclusive: false`) |
| Trend line | `objects.trend` | one entry, no selector |
| Forecast (**unchecked**) | `objects.forecast` (line charts, continuous x axis: Microsoft Learn) | `selector: {metadata: <series queryRef>, id}`; `transform: {algorithm: "Forecast", parameters: [{Literal: {Value: "7D"}, Name: "Unit"}, ForecastLength, IgnoreLast, ConfidenceLevel, MaxSeasonality?]}`, every parameter a double. No public schema describes `transform`: this is BIcraft's own shape, not compared with a file Desktop saved |
| Error bars (**unchecked**) | `objects.error` (bar, column, clustered and 100% charts, line chart, both combos, ribbon chart: the theme schema's types) | one entry per series, `selector: {metadata: <series queryRef>}` (no `id`); `enabled`, `errorRange`, then the theme schema's `bar*`, `marker*`, `label*`, `tooltip*` and, on line charts only, `shade*` properties. `errorRange` is typed `Other` and no public schema describes its value: BIcraft writes `{kind: "ErrorRange", explicit: {isRelative: <bool>, upperBound: {expr: <measure or aggregation>}, lowerBound: {expr: …}}}`, its own choice, not checked against a Desktop-saved file; relative bounds are read as series value + bound |
| Ratio line (**unchecked**) | `objects.ratioLine` (scatter chart) | one entry, no selector; `show`, `displayName`, `lineColor`, `style`, `width`, `transparency` (theme schema). BIcraft draws it from the origin through (Σx, Σy), its own reading |
| Symmetry shading (**unchecked**) | `objects.plotAreaShading` (scatter chart) | one entry, no selector; `show`, `displayName`, `upperShadingColor`, `lowerShadingColor`, `transparency` (theme schema); drawn above and below the y = x line |
| Anomaly detection (**unchecked**) | `objects.anomalyDetection` (line chart: the theme schema's only type) | one entry, `selector: {metadata: <series queryRef>}` (no `id`); `show`, `displayName`, `markerShow`, `markerShape` (`circle`, `square`, `diamond`, `triangle`, `droplet`), `markerShapeSize`, `markerColor`, `markerTransparency`, `confidenceBandShow`, `confidenceBandStyle` (`fill`, `line`, `none`), `confidenceBandColor`, `transparency` (theme schema). No public source names the sensitivity's property: BIcraft writes `transform: {algorithm: "AnomalyDetection", parameters: [{Literal: {Value: "70D"}, Name: "Sensitivity"}]}` (the forecast's shape), its own choice. Microsoft Learn ("Anomaly detection"): line charts with a time series axis, no legend, one value and no secondary values, at least four points, no forecast or min, max, average, median and percentile lines beside it; `visual.analytics` refuses those combinations. The expected range and the anomalies are BIcraft's own estimate (a centred moving median and the residuals' median absolute deviation, `crates/render/src/anomaly.rs`), not the SR-CNN model Learn names; the explanations (Explain by) are not modelled |
| Small multiples | role `Rows` in `query.queryState` (Desktop orders it before `Y`); `objects.smallMultiplesLayout` (`layoutType 'custom'`, `rowCount`, `columnCount`); `objects.subheader` | |
| Report-page tooltip | `visualContainerObjects.visualTooltip` | `show: true`, `type: 'Canvas'`, `section: '<page name>'` |
| Tooltip / drillthrough page | `page.type` `Tooltip`/`Drillthrough`; `page.pageBinding {name, type, parameters, referenceScope?, acceptsFilterContext?}` | Desktop writes `parameters: []` when empty; `acceptsFilterContext: "None"` = keep-all-filters off; `referenceScope: "CrossReport"`; drillthrough fields = page filters with `howCreated: "Drillthrough"` plus parameters `{name, boundFilter, asAggregation?, fieldExpr}` |
| Back button | `actionButton`: `objects.icon [{properties: {shapeType: 'back'}, selector: {id: "default"}}]`, `visualContainerObjects.visualLink {show: true, type: 'Back'}`, `drillFilterOtherVisuals: true`, `howCreated: "InsertVisualButton"`; Desktop inserts it at 0,0, 100 x 40 | without the `default` selector the icon is not drawn |
| Interactions | `page.visualInteractions [{source, target, type}]` | `DataFilter`, `HighlightFilter`, `NoFilter`; `Default` is never written |
| Mobile layout | `pages/<p>/visuals/<v>/mobile.json` | `{$schema, objects?, visualContainerObjects?, position: {x, y, z, height, width, tabOrder?}}`; Desktop 2.158 rewrites it as `visualContainerMobileState` 2.8.0 (unpublished) and adds `tabOrder`; we write 2.7.0. A visual placed anew gets the highest `z` of the page's mobile layout plus one (Microsoft Learn, "Create an initial layout": each dragged visual is its own layer, on top). `objects` and `visualContainerObjects` are the public schema's "mobile specific formatting changes" (`visual.mobile_format`); BIcraft reads them as overrides of the visual's own, per object, an entry with the same selector replacing its properties one by one and any other entry added (Learn, "Optimize visual formatting for mobile layout": a changed setting is disconnected from the desktop layout). Power BI Desktop 2.159.1231.0 (2026-11-23) saves a `mobile.json` BIcraft wrote back with the same content (CRLF line endings) and draws each visual on its phone canvas (324 CSS pixels wide) at the file's `position`, 1:1; a title set in `visualContainerObjects` shows on the phone only; its canvas scrolls to 3600 high and its grid cells are 18 wide by 20 high, which BIcraft's phone canvas and `visual.mobile {snap: true}` take since 2026-11-24 (docs/gaps.md G72). In a legacy layout they are the container's `singleVisual.mobileObjects` / `mobileVcObjects` beside `layouts` id 1 |

The theme schema has no axis label-angle or tick-mark property. The forecast, error-bar, ratio-line,
symmetry-shading and anomaly-detection rows come from the public report theme schema 2.157
(`crates/visuals/schemas/`) and, for anomaly detection's limits, Microsoft Learn; none was compared with a file Desktop saved. Lines on every object
also take `transparency` where the theme schema lists it (not on `forecast`).

### Conditional formats of a table's values (the dialog)

Sources: Microsoft Learn "Apply conditional table formatting in Power BI" (the dialogs' controls:
Gradient, Rules with an "and" condition, Field value, empty values, Apply to, data bars, icons,
web URL); the public report theme and semantic query schemas (`FillRule`, `Conditional`, `And`,
`Comparison`); the gradient's shape as Desktop 2.159.883.0 wrote it in the layout oracle's
corpus (`linearGradient2` with `nullColoringStrategy`). Implemented in
`crates/engine/src/visual_conditional.rs` (written and read), `preview_format.rs` (evaluated) and
`crates/ui/src/conditional_dialog.rs` (the dialog).

| Dialog control | Written as |
|---|---|
| Minimum / Center / Maximum: lowest, middle, highest value | the stop without `value` (`linearGradient2` / `linearGradient3` `min`, `mid`, `max` `{color}`) |
| Minimum / Center / Maximum: Number | the stop's `value` literal beside its `color` |
| How should we format empty values? | `nullColoringStrategy.strategy` `'asZero'`, `'noColor'` or `'specificColor'` with `color`; Don't format writes nothing |
| Apply to | the selector's `dataViewWildcard.matchingOption` 1 (values), 0 (values and totals), 2 (totals) |
| If value `op` x and `op2` y | one `Conditional` case whose condition is `And` of two `Comparison`s (kinds 0 `=`, 1 `>`, 2 `>=`, 3 `<`, 4 `<=`); "is blank" compares with a `null` literal |
| Icons: layout, alignment | `values.icon` `{kind: "Icon", layout: 'Before' / 'After' / 'IconOnly', verticalAlignment: 'Top' / 'Middle' / 'Bottom', value: {expr: Conditional}}` |
| Data bars | `columnFormatting.dataBars` with `selector.metadata` only: `positiveColor`, `negativeColor`, `axisColor`, `reverseDirection`, `hideText` (Show bar only), `minValue` / `maxValue` when a number |
| Web URL | `values.webURL` `{expr: <field>}` |

Rules' **Percent** bounds (a percent of the range from the lowest to the highest value,
Microsoft Learn) are written since 2026-11-25 (G54) as BIcraft's own shape built from the
semantic query schema's `Arithmetic` (operators 0 add, 1 subtract, 2 multiply) and
`Aggregation` (functions 3 min, 4 max) of the rule's field: the bound `p` is
`Min + (Max - Min) * (p / 100)`, the literal `p / 100` a `D` number. The decoder reads exactly
that shape back as `percent: true`; any other expression stays untouched in the file. No public
source gives Desktop's own shape, so a file Desktop formats may differ (black-box oracle to
run). The gradient's Percent and Percentile stops are not written (the dialog offers Number
only for them). The data bars', icons', percent and two-condition rules' shapes **were not
compared with a file Desktop formatted** through these dialogs.

### Conditional formats of the container and of labels

Sources: Microsoft Learn "Conditional formatting in Power BI visuals" (titles and subtitles
expression-based, cards' labels) and "Create expression-based titles and subtitles" (the
Title and Subtitle text's fx, a text field, Field value only); the public visual container
schema (`visualContainerObjects` with `title`, `subTitle`, `background`, `border`); the report
theme schema 2.157 (`labels.color`, `categoryLabels.color`). Implemented in
`crates/engine/src/visual_conditional.rs`; **not compared with a file Desktop saved**.

| Format | Where | Shape |
|---|---|---|
| Title / subtitle from a field | `visualContainerObjects.title` / `subTitle`, the entry without selector, `properties.text` | `{expr: <Measure or Aggregation>}` (a column through `Min`) |
| Title, subtitle colour; title background; visual background; border colour | `visualContainerObjects.title.fontColor`, `subTitle.fontColor`, `title.background`, `background.color`, `border.color`, the entry without selector | the rule as on any colour: `{solid: {color: {expr: FillRule / Conditional / field}}}`, beside the entry's plain properties |
| Data label colour (charts), category label colour (cards) | `objects.labels.color`, `objects.categoryLabels.color` | the entry `selector: {data: [{dataViewWildcard: {matchingOption: 1}}]}`, as `dataPoint.fill` |

| Axis label and title colours, legend colour (G54) | `objects.categoryAxis.labelColor` / `titleColor`, `valueAxis.labelColor` / `titleColor`, `legend.labelColor` | the entry without selector: one value for the whole visual |
| Gauge target, target label and callout colours; KPI indicator, goal, distance and status colours (G54) | `objects.dataPoint.target`, `target.color`, `calloutValue.color`; `indicator.fontColor`, `goals.goalFontColor` / `distanceFontColor`, `status.goodColor` / `neutralColor` / `badColor` | the entry without selector (report theme schema 2.159 names) |

A container format is one value for the whole visual: it has no selector, so no series and no
"Apply to". The whole-visual object formats of the last two rows (`visual.conditional` targets
`category_axis_color`, `category_title_color`, `value_axis_color`, `value_title_color`,
`legend_color`, `target_color`, `target_label_color`, `callout_color`, `indicator_color`,
`goal_color`, `distance_color`, `good_color`, `neutral_color`, `bad_color`, or the
`{object, property}` of one of them) are BIcraft's choice of selector (none); they follow the
schemas and were **not compared with a file Desktop saved**. The viewer evaluates its field in the visual's filter context without its grouping
(a gradient whose stops have no value gives its lowest colour).

## Text boxes, images, buttons and page format

Sources: the public report theme schema 2.157 (`crates/visuals/schemas/`, MIT: the
`paragraphs` definition, `visualLink`, `icon.shapeType`, the page objects `background`,
`outspace`, `displayArea` and `pageSize`); a text box written by Power BI Desktop 2.158 in the
layout oracle's corpus (`crates/engine/tests/fixtures/layout-oracle/visuals/desktop/`); the
report schema's `ResourcePackage` items. Implemented in `crates/engine/src/rich.rs` (ROADMAP
M11.20).

| Object | Where | Shape |
|---|---|---|
| Text box text | `visual.objects.general[0].properties.paragraphs` | `[{textRuns: [{value: "text", textStyle?: {fontFamily, fontSize: "14pt", fontWeight: "bold", fontStyle: "italic", textDecoration: "underline", color: "#118DFF"}, url?}], horizontalTextAlignment?: "left"/"center"/"right"}]`; an empty paragraph is one run with `value: ""` |
| Dynamic value ("+ Value") | the run's `value` is `{propertyIdentifier: {objectName: "values", propertyName: "expr"}, selector: {id}}`; `objects.values[{properties: {expr: {expr: <Measure / Aggregation>}}, selector: {id}}]` | BIcraft names the ids `value1`, `value2`… in run order (any string is read) |
| List paragraph | `listType: "bullet"` / `"number"` on the paragraph | **BIcraft's choice, not seen in a Desktop file yet** (the theme schema's `paragraphs` lists only `horizontalTextAlignment` and `textRuns`): Desktop is expected to ignore the key; docs/gaps.md G52 |
| Image file | `StaticResources/RegisteredResources/<name><16 digits>.<ext>` and an item `{name, path, type: "Image"}` of the `RegisteredResources` package in `report.json` `resourcePackages` (legacy layout: item type 100) | the digits come from the bytes (the same image is stored once), as for custom themes |
| Image visual | `objects.general[0].properties.imageUrl = {expr: {ResourcePackageItem: {PackageName: "RegisteredResources", PackageType: 1, ItemName}}}`; `objects.imageScaling[0].properties.imageScalingType = 'Normal' / 'Fit' / 'Fill'` | |
| Action (buttons, images, shapes) | `visualContainerObjects.visualLink[0].properties` | `show`, `type` (`Back`, `Bookmark`, `Drillthrough`, `PageNavigation`, `Qna`, `WebUrl`, `ApplyAllSlicers`, `ClearAllSlicers`; the schema also has `DataFunction`), `navigationSection` (a page name), `drillthroughSection` (a drillthrough page), `bookmark`, `webUrl`, `tooltip`; off = `show: false` |
| Button icon | `objects.icon[{properties: {shapeType}, selector: {id: "default"}}]` | `blank`, `leftArrow`, `rightArrow`, `back`, `reset`, `help`, `information`, `qna`, `bookmarks`, `applyAllSlicers`, `clearAllSlicers` (schema); Insert › Buttons writes the action that goes with Back, Bookmark, Q&A and the two slicer buttons |
| Button states | the state entries of `text`, `fill`, `icon`… (`selector: {id: "default" / "hover" / "press" / "disabled"}`) | see "State selectors" in [visual-options.md](visual-options.md) |
| Navigators | `pageNavigator`, `bookmarkNavigator` visuals | inserted without objects |
| Shape (Insert › Shapes) | `visualType: "shape"`, `objects.shape[{properties: {tileShape: '<shape>'}}]` without selector, `drillFilterOtherVisuals: true`, `howCreated: "InsertVisualButton"` (`shape.add`) | `tileShape` is one of the 23 values of the public report theme schema's `visual-shape` (`rectangle`, `rectangleRounded`, `oval`, `line`, `triangleIsoc`, `triangleRight`, `arrow`, …); its adjustments (`rectangleRoundedCurve`, `hexagonSlant`…) sit in the same entry. Fill (`fill.fillColor`, `transparency`, `show`) and outline (`outline.lineColor`, `weight`, `transparency`, `show`) go to the `default` state entry, rotation (`rotation.shapeAngle`) without selector, where the render captures found Desktop reads them ([visual-options.md](visual-options.md), "State selectors"). BIcraft's own choices, not compared with a file Desktop inserted: the default size (300 × 200, a line 300 × 40) and writing no default fill or outline; the rotation property (`shapeAngle`, the schema's "Shape rotation"; `angle` is read too) |
| Canvas background | `page.json` `objects.background[0].properties` | `color: {solid: {color: '#RRGGBB'}}`, `transparency: 20D`, `image: {image: {name: '<item>', url: <ResourcePackageItem>, scaling: 'Normal' / 'Fit' / 'Fill'}}` |
| Wallpaper | `objects.outspace[0].properties` | the same three properties |
| Vertical alignment | `objects.displayArea[0].properties.verticalAlignment` | `'Top'` / `'Middle'` |
| Page size | `page.json` `width`, `height` | 16:9 = 1280 × 720, 4:3 = 960 × 720, Letter = 816 × 1056, Tooltip = 320 × 240 (the theme schema's `pageSize.pageSizeTypes`; BIcraft writes only the dimensions) |

Image bytes are untrusted. `image.add`, `image.set` and `page.format` take at most 16 MB, type
the bytes by their content (PNG, JPEG, GIF, WebP, SVG; a `.png` name on text is refused) and
decode them with the renderer's decoder before storing them (raster: at most 16 million pixels
and 16,384 pixels a side, under `catch_unwind`; SVG: parsed with every reference outside the
bytes resolved to nothing, docs/render.md). BMP is not read. WebP is stored with a warning:
Desktop's Insert image dialog does not offer it.

## Report settings (`report.settings`, File › Options and settings)

Desktop's Options › Current file › Report settings are saved with the report. BIcraft writes them
with `report.settings` (`crates/engine/src/report_settings.rs`) and shows them on File › Options
and settings (`crates/ui/src/options_page.rs`). Sources: the public report schema 3.3.0
(`ExplorationSettings`, `ReportFormattingObjects`, vendored under
`crates/pbir/schemas/report/3.3.0/`) for the names, Microsoft Learn for the meanings: "Format
filters in Power BI reports" (`power-bi/create-reports/power-bi-report-filter`, dated 2026-10-21:
the Filters pane's eye, "Allow users to change filter types", persistent filters), "Let users
personalize visuals in a report" (`power-bi/create-reports/power-bi-personalize-visuals`),
"Drillthrough in Power BI reports" (`power-bi/create-reports/desktop-drillthrough`). No Desktop
file was compared; defaults marked *inferred* are our choice.

| Parameter | Desktop option (Learn) | `report.json` | Absent means |
|---|---|---|---|
| `persistent_filters` | "Don't allow end users to save filters on this file in the Power BI service" (checked = `false`) | `settings.isPersistentUserStateDisabled: true` when off, removed when on | allowed |
| `personalize_visuals` | Personalize visuals | `settings.allowInlineExploration`, written either way | off (*inferred*) |
| `cross_report_drillthrough` | Cross-report drillthrough (of the target report) | `settings.useCrossReportDrillthrough`, written either way | off (*inferred*) |
| `allow_change_filter_types` | Filtering experience: "Allow users to change filter types" | `settings.allowChangeFilterTypes`, written either way | off (*inferred*, like `defaultDrillFilterOtherVisuals`; a new report writes `true`) |
| `filter_pane_visible` | the eye next to Filters (hidden from readers in reading mode) | the `objects.outspacePane[]` entry without a selector: `properties.visible` `{"expr": {"Literal": {"Value": "false"}}}` when hidden, removed when shown (an entry left empty goes) | shown |

Desktop's View › Filters only hides the pane while editing and is not saved (Learn); BIcraft's
pane toggles are window preferences. A legacy layout keeps the same keys in `config.settings` and
`config.objects` (`legacy-layout-bb.md` §2); the reader keeps `outspacePane` `visible`.

## Page view and page options (View tab)

Sources: Microsoft Learn, "Apply page size and settings in a Power BI report"
(`power-bi/create-reports/power-bi-report-display-settings`, dated 2026-08-31) and "Use
gridlines and snap-to-grid in Power BI reports" (`power-bi/create-reports/desktop-gridlines-snap-to-grid`,
dated 2026-02-17); the public page schema (`PageDisplayOption`, "Defines how the page is
scaled", vendored under `crates/pbir/schemas/page/`). No Desktop file was compared.

- **Page view** (Fit to page, the default; Fit to width; Actual size) is the page's
  `displayOption` (`FitToPage`, `FitToWidth`, `ActualSize`), written by
  `page.set_display_option`; a legacy layout keeps it as the section's numeric `displayOption`
  (1, 2, 3; `legacy-layout-bb.md` §3). The app's canvas scales the page by it.
- **Show gridlines**, **Snap to grid** and **Lock objects** ("Lock all objects on the canvas so
  that they can't be moved or resized") have no property in the published report, page or
  visual container schemas, so BIcraft keeps them as the author's window preferences, never in
  the file. Learn says gridlines and snapping operate independently.
- The status bar's zoom is a view setting of the window, not saved.
- Page thumbnails: none of these pages describes them; "Use Report view in Power BI Desktop"
  (`power-bi/create-reports/desktop-report-view`) shows the pages as tabs in the navigation area
  below the canvas, which the app's page tabs are.

## Filters pane formatting and card order

Sources: the public page schema 2.1.0 (`PageFormattingObjects`: `outspacePane` → `OutspacePane`,
`filterCard` → `FilterCard`; `https://developer.microsoft.com/json-schemas/fabric/item/report/definition/page/2.1.0/schema.json`,
vendored under `crates/pbir/schemas/page/2.1.0/`); the public report schema 3.3.0, whose
`outspacePane` holds only `expanded` and `visible` (the formatting is the page's); the public
report theme schema 2.157 (`visualStyles.page.*.outspacePane`, `filterCard` with `$id`
`Available` / `Applied`, the property titles; `crates/visuals/schemas/`); Microsoft Learn,
"Format filters in Power BI reports" (`power-bi/create-reports/power-bi-report-filter`, dated
2026-10-21: the pane is formatted per page from the Format pane with nothing selected, applied
and available cards separately, theme defaults under `visualStyles` › `*` › `*`). No file
Desktop saved was compared.

**Formatting** (`page.format_filter_pane {page, card: pane|available|applied, …}`,
`crates/engine/src/filter_pane_format.rs`). The pane's properties go to the `objects.outspacePane`
entry without a selector, a card state's to the `objects.filterCard` entry whose `selector` is
`{"id": "Available"}` or `{"id": "Applied"}` (written `properties` then `selector`; the id is
the theme's `$id`, *inferred* for a page entry). Values are PBIR literals: colours
`{solid: {color: '#RRGGBB'}}`, numbers `12D`, booleans, the font family as a quoted text.

| Parameter | Pane (`OutspacePane`) | Cards (`FilterCard`) | Learn / theme schema title |
|---|---|---|---|
| `background_color`, `transparency` (0–100) | `backgroundColor`, `transparency` | the same | Background colour, Transparency |
| `foreground_color` | `foregroundColor` | the same | Font and icon colour |
| `font_family` | `fontFamily` | the same | Font family |
| `border`, `border_color` | `border`, `borderColor` (the line between the report and the pane) | the same (a one-pixel line around each card) | Border, Border colour |
| `input_box_color` | `inputBoxColor` | the same | Input box colour |
| `title_size`, `header_size`, `search_text_size` (6–72) | `titleSize`, `headerSize`, `searchTextSize` | — | Title, Header, Search text size |
| `checkbox_color` | `checkboxAndApplyColor` | — | Checkbox and Apply colour |
| `width` (100–1000) | `width` | — | (page schema only) |
| `text_size` (6–72) | — | `textSize` | Text size |

`null` removes a property, `reset` the card's properties (an entry left empty goes, then an
empty `objects`). The snapshot's `filter_pane_format: {pane, available, applied}` resolves each
property from the theme (`visualStyles.*.*` then `visualStyles.page.*`, base then custom theme;
a `filterCard` entry without `$id` formats both states under the state's own) and then the page
(an entry without a selector, then the state's). A card is *applied* when it has a condition.
The ranges are BIcraft's. A legacy layout keeps the page's `objects` in its section `config`
(`legacy-layout-bb.md`).

**Card order** (`filter.order {level: report|page|visual, page?, visual?, sort?, field?, to?}`,
`crates/engine/src/filter_edit.rs`). Source: the public `filterConfiguration` schema 1.3.0
(`filterSortOrder`: `Ascending` and `Descending` by the field's display name, `Custom` by each
filter's `ordinal`, filters without one at the end, then by display name; vendored under
`crates/pbir/schemas/filterConfiguration/1.3.0/`) and Microsoft Learn's page above (the cards
are listed alphabetically by default and can be dragged within their level). An absent
`filterSortOrder` is therefore read as `Ascending` (*inferred* from Learn). `sort` writes
`filterSortOrder`; `field` with `to` moves that card to the `to`-th place of the shown order
(past the end: last) and writes `Custom` with `ordinal` 0…n−1 on every card of the level. The
display name is the card's `displayName`, else the column or measure name, compared without
case. Ordinals are kept when the order goes back to A to Z. The snapshot lists each level's
cards in this order with `filter_sort` (`ascending`, `descending`, `custom`). A legacy layout
stores `filterSortOrder` as the codes 1, 2, 3 (`legacy-layout-bb.md`) and keeps the ordinals.

## Validation

`validate_documents` returns `Issue { path, pointer, message }`:

1. **Schema.** Each JSON document is validated against the vendored schema its `$schema` names:
   the exact version when vendored; a newer minor of a known major is validated against the
   newest vendored minor (Desktop opens it, ORA-j12; this is how the observed
   `visualContainer` 2.13.0 documents are checked); an unknown major, a wrong schema kind for the
   file, a missing `$schema`, invalid JSON or a BOM are issues.
2. **Consistency.** `version.json`, `report.json`, `pages.json` and at least one page exist;
   `pageOrder` entries exist and are unique (a page missing from it is fine: Desktop lists it
   last and adds it on save); `activePageName` exists; page
   names and visual names (per page) are unique; `parentGroupName` names a visual group on the
   same page; `mobile.json` has its `visual.json`; every `resourcePackages` item of a
   `RegisteredResources`/`SharedResources` package exists as `StaticResources/<package>/<path>`;
   `bookmarks.json` items exist and bookmark files have a `bookmarks.json`; no unknown files under
   `definition/`.

**Validator.** `crates/pbir/src/schema.rs` (draft-07) with its own pattern matcher
(`crates/pbir/src/regex.rs`). The same validator backs `bicraft_pbir::JsonSchema`, which the
engine uses for report themes against `crates/visuals/schemas/reportThemeSchema-2.157.json`.

- **Compiled once.** A schema is compiled into an arena of nodes: `$ref`s become node ids,
  properties a sorted table, patterns compiled programs; messages are built only when an issue is
  reported. Each vendored root file (with the files it references) is compiled on first use and
  cached for the life of the process (`OnceLock<Mutex<HashMap>>` keyed by the vendored path,
  which is the `$schema` URL minus its prefix); parsed files are cached too, so roots share them.
  Timings: `docs/benchmarks.md` hot spot 3 (G11).
- **Bounded.** Nesting depth 1,500 (also stops `$ref` cycles), a 2 M-step budget and at most 100
  issues per document, at most 2 M schema nodes; validation (including `JsonSchema::validate`)
  runs on a thread with a 64 MiB stack.
- **Patterns.** ECMA-262 subset, unanchored as JSON Schema requires: literals, `.`, classes with
  ranges and `\d \w \s` (and negations), `\b \B`, escapes (`\n \t \xHH \uHHHH`…), groups
  (capturing, `(?:)`, named), lookaheads `(?=)` `(?!)`, `|`, greedy and lazy `* + ? {n,m}`, `^ $`.
  Matching is a Pike VM (an NFA simulation): linear in the string, no backtracking, no recursion
  over the input. Programs are capped at 20,000 instructions, `{n,m}` at 1,000, group nesting at
  64. A pattern outside the subset (backreferences, lookbehind, `\p{…}`) does not compile and
  accepts every string; as a `patternProperties` key it covers every property name (no
  `additionalProperties` error) without applying its schema. No vendored pattern is in that case
  (a unit test checks it).

Keyword coverage. Counts are the files that use the keyword: 106 vendored PBIR files, plus the
report theme schema (1 file); all declare draft-07. Found by walking every schema position of every
vendored file.

| Keyword | PBIR files | Theme | Implemented |
|---|---:|:---:|---|
| `type` (name or list), `properties`, `required`, `additionalProperties`, `const` | 106 | yes | yes |
| `$ref` (local `#/…`, relative file, absolute PBIR URL; `~0 ~1` and `%XX` in fragments) | 103 | yes | yes; draft-07 rule: siblings of `$ref` are ignored |
| `definitions` | 103 | yes | through `$ref` (so is `$defs`) |
| `items` (one schema) | 94 | yes | yes; tuple form and `additionalItems` too |
| `anyOf` | 88 | yes | yes |
| `maxLength` / `minLength` | 51 / 10 | yes | yes (counted in characters) |
| `enum` | 38 | yes | yes |
| `oneOf` | 27 | yes | yes |
| `patternProperties` | 7 (`^[0-9]+$`) | yes (`.*`, `^.+$`, `^(?!\*$).+$`) | yes |
| `pattern` | 5 (2 version patterns) | yes (colour, `^[A-Za-z0-9]+$`, visual-state list) | yes, all 9 distinct patterns compile |
| `allOf`, `not`, `minimum`, `maximum` | 0 | yes | yes |
| `$id`, `$schema`, `title`, `description` (106 each), `default` (46), `$comment` (theme only) | 106 | yes | annotations, ignored (`$id` only appears on roots) |
| `if`/`then`/`else`, `propertyNames`, `uniqueItems`, `contains` (+ `minContains`/`maxContains`), `dependencies`, `dependentRequired`, `dependentSchemas`, `minProperties`/`maxProperties`, `minItems`/`maxItems`, `exclusiveMinimum`/`exclusiveMaximum` (number, or draft-04 boolean), `multipleOf`, `additionalItems`, boolean schemas | 0 | 0 | yes, with unit tests |
| `format` | 0 | 0 | annotation only (draft-07 makes assertion optional; avoids false errors) |
| nested `$id` base changes, `$anchor`, `$dynamicRef`, `unevaluated*` | 0 | 0 | no |

No false errors on Desktop-written files: `tests/real_data.rs` validates every PBIX/PBIT under
`$BICRAFT_TESTDATA` (any depth) and requires zero issues (28 packages with a PBIR report locally,
2026-10-14).

**Why not the `jsonschema` crate.** No dependency tree (that crate pulls regex engines, URL/IDNA
parsing and a reference resolver, and needs care to stay offline); it follows the crate's
never-crash rules above; and it gives stable JSON pointers and messages for CLI/MCP output.
Schemas are embedded with `include_str!`.

## Licenses of vendored schemas

`crates/pbir/schemas/` holds every `schema.json` / `schema-embedded.json` of
`fabric/item/report/definition/` (106 files, unmodified; `definitionStructure` omitted) from
Microsoft's `json-schemas` repository, commit `49f4a76cb7a75fee2669b7cc6ba12ca4332050f7`. That
repository is under the **MIT License** (Copyright (c) Microsoft Corporation); its text is in
`crates/pbir/schemas/LICENSE`, provenance in `crates/pbir/schemas/SOURCE.txt`. MIT is compatible
with BIcraft's MIT OR Apache-2.0 distribution as long as the notice is kept.

## Tests

- `crates/pbir/tests/fixtures/sales.Report`: a synthetic report we authored (2 pages, card,
  column chart with sort and visual filter, table, slicer in a visual group, image bound to a
  registered PNG we generated, line chart with a hierarchy-level field, hidden textbox, report and
  page filters, 2 bookmarks with a group, a report extension, a mobile layout, a custom theme).
  It validates with zero issues and reads → writes byte-identically.
- `tests/real_data.rs` (local only): every `*.pbix`/`*.pbit` at any depth under
  `$BICRAFT_TESTDATA` with PBIR parts validates with zero issues; every top-level `*.pbix`
  with PBIR parts, and the folder in `$BICRAFT_PBIR_DIR`: zero validation issues, every document equal as JSON
  and identical to the source re-indented (key order), resources byte-identical, a second
  read/write stable, `write_dir` into a scratch folder reads back equal.

## Model notes (for `crates/report`)

- There is no typed place for `bookmarks.json`, `reportExtensions.json`, `mobile.json` or
  unknown `definition/` files; they ride in `Report.resources`. A dedicated
  `Report.documents: Vec<Resource>` (non-StaticResources files) would make that explicit; a legacy
  layout writer must ignore `definition/…` resources.
- `Position` has no `extra`; unknown position keys are kept in `Visual.extra["position"]`.
- `Report.schemas` holds one URL per kind; per-object versions live in each object's `extra`.

## Desktop behaviour (settled 2026-10-14)

Checked with Power BI Desktop 2.158.1304.0 (Store) on PBIP projects BIcraft generated: each was
opened, saved with the Save button (driven through UI Automation) and the files compared before and
after.

| Question | Desktop |
|---|---|
| Page folder name different from the page's `name` | Opens; both pages shown in `pageOrder` order. Save keeps the folder name. The reader trusts `name`, the writer names folders after it. |
| A page missing from `pageOrder` | Opens with that page last; save adds it to `pageOrder`. Not a validation issue. |
| Empty `themeCollection` (`{}`) | Opens; saved unchanged. |
| U+2028/U+2029 and non-ASCII text | Desktop writes ` ` escaped and non-ASCII raw (UTF-8); it reads raw U+2028 (what serde_json writes) without trouble. |
| Line endings | Desktop saves every PBIR file with CRLF; we write LF (both read the same). |

## Report validation against Desktop

`report.validate` (`crates/engine/src/report_checks.rs`) checks filters, hierarchy references
and bookmarks beyond plain field references. Severities were calibrated on 2026-10-14 with Power
BI Desktop 2.158.1304.0 (Store) on PBIP projects BIcraft generated, each with deliberately
broken filters or bookmarks. Desktop was driven through UI Automation: the project was opened,
the accessibility tree of the window was read (each visual's container names its error, e.g.
"visual error: one or more fields have a problem"), and bookmarks were applied one by one in the
bookmarks pane's View mode. Each broken filter sat alone on its visual so one failure could not
hide another; report and page filters were tried in separate projects. The tables had no rows (the
project had never been refreshed), which does not change whether a query fails.

**Rule.** An error is what makes Desktop refuse the file or show an error in a visual. A warning
is what Desktop opens and runs without complaint, even if the result is not what the JSON seems to
say.

### Filters

Every `filterConfig.filters[]` container of the report, its pages and visuals, and every filter
state of a bookmark (`byName`, `byExpr`, `byType`, `byTransientState` at report, section and
visual level). `Passthrough` and unknown types are skipped (the latter are schema issues).

| Case | Desktop 2.158 | `report.validate` |
|---|---|---|
| `RelativeDate` / `RelativeTime` on a column that is not a date (text, whole number), or on a hierarchy level whose column is not a date | the visual shows "couldn't retrieve the data"; as a report filter it breaks every visual of every page (the filter card reads "is (All)") | error |
| `RelativeDate` / `RelativeTime` on a measure | the visual shows "couldn't retrieve the data" (tried 2026-10-15, visual level) | error |
| `Contains` / `StartsWith` / `EndsWith` on a column that is not text | the visual shows "something went wrong with one or more filters" | error |
| A literal of the wrong type for its column (`'abc'` on a whole number in `In`, `5L` on text in a comparison; `null` fits any type) | the visual shows "something went wrong with one or more filters" | error |
| `Categorical` with a range comparison (`>`) | no error; the condition is applied as written | warning |
| `TopN` with a plain `In` list (no `Table` subquery) | no error | warning |
| `Categorical` (also `Include`, `Exclude`, `Tuple`, `TopN`) on a measure, visual level | no error | warning |
| `Advanced` with relative dates (`Now`/`DateAdd`/`DateSpan`) | no error | warning |
| `RelativeDate` whose condition does not use `Now` | no error | warning |
| `Include` with a `Contains` condition | no error | warning |
| A measure filter on a page | shown in the filter pane ("Total is greater than 1"), no visual error | warning |
| `VisualTopN` | tried 2026-10-15: a `{"VisualTopN": {"ItemCount": 2}}` condition keeps two rows; a plain `In` list runs as that list, no error | accepted with a `VisualTopN` item count or an `In` over a subquery; warning otherwise |

Shape rules (warnings): `TopN` holds an `In` with `Table`; `RelativeDate`/`RelativeTime` use
`Now`, `DateAdd` or `DateSpan`; `Categorical`, `Include`, `Exclude`, `Tuple` hold value lists (no
text operators, range comparisons, relative dates or subqueries); `Advanced` holds no relative
dates or subqueries. A container without `filter` (an empty filter card) is not checked.

### Hierarchies and date variations

Every `Hierarchy`, `HierarchyLevel` and `PropertyVariationSource` expression in visuals (queries,
formatting), filters and bookmarks, with `From` aliases resolved:

- `Hierarchy` on a `SourceRef`: the table has a hierarchy of that name (TMSL `hierarchies`,
  case-insensitive);
- `Hierarchy` on a `PropertyVariationSource`: the hierarchy is on the table the variation points
  to (its relationship's other end, else its `defaultHierarchy` table);
- `HierarchyLevel`: the hierarchy has that level;
- `PropertyVariationSource`: the column has a variation of that `Name`.

A missing hierarchy or level in a visual's query: Desktop shows "one or more fields have a
problem" in the visual, so these are errors (warnings in bookmarks). Variations were not tried in
Desktop (the test model had no auto date tables); they follow the same rule. A missing table or
column is reported once, by the field check, not again as a hierarchy.

### Bookmarks

Checked: the active page (`activeSection`) exists; every `sections` key is a page; every
`visualContainers` key is a visual of that page and every `visualContainerGroups` key a group of
that page; a captured `singleVisual.visualType` matches the visual's type (unless `targetType` is
set); every `options.targetVisualNames` entry is a visual of the active page; filter states as
above; fields as before.

Desktop opened a project whose nine bookmarks were each broken in one way (missing active page,
state of a missing page, of a missing visual, a filter on a missing column, a missing target
visual, a wrong visual type, a filter on a missing hierarchy, a page filter on a missing table).
It listed all nine in the bookmarks pane and applied each with no error: a missing active page
leaves the current page shown; broken filters are ignored and the visuals still show their data.
Saving the unmodified project wrote nothing. So every bookmark finding is a **warning**.

**Drill states (G55, 2026-11-25).** `bookmark.capture` / `bookmark.update` write the reader's
drill of a visual into its `visualContainers` entry, in the bookmark schema 2.1.0's
`VisualContainerState`: `singleVisual.activeProjections` (role → the active fields, as
`QueryExpressionContainer`s), `singleVisual.expansionStates` for a matrix whose rows are
expanded, and the drilled values as `filters.byExpr` entries (`{name, type: "Categorical",
expression, filter, howCreated: 2}`; 2 is the filter schema's third origin, `Drill`, read from
the order of its `howCreated` strings). `bookmark.apply` and `view.interact {action: "bookmark"}`
read them back into the view's drill (a bookmark without them resets the visual's drill), within
the Data option. This shape is BIcraft's reading of the public schemas: **no file Desktop saved
with a drilled visual was compared**. The drill mode (drill down on click) is not captured.

### Bookmark display mode and sort

Sources: the public `bookmark` 2.1.0 schema (`crates/pbir/schemas/bookmark/2.1.0/schema.json`,
`VisualContainerState`, `SingleVisualConfigState`, `VisualContainerDisplayMode`) and Microsoft
Learn, "Add spotlights to Power BI reports" and "Change how a chart is sorted in a report"
(read 2026-11-19). No Desktop-saved bookmark of this shape was compared.

| What | Where | Values |
|---|---|---|
| Visibility, spotlight, focus | `sections[p].visualContainers[v].singleVisual.display.mode` | `hidden`, `spotlight` (the page's other visuals dimmed), `maximize` (focus mode, not written), `elevation` (not written) |
| Show as a table in focus | `singleVisual.display.maximizedOptions.dataTable` | `normal`, `accessible` (not written) |
| A reader's sort | `singleVisual.orderBy` | semanticQuery `QuerySortClause`s `{Direction, Expression}`: `Direction` 1 ascending, 2 descending; `Expression` the field as in `query.sortDefinition` |

- `VisualContainerState` takes only `filters`, `singleVisual` and `highlight`
  (`additionalProperties: false`); BIcraft wrote `display` on the container before 2026-11-19 and
  still reads it there.
- Learn says a spotlight active when a bookmark is added is kept in it; a bookmark's sort is the
  personalized sort a reader saves with a bookmark. BIcraft applies `orderBy` with the data
  options (not under `suppressData`) and `display.mode` with the display options.

**Slicer selections in a bookmark** (2026-11-19, G55; sources: the `bookmark` 2.1.0 schema,
`crates/pbir/schemas/bookmark/2.1.0/schema.json`, definitions `VisualContainerState`,
`SingleVisualConfigState`, `DataViewObjectDefinitionUpdates`; Microsoft Learn "Create report
bookmarks in Power BI", which puts slicers under the Data option). A slicer's selection is a
formatting-object update of its container:

```json
"visualContainers": { "<slicer>": { "singleVisual": {
  "visualType": "slicer",
  "objects": { "merge": {
    "general": [ { "properties": { "filter": { "filter": { "Version": 2, "From": [ … ], "Where": [ … ] } } } } ],
    "data": [ { "properties": { "numericStart": { "expr": { "Literal": { "Value": "10D" } } } } } ] } } } } }
```

`merge.general[0].properties.filter` has the shape a slicer stores in its own `objects.general`
(`visual.json`); `merge.data` carries the range and relative-date properties a range slicer
shows (`numericStart`, `numericEnd`, `startDate`, `endDate`, `relativeRange`,
`relativeDuration`, `relativePeriod`, `relativeTimePeriod`), only those set. A slicer with nothing
selected is written as `"remove": [{"object": "general", "property": "filter"}]`; applying it
also clears the range properties, as Clear does. BIcraft writes and reads only these two objects;
other merged objects, `remove` entries for other properties, `highlight` and `expansionStates` are kept
verbatim and not applied (`orderBy` and `display.mode`: above). The exact shape Desktop writes for an empty slicer
(a `remove`, or no `objects` at all) is *inferred* from the schema, not observed: a container
without either is left alone when applied. The legacy layout (`config.bookmarks` in
`Report/Layout`) keeps `explorationState` verbatim ([legacy-layout-bb.md](legacy-layout-bb.md)),
so the same shape is read and written there.

## Matrix expansion and per-level subtotals

Power BI Desktop 2.158.1304.0 (Store), 2026-10-15: black-box observations of our own report
(`oracle-gen grids`): Desktop opened it, its matrices were expanded through the report view
(visual header "Expand all down one level", context menu "Expand > Selection") and formatted in
the format pane, then saved; the saved `visual.json` files were read. The same shapes, written by
`oracle-gen expand`, were then opened and captured (`tools/render-oracle`).

| Action in Desktop | What it saves |
|---|---|
| Expand all down one level (rows Region, Category) | every projection of the role gets `"active": true`; `expansionStates: [{roles: ["Rows"], levels: [{queryRefs: ["Sales.Region"], isPinned: true}, {queryRefs: ["Sales.Category"], isCollapsed: true, isPinned: true}], root: {}}]` |
| Expand > Selection on member South | the level gets `isCollapsed: true` and `identityKeys: [<the Column expression>]`; `root: {children: [{identityValues: [{Literal: {Value: "'South'"}}], isToggled: true}]}` |
| The same on a column hierarchy | the same under `roles: ["Columns"]` |
| Row subtotals, "per row level" on, level Region, subtotal off | `subTotals: [{properties: {perRowLevel: true}}, {properties: {levelSubtotalEnabled: false}, selector: {metadata: "Sales.Region"}}]` |
| Per level Category, subtotal label "Sous-cat" | `{properties: {levelSubtotalLabel: 'Sous-cat'}, selector: {metadata: "Sales.Category"}}` |
| Column subtotals label "SousCol" (not per level) | `{properties: {columnSubtotalsLabel: 'SousCol'}}` |

What Desktop shows for them:

- The projections' `active` flags choose the displayed levels: none marked shows the top level
  only, even with `expansionStates` that open the others; all marked without `expansionStates`
  shows every level expanded; `[false, true]` shows the second level alone (drilled down).
- Within the active levels, a level without `isCollapsed` has all its members expanded; members
  listed under `root.children` with `isToggled` invert that (an expanded member of a collapsed
  level; a collapsed one of an open level); nested `children` address deeper members (each
  `identityValues` holds that level's value only).
- A per-level entry names the field a subtotal totals over: `Sales.Region` (the top row field)
  switched the grand total row off, `Sales.Category` the subtotals of the Region members. With
  the subtotal position at the top, a switched-off level leaves its member rows without values.
- `rowSubtotalsLabel` / `columnSubtotalsLabel` (not per level) rename every subtotal and the grand
  total ("SousCol" over each channel's subtotal and over the grand total column).
- A field on both the rows and the columns of one matrix makes Desktop fail the visual's query
  ("error retrieving the data").

BIcraft's preview reads all of this (`crates/engine/src/preview_matrix.rs`: `data.row_expansion`,
`column_expansion`, `row_level_totals`, `column_level_totals`; `format.table.row_subtotals_label`,
`column_subtotals_label`) and the renderer draws it (`docs/render.md`, Known differences).

Writing (2026-11-20): the author's `visual.expand {page, visual, role?, path? | level? | all? |
step?, collapse?}` (`crates/engine/src/interact_expand.rs`) applies the reader's matrix steps of
`view.interact` to the report visual itself and writes the shapes above: `path` is Expand or
Collapse › Selection (a `root.children` entry with `isToggled`), `level` › Entire level
(`isCollapsed` on that level, toggled members below it dropped), `all` › All, and `step`
(`expand_all`, `next`, `up`) the header's Expand all down one level, Go to the next level and
Drill up (the projections' `active` flags). It is undoable and round-trips through PBIP and
PBIX. The app sends it for the `+`/`-` buttons and the right-click Expand / Collapse entries
while editing; a reader's clicks stay `view.interact`. Each level of up to eight is evaluated
with the measure in its own filter context (never a sum of its children). The preview lists the
levels' fields as `data.row_level_refs` / `column_level_refs`, which the Format pane's Row
subtotals and Column subtotals cards use for their per-level rows (Show subtotal, Subtotal
label: one `subTotals` entry per field, `selector.metadata` = its queryRef), as Microsoft
Learn's "Create a matrix visual in Power BI" describes the cards and the `subTotals` object of
the public visual-container schema lists the properties. Not checked against a file Desktop
saved after 2026-10-15: the writer reuses the 2.158 shapes of the table above.

## Open questions

1. How Desktop orders several pages that are all missing from `pageOrder` (the reader sorts them
   by name).

## Public and black-box evidence

The PBIR loader facts (UTF-8 without BOM, `$schema` major rule, required files, `definition.pbir`
version) rest on Microsoft Learn and Desktop runs; see
[packaging-verification.md](packaging-verification.md) (P-* and PK-ENC-PBIR facts). New rule from
Microsoft Learn's PBIR naming convention (P-NAME): page, visual and bookmark names and their
folders must be letters, digits, underscores or hyphens, because Desktop ignores the others. The
writer refuses such names and `validate_documents` reports them.
