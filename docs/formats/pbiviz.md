# `.pbiviz` custom visual packages

Sources: the public `powerbi-visuals-tools` documentation on Microsoft Learn ("Package a Power BI
visual", "Power BI visual project structure", "Import a visual file from your local computer into
Power BI"), and the embedded-visual layout recorded in [pbix-package.md](pbix-package.md) §2 and
§11 "Custom visuals" (black-box: Desktop 2.158 opened a PBIT with a visual of our own).

## The file

A `.pbiviz` file is a ZIP archive that the `pbiviz package` command writes from a visual project:

| Entry | What it holds |
|---|---|
| `package.json` | the manifest: `visual` (`name`, `displayName`, `guid`, `version`, `description`, `supportUrl`), `author` (`name`, `email`), `resources[]` (`resourceId`, `sourceType`, `file`), `build`, `version`, `metadata` |
| `resources/<guid>.pbiviz.json` | the visual: the `visual` block again, `apiVersion`, `author`, `capabilities` (data roles, data view mappings, objects, privileges) and `content` with `js` (the bundled JavaScript), `css` and `iconBase64` (the 20×20 icon as a `data:image/png;base64,…` URL) |

The `guid` is the visual's identity: the visual type that the report's visuals carry
(`visualType`), the folder name of the embedded copy and the resource package name.

## In a report

Importing a visual from a file stores the package in the report (pbix-package.md §11, observed on
Desktop 2.158 with a visual of our own):

- `CustomVisuals/<guid>/package.json` and `CustomVisuals/<guid>/resources/<guid>.pbiviz.json`
  (`Report/CustomVisuals/…` in a PBIX, `<Name>.Report/CustomVisuals/…` in a PBIP), raw bytes;
- in `report.json` `resourcePackages`, `{name: <guid>, type: "CustomVisual", items: [{name:
  "<guid>.pbiviz.json", path: "<guid>.pbiviz.json", type: "CustomVisualMetadata"}]}`. The item
  path is relative to `CustomVisuals/<guid>/resources/`; the item's name equals its path because
  Desktop 2.159's save writes the name as the path ([pbix-package.md](pbix-package.md), "Custom
  visuals").

AppSource visuals (`publicCustomVisuals`) are not stored: Desktop fetches them from Microsoft's
store, so BIcraft does not import them (docs/gaps.md G80).

## BIcraft

`visual.import_custom {path | data}` (`bicraft_pbix::pbiviz::read_pbiviz`) reads a `.pbiviz` as
hostile input:

- the file at most 32 MB; at most 256 entries, 32 MB per decompressed entry, 64 MB in all, an
  entry past 1 MiB expanding at most 100 times (a ZIP bomb stops early); encrypted entries, links,
  special files and names that leave the folder are refused (`bicraft_opc::for_each_zip_entry`);
- only `package.json` and the files under `resources/` are kept, each path component made of
  ASCII letters, digits, `.`, `-` and `_` (no leading dot), at most 8 deep; other entries are
  listed in `skipped`;
- `visual.guid` is required and made of ASCII letters, digits and `_` (at most 128): it names a
  folder, so it can never leave `CustomVisuals/`;
- the `.pbiviz.json` is the one `package.json` names in `resources[]`, else the only
  `resources/*.pbiviz.json`; its `visual.guid`, when present, must be the manifest's.

The files go to `CustomVisuals/<guid>/` and the `CustomVisual` package to `resourcePackages` as
Desktop writes them; a visual with the same guid is replaced (its files and its package entry). A
legacy layout (`Report/Layout`) is refused: its resource packages use another shape that BIcraft
does not write (convert the report to PBIR first). `visual.remove_custom {name, force?}` drops the
package, its files and an AppSource reference of that name; it is refused while visuals use the
visual, unless `force`. Saving a PBIP removes the `CustomVisuals/<name>/` folder of each
`CustomVisual` package the report on disk registered and the new report no longer writes
(2026-11-23; before, the files stayed and the reopened report listed them as unregistered); a
folder no package registered is left alone. BIcraft's own visuals are listed even when not
embedded and are embedded on first use (docs/community-visuals.md "Built in by default").

**BIcraft never runs a custom visual's JavaScript.** The Visualizations pane lists each embedded
visual with the icon from its `.pbiviz.json` (`content.iconBase64`); a visual of that type is drawn
as a labelled placeholder: the icon above the visual's display name and title (`bicraft_render`,
docs/render.md). The snapshot carries both (`report.custom_visuals`, a visual's `custom_visual`,
docs/view-snapshot.md). The BIcraft Spec visual is the exception: BIcraft wrote its script and draws its
spec with the renderer's own layout ([spec-visual.md](spec-visual.md)).

