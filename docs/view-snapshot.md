# Live viewer: snapshot contract (M8)

The live viewer is a local web page that shows, as it happens, what BIcraft (and the agents
driving it over MCP) is building. Editing stays in the CLI and MCP. With `bicraft view` the page
is also a **report reader** (M8.3, "Interactions" below): slicer clicks, cross-filtering on data
points, page navigation and the slicer buttons change a reader state held in the session, never
the files.

```
engine (L5)  Session::snapshot() -> JSON (this contract), Session::revision
mcp    (L6)  calls an observer after every tools/call that may have changed the session
view   (L5)  ViewServer: localhost HTTP, serves the embedded page, /api/state, /api/events (SSE), /api/asset/<id>,
             POST /api/open-in-desktop (the "Open in Power BI Desktop" button),
             POST /api/interact (reader actions, `bicraft view` only)
             GET /app/ (the egui app in the browser, `--ui egui`: docs/webui.md)
cli    (L7)  `bicraft mcp --view [--view-port N]` wires mcp → view;
             `bicraft view PATH` opens a file/project and re-reads it when it changes on disk
```

## Snapshot JSON (version 1)

Producers may add fields; consumers ignore unknown fields. All names are as in the model/report.

```json
{
  "version": 1,
  "revision": 42,
  "source": { "path": "C:/x/Sales.pbip", "kind": "project|package|bim|none", "format": "pbip-tmdl|pbip-tmsl|pbix|pbit|bim|null",
              "desktop": { "path": "C:/x/Sales.pbip" | null, "reason": null | "…", "unsaved": false },
              "live_connection": null | { "type": "pbiServiceXmlaStyleLive" | null, "data_source": "powerbi://…" | null, "catalog": "Sales" | null } },
  "dirty": { "model": true, "report": false, "queries": false },
  "dax_queries": { "generation": 3, "tabs": [ { "name": "Query 1", "text": "EVALUATE …" } ], "default_tab": "Query 1" | null,
                   "saved_in": "project|session" },
  "last_command": { "id": "visual.add", "ok": true, "error": null, "at_ms": 1760363000000 },
  "model": null | {
    "name": "Sales", "culture": "fr-FR",
    "tables": [ {
      "name": "Ventes", "hidden": false, "calculated": false, "description": null,
      "columns":  [ { "name": "Montant", "data_type": "double", "format_string": "#,0.00" | null, "calculated": false, "hidden": false,
                      "description": null, "sort_by_column": null } ],
      "measures": [ { "name": "Total", "expression": "SUM(Ventes[Montant])", "format_string": "#,0", "hidden": false,
                      "display_folder": null, "valid": true, "description": null } ]
    } ],
    "relationships": [ { "from_table": "Ventes", "from_column": "ProduitID", "to_table": "Produit", "to_column": "ProduitID",
                         "active": true, "cross_filter": "single|both|automatic", "from_cardinality": "many", "to_cardinality": "one",
                         "security_filtering": "single|both|none" } ],
    "diagram": { "name": "All tables", "diagrams": ["All tables"],
                 "nodes": [ { "table": "Ventes", "x": 40, "y": 24 } ],
                 "layouts": [ { "name": "All tables", "all": true, "nodes": [ { "table": "Ventes", "x": 40, "y": 24 } ] } ] }
  },
  "report": null | {
    "active_page": "<page name>",
    "pages": [ {
      "name": "<20 hex>", "display_name": "Overview", "width": 1280, "height": 720, "hidden": false,
      "page_type": "report|tooltip|drillthrough",
      "visuals": [ {
        "name": "<20 hex>", "type": "card|clusteredColumnChart|tableEx|slicer|…|null (group)",
        "x": 10, "y": 10, "z": 0, "width": 300, "height": 120, "tab_order": 0,
        "title": "Total Ventes | null", "parent_group": null, "hidden": false,
        "fields": [ { "role": "Values", "kind": "measure|column|aggregation|other", "table": "Ventes", "name": "Total",
                      "function": null, "text": "[Total]", "ref": "Total" } ],
        "sort": [ { "field": "[Total]", "descending": true } ],
        "issues": [ { "severity": "error|warning", "message": "…" } ]
      } ]
    } ]
  },
  "validation": {
    "model": { "valid": true, "invalid_count": 0, "objects": [ { "table": "", "kind": "", "name": "", "diagnostics": [] } ] },
    "report": { "valid": true, "error_count": 0, "warning_count": 0, "issues": [] }
  }
}
```

`fields[].ref` is the name visual calculations refer to the field by (its `nativeQueryRef`), absent when the projection has none; `fields[].hidden` is true for a field hidden in the visual ([formats/visual-calculations.md](formats/visual-calculations.md)).

`fields[].format` is the field's format for this visual only (Data format), absent without one; a sparkline is `kind: "sparkline"` with `sparkline: {index, y, x}`, and its data column is `kind: "sparkline"` with the look in `sparkline` and cells `{"sparkline": [[x, y], …]}` ([formats/sparklines.md](formats/sparklines.md)).

`model.diagram` is the selected diagram of the file's model layout as the session holds it
(`diagram.move` edits it; [formats/diagram-layout.md](formats/diagram-layout.md)): `nodes` are
the cards the layout places, in the layout's coordinates. It is absent when the file has no
usable layout; a table without a node has no saved place. `layouts` lists every layout tab
(at most 100, 10 000 nodes in all) with its nodes; `all` marks the layout of all tables (the
others show only their nodes' tables; `diagram.add`, `diagram.rename`, `diagram.remove`).

### M8.2 additions (still version 1: every field below is optional; viewers must cope without it)

```json
{
  "report": {
    "theme": null | {
      "name": "CY24SU10", "data_colors": ["#118DFF", "#12239E", "…"],
      "background": "#FFFFFF", "foreground": "#252423", "table_accent": "#118DFF",
      "font_family": "Segoe UI", "font_size": 10,
      "good": "#1AAB40", "neutral": "…", "bad": "…", "maximum": "…", "center": "…", "minimum": "…",
      "text_classes": { "title": { "font_face": "Segoe UI Semibold", "font_size": 12, "color": "#252423" } },
      "styles": { "*": { "border": { "show": true, "radius": 6 } },
                  "page": { "outspace": { "color": "#F3F2F1" }, "filterCard.Applied": { "backgroundColor": "#E6E6E6" } } }
    },
    "pages": [ { "background": null | "#F3F2F1",
      "visuals": [ {
        "format": null | { "title": { "show": true, "font_size": 14, "color": "#252423", "alignment": "left" },
                           "background": { "show": true, "color": "#FFFFFF", "transparency": 0 },
                           "border": { "show": false, "color": "#000000", "radius": 0 },
                           "data_labels": true, "legend": { "show": true, "position": "top" } },
        "data": null | {
          "status": "ok | unavailable | unsupported | error",
          "reason": "why there is no data (status ≠ ok)",
          "columns": [ { "name": "Produit", "role": "Category", "kind": "column|measure|aggregation", "data_type": "string" } ],
          "rows": [ ["Vélo", 1234.5] ],
          "truncated": false
        }
      } ]
    } ]
  }
}
```

- `theme` is the report's resolved theme (custom theme over the base theme); `null` means "use the
  default Power BI theme". The sentiment and divergent colours appear only when a theme sets
  them; `text_classes` (`label`, `title`, `callout`, `header`, merged base then custom) and
  `styles` (the `visualStyles` objects the Customize theme dialog edits: `*` `background`,
  `border`, `visualHeader`, `visualTooltip`; `page` `outspace`, `background`, `outspacePane`,
  `filterCard.Available`, `filterCard.Applied`; plain values, colours as `#RRGGBB`) only when
  not empty.
- `sort` (a visual whose `query.sortDefinition` BIcraft reads; absent otherwise): its sort fields
  in order, `field` in `visual.set_sort`'s syntax (`"Table[Column]"`, `"[Measure]"`,
  `"Sum(Table[Column])"`), what the visual header's Sort axis menu checks
  ([desktop.md](desktop.md), "Focus mode, Show as a table, Sort and Spotlight").
- `data` is a small evaluated result for the visual's bound fields: group by the bound columns,
  aggregate the bound aggregations and the measures BIcraft can evaluate. Values are JSON
  numbers, strings, booleans or null; dates are ISO 8601 strings. Rows are capped (truncated
  sets `truncated`). `unavailable` = the session has no table data (e.g. a PBIP without cache, a
  PBIT); `unsupported` = a measure or visual BIcraft cannot evaluate yet. The viewer falls back
  to a labelled placeholder rendering whenever `status` ≠ `ok`.
- `data_state` (every visual): `{"state": "ok | no_rows | blank_values | partial_blank |
  not_bound | not_evaluated", "rows": 3, "value_cells": 3, "blank_cells": 0}`, what `data`
  holds, independent of the UI language; the counts only when there is a dataset. Same rules
  and values as `report.render`'s per-visual `data` ([render.md](render.md), "Data states").

#### How the producer fills them (`crates/engine/src/{theme,preview}.rs`)

**Theme.** Built-in defaults for the base theme name (the public default Power BI palette;
`CY18*`/`CY19*` get the classic one), overlaid with the base theme JSON when the report embeds it
(`StaticResources/SharedResources/BaseThemes/<name>.json`, as Desktop does), overlaid with the custom
theme (`StaticResources/RegisteredResources/<name>`). `font_family`/`font_size` come from
`textClasses.label`. `theme` is `null` only when the report has no `themeCollection`.

**Page background.** The page's `objects.background` color (`null` when fully transparent), else the
theme's `visualStyles.page.*.background`.

**Visual format.** Theme `visualStyles["*"]["*"]`, then `visualStyles[<visual type>]["*"]`, then the
visual's `objects` and `visualContainerObjects` (later wins). Only properties that are set appear.
Colors resolve literals and `ThemeDataColor` (`ColorId` 0 = background, 1 = foreground, n ≥ 2 =
`data_colors[n-2]`, `Percent` tints/shades). `null` when nothing is set.

**Data.** Computed for visuals with at least one field (others: `data: null`).
- Fields: a column groups; an aggregation (`Sum`, `Avg`, `Min`, `Max`, `Count` = distinct,
  `CountNonNull`, `Median`, `StandardDeviation`, `Variance`) and a measure produce values.
- Which evaluator (since 2026-11-25, G70): a visual with a value field goes to the general
  evaluator below first, whose aggregations the storage engine answers (`docs/storage-engine.md`:
  one grouped scan over the columns' codes serves every cell, as for `dax.evaluate`); the fast
  path below runs only when the general evaluator cannot (more than 20 000 groups, the work it
  allows), on tables of at most `SnapshotLimits::data_table_rows` (2 M) rows. A visual of columns
  only (a slicer, a list of categories) keeps the fast path first: it lists the combinations the
  rows hold (no blank row of a dimension, no cross of tables). Real-number sums may differ from
  the fast path's in their last digits (the storage engine adds rows in parallel, with
  compensated sums); every other value is the same (`preview_grid_tests.rs` compares both orders
  on a varied report, on literal and on stored data).
- Fast path (one grouped pass over the aggregated table, row by row): measures built from `SUM`, `AVERAGE`,
  `MIN`, `MAX`, `MEDIAN`, `COUNT`, `COUNTA`, `COUNTBLANK`, `DISTINCTCOUNT`, `DISTINCTCOUNTNOBLANK`,
  `STDEV.S`, `VAR.S` of a column, `COUNTROWS(Table)`, `DIVIDE`, two-argument `MIN`/`MAX`, `BLANK()`,
  single-argument `CALCULATE`, `VAR … RETURN`, references to other such measures, `+ - * /` and
  number constants (DAX blank arithmetic). Grouping by a column of another table follows active
  many-to-one (or one-to-one) relationships from the aggregated table, up to 4 hops; unmatched
  rows form the blank group.
