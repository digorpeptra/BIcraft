# Writing a TypeScript custom visual with BIcraft

Use this guide when neither a built-in BIcraft visual nor a Spec visual fits. The project format
is in `docs/formats/pbiviz.md` "Visual projects". BIcraft's shared runtime is described in
`docs/community-visuals.md` "Runtime", and the look in `docs/formats/visual-look.md`.

## The project and the toolchain

`visual.scaffold {folder, name, …}` writes these files:

- `pbiviz.json`: API 5.3.0, version `1.0.0.0`, and a guid made of the name and 32 hex digits;
- `capabilities.json`;
- `tsconfig.json`;
- `package.json`, which only lets editors fetch the `powerbi-visuals-api` types;
- `src/visual.ts`;
- `style/visual.css`;
- `assets/icon.png`, a 20 × 20 icon BIcraft draws.

It refuses to write into a folder that already holds a `pbiviz.json` unless `overwrite: true`.
`name` is letters, digits and `_`, and starts with a letter: the guid names a global variable
in the bundle.

`visual.build {folder}` and `visual.pack {folder}` compile with BIcraft's own bundler (swc, no
Node, no npm install):

- Types are stripped, not checked. Run `tsc --noEmit` yourself if you want type errors.
- Only relative imports are bundled. `powerbi-visuals-api` works because its imports are
  type-only. Any other package import, an import cycle, a `.tsx` file or LESS is refused.
- Sources are capped: 512 files, 4 MB each, 16 MB in all.

Two templates:

| `template` | What you get | When |
|---|---|---|
| `default` | Your roles (`data_roles: ["name:Grouping\|Measure\|GroupingOrMeasure[:Display]"]`) and format options (`format_options: ["object.property:bool\|color\|numeric\|text[:Display]"]`), a visual that draws a line per category and offers its options in the format pane | a small, self-contained visual |
| `spec` | The Spec visual's sources: `src/visual.ts`, `src/spec.ts`, BIcraft's runtime in `src/runtime/`, the Style card, EN/FR `stringResources`, the full capabilities | a visual that should behave like BIcraft's own |

With `spec`, keep `src/runtime/` and the `IVisual` wiring, replace the layout in `spec.ts` or
the drawing in `visual.ts`, and edit `capabilities.json` for your roles. The template does not
take `data_roles` or `format_options`.

## Data roles and capabilities

- Roles in `capabilities.json` `dataRoles` have a `name`, a `displayName` and a `kind`
  (`Grouping`, `Measure`, `GroupingOrMeasure`).
- The scaffold writes one categorical `dataViewMappings`: categories from the first grouping
  role (top 1,000), and values bound to each measure role. Add `conditions` for per-role limits.
  Raise or lower `dataReductionAlgorithm.top.count` to bound what the visual receives.
- Format options are `objects` with typed properties: `{bool: true}`, `{numeric: true}`,
  `{text: true}`, `{fill: {solid: {color: true}}}`. Read them from
  `dataView.metadata.objects.<object>.<property>`, and offer them in `getFormattingModel()`
  (cards, groups and slices).
- Declare what the visual supports. BIcraft's own visuals carry:

```json
"privileges": [],
"tooltips": {"supportedTypes": {"default": true, "canvas": true}},
"supportsLandingPage": true,
"supportsEmptyDataView": true,
"supportsKeyboardFocus": true,
"supportsHighlight": true
```

  The `default` scaffold writes `privileges: []` only. Add the others yourself, and only for
  features the visual implements. In particular, declare `supportsHighlight` only if the visual
  reads `highlights` (below).

## The shared runtime (`src/runtime/`, from `template: "spec"`)

These are BIcraft's own TypeScript modules. Import them with relative imports
(`./runtime/data`…).

| Module | Use |
|---|---|
| `data.ts` | `tableOf(dataView)` gives the rows: the category column and the value columns, each with its name, format string and roles. Also `boundRoles`, `missingRoles(dataView, required)` and `objectValue(dataView, object, property, fallback)` |
| `format.ts` | `formatValue`, `formatNumber`, `dateLabel`: values written with the column's format string, in English or French |
| `tooltips.ts` | `tooltipItems(table, row, locale)` gives a row's tooltip lines (shown through `host.tooltipService`) |
| `selection.ts` | `categoryIds(host, categorySource, rows)` gives one selection id per category value |
| `interactions.ts` | `new Interactions(host, root)` once. After each draw, `wire({container, targets: [{row, el, hit?, fade?}], ids, tooltip, label, contrast})` wires hover tooltips, click and Ctrl+click selection, background clear, the context menu (right click, Menu key, Shift+F10), Tab, Enter, Space, the arrows, Home, End and Escape, bookmark restore and the 40 % dimming |
| `landing.ts` | `showLanding(root, title, landingText(locale, roleNames))` with no fields, and `showMessage(root, text)` |
| `contrast.ts` | `contrastOf(host)` gives the high-contrast colours or `null`. `contrastPrim(prim, c)` recolours a primitive |
| `style.ts`, `paint.ts` | the look: `lookOf(style, hostTheme(host))`, the preset tokens, `decorate(svg, prim, el)` (corner radius, gradient, shadow), `motionAllowed`, `freshData` |
| `formatting.ts` | the format pane model: `card`, `group`, `toggle`, `color`, `number`, `dropdown`, `textInput`, `textArea`, `model`, and `withLook(cards, locale, look, palette)` to add the Style card |
| `strings.ts` | `langOf(host.locale)` (`fr` or `en`) and `pick(locale, {en, fr})` |

