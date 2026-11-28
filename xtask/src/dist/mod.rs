//! `cargo xtask dist`: release archives of BIcraft's two binaries, the desktop app `bicraft` and
//! the CLI `bicraft-cli`, built and packaged locally.
//!
//! On Windows it builds `x86_64-pc-windows-msvc` itself and the Linux archive inside WSL (the
//! `xtask wsl` mechanism runs `cargo xtask dist --only linux` there). On Linux it builds the Linux
//! archive only. Nothing is uploaded. See docs/releasing.md.

mod archive;
mod notices;
mod sha256;
mod smoke;

use std::path::{Path, PathBuf};
use std::process::Command;

use archive::Entry;

const USAGE: &str = "\
usage: cargo xtask dist [--out DIR] [--only windows|linux] [--no-build] [--no-smoke] [--webui DIR]

  --out DIR      output folder (default: $BICRAFT_DIST_DIR, else ~/bicraft-dist)
  --only T       build and package one target (default: both on Windows, linux on Linux)
  --no-build     package the release binaries already built
  --no-smoke     skip the checks run from the extracted archives
  --webui DIR    the browser app bundle to embed (default: built by `cargo xtask webui`;
                 docs/webui.md). Release binaries always embed it: a binary without it fails.
";

#[derive(Clone, Copy, PartialEq, Debug)]
enum Target {
    Windows,
    Linux,
}

impl Target {
    fn triple(self) -> &'static str {
        match self {
            Target::Windows => "x86_64-pc-windows-msvc",
            Target::Linux => "x86_64-unknown-linux-gnu",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Target::Windows => "windows-x64",
            Target::Linux => "linux-x64",
        }
    }
    /// The CLI's file name.
    fn cli(self) -> &'static str {
        match self {
            Target::Windows => "bicraft-cli.exe",
            Target::Linux => "bicraft-cli",
        }
    }
    /// The desktop app's file name.
    fn app(self) -> &'static str {
        match self {
            Target::Windows => "bicraft.exe",
            Target::Linux => "bicraft",
        }
    }
    /// Where the binaries sit in the archive: the top folder on Windows, `bin/` on Linux.
    fn bin_dir(self) -> &'static str {
        match self {
            Target::Windows => "",
            Target::Linux => "bin/",
        }
    }
    /// Where the README, the licences and the MCP reference sit: the top folder on Windows,
    /// `share/doc/bicraft/` on Linux (the layout of an installation prefix).
    fn doc_dir(self) -> &'static str {
        match self {
            Target::Windows => "",
            Target::Linux => "share/doc/bicraft/",
        }
    }
    /// The MCP reference in the archive.
    fn mcp_doc(self) -> String {
        match self {
            Target::Windows => "docs/mcp.md".to_owned(),
            Target::Linux => format!("{}mcp.md", self.doc_dir()),
        }
    }
    /// The folder of BIcraft's `.pbiviz` visuals in the archive.
    fn visuals_dir(self) -> &'static str {
        match self {
            Target::Windows => "visuals/",
            Target::Linux => "share/bicraft/visuals/",
        }
    }
    fn ext(self) -> &'static str {
        match self {
            Target::Windows => "zip",
            Target::Linux => "tar.gz",
        }
    }
    fn host() -> Target {
        if cfg!(windows) {
            Target::Windows
        } else {
            Target::Linux
        }
    }
}

#[derive(Debug, PartialEq)]
struct Opts {
    out: Option<PathBuf>,
    only: Option<Target>,
    build: bool,
    smoke: bool,
    /// The browser app bundle (`cargo xtask webui` output) to embed.
    webui: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        out: None,
        only: None,
        build: true,
        smoke: true,
        webui: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => o.out = Some(PathBuf::from(it.next().ok_or("--out needs a folder")?)),
            "--only" => {
                o.only = Some(match it.next().map(String::as_str) {
                    Some("windows") => Target::Windows,
                    Some("linux") => Target::Linux,
                    _ => return Err(format!("--only needs windows or linux\n\n{USAGE}")),
                })
            }
            "--no-build" => o.build = false,
            "--no-smoke" => o.smoke = false,
            "--webui" => o.webui = Some(PathBuf::from(it.next().ok_or("--webui needs a folder")?)),
            "-h" | "--help" => return Err(USAGE.to_owned()),
            other => return Err(format!("dist: unknown argument `{other}`\n\n{USAGE}")),
        }
    }
    Ok(o)
}

fn default_out() -> Result<PathBuf, String> {
    if let Some(d) = bicraft_env::var_os("BICRAFT_DIST_DIR").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(d));
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|d| !d.is_empty())
        .ok_or("dist: no home folder; pass --out DIR")?;
    Ok(PathBuf::from(home).join("bicraft-dist"))
}

