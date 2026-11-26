//! Tests of undo/redo (`edit.*`) and atomic batches (`history.rs`).

use bicraft_model::SemanticModel;
use bicraft_report::Report;
use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::{HistoryLimits, Session, Source};

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn temp(name: &str) -> TempDir {
    TempDir::new(&format!("engine-history-{name}"))
}

/// Everything an undo must restore.
#[derive(Debug, Clone, PartialEq)]
struct Shot {
    model: Option<SemanticModel>,
    report: Option<Report>,
    model_error: Option<String>,
    report_error: Option<String>,
    dirty: bool,
    report_dirty: bool,
}

fn shot(s: &Session) -> Shot {
    Shot {
        model: s.model.clone(),
        report: s.report.clone(),
        model_error: s.model_error.clone(),
        report_error: s.report_error.clone(),
        dirty: s.dirty,
        report_dirty: s.report_dirty,
    }
}

fn undo_count(s: &mut Session) -> usize {
    run(s, "edit.history", json!({}))["undo"]
        .as_array()
        .unwrap()
        .len()
}

/// Sales(Amount, Product), Product(Product, Name), [Total] and a blank report.
fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        ("table.add", json!({"name": "Sales"})),
        ("table.add", json!({"name": "Product"})),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
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
            json!({"table": "Product", "name": "Name", "data_type": "string"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
    ] {
        run(&mut s, id, p);
    }
    s
}

/// Runs a step, then checks that undo restores the state before it exactly, redo the state after
/// it, and that both bump the revision. Returns the step's result.
fn step(s: &mut Session, id: &str, params: Value) -> Value {
    let before = shot(s);
    let v = run(s, id, params.clone());
    let after = shot(s);
    assert_ne!(before, after, "{id} {params} changed nothing");
    let rev = s.revision;
    let u = run(s, "edit.undo", json!({}));
    assert_eq!(u["undone"][0]["commands"], json!([id]), "{u}");
    assert_eq!(shot(s), before, "undo of {id} {params}");
    assert_eq!(s.revision, rev + 1);
    run(s, "edit.redo", json!({}));
    assert_eq!(shot(s), after, "redo of {id} {params}");
    assert_eq!(s.revision, rev + 2);
    v
}

