//! Tests of `file.save`, `file.save_as` and `project.new`. Round trips through TMDL wait for the
//! TMDL writer (M1.2) and are ignored until it lands.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::{Session, Source};

fn temp(name: &str) -> TempDir {
    TempDir::new(&format!("engine-save-{name}"))
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A small model built with edit commands, covering every kind of object the commands create.
fn built() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "description": "Fact", "partition": {"source_type": "m",
                   "expression": "let\n    Source = #table({\"Amount\", \"Product\"}, {{1.5, \"A\"}})\nin\n    Source"}}),
        ),
        (
            "table.add",
            json!({"name": "Product", "partition": {"source_type": "m", "expression": "#table({\"Product\"}, {{\"A\"}})"}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double", "summarize_by": "sum"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Product", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Product", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Double", "data_type": "double", "expression": "Sales[Amount] * 2"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "0.00"}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[Product]", "to": "Product[Product]"}),
        ),
        ("role.add", json!({"name": "Readers"})),
        (
            "role.set_filter",
            json!({"role": "Readers", "table": "Product", "filter_expression": "Product[Product] = \"A\""}),
        ),
    ] {
        run(&mut s, id, p);
    }
    s
}

/// Reopens `path` in a fresh session and checks the model is the one `s` holds.
fn assert_reopens_equal(s: &Session, path: &Path) {
    let mut fresh = Session::new();
    run(&mut fresh, "file.open", json!({"path": path}));
    assert_eq!(fresh.model_error, None);
    assert_eq!(
        fresh.model,
        s.model,
        "{} does not reopen to the saved model",
        path.display()
    );
}

#[test]
fn save_needs_a_model_and_a_target() {
    let mut s = Session::new();
    assert!(
        s.execute("file.save", &json!({}))
            .unwrap_err()
            .contains("no model")
    );
    run(&mut s, "model.new", json!({"name": "M"}));
    let e = s.execute("file.save", &json!({})).unwrap_err();
    assert!(e.contains("file.save_as"), "{e}");
    assert!(s.execute("file.save", &json!({"path": "x"})).is_err());
    assert!(s.dirty);
}

#[test]
fn packages_without_a_model_cannot_be_saved() {
    let dir = temp("package");
    let path = dir.join("Empty.pbit");
    let mut pkg = bicraft_opc::Package::default();
    pkg.put("Version", b"1.28".to_vec(), false);
    let bytes = pkg.to_bytes().unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    s.model = Some(bicraft_model::SemanticModel::default());
    s.dirty = true;
    let e = s.execute("file.save", &json!({})).unwrap_err();
    assert!(e.contains("no semantic model"), "{e}");
    let e = s
        .execute("file.save_as", &json!({"path": dir.join("Out.pbix")}))
        .unwrap_err();
    assert!(e.contains("only be saved as"), "{e}");
    assert!(matches!(s.source, Some(Source::Package { .. })));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(!dir.join("Out.pbix").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

const SYNTHETIC_METADATA: &[u8] =
    include_bytes!("../../../metadata/tests/fixtures/synthetic_metadata.sqlitedb");

/// A synthetic PBIX (model: tables Calendar and Sales) written to `dir/name`.
fn synthetic_pbix(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let pkg = bicraft_pbix::synthetic::pbix(SYNTHETIC_METADATA).unwrap();
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    path
}

fn reopen(path: &Path) -> Session {
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    assert_eq!(s.model_error, None);
    s
}

/// Files in `dir` whose name ends with `suffix`.
fn files_ending(dir: &Path, suffix: &str) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(suffix))
        .collect()
}

