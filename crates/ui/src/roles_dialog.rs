//! Manage security roles (the Model explorer's Roles menu, or a role's), laid out as Desktop's
//! (Microsoft Learn, "Row-level security (RLS) with Power BI", Define roles): the model's roles
//! on the left (New, Rename, Delete), the tables of the role chosen in the middle (a filter or a
//! hidden table marked with an icon and a word), and the chosen table's filter on the right.
//!
//! The filter has two editors: the default one, rows of column / condition / value joined by And
//! or Or ([`crate::role_filter`] writes the DAX), and the DAX editor ([`crate::dax_box`], the
//! formula bar's colours and completion). A filter that the default editor cannot show opens in
//! the DAX editor; switching back then warns that it is lost. Check runs `role.check_filter`
//! (nothing saved) and lists the errors and warnings with an icon and words; Revert goes back to
//! the saved filter; Save runs `role.set_filter`. The table's object-level security stays here
//! (`role.set_permission`: Hide table, and Hide per column). The dialog reads the snapshot's
//! `model.security` (docs/view-snapshot.md), so it shows what the engine holds.

use std::sync::Arc;

use egui::{Id, Rect, RichText, Ui, vec2};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::Then;
use crate::dax_box::{self, DaxBox};
use crate::dax_text::{Func, Names};
use crate::formula::Diag;
use crate::i18n::Lang;
use crate::icons::{self, Icon};
use crate::role_filter::{self, Cond, Join, Op, Ty};
use crate::theme::{self, Tokens};
use crate::widgets;

pub const COMMAND: &str = "role.set_permission";
/// Most tables and columns the dialog lists.
const MAX_LISTED: usize = 2000;
/// Most rows of the default editor.
const MAX_CONDITIONS: usize = 64;
/// Most diagnostics listed.
const MAX_DIAGS: usize = 20;
/// The height of the three columns.
const BODY_HEIGHT: f32 = 460.0;
const ROLES_W: f32 = 190.0;
const TABLES_W: f32 = 170.0;
const EDITOR_W: f32 = 560.0;

/// The filter's editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Rows of column / condition / value.
    #[default]
    Default,
    Dax,
}

/// What an answer of the dialog's commands is for ([`Then::Roles`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// `role.check_filter` of edit `.0`.
    Check(u64),
    /// `role.set_filter` of edit `.0`.
    Save(u64),
}

/// The last check of the filter.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Verdict {
    /// The DAX that was checked (the verdict holds while the filter is that text).
    pub text: String,
    pub valid: bool,
    /// No filter: every row.
    pub empty: bool,
    pub diags: Vec<Diag>,
    /// The command's refusal.
    pub error: Option<String>,
}

/// The chosen table's filter being edited.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Editor {
    pub role: String,
    pub table: String,
    /// The filter as saved in the model.
    pub saved: String,
    pub mode: Mode,
    pub rows: Vec<Cond>,
    pub dax: DaxBox,
    /// The warning before a DAX filter the default editor cannot show is dropped.
    pub confirm_switch: bool,
    /// The last check, and the check or the save in flight (`edit`).
    pub verdict: Option<Verdict>,
    pub checking: bool,
    pub saving: bool,
    pub save_error: Option<String>,
    /// Bumped by each check and save: a late answer is dropped.
    pub edit: u64,
}

/// The dialog while it is open.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Dialog {
    /// The role shown.
    pub role: Option<String>,
    /// The new role's name being typed.
    pub new_name: String,
    /// The chosen role's new name being typed (Rename).
    pub renaming: Option<String>,
    /// The table shown.
    pub table: Option<String>,
    pub editor: Option<Editor>,
    /// The table's columns are listed (object-level security).
    pub columns_open: bool,
}

/// The table and column permissions of a role from the snapshot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableRow {
    pub table: String,
    pub filter: String,
    pub hidden: bool,
    /// (column, hidden).
    pub columns: Vec<(String, bool)>,
    /// (column, snapshot `data_type`).
    pub types: Vec<(String, String)>,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn snap_model(app: &App) -> Option<&Value> {
    app.loaded.as_ref().and_then(|l| l.snap.get("model"))
}

