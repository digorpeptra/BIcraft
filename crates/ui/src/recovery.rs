//! Auto-recovery and autosave in the window (docs/recovery.md): the prompt shown when the open
//! file has unsaved edits a crash, a closed tab or a dropped server view left (`recovery.apply`
//! or `recovery.discard`), the File view's list of recoverable files, the status bar's hint, and
//! "Save automatically" for PBIP projects. Everything runs through the engine's `recovery.*` and
//! `file.save` commands; the window keeps view state only (what was answered, the autosave clock).

use std::collections::HashSet;
use std::path::Path;

use egui::{Color32, RichText, Ui, Vec2};
use serde_json::{Value, json};

use crate::actions::Call;
use crate::app::{App, Then, clip};
use crate::backstage::fmt_date;
use crate::i18n::Lang;
use crate::prefs::{AUTOSAVE_EVERY_SECS, AUTOSAVE_IDLE_SECS, AUTOSAVE_MAX_SECS, AUTOSAVE_MIN_SECS, Autosave};
use crate::theme::{self, Tokens};
use crate::widgets;

/// Seconds between two reads of the recovery folder for the File view.
const LIST_EVERY_SECS: f64 = 5.0;
/// Shortest time between two automatic saves (a failing save is not retried at once).
const MIN_GAP_SECS: f64 = 5.0;
/// Rows of the File view's list.
const MAX_ROWS: usize = 8;

/// The window's recovery state.
#[derive(Default)]
pub struct RecoveryUi {
    /// Journals answered in the prompt (Recover, Discard or Later): not asked again here.
    pub seen: HashSet<String>,
    /// The recoverable journals for the File view, and when they were read (egui time); the
    /// revision of the session they were asked from.
    pub all: Option<Vec<Value>>,
    read_at: Option<f64>,
    asked_rev: Option<u64>,
    /// Automatic saves done, every document together (tests, the status line).
    pub saves: u64,
}

/// One document's autosave clock (each open document has its own, swapped in with it): the
/// revision seen and when it changed, since when the file has unsaved edits, the last automatic
/// save, a save in flight.
#[derive(Default)]
pub struct Clock {
    rev: Option<u64>,
    changed_at: f64,
    dirty_since: Option<f64>,
    saved_at: f64,
    pending: bool,
}

fn call(id: &'static str, params: Value) -> Call {
    Call { id, params }
}

/// The active document's snapshot, if any.
fn snap(app: &App) -> Option<&Value> {
    app.loaded.as_ref().map(|l| l.snap.as_ref())
}

