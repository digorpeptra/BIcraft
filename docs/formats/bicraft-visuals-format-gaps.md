# BIcraft visuals: Format pane options against the standard visuals

Sources: our own visuals' format objects on `main` at 836f340a (2026-11-25: `crates/engine/src/spec_visual.rs`, `bvis_*.rs`, `bvis.rs` `STYLE_PROPS`, `chart_pack/*.rs`, `chart_pack.rs` `STYLE`); the standard visuals' cards from BIcraft's own catalogue (`crates/ui/src/format_pane.rs` `VISUAL_OBJECTS`, from the public theme schema, [visual-options.md](visual-options.md)) and the Microsoft Learn pages "Customize X-axis and Y-axis properties" (<https://learn.microsoft.com/power-bi/visuals/power-bi-visualization-customize-x-axis-and-y-axis>), "Customize visualization titles, backgrounds, labels, and legends" (<https://learn.microsoft.com/power-bi/visuals/power-bi-visualization-customize-title-background-and-legend>) and "Customize the format pane in Power BI custom visuals" (<https://learn.microsoft.com/power-bi/developer/visuals/format-pane>).

## What is compared

For each of BIcraft's 14 custom visuals, this note lists the Format pane options the visual
declares (its `capabilities.json` objects and its `getFormattingModel` cards) next to the options
the closest standard Power BI visual offers in Desktop's Visual tab. It looks at the text of each
element (data labels, category and value axis labels, axis titles, legend text and title: show,
font family, size, colour, bold / italic / underline, display units, decimal places) and at the
usual chart options (legend position, gridlines, axis range and type, series colours, markers).

Not gaps: Desktop itself adds the General tab to every visual, custom visuals included: Title,
Subtitle, Effects (background, visual border, shadow), Header icons, Tooltips, Properties (size,
position, padding) and Alt text. A custom visual needs to declare nothing for them, so they are
left out of the matrix. BIcraft's Style card (preset, colours, rounded corners, gradient, shadow,
animation, card background, value chips, [visual-look.md](visual-look.md)) has no standard
counterpart; it is BIcraft's own and is not compared either.

Numbers in every BIcraft visual follow their field's format string (thousands, decimals,
percent, currency, in the report's language) or, without one, an automatic form (no decimals for
a whole number, else two). That is why "display units" and "decimal places" are marked "no"
below: the visual does the right thing by default but the Format pane cannot override it, which
the standard visuals allow per element.

## Closest standard visual

| Abbr. | BIcraft visual | Compared with |
|---|---|---|
| Spec | Spec (column, bar, line, area, combo) | Clustered column, line, line and clustered column |
| Hist | Histogram | Clustered column |
| Bul | Bullet | Clustered bar, gauge |
| Box | Box plot | Clustered column (no standard box plot: its axes, labels and legend) |
| Dum | Dumbbell and slope | Line chart, scatter chart |
| Org | Organization chart | Decomposition tree |
| Venn | Venn diagram | None: the cards every standard chart has |
| Mek | Marimekko | 100% stacked column |
| Cal | Calendar heatmap | Matrix with background-colour rules |
| San | Sankey | None: the cards every standard chart has |
| Sc+ | Scatter+ | Scatter chart |
| LA+ | Line / Area+ | Line, area, stacked area |
| Wf+ | Waterfall+ | Waterfall |
| G/K+ | Gauge / KPI+ | Gauge, KPI, card |

## Matrix (state on `main`, before this branch)

"yes": a Format pane option does it; "partial (…)": only part of it, or only by another route;
"no": the visual draws the element but no option changes it; "n/a": the visual draws no such
element. "JSON" means the Spec visual's spec text (card Spec), not a Format pane control.