#[test]
fn every_edit_kind_undoes_exactly() {
    let mut s = session();
    let start = shot(&s);
    let base = undo_count(&mut s);

    // Model.
    step(
        &mut s,
        "table.add",
        json!({"name": "Region", "partition": {"source_type": "m", "expression": "let x = 1 in x"}}),
    );
    step(
        &mut s,
        "table.set",
        json!({"name": "Region", "description": "Regions"}),
    );
    step(
        &mut s,
        "table.rename",
        json!({"name": "Region", "new_name": "Area"}),
    );
    step(
        &mut s,
        "column.add",
        json!({"table": "Area", "name": "Code", "data_type": "string"}),
    );
    step(
        &mut s,
        "column.set",
        json!({"table": "Area", "name": "Code", "description": "ISO code", "hidden": true}),
    );
    step(
        &mut s,
        "column.add",
        json!({"table": "Sales", "name": "Double", "data_type": "double", "expression": "Sales[Amount] * 2"}),
    );
    step(
        &mut s,
        "column.remove",
        json!({"table": "Sales", "name": "Double"}),
    );
    step(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Twice", "expression": "[Total] * 2", "format_string": "0.00"}),
    );
    step(
        &mut s,
        "measure.set",
        json!({"name": "Twice", "expression": "[Total] * 3", "display_folder": "KPI"}),
    );
    step(
        &mut s,
        "measure.move",
        json!({"name": "Twice", "to_table": "Product"}),
    );
    step(&mut s, "measure.remove", json!({"name": "Twice"}));
    let rel = step(
        &mut s,
        "relationship.add",
        json!({"from": "Sales[Product]", "to": "Product[Product]"}),
    );
    let rel_name = rel["name"].as_str().unwrap_or_default().to_string();
    step(&mut s, "role.add", json!({"name": "Readers"}));
    step(
        &mut s,
        "role.set_filter",
        json!({"role": "Readers", "table": "Product", "filter_expression": "Product[Name] = \"A\""}),
    );
    step(&mut s, "role.remove", json!({"name": "Readers"}));

    // Report.
    let page = step(&mut s, "page.add", json!({"display_name": "Details"}))["page"]
        .as_str()
        .unwrap()
        .to_string();
    step(
        &mut s,
        "page.rename",
        json!({"page": page, "display_name": "More"}),
    );
    step(&mut s, "page.move", json!({"page": page, "index": 0}));
    step(&mut s, "page.set_active", json!({"page": page}));
    let visual = step(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "clusteredColumnChart", "x": 10, "y": 20, "width": 400,
               "height": 300, "fields": [{"role": "Category", "field": "Product[Name]"}]}),
    )["visual"]
        .as_str()
        .unwrap()
        .to_string();
    step(
        &mut s,
        "visual.bind",
        json!({"page": page, "visual": visual, "role": "Y", "field": "[Total]"}),
    );
    step(
        &mut s,
        "visual.set_position",
        json!({"page": page, "visual": visual, "x": 50}),
    );
    step(
        &mut s,
        "visual.set_title",
        json!({"page": page, "visual": visual, "title": "Sales by product"}),
    );
    step(
        &mut s,
        "visual.format",
        json!({"page": page, "visual": visual, "background": {"show": true, "color": "#EEEEEE"}}),
    );
    step(
        &mut s,
        "visual.unbind",
        json!({"page": page, "visual": visual, "role": "Y"}),
    );
    step(&mut s, "theme.set_colors", json!({"data_colors": ["#123456"]}));
    step(&mut s, "visual.remove", json!({"page": page, "visual": visual}));
    step(&mut s, "page.remove", json!({"page": page}));

    // A model edit that rewrites the report (rename with update_report).
    let first = s.report.as_ref().unwrap().pages[0].name.clone();
    step(
        &mut s,
        "visual.add",
        json!({"page": first, "visual_type": "card",
               "x": 0, "y": 0, "width": 200, "height": 100, "fields": [{"role": "Values", "field": "[Total]"}]}),
    );
    step(
        &mut s,
        "measure.set",
        json!({"name": "Total", "new_name": "Revenue", "update_report": true}),
    );
    step(&mut s, "relationship.remove", json!({"name": rel_name}));
    step(&mut s, "table.remove", json!({"name": "Area"}));

    // All the way back, and forward again.
    let end = shot(&s);
    let steps = undo_count(&mut s) - base;
    let u = run(&mut s, "edit.undo", json!({"steps": steps}));
    assert_eq!(u["undone"].as_array().unwrap().len(), steps);
    assert_eq!(shot(&s), start);
    let r = run(&mut s, "edit.redo", json!({"steps": 10_000}));
    assert_eq!(r["redone"].as_array().unwrap().len(), steps);
    assert_eq!(r["can_redo"], false);
    assert_eq!(shot(&s), end);
}

#[test]
fn history_lists_steps_and_rejects_bad_params() {
    let mut s = session();
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "A", "expression": "1"}),
    );
    let h = run(&mut s, "edit.history", json!({}));
    assert_eq!(h["undo"][0]["commands"], json!(["measure.add"]));
    assert_eq!(h["undo"][0]["label"], "Add a measure");
    assert_eq!(h["can_redo"], false);
    assert!(h["bytes"].as_u64().unwrap() > 0);
    assert_eq!(h["limits"]["max_steps"], 100);
    // Reading the history is not a change.
    let rev = s.revision;
    run(&mut s, "edit.history", json!({}));
    assert_eq!(s.revision, rev);

    for bad in [
        json!({"steps": 0}),
        json!({"steps": -1}),
        json!({"steps": "2"}),
        json!({"x": 1}),
    ] {
        assert!(s.execute("edit.undo", &bad).is_err(), "{bad}");
    }
    assert!(s.execute("edit.history", &json!({"x": 1})).is_err());
    assert_eq!(s.execute("edit.redo", &json!({})).unwrap_err(), "nothing to redo");
    // A failed undo/redo does not bump the revision.
    assert_eq!(s.revision, rev);
}

