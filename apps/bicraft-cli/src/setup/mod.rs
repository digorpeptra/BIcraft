//! `bicraft setup`: register the `bicraft mcp` server with agent clients (Claude Code, Claude
//! Desktop, VS Code, Cursor, Codex).
//!
//! Setup detects the clients, plans the change for each selected one, shows the plan, and writes
//! only with `--yes` or an interactive confirmation. Every write backs up the file first
//! (`<file>.bicraft.bak`), replaces it atomically, keeps unrelated content and formatting, and is
//! verified by reading the file back. A malformed configuration blocks the whole run instead of
//! being overwritten. `--config-root DIR` (or `BICRAFT_SETUP_HOME`) resolves every path under
//! `DIR` instead of the real home directory, for tests and trial runs.

mod clients;
mod doctor;
mod fsio;
mod json_cfg;
mod toml_cfg;

use std::io::{BufRead, IsTerminal};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Failure, INVALID, OK, USAGE_ERROR, usage};
use clients::{Client, Ctx, Format, Platform, Scope};

/// The server's name in every client configuration.
pub const SERVER_NAME: &str = "bicraft";
/// The arguments that start the MCP server.
pub const SERVER_ARGS: [&str; 1] = ["mcp"];

/// The environment variable that sandboxes every path, like `--config-root`.
const ROOT_ENV: &str = "BICRAFT_SETUP_HOME";

pub const USAGE: &str = "usage: bicraft setup [--client ID]... [--all-detected] [--scope user|project]
                      [--project-dir DIR] [--command PATH] [--config-root DIR]
                      [--dry-run] [--yes] [--json] [--uninstall]
       bicraft setup uninstall [same options]
       bicraft setup doctor [--client ID]... [--scope user|project] [--project-dir DIR]
                             [--command PATH] [--config-root DIR] [--json]

Registers `bicraft mcp` (this binary, by absolute path) with agent clients.
Clients: claude_code, claude_desktop, vscode, cursor, codex.

  --client ID      configure this client (repeatable); skips the selection prompt
  --all-detected   configure every detected client
  --scope S        user (default) or project (files in --project-dir, default: current dir)
  --command PATH   register PATH instead of the running binary (absolute)
  --config-root D  resolve every configuration path under D instead of the home directory
                   (also BICRAFT_SETUP_HOME); --platform windows|linux|macos picks the layout
  --dry-run        show the plan and write nothing
  --yes            apply without asking
  --json           print one JSON document on stdout
  --uninstall      remove the bicraft entry (and nothing else)

Without --client or --all-detected, setup asks in a terminal; otherwise it only refreshes
clients where bicraft is already registered.
Exit codes: 0 planned, applied, unchanged, cancelled or nothing selected; 1 blocked (malformed
configuration) or needs confirmation (doctor: not healthy); 2 usage or I/O error.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Install,
    Uninstall,
}

/// What a configuration file holds for bicraft.
#[derive(Debug)]
pub enum EntryState {
    Absent,
    /// Registered with this command and `mcp`.
    Aligned,
    /// Registered differently (another binary path, other arguments…).
    Stale {
        command: Option<String>,
    },
}

/// The change a run would make to one file.
#[derive(Debug)]
pub enum Change {
    None,
    Write(Vec<u8>),
    /// A change is needed but cannot be made safely; the reason.
    Blocked(String),
}

#[derive(Debug)]
pub struct Analysis {
    pub state: EntryState,
    pub change: Change,
}

/// The registration to write.
pub struct Want {
    pub command: String,
    pub platform: Platform,
}

