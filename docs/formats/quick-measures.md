# Quick measures

Sources: Microsoft Learn, [Use quick measures for common and powerful calculations](https://learn.microsoft.com/en-us/power-bi/transform-model/desktop-quick-measures)
(the categories and calculations, the inputs they ask for, the result being an ordinary measure
whose DAX shows in the formula bar). The DAX templates below are BIcraft's own: Learn does not
publish Desktop's, and nothing here is compared with a measure Desktop wrote.

A quick measure leaves nothing special in the file: `measure.quick {kind, params, table?, name?}`
adds an ordinary measure (TMDL `measure`, TMSL `measures[]`), validated by the DAX binder as
`measure.add` is. Its table is the base value's table unless `table` is given; its name is the
default below (with ` 2`, ` 3`… when taken) unless `name` is given.

## Inputs

- A **value** (`base`, `weight`, `second`): a measure `[M]`, an aggregation `Sum(T[C])` (also
  `Avg`, `Min`, `Max`, `Count` as `DISTINCTCOUNT`, `CountNonNull` as `COUNT`, `Median`), or a
  column `T[C]`: `SUM` for a number, `COUNTA` otherwise. Its name in the measure's name is the
  measure's, or `Sum of C`.
- A **column** (`category`, `filter`, `field`), a **date column** (`date`, type date/time).
- `values`: literals of the filter column's type (dates as `YYYY-MM-DD`, `null` a blank).

## Calculations

`B` is the base value's DAX, `C`, `D`, `F` the columns.

| Kind | Params | Default name | DAX (shape) |
|---|---|---|---|
| `average_per_category`, `variance_per_category`, `max_per_category`, `min_per_category` | base, category | `B average per C` (variance, max, min) | `AVERAGEX` / `VARX.P` / `MAXX` / `MINX(KEEPFILTERS(VALUES(C)), CALCULATE(B))` |
| `weighted_average_per_category` | base, weight, category | `B weighted by W per C` | `DIVIDE(SUMX(cats, CALCULATE(B * W)), SUMX(cats, CALCULATE(W)))` |
| `filtered_value` | base, filter, values | `B for v1, v2` | `CALCULATE(B, F IN { v1, v2 })` |
| `difference_from_filtered_value` | same | `B difference from …` | `B - filtered`, blank when the filtered value is |
| `percentage_difference_from_filtered_value` | same | `B % difference from …` | `DIVIDE(B - filtered, filtered)`, format `0.00%` |
| `sales_from_new_customers` | base, customer, date | `B from new customers` | `CALCULATE(B, FILTER(VALUES(C), ISBLANK(CALCULATE(B, REMOVEFILTERS(), C = this customer, D < MIN(D)))))`: the customers that bought nothing before the first date of the context, whatever the other filters |
| `year_to_date`, `quarter_to_date`, `month_to_date` | base, date, year_end_month? (1–12, year to date only, default 12) | `B YTD` / `QTD` / `MTD` | `TOTALYTD` / `TOTALQTD` / `TOTALMTD(B, D)`; a fiscal year ending in month m adds the year-end text `"m/last day"` (`"6/30"`, `"2/28"`: the day is above 12, so every locale reads it the same) |
| `year_over_year_change`, `quarter_over_quarter_change`, `month_over_month_change` | base, date | `B YoY%` / `QoQ%` / `MoM%` | `DIVIDE(B - previous, previous)`, previous = `CALCULATE(B, DATEADD(D, -1, YEAR))` (quarter, month), format `0.00%` |
| `rolling_average` | base, date, period? (`day`, `month` default, `quarter`, `year`), before? (3), after? (0) | `B rolling average` | `AVERAGEX(DATESBETWEEN(D, first, end), CALCULATE(B))` around `MAX(D)`: from `before` periods back (excluded) to `after` periods on (`EDATE` for months, quarters, years) |
| `running_total` | base, field, descending? | `B running total in F` | `CALCULATE(B, FILTER(ALLSELECTED(F), F <= MAX(F)))` (`>=` descending) |
| `total_for_category_filtered`, `total_for_category_unfiltered` | base, category | `B total for C` (`(all)`) | `CALCULATE(B, ALLSELECTED(C))` / `ALL(C)` |
| `addition`, `subtraction`, `multiplication`, `division` | base, second | `B plus S` (minus, times, divided by) | `B + S`, `B - S`, `B * S`, `DIVIDE(B, S)` |
| `percentage_difference` | base, second | `B % difference from S` | `DIVIDE(B - S, S)`, blank when `S` is, format `0.00%` |
| `correlation_coefficient` | category, base (X), second (Y) | `X and Y correlation per C` | Pearson's coefficient over the categories of the context where both values are not blank: `ADDCOLUMNS(KEEPFILTERS(VALUES(C)), X, Y)`, then `(n Σxy − Σx Σy) / √((n Σx² − (Σx)²)(n Σy² − (Σy)²))` with `DIVIDE` (blank for one category or a constant value) |
| `star_rating` | base, max_value, min_value? (0), stars? (5) | `B star rating` | the value clamped to [min, max], `ROUND` to stars, `REPT(UNICHAR(9733), n) & REPT(UNICHAR(9734), stars - n)`, blank when the value is |
| `concatenated_list` | field, max_values? (3), separator? (`, `) | `List of F values` | `CONCATENATEX(TOPN(n, VALUES(F), F, ASC), F, sep, F, ASC)`, `& ", etc."` when there are more |

Learn names the two calculations added on 2026-11-24 (*Sales from new customers* under Filters,
*Correlation coefficient* under Mathematical operations) but not their inputs or DAX; the inputs
and templates above are BIcraft's reading of the calculations' usual meaning.

**Dates of time intelligence** (`D` above, for YTD/QTD/MTD, YoY/QoQ/MoM and the rolling
average): Learn asks for a date table marked as such. BIcraft writes:

- the date column itself when its table is marked as a date table (`dataCategory` `Time`);
- `T[C].[Date]` when the column has auto date/time (a date variation whose date table has a
  `Date` column): that table holds every date of the years the column spans, as a marked date
  table does, so `DATEADD` and the to-date windows find the dates without rows too;
- the column itself otherwise, and the result's `warnings` say that only the dates the column
  holds are seen. No template can do better there: from a column with gaps, the context's first
  and last dates present are not its period's bounds, so whether the context is a day, a month
  or a year cannot be known (shifting the dates present, as `DATEADD` does, misses the dates
  without rows; snapping to whole periods would change the result at a finer grain). The fix is
  a model one: mark a date table holding every date (`table.set {data_category: "Time",
  key_column}`), or keep auto date/time on.

*Sales from new customers* needs neither: the first date the column holds in the context does
as well as the period's first day, gaps or not.

## Not done

- The rolling average's window counted on a calendar of every date when the date column has
  gaps (`AVERAGEX` skips the dates without a value).
- Desktop shows the quick measure's DAX as it writes it; BIcraft's templates differ in text
  (same calculations, not compared with Desktop's results).
