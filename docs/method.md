# Development method: the Crafting Apps conventions, adapted

Decision of 2026-11-04: BIcraft follows the development method of the Crafting Apps family
(DeckCraft, GridCraft, PhotoCraft, PdfCraft, FilmCraft, VectorCraft, LightCraft, DesignCraft).
This page compares their practice with BIcraft's, says what BIcraft adopts, and where BIcraft
keeps a stricter rule of its own. [AGENTS.md](../AGENTS.md) states the rules that result; if this
page and AGENTS.md disagree, AGENTS.md wins.

Sources read on 2026-11-04 through the GitHub API (read only), at these commits:

| Repository | Commit | Files read |
|---|---|---|
| [storytold/deckcraft](https://github.com/storytold/deckcraft/tree/23ac4f7) | `23ac4f7` | `AGENTS.md`, `CLAUDE.md`, `README.md`, `ROADMAP.md`, `crates/engine/src/cmd.rs`, `crates/engine/src/tests.rs`, `crates/ui-egui/src/menus.rs`, `crates/ui-egui/src/ribbon.rs`, `crates/ui-egui/src/ribbon/home.rs`, `xtask/src/parity.rs`, `docs/mcp.md`, `docs/ui-parity.md`, `docs/target-app-parity.md`, `docs/parity-checklist.md`, `docs/localization-parity.md`, `.github/workflows/ci.yml`, `.github/dependabot.yml` |
| [storytold/gridcraft](https://github.com/storytold/gridcraft/tree/632e4a1) | `632e4a1` | `AGENTS.md`, `CLAUDE.md` |
| [storytold/photocraft](https://github.com/storytold/photocraft/tree/5617e00) | `5617e00` | `AGENTS.md`, `crates/ui-egui/src/menus/items.rs` |
| [storytold/pdfcraft](https://github.com/storytold/pdfcraft/tree/8717a08) | `8717a08` | `AGENTS.md`, `CLAUDE.md`, `deny.toml`, `.github/pull_request_template.md`, `xtask/src/parity.rs` |
| [storytold/filmcraft](https://github.com/storytold/filmcraft/tree/c9be142) | `c9be142` | `AGENTS.md`, `CLAUDE.md`, `SECURITY.md`, `.github/ISSUE_TEMPLATE/bug_report.yml` |
| [storytold/vectorcraft](https://github.com/storytold/vectorcraft/tree/7ba7b1c), [lightcraft](https://github.com/storytold/lightcraft/tree/f30dd5e), [designcraft](https://github.com/storytold/designcraft/tree/df9f065) | `7ba7b1c`, `f30dd5e`, `df9f065` | `AGENTS.md`, `CLAUDE.md`; lightcraft `.github/PULL_REQUEST_TEMPLATE.md` |
| [storytold/craft-common-actions](https://github.com/storytold/craft-common-actions/tree/88c42c9) | `88c42c9` | `README.md` (the shared `gate.yml`, `release.yml`, `supply-chain.yml`) |
| [storytold/craft-i18n](https://github.com/storytold/craft-i18n/tree/c199dc4) | `c199dc4` | `README.md` |
| [storytold/craft-launcher-assets](https://github.com/storytold/craft-launcher-assets/tree/8619bde), [storytold/artcraft](https://github.com/storytold/artcraft/tree/1f969a9), `storytold/artcraft-services` | `8619bde`, `1f969a9` | `README.md`; `AGENTS.md`, `CLAUDE.md` |

Commit messages: the last dozen commits of each app. `storytold/craftrules`, named as the
family's shared standards, was not readable from this account then; its
standards are known here only through the apps that cite them. ArtCraft and ArtCraft Services are
a different stack (a Tauri desktop app with a TypeScript front end, and web services): they share
the brand, not the method, and nothing below comes from them.

## 1. Comparison

Legend for the last column: **adopted** (now in AGENTS.md or CLAUDE.md), **planned** (a
[ROADMAP](../ROADMAP.md) item), **kept** (BIcraft keeps its own rule), **decision** (§3).

| Area | Crafting Apps practice | BIcraft today | Change |
|---|---|---|---|
| Rule files | `AGENTS.md` binds agents and humans and wins over `CLAUDE.md`; `CLAUDE.md` holds working notes ([deckcraft AGENTS.md](https://github.com/storytold/deckcraft/blob/23ac4f7/AGENTS.md), [CLAUDE.md](https://github.com/storytold/deckcraft/blob/23ac4f7/CLAUDE.md)) | same split, same precedence; since 2026-11-24 AGENTS.md starts with the shared block of `storytold/craft-contrib` `standards/app-agents.md`, then BIcraft's own rules and where they depart from it | **adopted** 2026-11-24 |
| Every command has its button | Each command declares its place in the UI: `CommandSpec.menu` is the ribbon tab and group or the menu ([deckcraft cmd.rs](https://github.com/storytold/deckcraft/blob/23ac4f7/crates/engine/src/cmd.rs)); ribbon buttons call commands by id (`ribbon/home.rs`); the command palette (Ctrl+Shift+P) lists every enabled engine command and the UI commands, leaving out those that need a value ([menus.rs](https://github.com/storytold/deckcraft/blob/23ac4f7/crates/ui-egui/src/menus.rs) `palette_items`); the README says every button is also a command. PhotoCraft wires a menu item live when a command uses the id of its menu catalogue, and keeps a parity floor that may only rise ([photocraft AGENTS.md](https://github.com/storytold/photocraft/blob/5617e00/AGENTS.md) §3–4). No test in DeckCraft checks that every registered command has a button: the rule is held by the `menu` field and the palette | `Command` has `id`, `label`, `params`, `run`, `server`; no UI place, shortcut or enabled check. The desktop and browser app (one egui UI) reaches a few actions through its own buttons; most commands are reachable only from the CLI and MCP | **adopted** as a rule (AGENTS.md §4 and §9), **done** M11.1 (place, shortcut, needs and form on every command: [commands-ui.md](commands-ui.md)), **planned**: M11.2 palette with a parameters form, M11.3 coverage test both ways |
| Everything is a command | Every user-visible action is a command with id, label, menu place, shortcut, params doc, `enabled`, `run` and tests; UI, CLI, control channel and MCP dispatch by id; programmatic calls never open dialogs ([gridcraft AGENTS.md](https://github.com/storytold/gridcraft/blob/632e4a1/AGENTS.md)) | same, plus a server-mode policy per command | **adopted**: "never open a dialog" and `enabled` (M11.1: `Session::command_enabled`) |
| Headless reach | "Everything is reachable headlessly": a feature is done when its automation tool exists with an end-to-end test ([pdfcraft AGENTS.md](https://github.com/storytold/pdfcraft/blob/8717a08/AGENTS.md) §3) | every command is reachable from the CLI and MCP | kept (already the same) |
| Thin UI | Panels read engine state and act through commands; colours come from theme tokens | the egui app draws snapshots and sends commands; tokens specified in `docs/design.md` (branch `feat/design`) | **adopted** (AGENTS.md §9) |
| Never crash | No `unwrap`/`expect`/`panic!` outside tests, checked arithmetic, bounded recursion, capped allocations; a panic guard around command execution as a safety net; lock poisoning handled; every command fuzzed with empty and hostile params (`every_command_survives_empty_and_hostile_params` in [deckcraft tests.rs](https://github.com/storytold/deckcraft/blob/23ac4f7/crates/engine/src/tests.rs), `panic_hunt` in PhotoCraft) | the same rules; `Session::execute` runs each command under `catch_unwind`; per-area hostile-input tests, no test over every command | **adopted** (guard, poisoning, threads), **done** M11.4 (`crates/engine/tests/hostile_params.rs`) |
| Regression tests | "Prove a test fails before the fix by reverting the fix" ([pdfcraft CLAUDE.md](https://github.com/storytold/pdfcraft/blob/8717a08/CLAUDE.md)) | every crash fix has a regression test | **adopted** (AGENTS.md §3) |
| Provenance wording | "Clean-room" in every app; "written from scratch", no reading of other implementations | interoperability implementation that builds on public specifications, open-source code under its licences and black-box tests | **kept**: BIcraft never uses "clean-room" or "written from scratch" (§2) |
| Vendor observation | The vendor app is observed as a black box on synthetic files; never read or disassemble its bundle; screenshots of it only in a gitignored `plan/` | Desktop observed as a black box; no analysis of vendor binaries; oracles outside the repository | kept (BIcraft's rule is stricter: no reflection either) |
| Assets | Nothing from the target vendor; open, provable licences only; every asset has an `ATTRIBUTION.md`/`.toml` row and `cargo xtask assets` fails otherwise; icons drawn in code | no Microsoft assets; `ATTRIBUTION.md` lists assets; `cargo xtask assets` (in the gate) fails on a file under `assets/`, a bundled font or the UI's icons without a row | **adopted** M11.5 |
| Fonts | Fonts live in `storytold/craft-fonts`, an optional build input (`CRAFT_FONTS_DIR`), never committed | Selawik faces bundled (OFL, listed in `THIRD_PARTY_NOTICES.md`) | kept (`docs/design.md` §3); **decision** whether new fonts go to a separate repository |
| Test corpora | Real-file corpora in a separate repository we own, fetched pinned and SHA-256-checked; large fixtures never committed ([photocraft AGENTS.md](https://github.com/storytold/photocraft/blob/5617e00/AGENTS.md) §5) | real files local only (`BICRAFT_TESTDATA`), never copied, named or quoted; generated fixtures committed with `SOURCE.txt` | **kept**: client data never leaves the machine; **decision** on a pinned corpus of our own synthetic files |
| Oracles | External tools (ffmpeg, MuPDF, veraPDF) run only as separate oracle processes, never linked | Desktop and Analysis Services as development oracles outside the repository; verdicts stored as our data | kept |
| Quality gate | `cargo xtask ci` = fmt, clippy `-D warnings`, tests (nextest when installed), layers, assets, wasm check | fmt, clippy, tests, layers, docs (no Desktop timing, licence copies); `cargo xtask webui` check; Windows and WSL | kept; assets check **planned** |
| Dependency licences | `cargo deny` with a permissive allowlist and source rules ([pdfcraft deny.toml](https://github.com/storytold/pdfcraft/blob/8717a08/deny.toml)); Dependabot daily, grouped, `chore(deps)` / `ci(deps)` | THIRD_PARTY_NOTICES and generated crate notices in `cargo xtask dist`; `deny.toml` (permissive allowlist, per-crate exceptions with reasons, crates.io only, advisories) in its own CI job and `cargo xtask deny`; Dependabot weekly, grouped, `deps:` / `ci:` prefixes (BIcraft's commit style); actions pinned by SHA | **adopted** M11.5 (weekly rather than daily) |
| CI | Workflows only call the shared `storytold/craft-common-actions` workflows at `@main`; no CI change inside a feature PR | own `.github/workflows/ci.yml` (Windows, Ubuntu, MSRV) and `release.yml` | **decision** (BIcraft is not in the storytold organisation) |
| Branches | `<type>/<slug>` (`feat/`, `fix/`, `docs/`, `chore/`, `refactor/`); `main` the only integration branch, never pushed to directly in PdfCraft; DeckCraft agents push to `main` | `feat/`, `perf/`, `exp/`, `chore/` worktrees; merges into `main` and pushes only as the maintainers decide | **adopted** the prefixes; kept the merge rule |
| Commit messages | One task per commit, green states only; `M<n>.<k>: what` (DeckCraft, GridCraft, PdfCraft) or `<area>: what it does now` with the PR number (`Render: …`, `engine: …`) | `M<n>.<k>: what`; recent commits use `Desktop:` / `Web UI:` | **adopted**: milestone id when there is one, else the area |
| Progress docs | `ROADMAP.md` one-page summary (stage, headline numbers, progress log); `docs/gaps.md` ranked work list; `docs/target-app-parity.md` with each number marked measured or estimated; per-dimension parity checklists with a status line and revision history; generated `docs/parity-checklist.md` (`cargo xtask parity`) | `ROADMAP.md` milestones and log, `docs/gaps.md`, `docs/webui.md` parity checklist | **adopted**: numbers marked measured or estimated, gaps entry updated when work lands; **decision** on a parity assessment and generated checklist |
| Local plan | `plan/` gitignored: `STATUS.md`, execution plan, observation notes and screenshots of the vendor app | since 2026-11-24 a gitignored `plan/` holds the shared memory (`plan/MEMORY.md`); provenance detail stays in the maintainers' private notes, never committed | **adopted** 2026-11-24 |
| Looking at the UI | "For UI work, look at the result": control channel `ui.screenshot` or an offscreen snapshot; never OS-level synthetic input (LightCraft) | egui_kittest tests over the accessibility tree; `BICRAFT_DESKTOP_SHOTS=DIR` saves screenshots; the control channel's `ui.screenshot` (M11.6) | **adopted** (CLAUDE.md) |
| Control channel | Opt-in JSON control channel (`--control`), loopback only, a random token per launch in an owner-only file | the same, JSON lines and DeckCraft's reply shape (`bicraft --control PORT`), relayed to the browser app by `bicraft-cli view --control`; client `bicraft-cli ui` ([desktop.md](desktop.md), [webui.md](webui.md)) | **done** M11.6 (decision 6) |
| MCP | `<app>-cli mcp` on stdio, opt-in, never started or registered by default; `--connect` drives the running app; file access confined by roots ([deckcraft docs/mcp.md](https://github.com/storytold/deckcraft/blob/23ac4f7/docs/mcp.md)) | `bicraft-cli mcp` on stdio, DeckCraft's tool shape; server mode confines paths | kept |
| i18n | Shared catalogues in `storytold/craft-i18n` (TSV keyed by English, optional build input `CRAFT_I18N_DIR`); translations written from the meaning of the English, never copied from another product ([craft-i18n README](https://github.com/storytold/craft-i18n/blob/c199dc4/README.md)) | EN/FR UI strings in the code (`ui_lang`); model cultures and Desktop-matching labels (M10) | **adopted** the translation rule for BIcraft's own UI strings; **decision** on craft-i18n |
| Accessibility | contrast checks in DeckCraft's own accessibility checker (a deck feature); kittest reads the AccessKit tree | `docs/design.md`: 4.5:1 text contrast, never colour alone, token pairs tested | **adopted** (AGENTS.md §9) |
| Performance | "Performance is a feature": budgets and baselines (`perf/budgets.toml`, `cargo xtask perf`, bench history with a regression threshold) | `cargo xtask bench`, `engine-bench check`; BIcraft's own numbers only | kept; **never** a Desktop timing in the repository |
| Time | All calendar math through `chrono` | `bicraft_locale::calendar` over `jiff` for DAX, M and locale dates (M11.9 with `chrono`, moved to `jiff` in M11.43 when chrono was flagged unmaintained; behind the differential test `crates/engine/tests/calendar_golden.rs`) | **adopted** (decision 3) |
| Security and templates | `SECURITY.md`, PR template with never-crash, provenance and security checklists, issue forms | `SECURITY.md` (private reporting; server, auth, readers in scope), `.github/pull_request_template.md` (never crash, provenance, client data, server policy, buttons, docs, CI on both systems); no issue forms | **adopted** M11.5 (issue forms not yet) |
| Contributor credits | About window credits from `contributors/people.toml`, only the contributor's own entry | none | **decision** (when there are outside contributors) |
| Scoped builds | build and test only the crates touched; separate `CARGO_TARGET_DIR` and worktree per agent | same, plus machine-wide build slots kept by the maintainers | kept |

## 2. Where BIcraft keeps its own, stricter rule

- **Provenance wording.** Every Crafting App calls itself "clean-room". BIcraft never does, and
  never describes itself as written from scratch. The wording stays that of
  [AGENTS.md](../AGENTS.md) §1 and [interoperability.md](interoperability.md): an
  interoperability implementation that builds on public specifications, open-source code under
  its licences and black-box tests; contributors follow the rule of no analysis of vendor
  binaries.
- **Client data.** The family keeps real files in a pinned corpus repository. BIcraft's real
  files may hold client material: they stay on the maintainer's machine, are read only through
  `BICRAFT_TESTDATA`, and are never copied, named or quoted anywhere, corpus repositories
  included.
- **Never crash** is the same rule; BIcraft also caps memory per query and per process
  (`docs/beyond-ram.md`).
- **No Desktop performance figures.** The family publishes timings and budgets of its own app.
  BIcraft does too, but never a Desktop timing or a ratio against Desktop: the Desktop licence
  forbids disclosing benchmark results, and `cargo xtask docs` checks it.
- **Machine notes stay out of the committed tree**: build slots, locks, paths, installed tools,
  oracle locations. They live in the maintainers' notes or the
  gitignored `plan/`, as in the family; scripts take them as parameters or environment variables.
- **Merges.** Work is committed on its branch; merging into `main` and pushing happen only as
  the maintainers decide, never on a task's own initiative (no PR flow, no direct push from a
  task).

## 3. Decisions for the maintainers

Decided on 2026-11-04:

| # | Decision |
|---|---|
| 1 | Keep BIcraft's own workflows for now (Windows and MSRV legs); slim them like the shared gate (`chore/ci-diet`). Revisit craft-common-actions later. |
| 2 | Keep `ui_lang` for now; move to craft-i18n when a UI crate shared by the craft apps exists. |
| 3 | Yes, move calendar math to `chrono`, behind a differential test against today's results (DAX, M, locales). |
| 4 | Yes: a pinned corpus repository of files we author ourselves (synthetic models saved by Desktop), never client files. |
| 5 | Yes: a parity assessment in the family's form, on two axes: authoring features and the report viewer. |
| 6 | Yes: an opt-in UI control channel (loopback, token) for agents to drive and screenshot the app. |
| 7 | A parameters form generated from the params doc (M11.2); commands that need values count as reachable through it. |
| 8 | Later, once outside contributors arrive. |

The questions as they were put:

1. **CI through craft-common-actions** or BIcraft's own workflows (today's). The shared gate
   needs craft-fonts and runs Linux and macOS legs; BIcraft needs Windows and the MSRV job.
2. **craft-i18n** for the UI strings (TSV keyed by English, optional build input) or BIcraft's
   own `ui_lang` tables.
3. **Calendar math through `chrono`**: BIcraft's DAX, M and locale date code is its own. A move
   would need a differential test against today's results.
4. **A pinned corpus repository** of files we author ourselves (synthetic models, saved by
   Desktop), fetched and SHA-256-checked like `photocraft-corpus`, next to the local client
   files.
5. **A parity assessment** in the family's form (`docs/target-app-parity.md`, a generated
   checklist, numbers marked measured or estimated), and against what: Desktop's authoring
   features, its report viewer, or both.
6. **A UI control channel** (opt-in, loopback, token) so agents can drive and screenshot the
   running app, as the family does, beyond the kittest harness.
7. **Commands that need values in the palette**: DeckCraft leaves them out; BIcraft has many
   (DAX, names, paths). The plan (M11.2) shows a parameters form generated from the params doc;
   the alternative is to count only commands that run with no parameters.
8. **Contributor credits** in the About window, once outside contributors arrive.
