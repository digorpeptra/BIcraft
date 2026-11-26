//! Model › Parameters › New parameter (Power BI Desktop's Modeling › New parameter): one ribbon
//! button whose menu holds the registry's two commands, Numeric range (`parameter.what_if`) and
//! Fields (`parameter.fields`), each opening its dialog. Create runs the command with the
//! dialog's values; the engine checks them (its error goes to the error bar). The palette's entry
//! of either command opens the same dialog.

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::Then;
use crate::i18n::Lang;
use crate::registry::{self, Cmd};
use crate::theme::{self, Tokens};
use crate::widgets;

/// The registry group the menu button stands for.
pub const GROUP: &str = "Parameters";
pub const WHAT_IF: &str = "parameter.what_if";
pub const FIELDS: &str = "parameter.fields";
/// Most fields the dialog lists.
const MAX_LISTED: usize = 2000;

/// Which dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    WhatIf,
    Fields,
}

/// The open dialog's values.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamDialog {
    pub kind: Kind,
    pub name: String,
    /// `whole`, `decimal` or `fixed`.
    pub data_type: &'static str,
    pub min: String,
    pub max: String,
    pub increment: String,
    pub default: String,
    /// The fields chosen, in order (`'T'[C]`, `[M]`).
    pub chosen: Vec<String>,
    pub search: String,
    pub add_slicer: bool,
    /// Why Create is refused (a value that is not a number).
    pub problem: Option<String>,
}

/// The data types of a numeric range, with their labels.
const TYPES: [(&str, &str, &str); 3] = [
    ("whole", "Whole number", "Nombre entier"),
    ("decimal", "Decimal number", "Nombre décimal"),
    ("fixed", "Fixed decimal number", "Nombre décimal fixe"),
];

/// The snapshot's tables.
fn tables(app: &App) -> Vec<Value> {
    app.loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/model/tables"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn name_of(v: &Value) -> String {
    v.get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// A table reference as a field writes it: `Sales`, `'Order Lines'`.
fn table_ref(t: &str) -> String {
    let mut chars = t.chars();
    let simple = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if simple {
        t.to_string()
    } else {
        format!("'{}'", t.replace('\'', "''"))
    }
}

fn bracket(n: &str) -> String {
    format!("[{}]", n.replace(']', "]]"))
}

/// Every column and measure the model shows, as (table, field text, label, is a measure).
fn all_fields(app: &App) -> Vec<(String, String, String, bool)> {
    let mut out = Vec::new();
    for t in tables(app) {
        let tn = name_of(&t);
        let arr = |k: &str| t.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
        for c in arr("columns") {
            let c = name_of(&c);
            if !c.is_empty() {
                out.push((tn.clone(), format!("{}{}", table_ref(&tn), bracket(&c)), c, false));
            }
        }
        for m in arr("measures") {
            let m = name_of(&m);
            if !m.is_empty() {
                out.push((tn.clone(), bracket(&m), m, true));
            }
        }
        if out.len() > MAX_LISTED {
            break;
        }
    }
    out.truncate(MAX_LISTED);
    out
}

/// `Parameter`, else `Parameter 2`, `Parameter 3`… the first name no table has.
fn free_name(app: &App, l: Lang) -> String {
    let base = l.t("Parameter", "Paramètre");
    let taken: Vec<String> = tables(app).iter().map(|t| name_of(t).to_lowercase()).collect();
    let free = |n: &str| !taken.contains(&n.to_lowercase());
    if free(base) {
        return base.to_string();
    }
    (2..1000)
        .map(|i| format!("{base} {i}"))
        .find(|n| free(n))
        .unwrap_or_else(|| base.to_string())
}

/// Opens the dialog of `kind`.
pub fn open(app: &mut App, kind: Kind) {
    let has_report = app
        .loaded
        .as_ref()
        .is_some_and(|l| l.snap.get("report").is_some_and(Value::is_object));
    app.cmd.parameter = Some(ParamDialog {
        kind,
        name: free_name(app, app.lang),
        data_type: "whole",
        min: "0".into(),
        max: "20".into(),
        increment: "1".into(),
        default: String::new(),
        chosen: Vec::new(),
        search: String::new(),
        add_slicer: has_report,
        problem: None,
    });
}

/// The two commands open their dialog when run without their values (a ribbon or palette click).
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    let kind = match id {
        WHAT_IF if params.get("min").is_none() => Kind::WhatIf,
        FIELDS if params.get("fields").is_none() => Kind::Fields,
        _ => return false,
    };
    open(app, kind);
    true
}

