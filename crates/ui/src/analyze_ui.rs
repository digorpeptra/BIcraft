//! Best practices (`model.analyze`): the report findings at the top of its answer window, one
//! row each with its severity (icon and word), its message, a Go to button that shows the
//! visual (or page) on the canvas, and a Fix button when the finding names a command that
//! removes the stale reference. Fix runs that command, then the analysis again.

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::actions::Call;
use crate::app::{App, Tab as View, Then};
use crate::i18n::Lang;
use crate::ribbon::VisualSel;
use crate::theme::Tokens;

/// The category of the report findings (`model.analyze`'s report rules).
const CATEGORY: &str = "Report references";
/// Rows shown; the rest stay in the answer below.
const MAX_ROWS: usize = 200;

/// What a row's button asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// Show page `page` and select `visual` on it.
    Go { page: String, visual: Option<String> },
    /// Run `command` with `params`, then the analysis again.
    Fix { command: String, params: Value },
}

/// The report findings of a `model.analyze` answer.
pub fn report_findings(v: &Value) -> Vec<&Value> {
    v.get("findings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|f| f.get("category").and_then(Value::as_str) == Some(CATEGORY))
        .collect()
}

/// The report findings section; returns the button clicked.
pub fn show(ui: &mut Ui, v: &Value, l: Lang, t: &Tokens) -> Option<Act> {
    let found = report_findings(v);
    let mut act = None;
    ui.label(RichText::new(l.t("Report references", "Références du rapport")).font(crate::theme::bold(14.0)));
    if found.is_empty() {
        ui.label(
            RichText::new(l.t(
                "✓ No broken reference in the report",
                "✓ Aucune référence rompue dans le rapport",
            ))
            .color(t.ok),
        );
        ui.separator();
        return None;
    }
    let max_h = (ui.available_height() * 0.45).max(120.0);
    egui::ScrollArea::vertical()
        .id_salt("analyze-report-findings")
        .max_height(max_h)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (i, f) in found.iter().take(MAX_ROWS).enumerate() {
                let s = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or("");
                let (mark, word, color) = match s("severity") {
                    "error" => ("✖", l.t("Error", "Erreur"), t.err),
                    "warning" => ("⚠", l.t("Warning", "Avertissement"), t.warn),
                    _ => ("ℹ", l.t("Info", "Info"), t.text_dim),
                };
                ui.push_id(("report-finding", i), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(format!("{mark} {word}")).color(color).strong());
                        ui.label(RichText::new(s("message")).color(t.text));
                    });
                    ui.horizontal(|ui| {
                        let loc = f.get("location");
                        let ls = |k: &str| loc.and_then(|x| x.get(k)).and_then(Value::as_str);
                        if let Some(page) = ls("page_name") {
                            let visual = ls("visual").map(str::to_string);
                            let text = if visual.is_some() {
                                l.t("Go to visual", "Aller au visuel")
                            } else {
                                l.t("Go to page", "Aller à la page")
                            };
                            if ui.button(text).clicked() {
                                act = Some(Act::Go {
                                    page: page.to_string(),
                                    visual,
                                });
                            }
                        }
                        if let Some(fix) = f.get("fix")
                            && let Some(command) = fix.get("command").and_then(Value::as_str)
                        {
                            let label = format!("{} ({command})", l.t("Fix", "Corriger"));
                            if ui.button(label).clicked() {
                                act = Some(Act::Fix {
                                    command: command.to_string(),
                                    params: fix.get("params").cloned().unwrap_or_else(|| json!({})),
                                });
                            }
                        }
                        if let Some(sug) = f.get("suggestion").and_then(Value::as_str) {
                            ui.label(
                                RichText::new(format!("{} {sug}", l.t("Did you mean", "Vouliez-vous dire")))
                                    .color(t.text_dim),
                            );
                        }
                    });
                    ui.add_space(4.0);
                });
            }
        });
    ui.separator();
    act
}

/// Carries out a row's button. Go to closes the window and shows the visual; Fix runs the
/// command, then the analysis again (its answer replaces this one).
pub fn apply(app: &mut App, act: Act) {
    match act {
        Act::Go { page, visual } => {
            app.cmd.result = None;
            app.cmd.view = None;
            app.backstage = None;
            if app.tab != View::Report {
                app.show_tab(View::Report);
            }
            let shown = app
                .page_names()
                .get(app.active_page())
                .is_some_and(|p| p.0 == page);
            if !shown {
                app.interact(json!({"action": "page", "page": page}));
            }
            app.cmd.visual = None;
            app.cmd.more.clear();
            app.cmd.reveal = visual.map(|visual| VisualSel { page, visual });
        }
        Act::Fix { command, params } => {
            let Some(c) = crate::registry::find(&command) else {
                app.error = Some(format!("✕ unknown command `{command}`"));
                return;
            };
            if let Err(why) = crate::ribbon::state(app, c.id) {
                app.status = format!("{command}: {why}");
                return;
            }
            if app.run(Call { id: c.id, params }, Then::Nothing)
                && let Some(c) = crate::registry::find("model.analyze")
            {
                crate::ribbon::send(app, c, json!({}));
            }
        }
    }
}