/// The snapshot's offers for the open file.
pub fn offers(app: &App) -> Vec<Value> {
    snap(app)
        .and_then(|s| s.pointer("/recovery/offers"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn edits_text(l: Lang, n: u64) -> String {
    match (l, n) {
        (Lang::En, 1) => "1 edit".into(),
        (Lang::En, n) => format!("{n} edits"),
        (Lang::Fr, 1) => "1 modification".into(),
        (Lang::Fr, n) => format!("{n} modifications"),
    }
}

/// `12 edits · 2026-11-05`, plus "and a recovery copy".
fn offer_detail(l: Lang, o: &Value) -> String {
    let n = o.get("edits").and_then(Value::as_u64).unwrap_or(0);
    let at = o.get("at_ms").and_then(Value::as_u64).unwrap_or(0) / 1000;
    let copy = o.get("copy").and_then(Value::as_bool) == Some(true);
    let mut s = if n == 0 && copy {
        l.t("a recovery copy", "une copie de récupération").to_string()
    } else {
        edits_text(l, n)
    };
    if copy && n > 0 {
        s.push_str(l.t(" after a recovery copy", " après une copie de récupération"));
    }
    format!("{s} · {}", fmt_date(at))
}

/// Why an offer cannot be (or can only partly be) recovered, in the window's language.
fn offer_note(l: Lang, o: &Value) -> Option<&'static str> {
    let copy = o.get("copy").and_then(Value::as_bool) == Some(true);
    match o.get("base").and_then(Value::as_str) {
        Some("same") => None,
        Some("changed") if copy => Some(l.t(
            "The file changed since these edits were made: recovering replaces its model and report with the recovery copy.",
            "Le fichier a changé depuis ces modifications : la récupération remplace son modèle et son rapport par la copie de récupération.",
        )),
        Some("changed") => Some(l.t(
            "The file changed since these edits were made and no recovery copy was kept: they cannot be replayed safely.",
            "Le fichier a changé depuis ces modifications et aucune copie de récupération n'a été gardée : elles ne peuvent pas être rejouées sans risque.",
        )),
        _ => Some(l.t("The file is gone.", "Le fichier n'existe plus.")),
    }
}

/// The recovery prompt: the open file has edits another session left unsaved.
pub fn prompt(app: &mut App, ctx: &egui::Context) {
    if app.confirm.is_some() || app.form.is_some() || app.palette.is_some() {
        return;
    }
    let Some(offer) = offers(app).into_iter().find(|o| {
        o.get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| !app.recovery.seen.contains(id))
    }) else {
        return;
    };
    let id = offer.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let reader = app.access.as_deref() == Some("reader");
    let recoverable = offer.get("recoverable").and_then(Value::as_bool) == Some(true) && !reader;
    let label = offer
        .get("label")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("—")
        .to_string();
    let (mut recover, mut discard) = (false, false);
    let mut later = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    egui::Window::new(l.t("Recover unsaved changes", "Récupérer les modifications non enregistrées"))
        .id(egui::Id::unique("bicraft-recovery-prompt"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::new(0.0, -40.0))
        .show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.label(format!(
                "{} “{}” {}",
                l.t("BIcraft kept edits to", "BIcraft a gardé des modifications de"),
                clip(&label, 80),
                l.t(
                    "that were not saved (the app or the tab closed before a save).",
                    "qui n'ont pas été enregistrées (l'application ou l'onglet s'est fermé avant un enregistrement).",
                )
            ));
            ui.add_space(4.0);
            ui.label(RichText::new(offer_detail(l, &offer)).color(t.text_dim));
            if let Some(note) = offer_note(l, &offer) {
                ui.add_space(6.0);
                let color = if recoverable { t.text_dim } else { t.err_text };
                ui.label(RichText::new(format!("⚠ {note}")).color(color));
            }
            ui.add_space(10.0);
            let w = ui.available_width();
            ui.allocate_ui_with_layout(
                Vec2::new(w, 28.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    let why = if reader {
                        l.t("read-only access", "accès en lecture seule")
                    } else {
                        l.t("cannot be replayed", "ne peut pas être rejoué")
                    };
                    if widgets::primary(ui, l.t("Recover", "Récupérer"), recoverable, why).clicked() {
                        recover = true;
                    }
                    if ui
                        .add_enabled(
                            !reader,
                            egui::Button::new(l.t("Discard", "Abandonner")).min_size(Vec2::new(72.0, 24.0)),
                        )
                        .clicked()
                    {
                        discard = true;
                    }
                    if ui
                        .add(egui::Button::new(l.t("Later", "Plus tard")).min_size(Vec2::new(72.0, 24.0)))
                        .clicked()
                    {
                        later = true;
                    }
                },
            );
        });
    if recover {
        app.recovery.seen.insert(id.clone());
        app.run(
            call("recovery.apply", json!({"id": id})),
            Then::Recovery("recovery.apply"),
        );
    } else if discard {
        app.recovery.seen.insert(id.clone());
        app.run(
            call("recovery.discard", json!({"id": id})),
            Then::Recovery("recovery.discard"),
        );
    } else if later {
        app.recovery.seen.insert(id);
    }
}

/// The answer of a recovery command or an automatic save.
pub fn answer(app: &mut App, what: &'static str, r: Result<Value, String>) {
    let l = app.lang;
    match (what, r) {
        ("recovery.list", Ok(v)) => {
            app.recovery.all = Some(
                v.get("offers")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            );
        }
        ("recovery.list", Err(_)) => app.recovery.all = Some(Vec::new()),
        ("recovery.apply", Ok(v)) => {
            let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
            if v.get("complete").and_then(Value::as_bool) == Some(true) {
                app.status = format!(
                    "{} {}",
                    l.t("Recovered", "Récupéré :"),
                    edits_text(l, n("applied"))
                );
            } else {
                let why = v.pointer("/failed/error").and_then(Value::as_str).unwrap_or("");
                app.error = Some(format!(
                    "✕ {} {} / {}: {}",
                    l.t("Recovery stopped after", "La récupération s'est arrêtée après"),
                    n("applied"),
                    n("total"),
                    clip(why, 400)
                ));
            }
            app.recovery.all = None;
        }
        ("recovery.discard", Ok(_)) => {
            app.status = l
                .t("Unsaved edits discarded", "Modifications abandonnées")
                .to_string();
            app.recovery.all = None;
        }
        ("autosave", r) => {
            app.autosave.pending = false;
            match r {
                Ok(_) => {
                    app.recovery.saves = app.recovery.saves.saturating_add(1);
                    app.status = l
                        .t("Saved automatically", "Enregistré automatiquement")
                        .to_string();
                }
                Err(e) => {
                    app.error = Some(format!(
                        "✕ {}: {}",
                        l.t("Automatic save", "Enregistrement automatique"),
                        clip(&e, 500)
                    ));
                }
            }
        }
        (id, Err(e)) => app.error = Some(format!("✕ {id}: {}", clip(&e, 600))),
        _ => {}
    }
}

