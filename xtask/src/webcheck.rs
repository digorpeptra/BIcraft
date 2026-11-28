//! `cargo xtask webcheck record|check`: regression checks of the web viewer (docs/webcheck.md,
//! ROADMAP M11.27) with Craft Anything, a local development aid (never a dependency).
//!
//! The command builds `bicraft` (debug), copies a synthetic fixture of ours (the layout-oracle
//! `visuals` project) outside the repository and adds a bookmark to it with `bicraft-cli run`, starts
//! `bicraft-cli view` on it on a free loopback port, and runs walkthroughs of the HTML viewer: Craft
//! Anything's MCP server (`craft-anything-cli mcp`, its restricted mode) opens the page in a private
//! headless browser (`web_site_open`) and takes a screenshot after each step
//! (`web_page_screenshot_take`); the steps are the reader gestures the page's clicks send
//! (`POST /api/interact`: page tabs, a slicer item, a data point, a drill, a bookmark), and
//! `web_page_states_capture` clicks the page's own tabs. `record` keeps the screenshots as
//! baselines in a folder outside the repository (`BICRAFT_WEBCHECK_DIR`, else the user's local
//! data folder); `check` replays the walkthroughs, compares every screenshot with its baseline
//! (`web_pixels_compare`), writes a diff image per changed step and prints a pass/fail table.
//!
//! Without `craft-anything-cli` the command prints a note and succeeds, as the other local
//! oracles do. It is not part of `cargo xtask ci`.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use serde_json::{Value, json};

use crate::{cargo, root, run};

const USAGE: &str = "\
usage: cargo xtask webcheck record|check [--dir DIR] [--bicraft PATH] [--only WALK]
                                         [--tolerance N] [--max-diff FRACTION]

  record          run the walkthroughs and keep their screenshots as baselines
  check           run them again and compare each screenshot with its baseline
  --dir DIR       where the fixture copy, baselines and runs go (default $BICRAFT_WEBCHECK_DIR,
                  else the local data folder's bicraft/webcheck); never inside the repository
  --bicraft PATH  use this bicraft-cli binary (default: cargo build -p bicraft-cli, debug)
  --only WALK     run one walkthrough (open, pages, slicer, point, drill, bookmark, states)
  --tolerance N   per-channel difference a pixel must exceed to count (0..=255, default 16)
  --max-diff F    share of changed pixels a step may have and still pass (0..=1, default 0.0005)

Needs craft-anything-cli ($BICRAFT_CRAFT_ANYTHING, PATH, or its default install folder);
skipped with a note without it.";

/// The tool's executable name.
const TOOL: &str = if cfg!(windows) {
    "craft-anything-cli.exe"
} else {
    "craft-anything-cli"
};

/// The fixture, relative to the repository: our synthetic legacy project (three pages: Charts,
/// Tables with two slicers, Shapes; `SOURCE.txt` next to it).
const FIXTURE: &str = "crates/engine/tests/fixtures/layout-oracle/visuals/legacy";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Record,
    Check,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Opts {
    pub mode: Mode,
    pub dir: Option<PathBuf>,
    pub bicraft: Option<PathBuf>,
    pub only: Option<String>,
    pub tolerance: u8,
    pub max_diff: f64,
}

pub fn parse_args(args: &[String]) -> Result<Opts, String> {
    let mut it = args.iter();
    let mode = match it.next().map(String::as_str) {
        Some("record") => Mode::Record,
        Some("check") => Mode::Check,
        Some(other) => return Err(format!("webcheck: unknown mode `{other}`\n\n{USAGE}")),
        None => return Err(format!("webcheck: record or check?\n\n{USAGE}")),
    };
    let mut o = Opts {
        mode,
        dir: None,
        bicraft: None,
        only: None,
        tolerance: 16,
        max_diff: 0.0005,
    };
    while let Some(a) = it.next() {
        let mut val = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("webcheck: {name} needs a value"))
        };
        match a.as_str() {
            "--dir" => o.dir = Some(PathBuf::from(val("--dir")?)),
            "--bicraft" => o.bicraft = Some(PathBuf::from(val("--bicraft")?)),
            "--only" => {
                let w = val("--only")?;
                if !WALKS.contains(&w.as_str()) {
                    return Err(format!(
                        "webcheck: --only: no walkthrough `{w}` (one of {})",
                        WALKS.join(", ")
                    ));
                }
                o.only = Some(w);
            }
            "--tolerance" => {
                let v = val("--tolerance")?;
                o.tolerance = v
                    .parse()
                    .map_err(|_| format!("webcheck: --tolerance: 0..=255, not `{v}`"))?;
            }
            "--max-diff" => {
                let v = val("--max-diff")?;
                o.max_diff = v
                    .parse::<f64>()
                    .ok()
                    .filter(|f| (0.0..=1.0).contains(f))
                    .ok_or_else(|| format!("webcheck: --max-diff: 0..=1, not `{v}`"))?;
            }
            "-h" | "--help" => return Err(USAGE.to_string()),
            other => return Err(format!("webcheck: unknown option `{other}`\n\n{USAGE}")),
        }
    }
    Ok(o)
}

