//! Home › Get data and Home › Enter data through the accessibility tree (egui_kittest, no GPU),
//! on an engine session (ROADMAP M11, gaps G58 and G59): the source gallery, Connect running
//! `source.navigate`, the Navigator's check boxes and `query.preview`, Load running one
//! `source.load`; the Enter data grid with a TSV paste, a column type and Load running
//! `table.enter_data`; the Data pane's Edit data reading `table.entered_data` back and OK
//! replacing the table.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use egui::accesskit::Role;
use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::backstage_tests::{engine_shell, steps};
use super::harness;
use crate::App;
use crate::registry::Tab as RTab;
use crate::shell::Caps;

/// A folder of our own with a CSV file and a JSON file.
fn sources(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bicraft-ui-get-data-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("sales.csv"), "Product,Amount\nBike,100\nCar,1000\n").unwrap();
    std::fs::write(
        dir.join("shop.json"),
        r#"{"shop": "North", "orders": [{"id": 1, "qty": 2}, {"id": 2, "qty": 5}]}"#,
    )
    .unwrap();
    dir
}

/// An empty model, and `Prices` made by Enter data.
fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.enter_data",
            json!({"name": "Prices", "columns": ["Item", "Price"], "rows": [["Pen", "2"], ["Ink", "7"]]}),
        ),
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    s.dirty = false;
    s.report_dirty = false;
    s
}

pub(super) fn app(lang: &str) -> Harness<'static, App> {
    let mut app = App::new(lang, engine_shell(Caps::DESKTOP, session)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1400.0, 900.0));
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().loaded.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    h.state_mut().ribbon_tab = RTab::Home;
    steps(&mut h, 3);
    h
}

pub(super) fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).click();
    steps(h, 3);
}

pub(super) fn text_input(r: Role) -> bool {
    matches!(
        r,
        Role::TextInput | Role::PasswordInput | Role::MultilineTextInput
    )
}

pub(super) fn type_into(h: &mut Harness<'_, App>, label: &str, text: &str) {
    h.get_by(|n| text_input(n.role()) && n.label().as_deref() == Some(label))
        .click();
    steps(h, 2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by(|n| text_input(n.role()) && n.label().as_deref() == Some(label))
        .type_text(text);
    steps(h, 2);
}

pub(super) fn last_sent(h: &Harness<'_, App>, id: &str) -> Option<Value> {
    h.state()
        .sent
        .iter()
        .rev()
        .find(|c| c.id == id)
        .map(|c| c.params.clone())
}

fn count_sent(h: &Harness<'_, App>, id: &str) -> usize {
    h.state().sent.iter().filter(|c| c.id == id).count()
}

/// Steps until `done` holds.
pub(super) fn until(h: &mut Harness<'_, App>, what: &str, done: impl Fn(&App) -> bool) {
    for _ in 0..400 {
        if done(h.state()) {
            steps(h, 2);
            return;
        }
        h.step();
    }
    panic!("never: {what} (status: {:?})", h.state().error);
}

fn tables(h: &Harness<'_, App>) -> Vec<String> {
    h.state()
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/model/tables").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|t| t.get("name").and_then(Value::as_str).map(str::to_string))
        .collect()
}

#[test]
fn get_data_connects_a_text_file_previews_it_and_loads_it() {
    let dir = sources("csv");
    let mut h = app("en-US");
    click(&mut h, "Get data");
    assert!(h.state().cmd.get_data.is_some(), "the gallery is open");
    assert!(h.state().form.is_none(), "no form: the gallery");
    assert_eq!(
        count_sent(&h, "source.navigate"),
        0,
        "a click opens the dialog only"
    );
    click(&mut h, "Text/CSV");
    type_into(&mut h, "File path", &dir.join("sales.csv").display().to_string());
    click(&mut h, "Connect");
    assert_eq!(
        last_sent(&h, "source.navigate").map(|p| p["kind"].clone()),
        Some(json!("csv"))
    );
    // one item: checked and previewed at once
    until(&mut h, "the preview", |a| {
        a.cmd
            .get_data
            .as_ref()
            .and_then(|d| d.nav.as_ref())
            .is_some_and(|n| matches!(n.preview, Some(Ok(_))))
    });
    let n = h
        .state()
        .cmd
        .get_data
        .as_ref()
        .and_then(|d| d.nav.clone())
        .unwrap();
    assert_eq!(n.checked.len(), 1);
    match &n.preview {
        Some(Ok(p)) => {
            assert_eq!(p.columns.first().map(|c| c.0.as_str()), Some("Product"));
            assert_eq!(p.rows.len(), 2);
        }
        other => panic!("{other:?}"),
    }
    h.get_by_label("Bike");
    // Transform data is there, enabled with an item ticked (the last: the ribbon has one too)
    let t = h
        .get_all_by_label("Transform data")
        .last()
        .expect("Transform data");
    assert!(!t.accesskit_node().is_disabled());
    // the window keeps its size (a full-height separator in a row made it grow every frame)
    let window = |h: &Harness<'_, App>| {
        h.ctx
            .memory(|m| m.area_rect(egui::Id::unique("bicraft-get-data")))
    };
    let before = window(&h);
    steps(&mut h, 3);
    assert_eq!(window(&h), before, "the Navigator grows");
    click(&mut h, "Load");
    let p = last_sent(&h, "source.load").expect("Load runs source.load");
    assert_eq!(p["tables"][0]["name"], "sales", "{p}");
    until(&mut h, "the dialog closes", |a| a.cmd.get_data.is_none());
    until(&mut h, "the table is in the model", |a| {
        a.loaded.as_ref().is_some_and(|l| {
            l.snap
                .pointer("/model/tables")
                .and_then(Value::as_array)
                .is_some_and(|t| t.iter().any(|t| t["name"] == "sales"))
        })
    });
    assert_eq!(count_sent(&h, "source.load"), 1, "one batch");
}

