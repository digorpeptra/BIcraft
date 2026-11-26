//! The visual header's reader tools, as Power BI Desktop offers them (docs/desktop.md, "Focus
//! mode, Show as a table, Sort and Spotlight"): **focus mode** shows one visual alone, filling the canvas,
//! with a Back to report button; **Show as a table** shows it in focus with its data as a table
//! under it; **Spotlight** dims the page's other visuals (`view.interact` `spotlight`; Escape
//! or a click outside the visual ends it); the **Sort axis** menu sorts it by one of its fields
//! (`visual.set_sort` when the report can be edited, the reader's `view.interact` `sort`, which
//! leaves the report alone, for a reader).
//!
//! Focus mode and Show as a table are the reader's view state: `view.interact` `focus` and
//! `show_table` set it in the engine (so MCP and the CLI reach it too), the snapshot's
//! `interaction.focus` / `show_table` say what to draw, and the report and the files never
//! change. The page drawn is a derived snapshot ([`snapshot`]): the
//! same report, the focused page holding only the visual, moved to fill the page, and for Show
//! as a table a table visual under it reading the same `data` (the visual's summarized data,
//! already evaluated under its filters, slicers and cross-filters). The renderer draws it as any
//! page, so the visual stays interactive: its clicks and tooltips are the page's
//! (`view.interact`), its header and menus the canvas's. The data table's own regions are not
//! clickable ([`TABLE_VISUAL`]).

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::Ui;
use serde_json::{Map, Value, json};

use crate::App;
use crate::i18n::Lang;
use crate::icons::Icon;
use crate::ribbon::{self, VisualSel};
use crate::source::Loaded;
use crate::widgets;

/// The name of the data table drawn under a visual shown as a table (its hits are ignored).
pub const TABLE_VISUAL: &str = "bicraft-focus-table";
/// Space around the focused visual, in report pixels.
const PAD: f64 = 8.0;
/// Space between the visual and its data table, in report pixels.
const GAP: f64 = 12.0;
/// Share of the page height the visual takes above its data table.
const VISUAL_SHARE: f64 = 0.5;
/// Text size of the data table under a visual shown as a table, in points.
const TABLE_PT: f64 = 14.0;
/// Most fields the Sort axis menu lists.
const MAX_SORT_FIELDS: usize = 40;

/// The visual shown in focus mode, and whether its data table is shown under it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Focus {
    pub page: String,
    pub visual: String,
    pub table: bool,
}

impl Focus {
    /// The render cache's view key of this layout: never 0 (the page itself).
    pub fn view_key(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut h);
        h.finish() | 1
    }
}

/// The derived snapshot last built: (snapshot seq, focus) and the snapshot to draw.
#[derive(Default)]
pub struct FocusCache {
    built: Option<((u64, Focus), Loaded)>,
}

/// Shows visual `visual` of page `page` in focus mode (with its data table when `table`):
/// `view.interact` `focus` or `show_table`; selects it (its header stays: More options, the
/// focus icon, the drill icons).
pub fn enter(app: &mut App, page: &str, visual: &str, table: bool) {
    let action = if table { "show_table" } else { "focus" };
    app.interact(json!({"action": action, "page": page, "visual": visual}));
    app.cmd.visual = Some(VisualSel {
        page: page.to_string(),
        visual: visual.to_string(),
    });
    app.cmd.more.clear();
    app.popup = None;
}

/// Back to the report (`view.interact {action: "focus", visual: null}`), the visual still
/// selected.
pub fn exit(app: &mut App) {
    let page = current(app).map(|f| f.page);
    app.interact(json!({"action": "focus", "page": page, "visual": null}));
    app.popup = None;
}

/// The visual the snapshot shows in focus mode (`interaction.focus`, `interaction.show_table`).
pub fn current(app: &App) -> Option<Focus> {
    let i = app.loaded.as_ref()?.snap.get("interaction")?;
    let f = i.get("focus")?;
    Some(Focus {
        page: f.get("page")?.as_str()?.to_string(),
        visual: f.get("visual")?.as_str()?.to_string(),
        table: i.get("show_table").and_then(Value::as_bool) == Some(true),
    })
}

/// The visual in spotlight (`interaction.spotlight`): (page, visual).
pub fn spotlight(app: &App) -> Option<(String, String)> {
    let s = app.loaded.as_ref()?.snap.pointer("/interaction/spotlight")?;
    Some((
        s.get("page")?.as_str()?.to_string(),
        s.get("visual")?.as_str()?.to_string(),
    ))
}

