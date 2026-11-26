# BIcraft's look: the shared style of BIcraft's own visuals

Sources: our own design. The palette was checked with our colour checks (lightness band,
chroma floor, colour-vision-deficiency separation of adjacent colours, contrast on white); the
SVG features are the W3C SVG 1.1 `linearGradient`, `feDropShadow` (Filter Effects 1) and CSS
animations; no existing visual's look, code or image was copied. ROADMAP M12.3.

One layer gives the Spec visual, the four chart visuals ([bicraft-visuals.md](bicraft-visuals.md))
and the chart pack ([chart-pack.md](chart-pack.md)) the same look. It exists twice, as each
layout does: `crates/render/src/look.rs` for BIcraft's renderer and
`crates/engine/src/visual_runtime/style.ts` for the visuals inside Power BI. Both turn the same
Style options and theme into the same tokens and decorated primitives, and the golden tests
compare them.

## Style options

| Option | Values | Default |
|---|---|---|
| `preset` | `modern`, `minimal`, `bold` | `modern` |
| `palette` | `theme` (the report theme's data colours), `bicraft` (BIcraft's own) | `theme` |
| `rounded`, `gradient`, `shadow`, `animation`, `card` | true / false | true, except under `minimal` (false) |
| `pills` (value chips) | true / false | true under `bold`, else false |

Where they live:

- The chart visuals: format object `style` (`preset`, `palette`, `rounded`, `gradient`, `card`, `pills`,
  `shadow`, `animation`), the same object as the bullet's `vertical` and the dumbbell's `slope`.
  The layout reads `options.style`.
- The chart pack: format object `style` with the same properties; the layout reads them as flat
  options `stylePreset`, `stylePalette`, `styleRounded`, `styleGradient`, `styleShadow`,
  `styleAnimation`, `styleCard`, `stylePills`.
- The Spec visual: the spec's `style` object with the same keys (checked like every spec key:
  an unknown key or value is an error naming it), and since 1.3.0.0 the format object `style`
  of its Style card, whose settings win over the spec's (`snapshot_fields` merges the valid ones
  into the snapshot's `spec.style`; the visual merges them before laying out).

The format pane shows them in the Style card (group "Look" / "Apparence", EN and FR): two
drop-down lists (enumeration properties) and six switches ("Card background" / "Fond en carte",
"Value chips" / "Pastilles de valeur" besides the four first ones).

## Colours

- The theme: the renderer passes the report theme's data colours (`ctx.colors`); in Power BI the
  visuals read `host.colorPalette.getColor("0")` to `"7"`. The layouts receive them as
  `{colors: ["#RRGGBB", …], background?: "#RRGGBB"}`: the chart visuals as option `$theme`, the
  chart pack as input `theme`, the Spec visual as `layout`'s `theme` (that object, or the colour
  list alone). The background is the theme's (`host.colorPalette.background`, not read in high
  contrast). A spec's own `palette` still wins.
