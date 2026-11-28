# Groups and bins (New group)

Sources: Microsoft Learn, [Use grouping and binning in Power BI Desktop](https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-grouping-and-binning)
(the dialog, `(groups)` and `(bins)` names, the Other group, bins on numbers and dates); the
TMSL/TMDL column schema on Microsoft Learn (calculated columns, `extendedProperties` of type
`json`). The stored DAX below is BIcraft's own; the `GroupingMetadata` property name and shape
are BIcraft's reading of TMDL folders written by Power BI Desktop; for list groups and bins they
match what Desktop 2.159.1231.0 saved on 2026-11-23 (see "Desktop answers").

## What Desktop does (Learn)

- New group on a column of the Data pane (or Column tools › Data groups) opens the Groups
  dialog: a name, the field, the group type (List or Bin).
- **List**: values are moved from *Ungrouped values* into named groups (*Groups and members*);
  *Include Other group* sends every value no group lists to `Other`. The new field is named
  `<column> (groups)`.
- **Bin**: numbers and dates. A bin size (or, for numbers, a number of bins); dates take a size
  in units. The new field is named `<column> (bins)`. Measures cannot be binned.
- The group is a field of the table: it can be renamed, edited (Edit groups) and used in any
  visual.

## How BIcraft stores a group

A calculated column of the grouped column's table, `summarizeBy: none`, with one `json`
extended property:

```
column 'Product (groups)' =
        SWITCH(
            TRUE,
            ISBLANK('Sales'[Product]), "(Blank)",
            'Sales'[Product] IN {"Bike", "Car"}, "Wheels",
            "Other"
        )
    summarizeBy: none

    extendedProperty GroupingMetadata =
            {
              "version": 0,
              "groupedColumns": [
                {"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Product"}}
              ]
            }
```

### List groups (`column.group {kind: "list", groups, other}`)

- One `SWITCH(TRUE, …)` branch per group, in order: `c IN {v1, v2…}`; a group listing a blank
  value adds `ISBLANK(c) ||` to its condition. Values are literals of the column's type (text,
  numbers, `TRUE`/`FALSE`, dates as `DATE(y, m, d)`); texts match without case, as the engine
  compares them, and one value may be in one group only.
- A blank no group lists: a first branch `ISBLANK(c), "(Blank)"`.
- The last argument: `"Other"` (Include Other group), else the column itself (a text column) or
  `c & ""` (other types), so each ungrouped value keeps its own value. The column is text.

### Bins (`column.group {kind: "bin", bin_size | bin_count, unit}`)

Since 2026-11-23 the expressions, unit codes, format strings and design state are the ones Power
BI Desktop 2.159.1231.0 wrote for the bins made in its own Groups dialog (see "Desktop answers").

| Bins | Expression (inside `IF(ISBLANK(c), BLANK(), …)`) | `binSize.unit` | `TimeUnit` | Format string |
|---|---|---|---|---|
| numbers by size `s` | `IF(c >= 0, ROUNDDOWN(c / s, 0) * s, ROUNDUP(c / s, 0) * s)` (toward zero) | 0 | — | — |
| numbers into `n` bins | `VAR __Count = n`, `__Min = MIN(c)`, `__Max = MAX(c)`, `__Size = IF(__Max - __Min > 0, (__Max - __Min) / __Count, 1)`, then `__Min + MIN(INT((c - __Min) / __Size), __Count - 1) * __Size` (the bins follow the data) | no `binningMetadata` | — | — |
| dates by `k` years | `DATE(INT(YEAR(c) / k) * k, 1, 1)` (`DATE(YEAR(c), 1, 1)` for 1) | 4 | 3 | `yyyy` |
| dates by `k` months | `DATE(YEAR(c), 1 + (INT((MONTH(c) - 1) / k) * k), 1)` (each year restarts) | 6 | 2 | `mmmm yyyy` |
| dates by `k` days | `1 + (INT((INT(c) - 1) / k) * k)` (day serial numbers) | 8 | 0 | `Short Date` |
| dates by `k` hours | `(INT((c * 24) / k) * k) / 24` | 9 | 7 | `General Date` |
| dates by `k` minutes | the same with 1440 | 10 | 6 | `General Date` |
| dates by `k` seconds | the same with 86400 | 11 | 5 | `General Date` |

