# Visual calculations (as implemented in BIcraft)

Status: implemented 2026-11-19 (M11). A visual calculation is a DAX expression stored on one
visual and evaluated over the visual's own data (its *visual matrix*), not over the model.
BIcraft stores, checks, evaluates, shows and round-trips them; the parts it does not evaluate
are listed below and in [../gaps.md](../gaps.md) G71.

**Sources.** Public documentation only: Microsoft Learn *Using visual calculations in Power BI
Desktop* (the visual matrix, axes, `RESET`, the templates, hiding fields, the edit mode and its
limitations) and the DAX reference pages of the visual calculation functions (`RUNNINGSUM`,
`MOVINGAVERAGE`, `PREVIOUS`, `NEXT`, `FIRST`, `LAST`, `COLLAPSE`, `COLLAPSEALL`, `EXPAND`,
`EXPANDALL`, `ISATLEVEL`, `RANGE`, `LOOKUP`, `LOOKUPWITHTOTALS`, `OFFSET`, `INDEX`, `WINDOW`,
`RANK`, `ROWNUMBER`); Microsoft's published PBIR
schemas vendored in `crates/pbir/schemas` (semanticQuery 1.0.0–1.4.0 `QueryNativeVisualCalc`,
visualContainer `hidden` and `nativeQueryRef` on a projection). No Desktop run and no DAX engine
was used: every expected value in the tests is computed by hand from those pages. **Not checked
against the engine.**

## Storage

A calculation is a projection of the visual's query whose field is a native visual calculation:

```json
{"field": {"NativeVisualCalculation": {"Language": "dax", "Expression": "RUNNINGSUM([Total])",
                                       "Name": "Running sum"}},
 "queryRef": "Running sum", "nativeQueryRef": "Running sum"}
```

It sits in the well (role) of the visual's values, after that role's last field unless a role is
given. A field hidden from the visual (any field, calculation or not) keeps its projection with
`"hidden": true`; it stays in the query and in the visual matrix, so calculations can use it.
The model sees neither: `FieldRef::Other` keeps the projection's JSON, so a PBIR folder and a
PBIX (whose legacy layout carries the same projection) reopen with the calculations and the
flags unchanged. The `queryRef` BIcraft writes is the calculation's name; Desktop's own choice is
not documented (G71).

## Binding (`crates/dax/src/visual_calc.rs`)

The expression is parsed by the DAX parser, with the axis keywords merged first (`ROWS COLUMNS`
is one argument). Names in brackets refer to the visual's fields: by `nativeQueryRef`, else the
field's display name, else its default name (`Sum of Amount`, the measure's or column's name).
Diagnostics: a model reference (`Sales[Amount]`, a table), a name not on the visual, the
calculation itself or a cycle through others, an unknown function, a wrong number of arguments,
a function the documentation lists as unavailable in visual calculations. The engine evaluates
calculations in their dependency order.

## Evaluation (`crates/engine/src/preview_viscalc.rs`)

The visual is evaluated without its calculations; its rows, total and subtotals give the visual
matrix: one level per number of row-axis and column-axis grouping fields kept, the leaves being
the visual's rows. Missing levels (the total and subtotals a table or chart does not show) are
evaluated too when an expression navigates. The calculation's values are then inserted in the
data as columns of kind `visual_calculation`, at their place among the fields.

