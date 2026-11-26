//! Screenshots of View as drawn with wgpu (`--features gpu-tests`), saved into
//! `BICRAFT_DESKTOP_SHOTS` when set: the dialog and the report with its banner, in English
//! light, French and dark.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use bicraft_ui::app::Tab;
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

/// Sales by region with roles East, West and Managers, a card of Total and a column chart by
/// region, saved as a PBIP project.
fn rls_shop(dir: &Path) -> PathBuf {
    let mut s = Session::new();
    let sales = r#"#table({"Region", "Manager", "Amount"}, {{"East", "alice@contoso.test", 100}, {"West", "bob@contoso.test", 1000}, {"East", "alice@contoso.test", 5}, {"North", "carol@contoso.test", 400}})"#;
    let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Region", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Manager", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
        ),
        ("role.add", json!({"name": "East"})),
        (
            "role.set_filter",
            json!({"role": "East", "table": "Sales", "filter_expression": "[Region] = \"East\""}),
        ),
        ("role.add", json!({"name": "West"})),
        (
            "role.set_filter",
            json!({"role": "West", "table": "Sales", "filter_expression": "[Region] = \"West\""}),
        ),
        ("role.add", json!({"name": "Managers"})),
        (
            "role.set_filter",
            json!({"role": "Managers", "table": "Sales", "filter_expression": "[Manager] = USERPRINCIPALNAME()"}),
        ),
        ("report.new", json!({})),
    ] {
        run(&mut s, id, p);
    }
    let page = s
        .report
        .as_ref()
        .map(|r| r.pages[0].name.clone())
        .unwrap_or_default();
    run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "card", "x": 40, "y": 40, "width": 300, "height": 160,
               "fields": [{"role": "Values", "field": "[Total]"}]}),
    );
    run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "clusteredColumnChart", "x": 380, "y": 40, "width": 520, "height": 320,
               "fields": [{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]}),
    );
    let path = dir.join("rls.pbip");
    run(&mut s, "file.save_as", json!({"path": path}));
    path
}

fn viewing(a: &App) -> bool {
    ready(a)
        && a.loaded
            .as_ref()
            .and_then(|l| l.snap.pointer("/interaction/view_as"))
            .is_some_and(|v| !v.is_null())
}

#[test]
fn view_as_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("view-as-shots");
    let path = rls_shop(dir.path());
    for (lang, suffix, dark, model, button) in [
        ("en-US", "en", false, "Model", "View as"),
        ("fr-FR", "fr", false, "Modèle", "Afficher en tant que"),
        ("en-US", "dark", true, "Model", "View as"),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.prefs.panes.data = false;
        app.prefs.panes.build = false;
        app.prefs.panes.filters = false;
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
        // the Model ribbon tab (the tab row's, not the view switcher's)
        let tab = h
            .get_all_by_label(model)
            .map(|n| n.rect())
            .min_by(|a, b| a.center().y.total_cmp(&b.center().y))
            .unwrap();
        super::click(&mut h, tab.center(), false);
        h.step();
        let steps = |h: &mut Harness<'_, App>| {
            for _ in 0..4 {
                h.step();
            }
        };
        h.get_by_label(button).click();
        steps(&mut h);
        h.get_by_label("East").click();
        steps(&mut h);
        let fr = lang == "fr-FR";
        h.get_by_label(if fr { "Autre utilisateur" } else { "Other user" })
            .click();
        steps(&mut h);
        let user = if fr {
            "Nom ou adresse e-mail de l'utilisateur"
        } else {
            "User name or email"
        };
        h.get_by_label(user).click();
        steps(&mut h);
        h.get_by_label(user).type_text("bob@contoso.test");
        steps(&mut h);
        h.get_by_label("Managers").click();
        steps(&mut h);
        save(&mut h, &shots, &format!("view-as-dialog-{suffix}"));
        h.get_by_label("OK").click();
        steps(&mut h);
        // the page is drawn in the Report view (`ready` waits for it)
        h.state_mut().tab = Tab::Report;
        wait(&mut h, "View as", viewing);
        save(&mut h, &shots, &format!("view-as-banner-{suffix}"));
    }
}
