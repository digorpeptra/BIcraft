# Web viewer regression checks (`cargo xtask webcheck`)

ROADMAP M11.27. A **local development aid**, run on demand like `cargo xtask heavy`; it is not
part of `cargo xtask ci`, no test needs it, and nothing it produces is committed.

```text
cargo xtask webcheck record     # once, and after an intended change to the viewer's look
cargo xtask webcheck check      # after a change: compare with the baselines
    [--dir DIR] [--bicraft PATH] [--only WALK] [--tolerance N] [--max-diff F]
```

## What it does

1. Builds `bicraft` (`cargo build -p bicraft-cli`, debug; `--bicraft PATH` takes a binary
   instead).
2. Copies our synthetic fixture `crates/engine/tests/fixtures/layout-oracle/visuals/legacy` (three
   pages: Charts, Tables with two slicers, Shapes; its `SOURCE.txt` says how it was made) to
   `<dir>/work/` and adds a bookmark to it with `bicraft run --cmd bookmark.capture … --save`
   (saved as PBIR). No client file is ever used.
3. Starts `bicraft view <dir>/work/visuals.pbip --port 0 --no-open-button --lang en
   --as-of 2026-01-01`: a free loopback port, read from its "live viewer at" line, and no
   "Open in Power BI Desktop" button.
4. Starts Craft Anything's MCP server (`craft-anything-cli --allow-run --clean-room mcp`) and,
   for each walkthrough, resets the reader state, opens the HTML viewer in Craft Anything's
   private headless browser (`web_site_open`), and takes a screenshot after each step
   (`web_page_screenshot_take`). Nothing opens on the screen.
5. `record` keeps the screenshots as the baselines (`<dir>/baseline/<walk>/<step>.png`);
   `check` writes the new ones under `<dir>/latest/`, compares each with its baseline
   (`web_pixels_compare`: per-channel tolerance, share of changed pixels, changed box), writes a
   diff image for each failing step (`<dir>/diff/<walk>-<step>.png`: the baseline in grey, changed
   pixels in red) and prints a table. It exits non-zero when a step fails.

The walkthroughs (`xtask/src/webcheck.rs`, `walkthroughs()`):

| Walk | Steps |
|---|---|
| `open` | the report as opened (Charts page) |
| `pages` | page tabs: Tables, Shapes, back to Charts |
| `slicer` | Tables, then the Region slicer: East, then East and West (Ctrl) |
| `point` | a data point of the clustered column chart (East) selected, then the selection cleared |
| `drill` | the stacked column chart's date hierarchy: next level, then drill up |
| `bookmark` | Tables, then the bookmark captured on Charts applied |
| `states` | Craft Anything clicks the page's own tabs (`web_page_states_capture`, up to 4): the Model and Validation tabs and the page tabs, each put back after its screenshot |

The gestures of `pages` to `bookmark` are the reader actions the page's clicks send
(`POST /api/interact` with the page's token, `Origin` and `Sec-Fetch-Site`; the actions of
docs/view-snapshot.md): the screenshots check how the HTML viewer draws each state it is sent.
Craft Anything's scripted clicks (`web_walkthrough_record`) read the page, which its restricted mode
does not allow, and `--deep` is not used, so the page's own click handlers are
exercised only by `states`. `web_walkthrough_check` and `web_snapshots_compare` are therefore not
used either (the second also needs Deep mode). The HTML viewer has no export in `bicraft view`
(the download link is `bicraft serve`'s), so there is no export walkthrough.

## Running it

- **Craft Anything** must be installed: `$BICRAFT_CRAFT_ANYTHING` names the executable, else
  `craft-anything-cli` is looked up on `PATH`, then in its default install folder under
  `%ProgramFiles%`. Without it the command prints a note and succeeds, as the other local oracles
  do. It drives a Chromium-based browser it finds on the machine (Edge on Windows), headless.
- **The folder** is `--dir`, else `$BICRAFT_WEBCHECK_DIR`, else `bicraft/webcheck` in the local
  data folder (`%LOCALAPPDATA%` on Windows, `$XDG_DATA_HOME` or `~/.local/share` elsewhere). A
  folder inside the repository is refused.
- **Baselines belong to one machine.** The page follows the browser's language and colour
  scheme, which come from the system (a French dark Windows draws the viewer's French labels in
  dark), and the browser window's size; record and check on the same machine and build
  environment. Record again after an intended change to the viewer's look.
- **Thresholds.** A pixel counts as changed when a channel differs by more than `--tolerance`
  (default 16 of 255); a step passes when at most `--max-diff` of its pixels changed (default
  0.0005, 0.05 %). Each step waits 3.5 s for the snapshot and the "changed" highlight (2.4 s) to
  settle; the first load waits 5 s.
- A `check` takes about three minutes, `bicraft` already built (measured 2026-11-19, Windows,
  debug `bicraft`): mostly the waits after each step. The screenshots are the browser's own
  window, 750 x 485 pixels on the proof machine (the 1280 x 800 asked of `web_site_open` is not
  what it captures).

## Proof run (2026-11-19, Windows, Craft Anything 0.6.1, Edge 157 headless)

1. `record`: 19 screenshots (7 walkthroughs; `web_page_states_capture` opened one tab, Model).
2. `check` on the same build: 19 of 19 pass, 0.000 % of pixels changed in every step.
3. The dark scheme's `--accent` colour token of the HTML viewer's CSS (`crates/view/src/page.html`)
   changed on purpose: `check` fails 18 of 19 steps, each with 0.135 % of pixels changed in the
   chrome (the active tab's underline, the page tab's dot, the revision and display-mode pills;
   box 12,13 627x87); the Model tab's screenshot does not use that colour and passes.
4. The token restored: `check` is green again, 19 of 19.

## Left

docs/gaps.md G74: the egui app (`/app/`, a canvas) is not covered (it needs the wasm bundle
embedded, and its controls are not DOM elements Craft Anything can click); the scripted clicks of
`web_walkthrough_record` are unavailable in its restricted mode; no export walkthrough; no Linux run.