#[test]
fn pbix_save_writes_measures_and_keeps_a_backup() {
    let dir = temp("pbix-save");
    let path = synthetic_pbix(&dir, "Shop.pbix");
    let original = std::fs::read(&path).unwrap();
    let mut s = reopen(&path);
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "bicraft test", "expression": "1", "format_string": "0"}),
    );
    run(&mut s, "role.add", json!({"name": "Auditors"}));
    run(
        &mut s,
        "role.set_filter",
        json!({"role": "Auditors", "table": "Sales", "filter_expression": "TRUE()"}),
    );
    assert!(s.dirty);
    let out = run(&mut s, "file.save", json!({}));
    assert_eq!(out["format"], "pbix");
    assert!(!s.dirty);
    let bak = dir.join("Shop.pbix.bak");
    assert_eq!(out["backup"], json!(bak.display().to_string()));
    assert_eq!(
        std::fs::read(&bak).unwrap(),
        original,
        "the backup is the previous file"
    );
    assert!(files_ending(&dir, ".bicraft-tmp").is_empty());

    let fresh = reopen(&path);
    let model = fresh.model.as_ref().unwrap();
    let (table, m) = model.measure("bicraft test").unwrap();
    assert_eq!((table.name.as_str(), m.expression.as_str()), ("Sales", "1"));
    assert_eq!(m.format_string.as_deref(), Some("0"));
    let role = model.roles.iter().find(|r| r.name == "Auditors").unwrap();
    assert_eq!(
        role.table_permissions[0].filter_expression.as_deref(),
        Some("TRUE()")
    );
    // the session now holds what the file holds
    assert_eq!(fresh.model, s.model);
    let Some(Source::Package { package, .. }) = &s.source else {
        panic!("not a package")
    };
    assert_eq!(package.to_bytes().unwrap(), std::fs::read(&path).unwrap());

    // a second save without backup leaves the first backup alone
    run(
        &mut s,
        "measure.set",
        json!({"name": "bicraft test", "format_string": "0.00"}),
    );
    let out = run(&mut s, "file.save", json!({"backup": false}));
    assert_eq!(out["backup"], Value::Null);
    assert_eq!(std::fs::read(&bak).unwrap(), original);
    let fresh = reopen(&path);
    assert_eq!(
        fresh
            .model
            .as_ref()
            .unwrap()
            .measure("bicraft test")
            .unwrap()
            .1
            .format_string
            .as_deref(),
        Some("0.00")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unsupported_pbix_changes_leave_the_file_untouched() {
    let dir = temp("pbix-unsupported");
    let path = synthetic_pbix(&dir, "Shop.pbix");
    let original = std::fs::read(&path).unwrap();
    let mut s = reopen(&path);
    run(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "Extra", "data_type": "string"}),
    );
    let e = s.execute("file.save", &json!({})).unwrap_err();
    assert!(e.contains("cannot be saved") && e.contains("Extra"), "{e}");
    assert!(s.dirty);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(!dir.join("Shop.pbix.bak").exists());
    assert!(files_ending(&dir, ".bicraft-tmp").is_empty());
    let e = s
        .execute("file.save_as", &json!({"path": dir.join("Other.pbix")}))
        .unwrap_err();
    assert!(e.contains("cannot be saved"), "{e}");
    assert!(!dir.join("Other.pbix").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pbix_save_as_writes_a_new_file_and_switches_to_it() {
    let dir = temp("pbix-save-as");
    let path = synthetic_pbix(&dir, "Shop.pbix");
    let original = std::fs::read(&path).unwrap();
    let mut s = reopen(&path);
    run(
        &mut s,
        "measure.add",
        json!({"table": "Calendar", "name": "Days", "expression": "COUNTROWS(Calendar)"}),
    );
    let target = dir.join("out").join("Copy.pbix");
    for p in [
        json!({"path": target, "format": "pbip"}),
        // A PBIX → PBIT conversion is valid (convert_tests.rs); a format that contradicts the
        // extension is not.
        json!({"path": dir.join("Copy.pbix"), "format": "pbit"}),
    ] {
        assert!(s.execute("file.save_as", &p).is_err(), "{p}");
    }
    let out = run(&mut s, "file.save_as", json!({"path": target}));
    assert_eq!(out["format"], "pbix");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        original,
        "the source file is untouched"
    );
    assert!(!s.dirty);
    let Some(Source::Package { path: now, .. }) = &s.source else {
        panic!("not a package")
    };
    assert_eq!(now, &target);
    assert!(reopen(&target).model.unwrap().measure("Days").is_some());
    // an existing target needs `overwrite`
    let e = s.execute("file.save_as", &json!({"path": path})).unwrap_err();
    assert!(e.contains("overwrite"), "{e}");
    run(&mut s, "file.save_as", json!({"path": path, "overwrite": true}));
    assert!(reopen(&path).model.unwrap().measure("Days").is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pbit_save_rewrites_the_schema() {
    let dir = temp("pbit-save");
    let path = dir.join("T.pbit");
    let schema = r#"{"name":"m","compatibilityLevel":1567,"model":{"tables":[{"name":"T","columns":[{"name":"A","dataType":"int64","sourceColumn":"A"}],"measures":[{"name":"M","expression":"1"}]}]}}"#;
    let mut pkg = bicraft_opc::Package::default();
    pkg.put(
        "Version",
        bicraft_opc::encode_text("1.28", bicraft_opc::Encoding::Utf16Le),
        false,
    );
    pkg.put(
        "DataModelSchema",
        bicraft_opc::encode_text(schema, bicraft_opc::Encoding::Utf16Le),
        false,
    );
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    let mut s = reopen(&path);
    run(
        &mut s,
        "column.add",
        json!({"table": "T", "name": "B", "data_type": "string"}),
    );
    let out = run(&mut s, "file.save", json!({}));
    assert_eq!(out["format"], "pbit");
    assert!(dir.join("T.pbit.bak").exists());
    assert_reopens_equal(&s, &path);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_models_are_saved_as_a_package_built_from_scratch() {
    let dir = temp("pbip-to-pbix");
    let mut s = built();
    let out = run(&mut s, "file.save_as", json!({"path": dir.join("Shop.pbix")}));
    assert_eq!(out["built_from_scratch"], true, "{out}");
    let back =
        bicraft_pbix::read_model(&bicraft_opc::Package::open(&dir.join("Shop.pbix")).unwrap()).unwrap();
    assert!(back.table("Sales").is_some() && back.table("Product").is_some());
    // A source bicraft cannot run is still refused, naming the table; nothing is written.
    let mut s = built();
    run(
        &mut s,
        "table.add",
        json!({"name": "Remote", "partition": {"source_type": "m", "expression": "Sql.Database(\"s\", \"d\")"}}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "Remote", "name": "Id", "data_type": "int64"}),
    );
    let e = s
        .execute("file.save_as", &json!({"path": dir.join("Remote.pbix")}))
        .unwrap_err();
    assert!(e.contains("'Remote'") && e.contains("empty_tables"), "{e}");
    assert!(!dir.join("Remote.pbix").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_as_rejects_bad_targets() {
    let dir = temp("bad-targets");
    let mut s = built();
    for p in [
        json!({}),
        json!({"path": dir.join("x.txt")}),
        json!({"path": dir.join("x.bim"), "format": "xlsx"}),
        json!({"path": dir.join("bad:name.pbip")}),
        json!({"path": dir.join("CON.pbip")}),
        json!({"path": dir.join("x.bim"), "extra": 1}),
    ] {
        assert!(s.execute("file.save_as", &p).is_err(), "{p}");
    }
    // An existing model.bim needs `overwrite`, an existing project is never replaced.
    let bim = dir.join("model.bim");
    std::fs::write(&bim, "{}").unwrap();
    let e = s.execute("file.save_as", &json!({"path": bim})).unwrap_err();
    assert!(e.contains("overwrite"), "{e}");
    std::fs::create_dir_all(dir.join("Shop.SemanticModel")).unwrap();
    let e = s
        .execute(
            "file.save_as",
            &json!({"path": dir.join("Shop.pbip"), "overwrite": true}),
        )
        .unwrap_err();
    assert!(e.contains("already exists"), "{e}");
    assert!(s.source.is_none() && s.dirty);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn project_new_rejects_bad_input() {
    let dir = temp("project-bad");
    let mut s = Session::new();
    for p in [
        json!({"dir": dir.path()}),
        json!({"name": "P"}),
        json!({"dir": dir.path(), "name": "a/b"}),
        json!({"dir": dir.path(), "name": "P."}),
        json!({"dir": dir.path(), "name": "P", "format": "xml"}),
    ] {
        assert!(s.execute("project.new", &p).is_err(), "{p}");
    }
    std::fs::write(dir.join("Taken.pbip"), "{}").unwrap();
    let e = s
        .execute("project.new", &json!({"dir": dir.path(), "name": "Taken"}))
        .unwrap_err();
    assert!(e.contains("already exists"), "{e}");
    assert!(s.source.is_none() && s.model.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bim_save_as_then_save_round_trips() {
    let dir = temp("bim");
    let mut s = built();
    let path = dir.join("sub").join("Shop.bim");
    let v = run(&mut s, "file.save_as", json!({"path": path}));
    assert_eq!(v["format"], "bim");
    assert!(!s.dirty);
    assert!(matches!(&s.source, Some(Source::Bim { path: p }) if *p == path));
    assert_reopens_equal(&s, &path);

    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Twice", "expression": "[Total] * 2"}),
    );
    assert!(s.dirty);
    let v = run(&mut s, "file.save", json!({}));
    assert_eq!(v["format"], "bim");
    assert!(!s.dirty);
    assert_eq!(run(&mut s, "file.inspect", json!({}))["dirty"], false);
    assert_reopens_equal(&s, &path);
    assert!(!dir.join("sub").join("Shop.bim.bicraft-tmp").exists());

    // Overwriting another model.bim on purpose.
    let other = dir.join("other.bim");
    std::fs::write(&other, "{}").unwrap();
    run(&mut s, "file.save_as", json!({"path": other, "overwrite": true}));
    assert_reopens_equal(&s, &other);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pbip_tmsl_save_as_then_save_round_trips() {
    let dir = temp("pbip-tmsl");
    let mut s = built();
    let v = run(
        &mut s,
        "file.save_as",
        json!({"path": dir.join("Shop.pbip"), "format": "pbip-tmsl"}),
    );
    assert_eq!(v["format"], "pbip-tmsl");
    assert!(dir.join("Shop.SemanticModel").join("model.bim").is_file());
    assert!(matches!(s.source, Some(Source::Project { .. })));
    assert_reopens_equal(&s, &dir.join("Shop.pbip"));

    run(
        &mut s,
        "table.rename",
        json!({"name": "Product", "new_name": "Item"}),
    );
    run(&mut s, "file.save", json!({}));
    assert_reopens_equal(&s, &dir);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn project_new_tmsl_from_an_empty_session_and_from_an_open_model() {
    let dir = temp("project-tmsl");
    let mut s = Session::new();
    let v = run(
        &mut s,
        "project.new",
        json!({"dir": dir.path(), "name": "Blank", "format": "tmsl"}),
    );
    assert_eq!(v["dirty"], false);
    assert_eq!(v["details"]["type"], "project");
    assert_eq!(s.model.as_ref().unwrap().name, "Blank");
    assert_reopens_equal(&s, &dir.join("Blank.pbip"));

    let mut s = built();
    run(
        &mut s,
        "project.new",
        json!({"dir": dir.path(), "name": "Shop", "format": "TMSL"}),
    );
    assert_reopens_equal(&s, &dir.join("Shop.pbip"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pbip_tmdl_save_as_then_save_round_trips() {
    let dir = temp("pbip-tmdl");
    let mut s = built();
    let v = run(&mut s, "file.save_as", json!({"path": dir.join("Shop")}));
    assert_eq!(v["format"], "pbip-tmdl");
    assert!(dir.join("Shop.SemanticModel").join("definition").is_dir());
    assert_reopens_equal(&s, &dir.join("Shop.pbip"));

    run(
        &mut s,
        "measure.set",
        json!({"name": "Total", "new_name": "Revenue"}),
    );
    run(&mut s, "file.save", json!({}));
    assert!(!s.dirty);
    assert_reopens_equal(&s, &dir);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_projects_carry_desktop_model_defaults_but_opened_models_do_not() {
    let dir = temp("project-defaults");
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "New", "culture": "fr-FR"}));
    run(&mut s, "project.new", json!({"dir": dir.path(), "name": "New"}));
    let def = dir.join("New.SemanticModel").join("definition");
    let model = std::fs::read_to_string(def.join("model.tmdl")).unwrap();
    for line in [
        "culture: fr-FR",
        "defaultPowerBIDataSourceVersion: powerBI_V3",
        "sourceQueryCulture: fr-FR",
        "valueFilterBehavior: independent",
        "legacyRedirects",
        "returnErrorValuesAsNull",
    ] {
        assert!(
            model.contains(line),
            "{line} missing:
{model}"
        );
    }
    let database = std::fs::read_to_string(def.join("database.tmdl")).unwrap();
    assert!(database.contains("compatibilityLevel: 1606"), "{database}");
    // project.new from an empty session too.
    let mut s = Session::new();
    run(&mut s, "project.new", json!({"dir": dir.path(), "name": "Blank"}));
    let m = s.model.as_ref().unwrap();
    assert_eq!(m.extra["defaultPowerBIDataSourceVersion"], "powerBI_V3");

    // A model read from a file keeps what it has: nothing is added on open or save.
    let mut s = Session::new();
    let bim = dir.join("plain.bim");
    std::fs::write(
        &bim,
        r#"{"name":"m","compatibilityLevel":1567,"model":{"culture":"en-US","tables":[]}}"#,
    )
    .unwrap();
    run(&mut s, "file.open", json!({"path": bim}));
    run(&mut s, "project.new", json!({"dir": dir.path(), "name": "Plain"}));
    let m = s.model.as_ref().unwrap();
    assert_eq!(m.compatibility_level, Some(1567));
    assert!(!m.extra.contains_key("defaultPowerBIDataSourceVersion"));
    assert!(!m.extra.contains_key("sourceQueryCulture"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn project_new_tmdl_round_trips() {
    let dir = temp("project-tmdl");
    let mut s = built();
    run(&mut s, "project.new", json!({"dir": dir.path(), "name": "Shop"}));
    assert_reopens_equal(&s, &dir.join("Shop.pbip"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn project_new_tmdl_creates_the_project_and_opens_it() {
    let dir = temp("project-tmdl-open");
    let mut s = built();
    let v = run(&mut s, "project.new", json!({"dir": dir.path(), "name": "Shop"}));
    assert_eq!(v["dirty"], false);
    assert!(dir.join("Shop.pbip").is_file());
    assert!(dir.join("Shop.SemanticModel").is_dir());
    assert!(dir.join("Shop.Report").is_dir());
    match &s.source {
        Some(Source::Project { path, project }) => {
            assert_eq!(*path, dir.join("Shop.pbip"));
            assert_eq!(project.model_format, bicraft_pbip::ModelFormat::Tmdl);
        }
        other => panic!("{other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A folder deep enough that a project's PBIR visual paths exceed Desktop's 260-character limit,
/// inside a guarded temp folder (dropping the guard removes the whole tree).
fn deep_dir(tag: &str) -> (TempDir, PathBuf) {
    let tmp = TempDir::new(&format!("maxpath-{tag}"));
    let mut d = tmp.to_path_buf();
    while d.to_string_lossy().encode_utf16().count() < 200 {
        d = d.join("very-long-folder-name");
    }
    (tmp, d)
}

#[test]
fn project_new_warns_about_paths_desktop_cannot_read() {
    let (_tmp, dir) = deep_dir("warn");
    let mut s = Session::new();
    s.execute("model.new", &json!({"name": "M"})).unwrap();
    let out = s
        .execute("project.new", &json!({"dir": dir, "name": "Sales"}))
        .unwrap();
    assert!(out["long_paths"]["count"].as_u64().unwrap() > 0, "{out}");
    assert!(out["warnings"][0].as_str().unwrap().contains("too long"));
    let report = s.execute("report.validate", &json!({})).unwrap();
    assert_eq!(report["valid"], false);
    assert!(report["issues"].to_string().contains("characters long"));
}

#[test]
fn strict_refuses_before_writing_anything() {
    let (_tmp, dir) = deep_dir("strict");
    let mut s = Session::new();
    s.execute("model.new", &json!({"name": "M"})).unwrap();
    let err = s
        .execute(
            "project.new",
            &json!({"dir": dir, "name": "Sales", "strict": true}),
        )
        .unwrap_err();
    assert!(err.contains("nothing was written"), "{err}");
    assert!(!dir.join("Sales.pbip").exists());
}

#[test]
fn short_project_paths_raise_no_warning() {
    // A short guard name keeps the project's paths well under the 260-character limit.
    let tmp = TempDir::new("mp");
    let dir = tmp.path();
    let mut s = Session::new();
    s.execute("model.new", &json!({"name": "M"})).unwrap();
    let out = s
        .execute("project.new", &json!({"dir": dir, "name": "S"}))
        .unwrap();
    assert!(out.get("long_paths").is_none(), "{out}");
}

/// A synthetic PBIX with a PBIR report (page `p1`, card `v1`) written to `dir/name`.
fn synthetic_pbir_pbix(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let pkg = bicraft_pbix::synthetic::pbir_pbix(SYNTHETIC_METADATA).unwrap();
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    path
}

fn part(path: &Path, name: &str) -> Option<Vec<u8>> {
    let pkg = bicraft_opc::Package::open(path).unwrap();
    pkg.get(name).map(|p| p.data.clone())
}

fn part_names(path: &Path) -> Vec<String> {
    let pkg = bicraft_opc::Package::open(path).unwrap();
    pkg.parts.into_iter().map(|p| p.name).collect()
}

/// Page edits, a moved visual, a new visual with a field and a title, and an active page change.
fn edit_report(s: &mut Session) -> (String, String) {
    run(
        s,
        "visual.set_position",
        json!({"page": "Overview", "visual": "v1", "x": 50, "y": 60, "width": 300}),
    );
    let page = run(s, "page.add", json!({"display_name": "Added"}))["page"]
        .as_str()
        .unwrap()
        .to_string();
    let visual = run(
        s,
        "visual.add",
        json!({"page": page, "visual_type": "card", "x": 20, "y": 20, "width": 200, "height": 120,
               "fields": [{"role": "Values", "field": "Sales[Total]"}]}),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        s,
        "visual.set_title",
        json!({"page": page, "visual": visual, "title": "Total sales"}),
    );
    run(s, "page.set_active", json!({"page": page}));
    (page, visual)
}

fn assert_report_edits(s: &Session, page: &str, visual: &str) {
    let r = s.report.as_ref().unwrap();
    assert_eq!(r.active_page.as_deref(), Some(page));
    let v1 = r.page("Overview").unwrap().visual("v1").unwrap();
    assert_eq!(
        (v1.position.x, v1.position.y, v1.position.width),
        (50.0, 60.0, 300.0)
    );
    let added = r.page(page).unwrap().visual(visual).unwrap();
    assert_eq!(added.visual_type.as_deref(), Some("card"));
    assert_eq!(
        added.fields[0].field,
        bicraft_report::FieldRef::Measure {
            table: "Sales".into(),
            measure: "Total".into()
        }
    );
    assert!(
        added.visual_extra["visualContainerObjects"]
            .to_string()
            .contains("Total sales")
    );
}

#[test]
fn report_edits_are_saved_into_a_pbir_pbix() {
    let dir = temp("pbir-report");
    let path = synthetic_pbir_pbix(&dir, "Shop.pbix");
    let model_part = part(&path, "DataModel").unwrap();
    let mut s = reopen(&path);
    let (page, visual) = edit_report(&mut s);
    assert!(s.report_dirty && !s.dirty);
    let out = run(&mut s, "file.save", json!({}));
    assert_eq!(out["report_saved"], true, "{out}");
    assert_eq!(out["model_saved"], false);
    assert_eq!(out["report_format"], "pbir");
    assert_eq!(out["security_bindings_removed"], true);
    assert_eq!(out["report_parts"]["added"], 2, "{out}");
    assert!(!s.report_dirty);
    // the model part keeps its compressed bytes
    assert_eq!(part(&path, "DataModel").unwrap(), model_part);
    assert!(part(&path, "SecurityBindings").is_none());
    assert_report_edits(&reopen(&path), &page, &visual);
    // the session holds what the file holds
    assert_eq!(reopen(&path).report, s.report);

    // removing the first page removes its parts
    run(&mut s, "page.remove", json!({"page": "Overview"}));
    let out = run(&mut s, "file.save", json!({"backup": false}));
    assert_eq!(out["report_parts"]["removed"], 2, "{out}");
    assert!(!part_names(&path).iter().any(|n| n.contains("/p1/")));
    assert_eq!(reopen(&path).report.unwrap().pages.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn report_edits_are_saved_into_a_legacy_pbix() {
    let dir = temp("legacy-report");
    let path = synthetic_pbix(&dir, "Shop.pbix");
    let model_part = part(&path, "DataModel").unwrap();
    let mut s = reopen(&path);
    run(
        &mut s,
        "page.rename",
        json!({"page": "Page 1", "display_name": "Overview"}),
    );
    let page = run(&mut s, "page.add", json!({"display_name": "Added"}))["page"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "card", "x": 20, "y": 20, "width": 200, "height": 120,
               "fields": [{"role": "Values", "field": "Sales[Total]"}]}),
    );
    let out = run(&mut s, "file.save", json!({}));
    assert_eq!(out["report_format"], "legacy", "{out}");
    assert_eq!(out["report_parts"]["changed"], 1);
    assert_eq!(part(&path, "DataModel").unwrap(), model_part);
    assert!(
        !part_names(&path)
            .iter()
            .any(|n| n.starts_with("Report/definition/"))
    );
    let fresh = reopen(&path);
    let r = fresh.report.as_ref().unwrap();
    assert_eq!(r.pages.len(), 2);
    assert_eq!(r.pages[0].display_name, "Overview");
    assert_eq!(r.pages[1].visuals[0].fields.len(), 1);
    assert_eq!(fresh.report, s.report);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn report_and_measure_edits_are_saved_together() {
    let dir = temp("report-and-model");
    let path = synthetic_pbir_pbix(&dir, "Shop.pbix");
    let mut s = reopen(&path);
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Double", "expression": "[Total] * 2"}),
    );
    run(
        &mut s,
        "visual.bind",
        // A card shows one value: the second is kept with `force` (a role warning).
        json!({"page": "Overview", "visual": "v1", "role": "Values", "field": "[Double]", "force": true}),
    );
    let target = dir.join("Copy.pbix");
    let out = run(&mut s, "file.save_as", json!({"path": target}));
    assert_eq!(
        (out["model_saved"].clone(), out["report_saved"].clone()),
        (json!(true), json!(true))
    );
    assert!(!s.dirty && !s.report_dirty);
    let fresh = reopen(&target);
    assert!(fresh.model.as_ref().unwrap().measure("Double").is_some());
    let v1 = fresh
        .report
        .as_ref()
        .unwrap()
        .page("Overview")
        .unwrap()
        .visual("v1")
        .unwrap();
    assert_eq!(v1.fields.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn report_edits_are_saved_into_a_pbit() {
    let dir = temp("pbit-report");
    let path = dir.join("T.pbit");
    let schema = r#"{"name":"m","compatibilityLevel":1567,"model":{"tables":[{"name":"Sales","columns":[{"name":"A","dataType":"int64","sourceColumn":"A"}],"measures":[{"name":"Total","expression":"1"}]}]}}"#;
    let mut pkg = bicraft_opc::Package::default();
    pkg.put(
        "Version",
        bicraft_opc::encode_text("1.32", bicraft_opc::Encoding::Utf16Le),
        false,
    );
    pkg.put(
        "DataModelSchema",
        bicraft_opc::encode_text(schema, bicraft_opc::Encoding::Utf16Le),
        false,
    );
    for (p, text) in bicraft_pbix::synthetic::pbir_documents() {
        pkg.put(&format!("Report/{p}"), text.into_bytes(), false);
    }
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    let schema_part = part(&path, "DataModelSchema").unwrap();
    let mut s = reopen(&path);
    let (page, visual) = edit_report(&mut s);
    let out = run(&mut s, "file.save", json!({}));
    assert_eq!(
        (out["format"].clone(), out["model_saved"].clone()),
        (json!("pbit"), json!(false))
    );
    assert_eq!(part(&path, "DataModelSchema").unwrap(), schema_part);
    assert_report_edits(&reopen(&path), &page, &visual);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn report_edits_a_package_cannot_hold_leave_the_file_untouched() {
    let dir = temp("report-refused");
    let path = synthetic_pbix(&dir, "Shop.pbix");
    let original = std::fs::read(&path).unwrap();
    let mut s = reopen(&path);
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Extra", "expression": "1"}),
    );
    // a legacy layout cannot carry a theme change
    if let Some(r) = s.report.as_mut() {
        r.theme_collection = Some(json!({"baseTheme": {"name": "Other"}}));
    }
    s.report_dirty = true;
    let e = s.execute("file.save", &json!({})).unwrap_err();
    assert!(e.contains("theme") && e.contains("nothing was saved"), "{e}");
    assert!(s.dirty && s.report_dirty);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(!dir.join("Shop.pbix.bak").exists());
    assert!(files_ending(&dir, ".bicraft-tmp").is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The local sample PBIX and PBIT (`BICRAFT_TESTDATA/sample1`): an identity save keeps every
/// part's bytes, and a report-only edit leaves the model part as it was.
#[test]
fn local_samples_save_report_edits() {
    let Some(root) = bicraft_env::var_os("BICRAFT_TESTDATA") else {
        eprintln!("BICRAFT_TESTDATA is not set: skipped");
        return;
    };
    for name in ["report.pbix", "template.pbit"] {
        let src = PathBuf::from(&root).join("sample1").join(name);
        if !src.exists() {
            eprintln!("{} is missing: skipped", src.display());
            continue;
        }
        let dir = temp(&format!("local-{name}"));
        let path = dir.join(name);
        std::fs::copy(&src, &path).unwrap();
        let before = bicraft_opc::Package::open(&path).unwrap();
        let mut s = reopen(&path);
        run(&mut s, "file.save", json!({"backup": false}));
        assert_eq!(
            bicraft_opc::Package::open(&path).unwrap(),
            before,
            "{name}: identity save"
        );

        let page = s.report.as_ref().unwrap().pages[0].name.clone();
        let visual = s.report.as_ref().unwrap().pages[0].visuals[0].name.clone();
        run(
            &mut s,
            "visual.set_position",
            json!({"page": page, "visual": visual, "x": 7, "y": 9}),
        );
        let out = run(&mut s, "file.save", json!({"backup": false}));
        assert_eq!(out["report_parts"]["changed"], 1, "{name}: {out}");
        let after = bicraft_opc::Package::open(&path).unwrap();
        for part in &before.parts {
            if part.name.starts_with("Report/")
                || part.name == "SecurityBindings"
                || part.name == "[Content_Types].xml"
            {
                continue;
            }
            assert_eq!(after.get(&part.name), Some(part), "{name}: {}", part.name);
        }
        let fresh = reopen(&path);
        assert_eq!(fresh.report, s.report, "{name}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Renames carried into perspectives and translations are written into a PBIX's metadata
/// (engine-written fixture with a perspective and a translated culture).
#[test]
fn pbix_renames_follow_perspectives_and_translations() {
    let dir = temp("pbix-rename-names");
    let path = dir.join("Objects.pbix");
    let sqlitedb = include_bytes!("../../../metadata/tests/engine/objects.sqlitedb");
    let pkg = bicraft_pbix::synthetic::pbix(sqlitedb).unwrap();
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    let mut s = reopen(&path);
    // `update_dax: false` keeps this a metadata-only save: the field parameter's
    // `NAMEOF([Total])` would change, and re-evaluating its table needs column storage, which
    // this metadata-only fixture lacks (see `field_parameter_follows_renames_into_a_pbix`).
    let v = run(
        &mut s,
        "measure.rename",
        json!({"name": "Total", "new_name": "Revenue", "update_dax": false}),
    );
    assert!(
        v["metadata_updated"].as_array().is_some_and(|a| a.len() >= 2),
        "{v}"
    );
    run(&mut s, "file.save", json!({"backup": false}));
    let back = reopen(&path);
    let m = back.model.as_ref().unwrap();
    let names = |v: &Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x["name"].as_str().map(str::to_string))
            .collect()
    };
    let lite = &m.extra["perspectives"][0]["tables"][0];
    assert_eq!(lite["name"], "Sales");
    assert_eq!(names(&lite["measures"]), ["Revenue"]);
    let fr = &m.extra["cultures"][1]["translations"]["model"]["tables"][0];
    assert_eq!(names(&fr["measures"]), ["Revenue"]);
    let _ = std::fs::remove_dir_all(&dir);
}
