# Visual roles: source log

The public documentation read and the black-box tests run to establish the per-visual-type data
roles. The facts and their tags are in [visual-roles-bb.md](visual-roles-bb.md).

Desktop opens reports BIcraft generates and the rendered page is observed (DevTools port of its
WebView2, the render-oracle convention of `tools/render-oracle`). Every test in the first table
ran on Power BI Desktop 2.159.883.0.

## Roles of the built-in visuals (2026-10-28)

| Time | Kind | What | Use |
|---|---|---|---|
| 13:34 | public doc | <https://learn.microsoft.com/javascript/api/overview/powerbi/get-visual-capabilities> | `clusteredColumnChart` roles `Category`, `Series`, `Y`, `Rows`, `Tooltips` with kinds |
| 13:34 | public doc | <https://learn.microsoft.com/javascript/api/overview/powerbi/data-fields> | role names are the binding keys; no counts |
| 13:35 | public doc | <https://learn.microsoft.com/javascript/api/overview/powerbi/create-add-visual> | the list of out-of-the-box visual type names |
| 13:38 | black-box | exploration session on `pilot`: DOM of Desktop's rendered UI read through the DevTools port (visible text, element classes and ARIA labels of the field wells); mouse click and drag events (`Input.dispatchMouseEvent`, `Input.dispatchDragEvent`) on Desktop's own Data pane, field wells and Save button; the PBIX Desktop saved, read back | method: a field dragged into a well is saved under that well's role name. The DevTools port sends input events and reads the rendered DOM |
| 13:36 | black-box | test set `pilot` (12 `clusteredColumnChart` visuals), Desktop 2.159.883.0 | states: drawn, blank, placeholder, error; unknown role names are ignored |
| 13:46 | black-box | wells-A: 48 visual types (every `visual-*` type of the vendored theme schema), one empty visual each; text column per well; Desktop saved the PBIX | role names, kinds (column kept or summarized) |
| 14:00 | black-box | wells-B: the same with one measure per well | which wells keep a measure |
| 14:13 | black-box | wells-C: 5 text columns per well, one well at a time, visual header read | maxima, drill-down roles, wells that stop taking several fields |
| 15:06 | black-box | wells-R: wells-A in reverse well order, 39 types with wells; Desktop saved the PBIX | the legend role of line and area charts; which wells refuse a field once others are filled |
| 17:00 | black-box | test set `kinds` (48 visuals on a model with seven related tables): scatter X/Y kinds, map kinds, slicer fields across relationships, a few no-well and count cases | scatter summarization rule; slicer relationship-chain rule; maps cannot be tested here (map visuals are switched off in Desktop's security options, Azure Maps needs a sign-in; settings left unchanged) |
| 17:15 | files Desktop saved | `active` flags in the PBIX files of wells-A/B/R | the drill-down roles: Desktop flags the first field of each hierarchy role |
| 17:18-18:21 | black-box | test set `render`, 4 batches of 26 pages (1229 visuals), Desktop version logged per batch: 2.159.883.0 | maxima confirmed (every over-max case is an error naming the well), combinations, roles a visual needs to draw (screenshots of the single-role and pair pages) |

## Gantt, maps (2026-11-23)

Power BI Desktop 2.159.1231.0 (Store edition). The DevTools port of Desktop's WebView2 sends
mouse and drag events and reads the rendered DOM (visible text, classes, ARIA labels, inline
colours of the Format pane's swatches).

| Time | Kind | What | Use |
|---|---|---|---|
| 12:32 | build | BIcraft CLI: a PBIX with a synthetic model (text `T1`…`T10`, dates `D1`/`D2`, a number, measures `M1`…`M10`) and an empty `ganttChart` visual | test input |
| 12:34 | black-box | opened in Desktop; the Visualizations gallery lists "Diagramme de Gantt (préversion)"; no preview option changed; the Gantt gallery button clicked to insert one more empty Gantt | the type is available in this Desktop |
| 12:35 | black-box | gantt-A: `T<i>` dragged into well *i* (a date column, then a date or other measure, where the text column was refused); Save clicked; the PBIX read back | role names `Task`, `StartDate`, `EndDate`, `Progress`, `Category`, `Columns`, `Tooltips`; how a column is kept or summarized |
| 12:37 | black-box | gantt-B: measures per well, on an empty Gantt, then with `Task` filled | which wells keep a measure; no well takes a field before `Task` |
| 12:40 | black-box | second PBIX with date measures `MD1`/`MD2` and a percentage measure; gantt-R (reverse order), gantt-C (up to five fields per well with `Task` bound by BIcraft), Save; the PBIX read back | date measures kept in Start/End; maxima (Category 3); no `active` flag; a BIcraft-bound `Task` shows in its well |
| 12:43 | black-box | maps PBIX made with the CLI (shape map with `shape_map.set_map {builtin: "world_countries"}`, Location and Color saturation; a filled map; a map with Latitude, Longitude and Size): pages shown, screenshots, the shape map's Format pane read (Map settings, Colors), Save clicked and the PBIX read back | docs/formats/maps.md, "Checked in Desktop" |