/// The roles' names.
pub fn roles(app: &App) -> Vec<String> {
    snap_model(app)
        .and_then(|m| m.get("roles"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

/// Every table of the model with `role`'s filter and object-level security on it and its columns.
pub fn rows(app: &App, role: &str) -> Vec<TableRow> {
    let Some(m) = snap_model(app) else {
        return Vec::new();
    };
    let sec = m
        .get("security")
        .and_then(Value::as_array)
        .and_then(|a| {
            a.iter().find(|r| {
                r.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| n.eq_ignore_ascii_case(role))
            })
        })
        .cloned()
        .unwrap_or(Value::Null);
    let perm = |table: &str| -> Option<Value> {
        sec.get("tables")
            .and_then(Value::as_array)?
            .iter()
            .find(|t| {
                t.get("table")
                    .and_then(Value::as_str)
                    .is_some_and(|n| n.eq_ignore_ascii_case(table))
            })
            .cloned()
    };
    let none = |v: &Value| v.get("metadata_permission").and_then(Value::as_str) == Some("none");
    m.get("tables")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_LISTED)
        .map(|t| {
            let table = s(t, "name");
            let p = perm(&table).unwrap_or(Value::Null);
            let hidden_cols: Vec<String> = p
                .get("columns")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|c| none(c))
                .map(|c| s(c, "name").to_lowercase())
                .collect();
            let cols: Vec<&Value> = t
                .get("columns")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(MAX_LISTED)
                .collect();
            let columns = cols
                .iter()
                .map(|c| {
                    let n = s(c, "name");
                    let h = hidden_cols.contains(&n.to_lowercase());
                    (n, h)
                })
                .collect();
            let types = cols.iter().map(|c| (s(c, "name"), s(c, "data_type"))).collect();
            TableRow {
                filter: s(&p, "filter_expression"),
                hidden: none(&p),
                table,
                columns,
                types,
            }
        })
        .collect()
}

/// `role.set_permission` for a table (`column` None) or a column, hidden or not.
pub fn permission(role: &str, table: &str, column: Option<&str>, hide: bool) -> Call {
    let mut p = json!({
        "role": role,
        "table": table,
        "metadata_permission": if hide { Value::from("none") } else { Value::Null },
    });
    if let (Some(c), Value::Object(o)) = (column, &mut p) {
        o.insert("column".into(), json!(c));
    }
    Call {
        id: COMMAND,
        params: p,
    }
}

/// Manage roles (the explorer's menu or the palette) without a permission to set opens the dialog.
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    if id != COMMAND || params.get("metadata_permission").is_some() {
        return false;
    }
    open(app, params.get("role").and_then(Value::as_str));
    true
}

/// Opens the dialog on `role` (default the first).
pub fn open(app: &mut App, role: Option<&str>) {
    let all = roles(app);
    let role = role
        .and_then(|r| all.iter().find(|x| x.eq_ignore_ascii_case(r)))
        .or_else(|| all.first())
        .cloned();
    app.cmd.roles = Some(Dialog {
        role,
        ..Dialog::default()
    });
}

impl Editor {
    /// The editor of `row`'s filter: the default editor when it can show it, else the DAX one.
    pub fn of(role: &str, row: &TableRow) -> Self {
        let parsed = role_filter::from_dax(&row.filter, &row.table);
        Editor {
            role: role.to_string(),
            table: row.table.clone(),
            saved: row.filter.clone(),
            mode: if parsed.is_some() {
                Mode::Default
            } else {
                Mode::Dax
            },
            rows: parsed.unwrap_or_default(),
            dax: DaxBox::new(&row.filter),
            ..Editor::default()
        }
    }

    /// The DAX of the editor shown.
    pub fn draft(&self, types: &[(String, String)]) -> String {
        match self.mode {
            Mode::Default => role_filter::to_dax(&self.rows, &|c: &str| column_type(types, c)),
            Mode::Dax => self.dax.text.trim().to_string(),
        }
    }
}

fn column_type(types: &[(String, String)], c: &str) -> Ty {
    types
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(c))
        .map_or(Ty::Text, |(_, t)| Ty::of(t))
}

fn diag_of(d: &Value) -> Diag {
    let n = |k: &str| d.get(k).and_then(Value::as_u64).unwrap_or(0);
    let at = |k: &str| {
        d.get("span")
            .and_then(|s| s.get(k))
            .and_then(Value::as_u64)
            .and_then(|x| usize::try_from(x).ok())
            .unwrap_or(0)
    };
    Diag {
        message: s(d, "message"),
        span: at("start")..at("end"),
        line: n("line"),
        column: n("column"),
        error: d.get("severity").and_then(Value::as_str) == Some("error"),
    }
}

