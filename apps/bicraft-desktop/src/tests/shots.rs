//! Screenshots of the window drawn with wgpu (`--features gpu-tests`; needs a GPU or a software
//! adapter). Saved as PNG into `BICRAFT_DESKTOP_SHOTS` when set (docs/desktop.md).

use std::path::PathBuf;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::{Temp, click_hit, ready, shop, wait};
use crate::App;
use bicraft_ui::app::Tab;

fn save(h: &mut Harness<'_, App>, dir: &Option<PathBuf>, name: &str) {
    let img = h.render().unwrap();
    assert!(img.width() > 0 && img.height() > 0);
    let Some(dir) = dir else { return };
    std::fs::create_dir_all(dir).unwrap();
    let size = tiny_skia::IntSize::from_wh(img.width(), img.height()).unwrap();
    let pm = tiny_skia::Pixmap::from_vec(img.as_raw().clone(), size).unwrap();
    pm.save_png(dir.join(format!("{name}.png"))).unwrap();
}

/// The Model tab's diagram: English light, French, dark, a table and a measure selected.
#[test]
fn model_screenshots() {
    use super::model_tests::star;
    use bicraft_ui::model_view::Sel;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("model-shots");
    let path = star(dir.path());
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.open_path(&path);
        app.tab = Tab::Model;
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
        wait(&mut h, "the diagram", |a| a.model_view.diagram().is_some());
        h.step();
        save(&mut h, &shots, &format!("model-{suffix}"));
        if suffix != "fr" {
            h.state_mut().model_view.sel = Some(Sel::Table("Sales".into()));
            h.step();
            h.step();
            save(&mut h, &shots, &format!("model-{suffix}-table"));
            h.state_mut().model_view.sel = Some(Sel::Measure {
                table: "Sales".into(),
                name: "Total".into(),
            });
            h.step();
            h.step();
            save(&mut h, &shots, &format!("model-{suffix}-measure"));
        }
        // the Properties pane of a column and of a relationship (lot 18)
        h.state_mut().model_view.sel = Some(Sel::Column {
            table: "Sales".into(),
            name: "Amount".into(),
        });
        h.step();
        h.step();
        save(&mut h, &shots, &format!("model-{suffix}-column"));
        let rel = h.state().model_view.diagram().unwrap().links[0].rel;
        h.state_mut().model_view.sel = Some(Sel::Rel(rel));
        h.step();
        h.step();
        save(&mut h, &shots, &format!("model-{suffix}-relationship"));
        h.state_mut().model_view.sel = None;
        h.step();
        // a column dragged towards another table's (the line, the target row lit), then a card
        // moved by its header
        let (from, to, head) = {
            let a = h.state();
            let d = a.model_view.diagram().unwrap();
            let snap = &a.loaded.as_ref().unwrap().snap["model"];
            let row = |t: &str, c: &str| {
                let k = d.card_named(t).unwrap();
                let idx = snap["tables"][d.cards[k].table]["columns"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .position(|x| x["name"] == c)
                    .unwrap();
                let r = d.cards[k].columns.iter().find(|r| r.index == idx).unwrap();
                a.model_view.screen_pos(r.rect.center())
            };
            let notes = &d.cards[d.card_named("Notes").unwrap()];
            (
                row("Notes", "Text"),
                row("Category", "Label"),
                a.model_view.screen_pos(notes.head.center()),
            )
        };
        let press = |h: &mut Harness<'_, App>, pos: egui::Pos2, pressed: bool| {
            h.event(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
        };
        h.hover_at(from);
        h.step();
        press(&mut h, from, true);
        for k in 1..=5 {
            h.hover_at(from + (to - from) * (k as f32 / 5.0));
            h.step();
        }
        save(&mut h, &shots, &format!("model-{suffix}-relate"));
        // let go over the empty canvas: no relationship
        let away = egui::pos2(to.x, h.state().model_view.canvas.max.y - 10.0);
        h.hover_at(away);
        h.step();
        press(&mut h, away, false);
        h.hover_at(head);
        h.step();
        press(&mut h, head, true);
        for k in 1..=5 {
            h.hover_at(head + egui::vec2(-40.0, 160.0) * (k as f32 / 5.0));
            h.step();
        }
        let end = head + egui::vec2(-40.0, 160.0);
        press(&mut h, end, false);
        wait(&mut h, "the moved card", |a| {
            a.loaded.as_ref().is_some_and(|l| {
                l.snap["model"]["diagram"]["nodes"]
                    .as_array()
                    .is_some_and(|n| !n.is_empty())
            })
        });
        h.step();
        save(&mut h, &shots, &format!("model-{suffix}-moved"));
    }
}

#[test]
fn window_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("shots");
    let (path, n) = shop(dir.path());
    for (lang, suffix) in [("en-US", "en"), ("fr-FR", "fr")] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        save(&mut h, &shots, &format!("report-{suffix}"));
        if suffix == "en" {
            click_hit(&mut h, &n.slicer, false, |a| a["value"] == "Toys");
            save(&mut h, &shots, "slicer-click");
            click_hit(&mut h, &n.slicer, true, |a| a["value"] == "Wheels");
            click_hit(&mut h, &n.chart, false, |a| a["action"] == "select");
            save(&mut h, &shots, "cross-highlight");
            h.state_mut().tab = Tab::Model;
            h.step();
            h.step();
            save(&mut h, &shots, "model");
            h.state_mut().tab = Tab::Validation;
            h.step();
            h.step();
            save(&mut h, &shots, "validation");
        }
    }
}

/// The design system's screens (docs/design.md §11): the start screen, the Home ribbon, the
/// Connect dialog, and the three tabs, in light and dark.
#[test]
fn design_screenshots() {
    use bicraft_ui::theme::Brightness;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("design-shots");
    let (path, _) = shop(dir.path());
    for (theme, b) in [("light", Brightness::Light), ("dark", Brightness::Dark)] {
        let build = |app: App| {
            Harness::builder()
                .with_size(egui::vec2(1280.0, 800.0))
                .wgpu()
                .build_ui_state(
                    |ui, app: &mut App| {
                        app.logic(ui.ctx());
                        app.ui(ui);
                    },
                    app,
                )
        };
        let mut app = crate::app("en-US", None).with_sync_render();
        app.set_theme(b);
        let mut h = build(app);
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("design-{theme}-start"));
        h.state_mut().ribbon_tab = bicraft_ui::app::RibbonTab::Home;
        h.state_mut().connect = Some(bicraft_ui::app::ConnectForm::default());
        for _ in 0..3 {
            h.step();
        }
        save(&mut h, &shots, &format!("design-{theme}-connect"));
        let mut app = crate::app("en-US", None).with_sync_render();
        app.set_theme(b);
        app.open_path(&path);
        let mut h = build(app);
        wait(&mut h, "the first snapshot", ready);
        for (tab, name) in [
            (Tab::Report, "report"),
            (Tab::Model, "model"),
            (Tab::Validation, "validation"),
        ] {
            h.state_mut().show_tab(tab);
            for _ in 0..4 {
                h.step();
            }
            save(&mut h, &shots, &format!("design-{theme}-{name}"));
        }
    }
}

/// The committed `visuals.pbip` fixture and a model-only `.bim` at the browser check's canvas
/// size (1366 × 775, French), light and dark, every tab, to compare with the headless-browser
/// screenshots of `/app/` (docs/webui.md, "Browser check").
#[test]
fn side_by_side_with_the_browser() {
    use bicraft_ui::theme::Brightness;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (file, name) in [
        (
            "crates/engine/tests/fixtures/layout-oracle/visuals/legacy/visuals.pbip",
            "visuals",
        ),
        ("crates/engine/tests/dax_queries/model.bim", "bim"),
    ] {
        for (theme, b) in [("light", Brightness::Light), ("dark", Brightness::Dark)] {
            let mut app = crate::app("fr-FR", None).with_sync_render();
            app.set_theme(b);
            app.open_path(&root.join(file));
            let mut h = Harness::builder()
                .with_size(egui::vec2(1366.0, 775.0))
                .wgpu()
                .build_ui_state(
                    |ui, app: &mut App| {
                        app.logic(ui.ctx());
                        app.ui(ui);
                    },
                    app,
                );
            wait(&mut h, "the first snapshot", |a| a.loaded.is_some());
            if name == "visuals" {
                wait(&mut h, "the page", ready);
            }
            for (tab, t) in [
                (Tab::Report, "report"),
                (Tab::Model, "model"),
                (Tab::Validation, "validation"),
            ] {
                h.state_mut().show_tab(tab);
                for _ in 0..4 {
                    h.step();
                }
                save(&mut h, &shots, &format!("desktop-{name}-{theme}-{t}"));
            }
        }
    }
}

