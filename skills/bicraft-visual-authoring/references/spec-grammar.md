# The Spec visual's grammar

The full specification is `docs/formats/spec-visual.md`. This page is the part an agent needs to
write a spec. `visual.spec_template {kind}` returns a starting spec for `bar`, `line`, `area`,
`point`, `scatter`, `combo`, `text` or `rule`, and `visual.set_spec {page, visual, spec}` stores
it. The spec is an object or its JSON text, at most 32 KB.

## Data

The visual has two roles:

- `category`: Grouping, at most 1 field, top 1,000 rows.
- `values`: Measure, up to 16 fields.

A spec reads them by these names:

| Field | Meaning |
|---|---|
| `category` | the value of the `category` role on the row |
| `value` | the first field of `values` (same as `value.0`) |
| `value.N` | the field at index N of `values`, N from 0 to 15, in the order they were bound |

A channel takes a field name, `{"field": "…"}`, or a constant `{"value": number | text}`.

## Top level

| Key | Value | Default |
|---|---|---|
| `version` | `1` | 1 |
| `$schema` | any text, ignored | |
| `padding` | 0–64 px | 8 |
| `palette` | 1–32 colours `#RRGGBB` | the report theme's data colours, else BIcraft's palette |
| `style` | `preset` (`modern`, `minimal`, `bold`), `palette` (`theme`, `bicraft`), `rounded`, `gradient`, `shadow`, `animation`, `card`, `pills` | Modern |
| `x` | `scale` (`band`, `linear`, `time`), `axis`, `title`, `grid`, `min`, `max`, `zero`, `padding` (band, 0–0.9) | band, axis, no grid, padding 0.2 |
| `y` | `scale` (`linear`), `axis`, `title`, `grid`, `min`, `max`, `zero` | axis, grid, zero |
| `legend` | `true` / `false` | shown when a mark has a `name` |
| `marks` | 1–16 marks, drawn in order | |

The format pane's Style card overrides the spec's `style`.

## Marks

| Key | Value | Default |
|---|---|---|
| `type` | `bar`, `line`, `point`, `area`, `text`, `rule`, `rect` | required |
| `name` | legend text | |
| `x`, `y`, `x2`, `y2` | a field or a constant | `x`: `category`, `y`: `value` (a rule: none) |
| `color` | `#RRGGBB`, `{"field": "category"}` (palette by category), `{"field": "value.N", "range": [c0, c1]}` | the palette's colour at the mark's index |
| `size` | a number, or `{"field": "value.N", "range": [a, b]}` | point 4, line 2, rule 1, text 11 |
| `label` | a field written at each value; the text of a `text` mark | |
| `opacity` | 0–1 | 1 (area 0.35) |
| `decimals` | 0–6, decimals of the label | none for whole numbers, else 2 |

## Rules the parser enforces

Every error names its place, for example `marks[1].colour: unknown key`. A refused spec leaves the
report unchanged.

- Unknown keys are refused.
- `y` and `y2` are measures or numbers.
- On a band x scale, x is `category` or a category's text. A measure on x needs `"scale":
  "linear"`.
- Bars need the band scale. A `rect` on a linear or time scale needs `x2`.
- A `rule` has `x` (vertical) or `y` (horizontal), not both. A `text` mark has a `label`.
- On a time scale, text is an ISO date (`2024-01-31`, optional `T13:45`).
- Texts are at most 200 characters, with no control characters. Numbers stay within ±10¹⁵.

## Examples

Bars with labels and a target line (two measures):

```json
{"version": 1, "y": {"title": "Amount"},
 "marks": [{"type": "bar", "y": "value", "name": "Sales", "label": "value"},
           {"type": "line", "y": "value.1", "name": "Target", "color": "#E07A1F"}]}
```

Bars coloured by category, with a horizontal reference line at 500:

```json
{"version": 1, "legend": false,
 "marks": [{"type": "bar", "y": "value", "color": {"field": "category"}},
           {"type": "rule", "y": {"value": 500}, "color": "#C23B5A", "size": 2}]}
```

Range bars (floating from `value.1` to `value`) with a point on top:

```json
{"version": 1,
 "marks": [{"type": "bar", "y": "value", "y2": "value.1", "name": "Range"},
           {"type": "point", "y": "value", "size": 5}]}
```

Bubble scatter: x and y are measures, and size and colour come from two more:

```json
{"version": 1, "x": {"scale": "linear", "title": "Price"}, "y": {"title": "Units"},
 "marks": [{"type": "point", "x": "value", "y": "value.1",
            "size": {"field": "value.2", "range": [3, 14]},
            "color": {"field": "value.3", "range": ["#9ECAE1", "#08519C"]}}]}
```

A line over dates with a shaded area under it, in the Minimal look:

```json
{"version": 1, "style": {"preset": "minimal"}, "x": {"scale": "time"},
 "marks": [{"type": "area", "y": "value"}, {"type": "line", "y": "value", "name": "Daily"}]}
```

## What you get for free

The Spec visual's behaviour comes from BIcraft's runtime, with nothing to write:

- tooltips with formatted values;
- selection and cross-filtering, with Ctrl to add;
- highlights from other visuals (the full bar at 40 % with the highlighted part over it);
- the context menu and keyboard navigation;
- high contrast;
- a landing page with no fields;
- names in English and French.

`bicraft render` draws the visual unselected, with the same marks as Power BI.
