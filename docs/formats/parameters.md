# What-if and field parameters (Modeling › New parameter)

Sources: Microsoft Learn ("Create and use what-if parameters", "Let report readers use field
parameters to change visuals"), the public TMSL/TMDL documentation and the PBIR JSON schemas;
shapes marked *unchecked* have not been compared with a file Power BI Desktop saved.

Both parameters are ordinary model objects (calculated tables, columns, a measure) plus a slicer
on a report page. BIcraft writes them with `parameter.what_if` and `parameter.fields`
(`crates/engine/src/parameters.rs`); each command is one edit (one `edit.undo` step) and changes
nothing when any part of it fails.

## What-if (numeric range) parameter

Desktop's dialog takes a name, a data type (whole number, decimal number, fixed decimal number),
a minimum, a maximum, an increment, a default value and whether to add a slicer to the current
page. It creates:

| Object | Shape | Status |
|---|---|---|
| Calculated table `<name>` | partition source `GENERATESERIES(min, max, increment)` | documented |
| Its one column `<name>` | `sourceColumn` `[Value]` (the column `GENERATESERIES` returns, renamed), `int64` for whole numbers, `double` for decimals, `decimal` for fixed decimals, summarize by none | column name documented; `sourceColumn`, types and summarization *unchecked* |
| Extended property on the column | `ParameterMetadata` (type `json`) `{"version": 0}` | *unchecked* |
| Measure `<name> Value` in the table | `SELECTEDVALUE('<name>'[<name>], default)` (no second argument without a default) | documented |
| Slicer on the page | classic `slicer` bound to `'<name>'[<name>]`, `objects.data[0].properties.mode` `'Single'` (the single-value slider) | mode literal *unchecked* |

Learn notes that the slider shows at most 1,000 values (more are sampled evenly); BIcraft warns
above 1,000 and refuses more than 1,000,000. A visual or measure reads the parameter through the
measure; the slider's selection filters the table, so `SELECTEDVALUE` returns it.

## Field parameter

Desktop's dialog takes a name, an ordered list of columns and measures (with editable labels)
and whether to add a slicer. It creates a calculated table whose expression is a table
constructor of one tuple per field, in order:

```dax
{
    ("Total", NAMEOF('Sales'[Total]), 0),
    ("Product", NAMEOF('Sales'[Product]), 1)
}
```

| Column | Source | Properties | Status |
|---|---|---|---|
| `<name>` (the labels) | `[Value1]` | visible; `relatedColumnDetails.groupByColumns: [{groupingColumn: "<name> Fields"}]` (TMDL `groupByColumn:`); sort by `<name> Order` | group-by documented; sort-by *unchecked* |
| `<name> Fields` | `[Value2]`, the text `NAMEOF` gives (`'Sales'[Total]`) | hidden; sort by `<name> Order`; extended property `ParameterMetadata` (type `json`) `{"version": 3, "kind": 2}` | `kind` 2 marks a field parameter; the values *unchecked* |
| `<name> Order` | `[Value3]` | hidden, whole number | documented |

Every column is a `calculatedTableColumn` with `isNameInferred: false`. The slicer is a classic
`slicer` bound to `'<name>'[<name>]`.

`NAMEOF` gives the escaped full name of a column or measure (`'T'[C]`, the measure's home table
for a measure), so renaming a field keeps the parameter working (`docs/formats/dax-functions.md`,
`preview_eval_nameof.rs`).

### How a visual uses it

A visual binds the parameter's label column (`'<name>'[<name>]`). It then shows the fields the
parameter lists, in the parameter's order, that the filters on the parameter's table keep: the
selection of its slicer, a page or report filter on it. With nothing selected, every field. A
slicer bound to the parameter lists its labels.

BIcraft's preview does the same (`crates/engine/src/preview_field_params.rs`): it reads the
fields from the table's expression (no data needed), evaluates the filters on the parameter's
columns (the label, the field's name or the order) against each tuple, and replaces the binding
by the chosen fields, in the same role, before the visual is evaluated. The filters on the
parameter's table are then dropped (the table is related to nothing). A filter it cannot read
keeps every field and is named in `filters_ignored_reason`. Hierarchies of chosen columns
(Desktop's auto-expanding axes) are not modelled.

## Desktop answers (2026-11-23, Power BI Desktop 2.159.1231.0, Store edition)

Black box: a PBIP BIcraft wrote (two what-if parameters, a field parameter with its slicer and a
table bound to its label column) opened in Desktop; then Desktop's own New parameter › Fields
and › Numeric range (0 to 10 by 1, default 5) made on the same model, a table bound to Desktop's
field parameter, and the project saved; the TMDL and PBIR Desktop wrote read back.

- **Model.** Desktop's field parameter table is BIcraft's shape: the same table constructor, the
  label column grouped by the `Fields` column and sorted by the order column, `ParameterMetadata`
  `{"version": 3, "kind": 2}` on the hidden `Fields` column; its order column is hidden with
  `summarizeBy: sum` and `formatString: 0`. Desktop's what-if table is BIcraft's shape too:
  `GENERATESERIES`, `ParameterMetadata` `{"version": 0}` and `summarizeBy: none` on the column
  (with `formatString: 0` for whole numbers), `SELECTEDVALUE(…, default)`; its measure also has
  `formatString: 0`, and its name follows the UI language (`Valeur <name>` in French). Desktop
  drops `dataType`, `isDataTypeInferred` and `isNameInferred` from calculated-table columns when
  it saves BIcraft's tables (they are inferred), and adds a `PBI_Id` annotation to its own tables.
- **What-if slicer.** BIcraft's slicers draw like Desktop's own: a single-value box with a slider.
  Desktop writes no `mode` for its own (it adds an empty Advanced filter card on the column);
  BIcraft's `'Single'` mode is kept as written.
- **A visual bound to a field parameter.** Desktop does **not** treat a table bound only to the
  label column (`Pick[Pick]`, what `visual.add` writes) as a field parameter: it shows the labels
  as text values. Desktop's own table bound to its parameter lists the parameter's fields as
  projections (each with `displayName` the label) and names the parameter in the role:

  ```json
  "Values": {
    "projections": [ {Sales.Region}, {Sales.Product}, {Sales.Total} ],
    "fieldParameters": [ {"parameterExpr": {"Column": {… "Entity": "Paramètre"}, "Property": "Paramètre"}},
                          "index": 0, "length": 3} ]
  }
  ```

  (the public `visualConfiguration` schema's `RoleFieldParameter`: `parameterExpr`, `index`,
  `length`, `sortDirection`). Each projection is the field's own (`queryRef` `Sales.Region`,
  `nativeQueryRef` `Region`) with `displayName` its label in the parameter.

### How BIcraft writes and reads it (since 2026-11-23)

- **Reading.** A `fieldParameters` entry whose `index` and `length` are a range of the role's
  projections is read as one binding of `parameterExpr` (the label column) in place of that range
  (`bicraft_report::query::derive`), so the visual is "bound to the parameter" for the preview,
  the Build pane and edits. An entry that is no such range leaves the projections as they are.
- **Writing.** After every successful edit, a visual (slicers aside) bound to a field
  parameter's label column gets that role's range rewritten from the model, in the shape above:
  one projection per field the parameter lists, in its order, labelled, and the
  `fieldParameters` entry (`crates/engine/src/field_param_queries.rs`). A new binding, or an edit
  of the parameter's fields (its table's expression), updates the visual in the same undo step;
  a file opened is not rewritten until it is edited. Regenerating a query after another edit
  keeps the range and moves its `index`; removing the parameter's binding drops the entry.
- **Checked in Desktop** (2026-11-23, 2.159.1231.0): see "Desktop answers, second session"
  below.

## Desktop answers, second session (2026-11-23, Power BI Desktop 2.159.1231.0, Store edition)

Black box: a PBIP BIcraft wrote with this branch (Sales: Region, Product, Amount; a field
parameter `Pick` listing `Sales[Region]`, `Sales[Product]` and `[Total]` labelled `Amount total`,
with its slicer, and a table bound only to `Pick[Pick]` through `visual.add`) opened in Desktop
and its data refreshed. The table showed the three fields as columns (`Region`, `Product`,
`Amount total`) with the data's rows and a total, not the labels as text; ticking `Product` in
the slicer left the table with the Product column alone. The file was not saved from Desktop.

## Literal loading

The preview loads both tables without the DAX evaluator: `GENERATESERIES` over literal numbers
and `NAMEOF('T'[C])` inside a table constructor are literal tables for `bicraft-data`'s loader
(`crates/data/src/dax.rs`), so a new parameter's slicer has its values before any refresh.
