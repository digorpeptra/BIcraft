//! The Format pane's General › Data format card (Power BI Desktop 2.159): a visual's own format
//! for one of its fields ("Apply settings to"), written by `visual.set_data_format` as the
//! projection's visual-scoped format string. The choices are BIcraft's: Auto (the model's
//! format), General, Whole number, Decimal number, Percentage, Currency, Scientific, Date and
//! Custom, with the decimals, thousands separator, currency symbol and date format they take.
//! What is being chosen is pure view state (egui memory); Apply runs the command.

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::i18n::Lang;
use crate::theme::Tokens;

/// The format types (code for `visual.set_data_format {type}`, EN, FR).
pub const TYPES: &[(&str, &str, &str)] = &[
    ("auto", "Auto (the model's format)", "Auto (le format du modèle)"),
    ("general", "General", "Général"),
    ("whole", "Whole number", "Nombre entier"),
    ("decimal", "Decimal number", "Nombre décimal"),
    ("percentage", "Percentage", "Pourcentage"),
    ("currency", "Currency", "Devise"),
    ("scientific", "Scientific", "Scientifique"),
    ("date", "Date", "Date"),
    ("custom", "Custom", "Personnalisé"),
];

/// The currency symbols offered (the renderer draws each of them).
pub const SYMBOLS: &[&str] = &["$", "€", "£", "¥", "₹", "CHF"];

/// Date formats offered (format string, EN, FR).
pub const DATES: &[(&str, &str, &str)] = &[
    ("Short Date", "Short date", "Date courte"),
    ("Long Date", "Long date", "Date longue"),
    ("General Date", "Date and time", "Date et heure"),
    ("yyyy-MM-dd", "yyyy-MM-dd", "aaaa-MM-jj"),
    ("dd/MM/yyyy", "dd/MM/yyyy", "jj/MM/aaaa"),
    ("MM/dd/yyyy", "MM/dd/yyyy", "MM/jj/aaaa"),
    ("MMMM yyyy", "Month and year", "Mois et année"),
];

/// The card's choices for one field.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    /// The field it was read from (`role#index`), to start again on another one.
    pub of: String,
    pub kind: &'static str,
    pub decimals: u32,
    pub thousands: bool,
    pub symbol: String,
    pub date: String,
    pub custom: String,
}

/// The choices that describe format `f` (`None`: no format of the visual's own).
pub fn read(of: &str, f: Option<&str>) -> Draft {
    let mut d = Draft {
        of: of.to_string(),
        kind: "auto",
        decimals: 2,
        thousands: true,
        symbol: "$".into(),
        date: "Short Date".into(),
        custom: f.unwrap_or_default().to_string(),
    };
    let Some(f) = f.map(str::trim).filter(|f| !f.is_empty()) else {
        return d;
    };
    let decimals = |s: &str| -> u32 {
        s.split_once('.')
            .map_or(0, |(_, t)| t.chars().take_while(|c| *c == '0').count() as u32)
    };
    let lower = f.to_ascii_lowercase();
    let number = |s: &str| {
        let core = s.strip_prefix("#,").unwrap_or(s);
        core.starts_with('0') && core.chars().all(|c| matches!(c, '0' | '.'))
    };
    // the .NET letters Power BI Desktop 2.159.1231.0's card writes for the named date formats
    // this card offers (`G` reads as General)
    let date_name = match f {
        "d" => Some("Short Date"),
        "D" => Some("Long Date"),
        _ => DATES.iter().find(|(x, ..)| *x == f).map(|(x, ..)| *x),
    };
    // a three-section format (Desktop's currencies and percentages) is read by its first section
    let f = f.split(';').next().unwrap_or(f).trim();
    d.kind = "custom";
    if lower == "general" || lower == "g" {
        d.kind = "general";
    } else if let Some(name) = date_name {
        d.kind = "date";
        d.date = name.to_string();
    } else if let Some(core) = f.strip_suffix("E+000").or_else(|| f.strip_suffix("E+00")) {
        if number(core) {
            d.kind = "scientific";
            d.decimals = decimals(core);
        }
    } else if let Some(core) = f.strip_suffix('%').map(str::trim_end) {
        if number(core) {
            d.kind = "percentage";
            d.decimals = decimals(core);
            d.thousands = core.starts_with("#,");
        }
    } else if number(f) {
        d.kind = if f.contains('.') { "decimal" } else { "whole" };
        d.decimals = decimals(f);
        d.thousands = f.starts_with("#,");
    } else if let Some((sym, core)) = SYMBOLS.iter().find_map(|s| {
        let text = match *s {
            "$" => "\\$".to_string(),
            "CHF" => "\"CHF \"".to_string(),
            other => other.to_string(),
        };
        f.strip_prefix(text.as_str())
            // Desktop's euro: `#,0.00 "€"`
            .or_else(|| f.strip_suffix(&format!(" \"{s}\"")))
            .map(|core| (*s, core))
    }) && number(core)
    {
        d.kind = "currency";
        d.symbol = sym.to_string();
        d.decimals = decimals(core);
        d.thousands = core.starts_with("#,");
    }
    d
}

