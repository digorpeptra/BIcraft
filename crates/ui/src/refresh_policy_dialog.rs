//! Incremental refresh and real-time data (a table's menu › Incremental refresh, in the Data pane
//! and on the model diagram), as Desktop's dialog: the table, the switch "Incrementally refresh
//! this table" (off, with the reason, until the model has the `RangeStart` / `RangeEnd`
//! parameters and the table's query filters on them), the archive and refresh periods, the
//! optional settings (real time with DirectQuery, only complete periods, detect data changes),
//! and the review sentences. Apply runs `table.set_refresh_policy`, or
//! `table.remove_refresh_policy` when the switch is turned off. The dialog reads the snapshot's
//! `refresh_policy`, `range_filtered` and `model.incremental_refresh` (docs/view-snapshot.md).

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::Then;
use crate::i18n::Lang;
use crate::theme::{self, Tokens};
use crate::widgets;

pub const COMMAND: &str = "table.set_refresh_policy";
pub const REMOVE: &str = "table.remove_refresh_policy";
/// The units, as the command spells them.
pub const UNITS: &[&str] = &["day", "month", "quarter", "year"];
/// Most columns the change detection list offers.
const MAX_COLUMNS: usize = 500;

/// The dialog while it is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dialog {
    pub table: String,
    /// The switch: incrementally refresh this table.
    pub on: bool,
    pub store_n: String,
    pub store_unit: &'static str,
    pub refresh_n: String,
    pub refresh_unit: &'static str,
    pub realtime: bool,
    pub complete: bool,
    pub detect: bool,
    pub detect_column: Option<String>,
    /// Whether the table had a policy when the dialog opened.
    pub had_policy: bool,
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn unit_of(v: &str) -> &'static str {
    UNITS.iter().copied().find(|u| *u == v).unwrap_or("day")
}

fn table_snap<'a>(app: &'a App, table: &str) -> Option<&'a Value> {
    app.loaded
        .as_ref()?
        .snap
        .pointer("/model/tables")?
        .as_array()?
        .iter()
        .find(|t| s(t, "name") == table)
}

/// A unit's word for `n` periods.
pub fn unit_text(unit: &str, n: u64, l: Lang) -> &'static str {
    let one = n == 1;
    match (unit, one) {
        ("year", true) => l.t("year", "année"),
        ("year", false) => l.t("years", "années"),
        ("quarter", true) => l.t("quarter", "trimestre"),
        ("quarter", false) => l.t("quarters", "trimestres"),
        ("month", true) => l.t("month", "mois"),
        ("month", false) => l.t("months", "mois"),
        (_, true) => l.t("day", "jour"),
        (_, false) => l.t("days", "jours"),
    }
}

/// The list's word for a unit (capitalized, plural, as the dialog's lists show it).
fn unit_choice(unit: &str, l: Lang) -> &'static str {
    match unit {
        "year" => l.t("Years", "Années"),
        "quarter" => l.t("Quarters", "Trimestres"),
        "month" => l.t("Months", "Mois"),
        _ => l.t("Days", "Jours"),
    }
}

/// The review sentences for the settings shown.
pub fn summary(d: &Dialog, l: Lang) -> Vec<String> {
    let n = |t: &str| t.trim().parse::<u64>().ok().filter(|n| *n >= 1);
    let mut out = Vec::new();
    if let Some(sn) = n(&d.store_n) {
        out.push(format!(
            "{} {sn} {} {}",
            l.t("Archive data starting", "Archiver les données à partir de"),
            unit_text(d.store_unit, sn, l),
            l.t("before refresh date", "avant la date d'actualisation"),
        ));
    }
    if let Some(rn) = n(&d.refresh_n) {
        out.push(format!(
            "{} {rn} {} {}",
            l.t(
                "Incrementally refresh data starting",
                "Actualiser les données de manière incrémentielle à partir de"
            ),
            unit_text(d.refresh_unit, rn, l),
            l.t("before refresh date", "avant la date d'actualisation"),
        ));
    }
    if d.realtime {
        out.push(
            l.t(
                "Get the latest data in real time with DirectQuery",
                "Obtenir les dernières données en temps réel avec DirectQuery",
            )
            .to_string(),
        );
    }
    out
}

