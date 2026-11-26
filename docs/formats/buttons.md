# Buttons, shapes, images and navigators: actions, states, conditional destinations

Sources: Microsoft Learn, "Create and configure buttons in Power BI reports"
(`create-reports/desktop-buttons`, updated 2026-07-23), "Customize buttons in Power BI reports"
(`create-reports/power-bi-customize-button`), "Create page and bookmark navigators"
(`create-reports/button-navigators`), "Drillthrough in Power BI reports"
(`create-reports/desktop-drillthrough`), "Add text boxes, shapes, and smart narratives"
(`create-reports/power-bi-reports-add-text-and-shapes`), all read 2026-11-20; Microsoft's public
report theme schema (`crates/visuals/schemas/reportThemeSchema-2.157.json`: `visual-actionButton`,
`visual-shape`, `visual-pageNavigator`, `visual-bookmarkNavigator`, `visualLink`); the state
selectors of [visual-options.md](visual-options.md) ("State selectors", Desktop 2.158.1304.0
render captures). **Nothing on this page was compared with a file Desktop saved** unless it says
so: the property names are the schema's, where Desktop puts them is BIcraft's reading.

The insertion side (Insert › Buttons, Shapes, Image; the `visualLink` action types; the image
resources) is in [pbir.md](pbir.md), "Text boxes, images, buttons and page format".

## What Desktop offers (Learn) and where BIcraft stands

