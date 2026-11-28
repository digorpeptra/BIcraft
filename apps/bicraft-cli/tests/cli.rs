//! End-to-end tests of the `bicraft-cli` binary.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;
use tempdir::TempDir;

fn bicraft() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bicraft-cli"))
}

fn exit_code(args: &[&str]) -> (i32, Value) {
    let out = bicraft().args(args).output().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    let json = serde_json::from_str(&stdout).unwrap_or(Value::Null);
    (out.status.code().unwrap(), json)
}

fn temp_dir(name: &str) -> TempDir {
    TempDir::new(&format!("cli-{name}"))
}

#[test]
fn mcp_handshake_and_validate_dax_over_stdio() {
    let mut child = bicraft()
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut send = |v: Value| {
        writeln!(stdin, "{v}").unwrap();
        stdin.flush().unwrap();
    };
    let mut recv = || {
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        serde_json::from_str::<Value>(&line).unwrap()
    };

    send(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}}));
    let init = recv();
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "bicraft");

    send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": {"name": "validate_dax", "arguments": {"expression": "SUM(1 +"}}}));
    // The notification got no reply: the next line answers request 2.
    let r = recv();
    assert_eq!(r["id"], 2);
    assert_eq!(r["result"]["isError"], false);
    assert_eq!(r["result"]["structuredContent"]["valid"], false);

    send(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": {"name": "validate_dax", "arguments": {"expression": "1 + 2"}}}));
    let r = recv();
    assert_eq!(r["id"], 3);
    assert_eq!(r["result"]["structuredContent"]["valid"], true);

    drop(send);
    drop(stdin);
    let status = child.wait().unwrap();
    assert!(status.success());
}

/// `bicraft-cli visual new / build / pack`: a project scaffolded, compiled with swc and packaged.
#[test]
fn visual_new_build_and_pack() {
    let dir = temp_dir("visual");
    let proj = dir.join("v");
    let p = proj.to_str().unwrap();
    let (code, v) = exit_code(&[
        "visual",
        "new",
        p,
        "cliVisual",
        "--guid",
        "cliVisual1",
        "--role",
        "cat:Grouping",
        "--role",
        "val:Measure",
        "--format",
        "dataPoint.fill:color",
    ]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data_roles"].as_array().map(Vec::len), Some(2));
    let (code, v) = exit_code(&["visual", "build", p]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["entry"], "src/visual.ts");
    let out = dir.join("v.pbiviz");
    let (code, v) = exit_code(&["visual", "pack", p, "--out", out.to_str().unwrap()]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["guid"], "cliVisual1");
    assert!(out.exists());
    // a malformed project: exit 1; usage errors: exit 2
    std::fs::write(proj.join("src/visual.ts"), "export class Visual {").unwrap();
    assert_eq!(exit_code(&["visual", "build", p]).0, 1);
    assert_eq!(exit_code(&["visual"]).0, 2);
    assert_eq!(exit_code(&["visual", "pack"]).0, 2);
    assert_eq!(exit_code(&["visual", "new", p]).0, 2);
    assert_eq!(exit_code(&["visual", "zap", p]).0, 2);
    assert_eq!(exit_code(&["visual", "build", p, "--role", "x"]).0, 2);
}

/// `bicraft-cli visual export`: one of BIcraft's visuals to a named file, all of them into a folder.
#[test]
fn visual_export_writes_bicraft_visuals() {
    let dir = temp_dir("visual-export");
    let one = dir.join("hist.pbiviz");
    let (code, v) = exit_code(&[
        "visual",
        "export",
        "bicraftHistogram",
        "--out",
        one.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["count"], 1);
    assert!(std::fs::read(&one).unwrap().starts_with(b"PK"));
    let all = dir.join("all");
    let (code, v) = exit_code(&["visual", "export", "--all", "--out", all.to_str().unwrap()]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["count"], 14);
    assert_eq!(std::fs::read_dir(&all).unwrap().count(), 14);
    // an unknown visual: exit 1; usage errors: exit 2
    assert_eq!(
        exit_code(&["visual", "export", "nope", "--out", all.to_str().unwrap()]).0,
        1
    );
    assert_eq!(exit_code(&["visual", "export"]).0, 2);
    assert_eq!(exit_code(&["visual", "export", "--all", "bicraftVenn"]).0, 2);
    assert_eq!(exit_code(&["visual", "pack", "--all"]).0, 2);
}

#[test]
fn validate_dax_exit_codes() {
    let (code, v) = exit_code(&["validate-dax", "1 + 2"]);
    assert_eq!(code, 0);
    assert_eq!(v["valid"], true);

    let (code, v) = exit_code(&["validate-dax", "SUM(1 +"]);
    assert_eq!(code, 1);
    assert_eq!(v["valid"], false);

    assert_eq!(exit_code(&["validate-dax"]).0, 2);
    assert_eq!(exit_code(&["validate-dax", "1", "2"]).0, 2);
    assert_eq!(exit_code(&["validate-dax", "1", "--table"]).0, 2);
    assert_eq!(
        exit_code(&["validate-dax", "1", "--model", "does/not/exist.bim"]).0,
        2
    );
    assert_eq!(exit_code(&["no-such-subcommand"]).0, 2);
}

#[test]
fn validate_against_a_model_file() {
    let dir = temp_dir("model");
    let bim = dir.join("model.bim");
    std::fs::write(
        &bim,
        r#"{"name":"M","model":{"tables":[{"name":"Sales",
            "columns":[{"name":"Amount","dataType":"double"},
                       {"name":"Bad","dataType":"double","type":"calculated","expression":"Sales[Nope]"}],
            "measures":[{"name":"Total","expression":"SUM(Sales[Amount])"}]}]}}"#,
    )
    .unwrap();
    let bim = bim.to_str().unwrap();

    let (code, v) = exit_code(&["validate-dax", "SUM(Sales[Amount])", "--model", bim]);
    assert_eq!(code, 0);
    assert_eq!(v["checked_against_model"], true);
    let (code, _) = exit_code(&[
        "validate-dax",
        "--model",
        bim,
        "SUM(Sales[Nope])",
        "--table",
        "Sales",
    ]);
    assert_eq!(code, 1);

    let (code, v) = exit_code(&["validate-model", bim]);
    assert_eq!(code, 1);
    assert_eq!(v["invalid_count"], 1);

    let (code, v) = exit_code(&["inspect", bim]);
    assert_eq!(code, 0);
    assert_eq!(v["model"]["tables"], 1);

    let (code, v) = exit_code(&[
        "run",
        "--open",
        bim,
        "--cmd",
        "model.summary",
        "--cmd",
        r#"dax.validate={"expression":"[Total]","table":"Sales"}"#,
    ]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["results"].as_array().unwrap().len(), 2);
    assert_eq!(v["results"][1]["valid"], true);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_reports_partial_results() {
    let (code, v) = exit_code(&[
        "run",
        "--cmd",
        r#"dax.validate={"expression":"1"}"#,
        "--cmd",
        "nope",
    ]);
    assert_eq!(code, 1);
    assert_eq!(v["failed_index"], 1);
    assert_eq!(v["results"].as_array().unwrap().len(), 1);
    assert_eq!(v["failed_command"], "nope");
    assert_eq!(v["rolled_back"], true);
    let (code, v) = exit_code(&[
        "run",
        "--no-atomic",
        "--cmd",
        r#"dax.validate={"expression":"1"}"#,
        "--cmd",
        "nope",
    ]);
    assert_eq!(code, 1);
    assert_eq!(v["rolled_back"], false);
    assert_eq!(exit_code(&["run", "--cmd", "dax.validate=[1]"]).0, 2);
    assert_eq!(exit_code(&["run"]).0, 2);
}

