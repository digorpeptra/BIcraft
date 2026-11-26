//! `every_command_survives_empty_and_hostile_params` (ROADMAP M11.4, AGENTS.md §3): every engine
//! command is run with `{}`, `null`, an array, wrong types, huge strings, deep nesting, extreme
//! numbers (JSON has no NaN or infinity: the largest doubles and integers stand in for them, and
//! their names as strings) and out-of-range indices, one parameter at a time over a valid
//! baseline built from the command's form (`command_ui.rs`), on
//!
//! - (a) an empty session,
//! - (b) a session with a sample PBIP project (model, data, report, theme, roles, cultures),
//! - (b2) the same model and report saved as a PBIX (VertiPaq data, the DataModel part),
//! - (c) the same project in server mode for a read-only caller (the reader gate of
//!   `Session::reader_check`, then the run).
//!
//! Each run must return `Ok` or `Err`. A panic fails the test even though `Session::execute`
//! catches it: a panic hook records every panic of the process (worker threads included), so this
//! file is its own test binary and holds this one test. After each command the session must still
//! answer the reading commands and draw its snapshot.
//!
//! Paths: hostile strings go only to parameters that are not paths; path parameters get
//! absolute paths under the test's temporary folder, empty or NUL-bearing strings and
//! non-strings, so nothing is written outside the temporary folder.

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bicraft_engine::{COMMANDS, Command, ExternalOpener, Kind, Obj, ServerConfig, Session, TokenSource};
use serde_json::{Map, Value, json};
use tempdir::TempDir;

static PANICS: Mutex<Vec<String>> = Mutex::new(Vec::new());

