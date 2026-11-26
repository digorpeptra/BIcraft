//! Screenshots of Home › Get data and Home › Enter data drawn with wgpu (`--features
//! gpu-tests`), saved into `BICRAFT_DESKTOP_SHOTS` when set: the source gallery, the Navigator
//! with a JSON file's items and a preview, the Enter data grid after a paste, and Edit data of a
//! typed-in table, in English (light and dark) and French.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use bicraft_ui::enter_data;
use bicraft_ui::get_data::{self, Source};
use bicraft_ui::registry::Tab as RTab;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::{Temp, build_shop, ready, wait};
use crate::App;

fn save(h: &mut Harness<'_, App>, dir: &Option<PathBuf>, name: &str) {
    let img = h.render().unwrap();
    assert!(img.width() > 0 && img.height() > 0);
    let Some(dir) = dir else { return };
    std::fs::create_dir_all(dir).unwrap();
    let size = tiny_skia::IntSize::from_wh(img.width(), img.height()).unwrap();
    let pm = tiny_skia::Pixmap::from_vec(img.as_raw().clone(), size).unwrap();
    pm.save_png(dir.join(format!("{name}.png"))).unwrap();
}

/// The shop fixture with a typed-in `Prices` table, and a JSON file next to it.
fn shop(dir: &Path) -> (PathBuf, PathBuf) {
    let mut s = Session::new();
    build_shop(&mut s);
    let run = |s: &mut Session, id: &str, p: serde_json::Value| {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    };
    run(
        &mut s,
        "table.enter_data",
        json!({"name": "Prices", "columns": ["Item", {"name": "Price", "type": "number"}, "Since"],
               "rows": [["Pen", 2.5, "2026-01-05"], ["Ink", 7, "2026-02-10"], ["Paper", 12.25, null]]}),
    );
    let path = dir.join("getdata.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    let json = dir.join("shop.json");
    std::fs::write(
        &json,
        r#"{"shop": "North", "orders": [{"id": 1, "product": "Bike", "qty": 2, "day": "2026-03-01"},
            {"id": 2, "product": "Car", "qty": 1, "day": "2026-03-02"},
            {"id": 3, "product": "Kite", "qty": 7, "day": "2026-03-04"}],
            "staff": [{"name": "Ana", "role": "Lead"}, {"name": "Bo", "role": "Sales"}]}"#,
    )
    .unwrap();
    (path, json)
}

#[test]
fn get_data_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("get-data-shots");
    let (path, json) = shop(dir.path());
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1400.0, 860.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        if dark {
            h.state_mut().set_theme(bicraft_ui::theme::Brightness::Dark);
        }
        wait(&mut h, "the first snapshot", ready);
        h.state_mut().ribbon_tab = RTab::Home;
        h.step();
        h.step();
        // the gallery, from the ribbon's button
        let label = if suffix == "fr" {
            "Obtenir des données"
        } else {
            "Get data"
        };
        h.get_by_label(label).click();
        h.step();
        h.step();
        if let Some(d) = h.state_mut().cmd.get_data.as_mut() {
            d.source = Source::Json;
            d.path = json.display().to_string();
        }
        h.step();
        h.step();
        save(&mut h, &shots, &format!("get-data-gallery-{suffix}"));
        // the Navigator: the orders ticked and previewed
        get_data::connect(h.state_mut());
        wait(&mut h, "the items", |a| {
            a.cmd.get_data.as_ref().and_then(|d| d.nav.as_ref()).is_some()
        });
        let orders = h
            .state()
            .cmd
            .get_data
            .as_ref()
            .and_then(|d| d.nav.as_ref())
            .and_then(|n| {
                n.items
                    .iter()
                    .position(|i| i.name == "orders" && i.error.is_none())
            })
            .expect("the orders item");
        if let Some(n) = h.state_mut().cmd.get_data.as_mut().and_then(|d| d.nav.as_mut()) {
            n.checked.insert(orders);
        }
        get_data::select(h.state_mut(), orders);
        wait(&mut h, "the preview", |a| {
            a.cmd
                .get_data
                .as_ref()
                .and_then(|d| d.nav.as_ref())
                .is_some_and(|n| n.preview.is_some())
        });
        h.step();
        save(&mut h, &shots, &format!("get-data-navigator-{suffix}"));
        h.state_mut().cmd.get_data = None;
        h.step();
        // Enter data after a paste from a spreadsheet, headers promoted, a type chosen
        enter_data::open(h.state_mut());
        if let Some(g) = h.state_mut().cmd.enter_data.as_mut() {
            enter_data::paste(
                g,
                (0, 0),
                "Region\tTarget\tStart\nNorth\t1200\t2026-01-01\nSouth\t950.5\t2026-02-01\n",
            );
            enter_data::promote(g);
            if let Some(c) = g.columns.get_mut(1) {
                c.ty = "number";
            }
        }
        h.step();
        h.step();
        save(&mut h, &shots, &format!("enter-data-{suffix}"));
        h.state_mut().cmd.enter_data = None;
        h.step();
        // Edit data of Prices, read back
        enter_data::edit(h.state_mut(), "Prices");
        wait(&mut h, "the grid", |a| {
            a.cmd.enter_data.as_ref().is_some_and(|g| !g.busy)
        });
        h.step();
        let g = h.state().cmd.enter_data.clone().unwrap();
        assert!(g.problem.is_none(), "{:?}", g.problem);
        assert_eq!(g.rows.len(), 3);
        save(&mut h, &shots, &format!("edit-data-{suffix}"));
        h.state_mut().cmd.enter_data = None;
    }
}

