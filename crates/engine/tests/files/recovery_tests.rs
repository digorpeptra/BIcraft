//! Auto-recovery: journal, replay, copies, base checks (`recovery.rs`, docs/recovery.md).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::*;
use bicraft_engine::{COMMANDS, READ_ONLY_COMMANDS, Session, is_mutating};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// A PBIP project `<dir>/Shop.pbip` with a table, a measure and a report page with a chart.
fn project(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    run(&mut s, "project.new", json!({"dir": dir, "name": "Shop"}));
    run(
        &mut s,
        "table.import",
        json!({"name": "Sales", "csv": "Product,Amount\nA,1\nB,2\nC,5\n"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
    );
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "clusteredColumnChart", "x": 0, "y": 0, "width": 300,
               "height": 200, "fields": [{"role": "Category", "field": "Sales[Product]"},
                                         {"role": "Y", "field": "[Total]"}]}),
    );
    run(&mut s, "file.save", json!({}));
    dir.join("Shop.pbip")
}

fn cfg(dir: &Path) -> RecoveryConfig {
    RecoveryConfig::new(dir.join("recovery"))
}

fn open(cfg: &RecoveryConfig, path: &Path) -> Session {
    let mut s = Session::new();
    s.set_recovery(Some(cfg.clone()));
    run(&mut s, "file.open", json!({"path": path}));
    s
}

fn offers(s: &Session) -> Vec<Value> {
    s.snapshot()
        .pointer("/recovery/offers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn journals(cfg: &RecoveryConfig) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(&cfg.dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// Varied edits: model, report, theme, an atomic batch, undo and redo.
fn edits(s: &mut Session) {
    run(
        s,
        "measure.add",
        json!({"table": "Sales", "name": "Double", "expression": "[Total] * 2", "format_string": "0.0"}),
    );
    run(
        s,
        "column.add",
        json!({"table": "Sales", "name": "Big", "data_type": "boolean", "expression": "Sales[Amount] > 2"}),
    );
    run(
        s,
        "measure.set",
        json!({"table": "Sales", "name": "Total", "description": "All sales"}),
    );
    run(s, "page.add", json!({"display_name": "Second"}));
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let visual = s.report.as_ref().unwrap().pages[0].visuals[0].name.clone();
    run(
        s,
        "visual.set_title",
        json!({"page": page, "visual": visual, "title": "Sales by product"}),
    );
    run(
        s,
        "visual.add",
        json!({"page": page, "visual_type": "card", "x": 320, "y": 0, "width": 200, "height": 100,
               "fields": [{"role": "Values", "field": "[Double]"}]}),
    );
    run(
        s,
        "measure.add",
        json!({"table": "Sales", "name": "Gone", "expression": "1"}),
    );
    run(s, "edit.undo", json!({}));
    run(s, "edit.redo", json!({}));
    run(s, "edit.undo", json!({}));
    s.batch_atomic(&[
        (
            "measure.add".into(),
            json!({"table": "Sales", "name": "Triple", "expression": "[Total] * 3"}),
        ),
        (
            "table.rename".into(),
            json!({"name": "Sales", "new_name": "Orders"}),
        ),
    ])
    .unwrap();
    // a failed command and a failed batch leave no line
    assert!(
        s.execute(
            "measure.add",
            &json!({"table": "Nope", "name": "X", "expression": "1"})
        )
        .is_err()
    );
    assert!(
        s.batch_atomic(&[
            (
                "measure.add".into(),
                json!({"table": "Orders", "name": "Y", "expression": "1"})
            ),
            (
                "measure.add".into(),
                json!({"table": "Nope", "name": "Z", "expression": "1"})
            ),
        ])
        .is_err()
    );
    run(s, "view.set_language", json!({"ui_language": "fr-FR"}));
}

#[test]
fn replay_after_a_crash_gives_the_same_model_and_report() {
    let dir = TempDir::new("recovery-roundtrip");
    let path = project(dir.path());
    let cfg = cfg(dir.path());
    let mut s = open(&cfg, &path);
    edits(&mut s);
    assert!(s.dirty && s.report_dirty);
    let (model, report) = (s.model.clone(), s.report.clone());
    let snap = s.snapshot();
    assert_eq!(
        snap.pointer("/recovery/edits"),
        Some(&json!(12)),
        "{}",
        snap["recovery"]
    );
    let journal = s.recovery_journal().unwrap();
    assert!(journal.starts_with(&cfg.dir));
    assert!(!journal.starts_with(dir.path().join("Shop.SemanticModel")));
    // the crash: the session goes without a save or a close
    drop(s);

    let mut r = open(&cfg, &path);
    let o = offers(&r);
    assert_eq!(o.len(), 1, "{o:?}");
    assert_eq!(o[0]["base"], "same");
    assert_eq!(o[0]["recoverable"], true);
    assert_eq!(o[0]["edits"], 12);
    let out = run(&mut r, "recovery.apply", json!({}));
    assert_eq!(out["complete"], true, "{out}");
    assert_eq!(out["applied"], 12);
    assert_eq!(r.model, model, "the replayed model differs");
    assert_eq!(r.report, report, "the replayed report differs");
    assert!(r.dirty && r.report_dirty);
    // the old journal is gone; the recovered edits are journaled again in r's own
    assert!(!journal.exists());
    assert!(offers(&r).is_empty());
    assert_eq!(r.snapshot().pointer("/recovery/edits"), Some(&json!(12)));
    // each replayed command is its own undo step
    run(&mut r, "edit.undo", json!({}));
    assert!(r.model.as_ref().unwrap().tables.iter().any(|t| t.name == "Sales"));
    // a second crash, a second recovery
    run(&mut r, "edit.redo", json!({}));
    drop(r);
    let mut r2 = open(&cfg, &path);
    let out = run(&mut r2, "recovery.apply", json!({}));
    assert_eq!(out["complete"], true, "{out}");
    assert_eq!(r2.model, model);
    assert_eq!(r2.report, report);
}

#[test]
fn a_save_deletes_the_journal_and_a_clean_session_has_none() {
    let dir = TempDir::new("recovery-save");
    let path = project(dir.path());
    let cfg = cfg(dir.path());
    let mut s = open(&cfg, &path);
    // nothing journaled before an edit
    assert!(s.recovery_journal().is_none());
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "M1", "expression": "1"}),
    );
    let j = s.recovery_journal().unwrap();
    assert!(j.exists());
    run(&mut s, "file.save", json!({}));
    assert!(!j.exists());
    assert!(journals(&cfg).is_empty(), "{:?}", journals(&cfg));
    // an edit undone back to the saved state: clean again, no journal
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "M2", "expression": "2"}),
    );
    assert!(s.recovery_journal().is_some());
    run(&mut s, "edit.undo", json!({}));
    assert!(!s.dirty);
    assert!(s.recovery_journal().is_none());
    assert!(journals(&cfg).is_empty());
    // a clean close
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "M3", "expression": "3"}),
    );
    s.recovery_close();
    drop(s);
    assert!(journals(&cfg).is_empty());
    let s = open(&cfg, &path);
    assert!(offers(&s).is_empty());
}

