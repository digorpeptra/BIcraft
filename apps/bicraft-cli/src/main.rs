//! The `bicraft-cli` command-line tool.
//!
//! ```text
//! bicraft-cli commands [FILTER]                            # list commands (JSON)
//! bicraft-cli describe ID                                  # one command (JSON)
//! bicraft-cli run [--open PATH] --cmd 'ID[={JSON}]' [--cmd ...] [--file PATH] [--save] [--no-atomic]
//!                                                           # run commands in order (JSON);
//!                                                           # --file: a JSON array of
//!                                                           # {command, params} (- for stdin);
//!                                                           # --save runs file.save at the end;
//!                                                           # all or nothing unless --no-atomic
//! bicraft-cli dax-query PATH (QUERY | --file PATH) [--max-rows N] [--timeout-ms N] [--no-cache-abf] [--as-of DATE] [--cache-abf F]
//!                                                           # dax.evaluate over the file's data
//! bicraft-cli refresh PATH [--allow-connector databricks]... [--table T]... [--external-links] [--data-root DIR]
//!                                                           # model.refresh (token: BICRAFT_DATABRICKS_TOKEN)
//! bicraft-cli new-project DIR NAME [--format tmdl|tmsl] [--open PATH]
//!                                                           # project.new (from PATH's model)
//! bicraft-cli inspect PATH                                 # file.open + file.inspect
//! bicraft-cli convert IN OUT [--format F] [--report-format keep|pbir] [--no-cache] [--empty-tables]
//!                  [--data-root DIR] [--overwrite] [--strict]
//!                                                           # file.convert (PBIX/PBIT/PBIP/bim)
//! bicraft-cli diff A B [--json|--markdown] [--model-only|--report-only] [--ignore-formatting]
//!              [--ignore-positions] [--ignore-whitespace] [--exit-code]
//!                                                           # file.diff (any formats; docs/diff.md)
//! bicraft-cli diff --git REV1 [REV2] PATH [options]        # a project between git revisions
//! bicraft-cli diff --textconv FILE                         # git textconv (file.textconv)
//! bicraft-cli validate-dax EXPRESSION [--model PATH] [--table T]
//! bicraft-cli validate-dax-query (QUERY | --file PATH) [--model PATH]
//! bicraft-cli validate-m (EXPRESSION | --file PATH) [--model PATH]
//!                                                           # dax.validate_query
//! bicraft-cli validate-model PATH
//! bicraft-cli analyze PATH [--rules FILE]... [--no-default-rules] [--rule-id ID]... [--severity-min S]
//!                                                           # model.analyze (best-practice rules)
//! bicraft-cli usage PATH [OBJECT] [--kind K] [--report P]... [--no-graph]   # model.usage
//! bicraft-cli unused PATH [--report P]... [--allow PATTERN]... [--ignore-metadata] [--remove [--object O]... [--dry-run] [--force] [--save]]
//!                                                           # model.unused / model.remove_unused
//! bicraft-cli validate-report PATH                         # report.validate (fields vs model)
//! bicraft-cli test [PATH] [--tests F] [--format text|json|junit|sarif] [--output F] [--fail-on error|warning]
//!                                                           # project.test (CI quality gates)
//! bicraft-cli sources PATH | sources rebase PATH --from A [--to B] [--param P] [--server O=N]... [--dry-run]
//!                                                           # sources.list / sources.rebase (saves)
//! bicraft-cli env apply|check PATH PROFILE [--env-file F] [--dry-run]   # env.apply / env.check
//! bicraft-cli render PATH [--page P] [--out F.png|DIR] [--scale N] [--lang TAG] [--culture NAME]
//!                 [--format-culture NAME] [--as-of DATE]    # report.render
//! bicraft-cli export-pdf PATH [--out F.pdf] [--pages all|current|2-4|NAMES] [--hidden]  # report.export_pdf
//! bicraft-cli mcp                                          # MCP server on stdio
//! bicraft-cli mcp --view [--view-port N] [--open] [--lang TAG] [--no-open-button]  # ... plus the live viewer
//! bicraft-cli serve --root DIR [--listen ADDR] [--allow-network] [--allow-origin O]... [--max-body-bytes N] [--queue N]
//!               [auth flags: --auth-config F, --public-url U, --oidc-issuer U, --api-tokens F,
//!               --trusted-proxy CIDR, --grant SUBJECT=WORKSPACES, --reader-grant ..., ... (docs/auth.md)]
//!                                                           # MCP over Streamable HTTP (2026-07-28),
//!                                                           # /w/<subfolder of DIR>/mcp; loopback only
//!                                                           # unless authentication is configured;
//!               [--max-upload-bytes N] [--shutdown-grace SECS] [--idle-timeout SECS]
//!               [--ready-memory-percent P] [--drain-on-stdin-close]  # files at /w/<id>/files/, /healthz, /readyz,
//!                                                           # graceful drain on SIGTERM / Ctrl+C
//! bicraft-cli serve token create --name NAME [--role R]... [--group G]...  # a new API token (hash entry)
//! bicraft-cli serve token hash                             # hash entry of a token read on stdin
//! bicraft-cli controller --config F [--listen ADDR] [--kube-api URL] [--namespace NS] [--admin SUBJECT]...
//!                    [--interval SECS] [--shutdown-grace SECS] [auth flags of serve]
//!                                                           # Kubernetes workspace controller + gateway
//! bicraft-cli vpax PATH OUT.vpax [--overwrite] [--no-model-bim] [--cache-abf F]  # model.export_vpax
//! bicraft-cli redact PATH OUT [--data] [--overwrite]      # file.redact: a copy without secrets
//! bicraft-cli visual new DIR NAME [--role R]... [--format F]... | build DIR [--out F] | pack DIR [--out F]
//! bicraft-cli visual export NAME... | --all [--out F.pbiviz|DIR]   # visual.export_bicraft_visual
//!                                                           # visual.scaffold / build / pack (swc, no Node)
//! bicraft-cli view PATH [--port N] [--open] [--data-root DIR] [--lang TAG] [--culture NAME]  # live viewer,
//!                                                           # re-read when it changes on disk;
//!                                                           # M queries may read files under DIR;
//!                                                           # slicers, cross-filtering, navigation
//!                                                           # (session only; --read-only: off)
//! bicraft-cli setup [--client ID]... [--dry-run] [--yes] [--json] [--uninstall]
//!                                                           # register `bicraft-cli mcp` with agents
//! bicraft-cli setup doctor [--client ID]... [--json]       # handshake + registration check
//! bicraft-cli licenses                                    # licence and third-party notices
//! bicraft-cli build-info                                  # version, commit, embedded browser app (JSON)
//! bicraft-cli --version | help
//! ```
//!
//! JSON goes to stdout, errors to stderr. Exit codes: 0 success / valid, 1 invalid or a command
//! failed, 2 usage or I/O error.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use std::process::ExitCode;