/// Where the tool is: `$BICRAFT_CRAFT_ANYTHING`, else the first `PATH` entry holding it, else
/// its default install folder under `%ProgramFiles%`. `None` when none of them is a file.
pub fn find_tool(
    env_override: Option<OsString>,
    path_var: Option<OsString>,
    program_files: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(p) = env_override {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let from_path = path_var
        .iter()
        .flat_map(std::env::split_paths)
        .map(|d| d.join(TOOL))
        .find(|p| p.is_file());
    from_path.or_else(|| {
        program_files
            .map(|d| PathBuf::from(d).join("Craft Anything").join(TOOL))
            .filter(|p| p.is_file())
    })
}

/// The default folder: `<local data>/bicraft/webcheck`, the local data folder being
/// `%LOCALAPPDATA%` on Windows, `$XDG_DATA_HOME` or `~/.local/share` elsewhere.
pub fn default_dir(
    local_app_data: Option<OsString>,
    xdg_data_home: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    let base = if cfg!(windows) {
        local_app_data.map(PathBuf::from)
    } else {
        xdg_data_home
            .map(PathBuf::from)
            .or_else(|| home.map(|h| PathBuf::from(h).join(".local").join("share")))
    };
    base.map(|b| b.join("bicraft").join("webcheck"))
}

// ------------------------------------------------------------------------------------------------
// The walkthroughs

/// The walkthroughs, by id.
pub const WALKS: &[&str] = &["open", "pages", "slicer", "point", "drill", "bookmark", "states"];

/// The fixture's pages and visuals the walkthroughs use (`bicraft-cli run --cmd report.summary`).
const CHARTS: &str = "ReportSection68b399afc6ae";
const TABLES: &str = "ReportSection0d3d12eff6b5";
const SHAPES: &str = "ReportSection604b5fcb060f";
/// Charts: the clustered column chart (Sales[Region] on its axis).
const COLUMN_CHART: &str = "a424065f0e659be6f184";
/// Charts: the clustered bar chart (Sales[Category] on its axis; `prepare_fixture` adds
/// Sales[Product] under it, so it drills).
const DRILL_CHART: &str = "d0269265f29f47b19a07";
/// Tables: the Sales[Region] slicer.
const REGION_SLICER: &str = "c3f6a83cc426087d1283";
/// The bookmark `prepare_fixture` captures on the Charts page.
const BOOKMARK: &str = "Charts view";

/// One step: the reader gestures it sends (`view.interact` actions, the bodies of the page's own
/// `POST /api/interact`), then a screenshot named by its id.
pub struct Step {
    pub id: &'static str,
    pub actions: Vec<Value>,
}

/// One walkthrough: steps from the report's first page, freshly loaded. `states`: instead of
/// steps, Craft Anything clicks the page's own tabs (`web_page_states_capture`).
pub struct Walk {
    pub id: &'static str,
    pub steps: Vec<Step>,
    pub states: bool,
}

fn st(id: &'static str, actions: Vec<Value>) -> Step {
    Step { id, actions }
}

fn page(p: &str) -> Value {
    json!({"action": "page", "page": p})
}

pub fn walkthroughs() -> Vec<Walk> {
    let open = || st("open", vec![]);
    let walk = |id, steps| Walk {
        id,
        steps,
        states: false,
    };
    vec![
        walk("open", vec![open()]),
        walk(
            "pages",
            vec![
                open(),
                st("tables", vec![page(TABLES)]),
                st("shapes", vec![page(SHAPES)]),
                st("charts", vec![page(CHARTS)]),
            ],
        ),
        walk(
            "slicer",
            vec![
                open(),
                st("tables", vec![page(TABLES)]),
                st(
                    "east",
                    vec![json!({"action": "slicer_toggle", "page": TABLES,
                                "visual": REGION_SLICER, "value": "East"})],
                ),
                st(
                    "east-west",
                    vec![json!({"action": "slicer_toggle", "page": TABLES,
                                "visual": REGION_SLICER, "value": "West", "ctrl": true})],
                ),
            ],
        ),
        walk(
            "point",
            vec![
                open(),
                st(
                    "select-east",
                    vec![json!({"action": "select", "page": CHARTS, "visual": COLUMN_CHART,
                                "point": [{"c": 0, "v": "East"}]})],
                ),
                st(
                    "clear",
                    vec![json!({"action": "clear_selection", "page": CHARTS})],
                ),
            ],
        ),
        walk(
            "drill",
            vec![
                open(),
                st(
                    "next-level",
                    vec![json!({"action": "drill_next", "page": CHARTS, "visual": DRILL_CHART})],
                ),
                st(
                    "up",
                    vec![json!({"action": "drill_up", "page": CHARTS, "visual": DRILL_CHART})],
                ),
            ],
        ),
        walk(
            "bookmark",
            vec![
                open(),
                st("tables", vec![page(TABLES)]),
                st("apply", vec![json!({"action": "bookmark", "bookmark": BOOKMARK})]),
            ],
        ),
        Walk {
            id: "states",
            steps: vec![],
            states: true,
        },
    ]
}

// ------------------------------------------------------------------------------------------------
// Results

/// The verdict of one step.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Within the tolerance: the share of changed pixels.
    Pass(f64),
    /// Over it: the share of changed pixels and the changed box (`x,y wxh`).
    Changed(f64, String),
    /// Not comparable or not run (sizes differ, a screenshot or baseline missing, a gesture
    /// refused): why.
    Fail(String),
    /// Recorded (record mode).
    Recorded,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub walk: String,
    pub step: String,
    pub outcome: Outcome,
}