#[test]
fn the_navigator_ticks_and_previews_items_and_goes_back() {
    let dir = sources("json");
    let mut h = app("en-US");
    click(&mut h, "Get data");
    click(&mut h, "JSON");
    type_into(&mut h, "File path", &dir.join("shop.json").display().to_string());
    click(&mut h, "Connect");
    until(&mut h, "the items", |a| {
        a.cmd
            .get_data
            .as_ref()
            .and_then(|d| d.nav.as_ref())
            .is_some_and(|n| n.items.len() > 1)
    });
    // several items: none ticked, Load disabled
    assert!(h.get_by_label("Load").accesskit_node().is_disabled());
    h.get_by_label("Load orders").click();
    steps(&mut h, 3);
    let n = h
        .state()
        .cmd
        .get_data
        .as_ref()
        .and_then(|d| d.nav.clone())
        .unwrap();
    let orders = n.items.iter().position(|i| i.name == "orders").unwrap();
    assert!(n.checked.contains(&orders), "{:?}", n.checked);
    // a click on the name previews it
    let before = count_sent(&h, "query.preview");
    h.get_by(|n| n.role() == Role::Button && n.label().is_some_and(|l| l.starts_with("orders  (")))
        .click();
    steps(&mut h, 3);
    assert_eq!(count_sent(&h, "query.preview"), before + 1);
    until(&mut h, "the preview", |a| {
        a.cmd
            .get_data
            .as_ref()
            .and_then(|d| d.nav.as_ref())
            .is_some_and(|n| matches!(n.preview, Some(Ok(_))))
    });
    h.get_by_label("qty");
    // Back: the gallery again, with its path
    click(&mut h, "Back");
    let d = h.state().cmd.get_data.clone().unwrap();
    assert!(d.nav.is_none());
    assert!(d.path.ends_with("shop.json"));
    // Escape closes it, nothing loaded
    h.key_press(Key::Escape);
    steps(&mut h, 2);
    assert!(h.state().cmd.get_data.is_none());
    assert_eq!(count_sent(&h, "source.load"), 0);
}

