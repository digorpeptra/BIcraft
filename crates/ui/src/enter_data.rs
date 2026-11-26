//! Home › Enter data and a table's Edit data (G59; Power BI Desktop's Create table): a grid of
//! typed columns, edited in place, that Load writes through `table.enter_data` (one undo step).
//! Columns and rows are added, renamed and removed with the grid's buttons; each column's type is
//! picked from Desktop's data types or left to detection; Ctrl+V of tab-separated text (copied
//! from a spreadsheet) fills the grid from the focused cell, growing it as needed.
//!
//! Edit data (the table's menu, `table.entered_data`) reads a typed-in table back into the grid
//! and Load gives the table the edited rows (`replace`), its columns' properties kept.

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::{Then, clip};
use crate::get_data::Step;
use crate::i18n::Lang;
use crate::theme::{self, Tokens};
use crate::widgets;

pub const ENTER: &str = "table.enter_data";
pub const READ: &str = "table.entered_data";
/// Most rows and columns the grid holds (a larger table is edited through the command).
pub const MAX_ROWS: usize = 3000;
pub const MAX_COLUMNS: usize = 200;

/// The column types offered: (command value, English, French). `auto`: detected on Load.
pub const TYPES: [(&str, &str, &str); 8] = [
    ("auto", "Detect", "Détecter"),
    ("text", "Text", "Texte"),
    ("whole", "Whole number", "Nombre entier"),
    ("number", "Decimal number", "Nombre décimal"),
    ("fixed", "Fixed decimal number", "Nombre décimal fixe"),
    ("date", "Date", "Date"),
    ("datetime", "Date/Time", "Date/Heure"),
    ("logical", "True/False", "Vrai/Faux"),
];

/// One column of the grid.
#[derive(Debug, Clone, PartialEq)]
pub struct GridCol {
    pub name: String,
    /// One of [`TYPES`]' values.
    pub ty: &'static str,
}

/// The open grid (pure view state).
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub name: String,
    /// Edit data of this existing table (Load replaces its rows).
    pub edit: Option<String>,
    pub columns: Vec<GridCol>,
    pub rows: Vec<Vec<String>>,
    /// The culture the texts are written in (Edit data: the model's query culture).
    pub culture: Option<String>,
    /// Edit data's read, or Load, is running.
    pub busy: bool,
    pub problem: Option<String>,
    pub generation: u64,
    /// The cell last focused (row, column): where a paste goes.
    pub focus: Option<(usize, usize)>,
}

fn ty_of(name: &str) -> &'static str {
    TYPES.iter().find(|t| t.0 == name).map_or("auto", |t| t.0)
}

/// `Table`, else `Table 2`, `Table 3`… the first name no table has.
fn free_name(app: &App) -> String {
    let base = app.lang.t("Table", "Table");
    let taken: Vec<String> = app
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/model/tables"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| t.get("name").and_then(Value::as_str))
        .map(str::to_lowercase)
        .collect();
    std::iter::once(base.to_string())
        .chain((2..1000).map(|i| format!("{base} {i}")))
        .find(|n| !taken.contains(&n.to_lowercase()))
        .unwrap_or_else(|| base.to_string())
}

fn column_name(i: usize) -> String {
    format!("Column{}", i.saturating_add(1))
}

/// Opens an empty grid (Enter data): one column, one row.
pub fn open(app: &mut App) {
    app.cmd.enter_data = Some(Grid {
        name: free_name(app),
        edit: None,
        columns: vec![GridCol {
            name: column_name(0),
            ty: "auto",
        }],
        rows: vec![vec![String::new()]],
        culture: None,
        busy: false,
        problem: None,
        generation: 0,
        focus: None,
    });
}

/// Edit data of `table`: the grid opens while `table.entered_data` reads it.
pub fn edit(app: &mut App, table: &str) {
    app.cmd.enter_data = Some(Grid {
        name: table.to_string(),
        edit: Some(table.to_string()),
        columns: Vec::new(),
        rows: Vec::new(),
        culture: None,
        busy: true,
        problem: None,
        generation: 1,
        focus: None,
    });
    app.run(
        Call {
            id: READ,
            params: json!({"name": table}),
        },
        Then::GetData(Step::Read(1)),
    );
}

/// Enter data without its values opens an empty grid; Edit data of a table opens it filled.
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    match id {
        ENTER if params.get("columns").is_none() => {
            open(app);
            true
        }
        READ => match params.get("name").and_then(Value::as_str) {
            Some(t) if !t.is_empty() => {
                let t = t.to_string();
                edit(app, &t);
                true
            }
            _ => false,
        },
        _ => false,
    }
}