| Option | Spec | Hist | Bul | Box | Dum | Org | Venn | Mek | Cal | San | Sc+ | LA+ | Wf+ | G/K+ |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **Data labels** show | partial (JSON `label`) | yes | yes | n/a | yes | partial (values; names always) | yes (counts) | yes | n/a | yes | yes (none / top / selected) | yes (mode) | yes (value / % / both / none) | no (always drawn) |
| labels font family | no | no | no | n/a | no | no | no | no | n/a | no | no | no | no | no |
| labels font size | partial (JSON, text marks) | no | no | n/a | no | yes | yes | yes | n/a | yes | yes | yes | yes | no |
| labels colour | no | no | no | n/a | no | yes | no | no | n/a | no | no | no | no | no |
| labels bold / italic / underline | no | no | no | n/a | no | no | no | no | n/a | no | no | no | no | no |
| labels display units | no | no | no | n/a | no | no | no | no | n/a | no | no | no | no | no |
| labels decimal places | partial (JSON `decimals`) | no | no | n/a | no | no | no | no | n/a | no | no | no | no | no |
| **Category axis** show | partial (JSON `x.axis`) | no | no | no | no | n/a | n/a | no | no (headers always) | n/a | no | no | no | n/a |
| axis labels font / size / colour / B-I-U | no | no | no | no | no | n/a | n/a | no | no | n/a | no | no | no | n/a |
| axis title | partial (JSON `x.title`) | no | no | no | no | n/a | n/a | no | n/a | n/a | partial (field name, fixed) | no | no | n/a |
| axis type (continuous / categorical) | partial (JSON `x.scale`) | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a | yes | no | n/a |
| **Value axis** show | partial (JSON `y.axis`) | no | no | no | no | n/a | n/a | no | n/a | n/a | no | no | no | n/a |
| axis labels font / size / colour / B-I-U | no | no | no | no | no | n/a | n/a | no | n/a | n/a | no | no | no | no |
| axis display units / decimals | no | no | no | no | no | n/a | n/a | n/a | n/a | n/a | no | no | no | no |
| axis range (min / max) | partial (JSON `min`, `max`, `zero`) | no | partial (Maximum role) | no | no | n/a | n/a | n/a | n/a | n/a | no | partial (start at zero) | no | yes |
| axis title | partial (JSON `y.title`) | no | no | no | no | n/a | n/a | no | n/a | n/a | partial (field name, fixed) | no | no | n/a |
| **Legend** show | partial (JSON `legend`) | n/a | n/a | n/a | no (always, dumbbell) | n/a | no (always) | yes | yes | n/a | yes | yes (or end labels) | n/a | n/a |
| legend position | no | n/a | n/a | n/a | no | n/a | no | no | no | n/a | no | no | n/a | n/a |
| legend font / size / colour | no | n/a | n/a | n/a | no | n/a | no | no | no | n/a | no | no | n/a | n/a |
| legend title | no | n/a | n/a | n/a | no | n/a | no | no | no | n/a | no | no | n/a | n/a |
| **Gridlines** (show, colour, style, width) | partial (JSON `grid`) | no (always) | no | no (always) | no (always) | n/a | n/a | no (always) | n/a | n/a | no | no | no | n/a |
| **Series / data colours** | partial (Style palette; JSON `color`) | yes (one) | yes (value, target) | yes (one) | yes (start, end, line) | yes (nodes) | yes (3 sets) | partial (6 slots by position) | yes (low, high, empty) | partial (12 node slots; links by rule) | partial (Style palette only) | partial (one line colour, area fill) | yes (increase, decrease, total, rule) | yes (value, bands, target) |
| **Markers** (show, shape, size, colour) | partial (JSON point marks) | n/a | n/a | n/a | partial (dot colours) | n/a | n/a | n/a | n/a | n/a | partial (size, fill, border, opacity; no shape) | partial (on / off) | n/a | n/a |

Summary: no BIcraft visual offers a font family, bold / italic / underline, display units or
decimal places in the Format pane; text colour exists only for the organization chart's nodes;
axis labels and axis titles have no option at all outside the Spec visual's JSON; the legend
can be switched on or off in five visuals but never moved or restyled.

## Missing options per visual

Ordered by what report authors change most often: label size and colour, number format of
labels, axis text and titles, legend position, then the rest.

- **Spec.** Everything textual sits in the spec JSON, where most authors will not look: a Data
  labels card (show, size, colour, decimals, display units), X and Y axis cards (show, label
  size and colour, title on/off and text, range), a Legend card (show, position).
