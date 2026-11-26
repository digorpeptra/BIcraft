//! Quick measures (Power BI Desktop's Quick measures pane, as a dialog): reached from Home ›
//! Calculations › Quick measure, the Data pane's New quick measure on a table or a field, and the
//! palette. A calculation picked by category, its fields and settings; OK runs `measure.quick
//! {kind, params, table?, name?}`, which writes an ordinary measure from BIcraft's DAX template
//! (the engine checks the fields and the DAX; its error goes to the error bar).

use egui::{RichText, Ui};
use serde_json::{Map, Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::Then;
use crate::i18n::Lang;
use crate::theme::Tokens;
use crate::widgets;

pub const QUICK: &str = "measure.quick";
/// Most fields a picker lists, values a filter lists.
const MAX_FIELDS: usize = 2_000;
const MAX_VALUES: usize = 1_000;

/// A calculation: its id, its category, its English and French names.
pub struct Calc {
    pub id: &'static str,
    pub category: (&'static str, &'static str),
    pub en: &'static str,
    pub fr: &'static str,
}

const PER_CATEGORY: (&str, &str) = ("Aggregate per category", "Agrégat par catégorie");
const FILTERS: (&str, &str) = ("Filters", "Filtres");
const TIME: (&str, &str) = ("Time intelligence", "Time intelligence");
const TOTALS: (&str, &str) = ("Totals", "Totaux");
const MATH: (&str, &str) = ("Mathematical operations", "Opérations mathématiques");
const TEXT: (&str, &str) = ("Text", "Texte");

/// The calculations, in the dialog's order (the engine's `measure.quick` kinds).
pub const CALCS: &[Calc] = &[
    Calc {
        id: "average_per_category",
        category: PER_CATEGORY,
        en: "Average per category",
        fr: "Moyenne par catégorie",
    },
    Calc {
        id: "variance_per_category",
        category: PER_CATEGORY,
        en: "Variance per category",
        fr: "Variance par catégorie",
    },
    Calc {
        id: "max_per_category",
        category: PER_CATEGORY,
        en: "Max per category",
        fr: "Maximum par catégorie",
    },
    Calc {
        id: "min_per_category",
        category: PER_CATEGORY,
        en: "Min per category",
        fr: "Minimum par catégorie",
    },
    Calc {
        id: "weighted_average_per_category",
        category: PER_CATEGORY,
        en: "Weighted average per category",
        fr: "Moyenne pondérée par catégorie",
    },
    Calc {
        id: "filtered_value",
        category: FILTERS,
        en: "Filtered value",
        fr: "Valeur filtrée",
    },
    Calc {
        id: "difference_from_filtered_value",
        category: FILTERS,
        en: "Difference from filtered value",
        fr: "Écart avec la valeur filtrée",
    },
    Calc {
        id: "percentage_difference_from_filtered_value",
        category: FILTERS,
        en: "Percentage difference from filtered value",
        fr: "Écart en pourcentage avec la valeur filtrée",
    },
    Calc {
        id: "sales_from_new_customers",
        category: FILTERS,
        en: "Sales from new customers",
        fr: "Ventes des nouveaux clients",
    },
    Calc {
        id: "year_to_date",
        category: TIME,
        en: "Year-to-date total",
        fr: "Cumul depuis le début de l'année",
    },
    Calc {
        id: "quarter_to_date",
        category: TIME,
        en: "Quarter-to-date total",
        fr: "Cumul depuis le début du trimestre",
    },
    Calc {
        id: "month_to_date",
        category: TIME,
        en: "Month-to-date total",
        fr: "Cumul depuis le début du mois",
    },
    Calc {
        id: "year_over_year_change",
        category: TIME,
        en: "Year-over-year change",
        fr: "Variation d'une année sur l'autre",
    },
    Calc {
        id: "quarter_over_quarter_change",
        category: TIME,
        en: "Quarter-over-quarter change",
        fr: "Variation d'un trimestre sur l'autre",
    },
    Calc {
        id: "month_over_month_change",
        category: TIME,
        en: "Month-over-month change",
        fr: "Variation d'un mois sur l'autre",
    },
    Calc {
        id: "rolling_average",
        category: TIME,
        en: "Rolling average",
        fr: "Moyenne mobile",
    },
    Calc {
        id: "running_total",
        category: TOTALS,
        en: "Running total",
        fr: "Total cumulé",
    },
    Calc {
        id: "total_for_category_filtered",
        category: TOTALS,
        en: "Total for category (filters applied)",
        fr: "Total de la catégorie (filtres appliqués)",
    },
    Calc {
        id: "total_for_category_unfiltered",
        category: TOTALS,
        en: "Total for category (filters not applied)",
        fr: "Total de la catégorie (sans les filtres)",
    },
    Calc {
        id: "addition",
        category: MATH,
        en: "Addition",
        fr: "Addition",
    },
    Calc {
        id: "subtraction",
        category: MATH,
        en: "Subtraction",
        fr: "Soustraction",
    },
    Calc {
        id: "multiplication",
        category: MATH,
        en: "Multiplication",
        fr: "Multiplication",
    },
    Calc {
        id: "division",
        category: MATH,
        en: "Division",
        fr: "Division",
    },
    Calc {
        id: "percentage_difference",
        category: MATH,
        en: "Percentage difference",
        fr: "Écart en pourcentage",
    },
    Calc {
        id: "correlation_coefficient",
        category: MATH,
        en: "Correlation coefficient",
        fr: "Coefficient de corrélation",
    },
    Calc {
        id: "star_rating",
        category: TEXT,
        en: "Star rating",
        fr: "Évaluation par étoiles",
    },
    Calc {
        id: "concatenated_list",
        category: TEXT,
        en: "Concatenated list of values",
        fr: "Liste concaténée de valeurs",
    },
];

/// One input of a calculation: its `params` key, its kind, its English and French labels.
#[derive(Clone, Copy)]
enum Input {
    /// A measure or a column (summed or counted).
    Value(&'static str, &'static str, &'static str),
    /// A column.
    Column(&'static str, &'static str, &'static str),
    /// A date column.
    Date(&'static str, &'static str, &'static str),
    /// A number typed, with its default text.
    Number(&'static str, &'static str, &'static str, &'static str),
    /// A text typed, with its default.
    Text(&'static str, &'static str, &'static str, &'static str),
    /// A check box.
    Flag(&'static str, &'static str, &'static str),
    /// The rolling average's period.
    Period,
    /// The filter's values (from `column.values` of the `filter` column).
    Values,
}

const BASE: Input = Input::Value("base", "Base value", "Valeur de base");
const CATEGORY: Input = Input::Column("category", "Category", "Catégorie");
const DATE: Input = Input::Date("date", "Date", "Date");

fn inputs(id: &str) -> Vec<Input> {
    match id {
        "average_per_category"
        | "variance_per_category"
        | "max_per_category"
        | "min_per_category"
        | "total_for_category_filtered"
        | "total_for_category_unfiltered" => vec![BASE, CATEGORY],
        "weighted_average_per_category" => {
            vec![BASE, Input::Value("weight", "Weight", "Pondération"), CATEGORY]
        }
        "filtered_value" | "difference_from_filtered_value" | "percentage_difference_from_filtered_value" => {
            vec![BASE, Input::Column("filter", "Filter", "Filtre"), Input::Values]
        }
        "sales_from_new_customers" => vec![BASE, Input::Column("customer", "Customer", "Client"), DATE],
        "year_to_date" => vec![
            BASE,
            DATE,
            Input::Number(
                "year_end_month",
                "Year ends in month (1–12)",
                "L'exercice finit le mois (1–12)",
                "12",
            ),
        ],
        "correlation_coefficient" => vec![
            CATEGORY,
            Input::Value("base", "Measure X", "Mesure X"),
            Input::Value("second", "Measure Y", "Mesure Y"),
        ],
        "rolling_average" => vec![
            BASE,
            DATE,
            Input::Period,
            Input::Number("before", "Periods before", "Périodes avant", "3"),
            Input::Number("after", "Periods after", "Périodes après", "0"),
        ],
        "running_total" => vec![
            BASE,
            Input::Column("field", "Field", "Champ"),
            Input::Flag("descending", "Descending order", "Ordre décroissant"),
        ],
        "addition" | "subtraction" | "multiplication" | "division" | "percentage_difference" => {
            vec![BASE, Input::Value("second", "Second value", "Deuxième valeur")]
        }
        "star_rating" => vec![
            BASE,
            Input::Number("min_value", "Lowest value", "Valeur la plus basse", "0"),
            Input::Number("max_value", "Highest value", "Valeur la plus haute", ""),
            Input::Number("stars", "Number of stars", "Nombre d'étoiles", "5"),
        ],
        "concatenated_list" => vec![
            Input::Column("field", "Field", "Champ"),
            Input::Number("max_values", "Values listed", "Valeurs listées", "3"),
            Input::Text("separator", "Separator", "Séparateur", ", "),
        ],
        _ => vec![BASE, DATE],
    }
}

/// The open dialog's values.
#[derive(Debug, Clone, PartialEq)]
pub struct QuickDialog {
    pub kind: &'static str,
    /// Field inputs (`'T'[C]` or `[M]`), numbers and texts as typed, flags.
    pub fields: Map<String, Value>,
    pub period: &'static str,
    /// The filter column's values once read (for the column named), and those ticked.
    pub values: Option<(String, Vec<Value>)>,
    pub picked: Vec<Value>,
    /// The measure's name (empty: the engine's default).
    pub name: String,
    pub problem: Option<String>,
}

/// A field picker's entry: (text the command takes, label, table, data type or `measure`).
type Entry = (String, String, String, String);

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

/// The model's columns and measures, as picker entries.
fn entries(app: &App) -> Vec<Entry> {
    let mut out = Vec::new();
    let tables = app
        .loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/model/tables"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for t in tables {
        let tn = t
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let arr = |k: &str| t.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
        for c in arr("columns") {
            let n = c.get("name").and_then(Value::as_str).unwrap_or_default();
            let ty = c.get("data_type").and_then(Value::as_str).unwrap_or_default();
            out.push((
                format!("{}{}", table_ref(&tn), bracket(n)),
                n.to_string(),
                tn.clone(),
                ty.to_string(),
            ));
        }
        for m in arr("measures") {
            let n = m.get("name").and_then(Value::as_str).unwrap_or_default();
            out.push((bracket(n), n.to_string(), tn.clone(), "measure".into()));
        }
        if out.len() > MAX_FIELDS {
            break;
        }
    }
    out.truncate(MAX_FIELDS);
    out
}

/// Opens the dialog, `base` (a field text) as its base value when given.
pub fn open(app: &mut App, base: Option<&str>) {
    let mut fields = Map::new();
    if let Some(b) = base {
        fields.insert("base".into(), json!(b));
    }
    app.cmd.quick = Some(QuickDialog {
        kind: "average_per_category",
        fields,
        period: "month",
        values: None,
        picked: Vec::new(),
        name: String::new(),
        problem: None,
    });
}

/// `measure.quick` without its kind (a ribbon, menu or palette click) opens the dialog; `base`
/// (from a field's menu) starts it.
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    if id != QUICK || params.get("kind").is_some() {
        return false;
    }
    let base = params.get("base").and_then(Value::as_str).map(str::to_string);
    open(app, base.as_deref());
    true
}

/// Keeps the answer of the filter column's `column.values` ([`Then::QuickValues`]).
pub fn values_answer(app: &mut App, column: String, r: Result<Value, String>) {
    let Some(d) = app.cmd.quick.as_mut() else {
        return;
    };
    let values = match r {
        Ok(v) => v
            .get("values")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        Err(e) => {
            d.problem = Some(crate::app::clip(&e, 300));
            Vec::new()
        }
    };
    d.values = Some((column, values));
}

fn number(s: &str) -> Option<f64> {
    s.trim()
        .replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|x| x.is_finite())
}

fn label(l: Lang, en: &'static str, fr: &'static str) -> &'static str {
    l.t(en, fr)
}

/// The command the dialog's values give, or what is missing.
pub fn call(d: &QuickDialog, l: Lang) -> Result<Call, String> {
    let mut params = Map::new();
    for input in inputs(d.kind) {
        match input {
            Input::Value(k, en, fr) | Input::Column(k, en, fr) | Input::Date(k, en, fr) => {
                let v = d.fields.get(k).and_then(Value::as_str).unwrap_or_default();
                if v.is_empty() {
                    return Err(format!(
                        "{} {}",
                        label(l, en, fr),
                        l.t("is needed.", ": à renseigner.")
                    ));
                }
                params.insert(k.into(), json!(v));
            }
            Input::Number(k, en, fr, default) => {
                let text = d.fields.get(k).and_then(Value::as_str).unwrap_or(default);
                if text.trim().is_empty() {
                    if default.is_empty() {
                        return Err(format!(
                            "{} {}",
                            label(l, en, fr),
                            l.t("is needed.", ": à renseigner.")
                        ));
                    }
                    continue;
                }
                let x = number(text).ok_or_else(|| {
                    format!(
                        "{} {}",
                        label(l, en, fr),
                        l.t("must be a number.", ": doit être un nombre.")
                    )
                })?;
                params.insert(k.into(), json!(x));
            }
            Input::Text(k, _, _, default) => {
                let t = d.fields.get(k).and_then(Value::as_str).unwrap_or(default);
                params.insert(k.into(), json!(t));
            }
            Input::Flag(k, _, _) => {
                params.insert(
                    k.into(),
                    json!(d.fields.get(k).and_then(Value::as_bool).unwrap_or(false)),
                );
            }
            Input::Period => {
                params.insert("period".into(), json!(d.period));
            }
            Input::Values => {
                if d.picked.is_empty() {
                    return Err(l
                        .t("Tick at least one value.", "Cochez au moins une valeur.")
                        .into());
                }
                params.insert("values".into(), Value::Array(d.picked.clone()));
            }
        }
    }
    let mut p = json!({"kind": d.kind, "params": params});
    if !d.name.trim().is_empty() {
        p["name"] = json!(d.name.trim());
    }
    Ok(Call { id: QUICK, params: p })
}

/// Draws the open dialog.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut d) = app.cmd.quick.clone() else {
        return;
    };
    let l = app.lang;
    let mut open = true;
    let mut close = false;
    egui::Window::new(l.t("Quick measures", "Mesures rapides"))
        .id(egui::Id::unique("bicraft-quick-measures"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(480.0);
            body(app, ui, &mut d, l, &mut close);
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    app.cmd.quick = (open && !close).then_some(d);
}

fn labelled(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui, egui::Id)) {
    ui.horizontal(|ui| {
        let r = ui.add_sized([140.0, 20.0], egui::Label::new(label));
        add(ui, r.id);
    });
}