pub fn cmd(args: &[String]) -> Result<(), String> {
    let mut opts = parse(args)?;
    let out = match &opts.out {
        Some(o) => o.clone(),
        None => default_out()?,
    };
    let root = super::root();
    if out.starts_with(&root) {
        return Err(format!(
            "dist: the output folder {} is inside the repository; pick one outside it",
            out.display()
        ));
    }
    std::fs::create_dir_all(&out).map_err(|e| format!("dist: {}: {e}", out.display()))?;
    let meta = super::metadata()?;
    let version = cli_version(&meta)?;
    let target_dir = meta
        .get("target_directory")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .ok_or("cargo metadata: no target_directory")?;

    // the browser app (docs/webui.md): built once, embedded in every release binary
    if opts.build && opts.webui.is_none() {
        opts.webui = Some(super::webui::bundle(Some(target_dir.join("webui-dist")), false)?);
    }
    let targets = match opts.only {
        Some(t) => vec![t],
        None if cfg!(windows) => vec![Target::Windows, Target::Linux],
        None => {
            eprintln!("dist: the Windows archive needs a Windows host (MSVC); building linux only");
            vec![Target::Linux]
        }
    };
    for t in targets {
        if t == Target::host() {
            package(t, &opts, &out, &version, &target_dir)?;
        } else if t == Target::Linux {
            in_wsl(&opts, &out)?;
        } else {
            return Err("dist: the Windows archive can only be built on Windows".into());
        }
    }
    let sums = write_sums(&out, &version)?;
    println!("\n{}:", out.display());
    for (hash, name, size) in &sums {
        println!("  {name}  {size} bytes  sha256 {hash}");
    }
    println!("  SHA256SUMS");
    Ok(())
}

fn cli_version(meta: &serde_json::Value) -> Result<String, String> {
    meta.get("packages")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .find(|p| p.get("name").and_then(serde_json::Value::as_str) == Some("bicraft-cli"))
        .and_then(|p| p.get("version")?.as_str())
        .map(str::to_owned)
        .ok_or_else(|| "cargo metadata: no bicraft-cli package".to_owned())
}

/// Runs `cargo xtask dist --only linux` inside WSL Ubuntu.
fn in_wsl(opts: &Opts, out: &Path) -> Result<(), String> {
    let repo = super::wsl::to_wsl_path(&super::root().to_string_lossy())?;
    let out = super::wsl::to_wsl_path(&out.to_string_lossy())?;
    let mut args = vec!["dist".to_owned(), "--only".into(), "linux".into()];
    args.extend(["--out".to_owned(), out]);
    if !opts.build {
        args.push("--no-build".into());
    }
    if !opts.smoke {
        args.push("--no-smoke".into());
    }
    if let Some(w) = &opts.webui {
        args.extend([
            "--webui".to_owned(),
            super::wsl::to_wsl_path(&w.to_string_lossy())?,
        ]);
    }
    let script = super::wsl::script_args(&repo, &args);
    eprintln!("$ wsl.exe -d Ubuntu -- bash -lc {script}");
    let mut c = Command::new("wsl.exe");
    c.args(["-d", "Ubuntu", "--exec", "bash", "-lc", &script]);
    c.env_remove("CARGO_TARGET_DIR");
    super::run(c, "wsl (dist --only linux)")
}