Not checked against Desktop: a package whose `.pbiviz.json` is not named `<guid>.pbiviz.json` (its
item path then names its own file), and Desktop's own import dialog (whether it replaces a visual
of the same guid without asking).

## Visual projects

Sources: Microsoft Learn's "Power BI visual project structure", "Capabilities and properties of
Power BI visuals", "Data view mappings" and "Formatting model utils / format pane" pages; the
MIT-licensed `powerbi-visuals-api` package (its `schema.capabilities.json`, `schema.pbiviz.json`
and the `IVisualPlugin`, `FormattingModel` and `FormattingComponent` declarations of its
`index.d.ts`, version 5.3.0). No Microsoft code is copied: BIcraft writes its own files from
these descriptions.

A project is a folder that `pbiviz package` turns into a `.pbiviz`:

| File | What BIcraft reads or writes |
|---|---|
| `pbiviz.json` | `visual` (`name`, `displayName`, `guid`, `visualClassName`, `version`, `description`, `supportUrl`, `gitHubUrl`), `apiVersion`, `author` (`name`, `email`), `assets.icon`, `style`, `capabilities` (the paths of those files) |
| `capabilities.json` | `dataRoles` (`name`, `displayName`, `kind`: `Grouping`, `Measure`, `GroupingOrMeasure`), `dataViewMappings`, `objects` (format objects: `displayName`, `properties` with a `type` such as `{bool: true}`, `{numeric: true}`, `{text: true}`, `{fill: {solid: {color: true}}}`), `privileges` |
| `src/*.ts` | the visual's TypeScript; the entry is `tsconfig.json` `files[0]`, else `src/visual.ts`, and exports the class `visualClassName` |
| `style/visual.css` | its styles (`pbiviz.json` `style`) |
| `assets/icon.png` | the 20×20 icon of the Visualizations pane |
| `stringResources/<locale>/resources.resjson` | the visual's names in one language (`en-US`, `fr-FR`): an object of texts by the `displayNameKey` (or `descriptionKey`) capabilities.json gives a role, an object or a property (Microsoft Learn, "Add the locale in Power BI for Power BI visuals") |

### Localized strings

`visual.pack` (and BIcraft's own visuals) put the texts of `stringResources/<locale>/` into the
package's `.pbiviz.json` as `stringResources: {"<locale>": {key: text}}`, where the MIT
`powerbi-visuals-webpack-plugin` writes them (`src/extractor/localization.js` and
`getVisualConfig` in `src/index.js`, commit 8abcf7a5): a locale folder is ASCII letters, digits
and `-`, at most 64 locales and 2,000 texts each; values that are not texts are left out, and a
locale folder without its file is skipped with a warning. Since 2026-11-24 BIcraft's nine
visuals carry `en-US` and `fr-FR` texts for every data role and format name
([bicraft-visuals.md](bicraft-visuals.md) "Names in English and French"); not yet seen in a
French Desktop.

The host page runs the package's script, which registers the visual in
`window.powerbi.visuals.plugins[<guid>]` as an `IVisualPlugin`: `name` (the guid), `displayName`,
`class`, `apiVersion`, `create(options)` (a new instance of the class) and `custom: true`.

### BIcraft

- `visual.scaffold {folder, name, …}` writes a project: `pbiviz.json` (API 5.3.0, version
  1.0.0.0, a guid made of the name and 32 hexadecimal digits unless one is given),
  `capabilities.json` with the data roles and format options of its form (one categorical data
  view mapping: categories from the first grouping role, values bound to each measure role;
  `privileges: []`), `tsconfig.json`, an npm `package.json` that only lets editors fetch the
  API's types, `src/visual.ts` (a line per category, its format options read from the data view's
  `metadata.objects` and offered to the format pane by `getFormattingModel`: a card per object, a
  slice per property with the `ToggleSwitch`, `ColorPicker`, `NumUpDown` or `TextInput` control),
  `style/visual.css` and an icon BIcraft draws (three bars on a tile).