#[test]
fn a_new_edit_clears_redo_and_failed_commands_are_not_steps() {
    let mut s = session();
    let n = undo_count(&mut s);
    run(&mut s, "page.add", json!({"display_name": "A"}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "page.add", json!({"display_name": "B"}));
    assert!(s.execute("edit.redo", &json!({})).is_err());
    assert!(
        s.execute(
            "measure.add",
            &json!({"table": "Nope", "name": "X", "expression": "1"})
        )
        .is_err()
    );
    // Read-only commands and commands that change nothing are not steps either.
    run(&mut s, "model.summary", json!({}));
    run(&mut s, "theme.remove", json!({}));
    assert_eq!(undo_count(&mut s), n + 1);
}

#[test]
fn open_and_new_clear_the_history() {
    let mut s = session();
    assert!(undo_count(&mut s) > 0);
    run(&mut s, "model.new", json!({"name": "Other"}));
    assert_eq!(undo_count(&mut s), 0);
    assert_eq!(s.execute("edit.undo", &json!({})).unwrap_err(), "nothing to undo");

    let dir = temp("open");
    let bim = dir.join("model.bim");
    std::fs::write(&bim, r#"{"name":"M","model":{"tables":[{"name":"T"}]}}"#).unwrap();
    run(&mut s, "table.add", json!({"name": "T2"}));
    assert_eq!(undo_count(&mut s), 1);
    run(&mut s, "file.open", json!({"path": bim}));
    assert_eq!(undo_count(&mut s), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn step_cap_drops_the_oldest_steps() {
    let mut s = session();
    s.set_history_limits(HistoryLimits {
        max_steps: 3,
        max_bytes: usize::MAX,
    });
    assert_eq!(undo_count(&mut s), 3);
    for i in 0..5 {
        run(
            &mut s,
            "measure.add",
            json!({"table": "Sales", "name": format!("M{i}"), "expression": "1"}),
        );
    }
    let u = run(&mut s, "edit.undo", json!({"steps": 10}));
    assert_eq!(u["undone"].as_array().unwrap().len(), 3);
    let m = s.model.as_ref().unwrap();
    let names: Vec<_> = m.tables[0].measures.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["Total", "M0", "M1"]);
    assert!(s.execute("edit.undo", &json!({})).is_err());
}

#[test]
fn byte_cap_drops_the_oldest_steps() {
    let mut s = session();
    for i in 0..5 {
        run(
            &mut s,
            "measure.add",
            json!({"table": "Sales", "name": format!("M{i}"), "expression": "1"}),
        );
    }
    let h = run(&mut s, "edit.history", json!({}));
    let full = usize::try_from(h["bytes"].as_u64().unwrap()).unwrap();
    let steps = h["undo"].as_array().unwrap().len();
    // Half the memory keeps fewer steps, within the cap.
    s.set_history_limits(HistoryLimits {
        max_steps: 100,
        max_bytes: full / 2,
    });
    let h = run(&mut s, "edit.history", json!({}));
    let kept = h["undo"].as_array().unwrap().len();
    assert!(kept < steps && kept > 0, "{kept} of {steps}");
    assert!(usize::try_from(h["bytes"].as_u64().unwrap()).unwrap() <= full / 2);
    // A cap smaller than the session itself keeps no step at all.
    s.set_history_limits(HistoryLimits {
        max_steps: 100,
        max_bytes: 1,
    });
    run(&mut s, "page.add", json!({"display_name": "X"}));
    assert_eq!(undo_count(&mut s), 0);
    // max_steps 0 disables the history.
    s.set_history_limits(HistoryLimits {
        max_steps: 0,
        max_bytes: usize::MAX,
    });
    run(&mut s, "page.add", json!({"display_name": "Y"}));
    assert_eq!(undo_count(&mut s), 0);
}

#[test]
fn dirty_flags_follow_saves() {
    let dir = temp("save");
    let mut s = session();
    run(&mut s, "project.new", json!({"dir": dir.path(), "name": "Shop"}));
    assert!(!s.dirty && !s.report_dirty);
    let saved_path = match &s.source {
        Some(Source::Project { path, .. }) => path.clone(),
        other => panic!("{other:?}"),
    };

    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "A", "expression": "1"}),
    );
    assert!(s.dirty && !s.report_dirty);
    run(&mut s, "edit.undo", json!({}));
    assert!(!s.dirty, "back to the saved model");
    run(&mut s, "edit.redo", json!({}));
    assert!(s.dirty);
    run(&mut s, "file.save", json!({}));
    assert!(!s.dirty);
    // The save is not a step: undo goes back past it, and the model now differs from the disk.
    let u = run(&mut s, "edit.undo", json!({}));
    assert_eq!(u["undone"][0]["commands"], json!(["measure.add"]));
    assert!(u["dirty"].as_bool().unwrap() && s.dirty);
    run(&mut s, "edit.redo", json!({}));
    assert!(!s.dirty, "redo returns to the saved state");

    // A report-only edit leaves the model clean.
    run(&mut s, "page.add", json!({"display_name": "P"}));
    assert!(!s.dirty && s.report_dirty);
    run(&mut s, "edit.undo", json!({}));
    assert!(!s.dirty && !s.report_dirty);
    // The open file is never undone.
    match &s.source {
        Some(Source::Project { path, .. }) => assert_eq!(path, &saved_path),
        other => panic!("{other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn atomic_batch_rolls_back_and_is_one_step() {
    let mut s = session();
    let before = shot(&s);
    let n = undo_count(&mut s);
    let rev = s.revision;
    let steps = vec![
        (
            "measure.add".to_string(),
            json!({"table": "Sales", "name": "A", "expression": "1"}),
        ),
        ("page.add".to_string(), json!({"display_name": "P"})),
        ("model.summary".to_string(), json!({})),
        (
            "measure.add".to_string(),
            json!({"table": "Nope", "name": "B", "expression": "1"}),
        ),
        ("page.add".to_string(), json!({"display_name": "Q"})),
    ];
    let (i, e, results) = s.batch_atomic(&steps).unwrap_err();
    assert_eq!(i, 3);
    assert!(e.contains("Nope"), "{e}");
    assert_eq!(s.last_command.as_ref().map(|c| c.ok), Some(false));
    assert_eq!(results.len(), 3);
    assert_eq!(shot(&s), before);
    assert_eq!(undo_count(&mut s), n);
    // Steps changed the session and the rollback changed it back: watchers must refresh.
    assert!(s.revision > rev);

    // Without the failing step, the batch commits as one undo step.
    let ok: Vec<_> = steps
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 3)
        .map(|(_, s)| s.clone())
        .collect();
    let results = s.batch_atomic(&ok).unwrap();
    assert_eq!(results.len(), 4);
    let after = shot(&s);
    let h = run(&mut s, "edit.history", json!({}));
    assert_eq!(h["undo"].as_array().unwrap().len(), n + 1);
    assert_eq!(h["undo"][0]["label"], "Batch of 3 commands");
    assert_eq!(
        h["undo"][0]["commands"],
        json!(["measure.add", "page.add", "page.add"])
    );
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(shot(&s), before);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(shot(&s), after);

    // A one-command batch is labelled like the command; a batch that changes nothing adds no step.
    s.batch_atomic(&[("page.add".to_string(), json!({"display_name": "R"}))])
        .unwrap();
    assert_eq!(
        run(&mut s, "edit.history", json!({}))["undo"][0]["label"],
        bicraft_engine::find_command("page.add").unwrap().label
    );
    let n = undo_count(&mut s);
    s.batch_atomic(&[("model.summary".to_string(), json!({}))])
        .unwrap();
    assert_eq!(undo_count(&mut s), n);

    // Undo inside an atomic batch is refused, which rolls the batch back.
    let before = shot(&s);
    let (i, e, _) = s
        .batch_atomic(&[
            ("page.add".to_string(), json!({"display_name": "S"})),
            ("edit.undo".to_string(), json!({})),
        ])
        .unwrap_err();
    assert_eq!(i, 1);
    assert!(e.contains("atomic"), "{e}");
    assert_eq!(shot(&s), before);
}

#[test]
fn non_atomic_batch_keeps_earlier_steps() {
    let mut s = session();
    let n = undo_count(&mut s);
    let (i, _, _) = s
        .batch(&[
            ("page.add".to_string(), json!({"display_name": "P"})),
            ("nope.nope".to_string(), json!({})),
        ])
        .unwrap_err();
    assert_eq!(i, 1);
    assert_eq!(s.report.as_ref().unwrap().pages.len(), 2);
    assert_eq!(undo_count(&mut s), n + 1);
}

#[test]
fn atomic_batch_restores_the_open_file_and_history() {
    let dir = temp("tx-open");
    let a = dir.join("a.bim");
    let b = dir.join("b.bim");
    std::fs::write(&a, r#"{"name":"A","model":{"tables":[{"name":"T"}]}}"#).unwrap();
    std::fs::write(&b, r#"{"name":"B","model":{"tables":[{"name":"U"}]}}"#).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": a}));
    run(&mut s, "table.add", json!({"name": "T2"}));
    let before = shot(&s);
    assert!(s.dirty);

    let (i, _, _) = s
        .batch_atomic(&[
            ("file.open".to_string(), json!({"path": b})),
            ("table.add".to_string(), json!({"name": "U2"})),
            ("table.add".to_string(), json!({"name": "U2"})),
        ])
        .unwrap_err();
    assert_eq!(i, 2);
    assert_eq!(shot(&s), before);
    match &s.source {
        Some(Source::Bim { path }) => assert_eq!(path, &a),
        other => panic!("{other:?}"),
    }
    assert_eq!(undo_count(&mut s), 1, "history survives the rolled-back open");
    run(&mut s, "edit.undo", json!({}));
    assert!(!s.dirty, "a.bim as opened");

    // Committed: the history restarts at the open, with the edits after it as one step.
    run(&mut s, "edit.redo", json!({}));
    s.batch_atomic(&[
        ("file.open".to_string(), json!({"path": b})),
        ("table.add".to_string(), json!({"name": "U2"})),
    ])
    .unwrap();
    let h = run(&mut s, "edit.history", json!({}));
    assert_eq!(h["undo"].as_array().unwrap().len(), 1);
    assert_eq!(h["undo"][0]["commands"], json!(["table.add"]));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.model.as_ref().unwrap().name, "B");
    assert_eq!(s.model.as_ref().unwrap().tables.len(), 1);
    assert!(!s.dirty);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn atomic_batch_after_a_save_marks_the_session_dirty() {
    let dir = temp("tx-save");
    let mut s = session();
    run(&mut s, "project.new", json!({"dir": dir.path(), "name": "Shop"}));
    let before = shot(&s);
    let (i, _, _) = s
        .batch_atomic(&[
            (
                "measure.add".to_string(),
                json!({"table": "Sales", "name": "A", "expression": "1"}),
            ),
            ("file.save".to_string(), json!({})),
            ("nope.nope".to_string(), json!({})),
        ])
        .unwrap_err();
    assert_eq!(i, 2);
    // The model is back, but the disk holds the batch's save: the session is dirty.
    assert_eq!(s.model, before.model);
    assert!(s.dirty);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn history_works_with_the_session_changed_outside_commands() {
    let mut s = session();
    let mut m = s.model.clone().unwrap();
    m.description = Some("set directly".into());
    s.model = Some(m);
    let direct = shot(&s);
    run(&mut s, "page.add", json!({"display_name": "P"}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(shot(&s).model, direct.model);
}

/// Clone, compare and size costs of history snapshots on a 10 000-measure model and, with
/// `BICRAFT_TESTDATA`, on the local real files. Run with
/// `cargo test -p bicraft-engine --release history_costs -- --ignored --nocapture`.
#[test]
#[ignore = "measurement, prints timings"]
fn history_costs() {
    use std::time::Instant;
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Big"}));
    let mut m = s.model.clone().unwrap();
    for t in 0..100 {
        let mut table = bicraft_model::Table {
            name: format!("T{t}"),
            ..Default::default()
        };
        for c in 0..10 {
            table.columns.push(bicraft_model::Column {
                name: format!("C{c}"),
                ..Default::default()
            });
        }
        for i in 0..100 {
            table.measures.push(bicraft_model::Measure {
                name: format!("M{t}_{i}"),
                expression: format!("CALCULATE(SUM('T{t}'[C1]), 'T{t}'[C2] = {i}) * 1.5"),
                format_string: Some("#,0.00".into()),
                display_folder: Some("Folder".into()),
                ..Default::default()
            });
        }
        m.tables.push(table);
    }
    s.model = Some(m);
    measure(&mut s, "10k measures");

    if let Some(dir) = bicraft_env::var_os("BICRAFT_TESTDATA") {
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .map(|r| r.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        files.sort();
        for path in files {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !matches!(ext, "pbix" | "pbit") {
                continue;
            }
            let mut s = Session::new();
            if s.execute("file.open", &json!({"path": path})).is_err() {
                continue;
            }
            if s.report.is_none() {
                continue;
            }
            measure(&mut s, &format!("real {ext} file"));
        }
    } else {
        eprintln!("BICRAFT_TESTDATA not set: real files skipped");
    }

    fn measure(s: &mut Session, what: &str) {
        let t = Instant::now();
        let model = s.model.clone();
        let report = s.report.clone();
        let clone = t.elapsed();
        let t = Instant::now();
        let same = model == s.model && report == s.report;
        let compare = t.elapsed();
        assert!(same);
        let measures = s
            .model
            .as_ref()
            .map_or(0, |m| m.tables.iter().map(|t| t.measures.len()).sum::<usize>());
        let visuals = s
            .report
            .as_ref()
            .map_or(0, |r| r.pages.iter().map(|p| p.visuals.len()).sum::<usize>());
        // Warm up: the first command after a direct change snapshots the whole session.
        run(s, "edit.history", json!({}));
        if s.report.is_some() {
            run(s, "page.add", json!({"display_name": "Warmup"}));
        } else {
            run(s, "model.summary", json!({}));
            let table = s
                .model
                .as_ref()
                .and_then(|m| m.tables.first())
                .map(|t| t.name.clone());
            if let Some(table) = table {
                run(
                    s,
                    "measure.add",
                    json!({"table": table, "name": "Warmup", "expression": "1", "force": true}),
                );
            }
        }
        // A model edit, then a report edit, as the session records them.
        let t = Instant::now();
        if let Some(table) = s
            .model
            .as_ref()
            .and_then(|m| m.tables.first())
            .map(|t| t.name.clone())
        {
            run(
                s,
                "measure.add",
                json!({"table": table, "name": "HistoryCost", "expression": "1", "force": true}),
            );
        }
        let model_step = t.elapsed();
        let t = Instant::now();
        if s.report.is_some() {
            run(s, "page.add", json!({"display_name": "HistoryCost"}));
        }
        let report_step = t.elapsed();
        let t = Instant::now();
        let _ = s.execute("edit.undo", &json!({"steps": 2}));
        let undo = t.elapsed();
        let h = run(s, "edit.history", json!({}));
        // The same edits without history, for the overhead.
        let limits = s.history_limits();
        s.set_history_limits(HistoryLimits {
            max_steps: 0,
            ..limits
        });
        let t = Instant::now();
        if let Some(table) = s
            .model
            .as_ref()
            .and_then(|m| m.tables.first())
            .map(|t| t.name.clone())
        {
            run(
                s,
                "measure.add",
                json!({"table": table, "name": "NoHistory", "expression": "1", "force": true}),
            );
        }
        let model_plain = t.elapsed();
        let t = Instant::now();
        if s.report.is_some() {
            run(s, "page.add", json!({"display_name": "NoHistory"}));
        }
        let report_plain = t.elapsed();
        s.set_history_limits(limits);
        eprintln!(
            "{what}: {measures} measures, {visuals} visuals; clone {clone:?}, compare {compare:?}; \
             measure.add {model_step:?} ({model_plain:?} without history), page.add {report_step:?} \
             ({report_plain:?} without history), undo x2 {undo:?}; history bytes {} (redo {})",
            h["bytes"],
            h["redo"].as_array().map_or(0, Vec::len)
        );
    }
}