/// Why the switch is off (`None`: the table can have a policy), from the snapshot.
pub fn not_ready(app: &App, table: &str, l: Lang) -> Option<String> {
    let snap = &app.loaded.as_ref()?.snap;
    let ir = snap.pointer("/model/incremental_refresh");
    let params = ir
        .and_then(|v| v.get("parameters"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !params {
        return Some(
            l.t(
                "Create the Date/Time parameters RangeStart and RangeEnd first (Power Query editor › Manage parameters).",
                "Créez d'abord les paramètres Date/Heure RangeStart et RangeEnd (éditeur Power Query › Gérer les paramètres).",
            )
            .to_string(),
        );
    }
    let t = table_snap(app, table)?;
    let mode = crate::storage_ui::table_mode(t);
    if !(mode == "import" || (mode == "hybrid" && t.get("refresh_policy").is_some())) {
        return Some(
            l.t(
                "Incremental refresh applies to Import tables: set the table's storage mode to Import.",
                "L'actualisation incrémentielle s'applique aux tables en importation : passez la table en mode Importation.",
            )
            .to_string(),
        );
    }
    if t.get("range_filtered").and_then(Value::as_bool) != Some(true) {
        return Some(
            l.t(
                "The table's query must filter its rows on RangeStart and RangeEnd (a Filter rows step).",
                "La requête de la table doit filtrer ses lignes sur RangeStart et RangeEnd (une étape Filtrer les lignes).",
            )
            .to_string(),
        );
    }
    None
}

/// The table's menu › Incremental refresh (no periods given) opens the dialog.
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    if id != COMMAND || params.get("store").is_some() {
        return false;
    }
    let Some(table) = params.get("table").and_then(Value::as_str) else {
        return false;
    };
    open(app, table);
    true
}

/// Opens the dialog on `table`, filled from its policy (else Desktop's usual start: 5 years
/// archived, 10 days refreshed).
pub fn open(app: &mut App, table: &str) {
    let policy = table_snap(app, table)
        .and_then(|t| t.get("refresh_policy"))
        .cloned();
    let p = policy.clone().unwrap_or(Value::Null);
    let n = |k: &str, dflt: u64| {
        p.pointer(&format!("/{k}/n"))
            .and_then(Value::as_u64)
            .unwrap_or(dflt)
            .to_string()
    };
    let unit = |k: &str, dflt: &'static str| {
        p.pointer(&format!("/{k}/unit"))
            .and_then(Value::as_str)
            .map_or(dflt, unit_of)
    };
    let detect_column = p
        .get("detect_changes")
        .and_then(Value::as_str)
        .map(str::to_string);
    app.cmd.refresh_policy = Some(Dialog {
        table: table.to_string(),
        on: policy.is_some(),
        store_n: n("store", 5),
        store_unit: unit("store", "year"),
        refresh_n: n("refresh", 10),
        refresh_unit: unit("refresh", "day"),
        realtime: p.get("realtime").and_then(Value::as_bool).unwrap_or(false),
        complete: p
            .get("only_complete_periods")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        detect: detect_column.is_some(),
        detect_column,
        had_policy: policy.is_some(),
    });
}

/// The call Apply sends (`None`: nothing to apply, or a period is not a number).
pub fn apply_call(d: &Dialog) -> Option<Call> {
    if !d.on {
        return d.had_policy.then(|| Call {
            id: REMOVE,
            params: json!({"table": d.table}),
        });
    }
    let n = |t: &str| t.trim().parse::<u64>().ok().filter(|n| *n >= 1);
    let (sn, rn) = (n(&d.store_n)?, n(&d.refresh_n)?);
    let mut p = json!({
        "table": d.table,
        "store": {"n": sn, "unit": d.store_unit},
        "refresh": {"n": rn, "unit": d.refresh_unit},
    });
    if let Value::Object(o) = &mut p {
        if d.realtime {
            o.insert("realtime".into(), json!(true));
        }
        if d.complete || d.realtime {
            o.insert("only_complete_periods".into(), json!(true));
        }
        if d.detect
            && let Some(c) = &d.detect_column
        {
            o.insert("detect_changes".into(), json!(c));
        }
    }
    Some(Call {
        id: COMMAND,
        params: p,
    })
}

