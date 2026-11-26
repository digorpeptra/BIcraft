# Community visuals (ROADMAP M12)

BIcraft's own custom visuals for chart types the Power BI community asks for and does not have,
or lost when a visual was retired. They are free, open source and built into BIcraft by default:
the binary carries their TypeScript sources, builds each `.pbiviz` with `bicraft_tsbuild` when a
report uses it, and embeds it in the report, so Power BI Desktop opens the file without
installing anything ([pbiviz.md](formats/pbiviz.md), [spec-visual.md](formats/spec-visual.md)).

Sources: public documentation, the MIT visuals API and black-box runs in Desktop. Names, icons
and drawing are BIcraft's; no listing text, image or code of an existing visual is copied, and
no existing visual is recreated (AGENTS.md §1, §2).

## Needs ledger (M12.1, 2026-11-23)

### How the numbers were read

- **Votes (measured).** The Fabric Community idea pages answer automated readers with a bot
  challenge, so each count was read from the Internet Archive's copy of the public idea page (the
  page's vote counter); the date given is the date of that copy, not today. Counts are those of one
  idea page, never summed across pages. Epic ideas are the Power BI team's own consolidated ideas
  on the [Fabric Ideas board][fi]; ideas from the read-only Custom Visuals Ideas board carry the
  old board's (much smaller) counts and are not comparable with them.
- **Cited.** No measured idea found: the demand is the sources named in the row.
- **Built-in coverage.** Against the built-in list of Microsoft Learn's
  [Overview of visualizations in Power BI][learn-vis] (page dated 2026-01-28, updated 2026-10-15),
  the [waterfall page][learn-wf] (no orientation or subtotal setting) and
  [grouping and binning][learn-bin]. Not yet checked in Desktop 2.159 itself.
- **Marketplace (AppSource).** Read from [DataChant's daily export of the Power BI visuals
  Marketplace][dc] (MIT; commit `e51752d` of 2026-11-22): listing name, publisher, price label,
  certification and the date of the listed version. *Free* = free, no paid tier; *free + paid* =
  free with a paid plan or licence (the listing's free-plan flag; our reading of that flag, an
  estimate); *paid* = the listing says another purchase is needed. ★ = average rating / number
  of ratings where there are 10 or more. The export carries no API version (not recorded here).
  Only metadata was read: no listing text, image or package.
- **Well served** = a free (no paid tier), certified listing whose version is less than 12 months
  old (after 2025-11-23) with at least 10 ratings averaging 4 or more, or a built-in visual.
  Verdicts: **build** (M12.3 or a later pack, ranked below), **later** (real gap, no measured
  demand yet), **no** (covered, well served, or not a chart type).

### Ledger