impl Want {
    /// Whether `other` names the same binary (case- and separator-insensitive on Windows).
    pub fn same_command(&self, other: &str) -> bool {
        if self.platform == Platform::Windows {
            let norm = |s: &str| {
                let s = s.strip_prefix(r"\\?\").unwrap_or(s);
                s.replace('/', "\\").to_lowercase()
            };
            norm(&self.command) == norm(other)
        } else {
            self.command == other
        }
    }
}

struct Opts {
    mode: Mode,
    doctor: bool,
    clients: Vec<Client>,
    all_detected: bool,
    scope: Scope,
    project_dir: Option<PathBuf>,
    command: Option<PathBuf>,
    root: Option<PathBuf>,
    platform: Option<Platform>,
    dry_run: bool,
    yes: bool,
    json: bool,
}

fn value(it: &mut std::iter::Peekable<std::slice::Iter<'_, String>>, name: &str) -> Result<String, Failure> {
    it.next()
        .cloned()
        .ok_or_else(|| usage(format!("setup: {name} needs a value")))
}

fn parse(args: &[String]) -> Result<Option<Opts>, Failure> {
    let mut o = Opts {
        mode: Mode::Install,
        doctor: false,
        clients: Vec::new(),
        all_detected: false,
        scope: Scope::User,
        project_dir: None,
        command: None,
        root: None,
        platform: None,
        dry_run: false,
        yes: false,
        json: false,
    };
    let mut it = args.iter().peekable();
    match it.peek().map(|s| s.as_str()) {
        Some("doctor") => {
            o.doctor = true;
            it.next();
        }
        Some("uninstall") => {
            o.mode = Mode::Uninstall;
            it.next();
        }
        Some("help" | "--help" | "-h") => return Ok(None),
        _ => {}
    }
    while let Some(a) = it.next() {
        match a.as_str() {
            "--client" => {
                let v = value(&mut it, "--client")?;
                for id in v.split(',').filter(|s| !s.trim().is_empty()) {
                    let c = Client::parse(id).ok_or_else(|| {
                        let ids: Vec<&str> = Client::ALL.iter().map(|c| c.id()).collect();
                        usage(format!(
                            "setup: unknown client `{id}` (one of: {})",
                            ids.join(", ")
                        ))
                    })?;
                    if !o.clients.contains(&c) {
                        o.clients.push(c);
                    }
                }
            }
            "--all-detected" => o.all_detected = true,
            "--scope" => {
                o.scope = match value(&mut it, "--scope")?.as_str() {
                    "user" | "global" => Scope::User,
                    "project" | "workspace" => Scope::Project,
                    other => return Err(usage(format!("setup: --scope is user or project, not `{other}`"))),
                }
            }
            "--project-dir" => o.project_dir = Some(PathBuf::from(value(&mut it, "--project-dir")?)),
            "--command" => o.command = Some(PathBuf::from(value(&mut it, "--command")?)),
            "--config-root" => o.root = Some(PathBuf::from(value(&mut it, "--config-root")?)),
            "--platform" => {
                let v = value(&mut it, "--platform")?;
                o.platform = Some(Platform::parse(&v).ok_or_else(|| {
                    usage(format!("setup: --platform is windows, linux or macos, not `{v}`"))
                })?);
            }
            "--dry-run" => o.dry_run = true,
            "--yes" | "-y" => o.yes = true,
            "--json" => o.json = true,
            "--uninstall" => o.mode = Mode::Uninstall,
            "--help" | "-h" => return Ok(None),
            other => return Err(usage(format!("setup: unexpected argument `{other}`\n\n{USAGE}"))),
        }
    }
    if o.doctor && (o.all_detected || o.dry_run || o.yes || o.mode == Mode::Uninstall) {
        return Err(usage(
            "setup doctor takes --client, --scope, --project-dir, --command, --config-root, --json",
        ));
    }
    Ok(Some(o))
}

/// Entry point of `bicraft setup ARGS`.
pub fn main(args: &[String]) -> Result<u8, Failure> {
    let Some(mut opts) = parse(args)? else {
        outln!("{USAGE}");
        return Ok(OK);
    };
    let cwd = std::env::current_dir().map_err(|e| (USAGE_ERROR, format!("setup: current directory: {e}")))?;
    let absolute = |p: PathBuf| -> PathBuf {
        let p = if p.is_absolute() { p } else { cwd.join(p) };
        p.components().collect()
    };
    if opts.root.is_none() {
        opts.root = bicraft_env::var_os(ROOT_ENV)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
    }
    let root = opts.root.take().map(absolute);
    if opts.platform.is_some() && root.is_none() {
        return Err(usage(
            "setup: --platform needs --config-root (or BICRAFT_SETUP_HOME)",
        ));
    }
    let platform = opts.platform.unwrap_or_else(Platform::current);
    let project_dir = absolute(opts.project_dir.take().unwrap_or_else(|| cwd.clone()));
    let ctx = Ctx::new(root, platform, project_dir).map_err(|e| (USAGE_ERROR, format!("setup: {e}")))?;
    let want = Want {
        command: resolve_command(opts.command.take())?,
        platform,
    };
    for c in &opts.clients {
        if ctx.locate(*c, opts.scope).is_none() {
            return Err(usage(format!(
                "setup: {} has no {} scope",
                c.id(),
                opts.scope.id()
            )));
        }
    }
    if opts.doctor {
        run_doctor(&opts, &ctx, &want)
    } else {
        run_setup(&opts, &ctx, &want)
    }
}

fn resolve_command(given: Option<PathBuf>) -> Result<String, Failure> {
    let path = match given {
        Some(p) => {
            if !p.is_absolute() {
                return Err(usage(format!(
                    "setup: --command must be an absolute path, not `{}`",
                    p.display()
                )));
            }
            p
        }
        None => std::env::current_exe().map_err(|e| {
            (
                USAGE_ERROR,
                format!("setup: cannot locate the running binary: {e}"),
            )
        })?,
    };
    let s = path.to_str().ok_or_else(|| {
        (
            USAGE_ERROR,
            format!("setup: the binary path is not UTF-8: {}", path.display()),
        )
    })?;
    let s = match s.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => s.strip_prefix(r"\\?\").unwrap_or(s).to_string(),
    };
    Ok(s)
}

/// One client in the plan.
struct Row {
    client: Client,
    path: PathBuf,
    detected: bool,
    original: Option<Vec<u8>>,
    /// `Err`: the file cannot be read or is malformed.
    analysis: Result<Analysis, String>,
    selected: bool,
    /// `Some(Ok)` written, `Some(Err)` failed or skipped.
    result: Option<Result<(), String>>,
    backup: Option<PathBuf>,
}

fn analyze(client: Client, bytes: Option<&[u8]>, want: &Want, mode: Mode) -> Result<Analysis, String> {
    match client.format() {
        Format::Json { servers_key, typed } => json_cfg::analyze(bytes, servers_key, typed, want, mode),
        Format::Toml => toml_cfg::analyze(bytes, want, mode),
    }
}

/// Why writes under `%APPDATA%` are refused from inside Claude Desktop's package.
const REDIRECTED: &str = "this process runs inside Claude Desktop's MSIX package (for example a                           terminal it started), so writes under %APPDATA% and %LOCALAPPDATA% would land                           in the package's private storage; run setup from an ordinary terminal";

fn evaluate(
    ctx: &Ctx,
    client: Client,
    scope: Scope,
    want: &Want,
    mode: Mode,
    redirected: bool,
) -> Option<Row> {
    let loc = ctx.locate(client, scope)?;
    let (original, mut analysis) = match fsio::read(&loc.path) {
        Ok(bytes) => {
            let a = analyze(client, bytes.as_deref(), want, mode);
            (bytes, a)
        }
        Err(e) => (None, Err(format!("cannot read the file: {e}"))),
    };
    if let Ok(a) = &mut analysis
        && redirected
        && matches!(a.change, Change::Write(_))
        && ctx.in_app_data(&loc.path)
    {
        a.change = Change::Blocked(REDIRECTED.to_string());
    }
    Some(Row {
        client,
        detected: loc.detected || original.is_some(),
        path: loc.path,
        original,
        analysis,
        selected: false,
        result: None,
        backup: None,
    })
}

impl Row {
    fn registration(&self) -> &'static str {
        match &self.analysis {
            Err(_) => "invalid",
            Ok(a) => match a.state {
                EntryState::Absent => "absent",
                EntryState::Aligned => "aligned",
                EntryState::Stale { .. } => "stale",
            },
        }
    }

