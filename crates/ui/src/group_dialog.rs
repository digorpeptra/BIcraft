//! New group / Edit groups (Power BI Desktop's Groups dialog): reached from a column's menu in
//! the Data pane, Column tools › Data groups in the Data view and the Model view's Table tab, and
//! the palette. One dialog for both group types:
//!
//! - **List**: the column's values (`column.values`) as Ungrouped values; Group makes a group of
//!   the ones selected (or adds them to the selected group), Ungroup puts selected members or a
//!   selected group back; Include Other group sends the values no group lists to `Other`;
//! - **Bin**: a number's bin size or number of bins (with the values' minimum and maximum), a
//!   date's bin size in years, months, days, hours, minutes or seconds.
//!
//! OK runs `column.group` with the dialog's values (the engine checks them; its error goes to the
//! error bar). On a group column it edits that group (`replace`), starting from the definition
//! the snapshot reads back (`group`).

use std::collections::BTreeSet;

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::Then;
use crate::i18n::Lang;
use crate::theme::{self, Tokens};
use crate::widgets;

pub const GROUP: &str = "column.group";
/// Most values the dialog lists.
const MAX_VALUES: usize = 2_000;

/// List or bin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    List,
    Bin,
}

/// The open dialog's values.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupDialog {
    pub table: String,
    /// The column grouped.
    pub column: String,
    /// Its type (`string`, `int64`, `dateTime`…).
    pub data_type: String,
    pub name: String,
    pub kind: Kind,
    /// The column's distinct values once `column.values` answered (blank as `null`).
    pub values: Option<Vec<Value>>,
    pub truncated: bool,
    /// The groups, in order: name and members.
    pub groups: Vec<(String, Vec<Value>)>,
    /// Ungrouped values selected (by value).
    pub picked: Vec<Value>,
    /// The group selected (its index), and members of it selected.
    pub group: Option<usize>,
    pub members: Vec<Value>,
    pub other: bool,
    /// Bins: by count instead of size; the texts typed; a date's unit.
    pub by_count: bool,
    pub bin_size: String,
    pub bin_count: String,
    pub unit: &'static str,
    /// Editing group column `name` (`replace`).
    pub edit: bool,
    pub problem: Option<String>,
}

fn is_number(t: &str) -> bool {
    matches!(t, "int64" | "double" | "decimal")
}

/// A value as the dialog shows it.
pub fn shown(v: &Value, l: Lang) -> String {
    match v {
        Value::Null => l.t("(Blank)", "(Vide)").to_string(),
        Value::String(s) => s
            .get(..19)
            .filter(|_| s.ends_with("T00:00:00"))
            .map_or_else(|| s.clone(), |d| d.get(..10).unwrap_or(d).to_string()),
        other => other.to_string(),
    }
}

/// The snapshot's column `table[column]`.
fn snap_column(app: &App, table: &str, column: &str) -> Option<Value> {
    app.loaded
        .as_ref()?
        .snap
        .pointer("/model/tables")?
        .as_array()?
        .iter()
        .find(|t| t.get("name").and_then(Value::as_str) == Some(table))?
        .get("columns")?
        .as_array()?
        .iter()
        .find(|c| c.get("name").and_then(Value::as_str) == Some(column))
        .cloned()
}

