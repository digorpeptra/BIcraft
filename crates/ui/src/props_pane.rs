//! The Model view's Properties pane (ROADMAP lot 18, docs/gaps.md G61): the selected table,
//! column, measure or relationship's properties as fields, as Desktop's Properties pane shows
//! them. Each field sends its engine command when it changes (a text field when it loses the
//! focus or Enter is pressed, a box or a list at once): `table.rename`/`table.set`,
//! `column.rename`/`column.set`, `measure.rename`/`measure.set`, `relationship.set`; for the
//! Model explorer's other objects `calculation_item.set`, `hierarchy.set`, `role.rename`/
//! `role.set`, `expression.rename`/`expression.set` (a DAX or M text in a multi-line field, sent
//! when it loses the focus; read-only when the snapshot clipped it), cultures and data sources
//! read-only. The command
//! goes back to the app through [`Props::run`], which sends it as is; the snapshot then shows the
//! new value. Read-only when the session cannot edit the model.
//!
//! Several selected tables, columns and measures ([`Props::multi`]) show the properties they
//! share; a value they differ on is blank ("Multiple values", a half-ticked box) and a change
//! sends one `model.set_properties` for all of them. One object's Synonyms go through it too.
//!
//! Lists come from the command registry's forms where it has them (data types, summarize by),
//! so the pane offers exactly what the command accepts.

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::i18n::Lang;
use crate::registry::{self, Kind};

/// The data categories `column.set` accepts (its params doc).
const DATA_CATEGORIES: &[&str] = &[
    "Address",
    "Place",
    "City",
    "County",
    "StateOrProvince",
    "PostalCode",
    "Country",
    "Continent",
    "Latitude",
    "Longitude",
    "WebUrl",
    "ImageUrl",
    "Barcode",
];
/// Longest text a field takes.
const MAX_TEXT: usize = 10_000;
/// Longest DAX or M a code field takes.
const MAX_CODE: usize = 200_000;
/// Most columns the Sort by column list offers.
const MAX_SORT_COLUMNS: usize = 500;

/// The pane's state for one frame.
pub struct Props<'a> {
    pub l: Lang,
    pub editable: bool,
    /// The text field being typed in: (field key, text).
    pub edit: &'a mut Option<(String, String)>,
    /// The command a field sends (id, params), for the app.
    pub run: &'a mut Option<(&'static str, Value)>,
    /// Muted text colour (the labels).
    pub muted: egui::Color32,
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn yes(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool) == Some(true)
}

/// The values of enum parameter `param` of command `id` in the registry.
fn enum_values(id: &str, param: &str) -> &'static [&'static str] {
    registry::find(id)
        .and_then(|c| c.form.iter().find(|p| p.name == param))
        .and_then(|p| match p.kind {
            Kind::Enum(v) => Some(v),
            _ => None,
        })
        .unwrap_or(&[])
}

/// `params` with `extra` added.
fn with(params: &Value, extra: Value) -> Value {
    let mut p = params.clone();
    if let (Some(o), Value::Object(e)) = (p.as_object_mut(), extra) {
        o.extend(e);
    }
    p
}