    fn registered(&self) -> bool {
        matches!(&self.analysis, Ok(a) if !matches!(a.state, EntryState::Absent))
    }

    fn registered_command(&self) -> Option<&str> {
        match &self.analysis {
            Ok(Analysis {
                state: EntryState::Stale { command },
                ..
            }) => command.as_deref(),
            _ => None,
        }
    }

    fn error(&self) -> Option<&str> {
        match &self.analysis {
            Err(e) => Some(e),
            Ok(Analysis {
                change: Change::Blocked(why),
                ..
            }) => Some(why),
            Ok(_) => match &self.result {
                Some(Err(e)) => Some(e),
                _ => None,
            },
        }
    }

    fn blocked(&self) -> bool {
        matches!(
            &self.analysis,
            Err(_)
                | Ok(Analysis {
                    change: Change::Blocked(_),
                    ..
                })
        )
    }

    fn pending(&self) -> Option<&[u8]> {
        match &self.analysis {
            Ok(Analysis {
                change: Change::Write(b),
                ..
            }) => Some(b),
            _ => None,
        }
    }

    fn action(&self, mode: Mode) -> &'static str {
        if self.blocked() {
            return "invalid";
        }
        match (&self.analysis, mode) {
            (Ok(a), Mode::Install) => match a.state {
                EntryState::Absent => "add",
                EntryState::Stale { .. } => "update",
                EntryState::Aligned => "unchanged",
            },
            (Ok(a), Mode::Uninstall) => match a.state {
                EntryState::Absent => "none",
                _ => "remove",
            },
            (Err(_), _) => "invalid",
        }
    }

    fn json(&self, mode: Mode) -> Value {
        let result = match &self.result {
            None => Value::Null,
            Some(Ok(())) => json!("written"),
            Some(Err(_)) => json!("failed"),
        };
        json!({
            "client": self.client.id(),
            "name": self.client.name(),
            "path": self.path.display().to_string(),
            "detected": self.detected,
            "exists": self.original.is_some(),
            "selected": self.selected,
            "registration": self.registration(),
            "registered_command": self.registered_command(),
            "action": if self.selected { self.action(mode) } else { "skip" },
            "error": self.error(),
            "backup": self.backup.as_ref().map(|p| p.display().to_string()),
            "result": result,
        })
    }
}