#[test]
fn enter_data_takes_a_paste_and_a_type_and_loads_one_table() {
    let mut h = app("en-US");
    click(&mut h, "Enter data");
    assert!(h.state().cmd.enter_data.is_some(), "the grid is open");
    assert_eq!(
        count_sent(&h, "table.enter_data"),
        0,
        "a click opens the grid only"
    );
    type_into(&mut h, "Name", "Stock");
    // a paste of three lines from a spreadsheet
    h.event(Event::Paste("Item\tQty\nPen\t3\nInk\t12\n".into()));
    steps(&mut h, 3);
    let g = h.state().cmd.enter_data.clone().unwrap();
    assert_eq!(g.columns.len(), 2);
    assert_eq!(g.rows.len(), 3, "{:?}", g.rows);
    click(&mut h, "Use first row as headers");
    let g = h.state().cmd.enter_data.clone().unwrap();
    assert_eq!(
        g.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["Item", "Qty"]
    );
    assert_eq!(g.rows.len(), 2);
    // Qty as a decimal number
    click(&mut h, "Column 2 type");
    click(&mut h, "Decimal number");
    // a row added, then removed
    click(&mut h, "Add row");
    assert_eq!(h.state().cmd.enter_data.as_ref().map(|g| g.rows.len()), Some(3));
    click(&mut h, "Remove row 3");
    assert_eq!(h.state().cmd.enter_data.as_ref().map(|g| g.rows.len()), Some(2));
    click(&mut h, "Load");
    let p = last_sent(&h, "table.enter_data").expect("Load runs table.enter_data");
    assert_eq!(p["name"], "Stock", "{p}");
    assert_eq!(p["columns"][1], json!({"name": "Qty", "type": "number"}), "{p}");
    assert_eq!(p["rows"], json!([["Pen", "3"], ["Ink", "12"]]), "{p}");
    until(&mut h, "the grid closes", |a| a.cmd.enter_data.is_none());
    until(&mut h, "Stock is in the model", |a| {
        a.loaded.as_ref().is_some_and(|l| {
            l.snap
                .pointer("/model/tables")
                .and_then(Value::as_array)
                .is_some_and(|t| t.iter().any(|t| t["name"] == "Stock"))
        })
    });
    assert!(tables(&h).contains(&"Prices".to_string()));
}

/// Ctrl+V as both shells deliver it: the OS or page clipboard's text in one `Paste` event, line
/// ends `\n`, a spreadsheet's copy ending in a line break.
#[test]
fn ctrl_v_fills_the_grid_from_the_selected_cell_and_grows_it() {
    let mut h = app("en-US");
    let status = h.state().status.clone();
    click(&mut h, "Enter data");
    // nothing focused, over the Report view: the grid takes the paste, not the canvas
    h.key_press_modifiers(Modifiers::COMMAND, Key::V);
    h.event(Event::Paste("a\tb\nc\td\n".into()));
    steps(&mut h, 6);
    assert_eq!(h.state().status, status, "the canvas did not read the paste");
    let g = h.state().cmd.enter_data.clone().unwrap();
    assert_eq!(g.rows, [["a", "b"], ["c", "d"]]);
    // from the selected cell, three columns wide and two rows deep: the grid grows
    h.get_by_label("Row 2, Column2").click();
    steps(&mut h, 2);
    h.event(Event::Paste("x\ty\tz\nu\tv\tw\n".into()));
    steps(&mut h, 3);
    let g = h.state().cmd.enter_data.clone().unwrap();
    assert_eq!(g.columns.len(), 4);
    assert_eq!(
        g.rows,
        [
            vec!["a", "b", "", ""],
            vec!["c", "x", "y", "z"],
            vec!["", "u", "v", "w"]
        ]
    );
    // one cell copied from a spreadsheet goes into the edited cell, without its line break
    type_into(&mut h, "Row 1, Column1", "");
    h.event(Event::Paste("Pen\n".into()));
    steps(&mut h, 3);
    let g = h.state().cmd.enter_data.clone().unwrap();
    assert_eq!(
        g.rows.first().and_then(|r| r.first()).map(String::as_str),
        Some("Pen")
    );
    // a single word into the Name box goes into the box
    type_into(&mut h, "Name", "");
    h.event(Event::Paste("Stock\n".into()));
    steps(&mut h, 3);
    let g = h.state().cmd.enter_data.clone().unwrap();
    assert_eq!(g.name, "Stock");
    assert_eq!(g.rows.len(), 3, "the grid kept its rows");
}

#[test]
fn edit_data_reads_the_table_back_and_replaces_it() {
    let mut h = app("en-US");
    h.state_mut().show_tab(crate::app::Tab::Report);
    steps(&mut h, 3);
    h.get_by_label("Prices").click_secondary();
    steps(&mut h, 3);
    click(&mut h, "Edit data");
    assert_eq!(
        last_sent(&h, "table.entered_data"),
        Some(json!({"name": "Prices"}))
    );
    until(&mut h, "the grid is filled", |a| {
        a.cmd
            .enter_data
            .as_ref()
            .is_some_and(|g| !g.busy && !g.rows.is_empty())
    });
    let g = h.state().cmd.enter_data.clone().unwrap();
    assert_eq!(g.edit.as_deref(), Some("Prices"));
    assert_eq!(
        g.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["Item", "Price"]
    );
    assert_eq!(g.rows, [["Pen", "2"], ["Ink", "7"]]);
    type_into(&mut h, "Row 2, Price", "8");
    click(&mut h, "OK");
    let p = last_sent(&h, "table.enter_data").expect("OK runs table.enter_data");
    assert_eq!(p["replace"], true, "{p}");
    assert_eq!(p["rows"][1], json!(["Ink", "8"]), "{p}");
    until(&mut h, "the grid closes", |a| a.cmd.enter_data.is_none());
}

