//! The window's editing buttons on the desktop's engine source (`local.rs`): page tabs, Save, a
//! new project in a new document tab.

use bicraft_engine::Session;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::{Temp, open, ready, wait};

/// `+` adds a page through `page.add` on the engine session; Save writes it; the file on disk
/// has it.
#[test]
fn page_tabs_and_save_run_on_the_engine_session() {
    let dir = Temp::new("edit");
    let (mut h, _, path) = open(dir.path(), (1280.0, 800.0));
    let n = h.state().page_names().len();
    h.get_by_label("Add a page").click();
    wait(&mut h, "the new page", |a| a.page_names().len() == n + 1);
    wait(&mut h, "the unsaved dot", |a| a.window_title().starts_with("● "));
    // page.add's answer, then the `page` interaction it asks for, then that snapshot
    wait(&mut h, "the new page shown", |a| a.active_page() == n);
    h.state_mut().save();
    wait(&mut h, "the save", |a| !a.window_title().starts_with("● "));
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path})).unwrap();
    assert_eq!(s.report.as_ref().unwrap().pages.len(), n + 1);
}

/// New > Blank report: a PBIP project in the folder picked, opened in a second tab.
#[test]
fn a_new_project_opens_in_a_new_tab() {
    let dir = Temp::new("new");
    let (mut h, _, _) = open(dir.path(), (1280.0, 800.0));
    let folder = dir.path().join("made");
    std::fs::create_dir_all(&folder).unwrap();
    let f = folder.clone();
    h.state_mut().shell.pick_folder = Some(Box::new(move |_| Some(f.clone())));
    assert!(h.state_mut().new_project("Fresh", None));
    wait(&mut h, "the new project", |a| {
        ready(a) && a.window_title().contains("Fresh")
    });
    assert_eq!(h.state().doc_tabs().0.len(), 2);
    assert!(folder.join("Fresh.pbip").exists());
    assert!(h.state().page_names().len() == 1, "a blank one-page report");
    // back to the first file
    h.state_mut().switch_doc(0);
    h.step();
    assert!(h.state().window_title().starts_with("shop.pbip"));
}