/// Get data › SQL Server database (lot 11): the gallery's entry with a server, a database and a
/// login typed, then the Navigator of a database's tables and views. No server is reached: the
/// Navigator is drawn from the answers `source.navigate` and `query.preview` give (the
/// connection itself is tested against the fake TDS server in `bicraft-ui` and
/// `bicraft-engine`).
#[test]
fn sql_server_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("get-data-sql-shots");
    let (path, _) = shop(dir.path());
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1400.0, 860.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        if dark {
            h.state_mut().set_theme(bicraft_ui::theme::Brightness::Dark);
        }
        wait(&mut h, "the first snapshot", ready);
        h.state_mut().ribbon_tab = RTab::Home;
        h.step();
        h.step();
        get_data::open(h.state_mut());
        if let Some(d) = h.state_mut().cmd.get_data.as_mut() {
            d.source = Source::Sql;
            d.sql.server = "sql.example.com,1433".into();
            d.sql.database = "Sales".into();
            d.sql.user = "report_reader".into();
        }
        h.step();
        h.step();
        save(&mut h, &shots, &format!("get-data-sql-{suffix}"));
        let g = h.state().cmd.get_data.as_ref().map_or(0, |d| d.generation);
        let m = |item: &str| {
            format!(
                "let\n    Source = Sql.Database(\"sql.example.com,1433\", \"Sales\"),\n    \
                 dbo_{item} = Source{{[Schema=\"dbo\",Item=\"{item}\"]}}[Data]\nin\n    dbo_{item}"
            )
        };
        get_data::answer(
            h.state_mut(),
            get_data::Step::Navigate(g),
            Ok(json!({
                "kind": "sqlserver", "server": "sql.example.com,1433", "database": "Sales",
                "items": [
                    {"name": "dbo.Customers", "kind": "Table", "m": m("Customers"), "hidden": false},
                    {"name": "dbo.Orders", "kind": "Table", "m": m("Orders"), "hidden": false},
                    {"name": "dbo.RecentOrders", "kind": "View", "m": m("RecentOrders"), "hidden": false},
                    {"name": "sales.Targets", "kind": "Table", "m": m("Targets"), "hidden": false},
                ],
                "truncated": false, "allow_connectors": ["sqlserver"],
            })),
        );
        if let Some(n) = h.state_mut().cmd.get_data.as_mut().and_then(|d| d.nav.as_mut()) {
            n.checked.insert(1);
            n.selected = Some(1);
        }
        get_data::answer(
            h.state_mut(),
            get_data::Step::Preview(g, 1),
            Ok(json!({
                "columns": [
                    {"name": "OrderID", "data_type": "int64"}, {"name": "Customer", "data_type": "string"},
                    {"name": "OrderDate", "data_type": "dateTime"}, {"name": "Amount", "data_type": "double"},
                ],
                "rows": [
                    [1001, "North Bikes", "2026-03-01T00:00:00", 125.5],
                    [1002, "Kite Corner", "2026-03-02T00:00:00", 980.0],
                    [1003, "Paper Mill", "2026-03-04T00:00:00", 42.25],
                ],
                "truncated": false,
            })),
        );
        h.step();
        h.step();
        let n = h
            .state()
            .cmd
            .get_data
            .as_ref()
            .and_then(|d| d.nav.clone())
            .unwrap();
        assert_eq!(n.connectors, ["sqlserver"]);
        assert!(matches!(n.preview, Some(Ok(_))), "{:?}", n.preview);
        save(&mut h, &shots, &format!("get-data-sql-navigator-{suffix}"));
        h.state_mut().cmd.get_data = None;
    }
}

