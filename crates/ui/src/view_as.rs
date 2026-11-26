//! View as (Power BI Desktop's Modeling › View as): the dialog (None, Other user and its name,
//! a check box per role of the model, OK) and, while it is on, the banner "Now viewing as: …"
//! with Stop viewing. Both run `view.interact {action: "view_as"}`: view state, so the model,
//! the report, the revision and the files never change; the engine applies the roles' row
//! filters to every evaluation (`docs/view-snapshot.md`, "View as"). On a server each viewer has
//! its own View as, which only narrows what that viewer already reads.

use egui::{RichText, Ui, vec2};
use serde_json::{Value, json};

use crate::App;
use crate::i18n::Lang;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::widgets;

/// The engine command the dialog and the banner run.
pub const COMMAND: &str = "view.interact";
/// Longest "Other user" name the engine takes.
const MAX_USER: usize = 256;

/// The dialog's state while it is open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dialog {
    /// "None": stop viewing as anyone.
    pub none: bool,
    /// "Other user", and the name typed.
    pub other: bool,
    pub user: String,
    /// The model's roles and whether each is checked.
    pub roles: Vec<(String, bool)>,
}

/// The View as state the snapshot shows: (roles, user); `None` when off.
pub fn current(app: &App) -> Option<(Vec<String>, Option<String>)> {
    let v = app.loaded.as_ref()?.snap.pointer("/interaction/view_as")?;
    if v.is_null() {
        return None;
    }
    let roles = v
        .get("roles")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let user = v.get("user").and_then(Value::as_str).map(str::to_string);
    Some((roles, user))
}

/// The model's role names from the snapshot.
fn model_roles(app: &App) -> Vec<String> {
    app.loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/model/roles"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

/// Opens the dialog on the current state.
pub fn open(app: &mut App) {
    let now = current(app);
    let (on, user) = match &now {
        Some((roles, user)) => (roles.clone(), user.clone()),
        None => (Vec::new(), None),
    };
    app.cmd.view_as = Some(Dialog {
        none: now.is_none(),
        other: user.is_some(),
        user: user.unwrap_or_default(),
        roles: model_roles(app)
            .into_iter()
            .map(|r| {
                let checked = on.iter().any(|x| x.eq_ignore_ascii_case(&r));
                (r, checked)
            })
            .collect(),
    });
}

/// The `view.interact` parameters of the dialog's choice.
pub fn action(d: &Dialog) -> Value {
    let roles: Vec<&str> = d
        .roles
        .iter()
        .filter(|(_, on)| *on)
        .map(|(r, _)| r.as_str())
        .collect();
    let user = d.user.trim();
    if d.none || (roles.is_empty() && (!d.other || user.is_empty())) {
        return json!({"action": "view_as", "roles": null});
    }
    json!({
        "action": "view_as",
        "roles": roles,
        "user": if d.other && !user.is_empty() { Value::from(user) } else { Value::Null },
    })
}

/// The Modeling tab's Security group button (after the registry's role buttons).
pub fn ribbon(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    // a reader's View as is a reader action, like a slicer click (the server runs it on the
    // reader's own view): the command's needs apply, not the reader's refusal of edits
    let st = crate::registry::enabled(
        COMMAND,
        crate::registry::Context {
            snap: app.loaded.as_ref().map(|x| x.snap.as_ref()),
            open: app.has_doc(),
            reader: false,
        },
    )
    .map_err(|why| crate::registry::reason_text(&why, l));
    widgets::group(ui, |ui| {
        let on = current(app).is_some();
        let r = widgets::toggle_button(
            ui,
            Icon::Lock,
            l.t("View as", "Afficher en tant que"),
            st.is_ok(),
            on,
            true,
        );
        let tip = match &st {
            Ok(()) => l
                .t(
                    "See the report as the members of roles see it (row-level security)",
                    "Voir le rapport comme le voient les membres de rôles (sécurité au niveau des lignes)",
                )
                .to_string(),
            Err(why) => why.clone(),
        };
        if r.on_hover_text(tip).clicked() && st.is_ok() {
            open(app);
        }
    });
}

/// Draws the dialog when it is open.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut d) = app.cmd.view_as.clone() else {
        return;
    };
    let l = app.lang;
    let mut open = true;
    let mut close = false;
    egui::Window::new(l.t("View as roles", "Afficher en tant que rôles"))
        .id(egui::Id::unique("bicraft-view-as"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(360.0);
            if let Some(a) = body(ui, &mut d, l, &mut close) {
                app.interact(a);
                close = true;
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
        });
    app.cmd.view_as = (open && !close).then_some(d);
}

