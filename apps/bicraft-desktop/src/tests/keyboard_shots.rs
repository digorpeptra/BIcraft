//! Screenshots of the keyboard focus ring drawn with wgpu (`--features gpu-tests`), saved into
//! `BICRAFT_DESKTOP_SHOTS` when set: a ribbon button reached by Ctrl+F6 and Tab, then the
//! Manage relationships dialog opened with Enter and its first control focused, in English
//! light, French and dark.

use std::path::PathBuf;

use egui::{Event, Key, Modifiers};
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

fn press(h: &mut Harness<'_, App>, key: Key, modifiers: Modifiers) {
    for pressed in [true, false] {
        h.event_modifiers(
            Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            },
            modifiers,
        );
    }
    steps(h);
}

#[test]
fn focus_ring_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("keyboard-shots");
    let path = shop_with(
        dir.path(),
        "kbd.pbip",
        vec![(
            "relationship.add",
            json!({"from": "Sales[ProductKey]", "to": "Product[ProductKey]"}),
        )],
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
        let target = h
            .get_all_by_label(button)
            .map(|n| n.rect())
            .min_by(|a, b| a.min.y.total_cmp(&b.min.y))
            .unwrap();
        // Ctrl+F6 to the ribbon, then Tab to the button
        press(&mut h, Key::F6, Modifiers::CTRL);
        for _ in 0..120 {
            let at = h.ctx.memory(|m| m.focused()).and_then(|id| {
                h.ctx
                    .viewport(|v| v.prev_pass.widgets.get(id).map(|w| w.interact_rect))
            });
            if at.is_some_and(|r| (r.center() - target.center()).length() < 2.0) {
                break;
            }
            press(&mut h, Key::Tab, Modifiers::NONE);
        }
        assert!(h.state().keyboard.keyboard, "the ring is shown");
        save(&mut h, &shots, &format!("keyboard-ribbon-{suffix}"));
        press(&mut h, Key::Enter, Modifiers::NONE);
        wait(&mut h, "the list", |a| {
            a.cmd.rels.as_ref().is_some_and(|m| m.rows.is_some())
        });
        steps(&mut h);
        press(&mut h, Key::Tab, Modifiers::NONE);
        save(&mut h, &shots, &format!("keyboard-dialog-{suffix}"));
        press(&mut h, Key::Escape, Modifiers::NONE);
        assert!(h.state().cmd.rels.is_none());
        save(&mut h, &shots, &format!("keyboard-back-{suffix}"));
    }
}