/// Opens the dialog on `table[column]`: a new group of it, or the edit of the group it is.
pub fn open(app: &mut App, table: &str, column: &str) {
    let l = app.lang;
    let Some(col) = snap_column(app, table, column) else {
        app.error = Some(format!("✕ {}", l.t("No such column.", "Colonne introuvable.")));
        return;
    };
    let def = col.get("group").cloned();
    let (source, edit) = match def.as_ref().and_then(|g| g.get("column")).and_then(Value::as_str) {
        Some(s) => (s.to_string(), true),
        None => (column.to_string(), false),
    };
    let data_type = snap_column(app, table, &source)
        .and_then(|c| c.get("data_type").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    let binnable = is_number(&data_type) || data_type == "dateTime";
    let mut d = GroupDialog {
        table: table.to_string(),
        column: source.clone(),
        data_type: data_type.clone(),
        name: if edit {
            column.to_string()
        } else if binnable {
            format!("{source} {}", l.t("(bins)", "(bins)"))
        } else {
            format!("{source} {}", l.t("(groups)", "(groupes)"))
        },
        kind: if binnable { Kind::Bin } else { Kind::List },
        values: None,
        truncated: false,
        groups: Vec::new(),
        picked: Vec::new(),
        group: None,
        members: Vec::new(),
        other: true,
        by_count: false,
        bin_size: if data_type == "dateTime" {
            "1".into()
        } else {
            String::new()
        },
        bin_count: "10".into(),
        unit: if data_type == "dateTime" { "year" } else { "number" },
        edit,
        problem: None,
    };
    if let Some(g) = def {
        let s = |k: &str| g.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        if s("kind") == "list" {
            d.kind = Kind::List;
            d.other = g.get("other").and_then(Value::as_bool).unwrap_or(true);
            d.groups = g
                .get("groups")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|x| {
                    (
                        x.get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        x.get("values")
                            .and_then(Value::as_array)
                            .cloned()
                            .unwrap_or_default(),
                    )
                })
                .collect();
        } else {
            d.kind = Kind::Bin;
            if let Some(n) = g.get("bin_count") {
                d.by_count = true;
                d.bin_count = n.to_string();
            }
            if let Some(n) = g.get("bin_size") {
                d.bin_size = n.to_string();
            }
            d.unit = match s("unit").as_str() {
                "month" => "month",
                "day" => "day",
                "year" => "year",
                "hour" => "hour",
                "minute" => "minute",
                "second" => "second",
                _ => "number",
            };
        }
    }
    app.cmd.group = Some(d);
    // the values: the list, and a number's minimum and maximum
    app.run(
        Call {
            id: "column.values",
            params: json!({"table": table, "column": source, "limit": MAX_VALUES}),
        },
        Then::GroupValues,
    );
}

/// `column.group` without its values (a ribbon, menu or palette click) opens the dialog on the
/// column it names.
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    if id != GROUP
        || ["groups", "bin_size", "bin_count", "kind"]
            .iter()
            .any(|k| params.get(*k).is_some())
    {
        return false;
    }
    let s = |k: &str| params.get(k).and_then(Value::as_str).map(str::to_string);
    match (s("table"), s("column")) {
        (Some(t), Some(c)) => open(app, &t, &c),
        _ => {
            app.error = Some(format!(
                "✕ {}",
                app.lang.t(
                    "Select a column first (New group groups the values of a column).",
                    "Sélectionnez d'abord une colonne (Nouveau groupe regroupe les valeurs d'une colonne).",
                )
            ));
        }
    }
    true
}

/// Keeps the answer of the dialog's `column.values` ([`Then::GroupValues`]).
pub fn values_answer(app: &mut App, r: Result<Value, String>) {
    let Some(d) = app.cmd.group.as_mut() else {
        return;
    };
    match r {
        Ok(v) => {
            d.values = Some(
                v.get("values")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            );
            d.truncated = v.get("truncated").and_then(Value::as_bool).unwrap_or(false);
        }
        Err(e) => {
            d.values = Some(Vec::new());
            d.problem = Some(crate::app::clip(&e, 300));
        }
    }
}

/// A number typed in the dialog (`0,5` in French too).
fn number(s: &str) -> Option<f64> {
    s.trim()
        .replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|x| x.is_finite())
}

