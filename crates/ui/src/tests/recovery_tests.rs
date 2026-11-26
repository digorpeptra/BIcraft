//! Auto-recovery and autosave in the window (`crate::recovery`, docs/recovery.md): the prompt over
//! a real engine journal, the File view's list, the status bar's hint, the autosave preference
//! and its runs, the same frames in both shells.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bicraft_engine::{RecoveryConfig, Session};
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, engine_shell_for, steps};
use super::{fixed, frame, harness, loaded, shell_app, shop_snapshot};
use crate::App;
use crate::backstage::{Backstage, Page};
use crate::prefs::Autosave;
use crate::shell::Caps;
use crate::source::{Loaded, Origin};

/// A scratch folder removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!(
            "bicraft-ui-recovery-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A saved PBIP project `Shop.pbip` in `dir`, and a crashed session's journal of one edit on it.
fn crashed(dir: &Path) -> (PathBuf, RecoveryConfig) {
    let mut s = Session::new();
    s.execute("project.new", &json!({"dir": dir, "name": "Shop"}))
        .unwrap();
    s.execute("table.add", &json!({"name": "Sales"})).unwrap();
    s.execute("file.save", &json!({})).unwrap();
    let path = dir.join("Shop.pbip");
    let cfg = RecoveryConfig::new(dir.join("recovery"));
    let mut lost = Session::new();
    lost.set_recovery(Some(cfg.clone()));
    lost.execute("file.open", &json!({"path": path})).unwrap();
    lost.execute(
        "measure.add",
        &json!({"table": "Sales", "name": "Kept", "expression": "1"}),
    )
    .unwrap();
    drop(lost);
    (path, cfg)
}

/// The app with `path` open in an engine session that journals in `cfg`.
fn recovering_app(path: &Path, cfg: &RecoveryConfig) -> egui_kittest::Harness<'static, App> {
    let (p, c) = (path.to_path_buf(), cfg.clone());
    let shell = engine_shell(Caps::DESKTOP, move || {
        let mut s = Session::new();
        s.set_recovery(Some(c.clone()));
        s.execute("file.open", &json!({"path": p})).unwrap();
        s
    });
    let mut app = App::new("en-US", shell);
    app.open_path(path);
    let mut h = harness(app, (1280.0, 800.0));
    steps(&mut h, 6);
    h
}

fn snap(h: &egui_kittest::Harness<'_, App>) -> Value {
    h.state()
        .loaded
        .as_ref()
        .map(|l| (*l.snap).clone())
        .unwrap_or(Value::Null)
}

#[test]
fn the_prompt_recovers_through_the_engine() {
    let dir = Scratch::new("prompt");
    let (path, cfg) = crashed(&dir.0);
    let mut h = recovering_app(&path, &cfg);
    h.get_by_label("Recover unsaved changes");
    h.get_by_label("Later");
    h.get_by_label("Discard");
    h.get_by_label("Recover").click();
    steps(&mut h, 4);
    assert!(h.state().sent.iter().any(|c| c.id == "recovery.apply"));
    let s = snap(&h);
    assert_eq!(
        s.pointer("/recovery/offers"),
        Some(&json!([])),
        "{}",
        s["recovery"]
    );
    assert_eq!(s.pointer("/dirty/model"), Some(&json!(true)));
    assert!(s.to_string().contains("\"Kept\""));
    assert_eq!(h.state().status, "Recovered 1 edit");
    // the prompt is gone; the status bar says the edit is journaled again
    assert!(h.query_by_label("Recover").is_none());
    h.get_by_label("Recovery: 1 edit");
}

#[test]
fn discard_and_later_answer_the_prompt() {
    let dir = Scratch::new("discard");
    let (path, cfg) = crashed(&dir.0);
    let mut h = recovering_app(&path, &cfg);
    h.get_by_label("Later").click();
    steps(&mut h, 3);
    assert!(h.query_by_label("Recover").is_none(), "Later keeps asking");
    // still offered by the engine: the File view lists it
    assert_eq!(
        snap(&h)
            .pointer("/recovery/offers")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );
    let mut h = recovering_app(&path, &cfg);
    h.get_by_label("Discard").click();
    steps(&mut h, 4);
    assert!(h.state().sent.iter().any(|c| c.id == "recovery.discard"));
    assert_eq!(snap(&h).pointer("/recovery/offers"), Some(&json!([])));
    assert!(bicraft_engine::recovery_offers(&cfg).is_empty());
    assert_eq!(snap(&h).pointer("/dirty/model"), Some(&json!(false)));
}

