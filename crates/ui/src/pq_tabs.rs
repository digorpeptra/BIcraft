//! The Power Query editor's Transform and Add column tabs, and the Home tab's Keep rows and
//! Remove rows menus (ROADMAP M11 lot 12, docs/gaps.md G67): row, column, text, number and date
//! transforms, each written by `query.apply_step` (`bicraft_data::m::transform_more` writes the
//! M). The tabs only ask for the step's values; every item runs the same engine command.

use egui::Ui;
use serde_json::{Value, json};

use crate::i18n::Lang;
use crate::icons::Icon;
use crate::pq_editor::{CondRule, Editor, Field, FieldKind, Form};
use crate::widgets;

/// What a tab's click asks for.
pub enum TabAct {
    Run(Value),
    Form(Form),
}

/// A column operation of the Format, Extract, Standard, Rounding, Date and Time menus: (`op`,
/// English, French).
type Op = (&'static str, &'static str, &'static str);

pub const FORMAT_OPS: [Op; 7] = [
    ("lower", "lowercase", "minuscules"),
    ("upper", "UPPERCASE", "MAJUSCULES"),
    ("proper", "Capitalize Each Word", "Majuscule à chaque mot"),
    ("trim", "Trim", "Supprimer les espaces"),
    ("clean", "Clean", "Nettoyer"),
    ("prefix", "Add prefix", "Ajouter un préfixe"),
    ("suffix", "Add suffix", "Ajouter un suffixe"),
];

pub const EXTRACT_OPS: [Op; 7] = [
    ("length", "Length", "Longueur"),
    ("first_chars", "First characters", "Premiers caractères"),
    ("last_chars", "Last characters", "Derniers caractères"),
    ("text_range", "Range", "Plage"),
    (
        "before_delimiter",
        "Text before delimiter",
        "Texte avant le délimiteur",
    ),
    (
        "after_delimiter",
        "Text after delimiter",
        "Texte après le délimiteur",
    ),
    (
        "between_delimiters",
        "Text between delimiters",
        "Texte entre les délimiteurs",
    ),
];

pub const STANDARD_OPS: [Op; 8] = [
    ("add", "Add", "Ajouter"),
    ("subtract", "Subtract", "Soustraire"),
    ("multiply", "Multiply", "Multiplier"),
    ("divide", "Divide", "Diviser"),
    ("integer_divide", "Integer-divide", "Division entière"),
    ("modulo", "Modulo", "Modulo"),
    ("percentage", "Percentage", "Pourcentage"),
    ("percent_of", "Percent of", "Pourcentage de"),
];

/// `square` and `cube` are `power` with 2 and 3 ([`op_act`]).
pub const SCIENTIFIC_OPS: [Op; 9] = [
    ("abs", "Absolute value", "Valeur absolue"),
    ("square", "Square", "Carré"),
    ("cube", "Cube", "Cube"),
    ("power", "Power…", "Puissance…"),
    ("square_root", "Square root", "Racine carrée"),
    ("exponent", "Exponent", "Exponentielle"),
    ("log10", "Base-10 logarithm", "Logarithme de base 10"),
    ("ln", "Natural logarithm", "Logarithme népérien"),
    ("factorial", "Factorial", "Factorielle"),
];

pub const TRIG_OPS: [Op; 6] = [
    ("sin", "Sine", "Sinus"),
    ("cos", "Cosine", "Cosinus"),
    ("tan", "Tangent", "Tangente"),
    ("asin", "Arcsine", "Arc sinus"),
    ("acos", "Arccosine", "Arc cosinus"),
    ("atan", "Arctangent", "Arc tangente"),
];

pub const ROUNDING_OPS: [Op; 3] = [
    ("round", "Round…", "Arrondir…"),
    ("round_up", "Round up", "Arrondir au supérieur"),
    ("round_down", "Round down", "Arrondir à l'inférieur"),
];

pub const INFO_OPS: [Op; 3] = [
    ("is_even", "Is even", "Est pair"),
    ("is_odd", "Is odd", "Est impair"),
    ("sign", "Sign", "Signe"),
];

pub const DATE_OPS: [Op; 23] = [
    ("age", "Age", "Âge"),
    ("year", "Year", "Année"),
    ("quarter", "Quarter of year", "Trimestre de l'année"),
    ("month", "Month", "Mois"),
    ("month_name", "Name of month", "Nom du mois"),
    ("week_of_year", "Week of year", "Semaine de l'année"),
    ("day", "Day", "Jour"),
    ("day_of_week", "Day of week", "Jour de la semaine"),
    ("day_name", "Name of day", "Nom du jour"),
    ("day_of_year", "Day of year", "Jour de l'année"),
    ("start_of_month", "Start of month", "Début du mois"),
    ("end_of_month", "End of month", "Fin du mois"),
    ("start_of_year", "Start of year", "Début de l'année"),
    ("end_of_year", "End of year", "Fin de l'année"),
    ("start_of_quarter", "Start of quarter", "Début du trimestre"),
    ("end_of_quarter", "End of quarter", "Fin du trimestre"),
    ("week_of_month", "Week of month", "Semaine du mois"),
    ("start_of_week", "Start of week", "Début de la semaine"),
    ("end_of_week", "End of week", "Fin de la semaine"),
    ("days_in_month", "Days in month", "Jours dans le mois"),
    ("start_of_day", "Start of day", "Début du jour"),
    ("end_of_day", "End of day", "Fin du jour"),
    ("date_only", "Date only", "Date seulement"),
];

pub const TIME_OPS: [Op; 6] = [
    ("time_only", "Time only", "Heure seulement"),
    ("hour", "Hour", "Heure"),
    ("minute", "Minute", "Minute"),
    ("second", "Second", "Seconde"),
    ("start_of_hour", "Start of hour", "Début de l'heure"),
    ("end_of_hour", "End of hour", "Fin de l'heure"),
];

pub const DURATION_OPS: [Op; 9] = [
    ("duration_days", "Days", "Jours"),
    ("duration_hours", "Hours", "Heures"),
    ("duration_minutes", "Minutes", "Minutes"),
    ("duration_seconds", "Seconds", "Secondes"),
    ("total_years", "Total years", "Total des années"),
    ("total_days", "Total days", "Total des jours"),
    ("total_hours", "Total hours", "Total des heures"),
    ("total_minutes", "Total minutes", "Total des minutes"),
    ("total_seconds", "Total seconds", "Total des secondes"),
];

fn f(key: &'static str, en: &'static str, fr: &'static str, kind: FieldKind, value: &str) -> Field {
    Field {
        key,
        en,
        fr,
        kind,
        value: value.to_string(),
    }
}

/// The values column operation `op` asks for.
fn op_fields(op: &str) -> Vec<Field> {
    match op {
        "prefix" => vec![f("text", "Prefix", "Préfixe", FieldKind::Text, "")],
        "suffix" => vec![f("text", "Suffix", "Suffixe", FieldKind::Text, "")],
        "before_delimiter" | "after_delimiter" => {
            vec![f("text", "Delimiter", "Délimiteur", FieldKind::Text, ",")]
        }
        "first_chars" | "last_chars" => vec![f("count", "Count", "Nombre", FieldKind::Count, "1")],
        "text_range" => vec![
            f(
                "start",
                "Starting index (0 is the first)",
                "Index de départ (0 pour le premier)",
                FieldKind::Count,
                "0",
            ),
            f(
                "count",
                "Number of characters",
                "Nombre de caractères",
                FieldKind::Count,
                "1",
            ),
        ],
        "between_delimiters" => vec![
            f(
                "text",
                "Start delimiter",
                "Délimiteur de début",
                FieldKind::Text,
                "(",
            ),
            f("end", "End delimiter", "Délimiteur de fin", FieldKind::Text, ")"),
        ],
        "round" => vec![f("count", "Decimal places", "Décimales", FieldKind::Count, "2")],
        "add" | "subtract" | "multiply" | "divide" | "integer_divide" | "modulo" | "percentage"
        | "percent_of" | "power" => {
            vec![f("number", "Value", "Valeur", FieldKind::Number, "")]
        }
        _ => Vec::new(),
    }
}

/// The step of column operation `op` over the picked columns, or the form asking its values.
fn op_act(e: &Editor, op: Op, add: bool, l: Lang) -> TabAct {
    let columns: Vec<String> = if add {
        e.columns.iter().take(1).cloned().collect()
    } else {
        e.columns.clone()
    };
    // Scientific › Square and Cube: a power with its exponent given
    let power = match op.0 {
        "square" => Some(2),
        "cube" => Some(3),
        _ => None,
    };
    if let Some(n) = power {
        return TabAct::Run(
            json!({"kind": "column_op", "op": "power", "number": n, "columns": columns, "add": add}),
        );
    }
    let step = json!({"kind": "column_op", "op": op.0, "columns": columns, "add": add});
    let fields = op_fields(op.0);
    if fields.is_empty() {
        return TabAct::Run(step);
    }
    TabAct::Form(Form::Params {
        title: l.t(op.1, op.2).trim_end_matches('…').to_string(),
        step,
        fields,
    })
}

fn params(title: &str, step: Value, fields: Vec<Field>) -> TabAct {
    TabAct::Form(Form::Params {
        title: title.to_string(),
        step,
        fields,
    })
}

/// A ribbon button opening a menu; its items push into `acts`.
fn menu(ui: &mut Ui, (icon, label): (Icon, &str), tip: &str, ok: bool, why: &str, add: impl FnOnce(&mut Ui)) {
    let r = widgets::small_button(ui, icon, label, ok, false);
    let r = r.on_hover_text(if ok { tip } else { why });
    if ok {
        egui::Popup::menu(&r).show(add);
    }
}

fn item(ui: &mut Ui, label: &str) -> bool {
    ui.button(label).clicked()
}

/// Items of `ops`, each adding its step.
fn op_items(ui: &mut Ui, e: &Editor, ops: &[Op], add: bool, l: Lang, acts: &mut Vec<TabAct>) {
    for op in ops {
        if item(ui, l.t(op.1, op.2)) {
            acts.push(op_act(e, *op, add, l));
        }
    }
}

/// The Home tab's Keep rows and Remove rows menus.
pub fn reduce_rows(ui: &mut Ui, e: &Editor, ready: bool, l: Lang, acts: &mut Vec<TabAct>) {
    let n = |v: &str| f("count", "Number of rows", "Nombre de lignes", FieldKind::Count, v);
    menu(
        ui,
        (Icon::Table, l.t("Keep rows", "Conserver les lignes")),
        l.t(
            "Keep the top, bottom or a range of rows, or the rows with errors",
            "Conserve les premières, les dernières ou une plage de lignes, ou les lignes en erreur",
        ),
        ready,
        "",
        |ui| {
            if item(ui, l.t("Keep top rows", "Conserver les premières lignes")) {
                acts.push(params(
                    l.t("Keep top rows", "Conserver les premières lignes"),
                    json!({"kind": "keep_top_rows"}),
                    vec![n("10")],
                ));
            }
            if item(ui, l.t("Keep bottom rows", "Conserver les dernières lignes")) {
                acts.push(params(
                    l.t("Keep bottom rows", "Conserver les dernières lignes"),
                    json!({"kind": "keep_bottom_rows"}),
                    vec![n("10")],
                ));
            }
            if item(ui, l.t("Keep range of rows", "Conserver une plage de lignes")) {
                acts.push(params(
                    l.t("Keep range of rows", "Conserver une plage de lignes"),
                    json!({"kind": "keep_range"}),
                    vec![
                        f("first", "First row", "Première ligne", FieldKind::Count, "1"),
                        n("10"),
                    ],
                ));
            }
            if item(ui, l.t("Keep errors", "Conserver les erreurs")) {
                acts.push(TabAct::Run(json!({"kind": "keep_errors", "columns": e.columns})));
            }
        },
    );
    menu(
        ui,
        (Icon::Minus, l.t("Remove rows", "Supprimer les lignes")),
        l.t(
            "Remove the top, bottom, alternate or blank rows, or the rows with errors",
            "Supprime les premières, les dernières, une ligne sur n ou les lignes vides ou en erreur",
        ),
        ready,
        "",
        |ui| {
            if item(ui, l.t("Remove top rows", "Supprimer les premières lignes")) {
                acts.push(params(
                    l.t("Remove top rows", "Supprimer les premières lignes"),
                    json!({"kind": "remove_top_rows"}),
                    vec![n("1")],
                ));
            }
            if item(ui, l.t("Remove bottom rows", "Supprimer les dernières lignes")) {
                acts.push(params(
                    l.t("Remove bottom rows", "Supprimer les dernières lignes"),
                    json!({"kind": "remove_bottom_rows"}),
                    vec![n("1")],
                ));
            }
            if item(ui, l.t("Remove alternate rows", "Supprimer une ligne sur n")) {
                acts.push(params(
                    l.t("Remove alternate rows", "Supprimer une ligne sur n"),
                    json!({"kind": "remove_alternate_rows"}),
                    vec![
                        f(
                            "first",
                            "First row to remove",
                            "Première ligne à supprimer",
                            FieldKind::Count,
                            "1",
                        ),
                        f(
                            "remove",
                            "Rows to remove",
                            "Lignes à supprimer",
                            FieldKind::Count,
                            "1",
                        ),
                        f(
                            "keep",
                            "Rows to keep",
                            "Lignes à conserver",
                            FieldKind::Count,
                            "1",
                        ),
                    ],
                ));
            }
            if item(ui, l.t("Remove blank rows", "Supprimer les lignes vides")) {
                acts.push(TabAct::Run(json!({"kind": "remove_blank_rows"})));
            }
            if item(ui, l.t("Remove errors", "Supprimer les erreurs")) {
                acts.push(TabAct::Run(
                    json!({"kind": "remove_errors", "columns": e.columns}),
                ));
            }
        },
    );
}

/// The Transform tab.
pub fn transform_ribbon(ui: &mut Ui, e: &Editor, l: Lang, acts: &mut Vec<TabAct>) {
    let ready = !e.busy && e.step.is_some() && e.advanced.is_none();
    let col_ok = ready && !e.columns.is_empty();
    let pick1 = l.t(
        "Click a column header first.",
        "Cliquez d'abord sur un en-tête de colonne.",
    );
    let pick2 = l.t(
        "Click two column headers or more first (Ctrl+click).",
        "Cliquez d'abord sur au moins deux en-têtes de colonne (Ctrl+clic).",
    );
    ui.horizontal(|ui| {
        ui.set_height(widgets::RIBBON_H);
        // Table
        widgets::group(ui, |ui| {
            widgets::rows(ui, |ui| {
                if widgets::small_button(ui, Icon::Move, l.t("Transpose", "Transposer"), ready, false)
                    .on_hover_text(l.t(
                        "Rows become columns and columns rows",
                        "Les lignes deviennent des colonnes et inversement",
                    ))
                    .clicked()
                    && ready
                {
                    acts.push(TabAct::Run(json!({"kind": "transpose"})));
                }
                if widgets::small_button(
                    ui,
                    Icon::Sort,
                    l.t("Reverse rows", "Inverser les lignes"),
                    ready,
                    false,
                )
                .on_hover_text(l.t("The last row first", "La dernière ligne en premier"))
                .clicked()
                    && ready
                {
                    acts.push(TabAct::Run(json!({"kind": "reverse_rows"})));
                }
                if widgets::small_button(
                    ui,
                    Icon::Table,
                    l.t("Use headers as first row", "En-têtes en première ligne"),
                    ready,
                    false,
                )
                .on_hover_text(l.t(
                    "The column names become the first row",
                    "Les noms de colonnes deviennent la première ligne",
                ))
                .clicked()
                    && ready
                {
                    acts.push(TabAct::Run(json!({"kind": "demote_headers"})));
                }
            });
        });
        // Any column
        widgets::group(ui, |ui| {
            widgets::rows(ui, |ui| {
                menu(
                    ui,
                    (Icon::ChevronDown, l.t("Fill", "Remplissage")),
                    l.t(
                        "Fill the empty cells with the value above or below",
                        "Remplit les cellules vides avec la valeur au-dessus ou en dessous",
                    ),
                    col_ok,
                    pick1,
                    |ui| {
                        if item(ui, l.t("Down", "Vers le bas")) {
                            acts.push(TabAct::Run(json!({"kind": "fill_down", "columns": e.columns})));
                        }
                        if item(ui, l.t("Up", "Vers le haut")) {
                            acts.push(TabAct::Run(json!({"kind": "fill_up", "columns": e.columns})));
                        }
                    },
                );
                if widgets::small_button(
                    ui,
                    Icon::Warning,
                    l.t("Replace errors", "Remplacer les erreurs"),
                    col_ok,
                    false,
                )
                .on_hover_text(if col_ok {
                    l.t(
                        "Replace the error values of the columns picked",
                        "Remplace les valeurs d'erreur des colonnes choisies",
                    )
                } else {
                    pick1
                })
                .clicked()
                    && col_ok
                {
                    acts.push(params(
                        l.t("Replace errors", "Remplacer les erreurs"),
                        json!({"kind": "replace_errors", "columns": e.columns}),
                        vec![f("value", "Value", "Valeur", FieldKind::Value, "")],
                    ));
                }
                menu(
                    ui,
                    (Icon::Move, l.t("Move", "Déplacer")),
                    l.t("Move the columns picked", "Déplace les colonnes choisies"),
                    col_ok,
                    pick1,
                    |ui| {
                        if item(ui, l.t("To beginning", "Au début")) {
                            acts.push(TabAct::Run(move_step(e, true)));
                        }
                        if item(ui, l.t("To end", "À la fin")) {
                            acts.push(TabAct::Run(move_step(e, false)));
                        }
                    },
                );
            });
        });
        text_number_date(ui, e, false, l, acts, (col_ok, pick1, pick2));
    });
}

/// Move: every preview column, the picked ones first (`first`) or last.
pub fn move_step(e: &Editor, first: bool) -> Value {
    let all: Vec<String> = match &e.preview {
        Some(Ok(p)) => p.columns.iter().map(|(n, _)| n.clone()).collect(),
        _ => Vec::new(),
    };
    let rest = all.iter().filter(|c| !e.columns.contains(c)).cloned();
    let order: Vec<String> = if first {
        e.columns.iter().cloned().chain(rest).collect()
    } else {
        rest.chain(e.columns.iter().cloned()).collect()
    };
    json!({"kind": "reorder_columns", "columns": order})
}

/// The Text, Number and Date & Time groups, shared by the Transform (in place) and Add column
/// (`add`) tabs.
fn text_number_date(
    ui: &mut Ui,
    e: &Editor,
    add: bool,
    l: Lang,
    acts: &mut Vec<TabAct>,
    (col_ok, pick1, pick2): (bool, &str, &str),
) {
    let two = col_ok && e.columns.len() >= 2;
    widgets::group(ui, |ui| {
        widgets::rows(ui, |ui| {
            menu(
                ui,
                (Icon::Format, l.t("Format", "Format")),
                l.t(
                    "Change the case, trim or clean the text, add a prefix or suffix",
                    "Change la casse, supprime les espaces, nettoie ou ajoute un préfixe ou un suffixe",
                ),
                col_ok,
                pick1,
                |ui| op_items(ui, e, &FORMAT_OPS, add, l, acts),
            );
            menu(
                ui,
                (Icon::Cut, l.t("Extract", "Extraire")),
                l.t(
                    "The length or part of the text",
                    "La longueur ou une partie du texte",
                ),
                col_ok,
                pick1,
                |ui| op_items(ui, e, &EXTRACT_OPS, add, l, acts),
            );
            if widgets::small_button(
                ui,
                Icon::Link,
                l.t("Merge columns", "Fusionner les colonnes"),
                two,
                false,
            )
            .on_hover_text(if two {
                l.t(
                    "Join the texts of the columns picked, in click order",
                    "Joint les textes des colonnes choisies, dans l'ordre des clics",
                )
            } else {
                pick2
            })
            .clicked()
                && two
            {
                acts.push(params(
                    l.t("Merge columns", "Fusionner les colonnes"),
                    json!({"kind": "merge_columns", "columns": e.columns, "add": add}),
                    vec![
                        f("delimiter", "Separator", "Séparateur", FieldKind::Text, " "),
                        f(
                            "name",
                            "New column name",
                            "Nom de la nouvelle colonne",
                            FieldKind::Text,
                            "Merged",
                        ),
                    ],
                ));
            }
        });
    });
    widgets::group(ui, |ui| {
        widgets::rows(ui, |ui| {
            menu(
                ui,
                (Icon::Calculator, l.t("Standard", "Standard")),
                l.t(
                    "Add, subtract, multiply, divide, integer-divide or take the remainder by a value; a percentage",
                    "Ajoute, soustrait, multiplie, divise, divise en entier ou prend le reste par une valeur ; un pourcentage",
                ),
                col_ok,
                pick1,
                |ui| op_items(ui, e, &STANDARD_OPS, add, l, acts),
            );
            menu(
                ui,
                (Icon::Calculator, l.t("Scientific", "Scientifique")),
                l.t(
                    "Absolute value, powers, square root, exponent, logarithms, factorial",
                    "Valeur absolue, puissances, racine carrée, exponentielle, logarithmes, factorielle",
                ),
                col_ok,
                pick1,
                |ui| op_items(ui, e, &SCIENTIFIC_OPS, add, l, acts),
            );
            menu(
                ui,
                (Icon::Calculator, l.t("Trigonometry", "Trigonométrie")),
                l.t(
                    "Sine, cosine, tangent and their inverses, in radians",
                    "Sinus, cosinus, tangente et leurs réciproques, en radians",
                ),
                col_ok,
                pick1,
                |ui| op_items(ui, e, &TRIG_OPS, add, l, acts),
            );
        });
    });
    widgets::group(ui, |ui| {
        widgets::rows(ui, |ui| {
            menu(
                ui,
                (Icon::Calculator, l.t("Rounding", "Arrondi")),
                l.t("Round the numbers", "Arrondit les nombres"),
                col_ok,
                pick1,
                |ui| op_items(ui, e, &ROUNDING_OPS, add, l, acts),
            );
            menu(
                ui,
                (Icon::Info, l.t("Information", "Information")),
                l.t(
                    "Whether the number is even or odd, its sign",
                    "Si le nombre est pair ou impair, son signe",
                ),
                col_ok,
                pick1,
                |ui| op_items(ui, e, &INFO_OPS, add, l, acts),
            );
        });
    });
    widgets::group(ui, |ui| {
        widgets::rows(ui, |ui| {
            menu(
                ui,
                (Icon::Calendar, l.t("Date", "Date")),
                l.t(
                    "A part of the date: year, month, day, start or end of month…",
                    "Une partie de la date : année, mois, jour, début ou fin du mois…",
                ),
                col_ok,
                pick1,
                |ui| op_items(ui, e, &DATE_OPS, add, l, acts),
            );
            menu(
                ui,
                (Icon::Calendar, l.t("Time", "Heure")),
                l.t("A part of the time", "Une partie de l'heure"),
                col_ok,
                pick1,
                |ui| op_items(ui, e, &TIME_OPS, add, l, acts),
            );
            menu(
                ui,
                (Icon::Calendar, l.t("Duration", "Durée")),
                l.t(
                    "A part of the duration, or its total in years, days, hours, minutes or seconds",
                    "Une partie de la durée, ou son total en années, jours, heures, minutes ou secondes",
                ),
                col_ok,
                pick1,
                |ui| op_items(ui, e, &DURATION_OPS, add, l, acts),
            );
        });
    });
}

/// The Add column tab.
pub fn add_column_ribbon(ui: &mut Ui, e: &Editor, l: Lang, acts: &mut Vec<TabAct>) {
    let ready = !e.busy && e.step.is_some() && e.advanced.is_none();
    let col_ok = ready && !e.columns.is_empty();
    let pick1 = l.t(
        "Click a column header first.",
        "Cliquez d'abord sur un en-tête de colonne.",
    );
    let pick2 = l.t(
        "Click two column headers or more first (Ctrl+click).",
        "Cliquez d'abord sur au moins deux en-têtes de colonne (Ctrl+clic).",
    );
    let first = e.columns.first().cloned().unwrap_or_default();
    ui.horizontal(|ui| {
        ui.set_height(widgets::RIBBON_H);
        // General
        widgets::group(ui, |ui| {
            if widgets::big_button(
                ui,
                Icon::Calculator,
                l.t("Custom\ncolumn", "Colonne\npersonnalisée"),
                ready,
            )
            .clicked()
                && ready
            {
                acts.push(TabAct::Form(Form::Custom {
                    name: l.t("Custom", "Personnalisée").into(),
                    formula: String::new(),
                }));
            }
            widgets::rows(ui, |ui| {
                if widgets::small_button(
                    ui,
                    Icon::Hierarchy,
                    l.t("Conditional column", "Colonne conditionnelle"),
                    ready,
                    false,
                )
                .on_hover_text(l.t(
                    "A value chosen by if … then … else rules",
                    "Une valeur choisie par des règles si … alors … sinon",
                ))
                .clicked()
                    && ready
                {
                    acts.push(TabAct::Form(Form::Conditional {
                        name: l.t("Custom", "Personnalisée").into(),
                        rules: vec![CondRule {
                            column: first.clone(),
                            op: 0,
                            value: String::new(),
                            output: String::new(),
                        }],
                        otherwise: String::new(),
                    }));
                }
                menu(
                    ui,
                    (Icon::List, l.t("Index column", "Colonne d'index")),
                    l.t("Number the rows", "Numérote les lignes"),
                    ready,
                    "",
                    |ui| {
                        if item(ui, l.t("From 0", "À partir de 0")) {
                            acts.push(TabAct::Run(json!({"kind": "add_index", "start": 0})));
                        }
                        if item(ui, l.t("From 1", "À partir de 1")) {
                            acts.push(TabAct::Run(json!({"kind": "add_index", "start": 1})));
                        }
                        if item(ui, l.t("Custom…", "Personnalisé…")) {
                            acts.push(params(
                                l.t("Index column", "Colonne d'index"),
                                json!({"kind": "add_index"}),
                                vec![
                                    f(
                                        "name",
                                        "New column name",
                                        "Nom de la nouvelle colonne",
                                        FieldKind::Text,
                                        "Index",
                                    ),
                                    f("start", "Starting index", "Index de départ", FieldKind::Int, "0"),
                                    f("step", "Increment", "Incrément", FieldKind::Int, "1"),
                                ],
                            ));
                        }
                    },
                );
                if widgets::small_button(
                    ui,
                    Icon::Duplicate,
                    l.t("Duplicate column", "Dupliquer la colonne"),
                    col_ok,
                    false,
                )
                .on_hover_text(if col_ok {
                    l.t(
                        "A copy of the first column picked",
                        "Une copie de la première colonne choisie",
                    )
                } else {
                    pick1
                })
                .clicked()
                    && col_ok
                {
                    acts.push(TabAct::Run(json!({"kind": "duplicate_column", "column": first})));
                }
            });
        });
        text_number_date(ui, e, true, l, acts, (col_ok, pick1, pick2));
    });
}

/// A settings value as a form shows it (none for a date, which the forms do not type).
fn shown(v: Option<&Value>) -> Option<String> {
    Some(match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(t)) => t.clone(),
        Some(Value::Number(n)) => match n.as_f64() {
            // a whole number as typed (`2`, not `2.0`)
            Some(x) if x.fract() == 0.0 && x.abs() < 1e15 => format!("{}", x as i64),
            _ => n.to_string(),
        },
        Some(Value::Bool(b)) => b.to_string(),
        Some(_) => return None,
    })
}