#[test]
fn the_dialogs_speak_french() {
    let mut h = app("fr-FR");
    click(&mut h, "Obtenir des données");
    h.get_by_label("Texte/CSV");
    h.get_by_label("Se connecter");
    // the gallery's entry (the ribbon has a button of the same name)
    h.query_all_by_label("Entrer des données").last().unwrap().click();
    steps(&mut h, 3);
    assert!(h.state().cmd.get_data.is_none());
    assert!(
        h.state().cmd.enter_data.is_some(),
        "the gallery's Enter data opens the grid"
    );
    h.get_by_label("Ajouter une colonne");
    h.get_by_label("Colonne 1 type");
}

/// The answers of the fake `Sales` database of the SQL Server tests.
fn sql_answer(q: &str) -> Option<Vec<u8>> {
    use bicraft_sqlserver::fake::tok;
    let orders = |rows: &[(i32, i32)]| {
        let mut t = tok::columns(&[("id", tok::INT), ("qty", tok::INT)]);
        for (id, qty) in rows {
            t.extend_from_slice(&tok::row(&[&tok::int(Some(*id)), &tok::int(Some(*qty))]));
        }
        t
    };
    if q == bicraft_sqlserver::sql::objects(None) {
        let mut t = tok::columns(&[
            ("name", tok::NVARCHAR),
            ("name", tok::NVARCHAR),
            ("type", tok::NVARCHAR),
        ]);
        for (s, o, k) in [("dbo", "Orders", "U "), ("dbo", "Recent", "V ")] {
            t.extend_from_slice(&tok::row(&[
                &tok::nvarchar(Some(s)),
                &tok::nvarchar(Some(o)),
                &tok::nvarchar(Some(k)),
            ]));
        }
        return Some(t);
    }
    Some(match q {
        "SELECT TOP (0) * FROM [dbo].[Orders]" | "SELECT TOP (0) [id], [qty] FROM [dbo].[Orders]" => {
            orders(&[])
        }
        "SELECT [id], [qty] FROM [dbo].[Orders]" => orders(&[(1, 3), (2, 5)]),
        _ => return None,
    })
}

/// The app on a session that trusts the fake server's test CA.
fn sql_app(lang: &str) -> (bicraft_sqlserver::fake::FakeServer, Harness<'static, App>) {
    use bicraft_sqlserver::fake;
    let srv = fake::start(fake::Script::new(
        "reader",
        "s3cret!",
        std::sync::Arc::new(sql_answer),
    ))
    .unwrap();
    let make = || {
        let mut s = session();
        s.sql_settings = Some(bicraft_engine::SqlSettings {
            extra_roots: bicraft_sqlserver::tls::pem_certificates(fake::CA_PEM).unwrap(),
            ..bicraft_engine::SqlSettings::default()
        });
        s
    };
    let mut app = App::new(lang, engine_shell(Caps::DESKTOP, make)).with_sync_render();
    app.open_path(Path::new("shop.pbip"));
    let mut h = harness(app, (1400.0, 900.0));
    for _ in 0..200 {
        h.step();
        if h.state().shown.is_some() && h.state().loaded.is_some() {
            break;
        }
    }
    steps(&mut h, 2);
    h.state_mut().ribbon_tab = RTab::Home;
    steps(&mut h, 3);
    (srv, h)
}