thread_local! {
    /// What the current thread is running, for the panic report.
    static CURRENT: RefCell<String> = const { RefCell::new(String::new()) };
    /// Panics on this thread (those of worker threads are only in [`PANICS`]).
    static MINE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn install_hook() {
    std::panic::set_hook(Box::new(|info| {
        MINE.with(|m| m.set(m.get().saturating_add(1)));
        let what = CURRENT.with(|c| c.borrow().clone());
        let thread = std::thread::current().name().unwrap_or("worker").to_string();
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        let at = info.location().map(|l| l.to_string()).unwrap_or_default();
        let mut p = PANICS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        p.push(format!("[{thread}] {what}: panicked at {at}: {msg}"));
    }));
}

fn set_current(what: String) {
    CURRENT.with(|c| *c.borrow_mut() = what);
}

/// Panics of this thread so far.
fn panic_count() -> usize {
    MINE.with(std::cell::Cell::get)
}

const DEPTH: usize = 120; // serde_json parses at most 128 levels: the deepest input a client sends

fn deep_array() -> Value {
    let mut v = json!(1);
    for _ in 0..DEPTH {
        v = Value::Array(vec![v]);
    }
    v
}

fn deep_object() -> Value {
    let mut v = json!(1);
    for _ in 0..DEPTH {
        let mut m = Map::new();
        m.insert("a".into(), v);
        v = Value::Object(m);
    }
    v
}

/// Members of the hostile objects given to `json` parameters (specs, themes, rules, patches).
const KEYS: &[&str] = &[
    "page",
    "visual",
    "name",
    "field",
    "role",
    "type",
    "value",
    "show",
    "color",
    "text",
    "x",
    "width",
    "pages",
    "visuals",
    "key",
    "table",
    "column",
    "fields",
    "gradient",
    "rules",
    "min",
    "max",
    "font_size",
    "object",
];

fn huge() -> String {
    "x".repeat(70_000)
}

/// Hostile values that are not strings.
fn non_strings() -> Vec<Value> {
    vec![
        Value::Null,
        json!(true),
        json!(false),
        json!(0),
        json!(-1),
        json!(-0.0),
        json!(1.5),
        json!(1e308),
        json!(-1e308),
        json!(5e-324),
        json!(u64::MAX),
        json!(i64::MIN),
        json!(4_294_967_297u64),
        json!([]),
        json!([null]),
        json!([1e308, -1, "x"]),
        json!({}),
        json!({"": null}),
        json!({"a": {"b": []}}),
        deep_array(),
        deep_object(),
    ]
}

/// Hostile strings for parameters that are not paths.
fn strings() -> Vec<Value> {
    [
        "",
        " ",
        "\u{0}",
        "NaN",
        "Infinity",
        "-inf",
        "-1",
        "99999999999999999999",
        "😀\u{202e}'\"[]{}\\",
        "Sales[Amount]",
        "[Total]",
        "Sum(Sales[Amount])",
        "'",
        "]",
        "[",
        "(",
        "#",
        "=",
        "*",
        "Sales[Amount]=A",
        "T[C",
        "EVALUATE",
        "let",
        "\n\n",
        "fr-FR",
        "zz-ZZ-x",
    ]
    .iter()
    .map(|s| json!(s))
    .chain([json!(huge()), json!(format!("[{}]", huge()))])
    .collect()
}

/// Hostile strings for path parameters: never a relative path.
fn path_strings(tmp: &Path) -> Vec<Value> {
    vec![
        json!(""),
        json!("\u{0}"),
        json!(huge()),
        json!(tmp.join("nul\u{0}byte").to_string_lossy()),
        json!(tmp.join("missing").join("deep").join("x.pbix").to_string_lossy()),
        json!(tmp.to_string_lossy()),
        json!(tmp.join("hostile-out.pbix").to_string_lossy()),
        json!(tmp.join("hostile-out.png").to_string_lossy()),
    ]
}

/// The names of the sample's objects, for valid baselines.
#[derive(Clone, Default)]
struct Names {
    page: String,
    visual: String,
    partition: String,
    relationship: String,
}

/// Path parameters that read an existing file: the session folder's copy of the sample.
const READ_PATHS: &[(&str, &str)] = &[
    ("file.open", "path"),
    ("project.test", "path"),
    ("file.textconv", "file"),
    ("file.diff", "a"),
    ("file.diff", "b"),
    ("file.convert", "from"),
    ("objects.import", "source"),
];

fn valid(id: &str, kind: Kind, name: &str, n: &Names, tmp: &Path) -> Value {
    if READ_PATHS.contains(&(id, name)) {
        return json!(tmp.join("Sample.pbip").to_string_lossy());
    }
    if (id, name) == ("theme.import", "path") {
        return json!(tmp.join("theme.json").to_string_lossy());
    }
    if name == "query" {
        return json!("EVALUATE ROW(\"a\", [Total])");
    }
    match kind {
        Kind::Name(o) => json!(match o {
            Obj::Table => "Sales",
            Obj::Column => "Amount",
            Obj::Measure => "Total",
            Obj::Hierarchy => "Products",
            Obj::Partition => n.partition.as_str(),
            Obj::Expression => "Region",
            Obj::Relationship => n.relationship.as_str(),
            Obj::Role => "Readers",
            Obj::Culture => "fr-FR",
            Obj::CalculationGroup => "Time Calc",
            Obj::CalculationItem => "Current",
            Obj::Page => n.page.as_str(),
            Obj::Visual => n.visual.as_str(),
            Obj::ReportMeasure => "Report Total",
            Obj::DataRole => "Category",
            Obj::VisualType => "clusteredColumnChart",
            Obj::Profile => "dev",
        }),
        Kind::Text => json!(format!("Hostile {name}")),
        Kind::Field => json!("Sales[Amount]"),
        Kind::Dax | Kind::Code => json!("1"),
        Kind::M => json!("1"),
        Kind::Path => json!(tmp.join(format!("valid-{name}")).to_string_lossy()),
        Kind::Paths => json!([]),
        Kind::PathOrText => json!("A,B\n1,2\n"),
        Kind::Bool => json!(true),
        Kind::Integer => json!(1),
        Kind::Number => json!(10),
        Kind::Enum(values) => json!(values.first().copied().unwrap_or("")),
        Kind::Color => json!("#112233"),
        Kind::List => json!([]),
        Kind::Json => json!({}),
    }
}

/// Every parameter set a command is run with.
fn variants(c: &Command, n: &Names, tmp: &Path) -> Vec<Value> {
    let mut base = Map::new();
    for p in c.form.iter().filter(|p| p.required) {
        base.insert(p.name.into(), valid(c.id, p.kind, p.name, n, tmp));
    }
    let base = Value::Object(base);
    let mut out = vec![
        Value::Null,
        json!({}),
        json!([]),
        json!([{}]),
        json!(42),
        json!("x"),
        json!(true),
        deep_array(),
        deep_object(),
        json!({"__unknown__": 1}),
        base.clone(),
    ];
    for p in c.form {
        let is_path = matches!(p.kind, Kind::Path | Kind::Paths);
        let mut values = non_strings();
        if is_path {
            values.extend(path_strings(tmp));
            values.push(json!([huge()]));
        } else {
            values.extend(strings());
        }
        match p.kind {
            // lists of hostile items, objects with hostile members
            Kind::List | Kind::Json => {
                if p.kind == Kind::List {
                    values.extend(non_strings().into_iter().chain(strings()).map(|v| json!([v])));
                }
                let few = [
                    Value::Null,
                    json!(-1),
                    json!(1e308),
                    json!(u64::MAX),
                    json!([]),
                    json!({}),
                    deep_object(),
                    json!(""),
                    json!("\u{0}"),
                    json!(huge()),
                    json!("Sales[Amount]"),
                    json!("NaN"),
                ];
                for key in KEYS {
                    if p.kind == Kind::List {
                        values.extend(few.iter().map(|v| json!([{*key: v}])));
                    } else {
                        values.extend(few.iter().map(|v| json!({*key: v})));
                    }
                }
            }
            _ => {}
        }
        values.push(valid(c.id, p.kind, p.name, n, tmp));
        for v in values {
            let mut params = base.clone();
            if let Some(m) = params.as_object_mut() {
                m.insert(p.name.into(), v);
            }
            out.push(params);
        }
    }
    out
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p)
        .unwrap_or_else(|e| panic!("sample setup: {id}: {e}"))
}

