//! The visual calculations edit window (Power BI Desktop's visual calculations edit mode,
//! Microsoft Learn "Visual calculations overview"): opened by a visual's New calculation (its
//! menu, `visual.calculation.add` run without an expression) or by the palette's Edit visual
//! calculation. It shows the visual's fields with a Hide switch each (`visual.calculation.set
//! {hidden}`), a name and a formula bar with the templates of Desktop's menu, and the visual
//! matrix as the engine evaluated it (hidden fields included, marked). Apply runs
//! `visual.calculation.add` for a new calculation, `visual.calculation.set` for an edited one;
//! Delete runs `visual.calculation.remove`. The engine checks the expression; its error goes to
//! the error bar and the window stays open.

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::{Then, clip};
use crate::i18n::Lang;
use crate::theme::{self, Tokens};
use crate::widgets;

pub const ADD: &str = "visual.calculation.add";
pub const SET: &str = "visual.calculation.set";
pub const REMOVE: &str = "visual.calculation.remove";
/// Rows of the visual matrix shown.
const MAX_ROWS: usize = 40;
/// Columns of the visual matrix shown.
const MAX_COLS: usize = 16;

/// The open window's values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editor {
    pub page: String,
    pub visual: String,
    /// The calculation edited (its name in the visual); `None`: a new one.
    pub editing: Option<String>,
    pub name: String,
    pub expression: String,
    /// The name was typed (a template then keeps it).
    pub named: bool,
}

/// The templates of Desktop's menu that BIcraft evaluates: (EN, FR, expression with `{}` the
/// first value field).
const TEMPLATES: [(&str, &str, &str); 9] = [
    ("Running sum", "Somme cumulée", "RUNNINGSUM({})"),
    ("Moving average", "Moyenne mobile", "MOVINGAVERAGE({}, 3)"),
    (
        "Percent of parent",
        "Pourcentage du parent",
        "DIVIDE({}, COLLAPSE({}, ROWS))",
    ),
    (
        "Percent of grand total",
        "Pourcentage du total général",
        "DIVIDE({}, COLLAPSEALL({}, ROWS))",
    ),
    (
        "Average of children",
        "Moyenne des enfants",
        "EXPAND(AVERAGE({}), ROWS)",
    ),
    ("Versus previous", "Par rapport au précédent", "{} - PREVIOUS({})"),
    ("Versus next", "Par rapport au suivant", "{} - NEXT({})"),
    ("Versus first", "Par rapport au premier", "{} - FIRST({})"),
    ("Versus last", "Par rapport au dernier", "{} - LAST({})"),
];

fn bracket(n: &str) -> String {
    format!("[{}]", n.replace(']', "]]"))
}

/// The snapshot entry of the edited visual.
fn visual_json<'a>(app: &'a App, page: &str, visual: &str) -> Option<&'a Value> {
    let snap = &app.loaded.as_ref()?.snap;
    snap.pointer("/report/pages")?
        .as_array()?
        .iter()
        .find(|p| p.get("name").and_then(Value::as_str) == Some(page))?
        .get("visuals")?
        .as_array()?
        .iter()
        .find(|v| v.get("name").and_then(Value::as_str) == Some(visual))
}

fn fields(v: Option<&Value>) -> Vec<Value> {
    v.and_then(|v| v.get("fields"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or_default()
}

/// The first free `Calculation`, `Calculation 2`… among the visual's fields.
fn free_name(v: Option<&Value>, l: Lang) -> String {
    let base = l.t("Calculation", "Calcul");
    let taken: Vec<String> = fields(v).iter().map(|f| s(f, "ref").to_lowercase()).collect();
    let free = |n: &str| !taken.contains(&n.to_lowercase());
    if free(base) {
        return base.to_string();
    }
    (2..1000)
        .map(|i| format!("{base} {i}"))
        .find(|n| free(n))
        .unwrap_or_else(|| base.to_string())
}

/// Opens the window on visual `visual` of `page`: a new calculation, or `editing`.
pub fn open(app: &mut App, page: String, visual: String, editing: Option<String>) {
    let v = visual_json(app, &page, &visual);
    let (name, expression) = match &editing {
        Some(n) => {
            let f = fields(v)
                .into_iter()
                .find(|f| s(f, "ref").eq_ignore_ascii_case(n) && s(f, "kind") == "visual_calculation");
            (
                n.clone(),
                f.map(|f| s(&f, "expression").to_string()).unwrap_or_default(),
            )
        }
        None => (free_name(v, app.lang), String::new()),
    };
    app.cmd.viscalc = Some(Editor {
        page,
        visual,
        editing,
        name,
        expression,
        named: false,
    });
}

/// New calculation without an expression (a menu or palette click) and Edit visual calculation
/// without a change open the window.
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    let wants = match id {
        ADD => params
            .get("expression")
            .is_none_or(|e| e.as_str().is_some_and(str::is_empty)),
        SET => ["expression", "new_name", "hidden"]
            .iter()
            .all(|k| params.get(*k).is_none_or(Value::is_null)),
        _ => false,
    };
    if !wants {
        return false;
    }
    let sel = app.cmd.visual.clone();
    let page = params
        .get("page")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| sel.as_ref().map(|x| x.page.clone()));
    let visual = params
        .get("visual")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| sel.as_ref().map(|x| x.visual.clone()));
    let (Some(page), Some(visual)) = (page, visual) else {
        app.status = app
            .lang
            .t("Select a visual first.", "Sélectionnez d'abord un visuel.")
            .to_string();
        return true;
    };
    let editing = if id == SET {
        params.get("name").and_then(Value::as_str).map(str::to_string)
    } else {
        None
    };
    open(app, page, visual, editing);
    true
}