#[test]
fn get_data_sql_server_logs_in_lists_previews_and_loads() {
    let (srv, mut h) = sql_app("en-US");
    click(&mut h, "Get data");
    click(&mut h, "SQL Server database");
    // Connect waits for a server and a database
    assert!(h.get_by_label("Connect").accesskit_node().is_disabled());
    type_into(&mut h, "Server", &srv.server());
    type_into(&mut h, "Database", "Sales");
    type_into(&mut h, "User name", "reader");
    type_into(&mut h, "Password", "s3cret!");
    click(&mut h, "Connect");
    // the login goes to the session, then the Navigator lists the tables and views
    let c = last_sent(&h, "source.credentials").expect("Connect stores the login");
    assert_eq!(c["user"], "reader", "{c}");
    assert_eq!(c["password"], Value::Null, "the history keeps no password: {c}");
    let d = h.state().cmd.get_data.clone().unwrap();
    assert!(
        d.sql.password.is_empty(),
        "the password field is emptied once sent"
    );
    assert!(!format!("{d:?}").contains("s3cret!"));
    until(&mut h, "the items", |a| {
        a.cmd
            .get_data
            .as_ref()
            .and_then(|d| d.nav.as_ref())
            .is_some_and(|n| n.items.len() == 2)
    });
    let p = last_sent(&h, "source.navigate").unwrap();
    assert_eq!(p["kind"], "sqlserver", "{p}");
    assert_eq!(p["database"], "Sales", "{p}");
    h.get_by(|n| n.role() == Role::Button && n.label().as_deref() == Some("dbo.Recent  (view)"));
    h.get_by(|n| n.role() == Role::Button && n.label().as_deref() == Some("dbo.Orders  (table)"))
        .click();
    steps(&mut h, 3);
    let p = last_sent(&h, "query.preview").unwrap();
    assert_eq!(p["allow_connectors"], json!(["sqlserver"]), "{p}");
    until(&mut h, "the preview", |a| {
        a.cmd
            .get_data
            .as_ref()
            .and_then(|d| d.nav.as_ref())
            .is_some_and(|n| matches!(n.preview, Some(Ok(_))))
    });
    h.get_by_label("qty");
    h.get_by_label("Load dbo.Orders").click();
    steps(&mut h, 3);
    click(&mut h, "Load");
    let p = last_sent(&h, "source.load").expect("Load runs source.load");
    assert_eq!(p["allow_connectors"], json!(["sqlserver"]), "{p}");
    assert_eq!(p["tables"][0]["name"], "dbo.Orders", "{p}");
    until(&mut h, "the dialog closes", |a| a.cmd.get_data.is_none());
    until(&mut h, "the table is in the model", |a| {
        a.loaded.as_ref().is_some_and(|l| {
            l.snap
                .pointer("/model/tables")
                .and_then(Value::as_array)
                .is_some_and(|t| t.iter().any(|t| t["name"] == "dbo.Orders"))
        })
    });
    assert!(srv.seen().logins.iter().all(|(u, _)| u == "reader"));
}

#[test]
fn a_refused_login_stays_in_the_gallery_and_the_palette_opens_it() {
    let (srv, mut h) = sql_app("en-US");
    // the palette's Data source credentials without a server: the gallery, on SQL Server
    assert!(crate::get_data::intercept(
        h.state_mut(),
        "source.credentials",
        &json!({})
    ));
    steps(&mut h, 3);
    assert_eq!(
        h.state().cmd.get_data.as_ref().map(|d| d.source),
        Some(crate::get_data::Source::Sql)
    );
    type_into(&mut h, "Server", &srv.server());
    type_into(&mut h, "Database", "Sales");
    type_into(&mut h, "User name", "reader");
    type_into(&mut h, "Password", "wrong");
    click(&mut h, "Connect");
    until(&mut h, "the refusal", |a| {
        a.cmd
            .get_data
            .as_ref()
            .is_some_and(|d| !d.busy && d.problem.is_some())
    });
    let d = h.state().cmd.get_data.clone().unwrap();
    assert!(d.nav.is_none(), "still the gallery");
    let problem = d.problem.unwrap_or_default();
    assert!(
        problem.contains("Login failed") || problem.contains("credentials"),
        "{problem}"
    );
}

#[test]
fn the_sql_server_entry_speaks_french() {
    let (_srv, mut h) = sql_app("fr-FR");
    click(&mut h, "Obtenir des données");
    click(&mut h, "Base de données SQL Server");
    h.get_by(|n| n.role() == Role::TextInput && n.label().as_deref() == Some("Serveur"));
    h.get_by(|n| n.role() == Role::PasswordInput && n.label().as_deref() == Some("Mot de passe"));
    h.get_by(|n| n.role() == Role::MultilineTextInput && n.label().as_deref() == Some("Instruction SQL"));
}

/// Whether the Navigator lists `n` items.
pub(super) fn listed(a: &App, n: usize) -> bool {
    a.cmd
        .get_data
        .as_ref()
        .and_then(|d| d.nav.as_ref())
        .is_some_and(|nav| nav.items.len() == n)
}

fn previewed(a: &App) -> bool {
    a.cmd
        .get_data
        .as_ref()
        .and_then(|d| d.nav.as_ref())
        .is_some_and(|n| matches!(n.preview, Some(Ok(_))))
}