/// Get data › Web and OData feed (lot 11): the gallery's Web entry with a URL typed, then the
/// Navigator of an OData service's entity sets with a preview. No server is reached: the
/// Navigator is drawn from the answers `source.navigate` and `query.preview` give (the
/// connectors are tested against the in-process HTTP server in `bicraft-webdata`,
/// `bicraft-engine` and `bicraft-ui`).
#[test]
fn web_and_odata_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("get-data-web-shots");
    let (path, _) = shop(dir.path());
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1400.0, 860.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        if dark {
            h.state_mut().set_theme(bicraft_ui::theme::Brightness::Dark);
        }
        wait(&mut h, "the first snapshot", ready);
        h.state_mut().ribbon_tab = RTab::Home;
        h.step();
        h.step();
        get_data::open(h.state_mut());
        if let Some(d) = h.state_mut().cmd.get_data.as_mut() {
            d.source = Source::Web;
            d.url = "https://data.example.com/exports/orders.json".into();
        }
        h.step();
        h.step();
        save(&mut h, &shots, &format!("get-data-web-{suffix}"));
        if let Some(d) = h.state_mut().cmd.get_data.as_mut() {
            d.source = Source::OData;
            d.url = "https://services.example.com/shop/odata/".into();
        }
        h.step();
        h.step();
        save(&mut h, &shots, &format!("get-data-odata-{suffix}"));
        let g = h.state().cmd.get_data.as_ref().map_or(0, |d| d.generation);
        let m = |set: &str| {
            format!(
                "let\n    Source = OData.Feed(\"https://services.example.com/shop/odata/\", null, \
                 [Implementation=\"2.0\"]),\n    {set}_table = Source{{[Name=\"{set}\",Signature=\"table\"]}}[Data]\n\
                 in\n    {set}_table"
            )
        };
        get_data::answer(
            h.state_mut(),
            get_data::Step::Navigate(g),
            Ok(json!({
                "kind": "odata", "url": "https://services.example.com/shop/odata/",
                "items": [
                    {"name": "Categories", "kind": "Table", "m": m("Categories"), "hidden": false},
                    {"name": "Customers", "kind": "Table", "m": m("Customers"), "hidden": false},
                    {"name": "Orders", "kind": "Table", "m": m("Orders"), "hidden": false},
                    {"name": "Products", "kind": "Table", "m": m("Products"), "hidden": false},
                ],
                "truncated": false, "allow_connectors": ["odata"],
            })),
        );
        if let Some(n) = h.state_mut().cmd.get_data.as_mut().and_then(|d| d.nav.as_mut()) {
            n.checked.insert(3);
            n.selected = Some(3);
        }
        get_data::answer(
            h.state_mut(),
            get_data::Step::Preview(g, 3),
            Ok(json!({
                "columns": [
                    {"name": "ID", "data_type": "int64"}, {"name": "Name", "data_type": "string"},
                    {"name": "Price", "data_type": "decimal"}, {"name": "InStock", "data_type": "boolean"},
                    {"name": "Released", "data_type": "dateTime"},
                ],
                "rows": [
                    [1, "Chai", 18.0, true, "2024-01-15T00:00:00"],
                    [2, "Chang", 19.5, false, "2024-02-01T00:00:00"],
                    [3, "Aniseed Syrup", 10.0, true, "2024-03-10T00:00:00"],
                    [4, "Tofu", 23.25, true, "2024-04-22T00:00:00"],
                ],
                "truncated": false,
            })),
        );
        h.step();
        h.step();
        let n = h
            .state()
            .cmd
            .get_data
            .as_ref()
            .and_then(|d| d.nav.clone())
            .unwrap();
        assert_eq!(n.connectors, ["odata"]);
        assert_eq!(n.path, "https://services.example.com/shop/odata/");
        assert!(matches!(n.preview, Some(Ok(_))), "{:?}", n.preview);
        save(&mut h, &shots, &format!("get-data-odata-navigator-{suffix}"));
        h.state_mut().cmd.get_data = None;
    }
}

