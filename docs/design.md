# Design system (the egui UI, desktop and browser)

Decision of 2026-11-04: BIcraft's one UI ([webui.md](webui.md), [desktop.md](desktop.md)) looks
like its sibling Crafting App [DeckCraft](https://github.com/storytold/deckcraft). It adopts
DeckCraft's design system (tokens, type scale, spacing, chrome, code-drawn icons, widgets) and
keeps BIcraft's own **amber** accent and logo instead of DeckCraft's orange. Light is the default;
dark is a variant the user picks. The desktop window and the browser page draw the same pixels.

This page is the specification. The code lives in `crates/ui` (`bicraft-ui`), in a `theme`
module and its neighbours (below). Source read: DeckCraft `crates/ui-egui/src/` (`theme.rs`,
`widgets.rs`, `ribbon.rs`, `panes.rs`, `status.rs`, `menus.rs`, `dialogs.rs`, `icons.rs`,
`thumbs.rs`, `canvas.rs`, `lib.rs`) and `crates/fonts/build.rs` at commit `0ec0cc5`
(2026-10-12), egui 0.36 in both projects.

## 1. Brand

| Element | Value | Use |
|---|---|---|
| Amber | `#f2a516` | the accent: tab underline, primary button, selected ring, slider, focus |
| Dark amber | `#d9820b` | pressed primary, selection ring on white, third logo bar |
| Teal | `#1f9e8f` | first logo bar; the "live" connection dot |
| Slate | `#2b3445` | logo square; text on amber fills |

The logo is `assets/logo/bicraft-icon.svg`: a slate square (corner radius 18 % of its side), an
amber frame inset 10 % (stroke 3 %, radius 11 %), three bars rising left to right in teal, amber
and dark amber. In the UI it is **painted in code** from those proportions
(`theme::paint_logo(painter, rect)`, the counterpart of DeckCraft's `dialogs::paint_logo`), so no
image is decoded and it is sharp at any DPI. Sizes: 18 px in the title row, 40 px on the start
screen, 64 px in About.

Text on an amber fill is **slate, never white**: white on `#f2a516` is 2.1:1, slate is 6.0:1
(DeckCraft's orange takes white text; amber does not).

## 2. Colour tokens

One `Tokens` struct, two constructors (`Tokens::light()`, `Tokens::dark()`), read per frame with
`Tokens::get(ctx)` (from `visuals.dark_mode`), applied once by `theme::apply(ctx, &tokens)`. No
widget names a colour literal; everything comes from here (DeckCraft AGENTS.md: "colours come
from theme tokens"). The first block is DeckCraft's tokens **verbatim** (same names, same
values) except the accent family; the second block is BIcraft's additions for a data tool.

### DeckCraft tokens (neutral values unchanged)

| Token | Light | Dark | Used for |
|---|---|---|---|
| `chrome` | `#f5f5f5` | `#262626` | window background: title row, tab row, side panes, status bar |
| `card` | `#ffffff` | `#333333` | ribbon card, right task pane, popovers, dialogs, model table cards |
| `card_border` | `#e3e3e3` | `#444444` | card outline, the 1 px "shadow line" under the ribbon card |
| `canvas` | `#f0f0f0` | `#1e1e1e` | around the report page and behind the model diagram |
| `text` | `#242424` | `#eeeeee` | body text, icon ink |
| `text_dim` | `#5c5c5c` | `#bbbbbb` | secondary text, status bar, unselected list numbers |
| `text_faint` | `#999999` | `#888888` | hints, disabled labels, shortcuts in menus |
| `border` | `#d6d6d6` | `#4a4a4a` | field outlines, unselected thumbnail outline |
| `separator` | `#e1e1e1` | `#484848` | ribbon group separators, pane rules |
| `hover` | `#ebebeb` | `#404040` | hovered button / tab / row background |
| `pressed` | `#dedede` | `#4c4c4c` | pressed button, open menu button |
| `selected` | `#e6e6e6` | `#474747` | toggled-on button, selected list row |
| `selection` | `#7a7a7a` | `#aaaaaa` | neutral selection outline (multi-select boxes) |
| `handle_fill` / `handle_stroke` | `#ffffff` / `#6e6e6e` | same | resize handles (reserved: editing) |
| `caret` | `#000000` | `#000000` | DeckCraft value; BIcraft uses egui's caret (`text`), see §2.4 |
| `text_selection` | `#5a8fe0` α 90 | same | text selected in a field (DeckCraft keeps blue) |
| `field_bg` | `#ffffff` | `#2a2a2a` | text fields, dropdown fields |
| `shadow` | black α 28 | black α 90 | window, popup and page shadows |

### Accent family (BIcraft amber replaces DeckCraft orange)

DeckCraft: light `accent #d95a1a`, `contextual #c24e14`, `guide #e85d2a`; dark `accent #f26b1d`,
`contextual #f58a4b`, `guide #f28a4b`; `accent_text` white. BIcraft:

| Token | Light | Dark | Rule |
|---|---|---|---|
| `accent` | `#f2a516` | `#f2a516` | brand amber, fills and 2–3 px marks only (2.1:1 on white: never text) |
| `accent_hover` | `#e6990f` | `#f5b942` | primary button hovered (light darkens, dark lightens, as DeckCraft's light/dark pair does) |
| `accent_pressed` | `#d9820b` | `#d9820b` | primary button pressed |
| `accent_text` | `#2b3445` | `#2b3445` | text and icons on any accent fill (6.0:1 on amber, 7.1:1 on the dark hover) |
| `contextual` | `#9a5a00` | `#f5b942` | accent-coloured **text**: contextual tab labels, transient status message, links (5.0:1 on `chrome` light, 8.6:1 dark) |
| `ring` | `#d9820b` | `#f2a516` | selection ring: selected page thumbnail, selected visual, selected table card, focused field (2.9:1 on white, 7.3:1 on `chrome` dark; always ≥ 2 px) |
| `accent_soft` | amber α 40 | amber α 56 | background of a selected row inside a card (details lists, palette), highlighted relationship halo |
| `guide` | `#f2a516` | `#f5b942` | reserved (alignment guides when editing comes) |

`egui::Visuals.selection` (egui uses it both for selected text and `selectable_label`): `bg_fill`
= `accent_soft` at double alpha, `stroke` = `text`. DeckCraft sets `accent × 0.85` with white
text, which is unreadable with amber in dark mode. `hyperlink_color` = `contextual`.

### BIcraft additions (status colours)

A report tool shows errors, warnings and states that DeckCraft has no tokens for. They follow the
viewer's palette (`page.html` `:root`), darkened where needed for 4.5:1 on `card`, and **always
come with an icon and a word** (never colour alone):

| Token | Light | Dark | Used for |
|---|---|---|---|
| `err` | `#cf3434` | `#ff6b6b` | invalid measure, refused action, offline |
| `err_soft` / `err_text` | `#fde7e9` / `#a4262c` | `#ff6b6b` α 33 / `#ff8a8a` | the error bar above the page (6.2:1) |
| `warn` | `#a94f07` | `#f59e4b` | warnings; deliberately redder than the accent (hue 25° vs 39°) so a warning never reads as "selected" |
| `ok` | `#157a6e` | `#3cc2b0` | valid model, accepted command (`✓`) |
| `live` | `#1f9e8f` | `#1f9e8f` | connection dot "live" (the logo teal; a dot, not text) |

The model diagram's palette (`Pal` in `model_view.rs`, today the viewer's blue-accented colours)
maps onto the tokens: `bg → canvas`, `surface → card`, `surface2 → hover`, `text`, `muted →
text_dim`, `faint → text_faint`, `border`, `accent → ring`, `accent_soft`, `err`, `err_soft`,
`shadow`. Type badges keep their own hues (data, not chrome).

The **report page** keeps the report's own colours (theme, page background, wallpaper): the
tokens never touch what `bicraft-render` draws. Only the frame around it (canvas, shadow) is
themed, so a page looks the same in light and dark, as in Power BI.

### 2.4 `theme::apply`

Port of DeckCraft's `apply` with the accent changes above: `Visuals::light()/dark()` as the base;
`panel_fill = chrome`, `window_fill = card`, `extreme_bg_color = field_bg`, `faint_bg_color =
hover`, `window_stroke = 1 px card_border`; widget radius 5 (all states); `noninteractive.bg_stroke
= 1 px separator`; inactive: transparent weak fill, `field_bg` fill, no stroke; hovered: `hover`;
active and open: `pressed`; `window_shadow {offset [0,4], blur 18, color shadow}`, `popup_shadow
{[0,3], 12}`; `override_text_color = text`. Spacing: `item_spacing (6, 4)`, `button_padding (6,
3)`, `interact_size.y 22`. Text styles below (§3). The caret stays egui's (`text` colour):
DeckCraft's black caret disappears on `field_bg` dark.

## 3. Typography

| Style | Size | Family | Where |
|---|---|---|---|
| `Body`, `Button` | 13 | regular | default text, buttons, list rows |
| tab label | 13 | regular; **semibold** when active | tab row |
| title | 13 | regular | title row (centred document title) |
| pane title | 16 | semibold | right task pane header |
| section header | 13 | semibold | collapsible pane sections |
| `Small` | 11 | regular | big-button labels, pills, badges |
| field / small button / status bar | 12 | regular | fields, dropdowns, status bar text |
| `Heading` | 18 | semibold | empty states, dialogs' first line |
| start screen | 26 / 16 | semibold | product name / section headings |
| `Monospace` | 12 | Hack | DAX, JSON, type badges |

**Font: Selawik, not Inter.** DeckCraft's UI face is Inter (Regular, SemiBold, Bold), embedded only
when the build is given a craft-fonts checkout (`CRAFT_FONTS_DIR`); without it DeckCraft falls back
to egui's Ubuntu Light, and its web build embeds Inter Regular and Bold only (no SemiBold, so its
`bold()` family falls back in the browser). BIcraft keeps the **Selawik** faces the ui-core lot
installs (`crates/ui/src/fonts.rs`):

- they are always embedded (`bicraft-render` bundles them for the pages), so every build, every
  CI run and both shells draw the same glyphs with no external checkout and no extra wasm bytes;
  Inter would add four faces to a bundle already near 4 MB gzip;
- the report pages are drawn in Selawik (Segoe UI metrics, Power BI's font), so chrome and page
  share one face;
- Selawik has the weights DeckCraft's scale needs: Regular for body, **Semibold** for DeckCraft's
  `theme::bold()` (DeckCraft maps it to Inter SemiBold), Bold, Light.

DeckCraft's sizes are kept as they are; Selawik is slightly narrower than Inter at the same size,
which only gives labels a little more room. The viewer's symbols (`✓`, `✕`, `✖`, `⚠`, `●`, `▶`…)
come from a 26 KB subset of **Noto Sans Symbols 2** (OFL-1.1, release v2.008,
`crates/ui/fonts/SOURCE.txt`), placed right after Selawik in every family, so `✓` and `✕` are
written exactly as page.html writes them. Next come the renderer's bundled Noto subsets
(2026-11-05; `crates/render/fonts/SOURCE.txt`, docs/render.md "Bundled fallback faces"): Noto
Sans Regular (Bold in the semibold and bold families) for Greek, Cyrillic, currency and
superscripts, then Noto Sans Symbols 2, Noto Sans Math and Noto Sans Symbols for shapes, arrows
and operators. The report pages embed them anyway, so the chrome gets them for no extra bytes,
and a measure named `CA Δ N-1` looks the same in an answer grid and on the page. Other glyphs
come from egui's embedded faces (Ubuntu Light, Noto Emoji…; already in the chain); a character
none has is drawn by egui as its replacement box, never dropped.

**Pages: no character is dropped.** The page renderer draws a character no bundled face has
(CJK, which is not bundled: docs/gaps.md G43) as an outlined box and lists it in
`report.render`'s `missing_glyphs`.

**Report pages use the bundled faces too** (decision of 2026-11-04): the app's renderer
(`bicraft_ui::pages::font_book`) reads no system font, on the desktop as in the browser, so a
page is the same pixels in both. `bicraft-cli render` keeps the system's fonts (Segoe UI on
Windows), for comparisons with Power BI Desktop. Chevrons and arrows in chrome are **painted** (DeckCraft's `chevron()`),
never typed, so no glyph is missing anywhere. API: `theme::font(size)`, `theme::bold(size)`
(= `fonts::semibold_px`), same names as DeckCraft.

## 4. Spacing, radii, shadows

| Item | Value (logical px) |
|---|---|
| Title row height (`TITLE_H`) | 38 |
| Tab row | 28 (tabs 24 high, 18 px horizontal padding, 2 px apart, first at x = 12) |
| Ribbon card | inset 8 px from the window, top at 30, content `RIBBON_H` 66 + 10, radius 10, 1 px `card_border` line under it |
| Ribbon collapsed | tab row + 4 |
| Status bar | 28 |
| Left pane (pages) | default 236, 150–420, resizable |
| Right pane (details) | default 300, 240–520, resizable, inner margin 10 |
| Big button | 32 px icon (painted at 30) over 1–2 label lines of 11 pt, height 64, min width 40 |
| Small button / icon toggle | 22 high; 17 px icon; toggle 24 × 22; status-bar icon 26 × 20 |
| Field, dropdown | 22 high, radius 4, 1 px `border` (`text_faint` hovered) |
| Corner radii | buttons and hovers 4, egui widgets 5, menus 8, windows/cards 10, pills 11 (full), primary pill 13 |
| Shadows | window `[0,4] blur 18`, popup `[0,3] blur 12`, page: `shadow` rect offset 1.5 px down, 1 px larger, radius 2 |
| Pane section header | 26 high, chevron at x+8, title at x+18 |
| Dialog | min width 340, label column 120, field 180, buttons 72 × 24 |

## 5. Chrome layout

Top to bottom, the same in both shells (in the browser it is the whole page; on the desktop it sits
under the OS title bar, as DeckCraft on Windows and Linux):

```text
┌ title row (chrome, 38) ────────────────────────────────────────────────────────────────────┐
│ [logo] [open][reload]      Sales.pbix · PBIX · rev 12 · ● model        [◐][search][Open in Desktop] │
├ tab row (chrome, 28): File Home Report Model Data Analyze View Optimize (contextual: Visual, …) ┤
│ ╭ ribbon card (card, radius 10) ───────────────────────────────────────────────────────────╮ │
│ │ groups of big and small buttons for the active tab, separators 13 px                     │ │
│ ╰──────────────────────────────────────────────────────────────────────────────────────────╯ │
├ formula bar (chrome, with a measure, calculated column or table selected): ✗ ✓ Name = DAX … ┤
├ left pane (chrome) ┬ canvas (canvas) ────────────────────────────┬ right pane (card) ───────┤
│ pages / tables     │      error bar (err_soft, radius 8)          │ Visual · Page · Table…   │
│                    │      ┌──── report page, shadow ────┐         │ sections, details        │
├ status bar (chrome, 28): Page 2 of 7 · message (contextual, 6 s)  ── role · ● live · zoom ─┤
```

- **Title row** (DeckCraft `ribbon::title_bar`): left, the logo then quick-access icon toggles
  (Open and Recent when the shell can open files, Reload; Reload has no shortcut: F5 is the
  registry's `model.refresh`); centre, the document title with its
  pills (§7); right, right-to-left: the primary pill button, search (command palette, Ctrl+Shift+P),
  the light/dark toggle. Double-click and drag move the window on the desktop only when the shell
  draws its own decorations (not now).
- **Tab row** (DeckCraft `ribbon::show`): the tabs of the engine's command registry
  ([commands-ui.md](commands-ui.md), ROADMAP M11): File (the Backstage), Home, Insert, Report, Model,
  Data, Analyze, View, Optimize, then contextual tabs in `contextual` colour that appear with a selection (a
  visual selected on the canvas → **Visual**; a table or column in the Model view → **Table**; a
  measure → **Measure**; a column in the Data view → **Table**, its Column tools). Insert, Report
  and Visual show the Report view, Data the Data view, Model and Measure the Model view (Table keeps
  the view it appeared in); the Validation view is Analyze's; Home, Analyze, View and Optimize keep
  the view shown.
  The active tab is semibold with a 3 px `accent` underline (width − 20); hover is a rounded
  `hover` fill; clicking the active tab (or double-clicking any) collapses the ribbon card, state
  kept in the prefs. The **Home** tab replaces the egui menu bar (File, View) of today.
- **Ribbon card**: DeckCraft's white card with groups (`widgets::group` + `separator`). Each tab
  starts with the view's own controls (pure UI state), then one group per registry group, in the
  registry's order (`crate::ribbon`): a group of one or two commands has big buttons, a larger one
  columns of three small buttons; the card scrolls sideways when a tab is wider than the window.
  - Home: File (Open, Connect to server, Reload, Recent files ▾, Recent servers ▾, Quit), Display
    (Light/Dark), then Share, Data, Calculations, Queries, Undo.
  - Report: Pages (Previous, Next), then Report, Visuals, Report
    measures, Spec, Pages.
  - Model: View (Fit, Zoom −/+, Actual size), then Properties, Relationships, Calculations,
    Calculation groups, Security, Translations.
  - Data, View: the registry's groups only. Analyze: Show validation, the registry's groups, then
    the validation counts. Optimize (Desktop's tab of that name, G60): Pause visuals (Pause
    visuals, Refresh all visuals), Optimization presets, then the Performance analyzer button
    that Analyze › Performance also has.
  - Contextual Visual, Table, Measure: the registry's groups for the selected object.
  Every command button has its label (EN/FR), icon and a tooltip (the engine's label, the
  shortcut, the id); greyed, the tooltip adds the reason (`commands.disabled` of the snapshot, the
  reader role on top). A click runs the command, opens the parameters form when it needs values,
  or shows a read command's answer. Groups whose shell lacks the capability are absent, never
  disabled-for-ever (`shell::Caps`).
- **Left pane**: DeckCraft's thumbnail pane (`thumbs.rs`) for pages: number at the left (12 pt,
  `text_dim`, `text` when selected), hidden pages dimmed with the struck number, issue count as a
  small `warn`/`err` badge, selected entry with a 3 px `ring` outline radius 4. Phase B ships
  named rows in that frame (26 px rows); page thumbnails rendered by `bicraft-render` at thumbnail
  size, lazily and with a budget, come later. In the Model tab the same pane lists tables.
- **Canvas**: `canvas` colour; the page centred with DeckCraft's page shadow; the error bar sits at
  the top of the canvas as a card (`err_soft`, radius 8, warning icon, close ×).
- **Right pane**: DeckCraft task pane (`panes::show`): `card` fill, inner margin 10, title 14
  bold with a 20 px collapse icon (hover fill), scrolling body of collapsible sections
  (`widgets::section`). Visual details, page info, model details all live here.
- **Right rail**: the closed right panes share **one** 30 px `chrome` strip at the window's edge,
  one icon per closed pane (layers, pin, link, table, chart, filter, top to bottom), so closing a
  pane never costs another strip: the canvas keeps its width. A click opens that pane
  (`panes::RAIL`); the icon says which pane it opens.
- **Selection pane**: a fourth right pane, rightmost and closed by default (its rail icon opens
  it), 230 wide and resizable within the panes' range. Rows 20 px high,
  indented 12 px per group level (96 px at most), the group's fold chevron then its label, the
  eye (24 × 22) and, in tab-order mode, the number (`text_dim`, 12 pt) in a right-to-left layout.
  A hidden row's label is `text_faint`; the selection is `strong` (the engine's selection, shared
  with the canvas). The rename edits in place, a one-line `TextEdit`; the layer and tab-order
  buttons sit in a bottom panel at the pane's foot (small buttons, icons and words), so a short
  window scrolls the list and never clips them.
- **Bookmarks pane**: left of the Selection pane, closed by default (its rail icon opens it),
  230 wide and resizable within the panes' range. Rows 20 px high, a group's caption
  `text` semibold with its fold chevron, its bookmarks indented 24 px; a bookmark's notes (how
  many hidden visuals and filters it captured) in `text_dim` 12 pt at the right. The selected row
  is `strong`; the rename edits in place, a one-line `TextEdit`. Add / View / Update / Rename /
  Delete / Group / Ungroup and the two moves, then Desktop's Data / Display / Current page
  checkboxes, sit in a bottom panel at the pane's foot so a short window never clips them.
- **Sync slicers pane**: left of the Bookmarks pane, closed by default (its rail icon opens it),
  230 wide. The slicers are grouped under their page's name (`text_dim` 12 pt
  semibold), each row its title (or type) and, when grouped, the group's name and the options it
  carries (`fields · filters`, `text_faint`). The footer holds the group name field, the Fields /
  Filters checkboxes and the Synchronize / Unsync buttons, in a bottom panel.