fn calc_name(id: &str, l: Lang) -> &'static str {
    CALCS.iter().find(|c| c.id == id).map_or("", |c| l.t(c.en, c.fr))
}

const PERIODS: [(&str, &str, &str); 4] = [
    ("day", "Days", "Jours"),
    ("month", "Months", "Mois"),
    ("quarter", "Quarters", "Trimestres"),
    ("year", "Years", "Années"),
];

fn body(app: &mut App, ui: &mut Ui, d: &mut QuickDialog, l: Lang, close: &mut bool) {
    let t = Tokens::of(app.prefs.theme);
    ui.label(
        RichText::new(l.t(
            "A measure written for you from a common calculation; its DAX shows in the formula bar.",
            "Une mesure écrite pour vous à partir d'un calcul courant ; son DAX s'affiche dans la barre de formule.",
        ))
        .color(t.text_dim),
    );
    ui.add_space(6.0);
    labelled(ui, l.t("Calculation", "Calcul"), |ui, id| {
        egui::ComboBox::from_id_salt("bicraft-quick-calc")
            .selected_text(calc_name(d.kind, l))
            .width(300.0)
            .height(420.0)
            .show_ui(ui, |ui| {
                let mut last = "";
                for c in CALCS {
                    let cat = l.t(c.category.0, c.category.1);
                    if cat != last {
                        ui.label(
                            RichText::new(cat)
                                .color(t.text_dim)
                                .font(crate::theme::bold(12.0)),
                        );
                        last = cat;
                    }
                    ui.selectable_value(&mut d.kind, c.id, l.t(c.en, c.fr));
                }
            })
            .response
            .labelled_by(id);
    });
    ui.add_space(4.0);
    let all = entries(app);
    for input in inputs(d.kind) {
        input_ui(app, ui, d, input, &all, l, &t);
    }
    ui.add_space(4.0);
    labelled(ui, l.t("Measure name", "Nom de la mesure"), |ui, id| {
        ui.add(
            egui::TextEdit::singleline(&mut d.name)
                .char_limit(512)
                .hint_text(l.t("(automatic)", "(automatique)"))
                .desired_width(300.0),
        )
        .labelled_by(id);
    });
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

/// A field picker over `all` filtered by `keep`.
fn picker(
    ui: &mut Ui,
    d: &mut QuickDialog,
    key: &str,
    label: &str,
    all: &[Entry],
    keep: impl Fn(&Entry) -> bool,
    l: Lang,
) {
    let current = d
        .fields
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let shown = all.iter().find(|e| e.0 == current).map_or_else(
        || l.t("(pick a field)", "(choisissez un champ)").to_string(),
        |e| format!("{} · {}", e.2, e.1),
    );
    labelled(ui, label, |ui, id| {
        egui::ComboBox::from_id_salt(("bicraft-quick-field", key))
            .selected_text(shown)
            .width(300.0)
            .height(320.0)
            .show_ui(ui, |ui| {
                for e in all.iter().filter(|e| keep(e)) {
                    let text = if e.3 == "measure" {
                        format!("{} · {} ({})", e.2, e.1, l.t("measure", "mesure"))
                    } else {
                        format!("{} · {}", e.2, e.1)
                    };
                    if ui.selectable_label(e.0 == current, text).clicked() {
                        d.fields.insert(key.into(), json!(e.0));
                    }
                }
            })
            .response
            .labelled_by(id);
    });
}

fn input_ui(
    app: &mut App,
    ui: &mut Ui,
    d: &mut QuickDialog,
    input: Input,
    all: &[Entry],
    l: Lang,
    t: &Tokens,
) {
    match input {
        Input::Value(k, en, fr) => picker(ui, d, k, l.t(en, fr), all, |_| true, l),
        Input::Column(k, en, fr) => picker(ui, d, k, l.t(en, fr), all, |e| e.3 != "measure", l),
        Input::Date(k, en, fr) => picker(ui, d, k, l.t(en, fr), all, |e| e.3 == "dateTime", l),
        Input::Number(k, en, fr, default) => {
            let mut text = d
                .fields
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or(default)
                .to_string();
            labelled(ui, l.t(en, fr), |ui, id| {
                ui.add(egui::TextEdit::singleline(&mut text).desired_width(100.0))
                    .labelled_by(id);
            });
            d.fields.insert(k.into(), json!(text));
        }
        Input::Text(k, en, fr, default) => {
            let mut text = d
                .fields
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or(default)
                .to_string();
            labelled(ui, l.t(en, fr), |ui, id| {
                ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .char_limit(20)
                        .desired_width(100.0),
                )
                .labelled_by(id);
            });
            d.fields.insert(k.into(), json!(text));
        }
        Input::Flag(k, en, fr) => {
            let mut on = d.fields.get(k).and_then(Value::as_bool).unwrap_or(false);
            ui.checkbox(&mut on, l.t(en, fr));
            d.fields.insert(k.into(), json!(on));
        }
        Input::Period => {
            labelled(ui, l.t("Period", "Période"), |ui, id| {
                let cur = PERIODS
                    .iter()
                    .find(|p| p.0 == d.period)
                    .map_or("", |p| l.t(p.1, p.2));
                egui::ComboBox::from_id_salt("bicraft-quick-period")
                    .selected_text(cur)
                    .width(140.0)
                    .show_ui(ui, |ui| {
                        for (k, en, fr) in PERIODS {
                            ui.selectable_value(&mut d.period, k, l.t(en, fr));
                        }
                    })
                    .response
                    .labelled_by(id);
            });
        }
        Input::Values => {
            let Some(column) = d.fields.get("filter").and_then(Value::as_str).map(str::to_string) else {
                return;
            };
            // the column's values, read once per column picked
            if d.values.as_ref().is_none_or(|(c, _)| *c != column) {
                let entry = all.iter().find(|e| e.0 == column);
                if let Some(e) = entry
                    && d.values.as_ref().is_none_or(|(c, _)| !c.is_empty())
                {
                    d.values = Some((String::new(), Vec::new()));
                    d.picked.clear();
                    app.run(
                        Call {
                            id: "column.values",
                            params: json!({"table": e.2, "column": e.1, "limit": MAX_VALUES}),
                        },
                        Then::QuickValues(column.clone()),
                    );
                }
            }
            ui.label(RichText::new(l.t("Values", "Valeurs")).font(crate::theme::bold(13.0)));
            egui::Frame::new()
                .stroke(egui::Stroke::new(1.0, t.border))
                .corner_radius(egui::CornerRadius::same(4))
                .inner_margin(egui::Margin::same(4))
                .show(ui, |ui| {
                    ui.set_width(460.0);
                    egui::ScrollArea::vertical()
                        .id_salt("bicraft-quick-values")
                        .max_height(140.0)
                        .show(ui, |ui| match &d.values {
                            Some((c, values)) if *c == column => {
                                for v in values.clone() {
                                    let mut on = d.picked.contains(&v);
                                    if ui.checkbox(&mut on, crate::group_dialog::shown(&v, l)).changed() {
                                        if on {
                                            d.picked.push(v);
                                        } else {
                                            d.picked.retain(|x| *x != v);
                                        }
                                    }
                                }
                            }
                            _ => {
                                ui.label(
                                    RichText::new(l.t("Reading the values…", "Lecture des valeurs…"))
                                        .color(t.text_dim),
                                );
                            }
                        });
                });
        }
    }
}
