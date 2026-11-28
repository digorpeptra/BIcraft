# Key influencers visual (`keyDriversVisual`)

Power BI's key influencers visual ranks what drives a field to a value (a rating to be low,
say) and finds top segments. Power BI computes it with its own machine-learning models. Their
internals are not public, and BIcraft does not reproduce them. BIcraft draws the visual two ways:

1. **Placeholder (default).** The body is laid out like Desktop's visual. The two tabs "Key
   influencers" and "Top segments" are drawn inactive, with the saved one underlined. The
   question line, in BIcraft's words, names the target and the goal ("What drives Score to
   Increase", "What drives Churn to be Yes"). On the left: the analysed field (Analyze), the Explain by
   fields, the Expand by fields and the target. On the right: the note "Computed by Power BI —
   not reproduced by BIcraft". `report.render` reports the visual as `placeholder`.
2. **BIcraft estimate (opt-in).** BIcraft's own transparent analysis of the session's data,
   drawn under a visible **BIcraft estimate** badge. `report.render` reports the visual as
   `estimate`. It is never presented as Power BI's result, and it can differ from it.

The estimate is turned on per render or per viewer:

- `report.render {key_influencers: "estimate" | "placeholder"}` (MCP `render_page`, CLI
  `bicraft-cli render --key-influencers estimate`) applies to that render only.
- `view.interact {action: "key_influencers", mode: "estimate" | "placeholder"}` sets the
  viewer's reader state. It is per viewer in server mode, `reset` keeps it, and the snapshot
  shows it as `interaction.key_influencers`. The desktop and web apps can offer it as a toggle.
  The render parameter wins over the viewer state for that render.

The drawing lives in `bicraft-render` (`crates/render/src/keyinf.rs`), so `bicraft-cli render`, the
desktop app and the web app show the same picture. The analysis lives in the engine
(`crates/engine/src/preview_key_influencers.rs`, arithmetic in `key_influencers_stats.rs`).

## Snapshot fields

Every `keyDriversVisual` in the snapshot carries `key_influencers` ([view-snapshot.md](view-snapshot.md)):

```json
"key_influencers": {
  "analysis": "key_influencers | top_segments",     // objects.keyDrivers.selectedAnalysis
  "target": 0, "explain_by": [1, 2], "details": [],  // indexes into the visual's `fields`
  "target_kind": "numeric | categorical",
  "goal": "increase | decrease | class",
  "class": "Yes" | null,                             // objects.keyDrivers.targetValue
  "mode": "placeholder | estimate",
  "estimate": {                                      // only in estimate mode
    "status": "ok | pending | unsupported | unavailable | no_rows | error", "reason": "…",
    "method": "bicraft-1", "rows": 1200, "overall": 71.4, "class": "Yes", "classes": 2,
    "tests": 14, "alpha": 0.0036,
    "influencers": [ { "field": 2, "kind": "is | range | up", "value": "North", "from": 20, "to": 30,
                       "step": 10, "value_mean": 89.9, "rest_mean": 69.8, "effect": 20.1,
                       "count": 300, "p": 1e-80 } ],
    "fields": [ { "field": 2, "numeric": false, "bars": [ { "value": "North", "mean": 89.9, "count": 300 } ] } ],
    "segments": [ { "conditions": [ { "field": 2, "op": "is | is_not | at_most | above", "value": "North" } ],
                    "mean": 95.2, "count": 150, "share": 0.125 } ],
    "segment_fields": [2, 3], "notes": ["…"]
  }
}
```

How the options are read (the public theme schema `reportThemeSchema-2.157.json`,
`visualStyles.keyDriversVisual.keyDrivers` names them):

- A **categorical** target is a column with a non-numeric data type, or a numeric column when
  `selectedNumericAnalysis` is `'Categorical'`. Aggregations and measures are numeric.
- The goal of a numeric target comes from `numericTargetSelectedKind`: `1` means decrease;
  anything else, or no value, means increase. Desktop 2.159.883.0 showed "Decrease" for a
  report we wrote with `1D` ([formats/key-influencers-bb.md](formats/key-influencers-bb.md)).
- A categorical target's class is `targetValue` (a string literal). When it is absent, or not
  in the data, the first class in sort order is used, and the estimate says so in `notes`.

## The estimate's method (`bicraft-1`)

The data comes from the preview evaluator ([view-snapshot.md](view-snapshot.md), "Data"). Each
analysis step is a synthetic grouped query of the visual's fields. The report, page and visual
filters, slicers and the reader's cross-filters apply, and relationships are followed, as for
any visual.