#[test]
fn undo_past_the_save_writes_a_copy_that_recovers_exactly() {
    let dir = TempDir::new("recovery-undo-save");
    let path = project(dir.path());
    let cfg = cfg(dir.path());
    let mut s = open(&cfg, &path);
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Saved", "expression": "1"}),
    );
    run(&mut s, "file.save", json!({}));
    // the journal starts at the save: undoing the step before it cannot replay from the file
    run(&mut s, "edit.undo", json!({}));
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "After", "expression": "2"}),
    );
    let snap = s.snapshot();
    assert_eq!(
        snap.pointer("/recovery/copy"),
        Some(&json!(true)),
        "{}",
        snap["recovery"]
    );
    assert_eq!(snap.pointer("/recovery/edits"), Some(&json!(1)));
    let model = s.model.clone();
    drop(s);
    let mut r = open(&cfg, &path);
    assert_eq!(offers(&r)[0]["copy"], true);
    let out = run(&mut r, "recovery.apply", json!({}));
    assert_eq!(out["copy"], true, "{out}");
    assert_eq!(out["complete"], true, "{out}");
    assert_eq!(r.model, model);
    let m = r.model.as_ref().unwrap();
    assert!(!m.tables[0].measures.iter().any(|x| x.name == "Saved"));
    assert!(m.tables[0].measures.iter().any(|x| x.name == "After"));
}

