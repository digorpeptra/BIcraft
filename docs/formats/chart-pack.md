# BIcraft's chart pack: organization chart, Venn, Marimekko, calendar heatmap, Sankey, Scatter+, Line / Area+

Sources: our own format. Each visual is a custom visual of BIcraft's, built as
[pbiviz.md](pbiviz.md) "Visual projects" describes (Microsoft Learn's custom visual pages and the
MIT `powerbi-visuals-api` 5.3.0 types: `IVisual`, `VisualUpdateOptions`, the categorical data
view, `FormattingModel`, `IVisualHost.persistProperties`), on BIcraft's visual runtime
([community-visuals.md](../community-visuals.md) "Runtime"). The chart types come from the M12.1
needs ledger; the drawings, names and code are BIcraft's (no existing visual's code, text or
image was read or copied). ROADMAP M12.3, part B of the first pack; the Sankey diagram (ledger
row 12) was added to it on 2026-11-24 at the user's request. The Waterfall+ and Gauge / KPI+,
built the same way, have their own note ([waterfall-gauge.md](waterfall-gauge.md)).

## The visuals

| | Organization chart | Venn diagram | Marimekko | Calendar heatmap | Sankey |
|---|---|---|---|---|---|
| display name | BIcraft Org Chart | BIcraft Venn | BIcraft Marimekko | BIcraft Calendar Heatmap | BIcraft Sankey |
| guid (visual type) | `bicraftOrgChart3C8E1A6F2D4B9E7A5C1F0D2B` | `bicraftVenn9A2E4C6B8D1F3A5E7C9B0D2F` | `bicraftMarimekko5D7F9B1E3A6C8E0B2D4F6A` | `bicraftCalendarHeatmap1E3B5D7F9A2C4E6B8D0F` | `bicraftSankey7B3D5F1A9C2E4B6D8F0A1C3E` |
| version | 1.3.5.0 | 1.3.4.0 | 1.3.5.0 | 1.3.3.0 | 1.3.6.0, the Sankey 1.3.7.0 (each raised on 2026-11-25 for the runtime's text settings it bundles, G119, nothing it draws changed; the Sankey's 1.3.7.0, 2026-11-26, places its labels beside the plot, below) |
| data roles (at most one field each) | `id`, `parent` (Grouping, required), `label` (Grouping), `value` (Measure) | `item`, `set` (Grouping, required) | `category` (Grouping, required), `series` (Grouping), `value` (Measure, required) | `date` (Grouping, required), `value` (Measure, required) | `source`, `destination` (Grouping, required), `weight` (Measure, required) |
| data view | categorical: categories `id`, `parent`, `label` (top 1,000), values `value` | categorical: categories `item`, `set` (top 5,000) | categorical: categories `category`, `series` (top 3,000), values `value` | categorical: category `date` (top 4,000), values `value` | categorical: categories `source`, `destination` (top 3,000), values `weight` |
| format objects | `nodes`: `fill`, `fontColor`, `fontSize`, `showValues`; `tree`: `levels`, `toggled`, `compact`, `leftToRight` | `sets`: `color1`–`color3`, `proportional`; `labels`: `show`, `fontSize` | `colors`: `series1`–`series6`; `labels`: `show`, `fontSize`; `legend`: `show`; `columns`: `sort`, `widthAxis` | `colors`: `low`, `high`, `empty`; `legend`: `show`; `range`: `months`, `sundayFirst` | `nodes`: `width`, `padding`; `colors` (since 1.3.4.0): `node1`–`node12`; `links`: `colorBy` (enumeration `source`, `destination`, `fixed`), `fill`, `opacity`; `labels`: `show`, `fontSize` |
| look (all five, since 1.1.0.0) | `style`: `preset`, `palette`, `rounded`, `gradient`, `shadow`, `animation`, `card`, `pills` (since 1.2.0.0), the layouts' options `stylePreset` … `stylePills`, and the input's `theme` (`{colors, background?}`); see [visual-look.md](visual-look.md). A colour left unset is the look's (the theme's first colours); Minimal draws what 1.0.3.0 drew | | | | |

Each package holds `src/visual.ts` (the `IVisual` class, `chart_pack/<kind>_visual.ts` in the
repository), its layout `src/<kind>.ts`, the pack's shared `src/kit.ts` (layout helpers) and
`src/base.ts` (data view in, SVG out, runtime wiring), the Spec visual's `src/spec.ts` (number and
date code) and `src/runtime/*.ts`. Capabilities also carry `privileges: []`, `tooltips`,
`supportsLandingPage`, `supportsEmptyDataView`, `supportsKeyboardFocus`; since 1.3.0.0
`supportsHighlight` for the four with a measure (not the Venn diagram): the Marimekko, calendar and
Sankey draw highlighted parts, the org chart dims nodes ([bicraft-visuals.md](bicraft-visuals.md)
"Highlights").

`bicraft_engine::chart_pack::install(session, name)` builds the package once per process and
embeds it as `visual.import_custom` does, when the report lacks it or holds an older build (a
newer one is kept). Since M12.4 the pack's visuals are in BIcraft's visual registry
(`crate::builtin_visuals`, docs/community-visuals.md "Built in by default"): the Visualizations
pane lists them, and `visual.add {visual_type: <name or guid>, fields: [{role, field}]}` embeds the
package on first use and places a visual (e.g. `bicraftOrgChart`, `bicraftCalendarHeatmap`,
`bicraftSankey`).