use bicraft_engine::Session;
use serde_json::{Value, json};

/// Writes a line to stdout; a closed pipe ends the process quietly.
macro_rules! outln {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        if let Err(e) = writeln!(std::io::stdout(), $($arg)*) {
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                std::process::exit(0);
            }
            eprintln!("bicraft: can't write to stdout: {e}");
            std::process::exit(2);
        }
    }};
}

mod controller_cmd;
mod diff_cmd;
mod env_cmd;
mod import_cmd;
mod refresh_cmd;
mod render_cmd;
mod serve_cmd;
mod setup;
#[cfg(test)]
#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;
mod test_cmd;
mod ui_cmd;
mod view_cmd;

const USAGE: &str = "usage: bicraft-cli commands [FILTER]
       bicraft-cli describe COMMAND
       bicraft-cli run [--open PATH] [--cmd 'ID[={JSON}]']... [--file COMMANDS.json] [--save] [--no-atomic]
       bicraft-cli dax-query PATH (QUERY | --file QUERY.dax) [--max-rows N] [--timeout-ms N] [--no-cache-abf]
                              [--as-of YYYY-MM-DD[THH:MM:SS]] [--cache-abf CACHE.abf]
       bicraft-cli refresh PATH [--allow-connector databricks]... [--table TABLE]... [--external-links]
                            [--data-root DIR] [--backup]
                            (PBIX: writes the new rows into the file's DataModel; PBIP: into
                            .pbi/bicraft-data.bin; the Databricks token comes from BICRAFT_DATABRICKS_TOKEN,
                            else a masked prompt, never an argument)
       bicraft-cli new-project DIR NAME [--format tmdl|tmsl] [--open PATH]
       bicraft-cli inspect PATH
       bicraft-cli convert IN OUT [--format pbix|pbit|pbip-tmdl|pbip-tmsl|bim]
                            [--report-format keep|pbir] [--no-cache] [--empty-tables]
                            [--data-root DIR] [--overwrite] [--strict]
                            (IN may be an Excel .xlsx/.xlsm workbook with a Power Pivot data model)
       bicraft-cli diff A B [--json|--markdown|--text] [--model-only|--report-only] [--ignore-formatting]
                        [--ignore-positions] [--ignore-whitespace] [--exit-code]
       bicraft-cli diff --git REV1 [REV2] PATH [diff options]   (REV2 omitted: the working tree)
       bicraft-cli diff --textconv FILE
       bicraft-cli import SOURCE TARGET [--measure SEL]... [--table SEL]... [--column TABLE[COLUMN]]...
                          [--calculation-group SEL]... [--function SEL]... [--expression SEL]... [--role SEL]...
                          [--page SEL]... [--visual PAGE/VISUAL]... [--target-page PAGE] [--bookmark SEL]... [--theme]
                          [--on-conflict fail|skip|replace|rename] [--rename-suffix S] [--into-table TABLE]
                          [--no-dependencies] [--no-relationships] [--allow-broken] [--dry-run] [--no-backup] [--json]
                          (copies objects from SOURCE into TARGET with what they use, then saves TARGET;
                          selectors are names, Table[Name] or * ? patterns: docs/mcp.md, Importing objects)
       bicraft-cli diff PATH OLD-FILE OLD-HEX OLD-MODE NEW-FILE NEW-HEX NEW-MODE   (git external diff)
                        (semantic diff of PBIX/PBIT/PBIP/model.bim, metadata only: docs/diff.md;
                        --exit-code: exit 1 when the files differ)
       bicraft-cli validate-dax EXPRESSION [--model PATH] [--table TABLE]
       bicraft-cli validate-dax-query (QUERY | --file PATH) [--model PATH]
       bicraft-cli validate-m (EXPRESSION | --file PATH) [--model PATH]
       bicraft-cli validate-model PATH
       bicraft-cli analyze PATH [--rules FILE]... [--no-default-rules] [--no-report-rules] [--rule-id ID]...
                            [--severity-min 1-5|info|warning|error] [--max-findings N]
       bicraft-cli usage PATH [OBJECT] [--kind table|column|measure|expression] [--report PATH]... [--no-graph]
                            (where OBJECT is used: DAX, RLS, relationships, the report; every object without it)
       bicraft-cli unused PATH [--report PATH]... [--allow PATTERN]... [--ignore-metadata]
                            [--remove [--object OBJECT]... [--dry-run] [--force] [--save]]
                            (unused measures, columns, calculated tables, M expressions; exit 1 when some are;
                            --ignore-metadata: perspectives, translations, Q&A keep nothing; --remove
                            removes them in one edit, --save writes the file)
       bicraft-cli validate-report PATH
       bicraft-cli test [PATH] [--tests FILE] [--format text|json|junit|sarif] [--output FILE]
                        [--fail-on error|warning]
                        (CI quality gates from bicraft.tests.yaml: docs/ci-tests.md; exit 0 passed,
                        1 a test failed, 2 usage error, invalid test file or unreadable project)
       bicraft-cli sources PATH
       bicraft-cli sources rebase PATH [--from ROOT] [--to ROOT] [--param NAME] [--server OLD=NEW]...
                        [--database OLD=NEW]... [--host OLD=NEW]... [--http-path OLD=NEW]... [--url OLD=NEW]...
                        [--dry-run]   (M data sources; rebase saves unless --dry-run: docs/sources-env.md)
       bicraft-cli env apply PATH PROFILE [--env-file FILE] [--dry-run]
       bicraft-cli env check PATH PROFILE [--env-file FILE]
                        (profiles of bicraft.env.yaml; check exits 1 when the profile leaves gaps)
       bicraft-cli mcp [--view [--view-port N] [--open] [--lang TAG] [--no-open-button]]
       bicraft-cli serve --root DIR [--listen 127.0.0.1:7878] [--allow-network] [--allow-origin ORIGIN]...
                         [--max-body-bytes N] [--queue N]
                         [--auth-config FILE] [--public-url URL] [--allow-host HOST]...
                         [--oidc-issuer URL]... [--oidc-audience AUD]... [--oidc-scope SCOPE]...
                         [--api-tokens FILE] [--trusted-proxy CIDR]... [--proxy-user-header H]
                         [--proxy-email-header H] [--proxy-groups-header H] [--grant SUBJECT=WORKSPACES]...
                         [--reader-grant SUBJECT=WORKSPACES]...
                         [--max-upload-bytes N] [--shutdown-grace SECS] [--idle-timeout SECS]
                         [--ready-memory-percent P] [--drain-on-stdin-close]
                         [--no-viewer] [--viewer-copies N] [--max-viewers N]
                         [--max-concurrent-reads N] [--max-views-per-user N] [--max-views N] [--view-idle SECS]
                         [--recovery-dir DIR] [--allow-autosave]
                         [--viewer-control [--viewer-control-file F]] [--local-offset ±HH:MM|MIN|UTC|machine]
                         (MCP over Streamable HTTP, revision 2026-07-28, at /w/<WORKSPACE>/mcp, one workspace
                         per subfolder of DIR, tool paths confined to it; loopback only unless an
                         authentication mode is configured: docs/auth.md; files at /w/<WORKSPACE>/files/,
                         the live viewer at /w/<WORKSPACE>/view/ (private views; readers see saved files),
                         --viewer-control: the UI control channel relayed to editors' app pages,
                         probes /healthz and /readyz, graceful drain on SIGTERM or Ctrl+C:
                         docs/web-deployment.md)
       bicraft-cli serve token create --name NAME [--role ROLE]... [--group GROUP]...
       bicraft-cli serve token hash   (reads a token on stdin, prints its tokens-file entry)
       bicraft-cli controller --config FILE [--listen 127.0.0.1:7979] [--kube-api URL] [--namespace NS]
                         [--admin SUBJECT]... [--interval SECS] [--shutdown-grace SECS] [auth flags of serve]
                         (the Kubernetes workspace controller: reconciles BicraftWorkspace resources, gateway
                         with wake on request and idle scale to zero, admin API: docs/kubernetes.md)
       bicraft-cli vpax PATH OUT.vpax [--overwrite] [--no-model-bim] [--cache-abf CACHE.abf]
       bicraft-cli redact PATH OUT [--data] [--overwrite]
                          (a copy of a PBIX, PBIT or PBIP for bug reports: connection strings, credentials,
                          servers, databases, paths and URLs replaced by stable placeholders; --data: the
                          stored text values too; every written byte checked)
       bicraft-cli visual new DIR NAME [--display-name N] [--guid G] [--author A] [--overwrite]
                          [--role NAME:Grouping|Measure|GroupingOrMeasure]... [--format OBJECT.PROP:bool|color|numeric|text]...
       bicraft-cli visual build DIR [--out FILE.js]
       bicraft-cli visual pack DIR [--out FILE.pbiviz]
                          (custom visual projects: the TypeScript compiled and bundled with swc, no Node;
                          pack writes the .pbiviz that visual.import_custom reads: docs/formats/pbiviz.md)
       bicraft-cli visual export NAME... | --all [--out FILE.pbiviz|DIR]
                          (BIcraft's own visuals as .pbiviz files for Desktop's Import a visual from a file
                          and organizational visuals; default --out: the current folder, each file named
                          GUID.VERSION.pbiviz; several names or --all write into a folder)
       bicraft-cli view PATH [--port N] [--open] [--data-root DIR] [--lang TAG] [--culture NAME]
                         [--as-of YYYY-MM-DD] [--cache-abf CACHE.abf] [--no-open-button] [--read-only]
                         [--ui html|egui] [--control [--control-file F]]
                         (egui: the desktop app's UI in the browser at /app/, docs/webui.md; --control:
                         the UI control channel relayed to that page, its token in an owner-only file)
       bicraft-cli ui [--control FILE | --view | --serve] [--url URL] [--workspace ID] [--view-id V]
                         METHOD [PARAMS-JSON] [--out FILE.png]
                         (drives a running app through its control channel: bicraft --control,
                         bicraft-cli view --control, bicraft-cli serve --viewer-control (credential:
                         BICRAFT_SERVER_TOKEN); `bicraft-cli ui methods` lists them; docs/desktop.md)
       bicraft-cli render PATH [--page NAME|INDEX] [--out FILE.png|DIR] [--scale N]
                           [--font-dir DIR]... [--no-system-fonts] [--no-badges] [--data-root DIR]
                           [--lang TAG] [--culture NAME] [--format-culture NAME] [--text-json] [--lcd-text]
                           [--high-contrast] [--as-of YYYY-MM-DD] [--cache-abf CACHE.abf] [--fail-on-empty]
                           [--key-influencers placeholder|estimate]
       bicraft-cli export-pdf PATH [--out FILE.pdf] [--pages all|current|2-4|NAME,NAME] [--hidden]
                           [--title TEXT] [--scale N] [--font-dir DIR]... [--no-system-fonts]
                           [--data-root DIR] [--lang TAG] [--format-culture NAME] [--as-of DATE]
       --cache-abf: a PBIP's data is read from CACHE.abf (read-only, never written) instead of its
                        own .pbi/cache.abf, e.g. from a throwaway copy of a project
       render/view languages: --lang TAG = generated text (auto titles, (Blank), Total);
                        --format-culture NAME = numbers and dates (default: the model's culture);
                        --culture NAME = the model translation whose captions name fields (one
                        without translations, or the model's own, shows the stored names + a warning)
       bicraft-cli setup [--client ID]... [--all-detected] [--scope user|project]
                          [--dry-run] [--yes] [--json] [--uninstall]   (see `bicraft-cli setup --help`)
       bicraft-cli setup doctor [--client ID]... [--json]
       bicraft-cli licenses                 (licence and third-party notices, plain text)
       bicraft-cli build-info               (JSON: version, commit, whether the browser app is embedded)
       bicraft-cli --version | help

Exit codes: 0 success/valid, 1 invalid or a command failed, 2 usage or I/O error.";

/// Stack of the worker thread. Windows main threads only get 1 MiB, and the DAX parser and
/// binder recurse (bounded) on nested expressions.
const STACK_BYTES: usize = 64 * 1024 * 1024;

const OK: u8 = 0;
const INVALID: u8 = 1;
const USAGE_ERROR: u8 = 2;

/// A failure: exit code plus message for stderr.
type Failure = (u8, String);

fn usage(msg: impl Into<String>) -> Failure {
    (USAGE_ERROR, msg.into())
}

fn main() -> ExitCode {
    let mut args = Vec::new();
    for a in std::env::args_os().skip(1) {
        match a.into_string() {
            Ok(s) => args.push(s),
            Err(a) => {
                eprintln!("bicraft: argument is not valid UTF-8: {}", a.to_string_lossy());
                return ExitCode::from(USAGE_ERROR);
            }
        }
    }
    let worker = std::thread::Builder::new()
        .name("bicraft".into())
        .stack_size(STACK_BYTES)
        .spawn(move || dispatch(&args));
    let code = match worker.map(std::thread::JoinHandle::join) {
        Ok(Ok(code)) => code,
        Ok(Err(_)) => {
            eprintln!("bicraft: internal error");
            USAGE_ERROR
        }
        Err(e) => {
            eprintln!("bicraft: can't start worker thread: {e}");
            USAGE_ERROR
        }
    };
    ExitCode::from(code)
}

/// `bicraft-cli 0.1.0 (commit 0123456789ab)`; the commit comes from `build.rs` and is left out when
/// the build had no git checkout.
fn version_line() -> String {
    match option_env!("BICRAFT_GIT_COMMIT") {
        Some(c) => format!("bicraft-cli {} (commit {c})", env!("CARGO_PKG_VERSION")),
        None => format!("bicraft-cli {}", env!("CARGO_PKG_VERSION")),
    }
}

/// `bicraft-cli build-info`: what this binary was built with. Release checks (`cargo xtask dist`, the
/// container image, `cargo xtask deploy check --binary`) require `webui.embedded`.
fn build_info() -> String {
    let (js, wasm, gzip) = bicraft_view::webui::sizes();
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "commit": option_env!("BICRAFT_GIT_COMMIT"),
        "webui": {
            "embedded": bicraft_view::webui::available(),
            "js_bytes": js,
            "wasm_bytes": wasm,
            "gzip_bytes": gzip,
        },
    })
    .to_string()
}

