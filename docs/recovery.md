# Auto-recovery and autosave

BIcraft keeps the edits made since the last save in a **journal**, so a crash, a closed window or
tab, an expired browser view or a server restart loses nothing: opening the file again offers
the edits back. **Autosave** (opt-in, PBIP projects only) saves the project itself on a timer.

Power BI Desktop's AutoRecovery writes a periodic recovery copy of the whole file and offers it
after a crash (black-box, user-facing behaviour). BIcraft's journal is cheaper: one short line per
command, and a full copy only when a line cannot replay the edit.

Code: `crates/engine/src/recovery.rs` (journal, replay, commands), `crates/ui/src/recovery.rs`
(prompt, File view, status bar, autosave), the hosts in `apps/bicraft-desktop/src/local.rs` and
`crates/mcp/src/http.rs` / `http/viewer.rs`.

## Where journals live

| Host | Folder | Owner |
|---|---|---|
| Desktop app | `%LOCALAPPDATA%\bicraft\recovery` on Windows, `$XDG_STATE_HOME/bicraft/recovery` (else `~/.local/state/bicraft/recovery`) elsewhere; `BICRAFT_RECOVERY_DIR` overrides it | the user |
| `bicraft-cli serve` | `--recovery-dir DIR` (or `BICRAFT_RECOVERY_DIR`), one subfolder per workspace; **off** without it | a hash of the identity holding the view; none for the workspace's shared session |
| CLI, `bicraft-cli mcp`, private sessions, tests | none: auto-recovery is off unless the host calls `Session::set_recovery` | — |

A journal is never written next to the user's file, nor in the repository.

**In Kubernetes.** The `bicraft` chart sets `--recovery-dir /var/lib/bicraft/recovery` on a claim
of its own (`<release>-recovery`, 1 GiB, kept on uninstall; `recovery.*` values, on by default),
outside the workspace root so the files API never lists or serves a journal. The controller gives
each workspace a second claim, `bws-<id>-recovery`, mounted at the same place and kept or deleted
with the workspace as its files are (`workspaces.recovery.*`; `status.recoveryClaim`). So an idle
scale to zero, an expired browser view or a pod restart keeps the unsaved edits. Autosave stays
off unless `recovery.allowAutosave` (controller: `workspaces.pod.allowAutosave`).

**Metrics** (`/metrics`, read when scraped): `bicraft_recovery_journals` (journals kept in
`--recovery-dir`) and `bicraft_recovery_journal_bytes` (journals and recovery copies); per
workspace with `--metrics-workspace-label` (`bicraft_workspace_recovery_journals`).

**Privacy.** A journal holds the commands' parameters: the user's own DAX, M, names and values.
They stay in the recovery folder on the user's machine (or the server's) and are never logged
or sent anywhere else. On a server, the journal header names its owner by a hash, never the
user's name or id. Deleting the folder deletes them all.

## The journal

One file per session and open file: `<path key>-<session id>.jsonl`, JSON lines.