/// The answer of Edit data's read or of Load.
pub fn answer(app: &mut App, step: Step, r: Result<Value, String>) {
    let Some(g) = app.cmd.enter_data.as_mut() else {
        return;
    };
    match step {
        Step::Read(n) if n == g.generation => {
            g.busy = false;
            match r {
                Ok(v) => fill(g, &v),
                Err(e) => g.problem = Some(clip(&e, 600)),
            }
        }
        Step::Enter(n) if n == g.generation => {
            g.busy = false;
            match r {
                Ok(_) => app.cmd.enter_data = None,
                Err(e) => g.problem = Some(clip(&e, 600)),
            }
        }
        _ => {}
    }
}

/// The grid of `table.entered_data`'s answer.
fn fill(g: &mut Grid, v: &Value) {
    g.columns = v
        .get("columns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_COLUMNS)
        .map(|c| GridCol {
            name: c
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            ty: ty_of(c.get("type").and_then(Value::as_str).unwrap_or("auto")),
        })
        .collect();
    let width = g.columns.len();
    g.rows = v
        .get("rows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_ROWS)
        .map(|r| {
            let mut row: Vec<String> = r
                .as_array()
                .into_iter()
                .flatten()
                .take(width)
                .map(|c| c.as_str().unwrap_or_default().to_string())
                .collect();
            row.resize(width, String::new());
            row
        })
        .collect();
    g.culture = v.get("culture").and_then(Value::as_str).map(str::to_string);
}

/// Adds a column named after its position (`Column3`, else the first `ColumnN` no column has).
pub fn add_column(g: &mut Grid) {
    if g.columns.len() >= MAX_COLUMNS {
        return;
    }
    let name = (g.columns.len()..g.columns.len() + MAX_COLUMNS + 1)
        .map(column_name)
        .find(|n| !g.columns.iter().any(|c| c.name.eq_ignore_ascii_case(n)))
        .unwrap_or_else(|| column_name(g.columns.len()));
    g.columns.push(GridCol { name, ty: "auto" });
    for r in &mut g.rows {
        r.push(String::new());
    }
}

pub fn add_row(g: &mut Grid) {
    if g.rows.len() < MAX_ROWS {
        g.rows.push(vec![String::new(); g.columns.len()]);
    }
}

pub fn remove_column(g: &mut Grid, c: usize) {
    if c < g.columns.len() {
        g.columns.remove(c);
        for r in &mut g.rows {
            if c < r.len() {
                r.remove(c);
            }
        }
    }
    g.focus = None;
}

pub fn remove_row(g: &mut Grid, r: usize) {
    if r < g.rows.len() {
        g.rows.remove(r);
    }
    g.focus = None;
}

/// The first row becomes the column names (an empty cell keeps the column's name).
pub fn promote(g: &mut Grid) {
    if g.rows.is_empty() {
        return;
    }
    let first = g.rows.remove(0);
    for (c, name) in g.columns.iter_mut().zip(first) {
        if !name.trim().is_empty() {
            c.name = name.trim().to_string();
        }
    }
}

/// Tab-separated text pasted at (`row`, `col`): the grid grows to hold it.
pub fn paste(g: &mut Grid, at: (usize, usize), text: &str) {
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let (r0, c0) = at;
    for (i, line) in lines.iter().enumerate() {
        let r = r0.saturating_add(i);
        if r >= MAX_ROWS {
            break;
        }
        while g.rows.len() <= r {
            g.rows.push(vec![String::new(); g.columns.len()]);
        }
        for (j, cell) in line.split('\t').enumerate() {
            let c = c0.saturating_add(j);
            if c >= MAX_COLUMNS {
                break;
            }
            while g.columns.len() <= c {
                add_column(g);
            }
            if let Some(slot) = g.rows.get_mut(r).and_then(|row| row.get_mut(c)) {
                *slot = cell.to_string();
            }
        }
    }
}

/// The `table.enter_data` call of the grid, or what is wrong with it.
pub fn call(g: &Grid, l: Lang) -> Result<Call, String> {
    let name = g.name.trim();
    if name.is_empty() {
        return Err(l
            .t("Type the table's name.", "Saisissez le nom de la table.")
            .into());
    }
    if g.columns.is_empty() {
        return Err(l.t("Add a column.", "Ajoutez une colonne.").into());
    }
    let columns: Vec<Value> = g
        .columns
        .iter()
        .map(|c| json!({"name": c.name.trim(), "type": c.ty}))
        .collect();
    // rows left empty are not data
    let rows: Vec<Value> = g
        .rows
        .iter()
        .filter(|r| r.iter().any(|c| !c.is_empty()))
        .map(|r| {
            Value::Array(
                r.iter()
                    .map(|c| if c.is_empty() { Value::Null } else { json!(c) })
                    .collect(),
            )
        })
        .collect();
    let mut p = json!({"name": name, "columns": columns, "rows": rows});
    if let Some(c) = &g.culture {
        p["culture"] = json!(c);
    }
    if g.edit.is_some() {
        p["replace"] = json!(true);
    }
    Ok(Call { id: ENTER, params: p })
}

enum Act {
    Load,
    AddColumn,
    AddRow,
    Promote,
    RemoveColumn(usize),
    RemoveRow(usize),
}

/// Draws the open grid.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut g) = app.cmd.enter_data.clone() else {
        return;
    };
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    // Ctrl+V (the Paste event of both shells, line ends `\n`): cells fill the grid from the
    // selected cell (the text box does not take them), and so does any paste while no text box
    // has the focus; a single value goes into the focused box without the line break a
    // spreadsheet's copy ends with
    if !g.busy {
        let typing = ctx.egui_wants_keyboard_input();
        let mut pasted = Vec::new();
        ctx.input_mut(|i| {
            i.events.retain_mut(|e| {
                let egui::Event::Paste(s) = e else {
                    return true;
                };
                let one = s.strip_suffix('\n').unwrap_or(s).to_string();
                if typing && !one.contains(['\t', '\n']) {
                    *s = one;
                    return true;
                }
                pasted.push(s.clone());
                false
            });
        });
        for s in pasted {
            let at = g.focus.unwrap_or((g.rows.len(), 0));
            // an empty grid of one blank row takes the paste at its top
            let at = if g.rows.iter().all(|r| r.iter().all(String::is_empty)) && g.focus.is_none() {
                (0, 0)
            } else {
                at
            };
            paste(&mut g, at, &s);
        }
    }
    let mut open = true;
    let mut close = false;
    let mut acts = Vec::new();
    let title = match &g.edit {
        Some(table) => format!(
            "{} › {}",
            l.t("Edit data", "Modifier les données"),
            clip(table, 60)
        ),
        None => l.t("Enter data", "Entrer des données").to_string(),
    };
    egui::Window::new(title)
        .id(egui::Id::new("bicraft-enter-data"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(760.0);
            body(ui, &mut g, l, &t, &mut acts, &mut close);
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    for a in acts {
        match a {
            Act::AddColumn => add_column(&mut g),
            Act::AddRow => add_row(&mut g),
            Act::Promote => promote(&mut g),
            Act::RemoveColumn(c) => remove_column(&mut g, c),
            Act::RemoveRow(r) => remove_row(&mut g, r),
            Act::Load => match call(&g, l) {
                Ok(c) => {
                    g.generation = g.generation.wrapping_add(1);
                    g.busy = true;
                    g.problem = None;
                    let n = g.generation;
                    app.cmd.enter_data = Some(g.clone());
                    app.run(c, Then::GetData(Step::Enter(n)));
                    return;
                }
                Err(e) => g.problem = Some(e),
            },
        }
    }
    app.cmd.enter_data = (open && !close).then_some(g);
}

/// `r` named `label` in the accessibility tree.
fn labelled(r: egui::Response, ty: egui::WidgetType, label: &str) -> egui::Response {
    let enabled = r.enabled();
    r.widget_info(|| egui::WidgetInfo::labeled(ty, enabled, label));
    r
}

fn body(ui: &mut Ui, g: &mut Grid, l: Lang, t: &Tokens, acts: &mut Vec<Act>, close: &mut bool) {
    let editing = !g.busy;
    ui.horizontal(|ui| {
        let r = ui.add_sized([90.0, 20.0], egui::Label::new(l.t("Name", "Nom")));
        ui.add_enabled(
            editing && g.edit.is_none(),
            egui::TextEdit::singleline(&mut g.name)
                .char_limit(1024)
                .desired_width(300.0),
        )
        .labelled_by(r.id);
    });
    ui.label(
        RichText::new(l.t(
            "Type or paste (Ctrl+V) cells copied from a spreadsheet. A column's type: Detect reads it from its cells.",
            "Saisissez ou collez (Ctrl+V) des cellules copiées d'un tableur. Type d'une colonne : Détecter le lit dans ses cellules.",
        ))
        .color(t.text_dim),
    );
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                editing && g.columns.len() < MAX_COLUMNS,
                egui::Button::new(l.t("Add column", "Ajouter une colonne")),
            )
            .clicked()
        {
            acts.push(Act::AddColumn);
        }
        if ui
            .add_enabled(
                editing && g.rows.len() < MAX_ROWS,
                egui::Button::new(l.t("Add row", "Ajouter une ligne")),
            )
            .clicked()
        {
            acts.push(Act::AddRow);
        }
        if ui
            .add_enabled(
                editing && !g.rows.is_empty(),
                egui::Button::new(l.t(
                    "Use first row as headers",
                    "Utiliser la première ligne pour les en-têtes",
                )),
            )
            .clicked()
        {
            acts.push(Act::Promote);
        }
    });
    ui.add_space(4.0);
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::same(4))
        .show(ui, |ui| {
            ui.set_min_height(240.0);
            if g.busy && g.columns.is_empty() {
                ui.spinner();
                return;
            }
            egui::ScrollArea::both()
                .id_salt("bicraft-enter-data-grid")
                .max_height(340.0)
                .show(ui, |ui| grid(ui, g, l, t, editing, acts));
        });
    if let Some(p) = &g.problem {
        ui.label(RichText::new(format!("⚠ {p}")).color(t.warn));
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if g.busy {
            ui.spinner();
        }
        let label = if g.edit.is_some() {
            l.t("OK", "OK")
        } else {
            l.t("Load", "Charger")
        };
        if widgets::primary(ui, label, editing, "").clicked() && editing {
            acts.push(Act::Load);
        }
        if ui.button(l.t("Cancel", "Annuler")).clicked() {
            *close = true;
        }
        let n = g.rows.iter().filter(|r| r.iter().any(|c| !c.is_empty())).count();
        ui.label(
            RichText::new(format!(
                "{n} {} · {} {}",
                l.t("rows", "lignes"),
                g.columns.len(),
                l.t("columns", "colonnes")
            ))
            .color(t.text_dim),
        );
    });
}

