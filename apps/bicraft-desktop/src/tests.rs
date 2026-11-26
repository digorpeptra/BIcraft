//! UI tests of the window (egui_kittest, no GPU): open a file, switch pages and tabs, click a
//! slicer and a data point the way a person does (pointer events on the drawn page), resize; the
//! results are compared with the live viewer's (the same `view.interact` actions on a session of
//! its own) and the pages with `bicraft render`'s. Fixtures are generated here, never client
//! data. With `--features gpu-tests` the window is also drawn with wgpu and saved as PNG
//! (`BICRAFT_DESKTOP_SHOTS=DIR`).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bicraft_engine::Session;
use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use crate::App;
use bicraft_ui::app::Tab;

/// A temporary folder removed at the end of the test.
pub struct Temp(PathBuf);

impl Temp {
    pub fn new(name: &str) -> Temp {
        let p = std::env::temp_dir().join(format!("bicraft-desktop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Temp(p)
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The names of the fixture's visuals.
pub struct Shop {
    pub page: String,
    pub slicer: String,
    pub card: String,
    pub chart: String,
}

/// Builds the fixture report in `s`: Sales by Category (a literal table), a slicer, a card and a
/// column chart on "Main", a card on "Second".
pub fn build_shop(s: &mut Session) -> Shop {
    let sales =
        r#"#table({"Category", "Amount"}, {{"Wheels", 100}, {"Wheels", 50}, {"Toys", 5}, {"Games", 20}})"#;
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Category", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
        ),
        ("report.new", json!({})),
        ("page.add", json!({"display_name": "Main"})),
        ("page.add", json!({"display_name": "Second"})),
    ] {
        s.execute(id, &p).unwrap();
    }
    // report.new made a first page: the report keeps Main and Second only
    let first = s.report.as_ref().unwrap().pages[0].name.clone();
    s.execute("page.remove", &json!({"page": first})).unwrap();
    let mut add = |page: &str, ty: &str, rect: [i32; 4], fields: Value| -> String {
        let r = s
            .execute(
                "visual.add",
                &json!({"page": page, "visual_type": ty, "x": rect[0], "y": rect[1], "width": rect[2],
                        "height": rect[3], "fields": fields}),
            )
            .unwrap();
        r["visual"].as_str().unwrap().to_string()
    };
    let slicer = add(
        "Main",
        "slicer",
        [20, 20, 220, 260],
        json!([{"role": "Values", "field": "Sales[Category]"}]),
    );
    let card = add(
        "Main",
        "card",
        [260, 20, 240, 120],
        json!([{"role": "Values", "field": "[Total]"}]),
    );
    let chart = add(
        "Main",
        "clusteredColumnChart",
        [260, 160, 600, 400],
        json!([{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]),
    );
    add(
        "Second",
        "card",
        [40, 40, 300, 140],
        json!([{"role": "Values", "field": "[Total]"}]),
    );
    let page = s
        .report
        .as_ref()
        .unwrap()
        .pages
        .iter()
        .find(|p| p.display_name == "Main")
        .unwrap()
        .name
        .clone();
    Shop {
        page,
        slicer,
        card,
        chart,
    }
}

/// Saves the fixture as `shop.pbip` in `dir`.
pub fn shop(dir: &Path) -> (PathBuf, Shop) {
    let mut s = Session::new();
    let names = build_shop(&mut s);
    let path = dir.join("shop.pbip");
    s.execute("file.save_as", &json!({"path": path})).unwrap();
    (path, names)
}

pub fn harness(app: App, size: (f32, f32)) -> Harness<'static, App> {
    Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
        .build_ui_state(
            |ui, app: &mut App| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        )
}

/// Steps the window until `done` holds (the source and the renderer work on other threads).
pub fn wait(h: &mut Harness<'_, App>, what: &str, done: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        h.step();
        if done(h.state()) {
            h.step();
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what} (error: {:?})",
            h.state().error
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The data rows of visual `name` on the page shown.
pub fn rows(app: &App, name: &str) -> Value {
    visual(app.loaded.as_ref().map(|l| l.snap.as_ref()), name)["data"]["rows"].clone()
}

pub fn visual(snap: Option<&Value>, name: &str) -> Value {
    snap.and_then(|s| s.pointer("/report/pages"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|p| p["visuals"].as_array().cloned().unwrap_or_default())
        .find(|v| v["name"] == name)
        .unwrap_or(Value::Null)
}

/// The first value of visual `name` (a card).
pub fn value(app: &App, name: &str) -> f64 {
    rows(app, name)[0][0].as_f64().unwrap_or(f64::NAN)
}

/// Whether the snapshot has every bound visual's data.
pub fn ready(app: &App) -> bool {
    app.loaded
        .as_ref()
        .is_some_and(|l| !bicraft_engine::watch::data_pending(&l.snap))
        && app
            .shown
            .as_ref()
            .is_some_and(|s| s.0.seq == app.loaded.as_ref().map_or(0, |l| l.seq))
}

/// The screen position of the centre of the first hit of `visual` whose action matches `pick`.
pub fn hit_pos(app: &App, visual: &str, pick: impl Fn(&Value) -> bool) -> Pos2 {
    let placed = app.placed.unwrap();
    let (_, img) = app.shown.as_ref().unwrap();
    let img = img.as_ref().unwrap();
    let h = img
        .hits
        .iter()
        .filter(|h| h.visual == visual)
        .find(|h| h.action_for(false).is_some_and(|a| pick(&a)))
        .unwrap_or_else(|| panic!("no such hit in {visual}"));
    let (x, y, w, hh) = h.shape.bounds();
    Pos2::new(
        placed.rect.min.x + (x + w / 2.0) * placed.zoom,
        placed.rect.min.y + (y + hh / 2.0) * placed.zoom,
    )
}

/// A left click at `pos`, with Ctrl or not.
pub fn click(h: &mut Harness<'_, App>, pos: Pos2, ctrl: bool) {
    let modifiers = if ctrl { Modifiers::COMMAND } else { Modifiers::NONE };
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event_modifiers(
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers,
            },
            modifiers,
        );
        h.step();
    }
}

/// Clicks a hit and waits for the source's answer and the next rendered page.
pub fn click_hit(h: &mut Harness<'_, App>, visual: &str, ctrl: bool, pick: impl Fn(&Value) -> bool) -> Value {
    let pos = hit_pos(h.state(), visual, pick);
    let before = h.state().answers;
    click(h, pos, ctrl);
    let sent = h.state().last_action.clone().expect("the click sent an action");
    wait(h, "the interaction's answer", |a| a.answers > before && ready(a));
    sent
}

/// Collapses the Report view's Build and Data panes and waits for the page at its new size.
fn collapse_panes(h: &mut Harness<'_, App>) {
    let before = h.state().placed.map(|p| p.rect.width());
    h.state_mut().prefs.panes.data = false;
    h.state_mut().prefs.panes.build = false;
    wait(h, "the page without the panes", |a| {
        ready(a) && a.placed.map(|p| p.rect.width()) != before
    });
}

fn open(dir: &Path, size: (f32, f32)) -> (Harness<'static, App>, Shop, PathBuf) {
    let (path, names) = shop(dir);
    let mut app = crate::app("en-US", None).with_sync_render();
    app.open_path(&path);
    let mut h = harness(app, size);
    wait(&mut h, "the first snapshot", ready);
    (h, names, path)
}

/// The viewer's side: the same file in a session of its own, the actions page.html sends.
fn viewer_session(path: &Path) -> Session {
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path})).unwrap();
    s
}

