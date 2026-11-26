//! The Performance analyzer pane (ROADMAP review lot 14), as Power BI Desktop's: Start recording,
//! Refresh visuals, Stop, Clear and Export at its top, then one row per visual evaluated with its
//! total time, which unrolls to BIcraft's evaluation time, the render time and Copy query.
//!
//! Every button runs an engine command (`perf.*`, AGENTS.md §9): the recording lives in the
//! session; the pane only keeps the last `perf.results` it read, which rows are unrolled and the
//! render it waits for. The times are BIcraft's own: its evaluator's (`evaluation_ms`, measured
//! by the engine) and its renderer's draw call of each visual (`render_ms`, measured by
//! `bicraft-render` where the page is drawn and handed back with `perf.render`; the browser has no
//! clock there, so it shows none).

use std::collections::BTreeSet;
use std::sync::Arc;

use bicraft_render::PageImage;
use egui::{RichText, Stroke, Ui};
use serde_json::{Map, Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::{Then, clip};
use crate::i18n::Lang;
use crate::icons::Icon;
use crate::theme::Tokens;
use crate::widgets;

/// The commands the pane runs (the coverage test reads it).
pub const COMMANDS: &[&str] = &[
    "perf.results",
    "perf.start",
    "perf.refresh",
    "perf.stop",
    "perf.clear",
    "perf.export",
    "perf.render",
];

/// The file Export writes where the shell has no Save dialog (the browser: a workspace file,
/// then downloaded).
pub const EXPORT_NAME: &str = "performance-analyzer.json";

/// The pane's state, per document.
#[derive(Debug, Default)]
pub struct PerfUi {
    /// The pane is shown.
    pub open: bool,
    /// The last `perf.results` read.
    pub results: Option<Value>,
    /// The last error of a `perf.*` command.
    pub error: Option<String>,
    /// The events unrolled, by `seq`.
    pub expanded: BTreeSet<u64>,
    /// Where the last export went, or why it failed.
    pub exported: Option<Result<String, String>>,
    /// A refresh whose page is drawn again: its render times go back to the engine.
    awaiting: Option<Await>,
}

#[derive(Debug)]
struct Await {
    page: String,
    index: usize,
    /// The page's image before the refresh (a newer one is the refresh's).
    before: Option<Arc<PageImage>>,
    visuals: Vec<String>,
}

/// Whether the shown page's image is not `before`.
fn is_new(before: Option<&Arc<PageImage>>, now: &Arc<PageImage>) -> bool {
    before.is_none_or(|b| !Arc::ptr_eq(b, now))
}

/// A `perf.*` command from the ribbon, a menu, the palette or the control channel: the pane runs
/// it (and opens). `perf.results` (the ribbon's Performance analyzer) toggles the pane.
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    if !COMMANDS.contains(&id) || !app.has_doc() {
        return false;
    }
    match id {
        "perf.results" => {
            app.cmd.perf.open = !app.cmd.perf.open;
            if app.cmd.perf.open {
                fetch(app);
            }
        }
        "perf.export" if params.get("out").is_none() => {
            app.cmd.perf.open = true;
            export(app);
        }
        "perf.refresh" => {
            app.cmd.perf.open = true;
            refresh(app, params.clone());
        }
        _ => {
            app.cmd.perf.open = true;
            send(app, id, params.clone());
        }
    }
    true
}

fn cmd_id(id: &str) -> Option<&'static str> {
    COMMANDS.iter().copied().find(|c| *c == id)
}

fn send(app: &mut App, id: &str, params: Value) {
    if let Some(id) = cmd_id(id) {
        app.cmd.perf.error = None;
        app.run(Call { id, params }, Then::Perf(id));
    }
}

/// Reads the recording again.
fn fetch(app: &mut App) {
    app.run(
        Call {
            id: "perf.results",
            params: json!({}),
        },
        Then::Perf("perf.results"),
    );
}

/// Refresh visuals: the page shown (unless `params` names one), evaluated afresh.
fn refresh(app: &mut App, mut params: Value) {
    if params.get("page").is_none() {
        let page = app.page_names().get(app.active_page()).map(|p| p.0.clone());
        let mut m = params.as_object().cloned().unwrap_or_default();
        if let Some(p) = page {
            m.insert("page".into(), json!(p));
        }
        params = Value::Object(m);
    }
    send(app, "perf.refresh", params);
}

