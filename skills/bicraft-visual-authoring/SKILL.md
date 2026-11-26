---
name: bicraft-visual-authoring
description: Turn a request such as "make me a visual that…" into a working Power BI visual with BIcraft. Pick one of BIcraft's 14 built-in visuals, a declarative Spec visual (no code) or a scaffolded TypeScript custom visual. Then check it, package it as a .pbiviz and export it, using only BIcraft's own commands over MCP or the bicraft CLI.
metadata:
  version: "1"
---

# BIcraft: from a visual request to a packaged Power BI visual

Use this skill when a user asks for a chart or visual in a Power BI report and BIcraft is the
tool: the `bicraft` MCP tools (`bicraft mcp`) or the `bicraft` command line. Each step below is an
engine command. Over MCP you call it with `run_command {command, params}` (or its convenience
tool). On the command line you use `bicraft run --open FILE --cmd 'ID={JSON}' [--save]` or the
dedicated subcommand.

- Trust the live registry over this file: `list_commands` (MCP) or `bicraft commands visual` /
  `bicraft describe visual.set_spec` (CLI) give the current ids and parameters.
- Programmatic calls never open a dialog. A missing parameter gets its default or an error.
- Visuals live in PBIR reports. A legacy report (`Report/Layout`) is refused: convert it first
  (`bicraft convert IN OUT --report-format pbir`).

## 1. Choose the path (cheapest first)

Work through these in order and stop at the first that fits.

### a. A built-in BIcraft visual already does it

BIcraft ships 14 visuals of its own. Name one in `visual.add` (or `visual.set_type`) and BIcraft
embeds its package in the report on first use. `report.custom_visuals` lists them under
`bicraft`, with their roles.

| Name (`visual_type`) | Use it for | Roles (required in bold) |
|---|---|---|
| `bicraftSpec` | any chart a spec can describe (section b) | **`category`**, `values` (up to 16 measures) |
| `bicraftHistogram` | distribution of a numeric column | **`values`** (the column), `frequency` |
| `bicraftBullet` | value against target with bands | `category`, **`value`**, `target`, `bands` (up to 4), `maximum` |
| `bicraftBoxPlot` | quartiles and outliers per group | `category`, **`sample`**, **`value`** |
| `bicraftDumbbell` | start against end per row; slope chart | **`category`**, **`start`**, **`end`** |
| `bicraftOrgChart` | hierarchy from id and parent | **`id`**, **`parent`**, `label`, `value` |
| `bicraftVenn` | overlap of up to three sets | **`item`**, **`set`** |
| `bicraftMarimekko` | share by two dimensions | **`category`**, `series`, **`value`** |
| `bicraftCalendarHeatmap` | a value per day | **`date`**, **`value`** |
| `bicraftSankey` | flows between nodes | **`source`**, **`destination`**, **`weight`** |
| `bicraftScatterPlus` | scatter with size and groups | **`details`**, `legend`, **`x`**, **`y`**, `size` |
| `bicraftLineAreaPlus` | lines, areas, bands between two measures | **`axis`**, `series`, **`value`**, `value2` |
| `bicraftWaterfallPlus` | waterfall with subtotals, horizontal or vertical | `category`, `group`, `subtotal`, `target`, **`values`** |
| `bicraftGaugeKpi` | arc, bullet or KPI gauge | `trend`, **`value`**, `target`, `min`, `max`, `threshold1`–`threshold3`, `previous` |

```json
{"command": "visual.add", "params": {"page": "Overview", "visual_type": "bicraftBullet",
  "x": 40, "y": 40, "width": 480, "height": 260,
  "fields": [{"role": "category", "field": "Region[Name]"},
             {"role": "value", "field": "[Revenue]"},
             {"role": "target", "field": "[Target]"}]}}
```

Set format options with `visual.set_bicraft_visual {page, visual, kind, options}` for the
histogram, bullet, box plot and dumbbell. It checks every option, for example
`{"bins": {"count": 20}, "style": {"preset": "bold"}}`. For all 14 visuals, `visual.format
{page, visual, objects: [{object, property, value}]}` also works, for example `{object: "style",
property: "preset", value: "bold"}`. It does not check custom visuals' options, so spell them as
`capabilities.json` does.
The roles and options of each visual are in `docs/formats/bicraft-visuals.md`,
`docs/formats/chart-pack.md` and `docs/formats/waterfall-gauge.md`.