/// The ribbon's New parameter button: a menu of the group's commands (each greyed with its
/// reason when it cannot run).
pub fn ribbon_button(app: &mut App, ui: &mut Ui, cmds: &[&'static Cmd]) {
    let l = app.lang;
    let any = cmds.iter().any(|c| crate::ribbon::state(app, c.id).is_ok());
    let r = widgets::big_button(
        ui,
        crate::icons::Icon::Calculator,
        l.t("New parameter", "Nouveau paramètre"),
        any,
    )
    .on_hover_text(l.t(
        "A numeric range (what-if) or a fields parameter",
        "Une plage numérique (scénario) ou un paramètre de champs",
    ));
    // each command's key tip on the one button runs that command, as its menu item does
    let mut picked: Option<&'static Cmd> = None;
    for c in cmds {
        if crate::key_tips::button(app, ui, c.id, &r) {
            picked = Some(c);
        }
    }
    if !any {
        if let Some(c) = picked {
            let p = crate::ribbon::prefill(app, c, None);
            crate::ribbon::invoke(app, c.id, p);
        }
        return;
    }
    egui::Popup::menu(&r).show(|ui| {
        for c in cmds {
            let st = crate::ribbon::state(app, c.id);
            let b = ui
                .add_enabled(st.is_ok(), egui::Button::new(c.text(l)))
                .on_hover_text(c.tooltip(l, &st, app.caps().reserved_keys));
            if b.clicked() {
                picked = Some(c);
                ui.close();
            }
        }
    });
    if let Some(c) = picked {
        let p = crate::ribbon::prefill(app, c, None);
        crate::ribbon::invoke(app, c.id, p);
    }
}

/// A number typed in the dialog (`0,5` in French too).
fn number(s: &str) -> Option<f64> {
    let t = s.trim().replace(',', ".");
    t.parse::<f64>().ok().filter(|x| x.is_finite())
}

/// The command and its parameters the dialog's values give, or what is wrong with them.
pub fn call(d: &ParamDialog, page: Option<&str>, l: Lang) -> Result<Call, String> {
    let name = d.name.trim();
    if name.is_empty() {
        return Err(l.t("Type a name.", "Saisissez un nom.").into());
    }
    let mut p = serde_json::Map::new();
    p.insert("name".into(), json!(name));
    p.insert("add_slicer".into(), json!(d.add_slicer));
    if d.add_slicer
        && let Some(page) = page
    {
        p.insert("page".into(), json!(page));
    }
    let id = match d.kind {
        Kind::WhatIf => {
            for (key, text, en, fr) in [
                ("min", &d.min, "Minimum", "Minimum"),
                ("max", &d.max, "Maximum", "Maximum"),
                ("increment", &d.increment, "Increment", "Incrément"),
            ] {
                let x = number(text).ok_or_else(|| {
                    format!(
                        "{} {}",
                        l.t(en, fr),
                        l.t("must be a number.", "doit être un nombre.")
                    )
                })?;
                p.insert(key.into(), json!(x));
            }
            if !d.default.trim().is_empty() {
                let x = number(&d.default).ok_or_else(|| {
                    l.t(
                        "The default must be a number (or left empty).",
                        "La valeur par défaut doit être un nombre (ou rester vide).",
                    )
                    .to_string()
                })?;
                p.insert("default".into(), json!(x));
            }
            p.insert("data_type".into(), json!(d.data_type));
            WHAT_IF
        }
        Kind::Fields => {
            if d.chosen.is_empty() {
                return Err(l
                    .t("Add at least one field.", "Ajoutez au moins un champ.")
                    .into());
            }
            p.insert("fields".into(), json!(d.chosen));
            FIELDS
        }
    };
    Ok(Call {
        id,
        params: Value::Object(p),
    })
}

/// Draws the open dialog.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut d) = app.cmd.parameter.clone() else {
        return;
    };
    let l = app.lang;
    let id = match d.kind {
        Kind::WhatIf => WHAT_IF,
        Kind::Fields => FIELDS,
    };
    let title = registry::find(id).map_or(id, |c| c.text(l));
    let mut open = true;
    let mut close = false;
    egui::Window::new(format!("{} › {title}", l.t("New parameter", "Nouveau paramètre")))
        .id(egui::Id::unique("bicraft-new-parameter"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(460.0);
            body(app, ui, &mut d, l, &mut close);
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    app.cmd.parameter = (open && !close).then_some(d);
}

fn labelled(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui, egui::Id)) {
    ui.horizontal(|ui| {
        let r = ui.add_sized([130.0, 20.0], egui::Label::new(label));
        add(ui, r.id);
    });
}

fn text_field(ui: &mut Ui, label: &str, value: &mut String, width: f32) {
    labelled(ui, label, |ui, label_id| {
        ui.add(
            egui::TextEdit::singleline(value)
                .char_limit(1024)
                .desired_width(width),
        )
        .labelled_by(label_id);
    });
}