/// Export: the Save dialog's file, else a workspace file downloaded.
fn export(app: &mut App) {
    let l = app.lang;
    app.cmd.perf.exported = None;
    let call = |out: String| Call {
        id: "perf.export",
        params: json!({"out": out}),
    };
    if let Some(pick) = &app.shell.pick_save {
        let Some(to) = pick(
            l,
            l.t(
                "Export the Performance analyzer",
                "Exporter l'Analyseur de performances",
            ),
            EXPORT_NAME,
            &["json"],
        ) else {
            return;
        };
        app.run(call(to.display().to_string()), Then::Perf("perf.export"));
    } else if app.shell.download.is_some() {
        app.run(
            call(EXPORT_NAME.to_string()),
            Then::Download("perf.export", EXPORT_NAME.to_string()),
        );
    } else {
        app.run(call(EXPORT_NAME.to_string()), Then::Perf("perf.export"));
    }
}

/// The answer of a `perf.*` command the pane ran.
pub fn answer(app: &mut App, id: &'static str, r: Result<Value, String>) {
    let v = match r {
        Ok(v) => v,
        Err(e) => {
            app.cmd.perf.error = Some(clip(&e, 600));
            if id == "perf.export" {
                app.cmd.perf.exported = Some(Err(e));
            }
            return;
        }
    };
    match id {
        "perf.results" => app.cmd.perf.results = Some(v),
        "perf.refresh" => {
            awaiting_render(app, &v);
            fetch(app);
        }
        "perf.export" => {
            let path = v.get("path").and_then(Value::as_str).unwrap_or_default();
            app.cmd.perf.exported = Some(Ok(path.to_string()));
        }
        _ => fetch(app),
    }
}

/// After a refresh, the page is drawn again; its render times are taken from that drawing.
fn awaiting_render(app: &mut App, v: &Value) {
    let Some(page) = v.get("page").and_then(Value::as_str) else {
        return;
    };
    let Some(index) = app.page_names().iter().position(|p| p.0 == page) else {
        return;
    };
    let visuals: Vec<String> = v
        .get("visuals")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| e.get("visual").and_then(Value::as_str).map(str::to_string))
        .collect();
    let before = match &app.shown {
        Some((k, Ok(img))) if k.page == index => Some(Arc::clone(img)),
        _ => None,
    };
    if let Some(p) = app.pages.as_mut() {
        p.forget(index);
    }
    app.cmd.perf.awaiting = Some(Await {
        page: page.to_string(),
        index,
        before,
        visuals,
    });
}

/// The render times of the refreshed page, once it is drawn again: handed to the engine.
fn collect_render(app: &mut App) {
    let Some(a) = &app.cmd.perf.awaiting else {
        return;
    };
    let Some((k, Ok(img))) = &app.shown else {
        return;
    };
    if k.page != a.index || !is_new(a.before.as_ref(), img) {
        return;
    }
    let mut times = Map::new();
    for n in &img.visuals {
        if let Some(ms) = n.render_ms
            && a.visuals.contains(&n.name)
        {
            times.insert(n.name.clone(), json!(ms));
        }
    }
    let page = a.page.clone();
    app.cmd.perf.awaiting = None;
    if !times.is_empty() {
        send(app, "perf.render", json!({"page": page, "times": times}));
    }
}

/// Milliseconds as the pane shows them: one decimal, a comma in French.
pub fn ms_text(ms: f64, l: Lang) -> String {
    let s = format!("{ms:.1}");
    match l {
        Lang::En => s,
        Lang::Fr => s.replace('.', ","),
    }
}

/// The name a row shows: the visual's title in the snapshot, else its type and name.
fn row_name(app: &App, e: &Value) -> String {
    let page = e.get("page").and_then(Value::as_str).unwrap_or_default();
    let visual = e.get("visual").and_then(Value::as_str).unwrap_or_default();
    let title = crate::rich::snap_visual(app, page, visual)
        .and_then(|v| v.get("title").and_then(Value::as_str).map(str::to_string))
        .filter(|t| !t.trim().is_empty());
    match title {
        Some(t) => t,
        None => {
            let ty = e.get("visual_type").and_then(Value::as_str).unwrap_or("visual");
            format!("{ty} ({visual})")
        }
    }
}