- Without a theme, with `palette: bicraft`, or (Modern and Bold) with Power BI's stock themes'
  colours (the first eight of the default 2020 theme, `#118DFF …`, or of the classic one,
  `#01B8AA …`: a report that never chose colours), BIcraft's palette, in this fixed order
  (since 2026-11-24, the look's second pass): indigo `#4A5BE8`, vermilion `#EA5A28`, teal
  `#0E9F84`, amber `#BF8700`, magenta `#D6408E`, sky `#1C8FCC`, violet `#8A4FE0`, crimson
  `#E23D4F`. Checked on white and on `#1F1F1F`: every check of our palette validator passes
  (lightness band, chroma floor, contrast as a mark), adjacent colours stay apart for the three
  common colour-vision deficiencies (worst adjacent ΔE 8.6, OKLab × 100), and the first three
  are apart in every pair. Single-series charts keep the first colour with its gradient.
- `minimal` without a theme keeps the earlier palette (`#1F6F8B #E07A1F …`) and the earlier
  tokens, so Minimal draws exactly what the visuals drew before the look (the golden cases check
  it, `crates/engine/tests/report/look_minimal_tests.rs`); it keeps a stock theme's colours.
  Since the second pass two deliberate changes are listed there: Sankey links that skip a column
  (routed through waypoints under every preset) and `palette: bicraft` (the new palette).

Dark backgrounds: when the theme's background has a relative luminance under 0.07, the look
switches to dark-surface tokens: axis and category text `#CFCEC4`, values and names `#F0EFEC`,
gridlines `#3B3A37`, axis lines `#5E5D58`, empty cells `#34332F`, bands from `#6E6D68`, and the
surface (rings, gaps, tints) is the background, or the card's fill. Both texts stay at 4.5:1 or
more on every such background, card or not (`look.rs` test `dark_backgrounds_get_night_text`).

Card and value chips: the card is a rect behind the plot (`role: card`, inset 1 px, corners 12 px,
14 in Bold, square when `rounded` is off; light fill `#F7F6F3` with a `#E9E8E3` hairline, on a
dark background the background mixed 6 % with white and an 8 % edge), drawn when the visual is
at least 24 px each way; it becomes the surface. A value chip is a rounded rect of the label ink
(`role: pill`, text width + 10 by size + 6) under the value, whose text takes the ink that reads
on it. High contrast draws the card and the chips in the background colour with a foreground
edge (Power BI) or leaves them out (the renderer).

Text sizes (Minimal / Modern / Bold): axis ticks 10 / 11 / 11, names and legends 11 / 12 / 12,
value labels 10 / 11 / 12; value labels sit further from their mark as they grow and, outside
Minimal, stay inside the visual's top.

Tokens (Modern and Bold / Minimal): axis and category text `#5F5E5A` / `#555555` (6.5:1 and
7.5:1 on white), values and names `#1F1E1C` / `#333333`, gridlines `#E8E7E2` / `#E5E5E5`, axis
lines `#C8C7C0` / `#B3B3B3`, empty cells `#F0EFEB` / `#EFEFEF`, bullet bands five warm greys
from `#C4C3BC` / five greys from `#9E9E9E`. Text inside a fill is white or the dark ink
(`#000000` / `#252423`), whichever contrasts more with both ends of the fill's gradient, so it
is always at least 4.5:1. Identity is never colour alone: each series keeps its legend or direct
label, and every value keeps its text.

## Marks

What a primitive may carry besides its plain drawing (written in the golden form only when set):

| Key | On | Meaning |
|---|---|---|
| `rx` | rect | corner radius |
| `sq` | rect (with `rx`) | the side left square: `b`, `t`, `l`, `r` (a bar's baseline) |
| `grad` | rect, circle | the top colour of a vertical gradient down to `fill` |
| `stroke`, `sw` | rect, circle | an edge over the fill and its width |
| `shadow` | rect, circle | a soft shadow's size in pixels |
| `weight` | text | font weight (written when not 400) |
| `role` | rect | `card` (the card behind the plot) or `pill` (a value chip) |

| | Minimal | Modern | Bold |
|---|---|---|---|
| bar and box corner radius | 0 | 4 | 6 |
| gradient top | none | fill mixed 22 % with white | 32 % |
| bars | flat | rounded except the baseline side, gradient | also an edge (fill mixed 22 % with black) and a 3 px shadow |
| value labels | 400 | 600 | 700 |
| dumbbell stem / slope line | 2 / 2 px lines | 4 / 2.5 px round-capped paths | 5 / 3 px |
| dots | plain | a 1.5 px surface ring | the same |
| org chart nodes | solid boxes, white text | tinted cards (fill mixed 88 % with white), an accent strip, a 6 px shadow | solid shaded cards, an 8 px shadow |
| Venn sets | translucent discs | gradient, edge in the set's colour, a 4 px shadow | 5 px shadow |
| Marimekko | 1 px column gaps | 2 px surface gaps between columns and segments, 3 px corners | the same |
| calendar | the earlier blue ramp and layout | the palette's first colour and its 86 % tint (of the surface on a dark background); cells sized to fill the visual (up to 48 px, rounded), month labels and separators, weekday names, a five-step legend ([chart-pack.md](chart-pack.md)) | the same |
| Sankey nodes | flat bars, 11 px labels | rounded bars (all four corners), gradient, 12 px labels | also the bar's edge and shadow; 13 px node labels at 600 |
| org chart colours | one colour | without a node colour set: the root in the first palette colour, each top-level branch (and its subtree) in the next ones | the same |
| Spec combo lines | 2 px | 2.5 px with a ringed 3.5 px marker on each point (up to 40) | the same |

The Spec visual's bars use the bar mark, its `rect` marks the tile (half the radius), its points
the dot; legends use rounded swatches.

## Drawing

- BIcraft's renderer (`crates/render/src/spec.rs` `decorated`, PNG and PDF): the shadow as three
  widening translucent copies below the shape, then the fill (a tiny-skia linear gradient; in
  PDF an axial shading, `ShadingType 2`, clipped to the shape), then the edge. High contrast
  draws neither gradient nor shadow. The renderer draws the final frame of the animation.
- In Power BI (`visual_runtime/paint.ts`): `rx`/`ry` or a path for a square side, a
  `linearGradient` and an `feDropShadow` filter in the SVG's `defs` (ids unique to the visual),
  `stroke`, `font-weight`. High contrast (`contrastPrim`) drops gradients and shadows and draws
  edges in the background colour.
- Typography: a system font stack (`system-ui, -apple-system, "Segoe UI", Roboto, …`, named
  only; no font file is bundled).

## Motion

When `animation` is on and the update brings data (the first update or one of type Data), the
marks enter in 280 ms (bars grow from their baseline, dots from their centre, lines and text
fade in). Hovering a data point dims the other points to 40 % (120 ms). Neither plays with
`prefers-reduced-motion: reduce` or in high contrast.

## Seen in Desktop

Power BI Desktop 2.159.1231.0 (Store edition, French UI, 2026-11-24), from a PBIX `bicraft` wrote
with the ten visuals on three pages (Modern, Bold and Minimal through each visual's `style`
options and the Spec visual's spec) under a custom report theme of purple and orange data
colours set by `theme.set_colors`. Driven by input events through the WebView2 DevTools port; each
visual's sandbox frame DOM and screenshots were read.

- **Drawing and colours.** On each page the ten draw the marks of `bicraft render`: rounded
  gradient bars, the Bold edges and shadows (the frames' `stroke` and `filter`), Minimal's earlier
  flat look and blue calendar ramp; the marks take the theme's data colours from
  `host.colorPalette`, as the renderer takes them from the report theme.
- **Titles under a theme without a base theme.** Desktop drew the default titles in the classic
  style (light grey, regular) where `report.render` drew them dark and bold: a custom theme
  written without a base theme keeps the classic look under its own colours. Fixed: the resolved
  theme carries `classic` when it has no base theme, and the renderer then takes the classic
  title and visual defaults (`a_custom_theme_without_a_base_theme_keeps_the_classic_look`,
  `a_custom_theme_without_a_base_theme_draws_classic_titles`). A theme with a base theme is
  unchanged.
- **Motion.** The marks carry the entry animation's classes and an animation runs when a
  cross-filter brings new data; hovering a Spec bar dims the other points. When the pointer left
  the frame through the DevTools port the dimming stayed until another point was hovered (the
  frame got no leave event that way; not seen with a real mouse).
- **Style card.** The card lists its options in French; the preset drop-down shows Moderne and
  Affirmé among its items, the palette drop-down Couleurs BIcraft; picking Affirmé on the
  histogram redraws it with edges and shadow, and Desktop's save writes `style.preset` as `bold`,
  which `bicraft render` draws the same. The Spec visual had no Style card in that run (its `style`
  was in its spec; it has one since 1.3.0.0, below).

The second pass (chart visuals and pack 1.2.0.0 and 1.3.0.0, Spec 1.3.0.0 and 1.4.0.0) in the
same Desktop later that day, from PBIX files `bicraft` wrote with the ten under Modern and Bold on
a light report and under a dark custom theme (`theme.set_colors` with background `#1F1F1F`, no
base theme):

- **Card and value chips.** On the light pages the ten draw as `bicraft render` draws them: the
  card behind the plot under Modern and Bold, the Bold value chips (dumbbell ends, Venn counts),
  the Sankey's bold labels (weight 600).
- **Spec visual's Style card.** Its format pane has the Style card in French (Apparence,
  Préréglage with Affirmé, Couleurs with Thème du rapport, Coins arrondis, Dégradé, Ombre,
  Animation, Fond en carte, Pastilles de valeur). Turning Fond en carte off removed the card;
  Desktop's save is drawn the same by `bicraft render` (the Spec visual without its card). The
  combo template's legend reads "First measure" and "Second measure" in both, the template's
  own field names.
- **Dark theme.** Inside their frames the ten draw the dark-surface tokens as `bicraft render`
  does (dark card, light text, grid and axes). Desktop kept the page canvas and each visual's
  container and title area white, the titles grey, where `bicraft render` paints the page and
  every container in the theme's `background`: a difference in the built-in renderer's page
  and container backgrounds, left (G114).

Not checked: high contrast, `prefers-reduced-motion`.