#[test]
fn commands_describe_version() {
    let (code, v) = exit_code(&["commands", "dax"]);
    assert_eq!(code, 0);
    assert_eq!(v[0]["id"], "dax.validate");
    let (code, v) = exit_code(&["describe", "file.open"]);
    assert_eq!(code, 0);
    assert_eq!(v["id"], "file.open");
    assert_eq!(exit_code(&["describe", "nope"]).0, 2);

    let out = bicraft().arg("--version").output().unwrap();
    assert!(out.status.success());
    let line = String::from_utf8(out.stdout).unwrap();
    let line = line.trim_end();
    let base = format!("bicraft-cli {}", env!("CARGO_PKG_VERSION"));
    // `bicraft-cli X.Y.Z`, or `bicraft-cli X.Y.Z (commit <12 hex>)` when built from a git checkout.
    match line.strip_prefix(&base) {
        Some("") => {}
        Some(rest) => {
            let hash = rest
                .strip_prefix(" (commit ")
                .and_then(|r| r.strip_suffix(')'))
                .unwrap_or_else(|| panic!("unexpected version line `{line}`"));
            assert_eq!(hash.len(), 12, "{line}");
            assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()), "{line}");
        }
        None => panic!("unexpected version line `{line}`"),
    }
    assert!(bicraft().arg("help").output().unwrap().status.success());
}