/// The command the dialog's values give, or what is wrong with them.
pub fn call(d: &GroupDialog, l: Lang) -> Result<Call, String> {
    let name = d.name.trim();
    if name.is_empty() {
        return Err(l.t("Type a name.", "Saisissez un nom.").into());
    }
    let mut p = json!({"table": d.table, "column": d.column, "name": name});
    if d.edit {
        p["replace"] = json!(true);
    }
    match d.kind {
        Kind::List => {
            if d.groups.is_empty() {
                return Err(l
                    .t(
                        "Make at least one group: select values, then Group.",
                        "Créez au moins un groupe : sélectionnez des valeurs, puis Grouper.",
                    )
                    .into());
            }
            p["kind"] = json!("list");
            p["other"] = json!(d.other);
            p["groups"] = Value::Array(
                d.groups
                    .iter()
                    .map(|(n, v)| json!({"name": n, "values": v}))
                    .collect(),
            );
        }
        Kind::Bin => {
            p["kind"] = json!("bin");
            p["unit"] = json!(d.unit);
            if d.by_count && d.unit == "number" {
                let n = number(&d.bin_count)
                    .filter(|n| *n >= 1.0 && n.fract() == 0.0)
                    .ok_or_else(|| {
                        l.t(
                            "The number of bins must be a whole number of 1 or more.",
                            "Le nombre de bins doit être un nombre entier d'au moins 1.",
                        )
                        .to_string()
                    })?;
                p["bin_count"] = json!(n);
            } else {
                let n = number(&d.bin_size).filter(|n| *n > 0.0).ok_or_else(|| {
                    l.t(
                        "The bin size must be a number greater than 0.",
                        "La taille des bins doit être un nombre supérieur à 0.",
                    )
                    .to_string()
                })?;
                p["bin_size"] = json!(n);
            }
        }
    }
    Ok(Call { id: GROUP, params: p })
}

/// Draws the open dialog.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut d) = app.cmd.group.clone() else {
        return;
    };
    let l = app.lang;
    let title = if d.edit {
        l.t("Edit groups", "Modifier les groupes")
    } else {
        l.t("Groups", "Groupes")
    };
    let mut open = true;
    let mut close = false;
    egui::Window::new(title)
        .id(egui::Id::unique("bicraft-groups"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(600.0);
            body(app, ui, &mut d, l, &mut close);
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    // the answer to column.values may have come while drawing from the clone
    if let Some(cur) = &app.cmd.group
        && d.values.is_none()
    {
        d.values.clone_from(&cur.values);
        d.truncated = cur.truncated;
    }
    app.cmd.group = (open && !close).then_some(d);
}

fn labelled(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui, egui::Id)) {
    ui.horizontal(|ui| {
        let r = ui.add_sized([120.0, 20.0], egui::Label::new(label));
        add(ui, r.id);
    });
}

fn body(app: &mut App, ui: &mut Ui, d: &mut GroupDialog, l: Lang, close: &mut bool) {
    let t = Tokens::of(app.prefs.theme);
    labelled(ui, l.t("Name", "Nom"), |ui, id| {
        ui.add(
            egui::TextEdit::singleline(&mut d.name)
                .char_limit(512)
                .desired_width(300.0),
        )
        .labelled_by(id);
    });
    labelled(ui, l.t("Field", "Champ"), |ui, _| {
        ui.label(RichText::new(format!("{}[{}]", d.table, d.column)).font(theme::bold(13.0)));
    });
    let binnable = is_number(&d.data_type) || d.data_type == "dateTime";
    labelled(ui, l.t("Group type", "Type de groupe"), |ui, id| {
        let text = |k: Kind| match k {
            Kind::List => l.t("List", "Liste"),
            Kind::Bin => l.t("Bin", "Bin"),
        };
        egui::ComboBox::from_id_salt("bicraft-group-type")
            .selected_text(text(d.kind))
            .width(160.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut d.kind, Kind::List, text(Kind::List));
                if binnable {
                    ui.selectable_value(&mut d.kind, Kind::Bin, text(Kind::Bin));
                }
            })
            .response
            .labelled_by(id);
    });
    ui.add_space(6.0);
    match d.kind {
        Kind::List => list_ui(ui, d, l, &t),
        Kind::Bin => bin_ui(ui, d, l, &t),
    }
    if let Some(p) = &d.problem {
        ui.label(RichText::new(format!("⚠ {p}")).color(t.warn));
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if widgets::primary(ui, l.t("OK", "OK"), true, "").clicked() {
            match call(d, l) {
                Ok(c) => {
                    app.run(c, Then::Nothing);
                    *close = true;
                }
                Err(e) => d.problem = Some(e),
            }
        }
        if ui.button(l.t("Cancel", "Annuler")).clicked() {
            *close = true;
        }
    });
}

/// A framed, scrolled list of `height`.
fn framed(ui: &mut Ui, t: &Tokens, salt: &str, width: f32, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::same(4))
        .show(ui, |ui| {
            ui.set_width(width);
            egui::ScrollArea::vertical()
                .id_salt(salt)
                .max_height(240.0)
                .min_scrolled_height(240.0)
                .auto_shrink([false, false])
                .show(ui, add);
        });
}