/// The machine's time zone, read at each evaluation (daylight saving included).
pub(crate) fn machine_clock() -> bicraft_engine::LocalClock {
    bicraft_engine::LocalClock::Zone(std::sync::Arc::new(
        bicraft_locale::calendar::machine_offset_minutes,
    ))
}

fn dispatch(args: &[String]) -> u8 {
    let rest = args.get(1..).unwrap_or(&[]);
    // M's local clock (`DateTime.LocalNow()`): the machine's time zone, as in Desktop; a server
    // keeps UTC, as the Power BI service does, unless `--local-offset` says otherwise
    if !matches!(args.first().map(String::as_str), Some("serve" | "controller")) {
        bicraft_engine::set_local_clock(machine_clock());
    }
    let result = match args.first().map(String::as_str) {
        Some("--version" | "-V" | "version") => {
            outln!("{}", version_line());
            Ok(OK)
        }
        Some("build-info") => no_more(rest).map(|()| {
            outln!("{}", build_info());
            OK
        }),
        Some("licenses") => no_more(rest).map(|()| {
            outln!("{}", bicraft_engine::licenses_text());
            OK
        }),
        Some("help" | "--help" | "-h") => {
            outln!("{USAGE}");
            Ok(OK)
        }
        Some("commands") => commands(rest),
        Some("describe") => describe(rest),
        Some("run") => run(rest),
        Some("new-project") => new_project(rest),
        Some("inspect") => inspect(rest),
        Some("convert") => convert(rest),
        Some("diff") => diff_cmd::diff(rest),
        Some("import") => import_cmd::import(rest),
        Some("validate-dax") => validate_dax(rest),
        Some("validate-model") => validate_model(rest),
        Some("analyze") => analyze(rest),
        Some("usage") => usage_cmd(rest),
        Some("unused") => unused_cmd(rest),
        Some("validate-report") => validate_report(rest),
        Some("test") => test_cmd::test(rest),
        Some("sources") => env_cmd::sources(rest),
        Some("env") => env_cmd::env(rest),
        Some("validate-dax-query") => validate_dax_query(rest),
        Some("validate-m") => validate_m(rest),
        Some("dax-query") => dax_query(rest),
        Some("refresh") => refresh_cmd::refresh(rest),
        Some("mcp") => mcp(rest),
        Some("view") => view_cmd::view(rest),
        Some("ui") => ui_cmd::ui(rest),
        Some("serve") => serve_cmd::serve(rest),
        Some("controller") => controller_cmd::controller(rest),
        Some("render") => render_cmd::render(rest),
        Some("export-pdf") => render_cmd::export_pdf(rest),
        Some("vpax") => vpax(rest),
        Some("redact") => redact(rest),
        Some("visual") => visual(rest),
        Some("setup") => setup::main(rest),
        Some(other) => Err(usage(format!("unknown subcommand `{other}`\n\n{USAGE}"))),
        None => Err(usage(USAGE)),
    };
    match result {
        Ok(code) => code,
        Err((code, msg)) => {
            eprintln!("bicraft: {msg}");
            code
        }
    }
}