fn viewer_report(s: &Session) -> Value {
    let snap = s.snapshot_with(&bicraft_engine::SnapshotLimits {
        data_budget_ms: 60_000,
        ..bicraft_engine::SnapshotLimits::default()
    });
    snap["report"].clone()
}

#[test]
fn opens_a_file_and_shows_its_pages_model_and_validation() {
    let dir = Temp::new("open");
    let (mut h, names, _) = open(dir.path(), (1280.0, 800.0));
    assert_eq!(h.state().title(), "shop.pbip · rev 0");
    // the page alone in the window: the Build and Data panes collapsed
    collapse_panes(&mut h);
    assert_eq!(value(h.state(), &names.card), 175.0);
    // the page tabs, the page drawn with its hit map
    // one page navigator only: each page name appears once, in its tab at the bottom
    assert_eq!(h.query_all_by_label("Second").count(), 1);
    assert!(h.query_all_by_label("Main").count() >= 1);
    let (_, img) = h.state().shown.clone().unwrap();
    let img = img.unwrap();
    assert!(img.hits.iter().any(|x| x.visual == names.slicer));
    // the page fills the window's width or height, at the window's DPI
    let placed = h.state().placed.unwrap();
    assert!(placed.rect.width() > 900.0, "{placed:?}");
    // the Model and Validation tabs
    h.get_by_label("Model").click();
    h.step();
    h.step();
    assert_eq!(h.state().tab, Tab::Model);
    h.get_by_label("Model diagram");
    h.get_by_label("1 table · 2 columns · 1 measure · 0 relationships");
    h.get_by_label("Validation view").click();
    h.step();
    h.step();
    h.get_by_label("✓ valid");
    // the recent files remember it
    assert!(
        h.state()
            .prefs
            .recent
            .iter()
            .any(|p| p.path.ends_with("shop.pbip"))
    );
}