/// The File view's recoverable files: the shell's list (the desktop's recovery folder) without
/// an open document, else the active session's `recovery.list {all: true}` once per revision.
fn rows(app: &mut App, now: f64) -> Vec<Value> {
    if !app.has_doc() {
        if let Some(list) = &app.shell.recovery_list {
            let stale = app.recovery.read_at.is_none_or(|t| now - t >= LIST_EVERY_SECS);
            if stale || app.recovery.all.is_none() {
                app.recovery.all = Some(list());
                app.recovery.read_at = Some(now);
            }
        }
        return app.recovery.all.clone().unwrap_or_default();
    }
    let rev = snap(app).and_then(|s| s.get("revision")).and_then(Value::as_u64);
    let enabled = snap(app).is_some_and(|s| s.get("recovery").is_some_and(Value::is_object));
    if enabled && rev.is_some() && app.recovery.asked_rev != rev {
        app.recovery.asked_rev = rev;
        app.run(
            call("recovery.list", json!({"all": true})),
            Then::Recovery("recovery.list"),
        );
    }
    app.recovery.all.clone().unwrap_or_default()
}

/// The File view's Home section: files with unsaved edits left by earlier sessions (nothing
/// when there are none). Open opens the file; the prompt then offers to recover. A model never
/// saved has no file to open: its Recover replaces the open document's content with it (as New
/// model does), so it needs an open document without unsaved edits.
pub fn home_section(app: &mut App, ui: &mut Ui) {
    let now = ui.ctx().input(|i| i.time);
    let rows = rows(app, now);
    if rows.is_empty() {
        return;
    }
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    ui.label(
        RichText::new(l.t(
            "Recover unsaved changes",
            "Récupérer les modifications non enregistrées",
        ))
        .font(theme::bold(16.0))
        .color(t.text),
    );
    ui.add_space(2.0);
    ui.label(
        RichText::new(l.t(
            "Edits a crash or a closed window left unsaved. Open the file to recover or discard them.",
            "Des modifications non enregistrées laissées par un arrêt ou une fenêtre fermée. Ouvrez le fichier pour les récupérer ou les abandonner.",
        ))
        .small()
        .color(t.text_dim),
    );
    ui.add_space(6.0);
    let mut open: Option<String> = None;
    let mut recover: Option<String> = None;
    let reader = app.access.as_deref() == Some("reader");
    let can_recover_new = app.loaded.is_some() && !reader;
    for (i, o) in rows.iter().take(MAX_ROWS).enumerate() {
        let untitled = o.get("base").and_then(Value::as_str) == Some("untitled");
        let path = if untitled {
            l.t("New model, never saved", "Nouveau modèle, jamais enregistré")
        } else {
            o.get("path").and_then(Value::as_str).unwrap_or("")
        };
        let label = o
            .get("label")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(path);
        let gone = o.get("base").and_then(Value::as_str) == Some("missing");
        ui.push_id(("bicraft-recovery-row", i), |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width() - 90.0);
                    ui.label(
                        RichText::new(clip(label, 80))
                            .font(theme::bold(13.0))
                            .color(t.text),
                    );
                    ui.label(
                        RichText::new(format!("{} · {}", clip(path, 120), offer_detail(l, o)))
                            .small()
                            .color(t.text_dim),
                    );
                });
                if untitled {
                    let ok = can_recover_new && o.get("recoverable").and_then(Value::as_bool) == Some(true);
                    let why = if reader {
                        l.t("Read-only access.", "Accès en lecture seule.")
                    } else {
                        l.t(
                            "Open a file first: the recovered model replaces it in its tab, as New model does.",
                            "Ouvrez d'abord un fichier : le modèle récupéré le remplace dans son onglet, comme Nouveau modèle.",
                        )
                    };
                    if ui
                        .add_enabled(ok, egui::Button::new(l.t("Recover", "Récupérer")))
                        .on_disabled_hover_text(why)
                        .clicked()
                    {
                        recover = o.get("id").and_then(Value::as_str).map(str::to_string);
                    }
                } else if ui
                    .add_enabled(
                        !gone && !path.is_empty(),
                        egui::Button::new(l.t("Open", "Ouvrir")),
                    )
                    .clicked()
                {
                    open = Some(path.to_string());
                }
            });
        });
        ui.add_space(4.0);
    }
    ui.add_space(14.0);
    if let Some(id) = recover {
        app.recovery.seen.insert(id.clone());
        app.run(
            call("recovery.apply", json!({"id": id})),
            Then::Recovery("recovery.apply"),
        );
        app.close_backstage_now = true;
    }
    if let Some(p) = open {
        if app.shell.workspace.is_some() && !app.shell.caps.open_files {
            app.open_ws_file(&p);
        } else {
            app.open_path(Path::new(&p));
        }
        app.close_backstage_now = true;
    }
}