fn print_json(v: &Value) {
    outln!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

/// Rejects anything left over after the expected positional arguments.
fn no_more(args: &[String]) -> Result<(), Failure> {
    match args.first() {
        Some(a) => Err(usage(format!("unexpected argument `{a}`"))),
        None => Ok(()),
    }
}

fn commands(args: &[String]) -> Result<u8, Failure> {
    no_more(args.get(1..).unwrap_or(&[]))?;
    let list = bicraft_engine::list_commands(args.first().map(String::as_str));
    let v = serde_json::to_value(list).map_err(|e| (USAGE_ERROR, e.to_string()))?;
    print_json(&v);
    Ok(OK)
}

fn describe(args: &[String]) -> Result<u8, Failure> {
    let id = args.first().ok_or_else(|| usage("describe needs a COMMAND id"))?;
    no_more(args.get(1..).unwrap_or(&[]))?;
    let cmd = bicraft_engine::find_command(id)
        .ok_or_else(|| usage(format!("unknown command `{id}` (see `bicraft-cli commands`)")))?;
    let v = serde_json::to_value(cmd.info()).map_err(|e| (USAGE_ERROR, e.to_string()))?;
    print_json(&v);
    Ok(OK)
}

/// The value following option `name`.
fn value<'a>(it: &mut impl Iterator<Item = &'a String>, name: &str) -> Result<&'a str, Failure> {
    it.next()
        .map(String::as_str)
        .ok_or_else(|| usage(format!("{name} needs a value")))
}

/// `ID` or `ID={JSON}` → `(id, params)`.
fn parse_cmd(spec: &str) -> Result<(String, Value), Failure> {
    let (id, params) = match spec.split_once('=') {
        Some((id, json)) => {
            let params: Value = serde_json::from_str(json)
                .map_err(|e| usage(format!("--cmd `{id}`: parameters are not valid JSON: {e}")))?;
            if !params.is_object() {
                return Err(usage(format!("--cmd `{id}`: parameters must be a JSON object")));
            }
            (id.trim(), params)
        }
        None => (spec.trim(), json!({})),
    };
    if id.is_empty() {
        return Err(usage("--cmd needs a command id"));
    }
    Ok((id.to_string(), params))
}

fn open(session: &mut Session, path: &str) -> Result<Value, Failure> {
    open_with(session, path, None)
}

/// `file.open` with an explicit PBIP data cache (`--cache-abf`).
fn open_with(session: &mut Session, path: &str, cache_abf: Option<&str>) -> Result<Value, Failure> {
    let mut params = json!({"path": path});
    if let Some(c) = cache_abf {
        params["cache_abf"] = json!(c);
    }
    session
        .execute("file.open", &params)
        .map_err(|e| (USAGE_ERROR, e))
}