/// Ends the spotlight (`view.interact {action: "spotlight", visual: null}`).
pub fn end_spotlight(app: &mut App) {
    let page = spotlight(app).map(|s| s.0);
    app.interact(json!({"action": "spotlight", "page": page, "visual": null}));
}

/// Whether a visual is shown in focus mode.
pub fn active(app: &App) -> bool {
    current(app).is_some()
}

/// The snapshot to draw for the focus the snapshot shows on the active page: `None` without a
/// focus, or when its page is not the one shown or its visual is gone.
pub fn loaded(app: &mut App) -> Option<Loaded> {
    let f = current(app)?;
    let l = app.loaded.as_ref()?;
    let key = (l.seq, f.clone());
    if let Some((k, built)) = &app.focus_view.built
        && *k == key
    {
        return Some(built.clone());
    }
    match snapshot(&l.snap, app.active_page(), &f) {
        Some(s) => {
            let built = Loaded {
                snap: Arc::new(s),
                assets: Arc::clone(&l.assets),
                seq: l.seq,
            };
            app.focus_view.built = Some((key, built.clone()));
            Some(built)
        }
        None => {
            app.focus_view.built = None;
            None
        }
    }
}

fn num(v: &Value, k: &str, d: f64) -> f64 {
    v.get(k)
        .and_then(Value::as_f64)
        .filter(|x| x.is_finite() && *x > 0.0)
        .unwrap_or(d)
}

/// `snap` with page `page` showing focus `f`: only the visual, filling the page (less a small
/// margin), and for Show as a table a table of its data under it. `None` when page `page` is
/// not `f.page` or has no such visual (a group or a hidden visual is not focused).
pub fn snapshot(snap: &Value, page: usize, f: &Focus) -> Option<Value> {
    let p = snap.pointer(&format!("/report/pages/{page}"))?;
    if p.get("name")?.as_str()? != f.page {
        return None;
    }
    let pw = num(p, "width", 1280.0).min(20_000.0);
    let ph = num(p, "height", 720.0).min(20_000.0);
    let v = p
        .get("visuals")?
        .as_array()?
        .iter()
        .take(2000)
        .find(|v| v.get("name").and_then(Value::as_str) == Some(f.visual.as_str()))?;
    if !v.get("type").is_some_and(Value::is_string) || v.get("hidden").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    let (w, h) = ((pw - 2.0 * PAD).max(1.0), (ph - 2.0 * PAD).max(1.0));
    let vh = if f.table {
        ((h - GAP) * VISUAL_SHARE).floor().max(1.0)
    } else {
        h
    };
    let mut focused = v.clone();
    place(&mut focused, PAD, PAD, w, vh, 0.0);
    if let Some(o) = focused.as_object_mut() {
        o.insert("parent_group".into(), Value::Null);
    }
    let mut list = vec![focused];
    if f.table {
        list.push(data_table(v, PAD, PAD + vh + GAP, w, (h - vh - GAP).max(1.0)));
    }
    let mut out = snap.clone();
    let slot = out.pointer_mut(&format!("/report/pages/{page}/visuals"))?;
    *slot = Value::Array(list);
    Some(out)
}

fn place(v: &mut Value, x: f64, y: f64, w: f64, h: f64, z: f64) {
    if let Some(o) = v.as_object_mut() {
        for (k, n) in [("x", x), ("y", y), ("width", w), ("height", h), ("z", z)] {
            o.insert(k.into(), json!(n));
        }
    }
}

/// A table visual showing the data of `v` (its columns in the visual's order, its rows as the
/// visual sorts them), with no title.
fn data_table(v: &Value, x: f64, y: f64, w: f64, h: f64) -> Value {
    let as_values = |list: Option<&Value>| -> Value {
        Value::Array(
            list.and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(200)
                .map(|c| {
                    let mut c = c.clone();
                    if let Some(o) = c.as_object_mut() {
                        o.insert("role".into(), json!("Values"));
                    }
                    c
                })
                .collect(),
        )
    };
    let mut data = v.get("data").cloned().unwrap_or(Value::Null);
    if let Some(d) = data.as_object_mut() {
        let cols = as_values(d.get("columns"));
        d.insert("columns".into(), cols);
        // the visual's own decorations do not apply to the table
        for k in ["cell_styles", "totals", "subtotals", "highlights"] {
            d.remove(k);
        }
    }
    let mut t = Map::new();
    t.insert("name".into(), json!(TABLE_VISUAL));
    t.insert("type".into(), json!("tableEx"));
    t.insert("title".into(), Value::Null);
    t.insert("parent_group".into(), Value::Null);
    t.insert("hidden".into(), json!(false));
    t.insert("fields".into(), as_values(v.get("fields")));
    t.insert("data".into(), data);
    // text a little larger than a table's default: the grid is read, not glanced at
    t.insert(
        "format".into(),
        json!({"title": {"show": false},
               "table": {"font_size": TABLE_PT, "header_font_size": TABLE_PT}}),
    );
    let mut t = Value::Object(t);
    place(&mut t, x, y, w, h, 1.0);
    t
}

