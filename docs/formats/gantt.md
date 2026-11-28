# Gantt visual (core visual, preview)

Sources: Microsoft Learn, "Gantt visual in Power BI (preview)" and "Create a Gantt chart in Power
BI (preview)" (both dated 2026-10-30, read 2026-11-20); Microsoft's published report theme JSON
schema 2.159 (`reportThemeSchema-2.159.json`, powerbi-desktop-samples commit `34f78db3`,
2026-10-15, MIT, vendored in `crates/visuals/schemas/`); a black-box wells test against Power BI
Desktop 2.159.1231.0 (Store edition) on 2026-11-23 (method wells-A of
[visual-roles-bb.md](visual-roles-bb.md), log in [visual-roles-bb.sources.md](visual-roles-bb.sources.md)).

## What the public sources give

**Visual type.** The theme schema 2.159 adds one entry to `visualStyles`: `ganttChart`, with the
definition `visual-ganttChart`. Theme `visualStyles` keys are PBIR `visual.visualType` names, so
BIcraft accepts `ganttChart` as a built-in type (`visual.add`, `report.apply`, `theme.*`).

**Formatting objects** (theme schema 2.159), besides the container objects every visual has:

| Object | Properties (schema names) |
|---|---|
| `axis` | `autoScale`, `start`, `end`, `formatDay`, `formatMonth`, `formatYear`, `primaryUnit`, `secondaryUnit`, `position`, `invertAxis`, gridlines (`gridlineShow`, colour, style, thickness, transparency, auto scale), shading (`shadingShow`, `shadingDays`, colour, transparency), zoom (`zoomShow`, `zoomMin`, `zoomMax`, `zoomShowTooltips`) |
| `axisTick` | font, colour, alignment, padding, `show` |
| `bar` | `show`, `color`, `customizeColor`, `transparency`, `height`, `cornerRadius`, border |
| `milestone` | `show`, `shape`, `size`, `color`, `customizeColor`, `transparency`, border |
| `progress` | `show`, `color`, `customizeColor`, `transparency`, `relativeHeight`, `cornerRadius`, border |
| `row` | `color`, `colorByCategory`, background (`backgroundShow`, colour, `backgroundCustomizeColor`, transparency), dividers, `expandCollapseIconColor`, `expandCollapseToggleType`, `isCollapsed`, `steppedLayoutIndentation` |
| `columnHeaders`, `columnValue`, `columnWidth`, `columnPanel` | the detail columns' header and value text, widths (`customColumnWidth`, `defaultColumnWidth`, `value`), the column area's `width`, `textWrap`, `urlIcon` |

**Behaviour** (Learn): one row per task; only the row label is required; **Start** and **End**
give a duration bar, a blank **End** draws a milestone; a measure of the percentage complete
draws progress inside the bar; up to three category levels group tasks (expand and collapse);
detail columns beside the task list and extra tooltip fields; one colour cascading to category
backgrounds at 70 %, 80 % and 90 % transparency and task rows at 100 %; an axis that auto-scales
between years, months and days, or two fixed levels; shaded week days; a zoom axis; at most
1,000 rows.

## Data roles (black-box, Desktop 2.159.1231.0)

