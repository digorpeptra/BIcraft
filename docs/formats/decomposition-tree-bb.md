# Decomposition tree: saved state, evaluation and drawing (black box)

How a decomposition tree visual (`decompositionTreeVisual`) stores what it shows, how Power BI
Desktop evaluates it, and how it draws it. Encoded in `crates/engine/src/preview_decomp.rs` (the
snapshot's `data.decomp`) and `crates/render/src/decomp.rs` (the drawing and its layout).

Sources: the public visualConfiguration schema (`ExpansionState`, `LevelExpansionState`,
`AILevelInformation`, `AIDecompositionMethod`), the public report theme schema (the visual's
formatting objects and their enumerations), and black-box tests against Power BI Desktop
2.159.1231.0 (Store edition, UI fr-FR, 2026-11-04): reports BIcraft generated, opened in Desktop,
the rendered page read through its WebView2 DevTools port (visible text, element classes,
ARIA labels, SVG geometry), Desktop's own UI driven by mouse events, the files Desktop saved read
back. Every source is listed, dated, in
[decomposition-tree-bb.sources.md](decomposition-tree-bb.sources.md).

## Roles

`Analyze` (one measure), `ExplainBy` (columns, any number), `Tooltips`
([visual-roles-bb.md](visual-roles-bb.md)).

## Saved state

The state is the `expansionStates` entry of role `ExplainBy` (PBIR `visual.expansionStates`):

```json
{"roles": ["ExplainBy"],
 "levels": [
   {"queryRefs": ["Sales.Region"], "isCollapsed": true, "identityKeys": [<Region column>], "isPinned": true},
   {"queryRefs": ["Sales.Channel"], "isCollapsed": true, "isPinned": true,
    "AIInformation": {"method": "MaxSplit", "disabled": false}},
   {"queryRefs": ["Sales.Category"], "isCollapsed": true}],
 "root": {"children": [{"identityValues": [{"Literal": {"Value": "'East'"}}], "isToggled": true}]}}
```

Observed by expanding a fresh tree in Desktop and saving (test `save`):

- `levels` lists every Explain-by field; the **shown levels** are the ones with `isPinned: true`,
  in order; the others follow. Every level carries `isCollapsed: true`. A level with a member on
  the selected path carries `identityKeys` (its column).
- The **selected path** is the chain of `root.children[]` (then `children[]`) with
  `isToggled: true`, one member per level from the first, as literals (`'East'`, `null` for a
  blank member).
- "High value" writes `AIInformation.method: "MaxSplit"`, "Low value" `"MinSplit"`, on the new
  level, with the field Desktop chose in `queryRefs`. Expanding a level under an AI level turns
  the AI level into a plain one (its `AIInformation` is removed, its field kept).
- `query.queryState.ExplainBy.projections` is reordered: the shown levels first, marked
  `active: true`, the others `active: false`.
- `objects.tree.effectiveBarsPerLevel` holds the number of bars that fit (Desktop writes it;
  BIcraft computes the same from the size).
- `objects.insights` entries with `isAINode: true` and a `selector.data` path mark the member an
  AI level selected (Desktop recomputes it; BIcraft does not need them).

No `sortDefinition` is written; one written by hand is honoured (test D07).

## Evaluation

- The root is the measure under the visual's filters (report, page, visual filters, slicers of
  the page; test D09).
- Each shown level is the measure grouped by its field under the filters and the path above it.
- **AI levels are chosen again whenever they are shown.** The saved field is ignored (D04 saved
  `Category`, Desktop showed `Channel`): among the Explain-by fields no level above uses, the one
  whose highest member (`MaxSplit`) is the highest of all candidates, or whose lowest member
  (`MinSplit`) is the lowest (absolute mode, `analysis.aiMode` absent or `absolute`; D04, D05,
  D13). In relative mode (`aiMode: 'relative'`), the member whose value divided by its level's
  mean is the highest (D12: a product at 1.9 times its mean won over a category at 1.8). That
  member is selected (drawn on the path) when the path names none.
- A level shows only under a selected member; a level whose parent has none is not shown, nor
  the ones after it.
- Members sort by category, ascending, blank first (D08), unless a `sortDefinition` says
  otherwise (D07: by the measure, descending). Blank members show as "(Blank)" in Desktop's UI
  language.

## Drawing

Measured in Desktop's SVG (report pixels: its CSS pixels at the 103 % zoom of the tests divided
by 1.029; the body is the visual less 5 px of padding on each side):