fn run_setup(opts: &Opts, ctx: &Ctx, want: &Want) -> Result<u8, Failure> {
    let mode = opts.mode;
    let redirected = ctx.msix_redirected();
    let mut rows: Vec<Row> = Client::ALL
        .into_iter()
        .filter_map(|c| evaluate(ctx, c, opts.scope, want, mode, redirected))
        .collect();
    let interactive = !opts.json
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::io::stderr().is_terminal();

    let mut warnings = Vec::new();
    if redirected {
        warnings.push(REDIRECTED.to_string());
    }
    if !opts.clients.is_empty() || opts.all_detected {
        for r in &mut rows {
            r.selected = opts.clients.contains(&r.client) || (opts.all_detected && r.detected);
        }
    } else if interactive && !opts.yes {
        match prompt_selection(&rows, mode)? {
            Some(chosen) => {
                for r in &mut rows {
                    r.selected = chosen.contains(&r.client);
                }
            }
            None => return finish(opts, ctx, want, &rows, "cancelled", OK, &warnings),
        }
    } else {
        for r in &mut rows {
            r.selected = r.registered();
        }
    }

    if mode == Mode::Install {
        if opts.scope == Scope::Project {
            warnings.push(
                "project files are often committed: the registered command is an absolute path on this \
                 machine"
                    .to_string(),
            );
        }
        if looks_like_build_output(&want.command) {
            warnings.push(format!(
                "{} is a cargo build output; a rebuild or `cargo clean` replaces it. Consider \
                 `cargo install --path apps/bicraft-cli`, then run setup from the installed binary",
                want.command
            ));
        }
        if !Path::new(&want.command).is_file() {
            warnings.push(format!("{} does not exist", want.command));
        }
    }
    let selected: Vec<&Row> = rows.iter().filter(|r| r.selected).collect();
    if selected
        .iter()
        .any(|r| r.client == Client::ClaudeDesktop && r.pending().is_some())
    {
        warnings.push(
            "quit Claude Desktop fully before applying: a running app can rewrite its configuration"
                .to_string(),
        );
    }

    let (status, code) = if selected.is_empty() {
        ("nothing_selected", OK)
    } else if selected.iter().any(|r| r.blocked()) {
        ("blocked", INVALID)
    } else if selected.iter().all(|r| r.pending().is_none()) {
        ("unchanged", OK)
    } else if opts.dry_run {
        ("planned", OK)
    } else if opts.yes {
        ("apply", OK)
    } else if interactive {
        print_plan(ctx, want, &rows, mode, &warnings);
        if confirm()? {
            ("apply", OK)
        } else {
            return finish(opts, ctx, want, &rows, "cancelled", OK, &warnings);
        }
    } else {
        ("needs_confirmation", INVALID)
    };
    if status != "apply" {
        return finish(opts, ctx, want, &rows, status, code, &warnings);
    }
    let ok = apply(&mut rows, want, mode);
    if ok {
        finish(opts, ctx, want, &rows, "applied", OK, &warnings)
    } else {
        finish(opts, ctx, want, &rows, "failed", USAGE_ERROR, &warnings)
    }
}