/// The status bar's hint: how many unsaved edits the journal keeps (or that it failed).
pub fn status_pill(app: &App) -> Option<(String, Color32, String)> {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let r = snap(app)?.get("recovery").filter(|r| r.is_object())?;
    if let Some(e) = r.get("error").and_then(Value::as_str) {
        return Some((
            l.t("Recovery journal: error", "Journal de récupération : erreur")
                .to_string(),
            t.warn,
            clip(e, 300),
        ));
    }
    let n = r.get("edits").and_then(Value::as_u64).unwrap_or(0);
    let copy = r.get("copy").and_then(Value::as_bool) == Some(true);
    if n == 0 && !copy {
        return None;
    }
    Some((
        format!("{} {}", l.t("Recovery:", "Récupération :"), edits_text(l, n.max(1))),
        t.ok,
        l.t(
            "Unsaved edits are journaled on this machine: after a crash, opening the file offers them back.",
            "Les modifications non enregistrées sont journalisées sur cette machine : après un arrêt, rouvrir le fichier les propose.",
        )
        .to_string(),
    ))
}

/// Why autosave does not apply to the active document (`None`: it does).
pub fn autosave_refusal(app: &App) -> Option<&'static str> {
    let l = app.lang;
    let s = snap(app)?;
    if s.pointer("/view/limits/autosave").and_then(Value::as_bool) == Some(false) {
        return Some(l.t(
            "This server does not allow automatic saves (bicraft-cli serve --allow-autosave).",
            "Ce serveur n'autorise pas l'enregistrement automatique (bicraft-cli serve --allow-autosave).",
        ));
    }
    if app.access.as_deref() == Some("reader") {
        return Some(l.t("Read-only access.", "Accès en lecture seule."));
    }
    match s.pointer("/source/kind").and_then(Value::as_str) {
        Some("project") => None,
        Some("package") => Some(l.t(
            "Not offered for PBIX and PBIT files: every save repacks the whole file, which takes seconds to minutes on large files. Unsaved edits are still kept for recovery.",
            "Pas proposé pour les fichiers PBIX et PBIT : chaque enregistrement réécrit tout le fichier, ce qui prend des secondes, voire des minutes, pour un gros fichier. Les modifications non enregistrées restent gardées pour la récupération.",
        )),
        _ => Some(l.t(
            "Offered for PBIP projects only.",
            "Proposé pour les projets PBIP seulement.",
        )),
    }
}

