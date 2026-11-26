//! Model › Relationships › Autodetect (Power BI Desktop's Manage relationships › Autodetect):
//! the button runs `relationship.autodetect` without `apply` (a preview that changes nothing),
//! the dialog lists the relationships found, each with a check box, and the pairs passed over with
//! why; Add runs the command again with `apply` and the ticked ones in `only` (one undo step).

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::Then;
use crate::i18n::Lang;
use crate::theme::{self, Tokens};
use crate::widgets;

pub const COMMAND: &str = "relationship.autodetect";
/// Most rows the dialog lists.
const MAX_LISTED: usize = 500;

/// One relationship found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub from: String,
    pub to: String,
    pub one_to_one: bool,
    pub on: bool,
}

/// The dialog while it is open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dialog {
    /// `None` while the preview runs.
    pub found: Option<Vec<Found>>,
    /// (from, to, code) of the pairs passed over.
    pub skipped: Vec<(String, String, String)>,
    pub error: Option<String>,
}

fn text(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// The button (or the palette) runs the command without `apply`: the dialog opens on its answer.
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    if id != COMMAND || params.get("apply").is_some_and(|a| a == &json!(true)) {
        return false;
    }
    open(app);
    true
}

/// Opens the dialog and runs the preview.
pub fn open(app: &mut App) {
    app.cmd.autodetect = Some(Dialog::default());
    app.run(
        Call {
            id: COMMAND,
            params: json!({}),
        },
        Then::Autodetect,
    );
}

/// The answer of the preview (fills the dialog) or of the apply (closes it).
pub fn answer(app: &mut App, r: Result<Value, String>) {
    let l = app.lang;
    let v = match r {
        Ok(v) => v,
        Err(e) => {
            match app.cmd.autodetect.as_mut() {
                Some(d) => {
                    d.error = Some(crate::app::clip(&e, 600));
                    if d.found.is_none() {
                        d.found = Some(Vec::new());
                    }
                }
                None => app.error = Some(e),
            }
            return;
        }
    };
    if v.get("applied") == Some(&Value::Bool(true)) {
        let n = v.get("added").and_then(Value::as_array).map_or(0, Vec::len);
        app.status = format!("{n} {}", l.t("relationships added", "relations ajoutées"));
        app.cmd.autodetect = None;
        return;
    }
    let Some(d) = app.cmd.autodetect.as_mut() else {
        return;
    };
    let list = |k: &str| {
        v.get(k)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .take(MAX_LISTED)
    };
    d.found = Some(
        list("candidates")
            .map(|c| Found {
                from: text(&c, "from"),
                to: text(&c, "to"),
                one_to_one: text(&c, "from_cardinality") == "one",
                on: true,
            })
            .collect(),
    );
    d.skipped = list("skipped")
        .map(|c| (text(&c, "from"), text(&c, "to"), text(&c, "code")))
        .collect();
}

/// Why a pair was passed over, in the window's language.
fn reason(code: &str, l: Lang) -> &'static str {
    match code {
        "no_data" => l.t(
            "no data to infer the cardinality from",
            "aucune donnée pour déduire la cardinalité",
        ),
        "no_unique_side" => l.t(
            "neither column has unique values",
            "aucune des colonnes n'a de valeurs uniques",
        ),
        "ambiguous" => l.t(
            "it would make filter paths ambiguous",
            "elle rendrait des chemins de filtre ambigus",
        ),
        "unreadable" => l.t(
            "the values could not be read",
            "les valeurs n'ont pas pu être lues",
        ),
        _ => l.t("too many paths to check", "trop de chemins à vérifier"),
    }
}

/// The `relationship.autodetect` call of the ticked relationships.
pub fn call(d: &Dialog) -> Option<Call> {
    let only: Vec<Value> = d
        .found
        .iter()
        .flatten()
        .filter(|f| f.on)
        .map(|f| json!({"from": f.from, "to": f.to}))
        .collect();
    (!only.is_empty()).then(|| Call {
        id: COMMAND,
        params: json!({"apply": true, "only": only}),
    })
}

/// Draws the dialog when it is open.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut d) = app.cmd.autodetect.clone() else {
        return;
    };
    let l = app.lang;
    let mut open = true;
    let mut close = false;
    let mut go = None;
    egui::Window::new(l.t(
        "Autodetect relationships",
        "Détecter automatiquement les relations",
    ))
    .id(egui::Id::new("bicraft-autodetect"))
    .collapsible(false)
    .resizable(false)
    .open(&mut open)
    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
    .show(ctx, |ui| {
        ui.set_width(480.0);
        go = body(ui, &mut d, l, &mut close);
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    let keep = open && !close;
    app.cmd.autodetect = keep.then_some(d);
    if let Some(c) = go {
        app.run(c, Then::Autodetect);
    }
}

fn body(ui: &mut Ui, d: &mut Dialog, l: Lang, close: &mut bool) -> Option<Call> {
    let t = Tokens::get(ui.ctx());
    ui.label(
        RichText::new(l.t(
            "Columns with the same name and data type, where the data shows a unique side:",
            "Colonnes de même nom et de même type, dont les données montrent un côté unique :",
        ))
        .color(t.text_dim),
    );
    ui.add_space(6.0);
    let Some(found) = d.found.as_mut() else {
        ui.label(l.t("Looking for relationships…", "Recherche des relations…"));
        return None;
    };
    if found.is_empty() && d.error.is_none() {
        ui.label(l.t(
            "No new relationship was found.",
            "Aucune nouvelle relation trouvée.",
        ));
    }
    egui::ScrollArea::vertical()
        .id_salt("bicraft-autodetect-list")
        .max_height(260.0)
        .show(ui, |ui| {
            for f in found.iter_mut() {
                let kind = if f.one_to_one {
                    l.t("one to one, both directions", "un à un, deux sens")
                } else {
                    l.t("many to one, single direction", "plusieurs à un, un seul sens")
                };
                ui.checkbox(&mut f.on, format!("{} → {}  ({kind})", f.from, f.to));
            }
            if !d.skipped.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(l.t("Passed over", "Écartées")).font(theme::bold(13.0)));
                for (from, to, code) in &d.skipped {
                    let colon = l.t(": ", " : ");
                    ui.label(
                        RichText::new(format!("{from} → {to}{colon}{}", reason(code, l))).color(t.text_dim),
                    );
                }
            }
        });
    if let Some(e) = &d.error {
        ui.label(RichText::new(format!("⚠ {e}")).color(t.warn));
    }
    ui.add_space(8.0);
    let mut go = None;
    ui.horizontal(|ui| {
        let c = call(d);
        let n = d.found.iter().flatten().filter(|f| f.on).count();
        let why = l.t("Tick a relationship first", "Cochez d'abord une relation");
        let label = format!("{} ({n})", l.t("Add", "Ajouter"));
        if widgets::primary(ui, &label, c.is_some(), why).clicked() && c.is_some() {
            d.error = None;
            go = c;
        }
        if ui.button(l.t("Cancel", "Annuler")).clicked() {
            *close = true;
        }
    });
    go
}