- **Header** (first line): `{"bicraft_recovery": 1, "path", "key", "base", "owner", "created_ms",
  "pid", "view"}`. `base` is the file's identity at open or last save: size and modification time; for a
  PBIP project, the number of its item files and a hash (FNV-1a) of their names, sizes and times,
  `.pbi` folders left out (Desktop rewrites `localSettings.json` and the cache on its own). A
  model never saved has `base.kind` `untitled`, an empty `path`, a shared key and `label` (the
  model's name). `view` is the session's view settings when the journal starts (below).
- **A command**: `{"seq", "at_ms", "rev", "cmd", "params", "guids"}`, written after the command
  succeeded. `guids` are the GUIDs the command generated (lineage tags, relationship and visual
  names): a replay hands them out again, so it gives the same model and report.
- **An atomic batch** (`batch` with `atomic`): one `{"seq", "at_ms", "rev", "batch": [{"cmd",
  "params"}], "guids"}` line when it commits; nothing when it rolls back.
- **View settings**: `{"seq", "at_ms", "rev", "view": {"ui_language", "culture", "as_of",
  "page"}}`, written when `view.set_language`, `view.set_culture`, `view.set_as_of` or a page
  change (`view.interact`) changes them while the journal exists. They count as no edit, and a
  view setting alone never starts a journal (the file has nothing to recover).
- **A recovery copy**: `{"seq", "at_ms", "rev", "copy": "<journal>.copy.json"}`. The copy is the
  session's model, report (resources in hex), read errors and dirty flags in the session's own
  JSON form, written through a temporary file. The journal restarts after it (header, copy line).

Each line is one `write`, so a crash of the process loses nothing written. The file is synced to
disk (`fsync`) with the first line written more than a second after the last sync, every 256
lines, at each recovery copy and when the session goes; a crash of the machine can lose the lines
written since the last sync. A live session holds an exclusive lock on its journal (`File::try_lock`), so no other session offers or applies
it; the lock goes with the process, so a crash leaves the journal unlocked.

### What each command does to the journal

Every row of `COMMANDS` declares `replay` next to `server` and `place`, so a new command has to
decide (`every_command_declares_how_it_is_journaled` checks it: read-only commands are `Skip`,
mutating ones are `Exact`, `Copy` or `Base` unless listed as view settings, and a command reading
a file or the network is never `Exact`).

| `replay` | Commands | Journal |
|---|---|---|
| `Exact` | model, report, theme, translation and role edits, `edit.undo`, `edit.redo` (86 commands) | one line |
| `Copy` | `objects.import`, `table.import`, `table.set_data`, `model.remove_unused`, `env.apply`, `theme.import`, `model.refresh`: outside inputs (a file, a CSV, a folder, the network) | a recovery copy after it |
| `Skip` | read-only commands; `view.set_language`, `view.set_culture`, `view.set_as_of`, `dax.clear_cache`, `dax.aggregations`, `dax.memory`, `file.set_cache_abf` (view settings and caches, not the file) | nothing (the view settings and the reader's page: a `view` line when a journal exists) |
| `Base` | `file.open`, `file.save`, `file.save_as`, `project.new`, `model.new`, `recovery.apply`, `recovery.discard` | a new base (below) |

- **Undo and redo** are lines too, so a replay runs them in the same order. An undo or redo that
  would move a step made before the journal started (before the last save) cannot replay from the
  file: it writes a recovery copy instead.
- **Images read from a file** (`image.add` and `image.set` with `path`, `page.format` with
  `background_image_path` or `wallpaper_image_path`, M11.20) write a recovery copy too: the line
  would read the file again. A journal line that names such a file is refused on replay, so a
  tampered journal cannot make the session read a file; with `data` (base64) the line replays.
- **Size cap.** Past 4 MiB (`RecoveryConfig::max_bytes`), a recovery copy replaces the lines.
- **Base changes.** `file.save` deletes the journal (the file holds the edits) and takes the
  saved file as the new base; so does undoing back to the saved state (the session is clean).
  `file.save_as` deletes it too (the edits are in the new file). `file.open`, `model.new` and
  `project.new` over unsaved edits **keep** the old journal (it is offered when that file is
  opened again) and start a new one.
- **A model never saved** (`model.new`, or a file read into a new model such as a `.vpax`) is
  journaled too, under a key of its own: its first edit writes a recovery copy (there is no file
  to replay lines onto), the next ones are lines. `project.new` and `file.save_as` end that
  journal (the edits are in the new file); `model.new` over it keeps it.
- **Clean close.** `recovery.discard {current: true}` (the window's "Close without saving") and
  `Session::recovery_close` delete the session's journal. Dropping a session, quitting the app
  with unsaved edits, a crash, an expired server view or a server restart leave it.

## Recovery

`file.open` lists the journals of that file left by other sessions (same path key and owner, not
locked) as **offers**: the snapshot's `recovery.offers` and `recovery.list`. Each offer says
`base`: `same` (the file is what it was when the edits started), `changed` or `missing`.

- **Same base**: `recovery.apply` replays the journal through `Session::execute`, each command its
  own undo step, stopping at the first failure; it answers how far it got (`applied`, `total`,
  `failed: {index, command, error}`). A recovery copy, when the journal has one, replaces the
  model and report first (one undo step).
- **Changed base**: only a journal with a recovery copy is recoverable (the copy replaces the
  model and report of the file as it is now, then the lines after it replay); without one, the
  edits cannot be replayed safely and the offer explains why (`why`); `recovery.discard` removes
  it.
- **A model never saved** is never offered for the open file: `recovery.list {all: true}` (and the
  desktop's start screen) lists it with `base: "untitled"` and the model's name as `label`, and
  `recovery.apply {id}` recovers it in any session without unsaved edits, an empty one too: the
  session starts a new model, the recovery copy replaces it, the lines after it replay (`base:
  "untitled"` in the answer).
- **View settings** come back after the edits: the journal's last `view` is put back through the
  `view.*` commands (those that differ), and the answer's `view: {applied, errors}` says how
  that went.
- The open file must have no unsaved edits. Only `Exact` commands replay: a tampered journal
  cannot run a file, network or recovery command. The applied journal is deleted; the recovered
  edits are journaled again in the new session's own journal.

| Command | Does |
|---|---|
| `recovery.list {all?}` | this session's journal (`edits`, `copy`, `error`) and the offers for the open file (with `all`: every file of the same owner; a server lists its workspace's files only) |
| `recovery.apply {id?}` | replays an offer (default: the newest recoverable one for the open file) |
| `recovery.discard {id?, all?, current?}` | removes an offer, the open file's offers, or all of them; `current` drops this session's own journal |

All three are in the command palette ("Recoverable edits", "Recover unsaved edits", "Discard
recoverable edits"), the same on the desktop and in the browser.

## In the app

- **Prompt.** When the open file has an offer, a dialog says how many edits were kept and when,
  with **Recover**, **Discard** and **Later** (Escape). An offer that cannot be recovered says why
  and has Discard and Later only. A reader cannot recover or discard.
- **File view.** Home lists the files with recoverable edits (the desktop reads its recovery
  folder at start, so edits left by a crash show on the start screen; the browser asks its
  server); Open opens the file and the prompt follows. A model never saved shows as "New model,
  never saved" with **Recover**, which runs `recovery.apply {id}` in the open tab (its content is
  replaced, as New model does; it needs an open document without unsaved edits). Info has "Save
  automatically" and the journal's state.
- **Status bar.** `Recovery: N edits` while the journal holds edits (hover: what it means), or
  `Recovery journal: error` with the error when a line could not be written (the edit itself
  succeeded).
- **Closing a tab** with unsaved edits asks first; "Close without saving" also discards its
  journal.

## Autosave

Off by default. File > Info > **Save automatically**: off, every N minutes while there are
unsaved edits, or N seconds after the last edit (5 s to 1 h). It runs the ordinary `file.save`,
so it writes only the files that changed (the TMDL and PBIR writers compare before writing) and
goes through the same checks (on a server, the per-file revision check: a save over a newer file
is refused with `file_conflict` and nothing is written; the error shows and the next attempt
waits for the delay again). Every open document tab is saved on its own clock, the ones behind
the active tab too; never two saves of one document at once.

- **PBIP projects only.** PBIX and PBIT files are not offered: each save repacks the whole
  package (seconds to minutes on large files); their unsaved edits are still journaled.
- **Servers** allow it only with `bicraft-cli serve --allow-autosave` (the snapshot's
  `view.limits.autosave`); otherwise the preference says why and nothing is saved.
- The preference is the window's (kept with the other UI preferences, in the browser too).

## Cost

Measured (`journal_overhead`, release build, Windows 11, a small PBIP project, `measure.set` 2,000
times, three runs): **+1.5 to +9 µs per command** (14 to 21 µs without the journal, 22 to 25 µs
with it). An `fsync` every 16 lines cost +75 µs per command in the same test, hence one per second
or 256 lines. Writing a line is one serialization
of the parameters and one `write`; a recovery copy costs one serialization of the model and the
report.
