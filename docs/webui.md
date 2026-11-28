# One UI for the desktop and the browser (`crates/ui`, `apps/bicraft-web`)

Decision of 2026-10-29: the web viewer and the desktop app must not differ. There is **one UI**:
the egui app of [`crates/ui`](../crates/ui) (`bicraft_ui::App`). It runs natively in
[`bicraft`](desktop.md), and compiled to WebAssembly in the browser, served by
`bicraft-cli view` and `bicraft-cli serve`. The HTML viewer (`crates/view/src/page.html`) stays the
default until the egui path has every one of its features (the checklist below); then it is
removed.

This page covers the toolchain, the two shells, serving, and the parity checklist.
The look (tokens, fonts, chrome, icons, widgets, light and dark) is specified in [design.md](design.md).

## Architecture: one UI crate, two shells

```text
                 crates/ui: bicraft_ui::App (egui) ── pages: bicraft-render (tiny-skia) + hit map
                  │  logic(ctx) / ui(ui) · fonts · Report / Model / Validation · i18n · prefs
                  │  Shell { caps, start_source, pick_file, prefs, set_title, server_token }
                  │  Worker ── source::Request / source::Update ──┐
   ┌──────────────┴─────────────┐                     ┌───────────┴───────────────┐
   │ native shell               │                     │ browser shell             │
   │ apps/bicraft-desktop       │                     │ apps/bicraft-web          │
   │  Caps::DESKTOP             │                     │  Caps::BROWSER            │
   │  eframe::run_native, wgpu  │                     │  eframe::WebRunner, wgpu  │
   │  source::spawn threads:    │                     │  (WebGPU, else WebGL2)    │
   │   local::run  (engine)     │                     │  Running::Driven source   │
   │   remote::run (ureq)       │                     │   fetch + EventSource     │
   │  rfd dialog, desktop.json  │                     │  document.title           │
   └────────────────────────────┘                     └───────────────────────────┘
                                                        same-origin viewer API
```