/// The command Apply runs for the window's values.
pub fn apply_call(e: &Editor) -> Call {
    match &e.editing {
        None => Call {
            id: ADD,
            params: json!({"page": e.page, "visual": e.visual, "name": e.name.trim(), "expression": e.expression}),
        },
        Some(old) => {
            let mut p = json!({"page": e.page, "visual": e.visual, "name": old, "expression": e.expression});
            if !old.eq_ignore_ascii_case(e.name.trim())
                && let Some(o) = p.as_object_mut()
            {
                o.insert("new_name".into(), json!(e.name.trim()));
            }
            Call { id: SET, params: p }
        }
    }
}

/// Draws the open window.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut e) = app.cmd.viscalc.clone() else {
        return;
    };
    let l = app.lang;
    let mut open = true;
    let mut close = false;
    egui::Window::new(l.t("Visual calculations", "Calculs visuels"))
        .id(egui::Id::new("bicraft-visual-calculations"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(860.0);
            body(app, ui, &mut e, l, &mut close);
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    app.cmd.viscalc = (open && !close).then_some(e);
}

fn body(app: &mut App, ui: &mut Ui, e: &mut Editor, l: Lang, close: &mut bool) {
    let t = Tokens::of(app.prefs.theme);
    let v = visual_json(app, &e.page, &e.visual).cloned();
    let fs = fields(v.as_ref());
    let title = v
        .as_ref()
        .map(|v| {
            let t = s(v, "title");
            if t.is_empty() {
                s(v, "auto_title").to_string()
            } else {
                t.to_string()
            }
        })
        .unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(if title.is_empty() {
                e.visual.clone()
            } else {
                clip(&title, 80)
            })
            .font(theme::bold(14.0)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(l.t("Back to report", "Retour au rapport")).clicked() {
                *close = true;
            }
        });
    });
    ui.label(
        RichText::new(l.t(
            "A calculation on this visual's data: refer to its fields by name in brackets, to its axes as ROWS and COLUMNS.",
            "Un calcul sur les données de ce visuel : ses champs par leur nom entre crochets, ses axes par ROWS et COLUMNS.",
        ))
        .color(t.text_dim),
    );
    ui.add_space(6.0);
    let mut action: Option<Call> = None;
    ui.horizontal_top(|ui| {
        // the fields of the visual, each with its Hide switch; a calculation opens in the bar
        ui.vertical(|ui| {
            ui.set_width(250.0);
            ui.label(RichText::new(l.t("On this visual", "Sur ce visuel")).font(theme::bold(13.0)));
            egui::Frame::new()
                .stroke(egui::Stroke::new(1.0, t.border))
                .corner_radius(egui::CornerRadius::same(4))
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    ui.set_min_width(236.0);
                    for f in fs.iter().take(60) {
                        let name = s(f, "ref").to_string();
                        if name.is_empty() {
                            continue;
                        }
                        let calc = s(f, "kind") == "visual_calculation";
                        let hidden = f.get("hidden") == Some(&Value::Bool(true));
                        ui.horizontal(|ui| {
                            let mut shown = !hidden;
                            let tip = format!("{} {name}", l.t("Show on the visual:", "Afficher sur le visuel :"));
                            let r = ui.checkbox(&mut shown, "").on_hover_text(&tip);
                            r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, shown, &tip));
                            if r.clicked() {
                                action = Some(Call {
                                    id: SET,
                                    params: json!({"page": e.page, "visual": e.visual, "name": name, "hidden": !shown}),
                                });
                            }
                            let kind = if calc { "ƒx" } else { "Σ" };
                            let mut text = RichText::new(format!("{kind}  {}", clip(&name, 40)));
                            if hidden {
                                text = text.color(t.text_dim).italics();
                            }
                            if calc {
                                let editing = e.editing.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(&name));
                                let r = ui.selectable_label(editing, text);
                                if r.clicked() {
                                    e.editing = Some(name.clone());
                                    e.name.clone_from(&name);
                                    e.expression = s(f, "expression").to_string();
                                    e.named = true;
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let label = format!("{} {name}", l.t("Delete", "Supprimer"));
                                    let b = ui.small_button("🗑").on_hover_text(&label);
                                    b.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
                                    if b.clicked() {
                                        action = Some(Call {
                                            id: REMOVE,
                                            params: json!({"page": e.page, "visual": e.visual, "name": name}),
                                        });
                                        if editing {
                                            e.editing = None;
                                            e.expression.clear();
                                        }
                                    }
                                });
                            } else {
                                ui.label(text);
                            }
                            if hidden {
                                ui.label(RichText::new(l.t("hidden", "masqué")).color(t.text_dim).small());
                            }
                        });
                    }
                    if fs.is_empty() {
                        ui.label(RichText::new(l.t("No fields.", "Aucun champ.")).color(t.text_dim));
                    }
                });
            ui.add_space(4.0);
            if ui.button(l.t("New calculation", "Nouveau calcul")).clicked() {
                e.editing = None;
                e.name = free_name(v.as_ref(), l);
                e.expression.clear();
                e.named = false;
            }
        });
        ui.add_space(8.0);
        ui.vertical(|ui| {
            formula_bar(ui, e, &fs, l, &t, &mut action);
            ui.add_space(8.0);
            ui.label(RichText::new(l.t("Visual matrix", "Matrice visuelle")).font(theme::bold(13.0)));
            matrix(ui, v.as_ref(), l, &t);
        });
    });
    if let Some(c) = action {
        let applying = c.id == ADD || (c.id == SET && c.params.get("expression").is_some());
        app.run(c, if applying { Then::VisualCalc } else { Then::Nothing });
    }
}