fn toggle(list: &mut Vec<Value>, v: &Value) {
    if let Some(i) = list.iter().position(|x| x == v) {
        list.remove(i);
    } else {
        list.push(v.clone());
    }
}

fn list_ui(ui: &mut Ui, d: &mut GroupDialog, l: Lang, t: &Tokens) {
    let grouped: BTreeSet<String> = d
        .groups
        .iter()
        .flat_map(|(_, v)| v.iter().map(Value::to_string))
        .collect();
    let ungrouped: Vec<Value> = d
        .values
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter(|v| !grouped.contains(&v.to_string()))
        .cloned()
        .collect();
    let mut make_group = false;
    let mut ungroup = false;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(l.t("Ungrouped values", "Valeurs non groupées")).font(theme::bold(13.0)));
            framed(ui, t, "bicraft-group-ungrouped", 250.0, |ui| match &d.values {
                None => {
                    ui.label(
                        RichText::new(l.t("Reading the values…", "Lecture des valeurs…")).color(t.text_dim),
                    );
                }
                Some(_) if ungrouped.is_empty() => {
                    ui.label(
                        RichText::new(l.t("(every value is grouped)", "(toutes les valeurs sont groupées)"))
                            .color(t.text_dim),
                    );
                }
                Some(_) => {
                    for v in &ungrouped {
                        let on = d.picked.contains(v);
                        if ui.selectable_label(on, shown(v, l)).clicked() {
                            toggle(&mut d.picked, v);
                        }
                    }
                }
            });
            if d.truncated {
                ui.label(
                    RichText::new(l.t(
                        "Only the first values are listed.",
                        "Seules les premières valeurs sont listées.",
                    ))
                    .color(t.text_dim),
                );
            }
        });
        ui.vertical(|ui| {
            ui.add_space(110.0);
            let can_group = !d.picked.is_empty();
            make_group = ui
                .add_enabled(
                    can_group,
                    egui::Button::new(format!("{} ›", l.t("Group", "Grouper"))),
                )
                .on_hover_text(l.t(
                    "A new group of the selected values (or add them to the selected group)",
                    "Un nouveau groupe des valeurs sélectionnées (ou les ajouter au groupe sélectionné)",
                ))
                .clicked();
            let can_ungroup = d.group.is_some() || !d.members.is_empty();
            ungroup = ui
                .add_enabled(
                    can_ungroup,
                    egui::Button::new(format!("‹ {}", l.t("Ungroup", "Dissocier"))),
                )
                .clicked();
        });
        ui.vertical(|ui| {
            ui.label(RichText::new(l.t("Groups and members", "Groupes et membres")).font(theme::bold(13.0)));
            framed(ui, t, "bicraft-group-groups", 230.0, |ui| {
                if d.groups.is_empty() {
                    ui.label(RichText::new(l.t("(no group yet)", "(aucun groupe)")).color(t.text_dim));
                }
                for (gi, (name, members)) in d.groups.iter().enumerate() {
                    let on = d.group == Some(gi) && d.members.is_empty();
                    if ui
                        .selectable_label(on, RichText::new(name).font(theme::bold(13.0)))
                        .clicked()
                    {
                        d.group = if on { None } else { Some(gi) };
                        d.members.clear();
                    }
                    for v in members {
                        ui.horizontal(|ui| {
                            ui.add_space(14.0);
                            let on = d.group == Some(gi) && d.members.contains(v);
                            if ui.selectable_label(on, shown(v, l)).clicked() {
                                if d.group != Some(gi) {
                                    d.group = Some(gi);
                                    d.members.clear();
                                }
                                toggle(&mut d.members, v);
                            }
                        });
                    }
                }
            });
        });
    });
    // the selected group's name
    if let Some(gi) = d.group
        && let Some((name, _)) = d.groups.get_mut(gi)
    {
        labelled(ui, l.t("Group name", "Nom du groupe"), |ui, id| {
            ui.add(
                egui::TextEdit::singleline(name)
                    .char_limit(512)
                    .desired_width(300.0),
            )
            .labelled_by(id);
        });
    }
    ui.checkbox(
        &mut d.other,
        l.t("Include Other group", "Inclure le groupe Autre"),
    )
    .on_hover_text(l.t(
        "The values no group lists go to Other; else each keeps its own value",
        "Les valeurs d'aucun groupe vont dans Autre ; sinon chacune garde sa valeur",
    ));
    if make_group {
        let picked = std::mem::take(&mut d.picked);
        match d.group.and_then(|gi| d.groups.get_mut(gi)) {
            Some((_, members)) => members.extend(picked),
            None => {
                let name = picked.iter().map(|v| shown(v, l)).collect::<Vec<_>>().join(" & ");
                let name = crate::app::clip(&name, 120);
                d.groups.push((name, picked));
                d.group = Some(d.groups.len() - 1);
            }
        }
        d.members.clear();
    }
    if ungroup && let Some(gi) = d.group {
        if d.members.is_empty() {
            if gi < d.groups.len() {
                d.groups.remove(gi);
            }
        } else if let Some((_, members)) = d.groups.get_mut(gi) {
            members.retain(|v| !d.members.contains(v));
            if members.is_empty() && gi < d.groups.len() {
                d.groups.remove(gi);
            }
        }
        d.group = None;
        d.members.clear();
    }
}

