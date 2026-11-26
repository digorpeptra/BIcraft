//! Screenshots of Manage security roles drawn with wgpu (`--features gpu-tests`), saved into
//! `BICRAFT_DESKTOP_SHOTS` when set: the default editor's conditions, the DAX editor after a
//! Check with an error, and the warning before a DAX-only filter is dropped, in English light,
//! French and dark.

use std::path::PathBuf;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::modelsec_shots::{explorer_menu, open, shop_with, steps};
use super::{Temp, wait};
use crate::App;
use bicraft_ui::role_filter::{Cond, Join, Op};
use bicraft_ui::roles_dialog::{Editor, Mode};

fn save(h: &mut Harness<'_, App>, dir: &Option<PathBuf>, name: &str) {
    let img = h.render().unwrap();
    assert!(img.width() > 0 && img.height() > 0);
    let Some(dir) = dir else { return };
    std::fs::create_dir_all(dir).unwrap();
    let size = tiny_skia::IntSize::from_wh(img.width(), img.height()).unwrap();
    let pm = tiny_skia::Pixmap::from_vec(img.as_raw().clone(), size).unwrap();
    pm.save_png(dir.join(format!("{name}.png"))).unwrap();
}

fn edit(h: &mut Harness<'_, App>, f: impl FnOnce(&mut Editor)) {
    if let Some(e) = h.state_mut().cmd.roles.as_mut().and_then(|d| d.editor.as_mut()) {
        f(e);
    }
    steps(h);
}

#[test]
fn manage_security_roles_screenshots() {
    let shots = bicraft_env::var_os("BICRAFT_DESKTOP_SHOTS").map(PathBuf::from);
    let dir = Temp::new("roles-shots");
    let path = shop_with(
        dir.path(),
        "roles.pbip",
        vec![
            ("role.add", json!({"name": "Readers"})),
            ("role.add", json!({"name": "Managers"})),
            (
                "role.set_filter",
                json!({"role": "Readers", "table": "Sales", "filter_expression": "[Region] = \"E\""}),
            ),
            (
                "role.set_permission",
                json!({"role": "Readers", "table": "Notes", "metadata_permission": "none"}),
            ),
        ],
    );
    for (lang, suffix, dark) in [
        ("en-US", "en", false),
        ("fr-FR", "fr", false),
        ("en-US", "dark", true),
    ] {
        let (model, row, item, check) = if lang == "fr-FR" {
            ("Modèle", "Rôles (2)", "Gérer les rôles", "Vérifier")
        } else {
            ("Model", "Roles (2)", "Manage roles", "Check")
        };
        let mut h = open(&path, lang, dark, model);
        explorer_menu(&mut h, row, item);
        wait(&mut h, "the dialog", |a| {
            a.cmd.roles.as_ref().is_some_and(|d| d.editor.is_some())
        });
        // the default editor: the saved condition and a second one joined by Or
        edit(&mut h, |e| {
            e.rows.push(Cond {
                join: Join::Or,
                column: "Amount".into(),
                op: Op::Ge,
                value: "10".into(),
            });
        });
        save(&mut h, &shots, &format!("roles-default-{suffix}"));
        // the DAX editor, checked: an unknown column
        edit(&mut h, |e| {
            e.mode = Mode::Dax;
            e.dax.text = "[Region] = \"E\"\n    && [Amont] > 10\n    && USERPRINCIPALNAME() <> \"\"".into();
        });
        let at = h.get_by_label(check).rect();
        super::click(&mut h, at.center(), false);
        wait(&mut h, "the check", |a| {
            a.cmd
                .roles
                .as_ref()
                .and_then(|d| d.editor.as_ref())
                .is_some_and(|e| !e.checking && e.verdict.is_some())
        });
        steps(&mut h);
        save(&mut h, &shots, &format!("roles-dax-{suffix}"));
        // the switch back to the default editor warns first
        edit(&mut h, |e| e.confirm_switch = true);
        save(&mut h, &shots, &format!("roles-switch-{suffix}"));
    }
}