fn looks_like_build_output(command: &str) -> bool {
    let parts: Vec<String> = command.split(['/', '\\']).map(str::to_ascii_lowercase).collect();
    parts.windows(2).any(|w| match w {
        [a, b] => a.contains("target") && (b == "debug" || b == "release"),
        _ => false,
    }) || parts
        .windows(3)
        .any(|w| matches!(w, [a, _, c] if a.contains("target") && (c == "debug" || c == "release")))
}

/// Writes every pending change; stops at the first failure. True if all succeeded.
fn apply(rows: &mut [Row], want: &Want, mode: Mode) -> bool {
    let mut failed = false;
    for r in rows.iter_mut().filter(|r| r.selected) {
        let Some(bytes) = r.pending().map(<[u8]>::to_vec) else {
            continue;
        };
        if failed {
            r.result = Some(Err("skipped after an earlier failure".into()));
            continue;
        }
        let outcome = (|| -> Result<Option<PathBuf>, String> {
            let now = fsio::read(&r.path).map_err(|e| format!("cannot read the file: {e}"))?;
            if now != r.original {
                return Err("the file changed after the plan was made; run setup again".into());
            }
            let backup = match &r.original {
                Some(orig) => Some(fsio::backup(&r.path, orig).map_err(|e| format!("cannot back up: {e}"))?),
                None => None,
            };
            fsio::write_atomic(&r.path, &bytes).map_err(|e| format!("cannot write: {e}"))?;
            let back = fsio::read(&r.path).map_err(|e| format!("cannot read back: {e}"))?;
            let check = analyze(r.client, back.as_deref(), want, mode)
                .map_err(|e| format!("verification failed: {e}"))?;
            if !matches!(check.change, Change::None) {
                return Err("verification failed: the file does not hold the expected entry".into());
            }
            Ok(backup)
        })();
        match outcome {
            Ok(backup) => {
                r.backup = backup;
                r.result = Some(Ok(()));
            }
            Err(e) => {
                r.result = Some(Err(e));
                failed = true;
            }
        }
    }
    !failed
}

fn prompt_selection(rows: &[Row], mode: Mode) -> Result<Option<Vec<Client>>, Failure> {
    let verb = match mode {
        Mode::Install => "register bicraft with",
        Mode::Uninstall => "remove bicraft from",
    };
    eprintln!("Clients (* = already registered):");
    for (i, r) in rows.iter().enumerate() {
        let mark = if r.registered() { "*" } else { " " };
        let state = if r.detected { "detected" } else { "not detected" };
        eprintln!(
            "  {}) {mark} {:<15} {state:<13} {}",
            i + 1,
            r.client.id(),
            r.path.display()
        );
    }
    let defaults: Vec<Client> = rows.iter().filter(|r| r.registered()).map(|r| r.client).collect();
    eprintln!(
        "Which clients should setup {verb}? Numbers or ids separated by commas, `all` for every \
         detected one, Enter for the registered ones, `q` to cancel:"
    );
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| (USAGE_ERROR, format!("setup: cannot read the answer: {e}")))?;
    let answer = line.trim();
    if answer.eq_ignore_ascii_case("q") || answer.eq_ignore_ascii_case("quit") {
        return Ok(None);
    }
    if answer.is_empty() {
        return Ok(Some(defaults));
    }
    if answer.eq_ignore_ascii_case("all") {
        return Ok(Some(
            rows.iter().filter(|r| r.detected).map(|r| r.client).collect(),
        ));
    }
    let mut chosen = Vec::new();
    for part in answer.split([',', ' ']).filter(|p| !p.is_empty()) {
        let client = match part.parse::<usize>() {
            Ok(n) => n.checked_sub(1).and_then(|i| rows.get(i)).map(|r| r.client),
            Err(_) => Client::parse(part).filter(|c| rows.iter().any(|r| r.client == *c)),
        }
        .ok_or_else(|| usage(format!("setup: `{part}` is not one of the listed clients")))?;
        if !chosen.contains(&client) {
            chosen.push(client);
        }
    }
    Ok(Some(chosen))
}

