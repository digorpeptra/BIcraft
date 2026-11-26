# Visual formatting options: the catalog

Implemented by `crates/visuals` (`bicraft_visuals::{visual_types, visual, object, object_in,
property, check_value, suggest}`), used by the engine's `visual.options` command and by every
formatting write (`crates/engine/src/visual_catalog.rs`, called from `visual_props.rs`).

> **Status 2026-10-29.** Formatting objects and properties come from the public theme schema and
> the PBIR `visualConfiguration` schema. The oracle sections below record runs of 2026-10-13/14;
> their counts describe the catalog of those dates.
> Data roles: see [`visual-roles-bb.md`](visual-roles-bb.md).

## Sources

| Source | Used for |
|---|---|
| Microsoft's report theme JSON schema, <https://github.com/microsoft/powerbi-desktop-samples>, folder `Report Theme JSON Schema`, file `reportThemeSchema-2.159.json` (the latest published; 2.157, commit `0a71afb5`, until 2026-11-20), commit `34f78db3` (2026-10-15), MIT; 2.159 adds `ganttChart` (docs/formats/gantt.md) | Visual types, their formatting objects, each property's JSON type, enumerated values, ranges, display names and descriptions. **Vendored unmodified** under `crates/visuals/schemas/` with its `LICENSE` and a `SOURCE.txt` (see `THIRD_PARTY_NOTICES.md`). |
| The vendored PBIR `visualConfiguration` 2.7.0 schema (`crates/pbir/schemas/`, MIT) | Which objects are `visualContainerObjects` and their property names (the schema rejects others). A test checks our table against it. |
| Reports Desktop wrote (our committed fixtures, and files held locally and never committed: only object, property and role *names* were tallied, never values) | Data roles per visual type, the properties themes cannot set (below), and the check that every value Desktop wrote passes the catalog. |
| Power BI Desktop 2.158.1304.0 (Store), as an oracle | Generated PBIP reports setting every catalogued property open, and Desktop's own save keeps them (see [Reproducibility oracle](#reproducibility-oracle)). |

Sources: the public report theme schema and PBIR schemas (MIT), black-box tests against Power BI
Desktop 2.158 (the reproducibility oracle and render captures below, and tallies of names in
reports Desktop wrote); detailed provenance is kept privately by the maintainers.
Data roles are in `visual-roles-bb.md`. The items marked
"own reading" below are our own reading, not taken from Desktop.

## How the schema maps to the catalog

The theme schema has, under `properties.visualStyles.properties`, one entry per visual type
(`clusteredColumnChart`...) plus `report`, `page`, `filter` and `group`, which are not visual
types. Each visual's styles point to `definitions.visual-<type>`: an `allOf` of
`definitions.commonCards` (the container objects) and an object whose `properties` are the visual's
formatting objects. Each object is an array of property bags (`items.properties`).

- The `*` object (a wildcard: properties a theme applies to every object) is skipped.
- Properties whose name starts with `$` (`$id`: the visual state an entry applies to, such as
  `default`, `hover`, `selected`) are not properties; an object that has one takes selectors.
- An object present in both parts of an `allOf` is merged.

Property types:

| Schema | `PropType` | Written in PBIR |
|---|---|---|
| `"type": "boolean"` | `Bool` | `true` / `false` |
| `"type": "number"` (+ `minimum`/`maximum`) | `Number { integer: false }` | `12D` |
| `"type": "integer"` | `Number { integer: true }` | `3L` (Desktop also writes `D` for some; both are accepted, the value must be whole) |
| `"type": "string"` with `oneOf` of `const` | `Enum` (the `const` values) | `'Bottom'` |
| `"type": "integer"` with `oneOf` of `const` (display units) | `Enum` with numeric values | `1000000D` |
| `"type": "string"`, name ending in `fontFamily` | `Font` | `'Segoe UI'` |
| `"type": "string"` | `Text` | `'text'` (quotes doubled inside) |
| `$ref: #/definitions/fill` | `Color` | `{"solid": {"color": {"expr": {"Literal": {"Value": "'#RRGGBB'"}}}}}` or a `ThemeDataColor` |
| `$ref: #/definitions/image` | `Image` | an image expression or literal |
| `$ref: #/definitions/fontSize` | `Number` of at least 8 (the schema's maximum, 60, is not enforced: see the oracle) | `12D` |
| `$ref` to `alignment`, `verticalAlignment`, `labelDisplayUnits`, `displayUnitsWithoutAuto` | `Enum` | |
| other `$ref` (`fillRule`, `dataBars`, `icon`, `paragraphs`, `annotationTemplate`, `itemLocation`), type arrays (`["number", "string"]`: axis `start`/`end`) | `Other` (kept as written) | |

The schema gives a range only for text sizes. Our own reading adds 0 to 100 to every number named
`transparency` or ending in `Transparency` (Desktop's format pane shows them as percentages).

## Own reading (not in the schema)

- **Container objects** are the 15 objects of PBIR `VisualContainerFormattingObjects` (`title`,
  `subTitle`, `divider`, `spacing`, `background`, `padding`, `lockAspect`, `general`, `border`,
  `dropShadow`, `visualLink`, `visualTooltip`, `stylePreset`, `visualHeader`,
  `visualHeaderTooltip`), with exactly the PBIR schema's properties, typed from `commonCards`.
  Every visual type has them. A visual object may share a name with a container object
  (`general`, and `border`/`padding`/`divider`/`spacing` on the new card and slicers); `object()`
  returns the visual one first, `object_in(type, name, container)` picks.
- **Selectors**: `dataPoint`, `labels`, `lineStyles`, `seriesLabels`, `markers`,
  `columnFormatting`, `columnWidth`, `values`, `columnHeaders`, `rowHeaders`, `subTotals`,
  `columnTotal`, `rowTotal`, `total`, the analytics lines (`referenceLine`, `xAxisReferenceLine`,
  `y1AxisReferenceLine`, `y2AxisReferenceLine`, `trend`, `forecast`, `error`,
  `anomalyDetection`, `ratioLine`), `fillPoint`, and every object with visual states (`$id`).
- **Analytics pane**: the Analytics tab offers a card per analytics object the type has in the
  catalogue (`y1AxisReferenceLine`, `xAxisReferenceLine`, `referenceLine`, `trend`, `forecast`,
  `error`, `ratioLine`, `plotAreaShading`), and the error bars' band options only where `error`
  has `shadeShow` (line charts in the theme schema 2.157). `error.errorRange` is typed `Other`;
  its value is BIcraft's own shape ([pbir.md](pbir.md) "Chart specifics", unchecked).
- **Conditional formatting** (`conditional: true`): every property with a literal type (bool,
  number, text, color, enumeration, font, image), because Desktop keeps a field value on every one
  of them when it saves (oracle below); `Other` properties accept any value anyway. This may be
  wider than the format pane's `fx` buttons, which were not surveyed. The flag decides only where field and rule expressions are accepted.
- **Data roles** (the `query.queryState` keys): from public documentation and black-box tests,
  with their kinds and accepted field counts; see `visual-roles-bb.md`. A hand-written
  list covers the types that note does not. `visual.options` returns each
  combination's bounds with `kind` (`measure`/`grouping`, scatter chart and maps) and `related`
  (slicers: fields must be hierarchically related) where Desktop's conditions set them.
- **Defaults** are not published anywhere public: `Property::default` is always `None`.

## Value checks (`check_value`)

A PBIR property value is `{"expr": E}` or, for colors, `{"solid": {"color": {"expr": E}}}`:

- `E` = `{"Literal": {"Value": s}}`: `s` must be `true`/`false` (Bool); a number with an optional
  `L`/`D`/`M` suffix, in range, whole for integers (Number); a single-quoted text with doubled
  inner quotes (Text, Font, Image); one of the values exactly, case included (Enum; numerically
  for numeric enums). A color literal must be `'#RGB'`, `'#RRGGBB'` or `'#RRGGBBAA'`.
- `E` = `{"ThemeDataColor": {"ColorId": n ≥ 0, "Percent": -1..1}}` (colors).
- `E` = a field or rule expression (`Measure`, `Column`, `Aggregation`, `HierarchyLevel`,
  `Conditional`, `FillRule`, `SelectRef`, `ResourcePackageItem`, `Arithmetic`, `Transform`):
  accepted on `conditional` properties (every literal-typed one).
- `Other` properties accept any value.

Messages name the property and what it expects (`` `transparency` expects a number from 0 to
100; got 120D ``), with a "did you mean" for enum values (edit distance at most a third of the word, at least
2).

## In the engine

- Every value `visual.format`, `visual.set_title` or `report.apply` writes is checked when the
  visual type is in the catalog: unknown object or property → error with a "did you mean"; bad
  value → the `check_value` message. Values the visual already has (a no-op) and removals (`null`)
  are not checked, so a visual's own `props` always re-apply.
- Custom visuals (types not in the catalog) are not checked; `visual.format` returns a warning.
- `check_object_entry(visual_type, object, property, value)` is the helper for other formatting
  modules: `Ok(None)` valid, `Ok(Some(warning))` unchecked type, `Err` with the reason.
- On the local real reports (345 visuals), every one of the 4,428 formatting values Desktop wrote
  passes the catalog (`real_visuals_pass_the_catalog`, local only).

## Desktop oracle

Power BI Desktop 2.158.1304.0 (Store), 2026-10-13. A PBIP generated by BIcraft (one import
table from an M `#table`, one measure) with five visuals formatted only through `report.apply`
props (typed groups and `objects` entries, all checked against the catalog), opened in Desktop and
refreshed through Tabular Editor 2. It opened without any error or repair prompt, and showed:

- column chart: title bold, centred, 18 pt, `#1F4E79`; bars `#2E7D32` (`dataPoint.fill`, no
  selector); data labels on, 14 pt, `#C00000`; value axis labels 12 pt `#7030A0` and range 0 to 400
  (`valueAxis.start`/`end`, `0D`/`400D`); `categoryAxis.innerPadding` `40L`;
- line chart: legend at the top right (`legend.position` `'TopRight'`, a value the theme schema
  lists), 14 pt; `lineStyles.strokeWidth` `6D`,
  `showMarker`, `markerShape` `'diamond'`; value axis 0 to 200; data labels 12 pt;
- card: background `#FFF2CC`, callout `#C00000` at 48 pt;
- a second card whose `labels.fontSize` was set to `80D` by editing the file (the catalog then
  capped it at 60): Desktop rendered it at 80 pt, larger than the 48 pt card, so the schema's
  maximum is a theme limit and is no longer enforced;
- pie chart: legend `'BottomCenter'`, `labelStyle` `'Category, percent of total'`, 16 pt labels,
  `position` `'outside'` (opened without error; the chart sat below the captured screen area).

## Reproducibility oracle

Power BI Desktop 2.158.1304.0 (Store), 2026-10-14. A script (no longer in the tree) drove
BIcraft over MCP (`bicraft mcp`). `gen` writes our own one-table model (an M `#table`, two
measures), creates the PBIP (`project.new`), adds one visual per type and per 150 visual-object
properties (`report.apply`, fields bound by data role: 82 visuals on 10 pages), and sets every
catalogued property through `visual.format` `objects` entries, all checked against the catalog:
bool `true` (`false` for `show…` sub-switches), an enumeration's second value, a number inside its
range (`13` for text sizes, `30` for transparencies, else `3`; `L` for whole numbers), `#C00000`,
`'Arial'`, a type-specific text (`'0.00'` for format strings, `'4 2'` for dash patterns, a URL for
URLs), `3D` for variants (axis `start`/`end`, dynamic label values). Images and other structured
values (`fillRule`, `filter`, `annotationTemplate`, `itemLocation`...) are not exercised. Desktop
then opens the project, driven through UI Automation (refresh, **Save**, close), and `diff`
compares every property in the files Desktop wrote with what BIcraft wrote: kept unchanged,
normalized (other JSON) or dropped.

| Run | Properties set | Kept unchanged | Normalized | Dropped |
|---|---|---|---|---|
| Catalog of 2026-10-14 | **14,085** (7,323 of the 7,429 visual-object properties, 6,762 of the 6,860 container ones) | **14,085** | 0 | 0 |
| Same, every property set to the measure `[Total]` instead of a literal (files rewritten after `gen`, bypassing our checks) | 14,085 | 14,085 | 0 | 0 |

Per visual type (catalog of 2026-10-14; properties set, all kept): actionButton 206,
advancedSlicerVisual 368, aiNarratives 148, areaChart 502, azureMap 334, barChart 444, basicShape
147, bookmarkNavigator 206, card 157, cardVisual 539, clusteredBarChart 444, clusteredColumnChart
444, columnChart 444, decompositionTreeVisual 186, donutChart 210, filledMap 172, funnel 171, gauge
172, hundredPercentStackedAreaChart 486, hundredPercentStackedBarChart 444,
hundredPercentStackedColumnChart 444, image 166, keyDriversVisual 155, kpi 182, lineChart 584,
lineClusteredColumnComboChart 533, lineStackedColumnComboChart 511, listSlicer 516, map 183,
multiRowCard 166, pageNavigator 205, pieChart 174, pivotTable 282, pythonVisual 140, qnaVisual 168,
rdlVisual 157, ribbonChart 444, scatterChart 358, scorecard 230, scriptVisual 140, shape 194,
shapeMap 162, slicer 266, stackedAreaChart 486, tableEx 214, textSlicer 242, textbox 143, treemap
169, waterfallChart 247. Not exercised: 204 (35 images, 169 structured values).

Each project opened without a repair prompt or error dialog (UI Automation tree checked: the only
dialog is Desktop's usual "enable script visuals" prompt for the R and Python visuals, and the
Problems tab shows 0). Desktop normalizes nothing it keeps: same literals, suffixes and color
spelling. What this does **not** show: that every value draws as intended (Desktop keeps values it
may ignore when drawing), and how selectors behave. Screenshots of the pages could not be taken in
this session (no interactive display: screen capture fails, and window capture shows Desktop's
frame but not the WebView2 report canvas).

## State selectors

Power BI Desktop 2.158.1304.0 (Store), 2026-10-14. "Kept by Desktop's save" (above) is not
"applied": the new card, the new slicers, buttons, navigators and shapes draw most formatting
objects from a **state entry**, the object entry with `"selector": {"id": "<state>"}`, and ignore
those properties in the entry without selector. An `accentBar` written without selector is kept by
Desktop's save and never drawn.

Sources: (1) the published report theme schema (
`visualStyles.<type>.*.<object>.items.properties.$id`):
the objects whose theme entries take a state `$id`, and the state names; (2) render captures
(`tools/render-oracle/capture.ps1`) of our own reports setting one striking property per object
(a colour, a margin) in four forms: without selector, in the `default` state, `show` without
selector plus the rest in `default`, and both entries (Desktop's own form: its files repeat the
default state's properties in the entry without selector).

| Visual type | Stateful objects | States |
|---|---|---|
| `cardVisual` | `accentBar`, `border`¹, `cardCalloutArea`¹, `divider`, `fillCustom`, `glowCustom`, `image`, `label`, `layout`, `outline`, `padding`, `referenceLabelDetail`, `referenceLabelTitle`, `referenceLabelValue`, `shadowCustom`, `spacing`, `value` | `default` |
| `listSlicer` | `accentBar`, `background`, `expansionIcon`, `fillCustom`¹, `icon`, `image`, `label`, `outline`, `padding`, `selectionIcon`, `value` | `default`, `hover`, `press`, `selection`, `selection:selected`, `selection:unselected`, `expansion:expanded`, `expansion:collapsed`, `series:selectall`, `interaction:hover`, `interaction:press` |
| `advancedSlicerVisual` | as `listSlicer` without `expansionIcon` | as `listSlicer` |
| `actionButton` | `fill`, `glow`, `icon`, `outline`, `shadow`, `shape`¹, `text` | `default`, `hover`, `press`, `selected`, `disabled` |
| `pageNavigator`, `bookmarkNavigator` | `accentBar`, `fill`, `glow`, `outline`, `shadow`, `text` | as `actionButton` |
| `shape` | `fill`¹, `outline`¹, `text`¹ | as `actionButton` |

¹ Not in the theme schema; found by the render captures.

Render findings (property drawn: ✓, ignored: ✗):

| Object (property) | No selector | `default` state | `show` without selector + `default` | Both entries |
|---|---|---|---|---|
| card `value`, `label` (font colour), `accentBar`, `outline`, `fillCustom`, `cardCalloutArea`, `border`, `shadowCustom`, `padding`, `spacing`, `image` | ✗ | ✓ | ✓ | ✓ |
| card `overFlow` | ✓ | ✗ | – | ✓ |
| `listSlicer` `value`, `background`, `outline`, `fillCustom`, `selectionIcon`; `advancedSlicerVisual` `value`, `fillCustom`, `outline`, `padding` | ✗ | ✓ | ✓ | ✓ |
| `advancedSlicerVisual` `layout` (column count) | ✓ | ✗ | ✗ | ✓ |
| `actionButton` `fill`, `text` | ✗ | ✗ | ✓ | ✓ |
| `actionButton` `icon`, `outline`; `pageNavigator` `fill`, `text`; `shape` `fill`, `outline` | ✗ | ✓ | ✓ | ✓ |
| `shape` `text` | ✗ | ✗ | ✓ | ✓ |
| `shape` `rotation` | ✓ | ✗ | – | ✓ |
| `textSlicer` `inputTextBox`, `basicShape` `fill` | ✓ | ✗ | – | ✓ |
| `actionButton` `shape` (`tileShape` `oval`) | ✗ | ✓ | – | – |
| `shape` `shape` (`tileShape` `oval`; `rectangleRounded` with `rectangleRoundedCurve`) | ✓ | ✗ | – | ✓ |
| card `accentBar` with only `show: true`, without selector (no `default` entry) | ✓ (Desktop's default bar) | ✓ | – | – |

So a button's `fill` / `text` and a shape's `text` need their `show` toggle in the entry without
selector as well, while `show` alone is honoured there for every stateful object (a lone
`show: true` without selector draws a card's accent bar with its default colour and width; other
properties beside it are ignored). A shape visual's `shape` object (tile shape, rounding) is read
without selector, a button's from the `default` state. Not settled (no visible effect in any form,
left without selector by BIcraft): the card's and slicers' rounded corners
(`shapeCustomRectangle`), the navigators' `shape`, card `grid`, `divider` and
`glowCustom` with one callout, the navigators' `glow`, slicer `spacing`, and the other objects of
these types. A card's per-callout formatting is the entry with `{"metadata": "<queryRef>"}` alone
(a second measure's `value.fontColor` drawn); `{"metadata", "id": "default"}` and
`{"id": "<queryRef>"}` are ignored, so `visual.series` keeps writing the plain `metadata`
selector there.

BIcraft (`crates/engine/src/visual_states.rs`): `visual.format` `objects` items (and
`report.apply` `format.objects`) of a stateful object go to the `default` state entry, or the one
named by `state` (`"none"`: the entry without selector); `show` in the `default` state is also
written without selector (the third form above, which every row draws). `visual.props` /
`report.extract` list the stateful objects' formatting as `format.objects` items with their state.
`report.validate` warns (never rewrites) about what Desktop ignores: stateful-object properties
other than `show` only in the entry without selector; a button fill / text or shape text whose
`show` is not in the entry without selector; a card entry whose selector combines `id` with
`metadata`; and, for the objects found read without selector above (card `overFlow`,
`advancedSlicerVisual` `layout`, shape `rotation` and `shape`, every `textSlicer` and
`basicShape` object), properties only in the `default` state entry.

Applied check: `cargo xtask oracle-gen states` sets a sample of these properties through
`visual.format` twice, as BIcraft writes them and with `state: "none"`, captures Desktop's
rendering and checks the first is drawn and the second not (controls: `textSlicer`,
`basicShape`, drawn both ways). Desktop 2.158.1304.0: 15 of 15 stateful samples drawn as BIcraft
writes them and ignored without selector, both controls drawn.

## Open questions

- Text sizes: the theme schema caps every `fontSize` at 60, but Desktop renders an 80 pt card
  callout; the true maximum (per visual) is unknown, so none is enforced. The minimum of 8 is the
  schema's and was not checked against Desktop.
- Selectors per object remain our own reading, and the oracle sets no selector. State
  selectors of the stateful visuals: see "State selectors" (objects not listed there are written
  without selector; a few could not be settled).
- Rendering: the oracle checks what Desktop keeps, not how it draws it; a visual check per family
  needs an interactive session (see above).
- Data roles: see `visual-roles-bb.md` (black-box tests against Desktop 2.159). The oracle bound fields to every
  visual type by those roles and Desktop kept every binding; the "needs one of" rules were not
  checked.