- The general evaluator (`preview_eval.rs`) evaluates the visual like
  `SUMMARIZECOLUMNS`: candidate rows are the existing combinations of the grouping columns of each
  table (plus a table's blank row when related rows have unmatched keys), crossed between tables;
  each value is evaluated in a real filter context (filters propagate through active
  relationships, into the one side only when they filter both ways). It adds `CALCULATE` /
  `CALCULATETABLE` with boolean filters (`=`, `<>`, `<`…, `IN`, `&&`, `||`, `NOT` over columns of
  one table), table filters (`FILTER`, `VALUES`, `DISTINCT`, `ALL…`, `TREATAS`, table names,
  variables), `ALL`/`REMOVEFILTERS`/`ALLSELECTED`/`ALLNOBLANKROW` (as modifiers: of the expanded
  table), `ALLEXCEPT`, `KEEPFILTERS`, `USERELATIONSHIP`, `CROSSFILTER`, context transition;
  time intelligence (`DATESYTD/QTD/MTD`, `TOTALYTD/QTD/MTD`, `SAMEPERIODLASTYEAR`, `DATEADD`,
  `PARALLELPERIOD`, `PREVIOUS*`/`NEXT*`, `DATESINPERIOD`, `DATESBETWEEN`, `STARTOF*`/`ENDOF*`,
  `FIRSTDATE`/`LASTDATE`, with Desktop's month-end rules; a filter on a date table's date column
  also removes the other filters on that table); `IF`, `SWITCH`, `SELECTEDVALUE`, `HASONEVALUE`,
  `ISFILTERED`, `ISINSCOPE`, `ISBLANK`, `ISEMPTY`, `COALESCE`, `AND`/`OR`/`NOT`, `SUMX`,
  `AVERAGEX`, `MINX`, `MAXX`, `COUNTX`, `RANKX`, `RELATED`, `DATE`/`YEAR`/`MONTH`/`DAY`/`EOMONTH`/
  `EDATE`, `ABS`/`INT`/`ROUND`/`SIGN`, `&` and comparisons. `ALLSELECTED` restores the visual's outer
  filters (report, page, visual filters and slicers) on its columns.
  Table functions (`preview_eval_tables.rs`): `ADDCOLUMNS`, `SELECTCOLUMNS`, `SUMMARIZE` (with
  `ROLLUP` and `ISSUBTOTAL`), `SUMMARIZECOLUMNS` (group-by columns, filter tables, named
  expressions), `GENERATESERIES`, `GENERATE`/`GENERATEALL`, `CROSSJOIN`, `TOPN` (ties at the N-th
  row kept, blank sorts first), `UNION`, `INTERSECT`, `EXCEPT`, `DISTINCT`/`VALUES` of a table,
  `CALENDAR`, `CALENDARAUTO` (the date columns of the data tables, whole (fiscal) years), `ROW`,
  `DATATABLE`. Columns keep the lineage of the model column they come from (a result used as a
  filter filters those columns; computed columns filter nothing); the expressions of
  `ADDCOLUMNS`/`SELECTCOLUMNS`/`GENERATE`/`TOPN` run in a row context (a measure or `CALCULATE`
  in them transitions it); `SUMMARIZE` evaluates its expressions with the group's rows as a
  filter of the summarized (expanded) table; `VALUES(table)` and `ALL(table)` include the virtual
  blank row, `DISTINCT(table)` does not. Added columns are read with `[Name]` in a row context.
  The same evaluator computes calculated tables and columns for structural PBIX saves
  (`preview_eval_struct.rs`); there, as the engine names them, an unnamed `SELECTCOLUMNS`
  column `T[C]` is `T_C`, and a column named after a model column gets `sourceColumn` `T[C]`.
  `tests/table_functions/` holds a synthetic model and the results
  the Analysis Services engine of Desktop 2.158 gave for 119 expressions (rows, column names and
  types; 64 of them `NAMEOF`, which the evaluator spells as the engine does); `table_functions_match_the_engine` compares BIcraft with them. Work is capped per visual (20 000 candidate rows, 4·10⁸ row visits,
  5 s, nesting depth); beyond that, and for any other function, hierarchy fields and AI/script
  visuals → `unsupported` + `reason` naming what is missing. Numbers are doubles, except Currency
  (`decimal` columns): both paths follow the engine's Currency rules (exact sums; `AVERAGE`,
  `c / n`, `c * x`, `DIVIDE(c, n)`… rounded half away from zero to 4 decimals; see
  docs/formats/dax-functions.md, "Currency arithmetic"). Column variations (`'T'[Date].[Year]`,
  auto date/time) read the local date table's column; in a row context of `T` they follow the
  relationship. Rows whose values are all blank are dropped (as Power BI does).
- Filters and slicers are applied (see "Filters, slicers and visual details" below), and so is
  the viewer's interaction state (slicer clicks, cross-filtering and highlighting: "Interactions"
  below).
- Order: `query.sortDefinition` when its field is bound; else by the grouping columns
  ascending (Desktop 2.158 queries a bar, column, pie, funnel, waterfall chart or treemap without
  a sort definition in category order). A grouping column with a model `sortByColumn` sorts by that column's
  value (as Desktop does: months by month number). `rows` holds at most `SnapshotLimits::data_rows_per_visual` (1000) rows;
  `total_rows` is the full count.
- Table data, per table, first match wins: the PBIX `DataModel` (or the PBIP `.pbi/cache.abf`),
  decompressed once and decoded column by column on first use; then literal partitions (M
  `#table`, DAX `DATATABLE`/`{…}`/`ROW`/`CALENDAR`); else `unavailable`. Calculated columns of
  literal tables, and those added since the data was stored, are computed: the expression once
  per row (at most 2 M rows, 60 s), kept until the model or the data changes. The general
  evaluator reads tables of any size up to 2·10⁹ rows (its memory bounded by the session's
  memory limit, `docs/beyond-ram.md`); the fast path reads at most `data_table_rows` (2 M) rows a
  table: a visual only it can evaluate is `unsupported` over that, its reason naming both the
  general evaluator's limit and the row cap; a visual of columns of several tables over it is an
  `error` that says so.
- Caching (per session): the decompressed backup and decoded columns live until the source data
  changes (checked when `revision` changes: hash of the `DataModel` part; path/size/mtime of
  `cache.abf`). Results are keyed by (data generation, hash of the model, visual type + fields +
  query): moving, resizing or formatting a visual reuses its result; a model edit recomputes.
- Budget: a snapshot spends at most `data_budget_ms` (1000 ms) computing results that are not
  cached, active page first, then pages whose visuals changed since the previous snapshot, then
  the rest. Visuals not reached keep `data: null` and are computed by the next snapshot; cached
  results are always served.

### Filters, slicers and visual details (still version 1; every field optional)

```json
{
  "report": { "pages": [ { "visuals": [ {
    "data": { "filters_applied": 3, "filters_ignored": true, "filters_ignored_reason": "…",
              "cell_styles": [ { "r": 0, "c": 1, "bg": "#F8696B", "fg": "#000000", "fill": "#118DFF",
                                 "bar": { "from": 0, "to": 0.62, "color": "#5F6B6D" } } ] },
    "text": { "paragraphs": [ { "align": "left|center|right", "runs": [ { "text": "…", "bold": true,
              "italic": true, "underline": true, "font_size": 14, "color": "#252423",
              "font_family": "Segoe UI", "url": "https://…" } ] } ], "truncated": true },
    "button": { "text": "Back", "show_text": true, "font_size": 12, "font_color": "#252423",
                "align": "center", "icon": "back", "show_icon": true, "icon_placement": "left",
                "icon_color": "#252423", "fill": "#FFFFFF", "show_fill": true, "fill_transparency": 0,
                "show_outline": false, "outline": "#605E5C", "outline_weight": 1, "outline_transparency": 0,
                "states": { "hover": {…}, "press": {…}, "disabled": {…} } },  // only what each state sets
    "shape": { …, "text": { "text": "…", "show_text": true, "font_color": "#252423", "font_size": 10,
               "bold": false, "italic": false, "align": "center", "valign": "middle" },
               "states": { "hover": {…}, "press": {…} } },  // fill, outline, text per state
    "navigator": { "orientation": "horizontal|vertical|grid" | null, "rows": 2, "columns": 3,
                   "shape": { "tile": "pill", "adjust": {…}, "tail": "…" } | null,  // objects.shape (no selector), as "shape" above
                   "padding": 6 | null,   // objects.layout.cellPadding: px between items
                   "styles": { "default": {…}, "selected": {…}, "hover": {…}, "press": {…} },  // per state, only what the visual sets:
                     // show_fill, fill, fill_transparency (objects.fill), show_outline, outline,
                     // outline_weight, outline_transparency (objects.outline), show_text, font_color,
                     // font_size (pt), bold, italic (objects.text), show_accent, accent,
                     // accent_position (top|bottom|left|right), accent_width (objects.accentBar);
                     // show_glow, glow, glow_blur, glow_transparency (objects.glow); show_shadow,
                     // shadow, shadow_blur, shadow_distance, shadow_angle, shadow_transparency,
                     // shadow_position (objects.shadow);
                     // each entry's selector.id is the state; "default" also takes the entry without one
                   "bookmark_group": "<group name>" | null, "selected_bookmark": "<name>" | null,
                   "show_hidden_pages": false, "show_tooltip_pages": false,
                   "allow_deselection": false, "deselection_bookmark": "<name>" | null,
                   "hide_deselection_bookmark": false },  // page and bookmark navigators
    "slicer": { "mode": "list|dropdown|tile|text|between|before|after|relative",
                "selected": ["Bikes"], "inverted": true,
                "search": "bi", "match": "contains|starts_with|ends_with",
                "single_select": false, "select_all": true,
                "range": { "from": 1, "from_exclusive": true, "to": "2025-09-30", "to_exclusive": true,
                           "filter_only": true },
                "domain": { "min": 0, "max": 1000 },                  // range slicers: exact column bounds
                "relative": { "range": "last|next|this", "duration": 3,
                              "period": "days|weeks|calendar weeks|months|calendar months|years|calendar years",
                              "time_period": "minutes|hours",
                              "from": "2026-07-15", "to": "2026-10-14" },      // covered days
                "search_box": true, "header_show": true, "header_text": "Month" },
    "format": {
      "title": { "…": "M8.2 keys", "font_family": "Segoe UI Semibold", "bold": true, "italic": false, "underline": false },
      "subtitle": { "show": true, "text": "…", "font_size": 10, "color": "#605E5C", "alignment": "left",
                    "font_family": "Segoe UI", "bold": false, "italic": true, "underline": false },
      "divider": { "show": true, "color": "#C8C6C4", "width": 1, "style": "solid|dashed|dotted" },
      "legend": { "title": true, "title_text": "…", "labels_color": "#…" },  // labels_color: legend.labelColor
      "x_axis": { "show": true, "title": false, "title_text": "…", "min": 0, "max": 100, "log": false,
                  "gridlines": true, "labels_color": "#605E5C", "title_color": "#…", "font_size": 9,
                  "inner_padding": 20, "category_width": 20, "max_margin": 25, "concatenate": true,
                  "axis_type": "Categorical|Scalar" },             // the last five: category axis only
                  // font_size (x, y and y2): the visual's own only; Desktop 2.158 ignores a theme's
      "y_axis": { "…": "same keys as x_axis" },
      "y2_axis": { "show": true, "title": false, "title_text": "…", "min": 0, "max": 1, "log": false,
                   "labels_color": "#…", "font_size": 9, "display_units": 0, "precision": 1 },
      "series_colors": [ { "color": "#118DFF" }, { "column": 1, "color": "#12239E" },
                         { "field": "Product[Name]", "column": 0, "value": "Bikes", "color": "#E66C37" } ],
      "column_styles": [ { "column": 1, "bg": "#F3F2F1", "fg": "#252423" } ],
      "reference_lines": [ { "kind": "constant|average|min|max|median|percentile|trend", "axis": "y|x",
                             "value": 100, "column": 1, "percentile": 90, "color": "#FF0000",
                             "style": "dashed|solid|dotted", "width": 2, "label": "Goal",
                             "data_label": true, "position": "front|back" } ],
      "small_multiples": { "column": 2, "rows": 2, "columns": 3 },
      "padding": { "top": 14, "bottom": 16, "left": 16, "right": 16 },
      "button_fill": { "show": true, "color": "#282D87", "transparency": 0 },
      "button_text": { "color": "#FFFFFF", "font_size": 10, "font_family": "Segoe UI Semibold", "bold": true }
    }
  } ] } ] }
}
```