#[test]
fn the_start_screen_lists_what_a_crash_left() {
    let dir = Scratch::new("start");
    let (path, cfg) = crashed(&dir.0);
    let mut shell = fixed(Caps::DESKTOP, None);
    let c = cfg.clone();
    shell.recovery_list = Some(Box::new(move || bicraft_engine::recovery_offers(&c)));
    let mut h = harness(App::new("en-US", shell), (1280.0, 800.0));
    steps(&mut h, 4);
    h.get_by_label("Recover unsaved changes");
    h.get_by_label("Shop.pbip");
    // the row's Open (the File view's own Open is in the left column)
    h.get_all_by_label("Open")
        .max_by(|a, b| a.rect().min.x.total_cmp(&b.rect().min.x))
        .unwrap()
        .click();
    steps(&mut h, 3);
    let opened = h.state().origin.as_ref().map(crate::source::Origin::label);
    assert_eq!(opened.as_deref(), Some("Shop.pbip"), "{path:?}");
}

/// A project or package snapshot with unsaved edits.
fn dirty_snapshot(kind: &str, autosave: Option<bool>) -> Loaded {
    let mut s = (*shop_snapshot().snap).clone();
    s["source"]["kind"] = json!(kind);
    s["dirty"] = json!({"model": true, "report": false});
    s["recovery"] = json!({"enabled": true, "journaled": true, "edits": 3, "copy": false,
                           "error": null, "offers": []});
    if let Some(a) = autosave {
        s["view"] = json!({"limits": {"autosave": a}});
    }
    loaded(s)
}

#[test]
fn autosave_is_offered_for_projects_and_explains_otherwise() {
    for (kind, server, expected) in [
        ("project", None, None),
        ("package", None, Some("Not offered for PBIX and PBIT files")),
        (
            "project",
            Some(false),
            Some("This server does not allow automatic saves"),
        ),
    ] {
        let mut app = App::new("en-US", fixed(Caps::DESKTOP, Some(dirty_snapshot(kind, server))));
        app.open_path(Path::new("shop.pbip"));
        app.backstage = Some(Backstage::new(Page::Info));
        app.info = Some(Ok(json!({})));
        let mut h = harness(app, (1280.0, 900.0));
        steps(&mut h, 4);
        h.get_by_label("Save automatically");
        h.get_by_label("Every");
        h.get_by_label("After").click();
        steps(&mut h, 2);
        assert!(matches!(h.state().prefs.autosave, Autosave::Idle(_)), "{kind}");
        match expected {
            None => {
                assert!(h.query_by_label_contains("Not offered").is_none(), "{kind}");
                assert!(h.query_by_label_contains("does not allow").is_none(), "{kind}");
            }
            Some(text) => {
                h.get_by_label_contains(text);
            }
        }
        h.get_by_label("Unsaved edits are journaled for recovery (3 edits)");
        h.get_by_label("Off").click();
        steps(&mut h, 2);
        assert_eq!(h.state().prefs.autosave, Autosave::Off);
    }
}

#[test]
fn autosave_saves_a_project_after_the_idle_delay_only() {
    let dir = Scratch::new("autosave");
    let mut s = Session::new();
    s.execute("project.new", &json!({"dir": dir.0, "name": "Auto"}))
        .unwrap();
    s.execute("table.add", &json!({"name": "Sales"})).unwrap();
    s.execute("file.save", &json!({})).unwrap();
    let path = dir.0.join("Auto.pbip");
    let p = path.clone();
    let shell = engine_shell(Caps::DESKTOP, move || {
        let mut s = Session::new();
        s.execute("file.open", &json!({"path": p})).unwrap();
        s
    });
    let mut app = App::new("en-US", shell);
    app.prefs.autosave = Autosave::Idle(5);
    app.open_path(&path);
    let mut h = harness(app, (1280.0, 800.0));
    steps(&mut h, 4);
    // nothing to save yet
    steps(&mut h, 30);
    assert_eq!(h.state().recovery.saves, 0);
    let c = crate::actions::Call {
        id: "measure.add",
        params: json!({"table": "Sales", "name": "Auto", "expression": "1"}),
    };
    h.state_mut().run(c, crate::app::Then::Nothing);
    // a step is a quarter of a second: 5 s are 20 steps
    steps(&mut h, 12);
    assert_eq!(h.state().recovery.saves, 0, "saved before the idle delay");
    steps(&mut h, 14);
    assert_eq!(h.state().recovery.saves, 1);
    assert_eq!(snap(&h).pointer("/dirty/model"), Some(&json!(false)));
    assert_eq!(h.state().status, "Saved automatically");
    let on_disk = std::fs::read_dir(dir.0.join("Auto.SemanticModel/definition/tables"))
        .unwrap()
        .flatten()
        .map(|e| std::fs::read_to_string(e.path()).unwrap_or_default())
        .collect::<String>();
    assert!(on_disk.contains("measure Auto"), "{on_disk}");
    // clean again: no more saves
    steps(&mut h, 40);
    assert_eq!(h.state().recovery.saves, 1);
}