/// Apply's answer: the calculation applied is the one edited now; an error goes to the error
/// bar and the window keeps the values typed.
pub fn answer(app: &mut App, r: Result<Value, String>) {
    match r {
        Ok(v) => {
            let name = v.get("name").and_then(Value::as_str);
            if let (Some(e), Some(name)) = (app.cmd.viscalc.as_mut(), name)
                && v.get("visual").and_then(Value::as_str) == Some(e.visual.as_str())
            {
                e.editing = Some(name.to_string());
                e.name = name.to_string();
            }
        }
        Err(e) => app.error = Some(format!("✕ {}", clip(&e, 600))),
    }
}

fn formula_bar(ui: &mut Ui, e: &mut Editor, fs: &[Value], l: Lang, t: &Tokens, action: &mut Option<Call>) {
    ui.horizontal(|ui| {
        let r = ui.label(l.t("Name", "Nom"));
        let n = ui
            .add(
                egui::TextEdit::singleline(&mut e.name)
                    .char_limit(200)
                    .desired_width(220.0),
            )
            .labelled_by(r.id);
        if n.changed() {
            e.named = true;
        }
        // Desktop's templates, on the first value field
        let first = fs
            .iter()
            .find(|f| matches!(s(f, "kind"), "measure" | "aggregation"))
            .or_else(|| fs.first())
            .map(|f| bracket(s(f, "ref")));
        let b = ui.add_enabled(
            first.is_some(),
            egui::Button::new(format!("{} ▾", l.t("Templates", "Modèles"))),
        );
        let label = l.t("Templates", "Modèles");
        b.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        if let Some(field) = first {
            egui::Popup::menu(&b).show(|ui| {
                for (en, fr, pattern) in TEMPLATES {
                    if ui.button(l.t(en, fr)).clicked() {
                        e.expression = pattern.replace("{}", &field);
                        if !e.named && e.editing.is_none() {
                            e.name = l.t(en, fr).to_string();
                        }
                        ui.close();
                    }
                }
            });
        }
    });
    ui.add_space(4.0);
    let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, wrap: f32| {
        let job = crate::formula::job(text.as_str(), t, None, wrap);
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let label = l.t("Formula", "Formule");
    let r = ui.add(
        egui::TextEdit::multiline(&mut e.expression)
            .char_limit(32_000)
            .desired_rows(3)
            .desired_width(560.0)
            .font(crate::formula::mono())
            .hint_text(l.t("RUNNINGSUM([Sales Amount])", "RUNNINGSUM([Montant des ventes])"))
            .layouter(&mut layouter),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, label));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let ok = !e.expression.trim().is_empty() && !e.name.trim().is_empty();
        let why = l.t(
            "Type a name and an expression.",
            "Saisissez un nom et une expression.",
        );
        if widgets::primary(ui, l.t("Apply", "Appliquer"), ok, why).clicked() {
            *action = Some(apply_call(e));
        }
        let state = match &e.editing {
            Some(n) => format!("{} [{n}]", l.t("Editing", "Modification de")),
            None => l.t("New calculation", "Nouveau calcul").to_string(),
        };
        ui.label(RichText::new(state).color(t.text_dim));
    });
}