impl Props<'_> {
    fn label(&self, ui: &mut Ui, text: &str) -> egui::Response {
        ui.label(RichText::new(text).color(self.muted))
    }

    /// A one-line text field `key` showing `current`; sends `make(new text)` when it is left
    /// changed.
    fn text(
        &mut self,
        ui: &mut Ui,
        key: &str,
        label: &str,
        current: &str,
        make: impl FnOnce(&str) -> (&'static str, Value),
    ) {
        self.text_hint(ui, key, label, current, "", make);
    }

    /// [`Props::text`] with `hint` shown in the empty field (the selection's values differ).
    fn text_hint(
        &mut self,
        ui: &mut Ui,
        key: &str,
        label: &str,
        current: &str,
        hint: &str,
        make: impl FnOnce(&str) -> (&'static str, Value),
    ) {
        let l = self.label(ui, label);
        let mut buf = match self.edit.as_ref() {
            Some((k, t)) if k == key => t.clone(),
            _ => current.to_string(),
        };
        let r = ui
            .add_enabled(
                self.editable,
                egui::TextEdit::singleline(&mut buf)
                    .char_limit(MAX_TEXT)
                    .hint_text(hint.to_string())
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(l.id);
        if r.changed() {
            *self.edit = Some((key.to_string(), buf.clone()));
        }
        if r.lost_focus() {
            if self.edit.as_ref().is_some_and(|(k, _)| k == key) {
                *self.edit = None;
            }
            if buf != current {
                *self.run = Some(make(&buf));
            }
        }
        ui.end_row();
    }

    /// A check box; sends `make(new value)` when it is clicked.
    fn check(
        &mut self,
        ui: &mut Ui,
        label: &str,
        current: bool,
        make: impl FnOnce(bool) -> (&'static str, Value),
    ) {
        self.check_mixed(ui, label, Some(current), make);
    }

    /// A check box; `None`: the selection's values differ (drawn half ticked; a click ticks it
    /// for all).
    fn check_mixed(
        &mut self,
        ui: &mut Ui,
        label: &str,
        current: Option<bool>,
        make: impl FnOnce(bool) -> (&'static str, Value),
    ) {
        ui.label("");
        let mut v = current.unwrap_or(false);
        if ui
            .add_enabled(
                self.editable,
                egui::Checkbox::new(&mut v, label).indeterminate(current.is_none()),
            )
            .changed()
            && Some(v) != current
        {
            *self.run = Some(make(v));
        }
        ui.end_row();
    }

    /// A list of `options` ((value, text)); sends `make(value)` when another one is picked.
    fn choose(
        &mut self,
        ui: &mut Ui,
        id: &str,
        label: &str,
        current: Option<&str>,
        options: &[(Option<String>, String)],
        make: impl FnOnce(Option<&str>) -> (&'static str, Value),
    ) {
        self.choose_mixed(ui, id, label, Some(current), options, make);
    }

    /// [`Props::choose`]; `current` `None`: the selection's values differ (the list says so, and
    /// any pick sets it for all).
    fn choose_mixed(
        &mut self,
        ui: &mut Ui,
        id: &str,
        label: &str,
        current: Option<Option<&str>>,
        options: &[(Option<String>, String)],
        make: impl FnOnce(Option<&str>) -> (&'static str, Value),
    ) {
        self.label(ui, label);
        let mixed = current.is_none();
        let current = current.flatten();
        let shown = if mixed {
            mixed_text(self.l).to_string()
        } else {
            options
                .iter()
                .find(|(v, _)| v.as_deref() == current)
                .map(|(_, t)| t.clone())
                .or_else(|| current.map(str::to_string))
                .unwrap_or_else(|| "–".into())
        };
        let mut picked: Option<Option<String>> = None;
        ui.add_enabled_ui(self.editable, |ui| {
            let r = egui::ComboBox::from_id_salt(("bicraft-props", id))
                .selected_text(crate::app::clip(&shown, 60))
                .width(ui.available_width().max(80.0))
                .show_ui(ui, |ui| {
                    for (v, t) in options {
                        if ui
                            .selectable_label(!mixed && v.as_deref() == current, crate::app::clip(t, 60))
                            .clicked()
                        {
                            picked = Some(v.clone());
                        }
                    }
                });
            let enabled = self.editable;
            r.response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, enabled, label));
        });
        if let Some(v) = picked
            && (mixed || v.as_deref() != current)
        {
            *self.run = Some(make(v.as_deref()));
        }
        ui.end_row();
    }

    fn grid(&mut self, ui: &mut Ui, id: &str, add: impl FnOnce(&mut Self, &mut Ui)) {
        egui::Grid::new(("bicraft-props-grid", id))
            .num_columns(2)
            .spacing([12.0, 6.0])
            .min_col_width(90.0)
            .show(ui, |ui| add(self, ui));
    }

    /// A table's properties.
    pub fn table(&mut self, ui: &mut Ui, t: &Value) {
        let l = self.l;
        let name = s(t, "name").to_string();
        let key = |f: &str| format!("table|{name}|{f}");
        let base = json!({ "name": name });
        self.grid(ui, "table", |p, ui| {
            p.text(ui, &key("name"), l.t("Name", "Nom"), &name, |v| {
                ("table.rename", json!({"name": name, "new_name": v}))
            });
            p.text(
                ui,
                &key("description"),
                l.t("Description", "Description"),
                s(t, "description"),
                |v| ("table.set", with(&base, json!({"description": nullable(v)}))),
            );
            p.synonyms(ui, &key("synonyms"), t, json!({ "table": name }));
            p.check(ui, l.t("Is hidden", "Masquée"), yes(t, "hidden"), |v| {
                ("table.set", with(&base, json!({"hidden": v})))
            });
            // Advanced › Storage mode, as in Desktop's Properties pane
            ui.label(RichText::new(l.t("Advanced", "Avancé")).strong());
            ui.end_row();
            let mode = s(t, "storage_mode");
            if yes(t, "calculated") {
                p.label(ui, l.t("Storage mode", "Mode de stockage"));
                ui.label(l.t("Import (calculated table)", "Importation (table calculée)"));
                ui.end_row();
            } else {
                let options: Vec<(Option<String>, String)> = crate::storage_ui::MODES
                    .iter()
                    .map(|m| {
                        (
                            Some((*m).to_string()),
                            crate::storage_ui::mode_text(m, l).to_string(),
                        )
                    })
                    .collect();
                let current = if mode.is_empty() { "import" } else { mode };
                let shown = crate::storage_ui::mode_text(current, l).to_string();
                let mut opts = options;
                if !crate::storage_ui::MODES.contains(&current) {
                    opts.push((Some(current.to_string()), shown));
                }
                p.choose(
                    ui,
                    &key("storage_mode"),
                    l.t("Storage mode", "Mode de stockage"),
                    Some(current),
                    &opts,
                    |v| {
                        (
                            crate::storage_ui::COMMAND,
                            json!({"table": name, "mode": v.unwrap_or("import")}),
                        )
                    },
                );
            }
        });
    }

    /// A column's properties (`table` is its table's snapshot object).
    pub fn column(&mut self, ui: &mut Ui, c: &Value, table: &Value) {
        let l = self.l;
        let tname = s(table, "name").to_string();
        let name = s(c, "name").to_string();
        let key = |f: &str| format!("column|{tname}|{name}|{f}");
        let base = json!({"table": tname, "name": name});
        self.grid(ui, "column", |p, ui| {
            p.text(ui, &key("name"), l.t("Name", "Nom"), &name, |v| {
                ("column.rename", with(&base, json!({"new_name": v})))
            });
            p.text(
                ui,
                &key("description"),
                l.t("Description", "Description"),
                s(c, "description"),
                |v| ("column.set", with(&base, json!({"description": nullable(v)}))),
            );
            p.synonyms(ui, &key("synonyms"), c, json!({"table": tname, "column": name}));
            p.text(
                ui,
                &key("display_folder"),
                l.t("Display folder", "Dossier d'affichage"),
                s(c, "display_folder"),
                |v| ("column.set", with(&base, json!({"display_folder": v}))),
            );
            p.check(ui, l.t("Is hidden", "Masquée"), yes(c, "hidden"), |v| {
                ("column.set", with(&base, json!({"hidden": v})))
            });
            let types = values(enum_values("column.set", "data_type"));
            p.choose(
                ui,
                &key("data_type"),
                l.t("Data type", "Type de données"),
                Some(s(c, "data_type")),
                &types,
                |v| ("column.set", with(&base, json!({"data_type": v}))),
            );
            p.text(
                ui,
                &key("format_string"),
                l.t("Format", "Format"),
                s(c, "format_string"),
                |v| ("column.set", with(&base, json!({"format_string": v}))),
            );
            let mut sorts: Vec<(Option<String>, String)> =
                vec![(None, l.t("(itself)", "(elle-même)").to_string())];
            sorts.extend(
                table
                    .get("columns")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|o| s(o, "name"))
                    .filter(|n| !n.is_empty() && *n != name)
                    .take(MAX_SORT_COLUMNS)
                    .map(|n| (Some(n.to_string()), n.to_string())),
            );
            let sort = c.get("sort_by_column").and_then(Value::as_str);
            p.choose(
                ui,
                &key("sort_by_column"),
                l.t("Sort by column", "Trier par colonne"),
                sort,
                &sorts,
                |v| ("column.set", with(&base, json!({"sort_by_column": v}))),
            );
            let mut cats = vec![(None, l.t("Uncategorized", "Non catégorisée").to_string())];
            cats.extend(values(DATA_CATEGORIES));
            let cat = c.get("data_category").and_then(Value::as_str);
            p.choose(
                ui,
                &key("data_category"),
                l.t("Data category", "Catégorie de données"),
                cat,
                &cats,
                |v| ("column.set", with(&base, json!({"data_category": v}))),
            );
            let sums = values(enum_values("column.set", "summarize_by"));
            let sum = c.get("summarize_by").and_then(Value::as_str);
            p.choose(
                ui,
                &key("summarize_by"),
                l.t("Summarize by", "Synthétiser par"),
                sum,
                &sums,
                |v| ("column.set", with(&base, json!({"summarize_by": v}))),
            );
        });
    }

    /// A measure's properties.
    pub fn measure(&mut self, ui: &mut Ui, m: &Value, table: &str) {
        let l = self.l;
        let name = s(m, "name").to_string();
        let key = |f: &str| format!("measure|{table}|{name}|{f}");
        let base = json!({"table": table, "name": name});
        self.grid(ui, "measure", |p, ui| {
            p.text(ui, &key("name"), l.t("Name", "Nom"), &name, |v| {
                ("measure.rename", with(&base, json!({"new_name": v})))
            });
            p.text(
                ui,
                &key("description"),
                l.t("Description", "Description"),
                s(m, "description"),
                |v| ("measure.set", with(&base, json!({"description": nullable(v)}))),
            );
            p.synonyms(ui, &key("synonyms"), m, json!({"table": table, "measure": name}));
            p.text(
                ui,
                &key("display_folder"),
                l.t("Display folder", "Dossier d'affichage"),
                s(m, "display_folder"),
                |v| ("measure.set", with(&base, json!({"display_folder": v}))),
            );
            p.check(ui, l.t("Is hidden", "Masquée"), yes(m, "hidden"), |v| {
                ("measure.set", with(&base, json!({"hidden": v})))
            });
            p.text(
                ui,
                &key("format_string"),
                l.t("Format", "Format"),
                s(m, "format_string"),
                |v| ("measure.set", with(&base, json!({"format_string": v}))),
            );
        });
    }

    /// A relationship's properties (`base` names it: `{name}` or `{from, to}`).
    pub fn relationship(&mut self, ui: &mut Ui, r: &Value, base: &Value) {
        let l = self.l;
        let key = |f: &str| format!("rel|{}|{f}", base);
        let card = [
            (Some("one".to_string()), l.t("One (1)", "Un (1)").to_string()),
            (
                Some("many".to_string()),
                l.t("Many (*)", "Plusieurs (*)").to_string(),
            ),
        ];
        self.grid(ui, "rel", |p, ui| {
            for (k, label) in [
                (
                    "from_cardinality",
                    l.t("From cardinality", "Cardinalité de départ"),
                ),
                ("to_cardinality", l.t("To cardinality", "Cardinalité d'arrivée")),
            ] {
                let cur = s(r, k).to_ascii_lowercase();
                p.choose(ui, &key(k), label, Some(cur.as_str()), &card, |v| {
                    ("relationship.set", with(base, json!({ k: v })))
                });
            }
            let cross = [
                (Some("single".to_string()), l.t("Single", "Unique").to_string()),
                (Some("both".to_string()), l.t("Both", "Les deux").to_string()),
            ];
            let cur = s(r, "cross_filter").to_ascii_lowercase();
            p.choose(
                ui,
                &key("cross_filter"),
                l.t("Cross-filter direction", "Direction du filtrage croisé"),
                Some(cur.as_str()),
                &cross,
                |v| ("relationship.set", with(base, json!({"cross_filter": v}))),
            );
            let active = r.get("active").and_then(Value::as_bool) != Some(false);
            p.check(
                ui,
                l.t("Make this relationship active", "Relation active"),
                active,
                |v| ("relationship.set", with(base, json!({"active": v}))),
            );
            let both = s(r, "security_filtering").eq_ignore_ascii_case("both");
            p.check(
                ui,
                l.t(
                    "Apply security filter in both directions",
                    "Filtre de sécurité dans les deux sens",
                ),
                both,
                |v| {
                    let to = if v { "both" } else { "single" };
                    ("relationship.set", with(base, json!({"security_filtering": to})))
                },
            );
        });
    }

    /// A multi-line code field `key` (DAX or M) showing `current`; sends `make(new text)` when it
    /// is left changed. Read-only when the snapshot clipped the text (`cut`): a change would lose
    /// the rest.
    fn code(
        &mut self,
        ui: &mut Ui,
        key: &str,
        label: &str,
        current: &str,
        cut: bool,
        make: impl FnOnce(&str) -> (&'static str, Value),
    ) {
        let l = self.label(ui, label);
        let mut buf = match self.edit.as_ref() {
            Some((k, t)) if k == key => t.clone(),
            _ => current.to_string(),
        };
        let r = ui
            .add_enabled(
                self.editable && !cut,
                egui::TextEdit::multiline(&mut buf)
                    .code_editor()
                    .char_limit(MAX_CODE)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(l.id);
        if r.changed() {
            *self.edit = Some((key.to_string(), buf.clone()));
        }
        if r.lost_focus() {
            if self.edit.as_ref().is_some_and(|(k, _)| k == key) {
                *self.edit = None;
            }
            if buf != current {
                *self.run = Some(make(&buf));
            }
        }
    }

    /// A read-only value.
    fn value(&mut self, ui: &mut Ui, label: &str, text: &str) {
        self.label(ui, label);
        ui.label(crate::app::clip(text, 200));
        ui.end_row();
    }

    /// A calculation item's properties (`group` its calculation group): `calculation_item.set`.
    pub fn calc_item(&mut self, ui: &mut Ui, it: &Value, group: &str) {
        let l = self.l;
        let name = s(it, "name").to_string();
        let key = |f: &str| format!("calc_item|{group}|{name}|{f}");
        let base = json!({"group": group, "name": name});
        let cut = yes(it, "truncated");
        self.grid(ui, "calc_item", |p, ui| {
            p.text(ui, &key("name"), l.t("Name", "Nom"), &name, |v| {
                ("calculation_item.set", with(&base, json!({"new_name": v})))
            });
            p.text(
                ui,
                &key("description"),
                l.t("Description", "Description"),
                s(it, "description"),
                |v| {
                    (
                        "calculation_item.set",
                        with(&base, json!({"description": nullable(v)})),
                    )
                },
            );
            let ordinal = it
                .get("ordinal")
                .and_then(Value::as_i64)
                .map(|o| o.to_string())
                .unwrap_or_else(|| "–".into());
            p.value(ui, l.t("Ordinal", "Ordinal"), &ordinal);
        });
        self.code(
            ui,
            &key("expression"),
            l.t("Expression", "Expression"),
            s(it, "expression"),
            cut,
            |v| ("calculation_item.set", with(&base, json!({"expression": v}))),
        );
        self.code(
            ui,
            &key("format_string_expression"),
            l.t("Format string expression", "Expression de chaîne de format"),
            s(it, "format_string_expression"),
            cut,
            |v| {
                (
                    "calculation_item.set",
                    with(&base, json!({"format_string_expression": nullable(v)})),
                )
            },
        );
    }

    /// A hierarchy's properties (`table` its table): `hierarchy.set`; its levels, read-only.
    pub fn hierarchy(&mut self, ui: &mut Ui, h: &Value, table: &str) {
        let l = self.l;
        let name = s(h, "name").to_string();
        let key = |f: &str| format!("hierarchy|{table}|{name}|{f}");
        let base = json!({"table": table, "name": name});
        self.grid(ui, "hierarchy", |p, ui| {
            p.text(ui, &key("name"), l.t("Name", "Nom"), &name, |v| {
                ("hierarchy.set", with(&base, json!({"new_name": v})))
            });
            p.text(
                ui,
                &key("description"),
                l.t("Description", "Description"),
                s(h, "description"),
                |v| ("hierarchy.set", with(&base, json!({"description": nullable(v)}))),
            );
            p.text(
                ui,
                &key("display_folder"),
                l.t("Display folder", "Dossier d'affichage"),
                s(h, "display_folder"),
                |v| {
                    (
                        "hierarchy.set",
                        with(&base, json!({"display_folder": nullable(v)})),
                    )
                },
            );
            p.check(ui, l.t("Is hidden", "Masquée"), yes(h, "hidden"), |v| {
                ("hierarchy.set", with(&base, json!({"hidden": v})))
            });
            let levels: Vec<&str> = h
                .get("levels")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|x| s(x, "name"))
                .collect();
            p.value(ui, l.t("Levels", "Niveaux"), &levels.join(" › "));
        });
    }

    /// A role's properties (`model.security`'s entry): `role.rename`, `role.set`; its row
    /// filters are counted (Manage roles edits them).
    pub fn role(&mut self, ui: &mut Ui, r: &Value) {
        let l = self.l;
        let name = s(r, "name").to_string();
        let key = |f: &str| format!("role|{name}|{f}");
        let base = json!({ "name": name });
        self.grid(ui, "role", |p, ui| {
            p.text(ui, &key("name"), l.t("Name", "Nom"), &name, |v| {
                ("role.rename", with(&base, json!({"new_name": v})))
            });
            p.text(
                ui,
                &key("description"),
                l.t("Description", "Description"),
                s(r, "description"),
                |v| ("role.set", with(&base, json!({"description": nullable(v)}))),
            );
            let perms = values(enum_values("role.set", "model_permission"));
            let cur = r
                .get("model_permission")
                .and_then(Value::as_str)
                .unwrap_or("read");
            p.choose(
                ui,
                &key("model_permission"),
                l.t("Model permission", "Autorisation sur le modèle"),
                Some(cur),
                &perms,
                |v| {
                    (
                        "role.set",
                        with(&base, json!({"model_permission": v.unwrap_or("read")})),
                    )
                },
            );
            let filters = r
                .get("tables")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|t| !s(t, "filter_expression").trim().is_empty())
                .count();
            p.value(
                ui,
                l.t("Tables filtered", "Tables filtrées"),
                &filters.to_string(),
            );
        });
    }

    /// A culture's properties, read-only (`culture.add` and `culture.remove` are its only
    /// commands; translations go through `translation.set`).
    pub fn culture(&mut self, ui: &mut Ui, c: &Value) {
        let l = self.l;
        let yes_no = |b: bool| if b { l.t("Yes", "Oui") } else { l.t("No", "Non") };
        self.grid(ui, "culture", |p, ui| {
            p.value(ui, l.t("Name", "Nom"), s(c, "name"));
            p.value(
                ui,
                l.t("Model culture", "Culture du modèle"),
                yes_no(yes(c, "model")),
            );
            p.value(
                ui,
                l.t("Linguistic schema", "Schéma linguistique"),
                yes_no(yes(c, "linguistic_metadata")),
            );
        });
    }

    /// A shared expression's properties: `expression.rename`, `expression.set` (description,
    /// query group, its M text).
    pub fn expression(&mut self, ui: &mut Ui, e: &Value) {
        let l = self.l;
        let name = s(e, "name").to_string();
        let key = |f: &str| format!("expression|{name}|{f}");
        let base = json!({ "name": name });
        self.grid(ui, "expression", |p, ui| {
            p.text(ui, &key("name"), l.t("Name", "Nom"), &name, |v| {
                ("expression.rename", with(&base, json!({"new_name": v})))
            });
            let kind = if s(e, "kind") == "parameter" {
                l.t("Parameter", "Paramètre")
            } else {
                l.t("Query", "Requête")
            };
            p.value(ui, l.t("Kind", "Type"), kind);
            p.text(
                ui,
                &key("description"),
                l.t("Description", "Description"),
                s(e, "description"),
                |v| ("expression.set", with(&base, json!({"description": nullable(v)}))),
            );
            p.text(
                ui,
                &key("query_group"),
                l.t("Query group", "Groupe de requêtes"),
                s(e, "query_group"),
                |v| ("expression.set", with(&base, json!({"query_group": nullable(v)}))),
            );
        });
        self.code(
            ui,
            &key("expression"),
            l.t("Expression (M)", "Expression (M)"),
            s(e, "expression"),
            yes(e, "truncated"),
            |v| ("expression.set", with(&base, json!({"expression": v}))),
        );
    }

    /// A data source's properties, read-only (no command edits them; connection strings and
    /// credentials are not in the snapshot).
    pub fn data_source(&mut self, ui: &mut Ui, d: &Value) {
        let l = self.l;
        let dash = |k: &str| match s(d, k) {
            "" => "–".to_string(),
            v => v.to_string(),
        };
        self.grid(ui, "data_source", |p, ui| {
            p.value(ui, l.t("Name", "Nom"), s(d, "name"));
            p.value(ui, l.t("Type", "Type"), &dash("type"));
            p.value(ui, l.t("Provider", "Fournisseur"), &dash("provider"));
            p.value(ui, l.t("Protocol", "Protocole"), &dash("protocol"));
            p.value(ui, l.t("Description", "Description"), &dash("description"));
        });
    }

    /// The Synonyms field (Q&A terms, comma separated) of the object `target` names.
    fn synonyms(&mut self, ui: &mut Ui, key: &str, obj: &Value, target: Value) {
        let l = self.l;
        let current = obj
            .get("synonyms")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
            .unwrap_or_default();
        self.text(ui, key, l.t("Synonyms", "Synonymes"), &current, |v| {
            (
                SET,
                json!({"objects": [target], "props": {"synonyms": split_terms(v)}}),
            )
        });
    }

    /// The properties several selected tables, columns and measures share, as Desktop's
    /// Properties pane shows a multi-selection: a value they differ on is blank ("Multiple
    /// values"), and a change sets it on all of them in one `model.set_properties` (one undo
    /// step). Names and synonyms are per object: they are not offered here.
    pub fn multi(&mut self, ui: &mut Ui, objs: &[Picked<'_>]) {
        let l = self.l;
        let refs: Vec<Value> = objs.iter().map(Picked::target).collect();
        let sig = serde_json::to_string(&refs).unwrap_or_default();
        let key = |f: &str| format!("multi|{sig}|{f}");
        let set = |props: Value| -> (&'static str, Value) { (SET, json!({"objects": refs, "props": props})) };
        let shared_text = |k: &str| -> Option<String> {
            let first = objs.first().map(|o| s(o.obj, k))?;
            objs.iter()
                .all(|o| s(o.obj, k) == first)
                .then(|| first.to_string())
        };
        let shared_opt = |k: &str| -> Option<Option<&str>> {
            let first = objs.first().map(|o| some_str(o.obj, k))?;
            objs.iter().all(|o| some_str(o.obj, k) == first).then_some(first)
        };
        let hidden = objs
            .first()
            .map(|o| yes(o.obj, "hidden"))
            .filter(|h| objs.iter().all(|o| yes(o.obj, "hidden") == *h));
        let kinds = |k: ObjKind| objs.iter().all(|o| o.kind == k);
        let no_tables = objs.iter().all(|o| o.kind != ObjKind::Table);
        let mixed = mixed_text(l);
        self.grid(ui, "multi", |p, ui| {
            let desc = shared_text("description");
            p.text_hint(
                ui,
                &key("description"),
                l.t("Description", "Description"),
                desc.as_deref().unwrap_or(""),
                if desc.is_none() { mixed } else { "" },
                |v| set(json!({"description": nullable(v)})),
            );
            if no_tables {
                let folder = shared_text("display_folder");
                p.text_hint(
                    ui,
                    &key("display_folder"),
                    l.t("Display folder", "Dossier d'affichage"),
                    folder.as_deref().unwrap_or(""),
                    if folder.is_none() { mixed } else { "" },
                    |v| set(json!({"display_folder": v})),
                );
            }
            p.check_mixed(ui, l.t("Is hidden", "Masquée"), hidden, |v| {
                set(json!({"hidden": v}))
            });
            if no_tables {
                let format = shared_text("format_string");
                p.text_hint(
                    ui,
                    &key("format_string"),
                    l.t("Format", "Format"),
                    format.as_deref().unwrap_or(""),
                    if format.is_none() { mixed } else { "" },
                    |v| set(json!({"format_string": v})),
                );
            }
            if kinds(ObjKind::Column) {
                // Sort by column: the columns of their table, when they are all in one
                let table = objs.first().map(|o| s(o.table, "name")).unwrap_or("");
                if objs.iter().all(|o| s(o.table, "name") == table)
                    && let Some(t) = objs.first().map(|o| o.table)
                {
                    let picked: Vec<&str> = objs.iter().map(|o| s(o.obj, "name")).collect();
                    let mut sorts: Vec<(Option<String>, String)> =
                        vec![(None, l.t("(itself)", "(elle-même)").to_string())];
                    sorts.extend(
                        t.get("columns")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .map(|o| s(o, "name"))
                            .filter(|n| !n.is_empty() && !picked.contains(n))
                            .take(MAX_SORT_COLUMNS)
                            .map(|n| (Some(n.to_string()), n.to_string())),
                    );
                    p.choose_mixed(
                        ui,
                        &key("sort_by_column"),
                        l.t("Sort by column", "Trier par colonne"),
                        shared_opt("sort_by_column"),
                        &sorts,
                        |v| set(json!({"sort_by_column": v})),
                    );
                }
                let mut cats = vec![(None, l.t("Uncategorized", "Non catégorisée").to_string())];
                cats.extend(values(DATA_CATEGORIES));
                p.choose_mixed(
                    ui,
                    &key("data_category"),
                    l.t("Data category", "Catégorie de données"),
                    shared_opt("data_category"),
                    &cats,
                    |v| set(json!({"data_category": v})),
                );
                let sums = values(enum_values("column.set", "summarize_by"));
                p.choose_mixed(
                    ui,
                    &key("summarize_by"),
                    l.t("Summarize by", "Synthétiser par"),
                    shared_opt("summarize_by"),
                    &sums,
                    |v| set(json!({"summarize_by": v})),
                );
            }
            if kinds(ObjKind::Table) && !objs.iter().any(|o| yes(o.obj, "calculated")) {
                ui.label(RichText::new(l.t("Advanced", "Avancé")).strong());
                ui.end_row();
                let first = objs.first().map(|o| mode_of(o.obj));
                let mode = first.filter(|f| objs.iter().all(|o| mode_of(o.obj) == *f));
                let options: Vec<(Option<String>, String)> = crate::storage_ui::MODES
                    .iter()
                    .map(|m| {
                        (
                            Some((*m).to_string()),
                            crate::storage_ui::mode_text(m, l).to_string(),
                        )
                    })
                    .collect();
                p.choose_mixed(
                    ui,
                    &key("storage_mode"),
                    l.t("Storage mode", "Mode de stockage"),
                    mode.map(Some),
                    &options,
                    |v| set(json!({"storage_mode": v.unwrap_or("import")})),
                );
            }
        });
    }
}

/// The command that sets the selection's properties.
pub const SET: &str = "model.set_properties";

/// What a selected object is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjKind {
    Table,
    Column,
    Measure,
}

/// One object of a multi-selection: its kind, its table's snapshot object and its own (the table
/// again for a table).
#[derive(Debug, Clone, Copy)]
pub struct Picked<'a> {
    pub kind: ObjKind,
    pub table: &'a Value,
    pub obj: &'a Value,
}

impl Picked<'_> {
    /// The object as `model.set_properties` names it.
    fn target(&self) -> Value {
        let t = s(self.table, "name");
        let n = s(self.obj, "name");
        match self.kind {
            ObjKind::Table => json!({ "table": t }),
            ObjKind::Column => json!({"table": t, "column": n}),
            ObjKind::Measure => json!({"table": t, "measure": n}),
        }
    }
}

/// What a field shows when the selected objects' values differ.
pub fn mixed_text(l: Lang) -> &'static str {
    l.t("Multiple values", "Plusieurs valeurs")
}

/// Synonyms typed as `a, b; c`: the terms, trimmed, without blanks.
fn split_terms(v: &str) -> Vec<String> {
    v.split([',', ';'])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// A text field's value for a property that `null` clears.
fn nullable(v: &str) -> Value {
    if v.is_empty() { Value::Null } else { json!(v) }
}

fn values(list: &[&str]) -> Vec<(Option<String>, String)> {
    list.iter()
        .map(|v| (Some((*v).to_string()), (*v).to_string()))
        .collect()
}

/// A string property, `None` when absent or empty.
fn some_str<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str).filter(|x| !x.is_empty())
}

/// A table's storage mode (Import when the snapshot names none).
fn mode_of(t: &Value) -> &str {
    match s(t, "storage_mode") {
        "" => "import",
        m => m,
    }
}
