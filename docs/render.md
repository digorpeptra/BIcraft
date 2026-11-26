# Page rendering (M9.6)

`report.render` draws report pages to PNG in pure Rust, so an agent can look at a page without
Power BI Desktop, on Windows and Linux alike.

```text
bicraft render Sales.pbix --page Overview --out overview.png      # one page
bicraft render Sales.pbip --out pages/                            # every page: pages/01-<name>.png …
bicraft render Sales.pbix --page 0 --scale 2 --out hi.png         # 2 device pixels per report pixel
```

MCP: the `render_page` tool (or `run_command` `report.render`) returns each page as an MCP
`image` content block; with `out` it writes files instead. Parameters and result: see
[mcp.md](mcp.md) (`report.render`).

## Pipeline

```text
engine  Session::snapshot_with(data budget 60 s)   the live viewer's snapshot (docs/view-snapshot.md):
        Session::snapshot_assets()                   layout, theme, evaluated data, slicer selections,
                                                     conditional formats, text, buttons, images
render  bicraft-render (L4, JSON in, PNG out)       a port of the viewer's drawing (crates/view/src/page.html)
        tiny-skia (rasterizer) + skrifa (font tables, glyph outlines) + harfrust (shaping)
        + resvg (SVG images) + bicraft-locale (culture data)
        zune-jpeg / gif / image-webp (raster images)
```

The renderer reads only the snapshot JSON, so it shares every data and formatting rule with the
viewer: what the viewer shows, the PNG shows. It depends on no other BIcraft crate (layer L4;
the engine command at L5 calls it).

**Page.** White, then the wallpaper (`outspace`) colour and image, the page background colour
with its transparency, the page background image (`normal` = natural size centred, `fit` =
stretched, `fill` = full width centred), then the visuals back to front (`z`, then file order; at
most 500). Hidden visuals and groups are not drawn.

**Visual container.** Background colour, transparency and image, border and radius, title (the
visual's, else Power BI's automatic "Total by Region"), subtitle, divider, then the body in the
viewer's padding (8 px for charts, none for shapes, images and buttons, 5 px otherwise), clipped to
the visual.

