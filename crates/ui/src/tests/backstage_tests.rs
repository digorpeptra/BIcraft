//! The File view (Backstage), the report's page tabs and the document tabs, through the
//! accessibility tree (egui_kittest, no GPU). Sources here are an engine session run in the test
//! (`EngineDriver`), so a page tab's action goes through the real `page.*` command and the next
//! snapshot redraws the tabs.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use bicraft_engine::Session;
use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::{fixed, frame, harness, loaded, shell_app, shop_snapshot};
use crate::App;
use crate::app::Tab;
use crate::backstage::{Page, Task};
use crate::prefs::Recent;
use crate::shell::{Caps, Shell};
use crate::source::{Driver, Link, Loaded, Origin, Request, Running, Update, WsFile};

/// A driven source over an engine session in the test: commands and interactions run at once,
/// then the snapshot is sent again, as the desktop's local source does.
struct EngineDriver {
    session: RefCell<Session>,
    link: Link,
    seq: Cell<u64>,
}

impl EngineDriver {
    fn publish(&self) {
        let snap = self.session.borrow().snapshot();
        // the snapshot's images, as the desktop's local source sends them
        let assets = self.session.borrow().snapshot_assets();
        let assets = assets.into_iter().map(|a| (a.id, a.bytes)).collect();
        self.seq.set(self.seq.get() + 1);
        self.link.send(Update::Snapshot(Loaded {
            snap: Arc::new(snap),
            assets: Arc::new(assets),
            seq: self.seq.get(),
        }));
    }
}

impl Driver for EngineDriver {
    fn request(&self, r: Request) {
        match r {
            Request::Command { tag, id, params } => {
                let result = self.session.borrow_mut().execute(&id, &params);
                if result.is_ok() {
                    self.publish();
                }
                self.link.send(Update::Ran { tag, result });
            }
            Request::Interact(a) => {
                let r = self.session.borrow_mut().execute("view.interact", &a);
                self.publish();
                self.link.send(Update::Interacted(r));
            }
            Request::ListFiles => self.link.send(Update::Files(Ok(vec![
                WsFile {
                    path: "reports/Sales.pbix".into(),
                    size: Some(2048),
                    modified: "2026-11-04T10:00:00Z".into(),
                },
                WsFile {
                    path: "notes.txt".into(),
                    size: Some(5),
                    modified: String::new(),
                },
            ]))),
            _ => {}
        }
    }
}

/// A session with a report of `pages` pages named `P1`, `P2`…
pub(super) fn report_session(pages: usize) -> Session {
    let mut s = Session::new();
    s.execute("model.new", &json!({"name": "M"})).unwrap();
    s.execute("report.new", &json!({})).unwrap();
    let first = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute("page.rename", &json!({"page": first, "display_name": "P1"}))
        .unwrap();
    for i in 2..=pages {
        s.execute("page.add", &json!({"display_name": format!("P{i}")}))
            .unwrap();
    }
    // as if just opened from disk: nothing unsaved yet
    s.dirty = false;
    s.report_dirty = false;
    s
}

/// The rectangle of page `label`'s tab (the bottom-most node of that name: the title row can
/// show the same name).
pub(super) fn page_tab(h: &Harness<'_, App>, label: &str) -> egui::Rect {
    h.get_all_by_label(label)
        .map(|n| n.rect())
        .max_by(|a, b| a.center().y.total_cmp(&b.center().y))
        .unwrap()
}

/// A shell whose every source is an engine session made by `make`.
pub(super) fn engine_shell(caps: Caps, make: impl Fn() -> Session + 'static) -> Shell {
    engine_shell_for(caps, move |_| make())
}

/// A shell whose sources are engine sessions made by `make` from the origin opened.
pub(super) fn engine_shell_for(caps: Caps, make: impl Fn(&Origin) -> Session + 'static) -> Shell {
    Shell::new(caps, move |o, link| {
        let d = EngineDriver {
            session: RefCell::new(make(&o)),
            link,
            seq: Cell::new(0),
        };
        d.publish();
        Running::Driven(Box::new(d))
    })
}

fn engine_app(pages: usize, size: (f32, f32)) -> Harness<'static, App> {
    let mut app = App::new(
        "en-US",
        engine_shell(Caps::DESKTOP, move || report_session(pages)),
    );
    app.open_path(Path::new("r.pbip"));
    let mut h = harness(app, size);
    for _ in 0..6 {
        h.step();
    }
    h
}