/// The File view's pages, the page tabs (with their menu) and two document tabs, light and dark.
#[test]
fn backstage_screenshots() {
    use bicraft_ui::backstage::{Backstage, Page, Task};
    use bicraft_ui::theme::Brightness;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("backstage-shots");
    let (path, _) = shop(dir.path());
    let other = dir.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    let (path2, _) = shop(&other);
    for (theme, b) in [("light", Brightness::Light), ("dark", Brightness::Dark)] {
        let mut app = crate::app("en-US", None).with_sync_render();
        app.set_theme(b);
        for (i, f) in ["Sales 2026.pbix", "Budget.pbip", "Forecast.pbit", "model.bim"]
            .iter()
            .enumerate()
        {
            app.prefs.add_recent(
                &std::path::Path::new("C:/Reports/Finance").join(f),
                1_762_214_400 + i as u64 * 86_400,
            );
        }
        app.open_path(&path2);
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("backstage-{theme}-report-tabs"));
        for (page, task, name) in [
            (Page::Home, None, "home"),
            (Page::Open, None, "open"),
            (Page::Info, None, "info"),
            (Page::Export, Some(Task::Convert), "export"),
        ] {
            let mut bs = Backstage::new(page);
            if let Some(t) = task {
                bs.task = t;
            }
            h.state_mut().backstage = Some(bs);
            for _ in 0..6 {
                h.step();
            }
            save(&mut h, &shots, &format!("backstage-{theme}-{name}"));
        }
        h.state_mut().backstage = None;
        for _ in 0..3 {
            h.step();
        }
        // the page tab's menu
        let tab = h
            .get_all_by_label("Main")
            .map(|n| n.rect())
            .max_by(|a, b| a.center().y.total_cmp(&b.center().y))
            .unwrap();
        h.hover_at(tab.center());
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: tab.center(),
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
        }
        for _ in 0..3 {
            h.step();
        }
        save(&mut h, &shots, &format!("backstage-{theme}-page-menu"));
    }
}

/// Every ribbon tab built from the command registry (ROADMAP M11), the contextual Visual, Table
/// and Measure tabs with their selection, in light, dark and French.
#[test]
fn ribbon_screenshots() {
    use bicraft_ui::model_view::Sel;
    use bicraft_ui::registry::Tab as RTab;
    use bicraft_ui::ribbon::{self, VisualSel};
    use bicraft_ui::theme::Brightness;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("ribbon-shots");
    let (path, _) = shop(dir.path());
    for (lang, theme, b) in [
        ("en-US", "light", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
        ("fr-FR", "fr", Brightness::Light),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        for tab in ribbon::TABS
            .into_iter()
            .chain([RTab::Visual, RTab::Table, RTab::Measure])
        {
            let st = h.state_mut();
            match tab {
                RTab::Visual => {
                    st.show_tab(Tab::Report);
                    let snap = st.loaded.as_ref().map(|l| l.snap.clone());
                    let name = |p: &str| {
                        snap.as_ref()
                            .and_then(|s| s.pointer(p)?.as_str().map(str::to_string))
                            .unwrap_or_default()
                    };
                    st.cmd.visual = Some(VisualSel {
                        page: name("/report/pages/0/name"),
                        visual: name("/report/pages/0/visuals/0/name"),
                    });
                }
                RTab::Table => {
                    st.show_tab(Tab::Model);
                    st.model_view.sel = Some(Sel::Table("Sales".into()));
                }
                RTab::Measure => {
                    st.show_tab(Tab::Model);
                    st.model_view.sel = Some(Sel::Measure {
                        table: "Sales".into(),
                        name: "Total".into(),
                    });
                }
                _ => {
                    if let Some(v) = ribbon::view_of(tab) {
                        st.show_tab(v);
                    }
                }
            }
            h.state_mut().ribbon_tab = tab;
            for _ in 0..5 {
                h.step();
            }
            assert_eq!(h.state().ribbon_tab, tab);
            save(&mut h, &shots, &format!("ribbon-{theme}-{tab:?}").to_lowercase());
        }
    }
}

/// The command palette (a search, a greyed command with its reason) and parameters forms (empty,
/// with their problems after Run, with an answer), English and French, light and dark.
#[test]
fn palette_and_form_screenshots() {
    use bicraft_ui::theme::Brightness;
    use bicraft_ui::{form, palette};
    use serde_json::json;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("palette-shots");
    let (path, _) = shop(dir.path());
    for (lang, suffix, b) in [
        ("en-US", "en", Brightness::Light),
        ("fr-FR", "fr", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        let step = |h: &mut Harness<'_, App>| {
            for _ in 0..5 {
                h.step();
            }
        };
        // a ribbon tab of the merged UI, with the palette over it next
        h.state_mut().ribbon_tab = bicraft_ui::registry::Tab::Data;
        step(&mut h);
        save(&mut h, &shots, &format!("ribbon-{suffix}-data"));
        palette::toggle(h.state_mut());
        step(&mut h);
        save(&mut h, &shots, &format!("palette-{suffix}"));
        h.state_mut().palette.as_mut().unwrap().query = "new".into();
        step(&mut h);
        save(&mut h, &shots, &format!("palette-{suffix}-search"));
        h.state_mut().palette.as_mut().unwrap().query = "report.new".into();
        step(&mut h);
        save(&mut h, &shots, &format!("palette-{suffix}-disabled"));
        form::open(h.state_mut(), "measure.add", json!({"table": "Sales"}));
        step(&mut h);
        save(&mut h, &shots, &format!("form-{suffix}-measure"));
        form::run(h.state_mut());
        step(&mut h);
        save(&mut h, &shots, &format!("form-{suffix}-errors"));
        form::open(
            h.state_mut(),
            "dax.validate",
            json!({"expression": "SUM(Sales[Amount])"}),
        );
        form::run(h.state_mut());
        wait(&mut h, "the answer", |a| {
            a.form.as_ref().is_some_and(|f| f.result.is_some())
        });
        step(&mut h);
        save(&mut h, &shots, &format!("form-{suffix}-answer"));
        form::open(h.state_mut(), "visual.format", json!({}));
        step(&mut h);
        save(&mut h, &shots, &format!("form-{suffix}-visual-format"));
    }
}

/// Command answers (a grid sorted on a column, a key/value tree, the JSON view, `dax.evaluate`'s
/// grid in its form), a form with its defaults, and the title row's search button with the
/// palette open, English and French, light and dark.
#[test]
fn answer_screenshots() {
    use bicraft_ui::theme::Brightness;
    use bicraft_ui::{form, palette, ribbon};
    use serde_json::json;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("answer-shots");
    let (path, _) = shop(dir.path());
    for (lang, suffix, b) in [
        ("en-US", "en", Brightness::Light),
        ("fr-FR", "fr", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        let step = |h: &mut Harness<'_, App>| {
            for _ in 0..5 {
                h.step();
            }
        };
        // a read command from its button: a grid, then sorted on its second column
        ribbon::invoke(h.state_mut(), "model.get", json!({}));
        wait(&mut h, "the answer", |a| {
            matches!(a.cmd.result, Some((_, Some(_))))
        });
        step(&mut h);
        save(&mut h, &shots, &format!("answer-{suffix}-tree"));
        ribbon::invoke(h.state_mut(), "model.summary", json!({}));
        wait(&mut h, "the answer", |a| {
            matches!(a.cmd.result, Some((_, Some(_))))
        });
        step(&mut h);
        save(&mut h, &shots, &format!("answer-{suffix}-summary"));
        // a synthetic grid of 100,000 rows sorted descending on `size`
        let rows: Vec<serde_json::Value> = (0..100_000u32)
            .map(|i| json!({"name": format!("row {i:06}"), "rows": (i * 7919) % 1000, "size": i, "kind": if i.is_multiple_of(3) { "fact" } else { "dimension" }}))
            .collect();
        h.state_mut().cmd.view = None;
        h.state_mut().cmd.result = Some(("model.storage_stats", Some(Ok(json!(rows)))));
        step(&mut h);
        if let Some(g) = h.state_mut().cmd.view.as_mut().and_then(|a| a.grid_mut()) {
            g.click(2);
            g.click(2);
        }
        step(&mut h);
        save(&mut h, &shots, &format!("answer-{suffix}-grid"));
        if let Some(a) = h.state_mut().cmd.view.as_mut() {
            a.json = true;
        }
        step(&mut h);
        save(&mut h, &shots, &format!("answer-{suffix}-json"));
        h.state_mut().cmd.result = None;
        h.state_mut().cmd.view = None;
        // the form's own answer
        form::open(
            h.state_mut(),
            "dax.evaluate",
            json!({"query": "EVALUATE SUMMARIZECOLUMNS(Sales[Category], \"Total\", [Total])"}),
        );
        form::run(h.state_mut());
        wait(&mut h, "the answer", |a| {
            a.form.as_ref().is_some_and(|f| f.result.is_some())
        });
        step(&mut h);
        save(&mut h, &shots, &format!("answer-{suffix}-form"));
        // defaults shown
        form::open(h.state_mut(), "model.unused", json!({}));
        step(&mut h);
        save(&mut h, &shots, &format!("form-{suffix}-defaults"));
        form::open(h.state_mut(), "table.set", json!({"name": "Sales"}));
        step(&mut h);
        save(&mut h, &shots, &format!("form-{suffix}-not-set"));
        h.state_mut().form = None;
        // the search button's palette
        palette::toggle(h.state_mut());
        step(&mut h);
        save(&mut h, &shots, &format!("search-{suffix}"));
    }
}

/// The recovery prompt over a file a crashed session left edits to, the start screen's list,
/// and File > Info with "Save automatically" (docs/recovery.md).
#[test]
fn recovery_screenshots() {
    use serde_json::json;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("recovery-shots");
    let (path, _) = shop(dir.path());
    let rec = dir.path().join("recovery");
    let cfg = bicraft_engine::RecoveryConfig::new(&rec);
    {
        let mut s = bicraft_engine::Session::new();
        s.set_recovery(Some(cfg.clone()));
        s.execute("file.open", &json!({"path": path})).unwrap();
        let table = s.model.as_ref().unwrap().tables[0].name.clone();
        for name in ["Kept", "Also kept"] {
            s.execute(
                "measure.add",
                &json!({"table": table, "name": name, "expression": "1"}),
            )
            .unwrap();
        }
    }
    let build = |app: App| {
        Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            )
    };
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let app = crate::app_with(lang, None, Some(rec.clone())).with_sync_render();
        let mut h = build(app);
        if dark {
            h.state_mut().set_theme(bicraft_ui::theme::Brightness::Dark);
        }
        wait(&mut h, "the start screen", |a| a.backstage.is_some());
        h.step();
        save(&mut h, &shots, &format!("recovery-start-{suffix}"));
        h.state_mut().open_path(&path);
        h.state_mut().backstage = None;
        wait(&mut h, "the offer", |a| {
            a.loaded.as_ref().is_some_and(|l| {
                l.snap
                    .pointer("/recovery/offers")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|o| !o.is_empty())
            }) && a.shown.is_some()
        });
        h.step();
        h.step();
        save(&mut h, &shots, &format!("recovery-prompt-{suffix}"));
        if suffix == "en" {
            h.state_mut().prefs.autosave = bicraft_ui::prefs::Autosave::Idle(30);
            h.state_mut().recovery.seen.extend(
                bicraft_engine::recovery_offers(&cfg)
                    .iter()
                    .filter_map(|o| o["id"].as_str().map(str::to_string)),
            );
            h.state_mut().backstage = Some(bicraft_ui::backstage::Backstage::new(
                bicraft_ui::backstage::Page::Info,
            ));
            h.step();
            h.step();
            h.step();
            save(&mut h, &shots, "recovery-info-autosave");
        }
    }
}