/// Builds (unless `--no-build`), packages and smoke-tests the archive of the host target.
fn package(t: Target, opts: &Opts, out: &Path, version: &str, target_dir: &Path) -> Result<(), String> {
    if opts.build {
        let args = [
            "build",
            "--profile",
            "dist",
            "--locked",
            "-p",
            "bicraft-cli",
            "--bin",
            "bicraft-cli",
            "-p",
            "bicraft-desktop",
            "--bin",
            "bicraft",
            "--target",
            t.triple(),
        ];
        let mut c = super::cargo();
        c.args(args);
        if let Some(w) = &opts.webui {
            // the browser app is part of every release binary (docs/webui.md)
            c.env("BICRAFT_WEBUI_DIR", w);
        }
        c.env("BICRAFT_REQUIRE_WEBUI", "1");
        if t == Target::Windows {
            // the icon and version information must be in the release executable
            c.env("BICRAFT_REQUIRE_WINRES", "1");
            // Static CRT: without it the binaries need the Visual C++ runtime (VCRUNTIME140.dll),
            // which a fresh Windows does not have. Applies to the target artifacts only.
            c.env(WINDOWS_RUSTFLAGS.0, WINDOWS_RUSTFLAGS.1);
        }
        super::run(c, &format!("cargo {}", args.join(" ")))?;
    }
    let built = target_dir.join(t.triple()).join("dist");
    let read_bin = |name: &str| {
        let path = built.join(name);
        std::fs::read(&path).map(|b| (path.clone(), b)).map_err(|e| {
            format!(
                "dist: {}: {e} (build it first or drop --no-build)",
                path.display()
            )
        })
    };
    let (cli_path, cli) = read_bin(t.cli())?;
    let (app_path, app) = read_bin(t.app())?;
    eprintln!("dist: {} is {} bytes", cli_path.display(), cli.len());
    eprintln!("dist: {} is {} bytes", app_path.display(), app.len());
    let version_line = version_of(&cli_path)?;
    if !super::webui::binary_embeds(&cli_path)? {
        return Err(format!(
            "dist: {} has no browser app (build it with BICRAFT_WEBUI_DIR, docs/webui.md)",
            cli_path.display()
        ));
    }
    if !version_line.starts_with(&format!("bicraft-cli {version}")) {
        return Err(format!(
            "dist: {} reports `{version_line}`, expected bicraft-cli {version}",
            cli_path.display()
        ));
    }
    let app_line = version_of(&app_path)?;
    if !app_line.starts_with(&format!("bicraft {version}")) {
        return Err(format!(
            "dist: {} reports `{app_line}`, expected bicraft {version}",
            app_path.display()
        ));
    }

    let requires = match t {
        Target::Windows => {
            "Windows 10 or later, x64. Nothing else to install: the C runtime is linked statically."
                .to_owned()
        }
        Target::Linux => {
            let floor = glibc_floor(&cli).max(glibc_floor(&app));
            let libc = match floor {
                Some((maj, min)) => {
                    eprintln!("dist: the binaries need glibc >= {maj}.{min} (their symbol versions)");
                    format!("glibc {maj}.{min} or later (`ldd --version` prints yours)")
                }
                None => "glibc".to_owned(),
            };
            format!(
                "Linux x64 with {libc}; musl-based distributions (Alpine) are not supported. \
                 `bicraft-cli` needs nothing else. The desktop app `bicraft` also needs a graphical \
                 session (Wayland or X11, with libxkbcommon) and a Vulkan or OpenGL (EGL) driver, \
                 which it loads at start."
            )
        }
    };

    let root = super::root();
    let read = |rel: &str| std::fs::read(root.join(rel)).map_err(|e| format!("dist: {rel}: {e}"));
    let file = |path: &str, data: Vec<u8>, exec: bool| Entry {
        path: path.to_owned(),
        data,
        exec,
    };
    let bin = t.bin_dir();
    let doc = t.doc_dir();
    let mut entries = vec![
        file(&format!("{bin}{}", t.app()), app, true),
        file(&format!("{bin}{}", t.cli()), cli, true),
        match t {
            Target::Windows => file(
                "install.ps1",
                include_bytes!("../../../packaging/windows/install.ps1").to_vec(),
                false,
            ),
            Target::Linux => file(
                "install.sh",
                include_bytes!("../../../packaging/linux/install.sh").to_vec(),
                true,
            ),
        },
    ];
    if t == Target::Linux {
        entries.extend(linux_integration(version)?);
    }
    entries.push(file(
        &format!("{doc}README.md"),
        readme(t, version, &version_line, &requires).into_bytes(),
        false,
    ));
    for name in [
        "LICENSE-MIT",
        "LICENSE-APACHE",
        "NOTICE",
        "THIRD_PARTY_NOTICES.md",
    ] {
        entries.push(file(&format!("{doc}{name}"), read(name)?, false));
    }
    let crates_md = third_party_crates(t, version)?;
    entries.push(file(
        &format!("{doc}THIRD_PARTY_CRATES.md"),
        crates_md.into_bytes(),
        false,
    ));
    for (src, dest) in BUNDLED_LICENSES {
        entries.push(file(&format!("{doc}{dest}"), read(src)?, false));
    }
    entries.push(file(&t.mcp_doc(), read("docs/mcp.md")?, false));
    entries.extend(visual_entries(&cli_path, out, t)?);

    let top = format!("bicraft-{version}-{}", t.label());
    let bytes = match t {
        Target::Windows => archive::zip(&top, &entries)?,
        Target::Linux => archive::tar_gz(&top, &entries)?,
    };
    let name = format!("{top}.{}", t.ext());
    let path = out.join(&name);
    let tmp = out.join(format!("{name}.part"));
    std::fs::write(&tmp, &bytes).map_err(|e| format!("dist: {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("dist: {}: {e}", path.display()))?;
    eprintln!("dist: wrote {} ({} bytes)", path.display(), bytes.len());

    if opts.smoke {
        smoke::run(t, &path, out, &top, version)?;
    }
    Ok(())
}

/// The freedesktop application id of the Linux archive's files (`DESKTOP_ID` of
/// apps/bicraft-desktop/src/register.rs, the window's Wayland app id).
pub(crate) const APP_ID: &str = "io.github.digorpeptra.bicraft";

/// The hicolor icon sizes drawn from the app icon (those the desktop app embeds on Windows).
const ICON_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

/// The app icon, the same file the desktop app's build draws its window icon from.
const ICON_SVG: &str = "assets/logo/bicraft-icon.svg";

/// The Linux desktop integration under `share/`: the menu entry, the icons (drawn from the app
/// icon at packaging time, no committed PNG), the file types and the AppStream metadata.
fn linux_integration(version: &str) -> Result<Vec<Entry>, String> {
    let svg = std::fs::read(super::root().join(ICON_SVG)).map_err(|e| format!("dist: {ICON_SVG}: {e}"))?;
    let file = |path: String, data: Vec<u8>| Entry {
        path,
        data,
        exec: false,
    };
    let mut entries = vec![
        file(
            format!("share/applications/{APP_ID}.desktop"),
            include_bytes!("../../../packaging/linux/bicraft.desktop").to_vec(),
        ),
        file(
            format!("share/mime/packages/{APP_ID}.xml"),
            include_bytes!("../../../packaging/linux/bicraft-mime.xml").to_vec(),
        ),
        file(
            format!("share/metainfo/{APP_ID}.metainfo.xml"),
            include_str!("../../../packaging/linux/bicraft.metainfo.xml")
                .replace("@VERSION@", version)
                .into_bytes(),
        ),
    ];
    for size in ICON_SIZES {
        entries.push(file(
            format!("share/icons/hicolor/{size}x{size}/apps/{APP_ID}.png"),
            icon_png(&svg, size)?,
        ));
    }
    entries.push(file(
        format!("share/icons/hicolor/scalable/apps/{APP_ID}.svg"),
        svg,
    ));
    Ok(entries)
}

/// The SVG drawn at `size` × `size` pixels, as PNG.
fn icon_png(svg: &[u8], size: u32) -> Result<Vec<u8>, String> {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default())
        .map_err(|e| format!("dist: {ICON_SVG}: {e}"))?;
    let mut pm = tiny_skia::Pixmap::new(size, size).ok_or_else(|| format!("dist: icon size {size}"))?;
    let s = tree.size();
    let t = tiny_skia::Transform::from_scale(size as f32 / s.width(), size as f32 / s.height());
    resvg::render(&tree, t, &mut pm.as_mut());
    pm.encode_png().map_err(|e| format!("dist: icon {size} px: {e}"))
}

/// BIcraft's own visuals as `.pbiviz` files under the visuals folder (ROADMAP M12.5): written by the
/// release binary itself (`visual export --all`), with a README naming their sources.
fn visual_entries(bin: &Path, out: &Path, t: Target) -> Result<Vec<Entry>, String> {
    let dir = out.join(format!(".visuals-{}", t.label()));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("dist: {}: {e}", dir.display()))?;
    }
    let o = Command::new(bin)
        .args(["visual", "export", "--all", "--out"])
        .arg(&dir)
        .output()
        .map_err(|e| format!("dist: {} visual export: {e}", bin.display()))?;
    if !o.status.success() {
        return Err(format!(
            "dist: {} visual export --all failed: {}",
            bin.display(),
            String::from_utf8_lossy(&o.stderr)
        ));
    }
    let answer: serde_json::Value =
        serde_json::from_slice(&o.stdout).map_err(|e| format!("dist: visual export printed no JSON: {e}"))?;
    let listed = answer
        .get("files")
        .and_then(serde_json::Value::as_array)
        .filter(|l| !l.is_empty())
        .ok_or_else(|| format!("dist: visual export answered {answer}"))?;
    let text = |v: &serde_json::Value, k: &str| {
        v.get(k)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("dist: visual export: a file without `{k}`: {v}"))
    };
    let mut rows = Vec::new();
    let mut entries = Vec::new();
    for f in listed {
        let path = PathBuf::from(text(f, "path")?);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| format!("dist: visual export: no file name in {}", path.display()))?;
        let data = std::fs::read(&path).map_err(|e| format!("dist: {}: {e}", path.display()))?;
        if !data.starts_with(b"PK") {
            return Err(format!("dist: {} is not a .pbiviz package", path.display()));
        }
        rows.push((name.clone(), text(f, "display_name")?, text(f, "version")?));
        entries.push(Entry {
            path: format!("{}{name}", t.visuals_dir()),
            data,
            exec: false,
        });
    }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("dist: {}: {e}", dir.display()))?;
    eprintln!("dist: {} visuals under {}", entries.len(), t.visuals_dir());
    entries.push(Entry {
        path: format!("{}README.md", t.visuals_dir()),
        data: visuals_readme(&rows).into_bytes(),
        exec: false,
    });
    Ok(entries)
}