A synthetic model (text columns `T1`…`T10`, date columns `D1` start and `D2` end, a number
column, measures `M1`…`M10`, date measures `MD1` = `MIN(F[D1])` and `MD2` = `MAX(F[D2])`, a
percentage measure `MP`) and an empty `ganttChart` visual written by BIcraft. Fields were dragged
from Desktop's Data pane into each well; Desktop saved the PBIX and its `queryState` keys were
read. In this Desktop the Gantt was already in the Visualizations gallery ("Diagramme de Gantt
(préversion)" in the French UI); no preview option was changed; a `ganttChart` visual written by BIcraft opens with its seven wells,
and a `Task` field BIcraft bound shows in its well.

| Well (Desktop caption, French UI) | Role (`queryState` key) | Takes | Max | Test |
|---|---|---|---|---|
| Ligne de tâche (task row) | `Task` | a column as it is; a measure is refused | 1 (a second field replaces it) | bb:gantt-A/B/C |
| Démarrer (start) | `StartDate` | a date column, summarized as Earliest (`Min`); a date measure; a text column and a numeric measure are refused | 1 | bb:gantt-A/B/C |
| Fin (end) | `EndDate` | a date column, summarized as Latest (`Max`); a date measure; text and numeric measures refused | 1 | bb:gantt-A/B/C |
| Progression (progress) | `Progress` | a measure; a text column becomes Count (`CountNonNull`) | 1 | bb:gantt-A/B/C |
| Catégorie (category) | `Category` | columns as they are; a measure is refused | 3 (a fourth is refused); no `active` flag, so not a drill role | bb:gantt-A/B/C |
| Colonnes (detail columns) | `Columns` | measures; a text column becomes First (`Min`) | none seen (5 kept) | bb:gantt-A/C |
| Info-bulles (tooltips) | `Tooltips` | measures; a text column becomes First (`Min`) | none seen (5 kept) | bb:gantt-A/C |

**The task row first** (bb:gantt-R): with `Task` empty, every other well refuses its field (date
columns, date measures and text columns alike); once `Task` has a field they take them.
BIcraft records `Task` as the role the visual needs (`required_roles`), a warning as for other
types.

Desktop saved the visual with `sortDefinition` on the start field (ascending, `isDefaultSort`),
`drillFilterOtherVisuals: true` and one visual-level filter per field, as for other visuals.
With Task, Start, End, Progress, a category and a detail column the canvas drew the task list,
the detail column, collapsible category rows and bars against a month axis.

BIcraft encodes these roles in `crates/visuals/src/roles_bb.rs`: `Task` grouping (max 1),
`StartDate`, `EndDate`, `Progress` measure (max 1; BIcraft's kinds have no "date only", so
the date restriction is not checked), `Category` grouping (max 3), `Columns` and `Tooltips`
measure. The Build pane shows the seven wells, the gallery offers the type, and `visual.bind`
checks role names and kinds against them.

## Evaluation

The engine evaluates a Gantt like a table: one row per task (the `Task` field and the
categories grouped), `StartDate` and `EndDate` as bound (a date column's Earliest / Latest, or
a date measure), `Progress`, `Columns` and `Tooltips` per row, under the visual's filters.
Without a saved `sortDefinition` the rows are sorted by start, earliest first (the sort Desktop
saved in bb:gantt-A); ties by the grouped fields. At most 1,000 rows are kept (Learn's limit;
`data.truncated` says when more exist). Tests: `crates/engine/tests/report/gantt_tests.rs`.

The snapshot's `format.gantt` carries the formatting objects (theme `visualStyles` for `*` and
`ganttChart`, then the visual's whole-visual entries): `bar`, `milestone`, `progress` (fill,
`customizeColor`, transparency, height, corner radius, relative height, shape, size, border),
`row` (colour, `colorByCategory`, background, dividers, icon colour and type, `isCollapsed`,
indentation), `axis` (auto-scale, start and end, day / month / year formats, units, position,
inversion, gridlines, shading and `shadingDays` as week-day numbers, 0 = Sunday), `axisTick`,
`columnHeaders`, `columnValue`, `columnPanel`. `format.gantt_rows` lists `row` entries with a
`data` selector (`[{path: [{c, v}], collapsed}]`: the category members named and their
`isCollapsed`) and `format.gantt_widths` the `columnWidth` entries with a `metadata` selector.
How Desktop saves a per-member collapse and `shadingDays` in a PBIR file has not been observed:
BIcraft reads a data selector of `Comparison`s, and `shadingDays` as a theme list or a literal
text naming the days.

## What BIcraft draws (`crates/render/src/gantt.rs`)

Following Learn's description and the schema's objects; the sizes, default colours and the
label rules below are **BIcraft's own choices**, not measured on Desktop and not compared with
its drawing pixel for pixel.

- **Task list** on the left: the task column (150 px by default) and one column per `Columns`
  field (`columnHeaders.defaultColumnWidth`, else 80 px; `columnWidth` per column;
  `columnPanel.width` sets the whole list), at most 60 % of the visual; column headers in the
  header band. Category rows come first in each group, in the order of their earliest task, with
  a chevron, plus/minus or caret icon (`expandCollapseToggleType` 0, 1, 2), indented 12 px per
  level (`steppedLayoutIndentation`); a collapsed category hides its members.
