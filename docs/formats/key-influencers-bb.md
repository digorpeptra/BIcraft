# Key influencers visual (`keyDriversVisual`): black-box observations

Sources: Microsoft's public report theme schema (`reportThemeSchema-2.157.json`,
`visualStyles.keyDriversVisual.keyDrivers` names the options), and one black-box run on
2026-11-04: Power BI Desktop 2.159.883.0 (Store edition, French UI) opened a PBIX that BIcraft
wrote from a synthetic table (1 200 generated rows). Its report canvas was captured with
`tools/render-oracle/capture.ps1`. Only the visual's appearance and the reading of the saved
options were taken from it. Desktop's results were not compared with BIcraft's estimate, and
nothing about its algorithm was inferred. Desktop showed its French texts; the English names
quoted below (tabs, question, goals, "When...", the check box) are those of Microsoft Learn's page
*Key Influencers Visualizations Tutorial*
(<https://learn.microsoft.com/power-bi/visuals/power-bi-visualization-influencers>, 2026-08-24),
and the texts Learn does not give are described in our own words. BIcraft's renderer draws its
own wording for these texts (`crates/render/src/keyinf.rs`).

| Saved in `objects.keyDrivers` | What Desktop showed |
|---|---|
| nothing (numeric target) | the goal "Increase" |
| `numericTargetSelectedKind` = `1D` | the goal "Decrease" |
| nothing (text target with the values No / Yes) | the class "No", the first value in order |
| `targetValue` = `'Yes'`, `selectedAnalysis` = `'topProfiles'` | the "Top segments" tab active (a progress message while it computes) |

Layout, in report pixels inside a 1240 × 330 visual with no title (Desktop draws none by
default):

- Tabs "Key influencers" and "Top segments" at y ≈ 25, text about 14.5 px. The active tab is
  dark and semibold, underlined 2 px in the accent at y ≈ 40.
- The question line at y ≈ 69, about 13 px, naming the target (a text target with its
  class), followed by a white 150 px dropdown with a chevron and a help mark.
- A light grey panel (≈ #F3F3F3) from y ≈ 97 to the bottom, the full width less about 6 px.
- On the left of the panel: "When..." at the top left, and the wrapped right-hand header
  "...the average of <target> increases by" at about 39 % of the width. Rows are about 64 px
  apart, the first centred about 81 px below the panel's top. The selected row has a 4 px
  accent bar and semibold text. A grey line with an arrow leads to a bubble of radius ≈ 26:
  the accent with white text when selected, light grey with dark text otherwise. Bubbles sit
  further right for larger values, up to about 49 % of the width.
- A white card with a light shadow, from about 54.6 % of the width to 16 px from the right
  edge, 11 px under the panel's top and 12 px above its bottom. It holds a back arrow and a
  sentence saying that the target tends to rise when the field has that value, on average.
  Below that, a column chart: dark slate columns (≈ #374649), the selected value in the accent,
  y ticks at round steps, x labels and the field as axis title. A red dashed reference line is
  labelled, in the same red, with the average of the values other than the selected one. A check box "Only show
  values that are influencers" sits at the bottom.
- Thumbs-up and thumbs-down icons at the top right (BIcraft draws none).
