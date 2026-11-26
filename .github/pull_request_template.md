## What this changes

<!-- One or two sentences: what the code does now, and the ROADMAP item (`M<n>.<k>`) or area. -->

## How it was checked

<!-- The tests added or changed (names), and anything you looked at by hand (screenshots of UI
changes: BICRAFT_DESKTOP_SHOTS). -->

## Checklist

The rules are in [AGENTS.md](../AGENTS.md); tick each box, or say why it does not apply.

- [ ] **Never crash** (§3): no `unwrap`, `expect`, `panic!`, `unreachable!`, `todo!` or
      `unimplemented!` outside tests; input-sized allocations capped, recursion bounded, checked
      arithmetic. A crash fix comes with a regression test I saw fail without the fix.
- [ ] **Provenance** (§1): Desktop used only as a black box, no analysis of vendor binaries (no
      disassembly, decompilation, CIL or metadata reading, byte scans, reflection); no Microsoft
      code or documentation prose pasted; new format facts written as a note under
      `docs/formats/` citing a public source or a black-box observation; honest wording (never
      "clean room" or "written from scratch"); no Power BI Desktop
      timing or ratio published.
- [ ] **No client data** (§2): no real Power BI files, client names or content in code, tests,
      docs, commit messages or this description; fixtures are files we generated; no local paths.
- [ ] **Server policy reviewed for new commands**: each new or changed command declares its
      `server` policy (paths it takes, read-only or not, denied when it launches a process or
      reaches the network) and it was checked against [docs/server-mode.md](../docs/server-mode.md).
- [ ] **Every command has its button** (§9): each new command has a ribbon, menu or palette place
      in the registry, and `every_command_has_a_button` passes.
- [ ] **Assets and dependencies**: every new asset has an `ATTRIBUTION.md` row
      (`cargo xtask assets`); a new crate has a permissive licence and comes from crates.io
      (`cargo xtask deny`), and embedded material is in `THIRD_PARTY_NOTICES.md`.
- [ ] **Docs and ROADMAP updated**: the item's status in `ROADMAP.md`, a line in `docs/history/roadmap-log.md`,
      its `docs/gaps.md` row and `docs/gaps-open.md` entry, and the docs of the area touched.
- [ ] **CI green on Windows and Linux**: `cargo xtask ci` and `cargo xtask wsl ci` (or the
      GitHub jobs on both).
