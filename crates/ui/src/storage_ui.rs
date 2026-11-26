//! Storage mode in the Model view (docs/gaps.md G10, "Storage mode"): the Properties pane's
//! Advanced › Storage mode list (`props_pane.rs`) sends `table.set_storage_mode`; switching a
//! DirectQuery table to Import first asks, as Desktop does, and offers to set the DirectQuery
//! dimension tables it reaches to Dual (`dual_related`). The diagram's cards show each table's
//! mode as a badge with an icon and a word ([`badge`]) when the model is composite.

use egui::RichText;
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::Then;
use crate::i18n::Lang;
use crate::icons::Icon;
use crate::theme::Tokens;
use crate::widgets;

pub const COMMAND: &str = "table.set_storage_mode";
/// The modes the list offers, as the command spells them.
pub const MODES: &[&str] = &["import", "directQuery", "dual"];
/// Most tables the dialog lists.
const MAX_LISTED: usize = 200;
/// Most relationships followed to find the tables to set to Dual.
const MAX_FOLLOWED: usize = 10_000;

/// The confirmation before a DirectQuery table turns to Import.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Confirm {
    pub table: String,
    /// The DirectQuery tables that would become Dual.
    pub candidates: Vec<String>,
    /// Set them to Dual.
    pub dual: bool,
}

/// A mode's word.
pub fn mode_text(mode: &str, l: Lang) -> &'static str {
    match mode {
        "directQuery" => "DirectQuery",
        "dual" => l.t("Dual", "Double"),
        "hybrid" => l.t("Hybrid", "Hybride"),
        "directLake" => "Direct Lake",
        "push" => l.t("Push", "Push"),
        _ => l.t("Import", "Importation"),
    }
}

/// A mode's icon.
pub fn mode_icon(mode: &str) -> Icon {
    match mode {
        "directQuery" | "directLake" => Icon::Server,
        "dual" | "hybrid" => Icon::Layers,
        _ => Icon::Import,
    }
}

/// A table's mode in the snapshot (`import` when absent).
pub fn table_mode(t: &Value) -> &str {
    t.get("storage_mode")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .unwrap_or("import")
}

/// Whether the model has a table in another mode than Import (a composite or DirectQuery model):
/// then every card shows its mode.
pub fn composite(model: &Value) -> bool {
    model
        .get("tables")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|t| table_mode(t) != "import"))
}

