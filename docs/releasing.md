# Release packaging

`cargo xtask dist` builds the `bicraft` release binaries and packs them into versioned archives,
on this machine. Nothing is uploaded: `dist` has no release step. (The GitHub workflow
`.github/workflows/release.yml` does the same for a pushed `v*` tag; no release has been
published yet.)

Archives ship the `bicraft` binary.

## Building a release

1. Set the version in the workspace `Cargo.toml` (`[workspace.package] version`). The CLI takes
   it from there; `bicraft --version` prints it with the git commit the binary was built from.
2. On Windows, from the repository:

   ```sh
   cargo xtask dist                       # both archives into ~/bicraft-dist
   cargo xtask dist --out D:\releases     # another folder (or set BICRAFT_DIST_DIR)
   cargo xtask dist --only windows        # one target
   cargo xtask dist --no-build            # repackage the release binaries already built
   cargo xtask dist --no-smoke            # skip the checks of the extracted archives
   ```

   The output folder must be outside the repository (a checkout in a synced folder would sync the
   archives).

What it does:

| Step | Windows archive | Linux archive |
|---|---|---|
| Build | `cargo build --profile dist --locked -p bicraft-cli --target x86_64-pc-windows-msvc`, C runtime linked statically (`-C target-feature=+crt-static` for that target only) | `--profile dist` for `x86_64-unknown-linux-musl`, inside WSL Ubuntu: xtask runs `cargo xtask dist --only linux` there through the `xtask wsl` mechanism, with the Linux-side target dir `~/target/bicraft` (or `$BICRAFT_WSL_TARGET_DIR`, read on the Windows side; `~/` means the WSL home) |
| Package | `bicraft-<version>-windows-x64.zip` | `bicraft-<version>-linux-x64.tar.gz` |
| Smoke test | extracted with Windows' own `tar.exe` | extracted with `tar` in WSL |

On Linux, `cargo xtask dist` builds the Linux archive only (the Windows binary needs MSVC).