/// `visuals/README.md`: each file with its visual and version, and where its sources are.
fn visuals_readme(rows: &[(String, String, String)]) -> String {
    let mut table = String::from("| File | Visual | Version |\n|---|---|---|\n");
    for (file, name, version) in rows {
        table.push_str(&format!("| `{file}` | {name} | {version} |\n"));
    }
    format!(
        "# BIcraft's visuals\n\n\
         The Power BI custom visuals BIcraft writes itself, as `.pbiviz` packages: the same bytes \
         BIcraft embeds in a report, written by `bicraft-cli visual export --all`. Import one in Power BI \
         Desktop with the Visualizations pane's \"…\" › Import a visual from a file, or upload it as \
         an organizational visual. They are not published on the Microsoft Marketplace (AppSource).\n\n\
         {table}\n\
         ## Sources\n\n\
         Their TypeScript sources are in the BIcraft repository, under bicraft's license (MIT OR \
         Apache-2.0, at your option): `crates/engine/src/spec_visual/` (the Spec visual), \
         `crates/engine/src/bvis/` (histogram, bullet chart, box plot, dumbbell), \
         `crates/engine/src/chart_pack/` (organization chart, Venn, Marimekko, calendar heatmap, Sankey, Scatter+, Line / Area+, Waterfall+, Gauge / KPI+) and \
         the runtime they share, `crates/engine/src/visual_runtime/`. bicraft compiles them itself \
         (`bicraft-cli visual export NAME` writes one again).\n"
    )
}

