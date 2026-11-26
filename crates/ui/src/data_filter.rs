//! The Data view's text, number and date filters (docs/desktop.md "Data view"): the section of a
//! column header's menu named after the column's type (Text filters, Number filters, Date
//! filters) lists the tests (Equals, Begins with, Greater than, Between…); each opens the
//! **Custom filter** window with that test, a second test joined by And or Or, OK and Cancel.
//! The filter is view state sent as `table.rows`' `filter` (one condition a test with And, an
//! `any` list with Or), as the values checklist is.

use egui::{Id, RichText, Ui};
use serde_json::{Value, json};

use crate::App;
use crate::app::clip;
use crate::data_view::{self, Col};
use crate::i18n::Lang;
use crate::theme::{self, Tokens};

/// A column's filter in the Data view.
#[derive(Debug, Clone, PartialEq)]
pub enum ColFilter {
    /// The values kept (the menu's checklist).
    Values(Vec<Value>),
    /// The custom filter's tests `(op, value)`, joined by Or (`or`) or And.
    Custom { tests: Vec<(String, Value)>, or: bool },
}

impl ColFilter {
    /// The `table.rows` conditions of the filter on `column`.
    pub fn conditions(&self, column: &str) -> Vec<Value> {
        match self {
            ColFilter::Values(v) => vec![json!({"column": column, "values": v})],
            ColFilter::Custom { tests, or: true } if tests.len() > 1 => vec![json!({
                "column": column,
                "any": tests.iter().map(|(op, v)| test_json(op, v)).collect::<Vec<_>>(),
            })],
            ColFilter::Custom { tests, .. } => tests
                .iter()
                .map(|(op, v)| {
                    let mut c = test_json(op, v);
                    c["column"] = json!(column);
                    c
                })
                .collect(),
        }
    }
}

fn test_json(op: &str, v: &Value) -> Value {
    if matches!(op, "blank" | "not_blank") {
        json!({"op": op})
    } else {
        json!({"op": op, "value": v})
    }
}

/// Which tests a column offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Number,
    Date,
}

impl Kind {
    /// The kind of a column of `data_type` (`None`: true/false and unknown types).
    pub fn of(data_type: &str) -> Option<Kind> {
        match data_type.to_ascii_lowercase().as_str() {
            "string" => Some(Kind::Text),
            "int64" | "double" | "decimal" => Some(Kind::Number),
            "datetime" => Some(Kind::Date),
            _ => None,
        }
    }

    /// The menu section's title.
    pub fn title(self, l: Lang) -> &'static str {
        match self {
            Kind::Text => l.t("Text filters", "Filtres de texte"),
            Kind::Number => l.t("Number filters", "Filtres de nombre"),
            Kind::Date => l.t("Date filters", "Filtres de date"),
        }
    }

    /// The tests: `table.rows` op, English and French labels.
    pub fn ops(self) -> &'static [(&'static str, &'static str, &'static str)] {
        match self {
            Kind::Text => TEXT_OPS,
            Kind::Number => NUMBER_OPS,
            Kind::Date => DATE_OPS,
        }
    }
}

const TEXT_OPS: &[(&str, &str, &str)] = &[
    ("=", "Equals", "Est égal à"),
    ("<>", "Does not equal", "N'est pas égal à"),
    ("starts_with", "Begins with", "Commence par"),
    ("not_starts_with", "Does not begin with", "Ne commence pas par"),
    ("ends_with", "Ends with", "Se termine par"),
    ("not_ends_with", "Does not end with", "Ne se termine pas par"),
    ("contains", "Contains", "Contient"),
    ("not_contains", "Does not contain", "Ne contient pas"),
    ("blank", "Is blank", "Est vide"),
    ("not_blank", "Is not blank", "N'est pas vide"),
];

const NUMBER_OPS: &[(&str, &str, &str)] = &[
    ("=", "Equals", "Est égal à"),
    ("<>", "Does not equal", "N'est pas égal à"),
    (">", "Greater than", "Supérieur à"),
    (">=", "Greater than or equal to", "Supérieur ou égal à"),
    ("<", "Less than", "Inférieur à"),
    ("<=", "Less than or equal to", "Inférieur ou égal à"),
    ("blank", "Is blank", "Est vide"),
    ("not_blank", "Is not blank", "N'est pas vide"),
];

