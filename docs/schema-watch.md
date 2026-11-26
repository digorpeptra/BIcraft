# Format version watch (`cargo xtask schema-watch`)

Tells when Microsoft publishes newer versions of the formats BIcraft writes or reads. It needs
the network, so it is **not** part of `cargo xtask ci`: run it by hand, for example at the start
of a milestone or after a Power BI Desktop release.

```
cargo xtask schema-watch            # table, plus a diff of every newer published version
cargo xtask schema-watch --json     # the same as JSON
cargo xtask schema-watch --no-diff  # skip the schema downloads (the two listings only)
```

The HTTP client (`ureq`, rustls) is behind the xtask `net` feature. Without it, the command
re-runs xtask with `--features net` in `<CARGO_TARGET_DIR>/xtask-net`, so the other xtask
commands never build it. Each run makes two unauthenticated GitHub API calls (60 per hour are
allowed) and reads schema bodies from raw.githubusercontent.com.

## What is compared

| Ours | Theirs |
|---|---|
| `bicraft-versions` (`crates/versions`): `WRITTEN`, the `$schema` version of every file the writers create (pbip, pbir, pbixwriter, engine use these constants). `KNOWN`, the Power BI schemas we know but do not write. It also lists the file `version` values, the compatibility level, the PBIX `/Version` range and the Mashup version. These have no published schema; they are documented in pbix-package.md §4/§6, are printed for reference, and are not checked online. | The git tree of [microsoft/json-schemas](https://github.com/microsoft/json-schemas), `fabric/<key>/<version>/schema.json` |
| The PBIR schemas vendored for validation, `crates/pbir/schemas/<name>/<version>/` (newest per name) | same |
| The report theme schema vendored in `crates/visuals/schemas/` (`REPORT_THEME_SCHEMA_VERSION`) | `Report Theme JSON Schema/` in [microsoft/powerbi-desktop-samples](https://github.com/microsoft/powerbi-desktop-samples) |

The latest Power BI Desktop version is not checked: the Download Center page refuses scripted
requests, and we found no other stable public source. A new report theme schema tracks Desktop's
monthly releases, so it serves as the signal.

## Statuses and what to do

| Status | Exit | Meaning | What to do |
|---|---|---|---|
| up to date | 0 | Our version is the newest published. | Nothing. |
| newer minor | 0 | Same major, newer minor or patch. Desktop reads our files unchanged (the loader accepts any minor of a known major) and our validator checks newer files against the newest minor we vendor. | Read the diff. To write the new version: vendor it under `crates/pbir/schemas/` (and `schema_files.rs`), bump the constant in `crates/versions/src/lib.rs`, update the pinned tests, and check the output in Desktop. |
| ahead of published | 0 | We vendor or know a version newer than the public one. | Nothing. |
| not published | **1** if written | The key, or the exact version we write, is not (or no longer) published. | Check the CHANGELOG upstream and choose a published version. |
| NEWER MAJOR | **1** if written or vendored | A new major. Our validator rejects files that declare it, and older Desktops refuse it. | Vendor it, extend the reader and validator, and decide when the writer moves (pbip.md, "Versions chosen"). |
| new schema | **1** | A schema appeared under `pbip/`, `gitIntegration/`, `item/report/`, `item/semanticModel/` or `item/version` that is in neither `WRITTEN` nor `KNOWN`: a new Power BI project file type. | Find what writes it (save a project in Desktop and inspect the files), then add it to `KNOWN`, or to `WRITTEN` with a writer. |
| (network or setup failure) | **2** | A listing could not be fetched or parsed, the GitHub rate limit was hit, or the `net` build failed. | Retry later. |

Known-only rows (`KNOWN`) show their status for information and never set exit 1. Schemas of
other Fabric item types are listed as "not Power BI Desktop files" (the JSON output names each
one).

The diff lists, for each newer version, the property paths that were added or removed
(definitions as `#Name`), required properties, enum members and referenced schema files. Changes
to descriptions or constraints alone show as "no property, required, enum or $ref change".

## Last run

2026-10-14, json-schemas tree `49f4a76`: everything up to date except
`gitIntegration/platformProperties` (we write 2.0.0, 2.1.0 is published). The only change is a new
optional `metadata.sensitivityLabelId`, already noted in pbip.md, so we keep writing 2.0.0. Exit 0.
