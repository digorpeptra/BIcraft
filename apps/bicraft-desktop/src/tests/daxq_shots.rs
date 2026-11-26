//! Screenshots of the DAX query view drawn with wgpu (`--features gpu-tests`), saved into
//! `BICRAFT_DESKTOP_SHOTS` when set: a query run with its grid and row count, a DEFINE MEASURE
//! query with its Update model lenses after one was applied, and an error inline, with three
//! query tabs, in English light, French and dark.

use std::path::PathBuf;

use egui_kittest::Harness;

use super::modelsec_shots::{open, shop_with, steps};
use super::{Temp, wait};
use crate::App;
use bicraft_ui::app::Tab as View;
use bicraft_ui::dax_query;

fn save(h: &mut Harness<'_, App>, dir: &Option<PathBuf>, name: &str) {
    let img = h.render().unwrap();
    assert!(img.width() > 0 && img.height() > 0);
    let Some(dir) = dir else { return };
    std::fs::create_dir_all(dir).unwrap();
    let size = tiny_skia::IntSize::from_wh(img.width(), img.height()).unwrap();
    let pm = tiny_skia::Pixmap::from_vec(img.as_raw().clone(), size).unwrap();
    pm.save_png(dir.join(format!("{name}.png"))).unwrap();
}

fn set_text(h: &mut Harness<'_, App>, text: &str) {
    let l = h.state().lang;
    if let Some(t) = h.state_mut().cmd.dax_query.current(l) {
        t.text = text.to_string();
    }
    steps(h);
}

fn run_and_wait(h: &mut Harness<'_, App>) {
    dax_query::run(h.state_mut());
    wait(h, "the query's answer", |a| {
        a.cmd
            .dax_query
            .active_tab()
            .is_some_and(|q| !q.running && q.outcome.is_some())
    });
    steps(h);
}

#[test]
fn dax_query_view_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("daxq-shots");
    let path = shop_with(dir.path(), "daxq.pbip", Vec::new());
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let model = if lang == "fr-FR" { "Modèle" } else { "Model" };
        let mut h = open(&path, lang, dark, model);
        h.state_mut().show_tab(View::Query);
        steps(&mut h);
        let l = h.state().lang;
        set_text(
            &mut h,
            "EVALUATE\n    SUMMARIZECOLUMNS(\n        Sales[Region],\n        \"Total\", [Total],\n        \"Lines\", COUNTROWS(Sales)\n    )\nORDER BY Sales[Region]",
        );
        run_and_wait(&mut h);
        h.state_mut().cmd.dax_query.add(l, String::new());
        h.state_mut().cmd.dax_query.add(l, String::new());
        h.state_mut().cmd.dax_query.active = 0;
        steps(&mut h);
        save(&mut h, &shots, &format!("daxq-run-{suffix}"));
        // a DEFINE block: one measure applied (its lens now overwrites), one still new
        h.state_mut().cmd.dax_query.active = 1;
        set_text(
            &mut h,
            "DEFINE\n    MEASURE Sales[Average] = DIVIDE([Total], [Lines])\n    MEASURE Sales[Lines] = COUNTROWS(Sales)\n    MEASURE Sales[Total] = SUM(Sales[Amount]) * 1.2\n\nEVALUATE\n    ROW(\"Average\", [Average], \"Lines\", [Lines])",
        );
        run_and_wait(&mut h);
        dax_query::apply(h.state_mut(), Some("Lines"));
        wait(&mut h, "the update", |a| {
            a.cmd.dax_query.active_tab().is_some_and(|q| q.note.is_some())
        });
        steps(&mut h);
        save(&mut h, &shots, &format!("daxq-define-{suffix}"));
        // an error inline
        h.state_mut().cmd.dax_query.active = 2;
        set_text(&mut h, "EVALUATE ROW(\"x\", NOSUCHFUNCTION(1))");
        run_and_wait(&mut h);
        save(&mut h, &shots, &format!("daxq-error-{suffix}"));
    }
}