#[test]
fn commands_with_outside_inputs_write_a_copy() {
    let dir = TempDir::new("recovery-copy");
    let path = project(dir.path());
    let cfg = cfg(dir.path());
    let mut s = open(&cfg, &path);
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Before", "expression": "1"}),
    );
    run(
        &mut s,
        "table.set_data",
        json!({"name": "Sales", "csv": "Product,Amount\nX,10\nY,20\n"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Later", "expression": "2"}),
    );
    let snap = s.snapshot();
    assert_eq!(snap.pointer("/recovery/copy"), Some(&json!(true)));
    assert_eq!(snap.pointer("/recovery/edits"), Some(&json!(1)));
    let (model, report) = (s.model.clone(), s.report.clone());
    drop(s);
    let mut r = open(&cfg, &path);
    let out = run(&mut r, "recovery.apply", json!({}));
    assert_eq!(out["complete"], true, "{out}");
    assert_eq!(r.model, model);
    assert_eq!(r.report, report);
}

#[test]
fn the_size_cap_falls_back_to_a_copy() {
    let dir = TempDir::new("recovery-cap");
    let path = project(dir.path());
    let cfg = cfg(dir.path()).with_max_bytes(2048);
    let mut s = open(&cfg, &path);
    for i in 0..40 {
        run(
            &mut s,
            "measure.add",
            json!({"table": "Sales", "name": format!("M{i}"), "expression": format!("{i} + [Total]")}),
        );
        let len = std::fs::metadata(s.recovery_journal().unwrap()).unwrap().len();
        assert!(len <= 2048, "journal of {len} bytes after {i} edits");
    }
    assert_eq!(s.snapshot().pointer("/recovery/copy"), Some(&json!(true)));
    let model = s.model.clone();
    drop(s);
    let mut r = open(&cfg, &path);
    let out = run(&mut r, "recovery.apply", json!({}));
    assert_eq!(out["complete"], true, "{out}");
    assert_eq!(r.model, model);
}

/// Changes the saved project from a session of its own.
fn edit_on_disk(path: &Path, name: &str) {
    let mut other = Session::new();
    run(&mut other, "file.open", json!({"path": path}));
    run(
        &mut other,
        "measure.add",
        json!({"table": "Sales", "name": name, "expression": "9"}),
    );
    run(&mut other, "file.save", json!({}));
}

#[test]
fn a_changed_base_is_refused_without_a_copy_and_recovered_from_one() {
    let dir = TempDir::new("recovery-base");
    let path = project(dir.path());
    let cfg = cfg(dir.path());
    let mut s = open(&cfg, &path);
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Mine", "expression": "1"}),
    );
    drop(s);
    edit_on_disk(&path, "Elsewhere");
    let mut r = open(&cfg, &path);
    let o = offers(&r);
    assert_eq!(o.len(), 1);
    assert_eq!(o[0]["base"], "changed");
    assert_eq!(o[0]["recoverable"], false);
    assert!(o[0]["why"].as_str().unwrap().contains("changed"));
    let e = r.execute("recovery.apply", &json!({})).unwrap_err();
    assert!(e.contains("no recoverable"), "{e}");
    let id = o[0]["id"].as_str().unwrap().to_string();
    let e = r.execute("recovery.apply", &json!({"id": id})).unwrap_err();
    assert!(e.contains("changed"), "{e}");
    assert!(!r.dirty);
    let out = run(&mut r, "recovery.discard", json!({}));
    assert_eq!(out["discarded"], 1);
    assert!(offers(&r).is_empty());

    // with a copy, the changed file takes the copy's model and report
    let mut s = open(&cfg, &path);
    run(
        &mut s,
        "table.set_data",
        json!({"name": "Sales", "csv": "Product,Amount\nQ,1\n"}),
    );
    let model = s.model.clone();
    drop(s);
    edit_on_disk(&path, "Elsewhere2");
    let mut r = open(&cfg, &path);
    let o = offers(&r);
    assert_eq!(
        (o[0]["base"].as_str(), o[0]["recoverable"].as_bool()),
        (Some("changed"), Some(true))
    );
    let out = run(&mut r, "recovery.apply", json!({}));
    assert_eq!(out["base"], "changed");
    assert_eq!(r.model, model);
}