Every release binary embeds the browser app (docs/webui.md): `cargo xtask dist` first runs
`cargo xtask webui` (needs the `wasm32-unknown-unknown` target and the wasm-bindgen CLI of
Cargo.lock's version, `BICRAFT_WEBUI_TOOLS`), or takes a bundle with `--webui DIR`, builds with
`BICRAFT_WEBUI_DIR` and `BICRAFT_REQUIRE_WEBUI=1`, and refuses a binary whose `bicraft build-info`
reports `"embedded": false`.

The build uses the `dist` profile of the workspace `Cargo.toml`: the `release` profile (one
codegen unit, symbols stripped) with fat LTO instead of thin. The binary lands in
`<target-dir>/<triple>/dist/`. `--locked` refuses a `Cargo.lock` that does not match the
manifests.

## What is inside

Each archive holds one folder, `bicraft-<version>-<os>-x64/`:

| File | |
|---|---|
| `bicraft.exe` / `bicraft` | the binary (self-contained; no runtime files) |
| `README.md` | generated: version line, install, MCP client configuration, checksum check |
| `install.ps1` / `install.sh` | from `packaging/windows/` / `packaging/linux/`: copies the binary to `%LOCALAPPDATA%\bicraft\bin` / `~/.local/bin` (or the folder given as `-Destination DIR` / first argument), runs `--version`, and prints how to add the folder to `PATH` and register the MCP server. It never changes `PATH`, the registry, shell profiles or client configurations. |
| `docs/mcp.md` | MCP tools reference and client configuration (this repository's file) |
| `visuals/` | BIcraft's fourteen Power BI visuals as `.pbiviz` files (ROADMAP M12.5), written by the release binary's `visual export --all` while the archive is made, for Desktop's Import a visual from a file and for organizational visuals; `visuals/README.md` lists them and says where their TypeScript sources are in this repository (`crates/engine/src/spec_visual/`, `bvis/`, `chart_pack/`, `visual_runtime/`). Not published on the Microsoft Marketplace (AppSource) or the Microsoft Store, by the maintainers' decision of 2026-11-23 |
| `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE` | BIcraft's license and required notices |
| `THIRD_PARTY_NOTICES.md` | the data bundled in the binary and where it comes from (this repository's file) |
| `licenses/` | the license files of that data: XPress9 port (MIT), Selawik font (OFL 1.1), PBIR schemas (MIT), report theme schema (MIT), best-practice rules (MIT, with `SOURCE.txt`), CLDR (Unicode License v3), each with its `SOURCE.txt` where the repository has one |
| `THIRD_PARTY_CRATES.md` | generated: every third-party Rust crate compiled in, with version, license expression, repository, and the license files the crate ships, each distinct text printed once |

`THIRD_PARTY_CRATES.md` is computed from `cargo metadata --locked --filter-platform <target>`:
the resolved graph from `bicraft-cli` following normal and build edges (dev-dependencies are not
shipped), so the Windows and Linux lists differ by their platform crates. The license texts are
read from each crate's folder in the cargo registry.

Archives are byte-stable for the same inputs: entries in a fixed order, timestamps fixed at
1980-01-01, no owner names, executables mode 755.

## Checksums

`SHA256SUMS` sits next to the archives, one `<sha256>  <file>` line per archive of the version
(the format of `sha256sum`). It is rewritten after each `dist` run from the archives present.

```sh
sha256sum -c --ignore-missing SHA256SUMS                                   # Linux
Get-FileHash .\bicraft-0.1.0-windows-x64.zip -Algorithm SHA256            # Windows
```

## Smoke test

Unless `--no-smoke`, each archive is extracted into `<out>/.smoke-<os>-x64/` and checked from
there; the folder is removed when every check passes and kept otherwise:

1. `bicraft --version` prints `bicraft <version>` (and the commit).
2. `bicraft new-project` writes a PBIP project and `bicraft validate-model` reports it valid.
3. `bicraft mcp`: `initialize` answers with `serverInfo` `bicraft` and the version,
   `tools/list` lists the tools, and the server exits when stdin closes.
4. The install script copies the binary into a scratch folder (never the real
   `%LOCALAPPDATA%\bicraft\bin` or `~/.local/bin`), and the copy runs.
5. `visuals/` holds its README and `.pbiviz` packages (ZIP files).

## Version and commit

`apps/bicraft-cli/build.rs` reads the commit from `.git` (no `git` process; inside WSL, git
refuses a checkout on `/mnt/c` owned by another user) and `bicraft --version` prints
`bicraft 0.1.0 (commit 0123456789ab)`. `BICRAFT_GIT_COMMIT` overrides it, for a build from a
source archive; without `.git` and the variable the line is `bicraft 0.1.0`. The MCP
`serverInfo.version` is the plain version.

## Binary sizes and portability

Measured on 2026-10-15 (version 0.1.0, Rust 1.99):

| Profile | Linux x64 binary | Windows x64 binary | Short CLI run (Linux, `validate-model` of a new project, mean of 50) |
|---|---|---|---|
| Cargo default `release` (no LTO, 16 codegen units, not stripped) | 33.5 MB | | 5.0 ms |
| workspace `release` (thin LTO, 1 codegen unit, stripped) | 23.8 MB | 27.1 MB (27.4 MB with static CRT) | 3.8 ms |
| `dist` (fat LTO) | **22.0 MB** | **25.9 MB** (static CRT) | 3.6 ms |

Archives: `bicraft-0.1.0-windows-x64.zip` 8.5 MB, `bicraft-0.1.0-linux-x64.tar.gz` 8.1 MB. A
`dist` build took 7 min on Windows and 5 min in WSL with `CARGO_BUILD_JOBS=2` (thin LTO: about
20 % less).

**Windows.** With the static CRT the binary imports only system DLLs (`KERNEL32`, `ntdll`,
`WS2_32`, `bcryptprimitives`, `api-ms-win-core-synch`); without it, it also needs
`VCRUNTIME140.dll` and the UCRT forwarders, which a fresh Windows may lack.

**Linux.** The local archive is built for `x86_64-unknown-linux-musl` (needs
`rustup target add x86_64-unknown-linux-musl` once in WSL). Since the Databricks connector
(2026-10-23) the TLS stack (rustls with `ring`) compiles a little C and assembly: install a musl C
compiler once (`sudo apt install musl-tools`, which provides `musl-gcc`); the binary stays static.
Not yet re-checked in WSL (no distribution was installed when the connector landed). The binary is `static-pie linked`: no `libc.so.6`, no glibc symbol version,
so it runs on any x86-64 Linux distribution, glibc or musl based (Ubuntu 22.04/24.04, Debian 12,
Alpine). It was first built against glibc in this WSL Ubuntu (glibc 2.43), which made it need
glibc 2.43: `atan2f` and `acosf` (single-precision math used by usvg, tiny-skia and the renderer)
got new symbol versions in glibc 2.43 and the linker binds the newest one. `dist` still prints the
glibc floor when a binary references glibc symbol versions (e.g. the GitHub workflow's
`x86_64-unknown-linux-gnu` build on `ubuntu-22.04`, glibc 2.35).

musl's allocator is slower than glibc's under heavy multithreaded allocation; the parallel
XPress9 encoder of large PBIX saves is the place to watch. A render of the 34-visual grids
report takes about 0.7 s.

## Not done

- No arm64 or macOS archive locally (the GitHub workflow has Linux arm64).
- No code signing of the Windows binary (SmartScreen may warn on first run).
