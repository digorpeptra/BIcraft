# BIcraft Waterfall+ and BIcraft Gauge / KPI+

Sources: our own format. Two visuals of BIcraft's chart pack ([chart-pack.md](chart-pack.md)),
built the same way: the `IVisual` class in `crates/engine/src/chart_pack/<kind>_visual.ts`, the
layout in `crates/engine/src/chart_pack/<kind>.ts` and its Rust twin in
`crates/render/src/chart_pack/<kind>.rs`, on BIcraft's visual runtime
([community-visuals.md](../community-visuals.md) "Runtime") and look ([visual-look.md](visual-look.md)).
ROADMAP M12.3, added 2026-11-24 at the product owner's request. The requirements come from the
needs ledger ([community-visuals.md](../community-visuals.md) rows 2, horizontal waterfall with
subtotals, and 4, linear gauge against a target); the designs, drawings, names and code are
BIcraft's (no visual's code, text or image was read or copied).

| | Waterfall+ | Gauge / KPI+ |
|---|---|---|
| display name | BIcraft Waterfall+ | BIcraft Gauge / KPI+ |
| guid (visual type) | `bicraftWaterfallPlus3C5E7A9B1D2F4A6C8E0B` | `bicraftGaugeKpi9E1A3C5B7D2F4E6A8C0B1D3F` |
| name (`visual.add`, `bicraft-cli visual export`) | `bicraftWaterfallPlus` | `bicraftGaugeKpi` |
| version | 1.0.4.0 (1.0.3.0 until 2026-11-27, 1.0.2.0, 1.0.1.0 and 1.0.0.0 until 2026-11-25; 1.0.3.0 only the runtime it bundles, G119; 1.0.4.0 the whole format string in its labels, below) | 1.0.4.0 (1.0.3.0 until 2026-11-27, 1.0.2.0 until 2026-11-26, 1.0.1.0 and 1.0.0.0 until 2026-11-25; 1.0.2.0 only the runtime it bundles, G119; 1.0.3.0 the whole format string, the arc's variance and the KPI card's sparkline, "Gauge / KPI+"; 1.0.4.0 only the runtime it bundles, which now reads the scaling commas itself) |
| data roles | `category`, `group`, `subtotal` (Grouping), `target` (Measure), `values` (Measure, required, up to 8 fields) | `trend` (Grouping), `value` (Measure, required), `target`, `min`, `max`, `threshold1`–`threshold3`, `previous` (Measure) |
| data view | categorical: categories `category`, `group`, `subtotal` (top 1,000), values `values`, `target` | categorical: category `trend` (top 1,000), values the eight measures |
| format objects | `waterfall`: `orientation` (`vertical`, `horizontal`), `showTotal`, `subtotalEvery`, `connectors`; `colors`: `colorBy` (`sign`, `category`, `rule`), `increase`, `decrease`, `total`, `ruleValue`, `ruleAbove`, `ruleBelow`; `labels`: `mode` (`value`, `percent`, `both`, `none`), `fontSize`; `target`: `fill`; `style` | `gauge`: `style` (`arc`, `bullet`, `kpi`), `compareTo` (`target`, `previous`), `lowerIsBetter`, `showTrend`; `range`: `min`, `max`, `target`, `threshold1`–`threshold3`; `colors`: `value`, `valueByBand`, `band1`–`band4`, `target`; `style` |
| layout options | `orientation`, `showTotal`, `subtotalEvery`, `showConnectors`, `colorBy`, `increaseFill`, `decreaseFill`, `totalFill`, `ruleValue`, `ruleAbove`, `ruleBelow`, `labelMode`, `fontSize`, `targetFill` | `gaugeStyle`, `compareTo`, `lowerIsBetter`, `showTrend`, `min`, `max`, `target`, `threshold1`–`threshold3`, `valueFill`, `valueByBand`, `band1`–`band4`, `targetFill` |

Both declare `supportsHighlight`, their role and option names have a `displayNameKey` with
English and French `stringResources`, and their package icons are drawings of BIcraft's
(`bicraft_visuals::icons`: floating steps ending on a total with a target tick; a half ring of
three bands with a needle).

## One input, two drawers

As for the pack ([chart-pack.md](chart-pack.md) "One input, two drawers"): `rows` hold the cells
of the roles in the table's order, `formats` their columns' format strings, `options` the format
properties under their layout keys, `lang` `en` or `fr`, `theme` the report's colours, and
`highlights` the highlighted parts when another visual's selection highlights the visual.