fn run(args: &[String]) -> Result<u8, Failure> {
    let mut open_path = None;
    let mut save = false;
    let mut atomic = true;
    let mut steps = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--open" => open_path = Some(value(&mut it, "--open")?),
            "--cmd" => steps.push(parse_cmd(value(&mut it, "--cmd")?)?),
            "--file" | "--cmd-file" => steps.extend(read_steps(value(&mut it, a)?)?),
            "--save" => save = true,
            "--no-atomic" => atomic = false,
            other => return Err(usage(format!("run: unexpected argument `{other}`"))),
        }
    }
    if steps.is_empty() && open_path.is_none() {
        return Err(usage("run needs at least one --cmd or --file (or --open)"));
    }
    if save {
        steps.push(("file.save".to_string(), json!({})));
    }
    let mut session = Session::new();
    if let Some(path) = open_path {
        open(&mut session, path)?;
    }
    // Atomic: a failed step rolls the session back (files written by earlier save steps stay).
    let outcome = if atomic {
        session.batch_atomic(&steps)
    } else {
        session.batch(&steps)
    };
    match outcome {
        Ok(results) => {
            print_json(&json!({"results": results}));
            Ok(OK)
        }
        Err((i, e, results)) => {
            let id = steps.get(i).map_or("?", |s| s.0.as_str());
            print_json(&json!({
                "results": results,
                "failed_index": i,
                "failed_command": id,
                "rolled_back": atomic,
                "error": e,
            }));
            Err((INVALID, format!("command {i} (`{id}`) failed: {e}")))
        }
    }
}

/// Most bytes read from a `run --file` commands file.
const MAX_COMMANDS_FILE: u64 = 256 << 20;

/// The steps of a commands file: a JSON array of `{command, params?}` (or `{commands: [...]}`,
/// the MCP `batch` tool's arguments); `-` reads stdin. Keeps long command lines out of argv
/// (Windows caps a command line at 32,767 characters).
fn read_steps(path: &str) -> Result<Vec<(String, Value)>, Failure> {
    use std::io::Read as _;
    let mut text = String::new();
    let read = if path == "-" {
        std::io::stdin().take(MAX_COMMANDS_FILE).read_to_string(&mut text)
    } else {
        std::fs::File::open(path).and_then(|f| f.take(MAX_COMMANDS_FILE).read_to_string(&mut text))
    };
    read.map_err(|e| (USAGE_ERROR, format!("{path}: {e}")))?;
    let doc: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| usage(format!("{path}: not valid JSON: {e}")))?;
    let shape = || usage(format!("{path}: expected a JSON array of {{command, params}}"));
    let list = match &doc {
        Value::Array(a) => a,
        Value::Object(o) => match o.get("commands") {
            Some(Value::Array(a)) => a,
            _ => return Err(shape()),
        },
        _ => return Err(shape()),
    };
    let mut steps = Vec::with_capacity(list.len());
    for (i, c) in list.iter().enumerate() {
        let Some(o) = c.as_object() else {
            return Err(usage(format!(
                "{path}: entry {i} is not an object {{command, params}}"
            )));
        };
        let id = o
            .get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| usage(format!("{path}: entry {i} needs a string `command`")))?;
        let params = match o.get("params") {
            None | Some(Value::Null) => json!({}),
            Some(p @ Value::Object(_)) => p.clone(),
            Some(_) => {
                return Err(usage(format!(
                    "{path}: entry {i} (`{id}`): `params` must be an object"
                )));
            }
        };
        if let Some(extra) = o.keys().find(|k| *k != "command" && *k != "params") {
            return Err(usage(format!(
                "{path}: entry {i} (`{id}`): unknown key `{extra}`"
            )));
        }
        steps.push((id.to_string(), params));
    }
    Ok(steps)
}

/// `dax-query PATH (QUERY | --file PATH)` → `file.open` + `dax.evaluate`.
fn dax_query(args: &[String]) -> Result<u8, Failure> {
    let mut positional: Vec<String> = Vec::new();
    let mut from_file: Option<String> = None;
    let mut params = serde_json::Map::new();
    let mut cache_abf = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--file" => {
                let f = value(&mut it, "--file")?;
                let text = std::fs::read_to_string(f).map_err(|e| (USAGE_ERROR, format!("{f}: {e}")))?;
                if from_file
                    .replace(text.trim_start_matches('\u{feff}').to_string())
                    .is_some()
                {
                    return Err(usage("dax-query takes one --file"));
                }
            }
            "--max-rows" | "--timeout-ms" => {
                let v = value(&mut it, a)?;
                let n: u64 = v
                    .parse()
                    .map_err(|_| usage(format!("{a} needs a positive integer, not `{v}`")))?;
                let key = if a == "--max-rows" {
                    "max_rows"
                } else {
                    "timeout_ms"
                };
                params.insert(key.into(), json!(n));
            }
            "--as-of" => {
                params.insert("as_of".into(), json!(value(&mut it, "--as-of")?));
            }
            "--no-cache-abf" => {
                params.insert("allow_cache_abf".into(), json!(false));
            }
            "--cache-abf" => cache_abf = Some(value(&mut it, "--cache-abf")?),
            // `--` ends options, so a query starting with `-` can be passed.
            "--" => positional.extend(it.by_ref().cloned()),
            other if other.starts_with("--") => return Err(usage(format!("unknown option `{other}`"))),
            other => positional.push(other.to_string()),
        }
    }
    let (path, query) = match (positional.as_slice(), from_file) {
        ([path], Some(q)) => (path.clone(), q),
        ([path, q], None) => (path.clone(), q.clone()),
        ([], _) => return Err(usage("dax-query needs a PATH and a QUERY (or --file QUERY.dax)")),
        ([_], None) => return Err(usage("dax-query needs a QUERY or --file QUERY.dax")),
        _ => {
            return Err(usage(
                "dax-query takes one PATH and one QUERY (quote it) or --file",
            ));
        }
    };
    params.insert("query".into(), json!(query));
    let mut session = Session::new();
    open_with(&mut session, &path, cache_abf)?;
    let v = session
        .execute("dax.evaluate", &Value::Object(params))
        .map_err(|e| (INVALID, e))?;
    print_json(&v);
    Ok(OK)
}

fn new_project(args: &[String]) -> Result<u8, Failure> {
    let mut positional = Vec::new();
    let mut format = None;
    let mut open_path = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--format" => format = Some(value(&mut it, "--format")?),
            "--open" => open_path = Some(value(&mut it, "--open")?),
            other if other.starts_with("--") => return Err(usage(format!("unknown option `{other}`"))),
            other => positional.push(other),
        }
    }
    let [dir, name] = positional.as_slice() else {
        return Err(usage("new-project needs DIR and NAME"));
    };
    let mut session = Session::new();
    if let Some(path) = open_path {
        open(&mut session, path)?;
    }
    let mut params = json!({"dir": dir, "name": name});
    if let (Some(f), Some(o)) = (format, params.as_object_mut()) {
        o.insert("format".into(), json!(f));
    }
    let v = session
        .execute("project.new", &params)
        .map_err(|e| (USAGE_ERROR, e))?;
    print_json(&v);
    Ok(OK)
}