/// The Report view's Build and Data panes: nothing selected, the chart selected (its wells, the
/// Sales table open), a field being dragged over a well and over a visual, collapsed panes;
/// light, dark and French.
#[test]
fn panes_screenshots() {
    use bicraft_ui::ribbon::VisualSel;
    use bicraft_ui::theme::Brightness;
    use egui::{Event, Modifiers, PointerButton};
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("panes-shots");
    let (path, names) = shop(dir.path());
    for (lang, suffix, b) in [
        ("en-US", "light", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
        ("fr-FR", "fr", Brightness::Light),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1440.0, 860.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        save(&mut h, &shots, &format!("panes-{suffix}-none"));
        h.state_mut().cmd.visual = Some(VisualSel {
            page: names.page.clone(),
            visual: names.chart.clone(),
        });
        h.state_mut().panes.open.insert("t:Sales".into());
        for _ in 0..3 {
            h.step();
        }
        save(&mut h, &shots, &format!("panes-{suffix}-chart"));
        // a field held over the Tooltips well, then over the card
        let field = h.get_by_label("Amount").rect().center();
        let well = h
            .get_all_by_role(egui::accesskit::Role::Label)
            .last()
            .map(|n| n.rect().center());
        let card = {
            let placed = h.state().placed.unwrap();
            placed.rect.min + egui::vec2(380.0, 80.0) * placed.zoom
        };
        h.hover_at(field);
        h.step();
        h.event(Event::PointerButton {
            pos: field,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        h.step();
        for target in [well.unwrap_or(field), card] {
            for i in 1..=6 {
                let t = i as f32 / 6.0;
                h.hover_at(field + (target - field) * t);
                h.step();
            }
            let what = if target == card { "canvas" } else { "well" };
            save(&mut h, &shots, &format!("panes-{suffix}-drag-{what}"));
        }
        // Escape drops the drag, then the button goes up
        h.key_press(egui::Key::Escape);
        h.step();
        h.event(Event::PointerButton {
            pos: card,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        h.step();
        // both panes collapsed
        h.state_mut().prefs.panes.data = false;
        h.state_mut().prefs.panes.build = false;
        h.state_mut().prefs.panes.filters = false;
        for _ in 0..3 {
            h.step();
        }
        save(&mut h, &shots, &format!("panes-{suffix}-collapsed"));
    }
}

/// The Filters pane: a page filter (basic, its values open), a report filter (advanced), the
/// chart selected with its cards (one Top N open), a field renamed for the chart; light, dark
/// and French.
#[test]
fn filters_screenshots() {
    use bicraft_ui::actions::Call;
    use bicraft_ui::app::Then;
    use bicraft_ui::ribbon::VisualSel;
    use bicraft_ui::theme::Brightness;
    use serde_json::json;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("filters-shots");
    let (path, names) = shop(dir.path());
    for (lang, suffix, b) in [
        ("en-US", "light", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
        ("fr-FR", "fr", Brightness::Light),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1600.0, 900.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        let page = names.page.clone();
        for (id, params) in [
            (
                "page.filter",
                json!({"page": page, "field": "Sales[Category]", "not_in": ["Toys"]}),
            ),
            (
                "report.filter",
                json!({"field": "Sales[Amount]", "op": ">", "value": 10}),
            ),
            (
                "visual.filter",
                json!({"page": page, "visual": names.chart, "field": "Sales[Category]", "top": 2, "by": "[Total]"}),
            ),
        ] {
            h.state_mut().run(Call { id, params }, Then::Nothing);
            wait(&mut h, id, |a| a.calls_pending() == 0);
        }
        h.state_mut()
            .filters
            .open
            .insert(format!("page:{page}::Sales[Category]"));
        h.state_mut().filters.open.insert("report:::Sales[Amount]".into());
        for _ in 0..20 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        save(&mut h, &shots, &format!("filters-{suffix}-page"));
        h.state_mut().cmd.visual = Some(VisualSel {
            page: page.clone(),
            visual: names.chart.clone(),
        });
        h.state_mut().filters.open.clear();
        h.state_mut()
            .filters
            .open
            .insert(format!("visual:{page}:{}:Sales[Category]", names.chart));
        for _ in 0..6 {
            h.step();
        }
        save(&mut h, &shots, &format!("filters-{suffix}-visual"));
    }
}

/// The Filters pane's relative date card (open, "is in the last 3 days") with the lock and the
/// eye on each card (a locked page card, a hidden report card); light, dark and French.
#[test]
fn relative_filter_screenshots() {
    use bicraft_engine::Session;
    use bicraft_ui::actions::Call;
    use bicraft_ui::app::Then;
    use bicraft_ui::theme::Brightness;
    use serde_json::json;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("relative-filter-shots");
    let mut s = Session::new();
    let names = super::build_shop(&mut s);
    s.execute(
        "column.add",
        &json!({"table": "Sales", "name": "Day", "expression": "DATE(2026, 11, 18)"}),
    )
    .unwrap();
    let path = dir.path().join("shop.pbip");
    s.execute("file.save_as", &json!({"path": path})).unwrap();
    for (lang, suffix, b) in [
        ("en-US", "light", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
        ("fr-FR", "fr", Brightness::Light),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1600.0, 900.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        let page = names.page.clone();
        for (id, params) in [
            (
                "page.filter",
                json!({"page": page, "field": "Sales[Day]", "relative": {"range": "last", "count": 3, "unit": "days"}}),
            ),
            (
                "page.filter",
                json!({"page": page, "field": "Sales[Day]", "locked": true}),
            ),
            (
                "report.filter",
                json!({"field": "Sales[Category]", "not_in": ["Toys"], "hidden": true}),
            ),
        ] {
            h.state_mut().run(Call { id, params }, Then::Nothing);
            wait(&mut h, id, |a| a.calls_pending() == 0);
        }
        h.state_mut()
            .filters
            .open
            .insert(format!("page:{page}::Sales[Day]"));
        for _ in 0..20 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        save(&mut h, &shots, &format!("filters-relative-{suffix}"));
    }
}

/// The DAX formula bar: a measure on it (collapsed, expanded), the autocomplete list, a
/// function's signature, the live check's error, a new measure; light, dark and French.
#[test]
fn formula_bar_screenshots() {
    use bicraft_ui::formula::{Kind, editor_id, pick};
    use bicraft_ui::theme::Brightness;
    use egui::Event;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("formula-shots");
    let (path, _) = shop(dir.path());
    for (lang, suffix, b) in [
        ("en-US", "light", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
        ("fr-FR", "fr", Brightness::Light),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1440.0, 860.0))
            .with_step_dt(1.0 / 60.0)
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        pick(&h.ctx, Kind::Measure, Some("Sales"), "Total", true);
        wait(&mut h, "Total on the bar", |a| {
            a.formula.text.starts_with("Total =")
        });
        wait(&mut h, "the check", |a| {
            a.sent.iter().any(|c| c.id == "dax.validate")
        });
        for _ in 0..30 {
            h.step();
        }
        save(&mut h, &shots, &format!("formula-{suffix}-measure"));
        let end = h.state().formula.text.chars().count();
        h.state_mut().formula.cursor_to = Some(end);
        for _ in 0..3 {
            h.step();
        }
        // a second line, then the autocomplete of a column
        h.event(Event::Text("\n    + DIVIDE('Sales'[Am".into()));
        for _ in 0..3 {
            h.step();
        }
        save(&mut h, &shots, &format!("formula-{suffix}-autocomplete"));
        h.key_press(egui::Key::Escape);
        h.event(Event::Text("ount], ".into()));
        for _ in 0..3 {
            h.step();
        }
        h.state_mut().formula.expanded = true;
        for _ in 0..3 {
            h.step();
        }
        save(&mut h, &shots, &format!("formula-{suffix}-signature"));
        // the live check: an unknown column, underlined, its message under the bar
        h.event(Event::Text("Sales[Nope])".into()));
        wait(&mut h, "the diagnostic", |a| !a.formula.diagnostics.is_empty());
        for _ in 0..3 {
            h.step();
        }
        save(&mut h, &shots, &format!("formula-{suffix}-error"));
        h.ctx.memory_mut(|m| m.surrender_focus(editor_id()));
        bicraft_ui::formula::cancel(h.state_mut());
        h.state_mut().formula.expanded = false;
        bicraft_ui::formula::new_measure(h.state_mut(), None);
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("formula-{suffix}-new"));
    }
}

/// The Edit interactions mode: the column chart selected as the source, the icon sets on the
/// pie (Filter, Highlight, None; Highlight the default), the card (None set by a rule) and the
/// slicer (Filter, None), then a data point clicked (the live preview); light, dark and French.
#[test]
fn interactions_screenshots() {
    use bicraft_ui::actions::Call;
    use bicraft_ui::app::Then;
    use bicraft_ui::ribbon::VisualSel;
    use bicraft_ui::theme::Brightness;
    use serde_json::json;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("interactions-shots");
    let (path, names) = shop(dir.path());
    for (lang, suffix, b) in [
        ("en-US", "light", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
        ("fr-FR", "fr", Brightness::Light),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1600.0, 900.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        let page = names.page.clone();
        for (id, params) in [
            (
                "visual.add",
                json!({"page": page, "visual_type": "pieChart", "x": 900, "y": 20, "width": 340, "height": 260,
                       "fields": [{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]}),
            ),
            (
                "page.interactions",
                json!({"page": page, "rules": [{"source": names.chart, "target": names.card, "type": "none"}]}),
            ),
        ] {
            h.state_mut().run(Call { id, params }, Then::Nothing);
            wait(&mut h, id, |a| a.calls_pending() == 0);
        }
        h.state_mut().cmd.visual = Some(VisualSel {
            page: page.clone(),
            visual: names.chart.clone(),
        });
        h.step();
        h.state_mut().select_ribbon_tab(bicraft_ui::registry::Tab::Visual);
        bicraft_ui::interactions::toggle(h.state_mut());
        for _ in 0..20 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(h.state().cmd.edit_interactions);
        save(&mut h, &shots, &format!("interactions-{suffix}"));
        h.state_mut().interact(
            json!({"action": "select", "page": page, "visual": names.chart, "point": [{"c": 0, "v": "Wheels"}]}),
        );
        for _ in 0..30 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        save(&mut h, &shots, &format!("interactions-{suffix}-preview"));
    }
}

/// The Data view (M11.17): the grid with the Data pane, a column selected with its Column tools,
/// a header's menu, the sorted grid, a query preview and See records, light, dark and French.
#[test]
fn data_view_screenshots() {
    use bicraft_ui::theme::Brightness;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("data-view-shots");
    let (path, n) = shop(dir.path());
    for (lang, suffix, b) in [
        ("en-US", "light", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
        ("fr-FR", "fr", Brightness::Light),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1440.0, 860.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        // See records of the chart's first bar, from the report
        let pos = super::hit_pos(h.state(), &n.chart, |a| {
            a["action"] == "select" && !a["point"].is_null()
        });
        let point = bicraft_ui::data_view::point_at(h.state(), pos);
        let p = serde_json::json!({"page": n.page, "visual": n.chart, "point": point});
        bicraft_ui::data_view::records(h.state_mut(), p);
        wait(&mut h, "the records", |a| a.cmd.data.stat.is_some());
        save(&mut h, &shots, &format!("data-{suffix}-records"));
        bicraft_ui::data_view::close_static(h.state_mut());
        wait(&mut h, "the table", |a| {
            a.cmd.data.row_count.is_some() && a.cmd.data.cell(0, 0).is_some()
        });
        save(&mut h, &shots, &format!("data-{suffix}-grid"));
        let ctx = h.ctx.clone();
        bicraft_ui::data_view::select_column(h.state_mut(), &ctx, "Category");
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("data-{suffix}-column-tools"));
        let at = h.get_by_label("Category").rect().right_bottom();
        bicraft_ui::data_view::open_menu(h.state_mut(), "Category", at);
        wait(&mut h, "the values", |a| a.cmd.data.values.is_some());
        save(&mut h, &shots, &format!("data-{suffix}-menu"));
        h.state_mut().cmd.data.menu = None;
        bicraft_ui::data_view::set_sort(h.state_mut(), Some(("Amount".into(), true)));
        wait(&mut h, "the sorted rows", |a| a.cmd.data.cell(0, 0).is_some());
        save(&mut h, &shots, &format!("data-{suffix}-sorted"));
        bicraft_ui::data_view::preview(h.state_mut(), serde_json::json!({"name": "Sales"}));
        wait(&mut h, "the preview", |a| a.cmd.data.stat.is_some());
        save(&mut h, &shots, &format!("data-{suffix}-preview"));
    }
}

/// Canvas editing (ROADMAP M11.16): two visuals selected with the handles of their rectangle, a
/// drag's ghost outlines and smart guide, the rubber band, the Align menu and the visual's menu;
/// light, dark and French.
#[test]
fn canvas_screenshots() {
    use bicraft_ui::canvas::{self, Gesture, Guide, Kind};
    use bicraft_ui::registry::Tab as RTab;
    use bicraft_ui::ribbon::VisualSel;
    use bicraft_ui::theme::Brightness;
    use egui::{Rect, pos2, vec2};
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("canvas-shots");
    let (path, names) = shop(dir.path());
    for (lang, suffix, b) in [
        ("en-US", "light", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
        ("fr-FR", "fr", Brightness::Light),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1600.0, 900.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        let page = names.page.clone();
        h.state_mut().cmd.visual = Some(VisualSel {
            page: page.clone(),
            visual: names.card.clone(),
        });
        h.state_mut().cmd.more = vec![names.chart.clone()];
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("canvas-{suffix}-selection"));
        // a drag of both, 40 px right, the card's left edge on the slicer's right one
        let items: Vec<(String, Rect)> = canvas::page_items(h.state())
            .unwrap()
            .2
            .iter()
            .filter(|i| i.name == names.card || i.name == names.chart)
            .map(|i| (i.name.clone(), i.rect))
            .collect();
        h.state_mut().canvas.gesture = Some(Gesture {
            page: page.clone(),
            kind: Kind::Move,
            start: pos2(300.0, 100.0),
            now: items
                .iter()
                .map(|(n, r)| (n.clone(), r.translate(vec2(-20.0, 30.0))))
                .collect(),
            items,
            band: None,
            guides: vec![Guide {
                vertical: true,
                at: 240.0,
                from: 20.0,
                to: 590.0,
            }],
        });
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("canvas-{suffix}-drag"));
        h.state_mut().canvas.gesture = Some(Gesture {
            page: page.clone(),
            kind: Kind::Band { add: false },
            start: pos2(10.0, 10.0),
            now: Vec::new(),
            items: Vec::new(),
            band: Some(Rect::from_min_max(pos2(10.0, 10.0), pos2(700.0, 300.0))),
            guides: Vec::new(),
        });
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("canvas-{suffix}-band"));
        h.state_mut().canvas.gesture = None;
        h.state_mut().ribbon_tab = RTab::Visual;
        for _ in 0..4 {
            h.step();
        }
        let align = if lang == "fr-FR" { "Aligner" } else { "Align" };
        h.get_by_label(align).click();
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("canvas-{suffix}-align"));
    }
}

/// The export flows (M11.21): Backstage › Export to PDF, the selected visual's More options and
/// its Export data dialog; with `BICRAFT_DESKTOP_SHOTS` set, a sample PDF of the fixture report
/// (every page, the bundled fonts) is written beside the screenshots.
#[test]
fn export_screenshots() {
    use bicraft_ui::backstage::{Backstage, Page, Task};
    use bicraft_ui::ribbon::VisualSel;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("export-shots");
    let (path, n) = shop(dir.path());
    let mut app = crate::app("en-US", None).with_sync_render();
    app.open_path(&path);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .wgpu()
        .build_ui_state(
            |ui, app: &mut App| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        );
    wait(&mut h, "the first snapshot", ready);
    h.state_mut().cmd.visual = Some(VisualSel {
        page: n.page.clone(),
        visual: n.chart.clone(),
    });
    h.step();
    h.step();
    save(&mut h, &shots, "export-visual-header");
    h.get_by_label("More options").click();
    h.step();
    h.step();
    save(&mut h, &shots, "export-visual-menu");
    h.get_by_label("Export data").click();
    h.step();
    h.step();
    save(&mut h, &shots, "export-data-dialog");
    h.state_mut().cmd.export_data = None;
    let mut bs = Backstage::new(Page::Export);
    bs.task = Task::Pdf;
    h.state_mut().backstage = Some(bs);
    h.step();
    h.step();
    save(&mut h, &shots, "export-pdf-backstage");
    if let Some(b) = h.state_mut().backstage.as_mut() {
        b.task = Task::Data;
    }
    h.step();
    h.step();
    save(&mut h, &shots, "export-data-setting");
    if let Some(d) = &shots {
        let mut s = bicraft_engine::Session::new();
        s.execute("file.open", &serde_json::json!({"path": path}))
            .unwrap();
        let out = d.join("sample-report.pdf");
        let r = s
            .execute(
                "report.export_pdf",
                &serde_json::json!({"out": out, "system_fonts": false, "title": "BIcraft sample report"}),
            )
            .unwrap();
        assert_eq!(r["pages"].as_array().map(Vec::len), Some(2));
    }
}

/// Rich content (M11.20): Insert › Text box with the editor and its toolbar, Insert › Buttons, the
/// action editor and the page format window, drawn with wgpu.
#[test]
fn rich_content_screenshots() {
    use egui::{Event, Key, Modifiers};
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("rich-shots");
    let (path, _) = shop(dir.path());
    let mut app = crate::app("en-US", None).with_sync_render();
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
    wait(&mut h, "the first snapshot", ready);
    // the tab shown by the registry's own call (a click on the active tab collapses the ribbon)
    let tab = |h: &mut Harness<'_, App>, caption: &str| {
        let t = bicraft_ui::registry::Tab::ALL
            .into_iter()
            .find(|t| t.caption(bicraft_ui::i18n::Lang::En) == caption)
            .unwrap();
        h.state_mut().select_ribbon_tab(t);
        for _ in 0..3 {
            h.step();
        }
    };
    tab(&mut h, "Insert");
    h.get_by_label("Text box").click();
    wait(&mut h, "the text box", |a| {
        a.cmd.rich.editor.is_some()
            && a.loaded
                .as_ref()
                .is_some_and(|l| l.snap.to_string().contains("\"textbox\""))
    });
    for _ in 0..4 {
        h.step();
    }
    h.event(Event::Text("Quarterly ".into()));
    h.step();
    for pressed in [true, false] {
        h.event_modifiers(
            Event::Key {
                key: Key::B,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Modifiers::COMMAND,
            },
            Modifiers::COMMAND,
        );
    }
    h.step();
    h.event(Event::Text("sales: ".into()));
    h.step();
    h.get_by_label("+ Value").click();
    h.step();
    h.step();
    h.get_by_label("[Total]").click();
    for _ in 0..4 {
        h.step();
    }
    save(&mut h, &shots, "rich-text-editor");
    h.get_by_label("✔ Done").click();
    wait(&mut h, "the committed text", |a| {
        a.cmd.rich.editor.is_none()
            && a.loaded
                .as_ref()
                .is_some_and(|l| l.snap.to_string().contains("Quarterly"))
    });
    wait(&mut h, "the page", ready);
    save(&mut h, &shots, "rich-text-committed");
    tab(&mut h, "Insert");
    h.get_by_label("Buttons").click();
    for _ in 0..4 {
        h.step();
    }
    save(&mut h, &shots, "rich-insert-buttons");
    h.get_by_label("Back").click();
    wait(&mut h, "the button", |a| {
        a.loaded
            .as_ref()
            .is_some_and(|l| l.snap.to_string().contains("\"actionButton\""))
    });
    wait(&mut h, "the page", ready);
    tab(&mut h, "Visual");
    h.get_by_label("Action").click();
    for _ in 0..4 {
        h.step();
    }
    save(&mut h, &shots, "rich-action-editor");
    h.state_mut().cmd.rich.action = None;
    tab(&mut h, "Report");
    h.get_by_label("Page format").click();
    for _ in 0..4 {
        h.step();
    }
    save(&mut h, &shots, "rich-page-format");
}

/// The Build pane's Format visual tab (M11.22): the chart's Visual cards, its General cards with
/// Title and Effects open, the colour popup, a search; light, dark and French.
#[test]
fn format_pane_screenshots() {
    use bicraft_ui::actions::Call;
    use bicraft_ui::app::Then;
    use bicraft_ui::format_pane::{Group, Tab as BuildTab};
    use bicraft_ui::ribbon::VisualSel;
    use bicraft_ui::theme::Brightness;
    use serde_json::json;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("format-shots");
    let (path, names) = shop(dir.path());
    for (lang, suffix, b) in [
        ("en-US", "light", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
        ("fr-FR", "fr", Brightness::Light),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1440.0, 900.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        h.state_mut().cmd.visual = Some(VisualSel {
            page: names.page.clone(),
            visual: names.chart.clone(),
        });
        h.state_mut().prefs.panes.build_width = 300.0;
        {
            let f = &mut h.state_mut().panes.format;
            f.tab = BuildTab::Format;
            f.group = Group::Visual;
            f.open.insert("x_axis".into());
            f.open.insert("legend".into());
        }
        for _ in 0..6 {
            h.step();
        }
        save(&mut h, &shots, &format!("format-{suffix}-visual"));
        {
            let f = &mut h.state_mut().panes.format;
            f.group = Group::General;
            f.open.insert("title".into());
            f.open.insert("effects".into());
        }
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("format-{suffix}-general"));
        let colour = if lang == "fr-FR" {
            "Effets Arrière-plan Couleur"
        } else {
            "Effects Background Colour"
        };
        h.get_by_label(colour).click();
        for _ in 0..3 {
            h.step();
        }
        save(&mut h, &shots, &format!("format-{suffix}-colour"));
        h.key_press(egui::Key::Escape);
        h.state_mut().panes.format.search = "font".into();
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("format-{suffix}-search"));
        // the fx button of Data colours and its conditional-formatting dialog: Gradient, Rules
        // (Escape above also cleared the selection)
        h.state_mut().cmd.visual = Some(VisualSel {
            page: names.page.clone(),
            visual: names.chart.clone(),
        });
        {
            let f = &mut h.state_mut().panes.format;
            f.search.clear();
            f.group = Group::Visual;
            f.open.clear();
            f.open.insert("dataPoint".into());
        }
        for _ in 0..4 {
            h.step();
        }
        let fx = if lang == "fr-FR" {
            "Couleurs des données Couleur Mise en forme conditionnelle"
        } else {
            "Data colours Colour Conditional formatting"
        };
        h.get_by_label(fx).click();
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("format-{suffix}-fx"));
        if let Some(d) = h.state_mut().panes.format.cf.as_mut() {
            d.style = bicraft_ui::conditional_dialog::Style::Rules;
            use bicraft_ui::conditional_dialog::Case;
            d.cases.push(Case::new(">", "100", "#1AAB40"));
            d.cases.push(Case::two(">=", "10", "<", "100", "#E66C37"));
        }
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("format-{suffix}-fx-rules"));
        h.state_mut().panes.format.cf = None;
        // the fx button marked once the chart has a conditional format
        h.state_mut().run(
            Call {
                id: "visual.conditional",
                params: json!({"page": names.page, "visual": names.chart, "target": "data_color",
                               "rule": {"gradient": {"field": "[Total]", "min": "#FFFFFF", "max": "#118DFF"}}}),
            },
            Then::Nothing,
        );
        wait(&mut h, "visual.conditional", |a| a.calls_pending() == 0);
        h.event(egui::Event::PointerMoved(egui::pos2(300.0, 780.0)));
        for _ in 0..20 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        wait(&mut h, "visual.props", |a| a.calls_pending() == 0);
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("format-{suffix}-fx-on"));
        // Data colours with one colour per category value: the chart made a pie
        h.state_mut().run(
            Call {
                id: "visual.set_type",
                params: json!({"page": names.page, "visual": names.chart, "visual_type": "pieChart"}),
            },
            Then::Nothing,
        );
        wait(&mut h, "visual.set_type", |a| a.calls_pending() == 0);
        h.state_mut().cmd.visual = Some(VisualSel {
            page: names.page.clone(),
            visual: names.chart.clone(),
        });
        {
            let f = &mut h.state_mut().panes.format;
            f.search.clear();
            f.group = Group::Visual;
            f.open.clear();
            f.open.insert("dataPoint".into());
        }
        for _ in 0..20 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        wait(&mut h, "column.values", |a| a.calls_pending() == 0);
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("format-{suffix}-colours"));
    }
}