The Waterfall+'s `values` role takes several fields: its rows are
`[category, group, subtotal, target, value₁, value₂, …]` (one cell per measure, at most 8), the
input's `names` are the measures' names, and each row's `highlights` entry is a list with one part
per measure. In BIcraft the renderer collects every column of the role
(`bicraft_render::chart_pack::multi_role`); the capabilities declare `values: {max: 8}` and the
registry lets `visual.add` place up to 8 fields there.

Numbers are written with their column's whole format string: by the runtime's `formatNumber` in
Power BI and by BIcraft's `FORMAT` rules (`bicraft_locale::vb`, grouped with the culture's display
separator) in BIcraft, so scaling commas and quoted text apply (`$#,0.00,,"M"` of 7,433,915 is
`$7.43M`, `#,0.0,," M€"` is `7,4 M€` in French, `#,##0,"K"` of 1,234,567 is `1,235K`); without a
format string both write `fmtAuto` (no decimals for a whole number, else two). A negative value
under a format with one section is written with the sign first (`-$40.00`). Percentages have one
decimal, `-37.5%` in English and `-37,5 %` in French. The Gauge / KPI+ does so since 1.0.3.0, the
Waterfall+ since 1.0.4.0 (2026-11-27); before, the Waterfall+'s labels went through the renderer's
`fmt_val`, which kept only the digits and the currency (`1,234,567` for `#,##0,"K"`), while its
Power BI drawing scaled them. Commas right after the last decimal placeholder are read as scaling
commas, moved before the decimal point (`scaleCommas` in the runtime's `formatNumber` since
2026-11-27, so every BIcraft visual reads them alike; `scale_commas` in the renderer), as the
dashboard that found the bug expected; DAX `FORMAT` itself leaves them out
([dax-functions.md](dax-functions.md)), and Desktop's own reading of them is not checked yet. The tooltips use the same formatting. Arcs use a sine and cosine series written
alike in both languages, so both drawers agree to the hundredth.

## Waterfall+

- **Steps.** With one measure, each row with a number is a step, in row order: a bar floating
  from the running total before it to the running total after it. With several measures each
  measure is one step, its total over the rows, in field order (the category is not split).
  Values above 10¹⁵ in size, texts and blanks are skipped.
- **Subtotals.** A subtotal bar (from zero to the running total) follows the last step of each
  group when the `group` text changes (labelled with the group), a row whose `subtotal` cell is
  set (true, a number other than 0, a text other than "", 0, false, no, non), and every N steps
  (`subtotalEvery`, 0 none, at most 100). Never first, never two in a row; one that would end the
  steps gives way to the total. At most 200 bars.
- **Total.** `showTotal` (default on) adds a final bar from zero to the end total.
- **Orientation.** `vertical` (columns, categories along the foot, value ticks on the left) or
  `horizontal` (bars top to bottom, names on the left, value ticks at the foot). Bars are 68 % of
  their slot (60 % in Minimal), BIcraft's look bars: steps rounded on every corner, subtotals and
  the total square on their zero side.
- **Colours.** `sign` (default): increases `#1F9D55`, decreases `#D64545`; `category`: the
  look's palette by step; `rule`: a step at or above `ruleValue` takes `ruleAbove` (`#1C8FCC`),
  below it `ruleBelow` (`#BF8700`). The total takes `totalFill` (the look's first colour), a
  subtotal the total's colour 35 % towards white. Each can be set in the format pane.
- **Connectors.** `connectors` (default on): a 1 px line in the axis colour at the running total
  from each bar to the next.
- **Labels.** `labels.mode`: `value` (default; a step's change signed, `+120`), `percent` (the
  change against the running total before it, `+12.5%`, the value when that total is 0), `both`
  (`+120 (+12.5%)`), `none`. At the look's value size (`fontSize`, 8–20) and weight, above a rise
  and below a fall (right and left when horizontal), on chips in Bold; left out where they do not
  fit the slot or the visual. Since 1.0.1.0 a fall's label in the horizontal layout goes left of
  its bar only when it ends at least 2 px inside the plot (clear of the category names, which end
  6 px left of it), else right of the bar's other end, else it is left out; 1.0.0.0 checked only
  the visual's edge, so a small fall near the axis's start wrote `-5` over its name.
- **Value axis.** Ticks with the decimals the step needs, grouped by the culture since 1.0.1.0:
  `1,000.5` in English, `1 000,5` in French (a no-break space, as the runtime's `formatNumber`
  and the tooltips group them); 1.0.0.0 wrote `1,000` in both.
- **Target.** The `target` field's total over the rows: a 3 px line (2 in Minimal, `target.fill`,
  default the label ink) across the last bar at the target, and the header line
  `vs target: +12 (+8.0%)` (`Écart à la cible : …`), the end total against the target.