/// The answers of Check and Save.
pub fn answer(app: &mut App, step: Step, r: Result<Value, String>) {
    let Some(e) = app.cmd.roles.as_mut().and_then(|d| d.editor.as_mut()) else {
        return;
    };
    match step {
        Step::Check(n) if n == e.edit => {
            e.checking = false;
            let text = e.verdict.take().map(|v| v.text).unwrap_or_default();
            e.verdict = Some(match r {
                Ok(v) => Verdict {
                    text,
                    valid: v.get("valid").and_then(Value::as_bool).unwrap_or(false),
                    empty: v.get("empty").and_then(Value::as_bool).unwrap_or(false),
                    diags: v
                        .get("diagnostics")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .take(MAX_DIAGS)
                        .map(diag_of)
                        .collect(),
                    error: None,
                },
                Err(err) => Verdict {
                    text,
                    error: Some(crate::app::clip(&err, 600)),
                    ..Verdict::default()
                },
            });
        }
        Step::Save(n) if n == e.edit => {
            e.saving = false;
            match r {
                Ok(v) => {
                    e.saved = v
                        .get("filter_expression")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    e.save_error = None;
                }
                Err(err) => e.save_error = Some(crate::app::clip(&err, 600)),
            }
        }
        _ => {}
    }
}

/// What the DAX editor completes.
struct Complete {
    names: Names,
    funcs: Arc<(Vec<Func>, Vec<String>)>,
}

/// Draws the dialog when it is open.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut d) = app.cmd.roles.clone() else {
        return;
    };
    let l = app.lang;
    let all = roles(app);
    // a role removed (or renamed) elsewhere: show another
    if d.role
        .as_ref()
        .is_none_or(|r| !all.iter().any(|x| x.eq_ignore_ascii_case(r)))
    {
        d.role = all.first().cloned();
    }
    let table_rows = d.role.as_deref().map(|r| rows(app, r)).unwrap_or_default();
    if d.table
        .as_ref()
        .is_none_or(|t| !table_rows.iter().any(|r| r.table.eq_ignore_ascii_case(t)))
    {
        d.table = table_rows.first().map(|r| r.table.clone());
    }
    // the editor follows the role and the table shown
    let row = d
        .table
        .as_ref()
        .and_then(|t| table_rows.iter().find(|r| r.table.eq_ignore_ascii_case(t)));
    match (&d.role, row) {
        (Some(role), Some(row)) => {
            if d.editor.as_ref().is_none_or(|e| {
                !e.role.eq_ignore_ascii_case(role) || !e.table.eq_ignore_ascii_case(&row.table)
            }) {
                d.editor = Some(Editor::of(role, row));
            }
        }
        _ => d.editor = None,
    }
    let complete = match d.editor.as_ref().map(|e| e.mode) {
        Some(Mode::Dax) => Some(Complete {
            names: app
                .loaded
                .as_ref()
                .map(|x| crate::formula::names(&x.snap))
                .unwrap_or_default(),
            funcs: crate::formula::functions(app),
        }),
        _ => None,
    };
    let mut open = true;
    let mut close = false;
    let mut calls: Vec<(Call, Then)> = Vec::new();
    egui::Window::new(l.t("Manage security roles", "Gérer les rôles de sécurité"))
        .id(egui::Id::unique("bicraft-manage-roles"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let mut cx = Cx {
                d: &mut d,
                all: &all,
                table_rows: &table_rows,
                l,
                calls: &mut calls,
                complete: complete.as_ref(),
            };
            body(ui, &mut cx, &mut close);
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    app.cmd.roles = (open && !close).then_some(d);
    for (c, then) in calls {
        app.run(c, then);
    }
}

struct Cx<'a> {
    d: &'a mut Dialog,
    all: &'a [String],
    table_rows: &'a [TableRow],
    l: Lang,
    calls: &'a mut Vec<(Call, Then)>,
    complete: Option<&'a Complete>,
}

impl Cx<'_> {
    fn run(&mut self, id: &'static str, params: Value) {
        self.calls.push((Call { id, params }, Then::Nothing));
    }
}

fn heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(theme::bold(13.0)));
}