/// The fx buttons of the General cards (title, subtitle, background, border) and of the data
/// labels, the title text's dialog (Field value only), then the chart drawn with a title from a
/// measure, its title colour, background and data label colours by rule (G54); light, dark and
/// French.
#[test]
fn format_fx_screenshots() {
    use bicraft_ui::actions::Call;
    use bicraft_ui::app::Then;
    use bicraft_ui::format_pane::{Group, Tab as BuildTab};
    use bicraft_ui::ribbon::VisualSel;
    use bicraft_ui::theme::Brightness;
    use serde_json::json;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("format-fx-shots");
    let (path, names) = shop(dir.path());
    for (lang, suffix, b) in [
        ("en-US", "light", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
        ("fr-FR", "fr", Brightness::Light),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1440.0, 900.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        h.state_mut().cmd.visual = Some(VisualSel {
            page: names.page.clone(),
            visual: names.chart.clone(),
        });
        h.state_mut().prefs.panes.build_width = 300.0;
        {
            let f = &mut h.state_mut().panes.format;
            f.tab = BuildTab::Format;
            f.group = Group::General;
            f.open.insert("title".into());
            f.open.insert("effects".into());
        }
        for _ in 0..6 {
            h.step();
        }
        save(&mut h, &shots, &format!("format-fx-{suffix}-general"));
        // the title text's fx: Field value only
        let fx = if lang == "fr-FR" {
            "Titre Texte Mise en forme conditionnelle"
        } else {
            "Title Text Conditional formatting"
        };
        h.get_by_label(fx).click();
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("format-fx-{suffix}-title-text"));
        h.state_mut().panes.format.cf = None;
        // the chart with its title, title colour, background and data label colours by rule
        let (page, chart) = (names.page.clone(), names.chart.clone());
        for (id, params) in [
            (
                "measure.add",
                json!({"table": "Sales", "name": "Caption", "expression": "\"Sales by category, \" & FORMAT([Total], \"0\")"}),
            ),
            (
                "visual.format",
                json!({"page": page, "visual": chart, "data_labels": true}),
            ),
            (
                "visual.conditional",
                json!({"page": page, "visual": chart, "conditional": [
                    {"target": "title_text", "rule": {"field_value": {"field": "[Caption]"}}},
                    {"target": "title_color", "rule": {"rules": {"field": "[Total]",
                        "cases": [{"op": ">", "value": 100, "color": "#1AAB40"}], "else": "#D64550"}}},
                    {"target": "visual_background", "rule": {"gradient": {"field": "[Total]",
                        "min": {"color": "#FFFFFF", "value": 0}, "max": {"color": "#FFF4CE", "value": 100}}}},
                    {"target": "label_color", "rule": {"rules": {"field": "[Total]",
                        "cases": [{"op": ">=", "value": 30, "color": "#1AAB40"}], "else": "#D64550"}}}]}),
            ),
        ] {
            h.state_mut().run(Call { id, params }, Then::Nothing);
            wait(&mut h, id, |a| a.calls_pending() == 0);
        }
        wait(&mut h, "the formatted chart", ready);
        {
            let f = &mut h.state_mut().panes.format;
            f.open.clear();
            f.open.insert("title".into());
        }
        h.event(egui::Event::PointerMoved(egui::pos2(300.0, 780.0)));
        for _ in 0..20 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        wait(&mut h, "visual.props", |a| a.calls_pending() == 0);
        wait(&mut h, "the formatted chart", ready);
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("format-fx-{suffix}-canvas"));
    }
}