impl Row {
    pub fn passed(&self) -> bool {
        matches!(self.outcome, Outcome::Pass(_) | Outcome::Recorded)
    }
}

/// The result table: one line per step, then the totals.
pub fn table(rows: &[Row]) -> String {
    let ww = rows.iter().map(|r| r.walk.len()).max().unwrap_or(0).max(4);
    let sw = rows.iter().map(|r| r.step.len()).max().unwrap_or(0).max(4);
    let mut s = format!("{:<ww$}  {:<sw$}  {:<8}  detail\n", "walk", "step", "result");
    for r in rows {
        let (res, detail) = match &r.outcome {
            Outcome::Pass(f) => ("pass", format!("{:.3} % of pixels changed", f * 100.0)),
            Outcome::Changed(f, b) => ("FAIL", format!("{:.3} % of pixels changed, in {b}", f * 100.0)),
            Outcome::Fail(why) => ("FAIL", why.clone()),
            Outcome::Recorded => ("recorded", String::new()),
        };
        let line = format!("{:<ww$}  {:<sw$}  {res:<8}  {detail}", r.walk, r.step);
        s.push_str(line.trim_end());
        s.push('\n');
    }
    let failed = rows.iter().filter(|r| !r.passed()).count();
    s.push_str(&format!(
        "\n{} step(s), {} passed, {failed} failed\n",
        rows.len(),
        rows.len() - failed
    ));
    s
}

/// The verdict from a `web_pixels_compare` result.
pub fn verdict(r: &Value, max_diff: f64) -> Outcome {
    if r.get("comparable").and_then(Value::as_bool) == Some(false) {
        let size = |k: &str| r.pointer(k).and_then(Value::as_u64).unwrap_or(0);
        return Outcome::Fail(format!(
            "not comparable (sizes {}x{} and {}x{})",
            size("/before/width"),
            size("/before/height"),
            size("/after/width"),
            size("/after/height")
        ));
    }
    let Some(f) = r.get("difference_fraction").and_then(Value::as_f64) else {
        return Outcome::Fail(format!("no difference_fraction in {}", clip(&r.to_string(), 200)));
    };
    if f <= max_diff {
        return Outcome::Pass(f);
    }
    let b = r
        .get("changed_bounding_box")
        .filter(|b| !b.is_null())
        .map(|b| {
            let n = |k: &str| b.get(k).and_then(Value::as_u64).unwrap_or(0);
            format!("{},{} {}x{}", n("x"), n("y"), n("width"), n("height"))
        })
        .unwrap_or_else(|| "?".into());
    Outcome::Changed(f, b)
}

fn clip(s: &str, n: usize) -> String {
    match s.char_indices().nth(n) {
        Some((i, _)) => format!("{}…", s.get(..i).unwrap_or(s)),
        None => s.to_string(),
    }
}

// ------------------------------------------------------------------------------------------------
// The command

pub fn cmd(args: &[String]) -> Result<(), String> {
    let o = parse_args(args)?;
    let tool = find_tool(
        bicraft_env::var_os("BICRAFT_CRAFT_ANYTHING"),
        std::env::var_os("PATH"),
        std::env::var_os("ProgramFiles"),
    );
    run_with(&o, tool)
}

/// The command once its options are read: without the tool, a note and success.
fn run_with(o: &Opts, tool: Option<PathBuf>) -> Result<(), String> {
    let Some(tool) = tool else {
        eprintln!("{}", skip_note());
        return Ok(());
    };
    let dir = match o.dir.clone() {
        Some(d) => d,
        None => bicraft_env::var_os("BICRAFT_WEBCHECK_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                default_dir(
                    std::env::var_os("LOCALAPPDATA"),
                    std::env::var_os("XDG_DATA_HOME"),
                    std::env::var_os("HOME"),
                )
            })
            .ok_or("webcheck: no local data folder: give --dir or set BICRAFT_WEBCHECK_DIR")?,
    };
    let dir = std::path::absolute(&dir).map_err(|e| format!("webcheck: {}: {e}", dir.display()))?;
    if dir.starts_with(root()) {
        return Err(format!(
            "webcheck: {} is inside the repository; baselines stay outside it",
            dir.display()
        ));
    }
    let bicraft = match &o.bicraft {
        Some(b) => b.clone(),
        None => build_bicraft()?,
    };
    let fixture = prepare_fixture(&bicraft, &dir)?;
    let mut viewer = Viewer::start(&bicraft, &fixture)?;
    let result = Craft::start(&tool).and_then(|mut craft| {
        let rows = run_walks(o, &mut craft, &viewer, &dir);
        craft.stop();
        rows
    });
    viewer.stop();
    let rows = result?;
    print!("{}", table(&rows));
    println!("folder: {}", dir.display());
    if rows.iter().all(Row::passed) {
        Ok(())
    } else {
        Err("webcheck: some steps differ from their baselines (diff images in the folder's diff/)".into())
    }
}

/// The note printed when the tool is missing.
pub fn skip_note() -> String {
    "note: craft-anything-cli was not found (set BICRAFT_CRAFT_ANYTHING or put it on PATH); \
     skipping the web viewer checks (docs/webcheck.md)"
        .into()
}