/// Per-target rustflags of the Windows build: the C runtime linked statically.
const WINDOWS_RUSTFLAGS: (&str, &str) = (
    "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS",
    "-C target-feature=+crt-static",
);

/// License files of the data bundled in the binary: `(repository path, archive path)`.
/// THIRD_PARTY_NOTICES.md describes each one. The archive path is also a committed copy at the
/// repository root (`licenses/`, shipped in the container image); `cargo xtask docs` checks
/// that each copy equals its source.
pub(crate) const BUNDLED_LICENSES: &[(&str, &str)] = &[
    (
        "crates/xpress9/LICENSE-MIT-microsoft-xpress9",
        "licenses/xpress9/LICENSE-MIT-microsoft-xpress9",
    ),
    (
        "crates/render/fonts/OFL-Selawik.txt",
        "licenses/selawik-font/OFL-Selawik.txt",
    ),
    (
        "crates/ui/fonts/OFL-NotoSansSymbols2.txt",
        "licenses/noto-sans-symbols2/OFL-NotoSansSymbols2.txt",
    ),
    (
        "crates/render/fonts/OFL-NotoSans.txt",
        "licenses/noto-sans/OFL-NotoSans.txt",
    ),
    (
        "crates/render/fonts/OFL-NotoSansMath.txt",
        "licenses/noto-sans-math/OFL-NotoSansMath.txt",
    ),
    (
        "crates/render/fonts/OFL-NotoSansSymbols.txt",
        "licenses/noto-sans-symbols/OFL-NotoSansSymbols.txt",
    ),
    (
        "crates/render/fonts/OFL-NotoEmoji.txt",
        "licenses/noto-emoji/OFL-NotoEmoji.txt",
    ),
    (
        "crates/render/fonts/carlito/OFL-Carlito.txt",
        "licenses/carlito/OFL-Carlito.txt",
    ),
    ("crates/bpa/rules/LICENSE", "licenses/bpa-rules/LICENSE"),
    ("crates/bpa/rules/SOURCE.txt", "licenses/bpa-rules/SOURCE.txt"),
    ("crates/pbir/schemas/LICENSE", "licenses/pbir-schemas/LICENSE"),
    (
        "crates/pbir/schemas/SOURCE.txt",
        "licenses/pbir-schemas/SOURCE.txt",
    ),
    (
        "crates/visuals/schemas/LICENSE",
        "licenses/report-theme-schema/LICENSE",
    ),
    (
        "crates/visuals/schemas/SOURCE.txt",
        "licenses/report-theme-schema/SOURCE.txt",
    ),
    (
        "crates/locale/LICENSE-UNICODE",
        "licenses/unicode-cldr/LICENSE-UNICODE",
    ),
    ("crates/locale/SOURCE.txt", "licenses/unicode-cldr/SOURCE.txt"),
    ("crates/render/LICENSE-BSD-d3", "licenses/d3/LICENSE-BSD-d3"),
    (
        "crates/render/LICENSE-BSD-d3-array",
        "licenses/d3/LICENSE-BSD-d3-array",
    ),
    (
        "crates/data/LICENSE-MIT-dashcraft",
        "licenses/dashcraft/LICENSE-MIT-dashcraft",
    ),
    (
        "crates/ui/LICENSE-MIT-deckcraft",
        "licenses/deckcraft/LICENSE-MIT-deckcraft",
    ),
];

/// Highest `GLIBC_x.y` symbol version named in an ELF binary: the oldest glibc it runs on.
/// A scan of the version strings (`.dynstr`), so an upper bound when some are weak references.
fn glibc_floor(binary: &[u8]) -> Option<(u32, u32)> {
    let tag = b"GLIBC_";
    let mut best = None;
    let mut i = 0;
    while let Some(pos) = binary.get(i..)?.windows(tag.len()).position(|w| w == tag) {
        let start = i + pos + tag.len();
        let digits: Vec<u8> = binary
            .get(start..)
            .unwrap_or_default()
            .iter()
            .take(12)
            .take_while(|b| b.is_ascii_digit() || **b == b'.')
            .copied()
            .collect();
        let text = String::from_utf8_lossy(&digits);
        let mut parts = text.split('.').map(str::parse::<u32>);
        if let (Some(Ok(maj)), Some(Ok(min))) = (parts.next(), parts.next()) {
            best = best.max(Some((maj, min)));
        }
        i = start;
    }
    best
}