/// A page with a page navigator and a bookmark navigator (G55), then a reader's bookmark applied
/// (the card hidden, the bookmark highlighted); light, dark and French.
#[test]
fn navigator_screenshots() {
    use bicraft_ui::actions::Call;
    use bicraft_ui::app::Then;
    use bicraft_ui::theme::Brightness;
    use serde_json::json;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("navigator-shots");
    let (path, names) = shop(dir.path());
    for (lang, suffix, b) in [
        ("en-US", "light", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
        ("fr-FR", "fr", Brightness::Light),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1440.0, 900.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        let page = names.page.clone();
        let card = names.card.clone();
        for (id, params) in [
            (
                "button.add",
                json!({"page": page, "kind": "page_navigator", "x": 20, "y": 590, "width": 500, "height": 50}),
            ),
            (
                "button.add",
                json!({"page": page, "kind": "bookmark_navigator", "x": 560, "y": 590, "width": 600, "height": 50}),
            ),
            ("bookmark.capture", json!({"display_name": "Everything"})),
            (
                "visual.set_hidden",
                json!({"page": page, "visual": card, "hidden": true}),
            ),
            ("bookmark.capture", json!({"display_name": "No card"})),
            (
                "visual.set_hidden",
                json!({"page": page, "visual": card, "hidden": false}),
            ),
        ] {
            h.state_mut().run(Call { id, params }, Then::Nothing);
            wait(&mut h, id, |a| a.calls_pending() == 0);
        }
        wait(&mut h, "the navigators", |a| {
            ready(a)
                && a.loaded
                    .as_ref()
                    .is_some_and(|l| l.snap.to_string().contains("bookmarkNavigator"))
        });
        for _ in 0..6 {
            h.step();
        }
        save(&mut h, &shots, &format!("navigators-{suffix}"));
        h.state_mut().access = Some("reader".into());
        h.state_mut()
            .interact(json!({"action": "bookmark", "bookmark": "No card"}));
        wait(&mut h, "the bookmark applied", |a| {
            a.loaded
                .as_ref()
                .is_some_and(|l| l.snap["interaction"]["bookmark"].is_string())
        });
        wait(&mut h, "the page", ready);
        for _ in 0..6 {
            h.step();
        }
        save(&mut h, &shots, &format!("navigators-{suffix}-applied"));
    }
}

/// Bookmarks of slicer selections (G55): Toys and Wheels clicked on the slicer and captured as
/// two bookmarks, a styled bookmark navigator (fill, font colour and accent bar per state), the
/// Bookmarks pane open, then the Toys bookmark applied after a click on Wheels; light, dark and
/// French.
#[test]
fn bookmark_slicer_screenshots() {
    use bicraft_ui::actions::Call;
    use bicraft_ui::app::Then;
    use bicraft_ui::theme::Brightness;
    use serde_json::{Value, json};
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("bookmark-slicer-shots");
    let (path, names) = shop(dir.path());
    for (lang, suffix, b) in [
        ("en-US", "light", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
        ("fr-FR", "fr", Brightness::Light),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
        app.prefs.panes.bookmarks = true;
        app.prefs.panes.filters = false;
        app.prefs.panes.build = false;
        app.prefs.panes.data = false;
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1600.0, 900.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        let page = names.page.clone();
        let run = |h: &mut Harness<'_, App>, id: &'static str, params: Value| {
            h.state_mut().run(Call { id, params }, Then::Nothing);
            wait(h, id, |a| a.calls_pending() == 0);
        };
        run(
            &mut h,
            "button.add",
            json!({"page": page, "kind": "bookmark_navigator", "x": 20, "y": 600, "width": 600, "height": 56}),
        );
        let nav = h.state().loaded.as_ref().unwrap().snap["report"]["pages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["name"] == json!(page))
            .flat_map(|p| p["visuals"].as_array().cloned().unwrap_or_default())
            .find(|v| v["type"] == "bookmarkNavigator")
            .and_then(|v| v["name"].as_str().map(str::to_string))
            .unwrap();
        let obj = |object: &str, property: &str, value: Value, state: &str| json!({"object": object, "property": property, "value": value, "state": state});
        run(
            &mut h,
            "visual.format",
            json!({"page": page, "visual": nav, "objects": [
                obj("fill", "show", json!(true), "default"),
                obj("fill", "fillColor", json!({"color": "#E6E6E6"}), "default"),
                obj("text", "fontColor", json!({"color": "#252423"}), "default"),
                obj("fill", "show", json!(true), "selected"),
                obj("fill", "fillColor", json!({"color": "#12239E"}), "selected"),
                obj("text", "fontColor", json!({"color": "#FFFFFF"}), "selected"),
                obj("accentBar", "show", json!(true), "selected"),
                obj("accentBar", "color", json!({"color": "#E66C37"}), "selected"),
                obj("accentBar", "position", json!("Left"), "selected"),
                obj("accentBar", "width", json!(6), "selected"),
                json!({"object": "layout", "property": "cellPadding", "value": 10}),
            ]}),
        );
        for (value, name) in [("Toys", "Toys only"), ("Wheels", "Wheels only")] {
            click_hit(&mut h, &names.slicer, false, |a: &Value| {
                a["action"] == "slicer_toggle" && a["value"] == value
            });
            wait(&mut h, "the click", ready);
            run(&mut h, "bookmark.capture", json!({"display_name": name}));
        }
        h.state_mut()
            .interact(json!({"action": "bookmark", "bookmark": "Toys only"}));
        wait(&mut h, "the bookmark applied", |a| {
            ready(a)
                && a.loaded.as_ref().is_some_and(|l| {
                    super::visual(Some(&l.snap), &names.slicer)["slicer"]["selected"] == json!(["Toys"])
                })
        });
        for _ in 0..20 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        save(&mut h, &shots, &format!("bookmarks2-{suffix}"));
    }
}

/// Synced slicers (G55): the slicers of Main and Second in one group, Toys clicked on Main's
/// slicer, then Second shown with its synced slicer ticked and its card filtered, the Sync
/// slicers pane open; light, dark and French.
#[test]
fn synced_slicer_screenshots() {
    use bicraft_ui::actions::Call;
    use bicraft_ui::app::Then;
    use bicraft_ui::theme::Brightness;
    use serde_json::{Value, json};
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("sync-shots");
    let (path, names) = shop(dir.path());
    for (lang, suffix, b) in [
        ("en-US", "light", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
        ("fr-FR", "fr", Brightness::Light),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
        app.prefs.panes.sync = true;
        // the page larger: the other right panes closed
        app.prefs.panes.filters = false;
        app.prefs.panes.build = false;
        app.prefs.panes.data = false;
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1600.0, 900.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        let page_of = |snap: &Value, display: &str| -> String {
            snap["report"]["pages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["display_name"] == display)
                .and_then(|p| p["name"].as_str())
                .unwrap()
                .to_string()
        };
        let second = page_of(&h.state().loaded.as_ref().unwrap().snap, "Second");
        h.state_mut().run(
            Call {
                id: "visual.add",
                params: json!({"page": second, "visual_type": "slicer", "x": 380, "y": 40,
                               "width": 220, "height": 200,
                               "fields": [{"role": "Values", "field": "Sales[Category]"}]}),
            },
            Then::Nothing,
        );
        wait(&mut h, "the second slicer", |a| a.calls_pending() == 0);
        let slicer2 = h.state().loaded.as_ref().unwrap().snap["report"]["pages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["name"] == json!(second))
            .flat_map(|p| p["visuals"].as_array().cloned().unwrap_or_default())
            .find(|v| v["type"] == "slicer")
            .and_then(|v| v["name"].as_str().map(str::to_string))
            .unwrap();
        h.state_mut().run(
            Call {
                id: "slicer.set_sync",
                params: json!({"segments": [{"page": names.page, "visual": names.slicer},
                                            {"page": second, "visual": slicer2}],
                               "group": "Category"}),
            },
            Then::Nothing,
        );
        wait(&mut h, "the sync group", |a| a.calls_pending() == 0 && ready(a));
        click_hit(&mut h, &names.slicer, false, |a: &Value| {
            a["action"] == "slicer_toggle" && a["value"] == "Toys"
        });
        h.state_mut().interact(json!({"action": "page", "page": second}));
        wait(&mut h, "page Second", |a| {
            ready(a)
                && a.loaded
                    .as_ref()
                    .is_some_and(|l| l.snap["interaction"]["page"] == json!(second))
        });
        let snap = h.state().loaded.as_ref().unwrap().snap.clone();
        let v2 = super::visual(Some(&snap), &slicer2);
        assert_eq!(v2["slicer"]["selected"], json!(["Toys"]), "{v2}");
        for _ in 0..20 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        save(&mut h, &shots, &format!("sync-slicers-{suffix}"));
    }
}

/// "Show value as" and "Show items with no data" (a well field's menu): the menu of a matrix's
/// value with its Show value as submenu open, the matrix shown as percentages of its row totals
/// (and a chart of shares of the grand total listing a category without sales), and a
/// category's menu; light, dark and French.
#[test]
fn show_value_as_screenshots() {
    use bicraft_engine::Session;
    use bicraft_ui::ribbon::VisualSel;
    use bicraft_ui::theme::Brightness;
    use egui::{Event, Modifiers, PointerButton};
    use egui_kittest::kittest::NodeT;
    use serde_json::json;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("show-as-shots");
    let mut s = Session::new();
    let sales = r#"#table({"Category", "Region", "Amount"}, {{"Wheels", "North", 300}, {"Wheels", "South", 100}, {"Toys", "North", 40}, {"Toys", "South", 60}, {"Games", "South", 20}})"#;
    let category = r#"#table({"Category"}, {{"Wheels"}, {"Toys"}, {"Games"}, {"Books"}})"#;
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "table.add",
            json!({"name": "Categories", "partition": {"source_type": "m", "expression": category}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Category", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Region", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Categories", "name": "Category", "data_type": "string"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
        ),
        (
            "relationship.add",
            json!({"from": "Sales[Category]", "to": "Categories[Category]"}),
        ),
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap();
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let mut add = |ty: &str, rect: [i32; 4], fields: serde_json::Value| -> String {
        let r = s
            .execute(
                "visual.add",
                &json!({"page": page, "visual_type": ty, "x": rect[0], "y": rect[1], "width": rect[2],
                        "height": rect[3], "fields": fields}),
            )
            .unwrap();
        r["visual"].as_str().unwrap().to_string()
    };
    let matrix = add(
        "pivotTable",
        [20, 20, 520, 260],
        json!([{"role": "Rows", "field": "Categories[Category]"}, {"role": "Columns", "field": "Sales[Region]"},
               {"role": "Values", "field": "[Total]"}]),
    );
    let chart = add(
        "clusteredColumnChart",
        [20, 300, 520, 300],
        json!([{"role": "Category", "field": "Categories[Category]"}, {"role": "Y", "field": "[Total]"}]),
    );
    for (id, p) in [
        (
            "visual.show_value_as",
            json!({"page": page, "visual": chart, "role": "Y", "index": 0, "as": "percent_of_grand_total"}),
        ),
        (
            "visual.show_items_no_data",
            json!({"page": page, "visual": chart, "role": "Category", "show": true}),
        ),
        (
            "visual.format",
            json!({"page": page, "visual": chart, "objects": [{"object": "labels", "property": "show", "value": true}]}),
        ),
    ] {
        s.execute(id, &p).unwrap();
    }
    let path = dir.path().join("shop.pbip");
    s.execute("file.save_as", &json!({"path": path})).unwrap();
    let right_click = |h: &mut Harness<'_, App>, pos: egui::Pos2| {
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
    };
    // a button by the start of its label (EN or FR)
    let find = |h: &Harness<'_, App>, starts: &[&str]| -> egui::Pos2 {
        h.get_all_by_role(egui::accesskit::Role::Button)
            .find(|n| {
                n.accesskit_node()
                    .label()
                    .is_some_and(|l| starts.iter().any(|s| l.starts_with(s)))
            })
            .map(|n| n.rect().center())
            .unwrap_or_else(|| panic!("no button starting with {starts:?}"))
    };
    for (lang, suffix, b) in [
        ("en-US", "light", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
        ("fr-FR", "fr", Brightness::Light),
    ] {
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
        app.prefs.panes.filters = false;
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1440.0, 860.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        h.state_mut().cmd.visual = Some(VisualSel {
            page: page.clone(),
            visual: matrix.clone(),
        });
        for _ in 0..3 {
            h.step();
        }
        let total = find(&h, &["Total in ", "Total dans "]);
        right_click(&mut h, total);
        let sub = find(&h, &["Show value as", "Afficher la valeur comme"]);
        super::click(&mut h, sub, false);
        for _ in 0..3 {
            h.step();
        }
        save(&mut h, &shots, &format!("show-as-menu-{suffix}"));
        let row_total = find(&h, &["Percent of row total", "Pourcentage du total de la ligne"]);
        super::click(&mut h, row_total, false);
        wait(&mut h, "the percentages", |a| {
            ready(a)
                && super::visual(a.loaded.as_ref().map(|l| l.snap.as_ref()), &matrix)["fields"][2]["show_as"]
                    == "percent_of_row_total"
        });
        for _ in 0..10 {
            h.step();
        }
        save(&mut h, &shots, &format!("show-as-matrix-{suffix}"));
        let cat = find(&h, &["Category in ", "Category dans "]);
        right_click(&mut h, cat);
        for _ in 0..3 {
            h.step();
        }
        save(&mut h, &shots, &format!("show-items-menu-{suffix}"));
    }
}

/// The visual header's reader tools (M11.19): the selected chart's header with its focus icon,
/// focus mode (the chart alone with Back to report), Show as a table (the chart over its data
/// table) and More options › Sort axis, in light, dark and French, drawn with wgpu.
#[test]
fn focus_mode_screenshots() {
    use bicraft_ui::ribbon::VisualSel;
    use bicraft_ui::theme::Brightness;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("focus-shots");
    let (path, n) = shop(dir.path());
    for (lang, suffix, b) in [
        ("en-US", "light", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
        ("fr-FR", "fr", Brightness::Light),
    ] {
        let fr = lang == "fr-FR";
        let t = |en: &'static str, fr_text: &'static str| if fr { fr_text } else { en };
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
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
        wait(&mut h, "the first snapshot", ready);
        h.state_mut().cmd.visual = Some(VisualSel {
            page: n.page.clone(),
            visual: n.chart.clone(),
        });
        for _ in 0..3 {
            h.step();
        }
        save(&mut h, &shots, &format!("focus-{suffix}-header"));
        h.get_by_label(t("Focus mode", "Mode focus")).click();
        wait(&mut h, "focus mode", |a| bicraft_ui::focus::active(a) && ready(a));
        for _ in 0..3 {
            h.step();
        }
        save(&mut h, &shots, &format!("focus-{suffix}-focus"));
        h.get_by_label(t("More options", "Plus d'options")).click();
        for _ in 0..2 {
            h.step();
        }
        h.get_by_label(t("Show as a table", "Afficher sous forme de tableau"))
            .click();
        wait(&mut h, "show as a table", |a| {
            bicraft_ui::focus::current(a).is_some_and(|f| f.table) && ready(a)
        });
        for _ in 0..3 {
            h.step();
        }
        save(&mut h, &shots, &format!("focus-{suffix}-table"));
        h.get_by_label(t("More options", "Plus d'options")).click();
        for _ in 0..2 {
            h.step();
        }
        let sort = t("Sort axis", "Trier l'axe");
        let r = h
            .query_by(|x| x.label().is_some_and(|l| l.starts_with(sort)))
            .unwrap_or_else(|| panic!("no Sort axis entry"))
            .rect();
        h.hover_at(r.center());
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("focus-{suffix}-sort-menu"));
    }
}

/// Spotlight (M11): the chart in spotlight, the page's other visuals faded, then its More
/// options menu with Spotlight checked; in light, dark and French, drawn with wgpu.
#[test]
fn spotlight_screenshots() {
    use bicraft_ui::ribbon::VisualSel;
    use bicraft_ui::theme::Brightness;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("spotlight-shots");
    let (path, n) = shop(dir.path());
    for (lang, suffix, b) in [
        ("en-US", "light", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
        ("fr-FR", "fr", Brightness::Light),
    ] {
        let fr = lang == "fr-FR";
        let t = |en: &'static str, fr_text: &'static str| if fr { fr_text } else { en };
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
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
        wait(&mut h, "the first snapshot", ready);
        h.state_mut().cmd.visual = Some(VisualSel {
            page: n.page.clone(),
            visual: n.chart.clone(),
        });
        for _ in 0..3 {
            h.step();
        }
        h.get_by_label(t("More options", "Plus d'options")).click();
        for _ in 0..2 {
            h.step();
        }
        h.get_by_label(t("Spotlight", "Projecteur")).click();
        wait(&mut h, "spotlight", |a| {
            bicraft_ui::focus::spotlight(a).is_some() && ready(a)
        });
        for _ in 0..3 {
            h.step();
        }
        save(&mut h, &shots, &format!("spotlight-{suffix}-on"));
        h.get_by_label(t("More options", "Plus d'options")).click();
        for _ in 0..3 {
            h.step();
        }
        save(&mut h, &shots, &format!("spotlight-{suffix}-menu"));
    }
}

/// The Visualizations pane's Analytics tab (G56): a column chart with error bars and an average
/// line, its Error bars card open; a line chart with an error band; a scatter chart with its
/// ratio line and symmetry shading, its Ratio line card open; in light, dark and French.
#[test]
fn analytics_screenshots() {
    use bicraft_engine::Session;
    use bicraft_ui::ribbon::VisualSel;
    use bicraft_ui::theme::Brightness;
    use serde_json::json;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("analytics-shots");
    let mut s = Session::new();
    let sales = r#"#table({"Category", "Amount", "Units", "Hi", "Lo"}, {{"Wheels", 300, 40, 40, 30}, {"Toys", 180, 55, 25, 20}, {"Games", 120, 20, 30, 15}, {"Books", 60, 35, 10, 10}})"#;
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
            "column.add",
            json!({"table": "Sales", "name": "Units", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Hi", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Lo", "data_type": "double"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "#,0"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Units sold", "expression": "SUM(Sales[Units]) * 5", "format_string": "#,0"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Upper", "expression": "SUM(Sales[Amount]) + SUM(Sales[Hi])"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Lower", "expression": "SUM(Sales[Amount]) - SUM(Sales[Lo])"}),
        ),
        ("report.new", json!({})),
    ] {
        s.execute(id, &p).unwrap();
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let mut add = |ty: &str, rect: [i32; 4], fields: serde_json::Value, lines: serde_json::Value| -> String {
        let r = s
            .execute(
                "visual.add",
                &json!({"page": page, "visual_type": ty, "x": rect[0], "y": rect[1], "width": rect[2],
                        "height": rect[3], "fields": fields}),
            )
            .unwrap();
        let v = r["visual"].as_str().unwrap().to_string();
        s.execute(
            "visual.analytics",
            &json!({"page": page, "visual": v, "lines": lines}),
        )
        .unwrap();
        v
    };
    let cat_total =
        json!([{"role": "Category", "field": "Sales[Category]"}, {"role": "Y", "field": "[Total]"}]);
    let column = add(
        "clusteredColumnChart",
        [20, 20, 460, 300],
        cat_total.clone(),
        json!([{"kind": "error_bars", "upper": "[Upper]", "lower": "[Lower]"},
               {"kind": "average", "color": "#E66C37", "data_label": true}]),
    );
    add(
        "lineChart",
        [500, 20, 460, 300],
        cat_total,
        json!([{"kind": "error_bars", "upper": "[Upper]", "lower": "[Lower]", "band": true, "band_style": "fillLine"}]),
    );
    let scatter = add(
        "scatterChart",
        [20, 340, 460, 300],
        json!([{"role": "Category", "field": "Sales[Category]"}, {"role": "X", "field": "[Units sold]"},
               {"role": "Y", "field": "[Total]"}]),
        json!([{"kind": "symmetry_shading"}, {"kind": "ratio_line", "color": "#744EC2"}]),
    );
    let path = dir.path().join("shop.pbip");
    s.execute("file.save_as", &json!({"path": path})).unwrap();
    for (lang, suffix, b) in [
        ("en-US", "light", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
        ("fr-FR", "fr", Brightness::Light),
    ] {
        let fr = lang == "fr-FR";
        let t = |en: &'static str, fr_text: &'static str| if fr { fr_text } else { en };
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
        app.prefs.panes.filters = false;
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1500.0, 900.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        h.state_mut().cmd.visual = Some(VisualSel {
            page: page.clone(),
            visual: column.clone(),
        });
        for _ in 0..3 {
            h.step();
        }
        h.get_by_role_and_label(egui::accesskit::Role::Button, t("Analytics", "Analyse"))
            .click();
        wait(&mut h, "the visual's props", |a| {
            ready(a) && a.panes.format.own.as_ref().is_some_and(|o| o.visual == column)
        });
        let card = |h: &mut Harness<'_, App>, title: &str| {
            let r = h
                .query_by(|x| {
                    x.label()
                        .is_some_and(|l| l.starts_with(title) && l.ends_with(')'))
                })
                .unwrap_or_else(|| panic!("no {title} card"))
                .rect();
            super::click(h, r.center(), false);
        };
        card(&mut h, t("Error bars", "Barres d'erreur"));
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("analytics-{suffix}-error-bars"));
        h.state_mut().cmd.visual = Some(VisualSel {
            page: page.clone(),
            visual: scatter.clone(),
        });
        wait(&mut h, "the scatter's props", |a| {
            ready(a) && a.panes.format.own.as_ref().is_some_and(|o| o.visual == scatter)
        });
        card(&mut h, t("Ratio line", "Ligne de rapport"));
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("analytics-{suffix}-scatter"));
    }
}

/// Anomaly detection (G56): a line chart over sixteen weeks with two spikes, its expected range
/// and anomaly markers (BIcraft's estimate), the Analytics tab's Find anomalies card open; in
/// light, dark and French.
#[test]
fn anomaly_screenshots() {
    use bicraft_engine::Session;
    use bicraft_ui::ribbon::VisualSel;
    use bicraft_ui::theme::Brightness;
    use serde_json::json;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("anomaly-shots");
    let mut s = Session::new();
    let amounts = [
        120, 124, 119, 127, 125, 130, 196, 128, 131, 126, 133, 129, 70, 134, 136, 132,
    ];
    let rows: Vec<String> = amounts
        .iter()
        .enumerate()
        .map(|(i, a)| format!("{{{}, {a}}}", i + 1))
        .collect();
    let sales = format!(r#"#table({{"Week", "Amount"}}, {{{}}})"#, rows.join(", "));
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        (
            "table.add",
            json!({"name": "Sales", "partition": {"source_type": "m", "expression": sales}}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Week", "data_type": "int64"}),
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
    ] {
        s.execute(id, &p).unwrap();
    }
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let r = s
        .execute(
            "visual.add",
            &json!({"page": page, "visual_type": "lineChart", "x": 20, "y": 20, "width": 760,
                    "height": 380, "fields": [{"role": "Category", "field": "Sales[Week]"},
                                              {"role": "Y", "field": "[Total]"}]}),
        )
        .unwrap();
    let line = r["visual"].as_str().unwrap().to_string();
    s.execute(
        "visual.analytics",
        &json!({"page": page, "visual": line, "lines": [{"kind": "anomaly_detection"}]}),
    )
    .unwrap();
    let path = dir.path().join("shop.pbip");
    s.execute("file.save_as", &json!({"path": path})).unwrap();
    for (lang, suffix, b) in [
        ("en-US", "light", Brightness::Light),
        ("en-US", "dark", Brightness::Dark),
        ("fr-FR", "fr", Brightness::Light),
    ] {
        let fr = lang == "fr-FR";
        let t = |en: &'static str, fr_text: &'static str| if fr { fr_text } else { en };
        let mut app = crate::app(lang, None).with_sync_render();
        app.set_theme(b);
        app.prefs.panes.filters = false;
        app.open_path(&path);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1500.0, 900.0))
            .wgpu()
            .build_ui_state(
                |ui, app: &mut App| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
        wait(&mut h, "the first snapshot", ready);
        h.state_mut().cmd.visual = Some(VisualSel {
            page: page.clone(),
            visual: line.clone(),
        });
        for _ in 0..3 {
            h.step();
        }
        h.get_by_role_and_label(egui::accesskit::Role::Button, t("Analytics", "Analyse"))
            .click();
        wait(&mut h, "the visual's props", |a| {
            ready(a) && a.panes.format.own.as_ref().is_some_and(|o| o.visual == line)
        });
        let title = t("Find anomalies", "Rechercher les anomalies");
        let r = h
            .query_by(|x| {
                x.label()
                    .is_some_and(|l| l.starts_with(title) && l.ends_with(')'))
            })
            .unwrap_or_else(|| panic!("no {title} card"))
            .rect();
        super::click(&mut h, r.center(), false);
        for _ in 0..4 {
            h.step();
        }
        // the card's options, scrolled into view
        let last = format!(
            "{title} {}",
            t("Range transparency (%)", "Transparence de la plage (%)")
        );
        h.get_by_label(&last).scroll_to_me();
        for _ in 0..4 {
            h.step();
        }
        save(&mut h, &shots, &format!("analytics2-{suffix}-anomalies"));
    }
}

/// The Model view's Data / Model pane (the Model explorer with a calculation group open, a
/// calculation item being dragged) and a second layout tab: English light, French, dark.
#[test]
fn model_explorer_screenshots() {
    use super::model_explorer_tests::{click_pane, pane_rect, rich_star};
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("model-explorer-shots");
    let path = rich_star(dir.path());
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let fr = suffix == "fr";
        let t = |en: &'static str, f: &'static str| if fr { f } else { en };
        let mut app = crate::app(lang, None).with_sync_render();
        app.open_path(&path);
        app.tab = Tab::Model;
        let mut h = Harness::builder()
            .with_size(egui::vec2(1500.0, 900.0))
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
        wait(&mut h, "the diagram", |a| a.model_view.diagram().is_some());
        h.step();
        save(&mut h, &shots, &format!("model-explorer-{suffix}-data"));
        click_pane(&mut h, t("Model", "Modèle"));
        click_pane(&mut h, t("Calculation groups (1)", "Groupes de calcul (1)"));
        click_pane(&mut h, "Time");
        click_pane(&mut h, t("Relationships (4)", "Relations (4)"));
        click_pane(&mut h, "Sales[RegionKey] → Region[RegionKey]");
        h.step();
        save(&mut h, &shots, &format!("model-explorer-{suffix}-model"));
        // a calculation item held over another (the drop line)
        let from = pane_rect(&h, "PY").center();
        let to = pane_rect(&h, "Current").center();
        h.hover_at(from);
        h.step();
        h.event(egui::Event::PointerButton {
            pos: from,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        for k in 1..=5 {
            h.hover_at(from + (to - from) * (k as f32 / 5.0));
            h.step();
        }
        save(&mut h, &shots, &format!("model-explorer-{suffix}-drag"));
        h.event(egui::Event::PointerButton {
            pos: to,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        // a second layout with two tables
        let name = t("Layout 1", "Disposition 1");
        h.state_mut().model_view.layout = None;
        bicraft_ui::ribbon::send(
            h.state_mut(),
            bicraft_ui::registry::find("diagram.add").unwrap(),
            serde_json::json!({"name": name, "tables": ["Sales", "Product"]}),
        );
        wait(&mut h, "the layout", |a| {
            a.model_view.shown_layout.as_deref() == Some(name)
                && a.model_view.diagram().is_some_and(|d| d.cards.len() == 2)
        });
        h.step();
        save(&mut h, &shots, &format!("model-explorer-{suffix}-layout"));
    }
}