#[test]
fn live_sessions_keep_their_journals_and_files_do_not_mix() {
    let dir = TempDir::new("recovery-concurrent");
    let a_dir = dir.path().join("a");
    let b_dir = dir.path().join("b");
    std::fs::create_dir_all(&a_dir).unwrap();
    std::fs::create_dir_all(&b_dir).unwrap();
    let a_path = project(&a_dir);
    let b_path = project(&b_dir);
    let cfg = cfg(dir.path());
    let (mut a, mut b) = (open(&cfg, &a_path), open(&cfg, &b_path));
    let ta = std::thread::spawn(move || {
        for i in 0..20 {
            run(
                &mut a,
                "measure.add",
                json!({"table": "Sales", "name": format!("A{i}"), "expression": "1"}),
            );
        }
        a
    });
    let tb = std::thread::spawn(move || {
        for i in 0..20 {
            run(
                &mut b,
                "measure.add",
                json!({"table": "Sales", "name": format!("B{i}"), "expression": "2"}),
            );
        }
        b
    });
    let (a, b) = (ta.join().unwrap(), tb.join().unwrap());
    // a second window on file a while a lives: a's journal is locked, not offered
    let c = open(&cfg, &a_path);
    assert!(offers(&c).is_empty());
    assert!(recovery_offers(&cfg).is_empty());
    let (ma, mb) = (a.model.clone(), b.model.clone());
    drop(a);
    drop(b);
    drop(c);
    let all = recovery_offers(&cfg);
    assert_eq!(all.len(), 2, "{all:?}");
    let mut ra = open(&cfg, &a_path);
    let mut rb = open(&cfg, &b_path);
    assert_eq!(offers(&ra).len(), 1);
    assert_eq!(offers(&rb).len(), 1);
    run(&mut ra, "recovery.apply", json!({}));
    run(&mut rb, "recovery.apply", json!({}));
    assert_eq!(ra.model, ma);
    assert_eq!(rb.model, mb);
}

#[test]
fn opening_another_file_over_unsaved_edits_keeps_them_recoverable() {
    let dir = TempDir::new("recovery-reopen");
    let path = project(dir.path());
    let cfg = cfg(dir.path());
    let mut s = open(&cfg, &path);
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Kept", "expression": "1"}),
    );
    // the same session opens the file again (a reload): its old edits are offered
    run(&mut s, "file.open", json!({"path": path}));
    assert!(!s.dirty);
    assert_eq!(offers(&s).len(), 1);
    let out = run(&mut s, "recovery.apply", json!({}));
    assert_eq!(out["complete"], true);
    assert!(
        s.model.as_ref().unwrap().tables[0]
            .measures
            .iter()
            .any(|m| m.name == "Kept")
    );
    // refused over unsaved edits
    run(&mut s, "file.open", json!({"path": path}));
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Other", "expression": "1"}),
    );
    let e = s.execute("recovery.apply", &json!({})).unwrap_err();
    assert!(e.contains("unsaved"), "{e}");
}

#[test]
fn a_torn_last_line_and_hostile_ids_are_harmless() {
    let dir = TempDir::new("recovery-torn");
    let path = project(dir.path());
    let cfg = cfg(dir.path());
    let mut s = open(&cfg, &path);
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "One", "expression": "1"}),
    );
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "Two", "expression": "2"}),
    );
    let j = s.recovery_journal().unwrap();
    drop(s);
    let mut text = std::fs::read_to_string(&j).unwrap();
    text.push_str("{\"seq\": 3, \"cmd\": \"measure.add\", \"par");
    std::fs::write(&j, text).unwrap();
    let mut r = open(&cfg, &path);
    assert_eq!(offers(&r)[0]["edits"], 2);
    for id in ["../x", "0123", "..\\..\\a-b", "zzzzzzzzzzzzzzzz-zzzzzzzzzzzzzzzz"] {
        assert!(r.execute("recovery.apply", &json!({"id": id})).is_err(), "{id}");
        assert!(r.execute("recovery.discard", &json!({"id": id})).is_err(), "{id}");
    }
    let out = run(&mut r, "recovery.apply", json!({}));
    assert_eq!(out["applied"], 2);
}