Numbers by size stay whole numbers when the column and `s` are whole, else decimal; a number of
bins is decimal; date bins are date/time. `binningMetadata` is added to `GroupingMetadata` as
`{"binSize": {"value": s, "unit": u}}` (`value` a JSON decimal, `2.0`), except for a number of bins.
The `GroupingDesignState` annotation of a bin holds the grouped column like a list group's, and a
`BinItem` instead of `GroupItems`:

```
{"Version":0,"Sources":[{"Name":"s","Entity":"Sales"}],"GroupedColumns":[<s.Amount>],
 "BinItem":{"Expression":{"Floor":{"Expression":<s.Amount>,"Size":100.0}}}}               numbers by size
 "BinItem":{"Expression":{"Floor":{"Expression":<s.Day>,"Size":2.0,"TimeUnit":2}}}         dates (2 months)
 "BinItem":{"Expression":{"Member":{"Expression":{"Discretize":{"Expression":<s.Qty>,"Count":4}},
                                    "Member":"Min"}}}                                       4 bins
```

`Floor` and `Discretize` are the public `semanticQuery` schema's `QueryFloorExpression` and
`QueryDiscretizeExpression`; `TimeUnit` is its enum (0 day, 2 month, 3 year, 5 second, 6 minute,
7 hour). The `binSize.unit` codes are a separate enum Desktop writes in `GroupingMetadata`; files
BIcraft wrote before 2026-11-23 used 1, 2 and 3 for years, months and days, still read as such.

### Reading a group back

`column.group` with `replace: true` rewrites an existing group column (Edit groups). The snapshot
gives a group column's definition as `group` (`docs/view-snapshot.md`): the grouped column and
kind; for a list, the groups and `other` read back from a `SWITCH(TRUE, …)` of the form above;
for bins, the size and unit from `binningMetadata`, a number of bins from the design state's
`Discretize` (or the `binCount` older BIcraft files carry). A list group whose expression has
another form gives its column and kind only.

## Evaluation

The column is an ordinary calculated column: the evaluator computes calculated columns that the
data does not hold (literal partitions, or added after the data was stored) once per row, kept
until the model or data changes (`docs/view-snapshot.md`). A structural PBIX save stores the
computed values like any calculated column.

## Desktop answers (2026-11-23, Power BI Desktop 2.159.1231.0, Store edition)

Black box: a PBIP BIcraft wrote (a list group with Other, number bins by size, date bins by
month) opened in Desktop; Desktop's Groups dialog driven through mouse input; the TMDL Desktop
saved read back.

- The three group columns load and compute as BIcraft's preview does; the Data pane and the
  field lists show them with Desktop's group icon.
