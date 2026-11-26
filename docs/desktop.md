# The desktop app (`bicraft-desktop`)

`bicraft-desktop` is a native report reader for Windows and Linux. It opens a `.pbix`, `.pbit`,
`.pbip` (or a project folder) or `.bim`, or connects to a BIcraft server, and shows the report
the way the live viewer does: same drawing, same slicers and cross-filtering, same results. It
uses no web engine: no browser, no Edge, no WebView2. The window is [egui](https://github.com/emilk/egui)
over wgpu (DirectX 12 on Windows, Vulkan or OpenGL on Linux), like DeckCraft. The pages are drawn
by `bicraft-render`, the pure-Rust port of the viewer's drawing that `bicraft render` uses
([render.md](render.md)).

The UI itself lives in `crates/ui` (`bicraft-ui`), shared with the browser app
([webui.md](webui.md)); `bicraft-desktop` is its native shell: the worker-thread sources (an
engine session for local files, the blocking HTTPS client for servers), the Open dialog, the
settings file, `--register` and the icon.

Clicks on a page change the reader state only (`view.interact`). The page tabs and the File view
edit and save through engine commands (`page.*`, `file.save`, `file.save_as`, `file.convert`,
`report.render`, `file.diff`, `project.new`; `crates/ui/src/actions.rs` lists them): on the
desktop's engine session for a local file, through the workspace's MCP endpoint
(`run_command`, editors only) on a server. Everything else stays in the CLI and MCP.

```text
bicraft-desktop                                      # the start screen: Open, Connect to server
bicraft-desktop Sales.pbix                           # a file (or Sales.pbip, a project folder, model.bim)
bicraft-desktop Sales.pbip --lang fr                 # French UI and French generated text in visuals
bicraft-desktop --server https://bi.example.com --workspace sales   # a server (token below)
bicraft-desktop --register                           # Windows: Start menu entry and Open with (per user)
bicraft-desktop --unregister
```

## The window

The window follows DeckCraft's design system with BIcraft's amber accent ([design.md](design.md)):
a title row, a tab row over a ribbon card, the page on its canvas with the page tabs, a status bar.
Light by default; dark from the title row's moon (or Home > Dark), kept in the settings.

- **Title row**: the BIcraft mark and Reload (re-reads the file; F5 too when the file is neither a
  PBIP project nor a PBIX, else F5 is the model's Refresh); in the middle the file and its pills: the
  format (`PBIX`, `PBIP`…), `rev N`, and `model` / `report` with an amber dot when they have
  unsaved edits; at the right the search button (the command palette) and the light/dark toggle.
- **The ribbon is the command registry** (ROADMAP M11, [commands-ui.md](commands-ui.md)): the tab
  row has File, Home, Insert, Report, Model, Data, Analyze and View, then the contextual **Visual** (a
  visual selected on the canvas), **Table** (a table or column selected in the Model view) and
  **Measure** (a measure selected) tabs. Every engine command the registry places on a tab is a
  button of its group there, with its label in English or French, its code-drawn icon and a
  tooltip (what it does, its shortcut, its id); a command the session cannot run now is greyed and
  its tooltip says why (the snapshot's `commands.disabled`, the reasons `Session::command_enabled`
  gives, plus the reader role on a server). A click runs the command on the document's source;
  one that needs values opens its parameters form with what the selection gives (the page shown,
  the visual, the table, column or measure); a read command shows its answer in a window: a
  grid for lists of records (a click on a header sorts, Copy as TSV), a key/value tree for
  objects, a JSON toggle with Copy JSON, Close. The registry's shortcuts work outside text fields, dialogs, the palette and forms:
  Ctrl+M New page, Ctrl+Z / Ctrl+Y Undo / Redo, Ctrl+N New model, Ctrl+Shift+S Save as, Delete
  the selected visual (not while a control has the focus), F5 Refresh on a PBIP project or a PBIX (else
  Reload); a table's right-click "Refresh data" (Data pane, model diagram) refreshes that table
  and what reads it, its answer window shows a spinner, then the per-table results.
- **Command palette** (Ctrl+Shift+P): every engine command, searchable in English and French by
  label, id and place, the recent ones first, each with its place (`Model › Relationships`) and
  shortcut, greyed with the same reason as its button. Enter runs it as its button does,
  Shift+Enter opens its **parameters form**: pickers of every object the command names (tables,
  columns, measures, hierarchies, partitions, calculation groups and items, queries, roles,
  cultures, report measures, relationships, pages, visuals, environment profiles), dropdowns,
  checkboxes showing the command's default, checked numbers, DAX and M editors, colour buttons, the Open, Save and folder
  dialogs for paths; `*` marks what is required, Run checks first, and the command's answer or
  error shows in the form. The ribbon's and menus' buttons open the same form when values are
  missing ([webui.md](webui.md), "Command palette and parameters forms").
- **Home**: Open (Ctrl+O; `.pbix .pbit .pbip .bim`, native dialog), Connect to server, Reload,
  Recent files, Recent servers, Quit; Light, Dark and High contrast (the app's theme; the report
  pages follow into and out of high contrast); then Open in Desktop, Get data, New measure,
  Refresh, Undo, Redo and History. A file dropped on the window opens it.
- **Data** (stored data, queries, partitions, sources, environments), **Analyze** (Show
  validation, then storage, DAX, Validate, usage, performance and tests, with the validation
  counts) and **View** (visual text language, translations, as-of date, themes) hold the rest of
  the registry's ribbon commands.
- **View › Themes** opens the gallery of BIcraft's eight built-in report themes (BIcraft, Harbor,
  Orchard, Ember, Slate, Clear sight, Dusk, High contrast), each a swatch of its background, text
  and data colours, the report's theme ringed; a click installs it (`theme.set {builtin}`, a
  custom theme in the file). Below: Browse for themes (`theme.import`), Customize current theme
  (`theme.set_colors`, its form filled with the theme's data colours, background, foreground and
  table accent), Save current theme (`theme.export`), Custom theme JSON (`theme.set`'s form). The
  themes are BIcraft's own (`crates/visuals/themes/`), not Power BI's.
- **View › High-contrast colours** draws the report pages with the high-contrast palette (black
  page, white text, bright series colours; [render.md](render.md), "High contrast"): a view
  option of the window (`report_contrast` in the preferences), the file never changes. Choosing
  the High contrast app theme turns it on, Light or Dark off.
- **Right-click menus** carry the menu-placed commands: a visual (Delete visual, Sort; a click on
  the page selects the topmost visual under the pointer, with a ring, and Escape clears it), a page
  tab (below, plus Open on this page), a model table (Rename, Delete, Delete hierarchy,
  calculation group or item), column, measure, relationship, and the diagram's background
  (Rename and Delete query, Delete role, Delete culture).
- **File** (the first tab, Alt+F): DeckCraft's Backstage over the whole window below the title
  row (Escape or Back returns). **Home**: New (a blank PBIP project, or one made from a `.pbit`
  template; a name, then the folder dialog), Open a file, Connect to server, and the recent files
  with a search box: format badge, name over folder (split as text, nothing is read from disk),
  the date last opened, Pin, Remove from Recent. **Open**: Recent, Browse (the Open dialog),
  Servers (the remembered ones, Connect). **Info**: format, size (a package's parts), revision,
  tables, columns, measures, pages, visuals, the validation summary and the unsaved state, with
  Check the file and Save, then New model, Import from a file and Licences (the registry's other
  File commands). **Save** (Ctrl+S; a file never saved goes to Save As), **Save As**,
  **Export**: Save a copy as, Convert to PBIX / PBIT / PBIP (the saved file; the open one is not
  touched), Render the page to PNG, Compare with another file (the diff as text), each with the
  Save or Open dialog and its output (what was written, warnings). **Close** asks first when the
  file has unsaved edits. With nothing open the File view's Home is the start screen.
- **Document tabs** under the title row: several files at once (at most 8; each one is an engine
  session with its model and data, so the cap bounds the memory). The file name, an amber dot with
  unsaved edits, × (or a middle click) to close, `+` for the File view, Ctrl+Tab and
  Ctrl+Shift+Tab to switch; opening a file already open switches to its tab. Background files keep
  their source (watching the file) and their view state.
- **Report** (tab and ribbon): Previous and Next page. The page is scaled by its View › **Page
  view** (`page.set_display_option`, saved with the page): Fit to page (the whole page in the
  window), Fit to width (the page as wide as the window, scrolling down) or Actual size (100 %:
  one report pixel per point, scrolled both ways; the sharpest text when a large page is shown on
  a 100 % screen). The status bar's zoom overrides it for the window only: Fit page, Fit width,
  Actual size, Zoom out, a 10–400 % slider, Zoom in and the percentage. The page tabs at the bottom are the only page navigator (one place
  for pages, as in Power BI Desktop); the page on the canvas with a shadow.
  The page is rendered at the size it is shown and at the screen's DPI, so it stays sharp at any
  size. A new render runs on a background thread when the window is resized or the snapshot
  changes; until it is ready the last image stands in, scaled. Rendered pages are cached by
  snapshot, page and scale (8 at most). The image lands on whole device pixels at its own size (no resampling), and horizontal text sits on a whole pixel row.
  Below 1.75 device pixels per report pixel (a fitted page on a 100 % or 125 % screen) the
  page's upright text on opaque backgrounds is drawn with subpixel (ClearType-like, RGB)
  antialiasing, as Power BI Desktop's canvas draws it; at higher DPI it stays grayscale
  (`pages.rs`, docs/render.md "Subpixel text"). The app's own UI text is egui's (grayscale).
- **Build and Data panes** at the right of the Report view, as Desktop's Visualizations and Data
  panes (`crates/ui/src/panes.rs`). **Data** lists the model's tables with their columns,
  measures (calculator icon), hierarchies and their levels, grouped by display folder; a search
  box filters it, the eye toggle shows hidden objects (dimmed). **Build** shows, as Desktop's
  Visualizations pane, the gallery of built-in types (fixed 30 × 28 cells, as many a row as the pane holds, each a small picture of
  the visual with its type's name as the tooltip, the common types first, scrolling above the
  wells): a click sets the
  selected visual's type (`visual.set_type`, its fields moved to the new type's roles) or adds a
  visual of that type (`visual.add`) when none is selected. Under it, one well per data role of
  the type (X-axis, Y-axis, Legend, Small multiples, Tooltips, Columns,
  Rows, Values…), each listing its fields with the aggregation of a column (Don't summarize, Sum,
  Average, Min, Max, Count…: `visual.set_aggregation`) and × (`visual.unbind`). **Drag and
  drop**: a field onto a well binds it there at the drop position (`visual.bind {role, index}`; a
  number column is summarized as its `summarize_by` says, Sum by default); onto a visual on the
  page, to that visual's default role (a measure or a number to the first value well with room, a
  column to the first grouping well), and selects it; a well's field onto a well moves or
  reorders it (`visual.move_field`), anywhere else removes it. The targets light up while
  dragging. A field's box, with a visual selected, adds it to the default role or removes it.
  Right-click a well's field: Rename for this visual (`visual.rename_field`); on a value
  (measure or summarized column) **Show value as** › No calculation, Percent of grand total, and
  on a matrix Percent of column total / of row total (`visual.show_value_as`; the well then
  names it `%GT …`, `%CT …`, `%RT …`); on a category **Show items with no data**, a check that
  toggles the field's role (`visual.show_items_no_data`).
  Right-click a table, field or hierarchy: New measure, Rename (the report follows), Hide /
  Unhide. Each edit is one command, so Undo takes it back and the page re-renders. A reader's
  panes are read-only. The chevron collapses a pane; the closed panes share one 30 px rail at the
  window's edge, whose icons reopen them; open or collapsed and the widths are kept in the
  settings.