| # | Chart type | Demand | Built into Desktop | Marketplace options (2026-11-22) | Retired or withdrawn | Verdict |
|---|---|---|---|---|---|---|
| 1 | Organization / hierarchy chart | measured: [Epic idea: Organization chart][e-org] **154** ([copy 2026-04-18][w-org]); old board [Org chart visual][i-org1] 9, status Delivered ([copy 2024-06-24][w-org1]) | no | Organization Chart by MAQ Software ([free, certified, 2026-08-21, ★2/10][a-maqorg]); Hierarchy Chart by Akvelon ([free, certified, 2025-09-19, ★3/24][a-akv]); Org Chart Drag and Drop, WorkforceVision ([free + paid, certified, 2026-11-03][a-wfv]); Organizational Chart, ChartPad ([free + paid, certified, 2026-09-11][a-cporg]) | two org charts now unlisted (DataChant's [unlisted list][dc]) | **build** (M12.3) |
| 2 | Waterfall, horizontal and with subtotals | measured: [Epic idea: New waterfall][e-wf] (axis, categories, totals and subtotals, series) **136** ([copy 2026-01-24][w-wf]); [Improving Waterfall chart][i-wf2] 136 ([copy 2026-03-13][w-wf2], an idea of 2015; the same count, so probably the same voters, estimated); cited: horizontal waterfalls ([Data Goblins wishlist][dg]) | partial: vertical only, no subtotals ([Learn][learn-wf]) | Simple Waterfall, Datanomy ([free, certified, 2026-11-09, ★4/48][a-sw], horizontal per its listing); Ultimate Waterfall, DataViz.Boutique ([free, certified, 2026-11-17, ★4/48][a-uw], horizontal per its listing); Waterfall Chart - xViz ([free + paid][a-xwf]); Drill Down Waterfall PRO, ZoomCharts ([free + paid][a-zcwf]) | — | **built** (M12.3, BIcraft Waterfall+; added 2026-11-24 at the user's request although two free certified visuals serve it: horizontal, subtotals, several measures, a target) |
| 3 | Histogram | measured: [Epic idea: Histogram support][e-hist] **123** ([copy 2026-03-13][w-hist]); [Power BI Histogram][i-hist2] 0 ([copy 2026-04-17][w-hist2]); cited: [Histogram thread][t-hist] | partial: a column chart over a binned column (bins on columns, not on measures; [Learn][learn-bin]) | Histogram by CBT ([free, certified, 2026-10-06][a-cbthist]); Histogram with points by MAQ Software ([free, certified, 2026-09-21, ★3/14][a-maqhist]); Histogram Pro, TCViz ([free + paid, certified][a-tchist]); Histogram Chart (Pro/Standard), PBIVizEdit ([free + paid][a-pvehist]); Histogram by PQ Systems ([paid][a-pqhist]) | Microsoft's Histogram: gone from the Marketplace, its repository [archived][gh-hist] (last push 2024-04-15) | **build** (M12.3) |
| 4 | Bullet chart / linear gauge | measured: [Epic idea: Linear gauge][e-lg] (compact progress against target) **112** ([copy 2026-05-20][w-lg]); cited: [Data Goblins wishlist][dg], [SQLBI on bullet charts][sqlbi-bullet] | no (radial gauge and KPI only) | Bullet Chart, Microsoft ([free, certified, 2024-12-23, ★3/40][a-msbullet]); Bullet Chart by OKVIZ ([free + paid, certified, 2026-05-21, ★4/65][a-okbullet]); Bullet Chart - xViz ([free + paid][a-xbullet]); Bullet Chart by CBT ([free, certified, 2026-09-24][a-cbtbullet]); Linear Gauge by MAQ Software ([free, certified, 2026-08-06, ★3/32][a-maqlg]) | — | **build** (M12.3) |
| 5 | Box and whisker | measured: [Epic idea: Box and wiskers][e-box] **86** ([copy 2026-03-12][w-box]); [Box and whisker][i-box2] 62, status Completed ([copy 2026-01-19][w-box2], an idea of 2014); old board [Introduce Standard Box Plot Visualizations][i-box3] 1 ([copy 2026-03-05][w-box3]); cited: [Box or violin plot as standard visuals][t-box] (third-party visuals not allowed in some organizations) | no | Box and Whisker chart by MAQ Software ([free, certified, 2024-06-10, ★4/65][a-maqbox]); Box and Whisker chart, DataScenarios ([free, certified, 2025-09-19, ★3/86][a-dsbox]); Box and Whisker (Pro), PBIVizEdit ([free + paid, certified][a-pvebox]); Box Plot by SmartVisuals ([free + paid, not certified][a-svbox]) | — | **build** (M12.3) |
| 6 | Venn diagram | measured: old board [3-Way Venn Diagram, Proportional and Dynamic][i-venn] **50** ([copy 2026-01-15][w-venn]); [Dynamic Venn Diagrams][i-venn2] 0 ([copy 2024-12-03][w-venn2]) | no | Venn Diagram by MAQ Software ([free, certified, 2026-09-11, ★3/30][a-maqvenn]); Venn Diagram, Samuel Gratzl ([free, certified, 2025-11-03][a-sgvenn]); Venn & UpSet by SmartVisuals ([free + paid, certified][a-svvenn]) | — | **build** (M12.3) |
| 7 | Marimekko / variable-width columns | measured, related: [Epic idea: Treemap bars][e-tmb] (blocks arranged in columns) **26** ([copy 2026-03-06][w-tmb]) | no | Mekko Chart, Microsoft ([free, certified, 2025-07-21, ★3/18][a-msmekko]); Marimekko Chart - xViz ([free + paid, certified][a-xmekko]); Atlyn Marimekko ([paid, certified][a-atmekko]) | — | **build** (M12.3) |
| 8 | Dumbbell and slope chart (lollipop as its one-series case) | cited: dumbbell, slope, lollipop and bullet overlaps ([Data Goblins wishlist][dg]); first search's [visual label ideas][cvi-label] | partial: workarounds drawn with line or scatter charts | Dumbbell Bar Chart, Nova Silva ([free + paid, certified, 2026-07-15][a-nsdb]); Dumbbell Chart by CBT ([free, not certified][a-cbtdb]); Lollipop Chart by Powerviz ([free + paid, certified][a-pvlol]); Slope Chart by SmartVisuals ([free + paid, certified, 2026-11-03][a-svslope]); BriqLab Slope Chart ([free + paid, not certified][a-bqslope]) | — | **build** (M12.3) |
| 9 | Calendar heatmap | cited: [Calendar heatmap with a matrix][t-calhm] (no built-in heatmap), [Help with Calendar Heat map][t-calhm2], [Data Stories calendar heatmap with Deneb][t-calhm3] | partial: a matrix with background colour scales, one month per matrix | Calendar Pro by OKVIZ ([free + paid, certified, 2026-05-18, ★4/24][a-okcal]); Calendar Heat Map, DataViz ([free, certified, 2026-06-08][a-dvcal]); ProCalendar Heatmap, Vectras ([free + paid, certified][a-vxcal]); Calendar by MAQ Software ([free, certified, 2026-07-24, ★4/85][a-maqcal], no heatmap per its listing) | — | **build** (M12.3) |
| 10 | Stream graph | measured: [Epic idea: Stream chart][e-stream] **24** ([copy 2026-02-10][w-stream]) | no | Stream Graph, Microsoft ([free, certified, 2024-12-23, no ratings][a-msstream]); Stream Plume ([paid][a-plstream]) | Heatstream (row 25) | build, later pack |
| 11 | Bump chart | cited: only the first search's candidate list; no idea or wishlist found | partial: the ribbon chart shows ranks over time | Bump Chart and Rankings, ChartPad ([free + paid, certified, 2026-10-18][a-cpbump]); BumpChart by iFour ([free + paid, certified, 2026-10-20][a-ifbump]); Time Series > Bump Chart, r42 ([free + paid, certified][a-r42bump]) | — | later |
| 12 | Sankey | cited: old board [Sankey diagram][i-sankey], [requests for the Sankey visual][i-sankey2], forum threads ([Sankey Chart][t-sankey]); vote count not read (no usable copy) | no | Sankey Chart, Microsoft ([free, certified, 2025-06-06, ★3/133][a-mssankey]); Sankey Diagram, VisioChart ([free + paid, certified, 2025-12-15, ★5/59][a-vcsankey]); Sankey Diagram by ChartExpo ([paid][a-cesankey]) | — | **built** (M12.3, BIcraft Sankey; added 2026-11-24 at the user's request: cited demand only, and Microsoft's own Sankey is free) |
| 13 | Violin plot | cited: [Box or violin plot as standard visuals][t-box] | no | Violin Plot, Daniel Marsh-Patrick ([free, certified, 2025-09-19, ★5/12][a-dmpviolin]); Violin Plot by MAQ Software ([free, certified, 2026-10-18][a-maqviolin]) | — | later (after the box plot) |
| 14 | Pareto chart | measured: old board [Pareto Chart][i-pareto] 0 ([copy 2023-12-04][w-pareto]) | partial: workaround, a combo chart with a cumulative-percent measure | Pareto by sio2Graphs ([free, certified, 2024-07-19, ★4/25][a-sio2]); Pareto Chart Pro, TCViz ([free + paid, certified][a-tcpareto]); Pareto Chart by CBT ([free, certified, 2026-10-25][a-cbtpareto]) | — | later |
| 15 | Word cloud | measured: [Word Cloud Visualization][i-wc] 0 ([copy 2025-08-13][w-wc], an idea of 2015) | no | Word Cloud, Microsoft ([free, certified, 2024-12-23, ★4/141][a-mswc]); Word Cloud by TCViz ([free, certified, 2026-10-29][a-tcwc]) | — | later |
| 16 | Waffle chart | cited: none found | no | Waffle Chart, kiewic.com ([free, certified, 2025-09-19, ★4/20][a-waffle]); Waffle, r42 ([free + paid, certified][a-r42waffle]) | — | later |
| 17 | Chord diagram | cited: none found | no | Chord, Microsoft ([free, certified, 2025-03-10, ★5/43][a-mschord]) | — | later |
| 18 | Network / force-directed graph | cited: none found | no | Force-Directed Graph, Microsoft ([free, certified, 2024-12-23, ★4/29][a-msforce]); Network Graph by Powerviz ([free + paid, certified][a-pvnet]) | — | later |
| 19 | Radar / spider chart | cited: forum threads ([Radar Chart][t-radar]) | no | Radar Chart by MAQ Software ([free, certified, 2026-06-15, ★4/13][a-maqradar]); Radar Chart, Microsoft ([free, certified, 2024-12-23, ★3/136][a-msradar]) | — | **no**: well served |
| 20 | Sunburst | cited: none found | no | Sunburst by MAQ Software ([free, certified, 2026-08-13, ★4/13][a-maqsun]); Sunburst, Microsoft ([free, certified, 2025-07-21, ★4/98][a-mssun]) | — | **no**: well served |
| 21 | Tornado / butterfly chart | cited: none found | partial: workaround, a stacked bar chart with negated values | Tornado chart, Microsoft ([free, certified, 2026-08-21, ★4/59][a-mstornado]) | — | **no**: well served |
| 22 | Candlestick | cited: none found | no | Candlestick by OKVIZ ([free, certified, 2025-12-22, ★5/11][a-okcandle]) | — | **no**: well served |
| 23 | Gantt chart | measured: [Epic idea: Gantt chart][e-gantt] **258** ([copy 2026-02-11][w-gantt]) | yes, in preview ([Learn][learn-vis]); BIcraft draws it (G99) | Gantt, Microsoft ([free, certified, 2026-05-26, ★3/186][a-msgantt]); Gantt Chart by MAQ Software ([free + paid, certified, ★4/95][a-maqgantt]) | — | **no**: covered by the built-in Gantt |
| 24 | Heatmap (category × category) | measured: [Heat maps in Power View][i-heat] 1,351, status Completed ([copy 2026-03-06][w-heat], a map heat layer of 2015) | partial: matrix background colour scales; heat layer in maps | Table Heatmap, Microsoft ([free, certified, 2024-12-23][a-msth]); Heatmap by Powerviz ([free + paid, certified][a-pvheat]) | — | **no**: covered by the matrix |
| 25 | Heatstream (retired) | cited: [Heatstream thread][t-heatstream] (no free alternative, users say) | no | none under that name | Microsoft's Heatstream: gone from the Marketplace, repository [archived][gh-heatstream] (MIT, last push 2023-10-02) | later (with row 10) |
| 26 | Card browser (retired) | measured: [Update Card Browser visual instead of deprecating it][i-card] 7 ([copy 2026-06-12][w-card]); cited: [Card Browser thread][t-card] | partial: the card visual's multi-card layout | none under that name | Microsoft's Card Browser: unlisted ([DataChant][dc]), repository [archived][gh-card] (MIT, last push 2023-04-23) | later |
| 27 | HTML viewer (retired) | cited: [HTML Viewer thread][t-html] | no | HTML Content Secure, Daniel Marsh-Patrick ([free, certified, 2026-09-23][a-htmlsec]); HTML Content ([free, not certified, ★5/36][a-html]) | the HTML Viewer visual: withdrawn ([thread][t-html]) | **no**: not a chart; served by HTML Content |
| 28 | Charticulator (retired original) | cited: [Charticulator retirement thread][t-chart] | no | Charticulator Visual Community (View) ([free, certified, 2025-05-26][a-chview]) and (Editor) ([free, not certified, 2025-09-22][a-ched]), Ilfat Galiev; Deneb, Daniel Marsh-Patrick ([free, certified, 2026-10-18, ★5/33][a-deneb]) | Microsoft Charticulator: unlisted ([DataChant][dc]), [repository archived][gh-chart] (last push 2024-01-09) | **no**: Deneb and BIcraft's Spec visual (M11.32) |
| 29 | Visuals on pre-2016 APIs | cited: [API 1.1 deprecation][api-dep] (warning in Desktop for visuals below API 1.2) | — | — | every visual below API 1.2 | **no**: not a chart type; BIcraft's visuals use the current API |
| 30 | Excel-style combo (overlaid series, order, opacity) | measured: old board [Custom Combo Chart][i-combo] 0 ([copy 2025-11-18][w-combo]); cited: [Data Goblins wishlist][dg] | partial: combo charts with two axes | (built-in enhancement) | — | **no**: a built-in's feature; BIcraft's Spec visual draws it |
| 31 | 100% stacked area | measured: [Epic idea: 100% Area chart][e-area] 7 ([copy 2026-04-12][w-area]) | partial: area and stacked area | — | — | later |

Requests about built-in visuals rather than chart types, for scale: [Epic idea: Small multiples
for cards][e-smc] 1,526 ([copy 2026-05-15][w-smc]), [Cards with trendlines][e-ctl] 173 ([copy
2026-01-24][w-ctl]), [Small multiples][e-sm] 35, [Legends with bars][e-lwb] 28,
[Animations][e-anim] 23 (copies 2026-05-11, 2026-06-14, 2026-03-14). M12 cannot change a
built-in visual; these stay out of the ledger.

### Ranking of the build rows

By measured votes, then cited sources:

1. Organization / hierarchy chart: 154 (2026-04-18)
2. Histogram: 123 (2026-03-13)
3. Bullet chart / linear gauge: 112 (2026-05-20)
4. Box and whisker: 86 (2026-03-12), plus 62 on the 2014 idea
5. Venn diagram: 50 (2026-01-15, old board)
6. Marimekko: 26 (2026-03-06, related epic)
7. Dumbbell and slope: cited
8. Calendar heatmap: cited
9. Stream graph: 24 (2026-02-10), later pack

The horizontal waterfall has the second largest count (136) but two free certified visuals with
a horizontal layout are current and well rated, so it leaves the pack.

## Candidates for the first pack (M12.3)

Organization / hierarchy chart, histogram, bullet chart, box plot, Venn diagram, Marimekko,
dumbbell and slope chart, calendar heatmap (ledger rows 1 and 3–9). Changes from the first
search: the horizontal waterfall leaves (well served), the bump chart moves to later (cited only,
the built-in ribbon chart shows ranks); the organization chart (largest measured count) and the
Venn diagram (measured) come in.

Already in BIcraft: the BIcraft Spec visual (a declarative chart spec, M11.32), sparklines in
tables and matrices (G106, a built-in Desktop feature BIcraft writes) and the Gantt drawing (G99).

## First pack (M12.3)

Part A, 2026-11-23 ([formats/bicraft-visuals.md](formats/bicraft-visuals.md)): four visuals of
ours, each with its data roles, format pane, the runtime's interactions, a `.pbiviz` built by
`bicraft_tsbuild` and the same layout in BIcraft's renderer (golden tests, the TypeScript run
under Node against the Rust). `visual.set_bicraft_visual {page, visual, kind}` embeds one and
turns a visual into it.

| Ledger row | Visual (`kind`) | What it draws |
|---|---|---|
| 3 | BIcraft Histogram (`histogram`) | a numeric column's values, weighted by a frequency measure, in bins of a count or a size |
| 4 | BIcraft Bullet (`bullet`) | per category a value bar, a target tick and up to five grey bands (band fields, or percentages of the target) on one shared scale |
| 5 | BIcraft Box Plot (`box_plot`) | per category the quartiles, whiskers at 1.5 IQR, outliers and mean of a measure over a column's items |
| 8 | BIcraft Dumbbell (`dumbbell`) | two measures per category as dumbbells, or with `style.slope` as a slope chart |

The four share `crates/engine/src/bvis/` (`common.ts`: data, options and drawing helpers;
`host.ts`: the `IVisual`) and `crates/render/src/bvis.rs`; each visual keeps its own files
(`bvis/<kind>.ts`, `bvis/<kind>_visual.ts`, `bvis_<kind>.rs` in the engine and the renderer). A
unit made of several category values (a histogram bin) is selected through an array of selection
ids, which the runtime now accepts (`rowsOf`, tooltips, context menu; the Spec visual is 1.1.1.0
with that runtime). Power BI Desktop 2.159.1231.0 drew the four from a generated PBIX
(2026-11-23, screenshot only); their interactions were tried there on 2026-11-24 (Runtime, "Seen in
Desktop").

Part B, 2026-11-23: four more are built ([formats/chart-pack.md](formats/chart-pack.md)): **BIcraft Org Chart**
(id, parent id, label, value: a tidy top-down tree whose nodes collapse and expand by their
button, the state persisted in the visual's `tree.toggled` property), **BIcraft Venn** (item and
set: the three largest sets as fixed circles with every region's count), **BIcraft Marimekko**
(category, series, value: variable-width 100 % stacked columns) and **BIcraft Calendar Heatmap**
(date, value: one cell per day, weeks as columns, months and weekdays in English or French). Each
is TypeScript of BIcraft's on the runtime below (`crates/engine/src/chart_pack/`), with its data
roles, format pane and the runtime's interactions, packaged as a `.pbiviz` like the Spec visual;
BIcraft's renderer draws the same primitives from twin Rust layouts
(`crates/render/src/chart_pack/`), held equal by golden tests. Their drawings follow the
textbook form of each chart; no existing visual's code, images or text was used.

Seen in Desktop with the other five on 2026-11-24 (Runtime, "Seen in Desktop"); inserted by name since
M12.4.

Added to part B on 2026-11-24 at the user's request: **BIcraft Sankey** (source, destination,
weight: one level of weighted links as ribbons between nodes in columns by longest path, cycles
left out and counted in a note; ledger row 12, whose demand is cited only and which Microsoft's
own free visual covers). Same files and checks as the four above
([formats/chart-pack.md](formats/chart-pack.md) "Sankey"); the four were rebuilt as 1.0.4.0 with
the shared `base.ts` that now draws its ribbons.

Added to part B on 2026-11-24 at the user's request, as richer versions of two built-in types:
**BIcraft Scatter+** (details, legend, x, y, size: every point up to 10,000 with none sampled, a
hexagonal density view above a threshold, marginal histograms or box strips, filled and bordered
markers, a trend line, quadrants with labels, labels on the top or the selected points) and
**BIcraft Line / Area+** (axis, series, value, value2: a band between two measures coloured by
which is above, shading beyond a threshold, an area formatted apart from its line, labels on all,
the last or the extreme points, line colours by series or by rule, a continuous date axis, a
dashed forecast stretch, series named at their line's end). Same files and checks as the others
([formats/chart-pack.md](formats/chart-pack.md) "Scatter+", "Line / Area+"); the other five were
rebuilt as 1.3.1.0 with the shared `base.ts` that now draws `path` primitives and the kit's dash
helper.

Added on 2026-11-24 at the user's request: **BIcraft Waterfall+** (vertical or horizontal steps,
subtotals by a group, a flag column or every N steps, colours by sign, step or rule, up to eight
measures as the steps, a target with its variance, value and percent labels, connectors; ledger
row 2) and **BIcraft Gauge / KPI+** (a radial arc, a linear bullet or a KPI card with free colour
bands from roles or the format pane, a target and up to three thresholds, the variance against
the target or the previous value, a sparkline from an optional trend field, the value in its
field's format; ledger row 4). Same files and checks as the others
([formats/waterfall-gauge.md](formats/waterfall-gauge.md)), 1.0.0.0, not yet seen in Desktop
([gaps.md](gaps.md) G117, G118).

## Runtime (M12.2)

Sources: Microsoft Learn's custom visual pages (tooltips, selection and interactivity, context
menu, landing page, keyboard navigation, high contrast support, the formatting model and
capabilities pages) and the MIT `powerbi-visuals-api` 5.3.0 types (`IVisualHost`,
`ITooltipService`, `ISelectionManager`, `ISelectionIdBuilder`, `ISandboxExtendedColorPalette`,
`FormattingModel`) and its `schema.capabilities.json`. No Microsoft code or utility package is
used: the runtime is our TypeScript, written from those descriptions.

BIcraft's visuals share one TypeScript library, `crates/engine/src/visual_runtime/`, that the
binary carries and adds to each visual's sources under `src/runtime/` (`visual_runtime::add_sources`);
the visual imports it with relative imports and `bicraft_tsbuild` bundles it in (no npm package).

| Module | What it gives a visual |
|---|---|
| `data.ts` | the categorical data view as rows (`tableOf`: the category, the value columns with display name, format string and roles), the roles that have fields (`boundRoles`, `missingRoles`), a format object's value (`objectValue`) |
| `format.ts` | values written with the column's format string: custom number formats (`0`, `#`, `,` grouping or ÷1000, `.`, `%`, `‰`, `E+0`, `positive;negative;zero` sections, quoted and `\` literals, rounding half away from zero), date and time tokens (`yyyy`, `MMMM`, `MMM`, `dd`, `dddd`, `HH`, `hh`, `mm`, `ss`, `tt`), named formats (General Number, Fixed, Standard, Percent, Short and Long Date, General Date); English or French separators and names (`fr`: no-break space and `,`); blanks as `(Blank)` / `(Vide)`, the texts Desktop draws for a blank ([locales.md](formats/locales.md)) |
| `tooltips.ts` | the hovered point's lines (category, then each value, formatted) shown, moved and hidden through `host.tooltipService` (only when it is `enabled()`) |
| `selection.ts` | one selection id per category value (`createSelectionIdBuilder().withCategory(column, i)`), `select` (Ctrl or ⌘ adds or removes), clicking the only selected point clears, `clear`, the host's selection kept across updates (`getSelectionIds`) and restored by bookmarks (`registerOnSelectCallback`), `showContextMenu`; nothing is selectable when `hostCapabilities.allowInteractions` is false |
| `interactions.ts` | the wiring of drawn points: hover tooltips, click to select, background click to clear, context menu (right click, the Menu key, Shift+F10), Tab to each point (one tab stop per category value, `role="option"`, `aria-selected`, an accessible name made of its tooltip lines), Enter or Space selects (Ctrl adds), arrows, Home and End move, Escape clears; points not selected are drawn at opacity 0.4 (in high contrast: as outlines), a selected line vertex gets a ring |
| `keys.ts` | the key rules (`keyAction`, `moveFocus`) |
| `contrast.ts` | `host.colorPalette.isHighContrast`: foreground, background and `foregroundSelected`, and a drawn primitive recoloured with them (fills and strokes in the foreground, areas as outlines) |
| `landing.ts` | the landing page shown while no field is bound, and one-line messages |
| `strings.ts` | the runtime's strings in English and French, chosen from `host.locale` (`fr`, `fr-*`: French; anything else: English) |
| `formatting.ts` | the format pane's model: cards (with or without a reset), groups and slices (`ToggleSwitch`, `ColorPicker`, `NumUpDown` with bounds, `TextInput`, `TextArea`) |

`visual_runtime::with_capabilities` adds what the runtime needs to a visual's
`capabilities.json`, keeping keys the visual sets: `privileges: []` (no web access, no download),
`tooltips` (default and canvas), `supportsLandingPage` and `supportsEmptyDataView`,
`supportsKeyboardFocus`, and `supportsHighlight` only for a visual that reads highlights (since
2026-11-24 every BIcraft visual with a measure role: [formats/bicraft-visuals.md](formats/bicraft-visuals.md)
"Highlights").

Selection and tooltips are runtime state: they change opacity, strokes and focus attributes of
the drawn elements, never the layout, so BIcraft's renderer and the visual still draw the same
primitives (the Spec visual's golden tests).

Checks (`crates/engine/tests/report/visual_runtime_tests.rs`): the Spec visual's packaged
capabilities validate against the API's `schema.capabilities.json` (vendored unmodified for the
tests, `crates/engine/tests/report/powerbi-visuals-api/`); the runtime, compiled by
`bicraft_tsbuild`, runs under Node when Node is on the PATH (else a note and a pass): number and
date formats, keys, selection rules, tooltips, roles, high contrast, the format pane model,
strings, and the interactions wired to a stand-in host (click, Ctrl+Enter, Ctrl+click, a click on
the only selected point, a bookmark's selection, Escape, arrows and End, tooltips, context menus,
a background click, a host without selection or tooltips).

### Seen in Desktop (2.159.1231.0, 2026-11-24)

Black-box run on Power BI Desktop 2.159.1231.0 (Store edition, French UI). The `bicraft` CLI
wrote a PBIX on a synthetic model (a table of twelve people with a department, a parent, a tier,
a score, four amounts and a day; a skills table related to it in both directions; measures with
the format strings `#,##0.0` and `0.0`) holding one page with a built-in column chart (Total by
Dept) and the nine visuals, each placed with `visual.add` by name, six of them sharing the Dept
category. Desktop was driven by input events (mouse, keys) through the WebView2 DevTools port;
the rendered DOM of each visual's sandbox frame, the report view's tooltip and menu texts and
screenshots were read; none of Desktop's scripts were read. Screenshots stayed local.

| Check | Spec | Histogram | Bullet | Box plot | Dumbbell | Org chart | Venn | Marimekko | Calendar |
|---|---|---|---|---|---|---|---|---|---|
| Drawn as `report.render` draws it | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ (French months) |
| Default title above the visual | ✅ | fixed | fixed | fixed | fixed | fixed | fixed | ✅ | fixed |
| Tooltip on hover, values with their format strings | ✅ | ✅ (bin range) | ✅ | ✅ (statistics in French) | ✅ | ✅ | ✅ (set, items) | ✅ (share) | ✅ (date) |
| Click selects and cross-filters the column chart | ✅ | ✅ (bin) | ✅ | fixed | ✅ | ✅ | fixed | ✅ | ✅ |
| Ctrl+click adds | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | fixed | ✅ | ✅ |
| Background click clears | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| The column chart's selection filters the visual | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Right click opens the host's data point menu | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Keyboard: Tab, arrows, Home, End, Enter/Space, Escape | fixed | — | — | — | ✅ | fixed | — | — | — |
| Format pane cards (French) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |

✅: as designed at the first try; fixed: a mismatch found in this run and fixed in BIcraft (below),
then seen right in Desktop; —: not tried on that visual (the runtime code is shared).

- **Drawing.** The nine draw the same marks, ticks, labels and texts as `report.render`. Desktop
  draws its default title over every one of them, by the kind of their roles: the fields of the
  measure roles by those of the grouping roles, in the visual's role order ("People par Score",
  "Total et Goal par Dept", "ScoreSum par Dept et Id", "Before et After par Dept", "Total par Id,
  Parent et Name", "Id et Skill", "Total par Day"). The renderer drew no title over the first
  pack's part A and the Venn diagram and titled the org chart and calendar "Total" only, so their
  drawings sat higher than in Desktop; the snapshot's `auto_title` now follows that rule for
  BIcraft's visuals and the renderer keeps the title over part A too. Under the title Desktop
  places the visual's frame at (5, 21) with the visual's size less 10 × 26 px (290 × 199 for
  300 × 225, read from the frame's viewport): the renderer left an 8 px gap under the 17 px
  classic title, so its drawings were 4 px shorter (a Marimekko segment's `10%` label, which needs
  14 px, was dropped); BIcraft's visuals now take the 4 px gap of classic charts.
- **Tooltips** show the hovered point's fields written with the fields' format strings in the
  report's locale (`275,0`, `43,1%`, the day `12/01/2026`), the box plot's statistics and the
  histogram's bin range (`45 – 50`).
- **Selection.** A click selects through the host and the column chart highlights the selected
  share; Ctrl+click adds; a click on the visual's background clears. With two category roles
  Desktop behaves as if it gave one identity per row (every category value of the row), so `withCategory` on one
  column of a row selects the row's combination: the box plot (category and sample) and the Venn
  diagram (item and set) selected only their first row. A box now selects all its rows and a Venn
  circle all the rows of its set (an array of ids, at most 5,000), and the page filters to the
  category or set; the Marimekko segment (category and series) and the org chart node (one row)
  already selected what they show. A selection in the column chart filtered the nine (they did
  not declare `supportsHighlight` then) and each drew the filtered data.
- **Context menu.** A right click on a point opens Desktop's data point menu (Copy, Show as a
  table, Include, Exclude, Group, Summarize, and the visual calculation entry).
- **Keyboard.** Ctrl+Right enters the visual, Tab reaches the first point with its accessible name
  (its tooltip lines), arrows, Home and End move between points, Enter or Space selects. Desktop
  redraws the visual after a selection: the focused point was replaced and the focus fell to the
  frame's document, so arrows and Escape no longer reached the visual. The runtime now gives the
  focus to the new point of the same row when the frame still has the focus; Escape then clears.
- **Landing page.** A Bullet without a Value field (added by `bicraft` that way, and another whose
  Value was removed in Desktop's Build pane) shows "Ajoutez un champ dans Valeur pour dessiner ce
  visuel."; an org chart without fields shows its name and the roles to fill.
- **Format pane.** Format visual lists each visual's cards in French (e.g. Intervalles, Couleurs,
  Étiquettes de données for the histogram). Setting the histogram's Nombre d'intervalles to 5 and
  switching Étiquettes de données on redraws it; Desktop's save writes `bins.count` as `5L` and
  `labels.show` as `true`, and `bicraft` reads them back (`visual.props`) and renders five bins with
  their counts as Desktop draws them (3, 3, 2, 2, 2).
- **Org chart collapse.** The button on Ivo's node collapses it ("Développer Ivo" then); Desktop's
  save writes `tree.toggled` as `["e1"]`, and `bicraft` renders the saved file with that node
  collapsed.
- **Round trip.** `bicraft` opens the PBIX Desktop saved: the nine packages are registered and
  current (`report.custom_visuals`), and `report.render` reports all ten visuals drawn.
- **A dropped package (G111).** A PBIX where `bicraft` removed the Venn, calendar and histogram
  visuals and saved opens in Desktop without a message, and its Visualizations pane lists only the
  six visuals still embedded. Yet the PBIX still held the three dropped visuals' files under
  `Report/CustomVisuals/` (the PBIX writer removed only unlisted `StaticResources` parts), and
  Desktop's save kept them too; the writer now removes a `Report/CustomVisuals/` part the report no
  longer lists, as for `StaticResources`.
- **Strings.** The data roles in Desktop's Build pane were in English in a French Desktop (the
  capabilities' `displayName`; no `stringResources` were packaged then). Since the packages carry
  `displayNameKey` and French `stringResources` (1.0.3.0 and later), a second run the same day saw
  every role in French for all ten visuals (below).

Not checked: high contrast (the visual host reports it from the Windows contrast setting, which
was not changed; Desktop's high-contrast report themes were not tried), a newer BIcraft build of a
package kept by an older BIcraft, touch input, and an English Desktop UI.

### Interactions with built-in visuals (2.159.1231.0, 2026-11-24)

Second black-box run on the same Desktop (Store edition, French UI), same method. `bicraft` wrote
a PBIX on a synthetic model (twelve people with a department, a parent, a tier, a score, amounts
and a day; a skills table related to it in both directions) with one page holding the ten
BIcraft visuals, a built-in clustered column chart (Total by Dept), a table (Name, Dept, Total),
a card (Total), a list slicer (Tier) and a slicer on the Day column; `page.interactions` set five
rules (Spec → table: none; Bullet → column chart: highlight; column chart → histogram: none,
→ Marimekko: highlight, → dumbbell: filter); a drillthrough page on Dept, a page filtered to two
departments and a report filter leaving one person out. Each visual's first point was clicked
and the built-ins and the nine other BIcraft visuals were read from the DOM (the column chart's
bars, the table's rows, the card's text, each visual's point count and `aria-selected`).

| Visual | (a) its mark → column chart, table, card | (b) column chart, slicers → it | (c) Edit interactions | (d) drillthrough, page and report filters | (e) bookmark |
|---|---|---|---|---|---|
| Spec | East: highlighted (135 of 135), card 185,0; the table kept every row (rule none honoured) | filtered | none rule honoured | filtered | selection restored |
| Histogram | bin 39–41: East 55 of 135, table Jade and Kai, card 75,0 | Tier and Day filter it; the column chart does not (rule none honoured) | none rule honoured | filtered | — |
| Bullet | East: highlighted 135, table Ivo, Jade, Kai, card 185,0 (rule highlight, as the default) | filtered | highlight rule honoured | its right-click Drill opens the drillthrough page filtered to the department | — |
| Box plot | East: as the bullet | filtered | — | filtered | — |
| Dumbbell | East: as the bullet | filtered (rule filter honoured) | filter rule honoured | filtered | — |
| Org chart | Ada: North 87 of 200, table Ada, card 120,0 | filtered | — | filtered (11 nodes without the person left out; 4 on the drillthrough page) | — |
| Venn | the SQL circle: 116, 58, 175, the table's six SQL people, card 480,0 | filtered (the Day range leaves 4 marks) | — | filtered | — |
| Marimekko | East · Bronze: East 18, table Kai, card 25,0 | filtered; the column chart's highlight rule draws it filtered | highlight rule: Desktop filters it | filtered (6 segments on the filtered page, as `report.render`) | — |
| Calendar | Ada's day: North 87, table Ada, card 120,0 | filtered | — | filtered (11 days, 8 on the filtered page) | — |
| Sankey | South → Gold: South 109, table Eve, card 150,0 | filtered | — | filtered | — |

- **(a)** A selection in any of the ten highlights the column chart (Desktop's default for that
  visual), filters the table and the card, and filters the nine other BIcraft visuals, which
  redraw the filtered rows (the histogram re-bins). A Venn region's count is a tooltip target
  only and selects nothing (a circle selects its set), by design.
- **(b)** A bar of the column chart, the Tier slicer and the Day range (its start date changed)
  filter all ten. The ten did not declare `supportsHighlight` in that run: in Edit interactions
  Desktop offered only Filter and None over them, and a `highlight` rule written by BIcraft
  towards one of them was applied as a filter.
- **Highlight support (after the run, 2026-11-24, seen in Desktop the same day).** Since chart
  visuals and pack 1.3.0.0 and Spec 1.4.0.0, the nine with a measure role declare
  `supportsHighlight` and read the data view's `highlights`: the Spec visual's bars, the bullet's
  value bar, the Marimekko's segments, the calendar's days and the Sankey's ribbons draw the
  highlighted part over the dimmed full value; the histogram's bins, the box plot's boxes, the
  dumbbell's rows, the org chart's nodes and the Sankey's nodes that nothing highlights are dimmed;
  the Venn diagram, which has no value column, stays filtered
  ([formats/bicraft-visuals.md](formats/bicraft-visuals.md) "Highlights"). BIcraft's own
  interactions do the same: a column chart or table selection highlights them (rules and the
  report's cross-filter default apply), slicers filter them. In Power BI Desktop 2.159.1231.0
  Edit interactions offers Filter, Highlight and None on the nine, Highlight by default (Filter
  and None on the Venn); a column chart bar or a table row highlights the nine as `bicraft
  render` draws the same selection, and filters the Venn. A visual of the nine with no field in
  its measure roles receives all its rows without `highlights` and stays as it is, as BIcraft
  now does too ([formats/bicraft-visuals.md](formats/bicraft-visuals.md) "Highlights").
- **(c)** Rules written by `page.interactions` are the ones Desktop applies and shows in Edit
  interactions; none, filter and highlight from a BIcraft visual towards a built-in behave as
  from a built-in.
- **(d)** The drillthrough from the bullet's context menu opens the target page with every
  BIcraft visual on it filtered to the department; the page filter and the report filter reach
  all ten, and `report.render` draws the filtered page with the same counts.
- **(e)** A bookmark taken with a Spec bar selected, applied after the selection was cleared,
  selects the bar again (the runtime's `registerOnSelectCallback`) and the built-ins are
  cross-filtered again; `bicraft` lists the bookmark from Desktop's save.
- **Fixed: Ctrl+click on a node of several rows.** The host's `select(ids, true)` toggles each
  id. On the Sankey, with Power selected (every link through it), Ctrl+click on Gas toggled
  Gas → Power off again and left only Gas → Heating, so the column chart showed Gas 45, not 125.
  The runtime now passes only the ids not yet selected when the row is not wholly selected
  (`multiIds`; the whole row when it is, to toggle it off); seen right in Desktop (Coal 120,
  Gas 125, Power 230, Solar 30). The same rule serves the Venn circle and the box. The packages'
  version numbers were not raised with the fix, so a report already holding 1.1.0.0 keeps its
  earlier build until a newer version is embedded.
- **Not seen in Desktop:** a table row or matrix cell as the source towards the ten, Edit
  interactions on the other seven visuals as sources (the rule is the host's, the same for all),
  bookmarks on the other nine, touch input.

## Built in by default (M12.4)

BIcraft's visuals are listed in one registry, `crates/engine/src/builtin_visuals.rs`
(`VISUALS`): each entry gives the visual's name (`pbiviz.json` `visual.name`, e.g.
`bicraftSpec`), its guid (the visual type its visuals carry and its package's name in the
report), display name, version, data roles (name, kind, most fields), a function that builds its
package once per process (`build_once` over its `visual_project::Project`) and its icon
(`builtin_visuals::icon_png`).
Registered today, fourteen visuals: the Spec visual (`spec_visual::SPEC`); the first pack's part A
(`bicraftHistogram`, `bicraftBullet`, `bicraftBoxPlot`, `bicraftDumbbell`), each through
`chart(bvis::DEFS[i])`; its part B (`bicraftOrgChart`, `bicraftVenn`, `bicraftMarimekko`,
`bicraftCalendarHeatmap`, `bicraftSankey`, `bicraftScatterPlus`, `bicraftLineAreaPlus`, `bicraftWaterfallPlus`, `bicraftGaugeKpi`), each through `pack(&chart_pack::VISUALS[i])`. Every package is still
built by its own module (`spec_visual`, `bvis`, `chart_pack`); `visual.set_spec`,
`visual.set_bicraft_visual` and `chart_pack::install` embed it through the registry's `ensure`,
so they all follow the same version rule. A new visual adds one line to `VISUALS`.

- **Icons** (2026-11-24). Each visual has its own drawing, BIcraft's original work in
  `crates/visuals/src/icons.rs`: marks (lines, fills, rounded rectangles, circles) on the UI
  icons' 24 × 24 design grid in two tones, the ink and the blue accent, as the pictures of the
  built-in types (docs/design.md §6). The Spec visual is a column chart between braces; the
  histogram contiguous bins; the bullet chart two bars over their bands with target ticks; the box
  plot two boxes with medians and whiskers; the dumbbell three rows from a ring to a dot; the org
  chart a head card linked to two cards; the Venn diagram three overlapping sets; the Marimekko
  full-height columns of different widths; the calendar heatmap a week grid of shaded days; the
  Sankey crossing flows between two pairs of nodes; Scatter+ dots of several sizes, filled or ringed,
  along a trend line under a marginal histogram; Line / Area+ a band shaded between two lines, the
  first ending in a dot. The pane paints that drawing in the theme's
  ink (light, dark, disabled, with the cells' hover and selected fills); each package carries the
  same drawing as its `assets/icon.png`, rasterized at 20 × 20 (the size Power BI shows) on a
  transparent background (`builtin_visuals::icon_png`). Since 2026-11-25 (G111) the PNG is
  darker and bolder than the gallery's drawing, near the weight of Desktop's own pane icons,
  which keep a dark ink (about `#3a3a38`) in both its themes: the ink is `#4a5058` (7.5:1 on
  Desktop's light pane `#f5f5f5`, 2.3:1 on its dark pane `#141414`), strokes are 1.3 times the
  UI's stroke rule (about 1.5 px at 20 × 20), and tones are fuller, ink at `1 - (1 - a)³` and the
  accent at `1 - (1 - a)²` of the drawing's opacity `a`, which keeps the Venn's overlaps and the
  calendar's shades apart. The gallery keeps the theme's ink. Tests: every drawing distinct and on the
  grid (`every_visual_has_its_own_drawing`), every gallery picture distinct and following the
  theme (`bicraft_visuals_have_distinct_pictures`), every package icon a distinct 20 × 20 PNG
  carried by its package (`every_visual_has_its_own_icon_in_its_package`), the package ink at
  least 4.5:1 on Desktop's light pane, each inked icon keeping 3:1 on white (WCAG 2.1 non-text
  contrast) and a one-stroke line covering about 1.5 px (`package_icons_have_dark_bold_ink`,
  seen failing on the previous ink), and the gallery
  screenshots (`visual_icon_screenshots`, `--features gpu-tests`). Seen in Power BI Desktop
  2.159.1231.0 (2026-11-24) with the app in its dark and light themes: the Visualizations pane
  shows the ten icons, each distinct, on buttons named "BIcraft …". Desktop draws the PNG as the
  button's background at its own size in a 20 × 20 CSS-pixel box, so on a scaled screen (150 %)
  it is upscaled and looks soft, and the slate ink of that release read fainter than the built-in
  types' vector icons in both themes. A larger PNG would be cropped, not scaled down, so the
  icons stay 20 × 20 and the 2026-11-25 ink is darker and bolder instead; seen in Power BI
  Desktop 2.160.1151.0 (2026-11-25, dark theme, 150 %) at about the weight of its own icons. The
  upscale's softness stays (G111).
- **Visualizations pane.** The snapshot's `report.bicraft_visuals` lists every entry, embedded
  or not, with its icon (docs/view-snapshot.md); the pane (desktop and browser, the same egui
  code) draws them right after the built-in types, then the report's other custom visuals. An
  embedded BIcraft visual is not listed twice.
- **`visual.add` and `visual.set_type`** take the name or the guid (ignoring case). The visual
  stores the guid; `visual.add` spells roles as the visual declares them and refuses a role it
  does not have unless `force`; `visual.set_type` moves fields into its roles as for a built-in
  type. Once nothing else can refuse the change, the package is embedded (`installed: true`):
  when it is missing, not registered with its item named as its file, or an older version; the
  same build or a newer version (from a later BIcraft) is kept. `visual.set_spec` follows the same
  rule. Legacy (`Report/Layout`) reports are refused, as `visual.import_custom` refuses them.
- **`report.apply`** (2026-11-24, G111) takes the same names and guids in a visual's `type`: the
  visual stores the guid, its fields' roles are spelled as the visual declares them and a role
  it does not have is an error unless `force`; once the whole plan is valid, the package of each
  BIcraft visual the plan creates or changes is embedded first (`installed` lists the names),
  then the plan is applied. Re-applying the same spec changes and installs nothing.
- **A Spec visual starts with a spec** (2026-11-24, G111): `visual.add`, `visual.set_type` and
  `report.apply` give a new Spec visual without a spec `visual.spec_template`'s, matched to its
  fields by count only: `combo` (a bar and a line over `value.1`) when it binds two values or
  more, else `bar` (`spec_template` in the result). Since 2026-11-25 (G119, Spec 1.5.0.0) it is
  always `bar`, which draws each value bound as its own series with a legend
  ([formats/spec-visual.md](formats/spec-visual.md) "Series"). It then draws at once instead of showing its
  missing-spec message; `visual.set_spec` replaces it. The template is not chosen by the
  category's data type (a date category still gets bars).
- **`report.custom_visuals`** adds `bicraft`: each entry with `embedded`, `embedded_version`,
  `current` and `used_by`; an embedded package of one of them carries `bicraft: <name>`.
- **Save.** `file.save` and `file.save_as` first drop the package of each BIcraft visual that the
  edited report carries but no visual uses (a report without unsaved edits, or a legacy one, is
  written as it is). Packages imported with `visual.import_custom` under another guid are never
  touched.

Checks: `crates/engine/tests/report/builtin_visuals_tests.rs` (names and guids, roles, first use,
older and newer builds, `visual.set_type`, `report.apply` by name, the starting spec, a save
dropping an unused package and keeping a used one) and the UI test `bicraft_visuals_are_in_the_gallery_before_any_import`
(`crates/ui/src/tests/custom_visuals_ui_tests.rs`).

## Outside BIcraft (M12.5)

BIcraft's visuals ship with BIcraft and as `.pbiviz` files, nothing else. By the maintainers'
decision of 2026-11-23 they are not submitted to the Microsoft Marketplace (AppSource) or the
Microsoft Store, and no certification is sought.

- **`visual.export_bicraft_visual {name, out}`** writes the `.pbiviz` of one visual, of a list or
  of `all` (into a folder, one `<guid>.<version>.pbiviz` each): the package BIcraft embeds, byte
  for byte. A report author imports it in Power BI Desktop with the Visualizations pane's "…" ›
  Import a visual from a file; a tenant administrator can upload it as an organizational visual.
  It needs no open file and leaves the session as it is. Also `bicraft visual export NAME... |
  --all [--out F|DIR]` and Insert › Visuals › Export a BIcraft visual (docs/formats/pbiviz.md).
- **Release archives.** `cargo xtask dist` runs the release binary's `visual export --all` and
  puts the nine files under `visuals/` in each archive, with a `visuals/README.md` that names the
  visuals and points to their TypeScript sources in this repository
  (`crates/engine/src/spec_visual/`, `bvis/`, `chart_pack/` and the shared runtime
  `visual_runtime/`), under BIcraft's licence.

Checks: `every_bicraft_visual_exports_and_imports_back` and
`export_one_visual_a_list_and_refusals` (`builtin_visuals_tests.rs`: every exported file read back
by `visual.import_custom` as the current package, its capabilities valid against the API schema),
the CLI test `visual_export_writes_bicraft_visuals` and the UI test
`insert_export_a_bicraft_visual`. Seen in Power BI Desktop 2.159.1231.0 (2026-11-24): the
exported histogram and Venn imported through its Import a visual from a file dialog (after its
custom visual warning), drew their data on a page and were saved at 1.0.2.0; the Build pane
names their data roles in English in a French Desktop. Not checked: an upload as an
organizational visual in the Power BI service (G112).

**Power BI service (not tried; from Microsoft's public documentation, 2026-11-24).** A report
embedding BIcraft's visuals should publish and draw in the service when the tenant allows
visuals created with the Power BI SDK and does not restrict reports to certified visuals
([Power BI visuals tenant settings](https://learn.microsoft.com/en-us/fabric/admin/service-admin-portal-power-bi-visuals));
where only organizational visuals are allowed, an administrator can upload the exported
`.pbiviz` files ([organizational visuals](https://learn.microsoft.com/power-bi/admin/organizational-visuals)).
Microsoft supports export to PowerPoint and PDF and email subscriptions for certified visuals
([custom visuals FAQ](https://learn.microsoft.com/power-bi/developer/visuals/power-bi-custom-visuals-faq)),
and BIcraft's visuals are not certified by decision, so they are expected to be left out of those
exports. They make no network calls and use no R or Python. Power BI Report Server (on premises)
takes custom visuals too, with its own Desktop release; not tried either.

## Gaps in the evidence

- Vote counts come from archived copies (2024-06 to 2026-06), not from today's pages; the live
  pages refuse automated readers. A maintainer reading the live pages in a browser would refresh
  them.
- No measured idea was found for the dumbbell, slope, bump, calendar heatmap, Sankey, word cloud
  and several others. The archive's index of the Fabric Ideas board is too large to list fully;
  a search of archived idea titles starting with 44 chart words (Bullet, Box, Histogram, Dumbbell,
  Slope, Bump, Calendar, Marimekko, Sankey, Violin, Native, Add-…) found only bug reports beyond
  the ideas cited, so ideas whose titles start otherwise may exist.
- Old-board counts and Fabric Ideas counts are not comparable, and counts on several ideas for the
  same chart are not summed.
- The *free + paid* label reads one flag of the Marketplace data; each listing's terms were not
  read. The Marketplace data has no API version.
- Built-in coverage follows Microsoft Learn, not a check in Desktop 2.159.

[fi]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/idb-p/fbc_ideas
[cvi-label]: https://community.fabric.microsoft.com/t5/Custom-Visuals-Ideas/idb-p/CustomVisualsIdeas/label-name/visual
[learn-vis]: https://learn.microsoft.com/en-us/power-bi/visuals/power-bi-visualizations-overview
[learn-wf]: https://learn.microsoft.com/en-us/power-bi/visuals/power-bi-visualization-waterfall-charts
[learn-bin]: https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-grouping-and-binning
[dc]: https://github.com/DataChant/PowerBI-Visuals-Marketplace
[dg]: https://data-goblins.com/power-bi/visual-wishlist
[sqlbi-bullet]: https://www.sqlbi.com/articles/building-bullet-charts-in-power-bi-reports/
[api-dep]: https://powerbi.microsoft.com/blog/announcement-custom-visual-api-deprecation
[e-org]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Organization-chart/idi-p/4520633
[w-org]: https://web.archive.org/web/20260418/https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Organization-chart/idi-p/4520633
[i-org1]: https://community.fabric.microsoft.com/t5/Custom-Visuals-Ideas/Org-chart-visual/idi-p/221878
[w-org1]: https://web.archive.org/web/20240624/https://community.fabric.microsoft.com/t5/Custom-Visuals-Ideas/Org-chart-visual/idi-p/221878
[e-wf]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-New-waterfall/idi-p/4491547
[w-wf]: https://web.archive.org/web/20260124/https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-New-waterfall/idi-p/4491547
[i-wf2]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/Improving-Waterfall-chart/idi-p/4460988
[w-wf2]: https://web.archive.org/web/20260313/https://community.fabric.microsoft.com/t5/Fabric-Ideas/Improving-Waterfall-chart/idi-p/4460988
[e-hist]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Histogram-support/idi-p/4491231
[w-hist]: https://web.archive.org/web/20260313/https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Histogram-support/idi-p/4491231
[i-hist2]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/Power-BI-Histogram/idi-p/4797515
[w-hist2]: https://web.archive.org/web/20260417/https://community.fabric.microsoft.com/t5/Fabric-Ideas/Power-BI-Histogram/idi-p/4797515
[t-hist]: https://community.fabric.microsoft.com/t5/Desktop/What-happened-to-Histogram-chart-custom-visual-from-Microsoft/td-p/2978494
[e-lg]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Linear-gauge/idi-p/4534016
[w-lg]: https://web.archive.org/web/20260520/https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Linear-gauge/idi-p/4534016
[e-box]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Box-and-wiskers/idi-p/4521016
[w-box]: https://web.archive.org/web/20260312/https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Box-and-wiskers/idi-p/4521016
[i-box2]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/Box-and-whisker/idi-p/4539144
[w-box2]: https://web.archive.org/web/20260119/https://community.fabric.microsoft.com/t5/Fabric-Ideas/Box-and-whisker/idi-p/4539144
[i-box3]: https://community.fabric.microsoft.com/t5/Custom-Visuals-Ideas-read-only/Introduce-Standard-Box-Plot-Visualizations/idi-p/2883736
[w-box3]: https://web.archive.org/web/20260305/https://community.fabric.microsoft.com/t5/Custom-Visuals-Ideas-read-only/Introduce-Standard-Box-Plot-Visualizations/idi-p/2883736
[t-box]: https://community.fabric.microsoft.com/t5/Health-and-Life-Sciences/Idea-Box-and-Whisker-or-Violin-Plot-Visuals-as-Standard-Visuals/td-p/4796172
[i-venn]: https://community.fabric.microsoft.com/t5/Custom-Visuals-Ideas-read-only/3-Way-Venn-Diagram-Proportional-and-Dynamic/idi-p/180907
[w-venn]: https://web.archive.org/web/20260115/https://community.fabric.microsoft.com/t5/Custom-Visuals-Ideas-read-only/3-Way-Venn-Diagram-Proportional-and-Dynamic/idi-p/180907
[i-venn2]: https://community.fabric.microsoft.com/t5/Custom-Visuals-Ideas/Dynamic-Venn-Diagrams/idi-p/1992802
[w-venn2]: https://web.archive.org/web/20241203/https://community.fabric.microsoft.com/t5/Custom-Visuals-Ideas/Dynamic-Venn-Diagrams/idi-p/1992802
[e-tmb]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Treemap-bars/idi-p/4491799
[w-tmb]: https://web.archive.org/web/20260306/https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Treemap-bars/idi-p/4491799
[t-calhm]: https://community.fabric.microsoft.com/t5/Power-BI-Community-Blog/Visualizing-an-Entire-Year-with-a-Power-BI-Calendar-Heatmap/ba-p/4816771
[t-calhm2]: https://community.fabric.microsoft.com/t5/Desktop/Help-with-Calendar-Heat-map/m-p/4807350
[t-calhm3]: https://community.fabric.microsoft.com/t5/Data-Stories-Gallery/2023-Week-06-Power-BI-Create-a-Calendar-Heatmap/td-p/3078378
[e-stream]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Stream-chart/idi-p/4521138
[w-stream]: https://web.archive.org/web/20260210/https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Stream-chart/idi-p/4521138
[i-sankey]: https://community.fabric.microsoft.com/t5/Custom-Visuals-Ideas/Sankey-diagram/idi-p/3440502
[i-sankey2]: https://community.fabric.microsoft.com/t5/Custom-Visuals-Ideas/How-can-I-make-requests-for-the-Sankey-visualization-or/idi-p/2883855
[t-sankey]: https://community.fabric.microsoft.com/t5/Desktop/Sankey-Chart/m-p/5184292
[i-pareto]: https://community.fabric.microsoft.com/t5/Custom-Visuals-Ideas/Pareto-Chart/idi-p/2275985
[w-pareto]: https://web.archive.org/web/20231204/https://community.fabric.microsoft.com/t5/Custom-Visuals-Ideas/Pareto-Chart/idi-p/2275985
[i-wc]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/Word-Cloud-Visualization/idi-p/4538659
[w-wc]: https://web.archive.org/web/20250813/https://community.fabric.microsoft.com/t5/Fabric-Ideas/Word-Cloud-Visualization/idi-p/4538659
[t-radar]: https://community.fabric.microsoft.com/t5/Desktop/Radar-Chart/m-p/1909088
[e-gantt]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Gantt-chart/idi-p/4521163
[w-gantt]: https://web.archive.org/web/20260211/https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Gantt-chart/idi-p/4521163
[i-heat]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/Heat-maps-in-Power-View/idi-p/4426617
[w-heat]: https://web.archive.org/web/20260306/https://community.fabric.microsoft.com/t5/Fabric-Ideas/Heat-maps-in-Power-View/idi-p/4426617
[t-heatstream]: https://community.fabric.microsoft.com/t5/Desktop/Heatstream-Visual-being-Deprecated/m-p/3997889
[i-card]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/Update-Card-Browser-visual-instead-of-deprecating-it/idi-p/4816612
[w-card]: https://web.archive.org/web/20260612/https://community.fabric.microsoft.com/t5/Fabric-Ideas/Update-Card-Browser-visual-instead-of-deprecating-it/idi-p/4816612
[t-card]: https://community.fabric.microsoft.com/t5/Desktop/Card-Browser-Deprecation/m-p/4793679
[t-html]: https://community.fabric.microsoft.com/t5/Desktop/What-happened-to-the-HTML-Viewer-Custom-Visual-It-is-no-longer/m-p/1023660
[t-chart]: https://community.fabric.microsoft.com/t5/Custom-Visuals-Development/Power-BI-Charticulator-Visuals-and-Retirement/td-p/4003267
[i-combo]: https://community.fabric.microsoft.com/t5/Custom-Visuals-Ideas-read-only/Add-Extensible-Excel-Style-quot-Custom-Combo-Chart-quot-to-Power/idi-p/4357804
[w-combo]: https://web.archive.org/web/20251118/https://community.fabric.microsoft.com/t5/Custom-Visuals-Ideas-read-only/Add-Extensible-Excel-Style-quot-Custom-Combo-Chart-quot-to-Power/idi-p/4357804
[e-area]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-100-Area-chart/idi-p/4520655
[w-area]: https://web.archive.org/web/20260412/https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-100-Area-chart/idi-p/4520655
[e-smc]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Small-multiples-for-cards/idi-p/4535704
[w-smc]: https://web.archive.org/web/20260515/https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Small-multiples-for-cards/idi-p/4535704
[e-ctl]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Cards-with-trendlines/idi-p/4534049
[w-ctl]: https://web.archive.org/web/20260124/https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Cards-with-trendlines/idi-p/4534049
[e-sm]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Small-multiples/idi-p/4535205
[e-lwb]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Legends-with-bars/idi-p/4540346
[e-anim]: https://community.fabric.microsoft.com/t5/Fabric-Ideas/Epic-idea-Animations-for-data-visualizations/idi-p/4537710
[gh-hist]: https://github.com/microsoft/powerbi-visuals-histogram
[gh-heatstream]: https://github.com/microsoft/PowerBI-visuals-HeatStreams
[gh-card]: https://github.com/microsoft/PowerBI-visuals-CardBrowser
[gh-chart]: https://github.com/microsoft/charticulator
[a-maqorg]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA200001120
[a-akv]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104381333
[a-wfv]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/workforcevision.org-chart-drag-and-drop
[a-cporg]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/chartpad.performance
[a-sw]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA200000195
[a-uw]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104380956
[a-xwf]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA200001336
[a-zcwf]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA200000767
[a-cbthist]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/convergentbusinesstechnologies1681574339335.cbt_histogram
[a-maqhist]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104381032
[a-tchist]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/tino_callarisa.histogram-pro-tcviz
[a-pvehist]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/yavdaanalyticspvtltd1628223732998.histogrampro
[a-pqhist]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA200001046
[a-msbullet]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104380755
[a-okbullet]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104380953
[a-xbullet]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA200000933
[a-cbtbullet]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/convergentbusinesstechnologies1681574339335.cbt_bullet_chart
[a-maqlg]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104380821
[a-maqbox]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104381351
[a-dsbox]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104380831
[a-pvebox]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/yavdaanalyticspvtltd1628223732998.boxandwhiskerpro
[a-svbox]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/smartvisuals.smartvisuals-box-plot
[a-maqvenn]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104381231
[a-sgvenn]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA200002019
[a-svvenn]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/smartvisuals.smartvisuals-venn-upset
[a-msmekko]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104380785
[a-xmekko]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA200000184
[a-atmekko]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/atlyn.atlyn-marimekko
[a-nsdb]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA200002332
[a-cbtdb]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/convergentbusinesstechnologies1681574339335.cbt_dumbbell_plot
[a-pvlol]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/truvizinc1674781244292.lollipop-chart-by-powerviz
[a-svslope]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/smartvisuals.smartvisuals-slope
[a-bqslope]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/briqbianalyticsprivatelimited1776423580325.briqlab-slope-chart
[a-okcal]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/okvizcorp1634637213047.calendarprobyokviz
[a-dvcal]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/0211995.calendar_heatmap
[a-vxcal]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/tmall-tech.procalendarheatmap
[a-maqcal]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104381844
[a-msstream]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104380772
[a-plstream]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/plume-studio.stream-graph
[a-cpbump]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/chartpad.bump
[a-ifbump]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/ifourtechnolabprivatelimited1715753920355.ifourtechnolab_bumpchart
[a-r42bump]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/r42-communication.time-bump
[a-mssankey]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104380777
[a-vcsankey]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/visio-chart.sankey_diagram_visiochart
[a-cesankey]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/polyvista1585910805469.sankey_diagram_for_power_bi_by_chartexpo
[a-dmpviolin]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104381947
[a-maqviolin]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/maqsoftware1587623472284.violinplotbymaqsoftware
[a-sio2]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA200001423
[a-tcpareto]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/tino_callarisa.pareto-chart-pro-tcviz
[a-cbtpareto]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/convergentbusinesstechnologies1681574339335.cbt_pareto_chart
[a-mswc]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104380752
[a-tcwc]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/tino_callarisa.word-cloud-tcviz
[a-waffle]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104381049
[a-r42waffle]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/r42-communication.waffle
[a-mschord]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104380761
[a-msforce]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104380764
[a-pvnet]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/truvizinc1674781244292.network-graph-by-powerviz
[a-maqradar]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA200001561
[a-msradar]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104380771
[a-maqsun]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA200000097
[a-mssun]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104380767
[a-mstornado]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104380768
[a-okcandle]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104380952
[a-msgantt]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104380765
[a-maqgantt]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104381364
[a-msth]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA104380818
[a-pvheat]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/truvizinc1674781244292.heatmap-by-powerviz
[a-htmlsec]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/coacervolimited1596856650797.htmlcontent_certified
[a-html]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/WA200001930
[a-chview]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/ilfatgaliev1696579877540.charticulator_visual_community_view
[a-ched]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/ilfatgaliev1696579877540.charticulator_visual_community_editor
[a-deneb]: https://appsource.microsoft.com/en-us/product/power-bi-visuals/coacervolimited1596856650797.deneb