The Spec visual's `update` is the reference wiring. In order:

1. contrast;
2. landing page;
3. `tableOf`;
4. layout;
5. draw SVG with `decorate`;
6. `interactions.wire`.

Its `getFormattingModel` returns `model(withLook([...cards], …))`.

## The modern look

- Read the host theme with `hostTheme(host)`, which uses `host.colorPalette`: `getColor("0")`…
  `"7"` and the background. Prefer the theme's colours over fixed ones. BIcraft's palette
  (`BICRAFT_PALETTE`) is the fallback.
- Presets: `modern` (rounded, gradient, ringed dots, bold value labels, entry animation),
  `minimal` (flat), `bold` (edges, shadows, value chips). Expose them through the Style card
  (`withLook`) so a report author can switch.
- Dark backgrounds: when the theme background's relative luminance is under 0.07, switch to the
  dark tokens, which `lookOf` does for you:
  - axis and category text `#CFCEC4`;
  - values `#F0EFEC`;
  - gridlines `#3B3A37`;
  - axis lines `#5E5D58`.
- Text contrast is at least 4.5:1, and identity never rests on colour alone: keep a legend or
  direct labels.
- Motion plays only when data arrives. It must not play under `prefers-reduced-motion: reduce`
  or in high contrast (`motionAllowed`).
- Fonts: a system stack (`system-ui, "Segoe UI", …`), named only. Bundle no font files.

## Highlights (`supportsHighlight`)

With `supportsHighlight: true`, another visual's selection no longer filters your data:

- Each value column keeps its full `values`.
- Each value column also gets a `highlights` array of the same length: `null` where not
  highlighted, else the highlighted part.

Read it next to `values` (the Spec visual's `data()` does). Draw the full mark at 40 % with the
highlighted part over it when a part makes sense (a bar's length). Otherwise dim the units none of
whose rows is highlighted (pass `fade: true` on their targets).

A visual with no measure role gets no highlights: leave `supportsHighlight` off, and it is
filtered instead. When none of its measure roles has a field, a highlighting visual receives all
rows and no `highlights`, and should draw normally.

## Selection, tooltips, context menu, keyboard

The runtime does all four once you pass it `targets` (the drawn element of each row), `ids`
(`categoryIds`), `tooltip` and `label` (the accessible name). Without the runtime, use the
public API yourself:

- `host.createSelectionIdBuilder().withCategory(column, i)` and `host.createSelectionManager()`
  (`select`, `clear`, `showContextMenu`, `registerOnSelectCallback`);
- `host.tooltipService` (check `enabled()`).

Check `host.hostCapabilities.allowInteractions`.

## Format pane strings in English and French

- Give each role, object, property and drop-down item a `displayNameKey` next to its English
  `displayName`, for example `Role_value`, `Object_labels` or `Object_labels_show`.
- Add `stringResources/en-US/resources.resjson` and `stringResources/fr-FR/resources.resjson`:
  JSON objects from key to text.
- `visual.pack` puts them into the package's `stringResources`. Limits: at most 64 locales and
  2,000 texts each. A locale folder without its file is skipped with a warning.
- Translate from the meaning of the English. Never copy another product's translations.
- Cards built in `getFormattingModel` are drawn in the host's language: pick their texts with
  `pick(locale, {en, fr})`.

## Landing page

With `supportsLandingPage` and `supportsEmptyDataView`, the visual is updated with no fields.
Show its name and the roles to fill (`landingText(locale, roleNames)`). When required roles are
still empty, show a one-line message naming them (`missingRoles`).

## High contrast

When `host.colorPalette.isHighContrast` is true:

- draw everything in `foreground` over `background`, with fills as outlines;
- draw selected points in `foregroundSelected`;
- drop gradients, shadows and animation.

`contrastOf` and `contrastPrim` do this for runtime primitives.

## Must not

- No network: keep `privileges: []`. Use no `fetch`, XHR, WebSocket, remote images or remote
  fonts. A package has no other files the visual could load; embed small data in the script.
- No R or Python visuals.
- No npm packages, no Microsoft utility packages, no code, text or images copied from another
  visual. Write from the public API documentation.
- Never let `update` throw. Guard against empty, blank, text, huge and NaN values, and cap
  what you draw.

## Version

Raise `pbiviz.json` `visual.version` (`a.b.c.d`) with every change to the package's bytes: code,
CSS, capabilities, strings, icon or runtime. Then `visual.pack` again. The output name carries
it (`dist/<guid>.<version>.pbiviz`). `visual.import_custom` replaces a package with the same
guid.