/// Draws the dialog when it is open.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut d) = app.cmd.refresh_policy.clone() else {
        return;
    };
    let l = app.lang;
    let blocked = not_ready(app, &d.table, l);
    let date_columns: Vec<String> = table_snap(app, &d.table)
        .and_then(|t| t.get("columns"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|c| s(c, "data_type").eq_ignore_ascii_case("dateTime"))
        .filter(|c| c.get("calculated").and_then(Value::as_bool) != Some(true))
        .take(MAX_COLUMNS)
        .map(|c| s(c, "name").to_string())
        .collect();
    let mut open = true;
    let mut close = false;
    let mut apply = false;
    egui::Window::new(l.t(
        "Incremental refresh and real-time data",
        "Actualisation incrémentielle et données en temps réel",
    ))
    .id(egui::Id::new("bicraft-incremental-refresh"))
    .collapsible(false)
    .resizable(false)
    .open(&mut open)
    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
    .show(ctx, |ui| {
        ui.set_width(600.0);
        body(
            ui,
            &mut d,
            blocked.as_deref(),
            &date_columns,
            l,
            &mut apply,
            &mut close,
        );
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    if apply {
        if let Some(c) = apply_call(&d) {
            app.run(c, Then::Nothing);
        }
        close = true;
    }
    app.cmd.refresh_policy = (open && !close).then_some(d);
}

fn heading(ui: &mut Ui, n: u8, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(format!("{n}. {text}")).font(theme::bold(13.0)));
}

fn units(ui: &mut Ui, id: &str, label: &str, unit: &mut &'static str, enabled: bool, l: Lang) {
    ui.add_enabled_ui(enabled, |ui| {
        let r = egui::ComboBox::from_id_salt(("bicraft-ir-unit", id))
            .selected_text(unit_choice(unit, l))
            .width(110.0)
            .show_ui(ui, |ui| {
                for u in UNITS {
                    if ui.selectable_label(*unit == *u, unit_choice(u, l)).clicked() {
                        *unit = u;
                    }
                }
            });
        r.response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, enabled, label));
    });
}

fn number(ui: &mut Ui, text: &mut String, label: &str, enabled: bool) {
    let r = ui.add_enabled(
        enabled,
        egui::TextEdit::singleline(text).char_limit(6).desired_width(56.0),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, enabled, label));
}