- **List group.** Edit groups opens on it (name, field, type List, the column's values) but
  lists no group: Desktop reads the groups from the `GroupingDesignState` annotation, not from the
  DAX. After North and South were grouped and OK pressed, Desktop wrote:

  ```
  column 'Region (groups)' =
          SWITCH(TRUE, ISBLANK('Sales'[Region]), "(Blank)",
                 'Sales'[Region] IN {"North", "South"}, "North & South", 'Sales'[Region])
      summarizeBy: none
      extendedProperty GroupingMetadata = {version 0, groupedColumns: [Sales.Region]}  (as BIcraft's)
      annotation SummarizationSetBy = User
      annotation GroupingDesignState = {"Version":0,"Sources":[{"Name":"s","Entity":"Sales"}],
        "GroupedColumns":[{"Column":{"Expression":{"SourceRef":{"Source":"s"}},"Property":"Region"}}],
        "GroupItems":[{"DisplayName":"(Blank)","BlankDefaultPlaceholder":true},
          {"DisplayName":"North & South","Expression":{"In":{"Expressions":[<the column>],
            "Values":[[{"Literal":{"Value":"'North'"}}],[{"Literal":{"Value":"'South'"}}]]}}}]}
  ```

  (condensed here; Desktop's labels are in the UI language, `(Vide)` in French). With Include
  Other group ticked, the last argument is the Other label and the items end with
  `{"DisplayName": "<Other>"}`. Desktop's SWITCH is BIcraft's form (Desktop names a group of
  several values by joining them with ` & `).
- **Since 2026-11-23** `column.group` writes this annotation for a list group (items: the blank
  placeholder, one per group with its values as query literals — `'text'`, `1L`, `1D`, `1M`,
  `true`, `datetime'…'` — and Other when on; the source alias is the table name's first letter
  in lower case, as Desktop's `s` for `Sales`). Only text values were seen in Desktop's file; the
  other literal kinds follow the public `semanticQuery` literal forms. A group that lists a blank
  holds it as an explicit member (BIcraft's form since 2026-11-27, below).
- **A blank in a list group** (second session, same day): on a text column with a null row
  (the row loaded, its other columns binned), Desktop's Ungrouped values listed only the
  non-blank values; the blank is never a value a group can take, only the `(Blank)` placeholder
  item the SWITCH sends it to. BIcraft's `values: [null]` (a blank grouped with other values) has
  no Desktop equivalent; its design state is BIcraft's own form (below).
- **Bins (first session).** Edit groups on BIcraft's number bins opened with the type Bin but an
  empty bin type: Desktop did not read BIcraft's `binningMetadata`. Opening Edit groups on the
  date bins showed an empty dialog, after which Desktop reported an error rendering the report.
- **Bins (second session).** A PBIP BIcraft wrote (Sales: Region with a null, Amount, Qty, Day
  as a date) opened in Desktop; New group made, through the dialog driven by input events:
  Amount bins of size 100, Qty in 4 bins (Desktop's dialog proposes `(max - min) / n` as the
  size it shows), Day bins of sizes 1 and 2 in months, 1 and 5 in years, 10 in days, 12 in hours, 15 in minutes
  and 30 in seconds (Desktop offers years, months, days, hours, minutes and seconds for a date column);
  the TMDL Desktop saved read back. The table under "Bins" above is what it wrote. Desktop
  writes no `dataType` on year and month bins (inferred), `dateTime` on the others, and
  `annotation SummarizationSetBy = Automatic` on its bins.
- **Re-check (third session).** A PBIP BIcraft wrote with these forms (Amount bins of 100, Qty
  in 4 bins, Day bins of 2 months and of 12 hours, a list group of North and South with Other)
  opened in Desktop: Edit groups opened on each with its definition filled in — type Bin, bin
  type Bin size and 100; Number of bins and 4 (size 1.25 shown); Bin size, 2 and Months; Bin
  size, 12 and Hours; type List with `North & South` and `Other` under Groups and members and
  East ungrouped. No error was reported drawing the report.

BIcraft's evaluator stores a calculated column's numbers as dates when the column is date/time
(`preview_eval_rows.rs` `as_declared`), as the engine does for Desktop's day and time bins,
whose DAX returns serial numbers.

## BIcraft's own forms (2026-11-27, not compared with Desktop)

- **A blank listed in a list group** is an explicit member of its group: the SWITCH branch is
  `ISBLANK(c) || c IN {…}` (above), and the design state lists the public `semanticQuery` null
  literal, `[{"Literal": {"Value": "null"}}]`, among the group's `In` values, with no `(Blank)`
  placeholder item (the blank is no longer left to it). The other choice, the blank sent to
  Other, would lose the group's member on a round trip. Desktop's dialog never lists a blank, so
  no Desktop file has this form; BIcraft writes and reads it back (PBIP tested).
- **A number of bins on dates** (`column.group {kind: bin, bin_count: n, unit?}` on a date
  column) is written as a bin size of the date form above: the span of the data's dates in the
  unit (calendar years or months touched, days, hours, minutes or seconds) divided by `n`,
  rounded up. Without a unit, the largest unit the span holds `n` times. The size is fixed when
  the group is made (it does not follow a refresh, unlike the numbers' `Discretize`), and the
  bins start on the unit's usual boundaries, so the data may fall into one bin more or fewer
  than `n`. Whether Desktop offers a number of bins on dates, and its form, were not checked; no
  `binCount` or other metadata is added.
- **Group data** (`visual.group_points`, the menu of a data point): the data points selected in
  a visual (Ctrl+click), else the point right-clicked, give the values of their category column
  (the first grouping value of the first point); `column.group` makes a list group of them,
  named by joining the values with ` & ` as Desktop names a group of several values, in the
  column `<column> (groups)`. When that column already groups the same column, the new group joins
  its groups (its values leave the groups that listed them). Desktop also adds the group to the
  visual's Legend (Microsoft Learn); BIcraft leaves the visual's fields as they are.