**Filters** (`crates/engine/src/preview_filters.rs`). The data of every visual is evaluated with
the filters Desktop applies to its query, each a semantic query `{Version, From, Where}` (public
`semanticQuery` schema; the `Where` conditions are AND-ed):
- the report's, the page's and the visual's `filterConfig` filters that carry a `filter`;
- the selection of every other slicer of the page (`objects.general.filter`, any slicer type,
  hidden ones too: a slicer synced to a page is a slicer of that page, so sync groups need nothing
  more), unless the page's `visualInteractions` holds `{source: <slicer>, target: <visual>,
  type: "NoFilter"}`. A slicer is filtered by the other slicers, never by its own selection;
- a slicer's own search (`objects.general.selfFilter`, the text typed in its search box) narrows
  that slicer's items only.

Conditions: `In` (values or tuples; text compares without case), `Not`, `And`, `Or`,
`Comparison` (`ComparisonKind` 0 `=`, 1 `>`, 2 `>=`, 3 `<`, 4 `<=`; DAX comparison rules, `= null`
is "is blank"), `Between`, `Contains`, `StartsWith`, `EndsWith`. Relative dates: `Now`,
`DateAdd(x, Amount, TimeUnit)` and `DateSpan(x, TimeUnit)` (TimeUnit 0 day, 1 week (from Sunday),
2 month, 3 year, 4 decade, 5 second, 6 minute, 7 hour) resolve against today (UTC); a comparison
with a span uses its start or end (`>=` start, `<` start, `<=`/`>` end, `=` the span). Top N:
an `In` over a `Subquery` with `Top` and `OrderBy` (a measure or an aggregation; `Direction` 1
bottom, 2 top), evaluated in the other filters, ties at the N-th value kept, blanks never.
- Conditions on columns of one table become filter-context filters (codes of one column, or rows
  of the table); filters propagate through relationships like every filter context (the fast
  path reads the rows they leave), so a filter on another table only reaches the visual's values.
- Conditions on measures or aggregations (visual-level filters) are applied to the visual's rows:
  a measure the visual does not show is evaluated as a hidden field.
- `filters_applied` counts the conditions applied. A condition that cannot be applied (hierarchy
  levels, columns of several tables in one condition, columns mixed with measures, unknown
  literals or fields, a measure the evaluator does not support) is skipped and named in
  `filters_ignored_reason` with `filters_ignored: true`; when the filters make the evaluation
  unsupported, the visual's unfiltered data is returned with that flag.
- The result cache key includes a hash of the filter sources (and today's date when a relative
  date is used).

**Conditional formatting** (`preview_format.rs`), read with the M2.5 decoder: gradients (stops
without a value at the lowest / middle / highest value over the visual's rows; RGB
interpolation; blanks per `empty`, else no color), rules (first matching case, else `else`),
field values (the field's text is the color), data bars (from the axis, 0, to the value as
fractions of the cell; explicit min/max honoured; `reverseDirection` mirrors them; the axis
line as `axis` (its place) and `axis_color` when it falls inside the cell; `only: true` for
Show bar only, the value's text not drawn), icons (the first matching case's icon, else
`else`) and web URLs (the rule field's text when it is an absolute `http`/`https` URL). Targets:
tables and matrices `background` → `bg`, `font_color` → `fg`, `data_bars` → `bar`, `icon` →
`icon: {name, position: before|after|only, align: top|middle|bottom}`, `web_url` → `url`, on the
`series` column (`occurrence` n: its n-th binding when the field is bound more than once, as
Desktop's data bars on a duplicated measure); cards `font_color`; charts `data_color` → `fill`
per row. `r` indexes `rows`, `c` `columns`.

**Visual details** (`snapshot_visual.rs`):
- `text` (text boxes): `objects.general.paragraphs[].textRuns[]` with `textStyle`
  (`fontWeight` bold or ≥ 600, `fontStyle`, `textDecoration`, `fontSize` in pt, `color`,
  `fontFamily`) and `url`; capped at 500 paragraphs, 2000 runs, 20 000 characters. A dynamic
  value (a run whose `value` is `{propertyIdentifier: {objectName: "values", propertyName:
  "expr"}, selector: {id}}`, the measure in `objects.values[]` with that `selector.id`) has
  `bound: "<id>"` and its `text` is the evaluated value (see *Bound values* below).
  The run also carries `field` (`[Total]`, the dynamic value's measure or aggregation as field
  text) so the app's editor can give it back to `textbox.set`; a paragraph with `listType` (BIcraft's
  list key, docs/formats/pbir.md) has `list: "bullet" | "number"` (M11.20).
- `action` (buttons, images, shapes with `visualLink`): `{type: back | bookmark | drillthrough |
  page_navigation | qna | web_url | apply_all_slicers | clear_all_slicers | null (off),
  destination?, bookmark?, url?, tooltip?}`, for the app's action editor (`visual.set_action`).
  `interaction.button` (what a reader's click does) now also carries `url` and covers
  `Drillthrough`, `WebUrl`, `Bookmark` and `Qna`.
- **Bound values** (`bound_values.rs`): text box dynamic values, a button's `text.text` and
  `fill.fillColor` (default state) and a visual's `title` / `subTitle` text bound to a measure
  (or an aggregation) are evaluated like a card in the visual's filter context (report, page and
  visual filters, slicers; no grouping) and written into `text.paragraphs[].runs[].text`,
  `button.text` / `button.fill`, `title` and `format.subtitle.text`, formatted with the
  measure's format string in the model culture. When one cannot be evaluated the visual gets
  `preview_reason` (string) and the renderer reports it as `preview`.
- `data.totals` (tables, `tableEx`): the total row, one cell per column (`null` under grouping
  columns), each value evaluated in the grand-total filter context (the visual's filters without
  its grouping columns: `ALLSELECTED` sees the outer filters, so shares are 100 % and ranks 1),
  not the sum of the rows; absent when measure filters apply (`totals_reason` when it could not
  be evaluated). `data.subtotals` (matrices, `pivotTable`): `[{rows, columns, data}]`, the
  visual evaluated again with only its first `rows` row fields and `columns` column fields as
  grouping (every displayed level but the leaf one, which is `data.rows`; `rows` = `columns` =
  0 is the grand total, evaluated like a table's total row), one row per member combination with
  a cell per column (`null` under the dropped fields); `subtotals_reason` when a level could not
  be evaluated. `data.row_levels` / `column_levels` (when a hierarchy has several fields): the
  levels Desktop displays when expanded level by level (1 unless the projections are `active`
  and `expansionStates` expands the next ones); `data.row_expansion` / `column_expansion`:
  `{open: [bool per level], toggled: [[member path]]}` (a member is expanded when its level is
  open XOR its path is toggled; absent: no active levels, the top level only);
  `data.row_level_totals` / `column_level_totals` (with `perRowLevel` / `perColumnLevel`):
  `[{show?, label?}]` per level, the top field's entry being the grand total (docs/formats/pbir.md,
  "Matrix expansion"). `format.table` also carries a matrix's `row_subtotals`,
  `column_subtotals` (booleans), `row_subtotals_position` (`top` / `bottom`) and
  `row_subtotals_label` / `column_subtotals_label`; `format.kpi` a KPI's `display_units`,
  `precision` and `font_size` (`indicator` object). `data.formatting_ignored_reason`: conditional formatting reading a measure that
  could not be evaluated was left out (the renderer reports `preview`).
- `button` (`actionButton`): the `default` state entry (`selector.id`), else the entry without
  a selector, of `text`, `icon` (`shapeType`) and `fill`.
- `shape` (`shape` visual): `tile` (`shape.tileShape`), `adjust` (the `shape` object's numeric
  adjustments by their schema names: `rectangleRoundedCurve`, `hexagonSlant`…), `tail`
  (`speechBubbleTailPosition`), `cap` (`linecapType`) and `angle` (`rotation.shapeAngle`, else
  `angle`, in 0–360) from the entries without selector; `fill`, `fill_transparency`, `line`,
  `line_weight`, `line_transparency` from the `default` state entry only, `show_fill` /
  `show_line` from it, else from the entry without selector (where Desktop reads them,
  [formats/visual-options.md](formats/visual-options.md), "State selectors"). Drawn by
  `crates/render/src/shapes.rs`.
- `slicer`: `mode` from the type (`listSlicer` list, `advancedSlicerVisual` tile, `textSlicer`
  text) or `objects.data.mode` (`Basic` list, horizontal `general.orientation` tile,
  `Dropdown`, `Between`, `Before`, `After`, `Relative`); `selected` / `inverted` / `range` from
  the selection (an `In`, a `Not(In)`, an `=` comparison, other comparisons); `search` and
  `match` from a `Contains` / `StartsWith` / `EndsWith` selection (a text slicer's typed text),
  else from `objects.general.selfFilter` (the search box); `single_select`, `select_all` from
  `objects.selection`. The displayed range: `objects.data` `numericStart`/`numericEnd` (numbers)
  or `startDate`/`endDate` (dates) win over the filter's bounds, as Desktop shows them (it shows
  the full domain for a filter-only range). A strict filter bound on a date becomes the last
  (first) included day — `< 2025-10-01` gives `to: "2025-09-30"` —; on a number it stays and
  `to_exclusive` / `from_exclusive` is set; `filter_only: true` when no bound comes from `data`
  (Desktop then shows the column's whole domain). `relative` from `data.relativeRange` /
  `relativeDuration` / `relativePeriod` / `relativeTimePeriod` (lower-cased, `None` left out),
  with `from` / `to` the days the settings cover, as Desktop shows them under the inputs, today
  (UTC) included: last n days / weeks / months / years end today (last 3 months on 2026-10-14:
  2026-07-15 – 2026-10-14), next n start today, this is the calendar period (weeks from Sunday),
  calendar variants are the n whole periods before / after the current one (checked against
  Desktop 2.158 for last / next days, last months, this week / month);
  `search_box` from `general.selfFilterEnabled`; `header_show` from `header.show`; `header_text` from `header.text` (the header shown
  instead of the field names); `domain`
  (modes `between`/`before`/`after`, column fields) is the min and max of the column over every
  stored row (dictionary values some row uses; up to 50 M rows), numbers or ISO dates. The same objects are read from a legacy `Report/Layout`
  (`singleVisual.objects`) and from PBIR `visual.json` (`visual.objects`).
- `format.subtitle` (`visualContainerObjects.subTitle`: `show`, `text`, `fontSize`, `fontColor`,
  `alignment`, `fontFamily`, `bold`, `italic`, `underline`; the title takes the same font keys),
  `format.divider` (`visualContainerObjects.divider`: `show`, `color`, `width`, `style`
  lower-cased), `format.legend.title`/`title_text`,
  `format.x_axis`/`y_axis` (`categoryAxis`/`valueAxis`: `show`, `showAxisTitle`, `titleText`,
  `start`, `end`, `logAxisScale`, `gridlineShow`, `labelColor`, `fontSize`, `labelDisplayUnits` →
  `display_units`, `labelPrecision` → `precision`; category layout: `innerPadding` →
  `inner_padding` (%), `preferredCategoryWidth` → `category_width` (px), `maxMarginFactor` →
  `max_margin` (%), `concatenateLabels` → `concatenate`, `axisType` → `axis_type`),
  `format.y2_axis` (the secondary axis: `valueAxis` `secShow`, `secShowAxisTitle`, `secTitleText`,
  `secStart`, `secEnd`, `secLogAxisScale`, `secLabelColor`, `secFontSize`,
  `secLabelDisplayUnits`, `secLabelPrecision`), `format.padding`
  (`visualContainerObjects.padding`), `format.button_fill` / `button_text` (action buttons'
  `fill` and `text` objects, so that a theme's button style reaches the page), all with the theme's
  visual styles under the visual's own values, like the other format keys.
- `format.series_colors`: `dataPoint.fill` literal colors — no selector (the default color of a
  single-series chart), `selector.metadata` (a series: `column` is the data column), or
  `selector.data[0].scopeId` comparison (one category or legend value).
- `format.column_styles`: table/matrix `values.backColor` / `fontColor` with a `metadata`
  selector.
- `format.reference_lines`: the analytics lines (M2.5 `analytics`), hidden ones and forecasts
  left out; statistical lines name their series by `column`. Each entry carries its friendly
  keys (`kind`, `axis`, `value`, `color`, `style`, `width`, `transparency`, `data_label`, …);
  error bars (`kind: "error_bars"`) add `relative`, `bar`, `match_series`, `marker`,
  `marker_shape`, `band`, `band_style`, `band_color`; symmetry shading
  (`kind: "symmetry_shading"`) adds `upper_color`, `lower_color`; the scatter's ratio line is
  `kind: "ratio_line"`; anomaly detection (`kind: "anomaly_detection"`) adds `sensitivity`,
  `marker`, `marker_shape`, `marker_size`, `marker_color`, `marker_transparency`, `band`,
  `band_style`, `band_color`. The Analytics tab reads the editable list from `visual.props`
  (`analytics`), not from this view.
- `data.error_bars`: `[{column, upper: [...], lower: [...]}]`, one entry per series with enabled
  error bars, `column` its data column, the bounds aligned with `rows` (`null` where a bound is
  missing; relative bounds already added to the series value). Absent when no error bars apply.
- `data.anomalies`: `{column, sensitivity, estimate: "bicraft", expected: [...], low: [...],
  high: [...], anomaly: [...]}` for a line chart's anomaly detection: per row, the expected
  value, the expected range and whether the value is outside it (`null` where the series has no
  value). BIcraft's own estimate (`crates/render/src/anomaly.rs`), the one the renderer draws,
  never Power BI's result. Absent when the setting is off, with a legend or under four values.
- `format.small_multiples`: the data `column` bound to the chart's small-multiples role, and the
  custom grid size.

**Viewer.** Data bars, cell colors, icons, web-URL links and column styles in tables and in the
value cells of matrices (a matrix cell takes the style of its data row; a cell that sums several
rows, subtotals and totals take none); card value color; per-series, per-value and conditional
data colors in bar/column/line/area/combo/pie charts; reference lines on the value axis
(statistics computed over the drawn series); trend lines (`kind: "trend"`: the least-squares line
of each series over the categories, clipped to the value axis; on scatter charts, of y on x; not
on stacked or 100 % charts); error bars (whiskers with caps on clustered column and bar charts
and on line series, and a line's band; not on stacked, 100 % or legend-split charts); on scatter
charts, x and y constant and statistical lines, the ratio line and symmetry shading;
value-axis `min`/`max`/log scale;
axis titles when the report turns them on; axis labels hidden with `show: false`; the subtitle
under the title, title and subtitle in their font (family, bold, italic, underline); the divider
as a line under them (`solid`/`dashed`/`dotted`, width clamped to 1–12 px); small multiples as a grid of panels (one per value, up to 36), each drawn by the
chart's renderer; slicers in their mode with the selection shown (checked items, highlighted
tiles, the dropdown's value, the range, a text slicer's typed text); text boxes with their runs; buttons with text, icon and
fill. A visual whose `filters_ignored` is set shows a "filters ignored" badge (the reason in its
tooltip). Snapshot text only reaches the page as text nodes; colors go through `hexColor`, fonts
through `safeFont`, numbers are clamped. Icons are drawn from the icon name (shape from
arrow/triangle/flag/star/sign/symbol/traffic, color from high/medium/low); a name only reaches an
attribute after matching `[A-Za-z0-9_-]{1,100}`. A web URL becomes a link only when `new URL()`
parses it with an `http:`/`https:` protocol; it opens in a new tab (`noopener noreferrer`, no
referrer) on a click, never by itself.

Numbers and chart text follow the PNG renderer's Desktop 2.158 rules (`docs/render.md`): the
model culture gives only the decimal and group separators (U+00A0 in French, not Intl's U+202F);
display units stay K, M, bn, T; `%` and a leading `+` sit where the format string puts them
("38,0%", "+30,4 %"); axes use "auto" units from 10 000 / 100 000 with uniform decimals, or the
axis' `display_units`; a date value with a date format string is written with that format in the
culture. Chart axis labels, legends and data labels are 9 pt (8 pt without a theme) unless the
visual sets its own axis `font_size`; tables and slicers keep the theme's label size. Category
labels stay flat with a gap of 0.4 font sizes, are skipped (every 1, 2, 3, 6 or 12) on a date
axis, word-wrapped when every word fits, else turned 35° (90° under 1.9 font sizes).

### Images (still version 1; every field optional)

```json
{
  "report": { "pages": [ {
    "wallpaper": null | { "color": "#EAEAF3" | null, "transparency": 0, "image": IMAGE | null, "fit": "normal|fit|fill" },
    "background_transparency": 0,
    "size": "16:9|4:3|letter|tooltip|custom", "vertical_alignment": "top|middle" (when set),
    "display_option": "FitToPage|FitToWidth|ActualSize" (when the page has one; View › Page view),
    "background_image": null | { "image": IMAGE, "fit": "normal|fit|fill", "transparency": 0 },
    "visuals": [ {
      "image": IMAGE + { "scaling": "normal|fit|fill" },           // image visuals only
      "format": { "background": { "image": IMAGE, "image_fit": "normal|fit|fill" } }
    } ]
  } ] }
}
```

`IMAGE` is `{ "id": "<32 hex>", "mime": "image/png|image/jpeg|image/gif|image/webp|image/svg+xml",
"width": 1800, "height": 272 }` (`width`/`height` only when the header gives them; never for SVG).
`id` is the first 128 bits of the SHA-256 of the bytes. **Bytes are never in the snapshot or the
SSE stream**: the viewer serves them at `GET /api/asset/<id>`. A key is absent when nothing sets it
(`wallpaper` absent → the viewer keeps its default grey).

**Producer** (`crates/engine/src/images.rs`, `Session::snapshot_assets()`):
- Wallpaper = page `objects.outspace`; page background = `objects.background`; each property from
  the page, else the theme's `visualStyles.page.*` (custom over base). The image is the `image`
  property: `{image: {name, url, scaling}}` in report objects, `{name, url, scaling}` in themes.
  One transparency covers the colour and the image (Desktop draws the image layer with that
  opacity). `background_transparency` is the page background's.
- Image visuals: `objects.general.imageUrl` and `objects.imageScaling.imageScalingType`.
  Visual backgrounds: `visualContainerObjects.background` `image` (the format's `show` and
  `transparency` apply to it).
- A `url` is a `ResourcePackageItem` → `StaticResources/<PackageName>/<item path>` (the item path
  from `report.json` `resourcePackages`, else the item name; `..` refused), or a `data:` URL
  literal (base64 or percent-encoded). Web URLs are never fetched.
- The type is sniffed from the bytes; anything else (or unreadable, > 16 MiB, past 1024 distinct
  images or 128 MiB in total) is left out of the snapshot. Resolved once per `revision` (plus a
  report fingerprint) and shared by `snapshot()` and `snapshot_assets()`.
- `bicraft view` and `bicraft mcp --view` publish both with `ViewHandle::publish_with_assets`
  (assets first, so a page never sees an id that is not served yet; the previous set is dropped).

**Server** (`crates/view`): `GET /api/asset/<id>` with `id` matching `[0-9a-f]{16,64}` (else 400;
unknown → 404), same Host check and GET-only rule as every route. Only the five types above are
accepted on publish, each served with its canonical `Content-Type`, `X-Content-Type-Options:
nosniff`, `Cache-Control: public, max-age=31536000, immutable` + `ETag` (304 on `If-None-Match`),
`Cross-Origin-Resource-Policy: same-origin` (no embedding from other sites) and
`Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; img-src data:; sandbox`.
SVG is allowed: the page only uses it as an `<img>`/CSS background (no script runs in image
context), and opened directly as a document the CSP sandbox gives it an opaque origin with no
script or fetch. Caps: 16 MiB per asset, 1024 assets, 128 MiB in total. The page CSP allows
`img-src 'self'`.

**Viewer** (rendering mode only; the wireframe skips images): Desktop's background-image rules:
`normal` = natural size, centred; `fit` = stretched to the box; `fill` = full width, centred.
Wallpaper = colour (blended with white by its transparency) and image layer behind the page;
page background colour uses `background_transparency`; page/visual background images are layers
with the transparency as opacity. Image visuals are an `<img>` with `object-fit` from `scaling`:
`normal` → `contain`, `fit` → `fill` (Desktop calls it "Stretch"), `fill` → `cover`. Every URL is
`/api/asset/` + an id checked against `^[0-9a-f]{16,64}$`; images decode asynchronously and never
change the layout.

### Custom visuals (2026-11-19; still version 1; every field optional)

```json
{
  "report": {
    "custom_visuals": [ { "name": "<guid>", "display_name": "Tiny gauge" | null, "icon": IMAGE | null } ],
    "pages": [ { "visuals": [ { "custom_visual": { "name": "<guid>", "display_name": "…", "icon": IMAGE | null } } ] } ]
  }
}
```

`custom_visuals` lists the custom visuals the report embeds and registers (a `CustomVisual`
resource package, not disabled, whose `.pbiviz.json` is present; docs/formats/pbiviz.md), in
`resourcePackages` order, at most 200: the Visualizations pane's cells. `icon` is the
`content.iconBase64` `data:` URL of the `.pbiviz.json`, served as an asset like the images above.
A visual whose type is one of them (ignoring case) carries the same object as `custom_visual`; the
renderer draws it as a labelled placeholder with that icon (docs/render.md), never by running the
visual's code. **Producer**: `crates/engine/src/images.rs` (`custom_visuals_json`, through
`custom_visuals::gallery`).

`bicraft_visuals` (2026-11-23, ROADMAP M12.4) lists BIcraft's own visuals whether the report
embeds them or not: `[{ "name": "bicraftSpec", "guid": "<guid>", "display_name": "BIcraft Spec",
"version": "1.1.0.0", "embedded": bool, "icon": IMAGE | null }]`. The Visualizations pane draws
them after the built-in types and before the other `custom_visuals` (an embedded one is not
listed twice), each with its own drawing in the theme's ink (`bicraft_visuals::icons`; `icon` is
the same drawing as the package's 20 × 20 PNG); a click sends `name` to `visual.add` or `visual.set_type`, which embed the package
(docs/community-visuals.md "Built in by default"). **Producer**: `crates/engine/src/images.rs`
(`bicraft_visuals_json`, through `builtin_visuals::gallery`).

A BIcraft Spec visual (type `bicraftSpecVisual7F3A9C2E5B1D4E6F8A0C2B4D`, docs/formats/spec-visual.md)
also carries `spec`: its `spec.json` property parsed and checked (the spec object as written),
or `spec_error` (text) when it has none or it does not parse; the renderer draws `spec` over
the visual's `data`. **Producer**: `crates/engine/src/spec_visual.rs` (`snapshot_fields`).

A visual of BIcraft's chart pack (docs/formats/chart-pack.md: BIcraft Org Chart, Venn, Marimekko,
Calendar Heatmap, Sankey, Scatter+, Line / Area+, by their guid) carries `chart_pack: {kind: "org" |
"venn" | "mekko" | "calendar" | "sankey" | "scatter" | "linearea", visual: display name, options:
{…}, today?}`: its format properties under the layout's keys (only those set), and for Line / Area+
`today` (the preview date at midnight UTC in epoch milliseconds, for a forecast from today); the renderer lays out the visual's `data` rows by their roles with them.
**Producer**: `crates/engine/src/chart_pack.rs` (`snapshot_fields`).

**BIcraft visuals** (2026-11-24, docs/gaps.md G29). The page does not draw BIcraft's fourteen visuals
itself: the publisher of `bicraft view` adds to each one whose `data.status` is `ok` (at most 64
per snapshot) `drawing: {asset: "<id>", width, height, hits: [{shape: {rect: [x, y, w, h]} |
{circle: [cx, cy, r]} | {poly: [[x, y], …]}, action: {…} | null, ctrl: bool, tip: [[label,
value]]}]}`: the renderer's PNG of the visual at twice its size (served at `/api/asset/<id>`,
drawn as `report.render` and the app draw it, the points the selection leaves out at 40 %) and
its hit map in the visual's own pixels, back to front. The page shows the image in the visual's
box and lays the hits over it: a click sends `action` (with `ctrl` from the click when `ctrl` is
true), a hover shows `tip`. A data point's action is `select {point, points?}`: `points` lists
every point of a mark that stands for several (a histogram bin, a Sankey node, a Venn set), the
background's `select {point: null}`. Without `drawing` the page keeps the custom-visual
placeholder. **Producer**: `crates/engine/src/view_drawings.rs` (`add_drawings`, called by
`apps/bicraft-cli/src/view_cmd.rs`).

### Renderer formatting (M9.6; still version 1; every field optional)

Keys under a visual's `format`, for the PNG renderer and the viewer. Each sub-key is present
only when the theme or the visual sets it.

```json
"format": {
  "labels": { "show": true, "color": "#252423", "font_size": 9, "font_family": "Segoe UI", "bold": false,
              "italic": false, "transparency": 0,
              "display_units": 0|1|1000|1000000|1000000000|1000000000000|-1, "precision": 2,
              "position": "auto|insideend|outsideend|insidecenter|insidebase|above|under|outside|inside|preferoutside|preferinside",
              "density": 50, "background": true, "background_color": "#FFFFFF", "background_transparency": 80,
              "label_style": "Category, percent of total", "percent_precision": 1, "overflow": false },
  "series_labels": [ { "column": 1, ...the "labels" keys } ],
  "lines": { "show": true, "width": 3, "style": "solid|dashed|dotted|custom", "chart_type": "linear|smooth|step",
             "stepped": true, "show_marker": true, "marker_shape": "circle|square|diamond|triangle|x|shortDash|longDash|plus",
             "marker_size": 5, "marker_color": "#118DFF", "show_area": true },
  "series_lines": [ { "column": 1, ...the "lines" keys } ],
  "card": {                                                        // cardVisual (new card) only
    "accent_bar": { "show": true, "position": "left|right|top|bottom", "color": "#118DFF", "width": 6, "transparency": 0 },
    "value": { "show": true, "font_size": 28, "color": "#252423", "font_family": "…", "bold": true, "italic": false,
               "transparency": 0, "display_units": 1000, "precision": 1, "alignment": "left|center|right", "blank_as": "-" },
    "label": { "show": true, "text": "…", "font_size": 12, "color": "#605E5C", "font_family": "…", "bold": false,
               "italic": false, "position": "belowvalue|abovevalue", "alignment": "left|center|right" },
    "layout": { "style": "cards|table", "orientation": 0|1|2, "rows": 2, "columns": 3, "max_tiles": 6, "auto_grid": true,
                "alignment": "…", "callout_size": 50, "cell_padding": 10, "collage": "Default", "content_order": "callout_image_referenceLabel" },
    "image": { "show": true, "type": "image|imageUrl|imageData", "url": "https://…", "position": "left|right|top|bottom",
               "size": 40, "fit": "fit|stretch|fill|normal", "transparency": 0 },
    "reference_label": { "position": "below|right|chartdominant|imagedominant", "arrangement": "rows|columns",
                         "style": "sentence|tabular", "background": true, "background_color": "#…", "background_transparency": 0,
                         "title": { "show": true, "text": "…", "font_size": 9, "color": "#…", "bold": false },
                         "value": { "show": true, "font_size": 9, "color": "#…", "bold": true, "display_units": 0, "precision": 0 },
                         "detail": { "show": true, "font_size": 9, "color": "#…", "display_units": 0, "precision": 0 } }
  },
  "gauge": { "min": 0 | { "field": "[Min]", "column": 2 }, "max": 500 | FIELD, "target": 300 | FIELD,  // gauge only
             "callout": { "show": true, "color": "#…", "font_size": 28, "font_family": "…", "bold": false,
                          "display_units": 1000, "precision": 0 },
             "target_label": { ...the "callout" keys }, "target_color": "#…" },
  "pie": { "inner_radius": 60, "start_angle": 90 },               // pieChart / donutChart only
  "ribbon": { "show": true, "color": "#A0A0A0", "match_color": true, "transparency": 30,  // ribbonChart only
              "border": false, "border_color": "#252423", "border_match": false, "border_width": 1,
              "border_transparency": 0, "gap": 5 },           // ribbonBands, layout.ribbonGapSize (Spacing)
  "totals": { "show": true, "color": "#252423", "font_size": 9, "font_family": "Segoe UI",  // ribbonChart only
              "bold": false, "display_units": 1000, "precision": 1 },
  "table": { "row_padding": 6, "font_size": 10.5, "header_font_size": 10.5,  // tableEx / pivotTable only
             "column_widths": [ { "column": 0, "width": 96 } ], "sort": [ { "column": 0, "descending": true } ] }
}
```

**Producer** (`crates/engine/src/theme_format.rs`): property names as Desktop writes them in
reports and themes. Sources, lowest priority
first: theme `visualStyles["*"]["*"]`, then `visualStyles[<type>]["*"]` (base then custom theme),
then every visual `objects` entry with no selector or with the `default` state selector. Mapping:
`labels` ← `labels` (`labelDisplayUnits`, `labelPrecision`, `labelPosition` or the pie's
`position`, `labelDensity`, `enableBackground` or the pie's `background` `on`/`off` — `auto`
clears it —, `backgroundColor`, `backgroundTransparency`, `labelStyle`,
`percentageLabelPrecision`); `lines` ← `lineStyles` (`strokeShow`, `strokeWidth`, `lineStyle`,
`lineChartType`, `stepped`, `showMarker`, `markerShape`, `markerSize`, `markerColor`, `areaShow`;
`stepped: true` also when `lineChartType` is `step`); `card` ← `accentBar`, `value`, `label`,
`layout` (`rowCount`, `columnCount`, `maxTiles`, …), `image`, `referenceLabelLayout`,
`referenceLabel`, `referenceLabelTitle`/`Value`/`Detail`; `gauge` ← `axis` (`min`/`max`/`target`
literals; a field expression — Desktop's fx binding — becomes `{field, column?}`, `column` being
the visual's data column when the field is also bound), `calloutValue`, `target`,
`dataPoint.target`; `pie` ← `slices` (`innerRadiusRatio`, `startAngle`); `table` ← `grid.rowPadding`,
`values.fontSize`, `columnHeaders.fontSize` (theme then visual), plus the visual's `columnWidth`
entries with a `metadata` selector (`column`: that projection's data column, `width` in px) and
its `sortDefinition` fields bound to the visual (`crates/engine/src/snapshot_visual.rs`). Enumerations shown
lower-case above are lower-cased; `marker_shape`, `label_style`, `collage`, `content_order` and
`image.type` keep Desktop's spelling. `series_labels` / `series_lines` list the `labels` /
`lineStyles` entries whose selector is `{"metadata": queryRef}` (the visual's own values only),
`column` being the data column of that projection (unknown refs are left out). `display_units`
`0` means auto, `1` none, otherwise the divisor.

**Consumers.** The PNG renderer (`crates/render/src/axes.rs`, `charts.rs`) and the live viewer
(`crates/view/src/page.html`) lay cartesian charts out with the same rules: value ticks from the
plot length (d3 nice domain, 3/5/8 intervals), category bands with scrolling, category labels
flat/skipped/wrapped/turned (horizontal bars: cut to `max_margin`), first-of-month date
categories as months ("Jan 2025"), `labels` / `series_labels` (display units, precision,
position, background; 100 % charts: two-decimal shares), `lines` / `series_lines`, the visual's
axis `font_size`, `gridlines`, `labels_color`, `inner_padding`, `category_width`, and `AMPM` with
the culture's designators. The viewer measures text with the browser's fonts. Both also share the
classic look of a report without a theme (classic palette, #C8C8C8 regular 12 px titles, #777
text, #EAEAEA gridlines, 5 px chart padding, plot 4 px under the title), `format.padding`, and
`parts.rs`'s pie, donut, funnel and treemap rules (12 px DIN labels — in the viewer an
`@font-face` stand-in, Segoe UI at `size-adjust: 92%` —, `label_style`, inside / outside labels,
funnel rate brackets, squarified treemap). In small multiples the viewer re-indexes
`series_labels` / `series_lines` columns past the split column (the renderer does not yet).
The viewer also draws `bodies.rs`'s classic card, new card (`format.card`: tiles, layout,
accent bar, label/value fonts, category table), multi-row card, `parts.rs`'s gauge and KPI
(`fmt.rs` `fmt_kpi` display units and precision, the fixed 59.7 px classic value, "!" shape,
goal as two runs, no area for a flat trend), and `tables.rs`'s grids (row and header heights,
sort arrow, natural column widths, engine `totals`/`subtotals`/`row_levels`/`total_rows`,
matrix `row_expansion`/`column_expansion` member by member with a header row per column level,
`row_level_totals`/`column_level_totals` and subtotal labels, bottom subtotals, bold upper
levels and the 10 px stepped indent, clipped last row and scrollbar thumbs, column scrolling,
conditional formats), `slicers.rs`'s slicers (classic list with "Select all", search
row and hierarchy chevrons, tiles, dropdown, between/before/after inputs and slider with the
short date and `#,0.00` input formats, relative date with its covered days, `listSlicer`,
`advancedSlicerVisual`, `textSlicer`), `bodies.rs`'s buttons (`button`, `format.button_fill` /
`button_text`, else Desktop's blank 2 px outline) and the empty-visual prompt by hand: an SVG for shapes plus absolutely placed text spans whose
baseline lands where the renderer's does (Base::Central = 0.414 em under the centre; Chrome's
whole-pixel baseline offset is measured per font and size). Text drawn by charts uses the same
placement. The page canvas is `content-box`, so its 1 px border no longer shifts and squeezes
every visual. Numbers and dates follow `fmt.rs`/`data.rs`: a column's `format_string` applies to
the column and its implicit measures (not the counts), a decimal column without one shows two
decimals, named formats (`Long Date`, `Short Date`, `General Date` with a midnight time, `Long
Time`…) use the culture's patterns (a copy of `bicraft-locale`'s table), a KPI goal its own
format. `bicraft view` and `bicraft mcp --view` republish until every bound visual has its
data (a snapshot computes for at most `data_budget_ms`; `mcp --view` does it between tool calls).