- **In Power BI** a step is a data point: its tooltip gives the category and group, the change
  (signed, the measure's format), the running total and the change in percent; it selects its
  row (category, group and flag). Subtotals, the total, the target and the header show a tooltip
  only (no row of their own); with several measures the steps show a tooltip only, as a custom
  visual cannot select a measure.

## Gauge / KPI+

- **The value.** The `value` of the last row with a number; with a `trend` field (dates or
  categories, in the data view's order) that is the latest point, and every point feeds the
  sparkline. The target, minimum, maximum, thresholds and previous value come from the same row's
  roles, else from the `range` card (target, min, max, thresholds); the previous value without a
  role is the point before the latest.
- **Range.** Minimum 0 by default; a maximum not above the minimum is ignored; without one the
  maximum is the first round step at or above every value, target and threshold (the axis
  rounding of BIcraft's charts, 4 steps). Thresholds outside the range, or repeated, are left
  out; the others, sorted, cut the range into up to four bands.
- **Bands.** Each band has its colour (`colors.band1`–`band4`); unset, worst to best red, amber,
  green, deep green (`#D64545`, `#E0A100`, `#2E9E5B`, `#1C7A45`; two bands red and green) softened
  62 % towards the surface, reversed with `lowerIsBetter`; a range without thresholds is one band
  in the look's empty colour. The value takes `colors.value` (the look's first colour), or with
  `valueByBand` the colour of the band it falls in.
- **Variance.** Against the target (`compareTo: target`, default) or the previous value
  (`previous`); when the chosen one is missing, the other. Written signed with the value's format
  and as a percentage of the reference: `-8 (-10.0%) vs target` (`vs cible`, `vs précédent`), in
  green when good and red when bad (`#137A3F` / `#C0392B`, on a dark theme `#5BD08A` / `#FF7B6E`;
  the sign and words say it too), lower being good with `lowerIsBetter`.
- **Arc** (`gauge.style: arc`, default): a half ring centred at the foot, as wide as the visual
  allows, the bands along its outer 26 %, the value as an inner ring from the minimum, the target
  a radial tick across the ring, the minimum and maximum under the ring's ends, the value (bold,
  30 % of the radius, 12–40 px) and the variance inside. Below 16 px of radius only the value.
  Since 1.0.3.0 the variance is never cut: the first of the whole line, the line without its
  words (`-0.5% (-1.3%)`) and the difference alone that fits the ring's inner width (and the
  room between the minimum and maximum when it reaches their line), at the name size, else the
  tick size; none fitting, none is drawn. 1.0.2.0 cut it with an ellipsis (`-0.5% (-1.3%) vs ta…`
  in a 300×136 arc).
- **Bullet** (`bullet`): the value top left (bold) and the variance top right; the bands as one
  rounded bar (at most 40 px tall), the value a bar 42 % as tall from the minimum, the target a
  vertical tick, the minimum and maximum under the ends.
- **KPI card** (`kpi`): the value centred (bold, up to 48 px), the variance, then the reference
  (`Target: 80`, `Cible : 80`, `Previous: 66`, `Précédent : 66`). Since 1.0.3.0 the text comes
  first: the sparkline takes the height the value, variance and reference leave (left out under
  16 px), then the reference and the variance lines give way when even the text does not fit;
  the variance drops its words before it is cut. 1.0.2.0 drew the sparkline over the reference
  line in a card about 100 px tall.
- **Trend.** With two points or more and `showTrend` (default on), a sparkline along the foot
  (its own vertical scale): a line in the value's colour through every point and a dot on each
  (the latest larger), only the latest past 60 points. The arc and the bullet keep it for a
  visual 120 px tall or more, the KPI card for 80 px.
- **Highlights.** The value mark is drawn at 40 % with its highlighted part over it (from the
  minimum); a sparkline dot whose row is not highlighted is dimmed; the KPI card dims its value
  when its row is not highlighted.
- **In Power BI** the value marks (arc, bar, text) and the target are data points of the latest
  row (tooltip: the trend point, the value, the target, the variance), a sparkline dot of its
  own row (trend point and value); with a trend field they select their row and cross-filter.