/// `convert IN OUT [options]` → `file.convert`.
fn convert(args: &[String]) -> Result<u8, Failure> {
    let mut positional = Vec::new();
    let mut params = serde_json::Map::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--format" => {
                params.insert("format".into(), json!(value(&mut it, "--format")?));
            }
            "--report-format" => {
                params.insert("report_format".into(), json!(value(&mut it, "--report-format")?));
            }
            "--no-cache" => {
                params.insert("cache".into(), json!(false));
            }
            "--empty-tables" => {
                params.insert("empty_tables".into(), json!(true));
            }
            "--data-root" => {
                params.insert("data_root".into(), json!(value(&mut it, "--data-root")?));
            }
            "--overwrite" => {
                params.insert("overwrite".into(), json!(true));
            }
            "--strict" => {
                params.insert("strict".into(), json!(true));
            }
            other if other.starts_with("--") => return Err(usage(format!("unknown option `{other}`"))),
            other => positional.push(other),
        }
    }
    let [from, to] = positional.as_slice() else {
        return Err(usage("convert needs IN and OUT"));
    };
    params.insert("from".into(), json!(from));
    params.insert("to".into(), json!(to));
    let mut session = Session::new();
    let v = session
        .execute("file.convert", &Value::Object(params))
        .map_err(|e| (INVALID, e))?;
    print_json(&v);
    Ok(OK)
}

fn inspect(args: &[String]) -> Result<u8, Failure> {
    let path = args.first().ok_or_else(|| usage("inspect needs a PATH"))?;
    no_more(args.get(1..).unwrap_or(&[]))?;
    let mut session = Session::new();
    open(&mut session, path)?;
    let v = session
        .execute("file.inspect", &json!({}))
        .map_err(|e| (USAGE_ERROR, e))?;
    print_json(&v);
    Ok(OK)
}

fn validate_dax(args: &[String]) -> Result<u8, Failure> {
    let mut expression = None;
    let mut model = None;
    let mut table = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--model" => model = Some(value(&mut it, "--model")?),
            "--table" => table = Some(value(&mut it, "--table")?),
            // `--` ends options, so expressions starting with `-` can be passed.
            "--" => {
                let e = it
                    .next()
                    .ok_or_else(|| usage("validate-dax needs an EXPRESSION"))?;
                if expression.replace(e.as_str()).is_some() {
                    return Err(usage("validate-dax takes one EXPRESSION (quote it)"));
                }
            }
            other if other.starts_with("--") => return Err(usage(format!("unknown option `{other}`"))),
            other => {
                if expression.replace(other).is_some() {
                    return Err(usage("validate-dax takes one EXPRESSION (quote it)"));
                }
            }
        }
    }
    let expression = expression.ok_or_else(|| usage("validate-dax needs an EXPRESSION"))?;
    let mut session = Session::new();
    if let Some(path) = model {
        open(&mut session, path)?;
        if session.model.is_none() {
            let why = session.model_error.as_deref().unwrap_or("no model found");
            return Err((USAGE_ERROR, format!("{path}: cannot read the model: {why}")));
        }
    }
    let mut params = json!({"expression": expression});
    if let (Some(t), Some(o)) = (table, params.as_object_mut()) {
        o.insert("table".into(), json!(t));
    }
    let v = session
        .execute("dax.validate", &params)
        .map_err(|e| (USAGE_ERROR, e))?;
    print_json(&v);
    Ok(if v.get("valid") == Some(&Value::Bool(true)) {
        OK
    } else {
        INVALID
    })
}

/// `bicraft-cli validate-m`: m.validate (syntax and undefined names, nothing evaluated), against
/// the queries and parameters of `--model` when given. Exit 1 when invalid.
fn validate_m(args: &[String]) -> Result<u8, Failure> {
    let mut expression: Option<String> = None;
    let mut model = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let text = match a.as_str() {
            "--model" => {
                model = Some(value(&mut it, "--model")?);
                continue;
            }
            "--file" => {
                let path = value(&mut it, "--file")?;
                std::fs::read_to_string(path).map_err(|e| (USAGE_ERROR, format!("{path}: {e}")))?
            }
            "--" => it
                .next()
                .ok_or_else(|| usage("validate-m needs an EXPRESSION"))?
                .clone(),
            other if other.starts_with("--") => return Err(usage(format!("unknown option `{other}`"))),
            other => other.to_string(),
        };
        if expression.replace(text).is_some() {
            return Err(usage("validate-m takes one EXPRESSION (quote it) or --file"));
        }
    }
    let expression = expression.ok_or_else(|| usage("validate-m needs an EXPRESSION or --file"))?;
    let mut session = Session::new();
    if let Some(path) = model {
        open(&mut session, path)?;
        if session.model.is_none() {
            let why = session.model_error.as_deref().unwrap_or("no model found");
            return Err((USAGE_ERROR, format!("{path}: cannot read the model: {why}")));
        }
    }
    let v = session
        .execute("m.validate", &json!({"expression": expression}))
        .map_err(|e| (USAGE_ERROR, e))?;
    print_json(&v);
    Ok(if v.get("valid") == Some(&Value::Bool(true)) {
        OK
    } else {
        INVALID
    })
}

/// `bicraft-cli redact PATH OUT [--data] [--overwrite]`: file.redact.
fn redact(args: &[String]) -> Result<u8, Failure> {
    let mut positional = Vec::new();
    let (mut data, mut overwrite) = (false, false);
    for a in args {
        match a.as_str() {
            "--data" => data = true,
            "--overwrite" => overwrite = true,
            other if other.starts_with("--") => return Err(usage(format!("unknown option `{other}`"))),
            other => positional.push(other),
        }
    }
    let [path, out] = positional.as_slice() else {
        return Err(usage("redact needs PATH and OUT"));
    };
    let mut session = Session::new();
    let v = session
        .execute(
            "file.redact",
            &json!({"path": path, "out": out, "data": data, "overwrite": overwrite}),
        )
        .map_err(|e| (INVALID, e))?;
    print_json(&v);
    Ok(OK)
}

/// `bicraft-cli vpax PATH OUT.vpax`: model.export_vpax on a file.
fn vpax(args: &[String]) -> Result<u8, Failure> {
    let mut positional = Vec::new();
    let mut overwrite = false;
    let mut model_bim = true;
    let mut cache_abf = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--overwrite" => overwrite = true,
            "--no-model-bim" => model_bim = false,
            "--cache-abf" => cache_abf = Some(value(&mut it, "--cache-abf")?),
            other if other.starts_with("--") => return Err(usage(format!("unknown option `{other}`"))),
            other => positional.push(other),
        }
    }
    let [path, out] = positional.as_slice() else {
        return Err(usage("vpax needs PATH and OUT.vpax"));
    };
    let mut session = Session::new();
    open_with(&mut session, path, cache_abf)?;
    let v = session
        .execute(
            "model.export_vpax",
            &json!({"path": out, "overwrite": overwrite, "model_bim": model_bim}),
        )
        .map_err(|e| (INVALID, e))?;
    print_json(&v);
    Ok(OK)
}

