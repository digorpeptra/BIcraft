# BIcraft's own chart visuals

Sources: our own format. Each visual is a custom visual of BIcraft's, built as
[pbiviz.md](pbiviz.md) "Visual projects" describes (Microsoft Learn's custom visual pages and the
MIT `powerbi-visuals-api` 5.3.0 types: `IVisual`, `VisualUpdateOptions`, the categorical data
view, `FormattingModel`, `ISelectionIdBuilder`); the drawing is SVG in the visual's sandbox.
ROADMAP M12.3 (first pack, part A), chosen by the needs ledger of
[community-visuals.md](../community-visuals.md). Names, drawing and code are ours; no existing
visual's code, listing text or image was used.

`visual.set_bicraft_visual {page, visual, kind, options?}` turns a visual into one of them.
Power BI draws it with the visual's TypeScript; BIcraft's renderer (`report.render`, PDF
export, the app's canvas) draws the same layout natively, as for the
[Spec visual](spec-visual.md). Since M12.4 they are built in by default
([community-visuals.md](../community-visuals.md) "Built in by default"): the Visualizations pane
lists them, `visual.add` and `visual.set_type` take their names (`bicraftHistogram`,
`bicraftBullet`, `bicraftBoxPlot`, `bicraftDumbbell`), and every path embeds the package through
the registry (missing or older replaced, newer kept).

## The visuals

| kind | Display name | guid (visual type) | Roles (kind, most fields) |
|---|---|---|---|
| `histogram` | BIcraft Histogram | `bicraftHistogram4C8E1A7D3F9B2E6A0D5C` | `values` (Grouping, 1): the numeric column; `frequency` (Measure, 1): each value's weight |
| `bullet` | BIcraft Bullet | `bicraftBullet9A3D6F1C8E2B5A7D4F0E` | `category` (Grouping, 1); `value` (Measure, 1); `target` (Measure, 1); `bands` (Measure, 4): band thresholds; `maximum` (Measure, 1) |
| `box_plot` | BIcraft Box Plot | `bicraftBoxPlot2E7B4D9A1F6C3E8B5D0A` | `category` (Grouping, 1): one box per value; `sample` (Grouping, 1): the items whose values make the distribution; `value` (Measure, 1) |
| `dumbbell` | BIcraft Dumbbell | `bicraftDumbbell6F1A8C3E9D2B7F4A0C5E` | `category` (Grouping, 1); `start` (Measure, 1); `end` (Measure, 1) |

The histogram, bullet chart and box plot are version 1.3.2.0, the dumbbell 1.3.4.0 (raised with every change of its TypeScript or of what it
bundles; 1.3.1.0 and 1.3.3.0 changed only the package icon, G111; the last raise, 2026-11-25, bundles the runtime's text settings and the
Spec layout's series and numbers, G119, and changes nothing these visuals draw). Since 1.1.0.0 they draw with BIcraft's look ([visual-look.md](visual-look.md)): the
format object `style` also holds `preset`, `palette`, `rounded`, `gradient`, `shadow`, `card` (since 1.2.0.0), `pills` (since 1.2.0.0) and
`animation` (the Style card), and the colours follow the report theme; the Minimal preset draws
what 1.0.3.0 drew. Since 1.2.0.0 (the look's second pass): the card and value chips, larger
tick, name and value text, dark-background tokens, histogram and bullet labels on the look's
chips, the dumbbell's stem in the mix of its two ends' colours. Its package holds `src/visual.ts` (the entry: the visual's roles, tooltips and format
pane), `src/<kind>.ts` (the layout), `src/common.ts` (data, options and drawing helpers shared by
the four), `src/host.ts` (the `IVisual` they share), `src/spec.ts` (the Spec visual's number
formatting, ticks and primitives, reused) and `src/runtime/*.ts` (BIcraft's visual runtime,
[community-visuals.md](../community-visuals.md) "Runtime"); its capabilities also carry
`privileges: []`, the default and canvas tooltips, `supportsLandingPage`,
`supportsEmptyDataView` and `supportsKeyboardFocus`, and since 1.3.0.0 `supportsHighlight`
(another visual's selection highlights them: "Highlights" below; before, it filtered them). The
icon is ours: each visual's own drawing (`bicraft_visuals::icons`, the UI icons' 24 × 24 grid),
rasterized at 20 × 20 into `assets/icon.png` (docs/community-visuals.md "Icons").

### Data views

| kind | Categorical mapping |
|---|---|
| `histogram` | categories: `values` (top 30,000: each distinct value one row); values: `frequency` |
| `bullet` | categories: `category` (top 100); values: `value`, `target`, `bands`, `maximum` |
| `box_plot` | categories: `category` and `sample` (top 30,000 rows); values: `value` |
| `dumbbell` | categories: `category` (top 200); values: `start`, `end` |

A histogram over a column without a frequency counts each distinct value once: bind a count
measure (`COUNTROWS`) to `frequency` to count rows.

### Format options

Objects and properties of the format pane (and of `visual.set_bicraft_visual`'s `options`,
checked: an unknown option or a value out of range is refused):

| kind | Option | Type | Default |
|---|---|---|---|
| `histogram` | `bins.count` | whole number 1–200 | 10 |
| | `bins.size` | number ≥ 0 (0: from `bins.count`) | 0 |
| | `bins.cumulative` | bool: each bar counts its bin and the bins before it | false |
| | `colors.fill` | colour | `#1F6F8B` |
| | `labels.show` | bool | false |
| `bullet` | `bands.first`, `bands.second`, `bands.third` | % of the target, 0–1000 (0: no band) | 60, 80, 100 |
| | `colors.value`, `colors.target` | colour | `#1F6F8B`, `#333333` |
| | `labels.show` | bool | false |
| | `style.vertical` | bool: one column per row | false |
| `box_plot` | `box.mean`, `box.outliers` | bool | true, true |
| | `box.horizontal` | bool: the boxes as rows | false |
| | `colors.fill` | colour | `#1F6F8B` |
| `dumbbell` | `style.slope` | bool: a slope chart instead of dumbbells | false |
| | `colors.start`, `colors.end`, `colors.line` | colour | `#1F6F8B`, `#E07A1F`, `#A6A6A6` |
| | `labels.show` | bool | true |

The values are stored as PBIR literals of the visual's `objects` (`12D`, `true`, a fill's
`{solid: {color}}`), the entry without selector. The snapshot gives them to the renderer as
`bicraft_visual: {kind, options: {object: {property: value}}}`; a value of the wrong type or
out of range falls back to its default in both drawers.

### Names in English and French

Since 1.0.3.0 (2026-11-24) every data role, format card and format option of
`capabilities.json` carries a `displayNameKey` (`Role_<role>`, `Object_<object>`,
`Object_<object>_<property>`; since 1.1.0.0 also each item of a drop-down list,
`Object_<object>_<property>_<value>`) next to its English `displayName`, and the package's
`.pbiviz.json` carries `stringResources: {"en-US": {key: text}, "fr-FR": {key: text}}`, the
localization Microsoft Learn describes ("Add the locale in Power BI for Power BI visuals":
`displayNameKey` in capabilities.json, `stringResources/<locale>/resources.resjson` in the
project), packaged where the MIT `powerbi-visuals-webpack-plugin` (`src/extractor/localization.js`,
`getVisualConfig` in `src/index.js`, commit 8abcf7a5) puts them. The French is BIcraft's own,
translated from the meaning of the English (`FR` in `crates/engine/src/visual_runtime.rs`, shared
by the nine visuals; a name without its French fails the build). The format pane's cards were
already drawn in the host's language by `getFormattingModel`. Not yet seen in a French Desktop.

### Dates in labels

Since 1.0.3.0 a date category (bullet and dumbbell row names, box names and the box tooltip, a
row tooltip's text cell) is written with its column's format string (the data view's
`source.format`; in the renderer the snapshot column's `fmt`), in the language of the host's
locale (the renderer: the UI language), English or French: a named format as the report's
visuals draw it (`Short Date`, `Long Date`, `General Date` with its time, `d`, `D`, `G`, `T`,
`t`…), else a date/time format string by the DAX `FORMAT` rules (`dd/mm/yyyy`, `mmm yyyy`,
`hh:nn`, `AM/PM`; `m` is minutes after an hour or before seconds). The TypeScript
(`dateLabel` in the runtime's `format.ts`) and the Rust (`bicraft_render::date_label`, over
`bicraft_locale`'s `FORMAT` code) are twins held equal on a corpus of 47 formats × 8 dates × 2
languages (`date_labels_match_the_renderer`). Without a format string, or with one that is not
a date format, the label stays `YYYY-MM-DD[ HH:MM]`. Box categories are still told apart by
their value: two dates of one month under `mmmm yyyy` are two boxes with the same label.

`General Date` keeps the time at midnight: Power BI Desktop 2.159.1231.0 (French, 2026-11-24)
wrote a General Date dateTime column's midnight as `05/01/2026 00:00:00` in its own table and
slicer, and BIcraft's Sankey nodes on that column read the same, so the rule stays.

## Layouts

Both drawers compute the same primitives (those of the Spec visual: `rect`, `line`, `path`,
`circle`, `text`, in the visual's pixels, rounded half up to hundredths), with the Spec
visual's text width estimate, number formatting and "nice" linear domains
([spec-visual.md](spec-visual.md) "Layout"). Padding is 8 px; a y axis takes 36 px on the left,
an x axis 18 px at the bottom; nothing is drawn when the plot would be under 8 px. X tick labels
stay inside the visual and one that would come within 4 px of the previous one is skipped.

### Histogram

The rows with a number in `values` and, when `frequency` is bound, a positive frequency (else a
weight of 1). With `bins.size` > 0, bins of that width start at the largest multiple of it not
above the smallest value, `floor((max − start) / size) + 1` of them (each `[from, to)`); more than
200 are widened to 200 over the same range. Else `bins.count` bins of equal width from the
smallest value to the largest (the largest in the last bin); a single distinct value gets one bin
of width 1 around it. A bin's count is the sum of its weights.

With `bins.cumulative` (since 1.0.2.0) each bin's count is the running sum of the counts up to
it (its tooltip gives the range from the first bin's start, and it selects the rows of every
bin up to it, at most 5,000). The x scale runs from the first bin's start to the last bin's end
(ticks at a nice step); the y scale from 0 to the largest count. Bars are contiguous with a 1 px gap (none under 3 px wide),
gridlines at the y ticks, value labels (`labels.show`) 7 px above each bar.

### Bullet chart

One row per category (at most 100; without a category, one row). Band thresholds: the `bands`
fields (sorted), else `bands.first/second/third` percent of the row's target (none without a
target). One scale for every row, from 0 (or the smallest value) to the largest value, target,
threshold or maximum, nice. In each row (height 70% of its share, at most 32 px): the bands from
the scale's start to each threshold and the last one to the row's `maximum` (else the scale's
end), shaded `#9E9E9E`, `#BDBDBD`, `#D3D3D3`, `#E4E4E4`, `#F0F0F0`, the last band always the
lightest; the value as a bar of a third of the height from 0; the target as a 2 px tick over 70%
of the height. Category labels (11 px, at most 24 characters, the label column at most 40% of the
width) on the left; value labels (`labels.show`) 4 px past the bar's end.

With `style.vertical` (since 1.0.2.0) each row is a column on the same scale, now vertical: the
plot from 44 px (the y axis labels) to the right, 14 px from the top, 18 px above the bottom when
there is a category. Each column is 70% of its share of the width, at most 32 px; bands from
the scale's start up to each threshold, the value as a bar a third of the column wide from 0, the
target as a 2 px tick over 70% of the width, value labels 7 px above the bar's end (below it for a
negative value), category names (10 px, cut to the column's width less 4 px) under the axis.

### Box plot

The rows with a number in `value`, grouped by the `category` value (first appearance order, at
most 100 boxes; without a category, one box). For each box, the sorted values give the quartiles
by linear interpolation between order statistics (`h = (n − 1) p`, Excel's QUARTILE.INC); the
whiskers end at the furthest values within 1.5 IQR of the box (Tukey's fences); values beyond
are outliers (at most 500 drawn per box); the mean is the arithmetic mean. The y scale covers the
whiskers, the mean (when shown) and the outliers (when shown), nice. Each box is half its band (at
most 48 px), filled at opacity 0.3 with an outline, a 2 px median line, whisker stems and caps,
the mean as a dark dot, outliers as dots at opacity 0.6; category names below. With
`box.horizontal` (since 1.0.2.0) the same boxes lie along rows: the values on an x axis at the
bottom (18 px) with vertical gridlines, one row per box from the top, the category names (10
px, at most 24 characters, a column of at most 40% of the width) on the left.

### Dumbbell and slope chart

One row per category (at most 200). As a dumbbell: a legend row (the two field names and
colours), one row per category with a 2 px line between its two values and a dot at each (radius
30% of the row, 2–6 px), the smaller value's label on the left of its dot and the larger's on
the right, vertical gridlines at the ticks, the scale (not from zero) on the x axis, category
labels on the left. As a slope chart (`style.slope`): the field names above two vertical axes,
start on the left and end on the right, one line per category from its start to its end with a
dot at each, and labels `Category value` on the left and `value Category` on the right (label
columns at most 30% of the width each). A row without one of its values draws that dot only.
Since 1.0.2.0 the slope chart's labels on each side are moved apart: taken from the top (ties in
row order), each at least 11 px below the one before, then moved up where they would pass the
axes' bottom; a label keeps its dot's height when nothing is near. Since 1.3.1.0 the dumbbell's
value labels are moved apart the same way, but only between labels whose horizontal spans (the
text's estimated width, plus the chip's padding with value chips) come within 2 px of each other:
in their order from the top, each at least the value text's size plus 1 px (plus 6 px with
chips) below the ones it meets, then moved up where they would pass half a text height above the
axis; labels far apart along the scale stay on their rows. Since 1.3.1.0 a slope or dumbbell
label moved more than 1 px from its dot gets a thin leader line (0.75 px, the axis colour) from
the dot's edge to the label's near end. Since 1.3.2.0 labels stay inside the plot: a dumbbell's
value label is never placed above the first row's centre or half a value text below the plot's
top, whichever is higher, and a slope chart's never above its axes' top; the labels the moves
would push there are left out (their values stay in the tooltip) and the others are moved apart
again without them (`fit_inside` / `fitInside`, in each drawer). 1.3.1.0 pushed them above the
plot, over the legend and past the visual's top edge ("Seen in Desktop").

## Runtime

The visuals use BIcraft's visual runtime ([community-visuals.md](../community-visuals.md)
"Runtime"): the landing page (with no field bound: the visual's name and the roles to fill, in
English or French); a message naming the required roles still empty (histogram `values`; bullet
`value`; box plot `sample` and `value`; dumbbell `category`, `start` and `end`); tooltips through
the host; selection and cross-filtering; the context menu; keyboard focus; high contrast.

What one selects:

- **histogram**: a bin, that is every distinct value in it (an array of selection ids, at most
  5,000); the tooltip gives the bin's range (`from – to`, in the column's format) and its count;
- **bullet**: a row's category value; the tooltip lists the row's fields;
- **box plot**: a box's category value, that is every row of the box (an array of selection ids, at
  most 5,000: Power BI gives one identity per category and sample row; seen in Desktop
  2.159.1231.0, 2026-11-24, where the first row's id selected one sample); the tooltip gives the count,
  maximum, third quartile, median, first quartile, minimum, mean and outlier count;
- **dumbbell**: a row's category value; the tooltip lists the row's fields.

The first element of each unit (a bar, a box, a dot) is its tab stop. In high contrast, bullet
bands and box fills become outlines over the background; everything else takes the foreground.

A unit made of several category values (a histogram bin) is selected when all its ids are
among the host's selection; its context menu is the first value's.

## Highlights

Since 2026-11-24 (chart visuals and pack 1.3.0.0, Spec 1.4.0.0) BIcraft's visuals that have a
measure role declare `supportsHighlight` in `capabilities.json`: nine of the ten, all but the
Venn diagram. Microsoft Learn's "Highlight data points in Power BI visuals" describes the
contract: with `supportsHighlight` the categorical data view keeps the full `values` of each
value column and adds a `highlights` array beside them, of the same length, `null` for a value
that is not highlighted and the highlighted part otherwise (less than the value for a partial
highlight). Highlights come on value columns only, so a visual without a measure role (the Venn
diagram: item and set are both groupings) gets nothing to draw and keeps being filtered. The
runtime reads `highlights` from the value columns (`bvis/common.ts` `dataOf`,
`chart_pack/base.ts` for the role `measureRole` names, the Spec visual's `visual.ts`); the twin
layouts take them in their golden form (`highlights` beside `values`, or beside `rows` for the
pack) and from the snapshot's `data.highlights` (`crates/render`: `bvis::data_of`,
`chart_pack::input_of`, `spec::data_of`).

Two ways to draw a highlighted visual, chosen per visual:

| Visual | Highlighted | Why |
|---|---|---|
| Spec | a bar or rect from zero on a value field: the full bar at 40 % of its opacity, the highlighted part drawn over it from the same baseline; points, rules, texts, line vertices and labels of a row with no highlighted value dimmed | a bar's length is its value; a point or a line has no part |
| Histogram | bins none of whose rows has a highlighted frequency dimmed | a bin is built from samples; a part of a count would mix weights and samples |
| Bullet | the value bar at 40 % and its highlighted part over it (bands and target as they are) | the bar is the value |
| Box plot | boxes none of whose samples is highlighted dimmed | quartiles of a part are a different distribution, not a share |
| Dumbbell, slope | rows with neither end highlighted dimmed | two values per row, no share to draw |
| Org chart | nodes whose row is not highlighted dimmed (its card; the texts stay) | a node is a person, not an amount |
| Venn | no `supportsHighlight` (filtered, as before) | no value column, so no highlights |
| Marimekko | each segment at 40 % and its highlighted part from its foot, in proportion of the segment's value (the column widths stay the full values') | a segment's area is its value |
| Calendar heatmap | each day's cell at 40 % and its highlighted part filling it from its foot, in proportion of the day's value (the colour stays the full value's) | a cell's colour is its value; the part shows the share |
| Sankey | each ribbon at 40 % and its highlighted part along its top edge, as thick as the highlighted weight; nodes none of whose links is highlighted dimmed | a ribbon's thickness is its weight |

Dimming is 40 % (`HIGHLIGHT_DIM` in `runtime/style.ts` and `crate::look`, the selection's
dimming too): a dimmed full mark is drawn by the layout (its opacity times 0.4, both layouts
compare it), a dimmed unit, node or row is drawn by the interactions (the runtime's `fade` on its
targets, the renderer's `own_hits::Target::faded`), as a selection dims points. A highlighted part
is clamped between zero and the value (`part`); a part of zero, or a row whose highlight is
`null`, draws no part. A visual with no highlighted row at all is entirely dimmed.

In BIcraft's own interactions (`crates/engine/src/interact.rs` `can_highlight`), a data-point
selection elsewhere highlights one of these visuals; with no field in its measure roles it gets
no `highlights` and keeps all its rows, drawn as it is (`merge_highlights`); page
`visualInteractions` rules and the report setting `defaultFilterActionIsDataFilter` apply as for
the built-in visuals, and slicers still filter. Edit interactions offers Highlight towards them.

Seen in Power BI Desktop 2.159.1231.0 (Store edition, French UI, 2026-11-24), on a PBIX the
`bicraft` CLI wrote with the ten, a built-in column chart, a table and a card on one category:

- Edit interactions offers Filter, Highlight and None on the nine, Highlight chosen by default;
  on the Venn diagram, the table and the card, Filter and None, Filter chosen.
- A bar of the column chart, then a row of the table, as the selection: the nine drew their
  highlighted parts and dimmed units as `bicraft render` draws the same selection
  (`view.interact` then `report.render`); the Venn was filtered. Neither showed the org chart's
  dimmed cards distinctly (its texts stay).
- A visual declaring `supportsHighlight` with no field in its measure roles (a histogram without
  a frequency, an org chart without a value, a Spec visual on a category only) received an update
  with all its rows and no `highlights`, and its drawing did not change: BIcraft filtered it until
  this check and now leaves it as it is (`a_bicraft_visual_without_a_measure_field_is_highlighted_unchanged`,
  seen failing first). A visual missing a required role kept its landing page.

## Checks

- `crates/render/src/bvis_tests.rs`: bins by count and by size, the 200-bin cap, quartiles,
  fences and outliers, options falling back to defaults, and hostile data (blank, text, huge,
  tiny values, wrong shapes, every size including NaN and zero) for every kind.
- `crates/engine/tests/report/bvis_tests.rs`: 30 cases (since 1.0.2.0 also the cumulative
  histogram, vertical bullets, horizontal boxes and a crowded slope chart whose labels end at
  least 11 px apart; since 1.0.3.0 five with date categories in French and English formats;
  since 1.3.1.0 two crowded dumbbells, Modern and Minimal with chips, whose close start labels end
  12 px apart with leader lines while the end labels stay on their rows where they clear each
  other; since 1.3.2.0 a dumbbell and a slope chart of twenty close rows in a short visual whose
  labels cannot all fit: every label stays inside the plot and the ones left over are left out,
  seen failing on 1.3.1.0, where the crowded dumbbells' top labels sat above the plot).
  The Rust layouts are held to
  `bvis_golden.json` (`BICRAFT_BLESS=1` rewrites it) and must draw what the data says (bins,
  bands, outliers, joined rows, labels); the TypeScript layouts, compiled by `bicraft_tsbuild`,
  run under Node when Node is on the PATH and must give the same primitives (to 0.011 px), with
  every hit naming a drawn primitive and a unit; without Node the comparison prints a note and
  passes. The packaged capabilities validate against the API's `schema.capabilities.json` and
  declare every option. `visual.set_bicraft_visual` embeds the four packages (registered, no
  issue), converts column charts and a table (columns to grouping roles, measures to measure
  roles, the rest dropped), sets and removes options, and `report.render` and
  `report.export_pdf` report the four visuals `drawn`; the packages, types and options survive
  PBIP and PBIX round trips; bad kinds, options and values are refused with the report
  unchanged.
- `crates/engine/src/bvis.rs`: every package builds and reads back under its guid.

## Seen in Desktop

Power BI Desktop 2.159.1231.0 (2026-11-23) opened a PBIX written by
`set_bicraft_visual_embeds_converts_and_the_renderer_draws_it` without a message about the
visuals: the histogram (4 bins, the fill colour set by `options`), the bullet chart (bands,
target marks, bars), the slope chart (`style.slope`) and the box plot drew the same shapes as
`report.render`. Only a screenshot was taken that day.

On 2026-11-24 the same Desktop ran the four from a PBIX `bicraft` wrote with `visual.add`
([community-visuals.md](../community-visuals.md) Runtime, "Seen in Desktop"): tooltips with
formatted values, selection, Ctrl+click and cross-filtering of a column chart, the column chart's
selection filtering them, the context menu, the landing page of a Bullet without Value, the format
pane's French cards, and the histogram's bin count and labels changed there, saved (`5L`, `true`)
and read back by BIcraft. Found and fixed (1.0.1.0): the box plot selected only its first row,
the renderer drew no default title over the four (Desktop does: "People par Score"), and the
keyboard focus was lost after a selection. High contrast was not checked.

Version 1.0.2.0 in the same Desktop (2026-11-24), from a PBIX `bicraft` wrote with
`bins.cumulative`, `style.vertical`, `box.horizontal` and `style.slope` on (the slope chart over
12 names, so its labels are moved apart): the four drew the same shapes as `bicraft render`, and
the format pane showed the new options as toggles, on (Cumulé, Vertical, Horizontal, Graphique de
pente). The exported histogram package imported through Desktop's Import a visual from a file
drew Score's bins with People as the frequency ([community-visuals.md](../community-visuals.md)
"Outside BIcraft").

Version 1.1.0.0 in the same Desktop (French UI, 2026-11-24): the Build pane shows the four's roles
in French (Valeurs, Fréquence; Catégorie, Valeur, Cible, Plages, Maximum; Catégorie, Échantillon,
Valeur; Catégorie, Début, Fin), and they draw under the Modern, Bold and Minimal looks as
`bicraft render` draws them ([visual-look.md](visual-look.md) "Seen in Desktop"); their
interactions with a column chart, a table, a card and slicers are in
[community-visuals.md](../community-visuals.md) "Interactions with built-in visuals".

The dumbbell 1.3.1.0 in Power BI Desktop 2.160.1151.0 (French UI, 2026-11-25), from a PBIX
`bicraft` wrote with the six close teams of the `slope_crowded` golden case as a slope chart and
the thirteen rows of `dumbbell_crowded` as dumbbells (Modern at 360 × 200, Minimal with chips at
360 × 260): the slope chart's labels were moved apart with their leader lines, and both
dumbbells drew as `bicraft render` draws them. On those dumbbells the close start labels need
more height than the plot has, so both drawers push the top ones above the plot, over the legend
and past the visual's top edge (clipped by Desktop's frame) ([chart-pack.md](chart-pack.md)
"Seen in Desktop"). Fixed in 1.3.2.0 ("Dumbbell and slope chart"), checked by the golden cases,
not yet opened in Desktop.