pub(super) fn has_table(a: &App, name: &str) -> bool {
    a.loaded.as_ref().is_some_and(|l| {
        l.snap
            .pointer("/model/tables")
            .and_then(Value::as_array)
            .is_some_and(|t| t.iter().any(|t| t["name"] == name))
    })
}

#[test]
fn get_data_web_reads_a_json_url_previews_and_loads() {
    let srv = bicraft_webdata::fake::start_shop().unwrap();
    let mut h = app("en-US");
    click(&mut h, "Get data");
    click(&mut h, "Web");
    assert!(h.get_by_label("Connect").accesskit_node().is_disabled());
    type_into(&mut h, "URL", &srv.url("/data.json"));
    click(&mut h, "Connect");
    let p = last_sent(&h, "source.navigate").expect("Connect runs source.navigate");
    assert_eq!(p, json!({"kind": "web", "url": srv.url("/data.json")}));
    until(&mut h, "the item", |a| listed(a, 1));
    // one item: checked and previewed at once
    until(&mut h, "the preview", previewed);
    let p = last_sent(&h, "query.preview").unwrap();
    assert_eq!(p["allow_connectors"], json!(["web"]), "{p}");
    h.get_by_label("score");
    click(&mut h, "Load");
    let p = last_sent(&h, "source.load").expect("Load runs source.load");
    assert_eq!(p["allow_connectors"], json!(["web"]), "{p}");
    assert_eq!(p["tables"][0]["name"], "data", "{p}");
    until(&mut h, "the dialog closes", |a| a.cmd.get_data.is_none());
    until(&mut h, "the table is in the model", |a| has_table(a, "data"));
}

#[test]
fn get_data_odata_lists_entity_sets_previews_and_loads() {
    let srv = bicraft_webdata::fake::start_shop().unwrap();
    let mut h = app("en-US");
    click(&mut h, "Get data");
    click(&mut h, "OData feed");
    type_into(&mut h, "Feed URL", &srv.url("/odata/"));
    click(&mut h, "Connect");
    until(&mut h, "the entity sets", |a| listed(a, 2));
    h.get_by(|n| n.role() == Role::Button && n.label().as_deref() == Some("Categories  (table)"));
    h.get_by(|n| n.role() == Role::Button && n.label().as_deref() == Some("Products  (table)"))
        .click();
    steps(&mut h, 3);
    let p = last_sent(&h, "query.preview").unwrap();
    assert_eq!(p["allow_connectors"], json!(["odata"]), "{p}");
    until(&mut h, "the preview", previewed);
    h.get_by_label("InStock");
    h.get_by_label("Load Products").click();
    steps(&mut h, 3);
    click(&mut h, "Load");
    let p = last_sent(&h, "source.load").expect("Load runs source.load");
    assert_eq!(p["allow_connectors"], json!(["odata"]), "{p}");
    until(&mut h, "the dialog closes", |a| a.cmd.get_data.is_none());
    until(&mut h, "the table is in the model", |a| has_table(a, "Products"));
}

#[test]
fn a_web_page_or_a_bad_address_stays_in_the_gallery() {
    let srv = bicraft_webdata::fake::start_shop().unwrap();
    let mut h = app("en-US");
    click(&mut h, "Get data");
    click(&mut h, "Web");
    type_into(&mut h, "URL", "example.com/data.json");
    click(&mut h, "Connect");
    let d = h.state().cmd.get_data.clone().unwrap();
    assert!(d.problem.is_some_and(|p| p.contains("http")));
    assert_eq!(count_sent(&h, "source.navigate"), 0);
    type_into(&mut h, "URL", &srv.url("/page.html"));
    click(&mut h, "Connect");
    until(&mut h, "the refusal", |a| {
        a.cmd
            .get_data
            .as_ref()
            .is_some_and(|d| !d.busy && d.problem.is_some())
    });
    let d = h.state().cmd.get_data.clone().unwrap();
    assert!(d.nav.is_none(), "still the gallery");
    assert!(d.problem.unwrap_or_default().contains("HTML"));
}

#[test]
fn the_web_entries_speak_french() {
    let mut h = app("fr-FR");
    click(&mut h, "Obtenir des données");
    click(&mut h, "Flux OData");
    h.get_by(|n| n.role() == Role::TextInput && n.label().as_deref() == Some("URL du flux"));
    click(&mut h, "Web");
    h.get_by(|n| n.role() == Role::TextInput && n.label().as_deref() == Some("URL"));
    h.get_by_label("Se connecter");
}
