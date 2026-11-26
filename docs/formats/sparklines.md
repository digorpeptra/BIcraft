# Sparklines and the visual's Data format (as implemented in BIcraft)

Status: implemented 2026-11-23 (G106), checked against Power BI Desktop 2.159.1231.0 the same
day and changed to write what it writes. Two field options of a visual that Power BI Desktop
2.159.1231.0 offers (seen in its UI inventory, [../parity-checklist.md](../parity-checklist.md)
"Desktop UI inventory"): the Format pane's **General › Data format** card and **Add a
sparkline** in a table or matrix. What is left is in [../gaps.md](../gaps.md) G106.

**Sources.** Public documentation only: Microsoft Learn *Create sparklines in a table or matrix
in a Power BI report* (git commit c2aea2c of MicrosoftDocs/powerbi-docs, 2026-11-20: Y axis,
summarization, X axis, line or column, colour, width, markers, up to five sparklines per visual
and 52 points per sparkline, calculation groups applied to individual values or the whole
sparkline); Microsoft's published PBIR schemas vendored in `crates/pbir/schemas`
(`semanticQuery` 1.0.0–1.4.0 `QuerySparklineDataExpression`, `visualConfiguration` /
`visualContainer` `RoleProjection.format` "format string scoped to the visual"); the
`sparklines` object of Microsoft's report theme schema vendored in `crates/visuals/schemas`
(`chartType` line or column, `dataColor`, `markerColor`, `markerShape`, `markerSize`,
`markers` "Show these markers", `strokeWidth`); and black-box runs of Power BI Desktop
2.159.1231.0 (Store, French UI) on 2026-11-23: a PBIX and a PBIP BIcraft wrote (a table and a
matrix with line and column sparklines over a date and a text X axis, markers, and tables whose
fields carry visual formats) were opened, their Edit sparkline dialog, Sparklines card and Data
format card read through the accessibility tree, sparklines and formats added in Desktop's own
dialogs and cards, and the PBIR Desktop saved compared with BIcraft's ("Seen in Desktop" below).

## Data format

A visual's own format string for one field is the projection's `format`:

```json
{"field": {"Aggregation": {"Expression": {"Column": {…}}, "Function": 0}},
 "queryRef": "Sum(Sales.Amount)", "nativeQueryRef": "Sum of Amount", "format": "#,0.0"}
```

The legacy layout of a PBIX keeps it as `columnProperties[queryRef].formatString` (already read
and written by `crates/layout-bb`). The model's own format string is not changed. The engine's
preview sets the data column's `format_string` to it (flagged `visual_format`), and the renderer
draws the values with it.

`visual.set_data_format` builds the string from the card's choices, in the strings Desktop
2.159.1231.0's card writes or recognizes ("Seen in Desktop" below):

