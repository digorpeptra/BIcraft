//! Screenshots of the TMDL view and of the Model explorer's Functions drawn with wgpu
//! (`--features gpu-tests`), saved into `BICRAFT_DESKTOP_SHOTS` when set: a scripted table and
//! function applied with the answer, an error with its line and column, and the Functions node
//! with a function's menu, in English light, French and dark.

use std::path::PathBuf;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::modelsec_shots::{open, shop_with, steps};
use super::{Temp, wait};
use crate::App;
use bicraft_ui::app::Tab as View;
use bicraft_ui::tmdl_view;

fn save(h: &mut Harness<'_, App>, dir: &Option<PathBuf>, name: &str) {
    let img = h.render().unwrap();
    assert!(img.width() > 0 && img.height() > 0);
    let Some(dir) = dir else { return };
    std::fs::create_dir_all(dir).unwrap();
    let size = tiny_skia::IntSize::from_wh(img.width(), img.height()).unwrap();
    let pm = tiny_skia::Pixmap::from_vec(img.as_raw().clone(), size).unwrap();
    pm.save_png(dir.join(format!("{name}.png"))).unwrap();
}

fn script(h: &mut Harness<'_, App>, objects: serde_json::Value) {
    let n = h.state().cmd.tmdl.tabs.len();
    let before: Vec<String> = h.state().cmd.tmdl.tabs.iter().map(|t| t.text.clone()).collect();
    bicraft_ui::ribbon::invoke(h.state_mut(), tmdl_view::SCRIPT, json!({"objects": objects}));
    wait(h, "the script", |a| {
        a.cmd.tmdl.tabs.len() > n
            || a.cmd.tmdl.tabs.iter().map(|t| t.text.clone()).collect::<Vec<_>>() != before
    });
    steps(h);
}

fn apply_and_wait(h: &mut Harness<'_, App>) {
    tmdl_view::apply(h.state_mut());
    wait(h, "the apply", |a| {
        a.cmd
            .tmdl
            .active_tab()
            .is_some_and(|q| !q.applying && q.note.is_some())
    });
    steps(h);
}

#[test]
fn tmdl_view_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("tmdl-shots");
    let path = shop_with(
        dir.path(),
        "tmdl.pbip",
        vec![
            (
                "function.add",
                json!({"name": "Sales.AddTax", "description": "Adds the tax",
                       "expression": "(amount : NUMERIC, rate : NUMERIC = 0.2) =>\n    amount * (1 + rate)"}),
            ),
            (
                "measure.add",
                json!({"table": "Sales", "name": "With Tax", "expression": "Sales.AddTax([Total])"}),
            ),
        ],
    );
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let model = if lang == "fr-FR" { "Modèle" } else { "Model" };
        let mut h = open(&path, lang, dark, model);
        // the Model explorer's Functions, open, with a function's menu
        h.state_mut().model_view.explorer.tab = bicraft_ui::model_explorer::Tab::Model;
        h.state_mut().model_view.explorer.open.insert("fun".to_string());
        steps(&mut h);
        let r = h.get_by_label("Sales.AddTax").rect();
        super::model_explorer_tests::right_click(&mut h, r.center());
        steps(&mut h);
        save(&mut h, &shots, &format!("tmdl-functions-{suffix}"));
        h.state_mut().show_tab(View::Tmdl);
        steps(&mut h);
        // the measures of Sales and the function, edited, then applied
        script(
            &mut h,
            json!([
                {"kind": "measure", "table": "Sales", "name": "Total"},
                {"kind": "measure", "table": "Sales", "name": "With Tax"},
                {"kind": "function", "name": "Sales.AddTax"},
            ]),
        );
        let l = h.state().lang;
        {
            let v = &mut h.state_mut().cmd.tmdl;
            if let Some(t) = v.tabs.get_mut(v.active) {
                t.text = t.text.replace("= 0.2", "= 0.25")
                    + "\n\tref table Sales\n\n\t\tmeasure Average = DIVIDE([Total], COUNTROWS(Sales))\n\t\t\tformatString: 0.00\n";
            }
        }
        steps(&mut h);
        apply_and_wait(&mut h);
        save(&mut h, &shots, &format!("tmdl-apply-{suffix}"));
        // an error: a second tab, an unknown column
        h.state_mut().cmd.tmdl.add(l, String::new());
        {
            let v = &mut h.state_mut().cmd.tmdl;
            if let Some(t) = v.tabs.get_mut(v.active) {
                t.text = "createOrReplace\n\n\tref table Sales\n\n\t\tmeasure Margin = SUM(Sales[Profit]) / [Total]\n".to_string();
            }
        }
        steps(&mut h);
        apply_and_wait(&mut h);
        save(&mut h, &shots, &format!("tmdl-error-{suffix}"));
    }
}