| Desktop (Learn) | BIcraft |
|---|---|
| Insert › Buttons: Left / Right arrow, Reset, Back, Information, Help, Q&A, Bookmark, Blank, Apply / Clear all slicers, Navigator › Page / Bookmark navigator | `button.add` (every kind) |
| Action types: Back, Bookmark, Drill through, Page navigation, Q&A, Web URL, Apply / Clear all slicers, Data function | `visual.set_action` (all but Data function, a preview feature calling Fabric user data functions: out of scope); readers run all but Q&A |
| Action on shapes and images ("Shapes and images for navigation") | `visual.set_action` takes buttons, images and shapes |
| Conditional (fx) Destination: "Select a single destination or use conditional formatting" (schema), the value of a field picked by a slicer, "exact string match" on page names | `visual.set_action {destination_field}`, evaluated by the snapshot, followed by the readers |
| Conditional tooltip ("you can conditionally format the Tooltip") | `visual.set_action {tooltip_field}` |
| Tooltip hidden when it repeats the button's text | the readers leave it out |
| Drillthrough button disabled without filter context; Enabled tooltip / Disabled tooltip (schema `enabledTooltip`, `disabledTooltip`) | `visual.set_action {enabled_tooltip, disabled_tooltip}`; readers draw the disabled state, take no click, show the tooltip |
| Button states Default, On hover, On press, Disabled (Loading: data functions only) for Shape and Style (Text, Icon, Outline, Fill, Glow, Shadow) | written by `visual.format {objects: [{…, state}]}` (Format pane › Apply settings to: State; the action editor's Button style); the readers draw text, icon, fill and border per state; glow and shadow per state are kept, not drawn |
| Shapes: text inside the shape, fill and outline per state, rotation | Format pane cards (Text, Fill, Outline with the state), `visual.format`; the readers draw the text (upright) and the press / hover fills |
| Navigators: Pages (show hidden pages, show tooltip pages), Bookmarks (bookmark group, Allow deselection, Launch on deselection, Hide deselection bookmark), Grid layout (Horizontal, Vertical, Grid, Padding), Selected state, Fill, Text, Outline, Shape, Shadow, Glow, Rotation | Format pane cards Grid layout, Pages, Bookmarks, Text, Fill, Border, Accent bar with the state (default, hover, press, selected); readers apply all but shape, shadow, glow and rotation of navigators |
| Fill images per state; Custom icon with Image fit and Icon placement (left, right, above, below, custom) and Icon size | not built (kept byte for byte when a file has them) |
| Image visual: Image fit Normal / Fit / Fill, an action | `image.add` / `image.set {scaling}`, `visual.set_action` |

## Conditional destinations (fx)

The schema types `navigationSection` and `drillthroughSection` as strings whose description says
"Select a single destination or use conditional formatting for custom destination logic, such as
multi-destination support". BIcraft writes a conditional destination as the field's expression
in place of the literal, the way Desktop writes a field-value fx elsewhere (the title's text, the
button's text; [visual-options.md](visual-options.md)):

```json
"visualLink": [{"properties": {
  "show": {"expr": {"Literal": {"Value": "true"}}},
  "type": {"expr": {"Literal": {"Value": "'PageNavigation'"}}},
  "navigationSection": {"expr": {"Measure": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Where"}}}
}}]
```

- The property is the type's own: `navigationSection` (Page navigation), `drillthroughSection`
  (Drill through), `bookmark` (Bookmark), `webUrl` (Web URL); `tooltip` for the tooltip.
- A column is read through `Aggregation` `Function: 3` (Min), Desktop's "First" for a text field
  (the same rule as `visual.conditional`'s field values).
- Learn: "Power BI uses an exact string match to set the drillthrough destination". The readers
  match the value to a page's name, then its display name; a bookmark's name, then its display
  name; a URL must be http, https or mailto. A value that names nothing disables the button.
- **Unchecked:** that Desktop writes exactly this `expr` (no Desktop-saved file was read).

The snapshot evaluates the field in the visual's filter context like any bound value
(`docs/view-snapshot.md`, "Bound values"): `action.dynamic_destination`, `action.dynamic_tooltip`,
and `interaction.button.{conditional, destination}`. The reader's click sends the destination it
shows (`view.interact {action: "button", destination}`); the engine accepts it only for a
conditional action.

## Drillthrough buttons: enabled and disabled

Learn (`desktop-drillthrough`): a drillthrough button is "disabled due to missing filter context";
the button states list Disabled "(for drillthrough)". BIcraft enables a drillthrough button when
the page has exactly one selected data point (the same rule the reader's click already applied)
and its destination is a drillthrough page of this report. `interaction.button.enabled` carries it;
`interaction.button.tooltip` is the conditional tooltip, else `enabledTooltip` / `disabledTooltip`
(schema "Enabled tooltip", "Disabled tooltip"), else `tooltip`. Desktop's own default disabled
tooltip text is not reproduced (none is published).

## States drawn by the readers

The state entries are the ones of [visual-options.md](visual-options.md) ("State selectors"):
`objects.<object>[{properties, selector: {id: "<state>"}}]` with `default`, `hover`, `press`,
`selected`, `disabled`. The snapshot carries what each state sets, over the default state:

| Visual | Snapshot | States |
|---|---|---|
| `actionButton` | `button.states.{hover, press, disabled}`: text, font colour and size, alignment, icon, icon colour and placement, fill, fill transparency, border (`outline`: show, colour, weight, transparency) | hover and press under the pointer, disabled from `interaction.button.enabled` |
| `shape` | `shape.states.{hover, press}`: fill, outline, text | hover and press |
| `pageNavigator`, `bookmarkNavigator` | `navigator.styles.{default, selected, hover, press}` | per navigator button: press over hover over selected over default |

The front end tells the renderer where the pointer is (`RenderOptions::pointer`: the visual, the
navigator button's page or bookmark name, pressed or not) and draws the page again when that
changes; the app does so only over visuals whose hover or press look differs. A disabled button
without a disabled look of its own is drawn at 45 % opacity: **BIcraft's choice** (Learn names the
state, not its default formatting). `bicraft render`, PDF export and the HTML viewer draw no
pointer state; the HTML viewer (`page.html`) does not draw the disabled state either.

## A shape's text

The schema's `visual-shape` `text` object: `text`, `show`, `fontColor`, `fontSize`, `fontFamily`,
`bold`, `italic`, `underline`, `horizontalAlignment`, `verticalAlignment`, the margins. BIcraft
writes it in the `default` state (render captures: Desktop reads a shape's `text` from its state
entries, [visual-options.md](visual-options.md)). The readers draw its lines upright in the
shape's box, centred unless aligned, 10 pt and the theme's text colour by default (BIcraft's
defaults; the text rotation `rotation.textAngle` is not drawn).

## Bookmark navigator: Allow deselection

Schema `bookmarkNavigator.bookmarks`: `allowDeselectionBookmark` ("Allow users to deselect buttons
in the bookmark navigator"), `deselectionBookmark` ("Launch this bookmark when a user deselects a
button"), `hideDeselectedBookmark` ("Hide the deselection bookmark from the bookmark navigator").
The snapshot's `navigator.{allow_deselection, deselection_bookmark, hide_deselection_bookmark}`.
With Allow deselection a click on the selected button applies the deselection bookmark, or, when
none is chosen, sends `view.interact {action: "bookmark_deselect"}` (no button selected, the view
unchanged: BIcraft's reading of Learn's "deselect all the buttons"). Hide deselection bookmark
leaves that bookmark out of the buttons.
