//! Screenshots of Modeling › Manage relationships drawn with wgpu (`--features gpu-tests`), saved
//! into `BICRAFT_DESKTOP_SHOTS` when set: the list, and the Create relationship window with both
//! tables' previews, the columns picked and the review, in English light, French and dark.

use std::path::PathBuf;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::modelsec_shots::{open, shop_with, steps};
use super::{Temp, wait};
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

#[test]
fn manage_relationships_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("rels-shots");
    let path = shop_with(
        dir.path(),
        "rels.pbip",
        vec![
            (
                "relationship.add",
                json!({"from": "Sales[ProductKey]", "to": "Product[ProductKey]"}),
            ),
            (
                "relationship.add",
                json!({"from": "Budget[Project]", "to": "Priority[Project]",
                       "from_cardinality": "one", "to_cardinality": "one"}),
            ),
        ],
    );
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let (model, button) = if lang == "fr-FR" {
            ("Modèle", "Gérer les relations")
        } else {
            ("Model", "Manage relationships")
        };
        let mut h = open(&path, lang, dark, model);
        let at = h
            .get_all_by_label(button)
            .map(|n| n.rect())
            .min_by(|a, b| a.min.y.total_cmp(&b.min.y))
            .unwrap();
        super::click(&mut h, at.center(), false);
        wait(&mut h, "the list", |a| {
            a.cmd.rels.as_ref().is_some_and(|m| m.rows.is_some())
        });
        steps(&mut h);
        save(&mut h, &shots, &format!("rels-list-{suffix}"));
        // New: Sales[Region] to Region[Region], as picked in the previews
        let mut e = bicraft_ui::relationships_dialog::Editor {
            active: true,
            active_auto: true,
            ..Default::default()
        };
        for (side, table, column) in [(0, "Sales", "Region"), (1, "Region", "Region")] {
            e.sides[side].table = table.into();
            e.sides[side].column = Some(column.into());
        }
        e.adopt = true;
        h.state_mut().cmd.rel_edit = Some(e);
        wait(&mut h, "the previews and the review", |a| {
            a.cmd.rel_edit.as_ref().is_some_and(|e| {
                e.verdict.is_some() && e.sides.iter().all(|s| s.preview.is_some()) && !e.adopt
            })
        });
        steps(&mut h);
        save(&mut h, &shots, &format!("rels-new-{suffix}"));
        // a column with duplicates on the "one" side: the review's error
        if let Some(e) = h.state_mut().cmd.rel_edit.as_mut() {
            e.sides[1].table = "Notes".into();
            e.sides[1].column = Some("Project".into());
            e.sides[0].table = "Budget".into();
            e.sides[0].column = Some("Project".into());
            e.card = bicraft_ui::relationships_dialog::Card::OneToOne;
            e.verdict = None;
        }
        wait(&mut h, "the review", |a| {
            a.cmd.rel_edit.as_ref().is_some_and(|e| {
                e.verdict.as_ref().is_some_and(|v| !v.errors.is_empty())
                    && e.sides.iter().all(|s| s.preview.is_some())
            })
        });
        steps(&mut h);
        save(&mut h, &shots, &format!("rels-error-{suffix}"));
    }
}