fn names(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(str::to_string))
        .collect()
}

fn index_of(table: &[(&str, &str, &str)], name: Option<&str>) -> Option<usize> {
    let name = name?;
    table.iter().position(|x| x.0 == name)
}

/// A form's fields with the values of `settings`.
fn filled(mut fields: Vec<Field>, settings: &Value) -> Option<Vec<Field>> {
    for x in &mut fields {
        if let Some(v) = settings.get(x.key) {
            x.value = shown(Some(v))?;
        }
    }
    Some(fields)
}

/// The step settings gear: the form of `settings` (`query.steps`' `{kind, …}`), filled with its
/// values, and the columns it reads (the window's picked columns while it is open). None when no
/// form asks these settings.
pub fn settings_form(v: &Value, l: Lang) -> Option<(Form, Vec<String>)> {
    use crate::pq_editor::{AGGS, JOINS, OPS};
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let kind = v.get("kind")?.as_str()?;
    let title = |en: &'static str, fr: &'static str| l.t(en, fr).to_string();
    let count = |key: &'static str, en: &'static str, fr: &'static str| f(key, en, fr, FieldKind::Count, "");
    let rows = || count("count", "Number of rows", "Nombre de lignes");
    let p = |t: String, fields: Vec<Field>| -> Option<(Form, Vec<String>)> {
        Some((
            Form::Params {
                title: t,
                step: v.clone(),
                fields: filled(fields, v)?,
            },
            names(v, "columns"),
        ))
    };
    match kind {
        "filter_rows" => Some((
            Form::Filter {
                column: s("column"),
                op: index_of(&OPS, v.get("op").and_then(Value::as_str))?,
                value: shown(v.get("value"))?,
            },
            vec![s("column")],
        )),
        "replace_values" => Some((
            Form::Replace {
                find: shown(v.get("find"))?,
                replace: shown(v.get("replace"))?,
            },
            names(v, "columns"),
        )),
        "group_by" => {
            let [a] = v.get("aggregations")?.as_array()?.as_slice() else {
                return None;
            };
            Some((
                Form::Group {
                    name: a.get("name")?.as_str()?.to_string(),
                    op: index_of(&AGGS, a.get("op").and_then(Value::as_str))?,
                    column: a
                        .get("column")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                },
                names(v, "keys"),
            ))
        }
        "merge" => {
            let (keys, other_keys) = (names(v, "keys"), names(v, "other_keys"));
            let ([key], [other_key]) = (keys.as_slice(), other_keys.as_slice()) else {
                return None;
            };
            Some((
                Form::Merge {
                    other: s("other"),
                    key: key.clone(),
                    other_key: other_key.clone(),
                    join: index_of(&JOINS, v.get("join").and_then(Value::as_str))?,
                },
                vec![key.clone()],
            ))
        }
        "split_column" => Some((
            Form::Split {
                delimiter: s("delimiter"),
                parts: shown(v.get("parts"))?,
            },
            vec![s("column")],
        )),
        "conditional_column" => {
            let mut rules = Vec::new();
            for r in v.get("rules")?.as_array()? {
                rules.push(CondRule {
                    column: r.get("column")?.as_str()?.to_string(),
                    op: index_of(&OPS, r.get("op").and_then(Value::as_str))?,
                    value: shown(r.get("value"))?,
                    output: shown(r.get("output"))?,
                });
            }
            Some((
                Form::Conditional {
                    name: s("name"),
                    rules,
                    otherwise: shown(v.get("otherwise"))?,
                },
                Vec::new(),
            ))
        }
        "remove_top_rows" => p(
            title("Remove top rows", "Supprimer les premières lignes"),
            vec![rows()],
        ),
        "remove_bottom_rows" => p(
            title("Remove bottom rows", "Supprimer les dernières lignes"),
            vec![rows()],
        ),
        "keep_top_rows" => p(
            title("Keep top rows", "Conserver les premières lignes"),
            vec![rows()],
        ),
        "keep_bottom_rows" => p(
            title("Keep bottom rows", "Conserver les dernières lignes"),
            vec![rows()],
        ),
        "keep_range" => p(
            title("Keep range of rows", "Conserver une plage de lignes"),
            vec![count("first", "First row", "Première ligne"), rows()],
        ),
        "remove_alternate_rows" => p(
            title("Remove alternate rows", "Supprimer une ligne sur n"),
            vec![
                count("first", "First row to remove", "Première ligne à supprimer"),
                count("remove", "Rows to remove", "Lignes à supprimer"),
                count("keep", "Rows to keep", "Lignes à conserver"),
            ],
        ),
        "replace_errors" => p(
            title("Replace errors", "Remplacer les erreurs"),
            vec![f("value", "Value", "Valeur", FieldKind::Value, "")],
        ),
        "add_index" => p(
            title("Index column", "Colonne d'index"),
            vec![
                f(
                    "name",
                    "New column name",
                    "Nom de la nouvelle colonne",
                    FieldKind::Text,
                    "",
                ),
                f("start", "Starting index", "Index de départ", FieldKind::Int, ""),
                f("step", "Increment", "Incrément", FieldKind::Int, ""),
            ],
        ),
        "merge_columns" => p(
            title("Merge columns", "Fusionner les colonnes"),
            vec![
                f("delimiter", "Separator", "Séparateur", FieldKind::Text, ""),
                f(
                    "name",
                    "New column name",
                    "Nom de la nouvelle colonne",
                    FieldKind::Text,
                    "",
                ),
            ],
        ),
        "column_op" => {
            let op = v.get("op")?.as_str()?;
            let fields = op_fields(op);
            if fields.is_empty() {
                return None;
            }
            let all = FORMAT_OPS
                .iter()
                .chain(&EXTRACT_OPS)
                .chain(&STANDARD_OPS)
                .chain(&SCIENTIFIC_OPS)
                .chain(&ROUNDING_OPS);
            let (_, en, fr) = all.copied().find(|x| x.0 == op)?;
            p(l.t(en, fr).trim_end_matches('…').to_string(), fields)
        }
        _ => None,
    }
}