#[test]
fn french_ui() {
    let dir = Temp::new("fr");
    let (path, _) = shop(dir.path());
    let mut app = crate::app("fr-FR", None).with_sync_render();
    app.open_path(&path);
    let mut h = harness(app, (1000.0, 700.0));
    wait(&mut h, "the first snapshot", ready);
    h.get_by_label("Accueil");
    h.get_by_label("Modèle");
    h.get_by_label("Rapport");
}

#[test]
fn switching_pages_through_the_page_tabs() {
    let dir = Temp::new("pages");
    let (mut h, _, _) = open(dir.path(), (1280.0, 800.0));
    assert_eq!(h.state().active_page(), 0);
    let before = h.state().answers;
    // the page tabs at the bottom (the only page navigator)
    h.get_by_label("Second").click();
    wait(&mut h, "the page change", |a| a.answers > before && ready(a));
    assert_eq!(h.state().active_page(), 1);
    assert_eq!(h.state().last_action.as_ref().unwrap()["action"], "page");
}

#[test]
fn slicer_and_data_point_clicks_match_the_viewer() {
    let dir = Temp::new("slicer");
    let (mut h, n, path) = open(dir.path(), (1280.0, 800.0));
    let mut viewer = viewer_session(&path);

    // a plain click on the slicer's "Toys" item
    let sent = click_hit(&mut h, &n.slicer, false, |a| {
        a["action"] == "slicer_toggle" && a["value"] == "Toys"
    });
    // what page.html's slActs().toggle sends for that click
    let expected = json!({"action": "slicer_toggle", "page": n.page, "visual": n.slicer, "value": "Toys", "ctrl": false});
    assert_eq!(sent, expected);
    viewer.execute("view.interact", &expected).unwrap();
    assert_eq!(value(h.state(), &n.card), 5.0);
    let app_report = h.state().loaded.as_ref().unwrap().snap["report"].clone();
    assert_eq!(
        app_report,
        viewer_report(&viewer),
        "the desktop shows what the viewer shows"
    );

    // Ctrl+click adds Wheels
    let sent = click_hit(&mut h, &n.slicer, true, |a| {
        a["action"] == "slicer_toggle" && a["value"] == "Wheels"
    });
    assert_eq!(sent["ctrl"], true);
    viewer.execute("view.interact", &sent).unwrap();
    assert_eq!(value(h.state(), &n.card), 155.0);
    assert_eq!(
        h.state().loaded.as_ref().unwrap().snap["report"],
        viewer_report(&viewer)
    );

    // a column of the chart: a data point selection that highlights / filters the others
    let sent = click_hit(&mut h, &n.chart, false, |a| {
        a["action"] == "select" && a["point"][0]["v"] == "Wheels"
    });
    assert_eq!(sent["point"], json!([{"c": 0, "v": "Wheels"}]));
    viewer.execute("view.interact", &sent).unwrap();
    let chart = visual(h.state().loaded.as_ref().map(|l| l.snap.as_ref()), &n.chart);
    assert!(chart["interaction"]["selected"].is_object(), "{chart}");
    assert_eq!(value(h.state(), &n.card), 150.0);
    assert_eq!(
        h.state().loaded.as_ref().unwrap().snap["report"],
        viewer_report(&viewer)
    );

    // the chart's background clears the selection
    let placed = h.state().placed.unwrap();
    let corner = Pos2::new(
        placed.rect.min.x + 262.0 * placed.zoom,
        placed.rect.min.y + 162.0 * placed.zoom,
    );
    let before = h.state().answers;
    click(&mut h, corner, false);
    wait(&mut h, "the selection cleared", |a| {
        a.answers > before && ready(a)
    });
    let sent = h.state().last_action.clone().unwrap();
    assert_eq!(
        sent,
        json!({"action": "select", "page": n.page, "visual": n.chart, "point": null})
    );
    viewer.execute("view.interact", &sent).unwrap();
    assert_eq!(
        h.state().loaded.as_ref().unwrap().snap["report"],
        viewer_report(&viewer)
    );
    // nothing was written: the file still has no selection
    assert_eq!(
        viewer_report(&viewer_session(&path))["pages"][0]["visuals"]
            .as_array()
            .map(Vec::len),
        Some(3)
    );
}