fn tables(app: &App) -> Vec<Value> {
    app.loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/model/tables"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn relationships(app: &App) -> Vec<Value> {
    app.loaded
        .as_ref()
        .and_then(|l| l.snap.pointer("/model/relationships"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// The DirectQuery tables reached from `table` through the one side of many-to-one
/// relationships (what Desktop offers to set to Dual), from the snapshot.
pub fn candidates(tables: &[Value], rels: &[Value], table: &str) -> Vec<String> {
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let mode = |n: &str| {
        tables
            .iter()
            .find(|t| s(t, "name").eq_ignore_ascii_case(n))
            .map(|t| table_mode(t).to_string())
    };
    let mut seen = vec![table.to_lowercase()];
    let mut out = Vec::new();
    let mut todo = vec![table.to_string()];
    let mut steps = 0usize;
    while let Some(cur) = todo.pop() {
        for r in rels {
            steps = steps.saturating_add(1);
            if steps > MAX_FOLLOWED {
                return out;
            }
            let many_to_one = !s(r, "from_cardinality").eq_ignore_ascii_case("one")
                && !s(r, "to_cardinality").eq_ignore_ascii_case("many");
            if !many_to_one || !s(r, "from_table").eq_ignore_ascii_case(&cur) {
                continue;
            }
            let one = s(r, "to_table");
            if seen.contains(&one.to_lowercase()) {
                continue;
            }
            seen.push(one.to_lowercase());
            if mode(&one).as_deref() == Some("directQuery") {
                out.push(one.clone());
            }
            todo.push(one);
        }
    }
    out
}

/// A storage mode change from the Properties pane: a DirectQuery table switched to Import asks
/// first (returns true: the command waits for the answer).
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    if id != COMMAND
        || params.get("mode").and_then(Value::as_str) != Some("import")
        || params.get("dual_related").is_some()
    {
        return false;
    }
    let Some(table) = params.get("table").and_then(Value::as_str) else {
        return false;
    };
    let all = tables(app);
    let current = all
        .iter()
        .find(|t| t.get("name").and_then(Value::as_str) == Some(table))
        .map(|t| table_mode(t).to_string());
    if current.as_deref() != Some("directQuery") {
        return false;
    }
    let candidates = candidates(&all, &relationships(app), table);
    app.cmd.storage = Some(Confirm {
        table: table.to_string(),
        dual: !candidates.is_empty(),
        candidates,
    });
    true
}

/// Draws the confirmation when it is open.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut c) = app.cmd.storage.clone() else {
        return;
    };
    let l = app.lang;
    let mut open = true;
    let mut go = false;
    let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    egui::Window::new(l.t("Storage mode", "Mode de stockage"))
        .id(egui::Id::unique("bicraft-storage-mode-confirm"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -40.0))
        .show(ctx, |ui| {
            let t = Tokens::get(ui.ctx());
            ui.set_width(440.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                crate::icons::paint(ui.painter(), r, Icon::Warning, t.text, false);
                ui.label(
                    RichText::new(format!(
                        "{} {}",
                        l.t("Switch to Import:", "Passer en importation :"),
                        c.table
                    ))
                    .strong(),
                );
            });
            ui.add_space(4.0);
            ui.label(l.t(
                "The table's rows will be loaded into the model on the next refresh. Power BI Desktop cannot switch an Import table back to DirectQuery (BIcraft can undo it).",
                "Les lignes de la table seront chargées dans le modèle à la prochaine actualisation. Power BI Desktop ne peut pas repasser une table en importation en DirectQuery (BIcraft peut l'annuler).",
            ));
            ui.add_space(6.0);
            if c.candidates.is_empty() {
                ui.label(
                    RichText::new(l.t(
                        "No DirectQuery table is on the one side of its relationships.",
                        "Aucune table DirectQuery n'est du côté un de ses relations.",
                    ))
                    .color(t.text_dim),
                );
            } else {
                ui.label(l.t(
                    "Its relationships with DirectQuery tables become limited. Dual tables keep them regular:",
                    "Ses relations avec des tables DirectQuery deviennent limitées. Des tables en mode double les gardent normales :",
                ));
                let label = l.t(
                    "Set these tables to Dual",
                    "Passer ces tables en mode double",
                );
                ui.checkbox(&mut c.dual, label);
                ui.indent("bicraft-storage-dual-list", |ui| {
                    for n in c.candidates.iter().take(MAX_LISTED) {
                        ui.label(RichText::new(format!("• {n}")).color(t.text_dim));
                    }
                });
            }
            ui.add_space(10.0);
            let w = ui.available_width();
            ui.allocate_ui_with_layout(
                egui::vec2(w, 28.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if widgets::primary(ui, l.t("OK", "OK"), true, "").clicked() {
                        go = true;
                    }
                    if ui
                        .add(egui::Button::new(l.t("Cancel", "Annuler")).min_size(egui::vec2(72.0, 24.0)))
                        .clicked()
                    {
                        cancel = true;
                    }
                },
            );
        });
    if go {
        app.cmd.storage = None;
        let p =
            json!({"table": c.table, "mode": "import", "dual_related": c.dual && !c.candidates.is_empty()});
        app.run(
            Call {
                id: COMMAND,
                params: p,
            },
            Then::Nothing,
        );
    } else if cancel || !open {
        app.cmd.storage = None;
    } else {
        app.cmd.storage = Some(c);
    }
}