/// The dialog's controls; the action to send when OK is clicked (`close` on Cancel).
fn body(ui: &mut Ui, d: &mut Dialog, l: Lang, close: &mut bool) -> Option<Value> {
    let t = Tokens::get(ui.ctx());
    ui.label(l.t(
        "See the report as the members of these roles see it:",
        "Voir le rapport comme le voient les membres de ces rôles :",
    ));
    ui.add_space(6.0);
    if ui.checkbox(&mut d.none, l.t("None", "Aucun")).changed() && d.none {
        d.other = false;
        for (_, on) in &mut d.roles {
            *on = false;
        }
    }
    if ui
        .checkbox(&mut d.other, l.t("Other user", "Autre utilisateur"))
        .changed()
        && d.other
    {
        d.none = false;
    }
    // the name under its label (the French label is long)
    ui.indent("bicraft-view-as-user", |ui| {
        let name = l.t("User name or email", "Nom ou adresse e-mail de l'utilisateur");
        let label = ui.label(RichText::new(name).color(t.text_dim));
        let r = ui.add_enabled(
            d.other,
            egui::TextEdit::singleline(&mut d.user)
                .char_limit(MAX_USER)
                .hint_text("user@contoso.com")
                .desired_width(300.0),
        );
        r.labelled_by(label.id);
    });
    ui.add_space(4.0);
    if d.roles.is_empty() {
        ui.label(
            RichText::new(l.t(
                "This model has no roles: create one with New role.",
                "Ce modèle n'a aucun rôle : créez-en un avec Nouveau rôle.",
            ))
            .color(t.text_dim),
        );
    }
    egui::ScrollArea::vertical()
        .max_height(240.0)
        .id_salt("bicraft-view-as-roles")
        .show(ui, |ui| {
            for (name, on) in &mut d.roles {
                if ui.checkbox(on, name.as_str()).changed() && *on {
                    d.none = false;
                }
            }
        });
    ui.add_space(10.0);
    let mut go = None;
    ui.horizontal(|ui| {
        if widgets::primary(ui, l.t("OK", "OK"), true, "").clicked() {
            go = Some(action(d));
        }
        if ui.button(l.t("Cancel", "Annuler")).clicked() {
            *close = true;
        }
    });
    go
}

/// The banner over the views while View as is on: who the report is seen as, and Stop viewing.
pub fn banner(app: &mut App, ui: &mut Ui) {
    let Some((roles, user)) = current(app) else {
        return;
    };
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let mut who = roles.join(", ");
    if let Some(u) = user {
        if who.is_empty() {
            who = u;
        } else {
            who = format!("{who} ({u})");
        }
    }
    let mut stop = false;
    egui::Panel::top("bicraft-view-as-banner")
        .frame(
            egui::Frame::NONE
                .fill(t.accent_soft)
                .inner_margin(egui::Margin::symmetric(12, 6)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::hover());
                icons::paint(ui.painter(), r, Icon::Lock, t.text, false);
                ui.label(
                    RichText::new(format!(
                        "{} {}",
                        l.t("Now viewing as:", "Affichage en tant que :"),
                        crate::app::clip(&who, 120)
                    ))
                    .font(theme::bold(13.0))
                    .color(t.text),
                );
                ui.add_space(12.0);
                if ui.button(l.t("Stop viewing", "Arrêter l'affichage")).clicked() {
                    stop = true;
                }
            });
        });
    if stop {
        app.interact(json!({"action": "view_as", "roles": null}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog(roles: &[(&str, bool)], other: bool, user: &str) -> Dialog {
        Dialog {
            none: false,
            other,
            user: user.into(),
            roles: roles.iter().map(|(r, on)| ((*r).to_string(), *on)).collect(),
        }
    }

    #[test]
    fn the_action_follows_the_choice() {
        let d = dialog(&[("East", true), ("West", false)], false, "");
        assert_eq!(
            action(&d),
            json!({"action": "view_as", "roles": ["East"], "user": null})
        );
        let d = dialog(&[("East", true)], true, " bob@x ");
        assert_eq!(
            action(&d),
            json!({"action": "view_as", "roles": ["East"], "user": "bob@x"})
        );
        // nothing chosen, or None: stop viewing
        let d = dialog(&[("East", false)], true, "");
        assert_eq!(action(&d), json!({"action": "view_as", "roles": null}));
        let mut d = dialog(&[("East", true)], true, "bob@x");
        d.none = true;
        assert_eq!(action(&d), json!({"action": "view_as", "roles": null}));
        // Other user alone
        let d = dialog(&[("East", false)], true, "bob@x");
        assert_eq!(
            action(&d),
            json!({"action": "view_as", "roles": [], "user": "bob@x"})
        );
    }
}