**Observations.** A row of the target's table is one observation:

- Numeric column target (raw or aggregated, no Expand by): each explain-by value gets
  `CountNonNull`, `Avg` and the population `Variance` of the target column. These moments give
  exact means and spreads without fetching rows.
- Expand by fields, with a measure or aggregated target: each row of (Expand by fields,
  explain-by field) is one observation, valued with the target field.
- A measure target without Expand by is not estimated: there is nothing to count as a row.
- Categorical target: rows are counted per (explain-by value, target value). The target is
  analysed as a 0/1 indicator of the selected class, so a group's mean is its class rate.

**Influencers, per explain-by column** (at most 12 fields; measures in Explain by are skipped):

| Explain-by field | Numeric target | Categorical target |
|---|---|---|
| Category (text, dates, booleans) | each category against all other rows: difference of the means, **Welch's t-test** | each category against all other rows: ratio of the class rates ("× as likely"), Welch's t-test on the indicator |
| Numeric (int64, double, decimal) | **least-squares slope** of the target on the field, shown per round step (1, 2 or 5 × 10ⁿ nearest the field's standard deviation): "when Age goes up 10, the average changes by +5"; t-test of the slope (n − 2 df). The detail chart shows 5 equal-count bins | the field cut into **5 equal-count bins**, each bin compared like a category |

Thresholds:

- A group needs at least **10 rows**, and so do the rows outside it.
- Significance level 0.05 with the **Bonferroni** correction: each test uses 0.05 ÷ the number
  of tests in the visual.
- Only effects in the goal's direction are kept: above 0 for increase, below 0 for decrease,
  a rate ratio above 1 for a class.
- Ranking: by the size of the effect (the ratio for a class). Ties keep the field order, then
  the value order. At most 10 influencers are kept.

The right pane draws the first influencer's field: the group means (or class rates), the
influencer's own bar in the accent colour, and the overall mean as a dashed line.

**Top segments.** A regression tree (CART on squared error; for a class this is equivalent to
Gini) is grown over the joint cells of at most three fields: the strongest influencers' fields
first, then the other fields in order. Splits are one category against the rest (each node
tries its 30 most frequent categories) or a numeric field at its bin edges. Depth is at most 3.
Each side needs at least max(20 rows, 5 % of the rows). Leaves whose mean (or rate) beats the
overall one in the goal's direction are the segments, best first, at most 6. When the joint
query has more than 20 000 cells, the last field is dropped and the query is retried.

**Bounds and determinism.**

- A query may return at most 20 000 rows. A field with more distinct values is not analysed
  and a note names it.
- Each visual has a time budget (20 s, and the snapshot's own budget: 1 s in the live viewer,
  60 s in `report.render`). An estimate cut short is not cached: it says
  `status: "pending"`, and its finished queries are cached so a later snapshot resumes.
- A complete estimate is cached like visual data, by data generation, model, visual, filters
  and clock.
- No step depends on hash order or randomness. Numbers are rounded to 12 significant digits, so
  the same data gives byte-identical JSON on Windows and Linux.
- The estimate runs under `catch_unwind`, and hostile input gives a `status` and `reason`,
  never an error.

## Differences from Power BI Desktop

- Desktop's models, binning, thresholds and segment algorithm are not public and are not
  reproduced. BIcraft's method is the one above. The numbers can differ, and an influencer
  Desktop shows can be missing or ranked differently.
- Desktop analyses a measure target, or measures in Explain by, at the Expand by level with its
  own rules. BIcraft needs Expand by for a measure target and skips measures in Explain by.
- Expand by is ignored for a categorical target. Hierarchy levels and other non-column fields
  in Explain by are skipped.
- Desktop's "Counts", sorting by count, "Only show values that are influencers", the
  continuous/categorical toggle UI and the drill into a selected influencer are not drawn. The
  first influencer is the selected one.
- The layout follows the arrangement observed in Desktop 2.159.883.0 on a synthetic report:
  tabs, question line, grey panel, list with bubbles, white card with columns and the dashed
  reference line. The positions are measured from a screenshot of that report
  ([formats/key-influencers-bb.md](formats/key-influencers-bb.md)). Desktop's thumbs-up and
  thumbs-down icons and its help mark are not drawn. The estimate's badge sits where the
  thumbs are. No pixel-level comparison is run.