| Part | Evaluated |
|---|---|
| Axes | `ROWS`, `COLUMNS`, `ROWPAGES` (the small multiples) and their combinations (`ROWS COLUMNS`, `ROWS ROWPAGES`…: one sequence, the last axis outermost); an axis the visual lacks is ignored; small multiples partition unless `ROWPAGES` is named |
| Reset | `NONE`, `HIGHESTPARENT`, `LOWESTPARENT`, an integer (absolute when positive, relative when negative), a field of the axis |
| Order | along each axis, by the grouping fields' values, or by their model sort-by column when they have one (a month name by its month number); `ORDERBY` of fields or expressions (`ASC`/`DESC`), ties broken by the axis order; `PARTITIONBY` of the visual's fields; `MATCHBY` of the visual's fields (those the level keeps must single out the current cell in its partition, else an error in that cell); blanks `DEFAULT`, `FIRST`, `LAST` |
| Window shortcuts | `RUNNINGSUM`, `MOVINGAVERAGE` (window size, include current), `PREVIOUS`, `NEXT` (steps), `FIRST`, `LAST` |
| Window functions | `OFFSET`, `INDEX` (1-based, negative from the end, 0 or blank: no cell), `WINDOW` (`ABS`/`REL` bounds, cut to the partition) as the table of `CALCULATE` and of the `X` functions; `RANK` (`SKIP`, `DENSE`), `ROWNUMBER` (blank on a total of the walked axes) |
| Lookups | `LOOKUP` (fields not filtered keep the current cell's values; `EXPLICIT`; with `INFERRED`, a field on the axis of a filtered field that has one value among the cells matching the filters takes that value instead of the current cell's), `LOOKUPWITHTOTALS` (fields not filtered are at their total); one matching cell, else an error in that cell (blank) |
| Navigation | `COLLAPSE`, `COLLAPSEALL`, `EXPAND`, `EXPANDALL` (scalar form, axis and levels, or fields), `ISATLEVEL`, `RANGE` (as the table of `CALCULATE` and of the `X` functions) |
| Aggregations over the matrix | `SUM`, `AVERAGE`, `MIN`, `MAX`, `COUNT`, `COUNTA`, `COUNTBLANK`, `DISTINCTCOUNT`, `COUNTROWS`, `SUMX`, `AVERAGEX`, `MINX`, `MAXX`, `COUNTX`, `COUNTAX` |
| Scalars | arithmetic, comparison, `&`, `IF`, `SWITCH`, `DIVIDE`, `BLANK`, `ISBLANK`, `AND`, `OR`, `NOT`, `COALESCE`, two-argument `MIN`/`MAX`, `ABS`, `SQRT`, `INT`, `SIGN`, `EXP`, `LN`, `ROUND`, `POWER`, `MOD`, `VAR … RETURN` |
| Not evaluated | `MATCHBY` of anything but the visual's fields, a model table as a window function's relation, `COLLAPSE`/`EXPAND` along `ROWPAGES`, `LOOKUPWITHTOTALS` without a value for each small multiple (the matrix has no level without them), any other function |

A calculation that is not evaluated keeps an empty column whose `reason` says why ("COLLAPSE along
ROWPAGES is not evaluated by BIcraft"); the visual's other values stay. Division by
zero is blank, as in DAX. Rows beyond the visual's row limit are not in the matrix. Window
functions visit at most 20 million cells per calculation and positions are capped, so a long
axis or a huge position gives a reason, never an overflow. The semantics come from the Learn
pages of each function (`LOOKUP`, `LOOKUPWITHTOTALS`, `OFFSET`, `INDEX`, `WINDOW`, `RANK`,
`ROWNUMBER` and the overview's Axis, Reset, OrderBy and PartitionBy sections); the test values
are computed by hand from them, **not checked against the engine**.

`LOOKUP … INFERRED` (2026-11-24): Learn says the engine fills in the axis columns that the
supplied coordinates determine functionally (a day from a year, quarter and month). BIcraft reads
"determined" from the visual matrix itself: a field on the axis of a supplied field that holds a
single value among the cells matching the supplied values; a field on another axis, or one with
several values there, keeps the current cell's value as with `EXPLICIT`. A supplied value that
conflicts with the inferred one matches no cell (blank). `MATCHBY` (2026-11-24): in the visual
matrix every cell is already its own row, so `MATCHBY` changes no value; it checks that its
fields single out the current cell within the partition (an error in that cell otherwise, as
DAX gives when the columns do not identify the current row).

## Display

Tables, matrices and charts draw calculations as any other value. Hidden fields are dropped by
the renderer (`crates/render/src/data.rs`, `drop_hidden`) with their cells, totals, styles,
highlights and error bars; the snapshot keeps them, flagged `hidden`.

## Commands and UI

`visual.calculation.add {page, visual, expression, name?, role?, hidden?, force?}`,
`visual.calculation.set {page, visual, name, expression?, new_name?, hidden?, force?}` (the
`hidden` switch works on any field; a rename is refused while another calculation refers to
it) and `visual.calculation.remove {page, visual, name, force?}` (refused while referred to,
unless `force`). Slicers, text boxes, Q&A, key influencers, decomposition trees and script
visuals refuse calculations, as the Learn page lists. In the app, a visual's More options ›
New calculation opens the edit window (`crates/ui/src/viscalc_dialog.rs`): the visual's fields
with a switch to hide each, a name, the templates, the formula bar, and the visual matrix as
evaluated; Apply runs add or set, the bin runs remove.