/// `bicraft-cli visual new DIR NAME [--display-name N] [--guid G] [--role R]... [--format F]...
/// [--overwrite]`, `bicraft-cli visual build DIR [--out F.js]`, `bicraft-cli visual pack DIR [--out
/// F.pbiviz]`: visual.scaffold, visual.build, visual.pack (custom visual projects, no Node);
/// `bicraft-cli visual export NAME... | --all [--out F.pbiviz|DIR]`: visual.export_bicraft_visual.
fn visual(args: &[String]) -> Result<u8, Failure> {
    let (sub, rest) = args
        .split_first()
        .ok_or_else(|| usage("visual needs new, build, pack or export"))?;
    let mut positional = Vec::new();
    let mut params = serde_json::Map::new();
    let (mut roles, mut formats) = (Vec::new(), Vec::new());
    let mut all = false;
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match (sub.as_str(), a.as_str()) {
            ("build" | "pack" | "export", "--out") => {
                params.insert("out".into(), json!(value(&mut it, "--out")?));
            }
            ("export", "--all") => all = true,
            ("new", "--display-name") => {
                params.insert("display_name".into(), json!(value(&mut it, a)?));
            }
            ("new", "--guid") => {
                params.insert("guid".into(), json!(value(&mut it, a)?));
            }
            ("new", "--author") => {
                params.insert("author".into(), json!(value(&mut it, a)?));
            }
            ("new", "--role") => roles.push(json!(value(&mut it, a)?)),
            ("new", "--format") => formats.push(json!(value(&mut it, a)?)),
            ("new", "--overwrite") => {
                params.insert("overwrite".into(), json!(true));
            }
            (_, other) if other.starts_with("--") => {
                return Err(usage(format!("unknown option `{other}`")));
            }
            (_, other) => positional.push(other),
        }
    }
    let id = match (sub.as_str(), positional.as_slice()) {
        ("new", [dir, name]) => {
            params.insert("folder".into(), json!(dir));
            params.insert("name".into(), json!(name));
            if !roles.is_empty() {
                params.insert("data_roles".into(), json!(roles));
            }
            if !formats.is_empty() {
                params.insert("format_options".into(), json!(formats));
            }
            "visual.scaffold"
        }
        ("build" | "pack", [dir]) => {
            params.insert("folder".into(), json!(dir));
            if sub == "build" {
                "visual.build"
            } else {
                "visual.pack"
            }
        }
        ("export", names) => {
            let name = match (all, names) {
                (true, []) => json!("all"),
                (false, [one]) => json!(one),
                (false, []) => {
                    return Err(usage(
                        "visual export needs a visual's NAME (bicraftSpec, bicraftHistogram, …) or --all",
                    ));
                }
                (false, several) => json!(several),
                (true, _) => return Err(usage("visual export takes NAMEs or --all, not both")),
            };
            params.insert("name".into(), name);
            params.entry("out").or_insert_with(|| json!("."));
            "visual.export_bicraft_visual"
        }
        ("new", _) => return Err(usage("visual new needs DIR and NAME")),
        ("build" | "pack", _) => return Err(usage(format!("visual {sub} needs the project's DIR"))),
        (other, _) => {
            return Err(usage(format!(
                "unknown visual command `{other}` (new, build, pack, export)"
            )));
        }
    };
    let v = Session::new()
        .execute(id, &Value::Object(params))
        .map_err(|e| (INVALID, e))?;
    print_json(&v);
    Ok(OK)
}

/// Opens `path` for `usage` / `unused`; a model is required.
fn open_model(session: &mut Session, path: &str) -> Result<(), Failure> {
    open(session, path)?;
    if session.model.is_none() {
        let why = session.model_error.as_deref().unwrap_or("no model found");
        return Err((USAGE_ERROR, format!("{path}: cannot read the model: {why}")));
    }
    Ok(())
}

/// `bicraft-cli usage PATH [OBJECT] [--kind K] [--report P]... [--no-graph]`: model.usage.
fn usage_cmd(args: &[String]) -> Result<u8, Failure> {
    let path = args.first().ok_or_else(|| usage("usage needs a PATH"))?;
    let mut params = serde_json::Map::new();
    let mut reports = Vec::new();
    let mut rest = args.get(1..).unwrap_or(&[]).iter();
    while let Some(a) = rest.next() {
        let mut value = |flag: &str| {
            rest.next()
                .cloned()
                .ok_or_else(|| usage(format!("{flag} needs a value")))
        };
        match a.as_str() {
            "--report" => reports.push(json!(value("--report")?)),
            "--kind" => {
                params.insert("kind".into(), json!(value("--kind")?));
            }
            "--no-graph" => {
                params.insert("graph".into(), json!(false));
            }
            other if other.starts_with("--") => {
                return Err(usage(format!("usage: unknown argument `{other}`")));
            }
            object if !params.contains_key("object") => {
                params.insert("object".into(), json!(object));
            }
            other => return Err(usage(format!("usage: unexpected argument `{other}`"))),
        }
    }
    if !reports.is_empty() {
        params.insert("reports".into(), Value::Array(reports));
    }
    let mut session = Session::new();
    open_model(&mut session, path)?;
    let v = session
        .execute("model.usage", &Value::Object(params))
        .map_err(|e| (USAGE_ERROR, e))?;
    print_json(&v);
    Ok(OK)
}

/// `bicraft-cli unused PATH [--report P]... [--allow PATTERN]... [--remove [--object O]... [--dry-run]
/// [--force] [--save]]`: model.unused (exit 1 when something is unused), or model.remove_unused.
fn unused_cmd(args: &[String]) -> Result<u8, Failure> {
    let path = args.first().ok_or_else(|| usage("unused needs a PATH"))?;
    let (mut reports, mut allow, mut objects) = (Vec::new(), Vec::new(), Vec::new());
    let (mut remove, mut dry_run, mut force, mut save) = (false, false, false, false);
    let mut keep_metadata = true;
    let mut rest = args.get(1..).unwrap_or(&[]).iter();
    while let Some(a) = rest.next() {
        let mut value = |flag: &str| {
            rest.next()
                .cloned()
                .ok_or_else(|| usage(format!("{flag} needs a value")))
        };
        match a.as_str() {
            "--report" => reports.push(json!(value("--report")?)),
            "--allow" => allow.push(json!(value("--allow")?)),
            "--object" => objects.push(json!(value("--object")?)),
            "--remove" => remove = true,
            "--ignore-metadata" => keep_metadata = false,
            "--dry-run" => dry_run = true,
            "--force" => force = true,
            "--save" => save = true,
            other => return Err(usage(format!("unused: unknown argument `{other}`"))),
        }
    }
    if !remove && (dry_run || force || save || !objects.is_empty()) {
        return Err(usage(
            "unused: --object, --dry-run, --force and --save need --remove",
        ));
    }
    if dry_run && save {
        return Err(usage("unused: --dry-run and --save exclude each other"));
    }
    let mut params = serde_json::Map::new();
    if !reports.is_empty() {
        params.insert("reports".into(), Value::Array(reports));
    }
    if !allow.is_empty() {
        params.insert("allow".into(), Value::Array(allow));
    }
    if !keep_metadata {
        params.insert("keep_metadata".into(), json!(false));
    }
    let mut session = Session::new();
    open_model(&mut session, path)?;
    if !remove {
        let v = session
            .execute("model.unused", &Value::Object(params))
            .map_err(|e| (USAGE_ERROR, e))?;
        print_json(&v);
        let c = &v["counts"];
        let n = ["tables", "columns", "measures", "expressions"]
            .iter()
            .map(|k| c[*k].as_u64().unwrap_or(0))
            .sum::<u64>();
        return Ok(if n == 0 { OK } else { INVALID });
    }
    if !objects.is_empty() {
        params.insert("objects".into(), Value::Array(objects));
    }
    params.insert("dry_run".into(), json!(dry_run));
    params.insert("force".into(), json!(force));
    let mut v = session
        .execute("model.remove_unused", &Value::Object(params))
        .map_err(|e| (INVALID, e))?;
    if save {
        let saved = session
            .execute("file.save", &json!({}))
            .map_err(|e| (INVALID, format!("removed, but file.save failed: {e}")))?;
        v["saved"] = saved;
    }
    print_json(&v);
    Ok(OK)
}