- **Selection pane** (ROADMAP M11.23, `crates/ui/src/selection_pane.rs`): Desktop's Selection
  pane, the rightmost of the right panes and closed until its rail icon (the layers) is clicked
  (open or collapsed and the width are preferences, `panes.selection`). It lists the page's
  visuals **front to back**; the members of a group sit under it, indented 12 px per level, with a
  fold chevron, and a group is listed by its display name. Each row's eye shows or hides the
  visual (`visual.set_hidden`: `isHidden`, a hidden group hides its members in the rendered page
  and the canvas no longer picks them). A click selects the visual on the canvas, Ctrl+click adds
  it or takes it out; a double click on a group (or the footer's Rename) renames it in place:
  Enter writes `visual.rename_group` (the group's `visualGroup.displayName`; PBIR has no display
  name for a visual, so a visual is listed by its title or its type), Esc cancels. The footer
  moves the selection forward or backward (`visual.order`), groups and ungroups it
  (`visual.group`, `visual.ungroup`), renames the selected group, and toggles **Tab order**: the
  rows then read flat in keyboard order, each with its `tabOrder`, and the two arrows send
  `visual.tab_order` (0, 1000, 2000… in the order listed). Every gesture is one command, so one
  undo step; folded groups, the tab-order mode and the text being typed are view state. A reader
  sees the list with the eye, rename and moves disabled.
- **Bookmarks pane** (ROADMAP M11.24, `crates/ui/src/bookmarks_pane.rs`): Desktop's Bookmarks
  pane, left of the Selection pane and closed until its rail icon (the pin) is clicked (open or collapsed
  and the width are preferences, `panes.bookmarks`). It lists the report's bookmarks in their
  saved order; a group's caption sits above its bookmarks, indented, with a fold chevron. A click
  selects a bookmark (Ctrl adds or removes), a double click or **Rename** edits it in place
  (Enter: `bookmark.set`, or `bookmark.rename_group` for a group; Esc cancels). The footer holds
  **Add bookmark** (`bookmark.capture`, the new name `Bookmark N`, one past the highest used),
  **View bookmark** (`bookmark.apply`: the captured page, hidden visuals and filters, one undo
  step), **Update bookmark** (`bookmark.update`), **Rename**, **Delete** (`bookmark.remove`, a
  group goes with its bookmarks), **Group** (`bookmark.group`) and **Ungroup**
  (`bookmark.ungroup`), the moves (`bookmark.set {index}` within the bookmark's group or the
  list) and Desktop's three options for the bookmark selected: **Data** (apply its filters,
  `suppressData`), **Display** (apply its hidden visuals, `suppressDisplay`) and **Current page**
  (go to its page, `suppressActiveSection`; off by default, as Desktop). A bookmark is stored as
  the raw PBIR `bookmark` document (`report.bookmarks`) plus its `bookmarks.json`
  (`definition/bookmarks/bookmarks.json`, `report.resources`), so a file Desktop wrote keeps
  its order and groups. The pane's View writes the bookmark into the report (one undo step); a
  bookmark button or a bookmark navigator on the page applies it to the view only
  (`view.interact {action: "bookmark"}`, "Navigators" below).
