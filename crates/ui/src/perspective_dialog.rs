//! Manage perspectives (the Model explorer's Perspectives menu, or a perspective's): the model's
//! perspectives on the left (New perspective, Delete), and for
//! the perspective chosen, each table with its columns, measures and hierarchies to tick; Save
//! runs `perspective.set` with the ticked members (and the new name when it was changed), one
//! undo step. The dialog reads the snapshot's `model.perspective_members`
//! (docs/view-snapshot.md). The Model view's Perspective list filters the diagram to one.

use std::collections::{BTreeMap, BTreeSet};

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::Then;
use crate::i18n::Lang;
use crate::theme::{self, Tokens};
use crate::widgets;

pub const ADD: &str = "perspective.add";
pub const SET: &str = "perspective.set";
pub const REMOVE: &str = "perspective.remove";
/// Most tables and members listed.
const MAX_LISTED: usize = 2000;
/// The height of the perspectives and tables columns.
const BODY_HEIGHT: f32 = 400.0;

/// What a perspective holds of one table (lower-cased table name → names).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Members {
    pub columns: BTreeSet<String>,
    pub measures: BTreeSet<String>,
    pub hierarchies: BTreeSet<String>,
}

impl Members {
    fn is_empty(&self) -> bool {
        self.columns.is_empty() && self.measures.is_empty() && self.hierarchies.is_empty()
    }
}

/// A table of the model and its members (names as written).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelTable {
    pub name: String,
    pub columns: Vec<String>,
    pub measures: Vec<String>,
    pub hierarchies: Vec<String>,
}

/// The dialog while it is open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dialog {
    /// The perspective shown.
    pub name: Option<String>,
    /// Its name being typed (a rename on Save).
    pub rename: String,
    /// Its members as ticked: table name → members. `None` until read from the snapshot.
    pub draft: Option<BTreeMap<String, Members>>,
    /// The new perspective's name being typed.
    pub new_name: String,
    /// Tables whose members are listed.
    pub open: Vec<String>,
}

fn names(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_LISTED)
        .filter_map(|x| x.as_str().or_else(|| x.get("name").and_then(Value::as_str)))
        .map(str::to_string)
        .collect()
}

fn model(app: &App) -> Option<&Value> {
    app.loaded.as_ref().and_then(|l| l.snap.get("model"))
}

/// The perspectives' names.
pub fn perspectives(app: &App) -> Vec<String> {
    names(model(app).and_then(|m| m.get("perspectives")))
}

/// The model's tables with their members.
pub fn tables(app: &App) -> Vec<ModelTable> {
    model(app)
        .and_then(|m| m.get("tables"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_LISTED)
        .map(|t| ModelTable {
            name: t
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            columns: names(t.get("columns")),
            measures: names(t.get("measures")),
            hierarchies: names(t.get("hierarchies")),
        })
        .collect()
}

/// The members of perspective `name` from a snapshot model: lower-cased table → members.
pub fn members_of(model: &Value, name: &str) -> Option<BTreeMap<String, Members>> {
    let p = model
        .get("perspective_members")
        .and_then(Value::as_array)?
        .iter()
        .find(|p| {
            p.get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })?;
    let set = |v: Option<&Value>| names(v).into_iter().collect::<BTreeSet<String>>();
    Some(
        p.get("tables")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|t| {
                (
                    t.get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_lowercase(),
                    Members {
                        columns: set(t.get("columns")),
                        measures: set(t.get("measures")),
                        hierarchies: set(t.get("hierarchies")),
                    },
                )
            })
            .collect(),
    )
}

/// `perspective.set` of the dialog's draft.
pub fn save_call(d: &Dialog, all: &[ModelTable]) -> Option<Call> {
    let name = d.name.as_ref()?;
    let draft = d.draft.as_ref()?;
    let tables: Vec<Value> = all
        .iter()
        .filter_map(|t| {
            let m = draft.get(&t.name.to_lowercase()).filter(|m| !m.is_empty())?;
            // in the model's order
            let pick = |have: &[String], on: &BTreeSet<String>| -> Vec<String> {
                have.iter().filter(|x| on.contains(*x)).cloned().collect()
            };
            Some(json!({
                "table": t.name,
                "columns": pick(&t.columns, &m.columns),
                "measures": pick(&t.measures, &m.measures),
                "hierarchies": pick(&t.hierarchies, &m.hierarchies),
            }))
        })
        .collect();
    let mut p = json!({"name": name, "tables": tables});
    let rename = d.rename.trim();
    if !rename.is_empty()
        && rename != name
        && let Value::Object(o) = &mut p
    {
        o.insert("new_name".into(), json!(rename));
    }
    Some(Call { id: SET, params: p })
}