fn confirm() -> Result<bool, Failure> {
    eprint!("Apply these changes? [y/N] ");
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| (USAGE_ERROR, format!("setup: cannot read the answer: {e}")))?;
    Ok(matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
}

fn header(ctx: &Ctx, want: &Want, what: &str) {
    outln!("bicraft setup{what}");
    outln!(
        "  server   {SERVER_NAME}: {} {}",
        want.command,
        SERVER_ARGS.join(" ")
    );
    if let Some(root) = &ctx.root {
        outln!(
            "  root     {} (sandbox, {} layout)",
            root.display(),
            ctx.platform.id()
        );
    }
}

fn print_plan(ctx: &Ctx, want: &Want, rows: &[Row], mode: Mode, warnings: &[String]) {
    let op = match mode {
        Mode::Install => "",
        Mode::Uninstall => " (uninstall)",
    };
    header(ctx, want, op);
    outln!("");
    for r in rows.iter().filter(|r| r.selected) {
        let mut line = format!(
            "  {:<15} {:<10} {}",
            r.client.id(),
            r.action(mode),
            r.path.display()
        );
        if let Some(cmd) = r.registered_command() {
            line.push_str(&format!("\n{:<28}was: {cmd}", ""));
        }
        if let Some(e) = r.error() {
            line.push_str(&format!("\n{:<28}{e}", ""));
        }
        if let Some(b) = &r.backup {
            line.push_str(&format!("\n{:<28}backup: {}", "", b.display()));
        }
        outln!("{line}");
    }
    let others: Vec<String> = rows
        .iter()
        .filter(|r| !r.selected && r.detected)
        .map(|r| format!("{} ({})", r.client.id(), r.registration()))
        .collect();
    if !others.is_empty() {
        outln!("  not selected: {}", others.join(", "));
    }
    for w in warnings {
        outln!("warning: {w}");
    }
}

fn finish(
    opts: &Opts,
    ctx: &Ctx,
    want: &Want,
    rows: &[Row],
    status: &str,
    code: u8,
    warnings: &[String],
) -> Result<u8, Failure> {
    let mode = opts.mode;
    let restart: Vec<&str> = rows
        .iter()
        .filter(|r| matches!(r.result, Some(Ok(()))))
        .map(|r| r.client.name())
        .collect();
    if opts.json {
        let v = json!({
            "status": status,
            "operation": match mode { Mode::Install => "install", Mode::Uninstall => "uninstall" },
            "scope": opts.scope.id(),
            "dry_run": opts.dry_run,
            "server": {"name": SERVER_NAME, "command": want.command, "args": SERVER_ARGS},
            "config_root": ctx.root.as_ref().map(|p| p.display().to_string()),
            "platform": ctx.platform.id(),
            "clients": rows.iter().map(|r| r.json(mode)).collect::<Vec<_>>(),
            "restart": restart,
            "warnings": warnings,
            "exit_code": code,
        });
        super::print_json(&v);
        return Ok(code);
    }
    if status != "cancelled" {
        print_plan(ctx, want, rows, mode, warnings);
    }
    let ids: Vec<&str> = Client::ALL.iter().map(|c| c.id()).collect();
    let msg = match status {
        "planned" => "Dry run: nothing was written. Apply with --yes.".to_string(),
        "applied" => format!("Done. Restart {} to load the change.", restart.join(", ")),
        "unchanged" => "Nothing to change: every selected client is up to date.".to_string(),
        "nothing_selected" => format!(
            "No client selected. Choose with --client ID ({}) or --all-detected.",
            ids.join(", ")
        ),
        "needs_confirmation" => {
            "Not applied: stdin is not a terminal; rerun with --yes to apply this plan.".to_string()
        }
        "blocked" => {
            "Not applied: fix (or deselect) the invalid configuration above. Nothing was written.".to_string()
        }
        "cancelled" => "Cancelled: nothing was written.".to_string(),
        _ => "Failed: see the errors above. Files already written keep their backups.".to_string(),
    };
    outln!("");
    outln!("{msg}");
    Ok(code)
}