### UI language (M10.2; still version 1; every field optional)

Power BI Desktop writes the text it generates for a visual in **its own UI language** (File >
Options > Regional settings), not in the model culture: the auto title ("Total by Region and
Category" / "Total par Region et Category"), implicit-measure names ("Sum of Amount" / "Somme de
Amount"), "(Blank)", the "Total" row and column of tables and matrices and the waterfall's total
bar, a slicer's "All" / "Multiple selections". The snapshot carries that text already in the
session's UI language, so the viewer and the PNG renderer only draw it.

```json
{
  "ui": { "language": "fr-FR", "strings_language": "fr", "fallback": true /* only when the language has no table */,
          "strings": { "title_by": "{v} par {c}", "and": " et ", "list_separator": ", ", "blank": "(Vide)", "total": "Total", "all": "Tout",
                       "multiple_selections": "Plusieurs sélections", "select_all": "Sélectionner tout", "open_in_desktop": "Ouvrir dans Power BI Desktop",
                       "search": "Rechercher", "other": "Autre", "increase": "Augmenter", "decrease": "Diminuer", "goal": "Objectif",
                       "empty_visual": "Ajoutez des champs pour afficher des données dans ce visuel", "no_filters_applied": "Aucun filtre actif",
                       "year": "Année", "quarter": "Trimestre", "month": "Mois", "day": "Jour",
                       "aggregations": { "sum": "Somme de {0}", "avg": "Moyenne de {0}", "min": "Min de {0}",
                                         "max": "Max de {0}", "countnonnull": "Nombre de {0}", "count": "Nombre de {0}",
                                         "median": "Médiane de {0}", "standarddeviation": "Écart type de {0}",
                                         "variance": "Variance de {0}" } } },
  "report": { "pages": [ { "visuals": [ {
    "auto_title": "Somme de Amount par Region et Category" | null,
    "fields": [ { "…": "as above", "label": "Somme de Amount" | null } ]
  } ] } ] }
}
```

- `ui.language` is the session's UI language (`view.set_language {ui_language}`, `report.render
  {ui_language}` for one call, `bicraft view|render|mcp --view --lang TAG`; default `en-US`). Tags
  are BCP 47 (`fr`, `fr-CA`, `pt_BR` → `pt-BR`); a language without a table uses the English one
  and sets `fallback`. Phrase tables: en, fr, de, es, it, nl, pt, ja (de to ja are BIcraft's own
  translations of the English, [locales.md](formats/locales.md) "UI phrases").
- `fields[].label`: the caption Desktop shows for the field: the visual's rename (projection
  `displayName`), else the measure or column name, the implicit-measure name for an aggregation
  (`aggregations` keys are the PBIR aggregation names, lowercased: `countnonnull` is Desktop's
  "Count", `count` its "Count (Distinct)"), or a hierarchy level's name. `null` when unknown.
- `auto_title`: the title Desktop shows when the visual sets none (consumers draw `title` when it
  is set, else `auto_title`): up to three value labels ("A and B", "A, B and C": `list_separator`
  then `and`), then `title_by` with up to two category / legend / column labels (`{v}`/`{c}`; Japanese puts the categories first);
  categories alone are comma-separated. `null` when no field has a label. A gauge names its
  value, minimum, maximum and target (up to four values, in that order: "Total, Low, High et
  Goal"), a KPI its indicator and goal by its trend axis ("Total et Goal par Month"), as Desktop
  2.158 does.
- `goal` labels a KPI's goal (`bicraft-render` `parts.rs`).
- `year`/`quarter`/`month`/`day` are for consumers that name date parts themselves. The levels of
  Desktop's own date hierarchies are model names (Desktop creates the auto date table's columns in
  its language at the time), so `label` keeps them as written.
- The strings are our own short wording per language. The texts Desktop also draws on a visual
  were **checked against Power BI Desktop 2.158.1304.0 (Store), French UI** on PBIX files written
  by BIcraft: the auto titles ("Somme de Amount par Region et
  Category", "Somme de Amount et Max de Amount par Region", "Moyenne de Amount, Min de Amount et Max
  de Amount par Category"), every implicit-measure name (Somme, Moyenne, Min, Max, Nombre for both
  counts, Médiane, Écart type, Variance), "(Vide)" on axes, tables and slicers, "Total" in tables,
  matrices and the waterfall, the waterfall legend (Augmenter / Diminuer / Total), a dropdown
  slicer's "Tout" and "Sélectionner tout". "Plusieurs sélections" checked against the slicer oracle. Not checked: "Autre",
  "Rechercher".
  **English checked in Desktop 2.158** with its application language switched to English for
  the test (and restored): the generated report of `cargo xtask oracle-gen uilang` and the slicer
  oracle report (en-US model). Seen: auto titles ("Sum of Amount by Region", "Total by Region
  and Category", "Sum of Amount and Max of Amount by Region", "Average of Amount, Min of Amount
  and Max of Amount by Month", a gauge's "Total, Low, High and Goal", a KPI's "Total and Goal by
  Month", a pie with a legend "Total by Region and Category"), the nine implicit-measure names
  (Sum, Average, Min, Max, "Count of" for both counts, Median, "Standard deviation of",
  Variance), "(Blank)" on axes, tables, slicers and donut labels, "Total" in tables, matrices and
  the waterfall, the waterfall legend Increase / Decrease / Total, "Goal:" on a KPI, slicer
  "Select all", "All", "Multiple selections", the relative-date slicer's "Last", "Next",
  "This", "Months", "Days", "Week", "Select", a line telling that no filter is active, and a
  prompt on an empty visual. BIcraft draws its own wording for those two prompts
  (`no_filters_applied` and `empty_visual` in `ui.strings`). Tooltips list "field label  value" rows (a pie adds the
  share: "60 521,95 (6,65%)"). The values of an implicit "Standard deviation of" / "Variance of"
  are the population forms (STDEV.P / VAR.P), which the engine now computes. de, es, it, nl, pt,
  ja remain unverified.

**Producer**: `crates/engine/src/ui_lang.rs`. **Viewer**: the visual's text (`auto_title`,
field `label`, and/blank/total/all/multiple selections/search) comes from the snapshot; its own
chrome (buttons, badges) still follows the browser language. Exception: the header's "Open in Power BI Desktop" button
is labelled with `ui.strings.open_in_desktop` (all eight languages).

### Open in Power BI Desktop (still version 1; every field optional)

`source.desktop` says what the viewer's "Open in Power BI Desktop" button opens
(`Session::desktop_target`, `crates/engine/src/desktop_open.rs`):

- `path`: the absolute path of the saved `.pbix` / `.pbit` file, or of the project's `.pbip` file
  (also when the session opened the project folder or an item folder: the `.pbip` named like the
  items, else the first one in the folder). `null` when there is nothing Desktop can open.
- `reason`: why `path` is `null`: nothing saved yet (a new model), a standalone `model.bim`
  (Desktop does not open one), a project folder without a `.pbip` file, a file that no longer
  exists.
- `unsaved`: the session has model or report edits that are not in that file (`dirty.model ||
  dirty.report`).

The page shows the button only when the server embedded a token (`<meta
name="bicraft-open-token">`; not with `--no-open-button`), disabled with `reason` as its tooltip
when `path` is `null`, and with a warning dot and tooltip when `unsaved` (the opened file will not
contain those edits: save first). A click sends `POST /api/open-in-desktop` with the header
`X-Bicraft-Token` and no body, and shows the result next to the button.

The server (`crates/view`) opens **only** the target the front end set from its own session on
the last publish (`ViewHandle::set_open_target`), never a path from the request, with the opener
it was started with (`bicraft_engine::open_with_default_app`: an existing `.pbix`, `.pbit` or
`.pbip` file only, handed to `explorer.exe` on Windows (full path under `%SystemRoot%`, the
path always quoted), `open` on macOS or `xdg-open` elsewhere; no shell, no other program). Checks,
in order: localhost `Host` (403; DNS rebinding), method `POST` (405), the action is enabled (404),
the token (128 random bits per run, compared in constant time), `Origin` equal to
`http://<Host>` when present (`null` and other sites refused) and `Sec-Fetch-Site: same-origin`
when present (403), a target exists (409), no launch in the last 3 s (429; a double click starts <!-- perf-guard: ok (a cooldown) -->
one Desktop). A cross-site page cannot read the token (no CORS headers) and cannot send the
custom header without a CORS preflight, which the server refuses. Response: JSON `{ok, path,
unsaved}` or `{ok: false, error}` (500 when the OS has no application for the file: on Linux
without `xdg-open` or without a registered handler). BIcraft never depends on Power BI Desktop
at runtime: this only hands a file to the OS on an explicit click. The same action is the
`file.open_in_desktop {dry_run?}` command (CLI `bicraft run --open F --cmd file.open_in_desktop`,
MCP `run_command`).

### Server mode (still version 1; every field optional)

`bicraft serve` serves the same page at `/w/<id>/view/` (`docs/kubernetes.md`, "Viewer"). The page
uses relative URLs only (`api/state`, `api/events`, `api/asset/<id>`, `api/interact`), so it works at
`/` and under a workspace prefix. The server fills two more meta slots: `bicraft-view-id` (the
page's private view, sent as `?v=` on every request) and `bicraft-download` (`1`: show a download
link instead of "Open in Power BI Desktop"). Its snapshots add:

- `source.path`: relative to the workspace (never the server's own path);
- `source.download`: `{path, unsaved, reason}`, the saved `.pbix` / `.pbit` / `.bim` file the link
  downloads through `../files/<path>` (`null` with a `reason` for a project, a folder, or nothing
  saved); the page encodes each segment and refuses empty, `.` and `..` segments;
- `view`: `{role: "editor" | "reader", session: "editor" | "copy" | "empty"}`, which session the
  snapshot came from.

The interaction state in `interaction` is the page's own (`Session::snapshot_view`), not the
session's.

### Command states (ROADMAP M11; still version 1; every field optional)

```json
{ "commands": { "disabled": { "edit.undo": "nothing to undo", "page.add": "needs a report" } } }
```

`commands.disabled` lists the engine commands an editor cannot run on the session now, each with
the reason `Session::command_enabled` gives (a missing need, or the server's policy on a
server). The app greys those buttons and shows the reason as their tooltip; it adds the reader
role itself (a reader is refused every command that changes the session). A command absent from
the map is enabled; an app given a snapshot without `commands` falls back on the needs the
snapshot shows.

### Names for the forms (ROADMAP M11; still version 1; every field optional)

The app's parameters forms offer pickers for the objects a command names (`docs/webui.md`,
"Command palette and parameters forms"). Besides the tables, columns, measures, relationships,
pages and visuals above, the snapshot lists the other objects by **name only** (at most 1000 of
each kind):

```json
{
  "model": {
    "tables": [ { "name": "Product", "hierarchies": ["Products"], "partitions": ["Product"] },
                { "name": "Time", "partitions": ["Time"], "calculation_items": ["Current", "YTD"] } ],
    "relationships": [ { "name": "4f1c…", "from_table": "Sales", "…": "…" } ],
    "expressions": ["RootFolder"], "roles": ["Readers"], "cultures": ["en-US", "de-DE"],
    "perspectives": ["Sales view"]
  },
  "report": { "measures": [ { "table": "Sales", "name": "Twice" } ] }
}
```

- `hierarchies` appears only on tables that have some; `calculation_items` only on calculation
  group tables (it marks them, even when empty), in ordinal order (items without an ordinal
  last); `partitions` on every table, and `storage_mode`: `import`, `directQuery`, `dual` or
  `hybrid` (its partitions in different modes: an incremental refresh policy with real time),
  from the partitions' `mode` (`table.set_storage_mode`).
- `synonyms` appears on a table, column or measure bound to an entity of the model culture's
  Q&A schema: its terms that are not `Suggested` or `Deleted` (`model.set_properties`,
  docs/formats/model-objects.md "Synonyms"); `is_key: true` on a table's key column.
- `expressions` are the model's shared queries and parameters, `roles` its security roles,
  `cultures` its translation cultures, `perspectives` its perspectives (the Model explorer);
  `report.measures` the report's own measures (report-level
  measure extensions), with the model table each extends.
