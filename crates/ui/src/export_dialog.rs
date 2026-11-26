//! The Export data dialog of a visual (its "More options" menu and right-click, as Power BI's
//! visual header): which data (summarized, or underlying when the report's export setting
//! allows it), which format (.xlsx up to 150,000 rows, .csv up to 30,000), where; it runs
//! `visual.export_data`. On the desktop the Save dialog picks the file; in the browser the file
//! is written in the server workspace under the name typed, then downloaded
//! ([`crate::shell::Shell::download`]).

use egui::{RichText, Ui};
use serde_json::Value;

use crate::App;
use crate::actions;
use crate::app::{Then, clip};
use crate::i18n::Lang;
use crate::theme::{self, Tokens};
use crate::widgets;

/// The id of the dialog's output ([`Then::Output`]).
pub const OUTPUT: &str = "export_data";

/// The dialog's state while it is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportData {
    pub page: String,
    pub visual: String,
    /// `summarized` or `underlying`.
    pub kind: &'static str,
    /// `xlsx` or `csv`.
    pub format: &'static str,
    /// The workspace-relative name typed (the browser).
    pub target: String,
    /// The export ran (its answer is shown).
    pub ran: bool,
}

/// Opens the dialog for the selected visual (nothing when none is selected).
pub fn open(app: &mut App) {
    let Some(sel) = app.cmd.visual.clone() else {
        app.status = app
            .lang
            .t("Select a visual first.", "Sélectionnez d'abord un visuel.")
            .to_string();
        return;
    };
    app.output = None;
    app.cmd.export_data = Some(ExportData {
        page: sel.page,
        visual: sel.visual,
        kind: "summarized",
        format: "xlsx",
        target: String::new(),
        ran: false,
    });
}

/// The report's export setting from the snapshot (`AllowSummarized` when unknown).
pub fn mode(app: &App) -> String {
    app.loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/report/export_data_mode"))
        .and_then(Value::as_str)
        .unwrap_or("AllowSummarized")
        .to_string()
}

/// A file-name-safe stem from a visual's title or name.
pub(crate) fn stem(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    let t = t.trim();
    if t.is_empty() {
        "data".into()
    } else {
        t.to_string()
    }
}

/// The visual's title (else its type) for the dialog's heading and the file name.
fn title(app: &App, page: &str, visual: &str) -> String {
    let Some(snap) = app.loaded.as_ref().map(|l| &l.snap) else {
        return visual.to_string();
    };
    snap.pointer("/report/pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|p| p.get("name").and_then(Value::as_str) == Some(page))
        .flat_map(|p| p.get("visuals").and_then(Value::as_array).into_iter().flatten())
        .find(|v| v.get("name").and_then(Value::as_str) == Some(visual))
        .and_then(|v| {
            v.get("title")
                .and_then(Value::as_str)
                .filter(|t| !t.trim().is_empty())
                .or_else(|| v.get("type").and_then(Value::as_str))
        })
        .unwrap_or(visual)
        .to_string()
}

/// Draws the dialog when it is open.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut st) = app.cmd.export_data.clone() else {
        return;
    };
    let l = app.lang;
    let mut open = true;
    let mut close = false;
    egui::Window::new(l.t("Export data", "Exporter les données"))
        .id(egui::Id::new("bicraft-export-data"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(420.0);
            body(app, ui, &mut st, l, &mut close);
        });
    app.cmd.export_data = (open && !close).then_some(st);
}