fn run_doctor(opts: &Opts, ctx: &Ctx, want: &Want) -> Result<u8, Failure> {
    let rows: Vec<Row> = Client::ALL
        .into_iter()
        .filter_map(|c| evaluate(ctx, c, opts.scope, want, Mode::Install, false))
        .collect();
    let handshake = doctor::handshake(Path::new(&want.command));
    let explicit = !opts.clients.is_empty();
    let mut problems = Vec::new();
    let mut warnings = Vec::new();
    if ctx.msix_redirected() {
        warnings.push(REDIRECTED.to_string());
    }
    if handshake.get("ok") != Some(&Value::Bool(true)) {
        let e = handshake.get("error").and_then(Value::as_str).unwrap_or("failed");
        problems.push(format!("`{} mcp`: {e}", want.command));
    }
    let mut clients = Vec::new();
    for r in &rows {
        let in_scope = !explicit || opts.clients.contains(&r.client);
        let reg = r.registration();
        let cmd_exists = r.registered_command().map(|c| Path::new(c).is_file());
        if in_scope {
            match reg {
                "stale" => problems.push(format!(
                    "{}: registered with another command ({}); run `bicraft setup --client {}`",
                    r.client.id(),
                    r.registered_command().unwrap_or("none"),
                    r.client.id()
                )),
                "absent" if explicit => problems.push(format!(
                    "{}: not registered; run `bicraft setup --client {}`",
                    r.client.id(),
                    r.client.id()
                )),
                "invalid" if explicit => {
                    problems.push(format!("{}: {}", r.client.id(), r.error().unwrap_or("invalid")))
                }
                "invalid" => warnings.push(format!("{}: {}", r.client.id(), r.error().unwrap_or("invalid"))),
                _ => {}
            }
        }
        clients.push(json!({
            "client": r.client.id(),
            "name": r.client.name(),
            "path": r.path.display().to_string(),
            "detected": r.detected,
            "exists": r.original.is_some(),
            "in_scope": in_scope,
            "registration": reg,
            "registered_command": r.registered_command(),
            "registered_command_exists": cmd_exists,
            "error": r.error(),
        }));
    }
    let healthy = problems.is_empty();
    let code = if healthy { OK } else { INVALID };
    if opts.json {
        super::print_json(&json!({
            "healthy": healthy,
            "scope": opts.scope.id(),
            "server": {"name": SERVER_NAME, "command": want.command, "args": SERVER_ARGS},
            "config_root": ctx.root.as_ref().map(|p| p.display().to_string()),
            "platform": ctx.platform.id(),
            "handshake": handshake,
            "clients": clients,
            "problems": problems,
            "warnings": warnings,
            "exit_code": code,
        }));
        return Ok(code);
    }
    header(ctx, want, " doctor");
    if handshake.get("ok") == Some(&Value::Bool(true)) {
        outln!(
            "  mcp      ok: {} {}, protocol {}, {} tools",
            handshake
                .get("server_name")
                .and_then(Value::as_str)
                .unwrap_or("?"),
            handshake
                .get("server_version")
                .and_then(Value::as_str)
                .unwrap_or("?"),
            handshake
                .get("protocol_version")
                .and_then(Value::as_str)
                .unwrap_or("?"),
            handshake.get("tools").and_then(Value::as_u64).unwrap_or(0)
        );
    } else {
        outln!("  mcp      FAILED");
    }
    outln!("");
    for r in &rows {
        let state = if r.detected || r.original.is_some() {
            r.registration()
        } else {
            "not detected"
        };
        outln!("  {:<15} {:<13} {}", r.client.id(), state, r.path.display());
    }
    for w in &warnings {
        outln!("warning: {w}");
    }
    outln!("");
    if healthy {
        outln!("Healthy.");
    } else {
        outln!("Not healthy:");
        for p in &problems {
            outln!("  - {p}");
        }
    }
    Ok(code)
}