/// First line of `bin --version`.
fn version_of(bin: &Path) -> Result<String, String> {
    let o = Command::new(bin)
        .arg("--version")
        .output()
        .map_err(|e| format!("dist: {}: {e}", bin.display()))?;
    if !o.status.success() {
        return Err(format!("dist: {} --version failed", bin.display()));
    }
    Ok(String::from_utf8_lossy(&o.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned())
}

fn third_party_crates(t: Target, version: &str) -> Result<String, String> {
    let out = super::cargo()
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--filter-platform",
            t.triple(),
        ])
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let meta: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: bad JSON: {e}"))?;
    // both binaries ship in both archives: the union of their graphs
    let mut crates = notices::crates(&meta, "bicraft-cli")?;
    for k in notices::crates(&meta, "bicraft-desktop")? {
        if !crates.iter().any(|c| c.name == k.name && c.version == k.version) {
            crates.push(k);
        }
    }
    crates.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    Ok(notices::render(&crates, version, t.triple()))
}

/// Writes `SHA256SUMS` for every archive of `version` in `out`; returns `(hash, name, size)`.
fn write_sums(out: &Path, version: &str) -> Result<Vec<(String, String, usize)>, String> {
    let prefix = format!("bicraft-{version}-");
    let mut names: Vec<String> = std::fs::read_dir(out)
        .map_err(|e| format!("dist: {}: {e}", out.display()))?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(&prefix) && (n.ends_with(".zip") || n.ends_with(".tar.gz")))
        .collect();
    names.sort();
    let mut rows = Vec::new();
    let mut text = String::new();
    for n in names {
        let data = std::fs::read(out.join(&n)).map_err(|e| format!("dist: {n}: {e}"))?;
        let hash = sha256::hex(&data);
        text.push_str(&format!("{hash}  {n}\n"));
        rows.push((hash, n, data.len()));
    }
    std::fs::write(out.join("SHA256SUMS"), text).map_err(|e| format!("dist: SHA256SUMS: {e}"))?;
    Ok(rows)
}

