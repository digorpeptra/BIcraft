use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::tempdir::TempDir;
use bicraft_engine::*;

/// A session whose opener records the paths instead of launching anything.
fn recording(s: &mut Session) -> Arc<Mutex<Vec<PathBuf>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    s.external_opener = Some(ExternalOpener(Arc::new(move |p: &Path| {
        log.lock().unwrap().push(p.to_path_buf());
        Ok(())
    })));
    seen
}

fn package_session(path: &Path) -> Session {
    let mut s = Session::new();
    s.source = Some(Source::Package {
        path: path.to_path_buf(),
        package: bicraft_opc::Package::default(),
    });
    s
}

#[test]
fn opens_the_saved_package_only() {
    let dir = TempDir::new("engine-desktop-open");
    let file = dir.join("Shop.pbix");
    std::fs::write(&file, b"x").unwrap();
    let mut s = package_session(&file);
    let seen = recording(&mut s);

    let t = s.desktop_target();
    assert_eq!(
        t.path.as_deref(),
        Some(std::path::absolute(&file).unwrap().as_path())
    );
    assert!(!t.unsaved && t.reason.is_none());

    // a dry run reports without opening
    let dry = s
        .execute("file.open_in_desktop", &json!({"dry_run": true}))
        .unwrap();
    assert_eq!(dry["opened"], false);
    assert!(seen.lock().unwrap().is_empty());

    let out = s.execute("file.open_in_desktop", &json!({})).unwrap();
    assert_eq!(out["opened"], true);
    assert_eq!(out["unsaved"], false);
    assert!(out.get("warning").is_none());
    assert_eq!(*seen.lock().unwrap(), vec![std::path::absolute(&file).unwrap()]);

    // the caller cannot name another file
    let e = s
        .execute("file.open_in_desktop", &json!({"path": "C:/Windows/notepad.exe"}))
        .unwrap_err();
    assert!(e.contains("path"), "{e}");
    assert_eq!(seen.lock().unwrap().len(), 1);
    // read-only: the revision does not move
    assert_eq!(s.revision, 0);
}

#[test]
fn unsaved_edits_are_reported() {
    let dir = TempDir::new("engine-desktop-open-dirty");
    let file = dir.join("Shop.pbit");
    std::fs::write(&file, b"x").unwrap();
    let mut s = package_session(&file);
    let _seen = recording(&mut s);
    s.report_dirty = true;
    let t = s.desktop_target();
    assert!(t.unsaved && t.path.is_some());
    let out = s.execute("file.open_in_desktop", &json!({})).unwrap();
    assert_eq!(out["unsaved"], true);
    assert!(out["warning"].as_str().unwrap().contains("unsaved"));
    let snap = s.snapshot();
    assert_eq!(snap["source"]["desktop"]["unsaved"], true);
    assert!(
        snap["source"]["desktop"]["path"]
            .as_str()
            .unwrap()
            .ends_with("Shop.pbit")
    );
}

#[test]
fn nothing_to_open_without_a_saved_file() {
    let mut s = Session::new();
    let seen = recording(&mut s);
    s.execute("model.new", &json!({"name": "M"})).unwrap();
    let t = s.desktop_target();
    assert!(t.path.is_none() && t.unsaved);
    assert!(t.reason.as_deref().unwrap().contains("save"), "{t:?}");
    let e = s.execute("file.open_in_desktop", &json!({})).unwrap_err();
    assert!(e.contains("nothing is saved yet"), "{e}");
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(s.snapshot()["source"]["desktop"]["path"], serde_json::Value::Null);

    // a standalone model.bim: Desktop cannot open it
    let dir = TempDir::new("engine-desktop-open-bim");
    let bim = dir.join("model.bim");
    std::fs::write(&bim, r#"{"name":"M","model":{"tables":[]}}"#).unwrap();
    s.execute("file.open", &json!({"path": bim})).unwrap();
    let e = s.execute("file.open_in_desktop", &json!({})).unwrap_err();
    assert!(e.contains("model.bim"), "{e}");

    // a package whose file is gone
    let gone = dir.join("Gone.pbix");
    let mut s = package_session(&gone);
    let seen = recording(&mut s);
    assert!(s.execute("file.open_in_desktop", &json!({})).is_err());
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn projects_open_their_pbip_file() {
    let dir = TempDir::new("engine-desktop-open-pbip");
    let mut s = Session::new();
    let seen = recording(&mut s);
    s.execute("model.new", &json!({"name": "Shop"})).unwrap();
    s.execute("project.new", &json!({"dir": dir.path(), "name": "Shop"}))
        .unwrap();
    let pbip = std::path::absolute(dir.join("Shop.pbip")).unwrap();
    assert!(pbip.is_file());
    assert_eq!(s.desktop_target().path.as_deref(), Some(pbip.as_path()));

    // opened from the folder or an item folder: still the .pbip file
    for open in [dir.to_path_buf(), dir.join("Shop.SemanticModel")] {
        let mut s = Session::new();
        let seen = recording(&mut s);
        s.execute("file.open", &json!({"path": open})).unwrap();
        s.execute("file.open_in_desktop", &json!({})).unwrap();
        let got = seen.lock().unwrap().clone();
        assert_eq!(got.len(), 1);
        assert!(
            got[0].file_name().unwrap().eq_ignore_ascii_case("Shop.pbip"),
            "{got:?}"
        );
    }

    // no .pbip file in the folder: nothing to open
    std::fs::remove_file(dir.join("Shop.pbip")).unwrap();
    let t = s.desktop_target();
    assert!(t.path.is_none(), "{t:?}");
    assert!(t.reason.unwrap().contains(".pbip"));
    assert!(s.execute("file.open_in_desktop", &json!({})).is_err());
    assert_eq!(seen.lock().unwrap().len(), 0);
}

#[test]
fn the_opener_error_is_the_command_error() {
    let dir = TempDir::new("engine-desktop-open-err");
    let file = dir.join("Shop.pbix");
    std::fs::write(&file, b"x").unwrap();
    let mut s = package_session(&file);
    s.external_opener = Some(ExternalOpener(Arc::new(|_: &Path| {
        Err("no application handles .pbix".to_string())
    })));
    let e = s.execute("file.open_in_desktop", &json!({})).unwrap_err();
    assert!(e.contains("no application"), "{e}");
}
