# TMDL view and TMDL scripts

Sources: Microsoft Learn, [Work with TMDL view in Power BI Desktop](https://learn.microsoft.com/en-us/power-bi/transform-model/desktop-tmdl-view)
(script objects from the Data pane or the Model explorer, script tabs, Apply, the success and
failure notifications, scripts saved with a PBIP project) and [TMDL scripts](https://learn.microsoft.com/en-us/analysis-services/tmdl/tmdl-scripts)
(`createOrReplace`, `ref`, one command per script). The TMDL syntax itself is in
[tmdl.md](tmdl.md). Nothing here was compared with a script Desktop wrote.

## Script

A script is one command and the objects under it, indented one tab:

```
createOrReplace

	ref table Sales

		measure Total = SUM(Sales[Amount])
			formatString: 0

	function Half = (x : NUMERIC) => x / 2
```

- `tmdl.script {objects?}` scripts model objects (`model`, `table`, `column`, `measure`,
  `hierarchy`, `partition`, `function`, `expression`, `relationship`, `role`, `perspective`,
  `culture`; members name their `table`). A table is scripted whole; a member is scripted under
  `ref table T`, consecutive members of one table under one `ref table`. Without objects, the
  whole model (database and model first). Output uses LF line endings.
- Learn: scripting fills the tab shown when it is empty, else opens a new tab. Scripts can also be
  copied (BIcraft's Copy button puts the tab's text on the clipboard).

## Apply (createOrReplace)

`tmdl.apply {script, force?}`:

- Each object the script declares is created, or replaces the model's object of that kind and
  name **with all its descendants** (a scripted table without one of its measures loses it).
  Object order does not matter.
- `ref table T` declares nothing itself: its child columns, measures, hierarchies and partitions
  replace or add T's; an unknown T is an error.
- `database` or `model` replaces the whole model.
- The script is parsed, merged into a copy of the model, and the DAX of every object it touched is
  validated (measures, calculated columns and tables, functions, row filters) before anything
  changes; a parse error, an unknown `ref`, an object kind it cannot apply, or invalid DAX
  (unless `force`, which keeps it with warnings) changes nothing. Errors read `line L, column
  C: message`, positions in the script.
- A successful Apply is one undo step and answers `{applied: [{kind, name, table?, action:
  added|replaced}], warnings}`.

## UI

The TMDL view (its status bar icon; the palette's Script TMDL and Apply TMDL script) has script tabs along the bottom (new,
rename, duplicate, close), the editor, **Apply** (Ctrl+Enter), **Script model** and **Copy**.
Script TMDL is on the Data pane's table and field menus and on the Model explorer's model,
table, measure and function menus. A failed Apply shows Error with the message and Go to line L,
column C, which puts the editor's cursor there.

Not done: Desktop's Preview (the diff before Apply), the Problems pane, the compatibility-level
upgrade prompt, and saving the script tabs in a PBIP project's `TMDLScripts` folder
([docs/gaps.md](../gaps.md)).