fn body(ui: &mut Ui, cx: &mut Cx<'_>, close: &mut bool) {
    let l = cx.l;
    ui.horizontal_top(|ui| {
        // a fixed height: the separators between the columns span it (not the whole screen)
        ui.set_height(BODY_HEIGHT);
        ui.vertical(|ui| {
            ui.set_width(ROLES_W);
            roles_ui(ui, cx);
        });
        ui.separator();
        ui.vertical(|ui| {
            ui.set_width(TABLES_W);
            tables_ui(ui, cx);
        });
        ui.separator();
        ui.vertical(|ui| {
            ui.set_width(EDITOR_W);
            editor_ui(ui, cx);
        });
    });
    ui.add_space(8.0);
    if widgets::primary(ui, l.t("Close", "Fermer"), true, "").clicked() {
        *close = true;
    }
}

fn roles_ui(ui: &mut Ui, cx: &mut Cx<'_>) {
    let l = cx.l;
    let t = Tokens::get(ui.ctx());
    heading(ui, l.t("Roles", "Rôles"));
    egui::ScrollArea::vertical()
        .id_salt("bicraft-roles-list")
        .max_height(250.0)
        .show(ui, |ui| {
            if cx.all.is_empty() {
                ui.label(RichText::new(l.t("No role yet.", "Aucun rôle pour l'instant.")).color(t.text_dim));
            }
            for r in cx.all {
                let on = cx.d.role.as_deref() == Some(r.as_str());
                if ui.selectable_label(on, r.as_str()).clicked() && !on {
                    cx.d.role = Some(r.clone());
                    cx.d.renaming = None;
                    cx.d.editor = None;
                }
            }
        });
    ui.add_space(6.0);
    // rename the chosen role
    if let Some(role) = cx.d.role.clone() {
        match cx.d.renaming.as_mut() {
            Some(name) => {
                let text = format!("{} {role}", l.t("New name of", "Nouveau nom de"));
                let r = ui.add(
                    egui::TextEdit::singleline(name)
                        .char_limit(256)
                        .desired_width(ROLES_W - 10.0),
                );
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, &text));
                let typed = name.trim().to_string();
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let ok = !typed.is_empty() && typed != role;
                let mut apply = false;
                let mut cancel = false;
                ui.horizontal(|ui| {
                    apply = ui
                        .add_enabled(ok, egui::Button::new(l.t("Apply", "Appliquer")))
                        .clicked();
                    cancel = ui.button(l.t("Cancel", "Annuler")).clicked();
                });
                if typed.contains(',') {
                    ui.label(
                        RichText::new(l.t(
                            "A role name cannot contain a comma.",
                            "Un nom de rôle ne peut pas contenir de virgule.",
                        ))
                        .color(t.err_text),
                    );
                }
                if (apply || (enter && ok)) && !typed.contains(',') {
                    cx.run("role.rename", json!({"name": role, "new_name": typed}));
                    cx.d.role = Some(typed.clone());
                    if let Some(e) = cx.d.editor.as_mut() {
                        e.role = typed;
                    }
                    cx.d.renaming = None;
                } else if cancel {
                    cx.d.renaming = None;
                }
            }
            None => {
                ui.horizontal(|ui| {
                    if ui.button(l.t("Rename", "Renommer")).clicked() {
                        cx.d.renaming = Some(role.clone());
                    }
                    if ui.button(l.t("Delete", "Supprimer")).clicked() {
                        cx.run("role.remove", json!({"name": role}));
                    }
                });
            }
        }
    }
    ui.add_space(6.0);
    let label = ui.label(RichText::new(l.t("New role name", "Nom du nouveau rôle")).color(t.text_dim));
    ui.add(
        egui::TextEdit::singleline(&mut cx.d.new_name)
            .char_limit(256)
            .desired_width(ROLES_W - 10.0),
    )
    .labelled_by(label.id);
    let name = cx.d.new_name.trim().to_string();
    if ui
        .add_enabled(
            !name.is_empty() && !name.contains(','),
            egui::Button::new(l.t("New", "Nouveau")),
        )
        .clicked()
    {
        cx.run("role.add", json!({"name": name}));
        cx.d.role = Some(name);
        cx.d.new_name.clear();
        cx.d.editor = None;
    }
}