/// Get data › SharePoint folder and SharePoint list (lot 11), then Home › Data source settings
/// with Edit Permissions open: the folder entry with a site URL and the access-token method, the
/// list entry with Basic, the Navigator of a site's lists, and the settings list with its
/// credential and privacy columns. No server is reached: the Navigator and the list are drawn
/// from the answers `source.navigate` and `source.settings` give (the connector is tested against
/// the in-process SharePoint site in `bicraft-webdata`, `bicraft-engine` and `bicraft-ui`).
#[test]
fn sharepoint_and_source_settings_screenshots() {
    use bicraft_ui::get_data::WebMethod;
    use bicraft_ui::source_settings::{self, Edit};
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("get-data-sp-shots");
    let (path, _) = shop(dir.path());
    let site = "https://contoso.sharepoint.com/sites/sales";
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1400.0, 860.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        if dark {
            h.state_mut().set_theme(bicraft_ui::theme::Brightness::Dark);
        }
        wait(&mut h, "the first snapshot", ready);
        h.state_mut().ribbon_tab = RTab::Home;
        h.step();
        h.step();
        get_data::open(h.state_mut());
        if let Some(d) = h.state_mut().cmd.get_data.as_mut() {
            d.source = Source::SharePointFolder;
            d.url = site.into();
            d.auth.method = WebMethod::Bearer;
        }
        h.step();
        h.step();
        save(&mut h, &shots, &format!("sp-folder-{suffix}"));
        if let Some(d) = h.state_mut().cmd.get_data.as_mut() {
            d.source = Source::SharePointList;
            d.auth.method = WebMethod::Basic;
            d.auth.user = "ana@contoso.com".into();
        }
        h.step();
        h.step();
        save(&mut h, &shots, &format!("sp-list-{suffix}"));
        let g = h.state().cmd.get_data.as_ref().map_or(0, |d| d.generation);
        let m = |id: &str| {
            format!(
                "let\n    Source = SharePoint.Tables(\"{site}\", [Implementation = \"2.0\", \
                 ViewMode = \"All\"]),\n    #\"{id}\" = Source{{[Id=\"{id}\"]}}[Items]\nin\n    #\"{id}\""
            )
        };
        get_data::answer(
            h.state_mut(),
            get_data::Step::Navigate(g),
            Ok(json!({
                "kind": "sharepoint_list", "url": site,
                "items": [
                    {"name": "Documents", "kind": "Table", "m": m("0b0c"), "hidden": false},
                    {"name": "Leads", "kind": "Table", "m": m("1a2b"), "hidden": false},
                    {"name": "Tasks", "kind": "Table", "m": m("3c4d"), "hidden": false},
                ],
                "truncated": false, "allow_connectors": ["sharepoint"],
            })),
        );
        if let Some(n) = h.state_mut().cmd.get_data.as_mut().and_then(|d| d.nav.as_mut()) {
            n.checked.insert(2);
            n.selected = Some(2);
        }
        get_data::answer(
            h.state_mut(),
            get_data::Step::Preview(g, 2),
            Ok(json!({
                "columns": [
                    {"name": "Id", "data_type": "int64"}, {"name": "Title", "data_type": "string"},
                    {"name": "Status", "data_type": "string"}, {"name": "Due", "data_type": "dateTime"},
                ],
                "rows": [
                    [1, "Call North Bikes", "Open", "2026-03-01T00:00:00"],
                    [2, "Send the price list", "Done", "2026-03-02T00:00:00"],
                    [3, "Book the venue", "Open", "2026-03-09T00:00:00"],
                ],
                "truncated": false,
            })),
        );
        h.step();
        h.step();
        let n = h
            .state()
            .cmd
            .get_data
            .as_ref()
            .and_then(|d| d.nav.clone())
            .unwrap();
        assert_eq!(n.connectors, ["sharepoint"]);
        save(&mut h, &shots, &format!("sp-list-navigator-{suffix}"));
        h.state_mut().cmd.get_data = None;
        h.step();
        // Data source settings: the session's own answer first, then a list drawn from ours
        source_settings::open(h.state_mut());
        wait(&mut h, "the list", |a| {
            a.cmd
                .source_settings
                .as_ref()
                .is_some_and(|d| d.rows.is_some() && d.pending == 0)
        });
        source_settings::answer(
            h.state_mut(),
            source_settings::Step::List,
            Ok(json!({"sources": [
                {"kind": "web", "source": site, "functions": ["SharePoint.Files"],
                 "credential": "bearer", "privacy": "organizational"},
                {"kind": "web", "source": "https://services.example.com/shop/odata",
                 "functions": ["OData.Feed"], "credential": null, "privacy": "public"},
                {"kind": "sqlserver", "source": "sql.example.com,1433", "functions": ["Sql.Database"],
                 "credential": "database", "privacy": null},
            ], "firewall": false})),
        );
        h.step();
        h.step();
        save(&mut h, &shots, &format!("sp-settings-{suffix}"));
        if let Some(d) = h.state_mut().cmd.source_settings.as_mut() {
            d.selected = Some(0);
            let mut e = Edit {
                row: 0,
                privacy: Some("organizational"),
                ..Edit::default()
            };
            e.auth.method = WebMethod::Bearer;
            d.edit = Some(e);
        }
        h.step();
        h.step();
        save(&mut h, &shots, &format!("sp-settings-edit-{suffix}"));
        h.state_mut().cmd.source_settings = None;
    }
}