### b. The Spec visual can draw it (no code)

The request is a composition of bars, lines, points, areas, text, rules and rectangles. It reads
one category and up to 16 measures, on band, linear or time scales. Write a JSON spec and store
it with `visual.set_spec`. Power BI draws it with BIcraft's script, and `bicraft render` draws the
same layout. The grammar and examples are in [references/spec-grammar.md](references/spec-grammar.md).

### c. Otherwise, write a TypeScript custom visual

Use code only when the visual needs a layout, data shape or interaction that a spec cannot
express. Examples: a custom geometry, several categories, a hierarchy, or a free-form text layout.
The loop is `visual.scaffold`, then edit, then `visual.pack`, then `visual.import_custom`. See
section 3 and [references/code-visuals.md](references/code-visuals.md).

## 2. Worked example: a Spec visual

The user asks: "Revenue as bars by region, with the target as a line and the values on the bars."

MCP:

```json
{"command": "visual.add", "params": {"page": "Overview", "visual_type": "bicraftSpec",
  "x": 40, "y": 40, "width": 640, "height": 360,
  "fields": [{"role": "category", "field": "Region[Name]"},
             {"role": "values", "field": "[Revenue]"},
             {"role": "values", "field": "[Target]"}]}}
```

`visual.add` returns the visual's name. With two values, the visual starts with
`visual.spec_template`'s `combo` spec. Replace it with the spec you want:

```json
{"command": "visual.set_spec", "params": {"page": "Overview", "visual": "<name>", "spec": {
  "version": 1,
  "x": {"scale": "band"},
  "y": {"title": "Amount"},
  "marks": [
    {"type": "bar", "y": "value", "name": "Revenue", "label": "value"},
    {"type": "line", "y": "value.1", "name": "Target"}
  ]}}}
```

Then check it with `render_page {page: "Overview", out: "out/overview.png"}` and `save`.

CLI (a commands file avoids quoting JSON on the command line):

```sh
bicraft run --open Sales.pbip --file spec-steps.json --save
bicraft render Sales.pbip --page Overview --out out/overview.png
```

`spec-steps.json` is a JSON array of `{command, params}` holding the same two steps. The second
step needs the visual's name: run `visual.add` first, or turn an existing column chart into a Spec
visual with `visual.set_spec` directly. That command converts the chart's columns to `category`
and its measures to `values`. `visual.spec_template {kind}` (bar, line, area, point, scatter,
combo, text, rule) returns starting specs.

## 3. Worked example: a scaffolded TypeScript visual

The user asks: "A lollipop chart: a stem and a dot for each category."

```json
{"command": "visual.scaffold", "params": {"folder": "visuals/lollipop", "name": "lollipopChart",
  "display_name": "Lollipop", "author": "Your name",
  "data_roles": ["category:Grouping:Category", "value:Measure:Value"],
  "format_options": ["dataPoint.fill:color:Color", "labels.show:bool:Show labels"]}}
```

CLI:

```sh
bicraft visual new visuals/lollipop lollipopChart --display-name Lollipop \
  --role category:Grouping:Category --role value:Measure:Value \
  --format dataPoint.fill:color:Color --format "labels.show:bool:Show labels"
```

1. Edit `src/visual.ts`. The scaffold draws a line per category and already reads its format
   options and gives them to the format pane. Replace the drawing with your SVG.
2. Compile while editing: `visual.build {folder}` or `bicraft visual build visuals/lollipop`. It
   reports refused imports, cycles or a missing class.
3. Package: `visual.pack {folder}` or `bicraft visual pack visuals/lollipop`. It writes
   `dist/<guid>.<version>.pbiviz`.
4. Use it in a report: `open_file`, then `visual.import_custom {path:
   "visuals/lollipop/dist/<file>.pbiviz"}`. Then call `visual.add {visual_type: "<guid>", …}` with
   the roles from `capabilities.json`, and `save`.

Need tooltips, selection, the look, a landing page and high contrast without writing them
yourself? Start from `visual.scaffold {folder, name, template: "spec"}` instead. It writes the Spec
visual's sources with BIcraft's shared runtime under `src/runtime/` and EN/FR `stringResources`,
under a new guid. Keep the runtime and replace the drawing. The CLI's `visual new` has no
`--template` flag, so use `bicraft run --cmd 'visual.scaffold={"folder":"visuals/x","name":"myVisual","template":"spec"}'`.

