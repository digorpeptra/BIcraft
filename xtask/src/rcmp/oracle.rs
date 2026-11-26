//! The Desktop oracle as a test: for every `tools/render-oracle/floors/<set>.json`, renders the
//! local reference report `%BICRAFT_TESTDATA%/render-oracle/<set>/report.pbix` with the current
//! renderer and compares it with Desktop's captures there (`desktop/`, `layout.json`, optional
//! `render.json` = extra `report.render` parameters such as `ui_language`). The references are
//! Microsoft-rendered images, so they stay local; without them (or without Segoe UI, which the
//! floors assume) the test prints a note and passes. Our own goldens
//! (crates/render/tests/golden) remain the Linux CI guard.

use std::path::PathBuf;

use bicraft_engine::Session;
use serde_json::{Value, json};

use super::{Inputs, check, compare, read_json, table};

#[test]
fn desktop_render_oracle() {
    let Some(td) = bicraft_env::var_os("BICRAFT_TESTDATA").map(PathBuf::from) else {
        eprintln!("note: BICRAFT_TESTDATA is not set; render oracle skipped");
        return;
    };
    let floors_dir = crate::root().join("tools").join("render-oracle").join("floors");
    let mut sets: Vec<PathBuf> = std::fs::read_dir(&floors_dir)
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    sets.retain(|p| p.extension().is_some_and(|x| x == "json"));
    sets.sort();
    let mut failures = Vec::new();
    for floors_path in sets {
        let Some(set) = floors_path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let dir = td.join("render-oracle").join(set);
        let (pbix, layout, desktop) = (
            dir.join("report.pbix"),
            dir.join("layout.json"),
            dir.join("desktop"),
        );
        if !pbix.is_file() || !layout.is_file() || !desktop.is_dir() {
            eprintln!("note: {} has no references; set `{set}` skipped", dir.display());
            continue;
        }
        let floors = read_json(&floors_path).expect("floors JSON");
        let ours = std::env::temp_dir().join(format!("bicraft-oracle-{}-{set}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ours);
        let mut s = Session::new();
        s.execute("file.open", &json!({"path": pbix.to_string_lossy()}))
            .expect("open the reference report");
        let mut params = read_json(&dir.join("render.json")).unwrap_or_else(|| json!({}));
        params["out"] = json!(ours.to_string_lossy());
        params["text_dump"] = Value::Bool(true);
        let r = s.execute("report.render", &params).expect("render");
        if r.get("font").and_then(Value::as_str) != Some("Segoe UI") {
            eprintln!("note: Segoe UI is not installed; set `{set}` skipped (floors assume it)");
            let _ = std::fs::remove_dir_all(&ours);
            continue;
        }
        let rows = compare(&Inputs {
            desktop: &desktop,
            ours: &ours,
            layout: &layout,
            diff: None,
        })
        .expect("compare");
        let _ = std::fs::remove_dir_all(&ours);
        eprintln!("== render oracle `{set}` ==\n{}", table(&rows));
        assert!(!rows.is_empty(), "set `{set}`: no visual compared");
        failures.extend(
            check(&rows, Some(&floors), None)
                .into_iter()
                .map(|f| format!("{set}: {f}")),
        );
    }
    assert!(
        failures.is_empty(),
        "render oracle regressions:\n{}",
        failures.join("\n")
    );
}