#[test]
fn a_tampered_journal_runs_no_file_command() {
    let dir = TempDir::new("recovery-tamper");
    let path = project(dir.path());
    let cfg = cfg(dir.path());
    let mut s = open(&cfg, &path);
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "One", "expression": "1"}),
    );
    let j = s.recovery_journal().unwrap();
    drop(s);
    let mut text = std::fs::read_to_string(&j).unwrap();
    let evil = dir.path().join("evil.pbix");
    text.push_str(&format!(
        "{}\n",
        json!({"seq": 2, "cmd": "file.save_as", "params": {"path": evil}, "guids": []})
    ));
    std::fs::write(&j, text).unwrap();
    let mut r = open(&cfg, &path);
    let out = run(&mut r, "recovery.apply", json!({}));
    assert_eq!(out["complete"], false, "{out}");
    assert_eq!(out["applied"], 1);
    assert_eq!(out["failed"]["command"], "file.save_as");
    assert!(!evil.exists());
}

#[test]
fn off_by_default_and_without_a_file() {
    let dir = TempDir::new("recovery-off");
    let path = project(dir.path());
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "X", "expression": "1"}),
    );
    assert!(s.recovery_journal().is_none());
    assert_eq!(s.snapshot()["recovery"], Value::Null);
    assert_eq!(run(&mut s, "recovery.list", json!({}))["enabled"], false);
    assert!(s.execute("recovery.apply", &json!({})).is_err());
    assert!(s.execute("recovery.discard", &json!({})).is_err());
    // a session with nothing open journals nothing
    let cfg = cfg(dir.path());
    let mut n = Session::new();
    n.set_recovery(Some(cfg.clone()));
    assert!(
        n.execute(
            "measure.add",
            &json!({"table": "T", "name": "X", "expression": "1"})
        )
        .is_err()
    );
    assert!(n.recovery_journal().is_none());
    assert!(journals(&cfg).is_empty());
}

/// A never-saved model: `model.new` and edits, then a crash.
fn untitled(cfg: &RecoveryConfig) -> Session {
    let mut n = Session::new();
    n.set_recovery(Some(cfg.clone()));
    run(&mut n, "model.new", json!({"name": "Draft"}));
    // nothing to recover before an edit
    assert!(n.recovery_journal().is_none());
    run(
        &mut n,
        "table.add",
        json!({"name": "T", "partition": {"source_type": "calculated", "expression": "{1, 2}"}}),
    );
    run(
        &mut n,
        "measure.add",
        json!({"table": "T", "name": "Count", "expression": "COUNTROWS(T)"}),
    );
    run(&mut n, "report.new", json!({}));
    run(&mut n, "page.add", json!({"display_name": "Second"}));
    n
}