/// The `visual.set_data_format` parameters (without page and visual) of `d` for the field at
/// `index` of `role`.
pub fn params(d: &Draft, role: &str, index: usize) -> Value {
    let mut p = json!({"role": role, "index": index});
    match d.kind {
        "custom" => p["format"] = json!(d.custom),
        k => {
            p["type"] = json!(k);
            if matches!(k, "decimal" | "percentage" | "currency" | "scientific") {
                p["decimals"] = json!(d.decimals.min(15));
            }
            if matches!(k, "whole" | "decimal" | "percentage" | "currency") {
                p["thousands"] = json!(d.thousands);
            }
            if k == "currency" {
                p["currency_symbol"] = json!(d.symbol);
            }
            if k == "date" {
                p["date_format"] = json!(d.date);
            }
        }
    }
    p
}

/// A field the card formats: (label, role, index in its role, current format).
fn formattable(v: &Value) -> Vec<(String, String, usize, Option<String>)> {
    let mut out = Vec::new();
    let mut seen: Vec<(String, usize)> = Vec::new();
    for f in v
        .get("fields")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(500)
    {
        let role = f.get("role").and_then(Value::as_str).unwrap_or("").to_string();
        let index = match seen.iter_mut().find(|(r, _)| *r == role) {
            Some((_, n)) => {
                *n += 1;
                *n - 1
            }
            None => {
                seen.push((role.clone(), 1));
                0
            }
        };
        let kind = f.get("kind").and_then(Value::as_str).unwrap_or("");
        if !matches!(kind, "measure" | "aggregation" | "column" | "visual_calculation") {
            continue;
        }
        let label = f
            .get("display_name")
            .or_else(|| f.get("label"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or_else(|| f.get("text").and_then(Value::as_str))
            .unwrap_or("?")
            .chars()
            .take(60)
            .collect();
        let fmt = f.get("format").and_then(Value::as_str).map(str::to_string);
        out.push((label, role, index, fmt));
    }
    out
}

/// The card's body: the field, its format's choices and Apply. Returns the parameters to run
/// `visual.set_data_format` with (without page and visual) when Apply is pressed.
pub fn ui(ui: &mut Ui, l: Lang, v: &Value, visual: &str, st: &Result<(), String>) -> Option<Value> {
    let t = Tokens::get(ui.ctx());
    let fields = formattable(v);
    if fields.is_empty() {
        ui.label(
            RichText::new(l.t(
                "This visual has no field to format.",
                "Ce visuel n'a pas de champ à mettre en forme.",
            ))
            .small()
            .color(t.text_dim),
        );
        return None;
    }
    let pick_id = egui::Id::new(("bicraft-data-format-field", visual));
    let mut pick: usize = ui.data(|d| d.get_temp(pick_id)).unwrap_or(0);
    if pick >= fields.len() {
        pick = 0;
    }
    let apply_to = l.t("Apply settings to", "Appliquer les paramètres à");
    let shown = fields.get(pick).map_or("", |f| f.0.as_str());
    egui::ComboBox::from_label(apply_to)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            for (i, f) in fields.iter().enumerate() {
                ui.selectable_value(&mut pick, i, &f.0);
            }
        });
    ui.data_mut(|d| d.insert_temp(pick_id, pick));
    let (_, role, index, cur) = fields.get(pick).cloned()?;
    let of = format!("{role}#{index}#{}", cur.as_deref().unwrap_or(""));
    let draft_id = egui::Id::new(("bicraft-data-format-draft", visual));
    let mut d: Draft = ui
        .data(|m| m.get_temp::<Draft>(draft_id))
        .filter(|d| d.of == of)
        .unwrap_or_else(|| read(&of, cur.as_deref()));
    let mut run = None;
    ui.add_enabled_ui(st.is_ok(), |ui| {
        let format_label = l.t("Format", "Format");
        let shown = TYPES
            .iter()
            .find(|(k, ..)| *k == d.kind)
            .map_or("", |(_, en, fr)| l.t(en, fr));
        egui::ComboBox::from_label(format_label)
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for (k, en, fr) in TYPES {
                    ui.selectable_value(&mut d.kind, *k, l.t(en, fr));
                }
            });
        if matches!(d.kind, "decimal" | "percentage" | "currency" | "scientific") {
            ui.horizontal(|ui| {
                let name = l.t("Decimal places", "Nombre de décimales");
                ui.label(RichText::new(name).small());
                let r = ui.add(egui::DragValue::new(&mut d.decimals).range(0..=15));
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, name));
            });
        }
        if matches!(d.kind, "whole" | "decimal" | "percentage" | "currency") {
            ui.checkbox(
                &mut d.thousands,
                l.t("Thousands separator", "Séparateur de milliers"),
            );
        }
        if d.kind == "currency" {
            egui::ComboBox::from_label(l.t("Currency symbol", "Symbole monétaire"))
                .selected_text(d.symbol.clone())
                .show_ui(ui, |ui| {
                    for s in SYMBOLS {
                        ui.selectable_value(&mut d.symbol, (*s).to_string(), *s);
                    }
                });
        }
        if d.kind == "date" {
            let shown = DATES
                .iter()
                .find(|(k, ..)| *k == d.date)
                .map_or(d.date.as_str(), |(_, en, fr)| l.t(en, fr))
                .to_string();
            egui::ComboBox::from_label(l.t("Date format", "Format de date"))
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (k, en, fr) in DATES {
                        ui.selectable_value(&mut d.date, (*k).to_string(), l.t(en, fr));
                    }
                });
        }
        if d.kind == "custom" {
            let name = l.t("Format code", "Code de format");
            ui.label(RichText::new(name).small());
            let r = ui.add(
                egui::TextEdit::singleline(&mut d.custom)
                    .char_limit(255)
                    .hint_text("#,0.00")
                    .desired_width(ui.available_width()),
            );
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, name));
        }
        let now = cur
            .as_deref()
            .unwrap_or_else(|| l.t("the model's", "celui du modèle"));
        ui.label(
            RichText::new(format!("{} {now}", l.t("Current format:", "Format actuel :")))
                .small()
                .color(t.text_dim),
        );
        let apply = l.t("Apply the data format", "Appliquer le format des données");
        let r = ui.button(l.t("Apply", "Appliquer"));
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, apply));
        if r.clicked() {
            run = Some(params(&d, &role, index));
        }
    });
    if let Err(why) = st {
        ui.label(
            RichText::new(format!("{} {}", l.t("Unavailable:", "Indisponible :"), why))
                .small()
                .color(t.text_faint),
        );
    }
    ui.data_mut(|m| m.insert_temp(draft_id, d));
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each format the command writes reads back to the choices that wrote it.
    #[test]
    fn formats_read_back_to_their_choices() {
        let cases = [
            ("#,0.0", "decimal", 1, true),
            ("0", "whole", 0, false),
            ("#,0%", "percentage", 0, true),
            ("0.000E+00", "scientific", 3, false),
            ("€#,0.00", "currency", 2, true),
            ("\\$#,0", "currency", 0, true),
            ("\"CHF \"#,0.00", "currency", 2, true),
            // as Power BI Desktop 2.159.1231.0's card writes them (fr-FR)
            ("0.00E+000", "scientific", 2, false),
            ("\\$#,0.00;(\\$#,0.00);\\$#,0.00", "currency", 2, true),
            ("#,0.0 \"€\";-#,0.0 \"€\";#,0.0 \"€\"", "currency", 1, true),
            ("0.00 %;-0.00 %;0.00 %", "percentage", 2, false),
            ("#,0.0 %;-#,0.0 %;#,0.0 %", "percentage", 1, true),
        ];
        for (f, kind, decimals, thousands) in cases {
            let d = read("x", Some(f));
            assert_eq!((d.kind, d.decimals), (kind, decimals), "{f}");
            if kind != "scientific" {
                assert_eq!(d.thousands, thousands, "{f}");
            }
        }
        assert_eq!(read("x", None).kind, "auto");
        assert_eq!(read("x", Some("General")).kind, "general");
        assert_eq!(read("x", Some("Long Date")).kind, "date");
        assert_eq!(read("x", Some("G")).kind, "general");
        let d = read("x", Some("d"));
        assert_eq!((d.kind, d.date.as_str()), ("date", "Short Date"));
        assert_eq!(read("x", Some("€#,0.00;-€#,0.00;€#,0.00")).symbol, "€");
        assert_eq!(
            read("x", Some("#,0.0 \"€\";-#,0.0 \"€\";#,0.0 \"€\"")).symbol,
            "€"
        );
        let c = read("x", Some("0.0 \"u\""));
        assert_eq!((c.kind, c.custom.as_str()), ("custom", "0.0 \"u\""));
        // the parameters the card sends
        let p = params(&read("x", Some("€#,0.00")), "Values", 1);
        assert_eq!(
            p,
            json!({"role": "Values", "index": 1, "type": "currency", "decimals": 2, "thousands": true, "currency_symbol": "€"})
        );
        assert_eq!(params(&c, "Values", 0)["format"], "0.0 \"u\"");
    }
}