Version 1.0.4.0 of the first four (2026-11-24) only rebuilds them with the shared `base.ts` that
now also draws `poly` primitives (the Sankey's ribbons); their drawings did not change. Version
1.1.0.0 of all five (2026-11-24) draws with BIcraft's look ([visual-look.md](visual-look.md));
the Sankey's nodes are the look's bars in its colours and its `links.colorBy` items have their
`displayNameKey`; under the Minimal preset every visual draws what 1.0.4.0 (the Sankey 1.0.0.0)
drew. Version 1.2.0.0 of all five (2026-11-24, the look's second pass) adds the card and the
value chips, the dark-background tokens and larger text, colours the org chart's branches, fills
the calendar's frame (below), and routes the Sankey's links that skip columns through waypoints
(a fix made under every preset, so the Minimal Sankey with such links differs from 1.1.0.0).
Version 1.3.3.0 of the org chart and the Marimekko (2026-11-25) adds the org chart's left-to-right
orientation (`tree.leftToRight`) and the Marimekko's series colours (`colors.series1`–`series6`);
without them they draw what 1.3.2.0 drew. Version 1.3.4.0 of the Sankey (2026-11-25; 1.3.3.0 is the
org chart's and the Marimekko's) adds the node colours (`colors.node1`–`node12`); without them it
draws what 1.3.2.0 drew. The next versions of the five (2026-11-25: org chart and Marimekko 1.3.4.0,
Venn 1.3.3.0, calendar 1.3.2.0, Sankey 1.3.5.0) change only the package icon, darker and bolder
(G111); they draw what the versions before drew. Version 1.3.7.0 of the Sankey (2026-11-26) moves
its node labels off the ribbons ("Labels" under Sankey below).

## One input, two drawers

Each layout exists twice, in `crates/render/src/chart_pack/<kind>.rs` and
`crates/engine/src/chart_pack/<kind>.ts`, and both read the same input:

```json
{"rows": [["ceo", null, "Ada", 120], ["cto", "ceo", "Grace", 45]],
 "options": {"color": "#1F6F8B", "levels": 0, "toggled": ["cto"]},
 "lang": "en"}
```

`rows` holds one array per data row with the cells of the kind's roles in the order of the table
above (null for an unbound role); `options` the format properties under their layout keys
(`color`, `textColor`, `fontSize`, `showValues`, `levels`, `toggled`, `compact`, `leftToRight`; `color1`–`color3`,
`proportional`, `showCounts`, `fontSize`; `color1`–`color6`, `showLabels`, `fontSize`, `showLegend`, `sortColumns`,
`widthAxis`; `low`, `high`, `empty`, `showLegend`, `monthsOnly`, `sundayFirst`; `nodeWidth`,
`nodePadding`, `nodeColor1`–`nodeColor12`, `linkColor`, `linkFill`, `linkOpacity`, `showLabels`, `fontSize`); `lang` is `fr` for a French host locale (BIcraft's renderer: the snapshot's UI
language), else `en`. In Power BI the visual builds it from the data view (a `Date` as ISO text on
its own clock); in BIcraft the snapshot carries `chart_pack: {kind, visual, options}` (read from
the PBIR objects) and the renderer adds the rows of the visual's evaluated data.

Since 1.0.3.0 (2026-11-24) the input also has `formats`: the format string of each cell's
column, in the order of `rows` (the data view's `source.format`; in BIcraft the snapshot
column's `fmt`). A date cell drawn as text (the Marimekko's category and series names, the
organization chart's labels and ids standing for a missing label, the Venn diagram's set names)
is written with it in `lang`, as `dateLabel` / `bicraft_render::date_label` write it
([bicraft-visuals.md](bicraft-visuals.md) "Dates in labels"); categories, series, nodes and sets
are still told apart by their value. The calendar draws its own month and day names.

Since 1.0.3.0 the data roles and format names have a `displayNameKey` and the package carries
their English and French `stringResources` ([bicraft-visuals.md](bicraft-visuals.md) "Names in
English and French").

Both return the same primitives (`rect`, `line`, `poly`, `circle`, `text`, rounded half up to hundredths)
and hits (`{row, prim, …}`: the drawn element of a data point, its first data row). Text widths
are estimated as `characters × size × 0.55`; numbers are written by our own code. Selection,
tooltips, focus and high contrast change only attributes of the drawn elements, so BIcraft's
renderer draws a visual unselected as Power BI draws it.

Common layout: 8 px padding; legends are a row of 10 px swatches and 11 px names (cut with an
ellipsis at 140 px, dropped once the row is full); texts in `#333333` or `#555555`; BIcraft's
eight palette colours with a text colour per colour of at least 4.5:1 contrast.

### Organization chart

- Nodes: the first row of each non-empty id (at most 1,000); the label is the `label` cell, else
  the id; numbers are written as BIcraft writes them (`1,234`, `4.50`).
- Parents: a node whose parent id is another node's id hangs under it; others are roots, in row
  order. Nodes left in a cycle (no root above them) are drawn from the first of them as a root;
  each node appears once.
- Open nodes: with `levels` 0 (default) every node is open; with `levels` n, nodes at depth below
  n − 1 are open. A node whose id is in `toggled` is the opposite. A closed node hides its subtree.
- Compact tidy tree (`tree.compact`, on by default, since 1.0.2.0): children before parents,
  each node's visible children are placed left to right, each subtree as far left as its
  contour allows (one unit between any two nodes of a level, compared depth by depth with the
  subtrees already placed), and the node sits at the middle of its first and last child; the
  roots are placed the same way. A narrow subtree beside a wide one thus takes less room than a
  leaf per slot. With `compact` off, the visible leaves take equal slots left to right in
  depth-first order and a parent sits at the middle of its first and last visible child. The
  width in units is the span of the placed nodes plus one: slot width `(w − 16) / units`, level
  height `(h − 16) / levels`; boxes `min(160, 0.85 × slot)` × `min(44, 0.6 × level)`, at the top
  of their level; connectors (1 px `#B3B3B3`) go down from the parent, across its children and
  down to each child.
- A box shows its label (the `fontSize`, default 11, in `fontColor`, default white), with its
  value under it when `showValues` and the box is at least `2 × size + 6` high; texts are cut
  with an ellipsis to the box width less 8 px.
- A node with children gets a round button (radius 6, `#605E5C`, `+` closed, `−` open) on its
  bottom edge when boxes are at least 16 × 12 px.
- Left to right (`tree.leftToRight`, off by default, since 1.3.3.0): the same placement with the
  axes swapped. The slots run down (slot height `(h − 16) / units`) and the levels across (level
  width `(w − 16) / levels`); boxes `min(160, 0.75 × level)` × `min(44, 0.85 × slot)`, at the left
  of their level and centred on their slot; connectors go right from the parent's right edge to
  the middle of the gap, down along its children and right to each child; the button sits on the
  box's right edge, at its middle.

In Power BI the button (click, Enter or Space) toggles the node and persists the list as the JSON
text of `tree.toggled` (`host.persistProperties`), so BIcraft's renderer and a saved report draw
the same tree. Each box is a data point: tooltip (label, id, parent, value), selection by id.

In BIcraft (since 2026-11-24) the button is a hit region of the renderer too (tooltip: Collapse or
Expand and the node's label). A click sends `view.interact {action: "tree_toggle", page, visual,
id}`: in the app's editing view it runs `visual.toggle_node {page, visual, id}`, which flips the id
in `tree.toggled` as the TypeScript does (removed when there, else added last, the newest 500
kept), saved with the file and undoable; a reader (the live viewer, a reader session) keeps the
list in its view state, as the visual does where the host persists nothing.

### Venn diagram

- Sets: the distinct non-empty `set` values (at most 50, in row order), each with its distinct
  non-empty items (at most 5,000 rows read). The three largest are drawn (ties: the first seen),
  in the order they were seen; `N more sets not drawn` / `N autres ensembles non dessinés` says
  how many were left out.
- By default the circles never depend on the counts, so the drawing does not move when the data
  changes: with the area below the legend (`w − 16` × `h − 38`, 14 px less with the note) of centre
  (cx, cy), one set is a circle of radius `min(w, h) / 2`; two sets have radius
  r = `min(w / 3, h / 2)` and centres `cx ± r / 2`; three sets have radius
  r = `min(w / 3, h / (2 + √3 / 2))` and centres on an equilateral triangle of side r
  (`(cx ∓ r / 2, gy − r / (2√3))`, `(cx, gy + r / √3)`, centroid `gy = cy − r√3 / 12`). Fills at
  opacity 0.35 in `color1`–`color3` (BIcraft's first three colours).
- Area-proportional (`sets.proportional`, off by default, two sets only, since 1.0.2.0): radii
  `√|A|` and `√|B|`, the centres `d` apart where the overlap of the two circles has the area
  `π·|A ∩ B|` (60 halvings of `[|ra − rb|, ra + rb]` over the circle–circle overlap area;
  touching when no item is shared, one inside the other when every item of the smaller set is
  shared), then scaled to fit the area (width `max(ra, d + rb) − min(−ra, d − rb)`, height
  `2 max(ra, rb)`) and centred. The counts are written at the middle of each crescent (`A only`
  between A's left edge and B's, `B only` between A's right edge and B's) and of the overlap;
  an empty crescent's 0 sits on the circle's edge. One or three sets keep the fixed circles.
- Counts: each item counts once, in the region of exactly the drawn sets it belongs to; every
  region's count (also 0) is written at a fixed point of the region (12 px, `fontSize`).

In Power BI a circle is a data point (tooltip: the set and its items; selection by every row of the
set, at most 5,000, which cross-filters the page: Power BI gives one identity per item and set
row, seen in Desktop 2.159.1231.0 on 2026-11-24); a region's count is a focusable point with a tooltip naming the
region (`P ∩ Q (not R)`) and its count, not selectable.

### Marimekko

- Categories (at most 50) and series (at most 20) in row order; a segment's value is the sum of
  its rows' positive values (negatives and blanks count 0).
- Plot: a legend row when there is a named series, a 36 px left axis with 0 %, 25 %, 50 %, 75 %,
  100 % and gridlines, an 18 px bottom row for the category names (cut to the column width).
- A column's width is its share of the grand total (1 px gap); its segments stack from the bottom
  in series order, each as high as its share of the column. A segment's share (`42%`) is written
  in it when it fits (the colour's text colour, `fontSize` default 10).
- Colours: series k takes the look's palette colour k (the report theme's, or BIcraft's with the
  Style card's BIcraft colors, as the other visuals; since 1.1.0.0), and since 1.3.3.0 the first
  six series take `colors.series1`–`series6` when set (layout options `color1`–`color6`; an
  invalid colour is ignored); the format pane shows each with the palette colour as its default.
- Columns (since 1.0.2.0): `columns.sort` draws the widest column first (ties in row order;
  off by default: row order); `columns.widthAxis` adds a width axis over the plot, 14 px high:
  0 %, 25 %, 50 %, 75 %, 100 % of the grand total (9 px, the first left-aligned, the last
  right-aligned) over 3 px ticks.

In Power BI a segment is a data point (tooltip: category, series, value, share of the category;
selection by category and series).

### Calendar heatmap

- Days: the `date` cells (ISO text or a `Date`), summed per day; rows without a date or a value
  are skipped (at most 4,000 rows). The years of the data are drawn, at most the latest four,
  each from 1 January to 31 December; with `range.months` (since 1.0.2.0) only the days from
  the first day of the first month with data to the last day of the last month with data
  (within those years), each year's block starting at its first drawn day's week.
- A year block: the year (11 px) and the month names above the week columns (`Jan`…, French
  `janv.`…, 9 px, drawn when four cells are as wide as the name, over the column of the month's
  first drawn day), the weekday names Mon, Wed, Fri (`lun.`, `mer.`, `ven.`) left of their rows
  when cells are at least 7 px; weeks start on Monday, or on Sunday with `range.sundayFirst`
  (since 1.0.2.0: Sunday is the first row and Mon, Wed, Fri label rows 1, 3, 5).
- Cell size `min(24, (w − 46) / weeks, (block − 22) / 7)`, gap 2 px from 10 px, 1 px from 4 px;
  nothing is drawn below 2 px. A day with data is coloured from `low` to `high` by
  `(v − min) / (max − min)` (1 when all are equal); a day without data takes `empty`.
- Legend (`show`): the minimum, five swatches from `low` to `high` and the maximum at the bottom
  right, when they fit.

The block above is the Minimal preset (the drawing before the look). Modern and Bold (since
1.2.0.0) fill the visual instead:

- A gutter left of the grid as wide as the year (name size) or the weekday names (tick size − 1)
  plus 10 px; each year's header is tick size + 10 px high; years 12 px apart; the legend 30 px.
- Cells: width `min(available width / weeks, 48)`, height `min((available height / years −
  header) / 7, 48)`, then each kept to at most twice (width) or 1.6 times (height) the other;
  nothing is drawn below 3 px. Gap 3 px from 12 px, 2 px from 6 px, else 1 px; corners
  `min((size − gap) × 0.3, 6)`. The grid is centred both ways, on the card.
- Header: the year right-aligned left of the grid at 600; month names (tick size) over the column
  of their first drawn day when four cells are as wide as the name. With cells of 8 px or more a
  1.5 px axis-colour line
  parts each month from the one before, along the gaps: straight when the month starts on the
  first row, else a step around the first week's earlier days.
- Weekday names (tick size − 1) when cells are at least 9 px tall.
- Legend (`show`): the minimum, five 12 px steps 15 px apart (3 px corners) and the maximum, under
  the grid, right-aligned to it.
- `low` defaults to `high` mixed 86 % with white, or with the surface on a dark background.

In Power BI a day with data is a data point (tooltip: the date and the day's value written with the
field's format; selection by date).

### Sankey

Added 2026-11-24 at the user's request (ledger row 12 in
[community-visuals.md](../community-visuals.md)). The layout is the textbook layered drawing of a
flow graph: columns by longest path, barycentre ordering, ribbons between cubic Bézier edges; no
Sankey visual's code, text or drawing was looked at.

- Links: rows with a non-empty source and destination and a weight above 0 (at most 3,000 rows
  read); rows of the same source and destination add up into one link (at most 1,000 links);
  nodes are the distinct texts, in the order seen, a row's source before its destination (at most
  100: a row with a new node beyond them is skipped).
- Cycles: a depth-first walk from each node not yet reached, in the order seen, follows the
  links in the order seen; a link to a node still on the walk's path (a link to itself too) is
  left out. A note at the bottom left says how many (`2 links in cycles not drawn`, `2 liens en
  boucle non dessinés`; 10 px, 14 px taken from the plot). A node left with no drawn link is not
  drawn.
- Columns: a node without an incoming link is in column 0; any other is one column right of its
  furthest source (the longest path, so in A → B → C and A → C the node C is in column 2). A
  destination with no outgoing link stays in the column its sources put it in.
- Sizes: the plot is the visual less 8 px, less (since 1.3.7.0, with labels) a margin on the left
  for the first column's labels and one on the right for the last column's, each the widest of
  them (its estimated width, 0.55 of the size a character) plus 6 px, at most 30% of the visual's
  width less 16 px; columns are spread so the first starts at the plot's left
  edge and the last ends at its right edge (`(w − 16 − node width) / (columns − 1)` apart). A
  node's value is the larger of its incoming and outgoing sums; one scale for every column, the
  largest that fits each column's values plus `nodePadding` (default 10 px, 0–60) between its
  nodes; the padding shrinks when the fullest column's gaps would take more than half the
  height. `nodeWidth` default 12 px (2–60). Each column is stacked from the top and centred.
- Waypoints (since 1.2.0.0, every preset): a link from column a to column b > a + 1 gets a
  waypoint in each column between, as tall as the link, added after the nodes in link order. A
  link is then a chain of pieces, each from one column to the next. Waypoints count in their
  column's values and gaps (scale and padding above) and are sorted with the nodes, so the link
  passes between the nodes of those columns, never over them; they are not drawn.
- Order: first in the order seen (waypoints after the nodes); then four sweeps, each over the
  columns left to right (a node or waypoint goes to the weighted mean of its pieces' sources'
  centres, weights the values) and then right to left (its pieces' destinations' centres); one
  without pieces on that side keeps its centre; ties keep the column's order. Each column is
  restacked after it is sorted.
- Ribbons: a node's outgoing links are stacked from its top in the order of the tops of the
  node or waypoint at the other end of their first piece, its incoming links in the order of the
  other end of their last piece (ties: the link seen first), each as thick as its value, so each
  link has its own slot of the node's height on each side. A ribbon is a polygon: its top edge,
  piece by piece, the cubic Bézier from the right side of the piece's start (the source, or a
  waypoint at the node width past its column's x) to the left side of its end with both control
  points at the middle x, in 16 segments, then its bottom edge back the same way (34 points a
  piece). Colour (`links.colorBy`): the source node's (default), the
  destination's, or `fixed` (`links.fill`, default `#8C8C8C`); opacity `links.opacity` in
  percent (default 40, 5–100). Links are drawn first, then the nodes in BIcraft's palette by
  their order seen. Since 1.3.4.0 each of the first twelve nodes seen takes its colour option
  (`colors.node1`–`node12`, layout keys `nodeColor1`–`nodeColor12`) when one is set, and the
  links coloured by that node (`links.colorBy` source or destination) follow it. In Power BI the
  format pane's Node colors card (Couleurs des nœuds) names each swatch after the node drawn in
  that place (Node 1 … Node 12, Nœud 1 … Nœud 12, for a place without a drawn node); BIcraft's
  format pane lists them as Node 1 … Node 12.
- Labels (`labels.show`, `labels.fontSize` default 11 in Minimal, 12 in Modern, 13 in Bold,
  8–20, the look's name ink): each node's name beside it, 4 px off, at the node's centre kept
  inside the visual. Since 1.3.7.0 the labels sit on the outer side: the first column's on the
  left of their nodes and the last column's on the right, in the margins kept for them, where no
  ribbon runs; a middle column's on the first side, right then left, whose box (estimated width,
  the size high) meets no ribbon piece and no label placed, else on the first side clear of the
  labels over a halo (a rect of the look's surface, 85% opaque, 2 px wider and 1 px taller each
  way), else left out. Each is cut with an ellipsis to its margin less 6 px, or for a middle
  column to the gap before the next column less 8 px. 1.3.6.0 drew the first and middle columns'
  labels on the right of their nodes and the last column's on the left, over the ribbons. In each
  column the labels of the larger nodes are placed first and a label whose centre is closer than
  its size to one placed is left out (dense columns). The cycle note uses the tick size.
- Dates: a node is told apart by its value; its drawn name (label and tooltip) is a date cell
  written with the format string of the column it was first seen in (`formats`, as above).

In Power BI a ribbon and a node are data points. A link's tooltip gives the source, the
destination and its weight written with the weight field's format; a node's gives its name and
its incoming and outgoing sums (those above 0). A link selects all its rows (source and
destination), a node every row it is the source or the destination of (Power BI gives one
identity per row of the categories, as for the Venn diagram); both cross-filter the page.

### Scatter+

Added 2026-11-24 at the user's request: a scatter chart for many points, with the textbook
additions (hexagonal binning, marginal distributions, least-squares fit, quadrants); no scatter
visual's code, text or drawing was looked at. Display name BIcraft Scatter+, guid
`bicraftScatterPlus4E8A2C6F0B3D5A7C9E1F`, version 1.0.3.0 (1.0.1.0 and 1.0.0.0 until
2026-11-25; 1.0.2.0 changes only its package icon, G111; 1.0.3.0 only the runtime it bundles, G119).

- Roles: `details` (Grouping, required: one point per value), `legend` (Grouping: the point's
  colour, at most 12 groups, later ones take the 12th colour), `x`, `y` (Measure, required),
  `size` (Measure). Data view: categories `details`, `legend` (top 10,000), values `x`, `y`,
  `size`. A row whose x or y is not a number is not drawn.
- Every point up to 10,000 (`MAX_POINTS`), none sampled; past it the rows after the first
  10,000 are left out and a note at the bottom left says so (`First 10,000 of 12,000 points`,
  `10,000 premiers points sur 12,000`; the digits are grouped the English way in both).
- Format objects: `markers` (`size` 2–20, default 5; `fill`, `border` both on by default, both
  off draws them filled; `opacity` of the fill 10–100 %, default 70; the border is the colour
  mixed 30 % with black, 1.25 px); `density` (`mode` `auto`, `on`, `off`; `threshold` default
  2,000 points, 100–10,000; `cellSize` the hexagon radius, 4–40 px, default 10); `marginals`
  (`kind` `none`, `histogram`, `box`); `trend` (`show`); `quadrants` (`show`, `at` `average` or
  `value`, `x`, `y`, and the text labels `topRight`, `topLeft`, `bottomLeft`, `bottomRight`, at
  most 60 characters drawn); `labels` (`mode` `none`, `top`, `selected`; `count` 1–50, default
  5; `fontSize` 8–20, the look's tick size when unset); `legend` (`show`); and the look.
- Scales: both axes are nice linear domains over the points (and the quadrant lines' values),
  with their tick labels; the x and y field names are the axis titles. With a `size` field the
  radius is `size × 0.6 + size × 1.8 × √((s − min) / (max − min))`; the larger points are drawn
  first.
- Density: `on`, or `auto` with more points than the threshold, draws hexagons instead of
  points: each point goes to the nearest centre of the two nearest rows of a pointy-top grid
  laid from the plot's top left (rows `1.5 r` apart, columns `√3 r` apart, odd rows shifted by
  half a column); a cell's colour runs from the first colour mixed 82 % with the background to the
  same colour 15 % darker, by `√(n / largest n)`. A note says `N points, density view`. A cell is
  one data point: it selects its rows (at most 1,000) and its tooltip gives the count.
- Marginals: a 40 px strip above and right of the plot. Histograms: 20 bins over each axis's
  domain, bars in the first colour mixed 35 % with the background, scaled to the largest bin of
  both. Box strips: the quartiles (linear interpolation) as a box with the median, whiskers to
  the extreme values within 1.5 times the interquartile range.
- Trend: the least-squares line `y = a + b x` over every point drawn, clipped to the plot, with
  `R² = 0.87` near its right end. A single x value draws none. Since 1.0.1.0 the R² text takes
  the first of these places whose box (estimated width, the tick size high) comes no closer than
  1 px to a point label's: 8 px above the line's end, 8 px below it, a line further up and down
  (8 px plus the tick size plus 3), 8 px above and below the line's start, the plot's top left
  and bottom right corners (each kept inside the plot as before); when none is clear it takes
  the first and the point labels it meets are left out. 1.0.0.0 always took the first, over a
  label at the plot's right edge.
- Quadrants: dashed lines at the means of x and y, or at `x`, `y` (`at: value`); each corner
  gets its label and its share of the points (`Stars · 25%`, or `25%` without a label), when the
  plot is at least 40 px tall.
- Labels: `top` names the `count` points with the highest y (ties: the row seen first);
  `selected` names the points of the visual's selection, at most 50. A label is the details
  value, right of its point, or left when it would leave the plot. In Power BI a selection
  redraws the visual with the new labels; BIcraft's renderer draws its selection from the same
  input field `selected` (rows), empty in the snapshot today (docs/gaps.md).
- Highlights: a point without a highlighted value is dimmed; a cell is dimmed when none of its
  points is highlighted.

### Line / Area+

Added 2026-11-24 at the user's request: lines and areas with the additions report authors
build by hand (bands, thresholds, forecast stretches, line-end names); no line visual's code,
text or drawing was looked at. Display name BIcraft Line / Area+, guid
`bicraftLineAreaPlus9D2B6F4A8C0E3B5D7F1A`, version 1.0.4.0 (1.0.4.0 only the runtime it bundles, G119; 1.0.3.0, 1.0.2.0, 1.0.1.0 and 1.0.0.0 until
2026-11-25; 1.0.3.0 changes only its package icon, G111).

- Roles: `axis` (Grouping, required), `series` (Grouping: one line per value, at most 12),
  `value` (Measure, required), `value2` (Measure: the second measure of the band). Data view:
  categories `axis`, `series` (top 20,000), values `value`, `value2`. Rows of the same series
  and axis value add up; at most 2,000 points a series.
- The axis: dates when every axis cell is a date, numbers when every one is a number, else
  categories in the order seen; `axes.type: category` keeps one slot per value. The date axis is
  continuous, the twin of the renderer's time axis (`crates/render/src/timeaxis.rs`: the same
  steps of seconds to years, the first step whose labels keep 8 px apart, steps finer than the
  data's smallest gap skipped), labelled with the axis column's date format or the culture's
  short month names (EN, FR). `axes.zero` starts the value axis at zero (on by default with an
  area).
- `lines`: `color` (one series only; else the look's colours), `width` 0.5–8 (the look's line
  width when unset), `markers`, `emphasize` (a series name: that line 1.6 times wider, the others
  0.7 times and muted). Gaps (a point without a value) break the line; a lone point is a dot.
- `area`: `show`, `fill` (default the line's colour), `opacity` 0–100 % (default 25): the area
  under each run of the line down to zero (or the axis's edge), formatted apart from its line.
  `stacked` (since 1.0.2.0, layout key `stacked`, off by default; Stack the series, Empiler les
  séries), with two series or more: each point stands on the tops of the series before it at the
  same axis value, values of 0 or more on the rises and negative ones under the falls (a series
  without a value there adds nothing); lines, markers, hits, the threshold, rule and end labels
  follow the tops; an area runs down to the tops below it (the series' own bases, kept inside the
  axis) instead of to zero; value labels write each point's own value (`minmax` picks a series'
  own highest and lowest); highlighted parts stand on the same bases; the `value2` measure is not
  drawn (no band); `axes.zero` is on by default, as with an area.
- `band` (on by default, drawn when `value2` has a field): between `value` and `value2`, shaded
  `up` (default `#0E9F84`) where the first is above and `down` (default `#E23D4F`) where it is
  below, split exactly at the crossings, at `opacity` (default 30 %); `value2` is drawn as a
  thinner, lighter line.
- `threshold`: `show`, `value`, `side` (`above`, `below`), `color`: the area between the line and
  the value on that side is shaded at 25 %, split at the crossings, with a dashed line and its
  value.
- `rule`: `mode` (`none`, `above`, `below` a value), `value`, `color`: the stretches of a line
  beyond the value take the rule's colour, split exactly where the line crosses it.
- `forecast`: `from`, an ISO date or `today` (the host's date, midnight UTC; in BIcraft the
  snapshot's preview date): on a date axis, the lines are dashed (6 on, 4 off) from that date,
  with a dashed vertical mark and the word `Forecast` / `Prévision`. A date outside the data or
  not a date draws no forecast.
- `labels`: `mode` `last` (default), `minmax` (the highest above, the lowest below), `all` (at
  most 60 a series, else none), `none`; `fontSize`; `endLabels` (on by default): with several
  series, each name is written at the end of its line (spread at least the name size apart),
  instead of the legend (`legend.show` when `endLabels` is off).
- Hits: each point up to 3,000 in all, on its marker or an invisible 4 px circle; past that,
  each line is one target that selects every row of its series. A point's tooltip gives its axis,
  series and values.
- Highlights: the lines are muted and the highlighted values drawn over them in full colour;
  points without a highlighted value are dimmed. Since 1.0.1.0 a highlighted point with no
  highlighted neighbour is drawn as a dot (as a lone point of the line is; without markers),
  where 1.0.0.0 drew nothing for it ("Seen in Desktop").

Both new visuals share the kit's `dashes` (dashed polylines as `line` primitives, at most 4,000
dashes) and `compact` (K, M, B suffixes); `base.ts` draws the `path` primitive as an SVG
polyline since that release, so the other five were rebuilt as 1.3.1.0 (the calendar's month
outlines, `path` primitives, now show in Power BI too); the Sankey, Marimekko, org chart and Venn
are 1.3.2.0 since, for their label size ("Seen in Desktop").

## Checks

- `crates/render/src/chart_pack_tests.rs`: the tidy tree's boxes never overlap and parents centre
  over their children, collapse and levels, cycles and repeated ids, Venn region counts and the
  fixed circles, Marimekko areas as shares of the whole, the calendar's days and French months,
  hostile input (odd JSON, extreme numbers and sizes, 20,000 rows) never panics and stays capped.
- `crates/render/src/chart_pack_tests.rs` also: compaction (6.5 units instead of 8 leaves, no
  overlap either way), the proportional Venn's radii and overlap area, the calendar's months
  only and Sunday-first rows; the Sankey's columns by longest path and heights by flow, ribbons
  of 34 points a piece inside the visual, no overlap in a column, cycles and self links left out with the
  note (EN, FR), a crossing removed by the sweeps, the options, and the caps under 20,000 rows.
- `crates/engine/tests/report/chart_pack_tests.rs`: 46 golden layouts (since 1.0.3.0 three with
  date cells under their format strings; 9 of them Sankey's, one at a narrow 264 px). Since
  1.2.0.0 every Sankey case is checked in Rust and TypeScript for two ribbons sharing a slot of a
  node and for a ribbon crossing a node it does not end at or a column it skips without a
  waypoint, and the narrow case for whole labels and ribbons meeting their nodes' edges (both
  seen red on 1.1.0.0). Since 1.3.7.0 every Sankey case with labels is checked in Rust and
  TypeScript for a label box outside the visual, over another label, or over a ribbon without a
  halo, and the two-column crossing case for any halo at all (seen red on 1.3.6.0). The Rust layout is held to
  `chart_pack_golden.json` (`BICRAFT_BLESS=1` rewrites it); the TypeScript layouts, compiled by
  `bicraft_tsbuild`, run under Node when Node is on the PATH and must give the same primitives and
  hits (to 0.011 px); every text stays inside the visual. Each package's capabilities validate
  against `powerbi-visuals-api` 5.3.0's schema; each packaged visual runs under Node with a
  stand-in host and document (landing page, missing role message in French, points, tooltip,
  selection ids, dimming, context menu, high contrast, format pane cards, the organization
  chart's button collapsing a node and persisting `tree.toggled`, a Sankey node selecting its
  rows and a ribbon its own). In a report: `install` embeds
  each package once, `visual.add` places the visuals, the snapshot carries `chart_pack`,
  `report.render` and `report.export_pdf` report them `drawn`, `visual.format` options reach the
  layout, and packages, types and options survive PBIP and PBIX round trips.
- 2026-11-25: `sankey_node_colors`, `scatter_trend_labels`, `line_stacked_area` and
  `line_stacked_lines_fr`, with `sankey_node_colour_options_colour_nodes_and_their_links`,
  `the_trend_text_keeps_clear_of_the_point_labels` and
  `stacked_series_stand_on_the_ones_before_them` (each seen failing on the earlier version).
- Scatter+ and Line / Area+ (2026-11-24): 42 more golden cases (17 Scatter+: markers filled,
  bordered or both, density automatic and forced, French notes, histograms with the trend line,
  box strips with quadrants at averages and at values, top and selected labels, highlights over
  points and cells, Bold and dark looks, a single point; 25 Line / Area+: dates by month, day and
  hour, area, band on and off, threshold above and below, rule above and below, labels min-max and
  all, forecast from a date and from `today` in French, emphasis, legend instead of end labels,
  category, number and text axes, gaps, twelve series, highlights, Bold and Minimal looks).
  `chart_pack_tests.rs` in the renderer: every point drawn below the cap and the note past it,
  density cells counting every point, markers' fill and border, trend, quadrant shares and
  labels, band and threshold colours, the forecast's dashes, end labels, the hit and series caps,
  hostile cells (NaN, ±1e308, odd dates, odd options) leaving every coordinate finite. Under Node,
  a click on a Scatter+ point in the `selected` mode redraws it with that point's label.
  `own_visual_interactions_tests.rs` clicks, Ctrl+clicks, highlights, filters and bookmarks both
  as the other ten.

## Seen in Desktop

Power BI Desktop 2.159.1231.0 (2026-11-24) drew the four from a PBIX `bicraft` wrote, with their
tooltips, selection and cross-filtering, context menu, landing page, format pane cards (French)
and the org chart's collapse saved in `tree.toggled` and read back by BIcraft; the Venn circle
selected one row until 1.0.1.0, and the org chart and calendar were titled "Total" by the renderer
only ([community-visuals.md](../community-visuals.md) Runtime, "Seen in Desktop"). High contrast
was not checked. A second run the same day (1.1.0.0) saw their roles in French in the Build pane
(Id, Id du parent, Étiquette, Valeur; Élément, Ensemble; Catégorie, Série, Valeur; Date, Valeur).

Version 1.0.2.0 in the same Desktop (2026-11-24), from a PBIX `bicraft` wrote with `tree.compact`,
`sets.proportional` (two sets), `columns.sort`, `columns.widthAxis`, `range.months` and
`range.sundayFirst` on: the compact org chart, the area-proportional Venn diagram, the Marimekko
with its widest column first and its width axis, and the calendar from January to June with
Sunday as the first row drew the same shapes as `bicraft render` (month and day names in French);
the format pane showed the new options as toggles, on (Arbre compact; Surfaces selon les nombres
(deux ensembles); Les plus larges d'abord, Axe des largeurs; Seulement les mois avec des données,
Semaines commençant le dimanche). The Venn package exported by `bicraft visual export` also
imported through Desktop's Import a visual from a file and drew its sets
([community-visuals.md](../community-visuals.md) "Outside BIcraft").

The Sankey in Power BI Desktop 2.159.1231.0 (Store edition, French UI, 2026-11-24), from a PBIX
`bicraft` wrote with energy flows (Coal, Gas, Solar → Power → Industry, Homes; Gas → Heating →
Homes; Coal → Industry skipping the Power column) next to a built-in column chart, a page with a
cycle and a page of date nodes. Driven by input events through the WebView2 DevTools port; the
sandbox frame's DOM, the report view's tooltip and menu texts and screenshots were read.

- **Drawing.** Nodes, ribbons and labels as `bicraft render` draws them, in the default theme's
  colours; the Coal → Industry ribbon passes behind the Power node (no dummy nodes, as above). On
  the cycle page the columns and ribbon thicknesses match the render and the note reads
  "2 liens en boucle non dessinés". Date nodes on a General Date column read `05/01/2026 00:00:00`
  at midnight, as Desktop's own table and slicer write the same values.
- **Tooltips.** A node gives its name ("Nœud Coal") and its sums, a ribbon its source, destination
  and weight with the weight's format string.
- **Selection.** A click on Power highlights in the column chart the rows it is in (Coal 120,
  Gas 80, Solar 30, Power 230); a background click clears; a bar of the column chart filters the
  Sankey (no `supportsHighlight`). Ctrl+click on Gas with Power selected toggled the shared row
  Gas → Power off (Gas 45 in the column chart): fixed in the runtime (`multiIds`, Gas 125 then;
  [community-visuals.md](../community-visuals.md) "Interactions with built-in visuals").
- **Context menu and keyboard.** A right click opens Desktop's data point menu; with the focus on
  a ribbon, Enter selects it and Escape clears the selection.
- **Format pane.** The cards and options in French; the Couleur selon drop-down opens on Source,
  and choosing Destination redraws the ribbons in the destination's
  colour; Desktop's save writes `links.colorBy` and `bicraft render` draws the saved file the same
  way. The Build pane's roles read Source, Destination, Poids.

The Sankey at 1.2.0.0 and 1.3.0.0 in the same Desktop, later on 2026-11-24, from a PBIX `bicraft`
wrote with links skipping one and two columns, Modern, Bold and Minimal, and visuals 220, 180 and
160 px wide or high:

- **Routing.** The waypoint routing draws as `bicraft render` draws it: the skipping ribbons run
  beside the middle columns' nodes, never over them, each node's links in their own slots, and
  the last two columns' labels stay whole on the narrow visuals.
- **Label size.** Desktop's labels were 11 px under Modern where the renderer drew 12 px, so in
  the dense column the renderer left out a label ("Kappa") that Desktop kept. The four pack
  visuals read an unset `fontSize` as a fixed default (10 to 12 px) in the frame, while the
  renderer used the look's text size. Fixed in 1.3.2.0 (Sankey, Marimekko, org chart, Venn): an
  unset `fontSize` takes the look's text size in Power BI too, and the format pane shows that
  size (`the_packaged_visuals_run_under_node` compares the drawn sizes and the pane's value with
  the Rust layouts; seen failing first).
- **Interactions.** A ribbon's tooltip names its source, destination and weight; a click on a
  ribbon highlights the column chart's bar it is in (Power 133 of 153) and dims the others; a
  second click clears. The four pack visuals are highlighted by a selection elsewhere as
  [bicraft-visuals.md](bicraft-visuals.md) "Highlights" describes (the Venn filtered).

Not checked: high contrast, the landing page in Desktop, touch input; 1.3.2.0 itself was checked
by its tests, not yet in Desktop.

### Scatter+, Line / Area+, org chart and Marimekko 1.3.3.0 (2026-11-25)

Power BI Desktop 2.160.1151.0 (Store edition, French UI; the Store had updated it from
2.159.1231.0 on 2026-11-24), from a PBIX `bicraft` wrote: one page with Scatter+ (legend by
department), Line / Area+ (three series over dates), Waterfall+ and Gauge / KPI+ next to a
built-in column chart, bar chart, table and card; one page with each visual's options on
(marginal histograms and box strips, trend, quadrants, top labels, density; area, band,
threshold, forecast, rule, emphasis, markers; and the Waterfall+ and Gauge / KPI+ options of
[waterfall-gauge.md](waterfall-gauge.md)); and a page with the org chart left to right, the
Marimekko with `colors.series1`–`series3` set, a crowded slope chart and two dense dumbbells.
Driven through the WebView2 DevTools port as above.

- **Drawing.** Every visual of the three pages drew the same shapes, texts and colours as
  `bicraft render` (Scatter+'s density cells, marginals, trend with R², quadrant shares and
  labels; Line / Area+'s band, threshold shading, dashed forecast with Prévision, rule colours,
  emphasis and end labels; the org chart's levels across with its buttons on the boxes' right
  edges; the Marimekko's series in the three set colours).
- **Tooltips.** A Scatter+ point gives its details, legend, x and y with their formats
  (`Total 40,0`, `ScoreSum 61,0`); a Line / Area+ point its date (`mardi 3 février 2026`), series
  and value.
- **Selection.** A click on a Scatter+ point or a Line / Area+ point cross-highlights the column
  and bar charts and filters the table and card to its row; the other BIcraft visuals are
  highlighted. A selection elsewhere (a table row, a column) dims the Scatter+ points without a
  highlighted value and mutes the lines.
- **Fixed.** A Scatter+ point dimmed by a highlight drew faded in Desktop (the SVG element's
  opacity fades its edge) but as a hollow ring in `report.render`, whose renderer drew a marker's
  edge at full opacity: a dimmed mark's edge now fades with it (`spec::decorated`, test
  `a_dimmed_mark_fades_its_edge_too`, seen failing first). A Line / Area+ point highlighted alone
  (a table row selected) was drawn by neither Desktop nor BIcraft (no segment to draw): 1.0.1.0
  draws it as a dot in both drawers (`line_highlight_of_a_lone_point_draws_a_dot` and the golden
  case `hl_linearea_lone`, both seen failing first).
- **Format pane.** Every card and option in French (Marqueurs, Densité, Distributions
  marginales, Courbe de tendance, Quadrants; Lignes, Aire, Bande entre les valeurs, Seuil, Règle
  de couleur, Prévision, Axes; the Style card with Moderne, Sobre, Affirmé). Set in Desktop and
  saved: Scatter+ density `on` and marginals `histogram`, Line / Area+ labels `minmax` and the
  Minimal preset; BIcraft read the saved options back and drew the file as Desktop did.
- **1.3.3.0.** The org chart left to right ("De gauche à droite" on in the Arbre card) and the
  Marimekko's colours (Série 1 to Série 6 in the Couleurs card) drew as `bicraft render`; a click
  on Ivo's button collapsed it, and Desktop's save wrote `tree.toggled` `["e1"]` beside
  `tree.leftToRight`, which BIcraft read back and drew collapsed.
- **Dumbbell and slope (1.3.1.0).** The crowded slope chart's labels were spread with leader
  lines, as in `bicraft render`. Thirteen rows of close start values in a 200 or 260 px visual:
  both drawers pushed the stacked labels up past the plot, over the legend and, in Desktop, out
  of the frame (clipped). Fixed in the dumbbell 1.3.2.0: labels that cannot fit inside the plot
  are left out ([bicraft-visuals.md](bicraft-visuals.md) "Dumbbell and slope chart").
- **Alike in both.** With top labels and the trend, the `R² = 0.xx` text could sit on a point's
  label (Eve at the plot's right edge); fixed in Scatter+ 1.0.1.0 ("Trend" above). High contrast
  was not checked.

The G110, G113, G115, G116 and G117 changes of 2026-11-25 (dumbbell 1.3.2.0, Sankey 1.3.4.0,
Scatter+ 1.0.1.0, Line / Area+ 1.0.2.0, Waterfall+ 1.0.1.0) were checked by their tests, not yet
in Desktop.
