//! Tests of conversions (`file.save_as` to another kind, `file.convert`), on synthetic files.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::*;

const SYNTHETIC_METADATA: &[u8] =
    include_bytes!("../../../metadata/tests/fixtures/synthetic_metadata.sqlitedb");

fn temp(name: &str) -> TempDir {
    TempDir::new(&format!("engine-convert-{name}"))
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn write_pkg(path: &Path, pkg: &bicraft_opc::Package) {
    std::fs::write(path, pkg.to_bytes().unwrap()).unwrap();
}

fn legacy_pbix(dir: &Path) -> PathBuf {
    let path = dir.join("Legacy.pbix");
    write_pkg(&path, &bicraft_pbix::synthetic::pbix(SYNTHETIC_METADATA).unwrap());
    path
}

fn pbir_pbix(dir: &Path) -> PathBuf {
    let path = dir.join("Pbir.pbix");
    write_pkg(
        &path,
        &bicraft_pbix::synthetic::pbir_pbix(SYNTHETIC_METADATA).unwrap(),
    );
    path
}

fn open(path: &Path) -> Session {
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    assert_eq!(s.model_error, None, "{}", path.display());
    s
}

/// Table and measure names: what must survive every conversion.
fn shape(s: &Session) -> Vec<(String, Vec<String>)> {
    let mut v: Vec<(String, Vec<String>)> = s
        .model
        .as_ref()
        .unwrap()
        .tables
        .iter()
        .map(|t| {
            (
                t.name.clone(),
                t.measures.iter().map(|m| m.name.clone()).collect(),
            )
        })
        .collect();
    v.sort();
    v
}

fn kind(path: &Path) -> bicraft_pbix::Kind {
    let pkg = bicraft_opc::Package::open(path).unwrap();
    bicraft_pbix::inventory(&pkg).kind
}

#[test]
fn legacy_pbix_to_pbit_keeps_the_layout() {
    let dir = temp("pbix-pbit");
    let src = legacy_pbix(&dir);
    let mut s = open(&src);
    let expected = shape(&s);
    let out = run(&mut s, "file.save_as", json!({"path": dir.join("T.pbit")}));
    assert_eq!(out["format"], "pbit", "{out}");
    assert_eq!(out["converted_from"], "pbix");
    assert_eq!(out["report_format"], "legacy");
    assert_eq!(kind(&dir.join("T.pbit")), bicraft_pbix::Kind::Pbit);
    let pkg = bicraft_opc::Package::open(&dir.join("T.pbit")).unwrap();
    let layout = pkg.get("Report/Layout").unwrap();
    assert_eq!(
        bicraft_opc::decode_text("Report/Layout", &layout.data).unwrap().0,
        bicraft_pbix::synthetic::LAYOUT
    );
    assert!(pkg.get("Report/StaticResources/x.png").is_some());
    assert!(bicraft_pixwriter_check(&pkg).is_empty());
    // The session now holds the template.
    assert!(matches!(&s.source, Some(Source::Package { path, .. }) if path.ends_with("T.pbit")));
    assert!(!s.dirty && !s.report_dirty);
    assert_eq!(shape(&open(&dir.join("T.pbit"))), expected);
    // An existing target needs `overwrite`.
    let mut again = open(&src);
    let e = again
        .execute("file.save_as", &json!({"path": dir.join("T.pbit")}))
        .unwrap_err();
    assert!(e.contains("overwrite"), "{e}");
    run(
        &mut again,
        "file.save_as",
        json!({"path": dir.join("T.pbit"), "overwrite": true}),
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn bicraft_pixwriter_check(pkg: &bicraft_opc::Package) -> Vec<String> {
    bicraft_pbixwriter::check_package(pkg)
}

#[test]
fn pbix_to_pbip_carries_cache_resources_and_legacy_report() {
    let dir = temp("pbix-pbip");
    let src = legacy_pbix(&dir);
    let mut s = open(&src);
    let expected = shape(&s);
    let out = run(
        &mut s,
        "file.save_as",
        json!({"path": dir.join("out").join("Shop.pbip")}),
    );
    assert_eq!(out["format"], "pbip-tmdl", "{out}");
    assert_eq!(out["cache"], true);
    assert_eq!(out["report_format"], "legacy");
    let root = dir.join("out");
    let report = root.join("Shop.Report");
    assert!(report.join("report.json").is_file());
    assert!(!report.join("definition").exists());
    assert!(report.join("StaticResources/x.png").is_file());
    let cache = std::fs::read(root.join("Shop.SemanticModel/.pbi/cache.abf")).unwrap();
    let data_model = bicraft_opc::Package::open(&src)
        .unwrap()
        .get("DataModel")
        .unwrap()
        .data
        .clone();
    assert_eq!(
        cache, data_model,
        "an unedited PBIX gives its DataModel as the cache"
    );
    assert!(root.join("Shop.SemanticModel/definition/model.tmdl").is_file());
    let reopened = open(&root.join("Shop.pbip"));
    assert_eq!(shape(&reopened), expected);
    assert_eq!(reopened.report.as_ref().map(|r| r.pages.len()), Some(1));

    // Back to a PBIX: the project's cache holds the data.
    let mut p = reopened;
    let out = run(&mut p, "file.save_as", json!({"path": dir.join("Back.pbix")}));
    assert_eq!(out["format"], "pbix", "{out}");
    assert_eq!(out["converted_from"], "pbip");
    assert_eq!(kind(&dir.join("Back.pbix")), bicraft_pbix::Kind::Pbix);
    let back = open(&dir.join("Back.pbix"));
    assert_eq!(shape(&back), expected);
    let pkg = bicraft_opc::Package::open(&dir.join("Back.pbix")).unwrap();
    assert!(pkg.get("Report/Layout").is_some());
    assert!(pkg.get("Report/StaticResources/x.png").is_some());
    assert!(bicraft_pixwriter_check(&pkg).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn legacy_reports_convert_to_pbir_only_on_request() {
    let dir = temp("legacy-pbir");
    let src = legacy_pbix(&dir);
    let mut s = open(&src);
    let out = run(
        &mut s,
        "file.save_as",
        json!({"path": dir.join("P.pbip"), "report_format": "pbir", "cache": false}),
    );
    assert_eq!(out["report_format"], "pbir", "{out}");
    assert_eq!(out["report_converted_to_pbir"], true);
    assert_eq!(out["cache"], false);
    let report = dir.join("P.Report");
    assert!(report.join("definition/report.json").is_file());
    assert!(!report.join("report.json").exists());
    assert!(report.join("StaticResources/x.png").is_file());
    assert!(!dir.join("P.SemanticModel/.pbi/cache.abf").exists());
    let text = std::fs::read_to_string(report.join("definition/report.json")).unwrap();
    assert!(!text.contains("legacy:"), "{text}");
    let warnings = out["warnings"].to_string();
    assert!(warnings.contains("without data"), "{warnings}");

    // Same kind: a legacy PBIX re-written as a PBIR PBIX.
    let mut s = open(&src);
    let out = run(
        &mut s,
        "file.save_as",
        json!({"path": dir.join("Pbir.pbix"), "report_format": "pbir"}),
    );
    assert_eq!(out["report_format"], "pbir", "{out}");
    let pkg = bicraft_opc::Package::open(&dir.join("Pbir.pbix")).unwrap();
    assert!(pkg.get("Report/Layout").is_none());
    assert!(pkg.get("Report/definition/report.json").is_some());
    assert_eq!(
        bicraft_opc::decode_text("Version", &pkg.get("Version").unwrap().data)
            .unwrap()
            .0,
        "1.32"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pbir_pbix_round_trips_through_pbip_and_pbit() {
    let dir = temp("pbir");
    let src = pbir_pbix(&dir);
    let mut s = open(&src);
    let expected = shape(&s);
    let out = run(
        &mut s,
        "file.save_as",
        json!({"path": dir.join("R.pbip"), "format": "pbip-tmsl"}),
    );
    assert_eq!(out["format"], "pbip-tmsl", "{out}");
    assert_eq!(out["report_format"], "pbir");
    assert_eq!(out["report"]["visuals"], 1);
    assert!(out["warnings"].to_string().contains("SecurityBindings"), "{out}");
    assert!(dir.join("R.SemanticModel/model.bim").is_file());
    assert!(
        dir.join("R.Report/StaticResources/SharedResources/BaseThemes/Base.json")
            .is_file()
    );
    // PBIP (TMSL) → PBIP (TMDL) → PBIT → PBIP.
    let mut p = open(&dir.join("R.pbip"));
    let out = run(
        &mut p,
        "file.save_as",
        json!({"path": dir.join("tmdl").join("R.pbip")}),
    );
    assert_eq!(out["format"], "pbip-tmdl", "{out}");
    assert_eq!(out["cache"], true);
    assert!(dir.join("tmdl/R.SemanticModel/definition").is_dir());
    let out = run(&mut p, "file.save_as", json!({"path": dir.join("R.pbit")}));
    assert_eq!(out["report_format"], "pbir", "{out}");
    let mut t = open(&dir.join("R.pbit"));
    assert_eq!(shape(&t), expected);
    let out = run(
        &mut t,
        "file.save_as",
        json!({"path": dir.join("fromt").join("R")}),
    );
    assert_eq!(out["cache"], false, "{out}");
    let back = open(&dir.join("fromt").join("R.pbip"));
    assert_eq!(shape(&back), expected);
    assert_eq!(back.report.as_ref().map(|r| r.pages[0].visuals.len()), Some(1));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pbix_targets_without_data_compute_it_or_name_the_table() {
    let dir = temp("no-data");
    let src = pbir_pbix(&dir);
    let mut s = open(&src);
    run(&mut s, "file.save_as", json!({"path": dir.join("T.pbit")}));
    // PBIT → PBIX: the data is computed; this table's query does not give its columns.
    let e = s
        .execute("file.save_as", &json!({"path": dir.join("X.pbix")}))
        .unwrap_err();
    assert!(
        e.contains("PBIT") && e.contains("'Sales'") && e.contains("empty_tables"),
        "{e}"
    );
    // PBIP without a cache → PBIX: the same.
    run(&mut s, "file.save_as", json!({"path": dir.join("P.pbip")}));
    let e = s
        .execute("file.save_as", &json!({"path": dir.join("X.pbix")}))
        .unwrap_err();
    assert!(e.contains("cache.abf") && e.contains("'Sales'"), "{e}");
    // `empty_tables` is for PBIX targets only.
    let e = s
        .execute(
            "file.save_as",
            &json!({"path": dir.join("Y.pbit"), "empty_tables": true}),
        )
        .unwrap_err();
    assert!(e.contains("empty_tables"), "{e}");
    // `cache: true` without data is an error, and nothing is written.
    let e = s
        .execute(
            "file.save_as",
            &json!({"path": dir.join("Q.pbip"), "cache": true}),
        )
        .unwrap_err();
    assert!(e.contains("cache"), "{e}");
    assert!(!dir.join("Q.pbip").exists());
    assert!(!dir.join("X.pbix").exists());
    // A new model gets a DataModel built from scratch.
    let mut n = Session::new();
    run(&mut n, "model.new", json!({"name": "M"}));
    let out = run(&mut n, "file.save_as", json!({"path": dir.join("N.pbix")}));
    assert_eq!(out["built_from_scratch"], true, "{out}");
    assert_eq!(kind(&dir.join("N.pbix")), bicraft_pbix::Kind::Pbix);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_models_become_templates() {
    let dir = temp("new-pbit");
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Shop"}));
    run(
        &mut s,
        "table.add",
        json!({"name": "T", "partition": {"source_type": "m", "expression": "#table({\"A\"}, {{1}})"}}),
    );
    run(
        &mut s,
        "column.add",
        json!({"table": "T", "name": "A", "data_type": "int64"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "T", "name": "Total", "expression": "SUM(T[A])"}),
    );
    let expected = shape(&s);
    let out = run(&mut s, "file.save_as", json!({"path": dir.join("Shop.pbit")}));
    assert_eq!(out["converted_from"], "new", "{out}");
    assert_eq!(out["report"]["pages"], 1);
    assert_eq!(shape(&open(&dir.join("Shop.pbit"))), expected);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn model_edits_follow_into_the_cache() {
    let dir = temp("edits");
    let src = legacy_pbix(&dir);
    let mut s = open(&src);
    let table = s.model.as_ref().unwrap().tables[0].name.clone();
    run(
        &mut s,
        "measure.add",
        json!({"table": table, "name": "Converted", "expression": "1", "force": true}),
    );
    run(&mut s, "file.save_as", json!({"path": dir.join("E.pbip")}));
    let cache = std::fs::read(dir.join("E.SemanticModel/.pbi/cache.abf")).unwrap();
    let cached = bicraft_pbix::read_data_model(&cache).unwrap();
    assert!(cached.measure("Converted").is_some());
    // The project and its cache agree, so it converts back to a PBIX.
    let mut p = open(&dir.join("E.pbip"));
    run(&mut p, "file.save_as", json!({"path": dir.join("E.pbix")}));
    assert!(
        open(&dir.join("E.pbix"))
            .model
            .unwrap()
            .measure("Converted")
            .is_some()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn convert_leaves_the_open_file_alone() {
    let dir = temp("convert-cmd");
    let src = legacy_pbix(&dir);
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Mine"}));
    let revision = s.revision;
    let out = run(
        &mut s,
        "file.convert",
        json!({"from": src, "to": dir.join("C.pbit")}),
    );
    assert_eq!(out["format"], "pbit", "{out}");
    assert_eq!(out["from"], json!(src));
    assert!(dir.join("C.pbit").is_file());
    assert!(s.source.is_none() && s.dirty);
    assert_eq!(s.model.as_ref().unwrap().name, "Mine");
    assert_eq!(s.revision, revision);
    for bad in [
        json!({"from": src}),
        json!({"to": dir.join("x.pbit")}),
        json!({"from": dir.join("missing.pbix"), "to": dir.join("x.pbit")}),
        json!({"from": src, "to": dir.join("x.pbit"), "extra": 1}),
    ] {
        assert!(s.execute("file.convert", &bad).is_err(), "{bad}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn conversion_options_are_checked() {
    let dir = temp("options");
    let src = legacy_pbix(&dir);
    let mut s = open(&src);
    for (p, needle) in [
        (json!({"path": dir.join("m.bim"), "cache": true}), "model.bim"),
        (
            json!({"path": dir.join("m.bim"), "report_format": "pbir"}),
            "model.bim",
        ),
        (json!({"path": dir.join("a.pbit"), "cache": true}), "cache"),
        (json!({"path": dir.join("a.pbix"), "cache": false}), "cache"),
        (
            json!({"path": dir.join("a.pbip"), "report_format": "legacy"}),
            "report_format",
        ),
        (json!({"path": dir.join("a.pbip"), "format": "pbit"}), "extension"),
        (json!({"path": dir.join("a.pbix"), "format": "pbip"}), "extension"),
        (json!({"path": dir.join("a.pbit"), "bogus": 1}), "bogus"),
    ] {
        let e = s.execute("file.save_as", &p).unwrap_err();
        assert!(e.contains(needle), "{p}: {e}");
    }
    assert!(
        std::fs::read_dir(&dir).unwrap().count() == 1,
        "nothing but the source"
    );
    // The same kind keeps its own save path; `format` naming it is accepted.
    let out = run(
        &mut s,
        "file.save_as",
        json!({"path": dir.join("Same.pbix"), "format": "pbix"}),
    );
    assert_eq!(out["format"], "pbix", "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn long_project_paths_are_refused_with_strict() {
    let dir = temp("strict");
    let src = legacy_pbix(&dir);
    let mut s = open(&src);
    let deep = dir.join("d".repeat(120)).join("e".repeat(100));
    let e = s
        .execute(
            "file.save_as",
            &json!({"path": deep.join("Shop.pbip"), "strict": true}),
        )
        .unwrap_err();
    assert!(e.contains("too long"), "{e}");
    assert!(!deep.join("Shop.pbip").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Optional: the real samples under `BICRAFT_TESTDATA` (never copied into the repo). Prints
/// counts only.
#[test]
fn real_samples_convert_in_every_direction() {
    let Some(data) = bicraft_env::var_os("BICRAFT_TESTDATA").map(PathBuf::from) else {
        eprintln!("note: BICRAFT_TESTDATA is not set; skipping real-sample conversions");
        return;
    };
    let dir = temp("real");
    for sample in ["sample1/report.pbix", "sample1/template.pbit"] {
        let src = data.join(sample);
        if !src.is_file() {
            eprintln!("note: {sample} not found; skipping");
            continue;
        }
        let base = open(&src);
        let expected = shape(&base);
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_string();
        let mut targets = vec![
            format!("{ext}-tmdl/S.pbip"),
            format!("{ext}-tmsl/S.pbip"),
            format!("{ext}-pbir/S.pbip"),
            format!("{ext}.pbit"),
        ];
        if ext == "pbix" {
            targets.push("pbix-pbir.pbix".into());
        }
        for t in &targets {
            let mut s = open(&src);
            let mut p = json!({"path": dir.join(t)});
            if t.contains("tmsl") {
                p["format"] = json!("pbip-tmsl");
            }
            if t.contains("pbir") {
                p["report_format"] = json!("pbir");
            }
            if t.ends_with(".pbit") && ext == "pbit" {
                p["report_format"] = json!("pbir");
            }
            // Client data: on failure, report the step without the message (it names model objects).
            let out = s
                .execute("file.save_as", &p)
                .unwrap_or_else(|e| panic!("{sample} -> {t} failed ({} characters of error)", e.len()));
            eprintln!(
                "{sample} -> {t}: model {}, report {}, cache {}, {} warnings",
                out["model"],
                out["report"],
                out["cache"],
                out["warnings"].as_array().map_or(0, Vec::len)
            );
            let reopened = open(&dir.join(t));
            assert_eq!(shape(&reopened), expected, "{sample} -> {t}");
        }
        // PBIP (with the cache from the PBIX) → PBIX.
        if ext == "pbix" {
            let mut p = open(&dir.join("pbix-tmdl/S.pbip"));
            let out = run(
                &mut p,
                "file.save_as",
                json!({"path": dir.join("pbip-back.pbix")}),
            );
            eprintln!("pbix -> pbip -> pbix: model {}", out["model"]);
            assert_eq!(shape(&open(&dir.join("pbip-back.pbix"))), expected);
        }
    }
    if bicraft_env::var_os("BICRAFT_KEEP_CONVERSIONS").is_some() {
        eprintln!("kept in {}", dir.display());
    } else {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The Copilot container (pbix-package.md §2, "Copilot containers") goes PBIX → PBIP → PBIX:
/// UTF-16LE parts become UTF-8 files under `<Name>.SemanticModel/Copilot/` and back, byte for
/// byte; `file.inspect` summarizes it; a save of the PBIX keeps the parts untouched.
#[test]
fn copilot_containers_survive_conversions() {
    let dir = temp("copilot");
    let mut pkg = bicraft_pbix::synthetic::pbir_pbix(SYNTHETIC_METADATA).unwrap();
    let utf16 = |t: &str| bicraft_opc::encode_text(t, bicraft_opc::Encoding::Utf16Le);
    let parts = [
        ("Copilot/version.json", utf16(r#"{"version":"1.0.0"}"#)),
        (
            "Copilot/Instructions/version.json",
            utf16(r#"{"version":"1.0.0"}"#),
        ),
        (
            "Copilot/Instructions/instructions.md",
            utf16("Réponds en français."),
        ),
        ("Copilot/settings.json", utf16(r#"{"indexingEnabled":true}"#)),
        (
            "Copilot/examplePrompts.json",
            utf16(r#"{"prompts":["Ventes par mois"]}"#),
        ),
        (
            "Copilot/VerifiedAnswers/ResourcePackages/p.bin",
            vec![0, 159, 146, 150],
        ),
    ];
    for (name, data) in &parts {
        pkg.put(name, data.clone(), false);
    }
    let src = dir.join("Ai.pbix");
    write_pkg(&src, &pkg);
    let mut s = open(&src);
    let inspect = run(&mut s, "file.inspect", json!({}));
    let c = &inspect["details"]["copilot"];
    assert_eq!(c["instructions_chars"], 20, "{c}");
    assert_eq!(c["example_prompts"], 1);
    assert_eq!(c["indexing_enabled"], true);
    assert_eq!(c["files"].as_array().unwrap().len(), parts.len());

    let out = run(&mut s, "file.save_as", json!({"path": dir.join("Ai.pbip")}));
    let warnings = out["warnings"].to_string();
    assert!(!warnings.contains("Copilot"), "{warnings}");
    let md = dir.join("Ai.SemanticModel/Copilot/Instructions/instructions.md");
    assert_eq!(std::fs::read(&md).unwrap(), "Réponds en français.".as_bytes());
    let raw = dir.join("Ai.SemanticModel/Copilot/VerifiedAnswers/ResourcePackages/p.bin");
    assert_eq!(std::fs::read(&raw).unwrap(), vec![0, 159, 146, 150]);
    let inspect = run(&mut s, "file.inspect", json!({}));
    assert_eq!(inspect["details"]["copilot"]["instructions_chars"], 20);

    run(&mut s, "file.save_as", json!({"path": dir.join("Back.pbix")}));
    let back = bicraft_opc::Package::open(&dir.join("Back.pbix")).unwrap();
    for (name, data) in &parts {
        assert_eq!(&back.get(name).unwrap().data, data, "{name}");
    }
    let version = back.get("Version").unwrap();
    assert_eq!(
        bicraft_opc::decode_text("Version", &version.data).unwrap().0,
        "1.36"
    );
}
