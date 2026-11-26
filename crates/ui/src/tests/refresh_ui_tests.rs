//! Refreshing an open PBIX from the app (ROADMAP M11.18) on an engine session: F5 and Home ›
//! Refresh run `model.refresh` (a reload stays F5's job without a refreshable file), the Data
//! pane's and the model diagram's table menu "Refresh data" names the table, and the answer
//! window shows the spinner, then a summary line and the per-table grid; the same in both shells.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use crate::App;
use crate::model_view::Sel;
use crate::shell::Caps;

/// A PBIX built by the engine (one `#table` import table, a measure, a report page) in a folder
/// of its own.
fn pbix(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bicraft-ui-refresh-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("shop.pbix");
    let sales = "let Source = #table(type table [Category = text, Amount = number], \
                 {{\"Wheels\", 100}, {\"Toys\", 5}, {\"Games\", 20}}) in Source";
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        ("table.import", json!({"name": "Sales", "m": sales})),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
        ("file.save_as", json!({"path": path})),
    ] {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    path
}

fn open(caps: Caps, path: &Path) -> Harness<'static, App> {
    let file = path.to_path_buf();
    let make = move || {
        let mut s = Session::new();
        s.execute("file.open", &json!({"path": file})).unwrap();
        s
    };
    let mut app = App::new("en-US", engine_shell(caps, make)).with_sync_render();
    app.open_path(path);
    let mut h = super::harness(app, (1400.0, 860.0));
    for _ in 0..200 {
        h.step();
        if h.state().loaded.is_some() && h.state().shown.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    h
}

fn press(h: &mut Harness<'_, App>, key: Key) {
    h.key_press(key);
    steps(h, 2);
}

/// The answer the window holds once the refresh is done.
fn answer(h: &mut Harness<'_, App>) -> Value {
    for _ in 0..400 {
        if let Some(("model.refresh", Some(r))) = &h.state().cmd.result {
            return r.clone().unwrap_or_else(|e| panic!("the refresh failed: {e}"));
        }
        h.step();
    }
    panic!("no refresh answer: {:?}", h.state().cmd.result);
}

fn last_refresh(h: &Harness<'_, App>) -> Option<Value> {
    h.state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == "model.refresh")
        .map(|c| c.params.clone())
}

#[test]
fn f5_refreshes_an_open_pbix_and_shows_the_tables() {
    let path = pbix("f5");
    let mut h = open(Caps::DESKTOP, &path);
    assert!(crate::ribbon::state(h.state(), "model.refresh").is_ok());
    press(&mut h, Key::F5);
    assert_eq!(last_refresh(&h), Some(json!({})), "F5 is model.refresh on a PBIX");
    let a = answer(&mut h);
    assert_eq!(a["format"], "pbix", "{a}");
    assert_eq!(a["tables"][0]["table"], "Sales", "{a}");
    assert_eq!(a["tables"][0]["rows"], 3, "{a}");
    steps(&mut h, 3);
    // the window: the summary line and the grid's table column
    let line = crate::ribbon::refresh_summary(&a, crate::i18n::Lang::En);
    assert!(line.starts_with("✓ 1 tables refreshed, 0 kept"), "{line}");
    h.get_by_label(&line);
    // Escape closes it
    press(&mut h, Key::Escape);
    assert!(h.state().cmd.result.is_none());
}

#[test]
fn the_refresh_shows_its_progress_while_it_runs() {
    let path = pbix("progress");
    let mut h = open(Caps::DESKTOP, &path);
    let c = crate::registry::find("model.refresh").unwrap();
    crate::ribbon::send(h.state_mut(), c, json!({}));
    // before the answer comes back: the window with its spinner and the progress line
    assert!(matches!(h.state().cmd.result, Some(("model.refresh", None))));
    answer(&mut h);
}

#[test]
fn the_data_pane_table_menu_refreshes_that_table() {
    let path = pbix("menu");
    let mut h = open(Caps::DESKTOP, &path);
    h.state_mut().show_tab(crate::app::Tab::Report);
    steps(&mut h, 3);
    h.get_by_label("Sales").click_secondary();
    steps(&mut h, 3);
    h.get_by_label("Refresh data").click();
    steps(&mut h, 3);
    assert_eq!(last_refresh(&h), Some(json!({"tables": ["Sales"]})));
    let a = answer(&mut h);
    assert_eq!(a["tables"].as_array().map(Vec::len), Some(1), "{a}");
}

#[test]
fn the_diagram_table_menu_names_the_table() {
    let sel = Sel::Table("Sales".into());
    assert!(crate::menus::model_items(Some(&sel)).contains(&"model.refresh"));
    assert_eq!(
        crate::menus::item_params("model.refresh", Some(&sel)),
        json!({"tables": ["Sales"]})
    );
    assert_eq!(crate::menus::item_params("table.rename", Some(&sel)), json!({}));
    let c = crate::registry::find("model.refresh").unwrap();
    assert_eq!(crate::menus::item_text(c, crate::i18n::Lang::En), "Refresh data");
    assert_eq!(
        crate::menus::item_text(c, crate::i18n::Lang::Fr),
        "Actualiser les données"
    );
}

/// Both shells refresh on F5 (the browser shell keeps F5 for the app).
#[test]
fn both_shells_refresh_the_same_way() {
    let path = pbix("shells");
    let mut desktop = open(Caps::DESKTOP, &path);
    let mut browser = open(Caps::BROWSER, &path);
    for h in [&mut desktop, &mut browser] {
        press(h, Key::F5);
        assert_eq!(last_refresh(h), Some(json!({})));
        answer(h);
    }
}
