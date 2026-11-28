# Render oracle

Compares `bicraft-cli render` with Power BI Desktop's own rendering, per visual, on synthetic
reports covering each visual family (and, locally only, on real reports). Method, metrics and
limits: `docs/render.md`.

```powershell
$w = "$env:TEMP\oracle-reports\render"   # any work folder outside the repository
cargo xtask oracle-gen synth $w\synth.pbix $w\cmds.json $w\layout.json
bicraft-cli run --file $w\cmds.json
Test-Path $w\synth.pbix    # capture.ps1 refuses a missing file, but check before taking the lock
bicraft-cli render $w\synth.pbix --out $w\ours --lang fr-FR --text-json
if (-not $env:BICRAFT_LOCKS_DIR) { $env:BICRAFT_LOCKS_DIR = "$env:LOCALAPPDATA\bicraft\locks" }
mkdir $env:BICRAFT_LOCKS_DIR\desktop  # one Desktop at a time; wait while the priority lock exists
.\capture.ps1 -File $w\synth.pbix -Out $w\desktop -Pages Cards,Columns,Lines,Parts,Tables,Stress,Labels,Combo,Axes
rmdir $env:BICRAFT_LOCKS_DIR\desktop
cargo xtask render-compare $w\desktop $w\ours $w\layout.json --diff $w\diff
```

`capture.ps1` needs no screen: Desktop's report view is a WebView2 page, opened with its DevTools
port (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`, set for that process only) and sRGB colour output;
`capture.mjs` (Node 22+) clicks each page tab and captures the report canvas, and writes
`NN-<page>.json` beside each PNG: every text node's computed font family, size, weight, colour,
box, `scale` (screen pixels per CSS pixel of its font size) and `vis` (false for visually hidden
accessibility text). Pass several pages as one PowerShell array (`-Command "& .\capture.ps1 …
-Pages A,B"`); with `powershell -File` the list arrives as one string. `capture.ps1` refuses a
missing or empty report file and closes the Desktop it started when the window never appears.

`cargo xtask render-compare DESKTOP_DIR OURS_DIR LAYOUT.json` prints a per-visual table (sim, ink,
SSIM, ΔE00, matched texts with the missing / extra / different ones) and a per-family summary;
`--json` prints the same as JSON, `--diff DIR` writes Desktop | BIcraft | difference crops,
`--min-sim X` and `--floors FILE` return non-zero below the thresholds. The text check runs when
both folders have the `NN-<page>.json` files (`bicraft-cli render --text-json` writes ours).

`cargo xtask oracle-gen uilang OUT.pbix CMDS.json LAYOUT.json [CULTURE]` writes the UI-language report
(M10.2: auto titles, aggregation names, (Blank), Total, empty visuals, formats; pages
Titles,Aggs,Blanks,Empty,Formats); run it for en-US and fr-FR and capture both under one Desktop
UI language (`docs/view-snapshot.md`, "UI language").

The generators are `cargo xtask oracle-gen <set>` (xtask/src/oracles; each file's header has its
steps). They only write `bicraft-cli run` command and layout JSON:

- `synth OUT.pbix CMDS.json LAYOUT.json`: the report above, one page per visual family.
- `grids OUT.pbix CMDS.json LAYOUT.json`: the tables and matrices report (scroll thumbs at
  several row counts and heights, grids wider than the visual, matrix `subTotals` options), in
  one `bicraft-cli run` like `synth`.
- `expand DIR` / `kpis DIR` (two runs, as `parts`): expanded matrices (the `expansionStates`,
  `active` flags and per-level `subTotals` Desktop saves; pages ExpandA,ExpandB,ExpandC) and
  KPI display units per format string and magnitude (pages KpiA,KpiB,KpiC,KpiD); render both
  with `{"ui_language": "fr-FR"}`. The patch step also writes `$visual` keys (`expansionStates`)
  and `$active` / `$activeAt` projection flags.
- `parts DIR` / `slicers DIR`: the pie / donut / gauge / card / KPI / funnel / treemap report and
  the slicer report (one page per slicer mode family). Two runs: `bicraft-cli run --file
  DIR\cmds1.json` writes a project, `cargo xtask oracle-gen parts DIR patch` writes the visual
  objects into its PBIR (new-card objects in the `default` state), `bicraft-cli run --file
  DIR\cmds2.json` writes the report.
- `states DIR`, then `states DIR check [CAPTURE_DIR]`: whether Desktop applies the formatting of
  stateful objects (docs/formats/visual-options.md, "State selectors").

They write the same JSON as the Python scripts they replace (LF line ends), so the local sets
below still match. All image and text comparison is Rust too.

## Local references and the oracle test

Desktop's captures are Microsoft-rendered images: they are never committed. Keep them under
`%BICRAFT_TESTDATA%\render-oracle\<set>\` with `report.pbix` (the generated report),
`layout.json`, `desktop\` (the capture) and `render.json` (extra `report.render` parameters, e.g.
`{"ui_language": "fr-FR"}`). Sets: `synth`, `parts`, `slicers`, `grids`, `expand` and `kpis` (the
`oracle-gen` sets of the same names; the family names of `grids` are the visual keys). `cargo test -p xtask oracle` renders each set with the current renderer and
checks it against `floors\<set>.json` (per-family means: `sim`, `ink`, `ssim`, `text` floors and a
`de` ceiling, set a little below the scores of 2026-10-14); it skips with a note when the
references or Segoe UI are missing. After a deliberate change, regenerate the floors from
`render-compare --json` (`families`).