/// An icon then its word, in `colour` (never colour alone).
fn icon_word(ui: &mut Ui, icon: Icon, word: &str, colour: egui::Color32) {
    let at = ui.cursor().min;
    icons::paint(
        ui.painter(),
        Rect::from_min_size(at + vec2(0.0, 2.0), vec2(14.0, 14.0)),
        icon,
        colour,
        false,
    );
    ui.add_space(16.0);
    ui.label(RichText::new(word).size(11.0).color(colour));
}

fn tables_ui(ui: &mut Ui, cx: &mut Cx<'_>) {
    let l = cx.l;
    let t = Tokens::get(ui.ctx());
    heading(ui, l.t("Select tables", "Sélectionner des tables"));
    if cx.d.role.is_none() {
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("bicraft-roles-tables")
        .max_height(BODY_HEIGHT - 30.0)
        .show(ui, |ui| {
            for row in cx.table_rows {
                let on =
                    cx.d.table
                        .as_ref()
                        .is_some_and(|x| x.eq_ignore_ascii_case(&row.table));
                let r = ui.selectable_label(on, crate::app::clip(&row.table, 28));
                let a11y = format!("{} {}", l.t("Table", "Table"), row.table);
                r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &a11y));
                if r.clicked() && !on {
                    cx.d.table = Some(row.table.clone());
                    cx.d.editor = None;
                    cx.d.columns_open = false;
                }
                if row.hidden || !row.filter.trim().is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        if row.hidden {
                            icon_word(ui, Icon::EyeOff, l.t("hidden", "masquée"), t.text_dim);
                        } else {
                            icon_word(ui, Icon::Filter, l.t("filtered", "filtrée"), t.text_dim);
                        }
                    });
                }
            }
        });
}

/// A combo box labelled `label`; the choice picked.
fn combo(ui: &mut Ui, id: Id, label: &str, current: &str, choices: &[String], w: f32) -> Option<usize> {
    let mut picked = None;
    let r = egui::ComboBox::from_id_salt(id)
        .selected_text(crate::app::clip(current, 40))
        .width(w)
        .show_ui(ui, |ui| {
            for (i, c) in choices.iter().enumerate() {
                if ui.selectable_label(c == current, c.as_str()).clicked() {
                    picked = Some(i);
                }
            }
        });
    r.response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, label));
    picked
}