| Type | Written | Desktop's card shows it as |
|---|---|---|
| auto | no `format` (the model's applies) | — |
| general | `G` (Desktop's) | General |
| whole | `#,0` or `0` (Desktop's `0`) | Whole number |
| decimal | `#,0.00` (decimals 0–15, thousands on or off) | Decimal number |
| percentage | `0.00%`, thousands off by default as in Desktop's card | Custom (fr-FR UI, see below) |
| currency `$` | `\$#,0.00;(\$#,0.00);\$#,0.00` | Currency, "$ English (United States)" |
| currency `€` | `#,0.00 "€";-#,0.00 "€";#,0.00 "€"` (Desktop's) | Currency, "€ Euro (123 €)" |
| other currency | symbol then number: `£#,0.00`, `"CHF "#,0.00` | Custom (drawn right) |
| scientific | `0.00E+000` (Desktop's) | Scientific |
| date | Short Date `d`, Long Date `D`, General Date `G`, Long Time `T`, Short Time `t` (Desktop's); any other code as given (`dd/MM/yyyy`) | the named format, or the code's own entry |
| custom | the code as typed (at most 255 characters, the schema's `maxLength`) | Custom |

Scientific and General formats, and a negative value under a format with its own negative
section, are drawn with BIcraft's VB `FORMAT` rules (`bicraft_locale::vb`); the others with the
renderer's own number formatting; `d`, `D`, `G`, `T`, `t` on a date as the named formats. Desktop
draws every one of them as BIcraft does (2026-11-23, fr-FR, cell by cell: "15/01/2025",
"mercredi 15 janvier 2025", "15/01/2025 00:00:00", "195,0 €", "$10,00", "3,25E+001",
"1000,00%").

## Sparklines

A sparkline is a projection of the table's or matrix's **Values** role whose field is a
`SparklineData` expression:

```json
{"field": {"SparklineData": {
    "Measure": {"Aggregation": {"Expression": {"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Amount"}}, "Function": 0}},
    "Groupings": [{"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Month"}}],
    "ApplyCalculationGroupTo": "Point"}},
 "queryRef": "SparklineData(Sum(Sales.Amount)_[Sales.Month])",
 "nativeQueryRef": "Sum of Amount by Month"}
```

`Measure` is the Y axis (a measure, or a column with its summarization), `Groupings` the X axis
(a column), `ApplyCalculationGroupTo` `Point` by default, as Desktop's Add dialog writes it
(`null` leaves it out: the schema's default, the whole sparkline). `PointsPerSparkline` (the
schema's constant 52) is not written, as in Desktop, and is read when present. The `queryRef`
is Desktop's spelling; the `nativeQueryRef` follows Desktop's pattern in English ("Sum of Amount
by Month"; Desktop's French UI writes "Somme de Amount par Day").

Its look is an `objects.sparklines` entry selected by the projection's `queryRef`
(`{"selector": {"metadata": queryRef}}`), with the theme schema's properties: `chartType`
(`'line'`, `'column'`), `dataColor`, `strokeWidth`, `markers`, `markerColor`, `markerShape`,
`markerSize`. An entry without selector applies to every sparkline of the visual (the Format
pane's Sparklines card writes it). `markers` is a double literal (`136D`), a set of bits, as
Desktop writes it: 1 every point, 2 highest, 4 lowest, 8 first, 16 last, 32 negative, 128
isolated. Without `markers` Desktop marks the isolated points only (its card shows Isolated
ticked); the renderer does the same. An `L` literal (BIcraft's earlier files) reads the same.

### Evaluation

The preview evaluates the visual without its sparklines, then, per sparkline, one flat table of
the visual's grouping columns (a table's columns; a matrix's row and column fields), the X axis
and the measure, under the visual's filters. Each data row gets the points of its group:
`{"sparkline": [[x, y], …]}`, sorted by X (numbers by value, text, so ISO dates, as text; a
sort-by column is not followed), at most 52 (the first ones). The total row gets the points of
one more flat table without the grouping columns (the X axis and the measure under the visual's
filters: the total's filter context), as Desktop draws a sparkline in a table's and a matrix's
Total row ("Seen in Desktop" below). A matrix's subtotal levels (`data.subtotals`, one per
depth of its row and column hierarchies) each get the points of a flat table grouped by the
fields that level keeps, so a member's row or column total has its own sparkline (2026-11-24).
A total whose points cannot be evaluated stays blank. When the X axis is one of the visual's own rows, each row would have one
point: the column carries a `reason` instead (a total over that axis still has its points).

### Drawing

`bicraft_render` draws each cell as a small line (a blank point breaks it) or a column per point
from zero, in the sparkline's colour (the theme's first data colour by default), with the markers
it asks for, the Total row's and a matrix's subtotals' cells too. A sparkline column is at least
seven text heights wide and is never summed. Each cell has its own Y range. A line whose every X
in the column (rows, totals and subtotals) is a number or a date (an ISO text, as its OLE serial)
puts each point at its X on the column's range, from the smallest X to the largest, so cells
share one X scale as in Desktop ("Seen in Desktop" below; one X in the whole column: the middle);
a text X axis, and a column sparkline, spread their points evenly over the cell.

### Seen in Desktop (2.159.1231.0, 2026-11-23)

- **Sparklines BIcraft wrote** open from a PBIX and from a PBIP and draw with the expected
  shapes: a line or columns per row, the chosen colour, square markers of size 5, width 2, markers
  at the expected points once the bits above are used; a date and a text X axis both work.
  Desktop also draws a sparkline in the table's and the matrix's Total row (BIcraft does since
  2026-11-24).
- **Matrix subtotals (2026-11-24).** A matrix with Dept › Name rows, Tier columns and a line
  sparkline of Total by Day (markers highest and lowest), expanded one level: Desktop draws a
  sparkline in every leaf row, each Dept subtotal row, the column subtotal (the Total column
  group) and the grand total row, as BIcraft does. Its rendered DOM (each cell an SVG path)
  gives the placement: X is the point's date on one range for the whole visual, the earliest to
  the latest Day of all cells, mapped from 4 px to the cell's width less 6 px (a 65 px cell: 4 to
  59), so a row whose days fall in May and June draws in the right part of its cell and a single
  point stands at its date; Y is the cell's own range, from its lowest value (4 px above the
  bottom) to its highest (4 px from the top), one point at mid-height. BIcraft spread each cell's
  points over the whole cell until then (fixed the same day, "Drawing" above).
- **Edit sparkline dialog** (the Values well's field menu › Edit sparkline) shows BIcraft's
  sparkline as Y axis Amount, summarization Sum, X axis Day, "Apply calculation group to" the
  whole sparkline (no `ApplyCalculationGroupTo` written). The Add dialog starts on "Individual
  values" and writes `Point`.
- **Desktop's own sparklines** (Add dialog on a new table: Sum of Amount by Day, Total by Month)
  are written as BIcraft now writes them: the same `SparklineData` fields without
  `PointsPerSparkline`, `ApplyCalculationGroupTo: "Point"`, queryRefs
  `SparklineData(Sum(Sales.Amount)_[Sales.Day])` and `SparklineData(Sales.Total_[Sales.Month])`,
  look entries selected by `metadata` = that queryRef, `chartType 'column'`.
- **Marker bits**, from the Sparklines card's checkboxes (every point, high, low, first, last,
  negative, isolated) over BIcraft's values and from what Desktop saved: 3 shows every point +
  highest; first + isolated saved `136D`; negative and isolated ticked over 3 saved `163D`; 12
  drew lowest + first, 16 the last point only.
- **Data format card** lists the visual's fields, sparklines included (a sparkline offers only
  Text), with General, Whole number, Decimal number, Currency, Percentage, Scientific, Custom
  (a date list for dates). Desktop saved: General `G`; Scientific `0.00E+000`; Short Date `d`;
  General Date `G`; Whole number `0`; Decimal number with 0 decimals `#,0`; Currency in the fr-FR
  default "€ Français (France)" `#,0.0 "€";-#,0.0 "€";#,0.0 "€"`; Percentage
  `0.00 %;-0.00 %;0.00 %` (`#,0.0 %;-#,0.0 %;#,0.0 %` with thousands). On opening it recognized
  `0.000`, `#,0.00`, `#,0`, `d`, `D`, `T`, `t`, `dd/MM/yyyy`, the two currencies of the table above
  and `0.00 %;…`, and showed as Custom `€#,0.0`, `€#,0.0;-€#,0.0;€#,0.0`, `#,0.0%`,
  `#,0.0%;-#,0.0%;#,0.0%`, `0.00%`, `General`, `0.00E+00`; `G` on a date showed an empty choice
  (yet it is what Desktop writes for General Date). A name such as `Short Date` as a visual format
  draws as a custom code ("S12or Da e"), so BIcraft no longer writes names. The space before `%`
  follows the UI's locale, so BIcraft keeps `0.00%`, drawn right everywhere (Custom in the card).

### Commands

`visual.add_sparkline {page, visual, y, summarization?, x, chart_type?, color?, width?, markers?,
marker_color?, marker_shape?, marker_size?, apply_calculation_group_to?}` adds one (tables and
matrices only, at most five, not twice the same Y and X); `visual.edit_sparkline {page, visual,
index?, …}` changes its axes or look and keeps its `queryRef`; `visual.unbind` removes it. A
sparkline is left out of `visual.export_data`.