- **`crates/ui` (`bicraft-ui`, layer L6)**: `app.rs` (the window), `pages.rs`, `model_view.rs`,
  `diagram.rs`, `i18n.rs`, `fonts.rs`, `prefs.rs` (the values; the store is the shell's),
  `shell.rs`, `source.rs`, `remote.rs` (the pure half of the server client: URLs, `classify`,
  `message`, `asset_ids`, `Failure`). No engine, file, socket or thread of its own: it builds for
  wasm32. Its tests may use the engine to build snapshots (`cargo xtask layers` allows the engine
  as its dev-dependency only).
- The app, its rendering (`bicraft-render` at the canvas size and DPI), its hit map and every
  `view.interact` action are the same code in both shells. The browser draws the page with the
  same renderer as `bicraft-cli render` and the desktop, so a page looks the same everywhere.
- **The shell** (`bicraft_ui::shell::Shell`) is all a front end supplies: its capabilities
  (`Caps`: open files, connect to a server, quit; open-in-Desktop and download for phase 2), how
  a data source starts, the Open dialog, where preferences are kept, how the title is set, and
  the token Connect to server starts with. The app hides what a shell cannot do: in the browser
  the Home ribbon's File group has Reload only (no Open, recent files, Connect or Quit, dropped
  files ignored; Ctrl+O opens the File view's Open), since the file and the server are the page's.
- **Data sources.** Natively, `source::spawn` runs `local::run` (an engine session) or
  `remote::run` (the blocking HTTPS client) on a thread. In the browser there are no threads and
  no blocking sockets: the shell's sources are *driven* (`Running::Driven`): each request goes to
  the driver as the window sends it, and it answers through the same `Link`, as futures on the
  page's event loop. The page renderer falls back to the window's thread when it cannot start one
  (the browser), with the bundled fonts.
- **Title.** Both shells show the viewer's tab title, `● file · rev N` (`●` with unsaved edits;
  the origin's name until the first snapshot): the window title natively, `document.title` in the
  browser.
- **Fonts.** The same in both shells, installed by the app on its first frame
  (`crates/ui/src/fonts.rs`): the four Selawik faces `bicraft-render` bundles (SIL OFL 1.1,
  metric-compatible with Segoe UI, the viewer's font; one copy in the binary, shared with the
  renderer) as egui's proportional family and as `semibold`, `bold` and `light` families;
  headings and card headers are semibold. Hack (egui's monospace face) draws DAX, badges and
  format strings, as the viewer's `ui-monospace`. The glyphs Selawik lacks come from the faces
  egui already embeds (Ubuntu Light, Hack, Noto Emoji under OFL, the emoji icon font; listed in
  `THIRD_PARTY_CRATES.md`): `→ ▶ ● Σ ✔ ✖ ⚠` render. No font in the build has `✓` (U+2713) or `✕`
  (U+2715), so the app writes `✔` and `×` (close) instead; a test reads the faces' cmaps and
  checks that every non-ASCII character in the UI's strings has a glyph. No system font is read
  for the chrome.
- **`apps/bicraft-web`**: `protocol.rs` (pure, tested natively: the page's configuration, URLs
  relative to the page, reading answers, refetch coalescing, back-off) and `web.rs` (wasm32
  only: the eframe web runner, the browser shell, `fetch`, `EventSource`, `document.title`).
  `cargo xtask webui check` (part of `cargo xtask ci`) builds it, and so `bicraft-ui`, for wasm32.

## The browser data source

The app page (`/app/` on `bicraft-cli view`, `/w/<id>/view/app/` on `bicraft-cli serve`) carries the
same `<meta>` slots as the HTML page: `bicraft-interact-token`, `bicraft-open-token`
(`bicraft-cli view` only), `bicraft-view-id` (`bicraft-cli serve` only). The API root is the page's
path without its last `app/` segment:

```text
GET  {root}api/state[?v=VIEW]          the snapshot (on each change; once per burst of events)
GET  {root}api/asset/ID[?v=VIEW]       its images (kept across snapshots by id)
GET  {root}api/events[?v=VIEW]         EventSource: `event: snapshot`; an inline snapshot
                                       (bicraft-cli view, up to 1 MiB) is used without a fetch
POST {root}api/interact[?v=VIEW]       view.interact, X-Bicraft-Token from the page
```

- **Credentials** are the browser's own: the viewer's login cookie or the auth proxy's session
  (OIDC through oauth2-proxy, [auth.md](auth.md)), sent with same-origin requests. No bearer
  token is handled by the app; the interaction token stays in the page's `<meta>` and in memory,
  as with the HTML page. `POST …/api/session` (bearer only) is for native clients and is not used
  here: the page load mints the view, with the same checks as the HTML page.
- **Browser storage** (decision of 2026-11-04): the app keeps **UI preferences only** in
  `localStorage`, as page.html keeps its tab and display mode: the theme (light/dark), the fit
  mode, the ribbon collapsed or not, the remembered tab, the palette's recent commands (ids
  only), the Report view's panes open or collapsed and their widths. Never a token, a path, a server URL,
  report data or anything from the snapshot; nothing in `sessionStorage` or IndexedDB. The store
  is the web shell's `PrefsStore`; every access is wrapped, so storage that is blocked, absent or
  full, or a damaged value, gives the defaults and the app keeps working ([design.md](design.md) §8).
- **Errors** use the desktop's wording (`remote::Failure`): 401, 403 with the server's reason,
  a 404 on the view ("the server forgot this view (a restart?): reload the page"), network
  errors. The event stream reconnects after 0.5 s doubling to 15 s (as the HTML page) and the
  snapshot is fetched again on reconnection.
- **Role**: on a server, the snapshot's `view.role` (`reader` or `editor`) is shown in the status
  bar, as the desktop shows the role of `POST …/api/session`.

## Serving

| Route | `bicraft-cli view` | `bicraft-cli serve` |
|---|---|---|
| app page | `/app/` (`/app` → 308) | `/w/<id>/view/app/` (new private view, as `…/view/`) |
| files | `/app/boot.js`, `/app/bicraft_web.js`, `/app/bicraft_web_bg.wasm` | same under `/w/<id>/view/app/` |
| default UI | `/` (HTML) unless `--ui egui` | `/w/<id>/view/` (HTML) |

- `bicraft-cli view FILE --ui egui` prints (and with `--open` opens) the `/app/` URL. The HTML page
  stays at `/`.
- **Same checks as the HTML page**: `Host` (DNS rebinding), authentication, the workspace ACL,
  GET only; interactions keep the token, `Origin` and `Sec-Fetch-Site` checks.
- **Headers.** The page: `Cache-Control: no-store` and its own CSP
  (`bicraft_view::webui::APP_CONTENT_SECURITY_POLICY`: `script-src 'self' 'wasm-unsafe-eval'`, no
  inline script, `connect-src 'self'`, `frame-ancestors 'none'`). The files: gzip-compressed when
  the client accepts it (stored compressed in the binary, inflated for clients that do not),
  `ETag` + `Cache-Control: no-cache` (a reload costs a 304), `Cross-Origin-Resource-Policy:
  same-origin`, `nosniff`.

## Building and embedding

Rust tools only (no npm, no Node):

```text
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked --root <TOOLS>   # the version in Cargo.lock
set BICRAFT_WEBUI_TOOLS=<TOOLS>                                             # or put it on PATH
cargo xtask webui [--out DIR] [--wasm-opt]
set BICRAFT_WEBUI_DIR=<DIR>          # printed by the command: <target>/webui by default
cargo build --release -p bicraft-cli
```

- `cargo xtask webui` builds `bicraft-web` for wasm32 with the `webui` profile (release, fat
  LTO), runs wasm-bindgen (`--target web`), optionally `wasm-opt -Oz` (binaryen, when installed),
  and prints the sizes against the budget (8 MB gzip).
- The wasm-bindgen CLI must be exactly the version of the `wasm-bindgen` crate in Cargo.lock
  (`apps/bicraft-web/Cargo.toml` pins it with `=`); xtask checks it and prints the install line.
  `BICRAFT_WASM_BINDGEN` may name the executable.
- **Embedding: opt-in for development, always for releases.** `crates/view/build.rs` embeds the
  two files when `BICRAFT_WEBUI_DIR` (an absolute path) is set, gzip-compressed with their entity
  tags. Without it, `cargo build` and `cargo xtask ci` need no wasm tools, and the app routes
  answer 404 with "This bicraft was built without the browser app…". `BICRAFT_REQUIRE_WEBUI=1`
  makes a missing bundle a build error. `bicraft-cli build-info` (JSON) says whether a binary embeds
  it (`webui.embedded`).
- **Release binaries always embed it**: `cargo xtask dist` builds the bundle once (or takes
  `--webui DIR`), passes it to the Windows build and to the WSL Linux build, builds with
  `BICRAFT_REQUIRE_WEBUI=1` and refuses a binary whose `build-info` says otherwise; the release
  workflow does the same per target; the container image (`deploy/docker/Dockerfile`) installs
  the wasm32 target and the wasm-bindgen CLI (`ARG WASM_BINDGEN_VERSION`), runs
  `cargo xtask webui`, builds with `BICRAFT_REQUIRE_WEBUI=1`, and checks `bicraft-cli build-info` on
  the binary it ships, prebuilt ones included. `cargo xtask deploy check` fails when the Dockerfile
  lacks any of these or pins another wasm-bindgen than Cargo.lock, and
  `cargo xtask deploy check --binary PATH` fails on a binary without the app.
- **MSRV.** egui/eframe 0.36 and 0.37 need Rust 1.95, which is the workspace's `rust-version` since
  2026-11-24 (`bicraft-ui`, `bicraft-desktop` and `bicraft-web` inherit it); the CI MSRV job
  builds and tests the whole workspace on it.
- **CI.** `cargo xtask ci` runs `cargo xtask webui check` (a wasm32 `cargo check` of the app,
  skipped with a note when the target is not installed). The bundle itself (wasm-bindgen CLI) is
  built by `cargo xtask webui`, not by CI.

## Size

Measured on Windows, 2026-10-29, `cargo xtask webui` (profile `webui`, no wasm-opt):

| File | Size | gzip (served) |
|---|---|---|
| `bicraft_web_bg.wasm` | 10.77 MB | 3.93 MB |
| `bicraft_web.js` | 0.15 MB | 0.02 MB |
| total | 10.93 MB | 3.95 MB (budget 8 MB) |

A browser downloads it once; later loads revalidate (304). The bulk is wgpu/naga, egui and
`bicraft-render` (with its fonts).

## Tests and checks

- `apps/bicraft-web/src/protocol.rs`: the page's configuration (bad tokens ignored), the API
  root and URLs for `bicraft-cli view` and `bicraft-cli serve`, answers (state, interactions accepted,
  refused, forbidden, malformed), inline snapshots, the role, refetch coalescing, back-off.
- `crates/view/src/webui.rs`: the page's slots (only 32-hex values), the CSP, gzip negotiation,
  the files (gzip, inflated, 304 on the entity tag, unknown names).
- `crates/view/tests/server.rs` (`serves_the_browser_app`) and `crates/mcp/tests/http_viewer.rs`
  (`the_browser_app_gets_its_own_view`): the routes of both servers with and without an embedded
  bundle (run the tests with `BICRAFT_WEBUI_DIR` set to cover the embedded case): redirects,
  authentication, ACL, the private view of the app page working with the HTML page's requests,
  the files, gzip, 304.
- `crates/ui/src/tests.rs` (no GPU, part of `cargo xtask ci`):
  `the_desktop_and_the_browser_draw_the_same_frames` builds the app with `Caps::DESKTOP` and
  `Caps::BROWSER` on one snapshot (a report with a slicer, a card and a chart, and its model) and
  compares, for the Report, Model and Validation tabs, in English and French, light and dark,
  everything the frame sends to the GPU: the tessellated meshes (positions, texture coordinates,
  colours, clip rectangles), the font atlas and the page image; equal inputs draw equal pixels.
  Also: the fonts are Selawik with semibold headings, every non-ASCII character of the UI's
  strings has a glyph, the Home ribbon per shell, the title, the empty Report tab of a model-only
  snapshot; light by default and the dark toggle kept (a blocked store too), the chrome's pills,
  `✓`/`✕` from the symbol face, the pages' bundled fonts on the render thread.
- The desktop's own tests cover the app through the native shell (clicks give the viewer's
  results, pages equal `bicraft-cli render`, a `.bim` without an error bar): the same code runs in
  the browser.