fn editor_ui(ui: &mut Ui, cx: &mut Cx<'_>) {
    let l = cx.l;
    let t = Tokens::get(ui.ctx());
    let Some(role) = cx.d.role.clone() else {
        ui.label(RichText::new(l.t("Create a role first.", "Créez d'abord un rôle.")).color(t.text_dim));
        return;
    };
    let Some(row) =
        cx.d.table
            .as_ref()
            .and_then(|x| cx.table_rows.iter().find(|r| r.table.eq_ignore_ascii_case(x)))
            .cloned()
    else {
        return;
    };
    ui.horizontal(|ui| {
        heading(
            ui,
            &format!(
                "{} · {}",
                l.t("Filter data", "Filtrer les données"),
                crate::app::clip(&row.table, 40)
            ),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut hidden = row.hidden;
            let text = format!("{} {}", l.t("Hide table", "Masquer la table"), row.table);
            let r = ui.checkbox(&mut hidden, l.t("Hide table", "Masquer la table"));
            r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, hidden, &text));
            if r.changed() {
                let c = permission(&role, &row.table, None, hidden);
                cx.calls.push((c, Then::Nothing));
            }
        });
    });
    if row.hidden {
        ui.label(
            RichText::new(l.t(
                "Hidden from this role (object-level security): the table does not exist for the role's members.",
                "Masquée pour ce rôle (sécurité au niveau des objets) : la table n'existe pas pour les membres du rôle.",
            ))
            .color(t.text_dim),
        );
        return;
    }
    let complete = cx.complete;
    let Some(e) = cx.d.editor.as_mut() else {
        return;
    };
    let columns: Vec<String> = row.columns.iter().map(|c| c.0.clone()).collect();
    // the editor switch
    ui.horizontal(|ui| {
        let (label, to) = match e.mode {
            Mode::Default => (l.t("Switch to DAX editor", "Passer à l'éditeur DAX"), Mode::Dax),
            Mode::Dax => (
                l.t("Switch to default editor", "Passer à l'éditeur par défaut"),
                Mode::Default,
            ),
        };
        if ui
            .add_enabled(!e.confirm_switch, egui::Button::new(label))
            .clicked()
        {
            match to {
                Mode::Dax => {
                    e.dax = DaxBox::new(&e.draft(&row.types));
                    e.dax.focus = true;
                    e.mode = Mode::Dax;
                }
                Mode::Default => match role_filter::from_dax(&e.dax.text, &row.table) {
                    Some(rows) => {
                        e.rows = rows;
                        e.mode = Mode::Default;
                    }
                    None => e.confirm_switch = true,
                },
            }
        }
    });
    if e.confirm_switch {
        egui::Frame::new()
            .stroke(egui::Stroke::new(1.0, t.warn))
            .corner_radius(egui::CornerRadius::same(4))
            .inner_margin(egui::Margin::same(6))
            .show(ui, |ui| {
                ui.set_width(EDITOR_W - 14.0);
                ui.horizontal_wrapped(|ui| {
                    icon_word(ui, Icon::Warning, l.t("Warning", "Avertissement"), t.err_text);
                    ui.label(l.t(
                        "The default editor cannot show this filter: switching loses it.",
                        "L'éditeur par défaut ne peut pas afficher ce filtre : il sera perdu en changeant d'éditeur.",
                    ));
                });
                ui.horizontal(|ui| {
                    if ui.button(l.t("Switch anyway", "Changer quand même")).clicked() {
                        e.rows = Vec::new();
                        e.mode = Mode::Default;
                        e.confirm_switch = false;
                    }
                    if ui.button(l.t("Cancel", "Annuler")).clicked() {
                        e.confirm_switch = false;
                    }
                });
            });
    }
    ui.add_space(4.0);
    match e.mode {
        Mode::Default => conditions_ui(ui, e, &columns, &row, l),
        Mode::Dax => {
            let empty = Names::default();
            let (names, funcs, keywords): (&Names, &[Func], &[String]) = match complete {
                Some(c) => (&c.names, &c.funcs.0, &c.funcs.1),
                None => (&empty, &[], &[]),
            };
            let scope = dax_box::Scope {
                names,
                funcs,
                keywords,
                home: Some(&row.table),
                columns_bare: true,
            };
            let current = e.dax.text.trim().to_string();
            let diags: &[Diag] = match &e.verdict {
                Some(v) if v.text == current => &v.diags,
                _ => &[],
            };
            let label = format!("{} {}", l.t("DAX filter of", "Filtre DAX de"), row.table);
            let diags = diags.to_vec();
            dax_box::show(
                ui,
                &mut e.dax,
                Id::unique(("bicraft-roles-dax", row.table.to_lowercase())),
                &label,
                &scope,
                &diags,
                EDITOR_W - 12.0,
                7,
                l,
            );
            ui.label(
                RichText::new(l.t(
                    "A True/False expression evaluated for each row: the rows where it is True stay. Ctrl+Space lists the names.",
                    "Une expression Vrai/Faux évaluée pour chaque ligne : les lignes où elle est vraie restent. Ctrl+Espace liste les noms.",
                ))
                .size(11.0)
                .color(t.text_dim),
            );
        }
    }
    ui.add_space(6.0);
    // Check, Revert, Save
    let draft = e.draft(&row.types);
    let dirty = draft.trim() != e.saved.trim();
    ui.horizontal(|ui| {
        let check = widgets::small_button(ui, Icon::Check, l.t("Check", "Vérifier"), !e.checking, false)
            .on_hover_text(l.t(
                "Check the filter's DAX without saving it",
                "Vérifier le DAX du filtre sans l'enregistrer",
            ));
        if check.clicked() {
            e.edit = e.edit.wrapping_add(1);
            e.checking = true;
            e.verdict = Some(Verdict {
                text: draft.clone(),
                ..Verdict::default()
            });
            cx.calls.push((
                Call {
                    id: "role.check_filter",
                    params: json!({"role": role, "table": row.table, "filter_expression": draft}),
                },
                Then::Roles(Step::Check(e.edit)),
            ));
        }
        let revert = widgets::small_button(ui, Icon::Undo, l.t("Revert", "Rétablir"), dirty, false)
            .on_hover_text(l.t("Go back to the saved filter", "Revenir au filtre enregistré"));
        if revert.clicked() {
            let saved = TableRow {
                filter: e.saved.clone(),
                ..row.clone()
            };
            *e = Editor {
                edit: e.edit.wrapping_add(1),
                ..Editor::of(&role, &saved)
            };
        }
        ui.add_space(8.0);
        let save_label = format!("{} {}", l.t("Save filter", "Enregistrer le filtre"), row.table);
        let b = widgets::primary(ui, l.t("Save", "Enregistrer"), dirty && !e.saving, "");
        b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, dirty, &save_label));
        if b.clicked() {
            e.edit = e.edit.wrapping_add(1);
            e.saving = true;
            e.save_error = None;
            let f = draft.trim();
            cx.calls.push((
                Call {
                    id: "role.set_filter",
                    params: json!({
                        "role": role,
                        "table": row.table,
                        "filter_expression": if f.is_empty() { Value::Null } else { Value::from(f) },
                    }),
                },
                Then::Roles(Step::Save(e.edit)),
            ));
        }
        if !dirty && !e.saved.trim().is_empty() {
            icon_word(ui, Icon::Filter, l.t("Saved", "Enregistré"), t.text_dim);
        }
    });
    verdict_ui(ui, e, &draft, l, &t);
    ui.add_space(6.0);
    columns_ui(ui, cx, &role, &row);
}