#[test]
fn the_prompt_draws_the_same_frames_in_both_shells() {
    let mut snap = (*shop_snapshot().snap).clone();
    snap["recovery"] = json!({"enabled": true, "journaled": true, "edits": 0, "copy": false,
        "error": null, "offers": [{"id": "0123456789abcdef-0123456789abcdef", "path": "shop.pbip",
        "label": "shop.pbip", "edits": 4, "copy": false, "at_ms": 1_793_000_000_000u64,
        "created_ms": 1_793_000_000_000u64, "base": "same", "recoverable": true, "why": null}]});
    let l = Loaded {
        snap: Arc::new(snap),
        ..shop_snapshot()
    };
    for (lang, dark) in [("en-US", false), ("fr-FR", false), ("en-US", true)] {
        let mut desktop = shell_app(Caps::DESKTOP, &l, lang);
        let mut browser = shell_app(Caps::BROWSER, &l, lang);
        for h in [&mut desktop, &mut browser] {
            if dark {
                h.state_mut().set_theme(crate::theme::Brightness::Dark);
            }
            steps(h, 3);
        }
        let label = if lang == "fr-FR" {
            "Récupérer les modifications non enregistrées"
        } else {
            "Recover unsaved changes"
        };
        desktop.get_by_label(label);
        browser.get_by_label(label);
        let (a, b) = (frame(&desktop), frame(&browser));
        assert!(a.0 == b.0, "{lang} dark={dark}: the meshes differ");
        assert!(a.1 == b.1, "{lang}: the font atlases differ");
    }
}

#[test]
fn autosave_saves_every_open_project_not_only_the_active_one() {
    let dir = Scratch::new("autosave-tabs");
    let mut paths = Vec::new();
    for name in ["One", "Two"] {
        let mut s = Session::new();
        s.execute("project.new", &json!({"dir": dir.0, "name": name}))
            .unwrap();
        s.execute("table.add", &json!({"name": "Sales"})).unwrap();
        s.execute("file.save", &json!({})).unwrap();
        paths.push(dir.0.join(format!("{name}.pbip")));
    }
    let shell = engine_shell_for(Caps::DESKTOP, |o| {
        let mut s = Session::new();
        if let Origin::Local { path, .. } = o {
            s.execute("file.open", &json!({"path": path})).unwrap();
        }
        s
    });
    let mut app = App::new("en-US", shell);
    app.prefs.autosave = Autosave::Idle(5);
    let mut h = harness(app, (1280.0, 800.0));
    for (i, p) in paths.iter().enumerate() {
        h.state_mut().open_path(p);
        steps(&mut h, 4);
        let c = crate::actions::Call {
            id: "measure.add",
            params: json!({"table": "Sales", "name": format!("Auto{i}"), "expression": "1"}),
        };
        h.state_mut().run(c, crate::app::Then::Nothing);
        steps(&mut h, 2);
    }
    // the second tab is shown; the first waits behind it, with unsaved edits too
    assert_eq!(h.state().active_doc(), 1);
    assert_eq!(h.state().doc_tabs().0.iter().filter(|t| t.1).count(), 2);
    steps(&mut h, 30);
    assert_eq!(h.state().recovery.saves, 2, "{}", h.state().status);
    assert!(
        h.state().doc_tabs().0.iter().all(|t| !t.1),
        "{:?}",
        h.state().doc_tabs()
    );
    for (i, name) in ["One", "Two"].iter().enumerate() {
        let on_disk = std::fs::read_dir(dir.0.join(format!("{name}.SemanticModel/definition/tables")))
            .unwrap()
            .flatten()
            .map(|e| std::fs::read_to_string(e.path()).unwrap_or_default())
            .collect::<String>();
        assert!(on_disk.contains(&format!("measure Auto{i}")), "{name}: {on_disk}");
    }
}

#[test]
fn a_never_saved_model_is_recovered_from_the_file_view() {
    let dir = Scratch::new("untitled");
    let (path, cfg) = crashed(&dir.0);
    // that journal is recovered first: only the new model's is left
    let mut r = Session::new();
    r.set_recovery(Some(cfg.clone()));
    r.execute("file.open", &json!({"path": path})).unwrap();
    r.execute("recovery.discard", &json!({})).unwrap();
    drop(r);
    let mut lost = Session::new();
    lost.set_recovery(Some(cfg.clone()));
    lost.execute("model.new", &json!({"name": "Draft"})).unwrap();
    lost.execute(
        "table.add",
        &json!({"name": "T", "partition": {"source_type": "calculated", "expression": "{1}"}}),
    )
    .unwrap();
    drop(lost);
    let mut h = recovering_app(&path, &cfg);
    // the open file has no offer of its own: no prompt
    assert!(h.query_by_label("Later").is_none());
    h.state_mut().backstage = Some(Backstage::new(Page::Home));
    steps(&mut h, 4);
    h.get_by_label("Draft");
    h.get_by_label_contains("New model, never saved");
    h.get_by_label("Recover").click();
    steps(&mut h, 4);
    assert!(h.state().sent.iter().any(|c| c.id == "recovery.apply"));
    let s = snap(&h);
    assert_eq!(s.pointer("/source/kind"), Some(&json!("none")), "{}", s["source"]);
    assert!(s.to_string().contains("\"Draft\""), "{}", s["model"]);
    assert!(bicraft_engine::recovery_offers(&cfg).is_empty());
}