## Checks

- `crates/render/src/chart_pack/waterfall.rs` and `gauge.rs` tests: subtotals after flags and
  every N, never doubled, the total replacing a final subtotal; the caps (200 bars, 8 measures);
  formatted and signed numbers; the series against the platform's sine and cosine (to 10⁻⁹); the
  range, thresholds and auto maximum; the gauge's whole format string (scaling commas after the
  decimals, quoted text, French), the arc's variance whole and clear of the other texts at five
  sizes down to 240×110, the KPI card's texts clear of each other and of the sparkline from 80 to
  240 px tall (`the_value_takes_its_whole_format_string`, `the_arc_variance_fits_a_short_arc`,
  `the_kpi_sparkline_stays_clear_of_the_text`, all three seen failing on 1.0.2.0). `crates/render/src/chart_pack_tests.rs`'s hostile test
  covers both kinds (odd JSON, extreme numbers and sizes, 20,000 rows).
- `crates/engine/tests/report/waterfall_gauge_tests.rs`: 41 golden layouts
  (`waterfall_gauge_golden.json`, every option, both orientations, French, Bold, Minimal, a dark
  theme, highlights, hostile and empty inputs), the TypeScript layouts equal to the Rust under
  Node, every text inside the visual, the running totals, the French axis grouped as the labels
  and a small fall's label clear of its name (`wf_thousands_fr`, `wf_small_fall_horizontal`, both
  seen failing on 1.0.0.0; `wf_all_negative`'s `-5 (-16.7%)` moved right of its bar too); both packaged visuals under Node with a
  stand-in host (landing page, the missing role in French, tooltips, selection ids, context menu,
  high contrast, the format pane's cards in English and French); in a report, both added by name
  (two measures in one Waterfall+), drawn by `report.render` and PDF export, the format pane's
  options reaching the layouts, kept through PBIP and PBIX.
- `crates/engine/tests/report/own_visual_interactions_tests.rs`: like the ten others, a click
  selects and filters the page, Ctrl adds, a table selection highlights them, a slicer filters,
  a bookmark restores the selection, the live viewer gets their drawing and hits.

## Seen in Desktop

Power BI Desktop 2.160.1151.0 (Store edition, French UI, 2026-11-25), from a PBIX `bicraft` wrote
with both visuals next to Scatter+, Line / Area+ and built-in visuals (the run of
[chart-pack.md](chart-pack.md) "Seen in Desktop"): a Waterfall+ of twelve steps grouped by
department with a target, and its horizontal, rule-coloured, Minimal, every-4-steps, by-category
and percent variants; a Gauge / KPI+ arc with a trend and a target, a bullet with two thresholds,
a KPI card against the previous value and a lower-is-better arc coloured by band in Bold.

- **Drawing.** Every one drew the same shapes and texts as `bicraft-cli render`: steps, subtotals
  ("Sous-total"), total, connectors, the target line and `Écart à la cible : …`; the arc, bands,
  target tick, sparkline, `+5,0 (+7,7 %) vs cible`, `Cible : 65,0`, `Précédent : 65,0`.
- **Tooltips.** A step: `Name Ben, Dept North, Delta +10, Cumul 30, Variation +50,0 %`; the gauge:
  the latest day, `Total 70,0`, `Cible 65,0`, `Écart à la cible +5,0 (+7,7 %)`.
- **Selection.** A click on a step filters the table and card to its row and highlights the
  column and bar charts; a sparkline dot selects its row the same way. A selection elsewhere
  draws the gauge's value at 40 % with no part over it when the latest row is not highlighted,
  and dims the sparkline dots of the rows not highlighted, as the layouts say.
- **Format pane.** Cascade, Couleurs, Étiquettes, Cible; Jauge, Échelle, Couleurs; the Style card.
  Changed in Desktop and saved: the gauge to Carte d'indicateur with the Affirmé preset, the
  Waterfall+ to Horizontal and Par règle; BIcraft read `gauge.style` `kpi`, `style.preset` `bold`,
  `waterfall.orientation` and `colors.colorBy` back and drew the file as Desktop did.

Seen in both drawers alike: the value axis wrote `1,000` with English grouping in French (the
tooltip writes `1 080`), and in the horizontal layout a small fall's label (`-5`) sat against its
category name. Fixed in 1.0.1.0 in both drawers ("Value axis" and "Labels" above), checked by
the golden cases, not yet opened in Desktop. High contrast was not checked.