/// Draws the pane (Report view), left of the other panes, when it is open.
pub fn show(app: &mut App, ui: &mut Ui, frame: egui::Frame) {
    collect_render(app);
    if !app.cmd.perf.open {
        return;
    }
    egui::Panel::right("bicraft-perf-pane")
        .resizable(true)
        .default_size(300.0)
        .size_range(*crate::prefs::PANE_WIDTHS.start()..=*crate::prefs::PANE_WIDTHS.end())
        .frame(frame)
        .show(ui, |ui| pane(app, ui));
}

fn pane(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(l.t("Performance analyzer", "Analyseur de performances"))
                .font(crate::theme::bold(16.0))
                .color(t.text),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let tip = l.t(
                "Close the Performance analyzer",
                "Fermer l'Analyseur de performances",
            );
            if widgets::icon_toggle(ui, Icon::Close, tip, true, false).clicked() {
                app.cmd.perf.open = false;
            }
        });
    });
    let recording = app
        .cmd
        .perf
        .results
        .as_ref()
        .and_then(|r| r.get("recording"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let can = |app: &App, id: &str| crate::ribbon::state(app, id).is_ok();
    let mut run: Option<&'static str> = None;
    ui.horizontal_wrapped(|ui| {
        if recording {
            if widgets::small_button(
                ui,
                Icon::Close,
                l.t("Stop", "Arrêter"),
                can(app, "perf.stop"),
                false,
            )
            .clicked()
            {
                run = Some("perf.stop");
            }
        } else if widgets::small_button(
            ui,
            Icon::Play,
            l.t("Start recording", "Démarrer l'enregistrement"),
            can(app, "perf.start"),
            false,
        )
        .clicked()
        {
            run = Some("perf.start");
        }
        let r = widgets::small_button(
            ui,
            Icon::Reload,
            l.t("Refresh visuals", "Actualiser les visuels"),
            recording && can(app, "perf.refresh"),
            false,
        );
        let r = if recording {
            r
        } else {
            r.on_hover_text(l.t("Start a recording first", "Démarrez d'abord un enregistrement"))
        };
        if r.clicked() {
            run = Some("perf.refresh");
        }
        if widgets::small_button(
            ui,
            Icon::Delete,
            l.t("Clear", "Effacer"),
            can(app, "perf.clear"),
            false,
        )
        .clicked()
        {
            run = Some("perf.clear");
        }
        if widgets::small_button(
            ui,
            Icon::Export,
            l.t("Export", "Exporter"),
            can(app, "perf.export"),
            false,
        )
        .clicked()
        {
            run = Some("perf.export");
        }
    });
    match run {
        Some("perf.refresh") => refresh(app, json!({})),
        Some("perf.export") => export(app),
        Some(id) => send(app, id, json!({})),
        None => {}
    }
    // the state in words, not by colour alone
    let state = if recording {
        RichText::new(format!("● {}", l.t("Recording", "Enregistrement en cours"))).color(t.err_text)
    } else {
        RichText::new(l.t("Not recording", "Pas d'enregistrement")).color(t.text_dim)
    };
    ui.label(state);
    if let Some(e) = &app.cmd.perf.error {
        ui.label(RichText::new(format!("✕ {e}")).color(t.err_text));
    }
    if let Some(Ok(p)) = &app.cmd.perf.exported {
        ui.label(RichText::new(format!("✓ {} {p}", l.t("Exported to", "Exporté vers"))).color(t.ok));
    }
    ui.add_space(4.0);
    ui.label(
        RichText::new(l.t(
            "BIcraft's own times, in milliseconds",
            "Temps propres à BIcraft, en millisecondes",
        ))
        .small()
        .color(t.text_dim),
    );
    ui.separator();
    let events: Vec<Value> = app
        .cmd
        .perf
        .results
        .as_ref()
        .and_then(|r| r.get("events"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if events.is_empty() {
        ui.label(RichText::new(l.t(
            "Start recording, then refresh the visuals or use the report: each visual BIcraft evaluates is listed here.",
            "Démarrez l'enregistrement, puis actualisez les visuels ou utilisez le rapport : chaque visuel évalué par BIcraft s'affiche ici.",
        ))
        .color(t.text_dim));
        return;
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new(l.t("Name", "Nom")).strong().color(t.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(l.t("Duration (ms)", "Durée (ms)"))
                    .strong()
                    .color(t.text),
            );
        });
    });
    egui::ScrollArea::vertical()
        .id_salt("bicraft-perf-rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // the latest first, as in Desktop
            for e in events.iter().rev() {
                row(app, ui, e, &t);
            }
        });
}

