# Release packaging

`cargo xtask dist` builds BIcraft's two release binaries and packs them into versioned archives,
on this machine:

- `bicraft` (`bicraft.exe`), the desktop app (`apps/bicraft-desktop`, [desktop.md](desktop.md));
- `bicraft-cli` (`bicraft-cli.exe`), the command-line tool and MCP server (`apps/bicraft-cli`,
  [mcp.md](mcp.md)).

The names and the Linux layout follow the sibling app DeckCraft. Nothing is uploaded: `dist` has
no release step. The GitHub workflow `.github/workflows/release.yml` runs the same `cargo xtask
dist` for a pushed `v*` tag and attaches the archives to a draft GitHub release
([below](#github-release-workflow)). The first release is v0.1.0.

## Building a release

1. Set the version in the workspace `Cargo.toml` (`[workspace.package] version`). Both programs
   take it from there; `bicraft-cli --version` prints it with the git commit the binary was built
   from, `bicraft --version` prints `bicraft <version>`.
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
| Build | `cargo build --profile dist --locked -p bicraft-cli --bin bicraft-cli -p bicraft-desktop --bin bicraft --target x86_64-pc-windows-msvc`, C runtime linked statically (`-C target-feature=+crt-static` for that target only), icon and version resources required (`BICRAFT_REQUIRE_WINRES=1`) | the same two binaries for `x86_64-unknown-linux-gnu`, inside WSL Ubuntu: xtask runs `cargo xtask dist --only linux` there through the `xtask wsl` mechanism, with the Linux-side target dir `~/target/bicraft` (or `$BICRAFT_WSL_TARGET_DIR`, read on the Windows side; `~/` means the WSL home) |
| Package | `bicraft-<version>-windows-x64.zip` | `bicraft-<version>-linux-x64.tar.gz` |
| Smoke test | extracted with Windows' own `tar.exe` | extracted with `tar` in WSL |

On Linux, `cargo xtask dist` builds the Linux archive only (the Windows binaries need MSVC).

Every release `bicraft-cli` embeds the browser app ([webui.md](webui.md)): `cargo xtask dist`
first runs `cargo xtask webui` (needs the `wasm32-unknown-unknown` target and the wasm-bindgen
CLI of Cargo.lock's version, `BICRAFT_WEBUI_TOOLS`), or takes a bundle with `--webui DIR`,
builds with `BICRAFT_WEBUI_DIR` and `BICRAFT_REQUIRE_WEBUI=1`, and refuses a binary whose
`bicraft-cli build-info` reports `"embedded": false`. It also refuses a `bicraft-cli` or a
`bicraft` whose `--version` does not name the workspace version.

The build uses the `dist` profile of the workspace `Cargo.toml`: the `release` profile (one
codegen unit, symbols stripped) with fat LTO instead of thin. The binaries land in
`<target-dir>/<triple>/dist/`. `--locked` refuses a `Cargo.lock` that does not match the
manifests.

## What is inside

Each archive holds one folder, `bicraft-<version>-<os>-x64/`.

**Windows** (`bicraft-<version>-windows-x64.zip`), everything at the top of that folder:

| File | |
|---|---|
| `bicraft.exe` | the desktop app (a Windows GUI program; `bicraft.exe --register` adds a Start menu entry and Open with, for the current user) |
| `bicraft-cli.exe` | the CLI and MCP server |
| `install.ps1` | from `packaging/windows/`: copies both programs to `%LOCALAPPDATA%\bicraft\bin` (or `-Destination DIR`), runs `bicraft-cli.exe --version`, and prints how to add the folder to `PATH` and register the MCP server. It never changes `PATH`, the registry or client configurations, and does not run `--register`. |
| `README.md` | generated: version line, requirements, contents, install, MCP client configuration, checksum check |
| `docs/mcp.md` | MCP tools reference and client configuration (this repository's file) |
| `visuals/` | BIcraft's Power BI visuals (below) |
| `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`, `THIRD_PARTY_NOTICES.md`, `licenses/`, `THIRD_PARTY_CRATES.md` | the licences and notices (below) |

**Linux** (`bicraft-<version>-linux-x64.tar.gz`), laid out as an installation prefix:

| File | |
|---|---|
| `bin/bicraft` | the desktop app |
| `bin/bicraft-cli` | the CLI and MCP server |
| `install.sh` | from `packaging/linux/`: copies `bin/` and `share/` under a prefix (default `~/.local`, or its first argument), writes the menu entry with the app's absolute path, refreshes that prefix's MIME and menu caches when `update-mime-database` and `update-desktop-database` are installed, runs `bicraft-cli --version`, and prints how to add `PREFIX/bin` to `PATH` and register the MCP server. It never changes `PATH`, shell profiles, default applications or client configurations. |
| `share/applications/io.github.digorpeptra.bicraft.desktop` | the menu entry (`packaging/linux/bicraft.desktop`): `Exec=bicraft %f`, the three file types below |
| `share/icons/hicolor/<size>/apps/io.github.digorpeptra.bicraft.png` | the app icon at 16, 24, 32, 48, 64, 128 and 256 px, drawn by `dist` from `assets/logo/bicraft-icon.svg` (the icon the Windows executable embeds); `scalable/apps/io.github.digorpeptra.bicraft.svg` is that file |
| `share/mime/packages/io.github.digorpeptra.bicraft.xml` | the file types the app opens (`packaging/linux/bicraft-mime.xml`): `application/x-pbix` (`*.pbix`, a ZIP), `application/x-pbit` (`*.pbit`, a ZIP), `application/x-pbip` (`*.pbip`, JSON) |
| `share/metainfo/io.github.digorpeptra.bicraft.metainfo.xml` | the AppStream metadata (`packaging/linux/bicraft.metainfo.xml`, the release version filled in by `dist`) |
| `share/doc/bicraft/` | `README.md` (generated, as on Windows), `mcp.md` (the MCP reference), `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`, `THIRD_PARTY_NOTICES.md`, `licenses/`, `THIRD_PARTY_CRATES.md` |
| `share/bicraft/visuals/` | BIcraft's Power BI visuals (below) |

`io.github.digorpeptra.bicraft` is the app's freedesktop id, after the public repository
github.com/digorpeptra/BIcraft. The desktop app gives its window the same id (the Wayland app id
and the X11 class, `DESKTOP_ID` in `apps/bicraft-desktop/src/register.rs`), so the desktop
matches the window with its menu entry and icon; on Windows the window keeps the
AppUserModelID `BIcraft.Desktop` of its Start menu shortcut.

Common to both:

- **Visuals.** BIcraft's fourteen Power BI visuals as `.pbiviz` files (ROADMAP M12.5), written
  by the release `bicraft-cli visual export --all` while the archive is made, for Desktop's
  Import a visual from a file and for organizational visuals; their README lists them and says
  where their TypeScript sources are in this repository (`crates/engine/src/spec_visual/`,
  `bvis/`, `chart_pack/`, `visual_runtime/`). Not published on the Microsoft Marketplace
  (AppSource) or the Microsoft Store, by the maintainers' decision of 2026-11-23.
- **Licences and notices.** `LICENSE-MIT`, `LICENSE-APACHE` and `NOTICE` are BIcraft's license
  and required notices; `THIRD_PARTY_NOTICES.md` describes the data bundled in the binaries
  (this repository's file) and `licenses/` holds their license files (XPress9 port, fonts, PBIR
  schemas, report theme schema, best-practice rules, CLDR, each with its `SOURCE.txt` where the
  repository has one). `THIRD_PARTY_CRATES.md` is generated: every third-party Rust crate
  compiled into either program, with version, license expression, repository, and the license
  files the crate ships, each distinct text printed once.

`THIRD_PARTY_CRATES.md` is computed from `cargo metadata --locked --filter-platform <target>`:
the resolved graphs from `bicraft-cli` and `bicraft-desktop` following normal and build edges
(dev-dependencies are not shipped), merged, so the Windows and Linux lists differ by their
platform crates. The license texts are read from each crate's folder in the cargo registry.

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

1. `bicraft-cli --version` prints `bicraft-cli <version>` (and the commit), `bicraft --version`
   prints `bicraft <version>`.
2. `bicraft-cli new-project` writes a PBIP project and `bicraft-cli validate-model` reports it
   valid.
3. `bicraft PROJECT.pbip --render app.png --size 800x500`: the desktop app opens that project and
   draws its whole window offscreen (no window, no GPU, no display) into a PNG.
4. `bicraft-cli mcp`: `initialize` answers with `serverInfo` `bicraft` and the version,
   `tools/list` lists the tools, and the server exits when stdin closes.
5. The install script installs into a scratch folder (never the real `%LOCALAPPDATA%\bicraft\bin`
   or `~/.local`), and both installed programs run. On Linux, the installed menu entry runs the
   installed app by its absolute path, and the icon, the MIME package, the metainfo file, the
   README and the visuals are in place.
6. The visuals folder holds its README and `.pbiviz` packages (ZIP files).

## GitHub release workflow

`.github/workflows/release.yml` runs on a pushed `v*` tag (or by hand for an existing tag). One
job per archive runs `cargo xtask dist --only windows|linux --out "$RUNNER_TEMP/dist"`, so CI
and a local release build, package and smoke-test the same archives:

- **windows** on `windows-latest` (MSVC);
- **linux** on `ubuntu-22.04`, so the binaries need only the glibc of that release (2.35) or
  older; `x86_64-unknown-linux-gnu` needs no system development package: the window libraries
  (Wayland, X11, xkbcommon) and the graphics drivers (Vulkan, OpenGL) are loaded at run time, and
  the file dialog talks to the XDG desktop portal over D-Bus in Rust.

Each job installs the stable toolchain with the target and `wasm32-unknown-unknown`, then the
wasm-bindgen CLI of Cargo.lock's version, which `dist` finds through `BICRAFT_WEBUI_TOOLS`, and
checks that the archive carries the tag's version. The release job collects both archives,
writes `SHA256SUMS` and creates a **draft** release for the tag; a maintainer publishes it.

## Version and commit

`apps/bicraft-cli/build.rs` reads the commit from `.git` (no `git` process; inside WSL, git
refuses a checkout on `/mnt/c` owned by another user) and `bicraft-cli --version` prints
`bicraft-cli 0.1.0 (commit 0123456789ab)`. `BICRAFT_GIT_COMMIT` overrides it, for a build from a
source archive; without `.git` and the variable the line is `bicraft-cli 0.1.0`. The desktop app
prints `bicraft 0.1.0`. The MCP `serverInfo` is `bicraft` with the plain version.

## Binary sizes and portability

Measured on 2026-11-28 (version 0.1.0, `dist` profile, the archives of a local `cargo xtask dist`;
the Linux build in WSL Ubuntu with glibc 2.43):

| | Windows x64 | Linux x64 |
|---|---|---|
| `bicraft-cli` | 69.4 MB (static CRT) | 58.1 MB |
| `bicraft` (desktop app) | 80.6 MB (static CRT) | 70.2 MB |
| archive | 58.7 MB (`.zip`) | 57.7 MB (`.tar.gz`) |

Both Linux binaries name only `libc.so.6`, `libm.so.6` and `libgcc_s.so.1` as shared libraries
(`readelf -d`). On 2026-10-15 the CLI alone was 22.0 MB (Linux, musl) and 25.9 MB (Windows) in the
`dist` profile; the bundled fonts, the browser app and the visuals added since account for most of
the growth (estimated, not measured file by file).


**Windows.** With the static CRT the binaries import only system DLLs (on 2026-10-15 the CLI imported `KERNEL32`, `ntdll`, `WS2_32`,
`bcryptprimitives` and `api-ms-win-core-synch`; the desktop app's imports have not been listed); without it, they also need `VCRUNTIME140.dll` and the UCRT forwarders, which a
fresh Windows may lack.

**Linux.** The archive is built for `x86_64-unknown-linux-gnu`: both binaries link glibc
dynamically, as DeckCraft's do. `dist` reads the highest `GLIBC_x.y` symbol version the binaries
name and writes it into the archive's README as the oldest glibc they run on; musl-based
distributions (Alpine) are not supported. The release workflow builds on Ubuntu 22.04 (glibc
2.35); a local `dist` in a newer WSL Ubuntu needs that Ubuntu's glibc or close to it (single
precision math such as `atan2f` and `acosf` got new symbol versions in glibc 2.43, and the linker
binds the newest one). Until 2026-11-27 the Linux archive held only a static musl CLI; the
container image still builds its own static musl `bicraft-cli` ([kubernetes.md](kubernetes.md)).

The desktop app needs a graphical session at run time: Wayland or X11 with libxkbcommon, and a
Vulkan or OpenGL (EGL) driver, which it loads when it starts; any desktop distribution has them.
It has been built and its offscreen render checked in WSL Ubuntu (the smoke test above), but not
yet run in a full Linux desktop session.

## Not done

- No arm64 or macOS archive.
- No code signing of the Windows binaries (SmartScreen may warn on first run).
- No distribution package (`.deb`, `.rpm`, Flatpak): the Linux archive and its `install.sh` only.