- `visual.build {folder, out?}` compiles the TypeScript with swc (`bicraft_tsbuild`): the `.ts`
  and `.js` files under `src/` are read in memory (capped: 512 files, 4 MB each, 16 MB in all,
  brackets nested at most 500 deep; links and `node_modules` skipped), types are stripped
  (type-only imports such as `powerbi-visuals-api` go with them; types are not checked), the
  relative imports are bundled with a registration module of BIcraft's into one script wrapped in
  a function, written to `dist/visual.js` by default. Refused, with the reason: a package import
  that is still there after the strip (npm packages: there is no `node_modules`), an import cycle,
  a `.tsx` or LESS file, a path in `pbiviz.json` that leaves the folder, an entry that does not
  export the class.
- `visual.pack {folder, out?}` builds the same way and writes `dist/<guid>.<version>.pbiviz`:
  `package.json` (`version`, `visual`, `author`, `resources: [{resourceId: "rId0", sourceType: 5,
  file: "resources/<guid>.pbiviz.json"}]`, `metadata.pbivizjson.resourceId`) and the
  `.pbiviz.json` (`visual`, `apiVersion`, `author`, `capabilities`, `content`: `js`, `css`,
  `iconBase64`). `visual.import_custom` reads it back (tested, and through PBIP and PBIX round
  trips).
- `bicraft visual new|build|pack` run these commands from the command line; Insert › Visuals ›
  New visual project, Compile a visual and Package a visual open their forms in the app.
- `visual.export_bicraft_visual {name, out}` (M12.5) writes the package of one of BIcraft's own
  visuals, the same bytes BIcraft embeds in a report, for Power BI Desktop's Import a visual from
  a file and for a tenant's organizational visuals: `name` is a visual's name or guid, a list of
  them or `all`; `out` is the `.pbiviz` file (one visual) or a folder, where each is written as
  `<guid>.<version>.pbiviz`. `visual.import_custom` reads every exported file back as the package
  `visual.add` would embed, and the capabilities validate against `powerbi-visuals-api` 5.3.0's
  schema (`builtin_visuals_tests.rs`). `bicraft visual export NAME... | --all [--out F|DIR]` and
  Insert › Visuals › Export a BIcraft visual run it. The release archives carry the nine files
  under `visuals/` (docs/releasing.md); their TypeScript is in this repository.

### Seen in Desktop

Black-box run on Power BI Desktop 2.159.1231.0 (Store edition), 2026-11-23. Two projects made
by `visual.scaffold` and packed by `visual.pack` were embedded with `visual.import_custom` in a
PBIX BIcraft wrote (table `T` with rows a, b, c of 100, 200, 442, measure `Total`, a card and the
two visuals bound to `T[K]` and `[Total]`). Desktop opened it, and the page's text was read
through the WebView2 DevTools port. Nothing was clicked, and none of Desktop's own scripts were
read.

- **The script's form.** Each visual runs in its own sandboxed frame. A script that only
  registers its plugin in `powerbi.visuals.plugins[<guid>]` is registered there but never drawn:
  the frame stays empty and no error is shown. With the bundle assigned to a global variable
  named after the guid, whose `default` is the plugin, Desktop creates the visual and updates it
  with its engine's data: the frames showed `a: 100`, `b: 200`, `c: 442`. That is the form the MIT
  `pbiviz` tools give it: their webpack build names the library after the guid
  (`microsoft/PowerBI-visuals-tools` `src/WebPackWrap.ts`), and their plugin module exports the
  plugin as `default` (`microsoft/powerbi-visuals-webpack-plugin`
  `templates/plugin-template.js`, commit 8abcf7a5). `visual.build` now writes
  `var <guid> = (function () { …; return { default: plugin }; })();`, so it refuses a guid that
  starts with a digit.
- **WebAssembly runs in the sandbox (M11.31).** The second visual compiles and instantiates a
  35-byte module written by hand (`(func (export "f") (result i32) i32.const 742)`) with
  `new WebAssembly.Module` and `new WebAssembly.Instance`, synchronously, from its own script,
  and draws the result. Desktop showed `WASM-OK 742`. The module's bytes were embedded in the
  script; a package has no other file the visual could fetch. wasm-bindgen output was not tried.
  Its glue code compiles the same way (`WebAssembly.instantiate`), and a module over 4 KB may
  need the asynchronous form on the main thread.

Not checked against Desktop: Desktop's own import dialog given a `visual.pack` package (only a
package embedded by BIcraft was opened), the format pane's cards, and `package.json`'s `build`
field, which `pbiviz package` writes (the tools' version) and BIcraft leaves out.
