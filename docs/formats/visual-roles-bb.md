# Visual data roles (public documentation and black-box tests)

The data roles of Power BI's built-in visual types: each role's name, what it takes (columns,
measures or both), how many fields it takes, which roles form a drill-down hierarchy, which
combinations of field counts Desktop accepts, and which roles a visual needs to draw. Encoded in
`crates/visuals/src/roles_bb.rs`; read by the engine's binding checks (`crates/engine/src/roles.rs`).

Every fact below carries a source tag:

- **doc**: Microsoft's public documentation (URL in the source log);
- **bb:&lt;test&gt;**: a black-box test against Power BI Desktop 2.159.883.0 (Store edition), or
  2.159.1231.0 for the `gantt-*` tests, with the test id listed under [Tests](#tests).

The documentation pages and tests are listed in
[visual-roles-bb.sources.md](visual-roles-bb.sources.md).

## Method

BIcraft generates a report (a model with text columns `T1`…`T10`, numeric columns, and measures
`M1`…`M10`), Desktop opens it with its WebView2 DevTools port (the render-oracle convention of
`tools/render-oracle`), and the harness observes the rendered page. The port sends mouse and drag
events and reads the rendered DOM (visible text, element classes and ARIA labels).

Four kinds of tests:

| Test | What is done | What is read |
|---|---|---|
| **wells-A** | One empty visual per type, one page each. Text column `T<i>` is dragged from Desktop's Data pane into field well *i*, in well order. Then Desktop's Save button is clicked. | The wells (caption, fields shown, `supports-multiple-items` on the drop zone) after every drop, and the PBIX Desktop saved: the role name (`queryState` key) under which each field landed, and whether it was kept as a column or summarized. |
| **wells-B** | The same with measure `M<i>`. | Whether the well keeps a measure; the saved role names. |
| **wells-R** | wells-A in reverse well order. | Role names of wells another well blocks in the forward order. |
| **wells-C** | Per well, alone: `T1`…`T5` dragged in one after the other, the visual header read, the fields removed again. | How many fields the well keeps (it replaces the field when it takes one); whether the header offers drill-down buttons (several fields form a hierarchy); which other wells stop taking several fields. |
| **render** | Generated visuals with chosen fields per role (a column for a column role, a measure otherwise): each role alone, each pair, each role with 2 fields next to each other role, all roles with one field, all roles with one field and one role with 2 or 3. 12 visuals per page. | Each visual's state: **drawn** (data marks), **empty** (title only, nothing drawn), **placeholder** (the "select or drag fields" landing), **error** (the error glyph and "See details"; the details text is read). |

Kinds, from wells-A and wells-B:

- **Grouping**: the well refuses a measure; a text column stays a column.
- **Measure**: the well keeps a measure; a text column is summarized (Count, First…), or refused.
- **GroupingOrMeasure**: the well keeps a measure, and a text column stays a column.

A role whose well keeps only one field (wells-C) has a maximum of 1. A well that keeps 5 has no
maximum we know of (`None`); drill roles have none by definition.

A **drill role** is one whose fields Desktop saves with an `active` flag on the current level (the
PBIX files of wells-A/B/R); the chart axes among them also show drill-down buttons in the visual
header once they hold two fields (wells-C).

## Unknown role names

A field bound in a file to a role the visual type does not have is ignored: the visual draws as
if the field were absent (bb:pilot-bogus). Role names are case-sensitive: a visual whose roles
are spelled `category` / `y` is not drawn at all (bb:pilot-lower).

## Roles per visual type

Kinds: **G** grouping, **M** measure, **GM** either. Max: `1`, `2`, or `—` (none observed, 5 kept).
**D**: drill role. All rows are bb:wells-A (name, kind of a column), bb:wells-B (measure kept or
refused), bb:wells-C (max, drill buttons) and the `active` flags of the saved files, unless noted.

| Visual type | Roles (well order): kind, max, drill |
|---|---|
| `barChart`, `clusteredBarChart`, `clusteredColumnChart`, `columnChart`, `hundredPercentStackedAreaChart`, `hundredPercentStackedBarChart`, `hundredPercentStackedColumnChart`, `ribbonChart`, `stackedAreaChart` | `Category` G — D; `Y` M —; `Series` G 1; `Rows` G —; `Tooltips` M — |
| `lineChart`, `areaChart`, `lineClusteredColumnComboChart`, `lineStackedColumnComboChart` | `Category` G — D; `Y` M —; `Y2` M —; `Series` G 1 (bb:wells-R for line and area); `Rows` G —; `Tooltips` M — |
| `pieChart`, `donutChart` | `Category` G — D (the legend well); `Y` M —; `Series` G 1 (the details well); `Tooltips` M — |
| `funnel` | `Category` G — D; `Y` M —; `Tooltips` M — |
| `treemap` | `Group` G — D; `Details` G 1; `Values` M —; `Tooltips` M — |
| `waterfallChart` | `Category` G — D; `Breakdown` G 1; `Y` M 1; `Tooltips` M — |
| `scatterChart` | `Category` G — D (the values well); `X` GM — D; `Y` M 1 (a text column is refused); `Series` G 1; `Size` M 1; `Play` G 1; `Tooltips` M — |
| `map` | `Category` G — D; `Series` G 1; `Y` M 1 (latitude); `X` M 1 (longitude); `Size` M 1; `Tooltips` M — |
| `filledMap` | `Category` G — D; `Series` G 1; `Y` M 1; `X` M 1; `Tooltips` M — |
| `azureMap` | `Category` G — D; `Y` M 1; `X` M 1; `Series` G 1; `Size` M 1; `Tooltips` M —; `PathID` G 1; `PointOrder` G 1 (its well takes a field only once `PathID` has one) |
| `shapeMap` | `Category` G 1; `Series` G 1; `Value` M 1; `Tooltips` M — |
| `ganttChart` | `Task` G 1; `StartDate` M 1; `EndDate` M 1 (a date column summarized as earliest / latest, or a date measure; text columns and numeric measures refused); `Progress` M 1; `Category` G 3 (no `active` flag); `Columns` M —; `Tooltips` M —. Every well but `Task` takes a field only once `Task` has one (bb:gantt-A/B/C/R, [gantt.md](gantt.md)) |
| `gauge` | `Y`, `MinValue`, `MaxValue`, `TargetValue`: M 1 each; `Tooltips` M — |
| `kpi` | `Indicator` M 1; `TrendLine` G 1; `Goal` M 2 |
| `card` | `Values` M 1 |
| `cardVisual` | `Data` M —; `Rows` G —; `Tooltips` M — |
| `multiRowCard` | `Values` GM — |
| `tableEx` | `Values` GM — |
| `pivotTable` | `Rows` G — D; `Columns` G — D; `Values` M — |
| `slicer` | `Values` G — D |
| `listSlicer` | `Values` G — D; `Tooltips` M — |
| `textSlicer` | `Values` G 1 |
| `advancedSlicerVisual` | `Values` G 1; `Label` M 1; `Tooltips` M — |
| `decompositionTreeVisual` | `Analyze` M 1; `ExplainBy` G —; `Tooltips` M — |
| `keyDriversVisual` | `Target` GM 1; `ExplainBy` GM —; `Details` G — (the last two wells take a field only once `Target` has one, bb:wells-R) |
| `pythonVisual`, `scriptVisual` | `Values` GM — |
| `rdlVisual` | `parameterValues` GM — |
| `scorecard` | no well; a field makes Desktop show an error (bb:kinds-t03): takes no fields |
| `actionButton`, `aiNarratives`, `bookmarkNavigator`, `image`, `pageNavigator`, `qnaVisual`, `shape`, `textbox` | no well (bb:wells-A). A textbox with a field draws nothing and shows no error (bb:kinds-t04). The others were not tested with fields: **unknown**, not checked |

The `clusteredColumnChart` role names and kinds also match Microsoft's report authoring
documentation (doc: get-visual-capabilities).

## Accepted combinations

| Visual types | Combination | Evidence |
|---|---|---|
| bars family (first row above), combo charts, pie, donut | with a `Series` field, `Y` takes one field (Desktop names the legend or details well as the one with too many fields) | bb:wells-A (the values well stops taking several fields once the legend has one), bb:wells-C, bb:pilot-sery2, render `all, Y=2` → error |
| `lineChart`, `areaChart` | with a `Series` field, one value field in all, in `Y` or `Y2` | bb:wells-A (legend refused with `Y` and `Y2` filled), bb:wells-R (`Y` refused with legend and `Y2`), render `pair Y=2+Series`, `pair Y2=2+Series`, `all one` → error |
| `treemap` | with a `Details` field, `Values` takes one field | bb:wells-A, bb:wells-C |
| `funnel` | with a `Category` field, `Y` takes one field | bb:wells-A, bb:wells-R, bb:wells-C |
| `scatterChart` | with a `Category` field, `X` and `Y` each take one measure or summarized column | bb:kinds-s09, s10, s13, s14 → error ("remove Values or summarize the axes"); s01, s07 draw; without `Category`, raw columns draw (s03, s04, s05, s11) |
| `slicer`, `listSlicer` | several fields must come from one table or from tables linked by one chain of active relationships | bb:kinds-r01..r06, l01, l05, l06 draw; r07 (two dimensions of one fact), r08 (unrelated), r09 (inactive relationship), r10 (many-to-many), r11, r12, l02 → error ("fields that don't have a supported set of relationships") |
| maps | **unknown**: map and filled map visuals are switched off in this Desktop's security options and Azure Maps needs a sign-in; every map test showed that message instead (bb:kinds-m01..m04, f01, f02, a01..a04). No combination is checked |

Known gap: in a scatter chart with no `Category`, several `X` columns with a raw `Y` column show
an error (bb:kinds-s12), but `X` is a drill role (one field per level in the checks), so this is
not reported.
| `scatterChart` | a play axis asks the same as details: X and Y summarized | render `pair X+Play`, `pair Y=2+Play`… → error ("remove the play axis or summarize the axes") |
| `keyDriversVisual` | needs a `Target` and an `ExplainBy` field; `Details` only with a summarized `Target` | render `single Target`, `single ExplainBy` → a "no field in …" message; `pair Target+Details` and `all one` with a raw `Target` column → error |
| `listSlicer` | needs a `Values` field | render `single Tooltips` → error ("not enough columns") |
| `waterfallChart` | a `Breakdown` needs a `Category` | render `pair Breakdown+Y` → empty chart |
| every role with max 1 (or 2) | one more field is an error naming the well | render `all, R=2`, `pair R=2+…` for every such role: error ("too many columns in …"); roles without a max drew with 2 and 3 fields |

## Roles a visual needs to draw

Render tests, each role alone and each pair: the visual draws data only with a field in one of
the roles listed, else it shows its title and nothing else (no error). These are warnings in
BIcraft.

| Visual type | Needs a field in |
|---|---|
| bars family, pie, donut, funnel, gauge, waterfall | `Y` (an axis, legend, small multiples or tooltips alone draw nothing; gauge min/max/target alone show "(Blank)") |
| line, area, combo charts | `Y` or `Y2` |
| `treemap` | `Values` |
| `ganttChart` | `Task` (Learn: the only required field; bb:gantt-R: no other well takes a field before it). Not render-tested |
| `kpi` | `Indicator` and `TrendLine` (each alone, or with `Goal`, draws nothing) |
| `cardVisual` | `Data` or `Rows` |
| `advancedSlicerVisual`, `listSlicer` | `Values` |
| `decompositionTreeVisual` | `Analyze` (without it Desktop shows a "no field to analyze" error) |
| `scatterChart`, `pivotTable`, `tableEx`, `multiRowCard`, `card`, `slicer`, `textSlicer` | nothing particular: every role alone draws something |
| `shapeMap` | **unknown**: it draws its base map with or without data |
| maps | **unknown** (see above) |

## Tests

Test sets, run on Power BI Desktop **2.159.883.0** (Store) on 2026-10-28 except the `gantt-*` row; the generated
reports and the observations stay outside the repository (in the maintainers' local work folder; Desktop's rendered pages
are not committed):

| Id | Content |
|---|---|
| pilot | 12 `clusteredColumnChart` visuals (`ok1`, `bogus`, `cat2`, `empty`, `ycol`, `catm`, `onlyy`, `onlycat`, `ser2`, `sery2`, `ysum`, `lower`) |
| wells-A, wells-B, wells-C, wells-R | 48 types, one empty visual per page; see [Method](#method) |
| kinds | 48 visuals: scatter `s01`…`s14`, maps `m01`…`m04`, `f01`, `f02`, `a01`…`a04`, slicers `r01`…`r12`, `l01`…`l06`, others `t01`…`t06` |
| gantt-A, gantt-B, gantt-C, gantt-R | Desktop **2.159.1231.0** (Store), 2026-11-23: `ganttChart` only, one empty visual per page written by BIcraft or inserted from the gallery; A: text columns, with a date column or date measure where the text column was refused; B: measures; C: up to five fields per well with `Task` filled; R: reverse well order. Files and screenshots kept outside the repository |
| render | 1229 visuals over 35 types (all with wells except `azureMap`, `pythonVisual`, `scriptVisual`, `rdlVisual`): each role alone, each pair, each role with 2 fields next to each other role, all roles with one field, all roles with one field and one role with 2 or 3 |

The unit tests in `crates/visuals/src/tests_bb.rs` and the role tests of the engine
(`crates/engine/src/roles.rs`, `roles_kind_tests.rs`) assert these observations.

## Unknowns (checked permissively)

- Map, filled map and Azure map combinations and required roles: never drawn here.
- `shapeMap` required roles.
- Role checks of `actionButton`, `aiNarratives`, `bookmarkNavigator`, `image`, `pageNavigator`,
  `qnaVisual`, `shape`, `textbox` (no well; Desktop's behaviour with fields not tested, except
  the textbox which ignores them). The engine's own Q&A and AI narratives warnings stay.
- Python, R and paginated-report visuals: wells known, combinations not tested (they need
  Python, R or the service).
- Several `X` columns with a raw `Y` column in a scatter chart (an error in Desktop, not reported).
- Hidden roles: a role that has no well cannot be seen by these tests; a field bound to it is
  reported as an unknown role.