/// The README.md of an archive: at the top on Windows, in `share/doc/bicraft/` on Linux.
fn readme(t: Target, version: &str, version_line: &str, requires: &str) -> String {
    let (contents, install, path_bin, mcp_link, check) = match t {
        Target::Windows => (
            "| `bicraft.exe` | the desktop app: a native report reader and editor; \
             `bicraft.exe --register` adds a Start menu entry and Open with, for your user only |\n\
             | `bicraft-cli.exe` | the command-line tool and the MCP server for agents |\n\
             | `install.ps1` | copies both programs to a user folder (see below) |\n\
             | `docs/mcp.md` | MCP tools reference and client configuration |\n\
             | `visuals/` | BIcraft's Power BI visuals as `.pbiviz` files, for Desktop's Import a visual from a file (their TypeScript sources are in the BIcraft repository: `visuals/README.md`) |\n\
             | `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE` | bicraft's license (MIT OR Apache-2.0, at your option) and required notices |\n\
             | `THIRD_PARTY_NOTICES.md`, `licenses/` | data bundled in the binaries (XPress9 port, fonts, JSON schemas, best-practice rules, CLDR) and their licenses |\n\
             | `THIRD_PARTY_CRATES.md` | the Rust crates compiled in, with their license texts |\n",
            "The programs run from anywhere; installing only copies them.\n\n\
             ```powershell\n\
             powershell -ExecutionPolicy Bypass -File .\\install.ps1                  # to %LOCALAPPDATA%\\bicraft\\bin\n\
             powershell -ExecutionPolicy Bypass -File .\\install.ps1 -Destination D:\\tools\n\
             ```\n\n\
             The script never changes `PATH`, the registry or any configuration: it prints how to add \
             the folder to `PATH` when it is not there.",
            "C:\\\\Users\\\\you\\\\AppData\\\\Local\\\\bicraft\\\\bin\\\\bicraft-cli.exe",
            "docs/mcp.md",
            "```powershell\nGet-FileHash .\\bicraft-VERSION-windows-x64.zip -Algorithm SHA256   # compare with SHA256SUMS\n```",
        ),
        Target::Linux => (
            "Paths from the archive's top folder.\n\n\
             | File | |\n|---|---|\n\
             | `bin/bicraft` | the desktop app: a native report reader and editor |\n\
             | `bin/bicraft-cli` | the command-line tool and the MCP server for agents |\n\
             | `install.sh` | installs everything below under a prefix (see below) |\n\
             | `share/applications/`, `share/icons/hicolor/`, `share/mime/packages/`, `share/metainfo/` | the menu entry, the icons, the file types (.pbix, .pbit, .pbip) and the AppStream metadata of the desktop app |\n\
             | `share/doc/bicraft/` | this README, `mcp.md` (MCP tools reference and client configuration), bicraft's license (`LICENSE-MIT`, `LICENSE-APACHE`, MIT OR Apache-2.0 at your option), `NOTICE`, `THIRD_PARTY_NOTICES.md` and `licenses/` (data bundled in the binaries and their licenses), `THIRD_PARTY_CRATES.md` (the Rust crates compiled in, with their license texts) |\n\
             | `share/bicraft/visuals/` | BIcraft's Power BI visuals as `.pbiviz` files, for Desktop's Import a visual from a file (their TypeScript sources are in the BIcraft repository: `share/bicraft/visuals/README.md`) |\n",
            "The programs run from the extracted folder (`bin/bicraft`, `bin/bicraft-cli`); installing \
             copies `bin/` and `share/` under a prefix, so the app gets its menu entry, icon and file \
             types.\n\n\
             ```sh\n\
             ./install.sh               # under ~/.local (~/.local/bin, ~/.local/share)\n\
             ./install.sh /opt/bicraft  # under another prefix\n\
             ```\n\n\
             The script refreshes the menu and MIME caches of that prefix when their tools are \
             installed. It never changes `PATH`, default applications or any configuration: it prints \
             how to add `PREFIX/bin` to `PATH` when it is not there.",
            "/home/you/.local/bin/bicraft-cli",
            "mcp.md",
            "```sh\nsha256sum -c --ignore-missing SHA256SUMS\n```",
        ),
    };
    let contents = match t {
        Target::Windows => format!("| File | |\n|---|---|\n{contents}"),
        Target::Linux => contents.to_owned(),
    };
    let check = check.replace("VERSION", version);
    format!(
        "# BIcraft {version} ({label})\n\n\
         `{version_line}`\n\n\
         BIcraft creates, reads, modifies, validates and compiles Power BI files (PBIX, PBIT, PBIP, \
         PBIR, TMDL/TMSL, DAX) without Power BI Desktop. It ships two self-contained programs: \
         `bicraft`, the desktop app, and `bicraft-cli`, the command-line tool and MCP server for \
         agents.\n\n\
         ## Requirements\n\n\
         {requires}\n\n\
         ## Contents\n\n\
         {contents}\n\
         ## Install\n\n\
         {install}\n\n\
         ## Use\n\n\
         ```sh\nbicraft Sales.pbix                     # the desktop app\n\
         bicraft-cli --version\nbicraft-cli help\nbicraft-cli validate-model path/to/Model.pbip\n```\n\n\
         ## MCP server\n\n\
         `bicraft-cli mcp` serves the Model Context Protocol on stdio. Register it with a client by hand:\n\n\
         ```json\n{{\n  \"mcpServers\": {{\n    \"bicraft\": {{ \"command\": \"{path_bin}\", \"args\": [\"mcp\"] }}\n  }}\n}}\n```\n\n\
         or let bicraft write the entry: `bicraft-cli setup --dry-run` shows the plan, \
         `bicraft-cli setup --client claude_code --yes` applies it (also `claude_desktop`, `vscode`, \
         `cursor`, `codex`). The tools, clients and options are described in \
         [{mcp_link}]({mcp_link}).\n\n\
         ## Verify the download\n\n\
         `SHA256SUMS`, next to the archives, lists their SHA-256:\n\n\
         {check}\n",
        label = t.label(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn parses_options() {
        let o = parse(&s(&["--out", "D", "--only", "linux", "--no-smoke"])).unwrap();
        assert_eq!(o.out, Some(PathBuf::from("D")));
        assert_eq!(o.only, Some(Target::Linux));
        assert!(o.build && !o.smoke);
        assert!(parse(&s(&["--only", "mac"])).is_err());
        assert!(parse(&s(&["--out"])).is_err());
        assert!(parse(&s(&["--bogus"])).is_err());
        assert_eq!(parse(&[]).unwrap().only, None);
    }

    #[test]
    fn readme_names_the_right_files() {
        let w = readme(
            Target::Windows,
            "1.2.3",
            "bicraft-cli 1.2.3 (commit 0123456789ab)",
            "Win",
        );
        assert!(w.contains("install.ps1") && !w.contains("install.sh"));
        assert!(w.contains("bicraft.exe --register") && w.contains("`bicraft-cli.exe`"));
        assert!(w.contains("bicraft-1.2.3-windows-x64.zip"));
        assert!(w.contains("(commit 0123456789ab)"));
        let l = readme(Target::Linux, "1.2.3", "bicraft-cli 1.2.3", "glibc 2.34");
        assert!(l.contains("install.sh") && l.contains("sha256sum -c"));
        assert!(l.contains("`bin/bicraft`") && l.contains("`bin/bicraft-cli`") && !l.contains(".exe"));
        assert!(!l.contains("bicraft-desktop") && !w.contains("bicraft-desktop"));
        assert!(l.contains("[mcp.md](mcp.md)") && w.contains("[docs/mcp.md](docs/mcp.md)"));
        assert!(l.contains("## Requirements\n\nglibc 2.34\n"));
        // The JSON sample is valid JSON.
        let json = l.split("```json\n").nth(1).and_then(|r| r.split("\n```").next());
        assert!(serde_json::from_str::<serde_json::Value>(json.unwrap()).is_ok());
        let json = w.split("```json\n").nth(1).and_then(|r| r.split("\n```").next());
        let v: serde_json::Value = serde_json::from_str(json.unwrap()).unwrap();
        assert!(
            v["mcpServers"]["bicraft"]["command"]
                .as_str()
                .unwrap()
                .ends_with("bicraft-cli.exe")
        );
    }

    #[test]
    fn visuals_readme_lists_the_files_and_their_sources() {
        let r = visuals_readme(&[(
            "bicraftVennX.1.0.0.0.pbiviz".into(),
            "BIcraft Venn".into(),
            "1.0.0.0".into(),
        )]);
        assert!(r.contains("| `bicraftVennX.1.0.0.0.pbiviz` | BIcraft Venn | 1.0.0.0 |\n"));
        for dir in ["spec_visual", "bvis", "chart_pack", "visual_runtime"] {
            let path = format!("crates/engine/src/{dir}/");
            assert!(r.contains(&path), "{dir}");
            assert!(crate::root().join(&path).is_dir(), "{path}");
        }
        assert!(readme(Target::Linux, "1", "bicraft-cli 1", "x").contains("`share/bicraft/visuals/`"));
        assert!(readme(Target::Windows, "1", "bicraft-cli 1", "x").contains("`visuals/`"));
    }

    #[test]
    fn linux_files_name_the_app_and_its_id() {
        // the window's app id (Wayland, X11) must be the one the menu entry and icons are named by
        let register =
            std::fs::read_to_string(crate::root().join("apps/bicraft-desktop/src/register.rs")).unwrap();
        assert!(register.contains(&format!("DESKTOP_ID: &str = \"{APP_ID}\"")));
        let entries = linux_integration("1.2.3").unwrap();
        let get = |p: &str| {
            entries
                .iter()
                .find(|e| e.path == p)
                .map(|e| String::from_utf8_lossy(&e.data).into_owned())
        };
        let desktop = get(&format!("share/applications/{APP_ID}.desktop")).unwrap();
        assert!(
            desktop.contains(
                "
Exec=bicraft %f
"
            ) && desktop.contains(&format!(
                "
Icon={APP_ID}
"
            ))
        );
        let metainfo = get(&format!("share/metainfo/{APP_ID}.metainfo.xml")).unwrap();
        assert!(
            metainfo.contains(&format!("<id>{APP_ID}</id>"))
                && metainfo.contains("<release version=\"1.2.3\"/>")
        );
        assert!(!metainfo.contains('@'));
        let mime = get(&format!("share/mime/packages/{APP_ID}.xml")).unwrap();
        for ext in ["pbix", "pbit", "pbip"] {
            assert!(
                mime.contains(&format!("pattern=\"*.{ext}\""))
                    && desktop.contains(&format!("application/x-{ext};"))
            );
        }
        // every hicolor size is a PNG of that size, plus the scalable SVG
        for size in ICON_SIZES {
            let png = entries
                .iter()
                .find(|e| e.path == format!("share/icons/hicolor/{size}x{size}/apps/{APP_ID}.png"))
                .unwrap();
            let pm = tiny_skia::Pixmap::decode_png(&png.data).unwrap();
            assert_eq!((pm.width(), pm.height()), (size, size));
        }
        assert!(
            get(&format!("share/icons/hicolor/scalable/apps/{APP_ID}.svg"))
                .unwrap()
                .starts_with("<svg")
        );
        let install = include_str!("../../../packaging/linux/install.sh");
        assert!(install.contains(&format!("share/applications/{APP_ID}.desktop")));
    }

    #[test]
    fn glibc_floor_takes_the_highest_version() {
        let elf = b"\0GLIBC_2.2.5\0GLIBC_2.34\0atan2f\0GLIBC_2.43\0GLIBC_2.9\0GLIBC_PRIVATE\0";
        assert_eq!(glibc_floor(elf), Some((2, 43)));
        assert_eq!(glibc_floor(b"no versions"), None);
        assert_eq!(glibc_floor(b"GLIBC_"), None);
        assert_eq!(glibc_floor(b"xGLIBC_2."), None);
    }

    #[test]
    fn bundled_license_files_exist() {
        let root = crate::root();
        for (src, dest) in BUNDLED_LICENSES {
            assert!(root.join(src).is_file(), "{src}");
            assert!(dest.starts_with("licenses/"));
        }
        for f in [
            "LICENSE-MIT",
            "LICENSE-APACHE",
            "NOTICE",
            "THIRD_PARTY_NOTICES.md",
            "docs/mcp.md",
        ] {
            assert!(root.join(f).is_file(), "{f}");
        }
    }

    #[test]
    fn sums_cover_this_version_only() {
        let dir = std::env::temp_dir().join(format!("xtask-dist-sums-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bicraft-1.0.0-linux-x64.tar.gz"), b"abc").unwrap();
        std::fs::write(dir.join("bicraft-1.0.0-windows-x64.zip"), b"").unwrap();
        std::fs::write(dir.join("bicraft-0.9.0-linux-x64.tar.gz"), b"old").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        let rows = write_sums(&dir, "1.0.0").unwrap();
        let text = std::fs::read_to_string(dir.join("SHA256SUMS")).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            text,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  bicraft-1.0.0-linux-x64.tar.gz\n\
             e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  bicraft-1.0.0-windows-x64.zip\n"
        );
    }
}