/// The smallest and largest number of the values read.
fn range(values: Option<&[Value]>) -> Option<(f64, f64)> {
    let nums: Vec<f64> = values?.iter().filter_map(Value::as_f64).collect();
    let lo = nums.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = nums.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (lo <= hi).then_some((lo, hi))
}

const UNITS: [(&str, &str, &str); 6] = [
    ("year", "Years", "Années"),
    ("month", "Months", "Mois"),
    ("day", "Days", "Jours"),
    ("hour", "Hours", "Heures"),
    ("minute", "Minutes", "Minutes"),
    ("second", "Seconds", "Secondes"),
];

fn bin_ui(ui: &mut Ui, d: &mut GroupDialog, l: Lang, t: &Tokens) {
    if d.unit == "number" {
        labelled(ui, l.t("Bin type", "Type de bin"), |ui, _| {
            ui.radio_value(&mut d.by_count, false, l.t("Size of bins", "Taille des bins"));
            ui.radio_value(&mut d.by_count, true, l.t("Number of bins", "Nombre de bins"));
        });
        let (lo, hi) = match range(d.values.as_deref()) {
            Some((lo, hi)) => (lo.to_string(), hi.to_string()),
            None => ("…".to_string(), "…".to_string()),
        };
        labelled(ui, l.t("Minimum value", "Valeur minimale"), |ui, _| {
            ui.label(RichText::new(lo).color(t.text_dim));
        });
        labelled(ui, l.t("Maximum value", "Valeur maximale"), |ui, _| {
            ui.label(RichText::new(hi).color(t.text_dim));
        });
        if d.by_count {
            labelled(ui, l.t("Bin count", "Nombre de bins"), |ui, id| {
                ui.add(egui::TextEdit::singleline(&mut d.bin_count).desired_width(120.0))
                    .labelled_by(id);
            });
        } else {
            labelled(ui, l.t("Bin size", "Taille des bins"), |ui, id| {
                ui.add(egui::TextEdit::singleline(&mut d.bin_size).desired_width(120.0))
                    .labelled_by(id);
            });
        }
    } else {
        labelled(ui, l.t("Bin size", "Taille des bins"), |ui, id| {
            ui.add(egui::TextEdit::singleline(&mut d.bin_size).desired_width(80.0))
                .labelled_by(id);
            let current = UNITS.iter().find(|u| u.0 == d.unit).map_or("", |u| l.t(u.1, u.2));
            egui::ComboBox::from_id_salt("bicraft-group-unit")
                .selected_text(current)
                .width(120.0)
                .show_ui(ui, |ui| {
                    for (k, en, fr) in UNITS {
                        ui.selectable_value(&mut d.unit, k, l.t(en, fr));
                    }
                })
                .response
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, l.t("Unit", "Unité")));
        });
    }
    ui.label(
        RichText::new(l.t(
            "Each value goes to the bin that starts at or below it; blanks stay blank.",
            "Chaque valeur va dans le bin qui commence à elle ou en dessous ; les vides restent vides.",
        ))
        .color(t.text_dim),
    );
}