#[test]
fn a_never_saved_model_is_journaled_and_recovered() {
    let dir = TempDir::new("recovery-untitled");
    let path = project(dir.path());
    let cfg = cfg(dir.path());
    let n = untitled(&cfg);
    assert!(n.recovery_journal().is_some());
    assert_eq!(n.snapshot().pointer("/recovery/journaled"), Some(&json!(true)));
    // a new model is not offered to itself or to another new model
    assert!(offers(&n).is_empty());
    let (model, report) = (n.model.clone(), n.report.clone());
    drop(n);

    // the start screen lists it, by the model's name
    let listed = recovery_offers(&cfg);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0]["base"], "untitled", "{listed:?}");
    assert_eq!(listed[0]["recoverable"], true);
    assert_eq!(listed[0]["label"], "Draft");
    assert_eq!(listed[0]["path"], "");
    let id = listed[0]["id"].as_str().unwrap().to_string();
    // a file's own offers never include it
    let mut f = open(&cfg, &path);
    assert!(offers(&f).is_empty());
    assert_eq!(
        run(&mut f, "recovery.list", json!({"all": true}))["offers"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    // refused over unsaved edits
    run(
        &mut f,
        "measure.add",
        json!({"table": "Sales", "name": "Busy", "expression": "1"}),
    );
    let e = f.execute("recovery.apply", &json!({"id": id})).unwrap_err();
    assert!(e.contains("unsaved"), "{e}");
    run(&mut f, "recovery.discard", json!({"current": true}));
    drop(f);

    // recovered into a session with nothing open: a new model again, with the edits
    let mut r = Session::new();
    r.set_recovery(Some(cfg.clone()));
    let out = run(&mut r, "recovery.apply", json!({"id": id}));
    assert_eq!(out["complete"], true, "{out}");
    assert_eq!(out["base"], "untitled");
    assert_eq!(r.model, model, "the recovered model differs");
    assert_eq!(r.report, report, "the recovered report differs");
    assert!(r.source.is_none());
    assert!(r.dirty);
    // journaled again in r's own journal, and r's is not offered to r
    assert!(r.recovery_journal().is_some());
    assert!(recovery_offers(&cfg).is_empty(), "{:?}", recovery_offers(&cfg));
    drop(r);
    let again = recovery_offers(&cfg);
    assert_eq!(again.len(), 1);
    let mut r2 = Session::new();
    r2.set_recovery(Some(cfg.clone()));
    run(&mut r2, "recovery.apply", json!({"id": again[0]["id"]}));
    assert_eq!(r2.model, model);
}

#[test]
fn saving_a_new_model_ends_its_journal() {
    let dir = TempDir::new("recovery-untitled-save");
    let cfg = cfg(dir.path());
    // as a project
    let mut n = untitled(&cfg);
    run(
        &mut n,
        "project.new",
        json!({"dir": dir.path().join("p"), "name": "Draft"}),
    );
    assert!(n.recovery_journal().is_none());
    // the project is the base now: its edits are journaled for it
    run(
        &mut n,
        "measure.add",
        json!({"table": "T", "name": "Later", "expression": "1"}),
    );
    assert!(n.recovery_journal().is_some());
    run(&mut n, "file.save", json!({}));
    drop(n);
    assert!(recovery_offers(&cfg).is_empty(), "{:?}", recovery_offers(&cfg));
    // as a file
    let mut n = untitled(&cfg);
    run(
        &mut n,
        "file.save_as",
        json!({"path": dir.path().join("Draft.pbix")}),
    );
    drop(n);
    assert!(recovery_offers(&cfg).is_empty(), "{:?}", recovery_offers(&cfg));
    // "the open file's journals" of a new model are none: another new model's journal stays
    let left = untitled(&cfg);
    drop(left);
    let mut n = untitled(&cfg);
    assert!(n.execute("recovery.discard", &json!({})).is_err());
    assert_eq!(recovery_offers(&cfg).len(), 1);
    run(&mut n, "recovery.discard", json!({"all": true, "current": true}));
    assert!(recovery_offers(&cfg).is_empty());
    drop(n);
    // a clean close
    let mut n = untitled(&cfg);
    run(&mut n, "recovery.discard", json!({"current": true}));
    drop(n);
    assert!(recovery_offers(&cfg).is_empty());
}

#[test]
fn view_settings_are_recovered_with_the_edits() {
    let dir = TempDir::new("recovery-view");
    let path = project(dir.path());
    let cfg = cfg(dir.path());
    let mut s = open(&cfg, &path);
    // before the first edit (the journal's header keeps it) and after (a line of its own)
    run(&mut s, "view.set_as_of", json!({"as_of": "2026-03-04"}));
    run(&mut s, "page.add", json!({"display_name": "Second"}));
    let second = s.report.as_ref().unwrap().pages[1].name.clone();
    run(&mut s, "view.set_language", json!({"ui_language": "fr-FR"}));
    run(&mut s, "view.interact", json!({"action": "page", "page": second}));
    assert_eq!(s.snapshot().pointer("/recovery/edits"), Some(&json!(1)));
    let as_of = s.as_of;
    drop(s);

    let mut r = open(&cfg, &path);
    assert_eq!(offers(&r)[0]["edits"], 1);
    let out = run(&mut r, "recovery.apply", json!({}));
    assert_eq!(out["complete"], true, "{out}");
    assert_eq!(out["view"]["errors"], json!([]), "{out}");
    assert_eq!(r.ui_language.tag(), "fr-FR");
    assert!(as_of.is_some());
    assert_eq!(r.as_of, as_of);
    assert_eq!(r.interactions.current_page(), Some(second.as_str()));
    // a view setting alone is not an edit: a clean session keeps no journal
    let mut c = open(&cfg, &path);
    run(&mut c, "view.set_language", json!({"ui_language": "de-DE"}));
    assert!(c.recovery_journal().is_none());
}

#[test]
fn every_command_declares_how_it_is_journaled() {
    // view settings, caches and exports change the session but not the file
    const SKIPPED_MUTATING: &[&str] = &[
        "view.set_language",
        "view.set_culture",
        "view.set_as_of",
        "dax.clear_cache",
        "dax.aggregations",
        "dax.memory",
        "file.set_cache_abf",
        // secrets live in session memory only: never written to a journal
        "source.credentials",
        // privacy levels are session settings, as credentials are
        "source.privacy",
    ];
    for c in COMMANDS {
        let ro = READ_ONLY_COMMANDS.contains(&c.id);
        match c.replay {
            Replay::Skip => assert!(
                ro || SKIPPED_MUTATING.contains(&c.id),
                "{}: a command that edits the session is journaled (Exact, Copy or Base)",
                c.id
            ),
            _ => assert!(is_mutating(c.id), "{}: a read-only command is Replay::Skip", c.id),
        }
        // a command reading a file or the network cannot replay from its parameters
        let reads = c
            .server
            .paths
            .iter()
            .any(|p| !["data_root", "cache_abf"].contains(&p.name))
            || matches!(c.server.access, bicraft_engine::Access::Network);
        // these take their image as bytes or as a file: a run that names a file writes a recovery
        // copy (`rich::reads_file`) and such a line is never replayed (tested below)
        const COPY_WHEN_A_FILE_IS_READ: &[&str] = &[
            "image.add",
            "image.set",
            "page.format",
            "visual.import_custom",
            "shape_map.set_map",
        ];
        if reads
            && is_mutating(c.id)
            && c.replay == Replay::Exact
            && !COPY_WHEN_A_FILE_IS_READ.contains(&c.id)
        {
            panic!(
                "{}: reads outside inputs, so it is Replay::Copy, Base or Skip",
                c.id
            );
        }
    }
}

/// The journal's cost per command (`cargo test -p bicraft-engine --release journal_overhead --
/// --ignored --nocapture`): measure.set alternating two descriptions, with and without the
/// journal.
#[test]
#[ignore]
fn journal_overhead() {
    let dir = TempDir::new("recovery-bench");
    let path = project(dir.path());
    let n = 2000u32;
    let time = |journal: bool| {
        let mut s = Session::new();
        if journal {
            s.set_recovery(Some(cfg(dir.path()).with_max_bytes(1 << 30)));
        }
        run(&mut s, "file.open", json!({"path": path}));
        run(
            &mut s,
            "measure.add",
            json!({"table": "Sales", "name": "B", "expression": "1"}),
        );
        let start = std::time::Instant::now();
        for i in 0..n {
            run(
                &mut s,
                "measure.set",
                json!({"table": "Sales", "name": "B", "description": format!("d{}", i % 2)}),
            );
        }
        start.elapsed().as_secs_f64() * 1e6 / f64::from(n)
    };
    let off = time(false);
    let on = time(true);
    println!(
        "measure.set: {off:.1} µs without the journal, {on:.1} µs with it ({:+.1} µs per command)",
        on - off
    );
}

/// An image read from a file writes a recovery copy (`rich::reads_file`), an image
/// given as bytes is a line; a tampered journal line that names a file is never replayed (M11.20).
#[test]
fn images_read_from_files_are_copied_and_never_replayed() {
    const PNG: &str =
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
    let dir = TempDir::new("recovery-images");
    let path = project(dir.path());
    let cfg = cfg(dir.path());
    let mut s = open(&cfg, &path);
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    run(
        &mut s,
        "image.add",
        json!({"page": page, "data": PNG, "name": "a.png"}),
    );
    let j = s.recovery_journal().unwrap();
    drop(s);
    let text = std::fs::read_to_string(&j).unwrap();
    assert!(
        text.contains("\"cmd\":\"image.add\""),
        "bytes replay from the line: {text}"
    );
    let png = dir.path().join("logo.png");
    // a tampered line naming a file is refused on replay
    let mut text = std::fs::read_to_string(&j).unwrap();
    text.push_str(&format!(
        "{}\n",
        json!({"seq": 99, "cmd": "image.add", "params": {"page": page, "path": png}, "guids": []})
    ));
    std::fs::write(&j, text).unwrap();
    let mut r = open(&cfg, &path);
    let out = run(&mut r, "recovery.apply", json!({}));
    assert_eq!(out["complete"], false, "{out}");
    assert_eq!(out["failed"]["command"], "image.add");
    assert!(
        out["failed"]["error"].as_str().unwrap().contains("not replayed"),
        "{out}"
    );
}