const DATE_OPS: &[(&str, &str, &str)] = &[
    ("=", "Equals", "Est égal à"),
    ("<>", "Does not equal", "N'est pas égal à"),
    ("<", "Before", "Avant"),
    ("<=", "On or before", "Le ou avant"),
    (">", "After", "Après"),
    (">=", "On or after", "Le ou après"),
    ("blank", "Is blank", "Est vide"),
    ("not_blank", "Is not blank", "N'est pas vide"),
];

/// The Custom filter window's state.
#[derive(Debug, Clone)]
pub struct Custom {
    pub column: String,
    pub kind: Kind,
    /// The first test: op and the text typed.
    pub first: (String, String),
    /// The second test (op `""`: none).
    pub second: (String, String),
    pub or: bool,
}

fn label(kind: Kind, op: &str, l: Lang) -> &'static str {
    kind.ops()
        .iter()
        .find(|(c, _, _)| *c == op)
        .map_or("", |(_, en, fr)| match l {
            Lang::En => en,
            Lang::Fr => fr,
        })
}

/// The menu's section of tests for column `col`; returns the window to open when one is clicked.
pub fn menu_section(ui: &mut Ui, col: &Col, current: Option<&ColFilter>, l: Lang) -> Option<Custom> {
    let kind = Kind::of(&col.data_type)?;
    let mut open = None;
    let start = |op: &str| -> Custom {
        // the filter set before, when it is a custom one
        if let Some(ColFilter::Custom { tests, or }) = current {
            let text = |i: usize| tests.get(i).map(|(o, v)| (o.clone(), value_text(v)));
            let first = text(0).unwrap_or_default();
            if first.0 == op {
                return Custom {
                    column: col.name.clone(),
                    kind,
                    first,
                    second: text(1).unwrap_or_default(),
                    or: *or,
                };
            }
        }
        Custom {
            column: col.name.clone(),
            kind,
            first: (op.to_string(), String::new()),
            second: (String::new(), String::new()),
            or: false,
        }
    };
    egui::CollapsingHeader::new(kind.title(l))
        .id_salt(("bicraft-data-tests", &col.name))
        .show(ui, |ui| {
            for (op, en, fr) in kind.ops() {
                let text = match l {
                    Lang::En => *en,
                    Lang::Fr => *fr,
                };
                if ui.button(format!("{text}…")).clicked() {
                    open = Some(start(op));
                }
            }
            if kind != Kind::Text && ui.button(l.t("Between…", "Entre…")).clicked() {
                open = Some(Custom {
                    second: ("<=".into(), String::new()),
                    ..start(">=")
                });
            }
            if ui.button(l.t("Custom filter…", "Filtre personnalisé…")).clicked() {
                let first = match current {
                    Some(ColFilter::Custom { tests, .. }) => {
                        tests.first().map_or("=".to_string(), |(o, _)| o.clone())
                    }
                    _ => "=".to_string(),
                };
                open = Some(start(&first));
            }
        });
    open
}

fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        // 100 as typed, not 100.0
        Value::Number(n) => match n.as_f64() {
            Some(x) if x.fract() == 0.0 && x.abs() < 1e15 => format!("{}", x as i64),
            _ => n.to_string(),
        },
        other => other.to_string(),
    }
}

/// The value sent for the text typed: a number for a number column when it reads as one.
fn value_of(kind: Kind, text: &str) -> Value {
    let t = text.trim();
    if kind == Kind::Number
        && let Ok(x) = t.replace(',', ".").parse::<f64>()
        && x.is_finite()
    {
        return json!(x);
    }
    json!(t)
}

