//! Screenshots of the Modeling ribbon's model dialogs drawn with wgpu (`--features gpu-tests`),
//! saved into `BICRAFT_DESKTOP_SHOTS` when set: Autodetect relationships and Manage perspectives
//! (Manage security roles: `roles_shots.rs`), in English light, French and dark.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::{Temp, ready, wait};
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

/// Sales(ProductKey, Region, Amount), Product(ProductKey, Name), Region(Region), Budget and
/// Priority (one-to-one on Project), Notes(Region, repeated): no relationship yet; saved as a
/// PBIP project.
pub(super) fn shop(dir: &Path) -> PathBuf {
    shop_with(dir, "modelsec.pbip", Vec::new())
}

/// [`shop`] plus the commands `extra`, saved as `name`.
pub(super) fn shop_with(dir: &Path, name: &str, extra: Vec<(&str, Value)>) -> PathBuf {
    let mut s = Session::new();
    let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.import",
            json!({"name": "Sales", "m": "#table(type table [ProductKey = number, Region = text, Amount = number], {{1, \"E\", 10}, {1, \"W\", 5}, {2, \"E\", 7}})"}),
        ),
        (
            "table.import",
            json!({"name": "Product", "m": "#table(type table [ProductKey = number, Name = text, Cost = number], {{1, \"Bike\", 4}, {2, \"Car\", 300}})"}),
        ),
        (
            "table.import",
            json!({"name": "Region", "m": "#table(type table [Region = text, Manager = text], {{\"E\", \"a\"}, {\"W\", \"b\"}})"}),
        ),
        (
            "table.import",
            json!({"name": "Budget", "m": "#table(type table [Project = text, Allocation = number], {{\"Blue\", 40}, {\"Red\", 100}})"}),
        ),
        (
            "table.import",
            json!({"name": "Priority", "m": "#table(type table [Project = text, Level = text], {{\"Blue\", \"A\"}, {\"Red\", \"B\"}, {\"Green\", \"C\"}})"}),
        ),
        (
            "table.import",
            json!({"name": "Notes", "m": "#table(type table [Project = text, Note = text], {{\"Blue\", \"x\"}, {\"Blue\", \"y\"}})"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("report.new", json!({})),
    ]
    .into_iter()
    .chain(extra)
    {
        run(&mut s, id, p);
    }
    let path = dir.join(name);
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

pub(super) fn open(path: &Path, lang: &str, dark: bool, model: &str) -> Harness<'static, App> {
    let mut app = crate::app(lang, None).with_sync_render();
    app.open_path(path);
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
    // the Model ribbon tab (the tab row's, not the view switcher's)
    let tab = h
        .get_all_by_label(model)
        .map(|n| n.rect())
        .min_by(|a, b| a.center().y.total_cmp(&b.center().y))
        .unwrap();
    super::click(&mut h, tab.center(), false);
    steps(&mut h);
    h
}

/// The Model explorer's Model tab, a right click on row `row` and its menu item `item` (the
/// lowest widget of that label).
pub(super) fn explorer_menu(h: &mut Harness<'_, App>, row: &str, item: &str) {
    h.state_mut().model_view.explorer.tab = bicraft_ui::model_explorer::Tab::Model;
    steps(h);
    let r = h.get_by_label(row).rect();
    super::model_explorer_tests::right_click(h, r.center());
    let at = h
        .get_all_by_label(item)
        .map(|n| n.rect())
        .max_by(|a, b| a.min.y.total_cmp(&b.min.y))
        .unwrap_or_else(|| panic!("no menu item `{item}`"));
    super::click(h, at.center(), false);
    steps(h);
}

pub(super) fn steps(h: &mut Harness<'_, App>) {
    for _ in 0..6 {
        h.step();
    }
}

#[test]
fn autodetect_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("modelsec-autodetect-shots");
    let path = shop(dir.path());
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let (model, row, item) = if lang == "fr-FR" {
            ("Modèle", "Relations (0)", "Détecter les relations")
        } else {
            ("Model", "Relationships (0)", "Autodetect relationships")
        };
        let mut h = open(&path, lang, dark, model);
        explorer_menu(&mut h, row, item);
        wait(&mut h, "the candidates", |a| {
            a.cmd.autodetect.as_ref().is_some_and(|d| d.found.is_some())
        });
        steps(&mut h);
        save(&mut h, &shots, &format!("modelsec-autodetect-{suffix}"));
    }
}

#[test]
fn perspectives_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("modelsec-persp-shots");
    let path = shop_with(
        dir.path(),
        "persp.pbip",
        vec![(
            "perspective.add",
            json!({"name": "Sales view", "tables": [
                {"table": "Sales", "columns": ["Amount", "ProductKey"], "measures": ["Total"]},
                "Product"
            ]}),
        )],
    );
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let (model, row, item, members) = if lang == "fr-FR" {
            (
                "Modèle",
                "Perspectives (1)",
                "Gérer les perspectives",
                "Membres de Product",
            )
        } else {
            (
                "Model",
                "Perspectives (1)",
                "Manage perspectives",
                "Members of Product",
            )
        };
        let mut h = open(&path, lang, dark, model);
        explorer_menu(&mut h, row, item);
        wait(&mut h, "the dialog", |a| a.cmd.perspective.is_some());
        h.get_by_label(members).click();
        steps(&mut h);
        save(&mut h, &shots, &format!("modelsec-persp-{suffix}"));
        // the Model view filtered to the perspective
        h.state_mut().cmd.perspective = None;
        h.state_mut().model_view.perspective = Some("Sales view".into());
        steps(&mut h);
        save(&mut h, &shots, &format!("modelsec-persp-view-{suffix}"));
    }
}
