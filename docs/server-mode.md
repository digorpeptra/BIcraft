# Server mode: workspace root and command policy

Status: implemented 2026-10-26 (web stage 1, lot 1; `docs/web-deployment.md`, security review
items 1 and 2). Code: `crates/engine/src/sandbox.rs`, tests in `sandbox_tests.rs`. No network
code: this lot is what the HTTP transport (`bicraft serve`) relies on before any exposure.

## Local mode and server mode

| | Local mode (default) | Server mode |
|---|---|---|
| How | `Session::new()` | `Session::server(ServerConfig::new(root)?)`, or `Session::from_env()` with `BICRAFT_SERVER_ROOT` set |
| Paths | as given, relative to the process' current directory | relative to the workspace root, checked (below) |
| Commands | all | `file.open_in_desktop` refused; the Databricks connector needs the network setting |
| Who uses it | the CLI, `bicraft mcp` (stdio), `bicraft view` | `bicraft serve` (one session per workspace, rooted at the workspace folder) |

The CLI and the stdio MCP server still call `Session::new()`: their behaviour is unchanged. The
mode is fixed when the session is built (`Session::server` / `from_env`); the field is private and
never changes afterwards. A private session a command opens itself (`file.convert`, `project.test {path}`) inherits it.

### Configuration

| Variable | Meaning |
|---|---|
| `BICRAFT_SERVER_ROOT` | turns server mode on: the workspace root, an existing folder. Unset or empty: local mode. A missing folder or a file is an error (`Session::from_env` fails), never a silent local mode. |
| `BICRAFT_SERVER_NETWORK` | `1`/`true`/`yes`/`on` allows network access (the Databricks connector of `model.refresh`); `0`/`false`/`no`/`off` or unset: refused. Anything else is an error. |

Both are read through `bicraft-env` (`ServerConfig::from_env`). `bicraft serve` picks one root
per workspace itself and reads only `BICRAFT_SERVER_NETWORK` (`ServerConfig::network_from_env`),
or `--allow-network`. In code:
`ServerConfig::new(root)?.allow_network(true)`. The root is canonicalized once (symbolic links and
junctions resolved) and kept in plain form (`C:\…`, not `\\?\C:\…`) for joining and messages.

## Path rules

Every path parameter (the audit below) goes through `ServerConfig::resolve`:

1. **Lexical checks**, refused with an error naming the class:
   empty; longer than 4096 bytes or 256 components; a NUL byte; UNC or device paths
   (`\\server\share`, `//server/share`, `\\?\…`, `\\.\…`); a drive letter (`C:…`, also on
   Linux, so a workspace means the same everywhere); an absolute path (`/…`, `\…`); any `..`
   component (refused outright, not normalized away: no ambiguity with links).
   On Windows also: any `:` (alternate data streams such as `x.pbix:s` or `::$DATA`), reserved
   device names in any component and with any extension (`CON`, `PRN`, `AUX`, `NUL`, `CONIN$`,
   `CONOUT$`, `COM0-9`, `LPT0-9`, and the superscript-digit forms), and components ending in a
   dot or a space (Windows strips them, so they alias another name).
   `/` and `\` both separate components on every OS; `.` components and repeated separators are
   dropped.
2. **Canonical check.** The components are walked from the canonical root: each existing entry
   is canonicalized (links and junctions resolved) and must stay under the canonical root. The
   first entry that does not exist ends the walk; the rest is appended as given (save targets).
   An entry that exists but cannot be resolved (a dangling link, no permission, a file used as a
   folder) is refused, so a dangling link cannot be written through to a target created later
   outside the root.
3. The command gets the resolved absolute path in place of the client's value
   (`sandbox::prepare`, called by `Session::execute` before the command runs).

**Projects.** A PBIP project reads and writes many files below the path it is given:

- `file.open` of a project checks the project folder and walks its item folders
  (`.SemanticModel`, `.Report`; at most 200 000 entries, 64 levels): a link whose target leaves
  the root refuses the open. `bicraft_pbip::locate` only picks child folders of the project
  folder (it does not follow `datasetReference.byPath`), so file contents cannot point elsewhere.
- `project.new`, `file.save_as` to a `.pbip` and `file.convert` to a `.pbip` check
  `<dir>/<name>.pbip`, `<name>.SemanticModel` and `<name>.Report` the same way before writing.
- `model.refresh` looks for a `.gitignore` above the project (a warning heuristic) only up to the
  workspace root in server mode.

**`table.import {csv}`** is a file path or CSV text. In server mode a single-line value is
resolved under the root and read when it is an existing file there; a value that plainly names a
file outside (absolute, drive, UNC/device, `..`, NUL) is refused; anything else (`a,b`, or on
Windows `time,12:30`) is CSV text and is never looked up relative to the process' directory.

### Residual risks

- **TOCTOU.** The check and the use are separate system calls. A process that can change the
  workspace between them (replace a folder by a link) can escape. Commands use the resolved path
  right after the check, and the only writer of a workspace volume should be BIcraft itself:
  uploads must create plain files only (never links, never archive extraction that keeps links),
  and no other pod may mount the volume (`ReadWriteOnce`, one PVC per workspace).
- **Links planted later** inside a project (after `file.open`) are not re-checked by `file.save`
  or by sibling files a save touches (`<name>.bak`, `<name>.bicraft-tmp`). Same mitigation.
- **Hard links** cannot be told apart from plain files; a hard link to a file outside the root
  would be read or written. Uploads never create them.
- **Fonts**: `report.render` scans the system font folders unless `system_fonts: false`
  (`BICRAFT_FONT_DIRS` too). They are operator-controlled, read-only and only parsed as fonts.
- **Spill files** go to `BICRAFT_SPILL_DIR` (else the temp folder): operator-controlled.

## Command policy

Each row of the command table (`commands.rs`) carries `server: ServerPolicy`:
`ServerPolicy::ALLOWED`, `ServerPolicy::paths(&[...])`, `ServerPolicy::denied(reason)` or
`ServerPolicy::network(&[...])`. The field is mandatory, so a new command has to decide.
`bicraft_engine::server_policy(id)` reads it.
`Session::command_available(id)` says whether a command may run in the session's mode, and
`Session::available_commands(filter)` is `list_commands` without the denied ones.

- **Denied** commands are left out of `list_commands` (and of `tools/list` when a convenience
  tool maps to one) over `bicraft serve`, and fail before running with "`<id>` is not available in server mode: …" (the
  failure is recorded in `last_command` like any other).
- **Network** commands run, and check `ServerConfig::network_allowed` where they would connect.

### Audit: process launch and network access (2026-10-26, 133 commands)

Method: every command's `run` function and the modules it reaches were searched for process
creation (`std::process::Command`, `spawn`), sockets (`TcpStream`, `TcpListener`, `UdpSocket`),
HTTP/TLS clients and filesystem calls. In the whole workspace only `engine/desktop_open.rs`
(process), `databricks/api.rs` (HTTPS), `view` (the viewer's HTTP server) and two CLI-only
modules (`setup/doctor.rs`, `view_cmd.rs`) touch processes or sockets.

| Command | Process | Network | Server policy |
|---|---|---|---|
| `file.open_in_desktop` | launches the OS handler (`explorer.exe`, `open`, `xdg-open`) | no | **denied** |
| `model.refresh` | no | Databricks (HTTPS to the workspace host, presigned links with `external_links`) when `allow_connectors` names it | **network**: refused unless `BICRAFT_SERVER_NETWORK` / `allow_network` |
| the 131 others | no | no | allowed (paths sandboxed where they take one) |

The viewer's `POST /api/open-in-desktop` launches through `Session::desktop_target`; in server
mode that returns no path ("not available"), so the route answers 409 and launches nothing even
before the viewer gets its own server mode (stage 2).

Power Query evaluation (`table.import {m}`, structural saves, `model.refresh`, the viewer) reads
files only under `data_root` (`bicraft-data`'s own canonical sandbox) and runs no database or web
connector except Databricks through `model.refresh`; `data_root` itself is a sandboxed path.

### Audit: path parameters (2026-10-26)

Method: every `str_param`/`opt_str`/`get` of the commands, every `PathBuf::from`/`Path::new`
and every `std::fs` call in non-test engine code, and every params doc mentioning a path, file
or folder. The test `every_path_parameter_is_declared_and_audited` pins this list against the
command table.

| Command | Parameter | Kind | Notes |
|---|---|---|---|
| `file.open` | `path` | path | PBIX/PBIT/PBIP/model.bim; a project is tree-checked |
| `file.open` | `data_root` | path or null/"" | Power Query `File.Contents` root |
| `file.open` | `cache_abf` | path or null/"" | read-only explicit `cache.abf` |
| `file.set_cache_abf` | `cache_abf` | path or null/"" | |
| `file.save` | `data_root` | path or null/"" | writes back to the open file (inside the root) |
| `file.save_as` | `path` | path | target; PBIP targets are tree-checked |
| `file.save_as` | `data_root` | path or null/"" | |
| `file.convert` | `from`, `to`, `data_root` | delegated | resolved by the private server-mode session (`file.open`, `file.save_as`) |
| `project.new` | `dir` | path | target folder; tree-checked |
| `project.test` | `path` | delegated | opened by a private server-mode session (`file.open`); the open file is left alone |
| `project.test` | `tests`, `base_dir` | path(s) | a test file (inline tests are not paths), the folder results are relative to; BPA rule files named inside the test file are resolved under the root relative to it, and the test file found next to a project must be under the root |
| `env.apply`, `env.list`, `env.check` | `env_file` | path(s) | a profile file (inline profiles are not paths); the `bicraft.env.yaml` found next to a project must be under the root. `sources.rebase` takes no path: `from`, `to` and the mappings are text in M, not files of the server |
| `model.storage_stats` | `cache_abf` | path or null/"" | |
| `model.export_vpax` | `path` | path | target |
| `model.export_vpax` | `cache_abf` | path or null/"" | |
| `model.analyze` | `rules` | path or list of paths | inline rule objects untouched |
| `model.usage`, `model.unused`, `model.remove_unused` | `reports` | path or list of paths | read only (report parts); PBIP folders tree-checked like `file.open`; results name a report by its file name, never by its path |
| `table.import`, `table.set_data` | `csv` | path or text | see above |
| `table.import`, `table.set_data` | `data_root` | path or null/"" | |
| `report.render` | `out` | path | a PNG file or a folder; page files are named from slugs (`[A-Za-z0-9_-]`) |
| `report.render` | `font_dirs` | list of paths | |
| `report.render` | `cache_abf` | path or null/"" | |
| `theme.import` | `path` | path | read |
| `theme.export` | `path` | path | target |
| `dax.evaluate` | `cache_abf` | path or null/"" | |
| `model.refresh` | `data_root` | path or null/"" | writes `.pbi/bicraft-data.bin` inside the open project |
| `visual.import_custom` | `path` | path | read (a `.pbiviz`; `data` takes its bytes instead) |

Not path parameters, checked: `theme.set {theme}` (JSON), `report.custom_visuals` and the report's
images (read from the open package or project only; `image.add`, `image.set` and
`visual.import_custom` declare their `path` parameter), `file.open_in_desktop`
(never a caller path), `dax.memory` (spill folder from the environment).

## Tests

`crates/engine/src/sandbox_tests.rs`:

- every declared path parameter of every command refuses every class (absolute, drive letter,
  `..`, UNC, device, NUL; on Windows also alternate data streams and reserved names), and
  nothing is written under the root;
- `table.import`/`table.set_data {csv}` and `file.convert {from, to, data_root}` per class;
- accepted relative paths for every path-taking command (files land under the root, results
  name resolved paths);
- link escapes: a folder link, a file through it, a new file through it, a dangling link, a
  project item folder linked outside, a link deep inside a project, writing a project over a
  linked item. Links are made with `symlink` (Unix) or `symlink_dir`, else a junction
  (`mklink /J`, no privilege needed) on Windows; a test prints a note and skips the part it
  cannot set up;
- policy: `file.open_in_desktop` refused (nothing launched, also not with `dry_run`),
  `desktop_target` empty, the Databricks connector refused without the network setting;
- configuration parsing; local mode unchanged (no sandbox error for the same inputs).

## Wired into `bicraft serve` (2026-10-26)

`bicraft_mcp::http::new_session(dir, allow_network)` builds each workspace's session with
`Session::server(ServerConfig::new(dir)?.allow_network(..))`: the workspace folder is the root,
so tool paths are relative to it and two workspaces cannot reach each other's files. A
workspace folder must really be the server root's direct child named exactly as the id
(canonicalized: a link or junction named like a workspace that points to another workspace or
outside the root, or another spelling of an id on a case-insensitive file system, is answered
404). Network access follows `bicraft serve --allow-network` or `BICRAFT_SERVER_NETWORK` (default
off; the operator may enable the Databricks refresh). Over HTTP, `list_commands` and `tools/list`
hide denied commands and calling one anyway returns the policy error. The stdio server and the
CLI keep local sessions. Tests: `crates/mcp/tests/http.rs` (absolute paths, `..`, link escape,
render output, batch, cross-workspace access, workspace alias links, denied command hidden and
refused, Databricks refresh with the network setting off and on).

Since the authentication lot (`docs/auth.md`), every request to a workspace is authenticated
before the workspace folder is looked up, and the ACL decides which workspaces an identity may
open (a closed one answers 404 like a missing one). The sandbox is unchanged: authentication says
*who* may reach a workspace, the sandbox keeps every caller inside it.
Each command's policy also carries an `effect` (`Effect::Mutating` by default, `ReadOnly`,
`Opens`) that decides what a caller with a `reader` grant may run (`docs/auth.md` "Roles";
`Session::reader_check`, `Session::reader_commands`).

## Files over HTTP (2026-10-26)

`bicraft serve` exposes each workspace folder at `/w/<id>/files/` (`docs/web-deployment.md`,
"Operating `bicraft serve`"). Every path of those routes goes through the same
`ServerConfig::resolve` as tool paths (lexical classes 400, a link leaving the workspace 403), and
the writes keep the residual-risk rules above: uploads create plain files only (`create_new`
temp file in the reserved `.bicraft-tmp/`, then a rename), never links; a zipped project is
extracted by `bicraft_opc::for_each_zip_entry`, which refuses link and special entries, zip-slip
names and ZIP bombs, into a staging folder whose every name is resolved by a `ServerConfig`
rooted there, then moved into place. A link in the last place of a write or a deletion is acted
on itself (a deletion removes the link, an upload replaces it), never followed. Downloads replace
"open in Desktop" for getting a saved file out.

## Paths in answers (2026-10-28)

Over HTTP no answer names a server folder: tool results, tool errors, JSON-RPC and file-route
errors and the viewer's snapshots carry paths relative to the workspace, as clients give them.
The engine keeps reporting resolved paths (the CLI and stdio MCP, local mode, are unchanged:
absolute paths are what a local user wants); the HTTP layer rewrites every string of an answer
(`crates/mcp/src/http/paths.rs`): the workspace folder in each spelling it can take (as given,
canonical, without Windows' verbatim `\?\` prefix, with either separator) followed by a separator
is removed, the folder alone becomes `.`, and the server root is removed the same way where a
message names another place under it (`<workspace>/…`). A folder that only shares a prefix with
the workspace (`demo2` next to `demo`) is left to the root rule. `crates/mcp/tests/http_paths.rs`
runs every engine command (with no parameters and with existing, missing, broken, outside and
output paths), the path-naming tools, the viewer's state and the file routes, and fails on any
answer containing the server's folder.

## Left for later lots
- A memory budget mandatory in server mode, caps on DAX `timeout_ms` and batch length (security
  item 3) are separate.