/// One test's row: the op picker and the value box.
fn test_row(ui: &mut Ui, id: &str, kind: Kind, test: &mut (String, String), optional: bool, l: Lang) {
    ui.horizontal(|ui| {
        let shown = if test.0.is_empty() {
            l.t("(none)", "(aucun)")
        } else {
            label(kind, &test.0, l)
        };
        let what = match id {
            "first" => l.t("First test", "Premier test"),
            _ => l.t("Second test", "Second test"),
        };
        let r = egui::ComboBox::from_id_salt(("bicraft-custom-filter", id))
            .selected_text(shown)
            .width(190.0)
            .show_ui(ui, |ui| {
                if optional
                    && ui
                        .selectable_label(test.0.is_empty(), l.t("(none)", "(aucun)"))
                        .clicked()
                {
                    test.0.clear();
                }
                for (op, en, fr) in kind.ops() {
                    let text = match l {
                        Lang::En => *en,
                        Lang::Fr => *fr,
                    };
                    if ui.selectable_label(test.0 == *op, text).clicked() {
                        test.0 = (*op).to_string();
                    }
                }
            });
        r.response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, what));
        let needs_value = !test.0.is_empty() && !matches!(test.0.as_str(), "blank" | "not_blank");
        let hint = match kind {
            Kind::Date => "2024-01-31",
            _ => "",
        };
        let value_label = format!("{what}: {}", l.t("value", "valeur"));
        let r = ui.add_enabled(
            needs_value,
            egui::TextEdit::singleline(&mut test.1)
                .hint_text(hint)
                .desired_width(160.0),
        );
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, needs_value, &value_label));
    });
}

/// The Custom filter window; OK sets the column's filter.
pub fn window(app: &mut App, ctx: &egui::Context) {
    let Some(mut d) = app.cmd.data.custom.take() else {
        return;
    };
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let mut open = true;
    let mut close = false;
    let mut apply: Option<ColFilter> = None;
    egui::Window::new(l.t("Custom filter", "Filtre personnalisé"))
        .id(Id::unique("bicraft-data-custom-filter"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(400.0);
            ui.label(
                RichText::new(format!(
                    "{} {}",
                    l.t("Show rows where", "Afficher les lignes où"),
                    clip(&d.column, 60)
                ))
                .font(theme::bold(13.0))
                .color(t.text),
            );
            test_row(ui, "first", d.kind, &mut d.first, false, l);
            ui.horizontal(|ui| {
                ui.radio_value(&mut d.or, false, l.t("And", "Et"));
                ui.radio_value(&mut d.or, true, l.t("Or", "Ou"));
            });
            test_row(ui, "second", d.kind, &mut d.second, true, l);
            ui.horizontal(|ui| {
                if ui.button(l.t("OK", "OK")).clicked() {
                    let tests: Vec<(String, Value)> = [&d.first, &d.second]
                        .into_iter()
                        .filter(|(op, _)| !op.is_empty())
                        .map(|(op, text)| (op.clone(), value_of(d.kind, text)))
                        .collect();
                    apply = Some(ColFilter::Custom { tests, or: d.or });
                }
                if ui.button(l.t("Cancel", "Annuler")).clicked() {
                    close = true;
                }
            });
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    if let Some(f) = apply {
        let column = d.column.clone();
        data_view::set_filter(app, &column, Some(f));
        return;
    }
    if open && !close {
        app.cmd.data.custom = Some(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_become_table_rows_conditions() {
        let v = ColFilter::Values(vec![json!("a")]);
        assert_eq!(v.conditions("C"), vec![json!({"column": "C", "values": ["a"]})]);
        let and = ColFilter::Custom {
            tests: vec![(">".into(), json!(1.0)), ("blank".into(), json!(""))],
            or: false,
        };
        assert_eq!(
            and.conditions("C"),
            vec![
                json!({"column": "C", "op": ">", "value": 1.0}),
                json!({"column": "C", "op": "blank"})
            ]
        );
        let or = ColFilter::Custom {
            tests: vec![("contains".into(), json!("x")), ("ends_with".into(), json!("y"))],
            or: true,
        };
        assert_eq!(
            or.conditions("C"),
            vec![
                json!({"column": "C", "any": [{"op": "contains", "value": "x"}, {"op": "ends_with", "value": "y"}]})
            ]
        );
        assert_eq!(value_of(Kind::Number, " 2,5 "), json!(2.5));
        assert_eq!(value_of(Kind::Number, "abc"), json!("abc"));
        assert_eq!(value_of(Kind::Text, "12"), json!("12"));
        assert_eq!(Kind::of("dateTime"), Some(Kind::Date));
        assert_eq!(Kind::of("boolean"), None);
    }
}