- **Histogram.** Label size, colour and decimals; X and Y axis label size and colour; axis titles
  (bin field, count); gridlines on/off.
- **Bullet.** Label size, colour, display units and decimals; category name size and colour;
  value scale minimum and maximum as constants (today only the Maximum role).
- **Box plot.** No value labels at all (median, quartiles as optional labels); axis label size and
  colour; axis title; gridlines on/off.
- **Dumbbell / slope.** Label size, colour and decimals; legend on/off and position; axis label
  size and colour; gridlines on/off; marker (dot) size.
- **Organization chart.** Font family and bold for node names; value display units and decimals;
  value text colour apart from the name.
- **Venn.** Count colour and decimals or display units; set name (legend) size, colour,
  on/off and position.
- **Marimekko.** Label colour and decimals or display units; legend position and text size;
  percentage axis label size and colour; colours per series name instead of by position.
- **Calendar heatmap.** Month and weekday header size and colour; legend position and text size;
  value labels in cells (on/off, size) for large cells.
- **Sankey.** Label colour; value shown with the node name (on/off, display units, decimals);
  colours per node name instead of by position.
- **Scatter+.** X and Y axis label size and colour; axis titles on/off and text; axis ranges;
  legend position; label colour; marker shape; colours per legend value.
- **Line / Area+.** Label colour, display units and decimals; axis label size and colour; axis
  titles; value axis minimum and maximum; legend position; one colour per series.
- **Waterfall+.** Label colour, display units and decimals; axis label size and colour; axis
  titles; value axis range; gridlines on/off.
- **Gauge / KPI+.** Value (callout) size, colour, display units and decimals; target and
  minimum / maximum label size and colour; label on/off for the reference line.

## Added on 2026-11-25 (G119)

- **Shared text cards** in BIcraft's visual runtime (`runtime/textsettings.ts`, read the same way by
  `crate::visual_runtime::text_of`): Data labels (`labels`), X axis (`categoryAxis`, labels and
  title), Y axis (`valueAxis`, labels, title, display units, decimal places) and Legend
  (`legend`), each with a show switch in the card header, the font row (family, size, bold,
  italic, underline) and a colour, under the property names Power BI's own charts use.
- **Spec visual 1.5.0.0** takes all of them ([spec-visual.md](spec-visual.md) "Text"): in the matrix
  its column now reads "yes" for data labels (show, family, size, colour, B/I/U, units,
  decimals), category and value axis labels (show, family, size, colour, B/I/U; units and
  decimals on the value axis), both axis titles (show, family, size, colour, B/I/U) and the
  legend text (show, family, size, colour, B/I/U). Legend position, legend title, axis range and
  gridlines stay in the spec JSON only.
- **Spec series.** Every bound measure is drawn as its own series when the spec draws one plain
  series, as a clustered column chart does; otherwise the visual says how many measures it does
  not draw ([spec-visual.md](spec-visual.md) "Series").
- **Numbers in the report's language** for the Spec visual's labels and ticks (`416 198,52` in
  French).
- The 13 other visuals were rebuilt with the new runtime (versions raised), their cards unchanged:
  the per-visual lists above remain open (G119).

## Kept on purpose

- **Small sizes.** BIcraft's visuals keep drawing at very small sizes: axes, labels and legends
  stay until the plot itself would be under 8 px ([bicraft-visuals.md](bicraft-visuals.md)
  "Layouts", [spec-visual.md](spec-visual.md) "Layout"), whereas Desktop's own charts drop axes,
  titles and labels well before that. Any new text option must keep this: a larger font or a
  title must shrink or skip text (as the X tick labels already skip when they would touch) rather
  than hide the whole element or the plot early.
- **Numbers from the format string.** Without a units or decimals setting, labels keep following
  the field's format string; a new option overrides it only when set.
- **No bundled fonts in the visuals.** Visuals draw with a system font stack
  ([visual-look.md](visual-look.md)); a font family option names a font, it does not embed one.
- **General cards stay Desktop's.** Title, background, border, shadow, header icons and tooltips
  are not duplicated as visual options.