fn conditions_ui(ui: &mut Ui, e: &mut Editor, columns: &[String], row: &TableRow, l: Lang) {
    let t = Tokens::get(ui.ctx());
    ui.label(
        RichText::new(l.t(
            "The rows that match stay visible to the role's members.",
            "Les lignes qui correspondent restent visibles pour les membres du rôle.",
        ))
        .size(11.0)
        .color(t.text_dim),
    );
    let ops: Vec<String> = Op::ALL.iter().map(|o| o.label(l).to_string()).collect();
    let joins = vec![l.t("And", "Et").to_string(), l.t("Or", "Ou").to_string()];
    let mut remove = None;
    egui::ScrollArea::vertical()
        .id_salt("bicraft-roles-conds")
        .max_height(170.0)
        .show(ui, |ui| {
            if e.rows.is_empty() {
                ui.label(
                    RichText::new(l.t(
                        "No condition: every row is visible. New adds one.",
                        "Aucune condition : toutes les lignes sont visibles. Nouveau en ajoute une.",
                    ))
                    .color(t.text_dim),
                );
            }
            for (i, c) in e.rows.iter_mut().enumerate() {
                let n = i + 1;
                let base = Id::unique(("bicraft-roles-cond", row.table.to_lowercase(), i));
                ui.horizontal(|ui| {
                    if i == 0 {
                        ui.add_space(72.0);
                    } else {
                        let cur = match c.join {
                            Join::And => &joins[0],
                            Join::Or => &joins[1],
                        };
                        let label = format!(
                            "{} {n}",
                            l.t("And or Or of condition", "Et ou Ou de la condition")
                        );
                        if let Some(k) = combo(ui, base.with("join"), &label, cur, &joins, 56.0) {
                            c.join = if k == 0 { Join::And } else { Join::Or };
                        }
                    }
                    let label = format!("{} {n}", l.t("Column of condition", "Colonne de la condition"));
                    let cur = if c.column.is_empty() {
                        l.t("Column", "Colonne").to_string()
                    } else {
                        c.column.clone()
                    };
                    if let Some(k) = combo(ui, base.with("col"), &label, &cur, columns, 130.0)
                        && let Some(name) = columns.get(k)
                    {
                        c.column = name.clone();
                    }
                    let label = format!("{} {n}", l.t("Condition", "Condition"));
                    if let Some(k) = combo(ui, base.with("op"), &label, c.op.label(l), &ops, 150.0)
                        && let Some(op) = Op::ALL.get(k)
                    {
                        c.op = *op;
                    }
                    if c.op.has_value() {
                        let label = format!("{} {n}", l.t("Value of condition", "Valeur de la condition"));
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut c.value)
                                .char_limit(4096)
                                .hint_text(l.t("Value", "Valeur"))
                                .desired_width(100.0),
                        );
                        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, &label));
                    }
                    let label = format!("{} {n}", l.t("Delete condition", "Supprimer la condition"));
                    if widgets::icon_toggle(ui, Icon::Delete, &label, true, false).clicked() {
                        remove = Some(i);
                    }
                });
            }
        });
    if let Some(i) = remove
        && i < e.rows.len()
    {
        e.rows.remove(i);
    }
    let can_add = e.rows.len() < MAX_CONDITIONS && !columns.is_empty();
    let add = widgets::small_button(ui, Icon::Plus, l.t("New", "Nouveau"), can_add, false);
    let add_label = l.t("New condition", "Nouvelle condition");
    add.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, can_add, add_label));
    if add.clicked() {
        e.rows.push(Cond {
            column: columns.first().cloned().unwrap_or_default(),
            ..Cond::default()
        });
    }
    // the DAX it makes
    let dax = e.draft(&row.types);
    if !dax.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("DAX").size(11.0).color(t.text_dim));
            let r = ui.label(
                RichText::new(crate::app::clip(&dax, 400))
                    .font(crate::formula::mono())
                    .color(t.text_dim),
            );
            let label = format!("{} {dax}", l.t("DAX of the conditions:", "DAX des conditions :"));
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &label));
        });
    }
}