- Environment profiles live in a file next to the project, not in the model: the form fetches
  them with `env.list` when it opens.
- `security` (2026-11-19, Manage roles): one entry per role, `{name, model_permission, tables:
  [{table, filter_expression, metadata_permission, columns: [{name, metadata_permission}]}]}`,
  each table permission as the model holds it (`metadata_permission` `none`, `read` or null; the
  row filter or null; `columns` only those with a column permission).
- `perspective_members` (2026-11-19, Manage perspectives): one entry per perspective, `{name,
  tables: [{name, columns, measures, hierarchies}]}` with the member names; a table the file
  marks `includeAll` lists every member it has now.
- Incremental refresh (2026-11-19, the table's menu › Incremental refresh): each table's
  `refresh_policy`, only when it has one, `{store: {n, unit}, refresh: {n, unit},
  only_complete_periods, realtime, detect_changes, polling_expression}` (`detect_changes`: the
  column BIcraft's polling expression names, else null), and `range_filtered` (its query filters
  on `RangeStart` and `RangeEnd`; false while the model lacks either); the model's
  `incremental_refresh` `{parameters, problems}` (whether both are Date/Time parameters, and what
  is wrong).

**Producer**: `crates/engine/src/snapshot.rs` (`table_json`, `model_json`, `report_json`,
`security_json`, `perspective_members`), and `crates/engine/src/refresh_policy.rs` (`summary`,
`range_filtered`, `parameters`).

### Auto-recovery (still version 1; every field optional)

```json
{ "recovery": { "enabled": true, "journaled": true, "edits": 3, "copy": false, "error": null,
                "offers": [ { "id": "…-…", "path": "Sales.pbip", "label": "Sales.pbip", "edits": 12,
                              "copy": false, "at_ms": 1793000000000, "created_ms": 1793000000000,
                              "base": "same", "recoverable": true, "why": null } ] } }
```

`recovery` is `null` when the session keeps no journal. `edits` counts the edits this session's
journal holds since the last save (`copy`: it has a recovery copy; `error`: the last journal write
failed); `offers` are the edits other sessions left unsaved for the open file, which the app
offers to recover ([recovery.md](recovery.md)). A document view's `view.limits` adds `recovery`
(the server keeps journals) and `autosave` (it allows automatic saves).

### Field panes (2026-11-05; still version 1; every field optional)

The app's Build and Data panes (`crates/ui/src/panes.rs`, docs/design.md §12) read:

```json
{
  "model": { "tables": [ {
    "columns": [ { "name": "Amount", "display_folder": "Money" | null, "summarize_by": "sum" | null, "…": "…" } ],
    "hierarchy_levels": [ { "name": "Products", "hidden": false, "display_folder": null,
                            "levels": [ { "name": "Category", "column": "Category" } ] } ]
  } ] },
  "report": {
    "visual_types": [ { "name": "clusteredColumnChart", "display_name": "Clustered column chart" } ],
    "pages": [ { "visuals": [ {
      "roles": [ { "name": "Category", "kind": "grouping|measure|grouping_or_measure", "max": null, "drill": true } ]
    } ] } ]
  }
}
```

- `hierarchy_levels` sits next to `hierarchies` (names only, for the forms) on tables that have
  hierarchies, levels in ordinal order (at most 64 per hierarchy).
- `roles` are the data roles of the visual's built-in type, in the catalog's order (the wells),
  from `bicraft_visuals` (`docs/formats/visual-roles-bb.md`); absent for groups, custom visuals
  and types whose roles are not known.
- `visual_types` are the built-in types whose roles are known and that take fields (the type
  picker).

**Producer**: `crates/engine/src/snapshot.rs` (`hierarchy_levels`, `visual_roles`, `visual_types`).

### Filters pane (2026-11-05, M11.13; still version 1; every field optional)

The app's Filters pane (`crates/ui/src/filters_pane.rs`) reads the filter cards of the report
(filters on all pages), of each page and of each visual, and the new visual and Data pane fields
read these additions:

```json
{
  "model": { "tables": [ { "columns": [ { "name": "City", "data_category": "City" | null } ] } ] },
  "report": {
    "filters": [ CARD ],
    "pages": [ { "filters": [ CARD ], "visuals": [ {
      "filters": [ CARD ],
      "fields": [ { "role": "Rows", "kind": "hierarchy_level", "table": "Sales", "hierarchy": "Geo",
                    "name": "Region", "text": "…", "label": "Region" },
                  { "role": "Y", "kind": "measure", "name": "Total", "display_name": "Revenue",
                    "label": "Revenue" } ]
    } ] } ]
  }
}
```

where a `CARD` is

```json
{ "name": "a1b2…", "field": "Sales[Category]", "kind": "column|measure|aggregation|other",
  "table": "Sales", "column": "Category", "function": null,
  "type": "basic|advanced|topn|relative|none|other",
  "condition": { "in": ["Toys"] } | { "not_in": […] } | { "op": ">", "value": 10 }
             | { "conditions": [ { "op": "contains", "value": "a" }, … ], "logic": "and|or" }
             | { "top": 3, "by": "[Total]", "direction": "top|bottom" }
             | { "relative": { "range": "last|next|this", "count": 3, "unit": "days|weeks|calendar_weeks|months|calendar_months|years|calendar_years|minutes|hours", "include_today": true } }
             | null,
  "how_created": "User|Auto|null", "hidden": false, "locked": false }
```

- Cards are the `filterConfig.filters` of the report, the page or the visual, in the Filters
  pane's order, at most 100 per level: A to Z by display name unless the level's `filter_sort`
  (2026-11-19: `ascending`, `descending` or `custom`, the `filterSortOrder`; `custom` follows
  the cards' `ordinal`, `filter.order`, docs/formats/pbir.md "Filters pane formatting and card
  order"). A measure card has `measure` instead of `column`.
- `type` `none` is a card without a condition (Desktop's "is (All)"): a field dropped on the
  pane, or a condition cleared with `*.clear_filters {keep_cards: true}`. `other` is a condition
  BIcraft does not read (relative dates, tuples…), kept as written; its `condition` is `null`.
- `condition` has the shape of `visual.filter`'s parameters, so a client edits a card by
  sending it back with its changes (`visual.filter`, `page.filter`, `report.filter`).
- `fields[].display_name` is the field's name for this visual only (`visual.rename_field`, the
  projection's `displayName`); `label` already uses it. `kind: hierarchy_level` is a level of a
  user hierarchy (`visual.bind_hierarchy`); its `text` names no field a command can resolve, so
  clients address it by its position in its role (`index`).
- `fields[].show_as` (`percent_of_grand_total`, `percent_of_column_total`,
  `percent_of_row_total`; absent: the value) is the field's "Show value as"
  (`visual.show_value_as`, docs/formats/pbir.md); `fields[].show_all: true` says the field's role
  shows its items with no data (`visual.show_items_no_data`, the role's `showAll`). The data
  then holds the percentages (fractions: 0.25 for 25 %) in the field's cells, totals and
  subtotals, and its `columns[]` entry carries `format_string: "0.00%"`, `show_as`, and
  Desktop's name (`%GT Total`, `%CT …`, `%RT …`) unless the field is renamed; when the totals
  cannot be evaluated the values stay as they are and `show_as_reason` says why. Rows of a
  show-all role's categories without data have blank values; when they cannot be listed,
  `filters_ignored_reason` says why.
- `data_category` is the column's TMSL `dataCategory` (the new visual a geography column makes
  is a map).
- `pages[].filter_pane_format` (2026-11-19, absent when neither the page nor the theme formats
  the pane): `{pane: {background_color, transparency, foreground_color, font_family, border,
  border_color, input_box_color, title_size, header_size, search_text_size, checkbox_color,
  width}, available: {…}, applied: {…}}` (the cards' keys: `background_color`, `transparency`,
  `foreground_color`, `font_family`, `text_size`, `border`, `border_color`, `input_box_color`),
  each only when set, resolved from the theme then the page (`page.format_filter_pane`,
  docs/formats/pbir.md "Filters pane formatting and card order"). The Filters pane draws with
  them, the app's tokens elsewhere.

**Producer**: `crates/engine/src/snapshot.rs`, `crates/engine/src/filter_edit.rs` (`cards`),
`crates/engine/src/visual_props.rs` (`card_json`).

### Model translations (M10.4; still version 1; every field optional)

A model's cultures can translate the captions of its tables, columns, measures, hierarchies and
levels (`translation.set`). Desktop shows them to users whose locale matches a culture. The
snapshot shows them for the session's **display culture** (`view.set_culture {culture}`,
`report.render {culture}` for one call):

```json
{
  "display_culture": "fr-FR" | null,
  "model": { "tables": [ { "name": "Sales", "caption": "Ventes",
                           "columns": [ { "name": "City", "caption": "Ville" } ],
                           "measures": [ { "name": "Total", "caption": "Total FR", "display_folder": "…" } ] } ] },
  "report": { "pages": [ { "visuals": [ {
    "auto_title": "Total FR par Ville",
    "fields": [ { "name": "City", "caption": "Ville", "label": "Ville" } ],
    "data": { "columns": [ { "name": "Sum of Amount", "caption": "Sum of Montant" } ] }
  } ] } ] }
}
```

- `caption` appears only on translated objects, fields and data columns; `name` stays the model
  name (data lookups, formats and DAX use it). Consumers label with `caption` when present.
- `fields[].label` and `auto_title` (M10.2) are built from the captions, so the auto title is
  the translated object names joined with the UI language's phrases. A field the visual renames
  keeps its rename.
- `data.columns[].caption`: the column's name with its field's name replaced by the caption.
- A display culture the model does not have (removed since) shows names.

**Producer**: `crates/engine/src/translations.rs` (`Captions::apply_snapshot`,
`translate_visual`).

Rules:
- `revision` increases by one after every command that changed the session (model, report, source, dirty flags).
- Snapshots are capped (the producer truncates huge lists and sets `"truncated": true` on that list's parent).
- The viewer never sends edit commands; the HTTP server only answers `GET`, except `POST
  /api/open-in-desktop` ("Open in Power BI Desktop" above), which changes nothing in the session,
  and `POST /api/interact` ("Interactions" below), which runs `view.interact`: it changes the
  reader state only (no revision bump, no undo step, nothing saved).

### Key influencers (2026-11-04; still version 1; every field optional)

Every `keyDriversVisual` carries `key_influencers`: what it analyses (`target`, `explain_by`,
`details` as indexes into its `fields`; `target_kind`, `goal`, `class`, `analysis` read from its
`objects.keyDrivers`) and the `mode` (`placeholder`, or `estimate` when
`view.interact {action: "key_influencers", mode: "estimate"}` or `report.render
{key_influencers: "estimate"}` asks for it). In estimate mode it also carries `estimate`,
BIcraft's own analysis of the data (influencers, detail bars, top segments; `status` +
`reason` when there is none). The estimate is never Power BI's result: renderers badge it.
Fields, method and thresholds: [key-influencers.md](key-influencers.md). Its `data` stays
`unsupported` (the visual is not a grouped query).

### Interactions (M8.3; still version 1; every field optional)

`bicraft view PATH` serves the page as a report reader (`--read-only` turns it off; `mcp --view`
stays read-only). What a click does is decided in Rust (`crates/engine/src/interact.rs`); the page
only says what was clicked and draws the next snapshot.

```
page (JS)   click -> POST /api/interact {action, ...}   (token header, JSON body)
view (L6)   checks, queues it for the front end, waits for its answer (at most 30 s)
cli         Session::execute("view.interact", body) -> publish the new snapshot -> answer
engine (L5) Session::interactions (the reader state) -> snapshot() shows the report under it
```

**State** (`Session::interactions`, `view.interactions` lists it): the page shown (and a Back
history), each slicer's applied selection and its pending one (pages with an "Apply all slicers"
button), the data-point selection of each page (one source visual, one or more points). It is
never written to the model, the report or the files: `file.save` saves what it saved before,
`revision` and the dirty flags do not move, `view.interact {action: "reset"}` drops it. It is
kept across re-reads of the files (unknown pages and visuals are ignored).

**How the snapshot shows it.** Reports are derived from the session's for each snapshot:
- the *data* report, which the evaluator reads: each slicer with its applied selection written
  where Desktop stores it (`objects.general.filter`, plus the `objects.data` range and
  relative-date properties), so every existing filter rule applies (a slicer filters the other
  visuals of its page and the other slicers, never itself; `NoFilter` interactions are honoured);
  the page's data-point selection added as a visual filter (`filterConfig`) of every visual it
  *filters*, so `ALLSELECTED` sees it as an outer filter, as it sees Desktop's cross-filter;
- the *highlight* report (pages with a selection only): the slicers and the visuals the
  selection *highlights*, those with the selection as a visual filter. Its results give each
  highlighted row's part (`data.highlights`); the data report keeps their full values. Desktop
  asks for both in one query (`IGNORE(CALCULATE(m, KEEPFILTERS(<selection>)))`, oracle rule 11);
  BIcraft's DAX evaluator does not take `IGNORE` yet, so the visual is evaluated twice and the two
  results are joined on the grouping values;
- the *display* report, which describes the visuals: the data report with a slicer's pending
  selection shown instead of its applied one.

```json
{
  "interaction": { "seq": 3, "page": "<page name>" | null, "can_go_back": true,
                   "pending_pages": ["<page name>"], "apply_pages": ["<page name>"],
                   "key_influencers": "placeholder|estimate",
                   "bookmark": "<the bookmark applied last>" | null,
                   "focus": { "page": "<page name>", "visual": "<visual name>" } | null,
                   "show_table": false,
                   "view_as": { "roles": ["<role>"], "user": "<name>" | null } | null },
  "report": { "active_page": "<the page shown>", "pages": [ { "visuals": [ {
    "interaction": {
      "selected": { "points": [[{"c": 0, "v": "Bikes"}]], "rows": [2] },   // the source visual
      "filtered_by": "<source visual>", "mode": "filter|highlight",         // the others
      "button": { "kind": "PageNavigation|Back|ApplyAllSlicers|ClearAllSlicers|Drillthrough|WebUrl|Bookmark",
                  "page": "<name>" | null, "url"?: "<link>", "bookmark"?: "<bookmark name>",
                  "conditional"?: true, "destination"?: "<page, bookmark or URL the fx field names>",
                  "enabled": true, "tooltip"?: "<the tooltip shown>" }  // docs/formats/buttons.md
    },
    "data": { "highlights": [[null, 1491], [null, null]] },  // highlighted visuals: one entry per row
    "slicer": { "pending": true }
  } ] } ] }
}
```

- `report.active_page` is the page the reader is on (the stored one until a navigation).
- `selected.points`: each point's grouping values, `c` being a `data.columns` index of that
  visual; `selected.rows`: the `data.rows` a point matches (every value it holds equal). The viewer
  fades the other points of the source visual (opacity 0.4; table rows under a 60 % white veil).
- `filtered_by` / `mode`: the selection reaches the visual; `filter`: its data are filtered;
  `highlight`: its data keep their full values and `data.highlights` holds, row by row, the
  highlighted part of each value column (`null` under grouping columns and where nothing is
  highlighted; `highlights_reason` when it could not be evaluated). The viewer draws the full
  mark at opacity 0.4 and the highlighted part opaque over it (bars and columns, stacked ones in
  each segment, 100 % ones as shares, combo columns, funnel stages); a pie or donut slice keeps
  its size and shows its highlighted share (`highlight / full`) as that share of its area from
  the centre; a treemap tile as that share of its height.
- `button`: what a click on that action button runs; buttons with other actions get no key and
  are not clickable.
- `interaction.focus` / `show_table`: the visual the reader shows in focus mode (alone, filling
  the canvas) and whether its data are shown as a table under it (`view.interact` `focus` and
  `show_table`; [desktop.md](desktop.md), "Focus mode, Show as a table, Sort and Spotlight"). Display state
  only: the data and display reports are not changed, the front end draws it; `null` when no
  visual is focused or the focused one is gone. A page change and `reset` leave it.
- `interaction.spotlight`: the visual in spotlight `{page, visual}` (`view.interact`
  `spotlight`), else `null`. Display state only: the renderer draws the page's other visuals at
  15 % opacity. A page change and `reset` end it; a bookmark sets it.
- The reader's sort (`view.interact` `sort`) is not an `interaction` field: the data report's
  visual takes it as its `query.sortDefinition`, so the visual's `sort` and `data` show it;
  `view.interactions` lists them as `sorts: [{page, visual, sort: [{field, descending}]}]`.

**Actions** (`view.interact {action, ...}`, the body of `POST /api/interact`): `page {page}` (name,
display name or index), `back`, `slicer_toggle {page, visual, value, ctrl?}`, `slicer_select_all`,
`slicer_set {values, inverted?}`, `slicer_clear`, `slicer_range {from?, to?}` (numbers, or ISO
dates for a date column; `null` leaves a side open), `slicer_text {text}`, `slicer_relative
{range: last|next|this, duration, period}`, `select {page, visual, point: [{c, v}] | null,
ctrl?}`, `clear_selection {page}`, `clear_slicers {page}`, `apply_slicers {page}`, `button {page,
visual}`, `bookmark {bookmark}` (name or display name), `focus {page, visual | null}`,
`show_table {page, visual, on?}`, `spotlight {page, visual | null}`, `sort {page, visual, field |
null, direction?}` (the reader's sort: `visual.set_sort`'s field syntax and defaults; `null`
goes back to the report's sort), `reset`. Answer: `{action, changed, state, pending?}`. An unknown action, page or visual,
a measure column in a point or a malformed value is an error (400 on the route) and leaves the
state unchanged.

**Rules.** Source: the interaction oracle recorded in Power BI Desktop 2.160.1151.0
([interaction-oracle.md](interaction-oracle.md), its rule numbers below), and the slicer and
`visualInteractions` rules this file already applies. The corpus' 20 click states are played on
`view.interact` by `cargo test -p xtask interaction` (`click_states_match_the_interactive_viewer`):
**180 / 180 visuals and 12 / 12 slicer tick sets agree** (2026-11-25 recording); its 19 file
states agree too (171 / 171).
- *Slicer items* (rule 8). A plain click selects that item alone (it replaces the selection); a
  plain click on the only selected item clears it; Ctrl+click adds or removes an item (on an
  inverted selection, "Select all" then unticked items, it flips that item); "Select all" clears
  the selection. A stored selection is the starting point. A selected value the other filters
  leave out is still listed and ticked, and still filters (rule 4). Values compare as the filters
  do (text without case, numbers by value; `null` is the blank item). Literals follow the model
  column's type (`'text'`, `12L`, `2.5D`, `2.5M`, `datetime'...'`). Not in the corpus, kept from
  the format pane's names: single select (`objects.selection.strictSingleSelect: true`) always
  selects the one item; "Multi-select with Ctrl" off (`objects.selection.singleSelect: false`)
  makes a plain click toggle.
- *Synced slicers* (rule 6). A click on a slicer of a sync group (`visual.syncGroup.groupName`,
  unless `filterChanges` is off) gives the same selection to every slicer of the group, on every
  page, hidden ones included; the files are not reconciled when the report opens (each copy
  starts from its own stored selection).
- *Range, text and relative-date slicers* (rules 1, 2). A range is `>= from` and `<= to` (for a
  date, `<` the day after `to`, as Desktop writes it), shown from `objects.data` `numericStart` /
  `numericEnd` or `startDate` / `endDate`; a text slicer filters with `Contains` (without case);
  a relative-date slicer stores `relativeRange` / `relativeDuration` / `relativePeriod` and filters
  the days they cover, resolved to literal dates with today (or `--as-of`) when the reader sets
  it, today included. `slicer_clear` removes the bounds.
- *Data points* (rules 9, 12, 13, 14, 15). A click selects the point (a chart's category and
  legend value, a table row's grouping columns, a slice, a treemap tile, a funnel stage, a line
  point); Ctrl+click adds or removes points; a plain click on one of several selected points
  keeps only that one; a plain click on the only selected point clears the selection, as does a
  click on the visual's background. One selection per page: a click in another visual replaces
  it. The source visual is not re-evaluated (its other points fade). Every other visual with
  fields is reached, slicers included, the slicers' filters still applying: with no
  `visualInteractions` entry, a visual that can highlight (the types Desktop 2.160 highlighted in the
  highlight probe, `testsupport/interaction_oracle/highlight_types.json`: bar and column charts,
  clustered, stacked and 100 %, combo, ribbon, pie, donut, funnel, treemap, decomposition tree)
  is highlighted, any other (cards, KPIs, gauges, tables, matrices, line, area, scatter,
  waterfall, slicers, and the types the probe did not settle: maps, Q&A) filtered; `DataFilter` filters;
  `HighlightFilter` highlights when the visual can, else filters; `NoFilter` leaves it alone.
- *Apply all slicers, Clear all slicers* (rules 16, 17). While a page has a visible action button
  of type `ApplyAllSlicers`, slicer clicks on that page wait for it, whatever the storage mode
  (Import included): `slicer.pending`, the page's other visuals keep the applied selection.
  `ClearAllSlicers` clears every slicer of the page, applied and pending, and keeps the
  data-point selection.
- *Navigation.* Page tabs, `pageNavigator` items and `PageNavigation` buttons change the page
  shown (its data are computed first); `Back` returns to the previous one. Each page keeps its own
  slicers and selection; a synced slicer's selection reaches its copies on the other pages.
- *Bookmarks* (G55, `crates/engine/src/interact_bookmark.rs`). `bookmark {bookmark}`, a
  `Bookmark` button (`button`) and a `bookmarkNavigator` item apply a bookmark to the reader's
  view only, as `bookmark.apply` reads it and within its options: its `activeSection` becomes the
  page shown (a navigation: `Back` returns) unless `suppressActiveSection`; each visual of its
  sections takes the bookmark's `display.mode` (hidden or shown) unless `suppressDisplay`; the
  report, page and visual filters it holds replace the stored ones in the data report unless
  `suppressData`, which also clears the data-point selection of its pages (a bookmark BIcraft
  captures holds none). `applyOnlyToTargetVisuals` limits the display and visual filters to
  `targetVisualNames`. Unless `suppressData`, each slicer whose container holds a selection
  (`singleVisual.objects`: `merge.general[0].properties.filter`, `merge.data[0]` range
  properties, or `remove` of `general.filter`) takes it as its applied selection, as a click
  would set it (a pending one is dropped; the answer's `slicers` lists the slicers changed); the
  next click goes on from it. The state keeps them as overrides (`view.interactions`: `hidden`,
  `bookmark_filters`, `slicers`; one equal to the stored report is dropped) and the snapshot shows them
  (`hidden`, the visual's `filters`); `interaction.bookmark` names the last bookmark applied, the
  one a bookmark navigator highlights. A bookmark navigator lists the snapshot's top-level
  `bookmarks` (`bookmark.list`'s answer: each bookmark's `name`, `display_name`, its group's
  display name `group` and name `group_name`) of the group its `navigator.bookmark_group` names
  (a group's name, or its display name), or all of them. Pages a bookmark names that no longer exist are listed in
  `warnings`. Not applied: drill states and cross-highlights (`highlight`) a Desktop bookmark
  captures, other `singleVisual` changes (formatting, sort, projections), Desktop's "Allow
  deselection" (`allowDeselectionBookmark`). Documented
  from the bookmark schema and Microsoft Learn's description of the options, not measured in
  Desktop.

**Tooltips.** Hovering a data point shows its category, legend value and value (a table row: its
cells), formatted like the visual, drawn by the page from the snapshot whether interactions are
on or off.

**Route** (`crates/view`): `POST /api/interact`, enabled by `ViewOptions::interactions`. Checks, in
order: localhost `Host` (403), the method (405), enabled (404), the per-run token (`<meta
name="bicraft-interact-token">`, header `X-Bicraft-Token`, compared in constant time), `Origin`
equal to `http://<Host>` and `Sec-Fetch-Site: same-origin` when present (403), `Content-Type:
application/json` (415: a cross-site form cannot send it without a CORS preflight, which the
server refuses), `Content-Length` (411; chunked bodies are refused), at most 64 KiB (413), a JSON
object (400), a free slot in the front end's queue of 16 (503). The connection waits up to 30 s
for the answer (503 after): `{ok: true, result}` or `{ok: false, error}` (400). The server
listens on 127.0.0.1 only and never acts on the body itself.

**Edit interactions** (2026-11-06, ROADMAP M11.15; every field optional). What the app's Edit
interactions mode (docs/design.md "Edit interactions") reads, so its icons and the viewer follow
one table (`crates/engine/src/interact.rs`):
- each visual's `interactivity`: `{target, highlight, slicer}`: `target` a visual with fields
  (it can be a source or a target), `highlight` its type can highlight (the table of oracle rule 9, from the
  highlight probe, the one the viewer uses), `slicer` a slicer (its selection only
  filters);
- each page's `interactions`: its `visualInteractions` as `[{source, target, type}]` in
  `page.interactions`' words (`filter`, `highlight`, `none`);
- `report.settings`: `{default_interaction: "highlight"|"filter", drill_filters_other_visuals,
  export_data_mode}` from `report.json` `settings` (`defaultFilterActionIsDataFilter`,
  `defaultDrillFilterOtherVisuals`, `exportDataMode`; the `report.settings` command sets them);
  `report.export_data_mode` repeats the last (`AllowSummarized` when the report has none): what a
  visual's Export data may write (`visual.export_data`, M11.21). Also `persistent_filters`,
  `personalize_visuals`, `cross_report_drillthrough`, `filter_pane_visible` (whether readers see
  the Filters pane) and `allow_change_filter_types` (G63, `docs/formats/pbir.md` "Report
  settings"); `paused` (Pause visuals, `settings.pauseQueries`) and `query_reduction: {preset:
  "query_reduction"|"interactivity"|"customized", disable_cross_highlighting, slicers:
  "instantly"|"apply_button", filters: "instantly"|"apply_buttons"|"single_button"}`
  (`slowDataSourceSettings`, G60). While paused, a visual's `data` that would need an evaluation
  is the data it last showed with `pending: true`, or `{status: "pending", reason}`.

With `default_interaction: "filter"` (Desktop's report option "Change default visual interaction
from cross highlighting to cross filtering") a data-point selection filters the visuals that
could highlight, unless their rule says `HighlightFilter`; an explicit `HighlightFilter` on a
visual that cannot highlight still filters (rule 14). `drill_filters_other_visuals` is stored and
shown only: the viewer has no drill yet (G29).

**Gaps** ([gaps.md](gaps.md) G29): a highlight costs a second evaluation of the visual (until
BIcraft's DAX evaluator takes `IGNORE`); the pie highlight's drawing (share of the slice's area)
and the highlight of ribbon charts, line series of combos, decomposition trees and Q&A visuals are
not checked against Desktop's drawing; a multi-point selection over columns of several tables
filters each table by its own values (a superset when the points differ on more than one table);
no point selection in matrices, scatter, waterfall and ribbon charts, gauges, KPIs, cards and
small-multiple panels; no bookmarks (drill, drillthrough and report-page tooltips: see "Drill,
drillthrough and page tooltips");
hierarchy slicers select on their first column; `strictSingleSelect` / `singleSelect` are not in
the oracle corpus; `mcp --view` is read-only.

### View as (2026-11-18, row-level security; still version 1; every field optional)

`view.interact {action: "view_as", roles: [names] | null, user?: string}` is Power BI Desktop's
Modeling › View as (the app: Model › Security › View as, a dialog with None, Other user and its
name, a check box per role, OK; a banner "Now viewing as: …" with Stop viewing while it is on).
It is view state like the other interactions: the model, the report, the revision and the
unsaved-changes flags never change, `reset` clears it, and `interaction.view_as` (also in
`view.interactions`) shows it (`null`: off). A role the model does not have is an error and
nothing changes; `roles: null` (or no role) without a user stops viewing; role names take the
model's spelling.

Under View as every evaluation reads the data through the roles' row filters: the visuals,
slicers and cards of the snapshot (and `report.render`), `dax.evaluate`, `table.rows`,
`visual.records`, `filter.column_values`, a slicer's range. The edit-time checks
(`relationship.set` cardinalities, date tables) read every row. Rules
(`crates/engine/src/preview_eval_rls.rs`, from Microsoft Learn's row-level security pages and the
tabular model's `TablePermission.FilterExpression` / `securityFilteringBehavior`):

- each table filter is a DAX boolean evaluated in the row context of each row of its table, with
  no filter context; `USERPRINCIPALNAME()` and `USERNAME()` return the "Other user" (empty text
  without one; outside View as they stay unsupported: there is no signed-in user);
- the rows kept filter the many side of every active relationship whose one side they are on,
  and the one side back from the many side when the relationship's `securityFilteringBehavior`
  is `bothDirections` (`none`: no security filter crosses it); a filtered table loses its rows
  whose key matches no kept row;
- several roles: the rows each role keeps are united per table; a role that filters nothing (or
  an `administrator` role) keeps every row, a `none` role keeps none; "Other user" with no role
  applies no filter;
- the rows are removed before any query, so `ALL`, `REMOVEFILTERS` and a relationship's blank
  row never bring the others back. Evaluations run on a state of their own (its own caches, no
  report-derived aggregations) that never merges into the session's;
- a filter BIcraft cannot evaluate fails every evaluation under View as with its reason (never
  the unfiltered data).

On a server each viewer has its own View as (`snapshot_view` evaluates under that viewer's
state, never the session's). A reader may use it: the server applies no model role to a reader
(a reader already reads every row through `dax.evaluate`), so View as only narrows what that
reader sees and never widens it. If the server ever maps callers to model roles, View as must
stay within the caller's own roles (an intersection), never replace them.

Object-level security (2026-11-19): a table or column that every chosen role hides
(`metadataPermission: none`, `role.set_permission`) does not exist under View as: reading it
fails with "cannot find table 'T' (object-level security)" or "cannot find column 'T'[C]
(object-level security)", so a visual or measure that uses it shows that error. Roles that
filter rows and roles that secure objects chosen together fail every read, as the engine refuses
that mix at query time (docs/formats/model-objects.md, "Object-level security").

Not checked against Analysis Services: whether a filter that reads another secured table sees it
filtered (here it reads every row), a one-to-one relationship (treated like one-to-many), and
the blank row of a table whose rows are all kept. Under a memory budget the secured columns are
copies of the kept rows, counted with the state's own columns.

### Mobile layout (2026-11-19; still version 1; every field optional)

A visual placed on its page's mobile (phone) layout has `mobile` (absent otherwise):

```json
"mobile": {"x": 0, "y": 208, "z": 1, "width": 323, "height": 240, "formatted": true,
           "format": { … }, "title": "Sales"}
```

`x`, `y`, `z`, `width` and `height` are the phone canvas's (`mobile.json` `position`, or a
legacy layout's container layout id 1); `formatted` says whether the mobile document holds
formatting (`objects`, `visualContainerObjects`). When it does, `format` and `title` are the
visual's `format` and `title` as the phone draws them: the visual's own formatting with the
mobile formatting over it (docs/formats/pbir.md, "Mobile layout"). The app's phone canvas
(`crates/ui/src/mobile.rs`) draws a derived page from them.

### Decomposition trees (2026-11-04; still version 1; every field optional)

A `decompositionTreeVisual` is evaluated level by level (`crates/engine/src/preview_decomp.rs`;
the rules: [formats/decomposition-tree-bb.md](formats/decomposition-tree-bb.md)). Its `data`:

```json
{ "status": "ok",
  "columns": [ {"name": "Total", "role": "Analyze", "kind": "measure"}, {"name": "Region", "role": "ExplainBy", "kind": "column"}, … ],
  "rows": [ [593333, null, …], [41223, "Central", null, …], … ],
  "decomp": {
    "analyze": 0, "root": 593333,
    "levels": [ { "column": 1, "ai": null | "high" | "low",
                  "nodes": [["Central", 41223], ["East", 119903], …],
                  "total": 5, "selected": 2 | null, "auto": false, "start": 3 | null } ],
    "shift": 0,
    "options": { "tree": {"density": "dense", "barsPerLevel": 6}, "dataBars": {…}, … } } }
```

- `columns`: one per field of the visual, in field order (a point's `c` is an index into them).
- `rows`: the root (every grouping cell `null`), then each level's members in order, each with
  the path above it in its ancestors' columns; a highlighted tree's `highlights` follow these
  rows (a blank member of the first level shares the root's key: its highlight may show the
  root's).
- `decomp.levels`: the levels shown, in order; `column` is the level's field (an AI level: the
  field chosen for the current path); `nodes` are `[member, measure]`, sorted, at most 1000;
  `selected` is the member on the path (`auto`: chosen by the AI split, not saved); a level
  without a selected member is the last one; `start` is the reader's scroll position (the
  first member shown; `null`: the window Desktop opens with).
- `decomp.shift`: the levels the reader scrolled out on the left of a tree wider than the visual.
- `decomp.options`: the visual's `tree`, `dataBars`, `dataLabels`, `categoryLabels`,
  `levelHeader`, `analysis` properties (literals decoded, colours as `#rrggbb`).

**Reader actions** (`view.interact`, never saved): `decomp_path {page, visual, level, value,
path?}` selects member `value` of level `level` (1 = under the root; `path`: the members above
as shown); `decomp_expand {page, visual, level, value, column | ai: "high" | "low", path?,
columns?}` adds a level under that member (`level` 0: under the root; `columns`: the fields of the
levels above, which fixes AI levels as Desktop does); `decomp_collapse {page, visual, level}`
removes a level and the ones below; `decomp_scroll {page, visual, level, start}` shows that
level's members from `start` on (its scroll arrows send the window before or after the one
shown) and `decomp_scroll {page, visual, shift}` scrolls the tree by levels (its side arrows); a
new path member or level drops the scroll positions of the levels below it. The changed tree is in `view.interactions`' `trees`; `reset`
drops it. A bar on the selected path is a data point: `select` with `[{c, v}]` for each level
down to it cross-filters (highlights) the page's other visuals.

### Drill, drillthrough and page tooltips (2026-11-06; still version 1; every field optional)

The reader drills, expands a matrix, drills through and hovers tooltip pages through
`view.interact` (`crates/engine/src/interact_drill.rs`); like slicers, the state lives in the
session or the viewer's private view state and never reaches the files on its own. It is applied
to the data report with the fields Power BI Desktop saves for a drilled visual, so the evaluator
reads a drilled visual as it reads a saved one: the drill role's projections get `active` flags
for the displayed levels, the values drilled into become `filterConfig` containers with
`howCreated: "Drill"`, a matrix's expansion becomes its `expansionStates` entry, a drillthrough
page's `howCreated: "Drillthrough"` filters get the point's values. `view.keep_drill` writes the
same fields into the report (one edit, undoable, Desktop's filter names) and drops the reader
state of those visuals: Desktop saves its drill state with the file, BIcraft when asked.

Behaviour from Microsoft Learn ("Drill mode in a visual", "Set up drillthrough", "Create report
tooltips", read 2026-11-06); not recorded in Desktop yet (**documented, not measured**).

**Per visual** (`report.pages[].visuals[]`):

```json
{ "drill": { "role": "Category", "levels": ["Category", "Product"], "top": 0, "bottom": 0,
             "mode": false, "can_up": false, "can_down": true,
             "path": [{"level": 0, "value": "Wheels"}], "matrix": false,
             "expansion": {"open": [false], "toggled": [["Wheels"]]} | null },
  "interaction": { "drillthrough": [{"page": "…", "label": "Detail", "columns": [0], "cross_report": false}],
                   "tooltip_page": "…" } }
```

- `drill`: a visual whose first drill role (`drilldown.roles`: chart axes, matrix rows) holds two
  or more columns or hierarchy levels. `top` / `bottom`: the displayed levels (`bottom > top`:
  expanded down); `path`: the values drilled into above `top`; `mode`: drill mode is on;
  `expansion`: a matrix's expansion as the reader set it (`null`: the saved one). Without a
  reader state the visual's saved state is described (`active` flags, `Drill` filters).
- `interaction.drillthrough`: the drillthrough pages a point of the visual reaches (a page one of
  whose drillthrough fields is a displayed column of the visual), with the data columns that feed
  them; `cross_report` pages are listed but refused.
- `interaction.tooltip_page`: the visual's report-page tooltip: the page its `visualTooltip`
  names (`type: Canvas`), else ("Auto") the first tooltip page one of whose tooltip fields the
  visual uses (a column as a column, a summarized field summarized).
- `interaction.tooltip` (top level): `{page, visual, point, target}` of the tooltip page being
  shown, `null` when none; the target page's visuals are evaluated under the point.
- A matrix's `format.table.expand_buttons`: its "+/- icons" (`rowHeaders.showExpandCollapseButtons`).

**Actions** (`view.interact`):

| Action | Parameters | What it does |
|---|---|---|
| `drill_down` | `page, visual, point, role?` | the displayed levels' values of the point become drill filters; the next level is shown |
| `drill_up` | `page, visual, role?` | undoes the last expansion, else the last level drilled into |
| `drill_next` | `page, visual, role?` | "Go to the next level": every point moves down one level, no filter |
| `drill_expand` | `page, visual, role?` | "Expand all down one level": the next level is added under the shown ones (charts group by both, labels concatenated) |
| `drill_mode` | `page, visual, on?` | toggles drill mode: a `select` on a point of the visual drills into it (answer `drilled: true`); at the lowest level it selects |
| `matrix_expand` / `matrix_collapse` | `page, visual, path: [values] \| level: n \| all: true` | a member (+/−, "Selection"; `path` from the top level shown), an entire level (0: the top one shown), or everything; expanding the deepest level shown adds the next one |
| `drillthrough` | `page, visual, point, target` | opens `target` with its drillthrough filters set from the point; "Keep all filters" (unless `acceptsFilterContext: None`) also carries the source visual's page filters, own filters (drill and cross-filter included) and the selections of the slicers that reach it; the source page goes in the Back history |
| `tree_toggle` | `page, visual, id` | a BIcraft Org Chart node's collapse button: the reader's list of toggled ids (the stored `tree.toggled`, the id removed or added last) replaces the stored one in this view, the report unchanged; back to the stored list, the override is dropped; `reset` drops it. The app's editing view saves it instead (`visual.toggle_node`, [formats/chart-pack.md](formats/chart-pack.md)) |
| `tooltip` | `page, visual, point \| null` | shows the visual's tooltip page under the point (a line chart's legend value excluded) and the source visual's filters; `null` hides it |

A `visualLink` button of type `Drillthrough` (`drillthroughSection`) drills through the page's
selection (one point; refused without one, as Desktop greys the button). Drilling filters the
page's other visuals when the drilled visual's `drillFilterOtherVisuals` says so (Desktop's "Apply
drill down filters to: Entire page", `visual.drill_filter`), else when the report's
`settings.defaultDrillFilterOtherVisuals` is on ("Drilling filters other visuals",
`report.settings`; absent: off), within the page's `visualInteractions` (`NoFilter` stops it). A drill
action drops the visual's own data-point selection (its points name levels no longer shown).
`view.interactions` adds `drills`, `drillthrough` (per page: source, values, `kept_filters`) and
`tooltip`. Not done: cross-report drillthrough, summarized (`asAggregation`) drillthrough fields,
drill on a matrix's Columns from the canvas (the actions take `role: "Columns"`), drilling the
selection with the header's ⇓ while drill mode is off (Desktop's "expand only the selected
point").