fn build_bicraft() -> Result<PathBuf, String> {
    let mut c = cargo();
    c.args(["build", "-p", "bicraft-cli"]);
    run(c, "cargo build -p bicraft-cli")?;
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("target"));
    let exe = target.join("debug").join(if cfg!(windows) {
        "bicraft-cli.exe"
    } else {
        "bicraft-cli"
    });
    if exe.is_file() {
        Ok(exe)
    } else {
        Err(format!("webcheck: no bicraft-cli binary at {}", exe.display()))
    }
}

/// The `bicraft-cli run` commands that complete the fixture: a second level on the bar chart's axis
/// (Sales[Category] then Sales[Product]: drillable) and a bookmark of the Charts page.
pub fn fixture_commands() -> Vec<String> {
    vec![
        format!(
            "visual.bind={}",
            json!({"page": CHARTS, "visual": DRILL_CHART, "role": "Category",
                   "field": "Sales[Product]"})
        ),
        format!(
            "bookmark.capture={}",
            json!({"display_name": BOOKMARK, "current_page": true})
        ),
    ]
}

/// Copies the fixture to `<dir>/work` (afresh) and completes it with [`fixture_commands`],
/// saved (as PBIR). Returns the `.pbip`.
fn prepare_fixture(bicraft: &Path, dir: &Path) -> Result<PathBuf, String> {
    let work = dir.join("work");
    if work.exists() {
        std::fs::remove_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    }
    copy_dir(&root().join(FIXTURE), &work)?;
    let pbip = work.join("visuals.pbip");
    let mut c = Command::new(bicraft);
    c.arg("run").arg("--open").arg(&pbip);
    for cmd in fixture_commands() {
        c.arg("--cmd").arg(cmd);
    }
    let out = c
        .arg("--save")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("webcheck: cannot run {}: {e}", bicraft.display()))?;
    if !out.status.success() {
        return Err(format!(
            "webcheck: bicraft-cli run (completing the fixture) failed: {}",
            clip(&String::from_utf8_lossy(&out.stdout), 600)
        ));
    }
    Ok(pbip)
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let entries = std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?;
    for e in entries {
        let e = e.map_err(|e| format!("{}: {e}", from.display()))?;
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if e.file_type().map_err(|e| e.to_string())?.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst).map_err(|e| format!("{}: {e}", src.display()))?;
        }
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------------
// bicraft-cli view

/// `bicraft-cli view` on a free loopback port (port 0: the system picks one, read from its
/// "live viewer at" line), with the page's interaction token.
struct Viewer {
    child: Child,
    url: String,
    token: String,
}

impl Viewer {
    fn start(bicraft: &Path, fixture: &Path) -> Result<Viewer, String> {
        let mut child = Command::new(bicraft)
            .arg("view")
            .arg(fixture)
            .args(["--port", "0", "--no-open-button", "--lang", "en"])
            .args(["--as-of", "2026-01-01"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("webcheck: cannot start {}: {e}", bicraft.display()))?;
        let Some(err) = child.stderr.take() else {
            let _ = child.kill();
            return Err("webcheck: no stderr from bicraft-cli view".into());
        };
        let (tx, rx) = channel();
        // reads every line (so the pipe never fills) and hands the URL over
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                if let Some(u) = viewer_url(&line) {
                    let _ = tx.send(u);
                }
            }
        });
        let url = match rx.recv_timeout(Duration::from_secs(60)) {
            Ok(url) => url,
            Err(_) => {
                let _ = child.kill();
                return Err("webcheck: bicraft-cli view printed no URL within 60 s".into());
            }
        };
        let token = http(&url, "GET", "/", &[], "")
            .and_then(|(status, body)| match status {
                200 => interact_token(&body).ok_or_else(|| "no interaction token".to_string()),
                s => Err(format!("HTTP {s}")),
            })
            .map_err(|e| format!("webcheck: the viewer's page at {url}: {e}"));
        match token {
            Ok(token) => {
                eprintln!("webcheck: bicraft-cli view at {url}");
                Ok(Viewer { child, url, token })
            }
            Err(e) => {
                let _ = child.kill();
                Err(e)
            }
        }
    }

    /// One reader gesture, as the page sends it.
    fn interact(&self, action: &Value) -> Result<(), String> {
        let origin = self.url.trim_end_matches('/');
        let headers = [
            ("X-Bicraft-Token", self.token.as_str()),
            ("Content-Type", "application/json"),
            ("Origin", origin),
            ("Sec-Fetch-Site", "same-origin"),
        ];
        let (status, body) = http(&self.url, "POST", "/api/interact", &headers, &action.to_string())?;
        let ok = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v.get("ok").and_then(Value::as_bool));
        if status == 200 && ok != Some(false) {
            Ok(())
        } else {
            Err(format!(
                "{} refused (HTTP {status}): {}",
                action,
                clip(&body, 300)
            ))
        }
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The URL of `bicraft-cli view`'s "bicraft: live viewer at URL" line (loopback only).
pub fn viewer_url(line: &str) -> Option<String> {
    let u = line.trim().strip_prefix("bicraft: live viewer at ")?.trim();
    let loopback = u.starts_with("http://127.0.0.1:") || u.starts_with("http://localhost:");
    loopback.then(|| u.to_string())
}

/// The interaction token of the viewer's page (`<meta name="bicraft-interact-token">`).
pub fn interact_token(html: &str) -> Option<String> {
    const SLOT: &str = r#"<meta name="bicraft-interact-token" content=""#;
    let rest = html.get(html.find(SLOT)? + SLOT.len()..)?;
    let t = rest.get(..rest.find('"')?)?;
    (!t.is_empty() && t.bytes().all(|b| b.is_ascii_alphanumeric())).then(|| t.to_string())
}