fn body(app: &mut App, ui: &mut Ui, d: &mut ParamDialog, l: Lang, close: &mut bool) {
    let t = Tokens::of(app.prefs.theme);
    let heading = match d.kind {
        Kind::WhatIf => l.t(
            "A table of numbers, a measure that reads the one selected, and a slicer to pick it.",
            "Une table de nombres, une mesure qui lit celui sélectionné et un segment pour le choisir.",
        ),
        Kind::Fields => l.t(
            "A slicer that picks which columns and measures the visuals bound to the parameter show.",
            "Un segment qui choisit les colonnes et mesures que montrent les visuels liés au paramètre.",
        ),
    };
    ui.label(RichText::new(heading).color(t.text_dim));
    ui.add_space(6.0);
    text_field(ui, l.t("Name", "Nom"), &mut d.name, 260.0);
    match d.kind {
        Kind::WhatIf => what_if_fields(ui, d, l),
        Kind::Fields => fields_lists(app, ui, d, l, &t),
    }
    ui.add_space(6.0);
    let report = app
        .loaded
        .as_ref()
        .is_some_and(|x| x.snap.get("report").is_some_and(Value::is_object));
    ui.add_enabled(
        report,
        egui::Checkbox::new(
            &mut d.add_slicer,
            l.t("Add a slicer to this page", "Ajouter un segment à cette page"),
        ),
    );
    if let Some(p) = &d.problem {
        ui.label(RichText::new(format!("⚠ {p}")).color(t.warn));
    }
    ui.add_space(8.0);
    let page = app.page_names().get(app.active_page()).map(|p| p.0.clone());
    ui.horizontal(|ui| {
        if widgets::primary(ui, l.t("Create", "Créer"), true, "").clicked() {
            match call(d, page.as_deref(), l) {
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

fn what_if_fields(ui: &mut Ui, d: &mut ParamDialog, l: Lang) {
    labelled(ui, l.t("Data type", "Type de données"), |ui, label_id| {
        let current = TYPES
            .iter()
            .find(|x| x.0 == d.data_type)
            .map_or("", |x| l.t(x.1, x.2));
        egui::ComboBox::from_id_salt("bicraft-param-type")
            .selected_text(current)
            .width(200.0)
            .show_ui(ui, |ui| {
                for (k, en, fr) in TYPES {
                    ui.selectable_value(&mut d.data_type, k, l.t(en, fr));
                }
            })
            .response
            .labelled_by(label_id);
    });
    text_field(ui, l.t("Minimum", "Minimum"), &mut d.min, 120.0);
    text_field(ui, l.t("Maximum", "Maximum"), &mut d.max, 120.0);
    text_field(ui, l.t("Increment", "Incrément"), &mut d.increment, 120.0);
    text_field(ui, l.t("Default", "Par défaut"), &mut d.default, 120.0);
}

fn fields_lists(app: &App, ui: &mut Ui, d: &mut ParamDialog, l: Lang, t: &Tokens) {
    ui.add_space(4.0);
    ui.label(RichText::new(l.t("Fields, in order", "Champs, dans l'ordre")).font(theme::bold(13.0)));
    let mut remove: Option<usize> = None;
    let mut up: Option<usize> = None;
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.set_min_width(430.0);
            if d.chosen.is_empty() {
                ui.label(
                    RichText::new(l.t(
                        "Tick fields below to add them.",
                        "Cochez des champs ci-dessous pour les ajouter.",
                    ))
                    .color(t.text_dim),
                );
            }
            for (i, f) in d.chosen.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("{}. {f}", i + 1));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button(l.t("Remove", "Retirer")).clicked() {
                            remove = Some(i);
                        }
                        if ui
                            .add_enabled(i > 0, egui::Button::new(l.t("Move up", "Monter")).small())
                            .clicked()
                        {
                            up = Some(i);
                        }
                    });
                });
            }
        });
    if let Some(i) = remove
        && i < d.chosen.len()
    {
        d.chosen.remove(i);
    }
    if let Some(i) = up
        && i > 0
        && i < d.chosen.len()
    {
        d.chosen.swap(i, i - 1);
    }
    ui.add_space(4.0);
    text_field(ui, l.t("Search", "Rechercher"), &mut d.search, 260.0);
    let needle = d.search.trim().to_lowercase();
    egui::ScrollArea::vertical()
        .id_salt("bicraft-param-fields")
        .max_height(220.0)
        .show(ui, |ui| {
            let mut last_table = String::new();
            for (table, field, label, measure) in all_fields(app) {
                if !needle.is_empty()
                    && !label.to_lowercase().contains(&needle)
                    && !table.to_lowercase().contains(&needle)
                {
                    continue;
                }
                if table != last_table {
                    ui.label(RichText::new(&table).font(theme::bold(12.0)));
                    last_table = table.clone();
                }
                let mut on = d.chosen.contains(&field);
                let kind = if measure {
                    l.t("measure", "mesure")
                } else {
                    l.t("column", "colonne")
                };
                let text = format!("{label} ({kind})");
                if ui.checkbox(&mut on, text).changed() {
                    if on {
                        d.chosen.push(field.clone());
                    } else {
                        d.chosen.retain(|x| *x != field);
                    }
                }
            }
        });
}