| What | Value |
|---|---|
| Bars | 130 × 9.7 px, background `#EFEFEF`, fill the first theme colour; length proportional to the level's largest absolute value (`dataBars.dataBarScalingType` `levelMaximum`; `parentNode`: to the parent's value, D11; `topNode`: to the root's) |
| Columns | root bar at x = 24.3; levels every 180 px (dense 162, sparse 216) |
| Rows | bars every 59.33 px (dense 47.47, sparse 71.2); the root and the bar area centred on `(h + 22.33) / 2`, a body under 319 px laid out as if 319 px high (D18) |
| Bars that fit | `floor((h − 104) / pitch)` (D02, D11, D18–D22); `tree.barsPerLevel` does not change it (D08: 6 asked, 4 shown; D22: 3 asked, 6 shown) |
| Children | centred on their parent's bar, kept inside the bar area (D03, D06) |
| Window | a level longer than the bars that fit shows `n` of them: with a selected member, that member in the middle (`start = sel − floor(n/2)`, clamped; D03, D05, D06, D12, D21, D23); without one, the window ends four members before the last (`start = max(0, total − n − 4)`; D02, D08, D14–D17, D19, D20, D22: an empirical fit of every case seen); scroll arrows above and below |
| Width | levels that do not fit are clipped on the right (D21) |
| Labels | member 12 px at bar centre + 14, value 12 px at + 27.6, 4.9 px from the bar's left; members on the path in bold, their bar background with a 1 px `#C8C8C8` border |
| Links | from the parent bar's right end to each child bar's left end, a cubic curve with both control points at mid-x (`tree.connectorType` `round`: the first at the start, D11); the link to the selected child 1.5 px in the accent colour (dotted when an AI split selected it, D13), the others 1 px `#EFEFEF` |
| Headers | a 56.4 px band; field name 16 px at y 19.9, an accent underline (2 px at y 29.1) as wide as the bars, a remove cross (×) at the right end, the selected member 12 px at y 41.8 when a level is shown below it; the AI lightbulb (accent colour) left of the name |
| "+" | right of each bar of the last level (and of the root alone) while Explain-by fields are left; its menu lists the two AI splits (Microsoft Learn's *Create and View Decomposition Tree Visuals in Power BI* names them "High Value" and "Low Value"; Desktop showed its French texts) when `analysis.aiEnabled` is not false, then the free fields |
| Values | the measure's format string; `dataLabels.dataLabelDisplayUnits` / `dataLabelPrecision` apply (D11: `593,3K`); no automatic display units |

## Interactions

Observed in Desktop (test `save`):

- a click on a member not on the path moves the path there (the levels below are kept and
  evaluated again under it, AI levels chosen again); nothing is cross-filtered;
- a click on a member on the path selects it: the other visuals of the page are highlighted (a
  column chart showed the region alone, the rest at 40 %); in the tree, the path up to that member
  stays opaque, every other bar at 40 %;
- the "+" adds a level under that member (the levels below it are replaced);
- the header's cross removes that level and the ones after it.

## Tests

| Id | Content |
|---|---|
| save | one tree (`Total` by Region, Category, Channel, Segment, Product) expanded in Desktop's UI: Region, "High value" under South, a click on East, Segment under Store, "Low value" under Retail; saved after each step and read back |
| D01–D23 | `cargo xtask oracle-gen decomp`: one tree per page with a written expansion state (see `xtask/src/oracles/decomp.rs` for each page's note), opened in Desktop and read (`xtask/decomposition-tree-oracle/read.mjs`); the corpus is `testsupport/decomp_oracle/corpus.json` |

## Unknowns

- Why a long level with no selected member starts where it does: the rule above fits the nine
  cases seen, no more.
- The layout of bodies under 319 px: one case (D18).
- Negative values, `dataBarScalingType` `topNode`, `axisStart`/`axisEnd`, conditional colours of
  bars: not tested.
- Ties between AI candidates: the first field in Explain-by order is kept (one tie seen, in
  Desktop's favour of the earlier field).
- `defaultClickAction: 'toggle'`: not observed in Desktop. BIcraft follows the public theme
  schema's description ("Collapse": a click on an intermediate member of the selected path
  collapses the tree instead of filtering): the levels below that member are removed.
- How far Desktop's scroll arrows move a level, and how it scrolls a tree wider than the visual:
  not recorded (BIcraft moves by one window and by one level).
