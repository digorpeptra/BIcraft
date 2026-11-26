//! Tests of `file.diff` and `file.textconv` on generated files: the same model in every format
//! has no differences, edits saved through the engine are reported, hostile files are errors.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

const SYNTHETIC_METADATA: &[u8] =
    include_bytes!("../../../metadata/tests/fixtures/synthetic_metadata.sqlitedb");

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

fn convert(from: &Path, to: &Path, extra: Value) {
    let mut p = json!({"from": from, "to": to});
    if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    run(&mut Session::new(), "file.convert", p);
}

fn diff(a: &Path, b: &Path, extra: Value) -> Value {
    let mut p = json!({"a": a, "b": b, "format": "text"});
    if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    run(&mut Session::new(), "file.diff", p)
}

fn text(d: &Value) -> &str {
    d["text"].as_str().unwrap_or("")
}

#[test]
fn the_same_model_in_every_format_has_no_differences() {
    let dir = TempDir::new("engine-diff-formats");
    for (src, report_format) in [(legacy_pbix(&dir), "keep"), (pbir_pbix(&dir), "pbir")] {
        let stem = src.file_stem().unwrap().to_string_lossy().into_owned();
        let tmdl = dir.join(format!("{stem}Tmdl/{stem}.pbip"));
        let tmsl = dir.join(format!("{stem}Tmsl/{stem}.pbip"));
        let pbit = dir.join(format!("{stem}.pbit"));
        let bim = dir.join(format!("{stem}.bim"));
        let pbir = dir.join(format!("{stem}Pbir/{stem}.pbip"));
        convert(&src, &tmdl, json!({"report_format": report_format}));
        convert(&src, &tmsl, json!({"format": "pbip-tmsl"}));
        convert(&src, &pbit, json!({}));
        convert(&src, &bim, json!({}));
        convert(&src, &pbir, json!({"report_format": "pbir"}));
        for other in [&tmdl, &tmsl, &pbit, &bim, &pbir] {
            let d = diff(&src, other, json!({}));
            assert_eq!(
                d["identical"],
                true,
                "{} vs {}:\n{}",
                src.display(),
                other.display(),
                text(&d)
            );
            let back = diff(other, &src, json!({}));
            assert_eq!(back["identical"], true, "{}", text(&back));
        }
        // across two converted forms too
        let d = diff(&tmdl, &tmsl, json!({}));
        assert_eq!(d["identical"], true, "{}", text(&d));
        // the model is the same in canonical text form
        let dump = |p: &Path| {
            run(&mut Session::new(), "file.textconv", json!({"file": p}))["text"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let model_part = |t: String| t.split("\nreport\n").next().unwrap_or("").trim_end().to_string();
        same_text(&model_part(dump(&src)), &model_part(dump(&tmdl)));
        same_text(&model_part(dump(&src)), &model_part(dump(&bim)));
    }
}

#[test]
fn edits_are_reported_with_renames_and_options() {
    let dir = TempDir::new("engine-diff-edits");
    let src = pbir_pbix(&dir);
    let before = dir.join("Before/Sales.pbip");
    let after = dir.join("After/Sales.pbip");
    convert(&src, &before, json!({}));
    convert(&src, &after, json!({}));
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": after}));
    let model = s.model.clone().unwrap();
    let (table, m) = model
        .tables
        .iter()
        .find_map(|t| t.measures.first().map(|m| (t.name.clone(), m.clone())))
        .expect("the synthetic model has a measure");
    // a cosmetic change, then a rename: the lineage tag (or the expression) keeps the identity
    run(
        &mut s,
        "measure.set",
        json!({"name": m.name, "expression": format!("{}  // reviewed", m.expression), "force": true}),
    );
    run(
        &mut s,
        "measure.rename",
        json!({"name": m.name, "new_name": "Renamed measure"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": table, "name": "Added measure", "expression": "1", "force": true}),
    );
    let page = s.report.as_ref().unwrap().pages[0].display_name.clone();
    run(
        &mut s,
        "page.rename",
        json!({"page": page, "display_name": "Renamed page"}),
    );
    run(&mut s, "file.save", json!({}));

    let d = diff(&before, &after, json!({}));
    let t = text(&d).to_string();
    assert_eq!(d["identical"], false);
    let changes = d["changes"].as_array().unwrap();
    let renamed = changes
        .iter()
        .find(|c| c["object"] == "measure" && c["change"] == "renamed")
        .unwrap_or_else(|| panic!("{t}"));
    assert!(renamed["id"].as_str().unwrap().contains("Renamed measure"), "{t}");
    assert!(renamed["properties"][0]["cosmetic"] == true, "{t}");
    assert!(
        changes
            .iter()
            .any(|c| c["object"] == "measure" && c["change"] == "added"),
        "{t}"
    );
    assert!(
        changes
            .iter()
            .any(|c| c["object"] == "page" && c["change"] == "renamed"),
        "{t}"
    );
    assert!(t.contains("renamed from"), "{t}");
    // options: model only, whitespace ignored
    let m_only = diff(
        &before,
        &after,
        json!({"scope": "model", "ignore_whitespace": true, "format": "markdown"}),
    );
    let md = m_only["markdown"].as_str().unwrap();
    assert!(md.contains("#### Model") && !md.contains("#### Report"), "{md}");
    assert!(!md.contains("whitespace, comments or case only"), "{md}");
    // a deleted file: everything removed
    let gone = run(&mut Session::new(), "file.diff", json!({"a": before, "b": null}));
    assert!(gone["summary"]["removed"].as_u64().unwrap() > 0, "{gone}");
    assert_eq!(gone["b"]["kind"], "none");
}

#[test]
fn pbix_data_model_is_compared_by_stamp_not_decompressed() {
    let dir = TempDir::new("engine-diff-stamp");
    let a = legacy_pbix(&dir);
    let d = run(&mut Session::new(), "file.diff", json!({"a": a, "b": a}));
    assert_eq!(d["identical"], true);
    assert_eq!(d["data"]["same"], true, "{d}");
    assert_eq!(d["a"]["kind"], "pbix");
    assert_eq!(d["a"]["report_format"], "legacy");
}

#[test]
fn hostile_files_are_errors_not_crashes() {
    let dir = TempDir::new("engine-diff-hostile");
    let good = legacy_pbix(&dir);
    let bytes = std::fs::read(&good).unwrap();
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty.pbix", Vec::new()),
        ("garbage.pbix", b"PK\x03\x04 not really a zip".to_vec()),
        ("truncated.pbix", bytes.get(..bytes.len() / 2).unwrap().to_vec()),
        ("noext", b"PK\x03\x04".to_vec()),
    ];
    for (name, data) in cases {
        let p = dir.join(name);
        std::fs::write(&p, data).unwrap();
        let r = Session::new().execute("file.diff", &json!({"a": good, "b": p}));
        assert!(r.is_err(), "{name}: {r:?}");
        let r = Session::new().execute("file.textconv", &json!({"file": p}));
        assert!(r.is_err(), "{name}: {r:?}");
    }
    // a corrupted DataModel inside a valid package
    let mut pkg = bicraft_opc::Package::open(&good).unwrap();
    for part in &mut pkg.parts {
        if part.name == bicraft_pbix::DATA_MODEL {
            for b in part.data.iter_mut().skip(64).step_by(7) {
                *b ^= 0x5A;
            }
        }
    }
    let bad = dir.join("bad-model.pbix");
    write_pkg(&bad, &pkg);
    let r = Session::new().execute("file.diff", &json!({"a": good, "b": bad}));
    assert!(r.is_err(), "{r:?}");
    // parameters
    let e = Session::new()
        .execute("file.diff", &json!({"a": good}))
        .unwrap_err();
    assert!(e.contains("`a` and `b`"), "{e}");
    let e = Session::new()
        .execute("file.diff", &json!({"a": good, "b": good, "scope": "everything"}))
        .unwrap_err();
    assert!(e.contains("scope"), "{e}");
}

#[test]
fn server_mode_confines_both_sides() {
    let dir = TempDir::new("engine-diff-server");
    let inside = legacy_pbix(&dir);
    let outside = TempDir::new("engine-diff-server-out");
    let other = legacy_pbix(&outside);
    let mut s = Session::server(bicraft_engine::ServerConfig::new(dir.path()).unwrap());
    let ok = s
        .execute("file.diff", &json!({"a": "Legacy.pbix", "b": "Legacy.pbix"}))
        .unwrap();
    assert_eq!(ok["identical"], true);
    for (a, b) in [
        (other.to_string_lossy().into_owned(), "Legacy.pbix".to_string()),
        ("Legacy.pbix".into(), "../x.pbix".into()),
    ] {
        assert!(s.execute("file.diff", &json!({"a": a, "b": b})).is_err());
    }
    assert!(s.execute("file.textconv", &json!({"file": other})).is_err());
    let _ = inside;
}

#[test]
fn server_answers_show_workspace_paths_only() {
    let dir = TempDir::new("engine-diff-server-labels");
    legacy_pbix(&dir);
    let mut s = Session::server(bicraft_engine::ServerConfig::new(dir.path()).unwrap());
    let d = s
        .execute(
            "file.diff",
            &json!({"a": "Legacy.pbix", "b": "Legacy.pbix", "format": "text"}),
        )
        .unwrap();
    assert_eq!(d["a"]["path"], "Legacy.pbix", "{d}");
    assert!(d["text"].as_str().unwrap().starts_with("a: Legacy.pbix\n"), "{d}");
}

#[test]
fn an_unreadable_report_is_a_note_unless_the_report_was_asked_for() {
    let dir = TempDir::new("engine-diff-bad-report");
    let src = pbir_pbix(&dir);
    let project = dir.join("P/Pbir.pbip");
    convert(&src, &project, json!({}));
    std::fs::write(dir.join("P/Pbir.Report/definition/report.json"), "{ not json").unwrap();
    let d = run(&mut Session::new(), "file.diff", json!({"a": src, "b": project}));
    assert_eq!(d["identical"], true, "{d}");
    assert!(
        d["notes"][0].as_str().unwrap().contains("reports not compared"),
        "{d}"
    );
    let e = Session::new()
        .execute("file.diff", &json!({"a": src, "b": project, "scope": "report"}))
        .unwrap_err();
    assert!(e.contains("report of b"), "{e}");
}

#[test]
fn a_model_bim_is_compared_on_its_model_only() {
    let dir = TempDir::new("engine-diff-bim");
    let src = legacy_pbix(&dir);
    let bim = dir.join("M.bim");
    convert(&src, &bim, json!({}));
    let d = run(&mut Session::new(), "file.diff", json!({"a": src, "b": bim}));
    assert_eq!(d["identical"], true, "{d}");
    assert!(d["notes"][0].as_str().unwrap().contains("holds no report"), "{d}");
    assert!(
        Session::new()
            .execute("file.diff", &json!({"a": src, "b": bim, "scope": "report"}))
            .is_err()
    );
}

/// Equal texts, or a panic naming the first line that differs.
fn same_text(a: &str, b: &str) {
    if a == b {
        return;
    }
    let first = a.lines().zip(b.lines()).position(|(x, y)| x != y);
    match first {
        Some(i) => panic!("line {}: {:?} vs {:?}", i + 1, a.lines().nth(i), b.lines().nth(i)),
        None => panic!("{} lines vs {} lines", a.lines().count(), b.lines().count()),
    }
}
