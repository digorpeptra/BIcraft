//! Screenshots of the conditional-formatting dialog (docs/gaps.md G54): a table's Cell elements,
//! the data bars, icons, gradient (numbers, a middle colour) and rules (two conditions) dialogs
//! with their previews, then the table drawn with data bars and icons; light, dark and French
//! (`--features gpu-tests`, saved into `BICRAFT_DESKTOP_SHOTS`).

use std::path::PathBuf;

use egui_kittest::Harness;
use serde_json::{Value, json};

use super::{Temp, ready, shop, wait};
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

fn steps(h: &mut Harness<'_, App>, n: usize) {
    // the pointer away from the visuals (no hover header over the dialog)
    h.event(egui::Event::PointerMoved(egui::pos2(1300.0, 860.0)));
    for _ in 0..n {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn conditional_dialog_screenshots() {
    use bicraft_ui::actions::Call;
    use bicraft_ui::app::Then;
    use bicraft_ui::conditional_dialog::{self as cf, Case, Stop};
    use bicraft_ui::format_pane::{Group, Tab as BuildTab};
    use bicraft_ui::ribbon::VisualSel;
    use bicraft_ui::theme::Brightness;
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("cf-dialog-shots");
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
        // a table with data bars and icons on [Total]
        h.state_mut().run(
            Call {
                id: "visual.add",
                params: json!({"page": page, "visual_type": "tableEx", "x": 880, "y": 160, "width": 360,
                    "height": 220, "fields": [{"role": "Values", "field": "Sales[Category]"},
                                             {"role": "Values", "field": "[Total]"}]}),
            },
            Then::Nothing,
        );
        wait(&mut h, "visual.add", |a| a.calls_pending() == 0);
        wait(&mut h, "the table", ready);
        let snap = h.state().loaded.as_ref().unwrap().snap.clone();
        let v = snap["report"]["pages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|p| p["visuals"].as_array().cloned().unwrap_or_default())
            .find(|v| v["type"] == "tableEx")
            .unwrap();
        let table = v["name"].as_str().unwrap().to_string();
        let sel = VisualSel {
            page: page.clone(),
            visual: table.clone(),
        };
        h.state_mut().cmd.visual = Some(sel.clone());
        h.state_mut().prefs.panes.build_width = 300.0;
        {
            let f = &mut h.state_mut().panes.format;
            f.tab = BuildTab::Format;
            f.group = Group::Visual;
            f.open.clear();
            f.open.insert("cellElements".into());
        }
        steps(&mut h, 20);
        wait(&mut h, "visual.props", |a| a.calls_pending() == 0);
        steps(&mut h, 4);
        save(&mut h, &shots, &format!("condfmt-{suffix}-cell-elements"));
        let open = |h: &mut Harness<'_, App>, target: &'static str, name: &str| {
            let app = h.state_mut();
            cf::open(app, &sel, &v, target, name.to_string());
        };
        // data bars, with a negative minimum, right to left
        open(&mut h, "data_bars", "Total");
        if let Some(d) = h.state_mut().panes.format.cf.as_mut() {
            d.bars.min = Some("-50".into());
            d.bars.negative = "#D64550".into();
        }
        steps(&mut h, 6);
        save(&mut h, &shots, &format!("condfmt-{suffix}-data-bars"));
        h.state_mut().panes.format.cf = None;
        // icons: arrows, right of data, middle
        open(&mut h, "icon", "Total");
        if let Some(d) = h.state_mut().panes.format.cf.as_mut() {
            d.layout = "after";
            d.align = "middle";
        }
        steps(&mut h, 6);
        save(&mut h, &shots, &format!("condfmt-{suffix}-icons"));
        h.state_mut().panes.format.cf = None;
        // a diverging gradient with numbers, empty values as zero
        open(&mut h, "background", "Values Background");
        if let Some(d) = h.state_mut().panes.format.cf.as_mut() {
            d.min = Stop {
                color: "#D64550".into(),
                number: Some("0".into()),
                percent: false,
            };
            d.mid = Some(Stop {
                color: "#FFFFFF".into(),
                number: Some("50".into()),
                percent: false,
            });
            d.max = Stop {
                color: "#1AAB40".into(),
                number: Some("150".into()),
                percent: false,
            };
            d.blank = cf::Blank::Zero;
            d.apply = "values_and_totals";
        }
        steps(&mut h, 6);
        save(&mut h, &shots, &format!("condfmt-{suffix}-gradient"));
        // rules with a second condition
        if let Some(d) = h.state_mut().panes.format.cf.as_mut() {
            d.style = cf::Style::Rules;
            d.cases = vec![
                Case::new("<", "20", "#D64550"),
                Case::two(">=", "20", "<", "100", "#F2C811"),
                Case::new(">=", "100", "#1AAB40"),
            ];
        }
        steps(&mut h, 6);
        save(&mut h, &shots, &format!("condfmt-{suffix}-rules"));
        h.state_mut().panes.format.cf = None;
        // the table drawn with them
        let formats: Value = json!([
            {"target": "data_bars", "series": "[Total]", "rule": {"data_bars": {"positive": "#118DFF",
                "negative": "#D64550", "axis": "#605E5C", "min": -50}}},
            {"target": "icon", "series": "[Total]", "rule": {"icons": {"field": "[Total]", "preset": "arrows",
                "thresholds": [20, 100], "position": "after", "align": "middle"}}},
            {"target": "font_color", "series": "Sales[Category]", "rule": {"rules": {"field": "[Total]",
                "cases": [{"op": ">=", "value": 100, "color": "#1AAB40"}], "else": "#D64550"}}}]);
        h.state_mut().run(
            Call {
                id: "visual.conditional",
                params: json!({"page": page, "visual": table, "conditional": formats}),
            },
            Then::Nothing,
        );
        wait(&mut h, "visual.conditional", |a| a.calls_pending() == 0);
        wait(&mut h, "the formatted table", ready);
        steps(&mut h, 20);
        wait(&mut h, "visual.props", |a| a.calls_pending() == 0);
        wait(&mut h, "the formatted table", ready);
        steps(&mut h, 4);
        save(&mut h, &shots, &format!("condfmt-{suffix}-canvas"));
    }
}
