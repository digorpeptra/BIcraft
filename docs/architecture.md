# Architecture

BIcraft is a Rust workspace (edition 2024): `crates/*` (libraries), `apps/*` (the binaries),
`xtask` (the project's own tooling) and `fuzz/` (a separate workspace). This page describes how
it is put together; the binding rules are [AGENTS.md](../AGENTS.md) §4, and where the two differ,
AGENTS.md wins.

## Layers

Crates are layered: primitives (OPC, XPress9, SQLite, ABF, VertiPaq) → model and report → DAX →
serializations (TMSL, TMDL, PBIR, legacy layout) → containers (PBIX, PBIP) → engine → MCP server,
viewer and UI → the apps. A crate depends only on strictly lower layers. `cargo xtask layers`
enforces it from the table in `xtask/src/layers.rs`, where every new crate is registered.

| Layer | Crates (`bicraft-*`) | Responsibility |
|---|---|---|
| below L0 | `env`, `mem` | `BICRAFT_*` environment variables; the process memory pool. No workspace dependency |
| L0 | `opc`, `locale`, `xpress9`, `xpress8`, `sqlite`, `abf`, `vertipaq`, `scan`, `versions`, `mlang`, `tsbuild`, `http`, `auth` | Primitives: ZIP/OPC and UTF-16LE text, cultures and calendars, the XPress9 and Plain LZ77 codecs, SQLite, ABF backups, VertiPaq storage and scans, M language services, the custom visuals' TypeScript build, HTTP plumbing, authentication |
| L1 | `model`, `report`, `visuals` | The in-memory semantic model and report shared by every format, and the catalog of visual types |
| L2 | `dax`, `data` | DAX lexer → parser → binder → diagnostics; table data and Power Query |
| L3 | `tmsl`, `tmdl`, `metadata`, `pbir`, `layout`, `diff`, `powerpivot`, `databricks`, `sqlserver`, `webdata` | Model and report serializations, the semantic diff, Power Pivot import, connectors |
| L4 | `pbix`, `pbip`, `pbixwriter`, `bpa`, `render` | Containers (PBIX/PBIT packages and PBIP folders), best-practice rules, the page renderer |
| L5 | `engine`, `view` | The session and every action as a command; the live viewer page and its loopback server |
| L6 | `mcp`, `controller`, `ui` | The MCP server over the engine; the Kubernetes workspace controller; the egui UI shared by the desktop and browser shells (no engine outside its tests) |
| apps | `bicraft-cli`, `bicraft-desktop`, `bicraft-web` | The binaries `bicraft-cli` (CLI and `bicraft-cli mcp`) and `bicraft` (the desktop app, the native shell of the UI); the browser shell of the UI |

Nothing below L6 depends on the engine, and nothing below L5 uses an async runtime.

## Principles

- **Everything is a command.** Each user-visible operation is an engine command with an id, a
  params doc, `run`, a server-mode policy and tests (`crates/engine/src/commands.rs`). The CLI,
  the MCP server ([mcp.md](mcp.md)) and the UI reach every operation through those commands, so
  they behave the same; programmatic calls never open a dialog.
- **Every command has its button.** The UI reaches every engine command through a ribbon button,
  a menu item or the command palette, and every button runs one ([commands-ui.md](commands-ui.md)).
- **One UI.** `bicraft-desktop` and `bicraft-web` run the same egui code from `crates/ui`
  ([webui.md](webui.md), [desktop.md](desktop.md)).
- **Conversion goes through `model`.** No format crate calls another format crate.
- **No runtime dependency on Windows or Power BI.** XPress9, ABF, VertiPaq and SQLite are pure
  Rust; BIcraft never loads a Microsoft DLL or connects to Desktop's engine at run time.
- **Input files are hostile.** No `unsafe`, no panicking calls outside tests, checked arithmetic,
  capped allocations, bounded recursion; every command runs under a panic guard and the readers
  are fuzzed ([fuzzing.md](fuzzing.md)).

## Where to read next

- Formats: [docs/formats/](formats/).
- Server mode and deployment: [server-mode.md](server-mode.md),
  [web-deployment.md](web-deployment.md), [kubernetes.md](kubernetes.md).
- Quality gate and CI: [ci.md](ci.md); releases: [releasing.md](releasing.md).