/// Writes the sample project `Sample.pbip` into `dir`.
fn build_sample(dir: &Path) {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Sample"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "csv": "Product,Region,Amount,Date\nA,North,1.5,2025-01-01\nB,South,2,2025-01-02\nA,South,-3,2025-01-03\n,North,0,2025-01-04\n"}),
    );
    run(
        &mut s,
        "table.import",
        json!({"name": "Product", "csv": "Product,Name\nA,Alpha\nB,Beta\n"}),
    );
    for (id, p) in [
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Double", "data_type": "double", "expression": "Sales[Amount] * 2"}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[Product]", "to": "Product[Product]"}),
        ),
        (
            "hierarchy.add",
            json!({"table": "Product", "name": "Products", "levels": ["Product", "Name"]}),
        ),
        ("role.add", json!({"name": "Readers"})),
        (
            "role.set_filter",
            json!({"role": "Readers", "table": "Sales", "filter_expression": "Sales[Amount] > 0"}),
        ),
        ("culture.add", json!({"name": "fr-FR"})),
        (
            "translation.set",
            json!({"culture": "fr-FR", "object": {"table": "Sales"}, "property": "caption", "value": "Ventes"}),
        ),
        (
            "expression.add",
            json!({"name": "Region", "expression": "\"North\" meta [IsParameterQuery = true, Type = \"Text\", IsParameterQueryRequired = true]"}),
        ),
        ("calculation_group.add", json!({"name": "Time Calc"})),
        (
            "calculation_item.add",
            json!({"group": "Time Calc", "name": "Current", "expression": "SELECTEDMEASURE()"}),
        ),
        ("report.new", json!({})),
    ] {
        run(&mut s, id, p);
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    for (ty, x, fields) in [
        (
            "clusteredColumnChart",
            0,
            json!([{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]),
        ),
        (
            "slicer",
            420,
            json!([{"role": "Values", "field": "Sales[Product]"}]),
        ),
        (
            "tableEx",
            0,
            json!([{"role": "Values", "field": "Product[Name]"}, {"role": "Values", "field": "[Total]"}]),
        ),
        ("card", 420, json!([{"role": "Values", "field": "[Total]"}])),
    ] {
        let y = if ty == "tableEx" || ty == "card" { 320 } else { 0 };
        run(
            &mut s,
            "visual.add",
            json!({"page": page, "visual_type": ty, "x": x, "y": y, "width": 400, "height": 300, "fields": fields}),
        );
    }
    run(&mut s, "page.add", json!({"display_name": "Second"}));
    run(
        &mut s,
        "report_measure.add",
        json!({"table": "Sales", "name": "Report Total", "expression": "[Total] + 1"}),
    );
    run(
        &mut s,
        "theme.set_colors",
        json!({"data_colors": ["#112233", "#445566"]}),
    );
    run(&mut s, "project.new", json!({"dir": dir, "name": "Sample"}));
    run(&mut s, "theme.export", json!({"path": dir.join("theme.json")}));
    // the same model and report as a PBIX, with its VertiPaq data
    run(&mut s, "file.save_as", json!({"path": dir.join("Sample.pbix")}));
    std::fs::write(
        dir.join("bicraft.env.yaml"),
        "profiles:\n  dev:\n    parameters:\n      Region: South\n",
    )
    .unwrap();
}

fn names(s: &Session) -> Names {
    let mut n = Names::default();
    if let Some(r) = &s.report
        && let Some(p) = r.pages.first()
    {
        n.page = p.name.clone();
        n.visual = p.visuals.first().map(|v| v.name.clone()).unwrap_or_default();
    }
    if let Some(m) = &s.model {
        n.partition = m
            .tables
            .iter()
            .find(|t| t.name == "Sales")
            .and_then(|t| t.partitions.first())
            .map(|p| p.name.clone())
            .unwrap_or_default();
        n.relationship = m
            .relationships
            .first()
            .map(|r| r.name.clone())
            .unwrap_or_default();
    }
    n
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind3 {
    Empty,
    Sample,
    Pbix,
    Reader,
}

/// A fresh session of kind `k` in its own temporary folder (holding a copy of the sample).
fn session(k: Kind3, template: &Path, tmp: &Path) -> Session {
    let mut s = match k {
        Kind3::Empty => return Session::new(),
        Kind3::Sample | Kind3::Pbix => Session::new(),
        Kind3::Reader => Session::server(ServerConfig::new(tmp).unwrap()),
    };
    copy_dir(template, tmp);
    let path = match k {
        Kind3::Reader => "Sample.pbip".to_string(),
        Kind3::Pbix => tmp.join("Sample.pbix").to_string_lossy().into_owned(),
        _ => tmp.join("Sample.pbip").to_string_lossy().into_owned(),
    };
    s.execute("file.open", &json!({"path": path}))
        .unwrap_or_else(|e| panic!("open the sample: {e}"));
    if k != Kind3::Reader {
        // a step to undo and one to redo
        for (id, p) in [
            ("table.add", json!({"name": "Scratch 1"})),
            ("table.add", json!({"name": "Scratch 2"})),
            ("edit.undo", json!({})),
        ] {
            run(&mut s, id, p);
        }
    }
    s
}

/// Nothing is launched and no token is ever found.
fn harmless(s: &mut Session) {
    s.external_opener = Some(ExternalOpener(Arc::new(|_| Ok(()))));
    s.token_source = Some(TokenSource(Arc::new(|| Err("no token in this test".into()))));
}

/// The session still answers: the reading commands run without a panic, those that must succeed
/// succeed, and the snapshot draws.
fn check_valid(s: &mut Session, k: Kind3, problems: &mut Vec<String>, id: &str) {
    for (cmd, must) in [
        ("session.revision", true),
        ("file.inspect", s.source.is_some()),
        ("edit.history", true),
        ("view.interactions", true),
        ("model.summary", false),
        ("model.validate", false),
        ("report.summary", false),
        ("report.validate", false),
    ] {
        let r = s.execute(cmd, &json!({}));
        if let Err(e) = &r
            && (e.starts_with("internal error") || (must && k != Kind3::Reader))
        {
            problems.push(format!("{k:?} after `{id}`: {cmd} fails: {e}"));
        }
        if s.model.is_some()
            && cmd == "model.validate"
            && let Err(e) = r
        {
            problems.push(format!(
                "{k:?} after `{id}`: model.validate fails on a model: {e}"
            ));
        }
    }
    let _ = s.snapshot();
}

fn sweep(k: Kind3, template: &Path, root: &Path) -> (Vec<String>, usize) {
    let mut problems = Vec::new();
    let mut runs = 0usize;
    let mut never_ok = Vec::new();
    let started = std::time::Instant::now();
    for (i, c) in COMMANDS.iter().enumerate() {
        let t0 = std::time::Instant::now();
        let mut oks = 0usize;
        let dir = root.join(format!("{k:?}-{i}"));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = session(k, template, &dir);
        let n = names(&s);
        for (j, p) in variants(c, &n, &dir).into_iter().enumerate() {
            harmless(&mut s);
            let shown = p.to_string();
            let short: String = shown.chars().take(160).collect();
            set_current(format!("{k:?} `{}` variant {j} {short}", c.id));
            let before = panic_count();
            if k == Kind3::Reader && s.reader_check(c.id, &p).is_err() {
                continue;
            }
            runs += 1;
            let r = s.execute(c.id, &p);
            oks += usize::from(r.is_ok());
            if let Err(e) = &r
                && e.starts_with("internal error")
            {
                problems.push(format!("{k:?} `{}` {short}: {e}", c.id));
            }
            if panic_count() != before {
                problems.push(format!("{k:?} `{}` {short}: panicked", c.id));
            }
        }
        harmless(&mut s);
        set_current(format!("{k:?} validity after `{}`", c.id));
        let before = panic_count();
        check_valid(&mut s, k, &mut problems, c.id);
        if panic_count() != before {
            problems.push(format!("{k:?} after `{}`: the validity checks panicked", c.id));
        }
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
        if oks == 0 {
            never_ok.push(c.id);
        }
        let took = t0.elapsed();
        if took.as_secs() >= 5 {
            eprintln!("{k:?}: `{}` took {took:.1?}", c.id);
        }
    }
    eprintln!(
        "{k:?}: {runs} runs in {:.1?}; no run succeeded for {} commands: {never_ok:?}",
        started.elapsed(),
        never_ok.len()
    );
    (problems, runs)
}

#[test]
fn every_command_survives_empty_and_hostile_params() {
    install_hook();
    // nothing may be written outside the temporary folder: the working directory is watched
    let cwd = std::env::current_dir().unwrap();
    let listing = |dir: &Path| -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    };
    let cwd_before = listing(&cwd);
    let tmp = TempDir::new("engine-hostile-params");
    let template = tmp.path().join("template");
    std::fs::create_dir_all(&template).unwrap();
    build_sample(&template);
    let template: PathBuf = template;
    let results: Vec<(Kind3, Vec<String>, usize)> = std::thread::scope(|scope| {
        let handles: Vec<_> = [Kind3::Empty, Kind3::Sample, Kind3::Pbix, Kind3::Reader]
            .into_iter()
            .map(|k| {
                let template = template.clone();
                let root = tmp.path().join(format!("{k:?}"));
                std::thread::Builder::new()
                    .name(format!("{k:?}"))
                    .spawn_scoped(scope, move || {
                        let (p, n) = sweep(k, &template, &root);
                        (k, p, n)
                    })
                    .unwrap()
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let cwd_after = listing(&cwd);
    let written: Vec<&String> = cwd_after.iter().filter(|f| !cwd_before.contains(f)).collect();
    assert!(
        written.is_empty(),
        "files written into the working directory {}: {written:?}",
        cwd.display()
    );
    let panics = PANICS.lock().unwrap().clone();
    let mut report = String::new();
    for (k, problems, runs) in &results {
        eprintln!("{k:?}: {runs} runs, {} problems", problems.len());
        for p in problems.iter().take(200) {
            report.push_str(&format!("{p}\n"));
        }
    }
    for p in panics.iter().take(200) {
        report.push_str(&format!("{p}\n"));
    }
    let _ = std::panic::take_hook();
    assert!(
        panics.is_empty() && results.iter().all(|(_, p, _)| p.is_empty()),
        "{} panics:\n{report}",
        panics.len()
    );
}