/// `bicraft-cli analyze`: exit 1 when a rule of severity error (3+) fires.
fn analyze(args: &[String]) -> Result<u8, Failure> {
    let path = args.first().ok_or_else(|| usage("analyze needs a PATH"))?;
    let mut rules = Vec::new();
    let mut ids = Vec::new();
    let mut params = serde_json::Map::new();
    let mut rest = args.get(1..).unwrap_or(&[]).iter();
    while let Some(a) = rest.next() {
        let mut value = |flag: &str| {
            rest.next()
                .cloned()
                .ok_or_else(|| usage(format!("{flag} needs a value")))
        };
        match a.as_str() {
            "--rules" => rules.push(json!(value("--rules")?)),
            "--rule-id" => ids.push(json!(value("--rule-id")?)),
            "--no-default-rules" => {
                params.insert("default_rules".into(), json!(false));
            }
            "--no-report-rules" => {
                params.insert("report_rules".into(), json!(false));
            }
            "--severity-min" => {
                let v = value("--severity-min")?;
                let v = v.parse::<u64>().map_or_else(|_| json!(v), |n| json!(n));
                params.insert("severity_min".into(), v);
            }
            "--max-findings" => {
                let v = value("--max-findings")?;
                let n = v
                    .parse::<u64>()
                    .map_err(|_| usage(format!("--max-findings: `{v}` is not a number")))?;
                params.insert("max_findings".into(), json!(n));
            }
            other => return Err(usage(format!("analyze: unknown argument `{other}`"))),
        }
    }
    if !rules.is_empty() {
        params.insert("rules".into(), Value::Array(rules));
    }
    if !ids.is_empty() {
        params.insert("rule_ids".into(), Value::Array(ids));
    }
    let mut session = Session::new();
    open(&mut session, path)?;
    if session.model.is_none() {
        let why = session.model_error.as_deref().unwrap_or("no model found");
        return Err((USAGE_ERROR, format!("{path}: cannot read the model: {why}")));
    }
    let v = session
        .execute("model.analyze", &Value::Object(params))
        .map_err(|e| (USAGE_ERROR, e))?;
    print_json(&v);
    let errors = v["summary"]["errors"].as_u64().unwrap_or(0);
    Ok(if errors == 0 { OK } else { INVALID })
}

fn validate_model(args: &[String]) -> Result<u8, Failure> {
    let path = args.first().ok_or_else(|| usage("validate-model needs a PATH"))?;
    no_more(args.get(1..).unwrap_or(&[]))?;
    let mut session = Session::new();
    open(&mut session, path)?;
    let v = session
        .execute("model.validate", &json!({}))
        .map_err(|e| (USAGE_ERROR, e))?;
    print_json(&v);
    Ok(if v.get("valid") == Some(&Value::Bool(true)) {
        OK
    } else {
        INVALID
    })
}

fn validate_report(args: &[String]) -> Result<u8, Failure> {
    let path = args
        .first()
        .ok_or_else(|| usage("validate-report needs a PATH"))?;
    no_more(args.get(1..).unwrap_or(&[]))?;
    let mut session = Session::new();
    open(&mut session, path)?;
    if session.report.is_none() {
        let why = session.report_error.as_deref().unwrap_or("no report found");
        return Err((USAGE_ERROR, format!("{path}: cannot read the report: {why}")));
    }
    let v = session
        .execute("report.validate", &json!({}))
        .map_err(|e| (USAGE_ERROR, e))?;
    print_json(&v);
    Ok(if v.get("valid") == Some(&Value::Bool(true)) {
        OK
    } else {
        INVALID
    })
}

fn mcp(args: &[String]) -> Result<u8, Failure> {
    if !args.is_empty() {
        return view_cmd::mcp(args);
    }
    eprintln!("bicraft {}: MCP server on stdio", env!("CARGO_PKG_VERSION"));
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    bicraft_mcp::serve(stdin.lock(), stdout.lock()).map_err(|e| (USAGE_ERROR, format!("mcp: {e}")))?;
    Ok(OK)
}

fn validate_dax_query(args: &[String]) -> Result<u8, Failure> {
    let mut query: Option<String> = None;
    let mut model = None;
    let mut it = args.iter();
    let mut set = |q: String| -> Result<(), Failure> {
        if query.replace(q).is_some() {
            return Err(usage("validate-dax-query takes one QUERY (quote it) or --file"));
        }
        Ok(())
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "--model" => model = Some(value(&mut it, "--model")?),
            "--file" => {
                let path = value(&mut it, "--file")?;
                let text =
                    std::fs::read_to_string(path).map_err(|e| (USAGE_ERROR, format!("{path}: {e}")))?;
                set(text)?;
            }
            "--" => {
                let q = it
                    .next()
                    .ok_or_else(|| usage("validate-dax-query needs a QUERY"))?;
                set(q.clone())?;
            }
            other if other.starts_with("--") => return Err(usage(format!("unknown option `{other}`"))),
            other => set(other.to_string())?,
        }
    }
    let query = query.ok_or_else(|| usage("validate-dax-query needs a QUERY or --file PATH"))?;
    let mut session = Session::new();
    if let Some(path) = model {
        open(&mut session, path)?;
        if session.model.is_none() {
            let why = session.model_error.as_deref().unwrap_or("no model found");
            return Err((USAGE_ERROR, format!("{path}: cannot read the model: {why}")));
        }
    }
    let v = session
        .execute("dax.validate_query", &json!({"query": query}))
        .map_err(|e| (USAGE_ERROR, e))?;
    print_json(&v);
    Ok(if v.get("valid") == Some(&Value::Bool(true)) {
        OK
    } else {
        INVALID
    })
}