const BIM: &str = r#"{"name":"M","model":{"tables":[{"name":"Sales",
    "columns":[{"name":"Amount","dataType":"double"}],
    "measures":[{"name":"Total","expression":"SUM(Sales[Amount])"}]}]}}"#;

fn measures(path: &str) -> Value {
    let (code, v) = exit_code(&["run", "--open", path, "--cmd", "model.summary"]);
    assert_eq!(code, 0, "{v}");
    v["results"][0]["tables"][0]["measures"].clone()
}

#[test]
fn new_project_creates_and_opens_a_project() {
    let dir = temp_dir("new-project");
    let _ = std::fs::remove_dir_all(&dir);
    let d = dir.to_str().unwrap();

    let (code, v) = exit_code(&["new-project", d, "Blank"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["details"]["type"], "project");
    assert_eq!(v["dirty"], false);
    assert!(dir.join("Blank.pbip").is_file());
    assert!(dir.join("Blank.SemanticModel").is_dir());
    assert!(dir.join("Blank.Report").is_dir());
    // Never over an existing project.
    assert_eq!(exit_code(&["new-project", d, "Blank"]).0, 2);

    // From an existing model, as TMSL; the project reopens with its measures.
    let bim = dir.join("source.bim");
    std::fs::write(&bim, BIM).unwrap();
    let (code, v) = exit_code(&[
        "new-project",
        d,
        "Shop",
        "--format",
        "tmsl",
        "--open",
        bim.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{v}");
    assert!(dir.join("Shop.SemanticModel").join("model.bim").is_file());
    assert_eq!(
        measures(dir.join("Shop.pbip").to_str().unwrap()),
        json!(["Total"])
    );

    assert_eq!(exit_code(&["new-project", d]).0, 2);
    assert_eq!(exit_code(&["new-project", d, "X", "Y"]).0, 2);
    assert_eq!(exit_code(&["new-project", d, "X", "--format", "xml"]).0, 2);
    assert_eq!(exit_code(&["new-project", d, "X", "--bogus"]).0, 2);
    assert_eq!(exit_code(&["new-project", d, "a/b"]).0, 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_edits_and_saves() {
    let dir = temp_dir("run-save");
    let bim = dir.join("model.bim");
    std::fs::write(&bim, BIM).unwrap();
    let b = bim.to_str().unwrap();

    // A failing step stops the run before --save: the file is untouched.
    let (code, v) = exit_code(&[
        "run",
        "--open",
        b,
        "--cmd",
        r#"measure.add={"table":"Sales","name":"Bad","expression":"SUM(Sales[Nope])"}"#,
        "--save",
    ]);
    assert_eq!(code, 1);
    assert_eq!(v["failed_index"], 0);
    assert_eq!(std::fs::read_to_string(&bim).unwrap(), BIM);

    let (code, v) = exit_code(&[
        "run",
        "--open",
        b,
        "--cmd",
        r#"measure.add={"table":"Sales","name":"Twice","expression":"[Total] * 2"}"#,
        "--save",
    ]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["results"][1]["format"], "bim");
    assert_eq!(measures(b), json!(["Total", "Twice"]));

    // file.save as an explicit last step works the same way.
    let (code, v) = exit_code(&[
        "run",
        "--open",
        b,
        "--cmd",
        r#"measure.remove={"name":"Twice"}"#,
        "--cmd",
        "file.save",
    ]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(measures(b), json!(["Total"]));

    // A new model saved with file.save_as.
    let out = dir.join("out").join("New.bim");
    let (code, v) = exit_code(&[
        "run",
        "--cmd",
        r#"model.new={"name":"New"}"#,
        "--cmd",
        r#"table.add={"name":"T"}"#,
        "--cmd",
        &format!(r#"file.save_as={}"#, json!({"path": out})),
    ]);
    assert_eq!(code, 0, "{v}");
    assert!(out.is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn validate_report_usage_and_unreadable_reports() {
    assert_eq!(exit_code(&["validate-report"]).0, 2);
    assert_eq!(exit_code(&["validate-report", "a", "b"]).0, 2);
    assert_eq!(exit_code(&["validate-report", "/does/not/exist.pbix"]).0, 2);
    // A model.bim has no report.
    let dir = temp_dir("validate-report-bim");
    let bim = dir.join("model.bim");
    std::fs::write(&bim, r#"{"name":"M","model":{"tables":[]}}"#).unwrap();
    let out = bicraft()
        .args(["validate-report", bim.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("report"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn validate_report_checks_fields_against_the_model() {
    let dir = temp_dir("validate-report");
    let _ = std::fs::remove_dir_all(&dir);
    let d = dir.to_str().unwrap();
    let (code, v) = exit_code(&[
        "run",
        "--cmd",
        r#"model.new={"name":"Shop"}"#,
        "--cmd",
        r#"table.add={"name":"Sales"}"#,
        "--cmd",
        r#"column.add={"table":"Sales","name":"Amount","data_type":"double"}"#,
        "--cmd",
        r#"measure.add={"table":"Sales","name":"Total","expression":"SUM(Sales[Amount])"}"#,
        "--cmd",
        "report.new",
        "--cmd",
        r#"visual.add={"page":"Page 1","visual_type":"card","x":0,"y":0,"width":200,"height":100,"fields":[{"role":"Values","field":"[Total]"}]}"#,
        "--cmd",
        &format!(
            r#"project.new={{"dir":{},"name":"Shop","format":"tmsl"}}"#,
            json!(d)
        ),
    ]);
    assert_eq!(code, 0, "{v}");
    let pbip = dir.join("Shop.pbip");
    let (code, v) = exit_code(&["validate-report", pbip.to_str().unwrap()]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["valid"], true);

    // Remove the measure the card uses: the report no longer validates.
    let (code, v) = exit_code(&[
        "run",
        "--open",
        pbip.to_str().unwrap(),
        "--cmd",
        r#"measure.remove={"name":"Total"}"#,
        "--save",
    ]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["results"][0]["report"]["affected_binding_count"], 1);
    let (code, v) = exit_code(&["validate-report", pbip.to_str().unwrap()]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["error_count"], 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `render --fail-on-empty`: exit 1 when a visual has no data (here a PBIP without data: the
/// card is `not_evaluated`), after writing the PNG and printing the result.
#[test]
fn render_fail_on_empty_exit_code() {
    let dir = temp_dir("render-empty");
    let _ = std::fs::remove_dir_all(&dir);
    let d = dir.to_str().unwrap();
    let (code, v) = exit_code(&[
        "run",
        "--cmd",
        r#"model.new={"name":"Shop"}"#,
        "--cmd",
        r#"table.add={"name":"Sales"}"#,
        "--cmd",
        r#"column.add={"table":"Sales","name":"Amount","data_type":"double"}"#,
        "--cmd",
        r#"measure.add={"table":"Sales","name":"Total","expression":"SUM(Sales[Amount])"}"#,
        "--cmd",
        "report.new",
        "--cmd",
        r#"visual.add={"page":"Page 1","visual_type":"card","x":0,"y":0,"width":200,"height":100,"fields":[{"role":"Values","field":"[Total]"}]}"#,
        "--cmd",
        &format!(
            r#"project.new={{"dir":{},"name":"Shop","format":"tmsl"}}"#,
            json!(d)
        ),
    ]);
    assert_eq!(code, 0, "{v}");
    let pbip = dir.join("Shop.pbip");
    let png = dir.join("page.png");
    let args = [
        "render",
        pbip.to_str().unwrap(),
        "--page",
        "0",
        "--out",
        png.to_str().unwrap(),
        "--no-system-fonts",
    ];
    let (code, v) = exit_code(&args);
    assert_eq!(code, 0, "{v}");
    let card = &v["pages"][0]["visuals"][0];
    assert_eq!(
        (card["status"].as_str(), card["data"].as_str()),
        (Some("preview"), Some("not_evaluated")),
        "{v}"
    );
    assert_eq!(v["pages"][0]["visuals_without_data"], 1, "{v}");
    std::fs::remove_file(&png).unwrap();
    let mut strict = args.to_vec();
    strict.push("--fail-on-empty");
    let (code, v) = exit_code(&strict);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["pages"][0]["visuals_without_data"], 1, "{v}");
    assert!(png.is_file(), "the PNG is still written");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn validate_m_exit_codes() {
    let (code, v) = exit_code(&["validate-m", "let a = Sql.Database(\"s\", \"d\") in a"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["result_type"], "Table");
    let (code, v) = exit_code(&["validate-m", "let a = Server in a"]);
    assert_eq!(code, 1);
    assert_eq!(v["diagnostics"][0]["code"], "m-undefined-name");
    assert_eq!(exit_code(&["validate-m"]).0, 2);
    assert_eq!(exit_code(&["validate-m", "1", "2"]).0, 2);
    assert_eq!(exit_code(&["validate-m", "--file", "does/not/exist.m"]).0, 2);
}

#[test]
fn validate_dax_query_exit_codes() {
    let (code, v) = exit_code(&["validate-dax-query", "EVALUATE { 1 }"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["valid"], true);
    let (code, v) = exit_code(&["validate-dax-query", "EVALUATE 1"]);
    assert_eq!(code, 1);
    assert_eq!(v["diagnostics"][0]["code"], "evaluate-needs-table");
    assert_eq!(exit_code(&["validate-dax-query"]).0, 2);
    assert_eq!(
        exit_code(&["validate-dax-query", "EVALUATE {1}", "EVALUATE {2}"]).0,
        2
    );
    assert_eq!(
        exit_code(&["validate-dax-query", "--file", "does/not/exist.dax"]).0,
        2
    );

    let dir = temp_dir("query");
    let bim = dir.join("model.bim");
    std::fs::write(
        &bim,
        r#"{"name":"M","model":{"tables":[{"name":"Sales",
            "columns":[{"name":"Amount","dataType":"double"}],
            "measures":[{"name":"Total","expression":"SUM(Sales[Amount])"}]}]}}"#,
    )
    .unwrap();
    let query = dir.join("q.dax");
    std::fs::write(
        &query,
        "DEFINE\r\n    MEASURE Sales[Twice] = [Total] * 2\r\nEVALUATE\r\n    ROW(\"T\", [Twice])\r\n",
    )
    .unwrap();
    let (bim, query) = (bim.to_str().unwrap(), query.to_str().unwrap());
    let (code, v) = exit_code(&["validate-dax-query", "--file", query, "--model", bim]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["checked_against_model"], true);
    let (code, v) = exit_code(&[
        "validate-dax-query",
        "--model",
        bim,
        "EVALUATE Sales ORDER BY Sales[Nope]",
    ]);
    assert_eq!(code, 1);
    assert_eq!(v["diagnostics"][0]["code"], "unknown-column");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dax_query_and_run_with_a_commands_file() {
    let dir = temp_dir("dax-query");
    let bim = dir.join("model.bim");
    std::fs::write(
        &bim,
        r#"{"name":"M","model":{"tables":[{"name":"T",
            "columns":[{"name":"K","dataType":"int64","sourceColumn":"K"},{"name":"V","dataType":"double","sourceColumn":"V"}],
            "partitions":[{"name":"T","source":{"type":"calculated","expression":"DATATABLE(\"K\", INTEGER, \"V\", DOUBLE, {{1, 2.5}, {2, 4}})"}}],
            "measures":[{"name":"Total","expression":"SUM(T[V])"}]}]}}"#,
    )
    .unwrap();
    let bim = bim.to_str().unwrap();
    let (code, v) = exit_code(&[
        "dax-query",
        bim,
        "EVALUATE ROW(\"t\", [Total]) EVALUATE T ORDER BY T[K] DESC",
    ]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["results"][0]["rows"][0]["[t]"], 6.5);
    assert_eq!(v["results"][1]["rows"][0]["T[K]"], 2);
    let q = dir.join("q.dax");
    std::fs::write(&q, "\u{feff}EVALUATE T").unwrap();
    let (code, v) = exit_code(&["dax-query", bim, "--file", q.to_str().unwrap(), "--max-rows", "1"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["results"][0]["truncated"], true);
    assert_eq!(exit_code(&["dax-query", bim, "EVALUATE ROW(\"x\", [Nope])"]).0, 1);
    assert_eq!(exit_code(&["dax-query", bim]).0, 2);
    assert_eq!(
        exit_code(&["dax-query", bim, "EVALUATE T", "--max-rows", "x"]).0,
        2
    );

    // run --file: the same steps as --cmd, in order after them, atomic
    let cmds = dir.join("cmds.json");
    let long = "x".repeat(40_000);
    std::fs::write(
        &cmds,
        json!([
            {"command": "measure.add", "params": {"table": "T", "name": "Big", "expression": "[Total] * 2", "description": long}},
            {"command": "dax.evaluate", "params": {"query": "EVALUATE ROW(\"b\", [Big])"}},
        ])
        .to_string(),
    )
    .unwrap();
    let c = cmds.to_str().unwrap();
    let (code, v) = exit_code(&["run", "--open", bim, "--cmd", "model.summary", "--file", c]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["results"].as_array().unwrap().len(), 3);
    assert_eq!(v["results"][2]["results"][0]["rows"][0]["[b]"], 13.0);
    std::fs::write(
        &cmds,
        r#"{"commands": [{"command": "dax.validate", "params": {"expression": "1"}}, {"command": "nope"}]}"#,
    )
    .unwrap();
    let (code, v) = exit_code(&["run", "--file", c]);
    assert_eq!(code, 1);
    assert_eq!(v["failed_index"], 1);
    assert_eq!(v["rolled_back"], true);
    for bad in [
        r#"{"a": 1}"#,
        r#"[{"params": {}}]"#,
        r#"[{"command": "x", "params": []}]"#,
        r#"[{"command": "x", "p": 1}]"#,
        "nope",
    ] {
        std::fs::write(&cmds, bad).unwrap();
        assert_eq!(exit_code(&["run", "--file", c]).0, 2, "{bad}");
    }
    assert_eq!(
        exit_code(&["run", "--file", dir.join("missing.json").to_str().unwrap()]).0,
        2
    );
}

#[test]
fn analyze_runs_default_and_extra_rules() {
    let dir = temp_dir("analyze");
    let bim = dir.join("model.bim");
    std::fs::write(
        &bim,
        r#"{"name":"M","compatibilityLevel":1567,"model":{"tables":[{"name":"Sales",
            "columns":[{"name":"Amount","dataType":"double","sourceColumn":"Amount"}],
            "measures":[{"name":"Total","expression":"SUM(Sales[Amount])"}]}]}}"#,
    )
    .unwrap();
    let rules = dir.join("rules.json");
    std::fs::write(
        &rules,
        r#"[{"ID":"MINE","Severity":1,"Scope":"Measure","Expression":"Name.StartsWith(\"T\")"}]"#,
    )
    .unwrap();
    let (bim, rules) = (bim.to_str().unwrap(), rules.to_str().unwrap());
    // The default rules include errors on this model (a measure without format string).
    let (code, v) = exit_code(&["analyze", bim]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["summary"]["evaluated"], 71);
    let (code, v) = exit_code(&["analyze", bim, "--rules", rules, "--no-default-rules"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["findings"][0]["rule_id"], "MINE");
    let (code, v) = exit_code(&[
        "analyze",
        bim,
        "--severity-min",
        "warning",
        "--rule-id",
        "MINE",
        "--rules",
        rules,
    ]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["summary"]["rules"], 0);
    let (code, _) = exit_code(&["analyze", bim, "--bogus"]);
    assert_eq!(code, 2);
}

#[test]
fn usage_and_unused_subcommands() {
    let dir = temp_dir("usage");
    let bim = dir.join("model.bim");
    std::fs::write(
        &bim,
        r#"{"name":"M","compatibilityLevel":1567,"model":{"tables":[{"name":"Sales",
            "columns":[{"name":"Amount","dataType":"double","sourceColumn":"Amount"},
                       {"name":"Spare","dataType":"double","sourceColumn":"Spare"}],
            "measures":[{"name":"Total","expression":"SUM(Sales[Amount])"},
                        {"name":"Twice","expression":"[Total] * 2"}]}]}}"#,
    )
    .unwrap();
    let bim = bim.to_str().unwrap();
    let (code, v) = exit_code(&["usage", bim, "[Total]"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["direct"][0]["name"], "Twice");
    let (code, v) = exit_code(&["usage", bim, "--kind", "measure", "--no-graph"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["counts"]["objects"], 2);
    // no report: everything is unused (exit 1), with the warning
    let (code, v) = exit_code(&["unused", bim, "--allow", "Sales[Spare]"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["counts"]["measures"], 2);
    assert_eq!(v["counts"]["columns"], 1);
    assert!(v["warnings"][0].as_str().unwrap().contains("no report"));
    let (code, v) = exit_code(&["unused", bim, "--remove", "--dry-run"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["count"], 4);
    let (code, v) = exit_code(&["unused", bim, "--remove", "--object", "[Twice]", "--save"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["count"], 1);
    let (_, v) = exit_code(&["unused", bim]);
    assert_eq!(v["counts"]["measures"], 1);
    assert_eq!(exit_code(&["unused", bim, "--ignore-metadata"]).0, 1);
    assert_eq!(exit_code(&["unused", bim, "--dry-run"]).0, 2);
    assert_eq!(exit_code(&["usage", bim, "[Total]", "extra"]).0, 2);
    assert_eq!(exit_code(&["usage", bim, "[Nope]"]).0, 2);
}

/// `bicraft-cli build-info`: what release checks read (docs/webui.md): the version and whether the
/// browser app is embedded, matching the build's `BICRAFT_WEBUI_DIR`.
#[test]
fn build_info_reports_the_browser_app() {
    let out = bicraft().arg("build-info").output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    let embedded = v["webui"]["embedded"].as_bool().unwrap();
    assert_eq!(embedded, bicraft_view::webui::available());
    assert_eq!(v["webui"]["wasm_bytes"].as_u64().unwrap() > 0, embedded);
    assert_eq!(
        bicraft()
            .args(["build-info", "x"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
}

#[test]
fn redact_writes_a_copy_without_the_secrets() {
    let dir = temp_dir("redact");
    let d = dir.to_str().unwrap();
    let bim = dir.join("source.bim");
    std::fs::write(
        &bim,
        r#"{"name":"M","model":{"tables":[{"name":"Sales","columns":[{"name":"Amount","dataType":"double"}],
        "partitions":[{"name":"Sales","mode":"import","source":{"type":"m",
        "expression":"let Source = Sql.Database(\"cli-secret-host.example\", \"CliSecretDb\") in Source"}}]}]}}"#,
    )
    .unwrap();
    let (code, v) = exit_code(&["new-project", d, "Src", "--open", bim.to_str().unwrap()]);
    assert_eq!(code, 0, "{v}");
    let out = dir.join("red");
    std::fs::create_dir_all(&out).unwrap();
    let src = dir.join("Src.pbip");
    let red = out.join("Red.pbip");
    let (code, v) = exit_code(&["redact", src.to_str().unwrap(), red.to_str().unwrap()]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["verified"]["survivors"], 0);
    assert_eq!(v["kinds"]["server"], 1, "{v}");
    let mut stack = vec![out.clone()];
    while let Some(p) = stack.pop() {
        if p.is_dir() {
            stack.extend(std::fs::read_dir(&p).unwrap().map(|e| e.unwrap().path()));
        } else {
            let text = String::from_utf8_lossy(&std::fs::read(&p).unwrap()).into_owned();
            assert!(
                !text.contains("cli-secret-host") && !text.contains("CliSecretDb"),
                "{}",
                p.display()
            );
        }
    }
    assert_eq!(exit_code(&["redact", src.to_str().unwrap()]).0, 2);
    assert_eq!(exit_code(&["redact", "a", "b", "--bogus"]).0, 2);
}