- **Browser check, 2026-11-05, parallel document tabs** (not part of CI): headless Firefox
  159.0.1, throwaway profile, WebDriver BiDi (pointer actions inside the page), on `bicraft-cli serve`
  (loopback, no authentication, `--allow-origin` its own origin) with a workspace holding the
  `visuals.pbip` fixture (opened in the workspace's session over MCP first, as an agent would) and
  `reports/model.bim`, at `/w/ws1/view/app/`: Open > Browse opened `reports/model.bim` and then
  `visuals.pbip` in two more tabs, each through `POST …/view/api/session` (two document views,
  read with their own `?v=`); New page in the third tab ran `page.add` at
  `/w/ws1/mcp/view/<its view>` and gave it a fourth page and the unsaved dot, while the first tab
  (the workspace's session, same file) kept its three pages and its revision, and the
  `model.bim` tab its "no report" state, switching freely with unsaved edits. With
  `--max-views-per-user 1` and a view of the same identity holding unsaved edits, opening a
  file closed the new tab and the tab strip showed the server's 429 message
  (`⚠ too many open files: at most 1 per user on this server; close a tab first`).
- **Browser check, 2026-11-04, File view and tabs** (not part of CI): headless Firefox 159.0.1,
  throwaway profile, WebDriver BiDi (pointer actions inside the page), on `bicraft-cli serve` (loopback,
  no authentication, `--allow-origin` its own origin) with a workspace holding the `visuals.pbip`
  fixture and `crates/engine/tests/dax_queries/model.bim`, at `/w/ws1/view/app/`, light and dark:
  the page tabs (with the `⚠ 2` badge of the Shapes page) and their menu, File > Home, Open >
  Browse listing the workspace's two Power BI files (no Servers place), Info, Export; a click on
  `reports/model.bim` opened it in a second document tab through the workspace's MCP endpoint
  (`file.open`, the status bar's `✓ file.open`), and `localStorage` kept only
  `{theme, zoom, ribbon, tab, recent: [{path: "reports/model.bim", ws: "ws1", opened, pinned}]}`.
  The same screens match the desktop's `backstage_screenshots`.
- **Browser check, 2026-11-04, design system** (not part of CI): headless Firefox 159.0.1, throwaway
  profile, WebDriver BiDi, on `bicraft-cli view …/visuals.pbip --ui egui --lang fr` and on
  `crates/engine/tests/dax_queries/model.bim`: for light and dark and each tab, the preference
  `{"theme", "tab", "zoom", "ribbon"}` written to `localStorage` (`bicraft.ui`, the only key)
  and the page reloaded; the window starts in that theme and tab, and matches the desktop's
  screenshots of `side_by_side_with_the_browser` (same chrome, same page pixels).
- **Browser check, 2026-11-04** (not part of CI, after phase 2 step 1): headless Firefox
  159.0.1, throwaway profile, WebDriver BiDi, on `bicraft-cli view …/visuals.pbip --ui egui` and on
  `crates/engine/tests/dax_queries/model.bim`: the chrome draws in Selawik exactly as the desktop
  (`side_by_side_with_the_browser` in `apps/bicraft-desktop/src/tests/shots.rs` takes the
  desktop's at the same size; the text inside visuals follows the session's language, so start
  `bicraft-cli view --lang fr` to compare a French window), `document.title` reads `visuals.pbip · rev 0` and
  `model.bim · rev 0`, the page has the viewer's two icons, the File menu shows Reload only,
  the Model tab has semibold headers and the filter-direction `▶`, and the `.bim` shows
  "Pas de rapport dans ce fichier" without an error bar.
- **Browser check, 2026-10-29** (not part of CI): headless Firefox 159 with a throwaway profile,
  driven over WebDriver BiDi, on `bicraft-cli view …/visuals.pbip --ui egui`: the report page draws
  with WebGPU, and with WebGPU turned off (`dom.webgpu.enabled = false`) through the WebGL2
  fallback; a click on a page tab changes the page, a click on a slicer item filters
  the visuals (French chrome from the browser's language). `?webgl` forces WebGL2.
- **Manual check**: build the bundle, build `bicraft-cli`
  with `BICRAFT_WEBUI_DIR`, run `bicraft-cli view crates/engine/tests/fixtures/layout-oracle/visuals/legacy/visuals.pbip --ui egui --open`,
  check that the page tabs and the page appear, a slicer click filters, the Model and
  Validation tabs show, and a change to the file appears without a reload. Then
  `bicraft-cli serve` with a token, `POST /w/<id>/view/login`, open the login URL and
  `/w/<id>/view/app/`.


## Editing from the window: File view, page tabs, document tabs

The File view (Backstage), the page tabs and the document tabs are the same code in both shells
(`crates/ui/src/backstage.rs`, `page_tabs.rs`, `app.rs`; [desktop.md](desktop.md) describes
them). Each button runs an engine command, built in one place (`crates/ui/src/actions.rs`), sent
to the active document's source as `Request::Command` and answered with `Update::Ran`:

| Source | Commands | Open > Browse |
|---|---|---|
| desktop, local file | the engine session on the worker thread | the Open dialog |
| desktop, server | `POST /w/<id>/mcp`, `tools/call run_command` with the bearer token | `GET /w/<id>/files/` |
| browser on `bicraft-cli serve` | the same MCP call, same origin, with the browser's credentials (the auth proxy's) | `GET /w/<id>/files/` |
| browser on `bicraft-cli view` | none: the answer says so (no MCP endpoint there) | not shown (no workspace) |

- The server's rules apply unchanged: readers are refused by the MCP endpoint, the server mode's
  path sandbox and command policy (`docs/server-mode.md`) hold, a busy workspace answers 503 and
  the window shows the message. The browser's MCP call needs the page's origin in the server's
  allowed origins and credentials the endpoint accepts (the auth proxy's headers); the viewer's
  login cookie is valid for the viewer and the file routes only, so with it commands are refused
  with the server's 401/403 message.
- What differs by shell is only where a path comes from: the desktop's Save and Open dialogs, a
  workspace-relative name typed in the browser (checked by `remote::safe_relative`), and New's
  folder (the dialog; the workspace's root in the browser). The pages draw the same frames in
  both shells (`the_new_screens_draw_the_same_frames_in_both_shells`: Info, Export, page tabs and
  document tabs).
- **Recent in the browser**: `localStorage` keeps `{ws, path, opened, pinned}` entries whose `path`
  is workspace-relative (no drive, no `..`, no backslash) and `ws` a workspace id; never a disk
  path or a URL (`prefs::ui_only`).
- **Document tabs in the browser** (decision of 2026-11-04: the same tabs and the same cap as the
  desktop, 8; parallel views since `feat/webui-parallel-views`, 2026-11-05): the page's first tab
  is the page's own view, which follows the workspace's session (what an agent edits over MCP).
  Every workspace file opened from Open > Browse or Recent gets a tab of its own whose source is a
  **document view** on the server ([mcp.md](mcp.md), "Document views"): `POST …/api/session
  {"file": …}` opens a private session of its own on that file, and the tab reads its own
  snapshot, event stream and assets (`?v=<its view>`), interacts with its own token, and runs its
  commands at its own MCP endpoint (`/w/<id>/mcp/view/<view>`). The tabs work side by side as the
  desktop's: one tab's edits never reach another, switching tabs with unsaved edits is allowed and
  reopens nothing, the background tabs keep taking in their updates, and Save writes that tab's
  file. Closing a tab closes its view (`POST …/api/close?v=`), its session with it. When the
  page itself goes (its browser tab or window closed, a reload, another page loaded), the app
  closes every view its tabs still hold the same way on `pagehide`, as `keepalive` requests that
  outlive the page, so the server drops them at once instead of after `--view-idle` (their
  unsaved edits stay in the recovery journal, below). A page the browser keeps for its back
  button (`pagehide` with `persisted`) keeps its views, since it may come back; a request the
  browser never sends (a crash, a lost network) leaves the view to the idle expiry.
  - **Shared code with the desktop.** `Shell::ws_tabs` (the browser) only says that a workspace
    file opens in a tab of its own (`Origin::Workspace`); the tabs, their switching and their
    background updates are the desktop's code (`app.rs`). The desktop is unchanged (connected to a
    server, a workspace file opens in the server tab's session).
  - **Saves** follow the server's rules for editors: `expected_revision` per view, and the
    per-file check (`file_conflict`): a tab saving a file another tab or an agent saved since it
    opened it is refused, nothing is written, and the window shows the message (Save As, or reopen
    the file).
  - **Limits.** The server caps document views per user (default 8) and per workspace (default
    32); every snapshot carries them (`view.limits`), and the window opens at most
    `views_per_user` workspace tabs beside its first one, 8 tabs in all. At a cap the server first
    closes the caller's least recently used view that has no event stream and no unsaved edit;
    otherwise it answers 429, and a 503 when its memory is nearly full or the workspace is busy:
    the tab closes and the **tab strip shows the server's message** (`⚠ …`, a click dismisses
    it). Idle views expire on the server (`--view-idle`, 30 min; four times as long with unsaved
    edits); a tab whose view expired shows the server's 404 message.
  - **Readers** get read-only views: the saved state of their file (never an editor's unsaved
    edits), read commands only; the ribbon greys editing commands as for the page's view.
  - Tests: `browser_tabs_show_different_files_at_once` (three tabs, three sources and files,
    edits kept per tab, switching with unsaved edits, no `file.open`),
    `the_servers_view_limits_show_in_the_tab_strip` (the server's cap, a refused tab, the
    message and its dismissal), `the_new_screens_draw_the_same_frames_in_both_shells` (two tabs
    and the tab strip's message, the same frames in both shells), `bicraft-web`'s
    `document_views` (the session answer, limits and refusals, URLs) and
    `a_hidden_page_releases_the_views_it_holds` (a closed tab's view released once, nothing on
    a page kept for the back button, the rest when the page goes), and the server's
    `crates/mcp/tests/http_docviews.rs`.

## Auto-recovery and autosave in the browser

The browser keeps nothing: a tab's unsaved edits are journaled **on the server** when it runs
with `--recovery-dir` ([recovery.md](recovery.md)), under a hash of the user's identity. A tab
closed by the browser, a view that expires (`--view-idle`, four times as long with unsaved
edits) or a server restart loses nothing: when the user opens the file again, the same recovery
prompt as on the desktop offers the edits (Recover, Discard, Later), and File > Home lists the
workspace's files with recoverable edits (`recovery.list {all: true}` on the tab's session). The
status bar's `Recovery: N edits` and File > Info > **Save automatically** are the desktop's;
autosave runs only on a server started with `--allow-autosave` (the snapshot's
`view.limits.autosave`), through the ordinary `file.save` and its per-file check. Tests:
`the_prompt_draws_the_same_frames_in_both_shells` and the other `recovery_tests` of the UI,
`crates/mcp/tests/http_recovery.rs`.

## The DAX formula bar in the browser

The formula bar (`crates/ui/src/formula.rs`, [desktop.md](desktop.md) "Formula bar") is the
shared UI's, so the browser has it as the desktop does: the same keys (Enter commits,
Shift+Enter and Alt+Enter add a line, Esc cancels, Ctrl+Space opens the autocomplete; none of
them is a key the browser keeps for itself), the same frames
(`the_bar_draws_the_same_frames_in_both_shells`). Everything it needs goes through the
document's source like any command: the full expression (`measure.get`, `column.get`,
`table.get`), the function list (`dax.functions`, asked once per window; a server without it
leaves the autocomplete to the model's names and the keywords), the live check
(`dax.validate {kind}`, 0.35 s after typing stops, answered asynchronously so typing never waits
for the network) and the commit. On a server, a reader's bar is read-only (the server refuses
the edit commands anyway).

## Canvas editing in the browser

Canvas editing (`crates/ui/src/canvas.rs`, [desktop.md](desktop.md) "Canvas editing") is the
shared UI's: the same selection, drags, handles, smart guides, Grid snap, arrow keys, Align,
Layer order and Group, the same commands (`visual.move`, `visual.align`, `visual.order`,
`visual.group`, `visual.copy`, `visual.paste`, `visual.duplicate`) and the same frames
(`the_canvas_draws_the_same_frames_in_both_shells`). The clipboard is the browser's: Ctrl+C
writes the clip's JSON text with the async clipboard API (`navigator.clipboard.writeText`, as
eframe does for every copy) and keeps it in the in-app clipboard; Ctrl+V reads the page's
`paste` event (no permission prompt), and when no paste event comes (an empty or refused
clipboard) the in-app clipboard is pasted three frames later; Home > Paste and the menus' Paste
ask `navigator.clipboard.readText()` (the browser may prompt), and use the in-app clipboard when
it is refused or holds no clip. The in-app clipboard is the window's, so a copy pastes into
another document tab of the same page too. Ctrl+D is Duplicate: the page stops the browser's
bookmark dialog for Ctrl+D (`protocol::stops_browser_key`; Ctrl+Shift+D stays the browser's).

## The Data view in the browser (M11.17)

The Data view (`crates/ui/src/data_view.rs`, [desktop.md](desktop.md) "Data view") is the shared
UI's too: the same grid, Data pane, header menu and Column tools in both shells, the same frames
(`the_data_view_draws_the_same_in_both_shells`, the grid with a column selected and with a
header's menu open). Its rows come through the document's source like any command, a page at a
time (`table.rows`, 200 rows, at most three requests in flight), so a server sends only what the
window shows and a table of millions of rows never crosses the network whole; the answers arrive
asynchronously and the grid shows `…` until they do. `table.rows`, `query.preview` and
`visual.records` are reads that run on the workspace's shared session
([web-deployment.md](web-deployment.md) "Concurrency"), so a reader can page, sort and filter
(view state only) and preview queries; the Column tools are greyed for a reader, and a preview
that names `allow_connectors` is refused to a reader and, unless the server allows network
access, to everyone. Ctrl+C in the grid copies through the browser's clipboard event, as the rest
of the UI does.

## Command palette and parameters forms (M11.2)

Every engine command is reachable from the window, the same in both shells: **Ctrl+Shift+P**
opens the command palette (`crates/ui/src/palette.rs`), and a command that takes values opens its
parameters form (`crates/ui/src/form.rs`, `form::open(app, id, prefill)`, which the ribbon and
the menus call too).

- **One table, one answer.** The palette lists `registry::COMMANDS` (the UI's copy of the
  engine's registry, compiled into both shells) and greys a command with the same reason as the
  ribbon (`ribbon::state`: the snapshot's `commands.disabled`, computed by the session where it
  runs, the reader role on top), in English or French (`registry::reason_text`).
- **The palette** lists every command, recent ones first (kept in the preferences, `commands`);
  typing filters on the label (English and French), the id, the engine's label and the place,
  accents ignored, letters in order counting. Each row shows the label, its place
  (`Model › Relationships`), its id and its shortcut; a command that cannot run is greyed with
  `Unavailable: <reason>`. ↑ ↓, Page Up/Down; Enter runs it as its button does
  (`ribbon::invoke`: the File view's flows, the form when values are missing, else the command,
  a read command's answer in the answer window); Shift+Enter always opens the form; Escape
  closes.
- **The form** has one row per parameter: a picker of every object a command names (pages and
  visuals by their names, tables, the chosen table's columns, hierarchies and partitions,
  measures, calculation groups and the chosen group's items, shared queries, roles, cultures,
  report measures, relationships by their ends `Sales[Category] → Product[Category]`, data roles
  and visual types), all from the snapshot's names (view-snapshot.md, "Names for the forms"),
  except the environment profiles, which the form fetches with `env.list` through the document's
  source when it opens (the same request for a local file, a server and the browser, where it
  goes to the tab's own document view; on `bicraft
  view`, which has no command endpoint, the field stays free text with the reason under it); a
  field picker (`Table[Column]`, `[Measure]`) next to free text, dropdowns for enums,
  checkboxes, checked integers and numbers, a monospace editor for DAX, M and code, a colour
  button, checked lists and JSON. An optional value shows the command's **default** (the
  engine's `Param::default`, declared on every optional boolean that has a fixed one): a
  checkbox is drawn in its default state with `(default)` and left out of the parameters until
  touched (Reset leaves it out again); one without a fixed default (left out means unchanged) is
  drawn mixed with `(not set)`; an enum says `(default) json`, a text field shows it as its hint.
  Required fields carry `*`; Run (or Ctrl+Enter) checks the values first and shows each problem
  under its field, then the command's answer (`✓ Done` and the answer, below) or its error
  (`✕ Error: …`) inline. Paths use the
  desktop's Open, Save or folder dialog, and in the browser the workspace's files
  (`Request::ListFiles`). `view.interact` and `visual.format` get a JSON field for keys their
  form does not list. A command the role or the policy refuses shows the reason and no fields.
- **Command answers** (`crates/ui/src/answer.rs`), in the form and in the ribbon's answer window
  alike: a list of records (`*.list`, `model.summary`'s tables, `model.storage_stats`, VPAX,
  `model.usage`, BPA and validation findings, `file.diff`'s changes, `dax.evaluate`'s rows in its
  columns' order, one grid per EVALUATE) is a **grid**: a click on a header sorts it ascending,
  then descending, then back to the answer's order (numbers by value before text, text without
  regard to case, empty cells last), Copy as TSV copies the rows in that order with the header,
  and only the visible rows are laid out (100,000 rows scroll as fast as 100; up to 200,000 rows
  and 64 columns, the rest stays in the JSON). An object is a **key/value tree** (nested objects,
  lists and grids fold). The **JSON** toggle shows the answer as it came, and Copy JSON copies
  it whole. The answer is read once when it arrives; frames only draw it.
- **Search**: the title row's search button opens the palette (and closes it), as Ctrl+Shift+P.
- **Shortcuts** (`ribbon::shortcuts`, the registry's chords): none while text is typed or a
  dialog, the palette or a form is open. Ctrl+S saves; Ctrl+O is the desktop's Open dialog and,
  in the browser, the File view's Open; Delete removes the selected visual, ignored while a
  control has the focus (a page tab deletes its page); F5 is `model.refresh` on a PBIP project
  or a PBIX and a reload of the file otherwise.
- **Keyboard in the browser** (the one difference of the shells, `Caps::reserved_keys`): the page
  stops the browser's own F5 (its reload) so F5 is the app's there too; Ctrl+F5, Shift+F5 and
  Ctrl+R still reload the page (`protocol::stops_browser_key`). Browsers never hand Ctrl+N (a new
  window) to a page, so New (`model.new`) is **Alt+N** in the browser
  (`registry::BROWSER_CHORDS`); its tooltip and its palette row say so in the browser only.
- Tests: `crates/ui/src/tests/palette_ui_tests.rs` (search, keyboard, French, reasons, a reader,
  every command's form drawn empty and with hostile values, runs through the engine, checks
  before Run, shortcuts, the same frames in both shells), and `every_command_has_a_button` reads
  the palette's own entries (`palette::entries`). `crates/ui/src/tests/answer_ui_tests.rs`: a
  grid answer sorted and copied, the JSON toggle, an object as a tree, `dax.evaluate`'s grid in
  the form, 100,000 rows (only the visible ones laid out, frames as fast as with 100, seen red
  with every row laid out), the same frames in both shells for a grid answer, every picker of
  every command filled from a sample session (profiles through `env.list`), the defaults shown,
  the search button, the model menus' object submenus, the browser's Alt+N;
  `crates/ui/src/answer_tests.rs` (grids, sorting, TSV, trees, hostile answers);
  `apps/bicraft-web/src/protocol.rs` `the_page_keeps_f5_and_alt_n`.

## Control channel in the browser (M11.6)

The desktop's control channel ([desktop.md](desktop.md), "Control channel": the same methods,
the same replies) works in the browser app too, so an agent drives and screenshots the page the
same way. A page cannot listen on a port, so the viewer server relays: the agent posts its
request to the server, the server hands it to the app page over an event stream, the page runs
it in the window (`bicraft_ui::control`, the code the desktop runs) and posts the reply back.

```text
bicraft-cli view Sales.pbip --ui egui --open --control     # the app page and the relay
bicraft-cli ui --view ui.inspect                        # the token file of bicraft-cli view
bicraft-cli ui --view ui.screenshot --out page.png      # the canvas as drawn (WebGPU or WebGL2)
```

```text
agent ── POST /api/control (Authorization: Bearer <control token>) ──▶ bicraft-cli view
                                                                         │ queue (64)
app page ◀── GET /api/control/events (SSE `event: control` {seq, request}) ┘
   │ runs it between frames: crates/ui/src/control.rs
   └── POST /api/control/reply {seq, reply} (X-Bicraft-Token: page token, same origin) ──▶ the agent's answer
```

- **Opt-in.** `bicraft-cli view --control [--control-file F]` (needs the embedded browser app). Off,
  the three routes answer 404 and the page has no control slot. The **control token** is new at
  each start, separate from the page's token, and written, never printed, to an owner-only file
  (`--control-file`, `BICRAFT_CONTROL_FILE`, else `control-view.json` next to the desktop's
  settings): `{"kind": "view", "url": "http://127.0.0.1:P/api/control", "token", "pid"}`.
- **The agent's route** `POST /api/control`: the request object as a JSON body (at most 1 MiB,
  `Content-Type: application/json`), the token as a bearer (compared in constant time). Refused:
  no or a wrong token (401; the page's own token is not it), a request carrying `Origin` or
  `Sec-Fetch-Site`, so a web page cannot use it even with the token (403), more than 200 a second
  (429), a body that is not an object with a string `method` (400, 413, 415, 411), no app page
  connected or 64 requests waiting (503). The answer is the reply object; after 60 s without one,
  a request no page took is dropped (it never runs) and one a page took is "outcome unknown".
  The agent's `token` key, if sent, is removed before the page sees the request.
- **The page's side.** The app page carries the page token in its `bicraft-control-token` slot
  only when the relay is on; the app then opens `GET /api/control/events` (Server-Sent Events,
  at most 8 pages, keepalives every 15 s) and posts each reply to `POST /api/control/reply` with
  `X-Bicraft-Token` and the same `Origin` / `Sec-Fetch-Site` checks as `/api/interact` (403
  otherwise), at most 48 MiB (a screenshot). A request goes to one page: the first stream that
  takes it, and only that page may answer it; a page that went away (a closed tab) leaves it to
  the next one. A reply to a request nobody waits for is a 404. Requests are checked by the page as the desktop checks a line
  (`protocol::control_event`), so a bad one is answered without reaching the window.
- **Security model.** The relay adds nothing a web page can reach: the agent's route refuses
  browsers and needs the control token, which only the file's owner can read; the page's routes
  need the page token, which only same-origin pages of this viewer have (the CSP and the
  `Host` check stand), and they only carry requests the agent sent. As with `bicraft-cli view` in
  general, other local processes can load the page and connect to its event stream: on a shared
  machine they could take (and answer) a relayed request, never send one. What a request may do
  is what the page may do: on `bicraft-cli view` the page runs no engine command (its answer says
  so), so the channel switches views, pages and selections, reads the state and takes
  screenshots; reader interactions go through `view.interact` as clicks do.
- **Screenshots** are egui's screenshot viewport command in the browser too: the canvas as the
  GPU drew it (eframe reads the WebGPU or WebGL2 frame back), PNG in base64.
- **On `bicraft-cli serve`** (`--viewer-control`, [web-deployment.md](web-deployment.md), "UI control
  channel"): the same three routes per workspace under `/w/<id>/view/`, behind the server's
  authentication. The agent's route needs an editor credential and the control token in
  `X-Bicraft-Control` (the bearer is the server's credential); each app page's slot carries its
  own view's token, its stream is `api/control/events?v=<its view>` and its replies go to
  `api/control/reply?v=<its view>`. A request goes to one of the caller's own pages: the one
  `?v=` names, the page that opened document view `?v=` (the event then carries `"doc": <file>`
  and the window switches to that file's tab first, `bicraft_ui::control::Job::in_doc`), or the
  caller's page connected last. The app opens its document views with `?v=<its view>` so the
  server knows which page holds them.
- **In a real browser** (2026-11-05, Firefox 159.0.1 headless, throwaway profile, nothing but the
  page and `bicraft-cli ui`): `apps/bicraft-cli/tests/browser_control.rs`, ignored by default. It
  starts `bicraft-cli view <visuals.pbip> --ui egui --control` and `bicraft-cli serve --viewer-control`
  (a workspace holding the `visuals.pbip` fixture), loads the app page in Firefox and drives it
  through the relay: the page tabs, a command (refused on `bicraft-cli view`, run on the document
  view's own session on `bicraft-cli serve`), `app.open` of a workspace file into a document tab,
  `server.views` and a request addressed to that document view, the palette, a form, the Model
  view, and `ui.screenshot` of the canvas (PNG). Run it with a `bicraft-cli` built with the app
  embedded:

  ```text
  set BICRAFT_WEBUI_TOOLS=<TOOLS> & cargo xtask webui        # the bundle
  set BICRAFT_WEBUI_DIR=<target>/webui                      # embedded at build time
  set BICRAFT_FIREFOX=C:\Program Files\Mozilla Firefox\firefox.exe
  set BICRAFT_BROWSER_SHOTS=<DIR>                           # optional: keeps the PNGs
  cargo test -p bicraft-cli --test browser_control -- --ignored --test-threads 1
  ```

  Without Firefox or the embedded app each test prints a note and passes.
- **Tests.** `crates/view/tests/control.rs` plays the page with raw HTTP against the real
  server: a request reaches the page without the agent's token and the page's reply is the
  agent's answer; tokens, web pages, bodies, no page, methods refused; the page token and the
  origin required for replies; the routes absent without `--control`; a page that went leaves
  the next request to another. `apps/bicraft-web/src/protocol.rs`: the control slot, the routes'
  URLs, relayed requests read and refused. The window's side is the desktop's
  (`crates/ui/src/tests/control_ui_tests.rs`); the wasm shell is checked by `cargo xtask webui
  check`.

## Parity checklist: page.html → egui (desktop and browser)

Status: ✅ done in egui · 🟡 partly · ❌ missing · ➖ not applicable / not wanted. The browser
column is the egui app in the browser (phase 1); where it differs from the desktop, the
difference is the shell's.

### Header and chrome

| page.html feature | desktop (egui) | browser (egui) | Notes |
|---|---|---|---|
| Brand logo and name | ✅ title row (painted mark) | ✅ | design.md: the mark drawn from `bicraft-icon.svg` |
| Source path (RTL-clipped), format/kind pill | 🟡 file name + format pill | 🟡 | the path is the name's tooltip, not RTL-clipped |
| `rev N` pill, unsaved `● model` / `● report` pills | ✅ title row | ✅ | amber dot for unsaved parts |
| Last command pill (✓/✕ id, flash, time) | 🟡 status bar `✓ id` / `✕ id` | 🟡 | no flash, no time |
| Snapshot version pill (newer format) | ❌ | ❌ | |
| Document title `● file · rev N` | ✅ window title | ✅ `document.title` | the shared `App::window_title`; the browser shell sets `document.title` |
| Connection state (connecting, live, offline · retrying in N s) | 🟡 status bar pill (live, connecting, offline) | 🟡 | no retry countdown |
| "Open in Power BI Desktop" button (token, busy, messages, unsaved warning) | ❌ | ❌ | `bicraft-cli view` only; the token is read by the browser source |
| Download of the saved file (server mode, unsaved warning) | ❌ | ❌ | server only |
| Role (reader/editor) | ✅ status bar | ✅ status bar (`view.role`) | |
| Favicon | ➖ window icon ✅ | ✅ | `/app/` carries the viewer's two icons (a test keeps them equal) |

### Report tab

| page.html feature | desktop | browser | Notes |
|---|---|---|---|
| Page tabs (active dot, hidden pages dimmed, issue counts, 500 cap, truncated note) | ✅ page tabs at the bottom (the only page navigator) | ✅ | also edits: add, rename, duplicate, delete, hide, type, drag to reorder ([desktop.md](desktop.md)) |
| File view: New, Open, Recent, Info, Save, Save As, Export (convert, PNG, compare) | ✅ | ✅ (Browse = workspace files) | not in page.html; commands through the source (see "Editing from the window") |
| Several files open (document tabs) | ✅ up to 8 | ✅ up to 8, each a document view of its own on the server (parallel) | "Document tabs in the browser" |
| Page change sends `page` when interactive; "Follow active page" otherwise | ✅ sends `page` | ✅ | follow mode: read-only viewers |
| Rendered mode (theme, data, wallpaper, page background, images) | ✅ bicraft-render | ✅ | the same renderer as `bicraft-cli render` |
| Wireframe mode (type icon, title, fields per role) | ❌ | ❌ | |
| Page info (size, visual count, theme name, hidden, top-500 note) | ❌ | ❌ | |
| Visual selection → details panel (issues, fields, data preview 30 rows, position, identity, group members, raw JSON) | partly: the Build pane (type, fields by well) | partly: the same | the egui app edits the fields (Build and Data panes, drag and drop, below); issues, data preview, position and raw JSON not shown yet |
| Field editing: Data pane (tables, columns, measures, hierarchies, folders, search, hidden toggle), Build pane (type picker, wells, aggregation, ×), drag and drop to wells and onto visuals | ✅ | ✅ | not in page.html; every edit an engine command (`visual.bind`, `visual.unbind`, `visual.move_field`, `visual.set_aggregation`, `visual.set_type`); readers read-only |
| Format visual (the Build pane's second sub-tab, ROADMAP M11.22): Visual and General cards (title as free text, subtitle, effects, header icons, tooltips, properties; axes, legend, data labels, data colours, card, table, slicer and image cards by type), search, on/off switches, Reset to default | ✅ | ✅ | not in page.html; every edit an engine command (`visual.set_title`, `visual.format`, `visual.axis`, `visual.tooltip_page`, `image.set`), values read with `visual.props`; the Visual ribbon tab's Format button opens it; no Format visual tab for readers |
| DAX formula bar: `Name = <DAX>` of the selected measure, calculated column or calculated table, colours, expand, Enter / Esc, autocomplete, signature help, live check | ✅ | ✅ | not in page.html (it shows a measure's DAX read-only in its details); commits through `measure.set`, `column.set`, `partition.set` + `table.rename`, `measure.add`; checks through `dax.validate` on the document's source; readers read-only |
| Canvas editing: select (click, Ctrl+click, rubber band, Ctrl+A), drag to move, eight resize handles, smart guides and Grid snap, arrow keys, align and distribute, layer order, group, copy, paste and duplicate across pages and documents | ✅ | ✅ (the browser's clipboard, an in-app fallback) | not in page.html; one engine command per gesture (`visual.move`, `visual.align`, `visual.order`, `visual.group`, `visual.copy`, `visual.paste`, `visual.duplicate`); readers select and copy only |
| Page panel (page props, visuals with issues, all visuals) | ❌ | ❌ | |
| Change flash on visuals that changed | ❌ | ❌ | |
| Fit page / fit width | ✅ (Report ribbon) | ✅ | page.html fits the width of its stage |
| Resizes without rebuilding | ✅ re-render at size and DPI | ✅ | |

### Interactions

| page.html feature | desktop | browser | Notes |
|---|---|---|---|
| Slicer item click, Ctrl+click, Select all | ✅ | ✅ | same hit map |
| Dropdown slicer popup | ✅ | ✅ | |
| Between / before / after popup (Apply, Clear, Enter) | ✅ | ✅ | |
| Relative date popup | ✅ | ✅ | |
| Text slicer popup | ✅ | ✅ | |
| Data-point select / Ctrl / background clears | ✅ | ✅ | |
| Selection dimming and cross-highlight drawing | ✅ | ✅ | |
| Page navigator, action buttons (page, back, apply/clear slicers) | ✅ | ✅ | |
| Tooltips (rows of the point, highlighted value) | ✅ | ✅ | |
| Edit interactions mode (Filter / Highlight / None icons per target, live preview, Esc) | ✅ | ✅ | not in page.html; the same frames in both shells (`the_mode_draws_the_same_frames_in_both_shells`); docs/desktop.md "Edit interactions" |
| Error toast of a refused action (6 s) | 🟡 red bar `✕ reason` | 🟡 red bar | stays until closed |
| Escape closes popup / clears selection | 🟡 popup only | 🟡 | |
| Read-only viewer (`--read-only`, no token) | 🟡 | 🟡 clicks answer "read-only" | page.html shows no hit regions then |
| Drill: header icons on hover (up, drill mode, next level, expand all), right-click Drill down / up / Show next level / Expand to next level | ✅ | ✅ | not in page.html (`view.interact` drill_*; gaps.md G51) |
| Matrix +/− and Expand / Collapse > Selection, Entire level, All | ✅ | ✅ | not in page.html |
| Drillthrough (right click > Drill through, Drillthrough buttons, Back) | ✅ | ✅ | not in page.html; cross-report refused |
| Report-page tooltips (the page drawn small under the pointer) | ✅ | ✅ | not in page.html |
| View as (Model › Security › View as: None, Other user, roles, OK; "Now viewing as" banner, Stop viewing) | ✅ | ✅ | not in page.html; `view.interact` view_as, view state (docs/view-snapshot.md "View as"); readers too (`crates/ui/src/tests/view_as_ui_tests.rs`) |
| Bookmarks, matrix points, scatter/waterfall/ribbon points | ❌ | ❌ | not in page.html either (gaps.md G29) |

### Model tab

| page.html feature | desktop | browser | Notes |
|---|---|---|---|
| Header: name, culture, counts, invalid measures, truncated, legend | 🟡 | 🟡 | |
| Model diagram: layered table cards, relationship lines (cardinality, direction, inactive), pan/zoom/fit | ✅ (`diagram.rs`, `model_view.rs`) | ✅ same code | `→` in tooltips and lists (fonts: see above) |
| Table cards: columns/measures, types, calculated, hidden, "show all" | ✅ | ✅ | headers semibold (Selawik); invalid measures with the wavy underline, under the name |
| Side panel: model, invalid measures, tables; table/column/measure/relationship details, DAX expression, diagnostics | ✅ details panel | ✅ | "Mentioned in measures" is a text search, not the dependency analysis |
| Change flash on tables and measures | ❌ | ❌ | |

### Validation tab

| page.html feature | desktop | browser | Notes |
|---|---|---|---|
| Model card: status, invalid objects with diagnostics | ✅ | ✅ | |
| Click jumps to the model object | ❌ | ❌ | |
| Report card: status, error and warning counts, issues | ✅ | ✅ | |
| Visual issues card (sorted, click jumps to the visual) | ❌ | ❌ | |
| Truncated notes | 🟡 | 🟡 | |

### Global

| page.html feature | desktop | browser | Notes |
|---|---|---|---|
| Live updates (SSE, inline snapshots, refetch) | ✅ (watcher / SSE) | ✅ (EventSource, inline used) | |
| Reconnect with back-off | ✅ 0.5–5 s | ✅ 0.5–15 s | |
| Language: chrome FR/EN from the browser; generated text from `ui.strings` | ✅ chrome from OS/`--lang`; visuals from the snapshot | ✅ chrome from `navigator.language` | |
| Light and dark themes (`prefers-color-scheme`) | ✅ design tokens, light by default, toggle kept | ✅ | the app's choice, not the OS's (design.md §8); page canvas colours are the report's |
| Empty states (waiting, no report, no pages, no model, snapshot error) | 🟡 | 🟡 | no report (a `.bim`) and no pages ✅ as page.html, without an error bar; snapshot error still the red bar |
| Remembered tab and display mode (`localStorage`) | ✅ settings file (tab, fit, theme, ribbon) | ✅ `localStorage` (UI preferences only, see "Browser storage") | |
| Hostile-input caps (500 visuals, 300 tables, 1500 relationships, virtualized lists) | ✅ renderer caps | ✅ | |
| Narrow / mobile layout (≤ 860 px stacks the panels) | ❌ | ❌ | egui panels do not reflow; fit width helps |
| Touch | ➖ | 🟡 egui touch events | untested |
| Accessibility: roles, `aria-*`, keyboard focus on visuals (Enter/Space) | 🟡 AccessKit tree, every control named (EN, FR); Ctrl+F6 sections, Tab inside a section, modal dialogs that give the focus back, the focus ring (docs/desktop.md "Keyboard navigation") | 🟡 eframe web AccessKit, the same code; F6 and Ctrl+F6 not checked in a browser | visuals are not focusable items |
| Keyboard: Escape; Ctrl+O; the command palette (Ctrl+Shift+P, or the title row's search); the registry's shortcuts (Ctrl+M, Ctrl+Z, Ctrl+Y, Ctrl+N, Ctrl+Shift+S, Delete, F5 Refresh or Reload) | ✅ | ✅ F5 kept from the browser; New is Alt+N (browsers keep Ctrl+N) | M11, "Keyboard in the browser" |
| Errors never crash the page | ✅ | ✅ | |

### Known gaps of the egui app itself (both shells)

- Fixed in phase 2, step 1: the fonts (Selawik with semibold and bold, `→` and the viewer's
  symbols, identical in both shells), the red "no report in this file" bar of a model-only
  `.bim` (the Report tab now shows the viewer's empty state; Model and Validation work), the
  straight invalid-measure underline (now wavy).
- Decided 2026-11-04: the page canvas uses the bundled faces only in both shells
  (`bicraft_ui::pages::font_book`, Selawik), so the desktop draws the browser's pixels (tested:
  `pages_use_the_bundled_fonts_everywhere`, the render thread against the browser's synchronous
  path). `bicraft-cli render` keeps the system's fonts plus `BICRAFT_FONT_DIRS`, for comparisons with
  Power BI Desktop. The symbols (`✓`, `✕`, `⚠`…) come from a bundled Noto Sans Symbols 2 subset
  in both shells.
- The look is DeckCraft's design system with BIcraft's amber accent ([design.md](design.md)):
  title row, tab row and ribbon card, status bar, light by default with a kept dark toggle.
- The ribbon is built from the engine's command registry (ROADMAP M11): every ribbon-placed
  command is a button of its tab and group (Home, Insert, Report, Model, Data, Analyze, View, and the
  contextual Visual, Table and Measure tabs), greyed with the session's reason; menu-placed ones
  are in the right-click menus. The browser links no engine: both shells compile the same table,
  `crates/ui/src/registry_table.rs`, generated from `bicraft_engine::COMMANDS` and checked by a
  test, and read the enabled state from the snapshot (`commands.disabled`, computed by the
  server's session), so their ribbons are the same (tested:
  `the_ribbon_tabs_draw_the_same_frames_in_both_shells`).

### Browser-only differences (by design, from the shell's capabilities)

- The Home ribbon's File group has Reload only: no Open, recent files, Connect or Quit (no
  file dialog; the file and the server are the page's); Ctrl+O opens the File view's Open; files
  dropped on the canvas are ignored (no paths in a browser). Everywhere else the two shells draw the same frames.
- The File view: Open lists Recent and Browse (the server workspace's files on `bicraft-cli serve`;
  nothing on `bicraft-cli view`), no Servers; Export and Save As take a workspace-relative name instead
  of the Save dialog; New creates the project at the workspace's root; no template tile.
- Local `bicraft-cli view` sends no role, so an error bar stays until closed after the stream
  recovers.
- Keyboard reservation: New is Alt+N instead of Ctrl+N, shown in its tooltip and palette row
  ("Keyboard in the browser").
- Exports (M11.21): Export to PDF, Render to PNG and a visual's Export data write their file in
  the server workspace under the name typed (`report.export_pdf`, `report.render`,
  `visual.export_data`, with the server's path rules for `out`), then hand it to the user as a
  download: the shell's `Shell::download` clicks a link to the workspace's file route
  (`GET /w/<ws>/files/<path>`, `Content-Disposition: attachment`, streamed with no size cap of
  its own; the export commands bound what they write: 500 pages, 150,000 rows). The desktop
  writes where its Save dialog says, with no download. File › Export › Print runs
  `report.print` with `out` = `<file>-print.pdf` in the workspace (nothing is launched on a
  server), then `Shell::print` fetches that file, shows it as an `application/pdf` blob in a
  hidden frame and calls the frame's `print()` once it loads: the browser's print dialog (its
  own PDF viewer prints); when the fetch or the frame fails, the file is downloaded instead.
- Insert › Image and the page format's images (M11.20): no file dialog, so the image is one of
  the server workspace's image files (uploaded with `PUT /w/<id>/files/<file>`), picked in a list
  and read by the server under the workspace's path rules (`image.add {path}`); on `bicraft-cli view`
  there is no workspace and the status bar says so. The desktop sends the file's bytes instead
  (`image.add {data}`), so the same command and the same stored resource result. A web URL
  button opens a new browser tab (the desktop: the system's browser).

## Phase 2

1. ✅ (`feat/ui-core`, 2026-11-04.) The UI moved into `crates/ui` (`bicraft-ui`), with the two
   shells as thin apps: native (`apps/bicraft-desktop`: files, threads, the engine, ureq, the
   settings file, registration) and web (`apps/bicraft-web`: driven source, `document.title`).
   The app takes a `Shell` (capabilities, source start, file dialog, preferences store, title
   setter) instead of showing File-menu items that make no sense in a browser. Same fonts in
   both shells, the `.bim` fix, the favicon of `/app/`, the wavy underline; a test proves the
   two shells draw the same frames.
2. Close the parity gaps above, in order of use: header (source, revision, unsaved, last
   command, connection), visual details panel (selection, fields, data preview), wireframe mode,
   validation jumps and visual issues, open-in-Desktop and download (`Caps::open_in_desktop`,
   `Caps::download`), remembered UI preferences in the browser (a `PrefsStore` over
   `localStorage`, never tokens), narrow layout, accessibility of the page's visuals.
3. Run the interaction oracle on the egui path: the desktop test that compares a click's result
   with page.html's action (`slicer_and_data_point_clicks_match_the_viewer`) extended to every
   row of the interaction table, plus a headless-browser smoke test of `/app/` where a browser
   is available (wasm-bindgen-test, or Edge/Chrome headless with WebGL via SwiftShader).
4. Make `/app/` the default (`/` redirects, `--ui html` keeps the old page for a release), then remove `page.html` and its tests once the checklist is all ✅ or ➖.