/// A small HTTP/1.1 request to the loopback viewer (`Connection: close`, read to the end).
fn http(
    base: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Result<(u16, String), String> {
    let host = base
        .strip_prefix("http://")
        .and_then(|h| h.split('/').next())
        .ok_or_else(|| format!("not an http URL: {base}"))?;
    let mut s = TcpStream::connect(host).map_err(|e| format!("{host}: {e}"))?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(60)));
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    parse_response(&raw)
}

/// Status and body of an HTTP/1.1 response (`Content-Length` or to the end; chunked bodies
/// decoded).
pub fn parse_response(raw: &[u8]) -> Result<(u16, String), String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("no end of headers")?;
    let head = String::from_utf8_lossy(raw.get(..split).unwrap_or_default());
    let body = raw.get(split + 4..).unwrap_or_default();
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or("no status")?;
    let chunked = head.lines().any(|l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("transfer-encoding:") && l.contains("chunked")
    });
    let body = if chunked { dechunk(body) } else { body.to_vec() };
    Ok((status, String::from_utf8_lossy(&body).into_owned()))
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(eol) = b.windows(2).position(|w| w == b"\r\n") {
        let size = std::str::from_utf8(b.get(..eol).unwrap_or_default())
            .ok()
            .and_then(|s| usize::from_str_radix(s.split(';').next().unwrap_or("").trim(), 16).ok())
            .unwrap_or(0);
        if size == 0 {
            break;
        }
        let start = eol + 2;
        out.extend_from_slice(b.get(start..start.saturating_add(size)).unwrap_or_default());
        b = b
            .get(start.saturating_add(size).saturating_add(2)..)
            .unwrap_or_default();
    }
    out
}

// ------------------------------------------------------------------------------------------------
// Craft Anything's MCP server

/// Craft Anything's server, started with `--allow-run`, its restricted-mode flag and `mcp`: one
/// server for the whole run, so the browser `web_site_open` starts stays open between calls.
struct Craft {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next_id: u64,
}

