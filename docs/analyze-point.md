# Analyze a data point (`visual.analyze`)

Power BI Desktop can explain why a data point went up or down and show where a point's
distribution is different. BIcraft has its own analysis for the same two questions. It is BIcraft's
method, described here in our own words; it does not try to reproduce Desktop's algorithm and its
ranking can differ from Desktop's. The answer says so in its `estimate` field.

## Where it runs

- **App** (desktop and browser): right-click a data point of a visual, then **Analyze this
  point** (French: *Analyser ce point*). The entry shows only when the right click landed on a data
  point. The answer window lists the ranked fields as a grid (sortable, copy as TSV) with the
  point, the change or the value and the skipped fields. From the palette or the visual's More options menu,
  the command opens its form, where `kind`, `fields` and `top` can be set.
- **MCP and CLI**: `run_command {id: "visual.analyze", params: {page, visual, point, kind?,
  fields?, top?}}`. `point` is what the renderer's hit map gives a click, as `view.interact`
  `select` takes it (`[{c, v}]`). The command is read-only, allowed in server mode and to readers.

## What is compared

The measure is the visual's first measure or aggregated column outside its tooltips. Every number
comes from grouped queries through BIcraft's preview evaluator. The report's, the page's and the
visual's filters apply, as do the reader's slicer selections and the model's relationships, so a
column of a related table is a candidate like a column of the fact table. The point's other
columns (a legend value) stay fixed.

Candidates are the columns named in `fields`, or by default every visible column of the model
except the point's columns, the measure's own column and decimal columns (amounts rather than
categories). A field with more than 100 values under the points, fewer than two values, or one
whose every value shows the whole measure (it does not filter it, for example a table with no
relationship to the measure's) is skipped and listed in `skipped` with the reason. So is a field
past the first 40 or past the 60-second budget.

### Increase or decrease (`kind: "change"`)

Used by default when the point's axis is a date or a number. The previous point is the axis
value just before the point's among the values the visual shows. The first point has none and
is refused, as is a point whose value equals the previous one.

For each candidate field, BIcraft computes each value's change between the two points. The value
whose change goes furthest in the direction of the total change is the field's top member. The
field's score is that change divided by the sum of every value's absolute change, from 0 to 1.
A field where one value carries all the movement scores 1. A field whose values all move
together scores about one over its number of values. The answer also gives the top member's
change and its share of the total change (`share_of_change`). That share can exceed 1 when other
values moved the other way.

### A different distribution (`kind: "distribution"`)

Used by default on a categorical axis. For each candidate field, BIcraft takes the field's shares
of the measure under the point and under each of the visual's other points, then averages the
other points' shares. Each point counts once, whatever its size. The score is the total
variation distance between the point's shares and that average: half the sum of the absolute
differences, from 0 (the same split) to 1 (no value in common). The top member is the value most
over-represented under the point (`share_at_point` against `share_elsewhere`). A field with
negative values has no shares and is skipped.

## Answer

`{page, visual, kind, measure, point, change | value and other_points, candidates: [{field,
score, member, members, …}], found, considered, skipped: [{field, reason}], estimate}`, best
candidate first, at most `top` (default 10). A point that cannot be analyzed is an error with
the reason, never an empty success. That covers no point, no measure, a change asked on a
categorical axis, the first point, and no other point to compare with.

## Explain an anomaly

A line chart with Find anomalies on (`visual.analytics` `anomaly_detection`) marks the points
outside BIcraft's expected range (`crates/render/src/anomaly.rs`). `visual.explain_anomaly` says
which fields account for one of them. This is BIcraft's own method too, built on the change score
above. It does not reproduce Power BI's explanations, and its answer says so in `estimate`.

- **App**: right-click an anomaly's marker, then **Explain the anomaly** (French: *Expliquer
  l'anomalie*). The entry shows only on a point the detection flags. The answer window lists the
  ranked fields. From the palette or the visual's More options menu, the form takes `point`,
  `compare`, `fields` and `top`.
- **MCP and CLI**: `run_command {id: "visual.explain_anomaly", params: {page, visual, point?,
  compare?, fields?, top?}}`. Without `point`, the anomaly furthest outside its range (its distance
  from the expected value over half the range's width) is explained. Read-only, allowed in server
  mode and to readers.

The line is evaluated by the visual's axis column, in axis order, and run through the same
detection at the visual's sensitivity, so the anomalies are the ones the chart draws. A point the
detection does not flag is refused with its value and expected range.

**Against the expected value (`compare: "expected"`, the default).** For each candidate field,
each value of the field gets its own line over the chart's points (0 where it has no rows). That
line goes through the same detection, which gives the value's expected amount at the anomaly.
The field is then scored as a change is: the value whose deviation from its expected amount goes
furthest in the anomaly's direction is the top member. The score is that deviation over the sum
of every value's absolute deviation, from 0 to 1. The answer gives the member's `member_current`,
`member_expected`, `member_deviation` and its `share_of_deviation` of the anomaly's own
deviation. The candidates, limits and skipped fields are those of `visual.analyze`.

**Against the previous point (`compare: "previous"`).** The anomaly is analyzed as
`visual.analyze`'s change from the previous point of the axis, with the same answer fields.

The answer is `{page, visual, compare, measure, anomaly: {axis, at, value, expected, low, high,
deviation, direction, sensitivity}, anomalies, candidates, found, considered, skipped,
estimate}`. `anomalies` lists the axis values the detection flags, at most 100.

## Not done

- The answer is a list, not a visual added to the page.
- Only the visual's first measure is analyzed. Measures as candidates, combinations of two
  fields and numeric columns grouped into bins are not considered.
- No comparison with Desktop's own results, for a point or an anomaly. An anomaly's
  explanations are not kept in the report (Desktop's Explain by fields are not modelled), and a
  left click on its marker selects the point as on any line.