/// The ribbon's buttons (or the palette) without their values open the dialog.
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    let open_on = match id {
        ADD if params.get("name").is_none() => None,
        SET if params.get("tables").is_none() && params.get("new_name").is_none() => {
            params.get("name").and_then(Value::as_str).map(str::to_string)
        }
        _ => return false,
    };
    open(app, open_on.as_deref());
    true
}

/// Opens the dialog on `name` (default the first perspective).
pub fn open(app: &mut App, name: Option<&str>) {
    let all = perspectives(app);
    let name = name
        .and_then(|n| all.iter().find(|x| x.eq_ignore_ascii_case(n)))
        .or_else(|| all.first())
        .cloned();
    app.cmd.perspective = Some(Dialog {
        rename: name.clone().unwrap_or_default(),
        name,
        ..Dialog::default()
    });
}

/// Draws the dialog when it is open.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut d) = app.cmd.perspective.clone() else {
        return;
    };
    let l = app.lang;
    let all = perspectives(app);
    if d.name
        .as_ref()
        .is_none_or(|n| !all.iter().any(|x| x.eq_ignore_ascii_case(n)))
    {
        d.name = all.first().cloned();
        d.rename = d.name.clone().unwrap_or_default();
        d.draft = None;
    }
    if d.draft.is_none()
        && let (Some(n), Some(m)) = (d.name.as_deref(), model(app))
    {
        d.draft = members_of(m, n);
    }
    let tabs = tables(app);
    let mut open = true;
    let mut close = false;
    let mut calls = Vec::new();
    egui::Window::new(l.t("Perspectives", "Perspectives"))
        .id(egui::Id::unique("bicraft-perspectives"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(720.0);
            body(ui, &mut d, &all, &tabs, l, &mut calls, &mut close);
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    app.cmd.perspective = (open && !close).then_some(d);
    for c in calls {
        app.run(c, Then::Nothing);
    }
}

#[allow(clippy::too_many_arguments)]
fn body(
    ui: &mut Ui,
    d: &mut Dialog,
    all: &[String],
    tabs: &[ModelTable],
    l: Lang,
    calls: &mut Vec<Call>,
    close: &mut bool,
) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal_top(|ui| {
        // a fixed height: the separator between the columns spans it (not the whole screen)
        ui.set_height(BODY_HEIGHT);
        ui.vertical(|ui| {
            ui.set_width(200.0);
            ui.label(RichText::new(l.t("Perspectives", "Perspectives")).font(theme::bold(13.0)));
            egui::ScrollArea::vertical()
                .id_salt("bicraft-perspectives-list")
                .max_height(320.0)
                .show(ui, |ui| {
                    if all.is_empty() {
                        ui.label(
                            RichText::new(l.t("No perspective yet.", "Aucune perspective pour l'instant."))
                                .color(t.text_dim),
                        );
                    }
                    for n in all {
                        let on = d.name.as_deref() == Some(n.as_str());
                        if ui.selectable_label(on, n.as_str()).clicked() && !on {
                            d.name = Some(n.clone());
                            d.rename = n.clone();
                            d.draft = None;
                        }
                    }
                });
            ui.add_space(6.0);
            let label = ui.label(
                RichText::new(l.t("New perspective name", "Nom de la nouvelle perspective"))
                    .color(t.text_dim),
            );
            ui.add(
                egui::TextEdit::singleline(&mut d.new_name)
                    .char_limit(256)
                    .desired_width(190.0),
            )
            .labelled_by(label.id);
            ui.horizontal(|ui| {
                let name = d.new_name.trim().to_string();
                if ui
                    .add_enabled(!name.is_empty(), egui::Button::new(l.t("Create", "Créer")))
                    .clicked()
                {
                    calls.push(Call {
                        id: ADD,
                        params: json!({"name": name}),
                    });
                    d.name = Some(name.clone());
                    d.rename = name;
                    d.draft = Some(BTreeMap::new());
                    d.new_name.clear();
                }
                if let Some(n) = d.name.clone()
                    && ui.button(l.t("Delete", "Supprimer")).clicked()
                {
                    calls.push(Call {
                        id: REMOVE,
                        params: json!({"name": n}),
                    });
                }
            });
        });
        ui.separator();
        ui.vertical(|ui| {
            if d.name.is_none() {
                ui.label(
                    RichText::new(l.t("Create a perspective first.", "Créez d'abord une perspective."))
                        .color(t.text_dim),
                );
                return;
            }
            ui.horizontal(|ui| {
                let label = ui.label(l.t("Name", "Nom"));
                ui.add(
                    egui::TextEdit::singleline(&mut d.rename)
                        .char_limit(256)
                        .desired_width(260.0),
                )
                .labelled_by(label.id);
            });
            ui.label(
                RichText::new(l.t(
                    "Tick the tables, columns, measures and hierarchies the perspective shows.",
                    "Cochez les tables, colonnes, mesures et hiérarchies que montre la perspective.",
                ))
                .color(t.text_dim),
            );
            ui.add_space(4.0);
            let draft = d.draft.get_or_insert_with(BTreeMap::new);
            egui::ScrollArea::vertical()
                .id_salt("bicraft-perspectives-tables")
                .max_height(BODY_HEIGHT - 60.0)
                .show(ui, |ui| {
                    for tab in tabs {
                        table_ui(ui, draft, &mut d.open, tab, l);
                    }
                });
        });
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let c = save_call(d, tabs);
        let r = widgets::primary(ui, l.t("Save", "Enregistrer"), c.is_some(), "");
        let a11y = l.t("Save the perspective", "Enregistrer la perspective");
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, c.is_some(), a11y));
        if r.clicked()
            && let Some(c) = c
        {
            if let Some(n) = c.params.get("new_name").and_then(Value::as_str) {
                d.name = Some(n.to_string());
            }
            calls.push(c);
        }
        if ui.button(l.t("Close", "Fermer")).clicked() {
            *close = true;
        }
    });
}