impl Craft {
    fn start(tool: &Path) -> Result<Craft, String> {
        let mut child = Command::new(tool)
            .args(["--allow-run", "--clean-room", "mcp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("webcheck: cannot start {}: {e}", tool.display()))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            return Err("webcheck: no pipes to craft-anything-cli".into());
        };
        let (tx, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut c = Craft {
            child,
            stdin,
            lines,
            next_id: 1,
        };
        c.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "bicraft-xtask-webcheck", "version": "0"}}),
            Duration::from_secs(60),
        )?;
        c.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))?;
        Ok(c)
    }

    fn send(&mut self, v: &Value) -> Result<(), String> {
        writeln!(self.stdin, "{v}")
            .and_then(|()| self.stdin.flush())
            .map_err(|e| format!("craft-anything-cli: {e}"))
    }

    fn request(&mut self, method: &str, params: Value, wait: Duration) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        loop {
            let line = self
                .lines
                .recv_timeout(wait)
                .map_err(|_| format!("craft-anything-cli: no answer to {method} in {wait:?}"))?;
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if v.get("id").and_then(Value::as_u64) == Some(id) {
                return Ok(v);
            }
        }
    }

    /// Calls a tool; returns its evidence's `result`.
    fn call(&mut self, tool: &str, args: Value) -> Result<Value, String> {
        let v = self.request(
            "tools/call",
            json!({"name": tool, "arguments": args}),
            Duration::from_secs(300),
        )?;
        tool_result(tool, &v)
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The `result` of a `tools/call` answer's evidence (`structuredContent.result`), or its error.
pub fn tool_result(tool: &str, answer: &Value) -> Result<Value, String> {
    if let Some(e) = answer.get("error") {
        return Err(format!("{tool}: {}", clip(&e.to_string(), 400)));
    }
    let r = answer.get("result").ok_or_else(|| format!("{tool}: no result"))?;
    let content = r.get("structuredContent");
    if r.get("isError").and_then(Value::as_bool) == Some(true) {
        let why = content
            .and_then(|c| c.pointer("/error/message"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| clip(&r.to_string(), 400));
        return Err(format!("{tool}: {why}"));
    }
    content
        .and_then(|c| c.get("result"))
        .cloned()
        .ok_or_else(|| format!("{tool}: no result in {}", clip(&r.to_string(), 300)))
}

/// Standard base64 (padding optional), `None` on a bad character.
pub fn decode_b64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3 + 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' => continue,
            _ => return None,
        };
        acc = ((acc << 6) | u32::from(v)) & 0x00ff_ffff;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

// ------------------------------------------------------------------------------------------------
// Running

/// Lets the page take a new snapshot and draw it (the changed visuals' highlight fades in 2.4 s).
const SETTLE: Duration = Duration::from_millis(3500);
/// The first load: the page, its first snapshot and the visuals' data.
const FIRST_LOAD: Duration = Duration::from_millis(5000);

fn run_walks(o: &Opts, craft: &mut Craft, viewer: &Viewer, dir: &Path) -> Result<Vec<Row>, String> {
    let run_dir = dir.join(match o.mode {
        Mode::Record => "baseline",
        Mode::Check => "latest",
    });
    // the diff images are this run's only
    let diffs = dir.join("diff");
    if o.mode == Mode::Check && diffs.exists() {
        std::fs::remove_dir_all(&diffs).map_err(|e| format!("{}: {e}", diffs.display()))?;
    }
    let mut rows = Vec::new();
    for w in walkthroughs() {
        if o.only.as_deref().is_some_and(|only| only != w.id) {
            continue;
        }
        eprintln!("webcheck: {}", w.id);
        let wdir = run_dir.join(w.id);
        if wdir.exists() {
            std::fs::remove_dir_all(&wdir).map_err(|e| format!("{}: {e}", wdir.display()))?;
        }
        std::fs::create_dir_all(&wdir).map_err(|e| format!("{}: {e}", wdir.display()))?;
        // every walk starts from the report as opened, on its first page, freshly loaded
        viewer.interact(&json!({"action": "reset"}))?;
        viewer.interact(&page(CHARTS))?;
        craft.call(
            "web_site_open",
            json!({"url": viewer.url, "width": 1280, "height": 800, "headless": true,
                   "settle_ms": 2000}),
        )?;
        std::thread::sleep(FIRST_LOAD);
        let shots = if w.states {
            states(craft, &wdir)
        } else {
            steps(craft, viewer, &w, &wdir)
        };
        let _ = craft.call("web_site_close", json!({}));
        for (sid, shot) in shots {
            let outcome = match (o.mode, shot) {
                (_, Err(why)) => Outcome::Fail(why),
                (Mode::Record, Ok(_)) => Outcome::Recorded,
                (Mode::Check, Ok(png)) => {
                    let base = dir.join("baseline").join(w.id).join(format!("{sid}.png"));
                    let diff = dir.join("diff").join(format!("{}-{sid}.png", w.id));
                    compare(craft, &base, &png, o, &diff)
                }
            };
            rows.push(Row {
                walk: w.id.to_string(),
                step: sid,
                outcome,
            });
        }
    }
    Ok(rows)
}

type Shots = Vec<(String, Result<PathBuf, String>)>;

fn steps(craft: &mut Craft, viewer: &Viewer, w: &Walk, wdir: &Path) -> Shots {
    let mut out = Vec::new();
    for s in &w.steps {
        let done = s.actions.iter().try_for_each(|a| viewer.interact(a));
        let shot = done.and_then(|()| {
            if !s.actions.is_empty() {
                std::thread::sleep(SETTLE);
            }
            screenshot(craft, &wdir.join(format!("{}.png", s.id)))
        });
        out.push((s.id.to_string(), shot));
    }
    out
}

fn screenshot(craft: &mut Craft, png: &Path) -> Result<PathBuf, String> {
    let r = craft.call("web_page_screenshot_take", json!({}))?;
    let b64 = r
        .pointer("/screenshot/base64")
        .and_then(Value::as_str)
        .ok_or("web_page_screenshot_take: no image")?;
    let bytes = decode_b64(b64).ok_or("web_page_screenshot_take: bad base64")?;
    std::fs::write(png, bytes).map_err(|e| format!("{}: {e}", png.display()))?;
    Ok(png.to_path_buf())
}

/// `web_page_states_capture`: Craft Anything clicks the page's tabs one at a time (the Model
/// and Validation tabs, the page tabs), a screenshot each, putting the page back in between.
fn states(craft: &mut Craft, wdir: &Path) -> Shots {
    let tmp = wdir.join("capture");
    let r = craft.call(
        "web_page_states_capture",
        json!({"output_directory": tmp, "max_states": 4, "width": 1280}),
    );
    if let Err(e) = r {
        return vec![("capture".into(), Err(e))];
    }
    let mut out = Vec::new();
    for n in 1..=8 {
        let src = tmp.join(format!("state-{n}.png"));
        if !src.is_file() {
            continue;
        }
        let dst = wdir.join(format!("state-{n}.png"));
        let shot = std::fs::rename(&src, &dst)
            .map(|()| dst)
            .map_err(|e| format!("{}: {e}", src.display()));
        out.push((format!("state-{n}"), shot));
    }
    if out.is_empty() {
        out.push(("capture".into(), Err("no state captured".into())));
    }
    out
}

/// Compares a step's screenshot with its baseline (`web_pixels_compare`); over the threshold,
/// writes a diff image.
fn compare(craft: &mut Craft, base: &Path, png: &Path, o: &Opts, diff: &Path) -> Outcome {
    if !base.is_file() {
        return Outcome::Fail("no baseline (run `cargo xtask webcheck record`)".into());
    }
    let params = json!({"before": {"path": base}, "after": {"path": png}, "tolerance": o.tolerance});
    match craft.call("web_pixels_compare", params) {
        Ok(r) => {
            let outcome = verdict(&r, o.max_diff);
            if matches!(outcome, Outcome::Changed(..))
                && let Err(e) = write_diff(base, png, o.tolerance, diff)
            {
                eprintln!("webcheck: diff image {}: {e}", diff.display());
            }
            outcome
        }
        Err(e) => Outcome::Fail(e),
    }
}

/// The diff image: the baseline faded to grey, the pixels whose channels differ by more than
/// `tolerance` in red.
fn write_diff(base: &Path, png: &Path, tolerance: u8, out: &Path) -> Result<(), String> {
    let a = tiny_skia::Pixmap::load_png(base).map_err(|e| e.to_string())?;
    let b = tiny_skia::Pixmap::load_png(png).map_err(|e| e.to_string())?;
    if (a.width(), a.height()) != (b.width(), b.height()) {
        return Err("sizes differ".into());
    }
    let mut d = a.clone();
    let ch = |p: &tiny_skia::PremultipliedColorU8| [p.red(), p.green(), p.blue(), p.alpha()];
    for (px, (pa, pb)) in d.pixels_mut().iter_mut().zip(a.pixels().iter().zip(b.pixels())) {
        let changed = ch(pa)
            .iter()
            .zip(ch(pb).iter())
            .any(|(x, y)| x.abs_diff(*y) > tolerance);
        let c = if changed {
            tiny_skia::ColorU8::from_rgba(230, 0, 0, 255)
        } else {
            let sum = u16::from(pa.red()) + u16::from(pa.green()) + u16::from(pa.blue());
            let g = 170 + (sum / 9).min(85) as u8;
            tiny_skia::ColorU8::from_rgba(g, g, g, 255)
        };
        *px = c.premultiply();
    }
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    d.save_png(out).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn row(walk: &str, step: &str, outcome: Outcome) -> Row {
        Row {
            walk: walk.into(),
            step: step.into(),
            outcome,
        }
    }

    /// A fresh empty folder under the system temp folder.
    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("xtask-webcheck-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn arguments_are_parsed() {
        let o = parse_args(&args(&["record"])).unwrap();
        assert_eq!(o.mode, Mode::Record);
        assert_eq!((o.tolerance, o.max_diff, o.dir, o.only), (16, 0.0005, None, None));
        let o = parse_args(&args(&[
            "check",
            "--dir",
            "D",
            "--bicraft",
            "B",
            "--only",
            "slicer",
            "--tolerance",
            "0",
            "--max-diff",
            "0.25",
        ]))
        .unwrap();
        assert_eq!(o.mode, Mode::Check);
        assert_eq!(o.dir, Some(PathBuf::from("D")));
        assert_eq!(o.bicraft, Some(PathBuf::from("B")));
        assert_eq!(o.only.as_deref(), Some("slicer"));
        assert_eq!((o.tolerance, o.max_diff), (0, 0.25));
    }

    #[test]
    fn bad_arguments_are_refused() {
        for (a, says) in [
            (vec![], "record or check"),
            (vec!["replay"], "unknown mode"),
            (vec!["check", "--frobnicate"], "unknown option"),
            (vec!["check", "--dir"], "needs a value"),
            (vec!["check", "--only", "export"], "no walkthrough `export`"),
            (vec!["check", "--tolerance", "256"], "0..=255"),
            (vec!["check", "--tolerance", "-1"], "0..=255"),
            (vec!["check", "--max-diff", "1.5"], "0..=1"),
            (vec!["check", "--max-diff", "x"], "0..=1"),
        ] {
            let e = parse_args(&args(&a)).unwrap_err();
            assert!(e.contains(says), "{a:?}: {e}");
        }
    }

    #[test]
    fn every_walkthrough_can_be_named() {
        let ids: Vec<&str> = walkthroughs().iter().map(|w| w.id).collect();
        assert_eq!(ids, WALKS);
        for w in walkthroughs() {
            assert!(parse_args(&args(&["check", "--only", w.id])).is_ok());
            // step ids name the screenshots: unique in a walk, file-name safe
            let mut seen = std::collections::HashSet::new();
            for s in &w.steps {
                assert!(seen.insert(s.id), "{}: step {} twice", w.id, s.id);
                assert!(s.id.bytes().all(|b| b.is_ascii_lowercase() || b == b'-'));
                for a in &s.actions {
                    assert!(a.get("action").and_then(Value::as_str).is_some());
                }
            }
            assert!(w.states || w.steps.first().is_some_and(|s| s.id == "open"));
        }
        // `bicraft-cli run --cmd ID={JSON}`: the drill and bookmark walks need these
        let cmds = fixture_commands();
        for c in &cmds {
            let (id, params) = c.split_once('=').unwrap();
            assert!(id.contains('.'));
            assert!(serde_json::from_str::<Value>(params).unwrap().is_object());
        }
        assert!(cmds[0].contains(DRILL_CHART) && cmds[1].contains(BOOKMARK));
    }

    #[test]
    fn a_missing_tool_is_not_found_and_the_command_skips() {
        let empty = temp("empty");
        let missing = empty.join(TOOL);
        // an override naming no file, an empty PATH folder, no install folder
        assert_eq!(find_tool(Some(missing.clone().into()), None, None), None);
        assert_eq!(
            find_tool(None, Some(empty.clone().into()), Some(empty.clone().into())),
            None
        );
        assert_eq!(find_tool(None, None, None), None);
        // the command itself: a note and success, nothing built or started
        let o = parse_args(&args(&["check", "--dir", "never-used"])).unwrap();
        assert_eq!(run_with(&o, None), Ok(()));
        assert!(skip_note().starts_with("note: craft-anything-cli was not found"));
        let _ = std::fs::remove_dir_all(empty);
    }

    #[test]
    fn the_tool_is_found_by_override_path_then_install_folder() {
        let d = temp("found");
        let on_path = d.join("bin");
        let install = d.join("pf").join("Craft Anything");
        std::fs::create_dir_all(&on_path).unwrap();
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(on_path.join(TOOL), b"").unwrap();
        std::fs::write(install.join(TOOL), b"").unwrap();
        let path_var = std::env::join_paths([d.join("nothing"), on_path.clone()]).unwrap();
        let pf: OsString = d.join("pf").into();
        assert_eq!(
            find_tool(None, Some(path_var.clone()), Some(pf.clone())),
            Some(on_path.join(TOOL))
        );
        assert_eq!(find_tool(None, None, Some(pf.clone())), Some(install.join(TOOL)));
        let other = d.join("other.exe");
        std::fs::write(&other, b"").unwrap();
        assert_eq!(
            find_tool(Some(other.clone().into()), Some(path_var), Some(pf)),
            Some(other)
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_default_folder_is_the_local_data_folder() {
        let d = default_dir(Some("LAD".into()), Some("XDG".into()), Some("HOME".into())).unwrap();
        let base = if cfg!(windows) { "LAD" } else { "XDG" };
        assert_eq!(d, Path::new(base).join("bicraft").join("webcheck"));
        let d = default_dir(None, None, Some("HOME".into()));
        if cfg!(windows) {
            assert_eq!(d, None);
        } else {
            assert_eq!(
                d,
                Some(Path::new("HOME/.local/share/bicraft/webcheck").to_path_buf())
            );
        }
    }

    #[test]
    fn the_table_lists_every_step_and_the_totals() {
        let t = table(&[
            row("open", "open", Outcome::Pass(0.0)),
            row("slicer", "east", Outcome::Changed(0.0123, "10,20 30x40".into())),
            row("drill", "up", Outcome::Fail("no baseline".into())),
            row("pages", "tables", Outcome::Recorded),
        ]);
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines[0], "walk    step    result    detail");
        assert_eq!(lines[1], "open    open    pass      0.000 % of pixels changed");
        assert_eq!(
            lines[2],
            "slicer  east    FAIL      1.230 % of pixels changed, in 10,20 30x40"
        );
        assert_eq!(lines[3], "drill   up      FAIL      no baseline");
        assert_eq!(lines[4], "pages   tables  recorded");
        assert_eq!(lines[6], "4 step(s), 2 passed, 2 failed");
        assert_eq!(table(&[]).lines().last(), Some("0 step(s), 0 passed, 0 failed"));
    }

    #[test]
    fn the_verdict_follows_the_pixel_comparison() {
        let same = json!({"comparable": true, "difference_fraction": 0.0, "changed_bounding_box": null});
        assert_eq!(verdict(&same, 0.0005), Outcome::Pass(0.0));
        let small = json!({"comparable": true, "difference_fraction": 0.0004});
        assert_eq!(verdict(&small, 0.0005), Outcome::Pass(0.0004));
        let big = json!({"comparable": true, "difference_fraction": 0.02,
                         "changed_bounding_box": {"x": 5, "y": 6, "width": 70, "height": 80}});
        assert_eq!(verdict(&big, 0.0005), Outcome::Changed(0.02, "5,6 70x80".into()));
        let sizes = json!({"comparable": false, "before": {"width": 10, "height": 20},
                           "after": {"width": 10, "height": 30}});
        assert_eq!(
            verdict(&sizes, 0.5),
            Outcome::Fail("not comparable (sizes 10x20 and 10x30)".into())
        );
        assert!(matches!(verdict(&json!({}), 0.5), Outcome::Fail(_)));
    }

    #[test]
    fn the_viewer_line_token_and_answers_are_read() {
        assert_eq!(
            viewer_url("bicraft: live viewer at http://127.0.0.1:5123/\n").as_deref(),
            Some("http://127.0.0.1:5123/")
        );
        assert_eq!(viewer_url("bicraft: live viewer at http://example.com/"), None);
        assert_eq!(viewer_url("bicraft: watching x"), None);
        let page = r#"<head><meta name="bicraft-interact-token" content="0123abcd"></head>"#;
        assert_eq!(interact_token(page).as_deref(), Some("0123abcd"));
        assert_eq!(
            interact_token(r#"<meta name="bicraft-interact-token" content="">"#),
            None
        );
        assert_eq!(interact_token("<html>"), None);
        let plain = b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n{\"ok\":true}";
        assert_eq!(parse_response(plain), Ok((200, "{\"ok\":true}".into())));
        let chunked =
            b"HTTP/1.1 400 Bad\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
        assert_eq!(parse_response(chunked), Ok((400, "abcde".into())));
        assert!(parse_response(b"garbage").is_err());
    }

    #[test]
    fn tool_answers_and_images_are_decoded() {
        let ok = json!({"result": {"isError": false, "structuredContent": {"result": {"x": 1}}}});
        assert_eq!(tool_result("t", &ok), Ok(json!({"x": 1})));
        let refused = json!({"result": {"isError": true,
            "structuredContent": {"error": {"message": "Restricted mode never does this"}}}});
        assert_eq!(
            tool_result("t", &refused),
            Err("t: Restricted mode never does this".into())
        );
        assert!(tool_result("t", &json!({"error": {"code": -32601}})).is_err());
        assert!(tool_result("t", &json!({"result": {}})).is_err());
        assert_eq!(decode_b64("aGVsbG8="), Some(b"hello".to_vec()));
        assert_eq!(decode_b64("aGVsbG8"), Some(b"hello".to_vec()));
        assert_eq!(decode_b64(""), Some(vec![]));
        assert_eq!(decode_b64("a*b"), None);
    }
}