- **Sync slicers pane** (ROADMAP M11.25, `crates/ui/src/sync_pane.rs`): Desktop's "Sync slicers"
  pane, left of the Bookmarks pane and closed until its rail icon (the link) is clicked (preferences,
  `panes.sync`). It lists the report's slicers page by page; a click selects one (Ctrl adds), and
  the footer holds the **group name** (typed), Desktop's **Fields** and **Filters** options and
  the **Synchronize** / **Unsync** buttons. Synchronize is `slicer.set_sync` with the selected
  slicers, the group name and the two options; it writes each slicer's `visual.syncGroup`
  (`groupName`, `fieldChanges`, `filterChanges`), kept verbatim in `visual_extra` and written back
  by the PBIR writer. Unsync is the same command with `group: null`. `slicer.sync_list` lists the
  slicers and the groups they form (the palette's "Synchronized slicers"). A reader sees the list
  with the buttons disabled. Each gesture is one command, so one undo step. A group name typed
  in another case joins the existing group under its spelling. What a group does
  (`crates/engine/src/slicer_sync.rs`): **Filters** on, a click on a slicer item, in the app
  (`view.interact`) as in the browser viewer (one state per viewer), reaches every slicer of the
  group on every page, so another page shown draws its synced slicer ticked and its visuals
  filtered; joining a group copies no selection (each slicer keeps its own until the next click
  in the group, as Desktop does for the files' selections when a report opens, interaction
  oracle rule 6). **Fields** on, a field bound, removed or moved on a slicer of the group
  (`visual.bind`, `visual.unbind`, `visual.move_field`, `visual.bind_hierarchy`, the Build
  pane's wells) goes to every slicer of the group whose Fields option is on, in the same command
  (one undo step; the answer's `synced` lists them).
- **Format visual** (ROADMAP M11.22, `crates/ui/src/format_pane.rs`): the Build pane has
  Desktop's two sub-tabs at its top, **Build visual** (the gallery and the wells above) and
  **Format visual**, where a visual is formatted, as in Desktop (the contextual Visual tab only
  keeps a **Format** button that opens this tab). The header row names the view shown at its
  left ("Build a visual" / "Format a visual", 13 bold) and puts the two icon tabs at its right,
  as Desktop. Format visual has the **Visual** and
  **General** groups of collapsible cards, a search box over the cards and their properties, an
  on/off switch in the header of the cards that have one and **Reset to default** at the bottom
  of each. General: **Title** (on/off; the text as free text; font, size, bold, italic,
  underline, colour, horizontal alignment: `visual.set_title`), **Subtitle** (the same keys:
  `visual.format {subtitle}`), **Effects** (Background with its colour and transparency, Visual
  border with colour, rounded corners and width, Shadow with colour and transparency, each with
  its switch), **Header icons** (on/off, background and icon colours), **Tooltips** (the tooltip
  page, Default or None: `visual.tooltip_page`), **Properties** (padding, alt text). Visual, by
  the visual's type: the cards are **generated from the visual catalog**
  (`bicraft_visuals`, the same objects and properties `visual.options` lists), so a type shows
  exactly the cards Desktop gives it: a column chart Legend, X-axis, Y-axis, Data labels, Data
  colours; a pie Legend, Detail labels, Data colours; a gauge Gauge axis, Data labels, Target; a
  table Values, Column headers, Row headers, Grid, Total, Subtotals; a slicer Slicer header,
  Items, Selection controls, Slider, Dropdown, Date inputs; a card Callout value, Category
  label, Card, Card title; a map Map controls, Map styles, Bubbles, Zoom; an image Image fit and
  Change image (its own static card). A card's title is the catalog's object name, normalised
  and translated; each row is one property of that object, shown only when the catalog gives it
  to the type, and written as `visual.format {objects: [{object, property, value}]}` (the
  color-valued properties as `{color: "#…"}`, the numeric enumerations, display units, as
  numbers). A type whose objects no card covers yet (script visuals, RDL, buttons, shapes, text
  boxes) points to Format visual's form, where any catalog property is written by name. The
  values shown are the visual's own
  (`visual.props`: each card's `{object, property}` sits in its `format.objects` list with the
  value written in PBIR form, `{color: "#…"}` and `{literal: "1000D"}`, so a switch, a list, a
  swatch and Reset to default read back what was written), then the snapshot's resolved
  formatting (theme included), then Desktop's defaults. Each edit is one command (undo takes it back): a switch, a list, a swatch of the
  colour popup (the theme's colours, the neutrals, a hex box) at once; text on Enter or when the
  box loses the focus (Escape keeps the old text); a number when it is let go. The colours
  Desktop formats conditionally have an **fx** button at the right of their swatch (a chart's
  Data colours › Colour, `data_color`; a table's or matrix's Values › Background and Text
  colour, `background` and `font_color`; a card's Callout value › Colour, `font_color`, and
  Category label › Colour, `category_color`; a chart's Data labels › Colour, `label_color`; in
  the General cards the Title's and Subtitle's Text colour, `title_color` and `subtitle_color`,
  and Effects › Background and Visual border › Colour, `visual_background` and `border_color`),
  and beside the Title's and Subtitle's Text box (`title_text`, `subtitle_text`: Field value
  only, the text from a text measure or column), marked (accent) when the visual has a
  conditional format there (`visual.props` `conditional`). The General cards' formats are one
  value for the whole visual (no series, even on a table). It
  opens the **Conditional formatting** dialog (`crates/ui/src/conditional_dialog.rs`): Format
  style (Gradient, Rules, Field value), for a table the series formatted, the field to base it
  on (the model's measures, or a column with its summarization: Sum, Average, Minimum, Maximum,
  Count; a field value takes any column, read through its first value), then the gradient's
  lowest, optional middle and highest colours, or the rules (operator `=`, `>`, `>=`, `<`, `<=`
  or between, one or two numbers, a colour, New rule, delete), or nothing more for a field
  value. A gradient's minimum, centre and maximum are the lowest / middle / highest value or a
  number, with "How should we format empty values?"; rules take a second condition ("and"),
  "is blank" and Reverse colour order; a table's format has Apply to (values, values and
  totals, totals). A table's or matrix's **Cell elements** card opens the **Data bars** (Show
  bar only, minimum, maximum, bar colours, axis, direction), **Icons** (rules with an icon
  each, icon layout, alignment, an icon set) and **Web URL** (the field) dialogs. A live
  **preview** draws five sample values with the format. OK sends one `visual.conditional
  {target, series?, apply_to?, rule}` (one undo step); **Remove conditional formatting** sends
  `rule: null`. Rules' Percent bounds are not offered (docs/gaps.md G54). A
  reader has no Format visual tab. `bicraft ui ui.set '{"buildTab": "format"}'` shows it.
- **Analytics** (G56, `crates/ui/src/analytics_pane.rs`): the Build pane's third sub-tab, as
  Desktop's Analytics pane. It reads the selected visual's `analytics` from `visual.props` and
  shows one card per analytics kind the type has in the catalogue: X- and Y-axis constant lines,
  min, max, average, median, percentile, trend, forecast, error bars, and on scatter charts the
  ratio line and symmetry shading. A card has **Add** (error bars first need an upper or lower
  bound picked), each line with its Show switch, its options (value, series, colour,
  transparency, style, width, position, data label; for error bars bounds, relative, bars,
  markers, marker shape and, on line charts, the band) and **Remove**. Every edit sends one
  `visual.analytics` with the whole list (one undo step). A list entry BIcraft cannot express is
  shown as "Kept as written". A reader has no Analytics tab;
  `bicraft ui ui.set '{"buildTab": "analytics"}'` shows it.
- **Formula bar** under the ribbon, as Desktop's (`crates/ui/src/formula.rs`): it appears when a
  measure, a calculated column or a calculated table is selected: a click on a measure or a
  calculated column in the Data pane or on a measure in a Build well, the model diagram's
  selection, a double click in the diagram, or right-click **Edit measure** / **Edit formula**
  in the Data pane (these put the cursor in it). It shows `Name = <DAX>` in a monospace font with
  the DAX colours (keywords, functions, strings, numbers, table, column and measure references,
  comments), on one line or expanded (the chevron at the right), next to the object's home table
  and format (the Measure tab keeps the format string and the display folder; the gear opens the
  measure's properties). **Enter** or ✓ commits, **Shift+Enter** or **Alt+Enter** adds a line,
  **Esc** or ✗ cancels (an unedited bar closes). A commit is one engine command: `measure.set`
  with the expression and, when the name part changed, the new name (one undo step; the report
  follows the rename), `column.set` for a calculated column, `partition.set` for a calculated table
  (its new expression re-derives the columns, and a new name goes in the same command: one undo
  step); **New measure** (Home, the Data pane's menu)
  opens the bar with `Measure = ` in the selected or first table and commits with `measure.add`.
  Ctrl+Z after a commit is the engine's undo: the bar shows the previous expression again. While
  you type, **autocomplete** lists DAX functions (the engine's list, `dax.functions`), keywords
  (VAR, RETURN…), tables (`'Sales'`), columns (`'Sales'[Amount]`; after `'Sales'[` that table's
  columns and measures) and measures (`[Total]`; after `[` the measures, and the home table's
  columns in a calculated column): ↑ ↓ choose, Enter or Tab insert, Esc closes the list,
  Ctrl+Space opens it. Inside a function's parentheses its **signature** shows under the cursor,
  the argument being typed underlined. A moment after typing stops (0.35 s) the text is checked
  with `dax.validate {kind}` through the document's source (local, a server, the browser alike;
  typing never waits for it): the first error is underlined with a wavy line and its message, line
  and column are shown under the bar; a refused commit says why there too and keeps the edit. As in Desktop,
  invalid DAX is committed all the same (`force`: the measure, column or table keeps its new
  expression) and the error stays under the bar ("Saved with an error: …"). A click away from the
  bar (the canvas, the panes) commits the edit, and so does picking another measure or column
  while one is edited; a click on the bar's own buttons does not (✗ still cancels). With the
  cursor next to a bracket, the bracket and its partner are marked (`(` `)`, `{` `}`, a reference's
  `[` `]`). **Ctrl+H** (or the bar's search button) opens a find and replace row under the editor:
  the matches are marked (letter case ignored unless **Match case**), Enter and Shift+Enter move
  between them, **Replace** and **Replace all** change the text as typing would (the commit stays
  yours), Esc closes the row. **Format
  DAX** (the bar's format button) lays the expression out with `dax.format {style: formula}`:
  BIcraft's own style, which follows the DAX formatting conventions SQLBI describes publicly
  (function names and keywords in upper case, `SUM ( Sales[Amount] )`, one argument per line in a
  call that does not fit, the expression after `RETURN` one level in); the text changes on the bar
  (expanded when it has several lines) and the commit stays with you. The autocomplete list shows
  the selected function's signature and BIcraft's one-sentence description of it, in English or
  French (`dax.functions` `description` / `description_fr`, written for BIcraft, not taken from
  Microsoft's reference). Readers see the expression read-only. The whole expression is read with `measure.get` / `column.get` /
  `table.get` (the snapshot caps measure expressions at 20,000 characters), so a 10,000-line
  measure loads whole; the colours are worked out once per change and egui keeps each line's
  layout, so a keystroke lays out the line it changed.
- **Canvas editing**, as Desktop's report canvas (`crates/ui/src/canvas.rs`, ROADMAP M11.16). A
  click selects the topmost visual under the pointer, **Ctrl+click** adds or removes one, a drag
  on the empty page draws a **rubber band** that selects every visual it touches (Ctrl keeps the
  selection), **Ctrl+A** selects every visual of the page, Escape none; the selected visuals get
  the `ring` outline and their rectangle the **eight resize handles**. **Drag** a selected visual
  to move the selection, a handle to resize it (several visuals scale inside their rectangle);
  the moving edges and centres **snap** to the other visuals' edges and centres and to the page's
  edges and centre (within 6 screen points), with dashed **smart guides** in the `guide` colour,
  and, with View › **Snap to grid** on, to an 8 px grid where no guide is near (View ›
  **Gridlines** shows it); Alt drags freely. View › **Lock objects** stops drags, resizes and
  nudges (a click or a rubber band still selects).
  The page image stays as it is during the drag; ghost outlines show where the visuals go and the
  status bar shows `x · y · width × height`; on release one `visual.move` is sent (one undo
  step) and the outlines stay until the next snapshot draws the moved visuals. The selection
  stays on the page and at least 10 px wide and high. **Arrow keys** nudge the selection 1 px
  (Shift: 10 px), one `visual.move` per press. **Delete** removes the selection in one
  `visual.remove {visual, visuals}`. The Visual tab's **Arrange** group has **Align** (left,
  centre, right, top, middle, bottom, distribute horizontally and vertically: `visual.align`; one
  visual aligns to the page), **Layer order** (bring to front, forward, backward, to back:
  `visual.order {visuals, to}`), **Group** and **Ungroup** (`visual.group`, `visual.ungroup`).
  **Copy, paste, duplicate**: Ctrl+C (and the visual's menu **Copy**) runs `visual.copy` and puts
  its JSON document on the OS clipboard as text and in an in-app clipboard; Ctrl+V pastes the
  clipboard's BIcraft visuals on the page shown (`visual.paste`: the same place, 20 px right and
  down while a visual is there; on another page or in another open document too), falling back
  to the in-app clipboard when the OS one is empty; Home > **Paste** and the menus' Paste read the
  OS clipboard (the `arboard` crate) and fall back the same way; Ctrl+D or **Duplicate** runs
  `visual.duplicate`. The pasted visuals are selected; fields the document's model lacks are kept
  as broken references and the status bar says how many warnings the paste gave. Clipboard text
  is untrusted: more than 8 MiB, text that is not a clip, or a clip the engine refuses pastes
  nothing and says why. Readers can select and copy, not move, paste or nudge.
- **Page tabs** at the bottom of the Report view, as in Power BI Desktop: every page, the shown
  one semibold on the card colour with an amber bar, hidden pages dimmed with a crossed eye,
  `⚠ N` / `✕ N` for the issues of the page and its visuals, a tooltip with the page type, `+` for
  a new page (shown at once). Right-click: Rename (inline; Enter keeps, Escape cancels),
  Duplicate (`report.extract` then `report.apply` as a new page `Name (2)`: positions, types,
  fields and properties of the spec), Delete (asks first; the last page cannot go), Hide / Show
  page, Page type (normal, tooltip, drillthrough: `page.set_type`), Move left / right. Drag a tab
  to reorder (`page.move`). Keyboard: Tab to a tab, Enter or Space shows it, F2 renames, Delete
  deletes, Alt+Shift+← / → moves.
- **Model**: the viewer's model diagram, painted with egui (see "The model diagram" below); the
  ribbon has Fit to window, Zoom in, Zoom out and Actual size, then the model's commands.
- **Data view** (the ribbon's Data tab, or the status bar's table button; Desktop's table view,
  `crates/ui/src/data_view.rs`): a table's rows in a virtualized grid, the same in both shells.
  The **Data** pane at the right lists the tables (a click shows its rows, its columns under it
  select one) and the model's Power Query queries (a click previews one; a table's right-click
  menu has Preview query too). The grid asks the engine for pages of 200 rows (`table.rows`,
  three requests in flight at most, the 48 pages nearest the view kept), so a table of millions
  of rows scrolls without being read whole; rows not read yet show `…`. Each header shows the
  column's type (`Aa` text, `123` whole number, `1.2` decimal, `$` fixed decimal, `T/F`, the
  calendar for dates, the calculator for a calculated column) and its name; a drag on its right
  edge resizes it. A click on a header selects the column: the contextual **Table** tab shows
  the **Column tools** (Name, Data type, Format, Summarization, Data category, Sort by column:
  each a `column.set`; New measure there and the Columns group's New column open the formula bar, a new column committed
  with `column.add` and the expression's type), and a calculated column goes on the formula bar.
  The header's arrow opens its menu: Sort ascending, Sort descending, Clear sort, Clear filter,
  **Text filters**, **Number filters** or **Date filters** after the column's type (Equals, Does
  not equal, Begins with, Ends with, Contains and their negations for text; Greater than, Less
  than, Between… for numbers; Before, After, Between… for dates; Is blank, Is not blank; Custom
  filter…), each opening the **Custom filter** window (Show rows where: two tests joined by And
  or Or, OK, Cancel; sent to `table.rows` as two conditions or one `any`), and the column's
  values to keep (`column.values`, a search box, Select all, OK). Sorting and
  filtering are view state, as in Desktop: the model does not change; Clear sort and filters
  resets both. A click selects a cell, Shift+click extends the selection, a header click selects
  the column; Ctrl+C, Copy or the right-click menu copies the selection as tab-separated text
  (rows not read yet are left out and the status bar says how many). The status bar has the table
  and its row count (`Table: Sales · 3,000 rows`, `of N` when filtered). The same grid shows a
  **query preview** (Home › Preview query, the pane's queries: `query.preview`, its first 1,000
  rows) and **See records** (right-click a data point of a visual, See records: the rows of the
  table behind the point, `visual.records`, under the report's, page's and visual's filters,
  the slicers, measure filters and a cross-highlight from another visual; the note under the title
  names what could not be applied), with Close back to the table.
- **Validation** (Analyze > Show validation, the status bar's view button, Info's Check the file):
  a card for the model (`✓ valid`, or `✕ N invalid objects` with their
  diagnostics) and one for the report (errors and warnings, `⚠` / `✖` per issue), from the
  snapshot's `validation` (the same data the viewer's panel shows).
- Clicking the active tab (or double-clicking a tab) collapses the ribbon; the state and the tab
  shown are kept in the settings.
- **Status bar**: the page (`Page 2 of 7`), the progress, the last command (`✓ id` / `✕ id`, as
  the viewer's), the connection (`live`, `connecting`, `offline`), on a server your role (reader or
  editor), and the Report / Data / Model / Validation view buttons; in the Data view, the table
  and its row count.
- The OS title bar reads what the viewer's tab reads: `<file> · rev N`, with `● ` in front when
  the model or the report has unsaved edits (`<workspace> · <host>` for a server until its first
  snapshot). Errors (a file that cannot be read, a report that cannot be read, a server that
  refuses: `✕ reason` for a refused click) show in a red bar above the page; the window keeps
  working. A model-only file (`.bim`) is no error: the Report tab says "No report in this file"
  and the Model and Validation tabs show the model.
- **Fonts**: the app's text is Selawik (regular, semibold, bold, light: the OFL faces
  `bicraft-render` bundles, metric-compatible with Segoe UI), headings and card headers in
  semibold, DAX and badges in Hack; the symbols (`✓`, `✕`, `⚠`, `●`, `▶`…) from a bundled subset
  of Noto Sans Symbols 2. **The report pages are drawn with the bundled faces too**, never the
  system's: the window and the browser draw the same pixels ([webui.md](webui.md)). `bicraft
  render` still uses the system's fonts (Segoe UI on Windows).

**Language.** English or French chrome, from `--lang` or the system's language. The tag is also
the session's UI language for local files: the text Power BI generates inside visuals (auto
titles, "(Blank)", "Total", slicer words) follows it, as `bicraft view --lang` does
([view-snapshot.md](view-snapshot.md), "UI language"). On a server, that text is the server's.

**Settings** (recent files with the date last opened and their pins, recent servers, zoom, light
or dark, ribbon collapsed, the tab shown, the palette's recent commands, the panes open or
collapsed and their widths) are
kept in `%APPDATA%\BIcraft\desktop.json` (Windows) or `$XDG_CONFIG_HOME/bicraft/desktop.json`
(else `~/.config/bicraft/desktop.json`). A missing or damaged file is an empty one. Tokens are
never written there.

## Keyboard navigation and screen readers (2026-11-20)

The window can be used without a pointer, after Microsoft Learn's "Keyboard shortcuts in Power BI
Desktop" (its frequently used shortcuts and pane navigation); `crates/ui/src/keyboard.rs`, the
same in the desktop and the browser app. It is view state: the only commands it runs are those
of the buttons a key tip presses, as their click does.

- **Sections.** Ctrl+F6 moves the focus to the next section of the window, Ctrl+Shift+F6 to the
  previous one: the title row, document tabs and ribbon; the formula bar (when shown); the view
  (report canvas, table view, model diagram, DAX query view); each side pane, left to right
  (Filters, Build, Data, Sync slicers, Bookmarks, Selection, the panes' rail); the page tabs; the
  status bar. The focus lands on the section's first control. F6 activates the Selection pane
  when it is open, as in Desktop; without it F6 and Shift+F6 move as Ctrl+F6 does. In the Model
  view the Properties pane and the Data pane are sections of their own, after the diagram.
- **Key tips** (`crates/ui/src/key_tips.rs`): Alt pressed and released alone, or F10, shows a
  letter under each ribbon tab; typing it selects the tab (File opens the File view) and shows a
  letter on each of the tab's command buttons; typing a button's letter does what its click does
  (the same command id: the form, the dialog, the toggle, or the menu of Shapes, Align, Layer
  order, Themes, Optimization presets, its first item focused; Model › New parameter carries one
  letter for each of its two commands). Escape backs out one level; Alt, F10 or a click hides
  them. The letters are generated from the registry: the tabs' captions and the buttons' texts in
  the UI's language, in the registry's order, initials first, then the text's other letters, then
  A–Z and 0–9; one character per button while a tab has 36 or fewer, else two for all, so no code
  collides with or starts another within a level, and a code changes only when the registry or
  the language does. The view's own buttons (zoom, Show validation) have no command and no
  letter. While the tips show, the keys they take reach nothing else.
- **Shift+?** (`crates/ui/src/shortcuts_list.rs`, a `?` typed outside a text field) opens
  Keyboard shortcuts: every shortcut the registry declares (the browser's chord in a browser),
  then the window's navigation keys, with a search box (keys or text), in English and French; a
  dialog like the others.
- **Shift+F10** opens the menu of the focused control, the one its right click opens (a Data pane
  table, column or measure, a page tab, a Build pane well field, a Model view row or layout tab,
  a Data view table, a Power Query step…; `keyboard::context_menu` replaces
  `Response::context_menu`). On the report page, focused or with nothing focused, it opens the
  selected visual's menu at its centre and keeps the selection. The first item takes the focus,
  the arrows move in the menu, Escape closes it and the focus returns to the control. A control
  without a right-click menu (a Selection pane row) opens nothing. The Menu key is not read:
  egui 0.36 maps it to no key event.
- **Tab and Shift+Tab move inside a section** and wrap at its ends (Desktop's "Move focus forward /
  backward in a section"). The arrow keys move the focus to the neighbouring control (ribbon
  buttons, page tabs, rows of a list). Enter or Space activates the focused control.
- **Data pane tree**: Right arrow unrolls the focused table, folder or hierarchy, Left arrow rolls
  it up; Enter and Space toggle it.
- **Dialogs** (every window: Manage relationships, Manage security roles, Customize theme, the
  parameters form, the command palette…) are modal: Tab and Shift+Tab cycle inside the topmost
  one and the window under it takes neither the keyboard nor clicks. Opened from the keyboard, a
  dialog focuses its first control under its title bar; Escape (or Cancel, OK) closes it and the
  focus returns to the control that had it, such as the ribbon button that opened it.
- **Focus ring.** While the focus moves by the keyboard, a 2 px ring in the theme's `focus` token
  surrounds the focused control (light `#0B57D0`, dark `#8CCBFF`, high contrast `#1AEBFF`; 4.5:1
  or more on the chrome, cards, canvas, fields, hover, pressed and selected fills, 7:1 on the
  high-contrast chrome: `theme::tests::the_focus_ring_meets_4_5_to_1`). A pointer press hides it.
- **Screen readers** read the AccessKit tree egui builds (Windows UI Automation on the desktop,
  the browser's accessibility tree on the web): every button, check box, text field, combo box,
  radio button, slider, tab and menu item has a name, in English and French, in the Report view
  (with and without a visual selected), the Data, Model and DAX query views and the Manage
  relationships, Manage security roles and Customize theme dialogs
  (`keyboard_ui_tests::every_control_has_an_accessible_name` walks the tree and fails on an
  unnamed one; the Data pane's and the Filters pane's search boxes were the two it found).

Tests: `crates/ui/src/tests/keyboard_ui_tests.rs` (key presses only: the Ctrl+F6 cycle and back,
F6 to the Selection pane, Tab wrapping in the page tabs and the ribbon, Right / Left / Enter on a
Data pane table, Manage relationships opened with Enter, Tab trapped, Escape giving the focus
back, the same for Manage security roles and Customize theme, a pointer press hiding the ring,
the DAX query view as a section, the Model view's two panes visited by Ctrl+F6, Alt and F10 key
tips over the tabs and the Model tab's buttons with Escape backing out and Manage relationships
opened by its letters, in English and French, Shift+? listing and filtering the shortcuts,
Shift+F10 on a Data pane table and on the selected visual, the accessible names);
`key_tips::tests` (codes unique and prefix-free on every tab in both languages, initials first,
two characters past 36, badge contrast), `shortcuts_list::tests`; offscreen captures `keyboard-*`
(`apps/bicraft-desktop/src/tests/keyboard_shots.rs`). Not done: keyboard focus on the visuals of
the canvas (Ctrl+Right to enter a visual, Alt+Shift+F10 its menu; the "Reader tools" row), the
Menu key, Alt+Shift+1 / 9. Not checked with Narrator, NVDA or JAWS, nor in a browser (which may
keep F6, Ctrl+F6, F10 and Alt for itself), nor whether Windows' own Alt / F10 window-menu handling
reaches the desktop window first; docs/gaps.md G84.

## Insert: text boxes, buttons, shapes, images, page format (M11.20)

The **Insert** tab (after Home, as in Desktop; it shows the Report view) has the Elements group:
**Text box**, **Buttons**, **Shapes**, **Image** (`crates/ui/src/rich.rs`). Each committed change is one
engine command, so Undo takes it back and the page re-renders; a reader's buttons are greyed.

- **Text box** inserts one (`textbox.add`, 280 × 110 in the middle of the page), selects it and
  opens the **editor in place**: the text over the box on the canvas, in the run styles it will
  have (bold, italic, underline, colour, size; links underlined; dynamic values on a tint; list
  paragraphs indented), and a toolbar above it: font family, size, **B** / *I* / U (Ctrl+B,
  Ctrl+I, Ctrl+U), font colour, left / center / right, bulleted and numbered list, a link for
  the selection (http, https or mailto; empty removes it), **+ Value** (the model's measures: the
  value is inserted as a token `«[Total]»` and becomes a dynamic value, evaluated in the page's
  filters) and **✔ Done** / **× Cancel**. A style button changes the selection, or the style of
  what is typed next when nothing is selected. ✔, a click on the page outside the box or another
  page commits: one `textbox.set` with the whole text, sent only when something changed; Esc
  cancels. A double click on a text box, or its right-click menu's **Edit text**, opens the
  editor again with its runs. The editor lays out the box with the app's bundled faces (the
  rendered page uses the run's own family) and with one alignment for the whole box when every
  paragraph shares it; bullets and numbers show as an indent there and are drawn on the page.
- **Buttons** opens the list Desktop shows: left and right arrows, reset, back, information,
  help, Q&A, bookmark, blank, page navigator, bookmark navigator, apply all slicers, clear all
  slicers (`button.add`); the new one is selected.
- **Shapes** opens a menu of the shapes of the public theme schema's `tileShape` list:
  rectangle, rounded rectangle (% and px), oval, line, triangle, right triangle, arrow, chevron
  and pentagon arrows, pentagon, hexagon, octagon, parallelogram, trapezoid, pill, heart, speech
  bubble and the snipped and rounded tabs (`shape.add {page, shape}`, 300 × 200 in the middle of
  the page, a line 300 × 40); the new one is selected. The renderer draws its tile shape, fill,
  outline and rotation (`crates/render/src/shapes.rs`). The Format visual tab's Visual group has
  the shape's cards: **Shape** (the tile shape, rounded corners), **Rotation**, **Fill** (switch,
  colour, transparency) and **Outline** (switch, colour, weight, transparency), each edit one
  `visual.format` (docs/formats/pbir.md, "Shape").
- **Image**: on the desktop, the Open dialog (PNG, JPEG, GIF, SVG, WebP); the shell reads the
  file (refused past 16 MB before it is read) and the app sends its bytes, so it works the same
  with a local file and a server (`image.add {data, name}`). In the browser there is no file
  dialog: the image comes from the server workspace's files (uploaded with `PUT
  /w/<id>/files/<file>`, [web-deployment.md](web-deployment.md)), listed by extension, and the
  server reads it under the workspace's path rules (`image.add {path}`). The engine checks and
  stores it as a report resource (docs/formats/pbir.md, "Text boxes, images, buttons and page
  format"). The Format visual tab's **Image** card (Visual) sets the scaling (normal, fit,
  fill) or replaces the picture (`image.set`; the palette's Change image too).
- **Action** (the Visual tab, a button, image or shape selected): type (none, back, bookmark,
  drill through, page navigation, Q&A, web URL, apply all slicers, clear all slicers), the
  destination page (drill through: the drillthrough pages only), the bookmark, the web URL, the
  tooltip (`visual.set_action`). For a button, **Button style** edits a state (default, on hover,
  on press, disabled): text, font colour, fill, fill transparency (`visual.format {objects,
  state}`).
- **Page format** (Report › Pages; the page shown): canvas background and wallpaper (colour,
  transparency, image with its fit), canvas size (16:9, 4:3, Letter, Tooltip, Custom with width
  and height) and vertical alignment (top, middle); Apply sends one `page.format` with what
  changed. A page set to Top is drawn at the top of the Report view in Fit page.
- **Clicking a button**: as in Desktop's editing view, a plain click selects a button (or an
  image or shape with an action) and **Ctrl+click** runs its action; a reader (`reader` role on a
  server) runs it with a plain click. Page navigation, Back, Apply / Clear all slicers and drill
  through (the selected data point, M11.19) change the page; a web URL opens in
  the system's browser (the browser shell: a new tab); a bookmark action applies its bookmark to
  the view (`view.interact`, below); Q&A actions are written but refused with a message.
- **Navigators** (Insert › Buttons › Page navigator / Bookmark navigator, `button.add`): a page
  navigator draws a button per page (hidden and tooltip pages left out unless its options show
  them), the page shown highlighted; a bookmark navigator a button per bookmark of its group (all
  bookmarks when it names none), the bookmark applied last highlighted. A click on one of their
  buttons is `view.interact {action: "page"}` or `{action: "bookmark"}` (Ctrl+click in the
  editing view, a plain click for a reader). A bookmark applied this way changes only the view:
  its page (unless Current page is off), the visuals it hides or shows (unless Display is off)
  and its filters (unless Data is off); the report, its undo history and the files are left as
  they were ([view-snapshot.md](view-snapshot.md), Interactions).

**Tests** (`crates/ui/src/tests/rich_ui_tests.rs`, egui_kittest on an engine session): insert a
text box, type, Ctrl+B, + Value, ✔ (the `textbox.set` paragraphs and the evaluated value), a
double click reopens it and Esc sends nothing; Insert › Image through the dialog stub and through
the workspace's files (the browser), a file past the cap refused before sending; Insert ›
Buttons, the action editor, a button state, a plain click that selects and a Ctrl+click that
navigates; Page format (the page drawn in the new colour, its new size, a background image);
the same frames in both shells; damaged snapshots. Screenshots: `rich_content_screenshots` in
`apps/bicraft-desktop/src/tests/shots.rs` (`--features gpu-tests`, `BICRAFT_DESKTOP_SHOTS`).

## The model diagram

The Model tab draws what the viewer's Model view draws (`renderModel`, `tableCard` and
`drawRels` in `crates/view/src/page.html`), from the same snapshot, so a local file and a server
look the same:

- **Layout** (`crates/ui/src/diagram.rs`, pure and unit-tested): the viewer's layered
  layout. Each relationship points from its "many" side to its "one" side; a table's layer is one
  more than the layers pointing at it (at most 8); tables in no relationship come last; a layer
  is a column of at most 10 cards, in the order the tables were first seen (cards keep their
  place across re-reads). Sizes are the viewer's CSS: 236 px cards, 33 px headers, 22 px rows,
  96 px between columns, 22 px between cards. The viewer saves no diagram positions; the app
  places the cards the file's layout places (docs/formats/diagram-layout.md), the others after
  them.
- **Cards**: a header with the table icon, the name and the column · measure counts; the
  columns with their type badge (`123`, `1.2`, `abc`, `date`, `t/f`, `bin`) and `ƒx` when
  calculated; a dashed line, then the measures (`Σ`, their format string; invalid ones `!` in
  red, underlined). Hidden tables and objects are dimmed. 12 rows per section, then "Show all"
  (up to 400).
- **Relationships**: a curve from the row of each column (the header when the row is not shown)
  on the facing sides of the cards, `*` / `1` at both ends, the filter-direction arrow at the
  middle (two for both directions), dashed when inactive; the hovered or selected one in the
  accent colour. Hovering shows the viewer's tooltip.
- **Pan and zoom**: drag or the wheel pans, Ctrl+wheel (or a pinch) zooms about the pointer
  (5 % to 400 %), the bar's `−` `+` `100 %` and **Fit** buttons. A diagram that fits is shown at
  100 % at the top left, as in the viewer; a bigger one is fitted. Everything is vector drawing
  and text at the screen's DPI, so it stays sharp at any zoom.
- **Details panel** (right): click a table, a column, a measure or a line. Tables: contents,
  relationships, the clickable list of columns and measures. Measures: valid / invalid / not
  validated, diagnostics, the DAX expression, format, folder, hidden. Columns: type, format,
  calculated, hidden, relationships. Relationships: both ends, cardinalities, cross filter,
  active. Measures and columns also list the measures whose DAX mentions them ("Mentioned in
  measures": a text search of the expressions, case-insensitive). Raw JSON at the bottom; ×
  or Escape closes it; nothing selected shows the model's summary and invalid measures.
- **Data / Model pane** (right-most, `crates/ui/src/model_explorer.rs`): the Data tab lists the
  tables with their columns, hierarchies (and their levels) and measures; the Model tab is the
  Model explorer (calculation groups and their items, cultures, data sources, functions,
  measures, perspectives, relationships, roles, shared expressions, tables, each with its
  count). A search box keeps the rows whose name holds its text. A click selects a table,
  column, measure, calculation group or item, hierarchy, relationship, role, culture, shared
  expression or data source in the details panel, whose Properties edit it through its
  commands: a calculation item's name, description, expression and format string expression
  (`calculation_item.set`), a hierarchy's name, description, display folder and hidden
  (`hierarchy.set`), a role's name (`role.rename`), description and model permission
  (`role.set`), a shared expression's name (`expression.rename`), description, query group and M
  text (`expression.set`); a culture and a data source are read-only (no connection string is
  shown). Right-click menus run the engine commands (new, edit, rename, delete); a calculation
  item dragged onto another takes its place (`calculation_item.set {ordinal}`).
- **Layout tabs** (under the diagram): the file's diagrams (`model.diagram.layouts`); a click
  shows one (view state), a double click or the menu renames it (`diagram.rename`), the menu
  deletes it (`diagram.remove`), `+` adds one (`diagram.add`). A layout other than the one of all
  tables draws only its tables; a table's menu in the Data pane adds it (`diagram.move`), adds
  the tables related to it (Add related tables, one `diagram.move`) or takes it out (Remove
  from layout, `diagram.remove_tables`).
- **Light and dark** follow the app's theme (the design tokens: cards, the amber selection ring);
  English or French.
- **Bounded**: at most 300 cards and 1,500 lines (the viewer's limits, "drawing the first 300
  tables"); only the cards, rows and lines on screen are painted, and text smaller than 3 points
  is skipped. The layout is computed once per snapshot (and per "Show all").

The diagram code uses egui and the snapshot only (no threads, no files), so it can move into a
WebAssembly build unchanged.

## Interactions

What a click does is decided by the engine, exactly as for the viewer
([view-snapshot.md](view-snapshot.md), "Interactions"): the app only finds what was clicked and
sends the same `view.interact` action the viewer's page sends.

To find it, `bicraft-render` records a **hit map** while it draws (`RenderOptions::hits`,
`bicraft_render::hits`): for every place the viewer registers a click (`ixHit`, `ixMark`,
`ixRect` in `crates/view/src/page.html`), the region in report pixels, the action and the
tooltip rows. The regions are the drawn marks themselves, so they cannot drift from the picture:

| Where | Action |
|---|---|
| Slicer item (list, tiles, new list, buttons) | `slicer_toggle {value, ctrl}` (Ctrl+click adds or removes) |
| "Select all" | `slicer_select_all` |
| Dropdown slicer | a popup with the items and "Select all" |
| Between / before / after slicer | a popup with the two bounds → `slicer_range {from, to}`, Clear → `slicer_clear` |
| Relative date slicer | a popup (last / next / this, the number, the period) → `slicer_relative` |
| Text slicer | a popup → `slicer_text {text}` |
| Column, bar (clustered, stacked, 100 %), combo column, line point (a 7 px disc), pie or donut slice, funnel stage, treemap tile, table row | `select {point, ctrl}`, the point being the grouping values (`{c, v}`) |
| A mark of one of BIcraft's fourteen visuals (a bar, box, bin, dot, node, ribbon, circle, tile, day, point, density cell, line point, a label or line of it) | `select {point, points?, ctrl}`: the rows the mark stands for, as their runtime selects them in Power BI (`bicraft_render::own_hits`); `points` when the mark stands for several points (a bin, a Sankey node) |
| A visual's background, when it is the source of the selection | `select {point: null}` (clears it) |
| Page navigator button, page tab | `page {page}` |
| Bookmark navigator button | `bookmark {bookmark}` |
| Action button (navigation, Back, Apply / Clear all slicers, Drillthrough on the selection, Bookmark) | `button` |
| A matrix row header's +/− (with "+/- icons" on) | `matrix_expand` / `matrix_collapse {path}` |

The topmost region under the pointer wins, and every visual catches the clicks over its own box,
as the viewer's page does. Hovering a data point shows its tooltip (category, legend value,
value, the highlighted part), formatted like the visual.

**Data points from the keyboard** (2026-11-24, `crates/ui/src/point_keys.rs`): with a visual
selected on the canvas, Ctrl+Right enters its data points (the hit map's distinct `select`
actions, in the order they are drawn); the arrows move between them instead of moving the visual,
Enter or Space selects the focused one, Ctrl+Space adds it, Escape clears the visual's selection
and leaves. The focused point has a ring in the theme's `focus` token and shows its tooltip. Each
key sends the action a click on that point sends.

BIcraft's visuals are **highlighted** by a data-point selection elsewhere on the page when
they have a field in a measure role (they declare `supportsHighlight` since 2026-11-24,
`interaction.mode` `highlight`: highlighted parts over dimmed marks, or dimmed units,
[formats/bicraft-visuals.md](formats/bicraft-visuals.md) "Highlights"), filtered otherwise and
by slicers (the Venn diagram is always filtered), and their
selection is saved by `bookmark.capture` as the container's `highlight` (public bookmark schema
2.1.0: one `SelectorsByColumn` per point, a `Comparison` per column) and restored by
`bookmark.apply` and a reader's bookmark (`crates/engine/src/interact_highlight.rs`).

The renderer also draws the interaction state the viewer draws: a point the selection leaves
out at 40 % opacity in its own visual, a table row under a 60 % white veil, and highlighted
visuals with the full mark at 40 % and the highlighted part opaque over it (columns and bars,
stacked and 100 % ones, combo columns, funnel stages, pie and donut slices as a share of their
area, treemap tiles as a share of their height). `bicraft render` shows the same when the session
has a reader state.

**Tests** (`apps/bicraft-desktop/src/tests.rs`): pointer clicks on the window (egui_kittest) on a
slicer item, Ctrl+click, a column and the chart's background; after each, the app's `report`
snapshot equals that of a separate session that ran the action page.html sends for the same click
(`slicer_and_data_point_clicks_match_the_viewer`). The page the app draws is byte-identical to
`bicraft render`'s (`report.render`) at the same scale (`pages_equal_bicraft_render`).
BIcraft's visuals: `crates/ui/src/tests/own_visuals_ui_tests.rs` (a click, Ctrl+click and the
keyboard on the BIcraft bullet filtering a table) and
`crates/engine/tests/report/own_visual_interactions_tests.rs` (each of the ten as the source and
the target, the bookmark round trip, the viewer's drawing).

### Edit interactions (M11.15)

Desktop's mode for setting how a selection in one visual reaches the others. **Edit
interactions** on the contextual Visual tab (Interactions group) turns it on and off; Escape
turns it off; readers cannot enter it (the button is greyed, and a session that becomes
read-only leaves the mode). While it is on, the visual selected on the canvas is the source and
every other visual with fields shows an icon strip at its top-right corner: **Filter**,
**Highlight**, **None**, only the ones the pair can take (Desktop's rules from the interaction
oracle: a slicer only filters; cards, KPIs, tables, matrices, line and area charts, maps,
slicers… cannot highlight, so they show Filter and None). The current effective mode is marked;
the default (no rule) is marked with a lighter look and its tooltip says "Default: Highlight" (or
"Default: Filter"). A click on an icon runs `page.interactions` with that one rule (one undo
step, Ctrl+Z restores); a click on the default removes the rule. Data points stay clickable, so
the page previews the rules being edited, as in Desktop; leaving the mode clears the data-point
selection. The report-wide defaults are **Interaction settings** (`report.settings`): "cross
filter by default" (Desktop's report option "Change default visual interaction from cross
highlighting to cross filtering", followed by the viewer) and "Drilling filters other visuals"
(followed by the drill since M11.19; a visual's own "Apply drill down filters to" overrides it). See docs/design.md "Edit interactions" for the drawing and
`crates/ui/src/interactions.rs` for the code; tests in `crates/ui/src/tests/interactions_ui_tests.rs`,
screenshots in `interactions_screenshots`.

### Drill, drillthrough and page tooltips (2026-11-06)

As in Desktop's reading view, and the same in the browser (`crates/ui/src/drill.rs`; the rules are
the engine's, docs/view-snapshot.md "Drill, drillthrough and page tooltips"):

- **Header icons.** Hovering a visual with a drill hierarchy (two or more columns or hierarchy
  levels on its axis or matrix rows) shows four icons at its top right: Drill up (↑, greyed at the
  top level), drill mode (↓ in a circle, highlighted while on: a click on a point then drills
  into it), Go to the next level (⇊) and Expand all down one level (the fork), greyed at the
  lowest level. Each is a labelled button of the accessibility tree and sends `drill_up`,
  `drill_mode`, `drill_next` or `drill_expand`.
- **Right click** on a visual adds, above its commands: Drill down (on the point clicked), Drill
  up, Show next level, Expand to next level; on a matrix row header Expand and Collapse >
  Selection, Entire level, All; on a point of a visual that holds a drillthrough page's field,
  Drill through > the pages (a cross-report page is greyed with the reason). The menu also has
  **Keep drill state** (`view.keep_drill`): the drill and expansion are written into the report,
  as Desktop saves them.
- **Drillthrough** opens the page filtered on the point (and the source visual's filters with
  "Keep all filters"); its Back button (or a `Back` action button) returns.
- **Matrix +/−.** With the matrix's "+/- icons" on, each row header with children has a + or −
  that expands or collapses that member.
- **Report-page tooltips.** Resting the pointer on a point of a visual whose tooltip is a report
  page (set, or "Auto" from the page's tooltip fields) asks the engine for that page under the
  point (`tooltip`, after 0.25 s), then draws the page small under the pointer (at most 480 points
  wide) instead of the default tooltip; leaving the point hides it.

Tests: `crates/ui/src/tests/drill_ui_tests.rs` (hover icons and their clicks, drill mode, matrix
+/−, right-click Drill through and Back, a tooltip page appearing, the same frames in both
shells); screenshots with `--features gpu-tests` (`drill_screenshots`, `BICRAFT_DESKTOP_SHOTS`).

### View as (2026-11-18)

Desktop's Modeling › View as, the same in the browser (`crates/ui/src/view_as.rs`; the rules are
the engine's, docs/view-snapshot.md "View as"): **Model › View as** (the Model ribbon's last
button, highlighted while on) opens the View as roles dialog: None, Other user with the
user's name (what `USERPRINCIPALNAME()` returns), a check box per role of the model, OK and
Cancel; it opens on the current state. OK sends `view.interact {action: "view_as"}`. While it is
on, a banner under the ribbon says "Now viewing as: East, West (user)" with **Stop viewing**, in
every view (Report, Data, Model). It is view state: nothing becomes unsaved. A reader may use it
(each viewer of a server has its own, which only narrows what the reader reads).

Tests: `crates/ui/src/tests/view_as_ui_tests.rs` (open, a role and OK, Other user, Stop viewing,
None and Cancel, French, a reader); screenshots with `--features gpu-tests`
(`view_as_screenshots`: `view-as-dialog-*`, `view-as-banner-*` in light, French, dark).

## Live reload

A local file or project is watched as `bicraft view` watches it (`bicraft_engine::watch`: the
paths, sizes and modification times of the model and report files, polled every 150 ms; a change
is re-read once it has been still for 300 ms). The session runs on a worker thread, so the window
never waits for a read or an evaluation. While visuals wait for their data (a snapshot computes
for a bounded time), the snapshot is taken again until every visual has it. The reader state
(slicers, selections, page) is kept across re-reads.

## Auto-recovery and autosave

Unsaved edits are journaled in the per-user recovery folder (`%LOCALAPPDATA%\bicraft\recovery`,
`$XDG_STATE_HOME/bicraft/recovery`, or `BICRAFT_RECOVERY_DIR`; [recovery.md](recovery.md)). After
a crash, or quitting with unsaved edits, the start screen (File > Home) lists the files with
recoverable edits; opening one shows the recovery prompt (Recover, Discard, Later). The status
bar shows `Recovery: N edits` while the journal holds edits. Closing a tab with "Close without
saving" discards its journal. A file re-read after a change on disk keeps the edits it had for
recovery. A model never saved (New model) is journaled too and listed on the start screen as
"New model, never saved", with Recover; the view settings (visual text language, captions'
culture, as-of date, the page shown) come back with the edits. File > Info > **Save
automatically** (off by default) saves every open PBIP project (each tab on its own clock) every N
minutes or N seconds after the last edit; PBIX and PBIT files are not offered (each save repacks
the whole file). Without a settings file (the tests' `app(lang, None)`) nothing is journaled;
`app_with` names a folder.

## Export to PDF and Export data (M11.21)

- **File › Export › Export to PDF** (Backstage; the ribbon's File tab Export group, the palette):
  all pages (hidden ones with *Include hidden pages*), the current page only, or the pages typed
  (`1-3, 5`); the Save dialog picks the file; `report.export_pdf` writes it ([render.md](render.md),
  "PDF": vector shapes, text with embedded fonts, a bookmark per page, a tagged structure).
- **File › Export › Print** (the ribbon's File tab, the palette): the same page choice, then
  *Print…*: `report.print` writes the PDF to the temporary folder's `bicraft-print` folder and
  hands it to the OS's PDF application, where it is printed; the task says where it was written
  and that it was opened. Tests inject the session's opener, so nothing is launched.
- **A visual's Export data**: the selected visual's header has a More options button (⋯, at its
  top right) and its right-click menu an *Export data* item; both open the Export data dialog:
  summarized data (what the visual shows) or underlying data (greyed unless the report allows
  it), `.xlsx` (up to 150,000 rows) or `.csv` (up to 30,000), then the Save dialog;
  `visual.export_data` writes it and the dialog shows the rows written and a truncation.
- **File › Export › Export data setting**: the report setting `report.settings {export_data}`
  (Allow summarized data only, summarized and underlying, or nothing), what the dialog honours.
- Tests: `crates/ui/src/tests/export_tests.rs` (the Backstage task runs `report.export_pdf` with
  the pages picked, the dialog runs `visual.export_data`, the setting greys underlying data, the
  header's More options opens the visual's menu; Print runs `report.print` with the pages picked
  and the recorded opener gets one tagged PDF; the browser's Print hands the workspace PDF to
  `Shell::print`).

## Focus mode, Show as a table, Sort and Spotlight (M11.19, M11)

The selected visual's header has a **Focus mode** icon beside More options (⋯), and More options
and the right-click menu start with **Spotlight**, **Show as a table** and **Sort axis**, as Power BI Desktop's
visual header does (`crates/ui/src/focus.rs`).

- **Focus mode** shows the visual alone, filling the canvas (8 px margin), with a *Back to
  report* button above it; the header's icon (now *Exit focus mode*) and Escape go back too. It is
  the reader's view state: the buttons send `view.interact` `focus {page, visual | null}` and
  `show_table {page, visual, on?}` (MCP and the CLI send the same), the window draws what the
  snapshot's `interaction.focus` / `show_table` say, and the report and the files never change.
  The focus icon shows when the header sits above the visual (a visual at the page's top keeps
  More options alone); More options and the right-click menu have a *Focus mode* item too. The page
  drawn is derived from the snapshot (the focused page holds only the visual, moved and resized
  to fill it; the renderer lays it out at that size, so it is not a scaled picture), so the visual
  stays interactive: cross-filter clicks, tooltips, the drill icons and the menus act as on the
  page. The canvas's editing (move, resize, drop a field, the arrow keys, paste) is off while it is
  shown. A page change, a drillthrough or the visual's removal leaves it.
- **Show as a table** shows the visual in focus with its data as a table under it (the visual on
  the upper half, the table on the lower half). The table reads the visual's own `data` in the
  snapshot (its summarized data, evaluated under its filters, slicers and cross-filters, in the
  visual's sort; at most `SnapshotLimits::data_rows_per_visual` rows), drawn by the renderer's
  table; its rows are not clickable. Unchecking it keeps focus mode without the table. Offered for
  every visual with fields except slicers.
- **Sort axis** lists the visual's columns, measures and aggregations, then *Sort descending* and
  *Sort ascending*, checked from the snapshot's `sort` ([view-snapshot.md](view-snapshot.md)). A
  field sends `visual.set_sort {page, visual, field, direction}` (descending for a measure or an
  aggregation, ascending for a column, the command's defaults); a direction sends the current field (else the first measure)
  with that direction. It writes the visual's `query.sortDefinition`, an undoable report change,
  as Desktop does. For a reader (a server session's reader role) the same items send
  `view.interact` `sort {page, visual, field, direction}` instead: the reader's view is sorted,
  the report is not (as the Power BI service's readers sort, Microsoft Learn, "Change how a chart
  is sorted in a report"); `reset` (Reset to default) drops it, a bookmark carries it. Not offered
  on cards, gauges, KPIs, slicers and the decorative types.
- **Spotlight** (More options and the right-click menu, checked while on) puts the visual
  forward: the page's other visuals fade to near transparency (Microsoft Learn, "Add spotlights
  to Power BI reports"; the renderer draws them at 15 % opacity, BIcraft's choice). It is view
  state: `view.interact` `spotlight {page, visual | null}`, the snapshot's
  `interaction.spotlight`. Escape, a click outside the visual (which does nothing else), a page
  change and `reset` end it; a click inside the visual acts as usual. A bookmark captured while
  it is on keeps it (`singleVisual.display.mode: "spotlight"`, [pbir.md](formats/pbir.md),
  "Bookmark display mode and sort") and applying one sets it.
- Tests: `crates/ui/src/tests/focus_ui_tests.rs` (the derived layout; focus from the header, Back
  to report and Escape, nothing sent; Show as a table from More options, its rows not clickable;
  Sort axis sending `visual.set_sort` and the menu checking the result; a reader's Sort axis
  sending `view.interact` `sort`; Spotlight dimming the card, ended by Escape and by a click
  outside); screenshots `focus_mode_screenshots` and `spotlight_screenshots` in
  `apps/bicraft-desktop/src/tests/shots.rs`.

## Mobile layout (2026-11-19)

View › **Mobile layout** switches the Report view to the page's phone canvas, as Power BI
Desktop's (Microsoft Learn, "Power BI mobile layout view", "Create an initial layout",
"Optimize visual formatting for mobile layout"; `crates/ui/src/mobile.rs`). It is view state;
every edit is an engine command, one undo step.

- **The phone canvas** is Desktop's (maintainers' decision, 2026-11-24): portrait, 324 points
  wide and 3600 high while it is edited, scrolled, with a grid of cells 18 wide by 20 high, as
  Power BI Desktop 2.159.1231.0 draws its own (black-box observation, 2026-11-23;
  [gaps.md](gaps.md) G72). It grows past 3600 only when visuals go further (unobserved in
  Desktop). The zoom fits about 568 points of it in the view (BIcraft's choice). The figures are
  `bicraft_render::phone`'s, shared with the engine. It is drawn from a derived snapshot, as focus mode is: the page holds only the
  visuals placed on the mobile layout, at their phone place and with their phone format
  (`visuals[].mobile`, [view-snapshot.md](view-snapshot.md) "Mobile layout"), so they render and
  interact as on the page.
- **Page visuals** takes the Data pane's place: every visual of the page (groups left out), front
  to back, with an eye and the word *shown* or *hidden*, and *On phone* with a check for those
  placed. A row dragged onto the phone canvas, or double-clicked (placed under the others), sends
  `visual.mobile {page, visual, x, y, width, height}`: as wide as the phone at most, its ratio
  kept, on the 18 × 20 grid (its corner on a grid point, its width a whole number of columns),
  on top of the other layers.
- **On the phone canvas** the report canvas's gestures apply (`crate::canvas`): click and
  Ctrl+click select, a drag moves, the eight handles resize (Shift keeps the ratio), the arrows
  nudge (Shift: 10), snapped to the other visuals' edges and to the 18 × 20 grid, its columns
  for left and right edges and its rows for top and bottom ones (always on, as Learn says a
  dragged visual snaps by default; Alt moves freely). Each sends one
  `visual.mobile {page, visuals: [{visual, x, y, width, height}]}`; the desktop layout never
  moves. Copy, paste, duplicate and group are the desktop canvas's only.
- **Remove**: Delete, the X at the selected visual's top right or the menu's *Remove from mobile
  layout* sends `visual.mobile {layout: null}`; the visual stays on the page and in Page visuals.
- **Format**: with the mobile layout on, the Build pane's Format tab says its settings apply to
  the phone only; its values are the phone's (`visual.props {layout: "mobile"}`) and its edits go
  to `visual.mobile_format` (titles, axes, the tooltip page and `visual.format`'s settings
  folded in); *Clear mobile formatting* (the pane's link and the canvas menu) sends
  `visual.mobile_format {clear: true}`. The image's scaling and the fx dialog stay the visual's.
- **Narrow windows**: View › *Phone layout when narrow* (a preference, off by default) reads a
  page that has a mobile layout on its phone canvas, full width, when the report area is
  narrower than 600 points, as the Power BI mobile apps show it in portrait. Nothing is edited
  there.
- Tests: `crates/ui/src/tests/mobile_ui_tests.rs`; screenshots `mobile_layout_screenshots` in
  `apps/bicraft-desktop/src/tests/mobile_shots.rs`. The canvas's size and grid are Desktop's;
  the rest is not compared with Desktop's view ([gaps.md](gaps.md) G72).

## Performance analyzer (review lot 14)

**Analyze › Performance › Performance analyzer** (`perf.results`) opens or closes a pane left of
the other panes, as Desktop's: *Start recording* (`perf.start`), *Refresh visuals*
(`perf.refresh`, the page shown), *Stop* (`perf.stop`), *Clear* (`perf.clear`), *Export*
(`perf.export`, a JSON file through the Save dialog). One row per visual evaluated, the latest
first, with its total; a row unrolls to BIcraft's evaluation time, the render time, the rows, why
it was evaluated (Refresh visuals, or the Report view while recording) and *Copy query*: the DAX
query BIcraft writes for the visual (`SUMMARIZECOLUMNS` over its grouping columns, its column
filters as `TREATAS`/`FILTER` tables, its measures and aggregations), with notes on what the text
leaves out (measure filters applied to the rows, top N, filters BIcraft does not apply).

- The times are BIcraft's own, from a monotonic clock, in milliseconds: the evaluation is timed
  by the engine around its evaluator (a refresh skips the result cache; decoded columns stay
  cached), the render by `bicraft-render` around each visual's draw call of the page drawn again
  after the refresh, handed back with `perf.render`. They are never compared with Desktop's
  (AGENTS.md §1), and docs give none.
- The recording is the session's diagnostics state: no revision, no undo step, never saved; on a
  server the `perf.*` commands are for editors only (the session is shared).
- The browser: the same pane; its renderer has no clock on wasm32, so the render time reads *not
  measured*, and Export writes `performance-analyzer.json` in the workspace, then downloads it.
- Tests: `crates/engine/src/perf_tests.rs` (events, ordering, the query text accepted and
  evaluated by `dax.evaluate` with the visual's rows, render times, export, hostile parameters),
  `crates/ui/src/tests/perf_ui_tests.rs` (each button), `apps/bicraft-desktop/src/tests/perf_shots.rs`
  (wgpu screenshots, light, dark, French).

## Remote mode (a BIcraft server)

Home > Connect to server (or the start screen) asks for the server's URL, the workspace and a token. The server
(`bicraft serve`, [web-deployment.md](web-deployment.md), [kubernetes.md](kubernetes.md))
computes the snapshots and runs the interactions; the app only draws them, with the same renderer
and hit map, so a click gives the result it gives in a browser.

```text
POST {url}/w/{ws}/view/api/session            Authorization: Bearer … → {version: 1, view, token, access}
GET  {url}/w/{ws}/view/api/state?v=VIEW        the view's snapshot
GET  {url}/w/{ws}/view/api/asset/ID?v=VIEW     its images
GET  {url}/w/{ws}/view/api/events?v=VIEW       server-sent events: refetch on `event: snapshot`
POST {url}/w/{ws}/view/api/interact?v=VIEW     a view.interact action, X-Bicraft-Token: token
```

- **The view is private** to the window: its slicers, selections and page never reach other
  viewers or the files. Readers see the saved files; editors see the editor session, unsaved
  edits included ([auth.md](auth.md), "Roles"). A reader can click (interactions are view
  state) but changes nothing. The status bar shows the role the server gave.
- **`POST …/view/api/session`** is the one route added for native clients: a new private view
  without the HTML page, with the page's own checks (`Host`, authentication, the workspace ACL; a
  workspace you may not open answers 404). It accepts bearer credentials and the auth proxy's
  identity headers, never the browser login cookie (a cross-site form could otherwise mint a
  view and read its token). It answers `{"version": 1, "view": "<32 hex>", "token": "<32 hex>",
  "access": "reader" | "editor"}`; the token goes in `X-Bicraft-Token` on `api/interact`.
- **Authentication**: an API token (`bicraft serve token create`) or an OIDC access token
  obtained by another tool, sent as `Authorization: Bearer`. OIDC device or browser login flows
  are not built in yet. On a server without authentication (loopback only) leave it empty. With
  `--server`, the token comes from `BICRAFT_SERVER_TOKEN`; the dialog fills it in from there too.
  The token is kept in memory only: never written to the settings file, never logged, never
  shown in a message.
- **Live updates**: the app reads the view's event stream (`api/events`) and fetches the
  snapshot again on each event: an editor's MCP edits, a save, an upload or a change on disk
  show without a click. Without a stream (a proxy that buffers it) the snapshot is fetched every
  30 s. Bursts of events are fetched once.
- **Restarts**: a view the server forgot (a restart, an idle eviction: 404 on the view) is
  replaced by a new one on the next call; the stream reconnects with a back-off (0.5 to 5 s).
  The reader state of the old view is lost, as in a browser.
- **Errors** show in the red bar, worded: 401 "authentication failed (check the token)", 403
  with the server's reason, 404 "no such workspace, or no access to it", "cannot reach the
  server" with the network error. Never a crash.
- **TLS**: `https://` URLs go through rustls with the bundled Mozilla root certificates (the
  same client as the OIDC and Databricks code); no OpenSSL, no system certificate store.

**Tests** (`apps/bicraft-desktop/src/tests/remote_tests.rs`, an in-process `bicraft serve`
with API tokens): a reader and an editor connect and see their role; a reader's slicer click
changes that reader's view only; an editor's window follows MCP edits live while the reader
keeps the saved state; after a server restart a click opens a new view and works; a wrong
token is a 401, an unknown workspace a 404, a stopped server a network error. The route's own
tests are in `crates/mcp/tests/http_viewer.rs` (`native_clients_get_a_view_without_the_page`).

## Control channel (agents drive the running app)

An opt-in channel through which an agent, a script or a test drives the running window and
looks at it: open a file, run a command by id as its button does, switch tabs and pages, select
a visual, open the palette and forms, read the window's state, take a screenshot. It is the
Crafting Apps' control protocol ([DeckCraft's](https://github.com/storytold/deckcraft/blob/2b6d27d/docs/control-protocol.md),
[LightCraft's](https://github.com/storytold/lightcraft/blob/2e66a00/docs/control-protocol.md)),
with a token BIcraft adds (decision 6 of [method.md](method.md)). It never uses OS-level
synthetic input: every request runs on the window's thread between frames, through the code a
click runs (`crates/ui/src/control.rs`). The browser app has the same channel through its
server ([webui.md](webui.md), "Control channel").

```text
bicraft-desktop Sales.pbip --control 0          # a free port; or --control 7990, BICRAFT_CONTROL_PORT
bicraft ui methods                               # the methods (reads the token file)
bicraft ui app.open '{"path": "Other.pbix"}'
bicraft ui ui.set '{"page": "Overview", "ribbonTab": "model"}'
bicraft ui command measure.add '{"table": "Sales", "name": "Margin", "expression": "[Total] - [Cost]"}'
bicraft ui ui.screenshot --out window.png
```

- **Off by default.** `--control PORT` or `BICRAFT_CONTROL_PORT=PORT` turns it on (`0`: a free
  port). It listens on `127.0.0.1` only. A new random token (128 bits) is made at each launch
  and written, never printed, to an owner-only file: `--control-file F`, `BICRAFT_CONTROL_FILE`,
  else `control.json` next to the settings file (`%APPDATA%\BIcraft\control.json`,
  `~/.config/bicraft/control.json`). The file is `{"kind": "desktop", "addr", "token", "pid"}`,
  mode 0600 on Unix; on Windows an explicit access list set before the token is written:
  inheritance removed, one entry, full control for the user's SID (`icacls`, the workspace
  forbids `unsafe` code; `bicraft_http::write_owner_only`). It is removed when the app quits. stderr says
  `control channel on 127.0.0.1:PORT (token in FILE)`.
- **Protocol.** JSON lines: one request object per line, one reply per line, in order.
  `{"id": 1, "method": "ui.inspect", "params": {}, "token": "…"}` → `{"id": 1, "ok": true,
  "result": …}` or `{"id": 1, "ok": false, "error": "…"}`. `id` is a number, a string or absent;
  `params` an object. Blank lines are skipped.
- **Refused, and the connection closed** (one error line, nothing after it runs): a missing or
  wrong token (each refusal waits a little longer, up to 2 s), a line that is not a JSON object
  with a string `method` (an HTTP request line, text that isn't JSON, invalid UTF-8, unknown
  keys), a line longer than 1 MiB. A web page's `fetch` to the port cannot smuggle a request: its
  request line comes first and closes the connection.
- **Bounds.** 16 connections at once (more get `busy` and are closed), 200 requests per second
  per connection (more are answered `busy`), 64 requests queued for the window (more: `busy`),
  5 minutes idle (a `{"closed": "idle"}` notice, then closed), 30 s to write a reply, 60 s waiting
  for the window. A request the window had not started when that wait ends is given up and never
  runs, so a retry cannot run it twice; one it had started is answered "outcome unknown". A
  minimized window that draws no frame takes no request (the reply says so).
- **Methods** (`methods` returns this list):

| Method | Params | What it does |
|---|---|---|
| `methods` | | the list |
| `ui.inspect` | | the window's state: view, ribbon tab and the tabs shown, File view page, documents, pages and the page shown, the selected visual and model object, palette (query, first rows), form (fields, required, running, result), answer window, title, error, status, revision, unsaved state, pending commands, `idle` |
| `ui.commands` | `{query?}` | the palette's rows: id, label, place, shortcut, enabled or the reason |
| `command` | `{command, params?}` | with every required value in `params`, runs the command as the form's Run does; without `params`, clicks its button (`ribbon::invoke`: the selection's values, the File view's flow, the form when values are missing; `measure.add` without an expression opens the formula bar: `{command, opened: "formula_bar", text}`); waits for the engine's answer: `{command, result}` or its error. `file.open` takes a `path` and never opens a dialog |
| `app.open` | `{path}` | opens a file or project in a document tab, as Open does; waits for its first snapshot, its data and its page drawn |
| `ui.set` | `{doc?, theme?, view?, backstage?, page?, visual?, model?, ribbonTab?, buildTab?}` | what a click would switch: a document tab, light or dark, the Report, Model or Validation view, a File view page (`home`, `open`, `info`, `export`, null), a page (name, display name or index: the page tab's `view.interact`), a visual (`name` or `{page, visual}`, null), a model object (`{table}`, `{table, column}`, `{table, measure}`, `{relationship}`), a ribbon tab (`file` … `measure`, as the tab row), the Build pane's sub-tab (`build`, `format`; none for a reader); waits until the page is shown; returns `ui.inspect` |
| `ui.palette` | `{open?, query?}` | opens (or closes) the palette with a query |
| `ui.palette.choose` | `{command, form?}` | Enter (Shift+Enter with `form`) on a palette row |
| `ui.form.open` | `{command, prefill?}` | the command's parameters form (prefill: the selection's) |
| `ui.form.set` | `{fields: {name: value}, json?}` | types values into the open form (strings, booleans, lists) |
| `ui.form.run` | | Run: the problems by field, or the command's answer |
| `ui.close` | `{what?}` | `form`, `palette`, `answer`, `confirm`, `popup`, `backstage`, `all`; default the topmost |
| `ui.wait` | `{timeout?}` | until the window is idle: no source starting, every command answered, every bound visual's data computed, the page drawn (at most 55 s) |
| `ui.screenshot` | | the window as the GPU drew it (egui's screenshot viewport command): `{png, width, height}`, PNG in base64 |

- **Client.** `bicraft ui [--control FILE | --view | --serve] [--url URL] [--workspace ID]
  [--view-id V] METHOD [PARAMS-JSON] [--out FILE.png]` reads the token file
  (`BICRAFT_CONTROL_FILE`, else the desktop's default; `--view` the default of `bicraft view
  --control`, `--serve` that of `bicraft serve --viewer-control`), sends one request and prints
  the reply. For a server, `--workspace` (or `--url`, the viewer's URL, with the token from
  `BICRAFT_CONTROL_TOKEN` or the file) says where, `--view-id` which view, the server's
  credential comes from `BICRAFT_SERVER_TOKEN`, and `server.views` lists the caller's views
  ([web-deployment.md](web-deployment.md), "UI control channel"); `--out` writes a
  screenshot's PNG. `bicraft ui command ID [PARAMS]` is short for `command`. Exit 0 on `ok`, 1 on
  a refusal, 2 when the app cannot be reached. Any client that writes a JSON line to the socket
  works too.
- **Tests.** `crates/ui/src/control_tests.rs` (the protocol: requests refused, replies, a
  given-up request never runs, PNG and base64) and `crates/ui/src/tests/control_ui_tests.rs`
  (every method in a kittest window over an engine session: open, pages, visual, ribbon tabs,
  model selection, theme, File view, commands with and without values, the form, the palette,
  hostile parameters, a screenshot without a GPU renderer answered with an error).
  `apps/bicraft-desktop/src/tests/control_tests.rs` runs the socket: tokens refused and the
  connection closed, HTTP and junk lines, the line, rate, connection, idle and reply bounds, the
  token file; then `an_agent_drives_the_desktop_app_through_the_socket` drives the real desktop
  app (native shell, engine session on its worker thread, kittest stepping the window) from a
  client thread: open `shop.pbip`, pages, a visual and the Visual tab, the palette, a command,
  the title's unsaved mark. With `--features gpu-tests`,
  `an_agent_takes_a_screenshot_of_the_window` checks the PNG of the window drawn by wgpu
  (`BICRAFT_DESKTOP_SHOTS=DIR` keeps it as `control-screenshot.png`).

## Windows integration (`--register`)

`bicraft-desktop.exe --register` (once, from where the executable stays, e.g.
`%LOCALAPPDATA%\bicraft\bin`) writes, for the current user only (`HKEY_CURRENT_USER`, no
administrator rights):

- the Start menu shortcut `%APPDATA%\Microsoft\Windows\Start Menu\Programs\BIcraft Desktop.lnk`,
  whose AppUserModelID is `BIcraft.Desktop`;
- `HKCU\Software\Classes\Applications\bicraft-desktop.exe`: `FriendlyAppName` "BIcraft",
  `DefaultIcon`, `shell\open\command` (`"…\bicraft-desktop.exe" "%1"`) and `SupportedTypes`
  (`.pbix`, `.pbit`, `.pbip`, `.bim`), so Explorer offers the app in "Open with";
- `HKCU\Software\Classes\.<ext>\OpenWithList\bicraft-desktop.exe` for each of these types.

It never changes the default application of a type: no `ProgId` becomes the default value of
`.pbix`, `UserChoice` is not touched; Power BI Desktop stays the default if it was. Choosing
BIcraft as the default is the user's choice, in "Open with > Always". `--unregister` removes
exactly these keys and the shortcut. Both print what they did (when standard output is
redirected: `bicraft-desktop.exe --register | more`) and answer with their exit code.

The shortcut is written by BIcraft itself from the public specifications [MS-SHLLINK] (Shell
Link Binary File Format) and [MS-PROPSTORE] (the property store carrying
`System.AppUserModel.ID`), without COM (`apps/bicraft-desktop/src/lnk.rs`). The window declares
the same application id; the executable embeds the icon and version information
(`build.rs` draws the icon from `assets/logo/bicraft-icon.svg` at 16 to 256 px).

On Linux, add a `.desktop` file by hand if you want a menu entry (`Exec=bicraft-desktop %f`).

## Building and distribution

`cargo build --release -p bicraft-desktop`. It is a Windows GUI program (no console window). On
Windows the release archive of `cargo xtask dist` carries `bicraft-desktop.exe` next to
`bicraft.exe` (statically linked C runtime, icon required), and `install.ps1` copies both; it
does not run `--register`. `THIRD_PARTY_CRATES.md` lists the crates of both programs, with the
licences of the fonts egui embeds. The Linux archive keeps only `bicraft` (a static musl binary;
a window needs the system's graphics stack): build the app from source there.

Graphics: on Windows the app asks wgpu for DirectX 12 with the FXC shader compiler that ships
with Windows (some OpenGL drivers and a stray `dxcompiler.dll` fail before the first frame);
`WGPU_BACKEND` and `WGPU_DX12_COMPILER` override it.

**UI tests** run without a GPU (egui_kittest, the window's logic and accessibility tree): open a
file, the title, the page tabs, switch pages, the Model and Validation tabs, French, slicer and
data-point clicks, resizing (the page re-renders at the new size, one device pixel per screen
pixel; fit width), live reload, unreadable files, remote mode, the model diagram
(`src/tests/model_tests.rs`: the viewer's layout for a star schema; clicks on a table header, a
measure row and a relationship line open their details; "Show all"; Ctrl+wheel zoom about the
pointer, drag, Fit; French; dark; a model-only `.bim` without an error bar). The shared crate's own
tests (`crates/ui/src/tests.rs`, snapshots handed over by a fixed source) cover what does not
need a file: the desktop and browser shells draw the same frames, the fonts, the Home ribbon per
shell, the title, the empty Report tab of a model-only snapshot, and 400 tables, 5,000 measures,
2,000 relationships and damaged snapshots never panic. Pages render on the test thread with
the bundled fonts only, so the results are the same on every machine. `cargo test -p
bicraft-desktop --features gpu-tests` also draws the window with wgpu and, with
`BICRAFT_DESKTOP_SHOTS=DIR`, saves screenshots (report in English and French, after a slicer
click, a cross-highlight, the Model and Validation tabs; the model diagram in English, French
and dark, with a table and a measure selected); it needs a GPU or a software adapter
and is not part of `cargo xtask ci`.

## Limits

- Reader only: no editing, no saving, no "Open in Power BI Desktop" button yet.
- The same app runs in the browser (`bicraft view --ui egui`, `/w/<id>/view/app/` on a server): [webui.md](webui.md).
- What the viewer does not do, the app does not either ([gaps.md](gaps.md) G29): drill down,
  drill through, bookmarks, report-page tooltips, point selection in matrices, scatter,
  waterfall and ribbon charts.
- Tooltips are the viewer's (rows of the point), not report-page tooltips.
- No OIDC login flow in the app: bring a token.
- Linux: no `--register`, no binary in the release archive.