fn grid(ui: &mut Ui, g: &mut Grid, l: Lang, t: &Tokens, editing: bool, acts: &mut Vec<Act>) {
    egui::Grid::new("bicraft-enter-data-cells")
        .spacing(egui::vec2(4.0, 3.0))
        .show(ui, |ui| {
            // the header: name, type and remove of each column
            ui.label("");
            for (c, col) in g.columns.iter_mut().enumerate() {
                ui.vertical(|ui| {
                    let r = ui.add_enabled(
                        editing,
                        egui::TextEdit::singleline(&mut col.name)
                            .char_limit(1024)
                            .desired_width(130.0)
                            .font(theme::bold(13.0)),
                    );
                    labelled(
                        r,
                        egui::WidgetType::TextEdit,
                        &format!("{} {} {}", l.t("Column", "Colonne"), c + 1, l.t("name", "nom")),
                    );
                    ui.horizontal(|ui| {
                        let current = TYPES.iter().find(|x| x.0 == col.ty).map_or("", |x| l.t(x.1, x.2));
                        let label = format!("{} {} {}", l.t("Column", "Colonne"), c + 1, l.t("type", "type"));
                        let r = egui::ComboBox::from_id_salt(("bicraft-enter-data-type", c))
                            .selected_text(current)
                            .width(100.0)
                            .show_ui(ui, |ui| {
                                for (k, en, fr) in TYPES {
                                    ui.selectable_value(&mut col.ty, k, l.t(en, fr));
                                }
                            })
                            .response;
                        labelled(r, egui::WidgetType::ComboBox, &label);
                        let r = ui.add_enabled(editing, egui::Button::new("✕").small());
                        let label = format!("{} {}", l.t("Remove column", "Supprimer la colonne"), col.name);
                        let r = labelled(r, egui::WidgetType::Button, &label).on_hover_text(&label);
                        if r.clicked() {
                            acts.push(Act::RemoveColumn(c));
                        }
                    });
                });
            }
            ui.end_row();
            let names: Vec<String> = g.columns.iter().map(|c| c.name.clone()).collect();
            let mut focus = None;
            for (ri, row) in g.rows.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let label = format!("{} {}", l.t("Remove row", "Supprimer la ligne"), ri + 1);
                    let r = ui.add_enabled(editing, egui::Button::new("✕").small());
                    let r = labelled(r, egui::WidgetType::Button, &label).on_hover_text(&label);
                    if r.clicked() {
                        acts.push(Act::RemoveRow(ri));
                    }
                    ui.label(RichText::new(format!("{}", ri + 1)).color(t.text_dim));
                });
                for (ci, cell) in row.iter_mut().enumerate() {
                    let r = ui.add_enabled(
                        editing,
                        egui::TextEdit::singleline(cell)
                            .char_limit(32_768)
                            .desired_width(130.0),
                    );
                    let name = names.get(ci).map_or("", String::as_str);
                    let label = format!("{} {}, {name}", l.t("Row", "Ligne"), ri + 1);
                    let r = labelled(r, egui::WidgetType::TextEdit, &label);
                    if r.has_focus() {
                        focus = Some((ri, ci));
                    }
                }
                ui.end_row();
            }
            if focus.is_some() {
                g.focus = focus;
            }
        });
}