fn table_ui(
    ui: &mut Ui,
    draft: &mut BTreeMap<String, Members>,
    open: &mut Vec<String>,
    tab: &ModelTable,
    l: Lang,
) {
    let key = tab.name.to_lowercase();
    let m = draft.entry(key.clone()).or_default();
    let total = tab.columns.len() + tab.measures.len() + tab.hierarchies.len();
    let have = m.columns.len() + m.measures.len() + m.hierarchies.len();
    let mut whole = total > 0 && have == total;
    ui.horizontal(|ui| {
        let label = format!("{} ({have}/{total})", tab.name);
        let r = ui.checkbox(&mut whole, RichText::new(&label).font(theme::bold(13.0)));
        let a11y = format!("{} {}", l.t("Include table", "Inclure la table"), tab.name);
        r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, whole, &a11y));
        if r.changed() {
            if whole {
                m.columns = tab.columns.iter().cloned().collect();
                m.measures = tab.measures.iter().cloned().collect();
                m.hierarchies = tab.hierarchies.iter().cloned().collect();
            } else {
                *m = Members::default();
            }
        }
        let is_open = open.contains(&key);
        let more = format!("{} {}", l.t("Members of", "Membres de"), tab.name);
        let b = ui.small_button(if is_open {
            l.t("Hide members", "Masquer les membres")
        } else {
            l.t("Members", "Membres")
        });
        b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &more));
        if b.clicked() {
            if is_open {
                open.retain(|x| *x != key);
            } else {
                open.push(key.clone());
            }
        }
    });
    if !open.contains(&key) {
        return;
    }
    ui.indent(("bicraft-persp-members", &key), |ui| {
        for (kind, have, on) in [
            (l.t("column", "colonne"), &tab.columns, &mut m.columns),
            (l.t("measure", "mesure"), &tab.measures, &mut m.measures),
            (
                l.t("hierarchy", "hiérarchie"),
                &tab.hierarchies,
                &mut m.hierarchies,
            ),
        ] {
            for n in have {
                let mut x = on.contains(n);
                let a11y = format!("{}[{n}]", tab.name);
                let r = ui.checkbox(&mut x, format!("{n} ({kind})"));
                r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, x, &a11y));
                if r.changed() {
                    if x {
                        on.insert(n.clone());
                    } else {
                        on.remove(n);
                    }
                }
            }
        }
    });
}

/// `model` with only what perspective `name` holds: its tables, and in each its columns,
/// measures and hierarchies; the relationships stay. `None` when the
/// snapshot does not list that perspective.
pub fn filter_model(model: &Value, name: &str) -> Option<Value> {
    let members = members_of(model, name)?;
    let mut m = model.clone();
    if let Some(tables) = m.get_mut("tables").and_then(Value::as_array_mut) {
        tables.retain(|t| {
            t.get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| members.get(&n.to_lowercase()).is_some_and(|x| !x.is_empty()))
        });
        for t in tables.iter_mut() {
            let n = t
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_lowercase();
            let Some(x) = members.get(&n) else { continue };
            for (k, keep) in [
                ("columns", &x.columns),
                ("measures", &x.measures),
                ("hierarchies", &x.hierarchies),
            ] {
                if let Some(list) = t.get_mut(k).and_then(Value::as_array_mut) {
                    list.retain(|e| {
                        e.get("name")
                            .and_then(Value::as_str)
                            .or_else(|| e.as_str())
                            .is_some_and(|n| keep.contains(n))
                    });
                }
            }
        }
    }
    // the relationships stay (the diagram, as for a layout tab, draws those between the tables
    // shown; their indexes are the model's)
    Some(m)
}