#[allow(clippy::too_many_arguments)]
fn body(
    ui: &mut Ui,
    d: &mut Dialog,
    blocked: Option<&str>,
    date_columns: &[String],
    l: Lang,
    apply: &mut bool,
    close: &mut bool,
) {
    let t = Tokens::get(ui.ctx());
    ui.label(
        RichText::new(l.t(
            "The policy applies when the Power BI service refreshes the published model; BIcraft and Power BI Desktop keep the table's data as it is.",
            "La stratégie s'applique quand le service Power BI actualise le modèle publié ; BIcraft et Power BI Desktop gardent les données de la table telles quelles.",
        ))
        .color(t.text_dim),
    );
    heading(ui, 1, l.t("Select table", "Sélectionner la table"));
    ui.label(RichText::new(&d.table).strong());
    heading(
        ui,
        2,
        l.t(
            "Set import and refresh ranges",
            "Définir les plages d'importation et d'actualisation",
        ),
    );
    let can = blocked.is_none();
    let label = l.t(
        "Incrementally refresh this table",
        "Actualiser cette table de manière incrémentielle",
    );
    ui.horizontal(|ui| {
        if widgets::switch(ui, d.on && can, can, label).clicked() && can {
            d.on = !d.on;
        }
        ui.add_enabled(can, egui::Label::new(label));
    });
    if let Some(why) = blocked {
        ui.horizontal_wrapped(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
            crate::icons::paint(ui.painter(), r, crate::icons::Icon::Warning, t.text, false);
            ui.label(RichText::new(why).color(t.text));
        });
    }
    let on = d.on && can;
    for (key, text, n, unit) in [
        (
            "store",
            l.t("Archive data starting", "Archiver les données à partir de"),
            &mut d.store_n,
            &mut d.store_unit,
        ),
        (
            "refresh",
            l.t(
                "Incrementally refresh data starting",
                "Actualiser les données de manière incrémentielle à partir de",
            ),
            &mut d.refresh_n,
            &mut d.refresh_unit,
        ),
    ] {
        ui.horizontal(|ui| {
            ui.add_enabled(on, egui::Label::new(text));
            number(ui, n, &format!("{text} ({})", l.t("number", "nombre")), on);
            units(
                ui,
                key,
                &format!("{text} ({})", l.t("unit", "unité")),
                unit,
                on,
                l,
            );
            ui.add_enabled(
                on,
                egui::Label::new(l.t("before refresh date", "avant la date d'actualisation")),
            );
        });
    }
    heading(
        ui,
        3,
        l.t("Choose optional settings", "Choisir les paramètres facultatifs"),
    );
    ui.add_enabled_ui(on, |ui| {
        ui.checkbox(
            &mut d.realtime,
            l.t(
                "Get the latest data in real time with DirectQuery (Premium only)",
                "Obtenir les dernières données en temps réel avec DirectQuery (Premium uniquement)",
            ),
        );
        let mut complete = d.complete || d.realtime;
        ui.add_enabled_ui(!d.realtime, |ui| {
            ui.checkbox(
                &mut complete,
                l.t(
                    "Only refresh complete periods",
                    "Actualiser uniquement les périodes complètes",
                ),
            );
        });
        if !d.realtime {
            d.complete = complete;
        }
        ui.horizontal(|ui| {
            let enabled = !date_columns.is_empty();
            ui.add_enabled_ui(enabled, |ui| {
                ui.checkbox(
                    &mut d.detect,
                    l.t("Detect data changes", "Détecter les modifications de données"),
                );
            });
            if !enabled {
                ui.label(
                    RichText::new(l.t(
                        "(the table has no date/time column)",
                        "(la table n'a pas de colonne date/heure)",
                    ))
                    .color(t.text_dim),
                );
            }
            if d.detect && enabled {
                if d.detect_column.is_none() {
                    d.detect_column = date_columns.first().cloned();
                }
                let shown = d.detect_column.clone().unwrap_or_default();
                let label = l.t(
                    "Column to detect data changes",
                    "Colonne de détection des modifications",
                );
                let r = egui::ComboBox::from_id_salt("bicraft-ir-detect")
                    .selected_text(crate::app::clip(&shown, 40))
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        for c in date_columns {
                            if ui
                                .selectable_label(
                                    d.detect_column.as_deref() == Some(c),
                                    crate::app::clip(c, 40),
                                )
                                .clicked()
                            {
                                d.detect_column = Some(c.clone());
                            }
                        }
                    });
                r.response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, label));
            }
        });
    });
    heading(ui, 4, l.t("Review and apply", "Vérifier et appliquer"));
    if on {
        egui::Frame::new()
            .stroke(egui::Stroke::new(1.0, t.border))
            .corner_radius(egui::CornerRadius::same(4))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.set_min_width(560.0);
                for line in summary(d, l) {
                    ui.label(line);
                }
            });
    } else {
        ui.label(
            RichText::new(if d.had_policy {
                l.t(
                    "Apply removes the table's incremental refresh policy.",
                    "Appliquer supprime la stratégie d'actualisation incrémentielle de la table.",
                )
            } else {
                l.t(
                    "Incremental refresh is off for this table.",
                    "L'actualisation incrémentielle est désactivée pour cette table.",
                )
            })
            .color(t.text_dim),
        );
    }
    ui.add_space(10.0);
    let ready = apply_call(d).is_some();
    let w = ui.available_width();
    ui.allocate_ui_with_layout(
        egui::vec2(w, 28.0),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            let reason = l.t(
                "Nothing to apply, or a period is not a whole number of at least 1",
                "Rien à appliquer, ou une période n'est pas un nombre entier d'au moins 1",
            );
            if widgets::primary(ui, l.t("Apply", "Appliquer"), ready, reason).clicked() {
                *apply = true;
            }
            if ui
                .add(egui::Button::new(l.t("Cancel", "Annuler")).min_size(egui::vec2(72.0, 24.0)))
                .clicked()
            {
                *close = true;
            }
        },
    );
}
