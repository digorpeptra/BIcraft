# The BIcraft Spec visual

Sources: our own format. The visual is a custom visual of BIcraft's, built as
[pbiviz.md](pbiviz.md) "Visual projects" describes (Microsoft Learn's custom visual pages and the
MIT `powerbi-visuals-api` 5.3.0 types: `IVisual`, `VisualUpdateOptions`, the categorical data
view, `FormattingModel`); the drawing is SVG in the visual's sandbox. ROADMAP M11.32.

An agent draws a chart without writing JavaScript: it writes a JSON **spec** (marks, encodings,
scales, axes, legend) and `visual.set_spec` stores it in a visual of type **BIcraft Spec**.
Power BI draws it with the visual's script; BIcraft's renderer (`report.render`, PDF export, the
app's canvas) draws the same layout natively.

## Why a spec and TypeScript (not Rust compiled to WebAssembly)

M11.31 showed that WebAssembly runs in the sandbox, so a Rust drawer compiled to wasm was
possible. It was not chosen: the `.wasm` bytes would have to be built with the
`wasm32-unknown-unknown` target and committed, or built at the user's side, and neither is
reproducible from the repository with the tools a user has (the `bicraft-cli` binary carries swc,
not a Rust compiler). The visual's script is TypeScript in the repository, compiled at run time
by BIcraft's own bundler (`bicraft_tsbuild`, no Node), so the package is rebuilt from source by
every `bicraft-cli` binary. The layout exists twice, in `crates/render/src/spec.rs` and
`crates/engine/src/spec_visual/spec.ts`, kept equal by golden tests (below).

## The visual

| | |
|---|---|
| guid (visual type) | `bicraftSpecVisual7F3A9C2E5B1D4E6F8A0C2B4D` |
| display name | BIcraft Spec |
| version | 1.5.1.0 (raised with every change of its TypeScript, of the runtime it bundles or of its package icon; 1.4.1.0 changed only the icon, G111; 1.5.0.0 adds the text cards, a series for every bound measure and numbers in the report's language, G119; 1.5.1.0, 2026-11-27, only the runtime's scaling commas after the decimals) |
| data roles | `category` (Grouping, at most 1) and `values` (Measure, at most 16) |
| data view | categorical: categories from `category` (top 1,000), values bound to `values` |
| format objects | `spec`, property `json` (text): the spec; since 1.5.0.0 the text cards `labels`, `categoryAxis`, `valueAxis`, `legend` ("Text" below); `style` (the look) |
| sources | `src/visual.ts` (the `IVisual` class: data view in, SVG out), `src/spec.ts` (parsing and layout), `src/runtime/*.ts` (BIcraft's visual runtime, [community-visuals.md](../community-visuals.md) "Runtime"), `style/visual.css` (with the runtime's styles) |
| capabilities | also `privileges: []`, `tooltips` (default, canvas), `supportsLandingPage`, `supportsEmptyDataView`, `supportsKeyboardFocus`; `supportsHighlight` since 1.4.0.0 |

`visual.set_spec` builds the package from these sources once per process and embeds it as
`visual.import_custom` does (`CustomVisuals/<guid>/…` and a `CustomVisual` resource package);
a report that holds another build of it gets this one. `visual.scaffold {template: "spec"}` writes
the same sources as a project to edit (then a new guid, so it is another visual).

The spec is written as the PBIR text literal of `objects.spec[0].properties.json` (the compact
JSON the command was given, at most 32 KB). Power BI hands it to the visual as
`dataView.metadata.objects.spec.json`. The format pane shows it in a text area (card Spec;
Spécification in French since 1.1.3.0). Since 1.1.3.0 the roles and the format object carry a
`displayNameKey` with English and French `stringResources` ([bicraft-visuals.md](bicraft-visuals.md)
"Names in English and French"); `visual.scaffold {template: "spec"}` writes them as
`stringResources/en-US/resources.resjson` and `stringResources/fr-FR/resources.resjson`.

## The spec

```json
{
  "version": 1,
  "x": {"scale": "band"},
  "y": {"title": "Amount"},
  "marks": [
    {"type": "bar", "y": "value", "name": "Sales", "label": "value"},
    {"type": "line", "y": "value.1", "name": "Target", "color": "#E07A1F"}
  ]
}
```

Unknown keys are refused, with their place (`marks[1].colour: unknown key`). Top level:

| Key | Value | Default |
|---|---|---|
| `version` | `1` | 1 |
| `$schema` | any text (ignored) | |
| `padding` | 0–64 px around the chart | 8 |
| `palette` | 1–32 colours `#RRGGBB` | the report theme's data colours, else BIcraft's palette ([visual-look.md](visual-look.md)); under the Minimal preset without a theme the earlier eight (`#1F6F8B`, `#E07A1F`, `#5A9E3A`, `#C23B5A`, `#7A5BA8`, `#2E9C9C`, `#B08A1E`, `#6B6B6B`) |
| `style` | `preset` `modern` \| `minimal` \| `bold`, `palette` `theme` \| `bicraft`, `rounded`, `gradient`, `shadow`, `animation` ([visual-look.md](visual-look.md)) | Modern: rounded bars from their baseline, gradients, ringed points, bold value labels, the entry animation |
| `x` | axis object: `scale` `band` \| `linear` \| `time`, `axis`, `title`, `grid`, `min`, `max`, `zero`, `padding` (band, 0–0.9) | band, axis, no grid, no zero, padding 0.2 |
| `y` | axis object: `scale` `linear`, `axis`, `title`, `grid`, `min`, `max`, `zero` | axis, grid, zero |
| `legend` | `true` / `false` | shown when a mark has a `name` (else the categories of a mark coloured by category) |
| `text` | the text settings of `labels`, `x`, `y`, `xTitle`, `yTitle`, `legend` ("Text" below) | the look's sizes and inks |
| `marks` | 1–16 marks, drawn in order | |

A mark:

| Key | Value | Default |
|---|---|---|
| `type` | `bar`, `line`, `point`, `area`, `text`, `rule`, `rect` | |
| `name` | text (the legend) | |
| `x`, `y`, `x2`, `y2` | a field, or `{"field": …}`, or `{"value": number \| text}` | `x`: `category`, `y`: `value` (a rule: none) |
| `color` | `#RRGGBB`, `{"field": "category"}` (the palette), `{"field": "value.N", "range": [c0, c1]}` | the palette's colour at the mark's index |
| `size` | a number (point radius, line or rule width, text size) or `{"field": "value.N", "range": [a, b]}` | point 4, line 2, rule 1, text 11 |
| `label` | a field written at each value (bar, line, area, point); the text of a `text` mark | |
| `opacity` | 0–1 | 1 (area 0.35) |
| `decimals` | 0–6, the label's decimals | none for whole numbers, else 2 |

Fields: `category` (the `category` role's value), `value` (the first field of `values`),
`value.N` (the field at index N of `values`, N from 0 to 15).

Rules checked by both parsers: `y` and `y2` are measures or numbers; on a band x scale, x is
`category` or a category's text; a measure on x needs the linear scale; text on a time scale is an
ISO date (`2024-01-31`, optionally `T13:45[:10[.5]]`, a zone suffix ignored); bars need the band
scale; a rect on a linear or time scale needs `x2`; a rule has `x` (vertical) or `y` (horizontal),
not both; a text mark has a `label`. Texts are at most 200 characters, without control
characters; numbers stay within ±10¹⁵.

## Layout

Both drawers compute the same primitives (`rect`, `line`, `path`, `poly`, `circle`, `text`, in
the visual's pixels, rounded half up to hundredths):

- the legend row (18 px) at the top, then 6 px more when the y axis is shown, so the top tick
  label (10 px, centred on its tick) stays below the legend; the y axis takes 36 px on the left
  and its title 14 px; the x axis 18 px at the bottom and its title 14 px; nothing is drawn when
  the plot would be under 8 px;
- linear domains include `zero` and the data, then `min`/`max`, are extended to a "nice" step
  (1, 2 or 5 × 10ⁿ for about 5 ticks; 4 on a time scale) and carry at most 50 ticks; an empty
  domain is 0–1;
- a band scale gives each row `width / rows`, the bar `1 − padding` of it; several bar marks
  share the band side by side; bars and areas stand on `y2`, else 0, kept inside the domain;
- a missing value breaks a line or area; a line run of one point draws no line;
- band labels skip rows so the estimated label widths fit; text widths are estimated as
  `characters × size × 0.55` (no font is measured, so both drawers agree);
- an x tick label (band, linear or time) stays centred on its tick unless its estimated width
  would pass an edge of the visual: then it moves inwards to touch that edge (a label wider than
  the visual starts at 0), so the first and last labels are never cut;
- numbers are written by our own code (half away from zero, `,` thousands in English, a no-break
  space in French: "Numbers in the report's language" below); dates as
  `YYYY-MM-DD`, with ` HH:MM` when the tick step is under a day;
- since 1.1.3.0 (2026-11-24) a date category with a date format string (the category column's
  `source.format`; in BIcraft the snapshot column's `fmt`) is written with it in the host
  locale's language (the renderer: the UI language), English or French, in band labels, data
  labels and a legend by category, and so are time-axis ticks of a day or more
  ([bicraft-visuals.md](bicraft-visuals.md) "Dates in labels"); the golden data form takes it as
  `categoryFormat` and `locale`.

Category values: text, numbers, or dates (a `Date` from Power BI is read on its own clock; in
BIcraft a date column's values). The order is the data view's.

## Series

Since 1.5.0.0 a spec that draws only the first measure as one plain series draws every bound
measure the same way, as Power BI's clustered column, line and area charts draw one series per
measure: when more than one measure is bound and every mark is a bar, line, area or point whose
`y` is `value`, whose x is the category (band or time scale), with no `x2`, a `y2` that is a
number or none, a fixed colour or none, a fixed size or none and a label of `value` or none, the
marks are repeated for `value.1`, `value.2`… (at most 64 marks). Each series takes the palette's
colour at its index (the first keeps a colour the spec gives) and its first mark is named after
the measure (the data view's display name; `names` in the golden data form), so the legend lists
the measures; the bars of the series stand side by side in each band. The `bar`, `line` and
`area` templates are such specs, so a scatter or column chart with two measures switched to the
Spec visual (`visual.set_type`), or a Spec visual given a second measure in Desktop, draws both.

A spec that reads fewer measures than are bound and cannot be repeated (colours or sizes by value
or category, a linear x scale, rules or text marks at a measure) draws as written (a rule at a
fixed value reads no measure: it is drawn once and does not stop the repeat) and says so on a first line in
the tick size: "1 of 2 measures not drawn: the spec does not read them" (French: "1 mesure(s) sur
2 non dessinée(s) : la spécification ne les lit pas"); the plot starts 14 px lower. Measures are
never dropped silently.

## Text

Since 1.5.0.0 the spec's `text` key and the format pane's text cards set the font and the
numbers of each text element. `text` holds an object per element: `labels` (value labels), `x`
and `y` (the axes' tick labels), `xTitle`, `yTitle`, `legend`. Each takes:

| Key | Value | Default |
|---|---|---|
| `show` | `true` / `false`, over the spec's `axis`, `title`, `legend` and the marks' `label` | the spec's |
| `size` | 6–60 px | the look's: ticks 11, names and titles 12, values 11 (Minimal 10 / 11 / 10, Bold values 12) |
| `family` | a CSS family list (at most 200 characters) | the visual's own |
| `color` | `#RRGGBB` | the look's text ink (value labels: their ink, or the ink on their chip) |
| `bold`, `italic`, `underline` | `true` / `false` | the look's weight, upright, plain |
| `units` (`labels`, `y`) | 0 automatic, 1 none, 1000, 1000000, 1000000000, 1000000000000 | none |
| `decimals` (`labels`, `y`) | 0–6 | value labels: the mark's `decimals`, else none for whole numbers and 2 otherwise; ticks: from the step |

- `labels.show: true` gives every bar, line, area and point a label of its `y` measure; `false`
  removes them (text marks keep theirs). `xTitle.show: true` without a spec title writes the
  category field's name, `yTitle.show: true` the drawn measures' names joined by commas.
- Automatic units follow the largest absolute value of the labelled measure (labels) or of the
  domain's ends (ticks): thousands from 1,000, millions from 1,000,000 and so on. The suffixes are
  `K`, `M`, `bn`, `T` in English and `k`, `M`, `Md`, `Bn` after a no-break space in French (our
  choice of the usual French abbreviations). With units and no decimals, a whole quotient has
  none and another 2.
- Room grows with the text: the legend row by `legend.size − 12`, the y axis to
  `36 × y.size / 11`, the x axis by `x.size − 11` (its labels move down by half of that), each
  title by its size less 12; at the default sizes the layout is unchanged. Widths are still
  estimated from character counts (the family does not change them), so both drawers agree, and
  a small visual still draws until its plot is under 8 px.
- Primitives carry `family`, `italic` and `underline` only when set, and `weight` 700 for bold;
  the TypeScript writes them as SVG `font-family`, `font-style`, `text-decoration` and
  `font-weight`; the renderer draws the family when it has the font, italic and underline.

The format pane's cards (built with the formatting model of the `powerbi-visuals-api` types: a
`FontControl` row for family, size, bold, italic and underline, a colour picker, a drop-down on a
`labelDisplayUnits` property whose items Desktop supplies, a number box for the decimal places,
and the card header's show switch) write the property names Power BI's own charts use:

| Card | Object | Properties | Element |
|---|---|---|---|
| Data labels | `labels` | `show`, `color`, `fontFamily`, `fontSize`, `bold`, `italic`, `underline`, `labelDisplayUnits`, `labelPrecision` | `labels` |
| X axis | `categoryAxis` | `show`, `labelColor`, `fontFamily`, `fontSize`, `bold`, `italic`, `underline`; `showAxisTitle`, `titleColor`, `titleFontFamily`, `titleFontSize`, `titleBold`, `titleItalic`, `titleUnderline` | `x`, `xTitle` |
| Y axis | `valueAxis` | as X axis, with `labelDisplayUnits` and `labelPrecision` | `y`, `yTitle` |
| Legend | `legend` | `show`, `labelColor`, `fontFamily`, `fontSize`, `bold`, `italic`, `underline` | `legend` |

Their settings win over the spec's `text`, as the Style card's win over its `style`; a value the
layout cannot draw (a size outside 6–60, units outside the list) is ignored by both readers
(`runtime/textsettings.ts` and `crate::visual_runtime::text_of`). The objects and their reading
live in the shared runtime, so BIcraft's other visuals can take the same cards.

## Numbers in the report's language

Since 1.5.0.0 value labels, y ticks, linear x ticks and number categories are written in the
culture of the host locale (the renderer: the UI language): `416,198.52` in English,
`416 198,52` in French (a no-break space groups the thousands), as Waterfall+'s axis already did
and as the runtime's `formatNumber` writes tooltips.

**Seen in Desktop** (2.160.1151.0, French UI, 2026-11-25): a PBIX written by BIcraft with a
scatter chart over two measures switched to the Spec visual (`visual.set_type`) and a Spec visual
over one measure, both with the data labels' text size set to 18 through `visual.format`
(`labels.fontSize`), the second also with thousands (`labels.labelDisplayUnits` 1000). Desktop
drew the first as two series side by side with a legend naming both measures, labels at the
larger size and numbers in French (`416 198,52`, y ticks `800 000`); the second with labels
`416,20 k`. Only the report canvas was captured: the Format pane's new cards were not opened
there.

## Runtime

Since 1.1.0.0 (ROADMAP M12.2) the visual imports BIcraft's visual runtime
([community-visuals.md](../community-visuals.md) "Runtime"); 1.1.1.0 (2026-11-23) bundles the
runtime that also selects units made of several category values, for BIcraft's chart visuals
([bicraft-visuals.md](bicraft-visuals.md)), and exports the layout helpers they reuse from
`spec.ts`; the Spec visual's own behaviour and layout are unchanged:

- **Tooltips.** Hovering a bar, point, text, rule segment, value label or a line's or area's
  vertex shows the host's tooltip with the category and every value of that row, written with the
  fields' format strings in the report's language (English or French).
- **Selection and cross-filtering.** A click selects the row's category value through the host's
  selection manager (one selection id per category value), so the page's other visuals filter or
  highlight; Ctrl (⌘) adds or removes; a click on the only selected point or on the background
  clears; points of other rows are drawn at opacity 0.4, and a selected vertex of a line or area
  gets a ring. A bookmark's selection is restored (`registerOnSelectCallback`). Without a field
  in `category` nothing is selectable (tooltips still show).
- **Context menu.** Right click (or the Menu key, Shift+F10) opens the host's menu for the point,
  or for the visual on the background.
- **Keyboard.** Each row's first mark is a tab stop with an accessible name (its tooltip lines);
  Enter or Space selects, Ctrl+Enter adds, arrows, Home and End move between rows, Escape clears.
- **High contrast.** With `host.colorPalette.isHighContrast`, everything is drawn in the
  foreground colour over the background (areas as outlines), selected points take
  `foregroundSelected` and dimmed points become outlines.
- **Landing page.** With no field in either role, the visual shows its name and which roles to
  fill (in English or French); a visual with fields and no spec says so, and a spec it cannot
  draw shows the reason. A Spec visual BIcraft adds (`visual.add`, `visual.set_type`,
  `report.apply`) starts with `visual.spec_template`'s `bar` (2026-11-24; from 2026-11-24 to
  1.5.0.0 two values or more took the `combo`, whose second measure, a line on the bars' scale,
  could look missing), so only a file written elsewhere shows the missing-spec message.

Since 1.4.0.0 the visual declares `supportsHighlight`: another visual's selection highlights it.
A bar or rect from zero on a value field draws the full bar at 40 % and its highlighted part
over it; the other marks and labels of a row none of whose values is highlighted are dimmed
([bicraft-visuals.md](bicraft-visuals.md) "Highlights"). Before, the selection filtered the data,
which was then laid out as BIcraft's renderer lays out the same data. Selection,
tooltips, focus and high contrast change only attributes of the drawn elements (opacity, strokes,
focus) and add invisible hit circles at line and area vertices; the primitives are the layout's,
unchanged (`layout(spec, data, w, h, hits)` only lists which primitive belongs to which row).
BIcraft's renderer draws the visual unselected.

## Checks

- `crates/render/src/spec_tests.rs`: parsing, refusals with their place, hostile sizes, the
  number and date formatting.
- `crates/engine/tests/report/spec_visual_tests.rs`: 22 cases (17 layouts, 5 refused specs) until
  1.4.1.0; 1.5.0.0 adds three series cases (two named measures as clustered bars, three as lines,
  the notice of a spec that cannot repeat), four text cases (big labels with units and a family,
  titles and a French legend, everything hidden, big text on a 90 × 70 visual that still draws)
  and three refused text settings.
  Three label dates with their category's format string (since 1.1.3.0).
  The Rust layout is held to `spec_golden.json` (`BICRAFT_BLESS=1` rewrites it); the visual's
  `spec.ts`, compiled by `bicraft_tsbuild`, runs under Node when Node is on the PATH and must give
  the same primitives (to 0.011 px) and the same refusal messages. Without Node the comparison
  prints a note and passes: it is a development check, Node is never needed by BIcraft. In every
  layout the legend ends above the top y tick label and each x tick label lies within the
  visual's width.
- `crates/engine/tests/report/visual_runtime_tests.rs`: the packaged capabilities validate against
  the API's `schema.capabilities.json`; the runtime's TypeScript runs under Node (where installed)
  with a stand-in host (Runtime above).
- `crates/engine/tests/report/builtin_visuals_tests.rs`: the Spec visual is built in by default
  (ROADMAP M12.4): `visual.add {visual_type: "bicraftSpec"}` and `visual.set_type` embed it on
  first use, an older embedded build is replaced and a newer one kept (also by
  `visual.set_spec`), and a save drops the package once no visual uses it
  (docs/community-visuals.md "Built in by default").
- `spec_visual_tests.rs`: `visual.set_spec` embeds the package (listed by `report.custom_visuals` as
  registered), converts a column chart's fields, the snapshot carries `spec`, `report.render` and
  `report.export_pdf` report the visual `drawn`, and the package, type and spec survive PBIP and
  PBIX round trips; refused specs leave the report unchanged.

### Seen in Desktop

Black-box run on Power BI Desktop 2.159.1231.0 (Store edition), 2026-11-23. A PBIX BIcraft wrote
(table `T` with categories a, b, c and two measures, table `D` with three `Date` days and a
measure) held two column charts that `visual.set_spec` turned into BIcraft Spec visuals: a bar
and line combo with value labels, and a line with points on a time scale over the `Date`
category. Desktop opened it and drew both; the rendered SVG of each sandbox frame and a
screenshot of the report view were read through the WebView2 DevTools port. Nothing was clicked,
and none of Desktop's own scripts were read.

- Both visuals are drawn with the same marks, ticks and labels as `report.render`: bars 100, 200
  and 442 with their labels, the Target line, y ticks 0 to 500 and the title Amount; the `Date`
  category on a time scale with ticks 2025-12-17 to 2026-03-20 and three points.
- Desktop shows its default title above each visual ("Total et Target par K" in a French
  Desktop), where a built-in chart has it, and draws the spec below it. The renderer draws the
  same default title (switched off by `title.show: false`) and lays the spec out below it.
- That run (visual 1.0.0.0) showed two defects of both drawers: the legend row overlapped the top
  y tick label, and the last time tick label (2026-03-20) was cut at the right edge. Version
  1.0.1.0 reserves the space below the legend and keeps x tick labels inside the visual (Layout
  above). Reopened the same day, 1.0.1.0 shows the legend above the top tick (500, 30) and the
  last date tick (2026-03-20) whole.
- Selecting the visual and opening Format visual (input events only) shows the card Spec with
  the text area Spec (JSON) holding the spec `visual.set_spec` wrote.

- Editing the spec in that text area (the line colour, input events) redraws the visual with the
  new colour; Desktop's save stores the edited spec as the same PBIR literal form BIcraft writes
  (line endings aside), and BIcraft renders the saved project with it. The same save writes the
  `CustomVisualMetadata` item's name as its path, so the package item is named
  `<guid>.pbiviz.json` ([pbix-package.md](pbix-package.md), "Custom visuals").

Version 1.2.0.0 (2026-11-24) draws with BIcraft's look ([visual-look.md](visual-look.md)): the
`style` key, the report theme's colours, rounded and shaded bars, the entry animation and the
hover emphasis; Minimal draws what 1.1.3.0 drew. Not yet seen in Desktop.

Version 1.3.0.0 (2026-11-24, the look's second pass) adds the Style card to its format pane
(format object `style` with the look's eight options in `capabilities.json`; its settings win
over the spec's `style`, which also takes `card` and `pills`), the card behind the plot, value
chips, the look's tick, name and value text sizes (10 / 11 / 10 px in Minimal as before), a
2.5 px combo line with a ringed marker on each of up to 40 points, and the report theme's
background (`layout`'s `theme` takes `{colors, background}` or the colour list). Value labels
sit on their mark's side (above a bar standing, below one hanging, above a point) at the look's
distance. Minimal draws what 1.2.0.0 drew. Not yet seen in Desktop.

Version 1.1.2.0 (2026-11-24) bundles the runtime that keeps the keyboard focus across the redraw
Desktop makes after a selection; its tooltips, cross-filtering, context menu and keyboard focus
were seen in Desktop 2.159.1231.0 that day ([community-visuals.md](../community-visuals.md)
Runtime, "Seen in Desktop"); its landing page and high contrast were not checked there. The same
run measured the visual's frame under Desktop's classic title at (5, 21), the visual's size less
10 × 26 px; the renderer now lays the spec out there (it was 4 px lower).