/// One visual's row: its name and total, unrolled to its parts and Copy query.
fn row(app: &mut App, ui: &mut Ui, e: &Value, t: &Tokens) {
    let l = app.lang;
    let Some(seq) = e.get("seq").and_then(Value::as_u64) else {
        return;
    };
    let name = row_name(app, e);
    let num = |k: &str| e.get(k).and_then(Value::as_f64);
    let total = num("total_ms").unwrap_or(0.0);
    let open = app.cmd.perf.expanded.contains(&seq);
    let label = clip(&name, 80);
    let mut toggle = false;
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 22.0),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.label(RichText::new(ms_text(total, l)).color(t.text));
            ui.add_space(6.0);
            // the name takes what is left, cut at its edge; a chevron says unrolled or not
            let w = ui.available_width().max(40.0);
            let (rect, r) = ui.allocate_exact_size(egui::vec2(w, 22.0), egui::Sense::click());
            let painter = ui.painter_at(rect);
            if r.hovered() {
                painter.rect_filled(rect, egui::CornerRadius::same(4), t.hover);
            }
            let icon = if open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            };
            let at = egui::Rect::from_center_size(
                egui::pos2(rect.min.x + 8.0, rect.center().y),
                egui::vec2(12.0, 12.0),
            );
            crate::icons::paint(&painter, at, icon, t.text_dim, false);
            painter.text(
                egui::pos2(rect.min.x + 18.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                &label,
                egui::TextStyle::Body.resolve(ui.style()),
                t.text,
            );
            r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, open, &label));
            toggle = r
                .on_hover_text(l.t("Show the details", "Afficher le détail"))
                .clicked();
        },
    );
    if toggle {
        if open {
            app.cmd.perf.expanded.remove(&seq);
        } else {
            app.cmd.perf.expanded.insert(seq);
        }
    }
    if !open {
        return;
    }
    egui::Frame::NONE
        .inner_margin(egui::Margin {
            left: 16,
            right: 0,
            top: 0,
            bottom: 4,
        })
        .stroke(Stroke::NONE)
        .show(ui, |ui| {
            let line = |ui: &mut Ui, k: &str, v: String| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(k).color(t.text_dim));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(v).color(t.text));
                    });
                });
            };
            line(
                ui,
                l.t("Evaluation (BIcraft)", "Évaluation (BIcraft)"),
                num("evaluation_ms").map_or_else(|| "—".into(), |x| ms_text(x, l)),
            );
            line(
                ui,
                l.t("Render", "Rendu"),
                num("render_ms").map_or_else(|| l.t("not measured", "non mesuré").into(), |x| ms_text(x, l)),
            );
            if let Some(rows) = e.get("rows").and_then(Value::as_u64) {
                line(ui, l.t("Rows", "Lignes"), rows.to_string());
            }
            let cause = match e.get("cause").and_then(Value::as_str) {
                Some("refresh") => l.t("Refresh visuals", "Actualiser les visuels"),
                _ => l.t("Report view", "Vue Rapport"),
            };
            line(ui, l.t("Cause", "Origine"), cause.into());
            if let Some(st) = e.get("status").and_then(Value::as_str).filter(|s| *s != "ok") {
                let why = e.get("reason").and_then(Value::as_str).unwrap_or_default();
                ui.label(RichText::new(format!("⚠ {st}: {}", clip(why, 300))).color(t.err_text));
            }
            match e.get("query").and_then(Value::as_str) {
                Some(q) => {
                    if ui
                        .button(l.t("Copy query", "Copier la requête"))
                        .on_hover_text(l.t(
                            "The DAX query BIcraft evaluates for this visual",
                            "La requête DAX que BIcraft évalue pour ce visuel",
                        ))
                        .clicked()
                    {
                        ui.ctx().copy_text(q.to_string());
                        app.status = l.t("Query copied", "Requête copiée").to_string();
                    }
                }
                None => {
                    ui.label(
                        RichText::new(l.t("No query text", "Pas de texte de requête")).color(t.text_dim),
                    );
                }
            }
            for n in e
                .get("query_notes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                ui.label(
                    RichText::new(format!("· {}", clip(n, 300)))
                        .small()
                        .color(t.text_dim),
                );
            }
        });
}