The details of data roles, capabilities, the look, highlights, interactions, localized names,
the landing page, high contrast and what a visual must not do are in
[references/code-visuals.md](references/code-visuals.md).

## 4. Check it

- **Render without Power BI:** `render_page {page, out}` or `bicraft render FILE --page P --out
  shot.png`. The result lists each visual's `status` (`drawn`, `placeholder`, `blank`…) and
  `data`. `--fail-on-empty` (`fail_on_empty: true`) fails when a visual has no data.
  - BIcraft draws its own 14 visuals, Spec visuals included, natively with the same layout as
    Power BI.
  - **BIcraft never runs a custom visual's JavaScript.** A scaffolded visual renders as a labelled
    placeholder with its icon and name, so its drawing can only be checked in Power BI Desktop.
- **Validate:** `validate_report` (`bicraft validate-report FILE`) checks fields against the model.
  `report.custom_visuals` shows each package's `registered` state, `issues` and `used_by`.
  Keep projects in short folders: the embedded package's path
  (`<Name>.Report/CustomVisuals/<guid>/resources/<guid>.pbiviz.json`) is long, and
  `validate-report` flags any path over 259 characters, which Desktop cannot read.
- **Open in Power BI Desktop** (Windows): save, then `file.open_in_desktop`. For a standalone
  `.pbiviz`, use the Visualizations pane: "…", then Import a visual from a file. Note the Desktop
  version with any observation.
- **When contributing a visual to BIcraft itself:** each built-in layout exists twice, in Rust
  (`crates/render/src/…`) and in TypeScript (`crates/engine/src/…`). Golden tests hold the Rust
  layout to a JSON file (`BICRAFT_BLESS=1` rewrites it). They run the TypeScript under Node when
  Node is on the PATH and require the same primitives to 0.011 px. Node is a development check,
  never a dependency (see `crates/engine/tests/report/spec_visual_tests.rs`, `bvis_tests.rs`).

## 5. Export

- One of BIcraft's visuals as a file, for Import a visual from a file or a tenant's
  organizational visuals: `visual.export_bicraft_visual {name, out}` or `bicraft visual export
  bicraftSankey --out sankey.pbiviz` (several names or `--all` write
  `<guid>.<version>.pbiviz` files into a folder). It needs no open file.
- A scaffolded visual: the `.pbiviz` written by `visual.pack` is the deliverable.
- In the Power BI service (from Microsoft's documentation, not tried by BIcraft;
  `docs/community-visuals.md` "Outside BIcraft"):
  - A report with these visuals should work where the tenant allows visuals made with the Power BI
    SDK. Where only organizational visuals are allowed, an admin uploads the `.pbiviz`.
  - Uncertified visuals are expected to be left out of export to PowerPoint or PDF and out of
    email subscriptions.

## 6. Rules (non-negotiable)

- **No Microsoft assets.** No icons, logos, images, themes or sample files from Microsoft or
  Desktop. The scaffold's icon and BIcraft's icons are BIcraft's own drawings.
- **No copying other visuals' code, text or images**, and no recreating an existing visual.
  Write from the public API documentation and `powerbi-visuals-api` types; no Microsoft utility
  package is needed.
- **No network calls, no R or Python.** Keep `privileges: []` in `capabilities.json`. Use no
  `fetch` or remote fonts, and no npm packages (the bundler refuses them).
- **No Marketplace (AppSource) or Microsoft Store submission.** BIcraft's maintainers decided
  this; BIcraft visuals ship as `.pbiviz` files only.
- **Bump the version whenever the package bytes change.** Raise `visual.version` (`a.b.c.d`) in
  `pbiviz.json` for any change to the TypeScript, CSS, capabilities, strings, icon or bundled
  runtime. For a built-in visual, raise its version constant (e.g. `SPEC_PACKAGE_VERSION`).
  BIcraft replaces an older embedded build and keeps a newer one, so an unchanged version hides
  the change.
- Hostile input stays hostile: a visual must draw something sensible or its landing page for
  empty, blank, huge or text values. It must never throw out of `update`.