/// A number as the matrix shows it (the UI language's decimal separator).
fn cell_text(v: &Value, l: Lang) -> String {
    match v {
        Value::Null => String::new(),
        Value::Number(n) => {
            let x = n.as_f64().unwrap_or(0.0);
            let text = if x.fract() == 0.0 && x.abs() < 1e15 {
                format!("{x:.0}")
            } else {
                let t = format!("{x:.4}");
                t.trim_end_matches('0').trim_end_matches('.').to_string()
            };
            if l == Lang::Fr {
                text.replace('.', ",")
            } else {
                text
            }
        }
        Value::String(t) => clip(t, 40),
        Value::Bool(b) => if *b {
            l.t("True", "Vrai")
        } else {
            l.t("False", "Faux")
        }
        .to_string(),
        other => clip(&other.to_string(), 40),
    }
}

/// The visual matrix the engine evaluated: its columns (hidden ones marked), its rows and total.
fn matrix(ui: &mut Ui, v: Option<&Value>, l: Lang, t: &Tokens) {
    let d = v.and_then(|v| v.get("data"));
    let ok = d.and_then(|d| d.get("status")).and_then(Value::as_str) == Some("ok");
    let Some(d) = d.filter(|_| ok) else {
        ui.label(
            RichText::new(l.t(
                "The visual's data is not evaluated yet.",
                "Les données du visuel ne sont pas encore évaluées.",
            ))
            .color(t.text_dim),
        );
        return;
    };
    let cols: Vec<Value> = d
        .get("columns")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .take(MAX_COLS)
        .collect();
    let rows: Vec<Value> = d
        .get("rows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for c in &cols {
        if let Some(r) = c.get("reason").and_then(Value::as_str) {
            ui.label(
                RichText::new(format!(
                    "⚠ [{}] {}: {}",
                    s(c, "name"),
                    l.t("has no value", "n'a pas de valeur"),
                    clip(r, 200)
                ))
                .color(t.warn),
            );
        }
    }
    egui::ScrollArea::both()
        .id_salt("bicraft-viscalc-matrix")
        .max_height(300.0)
        .max_width(580.0)
        .show(ui, |ui| {
            egui::Grid::new("bicraft-viscalc-grid")
                .striped(true)
                .spacing(egui::vec2(14.0, 3.0))
                .show(ui, |ui| {
                    for c in &cols {
                        let hidden = c.get("hidden") == Some(&Value::Bool(true));
                        let mut text = clip(s(c, "name"), 30);
                        if hidden {
                            text = format!("{text} ({})", l.t("hidden", "masqué"));
                        }
                        let mut rt = RichText::new(text).font(theme::bold(12.0));
                        if hidden {
                            rt = rt.color(t.text_dim);
                        }
                        ui.label(rt);
                    }
                    ui.end_row();
                    for r in rows.iter().take(MAX_ROWS) {
                        for (i, _) in cols.iter().enumerate() {
                            let x = r.get(i).unwrap_or(&Value::Null);
                            ui.label(cell_text(x, l));
                        }
                        ui.end_row();
                    }
                    if let Some(tot) = d.get("totals").and_then(Value::as_array) {
                        for (i, _) in cols.iter().enumerate() {
                            let x = tot.get(i).unwrap_or(&Value::Null);
                            let text = if i == 0 && x.is_null() {
                                l.t("Total", "Total").to_string()
                            } else {
                                cell_text(x, l)
                            };
                            ui.label(RichText::new(text).font(theme::bold(12.0)));
                        }
                        ui.end_row();
                    }
                });
        });
    if rows.len() > MAX_ROWS {
        ui.label(
            RichText::new(format!(
                "{} {}",
                rows.len().saturating_sub(MAX_ROWS),
                l.t("more rows", "lignes de plus")
            ))
            .color(t.text_dim),
        );
    }
}