/// The File view's Info section: "Save automatically" and the journal's state.
pub fn info_section(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    ui.add_space(14.0);
    ui.label(
        RichText::new(l.t("Save automatically", "Enregistrer automatiquement"))
            .font(theme::bold(14.0))
            .color(t.text),
    );
    let mut mode = app.prefs.autosave;
    let (mut every, mut idle) = (AUTOSAVE_EVERY_SECS / 60, AUTOSAVE_IDLE_SECS);
    match mode {
        Autosave::Every(s) => every = (s / 60).max(1),
        Autosave::Idle(s) => idle = s,
        Autosave::Off => {}
    }
    ui.horizontal_wrapped(|ui| {
        if ui
            .radio(matches!(mode, Autosave::Off), l.t("Off", "Non"))
            .clicked()
        {
            mode = Autosave::Off;
        }
        if ui
            .radio(matches!(mode, Autosave::Every(_)), l.t("Every", "Toutes les"))
            .clicked()
        {
            mode = Autosave::Every(every.saturating_mul(60));
        }
        let max_min = AUTOSAVE_MAX_SECS / 60;
        if ui
            .add(egui::DragValue::new(&mut every).range(1..=max_min).suffix(" min"))
            .changed()
            && matches!(mode, Autosave::Every(_))
        {
            mode = Autosave::Every(every.saturating_mul(60));
        }
        if ui
            .radio(matches!(mode, Autosave::Idle(_)), l.t("After", "Après"))
            .clicked()
        {
            mode = Autosave::Idle(idle);
        }
        if ui
            .add(
                egui::DragValue::new(&mut idle)
                    .range(AUTOSAVE_MIN_SECS..=AUTOSAVE_MAX_SECS)
                    .suffix(l.t(" s without an edit", " s sans modification")),
            )
            .changed()
            && matches!(mode, Autosave::Idle(_))
        {
            mode = Autosave::Idle(idle);
        }
    });
    if mode != app.prefs.autosave {
        app.prefs.autosave = mode;
        app.save_prefs();
    }
    if let Some(why) = autosave_refusal(app) {
        ui.label(RichText::new(why).small().color(t.text_dim));
    }
    let journal = snap(app)
        .and_then(|s| s.get("recovery"))
        .filter(|r| r.is_object());
    let text = match journal {
        None => l
            .t(
                "Auto-recovery is off for this file: unsaved edits are lost if the app closes.",
                "La récupération automatique est désactivée pour ce fichier : les modifications non enregistrées sont perdues si l'application se ferme.",
            )
            .to_string(),
        Some(r) => {
            let n = r.get("edits").and_then(Value::as_u64).unwrap_or(0);
            format!(
                "{} ({})",
                l.t(
                    "Unsaved edits are journaled for recovery",
                    "Les modifications non enregistrées sont journalisées pour la récupération",
                ),
                edits_text(l, n)
            )
        }
    };
    ui.label(RichText::new(text).small().color(t.text_dim));
}

/// Autosave: saves the active document's PBIP project when the preference's delay has passed,
/// through the ordinary `file.save` (with the server's conflict checks). Never two saves of one
/// document in flight. The window runs it for every open document in turn, each swapped in with
/// its own clock ([`App::logic`]).
pub fn tick(app: &mut App, ctx: &egui::Context) {
    let mode = app.prefs.autosave;
    if mode == Autosave::Off || app.autosave.pending || autosave_refusal(app).is_some() {
        return;
    }
    let Some(s) = snap(app) else { return };
    let now = ctx.input(|i| i.time);
    let rev = s.get("revision").and_then(Value::as_u64);
    let dirty = ["model", "report", "queries"]
        .iter()
        .any(|k| s.pointer(&format!("/dirty/{k}")).and_then(Value::as_bool) == Some(true));
    let r = &mut app.autosave;
    if r.rev != rev {
        r.rev = rev;
        r.changed_at = now;
    }
    if !dirty {
        r.dirty_since = None;
        return;
    }
    let since = *r.dirty_since.get_or_insert(now);
    let (due, wait) = match mode {
        Autosave::Every(secs) => {
            let from = since.max(r.saved_at);
            (now - from >= f64::from(secs), f64::from(secs) - (now - from))
        }
        Autosave::Idle(secs) => (
            now - r.changed_at >= f64::from(secs),
            f64::from(secs) - (now - r.changed_at),
        ),
        Autosave::Off => (false, 0.0),
    };
    if due && now - r.saved_at >= MIN_GAP_SECS {
        r.saved_at = now;
        r.pending = true;
        app.run(crate::actions::file_save(), Then::Recovery("autosave"));
    } else {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.clamp(0.2, 60.0)));
    }
}

/// The active document is closed without saving: its own journal goes (`recovery.discard
/// {current: true}`), when its session keeps one.
pub fn discard_current(app: &mut App) {
    let journaled = snap(app).is_some_and(|s| s.get("recovery").is_some_and(Value::is_object));
    if journaled && app.access.as_deref() != Some("reader") {
        app.run(call("recovery.discard", json!({"current": true})), Then::Nothing);
    }
}