/// The bar above a focused visual: Back to report (Escape too).
pub fn bar(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        let r = widgets::small_button(
            ui,
            Icon::Back,
            l.t("Back to report", "Retour au rapport"),
            true,
            false,
        );
        if r.clicked() {
            exit(app);
        }
    });
}

/// The focus icon of a visual's header: focus mode on, or back to the report when it is on.
pub fn header_button(app: &mut App, ui: &mut Ui, sel: &VisualSel) {
    let l = app.lang;
    let on = current(app).is_some_and(|f| f.page == sel.page && f.visual == sel.visual);
    let tip = if on {
        l.t("Exit focus mode", "Quitter le mode focus")
    } else {
        l.t("Focus mode", "Mode focus")
    };
    if widgets::icon_toggle(ui, Icon::Focus, tip, true, on).clicked() {
        if on {
            exit(app);
        } else {
            enter(app, &sel.page, &sel.visual, false);
        }
    }
}

/// The snapshot entry of visual `visual` on the page shown.
fn visual_of(app: &App, visual: &str) -> Option<Value> {
    let l = app.loaded.as_ref()?;
    l.snap
        .pointer(&format!("/report/pages/{}/visuals", app.active_page()))?
        .as_array()?
        .iter()
        .take(2000)
        .find(|v| v.get("name").and_then(Value::as_str) == Some(visual))
        .cloned()
}

/// Whether visual type `ty` is sorted along an axis (charts, tables, matrices): not cards,
/// gauges, slicers or the decorative types.
fn sortable_type(ty: &str) -> bool {
    let t = ty.to_ascii_lowercase();
    !(t.contains("slicer")
        || t.contains("card")
        || matches!(
            t.as_str(),
            "gauge"
                | "kpi"
                | "textbox"
                | "image"
                | "shape"
                | "basicshape"
                | "actionbutton"
                | "pagenavigator"
                | "bookmarknavigator"
                | "scriptvisual"
                | "pythonvisual"
                | "qnavisual"
        ))
}