- **Status bar** (DeckCraft `status::status_bar`): left "Page 2 of 7" (12 pt, `text_dim`), the
  language, then the transient message in `contextual` for 6 s; right, right-to-left: fit icon,
  zoom %, zoom in, a 110 px log slider 10–400 %, zoom out, then view buttons (Report, Model,
  Validation as small icons, the active one `selected`), then the role, the connection pill and the
  last-command pill.
- **Start screen** (DeckCraft `dialogs::start_screen`): shown when nothing is open, in the canvas:
  the logo 40 px and "BIcraft" 26 semibold, 40 px left inset; "Open…" and "Connect to server…"
  buttons; Recent files and Recent servers as link lists. The browser never shows it (its source
  is the page's).

## 6. Icons

DeckCraft's icons are drawn in code (`icons.rs`): each is an original drawing with egui painter
primitives on a **24 × 24 design grid**, scaled to its rect (`Pen { o, k = size / 24 }`), line
icons in the theme's ink with one or two fixed accent colours, a stroke of size/16 at 16 px
growing to size/20 at 32 px (never under 1 px), and, when disabled, everything in ink at 35 %
alpha. Icons are an enum with stable kebab-case names (`Icon::name`, `Icon::from_name`), so tests
and the accessibility tree can name them. BIcraft ports the toolkit and takes the subset it needs,
adding a few drawn the same way:

| From DeckCraft | New for BIcraft |
|---|---|
| `home`, `folder`, `file`, `search`, `settings`, `help`, `info`, `warning`, `lock`, `eye`, `eye-off`, `close`, `check`, `plus`, `minus`, `chevron-*`, `more`, `zoom-in`, `zoom-out`, `fit-small`, `fit-to-window`, `table`, `chart`, `link`, `export`, `grid`, `list`, `collapse`, `expand`, `pin`, `star`, `sparkle`, `palette` | `reload`, `page`, `model` (three linked cards), `validate` (check in a shield), `measure` (Σ), `column`, `relationship`, `server`, `sun`, `moon`, `open-in-desktop` (`export` with a window), `wireframe`, `logo`, `calculator` (the Data pane's measures); for the command registry's buttons (M11): `calendar`, `compare` (two pages), `delete` (a bin), `edit` (a pencil), `filter` (a funnel), `format` (a roller), `hierarchy`, `import` (`export` reversed), `layers`, `link` (two links), `mobile`, `palette`, `play`, `undo`, `redo`, `rename` (a field with a caret), `save-as` (`save` with a pencil), `sort`; for the Edit interactions mode (M11.15): `highlight` (three dimmed bars, the middle one opaque over its dimmed part), `no-interaction` (a struck circle); for the Insert tab (M11.20): `text-box` (a T in a frame), `image` (a frame with a hill and a sun), `button` (a rounded button with a pointer) |

- Accent constants: DeckCraft's `ORANGE` becomes BIcraft amber `#f2a516`; `BLUE #2e6fd8`,
  `GREEN #2ea36f`, `PURPLE`, `RED #d83f6b`, `GOLD #e3b21c` and the `DARK` ink stay. `warning` uses
  `warn`, `check` uses `ok` when it states validity.
- The light/dark toggle shows `moon` in light mode and `sun` in dark mode (DeckCraft uses
  `palette`/`sparkle`; a sun and a moon say what the button does).
- The gallery pictures of the visual types (`icons::paint_visual_type`) follow the same rules:
  light axes in ink at 45 %, marks in `BLUE` at full, 55 % and 25 % (`tint`), radius 0.4. BIcraft's
  own ten visuals are drawn the same way from marks kept in `crates/visuals/src/icons.rs`, so the
  engine rasterizes the identical drawing into each package's 20 × 20 icon
  (docs/community-visuals.md "Icons").
- `icons::paint(painter, rect, icon, ink, disabled)` is the only entry point; it returns early on a
  non-finite or sub-pixel rect, as DeckCraft's does.

## 7. Widgets

All in `crates/ui/src/theme/widgets.rs` (or `widgets.rs` beside `theme.rs`), ported from DeckCraft
`widgets.rs` unless marked new. Every interactive widget has an accessible label (its text or its
tooltip) so egui_kittest and AccessKit find it.

| Widget | Spec |
|---|---|
| Hover/press backgrounds | DeckCraft `bg()`: `selected` when on, `pressed` while held, `hover` when hovered and enabled; radius 4 |
| `big_button`, `big_button_ex` (split) | 32 px icon over a label wrapped on two lines near the middle; split: lower part opens a menu with a painted chevron |
| `small_button`, `small_button_ex` | 17 px icon + 12 pt label, 22 high; split arrow 12 px |
| `icon_toggle` | 24 × 22 icon button, `selected` fill when on, tooltip = label + shortcut |
| Primary pill (new name; DeckCraft's inline "Present") | `accent` fill, `accent_text` semibold 13, radius 13, min 84 × 26; `accent_hover` / `accent_pressed`; disabled: `hover` fill and `text_faint` with the reason as tooltip |
| Secondary button | egui `Button` under `apply` (transparent at rest, `hover`, `pressed`), radius 5 |
| Tabs | the tab row (§5); inside panes, segmented icon tabs as DeckCraft's Format pane (icon toggles in a row) |
| Pill (new) | 20 high, radius 11, 11 pt, padding 8; neutral: `selected` fill, `text_dim`; with a 6 px leading dot in a status colour; states below |
| Toggle | egui checkbox and radio under `apply`; the light/dark switch is an `icon_toggle` |
| Field | egui `TextEdit` on `field_bg`, 1 px `border`, `ring` 2 px when focused, radius 4 |
| `dropdown` | DeckCraft field-looking button with a painted chevron; popup is `Popup::menu`, scroll 360 max |
| `spinner` | DeckCraft number field with painted up/down steppers (zoom %) |
| `section` | collapsible pane header: chevron, 13 semibold |
| `drop_button` | button with painted chevron (no ▾ glyph) |
| Popover | `Popup::menu`, `card` fill, radius 8, popup shadow; slicer popups (dropdown, between, relative date, text) use it with `CloseOnClickOutside` like DeckCraft's colour popup |
| Tooltip | egui tooltip under `apply` (`card`, `card_border`, radius 8); data-point tooltips: rows as a two-column grid, the highlighted value semibold |
| Dialog | DeckCraft `dialogs::show`: `egui::Window`, not collapsible or resizable, anchored centre −40 px, min width 340, `field` rows (label 120, field 180), buttons right-aligned: primary (OK label) then Cancel, Enter = OK, Escape = Cancel |
| Command palette | DeckCraft `menus::palette`: Ctrl+Shift+P, a 520 × 380 window 90 px from the top, a search field, selectable rows with shortcuts in `text_faint`, ↑ ↓ Enter Escape |
| Error bar (new) | card on `err_soft`, radius 8, `warning` icon in `err`, text `err_text` wrapped (at most 120 px high, then it scrolls), close × 20 px |
| Empty state | logo or icon 32 px, `Heading` line, `text_dim` sub-line, optional primary pill |

Pill states (header and status bar):

| Pill | Look |
|---|---|
| format (`PBIX`, `PBIP`, `PBIT`, `BIM`, `server`) | neutral |
| `rev 12` | neutral, monospace digits |
| unsaved `● model`, `● report` | dot in `accent`, text `text` |
| last command `✓ report.set_visual 12:03` / `✕ …` | dot `ok` / `err`; flashes (`accent_soft` fill fading over 1 s) when it changes |
| connection: connecting / live / offline · retrying in N s | dot `warn` / `live` / `err` |
| role: reader / editor | neutral; editor with an `accent` dot |
| newer snapshot version | `warn` dot, tooltip with the version |

## 8. Light, dark and high contrast

- **High contrast** (2026-11-19) is a third token set (`Tokens::high_contrast`): white text on
  black (21:1), every text token at 7:1 or more on the chrome, cards, fields, hover and selected
  rows (WCAG 1.4.6, `high_contrast_tokens_meet_7_to_1`), white borders, yellow marks, ring and
  links, no shadow. It is egui's dark style with a black chrome (`Tokens::get` tells it from dark
  by the panel fill). Home › High contrast picks it; `"theme": "high-contrast"` in the prefs. The
  browser follows the system's high contrast (`forced-colors: active` or `prefers-contrast:
  more`) while no theme is stored. The desktop does the same with Windows contrast themes, read
  once at start from the per-user high-contrast flags (`HKCU\Control Panel\Accessibility\HighContrast`
  `Flags`, its "on" bit, through `windows-registry`'s safe API; `bicraft_desktop::contrast`); a
  change while the app runs is not followed. On Linux winit and eframe expose only light and
  dark, so the desktop does not follow a desktop environment's contrast preference (the Home
  button instead).
- **Light by default**, whatever the OS says. The toggle (title row, Home tab, palette command
  "Dark mode") switches and repaints at once (`theme::apply`).
- Kept as `"theme": "light" | "dark"` in the prefs (`crates/ui/src/prefs.rs`), saved through the
  shell's `PrefsStore`: the desktop's settings file; in the browser a new store over
  `localStorage` that holds **only UI preferences** (theme, fit mode, ribbon collapsed, remembered
  tab) and never a token, a path, a server URL or report data (approved 2026-11-04; the rule is in
  [webui.md](webui.md), "Browser storage"). Every storage access is wrapped: blocked, absent or
  full storage, or a damaged value, falls back to the defaults. A missing or unknown theme is
  `light`.
- Both themes are built from the same token names; tests check every text/background pair (§10).

## 9. Screens → components

| BIcraft screen (today) | DeckCraft component | Notes |
|---|---|---|
| Menu bar File / View | Home tab of the ribbon + title-row quick access + start screen | menu bar removed |
| Tabs Report / Model / Validation (`selectable_label`s) | ribbon tab row | Visual / Table as contextual tabs |
| Report: page list (left `selectable_label`s) | removed (2026-11-05): the page tabs at the bottom are the one page navigator | the user kept only the bottom tabs |
| Report: page canvas | slide canvas (`canvas.rs`): `canvas` fill, page shadow, status-bar zoom | the page pixels stay `bicraft-render`'s |
| Report: visual details (parity item) | right task pane (`panes.rs`) with sections | fields, data preview, position, raw JSON |
| Report: slicer popups | `Popup::menu` + colour-popup close behaviour | fields as §7 |
| Model: diagram | canvas + DeckCraft cards (`card`, radius 10, `card_border`, `shadow`) | `Pal` → tokens; selected card `ring` |
| Model: bar (− + 100 % Fit) | Model tab ribbon group + status-bar zoom | |
| Model: details panel | right task pane | |
| Validation | canvas with cards per section (`card`, radius 10), rows with `err`/`warn` icons | jumps → contextual tabs |
| Header: file, format | title row centre: name + format pill | path in the tooltip (RTL-clipped) |
| Header: rev, unsaved | title row pills | DeckCraft's "— Edited" suffix as pills |
| Header: last command, connection, role | status bar pills, right side | |
| Status line + spinner | status bar left (transient message in `contextual`, §5) + spinner | |
| Open in Power BI Desktop | title row primary pill (DeckCraft's "Present") | only with `Caps::open_in_desktop`; unsaved warning as a `warn` dot |
| Download (server) | Home tab File group | with `Caps::download` |
| Connect dialog | DeckCraft dialog (`dialogs::show`, `buttons`, `field`) | token field masked; primary "Connect" |
| Red error bar | error bar card (§7) | |
| Empty states | start screen / empty state widget | |
| About (new) | DeckCraft `about` | BIcraft wording: never "clean-room" (AGENTS.md §1) |
| Report: Edit interactions (Desktop's mode) | ribbon toggle (a big button kept pressed) + one icon strip per target | below |

### Edit interactions (M11.15)

Desktop's "Edit interactions" is on its Format tab; here it is on the contextual **Visual** tab,
Interactions group, next to **Interaction settings** (`report.settings`). The button is a toggle:
kept pressed (`selected` fill, `toggled` in the accessibility tree) while the mode is on. The
Visual tab stays shown while the mode is on, even with nothing selected, so the button can turn
it off; Escape turns it off too (the source stays selected). Readers get the button greyed with
the reason. The palette's `page.interactions` still opens the generic form.

With the mode on, the selected visual is the source (its `ring` outline as usual), and every other
visual with fields gets an icon strip at its top-right corner, inside the visual: a `card` panel
with a 1 px `border`, radius 4, holding 20 × 20 buttons 2 px apart with the icons `filter`,
`highlight` and `no-interaction` in the theme's ink. Only the modes the pair can take are drawn (a
slicer source, or a target that cannot highlight: Filter and None), from the snapshot's
`interactivity` (docs/view-snapshot.md), which the viewer's rules use too. The current effective
mode has an `accent_soft` fill; a rule of the page adds a 1.5 px `ring` stroke, the default has the
fill only and the tooltip "Default: Highlight" (FR "Par défaut : …"). The difference is never
colour alone: the accessibility tree names each icon "Mode: target title" with its `toggled`
state. Hover is a `hover` fill. A click on another mode runs `page.interactions` with one rule (one
undo step); a click on the default mode removes the rule; a click on the marked mode does nothing.
Clicks on data points still go to the viewer, so the page previews the rules being edited;
leaving the mode clears the page's data-point selection. The icons take keyboard focus: Tab moves
through them, Enter or Space runs one. Targets with the same title are named "Title (name)".
A selected visual group is the source for all its visuals: they take no icons; another visual
shows the modes any of them can take, and when they reach it differently no icon is marked (the
tooltip adds "the group's visuals differ here"); a click writes the rule for every one of them.

Screenshots (`gpu-tests`, `BICRAFT_DESKTOP_SHOTS`): `interactions-light`, `-dark`, `-fr` and their
`-preview` (a column clicked), from `interactions_screenshots`.
| Text box editor (new, M11.20) | DeckCraft `card` toolbar floating over the canvas (`egui::Area`), a frameless `TextEdit` with a layouter over the box | the box is white with a 1.5 px `ring` outline; dynamic-value tokens on `accent_soft`; the run colours are the report's own |
| Insert › Buttons, Action, Image, Page format (new, M11.20) | DeckCraft dialog windows (`egui::Window`, grids of label + control) | one Apply per window, one command per Apply |

## 10. Porting: verbatim, adapted, not taken

DeckCraft is the same owner's project, under **MIT OR Apache-2.0** like BIcraft
(© 2026 ArtCraft Team and the DeckCraft contributors). Ported files carry a header naming the
upstream file and commit (`DeckCraft crates/ui-egui/src/widgets.rs @ 0ec0cc5, MIT OR
Apache-2.0`), DeckCraft gets a row in `THIRD_PARTY_NOTICES.md` and `ATTRIBUTION.md` (code ported,
icons drawn in code by the DeckCraft contributors), and `NOTICE` keeps its copyright line. Its
licence texts are the ones BIcraft already ships, so `licenses/` gains nothing.

| | What |
|---|---|
| **Verbatim** (rustfmt'd to BIcraft's width) | `Tokens` neutral values; `apply` structure; `widgets`: `bg`, `big_button(_ex)`, `wrap_label`, `chevron`, `small_button(_ex)`, `icon_toggle`, `group`, `separator`, `rows`, `dropdown`, `spinner`, `fmt_num`, `section`, `drop_button`; `status_bar` layout and `small_icon`/`labeled`; the tab-row and ribbon-card drawing of `ribbon::show`; `panes::show` frame; `dialogs::show`/`buttons`/`field`; `menus::palette`; `icons.rs` toolkit (`Pen`, path helpers, `triangulate`, `stroke_width`, the `icons!` macro) and the icons listed in §6 |
| **Adapted** | accent family and `accent_text`; `selection` visuals; caret; fonts (Selawik, `bold()` = semibold); logo; icon accent `ORANGE`; app type (`SlideApp` → `bicraft_ui::App`, commands → BIcraft's actions and `Caps`); strings through `i18n` (EN/FR); `pretty_shortcut` (Ctrl on Windows/Linux and the browser) |
| **Never-crash rewrite** | DeckCraft indexes slices directly (`idx[i]`, `pts[i]` in `triangulate`, the area sum); BIcraft's rule (AGENTS.md §3) wants `get()`: rewrite with iterators/`get`, cap point counts, and add a test per helper with hostile input (empty, 1–2 points, NaN, huge sizes) |
| **Not taken** | colour grid, galleries, shape/transition/animation tiles, slide-show, media, sorter, text editing, macOS integrated title bar, every PowerPoint-specific icon, DeckCraft's About wording |

DeckCraft's own asset rule (no vendor icons, nothing traced) matches BIcraft's AGENTS.md §2; the
new icons are drawn the same way, original, on the 24 grid.

## 11. Test plan

Unit (in `crates/ui`, part of `cargo xtask ci`, both platforms):

- **Contrast**: for both themes, `text`, `text_dim` on `chrome` and `card` ≥ 4.5:1; `contextual`,
  `err`, `warn`, `ok` on `chrome` and `card` ≥ 4.5:1; `accent_text` on `accent`, `accent_hover` ≥
  4.5:1; `ring` on `card` ≥ 2.9:1 (light) and ≥ 3:1 (dark); `text_faint` is exempt (hints).
- **Tokens and `apply`**: light is the default; `Tokens::get` follows `apply`; every egui widget
  state uses token colours (no egui default colour left).
- **Prefs**: `theme` round-trips; missing, unknown or damaged → light; the toggle saves through a
  test `PrefsStore`; the browser store writes only the allowed keys, and a store whose every call fails (blocked storage) gives the defaults without an error.
- **Icons**: names unique and kebab-case, `from_name` inverse; every icon paints at 8, 16, 17, 30,
  64 px and disabled without panicking; empty, NaN and huge rects are no-ops; helpers on hostile
  point lists.
- **Fonts**: every family resolves to embedded faces (exists); the bold family is Selawik
  Semibold.

egui_kittest (no GPU, `cargo xtask ci`): the title row, tabs, ribbon groups, panes, pills and status
bar are found by their labels in both themes; clicking the toggle switches `dark_mode` and is kept
after a restart (same store); the active tab is collapsible; the Connect dialog's Enter/Escape;
the primary pill is absent without `Caps::open_in_desktop`; the browser caps show no File group.

Screenshots (not in `ci`; `gpu-tests` feature, `BICRAFT_DESKTOP_SHOTS=DIR`): the Report (with a
selected visual), Model (with a selected table) and Validation tabs, the start screen and the
Connect dialog, each in light and dark, at 1280 × 800, pixels-per-point 1 and 2. The **browser**
set: the same screens from `bicraft-cli view … --ui egui` in one headless browser (WebGPU, then
WebGL2), same size and DPI, captured over WebDriver BiDi as in [webui.md](webui.md). Compared
pairwise (desktop vs browser) with a small per-pixel tolerance for rasterisation differences;
the chrome must match, the report page must be byte-identical to `bicraft-cli render`'s (existing
test). Screenshots stay local unless they show only our own synthetic fixtures (AGENTS.md §2).

## 12. Status (phase B, 2026-11-04)

Built in `crates/ui` and used by both shells, identically:

| Module | What |
|---|---|
| `theme.rs` | `Tokens` (light, dark), `Brightness`, `apply` (explicit egui theme: the app chooses, not the OS), `font`/`bold`, `contrast`, `paint_logo` |
| `icons.rs` | the 24-grid toolkit and 77 icons (28 from DeckCraft, 49 new; every icon name the command registry uses has a drawing, tested); `Icon::ALL`, kebab names |
| `widgets.rs` | `big_button`, `small_button`, `icon_toggle`, `status_icon`, `group`, `rows`, `separator`, `section`, `primary`, `pill`, `tab`, `card`, `error_bar`, `chevron` |
| `app.rs` | title row, tab row + ribbon card, page pane, canvas with the page's shadow, status bar, start screen, Connect dialog, Validation cards |
| `prefs.rs` | `theme`, `ribbon`, `tab` next to `zoom`; `ui_only` (what a browser keeps) |
| `fonts.rs` | Selawik + the Noto Sans Symbols 2 subset; `pages::font_book` bundled-only |
| `backstage.rs` | the File view (DeckCraft's Backstage @ c252a68): side menu 160 px on `chrome` (Back, Home, Open, Info, Save, Save As, Export, Close; the current page in `contextual` semibold with a 3 px `accent` bar; ↑ ↓ Enter Escape), the page on `card` (content ≤ 1100 px, 48 px left inset, title 22 semibold); Home = the start screen (logo 40 px, New tiles 176 × 134, Recent rows 44 px: format badge, name 13 over folder 11 `text_dim`, date, pin, remove); Open's places and Export's tasks as 44 px place rows; Info's square task buttons 96 × 80 |
| `page_tabs.rs` | the Report view's page tabs (32 px strip on `chrome`; tabs 26 high, the shown one on `card` with a 3 px `accent` bar and a semibold label; hidden ones `text_faint` with `eye-off`; `⚠ N` in `warn`, `✕ N` in `err`; drag mark 2 px `ring`; focus ring 1.5 px `ring`) |
| `actions.rs` | every engine command the buttons run (ids and parameters) |
| `registry.rs`, `registry_table.rs` | the engine's command registry as data (generated from `bicraft_engine::COMMANDS`, checked by a test): ids, labels (FR by id), places, icons, shortcuts, needs, forms, effects; `enabled` from the snapshot's `commands.disabled` and the role |
| `ribbon.rs` | the registry's tabs (File, Home, Insert, Report, Model, Data, Analyze, View; contextual Visual, Table, Measure), groups and buttons, the click (File view flows, the form, the command, a read command's answer window), the shortcuts |
| `menus.rs` | the right-click menus of the selected visuals (Copy, Paste, Duplicate, Delete visual, Sort), of the empty page (Paste) and of the model diagram's objects; the page tab's menu adds Open on this page |
| `form.rs` | the parameters form of any command, `form::open(app, id, prefill)`, shared by the ribbon, the menus and the palette |
| `palette.rs` | the command palette (Ctrl+Shift+P) over the registry, with the ribbon's enabled state |
| `format_pane.rs` | the Build pane's sub-tabs (30 × 26 icon tabs, `chart` and `format`, a 3 px `accent` underline on the shown one) and its **Format visual** tab: Visual / General group tabs (12 pt, 2 px `accent` underline), the search box, cards in a 1 px `card_border` frame with radius 6 (the pane section header, a 36 × 18 switch on `accent` when on, `field_bg` when off), 12 pt semibold sub-headers with their switch, property rows (11 pt name, the control right-aligned), colour swatches 40 × 20 with a popup grid of 18 px theme swatches and a hex box, Reset to default in small `accent` text; the Visual group's cards and rows are built from the `bicraft_visuals` catalog for the selected type (`ObjSpec`/`Row`), each row an object property written by `visual.format` |
| `panes.rs` | the Report view's right task panes: **Build** (the visual type gallery and one well per data role) and **Data** (search, hidden toggle, the model's tables, columns, measures with the `calculator` icon, hierarchies with their levels, display folders); drag and drop between them and onto the canvas |
| `filters_pane.rs` | the Report view's **Filters** pane (left of Build): filters on this visual, on this page and on all pages; basic, advanced and top N cards; drop a field on a section to add a card |
| `interactions.rs` | the Edit interactions mode: the Visual tab's toggle, Escape, an icon strip (Filter, Highlight, None) over every target of the selected source, the marked mode (rule or default), one `page.interactions` rule per click |
| `formula.rs`, `dax_text.rs` | the DAX formula bar under the ribbon: `Name = <DAX>` with the `syn_*` colours, ✗ and ✓, the home table and format, the chevron; autocomplete, signature help, the live check's underline and message |
| `canvas.rs` | canvas editing: the selection of one or several visuals (`ring` outlines), the eight 8 pt resize handles of their rectangle (`handle_fill`, `handle_stroke`), a drag's ghost outlines (`ring` at 14 % over the page, 1.5 px `ring`), dashed 1 px smart guides in `guide`, the rubber band (the same fill, 1 px `ring`), the 8 px grid's dots in `border` with View › Gridlines (drawn by the renderer under the visuals), the position and size in the status bar in `text`; the Arrange menus (Align, Layer order), View › Page view and the View tab's Page options (Gridlines, Snap to grid, Smart guides, Lock objects); the clipboard |
| `data_view.rs` | the Data view (Desktop's table view): the grid (22 px rows on `card`, the report surface, without stripes, a 1 px `separator` line under each row and between columns, the 28 px header on `chrome` with the type tag in `text_dim` 10.5, the name in semibold 12, the menu arrow, the sort triangle or the `filter` icon in `accent_text`, a selected column on `selected`, selected cells on `accent_soft`, numbers right-aligned in `text`, `…` in `text_faint` for rows not read yet); the 240 px Data pane on `chrome` (tables, the shown table's columns, queries; 22 px rows with their icon); the header's menu (a popup frame: sort, clear, the values' checklist, OK / Cancel); the Column tools on the Table tab (labelled pickers 150 px, New column and New measure big buttons) |
| `app.rs` (documents) | document tabs under the title row (30 px; the active one on `card` with a 2 px `accent` top line; unsaved `accent` dot; × on hover; `+`); the confirmation dialog (DeckCraft's dialog, primary then Cancel) |

Differences from the specification above, on purpose:

- **No quick-access Open in the title row**: Open, Recent and Connect are in the Home ribbon and
  on the start screen, so the title row, the ribbons of the views and the status bar are the same
  in both shells and the frames test (`the_desktop_and_the_browser_draw_the_same_frames`) still
  compares every pixel the two shells draw outside the Home tab.
- The start screen (§5) is now the File view's Home in a shell that opens files or connects; a
  shell that does neither (the browser) shows the plain empty state until its source answers. A
  File tab (`contextual` text) comes first in the tab row. Icons added: `save`, `back`, `pin`,
  `copy` (46 in all). The frames test also compares Info, Export, the page tabs and the document
  tabs (`the_new_screens_draw_the_same_frames_in_both_shells`).
- The ribbon is the command registry's (M11): the Validation tab became Analyze (its Show
  validation button and the validation counts), Data and View were added, and the contextual
  Visual, Table and Measure tabs appear with a selection. The frames test covers every tab but
  Home (`the_ribbon_tabs_draw_the_same_frames_in_both_shells`).
- The command palette (`palette.rs`, M11.2) follows §7: 520 × 380 at 90 px from the top, a search
  field, rows of 36 px (50 px with a reason) on `accent_soft` when selected, the label 13 in
  `text` (`text_faint` and an `Unavailable: …` line when it cannot run), its place and id 11 in
  `text_dim`, the shortcut and `recent` in `text_faint` at the right; ↑ ↓ Enter Escape. The
  parameters form (`form.rs`) is a §7 dialog: a 130 px label column (`*` for required), 300 px
  fields, problems as `✕ …` in `err_text` under the field, the primary Run then Cancel/Close, the
  answer as `✓ Done` in `ok` with the JSON in Hack, an error on `err_soft`; a refused command
  shows the `warning` icon and its reason on `err_soft`. The frames test covers both
  (`the_palette_and_a_form_draw_the_same_frames_in_both_shells`). The title row's search button
  (the `search` icon toggle, between the primary pill and the theme toggle, `selected` while the
  palette is open) opens the palette.
- Command answers (`answer.rs`): a grid of 22 px rows in Selawik 12, the header on `chrome` in
  semibold 12 with `separator` lines, `hover` under the pointer and a small triangle in
  `accent_text` for the sorted column; odd rows on `canvas`, numbers right-aligned in
  `accent_text`, text in `text`, nulls empty; the toolbar's JSON toggle, Copy JSON and Copy as TSV
  are plain buttons with the row and column counts in `text_dim`. A tree has keys in `text_dim`
  (170 px) and values in `text` (numbers and booleans in `accent_text`, `null` in `text_faint`);
  nested objects, lists and grids are collapsing headers. The JSON view is the Hack code editor,
  read-only. Errors keep `err_soft` with `✕` in `err_text`.
- The right task panes (`panes.rs`, 2026-11-05) follow §5's task pane: two `Panel::right`s on
  `card` with a 1 px `card_border`, inner margin 10, the title 16 semibold with a `chevron-right`
  icon toggle that collapses the pane to a 30 px `chrome` strip (its `table` or `chart` icon
  opens it again); both are resizable within 180–520 px, and open or collapsed and the widths are
  kept in the prefs (`panes`, a UI key the browser keeps too). **Data** is the rightmost, as in
  Desktop: a search field and an `eye`/`eye-off` toggle for hidden objects, then 22 px rows
  indented 14 px per level: tables (semibold, `table` icon, a chevron), display folders (`folder`),
  hierarchies (`hierarchy`) and fields with a 13 px box (amber with a check when the field is in
  the selected visual), the `column` icon or the new `calculator` icon for measures; hidden
  objects in `text_faint`; `hover` under the pointer. **Build** (2026-11-17, M11.26) follows
  Desktop's layout: the view's name at the left of the header row ("Build a visual" / "Format a
  visual", 13 bold) with the two 30 × 26 icon tabs at its right, then **the gallery** of the
  snapshot's `report.visual_types` (those with known roles) in a scrolling grid of five cells per
  row, the common types first: each cell paints our own little picture of the visual (`icons::
  paint_visual_type`: blue chart shapes on a grey baseline, no Microsoft asset) and carries the
  type's name as its tooltip and accessible name, `hover` under the pointer, `selected` on the
  current type; a click sets the selected visual's type (`visual.set_type`) or adds a visual of
  that type (`visual.add`) when none is selected. Under the gallery, one well per data role: its
  caption 12
  semibold in `text_dim` (BIcraft's words per type: X-axis, Y-axis, Legend, Small multiples,
  Tooltips, Columns, Rows, Values, Fields…), its fields as 24 px chips on `field_bg` with a 1 px
  `border` (kind icon, label, the aggregation menu for a column, × on `pressed` when hovered), and
  an empty slot ("Add data fields here", `text_faint`, 1 px `border`). While something is
  dragged, the wells that take it get an `accent_soft` outline, the one under the pointer a 2 px
  `ring` and a 2 px `ring` insertion line; a visual under a dragged field gets a 2 px `ring`
  outline on the canvas; the dragged item follows the pointer as a `card` chip with a `ring`
  border. The frames test covers them (`the_panes_draw_the_same_frames_in_both_shells`).
- The **Filters** pane (`filters_pane.rs`, 2026-11-05, M11.13) is a third `Panel::right` left of Build
  with the same frame, title row, 30 px strip (`filter` icon) and prefs (`filters`,
  `filters_width`). A search field under the title keeps the cards whose field matches it, as
  Desktop's pane does. Three sections, each a 12 semibold caption in `text_dim` (Filters on this
  visual, only with a visual selected; on this page; on all pages), its cards and an "Add data
  fields here" slot (1 px `border`; `accent_soft` outline while a field it takes is dragged, 2 px
  `ring` under the pointer). A card is a `field_bg` frame with a 1 px `border`, radius 4: a
  chevron, the field 12 semibold, its condition 11 in `text_dim` in plain words ("is (All)",
  "is not Car", "Top 3 by Total"), at the right the clear button (`undo` icon: keeps the card) and
  × (cards added by hand only). Open, it shows the Filter type combo (Basic filtering, Advanced
  filtering, Top N on visuals only; measures advanced only), then the search field and the
  values as checkboxes with Select all (basic, applied on each click), two condition combos with
  their values and And / Or radios (advanced), or Top / Bottom, the count and a By value field
  that takes a dragged measure (top N), each with Apply filter. A well's field is renamed for its
  visual by a double click (an inline field over its label; Enter or leaving it keeps the name,
  Escape drops it) or its right-click menu (Rename for this visual, Restore the field's name).
  The frames test covers the pane (`the_filters_pane_draws_the_same_frames_in_both_shells`).
  For a reader (G41, 2026-11-26) the same controls change an unlocked card for that reader only
  (`view.interact` `filter` / `filter_clear`); a locked card shows its lock and its controls are
  greyed with the reason as tooltip; a reader has no ×, no drop slot, no sort list and keeps
  the card's filter type unless the report lets readers change it.
- The **formula bar** (`formula.rs`, 2026-11-05, Desktop's formula bar) is a `Panel::top` on
  `chrome` right under the ribbon, across the window, shown only while a measure, a calculated
  column or a calculated table is selected (the Data pane, a Build pane well, the model diagram's
  selection or double click, a right-click Edit). Left to right: ✗ (cancel, the `close` icon) and
  ✓ (commit, `check`, greyed until the text changes) as 24 × 22 icon toggles, then the editor:
  a `field_bg` box with a 1 px `border` (`ring` while focused), radius 4, the text `Name = <DAX>`
  in Hack 13 (one line, or 3 to 14 lines expanded, scrolling beyond). The DAX colours are
  tokens, each at 4.5:1 or more on `field_bg` and `card` (`tokens_meet_their_contrast`):
  `syn_keyword` (VAR, RETURN…; light `#0b57d0`, dark `#6cb6ff`), `syn_function` (a word before
  `(`; `#7a3e9d`, `#d2a8ff`), `syn_string` (`#a4262c`, `#ff9b8f`), `syn_number` (`#0e7a3e`,
  `#7ee0a1`), `syn_ref` (`'Table'`, `Table[`, `[Column]`, `[Measure]`; `#8a5300`, `#f5c26b`),
  `syn_comment` (`#4f6b4f`, `#8fb08f`); operators and names in `text`. The first error of the
  live check is underlined 1.5 px in `err`, and its message (with its line and column) sits
  under the bar in `err_text` 12 after a `warning` icon, as does a refused commit. At the right:
  the `table` icon, the home table and the format (or "Calculated column", "Calculated table")
  in `text_dim` 12, a `settings` toggle for the measure's properties form, and a
  `chevron-down` / `chevron-up` toggle that expands or collapses the editor; a reader gets
  "Read-only: …" under them. The autocomplete list is a popup frame (at least 260 px wide, 220 px
  high, scrolling) under the cursor: Hack 13 rows, the selected one `selected`, the kind
  (function, keyword, table, column, measure) in `text_dim` 11. The signature help is a tooltip
  frame under the cursor: the function's signature in Hack 13, the argument being typed in
  `contextual`, underlined. The frames test covers the bar collapsed and expanded, light and dark
  (`the_bar_draws_the_same_frames_in_both_shells`).
- Not built yet: page thumbnails, the zoom slider. The Model tab keeps its own bar and details panel, restyled with the
  tokens (card fill, ring, amber accent).
- The primary pill is drawn only with `Caps::open_in_desktop` (no shell has it yet).

Screenshots (`cargo test -p bicraft --features gpu-tests` with `BICRAFT_DESKTOP_SHOTS`:
`design_screenshots`, `side_by_side_with_the_browser`, `ribbon_screenshots`) and the browser's (headless browser on
`bicraft-cli view --ui egui`, light and dark through the stored preference) are kept locally, not
committed.