fn page_labels(h: &Harness<'_, App>) -> Vec<String> {
    h.state().page_names().into_iter().map(|p| p.1).collect()
}

fn last_ids(h: &Harness<'_, App>) -> Vec<&'static str> {
    h.state().sent.iter().map(|c| c.id).collect()
}

pub(super) fn steps(h: &mut Harness<'_, App>, n: usize) {
    for _ in 0..n {
        h.step();
    }
}

#[test]
fn the_file_view_pages() {
    let mut h = engine_app(2, (1280.0, 800.0));
    h.get_by_label("File").click();
    steps(&mut h, 3);
    assert_eq!(h.state().backstage.as_ref().map(|b| b.page), Some(Page::Home));
    for label in [
        "Back",
        "Home",
        "Open",
        "Info",
        "Save",
        "Save As",
        "Export",
        "Close",
        "Blank report",
    ] {
        h.get_by_label(label);
    }
    h.get_by_label("Info").click();
    steps(&mut h, 4);
    for label in [
        "Properties",
        "Format",
        "Tables",
        "Pages",
        "Visuals",
        "Check the file",
    ] {
        h.get_by_label(label);
    }
    // Info reads file.inspect
    assert!(last_ids(&h).contains(&"file.inspect"));
    assert!(h.state().info.is_some());
    h.get_by_label("Export").click();
    steps(&mut h, 3);
    for label in [
        "Save a copy as",
        "Convert to another format",
        "Render the page to PNG",
        "Compare with another file",
    ] {
        // the task's row, and its heading when it is the one shown
        assert!(h.query_all_by_label(label).count() >= 1, "{label}");
    }
    h.get_by_label("Open").click();
    steps(&mut h, 3);
    for label in ["Recent", "Browse", "Servers"] {
        h.get_by_label(label);
    }
    // Escape returns to the document
    h.key_press(egui::Key::Escape);
    steps(&mut h, 3);
    assert!(h.state().backstage.is_none());
}

/// Export runs its command with the Save dialog's path and shows the output.
#[test]
fn export_runs_the_commands_and_shows_their_output() {
    let dir = std::env::temp_dir().join(format!("bicraft-ui-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let project = dir.join("src");
    std::fs::create_dir_all(&project).unwrap();
    let saved = {
        let mut s = report_session(1);
        s.execute("project.new", &json!({"dir": project, "name": "Shop"}))
            .unwrap();
        project.join("Shop.pbip")
    };
    let p2 = saved.clone();
    let mut shell = engine_shell(Caps::DESKTOP, move || {
        let mut s = Session::new();
        s.execute("file.open", &json!({"path": p2})).unwrap();
        s
    });
    let png = dir.join("page.png");
    let target = Rc::new(RefCell::new(png.clone()));
    let t2 = Rc::clone(&target);
    shell.pick_save = Some(Box::new(move |_, _, _, _| Some(t2.borrow().clone())));
    let mut app = App::new("en-US", shell);
    app.open_path(&saved);
    let mut h = harness(app, (1280.0, 800.0));
    steps(&mut h, 5);
    let mut bs = crate::backstage::Backstage::new(Page::Export);
    bs.task = Task::Render;
    h.state_mut().backstage = Some(bs);
    steps(&mut h, 3);
    h.get_by_label("Choose where…").click();
    steps(&mut h, 4);
    assert_eq!(last_ids(&h).last(), Some(&"report.render"));
    assert!(png.exists(), "the PNG was written");
    assert!(
        matches!(&h.state().output, Some(("render", Some(Ok(_))))),
        "{:?}",
        h.state().output.as_ref().map(|o| o.0)
    );
    h.get_by_label_contains("Written:");
    // Convert to PBIT
    *target.borrow_mut() = dir.join("Shop.pbit");
    if let Some(b) = h.state_mut().backstage.as_mut() {
        b.task = Task::Convert;
        b.format = "pbit";
    }
    steps(&mut h, 3);
    h.get_by_label("Choose where…").click();
    steps(&mut h, 4);
    assert_eq!(last_ids(&h).last(), Some(&"file.convert"));
    assert!(dir.join("Shop.pbit").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Recent: search by name or folder, remove one, pin one; 1,000 entries never panic.
#[test]
fn recent_search_remove_and_huge_lists() {
    let mut app = App::new("en-US", fixed(Caps::DESKTOP, None));
    for i in 0..1000 {
        app.prefs.recent.push(Recent {
            path: format!("C:/data/folder{}/Report {i}.pbix", i % 7),
            ws: None,
            opened: 1_762_214_400 + i,
            pinned: false,
        });
    }
    app.prefs.recent.push(Recent {
        path: "C:/other/Shop.pbip".into(),
        ..Recent::default()
    });
    let mut h = harness(app, (1280.0, 900.0));
    steps(&mut h, 4);
    // the start screen is the File view's Home
    assert_eq!(h.state().backstage.as_ref().map(|b| b.page), Some(Page::Home));
    h.get_by_label("Report 0.pbix");
    h.get_by_label("Open").click();
    steps(&mut h, 3);
    if let Some(b) = h.state_mut().backstage.as_mut() {
        b.query = "shop".into();
    }
    steps(&mut h, 3);
    h.get_by_label("Shop.pbip");
    assert!(h.query_by_label("Report 0.pbix").is_none());
    h.get_by_label("Remove from Recent").click();
    steps(&mut h, 3);
    assert!(
        !h.state()
            .prefs
            .recent
            .iter()
            .any(|r| r.path.ends_with("Shop.pbip"))
    );
    h.get_by_label_contains("No recent file matches");
    // pin: the pinned file goes first
    if let Some(b) = h.state_mut().backstage.as_mut() {
        b.query = "Report 999.pbix".into();
    }
    steps(&mut h, 3);
    h.get_by_label("Pin to the list").click();
    steps(&mut h, 3);
    assert!(h.state().prefs.recent[0].pinned && h.state().prefs.recent[0].path.ends_with("Report 999.pbix"));
    // everything listed: capped rows, no panic
    if let Some(b) = h.state_mut().backstage.as_mut() {
        b.query.clear();
    }
    steps(&mut h, 3);
    h.get_by_label_contains("more: refine the search");
}

/// The browser's Browse lists the workspace's Power BI files; opening one opens it in a new tab.
#[test]
fn the_browser_browses_the_workspace() {
    let mut shell = engine_shell(Caps::BROWSER, || report_session(1));
    shell.workspace = Some("ws1".into());
    shell.ws_tabs = true;
    let mut app = App::new("en-US", shell);
    app.open_server(crate::source::Server {
        url: String::new(),
        workspace: "ws1".into(),
        token: String::new(),
    });
    let mut h = harness(app, (1280.0, 800.0));
    steps(&mut h, 5);
    h.state_mut().backstage = Some(crate::backstage::Backstage::new(Page::Open));
    steps(&mut h, 3);
    assert!(h.query_by_label("Servers").is_none(), "no servers in the browser");
    h.get_by_label("Browse").click();
    steps(&mut h, 4);
    h.get_by_label("Sales.pbix");
    assert!(h.query_by_label("notes.txt").is_none(), "Power BI files only");
    h.get_by_label("Sales.pbix").click();
    steps(&mut h, 5);
    assert!(h.state().backstage.is_none());
    let (tabs, active) = h.state().doc_tabs();
    assert_eq!((tabs.len(), active), (2, 1), "{tabs:?}");
    // the tab's source is a document view of its own on that file (no file.open on the page's)
    assert_eq!(
        h.state().origin,
        Some(Origin::Workspace {
            ws: "ws1".into(),
            file: "reports/Sales.pbix".into()
        })
    );
    assert!(!h.state().sent.iter().any(|c| c.id == "file.open"));
    // kept in Recent as a workspace-relative name
    assert_eq!(h.state().prefs.recent[0].path, "reports/Sales.pbix");
    assert_eq!(h.state().prefs.recent[0].ws.as_deref(), Some("ws1"));
}

/// The page tabs: one per page, the + adds one and shows it, the menu's actions run their
/// commands and the next snapshot redraws the tabs.
#[test]
fn page_tab_actions_run_their_commands() {
    let mut h = engine_app(3, (1280.0, 800.0));
    assert_eq!(page_labels(&h), ["P1", "P2", "P3"]);
    // +: a new page, shown
    h.get_by_label("Add a page").click();
    steps(&mut h, 4);
    assert_eq!(page_labels(&h), ["P1", "P2", "P3", "Page 4"]);
    assert!(last_ids(&h).contains(&"page.add"));
    assert_eq!(h.state().active_page(), 3);
    // the menu: hide P2
    let tab = |h: &Harness<'_, App>, l: &str| page_tab(h, l).center();
    let p2 = tab(&h, "P2");
    right_click(&mut h, p2);
    h.get_by_label("Hide page").click();
    steps(&mut h, 4);
    let c = h.state().sent.last().cloned().unwrap();
    assert_eq!(
        (c.id, c.params["hidden"].clone(), c.params["type"].clone()),
        ("page.set_type", json!(true), json!("report"))
    );
    assert!(h.state().page_names()[1].2, "hidden");
    h.get_by_label("P2 (hidden)");
    // page type: tooltip
    let p3 = tab(&h, "P3");
    right_click(&mut h, p3);
    h.get_by_label_contains("Page type").hover();
    steps(&mut h, 3);
    h.get_by_label("Tooltip page").click();
    steps(&mut h, 4);
    let snap = h.state().loaded.clone().unwrap().snap;
    assert_eq!(snap["report"]["pages"][2]["page_type"], "tooltip");
    // duplicate: report.extract then report.apply
    let p1 = tab(&h, "P1");
    right_click(&mut h, p1);
    h.get_by_label("Duplicate").click();
    steps(&mut h, 5);
    let ids = last_ids(&h);
    assert_eq!(&ids[ids.len() - 2..], ["report.extract", "report.apply"]);
    assert_eq!(page_labels(&h).last().map(String::as_str), Some("P1 (2)"));
    // rename inline: F2 on the focused tab, type, Enter
    h.get_all_by_label("P1")
        .max_by(|a, b| a.rect().center().y.total_cmp(&b.rect().center().y))
        .unwrap()
        .focus();
    h.step();
    h.key_press(egui::Key::F2);
    steps(&mut h, 3);
    assert!(h.state().page_tabs.renaming.is_some());
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.event(Event::Text("Overview".into()));
    h.key_press(egui::Key::Enter);
    steps(&mut h, 4);
    assert_eq!(page_labels(&h)[0], "Overview");
    // delete asks first
    let p3 = tab(&h, "P3");
    right_click(&mut h, p3);
    h.get_by_label("Delete").click();
    steps(&mut h, 3);
    assert!(h.state().confirm.is_some());
    h.key_press(egui::Key::Enter);
    steps(&mut h, 4);
    assert!(!page_labels(&h).contains(&"P3".to_string()));
    assert!(last_ids(&h).contains(&"page.remove"));
}

/// Dragging a tab moves the page (page.move); the keyboard does too.
#[test]
fn dragging_a_page_tab_reorders_the_pages() {
    let mut h = engine_app(3, (1280.0, 800.0));
    let at = |h: &Harness<'_, App>, l: &str| page_tab(h, l);
    let (from, to) = (
        at(&h, "P1").center(),
        at(&h, "P3").right_center() + egui::vec2(4.0, 0.0),
    );
    h.hover_at(from);
    h.step();
    h.event(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    for k in 1..=10 {
        let p = from + (to - from) * (k as f32 / 10.0);
        h.event(Event::PointerMoved(p));
        h.step();
    }
    h.event(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    steps(&mut h, 4);
    let c = h.state().sent.last().cloned().unwrap();
    assert_eq!((c.id, c.params["index"].clone()), ("page.move", json!(2)));
    assert_eq!(page_labels(&h), ["P2", "P3", "P1"]);
    // the keyboard: focus P3's tab, Alt+Shift+← moves it left
    h.get_all_by_label("P3")
        .max_by(|a, b| a.rect().center().y.total_cmp(&b.rect().center().y))
        .unwrap()
        .focus();
    h.step();
    h.key_press_modifiers(Modifiers::ALT | Modifiers::SHIFT, egui::Key::ArrowLeft);
    steps(&mut h, 4);
    assert_eq!(page_labels(&h), ["P3", "P2", "P1"]);
}

/// 500 pages draw without a panic, and the last one can be reached.
#[test]
fn five_hundred_page_tabs_never_panic() {
    let mut snap = shop_snapshot().snap.as_ref().clone();
    let pages: Vec<Value> = (0..500)
        .map(|i| json!({"name": format!("p{i}"), "display_name": format!("Page {i} with a long name to clip at forty characters"), "width": 1280, "height": 720, "hidden": i % 3 == 0, "visuals": [], "issues": [{"severity": "warning"}]}))
        .collect();
    snap["report"]["pages"] = Value::Array(pages);
    let mut app = App::new("en-US", fixed(Caps::DESKTOP, Some(loaded(snap)))).with_sync_render();
    app.open_path(Path::new("big.pbip"));
    let mut h = harness(app, (1000.0, 700.0));
    steps(&mut h, 6);
    assert_eq!(h.state().page_names().len(), 500);
    h.get_by_label("Add a page");
}

/// Several files at once: a tab each, switching keeps each one's state, Ctrl+Tab, closing.
#[test]
fn document_tabs_open_switch_and_close() {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, || report_session(2)));
    app.open_path(Path::new("a.pbip"));
    let mut h = harness(app, (1280.0, 800.0));
    steps(&mut h, 4);
    h.state_mut().open_path(Path::new("b.pbip"));
    steps(&mut h, 4);
    let (tabs, active) = h.state().doc_tabs();
    assert_eq!(tabs.len(), 2);
    assert_eq!(active, 1);
    // a page added in b stays in b
    h.get_by_label("Add a page").click();
    steps(&mut h, 4);
    assert_eq!(h.state().page_names().len(), 3);
    h.key_press_modifiers(Modifiers::CTRL, egui::Key::Tab);
    steps(&mut h, 3);
    assert_eq!(h.state().active_doc(), 0);
    assert_eq!(h.state().page_names().len(), 2, "a is untouched");
    // opening a file already open switches to its tab
    h.state_mut().open_path(Path::new("b.pbip"));
    steps(&mut h, 3);
    assert_eq!((h.state().doc_tabs().0.len(), h.state().active_doc()), (2, 1));
    assert_eq!(h.state().page_names().len(), 3);
    // b has unsaved edits: closing asks
    h.get_by_label("Close b.pbip").click();
    steps(&mut h, 3);
    assert!(matches!(
        h.state().confirm,
        Some(crate::app::Confirm::CloseDoc(1))
    ));
    h.key_press(egui::Key::Enter);
    steps(&mut h, 3);
    assert_eq!(h.state().doc_tabs().0.len(), 1);
    assert_eq!(h.state().active_doc(), 0);
    // a middle click closes a's clean tab: the start screen
    let a = h
        .get_all_by_label("a.pbip")
        .map(|n| n.rect())
        .find(|r| r.center().y > 38.0 && r.center().y < 70.0)
        .map(|r| r.center())
        .unwrap();
    h.hover_at(a);
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: a,
            button: PointerButton::Middle,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    steps(&mut h, 3);
    assert!(!h.state().has_doc());
    assert_eq!(h.state().backstage.as_ref().map(|b| b.page), Some(Page::Home));
}

/// More than the cap: refused with a message, nothing replaced.
#[test]
fn the_number_of_open_documents_is_capped() {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, || report_session(1)));
    let mut h = harness(app_with(&mut app), (1280.0, 800.0));
    for i in 0..crate::shell::MAX_DOCS + 2 {
        h.state_mut().open_path(&PathBuf::from(format!("f{i}.pbip")));
        steps(&mut h, 2);
    }
    assert_eq!(h.state().doc_tabs().0.len(), crate::shell::MAX_DOCS);
    assert!(h.state().error.as_deref().unwrap_or("").contains("At most"));
}

fn app_with(app: &mut App) -> App {
    std::mem::replace(app, App::new("en-US", fixed(Caps::DESKTOP, None)))
}

/// The browser's tabs share the workspace's session: a tab with unsaved edits keeps it.
/// A session whose report has `pages` pages named `<prefix>1`, `<prefix>2`…
fn named_session(prefix: &str, pages: usize) -> Session {
    let mut s = report_session(pages);
    for i in 1..=pages {
        let name = s.report.as_ref().unwrap().pages[i - 1].name.clone();
        s.execute(
            "page.rename",
            &json!({"page": name, "display_name": format!("{prefix}{i}")}),
        )
        .unwrap();
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

/// The browser's shell on workspace `ws1`: the page's tab shows `P1`; each workspace file's tab
/// is a source of its own (a document view on the server), `a.pbix` with pages `A1 A2`,
/// `b.pbix` with `B1 B2 B3`; `full.pbix` is refused by the server's cap; the page's source says
/// the server allows `views_per_user` document views.
fn browser_views_shell(views_per_user: usize) -> Shell {
    let mut shell = Shell::new(Caps::BROWSER, move |o, link| {
        let session = match &o {
            Origin::Workspace { file, .. } if file == "full.pbix" => {
                link.send(Update::Refused(
                    "too many open files: at most 2 per user on this server; close a tab first".into(),
                ));
                return Running::Driven(Box::new(super::Idle));
            }
            Origin::Workspace { file, .. } if file == "a.pbix" => named_session("A", 2),
            Origin::Workspace { .. } => named_session("B", 3),
            _ => {
                link.send(Update::Limits { views_per_user });
                named_session("P", 1)
            }
        };
        let d = EngineDriver {
            session: RefCell::new(session),
            link,
            seq: Cell::new(0),
        };
        d.publish();
        Running::Driven(Box::new(d))
    });
    shell.workspace = Some("ws1".into());
    shell.ws_tabs = true;
    shell
}

fn browser_views_app(views_per_user: usize) -> Harness<'static, App> {
    let mut app = App::new("en-US", browser_views_shell(views_per_user));
    app.open_server(crate::source::Server {
        url: String::new(),
        workspace: "ws1".into(),
        token: String::new(),
    });
    let mut h = harness(app, (1280.0, 800.0));
    steps(&mut h, 4);
    h
}

/// The browser's document tabs are parallel views, as the desktop's: each workspace file's tab
/// has its own source and shows its own file; one tab's edits stay in it, switching with unsaved
/// edits is allowed (nothing is reopened), and no tab runs `file.open` on another's session.
#[test]
fn browser_tabs_show_different_files_at_once() {
    let mut h = browser_views_app(8);
    assert_eq!(page_labels(&h), ["P1"]);
    h.state_mut().open_ws_file("a.pbix");
    steps(&mut h, 4);
    h.state_mut().open_ws_file("b.pbix");
    steps(&mut h, 4);
    let (tabs, active) = h.state().doc_tabs();
    let names: Vec<&str> = tabs.iter().map(|t| t.0.as_str()).collect();
    assert_eq!(
        (names.as_slice(), active),
        (&["ws1 · ", "a.pbix", "b.pbix"][..], 2),
        "{tabs:?}"
    );
    assert_eq!(page_labels(&h), ["B1", "B2", "B3"]);
    // an edit in b.pbix's tab
    h.get_by_label("Add a page").click();
    steps(&mut h, 4);
    assert_eq!(page_labels(&h), ["B1", "B2", "B3", "Page 4"]);
    assert!(h.state().doc_tabs().0[2].1, "b.pbix has unsaved edits");
    // switching away with unsaved edits: allowed, a.pbix's tab shows its own file
    h.get_by_label("a.pbix").click();
    steps(&mut h, 4);
    assert_eq!(h.state().active_doc(), 1);
    assert_eq!(h.state().error, None);
    assert_eq!(page_labels(&h), ["A1", "A2"]);
    assert!(!h.state().doc_tabs().0[1].1, "a.pbix got b.pbix's edits");
    // an edit here, then back: each tab kept its own
    h.get_by_label("Add a page").click();
    steps(&mut h, 4);
    assert_eq!(page_labels(&h), ["A1", "A2", "Page 3"]);
    h.get_by_label("b.pbix").click();
    steps(&mut h, 4);
    assert_eq!(page_labels(&h), ["B1", "B2", "B3", "Page 4"]);
    h.state_mut().switch_doc(0);
    steps(&mut h, 4);
    assert_eq!(page_labels(&h), ["P1"]);
    let tabs = h.state().doc_tabs().0;
    assert!(!tabs[0].1 && tabs[1].1 && tabs[2].1, "{tabs:?}");
    // no tab reopened a file in another's session
    assert!(!h.state().sent.iter().any(|c| c.id == "file.open"));
}

/// The server's cap and refusals: the window opens at most the server's views per user beside
/// its first tab; a refused tab closes and the tab strip says why, until clicked.
#[test]
fn the_servers_view_limits_show_in_the_tab_strip() {
    let mut h = browser_views_app(1);
    assert_eq!(h.state().shell.max_docs, 2);
    h.state_mut().open_ws_file("full.pbix");
    steps(&mut h, 4);
    assert_eq!(h.state().doc_tabs().0.len(), 1, "the refused tab stayed open");
    let note = "too many open files: at most 2 per user";
    assert!(h.state().strip_note.as_deref().unwrap_or("").contains(note));
    h.get_by_label_contains(note);
    // the window's own cap, from the server's: one document view beside the page's tab
    h.state_mut().open_ws_file("a.pbix");
    steps(&mut h, 4);
    h.state_mut().open_ws_file("b.pbix");
    steps(&mut h, 4);
    assert_eq!(h.state().doc_tabs().0.len(), 2);
    assert!(
        h.state()
            .error
            .as_deref()
            .unwrap_or("")
            .contains("At most 2 files")
    );
    // a click dismisses the note
    h.get_by_label_contains(note).click();
    steps(&mut h, 3);
    assert_eq!(h.state().strip_note, None);
    assert!(h.query_by_label_contains(note).is_none());
}

/// The desktop's and the browser's frames are the same on the new screens too: the File view's
/// Info and Export pages, the page tabs and the document tabs (in the Report tab).
#[test]
fn the_new_screens_draw_the_same_frames_in_both_shells() {
    let snap = shop_snapshot();
    for dark in [false, true] {
        let mut desktop = shell_app(Caps::DESKTOP, &snap, "en-US");
        let mut browser = shell_app(Caps::BROWSER, &snap, "en-US");
        for page in [Page::Info, Page::Export] {
            for h in [&mut desktop, &mut browser] {
                if dark {
                    h.state_mut().set_theme(crate::theme::Brightness::Dark);
                }
                h.state_mut().tab = Tab::Report;
                h.state_mut().backstage = Some(crate::backstage::Backstage::new(page));
                // the same file.inspect answer in both (the fixed source answers nothing)
                h.state_mut().info = Some(Ok(json!({})));
                steps(h, 3);
            }
            let (a, b) = (frame(&desktop), frame(&browser));
            assert!(a.0 == b.0, "{page:?} dark={dark}: the meshes differ");
            assert!(a.1 == b.1, "{page:?}: the font atlases differ");
        }
    }
    // two document tabs (a local file beside the first on the desktop, a workspace file's
    // document view in the browser), then the tab strip's message of a refused tab
    let mut desktop = shell_app(Caps::DESKTOP, &snap, "en-US");
    let mut browser = shell_app(Caps::BROWSER, &snap, "en-US");
    desktop.state_mut().open_origin(Origin::Local {
        path: PathBuf::from("two.pbip"),
        lang: "en-US".into(),
    });
    browser.state_mut().open_origin(Origin::Workspace {
        ws: "ws1".into(),
        file: "two.pbip".into(),
    });
    for note in [
        None,
        Some("too many open files: at most 8 per user on this server"),
    ] {
        for h in [&mut desktop, &mut browser] {
            h.state_mut().backstage = None;
            h.state_mut().tab = Tab::Report;
            h.state_mut().ribbon_tab = crate::registry::Tab::Report;
            h.state_mut().strip_note = note.map(str::to_string);
            for _ in 0..200 {
                h.step();
                if h.state().shown.is_some() {
                    break;
                }
            }
            steps(h, 3);
            assert_eq!(h.state().doc_tabs().0.len(), 2);
        }
        let (a, b) = (frame(&desktop), frame(&browser));
        assert!(a.0 == b.0, "two tabs, note {note:?}: the meshes differ");
        assert!(a.1 == b.1, "two tabs, note {note:?}: the font atlases differ");
    }
}

pub(super) fn right_click(h: &mut Harness<'_, App>, pos: Pos2) {
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.step();
}

/// A shell whose source opens the origin's path with `file.open`, as the desktop's local source
/// does.
fn opening_shell() -> Shell {
    Shell::new(Caps::DESKTOP, move |o, link| {
        let mut s = Session::new();
        if let Origin::Local { path, .. } = &o {
            let r = s.execute("file.open", &json!({"path": path}));
            if let Err(e) = r {
                link.send(Update::Error(e));
            }
        }
        let d = EngineDriver {
            session: RefCell::new(s),
            link,
            seq: Cell::new(0),
        };
        d.publish();
        Running::Driven(Box::new(d))
    })
}

/// Ctrl+O on a `.pbids` (M11.36): the Open dialog's file opens as a new model with the
/// source's table.
#[test]
fn open_a_pbids_file_gives_a_new_model() {
    let dir = std::env::temp_dir().join(format!("bicraft-ui-pbids-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pbids = dir.join("Warehouse.pbids");
    std::fs::write(
        &pbids,
        r#"{"version":"0.1","connections":[{"details":{"protocol":"tds","address":{"server":"sql.example","database":"Stock"}},"mode":"Import"}]}"#,
    )
    .unwrap();
    let mut shell = opening_shell();
    let p2 = pbids.clone();
    shell.pick_file = Some(Box::new(move |_| Some(p2.clone())));
    let mut h = harness(App::new("en-US", shell), (1280.0, 800.0));
    steps(&mut h, 2);
    // what the Open button and Ctrl+O run once the picker returns
    h.state_mut().pick_and_open();
    steps(&mut h, 6);
    let snap = h
        .state()
        .loaded
        .as_ref()
        .map(|l| l.snap.to_string())
        .unwrap_or_default();
    assert!(
        snap.contains("Warehouse"),
        "the .pbids opened as a model with its table"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// File › Export › Redact a copy for sharing (M11.37): the button runs `file.redact` with the
/// Save dialog's path and the data option, and shows what was replaced.
#[test]
fn redact_a_copy_runs_file_redact() {
    let dir = std::env::temp_dir().join(format!("bicraft-ui-redact-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    {
        let mut s = report_session(1);
        s.execute(
            "table.add",
            &json!({"name": "Files", "partition": {"source_type": "m",
                "expression": "Folder.Files(\"\\\\\\\\filer-secret.example\\\\share\")"}}),
        )
        .unwrap();
        s.execute("project.new", &json!({"dir": src, "name": "Shop"}))
            .unwrap();
    }
    let saved = src.join("Shop.pbip");
    let p2 = saved.clone();
    let mut shell = engine_shell(Caps::DESKTOP, move || {
        let mut s = Session::new();
        s.execute("file.open", &json!({"path": p2})).unwrap();
        s
    });
    let out = dir.join("red").join("Shop (redacted).pbip");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let o2 = out.clone();
    shell.pick_save = Some(Box::new(move |_, _, _, _| Some(o2.clone())));
    let mut app = App::new("en-US", shell);
    app.open_path(&saved);
    let mut h = harness(app, (1280.0, 800.0));
    steps(&mut h, 5);
    let mut bs = crate::backstage::Backstage::new(Page::Export);
    bs.task = Task::Redact;
    h.state_mut().backstage = Some(bs);
    steps(&mut h, 3);
    h.get_by_label("Replace the stored text values too (PBIX)")
        .click();
    steps(&mut h, 2);
    assert!(h.state().backstage.as_ref().unwrap().redact_data);
    h.get_by_label("Choose where…").click();
    steps(&mut h, 4);
    let sent = h.state().sent.last().unwrap();
    assert_eq!(sent.id, "file.redact");
    assert_eq!(sent.params["data"], true);
    assert!(out.exists(), "the redacted project was written");
    h.get_by_label_contains("values replaced");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two files opened before the first frame (the command line, a restored session): both sources
/// start, the first one in the background.
#[test]
fn files_opened_before_the_first_frame_all_start() {
    let mut app = App::new("en-US", engine_shell(Caps::DESKTOP, || report_session(2)));
    app.open_path(Path::new("one.pbip"));
    app.open_path(Path::new("two.pbip"));
    let mut h = harness(app, (1280.0, 800.0));
    steps(&mut h, 4);
    assert_eq!(h.state().active_doc(), 1);
    h.state_mut().switch_doc(0);
    steps(&mut h, 2);
    assert!(h.state().loaded.is_some(), "the first file's source started too");
}