#[test]
fn resizing_rerenders_at_the_new_size() {
    let dir = Temp::new("resize");
    let (mut h, _, _) = open(dir.path(), (1280.0, 800.0));
    collapse_panes(&mut h);
    let w1 = h.state().shown.as_ref().unwrap().1.as_ref().unwrap().width;
    h.set_size(egui::vec2(800.0, 600.0));
    wait(&mut h, "a smaller page", |a| {
        a.shown
            .as_ref()
            .and_then(|s| s.1.as_ref().ok())
            .is_some_and(|i| i.width < w1)
    });
    let w2 = h.state().shown.as_ref().unwrap().1.as_ref().unwrap().width;
    let placed = h.state().placed.unwrap();
    // crisp: one device pixel per screen pixel (the harness draws at 1 point per pixel)
    assert!(
        (w2 as f32 - placed.rect.width()).abs() <= 1.5,
        "{w2} vs {placed:?}"
    );
    // a wide window: fit page is limited by the height, fit width takes the width
    h.set_size(egui::vec2(1600.0, 500.0));
    wait(&mut h, "fit page", |a| {
        a.placed.is_some_and(|p| p.rect.width() < 900.0)
    });
    h.state_mut().view_zoom = Some(bicraft_ui::app::ViewZoom::FitWidth);
    wait(&mut h, "fit width", |a| {
        a.placed.is_some_and(|p| p.rect.width() > 1200.0)
    });
}

#[test]
fn pages_equal_bicraft_render() {
    let dir = Temp::new("equal");
    let (h, _, path) = open(dir.path(), (1280.0, 800.0));
    let loaded = h.state().loaded.clone().unwrap();
    // the app's renderer at scale 1 with the bundled fonts …
    let fonts = bicraft_render::FontBook::bundled_only();
    let key = bicraft_ui::pages::Key::new(loaded.seq, 0, 1.0);
    let ours = bicraft_ui::pages::render(&loaded.snap, &loaded.assets, &fonts, key).unwrap();
    // … and `bicraft render` (report.render) on the same file
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path})).unwrap();
    let r = s
        .execute(
            "report.render",
            // the window draws subpixel text at scale 1 (pages.rs), as `--lcd-text` does
            &json!({"page": 0, "system_fonts": false, "scale": 1, "lcd_text": true}),
        )
        .unwrap();
    let b64 = r["pages"][0]["image"]["base64"].as_str().unwrap();
    let png = decode_base64(b64);
    let pm = tiny_skia::Pixmap::decode_png(&png).unwrap();
    assert_eq!((pm.width(), pm.height()), (ours.width, ours.height));
    assert!(
        pm.data() == ours.rgba.as_slice(),
        "the desktop draws what `bicraft render` draws"
    );
}

#[test]
fn file_errors_are_shown_never_a_panic() {
    let dir = Temp::new("errors");
    let mut app = crate::app("en-US", None).with_sync_render();
    app.open_path(&dir.path().join("missing.pbix"));
    let mut h = harness(app, (900.0, 600.0));
    wait(&mut h, "the error", |a| a.error.is_some());
    assert!(h.state().error.as_deref().unwrap().contains("missing.pbix"));
    // a damaged file
    let bad = dir.path().join("bad.pbix");
    std::fs::write(&bad, b"PK\x03\x04 not a zip").unwrap();
    h.state_mut().open_path(&bad);
    wait(&mut h, "the error", |a| {
        a.error.as_deref().is_some_and(|e| e.contains("bad.pbix"))
    });
    // and the window keeps working
    h.get_by_label("Model").click();
    h.step();
    h.get_by_label("This file has no model.");
}