fn body(app: &mut App, ui: &mut Ui, st: &mut ExportData, l: Lang, close: &mut bool) {
    let t = Tokens::of(app.prefs.theme);
    let name = title(app, &st.page, &st.visual);
    ui.label(RichText::new(clip(&name, 80)).font(theme::bold(14.0)));
    ui.add_space(6.0);
    let mode = mode(app);
    if mode == "None" {
        ui.label(
            RichText::new(l.t(
                "⚠ This report does not allow exporting data (File > Export > Export data setting).",
                "⚠ Ce rapport n'autorise pas l'export des données (Fichier > Exporter > Paramètre d'export des données).",
            ))
            .color(t.warn),
        );
    }
    let underlying_ok = mode == "AllowSummarizedAndUnderlying";
    if !underlying_ok && st.kind == "underlying" {
        st.kind = "summarized";
    }
    ui.label(l.t(
        "Which data do you want to export?",
        "Quelles données voulez-vous exporter ?",
    ));
    ui.radio_value(
        &mut st.kind,
        "summarized",
        l.t(
            "Summarized data: what the visual shows",
            "Données résumées : ce que montre le visuel",
        ),
    );
    let r = ui.add_enabled(
        underlying_ok,
        egui::RadioButton::new(
            st.kind == "underlying",
            l.t(
                "Underlying data: the rows behind it, not aggregated",
                "Données sous-jacentes : les lignes qui le composent, sans agrégation",
            ),
        ),
    );
    if r.clicked() {
        st.kind = "underlying";
    }
    if !underlying_ok {
        r.on_disabled_hover_text(l.t(
            "The report's export setting allows summarized data only.",
            "Le paramètre d'export du rapport n'autorise que les données résumées.",
        ));
    }
    ui.add_space(6.0);
    ui.label(l.t("File format", "Format de fichier"));
    ui.radio_value(
        &mut st.format,
        "xlsx",
        l.t(".xlsx (up to 150,000 rows)", ".xlsx (jusqu'à 150 000 lignes)"),
    );
    ui.radio_value(
        &mut st.format,
        "csv",
        l.t(".csv (up to 30,000 rows)", ".csv (jusqu'à 30 000 lignes)"),
    );
    ui.add_space(10.0);
    let can = mode != "None";
    let default = format!("{}.{}", stem(&name), st.format);
    let mut go: Option<String> = None;
    if let Some(pick) = &app.shell.pick_save {
        ui.horizontal(|ui| {
            if widgets::primary(ui, l.t("Export…", "Exporter…"), can, "").clicked() {
                go = pick(
                    l,
                    l.t("Export data", "Exporter les données"),
                    &default,
                    &[st.format],
                )
                .map(|p| p.display().to_string());
            }
            if ui.button(l.t("Cancel", "Annuler")).clicked() {
                *close = true;
            }
        });
    } else {
        if st.target.is_empty() || !st.target.ends_with(&format!(".{}", st.format)) {
            st.target = default;
        }
        ui.horizontal(|ui| {
            ui.label(l.t("File name", "Nom du fichier"));
            ui.add(
                egui::TextEdit::singleline(&mut st.target)
                    .char_limit(300)
                    .desired_width(220.0),
            );
        });
        let name = st.target.trim().to_string();
        let ok = can && crate::remote::safe_relative(&name);
        ui.horizontal(|ui| {
            if widgets::primary(
                ui,
                l.t("Export", "Exporter"),
                ok,
                l.t(
                    "A workspace-relative name",
                    "Un nom relatif à l'espace de travail",
                ),
            )
            .clicked()
            {
                go = Some(name.clone());
            }
            if ui.button(l.t("Cancel", "Annuler")).clicked() {
                *close = true;
            }
        });
    }
    if let Some(to) = go {
        let c = actions::visual_export_data(&st.page, &st.visual, st.kind, &to);
        let then = if app.shell.download.is_some() {
            Then::Download(OUTPUT, to)
        } else {
            Then::Output(OUTPUT)
        };
        app.run(c, then);
        st.ran = true;
    }
    if st.ran
        && let Some((id, r)) = &app.output
        && *id == OUTPUT
    {
        ui.add_space(8.0);
        match r {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(l.t("Exporting…", "Export en cours…"));
                });
            }
            Some(Err(e)) => {
                ui.label(RichText::new(format!("✕ {}", clip(e, 400))).color(t.err));
            }
            Some(Ok(v)) => {
                let rows = v.get("rows").and_then(Value::as_u64).unwrap_or(0);
                let path = v.get("path").and_then(Value::as_str).unwrap_or("");
                ui.label(
                    RichText::new(format!(
                        "✓ {} {} ({rows} {})",
                        l.t("Written:", "Écrit :"),
                        clip(path, 200),
                        l.t("rows", "lignes")
                    ))
                    .color(t.ok),
                );
                if v.get("truncated").and_then(Value::as_bool) == Some(true) {
                    ui.label(
                        RichText::new(l.t(
                            "⚠ The visual has more rows than the format's limit: the first ones were written.",
                            "⚠ Le visuel a plus de lignes que la limite du format : les premières ont été écrites.",
                        ))
                        .color(t.warn),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_are_safe() {
        assert_eq!(stem("Sales / Region"), "Sales _ Region");
        assert_eq!(stem("///"), "___");
        assert_eq!(stem("   "), "data");
    }
}