fn verdict_ui(ui: &mut Ui, e: &Editor, draft: &str, l: Lang, t: &Tokens) {
    if let Some(err) = &e.save_error {
        ui.horizontal_wrapped(|ui| {
            icon_word(
                ui,
                Icon::Warning,
                l.t("Not saved:", "Non enregistré :"),
                t.err_text,
            );
            ui.label(RichText::new(err).size(12.0).color(t.err_text));
        });
    }
    let Some(v) = &e.verdict else { return };
    if e.checking {
        ui.label(RichText::new(l.t("Checking…", "Vérification…")).color(t.text_dim));
        return;
    }
    if v.text.trim() != draft.trim() {
        ui.label(
            RichText::new(l.t(
                "The filter changed since the last check.",
                "Le filtre a changé depuis la dernière vérification.",
            ))
            .size(11.0)
            .color(t.text_dim),
        );
        return;
    }
    if let Some(err) = &v.error {
        ui.horizontal_wrapped(|ui| {
            icon_word(ui, Icon::Warning, l.t("Error:", "Erreur :"), t.err_text);
            ui.label(RichText::new(err).size(12.0).color(t.err_text));
        });
        return;
    }
    if v.valid && v.diags.is_empty() {
        let word = if v.empty {
            l.t(
                "No filter: the role's members see every row.",
                "Aucun filtre : les membres du rôle voient toutes les lignes.",
            )
        } else {
            l.t("No errors found.", "Aucune erreur détectée.")
        };
        ui.horizontal(|ui| icon_word(ui, Icon::Check, word, t.text));
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("bicraft-roles-diags")
        .max_height(80.0)
        .show(ui, |ui| {
            for d in &v.diags {
                ui.horizontal_wrapped(|ui| {
                    let (word, colour) = if d.error {
                        (l.t("Error:", "Erreur :"), t.err_text)
                    } else {
                        (l.t("Warning:", "Avertissement :"), t.text)
                    };
                    icon_word(ui, Icon::Warning, word, colour);
                    ui.label(
                        RichText::new(format!(
                            "{} ({} {}, {} {})",
                            crate::app::clip(&d.message, 300),
                            l.t("line", "ligne"),
                            d.line,
                            l.t("column", "colonne"),
                            d.column
                        ))
                        .size(12.0)
                        .color(colour),
                    );
                });
            }
        });
}

fn columns_ui(ui: &mut Ui, cx: &mut Cx<'_>, role: &str, row: &TableRow) {
    let l = cx.l;
    let open = cx.d.columns_open;
    let n = row.columns.iter().filter(|c| c.1).count();
    let text = format!(
        "{} ({n} {})",
        l.t("Columns", "Colonnes"),
        l.t("hidden", "masquées")
    );
    let a11y = format!("{} {}", l.t("Columns of", "Colonnes de"), row.table);
    let r = ui.selectable_label(open, text.as_str());
    r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, open, &a11y));
    if r.clicked() {
        cx.d.columns_open = !open;
    }
    if open {
        egui::ScrollArea::vertical()
            .id_salt("bicraft-roles-cols")
            .max_height(110.0)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.set_width(EDITOR_W - 12.0);
                    for (c, hidden) in &row.columns {
                        let mut h = *hidden;
                        let label = format!("{} {}[{c}]", l.t("Hide", "Masquer"), row.table);
                        let r = ui.checkbox(&mut h, format!("{} {c}", l.t("Hide", "Masquer")));
                        r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, h, &label));
                        if r.changed() {
                            cx.calls
                                .push((permission(role, &row.table, Some(c), h), Then::Nothing));
                        }
                    }
                });
            });
    }
}
