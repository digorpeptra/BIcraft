# The pinned corpus of our own files

ROADMAP M11.8, decision 4 of [method.md](method.md): a corpus of Power BI files **we author
ourselves** (synthetic models, made with `bicraft`, then opened and saved by Power BI Desktop as a
black box), pinned by SHA-256 and read by the tests. It is never a home for client files: those
stay local under `BICRAFT_TESTDATA` (AGENTS.md §2) and are never copied into the corpus.

## What is committed, what is not

| In this repository | Outside it |
|---|---|
| `corpus/manifest.json`: each entry's name, the file to open, how it was made (the `bicraft` steps, then what Desktop did), a content summary, its files with size and SHA-256, and the summary the tests expect | the files themselves, in a folder named by `BICRAFT_CORPUS` |
| `corpus/recipes/*.json`: the `bicraft run --file` steps of each entry | |

**Hosting (maintainers' decision, 2026-11-24).** The files are the assets of a GitHub release of
the repository `digorpeptra/BIcraft`, tag `corpus-v1`: the manifest's `base_url` is
`https://github.com/digorpeptra/BIcraft/releases/download/corpus-v1`. A release's assets have flat
names, so each file is stored under its path with every `/` written `__`
(`shop/Shop.Report/.platform` is the asset `shop__Shop.Report__.platform`); `cargo xtask corpus`
refuses a path whose asset name would not be kept as is by GitHub (anything but ASCII letters,
digits, `.`, `-` and `_`, or a leading or trailing `.`) or that two paths would share.

The repository is public; until the `corpus-v1` release is published there, `fetch` finds
nothing and the tests that need the corpus skip with a note. A copy of the same files hosted elsewhere is used by pointing `BICRAFT_CORPUS_URL`
(or `--url`) at it; without the corpus, the tests that need it skip with a note.

## Use it

```sh
export BICRAFT_CORPUS=/path/to/corpus           # the folder of the files
export BICRAFT_CORPUS_URL=https://host/corpus   # optional: another base URL than the manifest's
cargo xtask corpus fetch     # downloads missing or wrong files
cargo xtask corpus verify    # every file's size and SHA-256 against the manifest
cargo test -p bicraft-engine --test corpus -- --nocapture
```

`verify` fails on a missing file or a mismatch and prints the size or hash it found. `fetch`
downloads each file whose local copy is missing or wrong (`--url` and `--dir` override the
variables; the manifest's `base_url` is the fallback), checks the size and SHA-256 before it
writes the file, and verifies the whole corpus after. It builds xtask with its HTTP client
(`--features net`, as `schema-watch` does), so the other xtask commands stay offline.

- **A GitHub release URL** (`https://github.com/<owner>/<repo>/releases/download/<tag>`, the
  manifest's): `fetch` reads the release through the GitHub API, which also serves a private
  fork's release
  (`/repos/<owner>/<repo>/releases/tags/<tag>`, then each asset with `Accept:
  application/octet-stream`), authenticated by `GITHUB_TOKEN`, else `GH_TOKEN`, else what `gh auth
  token` prints when the GitHub CLI is signed in (`gh auth login`). The token goes in the
  `Authorization` header only: it is never printed, never put in a URL, and not sent on to the
  storage host the asset redirects to. A token needs read access to the repository's contents
  (a fine-grained token: *Contents: read*). Without a token `fetch` says so and tries the public
  download URL, which a public repository's release answers.
- **Any other base URL**: `<base>/<path>` for each file, without authentication.

The test (`crates/engine/tests/corpus.rs`) checks each entry's files (a size or hash mismatch
fails), opens it, compares its model (tables, calculated tables and columns, measures,
relationships, roles, calculation groups) and report (page names, visual types) with the
manifest's `expect`, saves it as a PBIX and as a PBIP project, and reopens both copies, which
must summarise the same. Without `BICRAFT_CORPUS`, or for an entry whose files are missing, it
prints a note and passes, so it runs everywhere, Linux included.

## Add a file

1. Write the steps as `corpus/recipes/NAME.json` (a JSON array of `{command, params}`, ending
   with `file.save_as` or `project.new`) and run them in a scratch folder outside the checkout:
   `bicraft run --file corpus/recipes/NAME.json`. Synthetic values only: no real names, no
   client material, no Microsoft sample.
2. Open the result in Power BI Desktop and save it from Desktop (Ctrl+S). Desktop is a black
   box: only keyboard input, the rendered page and screenshots (AGENTS.md §1, §8). Take the
   machine's Desktop lock first and close only the Desktop you started. For a PBIP, keep the
   files a repository would hold: leave out `.pbi/cache.abf` and `.pbi/localSettings.json`.
   A template is saved through File › Export › Power BI template (a description dialog, then
   the Save As dialog), which keyboard input reaches.
3. Copy the saved files into `BICRAFT_CORPUS` and add the entry to the manifest: the files'
   sizes and SHA-256 (`sha256sum`, or `Get-FileHash` in PowerShell, lower-cased), the Desktop
   version, what Desktop did, and a content summary.
4. Run the corpus test with `--nocapture`: it prints each entry's summary; copy it into
   `expect` after checking it describes what the recipe made (Desktop may add tables, such as
   its automatic date tables, and the entry should say so).
5. Run `cargo xtask corpus verify`, then publish the files (below).

A file is never replaced in place: a new Desktop save is a new entry or a new hash in a commit
that says why.

## Publish the files (maintainers)

Only a maintainer uploads, after reviewing what goes out; no agent or script of the repository
runs the upload. Every file must be our own synthetic content (AGENTS.md §2): no client file, no
Microsoft sample, no file Desktop made from someone else's content.

1. Stage the verified corpus under its asset names, in a new or empty folder outside the
   checkout and outside any synced folder:

   ```sh
   cargo xtask corpus stage --dir "$BICRAFT_CORPUS" --out /path/to/corpus-stage
   ```

   It runs `verify` first, copies each file as `<asset name>`, lists them with their sizes and
   SHA-256, and prints the upload command. It uploads nothing.
2. Review the staged files, then create the release with its assets (the command `stage`
   printed, of this form):

   ```sh
   gh release create corpus-v1 --repo digorpeptra/BIcraft --title "Pinned corpus (corpus-v1)" \
     --notes "Synthetic Power BI files authored by the BIcraft project, pinned by SHA-256 in corpus/manifest.json (docs/corpus.md). No client material." \
     /path/to/corpus-stage/basic.pbix /path/to/corpus-stage/shop__Shop.pbip …
   ```

   A later entry goes to the same release (`gh release upload corpus-v1 FILE… --repo
   digorpeptra/BIcraft`) when it only adds files; a corpus whose files change gets a new tag
   (`corpus-v2`) and a manifest commit pointing at it, so older commits still fetch their own
   files.
3. Check from a clean folder: `cargo xtask corpus fetch --dir /tmp/corpus-check` must download
   and verify every file.