/// The fields of `v` the Sort axis menu lists: (field text as `visual.set_sort` takes it,
/// label, whether it is a measure or aggregation).
fn sort_fields(v: &Value) -> Vec<(String, String, bool)> {
    let mut out: Vec<(String, String, bool)> = Vec::new();
    for f in v
        .get("fields")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(200)
    {
        let kind = f.get("kind").and_then(Value::as_str).unwrap_or("");
        if !matches!(kind, "column" | "measure" | "aggregation") {
            continue;
        }
        let Some(text) = f.get("text").and_then(Value::as_str).filter(|t| !t.is_empty()) else {
            continue;
        };
        if out.iter().any(|(t, _, _)| t == text) {
            continue;
        }
        let label = f
            .get("display_name")
            .or_else(|| f.get("label"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or_else(|| f.get("name").and_then(Value::as_str))
            .unwrap_or(text);
        out.push((text.to_string(), label.to_string(), kind != "column"));
        if out.len() >= MAX_SORT_FIELDS {
            break;
        }
    }
    out
}

/// The current sort of `v`: (field text, descending), from the snapshot's `sort`.
fn current_sort(v: &Value) -> Option<(String, bool)> {
    let s = v.get("sort")?.as_array()?.first()?;
    Some((
        s.get("field")?.as_str()?.to_string(),
        s.get("descending").and_then(Value::as_bool).unwrap_or(false),
    ))
}

/// The header menu's reader entries of visual `visual` (its More options and its right-click
/// menu): Show as a table, then Sort axis with the visual's fields and the two directions.
pub fn menu(app: &mut App, ui: &mut Ui, visual: &str) {
    let l = app.lang;
    let Some(v) = visual_of(app, visual) else { return };
    let Some(ty) = v.get("type").and_then(Value::as_str).map(str::to_string) else {
        return;
    };
    let Some(page) = app.page_names().get(app.active_page()).map(|p| p.0.clone()) else {
        return;
    };
    // Focus mode (also the header's icon, which a visual at the page's top does not show)
    let focused = current(app).is_some_and(|f| f.page == page && f.visual == visual);
    let label = if focused {
        l.t("Exit focus mode", "Quitter le mode focus")
    } else {
        l.t("Focus mode", "Mode focus")
    };
    if ui.button(label).clicked() {
        if focused {
            exit(app);
        } else {
            enter(app, &page, visual, false);
        }
        ui.close();
    }
    // Spotlight: the page's other visuals fade (checked while it is on)
    let lit = spotlight(app).is_some_and(|(p, v)| p == page && v == visual);
    if ui.selectable_label(lit, l.t("Spotlight", "Projecteur")).clicked() {
        let target = if lit { Value::Null } else { json!(visual) };
        app.interact(json!({"action": "spotlight", "page": page, "visual": target}));
        ui.close();
    }
    // Show as a table: a visual with data (not a slicer)
    let has_fields = v
        .get("fields")
        .and_then(Value::as_array)
        .is_some_and(|f| !f.is_empty());
    if has_fields && !ty.to_ascii_lowercase().contains("slicer") {
        let on = current(app).is_some_and(|f| f.visual == visual && f.table);
        if ui
            .selectable_label(on, l.t("Show as a table", "Afficher sous forme de tableau"))
            .clicked()
        {
            if on {
                // off: the visual stays in focus, without its table
                app.interact(json!({"action": "show_table", "page": page, "visual": visual, "on": false}));
            } else {
                enter(app, &page, visual, true);
            }
            ui.close();
        }
    }
    let fields = sort_fields(&v);
    if sortable_type(&ty) && !fields.is_empty() {
        // a reader sorts the view only (view.interact sort); an editor sorts the report
        let reader = app.access.as_deref() == Some("reader");
        // (reader actions go through App::interact, as focus mode and Spotlight do)
        let st = if reader {
            Ok(())
        } else {
            ribbon::state(app, "visual.set_sort")
        };
        let cur = current_sort(&v);
        let mut send: Option<Value> = None;
        let r = ui.add_enabled_ui(st.is_ok(), |ui| {
            ui.menu_button(l.t("Sort axis", "Trier l'axe"), |ui| {
                ui.set_min_width(180.0);
                for (text, label, measure) in &fields {
                    let on = cur.as_ref().is_some_and(|(f, _)| f == text);
                    if ui.selectable_label(on, crate::app::clip(label, 60)).clicked() {
                        // a measure sorts descending, a column ascending (the command's default)
                        let dir = if *measure { "desc" } else { "asc" };
                        send = Some(json!({"page": page, "visual": visual, "field": text,
                                           "direction": dir}));
                        ui.close();
                    }
                }
                ui.separator();
                // the direction applies to the current sort field, else to the first measure
                // (the field a chart is sorted by when it has no sort of its own)
                let base = cur.as_ref().map(|(f, _)| f.clone()).or_else(|| {
                    fields
                        .iter()
                        .find(|(_, _, m)| *m)
                        .or_else(|| fields.first())
                        .map(|(t, _, _)| t.clone())
                });
                for (desc, label) in [
                    (true, l.t("Sort descending", "Tri décroissant")),
                    (false, l.t("Sort ascending", "Tri croissant")),
                ] {
                    let on = cur.as_ref().is_some_and(|(_, d)| *d == desc);
                    if ui.selectable_label(on, label).clicked()
                        && let Some(f) = &base
                    {
                        send = Some(json!({"page": page, "visual": visual, "field": f,
                                           "direction": if desc { "desc" } else { "asc" }}));
                        ui.close();
                    }
                }
            });
        });
        if let Err(why) = &st {
            r.response
                .on_disabled_hover_text(format!("{} {why}", unavailable(l)));
        }
        if let Some(mut p) = send {
            ui.close();
            if reader {
                p["action"] = json!("sort");
                app.interact(p);
            } else {
                ribbon::invoke(app, "visual.set_sort", p);
            }
        }
    }
    ui.separator();
}

fn unavailable(l: Lang) -> &'static str {
    l.t("Unavailable:", "Indisponible :")
}