- **Row backgrounds**: category rows take the row colour at 70 %, 80 % and 90 % transparency by
  level (Learn), task rows none; a `backgroundColor` (unless `backgroundCustomizeColor` is
  `false`) replaces it, at `backgroundTransparency` (default 70 %).
  The row colour is `row.color`, else the theme's first data colour; with `colorByCategory`,
  each top-level category takes the next theme colour. Dividers under each row (`dividerShow`,
  on by default).
- **Time line**: from the earliest to the latest date of the tasks (collapsed ones included),
  widened to whole units, or `axis.start` / `axis.end` (an ISO date or a serial). Auto-scale
  (the default) picks one unit: days when a day gets at least 18 px, months when a month gets
  34 px, else years; month labels carry the year on the first month and each January ("Jan
  2025", "Feb"…), day labels the month on the first day and each 1st. With `autoScale` off,
  `primaryUnit` (default month) above `secondaryUnit` (`none`: one level). `formatDay`,
  `formatMonth`, `formatYear` are read as DAX `FORMAT` date patterns. Labels thin out when they
  do not fit; `position` top, bottom or both; `invertAxis` runs time right to left.
  Gridlines at the finest unit (on by default), shaded week days (`shadingShow`, Saturday and
  Sunday unless `shadingDays` says) where a day is at least 2 px wide.
- **Bars** from start to end (at least 2 px), 14 px high by default, corner radius 2; a task
  without an end (or with an end not after its start) is a **milestone** at its start (diamond
  by default, or the `shape` given, `size` default the bar height). **Progress** fills the bar
  from its start by the progress value, taken as a share and clamped to 0–1, in the bar colour
  mixed with 35 % black. Bars, milestones and progress take their object's `color` when it is
  set and `customizeColor` is not `false` (the Format pane writes the colour alone), else the
  row colour. A category row draws a thin summary bar over its members' dates.
- **Interaction**: a click on a task row selects the task with its categories (a category row:
  its member), so other visuals cross-filter or highlight as for any visual, and the other rows
  dim; a highlight from another visual dims tasks without highlighted values. Hovering shows
  the categories, task, start, end, progress, detail and tooltip fields.
- **Expand and collapse**: a click on a category's icon sends `view.interact`
  `gantt_toggle {page, visual, path, collapsed}`, the reader's state of that category in this
  view (the report is not changed; `reset` drops it). The snapshot lists it after the saved
  `row` entries in `format.gantt_rows`, so it wins.
- **Scrolling**: when the lines do not fit, a scroll bar (8 px) runs on the right of the time
  line and the reader's first line (`RenderOptions::gantt`, the app's view state, never in the
  file or the snapshot) says where the list starts; it is clamped so that the last line ends the
  list. A static render (`report.render`, PNG, PDF) shows the first lines; its note counts the
  lines it does not draw.
- **Zoom**: the reader's zoom (1 to 512 times, the window at least a day wide) shows a window of
  the time line at a position from 0 (its start) to 1 (its end); auto-scale picks the unit again
  for the window, and a bar under the rows shows the window within the whole time line. This is
  BIcraft's own control: the schema's `zoomShow`, `zoomMin` and `zoomMax` (Desktop's zoom
  slider) are read by neither the drawing nor the app, since what they hold was not observed.
- **The app's controls** (`crates/ui/src/gantt_ui.rs`, desktop and browser): each category's
  icon is a labelled button (Expand or Collapse and the member); the rows' scroll bar takes the
  arrows, Page Up / Page Down, Home and End, a click on its track and a drag; the time line band
  is a zoom slider (+ and -, the left and right arrows, Home or a double click for all of it);
  the window's bar is dragged; the mouse wheel scrolls the rows (the page once the rows are at
  their end), a horizontal scroll moves the window, Ctrl+wheel zooms. page.html does not draw the
  Gantt (its wireframe box only), so it has none of these.
- A Gantt without a `Task` field is a labelled placeholder.

## Not yet done

- No today line; Desktop's zoom slider (`zoomShow`, `zoomMin`, `zoomMax`) is not read: BIcraft's
  zoom is its own view state. The reader's collapse is not saved to the file (how Desktop saves a
  per-member collapse in PBIR was not observed).
- Not compared with Desktop's drawing (sizes, colours, label formats, the meaning of
  `relativeHeight` and of a progress above 1 are BIcraft's reading).
- The 2.159 schema lists the milestone shapes, date units, gridline styles and toggle types as
  enumerations; the Format pane does not offer them as choices yet.