#[test]
fn live_reload_follows_the_file() {
    let dir = Temp::new("reload");
    let (mut h, n, path) = open(dir.path(), (1000.0, 700.0));
    // another tool changes the measure and saves the project
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path})).unwrap();
    s.execute(
        "measure.set",
        &json!({"name": "Total", "expression": "SUM(Sales[Amount]) * 2"}),
    )
    .unwrap();
    s.execute("file.save", &json!({})).unwrap();
    wait(&mut h, "the re-read", |a| value(a, &n.card) == 350.0 && ready(a));
}

/// Standard base64 (the inline images of `report.render`).
fn decode_base64(s: &str) -> Vec<u8> {
    let val = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            _ => 63,
        }
    };
    let bytes: Vec<u8> = s
        .bytes()
        .filter(|c| *c != b'=' && !c.is_ascii_whitespace())
        .collect();
    let mut out = Vec::new();
    for chunk in bytes.chunks(4) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, c)| n | val(*c) << (18 - 6 * i));
        let k = chunk.len();
        out.push((n >> 16) as u8);
        if k > 2 {
            out.push((n >> 8) as u8);
        }
        if k > 3 {
            out.push(n as u8);
        }
    }
    out
}

#[cfg(feature = "gpu-tests")]
mod analyze_shots;
#[cfg(feature = "gpu-tests")]
mod buttons_shots;
#[cfg(feature = "gpu-tests")]
mod cf_shots;
#[cfg(feature = "gpu-tests")]
mod dataview_shots;
#[cfg(feature = "gpu-tests")]
mod daxq_shots;
#[cfg(feature = "gpu-tests")]
mod drill_shots;
mod edit_tests;
#[cfg(feature = "gpu-tests")]
mod filterpane_shots;
#[cfg(feature = "gpu-tests")]
mod get_data_shots;
#[cfg(feature = "gpu-tests")]
mod groups_shots;
#[cfg(feature = "gpu-tests")]
mod keyboard_shots;
#[cfg(feature = "gpu-tests")]
mod maps_shots;
#[cfg(feature = "gpu-tests")]
mod matrix_shots;
#[cfg(feature = "gpu-tests")]
mod mobile_shots;
mod model_edit_tests;
mod model_explorer_tests;
mod model_multi_tests;
mod model_tests;
#[cfg(feature = "gpu-tests")]
mod modelsec_shots;
mod open_speed_tests;
#[cfg(feature = "gpu-tests")]
mod optimize_shots;
#[cfg(feature = "gpu-tests")]
mod painter_shots;
#[cfg(feature = "gpu-tests")]
mod params_shots;
#[cfg(feature = "gpu-tests")]
mod pbiviz_shots;
#[cfg(feature = "gpu-tests")]
mod perf_shots;
#[cfg(feature = "gpu-tests")]
mod pq_fold_shots;
#[cfg(feature = "gpu-tests")]
mod pq_shots;
#[cfg(feature = "gpu-tests")]
mod print_shots;
#[cfg(feature = "gpu-tests")]
mod props_shots;
mod recovery_tests;
#[cfg(feature = "gpu-tests")]
mod redact_shots;
#[cfg(feature = "gpu-tests")]
mod rels_shots;
mod remote_tests;
#[cfg(feature = "gpu-tests")]
mod report_settings_shots;
#[cfg(feature = "gpu-tests")]
mod roles_shots;
#[cfg(feature = "gpu-tests")]
mod shapes_shots;
#[cfg(feature = "gpu-tests")]
mod shots;
#[cfg(feature = "gpu-tests")]
mod storage_shots;
#[cfg(feature = "gpu-tests")]
mod theme_dialog_shots;
#[cfg(feature = "gpu-tests")]
mod themes_shots;
#[cfg(feature = "gpu-tests")]
mod tmdl_shots;
#[cfg(feature = "gpu-tests")]
mod view_as_shots;
#[cfg(feature = "gpu-tests")]
mod view_options_shots;
#[cfg(feature = "gpu-tests")]
mod viscalc_shots;
#[cfg(feature = "gpu-tests")]
mod visual_icons_shots;
#[cfg(feature = "gpu-tests")]
mod visual_project_shots;
#[cfg(feature = "gpu-tests")]
mod workbook_shots;
mod workbook_tests;