**Bodies.** Cards (classic, new, multi-row), KPI, gauge, clustered / stacked / 100 % bar and
column, ribbon (ranked columns joined by ribbons), line, area (stacked, 100 %), line-and-column combos (secondary
axis), pie, donut, treemap (squarified), funnel, waterfall, scatter (bubble size, legend
colours, trend line), table and matrix (column widths from content, totals, subtotals of a
two-level matrix, data bars, background and font colours, icons, web-URL underlines, column
styles), slicers (`slicers.rs`: the classic slicer as a list with check boxes, "Select all",
search row and hierarchy chevrons, as tiles, as a dropdown ("All", the value, "Multiple
selections"), between / less-or-equal / greater-or-equal with its two inputs and slider, relative
date; the new list slicer with radio buttons, the button slicer, the text slicer), text boxes (runs with size, weight, italic, colour, underline, links underlined,
bulleted and numbered paragraphs (`list`: a `•` or `N.` marker in the first run's font), wrapped to
the box), buttons (fill, text, icon), shapes, images (PNG, JPEG, GIF first frame, WebP, SVG;
`normal` = fit inside, `fit` = stretch, `fill` = cover), page and bookmark navigators (below), reference and trend
lines, axis titles, log scales, explicit axis bounds, legends, data labels, small multiples,
decomposition trees (`decomp.rs`, below). Anything else (maps, Q&A…) is a labelled box with the
type and title; the result lists it as `placeholder`. A custom visual whose package the report
embeds (the snapshot's `custom_visual`, docs/formats/pbiviz.md) is the same box with the
package's icon above its display name and the title: BIcraft never runs a custom visual's
JavaScript. The BIcraft Spec visual (the snapshot's `spec`) is drawn from its spec by `spec.rs`, the
layout its own TypeScript computes in Power BI (docs/formats/spec-visual.md); the visuals of
BIcraft's chart pack (the snapshot's `chart_pack`: organization chart, Venn, Marimekko, calendar
heatmap, Sankey, Scatter+, Line / Area+) by `chart_pack/*.rs`, the twins of their TypeScript layouts (docs/formats/chart-pack.md). The key influencers
visual (`keyDriversVisual`) has its own placeholder laid out like Desktop's (tabs, question line,
the analysed fields, "Computed by Power BI — not reproduced by BIcraft"), or, with
`key_influencers: "estimate"`, BIcraft's own badged estimate (status `estimate`):
[key-influencers.md](key-influencers.md).

**No data.** A visual whose snapshot data is not `ok` (a PBIP without cache, an unsupported
measure…) is drawn at half opacity with deterministic made-up values (the viewer's generator) and
a "no data · preview" badge; the result says `preview` with the reason. `badges: false` removes
the badges.

**Numbers and dates.** Power BI's display units (K, M, bn), the measure's format string
(decimals, %, currency, grouping, a leading `+`), with the culture's separators and dates. The
culture is one parameter (`RenderOptions::culture`, en-US when empty or unknown): its decimal
and group separators, minimum grouping (es: `1234`, `12.345`), percent layout, display-unit
suffixes (`1,2 Mio.`), date order and separator and 12-hour clock come from `bicraft-locale`'s
display data (59 cultures, roadmap M10.1). French groups with U+00A0, as Desktop 2.158's visuals
do (its DOM text), not CLDR's U+202F. Only the separators follow the culture: with a French
model and a French UI Desktop 2.158 still writes display units as K, M, bn ("912K") and puts
`%` where the format string does ("0.0%" → "38,0%", "0.0 %" → "38,0 %"), so the renderer
does the same. A date or date-time value with a date format string (`dd/MM/yyyy HH:mm` on a
measure) is written with that format in the culture (`bicraft-locale`'s VB formatter, as DAX
`FORMAT`). Without one the renderer formats in the snapshot's model
culture (`model.culture`), like the live viewer, en-US when the model has none. Desktop itself
does not follow the model culture: Desktop 2.158 in English on a French Windows drew the same
report with an en-US model and with a fr-FR model identically, in fr-FR ("0,3M", "78,27K",
"janv. 2025", "mercredi 1 janvier 2025"): its visuals format in its report locale (File > Options
> Regional settings, default: the Windows regional format), neither the model culture nor the UI
language (M10.2 oracle). BIcraft keeps the model culture as its default on purpose (decided 2026-10-14): the same file renders the same on every machine and on Linux, with no dependence on the host's regional settings. Pass Desktop's locale as `format_culture` to match a given Desktop. Named
formats follow the visuals, not DAX `FORMAT`: a `Long Date` column is the long date, a `General
Date` measure the short date and the long time even at midnight ("01/12/2025 00:00:00"); a
decimal column without a format string (and its Sum, Average…) shows two decimals ("3 081,30").
Implicit measures (other than the counts) keep their column's format string.
A `General Date` dateTime *column* keeps the time at midnight too: Desktop 2.159.1231.0 (French,
2026-11-24) wrote `05/01/2026 00:00:00` in a table, a slicer and a BIcraft Sankey's nodes,
as BIcraft writes it. In that run a slicer on that column with no mode written was drawn as the
Between range with two date inputs; the snapshot now gives such a classic slicer on a dateTime
column the `between` mode and its domain (`a_date_slicer_without_a_mode_is_a_range`). A column
chart on the same column drew a continuous time axis ("5 janv. 00:00" …); the renderer now
draws one too ("Continuous date axes" below), where it drew one category per value.
Three options, three jobs: `report.render`'s `format_culture` (`bicraft render
--format-culture`) overrides the formatting culture; `culture` (`--culture`, `view.set_culture`)
only picks the model translation whose captions name tables, columns and measures (M10.4); and
`ui_language` (`--lang`) is the language of generated phrases (auto titles, (Blank), Total).
A `culture` the model has no translations for (or the model's own culture, e.g. `en-US` on an
English model) is not an error: the page shows the stored names, as Desktop shows untranslated
captions, and the result carries `warnings` naming `format_culture` and `ui_language`; only a
malformed name (not shaped like `fr-FR`) is refused, with the same pointer. Renders are
deterministic: the same input and `as_of` give byte-identical PNGs (no result depends on hash
iteration order; `renders_are_deterministic` checks it across fresh sessions). Without `as_of`,
`TODAY()`/`NOW()` and relative date filters follow the clock.

**Fonts.** In order: the families the report asks for, Segoe UI, then the bundled
[Selawik](https://github.com/microsoft/Selawik) (SIL OFL 1.1, metric-compatible with Segoe UI;
see `THIRD_PARTY_NOTICES.md`), Arial, Liberation Sans, DejaVu Sans, Noto Sans. Font files come
from the system font folders (Windows `%WINDIR%\Fonts` and the per-user folder; Linux
`/usr/share/fonts`, `/usr/local/share/fonts`, `~/.local/share/fonts`, `~/.fonts`; macOS
`/Library/Fonts`…), from `font_dirs` / `--font-dir`, and from `BICRAFT_FONT_DIRS` (a path list).
Only each file's table directory, `name` and `OS/2` tables are read to index it; faces load on
first use. Power BI's web font aliases as Desktop's page names them (`wf_segoe-ui_semibold`…,
and `wf_standard-font` / `wf_standard-font_light`, the classic look's KPI and gauge callouts)
are drawn with Segoe UI weights (`wf_standard-font_light` as Segoe UI Light); weights match in CSS order (400 tries 500, then lighter, then heavier). The font `DIN`
(not a system font) falls back to Segoe UI drawn and measured at 0.92 of the requested size
(the width measured on Desktop's rendered page: "100%" at 12 px is 26.95 px in DIN, 29.2 px in
Segoe UI), like a CSS
`size-adjust` on a fallback face; the text keeps its requested size (text dump: 12 px `DIN`).

**Calibri** (2026-11-20, M11.38). A family the system lacks is replaced by a face with the same
advance widths where one exists: Calibri by Carlito (an installed `Carlito` first, as Linux font
configurations and LibreOffice substitute it, then the bundled subsets). The bundled faces are
OFL glyph subsets of Carlito Regular, Bold, Italic and Bold Italic, Version 1.104
(`crates/render/fonts/carlito/SOURCE.txt`, made by `cargo xtask font-subset carlito`), renamed
"BIcraft Metric Sans" because Carlito's licence reserves its name for unmodified files; they keep
407 characters (Basic Latin, Latin-1, Latin Extended-A, Romanian, punctuation, superscripts,
currency, common letterlike symbols, fractions). So text a theme or a visual sets in Calibri
measures, wraps and truncates as with Calibri in the app's pages, `bicraft render` and PDF export
(`calibri_text_takes_carlito_widths`: widths equal Carlito's advances, about 8% narrower than
Selawik's; `calibri_text_embeds_the_carlito_subset`); real italics are used, not a slant.
"Calibri Light" takes the regular face (Carlito has no light weight). Limits: Greek, Cyrillic
and Vietnamese text in Calibri falls through to the Noto faces (their widths, not Calibri's);
the subsets have no kerning table (sums of advances, as for Selawik).

**Text.** Each line is split into runs of one face: a character the first faces lack comes from
the next face of the chain that has it (after the families above: Segoe UI Symbol / Emoji /
Historic, Nirmala UI, Microsoft YaHei / JhengHei, Yu Gothic UI, Malgun Gothic, the Noto CJK,
script and symbol families, Droid Sans Fallback…, and last the **bundled fallback faces**);
combining marks, joiners and variation selectors stay with their base (a mark the base's face
lacks moves the base to a face that has both). Runs are shaped with [harfrust](https://github.com/harfbuzz/harfrust)
(HarfBuzz's default features as Chromium uses them: GPOS and `kern` kerning, ligatures, mark
positioning) and cached per font and text. Horizontal positions and advances are unhinted and
fractional (Chromium positions glyphs by subpixels), so text widths, wrapping, truncation and the
hit map do not depend on the device scale. Glyph coverage is
lifted like Chromium's text contrast (`1 − (1 − c)^k`, k = 1.6 for black text down to 1.2 for
white), which matches the ink of Desktop's table text. Italics without an italic face are slanted.

**Hinting** (2026-11-09). A report page shown fitted in the app (a 1920×1080 page at about 68 %
on a 1920-pixel screen) draws table and axis text 5 to 9 device pixels tall; unhinted, every
horizontal stroke and the x-height fall between two pixel rows and the text reads blurry next to
the app's own crisp text. Upright raster text (no rotation) is therefore drawn with outlines
hinted at the device size, pixels per em = font size × scale, between 2 and 48: vertical-only
("light") hinting, as Chromium draws text with FreeType's light target on Linux and close to
DirectWrite's vertical-only hinting on Windows. Baselines (whole device rows, also for unhinted
text), x-heights, cap heights and horizontal stems snap to the pixel grid; horizontal outlines
and advances are not touched (skrifa's `HintingInstance`, `SmoothMode::Light` with
`preserve_linear_metrics`). Faces with TrueType or PostScript hints use them (the TrueType
interpreter in its backward-compatible mode, which ignores horizontal moves); the others,
the bundled Selawik and Noto subsets among them, use the automatic hinter, which also scales the
whole glyph height so the x-height lands on a whole row (up to about ±0.5 pixel: a 10 px label
at 68 % has a 3-pixel x-height, a 10.67 px one a 4-pixel one). On a synthetic sample (8 to 14 px
text, regular and semibold, `hinting_sharpens_small_upright_text`) the vertical edge energy per
unit of ink rises ×1.13 at scale 0.68 and ×1.11 at scale 1, the horizontal one by 3 % at most.
Rotated text, text above 48 pixels per em and the PDF's vector text keep the unhinted outlines
(PDF glyphs are the font's unhinted outlines, resolution-independent; the raster drawn while
recording is the same as without recording). Hinting instances (16 sizes per face) and hinted
outlines (8,192 per face) are cached per font book and emptied when full; a face or glyph that
fails to hint, or whose hinted points land far outside its em square, falls back to the unhinted
outline. Glyph outline reading runs under `catch_unwind` (read-fonts 0.41 indexes an empty
array on a simple glyph with flags but no points), so a hostile font draws nothing for that
glyph instead of failing the render. The contrast curve above is unchanged: it is calibrated
on Desktop's ink, and hinting adds contrast on its own.

**Subpixel text** (2026-11-09). On a 100 % screen a fitted 1920×1080 page (scale 0.55 to 0.7)
still draws small text 5 to 8 device pixels tall, and hinting only sharpens its rows. Power BI
Desktop's report canvas (Chromium on Windows) draws such text with ClearType-style subpixel
antialiasing on opaque backgrounds, which triples the horizontal resolution. The renderer does
the same when `RenderOptions::lcd_text` is on (`Canvas::set_lcd_text`): the window's pages (and
its report-page tooltips, also shown at their own size) turn it on below 1.75 device pixels per report pixel (`crates/ui/src/pages.rs`, `LCD_TEXT_BELOW`; at
high DPI grayscale is sharp enough), and `report.render {lcd_text: true}` / `bicraft render
--lcd-text` draw it for a look. Off is the default everywhere else, so PNGs, goldens, PDF and
exports keep grayscale text, pixel for pixel. For an upright run (no rotation;
a synthetic oblique is fine) of opaque text, the run's coverage is rasterized at 3× the
horizontal resolution, spread by the 5-tap LCD filter (FreeType's default weights
8, 77, 86, 77, 8 / 256, which keeps the fringes faint), lifted per subpixel by the same contrast
curve as grayscale text, and each device pixel takes its red, green and blue from its three
subpixels (RGB stripes, left to right) and is composited per channel. It needs opaque pixels
under the whole run, as in Chromium: translucent text, rotated text and text over transparent
pixels stay grayscale. Visuals, their bodies and clipped parts are drawn on transparent layers
and composited, so a layer made with `Canvas::layer(x, y, w, h)` keeps a copy of the opaque
pixels it will be drawn over (at most 16 M pixels): text on it is antialiased against them and
stored as the premultiplied colour that gives the subpixel result once composited there
(`S = T·c + D·(1 − c) + B·(1 − Da)·(cmax − c)`, alpha `Da + cmax·(1 − Da)`; exact within 2/255,
`lcd_text_on_a_layer_is_antialiased_against_what_it_is_drawn_over`). A run whose 3× mask would
exceed 8 MB is drawn in grayscale. Advances, positions, the text dump and the hit map do not
change. On the synthetic sample (`lcd_text_colours_upright_text_on_opaque_pixels`) the
subpixel values sit ×1.68 closer at scale 0.6 (×1.91 at scale 1) to the glyph coverage each
subpixel actually has (unfiltered 3× coverage: RMS 0.254 → 0.151) than grayscale pixels; the
vertical edge energy is within 11 % and the mean channel spread of inked pixels is 0.30 (the
usual orange and blue ClearType edges; unfiltered stripes would be stronger). The window shows
the page texture 1:1 (`texel_rect`); while a scaled stand-in is shown during a re-render, its
fringes are resampled for a moment. The stripes assume an RGB panel (the common one); BGR and
vertical panels are not detected.

**Bundled fallback faces and missing glyphs** (2026-11-05). Selawik has Latin only, so text such
as a measure named `CA Δ N-1` once drew `CA   N-1` wherever no system font had the character
(the app's pages, which use the bundled faces only, and `bicraft render` on Linux). After Selawik
the chain now has OFL glyph subsets of Noto fonts (`crates/render/fonts/SOURCE.txt`, made by
`cargo xtask font-subset bundle`): Noto Sans Regular and Bold (Greek, Cyrillic, Latin extensions
with Vietnamese, punctuation, superscripts and subscripts, currency such as `₹ ₽ ₺ ₩ ₿`,
letterlike symbols, fractions), Noto Sans Symbols 2 (geometric shapes `▲ ▼ ● ◆ ■`, dingbats
`✓ ✗`, box drawing and block elements, stars, arrows of the symbols blocks), Noto Sans Math
(arrows `↑ ↓ → ⇒`, operators `≥ ≤ ± ≠ ∑ √ ∞`), Noto Sans Symbols (what the others lack) and Noto
Emoji, monochrome (`🔴 🟢 📈 🚀 👍`). Power BI Desktop draws such characters through Segoe UI and the
Windows fallback fonts (Segoe UI Symbol, Segoe UI Emoji; observed as a black box); with
those installed, `bicraft render` uses them first, as the bundled faces come last. Noto Sans's
capital height is within 3% of Selawik's (Segoe UI's metrics; checked by
`fallback_faces_match_selawik_metrics`), so fallback glyphs sit on the same line at the same
size; no size adjustment is applied. Emoji are monochrome, in the text's
colour (Desktop shows colour emoji).

A character no face has is never dropped: it is drawn as an outlined box the height of a capital
(0.6 em wide), the same for a face's `.notdef`, and recorded. `report.render` lists those
characters once per render: `missing_glyphs` (`[{"char": "北", "codepoint": "U+5317"}…]`) and a
warning ("N character(s) have no glyph in the fonts and are drawn as boxes: …"). Controls and
default-ignorable characters (zero-width spaces and joiners, variation selectors, the BOM) draw
nothing. **CJK is not bundled** (a full CJK face is 15–20 MB; docs/gaps.md G43): Chinese,
Japanese and Korean text uses the system's fonts (Microsoft YaHei, Yu Gothic, Malgun Gothic,
Noto Sans CJK…) or a face given with `--font-dir` / `BICRAFT_FONT_DIRS`, and otherwise draws as
boxes, listed. `FontBook::missing(font, text)` is the coverage query (the characters of a text
the chain lacks); `crates/render/tests/glyph_coverage.rs` checks every character of the
repository's fixtures (`crates/*/tests/fixtures`) and a list of characters common in reports
against the bundled faces in every weight.

Size (measured 2026-11-05, Windows): the subsets are 423 KB, embedded in every binary that links
the renderer; the browser bundle grows from 4.39 to 4.62 MB gzip (`cargo xtask webui`).

**Hit map and reader state** (2026-10-29). With `RenderOptions::hits` the renderer also returns
the page's hit map (`PageImage::hits`, `bicraft_render::hits`): the regions the live viewer makes
clickable, in report pixels and back to front, each with the `view.interact` action it sends and
its tooltip rows (slicer items and popups, data points of bars, columns, combos, lines, pies,
donuts, funnels, treemaps and table rows, page and bookmark navigators, action buttons, images and shapes with an action (`visual.set_action`), the background of the
visual that holds a selection). `RenderOptions::raw` returns the premultiplied RGBA pixels
instead of a PNG. The desktop app uses both ([desktop.md](desktop.md)). The page also shows the
session's reader state as the viewer does: points left out of a selection at 40 % opacity in
their own visual (table rows under a white veil), highlighted visuals with the full mark at 40 %
and the highlighted part opaque. A snapshot without a reader state draws as before.

**Page and bookmark navigators** (2026-11-18, G55, `bodies.rs`). Both draw one text button per
item, laid out by the visual's `navigator.orientation` (`objects.layout.orientation` in the
report theme schema: 2 horizontal, 1 vertical, 0 grid with `columnCount` / `rowCount`; not set:
a column when the box is more than 1.4 times taller than wide, else a row), 6 px apart, the
selected item in semibold accent text with a 3 px accent bar along its bottom. A **page navigator** lists the
report's pages in order, hidden pages and tooltip pages left out unless its
`show_hidden_pages` / `show_tooltip_pages` say otherwise (`objects.pages`); the page drawn is the
selected one; a click sends `page {page}`. A **bookmark navigator** lists the snapshot's
`bookmarks` of the group its `bookmark_group` names (`objects.bookmarks.bookmarkGroup`: a group's
name or display name), or every bookmark when it names none; the selected one is the bookmark the
reader applied last (`interaction.bookmark`), else its `selected_bookmark`; a click sends
`bookmark {bookmark}`; with no bookmark to list it is an empty dashed frame. The hits carry the
navigator's name (`visual`), so the app asks for Ctrl+click in its editing view, as for buttons.
**Navigator styles** (2026-11-19): the visual's per-state styles (`navigator.styles`, from
`objects.fill`, `outline`, `text` and `accentBar` whose `selector.id` is `default` or `selected`,
the cards and `$id` states the report theme schema documents for `bookmarkNavigator` and
`pageNavigator`) are drawn: the fill (colour and transparency) and the outline (colour, weight)
of each item, its font colour, size, bold and italic, and the accent bar (colour, top, bottom,
left or right, width); the selected state's properties win over the default state's on the
selected item; `objects.layout.cellPadding` is the space between items. Unset, an item draws as
above (no fill, no outline, the accent bar on the selected item only, semibold accent text).
Hover and press states are drawn since 2026-11-20. **Shapes, glow and shadow** (2026-11-25):
`navigator.shape` (the `shape` object without selector) gives each item the shape visual's tile
outline (pill, rounded rectangle and the others), filled and outlined along it, the outline
inset by half its weight; a state's `show_shadow` draws the outline moved `shadow_distance` px
(3) towards `shadow_angle` degrees (45) under the item, translucent with three widening
`shadow_blur` (5) strokes, in `shadow` (black) at `shadow_transparency` (70 %); `show_glow`
strokes the outline three times up to twice `glow_blur` (5) px wide in `glow` (the accent) at
`glow_transparency` (50 %). These defaults and the blur drawn as strokes are BIcraft's, inner
shadows are drawn outer; neither is drawn under high contrast. Not drawn: disabled state,
rotation, text alignment and margins; theme `visualStyles` for navigators are not read.
Desktop's own defaults (the look of a freshly inserted navigator) are not known from a public
source and are not reproduced; not compared with Desktop pixel for pixel.

**High contrast** (2026-11-19). `RenderOptions::high_contrast` (the app's View › High-contrast
colours, never a file, a PDF or `bicraft render`) maps every colour a canvas paints
(`Rgb::high_contrast`, under a thread-local `ContrastScope`): a light neutral (relative luminance
above 0.4, or lightness above 90 %) becomes black, a dark neutral white, and a colour with a hue
the bright colour of its hue band (yellow, cyan, green, light red, light magenta, amber, light
blue, each at 7:1 or more on black). Text takes black where the pixels under it (sampled at three
points of the run, or the layer's known backdrop) are light, else its mapped colour when that is
light, else white. Images keep their pixels. Series of close hues share one bright colour (two
blues draw the same); Power BI's own high-contrast rendering was not compared.

**SVG images are sandboxed** (2026-11-06, M11.20). An SVG is parsed by usvg with its image
resolvers replaced by ones that return nothing: an `<image>` naming a file (`C:…`, `/etc/…`,
`file:///…`), a `data:` URL or a web link draws nothing, so a hostile SVG in a report cannot make
`bicraft render`, a server or the app read a local file and draw it into a page (usvg's default
resolver reads any path it is given; `svg_references_outside_the_bytes_are_not_read` fails
without the fix). Scripts, styles from outside and fonts are never loaded (resvg runs no
script; its `text` feature is off, so SVG text is not drawn). `bicraft_render::image_size` is the
same decode, used by `image.add` to check bytes before storing them.

**Never crash.** The renderer runs under `catch_unwind`; every number from the snapshot is
clamped; images are capped (16 M pixels, 16 384 px a side, sniffed by content, decoded in a
`catch_unwind`); the page is capped at 40 M device pixels (`scale` is lowered to fit, the result
says the scale used); text, rows and series are bounded like the viewer's.

## PDF (`report.export_pdf`, M11.21)

`report.export_pdf` (`bicraft export-pdf FILE [--out F.pdf] [--pages all|current|2-4|NAMES]
[--hidden]`; Backstage › Export › Export to PDF in the app) writes report pages to one PDF, as
Desktop's File › Export › Export to PDF does. Pages: every page not hidden (the default), the
current page, a 1-based range (`"2-4"`, `"1,3-5"`; hidden pages skipped unless `hidden: true`),
or pages named one by one (exported even when hidden). Each report page is one PDF page at its
size and aspect ratio: 0.75 pt per report pixel (96 px per inch), so a 1280 × 720 page is
960 × 540 pt.

**How it is drawn.** The pages go through the same snapshot and drawing code as the PNG. Under
a vector scope (`canvas.rs` `VectorScope`) every canvas also records what it draws, in report
pixels: filled and stroked paths (with their transform, alpha, dash, caps and joins), text runs
as shaped glyphs (face, glyph id, pen position), images, and the canvases composited into it
(a visual, a table's scrolled grid, a clipped image) as clipped layers. `pdf.rs` writes that
display list; the PNG drawing is unchanged (`rendering_raster_is_unchanged_by_recording`).

- **Vectors.** Shapes, chart marks, axes, gridlines, borders and backgrounds are PDF paths;
  transparency is an `ExtGState` (`ca`/`CA`); a layer drawn at less than full opacity (the
  faded "no data" preview) is a transparency group form.
- **Text is text.** Every face used is embedded as a TrueType subset (`ttf_subset.rs`: the
  glyphs used, `.notdef` and composite components kept at their glyph ids, every other glyph
  emptied; `head`, `hhea`, `maxp`, `hmtx`, `loca`, `glyf`, the hinting tables, `OS/2`, `cmap`,
  `name` with the font's copyright and licence notices, a format 3 `post`) under a `Type0` /
  `CIDFontType2` font with `Identity-H` encoding and a `ToUnicode` map. Glyph positions come from
  the shaper (kerning as `TJ` adjustments), so the text lands where the PNG draws it and can be
  selected, searched and copied. Bundled faces (Selawik and the Noto subsets, SIL OFL 1.1) are
  embedded under the OFL's terms: the subsets keep the fonts' names and notices, and the OFL
  allows embedding in documents.
- **Font licences.** A system face whose `OS/2` `fsType` forbids it (restricted licence,
  no subsetting, bitmap only) is not embedded: its glyphs are drawn as outlines, with invisible
  Helvetica text (render mode 3) over them so the page stays searchable; the result lists the
  face with `embedded: false` and a warning. CFF-based faces (OpenType `.otf`) are drawn the
  same way (no CFF subsetter yet). `system_fonts: false` uses the bundled faces only.
- **Raster fallback.** Images are Flate-compressed RGB with a soft mask for transparency, at
  the `scale` resolution (default 2, 192 px per inch). A canvas whose recording passes 200,000
  operations (a scatter of a million points, a huge table) is placed as an image of itself, one
  visual at a time (`raster_parts` in the result); SVG images are rasterized as in the PNG.
- **File.** PDF 1.7, a classic cross-reference table, Flate-compressed streams, one shared
  resource dictionary, the title (`title`, default the file's name) in the information
  dictionary; deterministic (no dates, no random ids): the same input gives the same bytes.
- **Outline.** One bookmark per exported page, in export order, titled with the page's display
  name (its name when it has none) and going to that page (`/Dest [page /Fit]`); with more than
  one page the file opens with the bookmarks shown (`/PageMode /UseOutlines`). ISO 32000-1
  §12.3.3.
- **Tagged.** The catalog says the file is tagged (`/MarkInfo << /Marked true >>`), gives its
  language (`/Lang`: the formatting culture, else the model's culture, else `en-US`) and asks
  viewers to show the title (`/DisplayDocTitle`). The structure tree (ISO 32000-1 §14.7, the
  standard types of §14.8.4) is a `Document` with a `Sect` per page (`/T` its title), a `Sect`
  per visual drawn (`/T` its title, else its automatic title, else its type), and in it a `P`
  per line of text (runs that go on where the last one ended, on its baseline, join one
  paragraph: a text box's words). A visual with alt text (General › Alt text, `altText`) is a
  `Figure` with that `/Alt`: its marks and its labels are the figure's content, read as the alt
  text. Everything else (page and visual backgrounds, borders, the marks of a visual without alt
  text) is an `/Artifact`. Each piece is marked content with an `MCID`; pages carry
  `/StructParents` and `/Tabs /S`, a faded layer (a transparency group form) has its own
  `/StructParents` and its content is referenced with `/Stm`; the parent tree maps every `MCID`
  back to its element. Elements with no content are left out. Not checked against the PDF/UA
  validators (PAC, veraPDF): no PDF/UA identification is written.

**Checking a PDF.** `bicraft_render::pdf_read::inspect` reads a PDF without other tools: the
header, the cross-reference table (every object listed is parsed at its offset), the page tree
and `Count`, each page's `MediaBox`, its fonts (embedded or not, with a `ToUnicode` map or not),
images drawn and the text extracted through the maps (forms followed), the text drawn outside
any marked content and whether marked content nests, the catalog's `Lang` and `MarkInfo`, the
outline (title, depth, page) and the structure tree (type, title, alt text, page and the text of
each element's marked content), with what does not hold together (an `MCID` the parent tree
does not map back to its element). It reads classic cross-reference tables only and is bounded
against hostile input (`pdf_tests.rs`, its fuzz loop over truncations and flipped bytes).

**Print** (`report.print`, File › Export › Print in the app): the pages picked as for Export to
PDF are written to a PDF, by default `bicraft-print/<title>.pdf` in the temporary folder (the
next free `<title> (n).pdf` when an earlier one is still open), and handed to the operating
system's default application for `.pdf` files (`explorer.exe` on Windows, `open` on macOS,
`xdg-open` elsewhere, no shell; `desktop_open.rs`), where the user prints it; BIcraft does not
drive a printer itself. On a server nothing is launched: the browser's app has the PDF written
in the workspace, fetches it, and prints it from a hidden frame with the browser's own print
(the download of the file when that fails). `dry_run` writes the file and opens nothing.
BIcraft's own design: no Desktop print behaviour was observed or copied.

**Measured** (2026-11-06, bundled fonts): the desktop tests' two-page fixture (a slicer, a card and
a column chart, then a card) is a 7.7 KB PDF; poppler's `pdftotext` (an independent reader)
extracts its titles, labels and values. The font subsets hold the glyphs used only.

**Not yet.** PDF/A and PDF/UA identification (and a validator run), tables tagged as `Table`
structures (a table visual's cells are paragraphs), a print dialog of BIcraft's own (page setup,
printer choice), the page background image's tiling options beyond the PNG's, CJK text without
a CJK system font (drawn as boxes, as in the PNG; G43).

## Decomposition tree (2026-11-04)

`decompositionTreeVisual` is drawn from the snapshot's `data.decomp` (the engine evaluates the
root, then each level the visual's expansion state shows: docs/view-snapshot.md, "Decomposition
trees"), with the geometry, colours and rules measured in Power BI Desktop 2.159 as a black box
([formats/decomposition-tree-bb.md](formats/decomposition-tree-bb.md)): the root bar labelled
with the measure, one column per level with its header (field name, accent underline, remove
cross, the selected member as subtitle, the AI lightbulb on "High value" / "Low value" levels),
bars proportional to the level's largest value (or the parent's or the root's,
`dataBars.dataBarScalingType`), member and value labels (members on the path in bold, their bar
outlined), curved connectors from the parent (the selected one 1.5 px in the accent colour, the
others light grey; `tree.connectorType: round`), scroll arrows on a level longer than the bars
that fit (the selected member kept in the middle of the window), and the "+" right of the last
level's bars. Options read: `tree` (`density`, `barsPerLevel`, `accentColor`,
`connectorDefaultColor`, `connectorType`), `dataBars` (colours, scaling, `dataBarWidthPercent`),
`dataLabels` and `categoryLabels` (font, colour, size, display units, precision), `levelHeader`
(fonts, colours, background, `showSubtitles`), `analysis` (`aiEnabled`, `aiMode`).

`bicraft_render::decomp::layout` gives the positions (level x, window, bar centres and lengths);
the oracle test compares them with Desktop's.

**Oracle.** `cargo xtask oracle-gen decomp DIR` writes 23 trees, one per page (root only, one and
several levels, a selected path, "High value" saved with the wrong field, "Low value" from the
root, a selected member below the bars shown, a sort by the measure, a ratio measure with a
blank member, a slicer and a visual filter, sparse and dense, round connectors, bars scaled to
the parent, values in thousands, relative AI mode, two AI levels, levels in another order than
the fields, heights from 300 to 700 px, a visual narrower than its levels);
`xtask/decomposition-tree-oracle/record.ps1 -Dir DIR` reads what Desktop draws (Desktop lock, its own
instance, closed when done); `oracle-gen decomp DIR collect` writes
`testsupport/decomp_oracle/corpus.json` (values parsed from Desktop's text, positions in report
pixels; no Desktop UI string). `cargo test -p xtask decomp` (Linux too) builds the same report,
takes the snapshot and compares, tree by tree: the root value, the levels and their fields, the
AI levels, each level's window (members, values, order), the selected path, the level x, bar
centres and bar lengths (1.5 px). Result: see the progress log of 2026-11-04 in docs/history/roadmap-log.md.

**Hit map.** A bar on the selected path sends `select` with its path (`{c, v}` per level: the
other visuals are highlighted or filtered as by any data point); another bar sends
`decomp_path` (the path moves there, levels below evaluated again); the "+" opens a
`decomp_expand` popup whose `items` carry their own `view.interact` actions ("High value",
"Low value", the free fields); the header's cross sends `decomp_collapse`. In the source tree,
the bars off the selected node's path are drawn at 40 %; a highlighted tree draws each bar at
40 % with its highlighted part opaque. The live viewer's `page.html` does not draw from the
renderer: it still shows the tree as a labelled box (the egui UI replaces it, docs/webui.md).

**Scrolling and Collapse (2026-11-26, G34).** A level's scroll arrows send `decomp_scroll`
with the window before or after the one shown (the size of the window: BIcraft's choice, Desktop's
step was not measured); a selected member scrolled out of the window keeps the levels below it,
hung from the end of the bar area it went past. A tree wider than the visual gets an arrow on
each side it is cut (BIcraft's own: Desktop clips it, D21, and scrolls in a way not recorded),
which scroll it by one level (`decomp_scroll {shift}`; the last level always stays); regions
scrolled out of the body are not clickable. `tree.defaultClickAction: toggle` (the public theme
schema's "Collapse": a click on an intermediate member of the selected path collapses the tree)
makes a click on a path member with a level below it send `decomp_collapse` of that level; the
last level's path member still selects. In the egui UI, the "+", the header crosses and the
arrows are also labelled buttons over the page (`crates/ui/src/decomp_ui.rs`: "Add a level
under North", "Remove the level Region", "Scroll Region down", "Scroll the tree right"),
reached with Tab and pressed with Enter or Space; the "+" opens the "Expand by" popup ("High
value", "Low value", the fields left; the first item focused, Escape closes it).

**Left.** The window of a long level without a selected member follows an empirical rule;
negative bars and `axisStart` / `axisEnd` are not reproduced; the lightbulb and cross are drawn
shapes, not Desktop's icon font.

## Maps (2026-11-20)

`crates/render/src/maps.rs` draws map visuals offline ([formats/maps.md](formats/maps.md)): maps,
filled maps and Azure Maps on a Web Mercator base map of Natural Earth's 1:110m country outlines
(public domain, bundled in `crates/render/geo/`, read by `crates/render/src/geo.rs`; 41 KB):
the sea a tint of the first data colour, land near the page background, borders between, all
from the theme's text and background, so light and dark both work; no tiles, a "Made with
Natural Earth" mark. Since 2026-11-24 (the open basemap, by the maintainers' decision), a view
under 90° of longitude takes Natural Earth's 1:50m countries and lakes
(`crates/render/geo/ne_50m_basemap.bin`, 330 KB), and under 70° also the admin-1 boundaries and
city names (golden pages `maps.png` and `maps-dark.png`). The view fits the data (15 % around
bubbles at Latitude and Longitude or at cities, 5 % around the main land of the countries
named). Locations that are country names (English, Natural Earth's French name, long and formal
names) or ISO alpha-2 / alpha-3 codes are placed offline: bubbles at the country's label point,
or, on a filled map, the country filled with its Legend colour (else the first data colour); the
names of Natural Earth's 1,251 populated places put bubbles at the city. Other place names keep
a placeholder saying geocoding needs the online map service (status `placeholder`). A shape map's
custom TopoJSON or GeoJSON map (the snapshot's `shape_map`; Zoom › Auto zoom fits the areas with
data) is drawn as projected areas coloured by Legend with a legend, else in one colour (Color
saturation alone does not shade them, as in Desktop 2.159.1231.0). Bubbles
are circle hits and areas polygon hits (`HitShape::Polygon`, even-odd); both select their
location. Drawing only: the outlines and the placing of countries write nothing to the file.

## Ribbon chart (2026-11-20)

`crates/render/src/ribbon.rs`: per category a column whose segments are stacked by rank (the
largest value on top, Learn: "the ribbon for the highest rank is always displayed on top"), and
between adjacent columns one band per series joining its two segments. Learn's Ribbons options
are read from the snapshot's `format.ribbon` (`ribbonBands`, `layout.ribbonGapSize`): Spacing
(`gap`, a percentage of the tallest column between the segments of a column), Match series
colour (off: grey bands, or the band colour set), Transparency (30 by default), Border (dark
edges along the top and bottom of each band, off by default; its colour, width, transparency,
or the band's colour darkened). Total labels (`format.totals`, `totals.show`) write each
column's total above it with their display units, decimals, size and colour. Each segment is a
rectangle hit with the point's tooltip and selects its category and series (cross-filter or
highlight of the page); a selection fades the other segments and the bands whose two ends are
both left out. Not compared with Desktop: the band border colour, the label placement of totals
and the bands' hover (Desktop's tooltip over a band is not documented).

## Continuous date axes (2026-11-24)

`crates/render/src/timeaxis.rs`. Power BI's category axis type (`categoryAxis.axisType`, the
snapshot's `format.x_axis.axis_type`: `Scalar` continuous, `Categorical`) chooses between one
band per category and a time scale. Column, bar (stacked and 100 % too), line, area and combo
charts on a single date or date-time category column use the time scale unless the axis is set
`Categorical`; an axis expanded down a hierarchy (several category levels, concatenated labels)
stays categorical, and so does a category with fewer than two different dates or a value that is
not a date. Power BI Desktop 2.159.1231.0 drew a column chart on a `General Date` column on such
an axis (French, 2026-11-24, "General Date" above); the label texts, the eight-label limit and
the room between labels below follow the same Desktop's drawings of daily, hourly, monthly and
quarterly data ("Seen in Desktop" below); the list of steps is BIcraft's.

- *Positions.* Each category sits at its date, proportional to time. Columns: the first and
  last stop half a pixel inside the plot (13 px short of the right edge), every column as wide as
  the smallest gap between two dates less the inner padding (20 % by default; on monthly data
  this is February's share of the mean month, the 0.92 measured on Desktop 2.158). Lines and
  areas without columns: points from 10.5 px inside each end, joined in time order whatever the
  order of the rows. Combo charts put their line points over the columns' centres. Horizontal
  bars run down the plot, the earliest date on top. A time axis never scrolls.
- *Ticks.* The finest round step that gives at most eight labels, each with the room of its
  width plus half a font size and at least six font sizes (a line and a half between horizontal
  bars' labels), ticks inside the data's range: 1, 5, 15 or 30 minutes, 1, 3, 6 or 12 hours,
  1 or 2 days (every other day of the month), weeks (Sundays, as d3), 1, 2, 3 or 6 months (the
  first of the month), then 1, 2, 5, 10… years. Steps finer than the data's smallest gap are not
  used; a step with fewer than two ticks in the range labels the first and last dates instead.
  Labels: years "2025", months "Jan 2025" (the culture's short month name), days "Jan 5" /
  "5 janv." (in the culture's day–month order) whatever the column's format string; when some
  date has a time, hour and day ticks carry the day and the culture's short time
  ("5 janv. 12:00", midnight included). Gridlines stay on the value axis only.
- *Interactions.* Hit regions are the drawn columns, bars and points, so clicks, tooltips and
  selections work as on a categorical axis; trend lines are fitted against time.
- *Tests.* `timeaxis_tests` (positions, widths, hits, categorical override, time order, ticks in
  en-US and fr-FR, an expanded hierarchy, a combo, hostile dates) and `timeaxis::tests`; golden
  `dates.png` (columns on irregular days and the same set `Categorical`, a line from unsorted
  rows, an area over hours, horizontal bars over quarters, a combo). `charts.png` changed by up
  to a pixel and a half: its monthly line now follows the months' lengths.
- *Seen in Desktop.* Power BI Desktop 2.159.1231.0 (Store edition, French UI, 2026-11-24) drew
  line and column charts the `bicraft` CLI wrote, 170 to 1,200 px wide, on 20 days, 60 days,
  72 hours, 12 and 36 months and 12 quarters, in General Date, Long Date and short date formats.
  Its tick texts: "4 janv." for days whatever the format, "janv. 2025" for months, "2025" for
  years, "1 mars 00:00" for hours (the day always, midnight too; no time on midnight-only data).
  Never more than eight labels, so a wider chart takes a coarser step (60 days at 1,200 px:
  months; 72 hours at 600 and 1,200 px: six hours; 36 months: six months); six-hour labels 66 px
  apart and yearly labels 69 px apart were kept, six-month labels 59 px apart were not. A
  `Categorical` axis on Long Date values turns its long labels and ends them with an ellipsis.
  `timeaxis_tests` `date_ticks_follow_desktop` holds the steps and texts of these charts (seen
  failing first); `charts.png` and `dates.png` were blessed again (the monthly line now
  quarterly, hour ticks with their day, no tick past the last date).
- *Left.* At 240 px Desktop labelled 20 days by their first and last dates only, where BIcraft
  labels weeks. In the same charts Desktop's line value axes did not start at zero (data from 10
  to 59: ticks 20, 40, 60 on a domain from about 8), on continuous and categorical axes alike,
  unlike the rule "line axes start at zero when the data are mostly positive" above: not changed
  until measured on more charts. With an axis `start` / `end`, date ticks run within those
  bounds (not seen in Desktop). Numbers, the axis `start` / `end` and the HTML viewer: next
  section; the viewer's date ticks keep the earlier steps and texts (2.3 font sizes apart, no
  eight-label limit, days in the column's format).

## Continuous numeric axes, axis start and end (2026-11-24)

Same module, same rules as dates, for numbers: Power BI's default type for a numeric category
field is continuous, as for dates (BIcraft's understanding of Desktop's behaviour; not yet
compared with a Desktop drawing).

- *Which axes.* A single category column whose data type is a number (`int64`, `double`,
  `decimal`, `currency`…, a model column, not a measure) is continuous unless the axis is set
  `Categorical`. It stays categorical when a value is blank or not a number, when the axis is
  expanded down a hierarchy, and when the visual has a drill hierarchy (the snapshot's `drill`):
  a level such as an auto date/time `Year` or `Day` is a number column that is categorical in
  Power BI (BIcraft's reading, not measured). Text columns holding digits stay categorical.
- *Positions.* As on a date axis: each category at its value, columns as wide as the smallest
  gap less the inner padding, lines and areas 10.5 px inside the ends and joined in value order,
  combos over the columns' centres, horizontal bars down the plot from the smallest value.
- *Ticks.* The finest step of 1, 2 or 5 × 10ⁿ whose labels keep 2.3 font sizes apart (a line and
  a half between horizontal bars' labels), at least 1 on a whole-number column, at the round
  multiples within the axis. Labels are written with the column's format string as its values
  are elsewhere (a whole-number column without one: "2018", no grouping); a decimal column
  without a format string takes the value axis' writing (the fewest decimals that write every
  tick, auto display units, or the axis' `labelDisplayUnits`).
- *Start and end.* `categoryAxis.start` / `end` (snapshot `x_axis.min` / `max`) of a continuous
  axis, numeric or date, extend it beyond the data and drop the categories outside it, which are
  then neither drawn nor hit. A date bound is read from an ISO text (the engine writes a
  `datetime'2026-01-01T00:00:00'` literal as its text), an OLE serial, or milliseconds since 1970
  (a number above 10⁸); how Desktop saves a date bound has not been observed, hence the three
  readings. A `start` not before `end` is ignored, both ends. Categorical axes ignore them.
- *HTML viewer.* The live viewer (`crates/view/src/page.html`, `contAxis`, `contTicks`) draws
  its own axes and now follows the same rules for dates and numbers: positions at the values,
  the same tick steps and label forms, `start` / `end`, trend lines against the values, lines in
  value order; its former evenly spaced date bands and skipped labels are gone. Its day labels
  go through its own date formatting (`fmtCell`), so texts may differ slightly from the PNG's.
- *Ribbon chart.* `ribbon.rs` keeps its own positions: dates proportional when every category
  is a date in increasing order, else even, numbers even. The ribbon joins each column to the
  next by rank, a categorical construction, and whether Desktop offers a continuous type on a
  ribbon chart has not been observed (the report theme schema lists `axisType` for every
  cartesian visual alike), so it is left as it was.
- *Tests.* `numaxis_tests` (positions, widths and hits on numbers, the categorical fallbacks,
  format-string ticks and value order, `start` / `end` on number and date axes, hostile numbers),
  the engine's `literals_and_shades` (`datetime'…'`), the viewer's static
  `continuous_axes_follow_the_renderer`; golden `numbers.png` (columns on irregular whole numbers
  and the same set `Categorical`, a line on formatted decimals from unsorted rows, an area widened
  by `start` / `end` with the last value dropped, horizontal bars on years, a date line cut to
  January). No existing golden changed.
- *Left.* Desktop's numeric tick steps and texts are not measured; the viewer's behaviour is
  checked by its static tests and a Node run of its axis functions, not by `cargo xtask
  webcheck` (its fixture has no continuous axis); the ribbon chart as above.

## Drilled visuals and matrix expansion (2026-11-06)

The renderer draws the drill state the engine sends (docs/view-snapshot.md, "Drill, drillthrough
and page tooltips"): a visual is evaluated at its displayed levels, so a drilled-down axis is an
ordinary axis of the lower level. An axis expanded down a level (`drill_expand`, two or more
`Category` columns in `data.columns`) groups its categories by every displayed level and labels
each with the levels' values joined by a space, as Desktop's concatenated labels do ("Wheels
Bike"); a point of such an axis carries one `{c, v}` per level. A matrix draws its saved or
reader expansion (`data.row_expansion`); with "+/- icons" on (`format.table.expand_buttons`) each
row header of a member with children (or of the deepest level shown when `drill.can_down` says
the hierarchy goes further) gets a boxed + (collapsed) or − (expanded) before its label, the
label moved right of it.

**Hit map.** A +/− sends `matrix_expand` / `matrix_collapse` with the member's path; every
matrix row header is a `HitAction::Menu` region `{"kind": "matrix_row", "page", "visual",
"path", "open"}` the front end's right-click menu reads (Expand / Collapse > Selection, Entire
level, All). The visual header's drill icons are drawn by the egui UI over the page, not by the
renderer (`bicraft render` output stays the published look).

## Tests

- `crates/render/src/tests.rs`: every family renders and draws something; scatter, text, shapes,
  buttons, images, unknown types, hidden visuals and groups; made-up values; page selection and
  the pixel cap; page images; hostile snapshots (huge sizes, NaN-like values, bad colours and
  icon names, 50 000-character runs, log axes over non-positive values, a small-multiples column
  out of range).
- `crates/render/src/slicers.rs`: each slicer mode draws its parts (boxes filled for the
  selection, partial "Select all", search rule, dropdown chevron, handles per range mode,
  hierarchy parents only) and hostile slicer states never panic.
- `crates/render/tests/golden.rs`: synthetic pages (charts; cards, gauge, table with data
  bars and icons, matrix, slicer, treemap, text box, placeholder; drilled visuals: an axis
  expanded down a level, an axis drilled into a member, a matrix with +/- icons and one member
  expanded; charts on continuous date axes, `dates.png`) and further pages (Gantt, maps) rendered with the bundled fonts
  only and compared with `tests/golden/*.png` (mean channel difference < 1, < 0.5 % of pixels off
  by more than 64), so it passes on Windows and Linux; `BICRAFT_BLESS=1` rewrites the goldens.
- `crates/engine/src/render.rs`: `report.render` on a session built with commands (inline base64,
  files, read-only, bad parameters); `crates/mcp/src/tests.rs`: `render_page` returns an image
  block.

## Fidelity oracle (Power BI Desktop)

`tools/render-oracle/` compares the renderer with Desktop's own rendering:

1. `cargo xtask oracle-gen synth` writes a synthetic report (one page per family: cards/KPI/gauge, bars and columns,
   lines/areas/combo, pie/donut/treemap/funnel/waterfall/scatter, table/matrix/slicer/shape/button,
   then the cartesian stress pages Stress, Labels, Combo and Axes) whose data is an M literal, and the visual rectangles; `bicraft run --file` builds the PBIX
   (DataModel included, so Desktop opens it with data). The report has no theme collection, so
   Desktop draws it in its classic look.
2. `bicraft render synth.pbix --out ours` renders it.
3. `capture.ps1` opens the PBIX in Desktop 2.158 and captures each page **without the screen**
   (the session's screen is off or locked): Desktop's report view is a WebView2 page; Desktop is
   started through its app execution alias with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` set for
   that process only (`--remote-debugging-port`, `--force-color-profile=srgb`), and
   `capture.mjs` clicks each page tab and takes `Page.captureScreenshot` of the report canvas.
   File > Export > Export to PDF driven through UI Automation was tried first: the menu items
   are reachable, but no Save dialog ever appears while the screen is locked. Without the sRGB
   flag the captures carry the display's colour profile (the classic teal #01B8AA came out as
   #63B5AA), which skews colour comparisons.
4. `cargo xtask render-compare DESKTOP OURS LAYOUT` (Rust, `xtask/src/rcmp/`) scales each
   capture to the render's size and scores every visual rectangle; see *Comparison method*.
   `--diff` writes Desktop | BIcraft | difference crops side by side, `--json` the full report,
   `--min-sim` / `--floors` make it return non-zero on a regression.

### Comparison method

Each capture is resized to the render's size (Pillow's bicubic filter: Keys a = −0.5, separable,
fixed-point weights) and both images are cropped to every visual rectangle of the layout. Per
visual:

| Metric | Definition | Reads |
|---|---|---|
| `sim` | 1 − mean \|grey(Desktop) − grey(ours)\| / 255, grey = ITU-R 601 luma, after a 2 px box blur | overall likeness; tolerant of anti-aliasing |
| `ink` | intersection over union of the ink masks: pixels whose grey differs from the crop's median by more than 24, dilated 2 px | whether marks are where Desktop has them (1–2 px offsets tolerated); meaningless on a box filled with one colour (shape: 0) |
| `ssim` | mean SSIM of the grey crops (Wang et al. 2004: 11×11 Gaussian window, σ 1.5, K1 0.01, K2 0.03, L 255; valid windows only, no padding); none below 11 px | structure: edges, text strokes, thin lines; much stricter than `sim` |
| `ΔE00` | mean CIEDE2000 (sRGB, D65) over the pixels that are ink in both crops (masks not dilated), on the box-blurred colours | colour fidelity of the shared marks; CIEDE2000 rather than CIE76 because the palette has saturated blues and greys where CIE76 overstates differences |
| text | matched share of the larger of the two text lists, plus missing, extra and differing runs | strings, sizes, colours, weights, positions |

`sim` and `ink` reproduce the earlier Python script's arithmetic (Pillow's luma, box blur and
rank filter, numpy's median, Python's rounding of the crop box): on the three oracle reports
(141 visuals) both agree with it to the fourth decimal.

The text check compares Desktop's DOM text (capture.mjs: computed family, size, weight, colour and
box of every visible text node, converted to report pixels with the canvas width and the
element's screen scale) with the runs the renderer drew (`report.render {text_dump: true}`,
`bicraft render --text-json`: string, family, px size, weight, colour and line box of each run).
Runs belong to the visual whose rectangle holds their centre. Strings match after whitespace
normalisation (no-break and thin spaces), equal strings pair by nearest centre, and a run of ours
ending in "…" matches the Desktop string it starts (Desktop truncates in CSS, its DOM keeps the
whole string). A pair *differs* when the size is off by more than 0.5 px, the colour by ΔE00 > 3,
the weight or base family differ (`Segoe UI Semibold` = Segoe UI 600; the comparison counts
`wf_standard-font` as BIcraft's `DIN` and `wf_standard-font_light` as Segoe UI Light), or the box centre moved more than 3 px.

Limits: the captures come from one machine (Desktop 2.158, French UI, sRGB forced, 150 %
display scaling, so a 1920-wide capture resized to 1280); text drawn inside images or canvases
(none in these reports) is invisible to the text check; Desktop's line boxes include its line
height while ours are ascent + descent, so vertical offsets of a pixel or two are normal; strings
formatted differently (en-US numbers in a French UI, M10.1) count as missing + extra. The
captures are Microsoft-rendered and stay local (`%BICRAFT_TESTDATA%\render-oracle`, see
`tools/render-oracle/README.md`); `cargo test -p xtask oracle` re-renders the three reports and
holds per-family floors (`tools/render-oracle/floors/`), and skips without them. The committed
goldens (`crates/render/tests/golden`) remain the Linux CI guard.

First run of the full method (2026-10-14, renders in fr-FR, sim / ink / SSIM / ΔE00 / text):
synthetic report 0.988 / 0.85 / 0.87 / 1.9 / 0.67 (413 of 661 Desktop texts matched; most
misses are en-US number and date formats), parts 0.992 / 0.80 / 0.90 / 2.6 / 0.80, slicers
0.995 / 0.84 / 0.92 / 3.1 / 0.69. The text check found pie, donut and funnel labels drawn at
11.04 px in Segoe UI where Desktop uses 12 px `wf_standard-font`: they are now 12 px `DIN`
(classic look; Segoe UI stands in at DIN's width, so the pixels did not change), the funnel's
rates and after-bar labels #777 like its categories.

Re-run after the later renderer changes of the same day (date-axis label gap, scrolled-label
angle, model culture): a 12-month date axis whose labels fit their category but not with the
0.4-font-size gap word-wrapped every month ("May / 2025") where Desktop shows every other month
flat, so a date axis now thins its labels at that gap (line, area and stacked area back above
their floors: ink 0.52 → 0.85 on line charts); a 28-category chart that scrolls turned its labels
35° where Desktop turns them 90° (the scrolled band step, 20.7–21.5 px, sits just above
1.9 × 8 pt), so a scrolling chart decides on its preferred category width (sim 0.875 → 0.976,
text 18/33 → 33/33). The KPI goal's distance keeps the culture's percent pattern ("-33,33 %",
as Desktop; a value's `0.0%` format string stays "38,0%"). With numbers in the model culture
(fr-FR on the parts report) the KPI value reads "78 270" like Desktop; on `kpi_trend`, whose ink
masks barely overlap (5 %, the trend area flips the crop's median), ΔE00 moved 15.4 → 17.0, so
that one ceiling was raised to 18 (`floors/parts.json`); every other floor is unchanged.

Grids (2026-10-14, `oracle-gen grids`, set `grids`: 34 tables and matrices on 5 pages: scroll thumbs
at 20 to 1000 rows, wide grids, matrix subtotal options), sim / ink / SSIM before → after the
grid work: 0.958 / 0.47 / 0.61 → **0.981 / 0.77 / 0.73**; on the synthetic report tables
0.659 → 0.781 ink, matrices 0.559 → 0.734 ink (ΔE00 7.7 → 4.7, classic grid text #333333). The
text check also found pie and donut inside labels drawn at weight 600 and funnel values at 700
where Desktop draws 400 (unless `labels.bold`): pie inside labels 0.12 → 1.00 matched texts.
Those families' floors were raised; none was lowered.

The same steps ran, locally only, on a real themed report (115 visuals on 10 pages, never
committed or quoted). Desktop draws numbers, dates and automatic titles in its UI culture
(French here) while `report.render` then rendered in en-US (it now formats in the model's
culture); those text differences are included in the scores below.

### Results (Desktop 2.158)

Per family, synthetic report (sim / ink):

| Family | sim | ink | Family | sim | ink |
|---|---|---|---|---|---|
| card | 0.991 | 0.59 | pie | 0.978 | 0.81 |
| card (new) | 0.985 | 0.63 | donut | 0.966 | 0.68 |
| multi-row card | 0.977 | 0.33 | treemap | 0.978 | 0.89 |
| KPI | 0.989 | 0.90 | funnel | 0.962 | 0.84 |
| gauge | 0.971 | 0.36 | waterfall | 0.992 | 0.90 |
| clustered column | 0.989 | 0.93 | scatter | 0.992 | 0.29 |
| stacked column | 0.990 | 0.93 | table | 0.991 | 0.48 |
| 100 % column | 0.991 | 0.96 | matrix | 0.982 | 0.40 |
| clustered bar | 0.983 | 0.89 | slicer (list) | 0.997 | 0.85 |
| stacked bar | 0.986 | 0.91 | text box | 0.997 | 1.00 |
| 100 % bar | 0.979 | 0.92 | shape | 0.995 | – |
| line | 0.994 | 0.64 | button | 0.979 | 0.74 |
| area | 0.975 | 0.93 | stacked area | 0.992 | 0.98 |
| line and column | 0.994 | 0.96 | | | |

Means: synthetic report 0.930 → **0.970** sim, 0.46 → **0.64** ink (the first figures are the
viewer's drawing ported as is, measured on the colour-managed captures), then **0.981 / 0.79**
after the text work below (the 6-page report with the stress page, together with the chart
helpers' changes of the same day); real report 0.898 →
**0.959** sim, 0.41 → **0.45** ink. On the real report the weakest families are columns and
line-and-column combos (0.85–0.91: bar widths and axis label rotation), 100 % charts (0.88) and
the gauge (0.90 then; 0.968 with the snapshot's `gauge` keys and Desktop's layout, below).

Cartesian charts (bars and columns rows above, with the cartesian rules below; plus the stress
pages of `oracle-gen synth`: 36 categories with a scroll bar, long labels turned 35° and 90°, word-wrapped
labels, data labels in each position with units, precision, background and per-series
settings, combos with two axes, per-series dashes and markers, columns on a date axis, and eight
value axes ending at different distances from a tick): every column, bar and combo visual 0.976
or more (Stress page 0.87–0.89 before). Real report, family means before → after: clustered
column 0.855 → 0.969, stacked column 0.909 → 0.976, stacked bar 0.933 → 0.971, clustered bar
0.927 → 0.970, 100 % bar 0.881 → 0.970, 100 % column 0.885 → 0.962, line-and-clustered-column
0.878 → 0.967, line-and-stacked-column 0.888 → 0.964, area 0.948 → 0.960, waterfall 0.951 →
0.963.

Slicers (`cargo xtask oracle-gen slicers`, 41 slicers on 10 pages, French Desktop UI,
rendered with `--lang fr-FR`, numbers and dates still formatted en-US (M10.1), sim / ink): list
0.997–0.998 / 0.90–0.94, with "Select all" 0.997 / 0.94, single select 0.998 / 0.93, inverted
0.997 / 0.93, hierarchy 0.998 / 0.93, search box 0.998 / 0.93–0.94 (`search_box`), header off
0.998–1.000 / 0.77–0.94 (`header_show`), tiles 0.995–0.999 / 0.93, button slicer 0.996 / 0.74,
new list slicer 0.999 / 0.87, dropdown 0.996–0.998 / 0.94, between / before / after on numbers
and dates 0.993–0.996 / 0.90–0.95, filter-only ranges (shown as the whole domain) 0.992–0.993 /
0.92–0.94, displayed ranges (`numericStart`/`startDate`…, the slider on the column's `domain`)
0.992–0.998 / 0.69–0.77 (number and date formats), relative date unset 0.995 / 0.82, relative
date with settings (last 3 months, next 7 days, last 7 days, this week, this month) 0.995–0.996 /
0.82–0.87, text slicer 0.993–0.994 / 0.53–0.54; mean 0.996 / 0.86.

Parts, indicators and cards (`cargo xtask oracle-gen parts`, 49 visuals on 8 pages: pie
label styles, positions, legend and start angle, donut holes, gauges with literal and
field-bound min / max / target, new-card layouts, rows tables and accent bars, multi-row cards,
KPI variants, funnels, treemaps; French Desktop UI, sim / ink): pie 0.992–0.998 / 0.95, donut
0.991–0.997 / 0.94, gauge 0.985–0.992 / 0.52, new card 0.987–0.995 / 0.78, multi-row card
0.990–0.998 / 0.79, KPI 0.986–1.000 (ink is meaningless on the filled trend area), funnel
0.969–0.992 / 0.89, treemap 0.982–0.987 / 0.95; mean 0.966 → **0.992**. On the real report: gauge
0.90 → 0.968, pie 0.989, donut 0.989, treemap 0.990, KPI 0.98, new cards 0.945–0.985 (French
number formats), funnel 0.958 (its label style is not read yet).

### What the oracle changed

- **Text styles from Desktop's DOM.** The capture can also read every text node's computed
  font family, size, weight and colour from the report page. In the classic look, Desktop draws
  axis labels, legend items, table and matrix cells and data labels at 8 pt (10.67 px; the
  renderer had 9 pt), titles at 9 pt (12 px), and legend items and titles in #666. Card callouts
  use `wf_standard-font` at 27 pt, KPI and gauge callouts use `wf_standard-font_light` (Segoe UI
  Light) at 45 pt, and pie, donut, funnel and treemap labels are 9 pt. Classic tables keep 17 px
  rows and 6 px cell padding at 8 pt. Regular text had been drawn with the Light or Semilight
  face because weight matching was wrong. Fixing both took tables from 0.48 to 0.69 ink and
  matrices from 0.40 to 0.59.
- **Category order.** Without a `sortDefinition`, Desktop queries bar, column, pie, funnel,
  waterfall charts and treemaps in category order (the treemap lays out by size but colours in
  that order). The snapshot producer sorted those by value; it now follows Desktop
  (`crates/engine/src/preview.rs`, docs/view-snapshot.md).
- **Reports without a theme** use Desktop's classic look: palette #01B8AA, #374649, #FD625E,
  #F2C80F…, light grey regular titles (#C8C8C8, about 9.5 pt), grey axis labels (#777777),
  solid #EAEAEA gridlines, compact tables (columns sized to their content, accent rules, banded
  rows), "good" #3BB44A.
- **Container.** The report's padding (`visualContainerObjects.padding`, theme included; new in
  the snapshot), title at the padding, an empty subtitle showing the automatic title on charts,
  8 px under the header.
- **Axes.** Solid gridlines; "auto" display units on axes from 10 000 (K) and 100 000 (M, so
  300 000 is 0.3M) with uniform decimals; the axis' `labelDisplayUnits` (new in the snapshot);
  line axes start at zero when the data are mostly positive; a continuous date axis labels
  round dates ("Jan 2025", "Jan 5"), never turned ("Continuous date axes" below); scatter axes hug the data and show axis
  titles. Value domains, ticks and category layout: "Cartesian charts" below.
- **Cartesian charts** (`axes.rs`, measured on Desktop's own SVG through the DevTools port: tick
  positions, bar rectangles, label transforms, 23 synthetic and 12 real charts).
  - *Value axis.* BIcraft's fit to these captures uses d3's published algorithms (ported, BSD-3-Clause,
    THIRD_PARTY_NOTICES.md), chosen because they matched the measured ticks best: the domain is
    d3 v3's nice range of the data with 3, 5 or 8 intervals (a
    vertical axis under 150 / 300 px, a horizontal one under 300 / 500 px); the ticks are
    d3-array's tick increment for 2, 3 or 5 intervals of that domain, so the axis may end between ticks
    (224.7K on 272 px: domain 0–250K, ticks every 100K; 1,230 on 202 px: 0–1,400, ticks every
    500). Column and line axes alike; a line axis does not include zero unless the data are
    mostly positive.
  - *Categories.* d3 v3's `rangeBands` (ported; chosen because it matched the measured bars
    best) with the category axis' `innerPadding` (20 % by default) and
    an outer padding of 0.4 categories each side; a bar is 0.966 of its slot. Categories never
    get narrower than `preferredCategoryWidth` (20 px): the chart then shows one category fewer
    than fit and a 9 px scroll bar (track #F4F4F4, thumb #C4C4C4) under the columns or right of
    the bars. Clustered series share the bar without gaps. Columns on a continuous date axis sit at
    their dates from edge to edge, 13 px short of the right edge, like line charts' date axes.
  - *Chart text size.* Axis labels, legends and data labels are 9 pt (12 px; 8 pt in the classic
    look) whatever the theme says: with Fluent2-CY26SU08 as base theme and a custom theme whose
    `textClasses.label` is 10.5 pt, whose `*` `categoryAxis.fontSize` is 14 and whose per-type
    `valueAxis.fontSize` / `legend.fontSize` are 10.5 and 11, Desktop 2.158 still drew every axis
    label and legend item at 12 px (its DOM's computed style), while the theme's title size and
    axis label colour applied. A visual's own `fontSize` applies. The snapshot's axis
    `font_size` therefore comes from the visual only; tables keep the label class size.
  - *Category labels* (8 pt, #777): flat when every label fits its category with a gap of 0.4
    font sizes (4.8 px at 9 pt; 43.6 px "2025-01" labels: flat on 54 px categories, turned on 47 px ones);
    else word-wrapped when every word fits ("Public Sector / Organizations"); else turned 35°
    (90° when a category is narrower than 1.9 font sizes: 90° on 20 px at 8 pt, 35° on 24.2 px
    at 9 pt; a scrolling chart compares its preferred category width, 20 px, not its slightly
    wider laid-out step), the turned labels' end at the category centre, cut
    with an ellipsis to a quarter of the chart's height, which is reserved under the plot; the
    plot moves right so that the first 35° label stays inside. Date axes skip labels by 1, 2, 3,
    6 or 12 periods as soon as they would not keep the 0.4-font-size gap (never word-wrapped). Horizontal bars: labels right-aligned 9.5 px from the plot, their column at
    most `maxMarginFactor` of the width.
  - *Layout.* Value labels 10 px left of the plot, the plot's top half a label height under the
    chart (8 px for bars), 22.5 px of flat labels under it; legend rows one line plus 12.7 px with
    5 px dots; a classic chart's plot area starts 4 px under its 17 px title. The secondary axis
    of a combo: its own domain and ticks on the full height, labels 9 px right of the plot, 6 px
    tick marks inside it, no gridlines. A line of the columns' magnitude (the two domains
    overlap on at least a tenth of their union, whatever the format strings) or with the
    secondary axis turned off is drawn on the primary axis instead, which then covers both
    domains (a signed balance next to amounts goes below zero; Desktop labels no tick there).
  - *Data labels.* Auto position: outside the end of clustered bars and columns, inside the
    centre of stacked ones; `labels.position` / `series_labels` give inside end (12 px in),
    inside base, inside centre. Auto display units use the value axis' unit with 2 decimals
    ("0.07M"; 100 % charts: two-decimal percentages), else the label's units and precision.
    Inside labels are white or black unless the report's colour reads on the bar; they are cut
    to the column's width (left out on horizontal bars); stacked segments lower than a label are
    skipped; an outside label of a bar thinner than the text uses the gap to the next bar; outside labels take the axis' top (label height + 8 px) or right (label width + 19
    px) away from the domain; `background` draws a rounded box 8 px wider each side at the
    background colour and transparency.
  - *Lines.* `lines` then `series_lines` per series: width (2 px by default), dashed or dotted,
    markers (circles, `marker_size`), marker colour; line labels 9 px above the points.
- **Cards and indicators.** Card callout 27 pt; display units use the format string's decimals
  ("910K"); new cards with label and value; multi-row cards in columns at Desktop's sizes; KPI
  value with its format string, the trend area under it; gauges filling the visual (ring 30 % of
  the radius, percentages to 100 %).
- **Slicers.** Classic header black 12 px (9 pt, Desktop's computed style), items #666666 10 pt every 19 px, 13 px boxes filled
  #212121 when selected, "Select all" partial state; horizontal tiles in an 18 px inset, the
  selected one #333333; dropdown and range inputs with #EAEAEA borders, disabled bound grey,
  slider 3 px track between 4.5 x 15 px handles; the new list, button and text slicers show
  their field name as a light title, radio buttons at 8 pt, 8 px gaps between buttons. A range
  input of a decimal column without a format string shows two decimals.
- **Pie and donut** (`parts.rs`). Radius: 80 % of min(w, h) / 2, divided instead by
  2 + 1 / (1 + e^(−5 (h/w − 1))) when labels show (an empirical formula of BIcraft's, checked
  against the pie and donut captures of the parts oracle above); donut hole 60 %; slices from 12 o'clock
  plus `pie.start_angle`, coloured in category order whatever the sort. Outside labels: a leader
  line to 1.13 r then 13 px across, #666; per side, labels pushed apart (the smallest slices'
  dropped when a side is full), a label too wide for the visual's edge wrapped after the
  category (or the value) and truncated. `labels.label_style` (Category, Data, Percent of total,
  Both, and the comma combinations; percentages with `percent_precision`, 2 by default) and
  `labels.position` (inside: white text on 50 % black rounded boxes at the slice centroid,
  wrapped when wider than the ring).
- **Gauge.** Width-bound gauges centre the arc with the target label's room above (when the
  target is near the top) and min / max 13 px under the ends; height-bound ones put min / max
  beside the ends. Axis numbers in auto display units picked per value (zero takes the
  maximum's: "0M") with no decimals, percentages with their format string; callout Light at
  0.51 × the hole's radius (at most 57 px), centred 0.263 hole radii above the centre; target
  line from the hole to the rim, #374649 classic.
- **KPI.** Nothing without a trend axis; the trend area fills the plot (maximum at the top), the
  value centred (raised when a goal shows), "Goal: … (±x %)" semibold #333 under it (the
  distance in the culture's percent pattern: "(-33,33 %)" in fr-FR), a check
  mark or "!" after the value, classic bad #E81123. The value's display units (Desktop 2.158 on
  `oracle-gen kpis`: 8 + 12 format strings at 950 to 78 million, and the `indicator` options):
  automatic units scale 1000 and more to K, M… only when the format string has two decimals or
  more (`#,0.00` "78,27K", `#,0.000` "7,827M", `#,0.###` "78,27K") or there is none (two
  decimals: "7,83K", "950,50"); `#,0`, `0`, `#,0.0`, `\$#,0.0` and percentages keep the format
  ("78 270", "7 827 025,0", "95050,0%": a percentage without `,` is not grouped).
  `indicatorDisplayUnits` 1000… scales whatever the format (its decimals when two or more, else
  two: "78,27K" for `#,0`), 1 never; `indicatorPrecision` applies to a scaled value only
  ("78K", "7,8M"; `#,0` with precision 2 stays "78 270"). A scaled value keeps the format's
  literal ends without their spaces ("78,27K€", "78,27Ku"). Not checked: whole numbers without a
  format string under 1000 (drawn without decimals), negative values. Classic KPIs draw the
  value at a fixed 59.7 px (45 pt; `indicator.fontSize` when set) whatever the visual's size,
  overflowing a narrow one as Desktop does, its box centred at 0.519 of the visual's height
  (-1.7 px), raised 0.12 of its size above a goal and moved left by half the status icon; the
  goal is two runs ("Objectif: 74 356", "(+5,26 %)") centred together; a flat trend draws no
  area. Desktop's value font is `wf_standard-font_light`, drawn here in Segoe UI
  Light, which limits the `kpis` set to sim ≈ 0.95.
- **New card.** Fixed sizes (classic 18 pt label #666 and bold value; themed 12 / 28 pt) as two
  line boxes 8 px apart (2 px with a theme), centred in each tile; tiles 8.7 px apart in a row
  or a column, whichever gives tiles nearer 3:2 (`layout.orientation` 0 column, 1 row, 2 grid);
  accent bar 2 px #333 by default; `label.position`, alignments, value units and precision. A
  `Rows` category gives Desktop's table: three rows a page, the category bold in a left column,
  a chevron when more rows follow.
- **Multi-row card.** Title semibold 13 pt #777, values 10 pt, labels 9 pt #A6A6A6 every
  41.3 px, cards 82.7 px apart with a 2 px bar, a scroll thumb when they overflow.
- **Funnel and treemap.** Funnel rate brackets each take one stage's height (at least 22.7 px),
  value labels in the visual's display units, after a bar too thin for them; treemap in d3 v3's
  squarified layout (ported) with the golden-ratio aspect, chosen because it matched the treemap
  captures of the parts oracle best, white labels.
- **Charts.** Bars fill 76 % of their band; legend titles shown for legend fields; pie and donut
  without legend, labels naming slices; treemap labels; funnel conversion brackets; waterfall
  legend and connectors; buttons take their fill and text style from the theme (new in the
  snapshot: `format.button_fill`, `format.button_text`).

### Snapshot keys the renderer uses (beyond the viewer's)

`format.padding`, axis `display_units`, `font_size`, `inner_padding`, `category_width`,
`max_margin`, the secondary axis `format.y2_axis`, `format.button_fill` / `button_text`, and the renderer
formatting keys (`docs/view-snapshot.md`, "Renderer formatting"): `labels` (display units and
precision of data labels, `label_style`, `position`, `percent_precision`, font), `lines` (width,
dashes, markers), `series_labels` / `series_lines` (per series), label `background`,
`background_color`, `background_transparency`, `gauge` (min, max, target, literal or field-bound; `callout`, `target_label`,
`target_color`), `pie.inner_radius` / `start_angle`, `card` (`accent_bar`, `value`, `label`,
`layout.orientation` / `columns`), `table` (grid `row_padding`, value and header
`font_size`, `column_widths`, `sort`; tables and matrices), `slicer.header_text`, `ribbon` and
`totals` (ribbon charts). Text follows the snapshot's UI language: the visual's
`auto_title`, `fields[].label`, translated `caption`s, and `ui.strings` (blank, total, all,
multiple selections, search, increase/decrease, a KPI's goal, aggregation names).

### Known differences

Cartesian charts: label density and overlap avoidance between line and column labels (Desktop
moves a line label under its point or boxes it), the themed report's 2 px gaps between stacked
segments and
`concatenateLabels` hierarchies are not modelled (continuous date axes: see "Continuous date
axes") (ribbon charts: see "Ribbon chart"). A hierarchical axis (several projections of a
role with `active` flags) is queried, sorted and auto-titled at the level(s) marked `active`
(the top one when no drill state is saved), like Desktop; the merge rule of combo axes (overlap
of a tenth) is inferred from a few reports. Other renderer formatting keys (new-card images, reference
labels and the table / cards `layout.style` difference) are not used yet; themed pie labels are
placed by angle where Desktop sometimes moves them further round; the KPI's goal word is
English outside `ui.strings.goal`; slicers use the snapshot's `search_box`,
`header_show`, `range` (Desktop's displayed bounds; a `filter_only` range shows the domain, the
`*_exclusive` flags are not displayed), `domain` (slider ends) and `relative` (range, duration,
period, singular after "This", and the covered days `from` / `to`) keys, with the relative-date
words in the snapshot's UI language (English and French tables); the calendar period variants
are not checked against Desktop (the oracle's `CalendarMonths` / `CalendarYears` literals are
not Desktop's: it shows them unset); custom visuals (with their package's icon), maps and key influencers are labelled
boxes; decomposition trees: see "Decomposition tree" for what is left; hinting is vertical only and by the automatic hinter
where the font has no hints (DirectWrite's own rendering is not reproduced, see "Hinting");
right-to-left text is shaped but not reordered by the
bidi algorithm.

Statuses: a visual is `drawn` only when everything it shows was evaluated. Text box dynamic
values, button text and fill and titles bound to measures are evaluated (`docs/view-snapshot.md`,
"Bound values"); when one of them, or a conditional format reading a measure, cannot be, the
visual is `preview` with the reason. `TODAY()` / `NOW()` (and `UTCTODAY()` / `UTCNOW()`) return
the system clock in UTC (Desktop uses the local time: they can differ by a day around midnight),
or the fixed `as_of` of `report.render` / `view.set_as_of` / `bicraft render --as-of` for
reproducible renders; a Desktop capture taken on another day shows other ages.

All statuses: `drawn` (the renderer drew the visual from evaluated values, or a text box,
shape, image, button), `preview` (made-up values or an unevaluated bound value, with the
reason in `note`), `placeholder` (a type drawn as a labelled box), `blank` (a data visual with
no field bound: Desktop's empty-visual prompt) and `skipped` (outside the page, too large).
`status` says what was **drawn**, not whether data came back: a card whose measure is blank
(it shows "(Blank)") or a chart whose filters remove every row is still `drawn`.

### Data states

Each visual of `report.render` also has `data`, read from its evaluated dataset (the
snapshot's `data`), never from the drawn text, so it does not depend on the UI language:

| `data` | Meaning |
|---|---|
| `ok` | Rows came back and at least one measure/aggregation cell is not blank (or there are rows and no value columns: a slicer, a table of columns) |
| `no_rows` | Zero rows, and the visual groups by at least one column (charts, tables, matrices, slicers, a KPI's trend axis) |
| `blank_values` | Every measure/aggregation cell is blank, or zero rows with no grouping column (a card, multi-row card, gauge or KPI bound to measures only: the engine's query returned nothing) |
| `partial_blank` | Rows, some value cells blank and some not (sparse matrices, gaps in a series): informational |
| `not_bound` | No data field: text box, shape, image, button, navigator, a data visual with nothing bound (`status: blank`) |
| `not_evaluated` | Fields are bound but the engine returned no dataset (`data.status` ≠ `ok`: no table data, an expression it cannot evaluate, an error); `status` is then `preview` |

With a dataset, the visual also has `rows` (the whole result's row count, `data.total_rows`
when truncated), `value_cells` (measure and aggregation cells of the returned rows, at most
100 000 rows examined) and `blank_cells` (those that are blank). Each page has
`visuals_without_data`, the number of its (non-hidden) visuals whose `data` is `no_rows`,
`blank_values` or `not_evaluated`. `fail_on_empty: true` (CLI `bicraft render
--fail-on-empty`) turns that into a failure: the command returns an error naming up to 20 such
visuals (`page / visual (type): data`); the CLI still writes the PNGs and prints the result,
then exits 1. The live viewer's snapshot carries the same state per visual as `data_state`
(`docs/view-snapshot.md`). The computation is `bicraft_render::data_state`
(`crates/render/src/state.rs`).

Tables and matrices (checked against Desktop 2.158 captures): with a grid row padding (theme or
visual) a row is the text line (1.36 × the value font size: 19 px at 10.5 pt), twice the
padding and a 1 px rule; the header adds 8 px under its text for the sort arrow (▲ / ▼ under the
sorted column, from `sortDefinition`). In the classic look rows are 17 px (matrices 16 px; with a
row padding: line + twice the padding, no rule), the header is 24 px with its text in the top
16 px (the arrow's room is always there), headers are left aligned (numeric ones too) and the
grid text is #333333; a visual's value font size does not change the header's. When the rows
overflow (classic or themed), the row cut by the bottom is drawn clipped, the total row stays at
the bottom and a scrollbar thumb is drawn: an 8 px #AFAEAD pill against the body's right edge,
its track 1.5 px from the top to 2.5 px from the bottom, its length the track × the rows' room /
(row count × row height), at least 8 px (the engine's `total_rows`, so a 1000-row table gets the
minimum). Measured on `oracle-gen grids` (20 to 1000 rows, 150 to 360 px high, two row paddings):
every thumb within 2 px of Desktop's; the earlier "28 % for 59 rows" was this same rule (the
track is the visual's height, not the table's). Themed grids keep a 19 px strip for it. Grids
wider than the visual keep their natural (or `columnWidth`) widths, are cut at the body's edge
(at the vertical thumb when both scroll) and get a horizontal thumb 8 px high at the body's
bottom, from 1.5 px, its length the visible share of the width; the rows lose those 8 px.
A table's total row is the engine's `data.totals` (each measure evaluated in the grand-total
filter context, as Desktop: % of total 100 %, `RANKX` over `ALLSELECTED` 1, year-on-year change
over all dates; checked against Desktop 2.158 and the `totals-*` entries of the `dax.evaluate`
oracle corpus); without it (measure filters) additive columns are summed.
A matrix shows up to 60 column groups (blank group: empty header, as Desktop). Its subtotals
and grand totals are the engine's `data.subtotals`: the visual evaluated again at each displayed
(row level, column level), so a ratio's total is the ratio of the totals (checked against
Desktop 2.158 on `oracle-gen grids`: 41,8 % where the cells average otherwise); without them (fake
data, measure filters) additive values are summed. Expansion (checked against Desktop 2.158 on
`oracle-gen expand`, whose `expansionStates` and `subTotals` shapes are the ones Desktop saved
after the same actions in its UI; `docs/formats/pbir.md`, "Matrix expansion"): a hierarchy
shows the levels its projections mark `active` (none marked: the top level only, whatever
`expansionStates` says; `[false, true]`: drilled down to the second level), each active level
expanded unless `expansionStates` lists it with `isCollapsed`, and `root.children` members with
`isToggled` are the exceptions (an expanded member of a collapsed level, nested `children` for
deeper ones, or a collapsed member of an open level). Rows and column groups are drawn per
member: a collapsed member shows its values, an expanded one its children; every row of a
level above the deepest one shown is bold. Expanded column members get one header row per
level (the member over its children, then a bold "Total" column after them; the corner shows
the column field names above the first row field's name; the grand total's "Total" sits in the
first header row). Row subtotals sit on the member's row, or with `rowSubtotalsPosition` Bottom
in a bold row after its children at their indent (the member's row then has no values). The
total column and row follow `subTotals.columnSubtotals` / `rowSubtotals` (off: no grand total
either); with `perRowLevel` / `perColumnLevel` the entries whose `selector.metadata` names a
level field switch (`levelSubtotalEnabled`) and label (`levelSubtotalLabel`) the subtotals that
total over that field: the top field's entry is the grand total, the second's the subtotals of
the top members (a subtotal switched off at the top position leaves its member's row empty).
`rowSubtotalsLabel` / `columnSubtotalsLabel` rename every subtotal and the grand total. The
`+`/`-` buttons (`rowHeaders.showExpandCollapseButtons`) are drawn left of each row header that
has a deeper level, and up to eight levels per hierarchy are evaluated and drawn (each its own
indent and subtotal). Not modelled: the tabular (not stepped) layout, more than eight levels per
hierarchy, more than four values. Rows tied on the sort columns follow the other grouping
columns, ascending and case-insensitive (Desktop's query order). The text check counts rows and
columns hidden by a grid's overflow (they stay in Desktop's DOM) against the visual whose
rectangle they fall into, so `grids` text scores of stacked scrolling grids stay below 1.
