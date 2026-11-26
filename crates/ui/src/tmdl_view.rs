//! The TMDL view (Power BI Desktop's TMDL view, as Microsoft Learn's "Work with TMDL view in
//! Power BI Desktop" page describes it): script tabs along the bottom (add, rename, duplicate,
//! close), a code editor, Script model and the Data pane's or the Model explorer's Script TMDL
//! (`tmdl.script`: the script fills the tab shown when it is empty, else a new tab), and Apply
//! (`tmdl.apply`: the script's `createOrReplace`, one undo step). A failed Apply says why with
//! the script's line and column, and Go to puts the cursor there; nothing is applied then.
//!
//! The tabs and their text are pure view state, kept per document ([`crate::ribbon::DocUi`]);
//! only Script and Apply reach the engine.

use egui::text::{CCursor, CCursorRange};
use egui::{CornerRadius, Id, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::App;
use crate::actions::Call;
use crate::app::{Tab as View, Then};
use crate::i18n::Lang;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::widgets;

/// Script TMDL (the model, or the objects picked).
pub const SCRIPT: &str = "tmdl.script";
/// Apply.
pub const APPLY: &str = "tmdl.apply";
/// Most script tabs of one document.
pub const MAX_TABS: usize = 64;
/// Longest tab name, in characters.
const MAX_NAME: usize = 100;
/// Height of the tab strip and of the tool bar.
const TABS_H: f32 = 32.0;
const BAR_H: f32 = 40.0;

/// What an answer is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// A script of model objects, for the tab shown when it is empty, else a new one.
    Script,
    /// Apply of the tab (by id).
    Apply(u64),
}

/// The answer of the last Apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub ok: bool,
    pub text: String,
    /// Where the error is in the script (line, column, from 1).
    pub at: Option<(usize, usize)>,
}

/// One script tab.
#[derive(Debug, Clone)]
pub struct ScriptTab {
    pub id: u64,
    pub name: String,
    pub text: String,
    pub applying: bool,
    pub note: Option<Note>,
}

/// The view's state for one document.
#[derive(Debug, Clone, Default)]
pub struct TmdlView {
    pub tabs: Vec<ScriptTab>,
    pub active: usize,
    next: u64,
    /// The tab being renamed and the name typed.
    pub renaming: Option<(u64, String)>,
    /// Give the editor the focus on the next frame.
    pub focus: bool,
    /// Put the editor's cursor at this character on the next frame.
    goto: Option<usize>,
}

impl TmdlView {
    /// Makes a first tab when there is none and keeps `active` on a tab.
    pub fn ensure(&mut self, l: Lang) {
        if self.tabs.is_empty() {
            self.add(l, String::new());
        }
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
    }

    /// The tab shown, if any.
    pub fn active_tab(&self) -> Option<&ScriptTab> {
        self.tabs.get(self.active)
    }

    /// A new tab `Script N` (the first free N) with `text`, shown; none past [`MAX_TABS`].
    pub fn add(&mut self, l: Lang, text: String) -> bool {
        if self.tabs.len() >= MAX_TABS {
            return false;
        }
        let prefix = l.t("Script", "Script");
        let name = (1..=MAX_TABS + 1)
            .map(|n| format!("{prefix} {n}"))
            .find(|n| !self.tabs.iter().any(|t| t.name.eq_ignore_ascii_case(n)))
            .unwrap_or_else(|| prefix.to_string());
        self.next = self.next.wrapping_add(1);
        self.tabs.push(ScriptTab {
            id: self.next,
            name,
            text,
            applying: false,
            note: None,
        });
        self.active = self.tabs.len() - 1;
        self.focus = true;
        true
    }

    /// A script: in the tab shown when its text is blank, else in a new tab (Desktop's rule).
    pub fn put(&mut self, l: Lang, text: String) -> bool {
        self.ensure(l);
        match self.tabs.get_mut(self.active) {
            Some(t) if t.text.trim().is_empty() => {
                t.text = text;
                t.note = None;
                self.focus = true;
                true
            }
            _ => self.add(l, text),
        }
    }

    /// Closes the tab `id`; the last one closed leaves a new empty tab.
    pub fn close(&mut self, id: u64, l: Lang) {
        let Some(i) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        self.tabs.remove(i);
        if self.active > i || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        if self.tabs.is_empty() {
            self.add(l, String::new());
        }
    }

    /// A copy of tab `id` after the others (its text, not its answer).
    pub fn duplicate(&mut self, id: u64, l: Lang) {
        let Some(text) = self.tabs.iter().find(|t| t.id == id).map(|t| t.text.clone()) else {
            return;
        };
        self.add(l, text);
    }

    /// Renames tab `id` (a blank name or one another tab has is refused).
    pub fn rename(&mut self, id: u64, name: &str) -> bool {
        let name: String = name.trim().chars().take(MAX_NAME).collect();
        if name.is_empty()
            || self
                .tabs
                .iter()
                .any(|t| t.id != id && t.name.eq_ignore_ascii_case(&name))
        {
            return false;
        }
        match self.tabs.iter_mut().find(|t| t.id == id) {
            Some(t) => {
                t.name = name;
                true
            }
            None => false,
        }
    }

    fn by_id(&mut self, id: u64) -> Option<&mut ScriptTab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }
}

/// `line L, column C` in an error (the first one), from 1.
pub fn position(e: &str) -> Option<(usize, usize)> {
    let at = e.find("line ")?;
    let rest = e.get(at + 5..)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let line = digits.parse().ok()?;
    let col = rest
        .get(digits.len()..)
        .and_then(|r| r.strip_prefix(", column "))
        .map(|r| r.chars().take_while(char::is_ascii_digit).collect::<String>())
        .and_then(|d| d.parse().ok())
        .unwrap_or(1);
    Some((line, col))
}

/// The character index of `line`:`col` (from 1) in `text`, clamped to the text.
pub fn char_at(text: &str, line: usize, col: usize) -> usize {
    let mut chars = 0usize;
    for (i, l) in text.split('\n').enumerate() {
        let n = l.chars().count();
        if i + 1 == line {
            return chars + col.saturating_sub(1).min(n);
        }
        chars += n + 1;
    }
    text.chars().count()
}

/// Script TMDL and Apply, from the ribbon, the palette and the menus: Script fills the view (the
/// whole model without objects), Apply without a script applies the tab shown.
pub fn intercept(app: &mut App, id: &str, params: &Value) -> bool {
    match id {
        SCRIPT => {
            app.show_tab(View::Tmdl);
            app.run(
                Call {
                    id: SCRIPT,
                    params: params.clone(),
                },
                Then::Tmdl(Step::Script),
            );
            true
        }
        APPLY if params.get("script").is_none() => {
            app.show_tab(View::Tmdl);
            apply(app);
            true
        }
        _ => false,
    }
}

/// Applies the tab shown (nothing for an empty script, with a word in the status bar).
pub fn apply(app: &mut App) {
    let l = app.lang;
    app.cmd.tmdl.ensure(l);
    let v = &mut app.cmd.tmdl;
    let Some(tab) = v.tabs.get_mut(v.active) else {
        return;
    };
    if tab.text.trim().is_empty() {
        app.status = l
            .t(
                "Script objects or write a TMDL script first.",
                "Scriptez d'abord des objets ou écrivez un script TMDL.",
            )
            .to_string();
        return;
    }
    let (id, script) = (tab.id, tab.text.clone());
    tab.applying = true;
    tab.note = None;
    let sent = app.run(
        Call {
            id: APPLY,
            params: json!({"script": script}),
        },
        Then::Tmdl(Step::Apply(id)),
    );
    if !sent && let Some(t) = app.cmd.tmdl.by_id(id) {
        t.applying = false;
    }
}

/// The answer of a Script or an Apply.
pub fn answer(app: &mut App, step: Step, r: Result<Value, String>) {
    let l = app.lang;
    match step {
        Step::Script => match r {
            Ok(v) => {
                let text = v.get("script").and_then(Value::as_str).unwrap_or("").to_string();
                if !app.cmd.tmdl.put(l, text) {
                    app.status = l
                        .t("Close a script tab first.", "Fermez d'abord un onglet de script.")
                        .to_string();
                }
            }
            Err(e) => app.status = crate::app::clip(&e, 300),
        },
        Step::Apply(id) => {
            let note = match r {
                Ok(v) => Note {
                    ok: true,
                    text: applied_text(&v, l),
                    at: None,
                },
                Err(e) => Note {
                    ok: false,
                    at: position(&e),
                    text: crate::app::clip(&e, 2000),
                },
            };
            if note.ok {
                app.status = note.text.clone();
            }
            if let Some(t) = app.cmd.tmdl.by_id(id) {
                t.applying = false;
                t.note = Some(note);
            }
        }
    }
}

/// `Applied: 1 added, 2 replaced.` in the window's language, with the warnings' count.
fn applied_text(v: &Value, l: Lang) -> String {
    let list = v.get("applied").and_then(Value::as_array);
    let count = |a: &str| {
        list.into_iter()
            .flatten()
            .filter(|x| x.get("action").and_then(Value::as_str) == Some(a))
            .count()
    };
    let (added, replaced) = (count("added"), count("replaced"));
    let warnings = v.get("warnings").and_then(Value::as_array).map_or(0, Vec::len);
    let mut s = match l {
        Lang::En => format!("Script applied: {added} added, {replaced} replaced."),
        Lang::Fr => format!("Script appliqué : {added} ajouté(s), {replaced} remplacé(s)."),
    };
    if warnings > 0 {
        s.push_str(&match l {
            Lang::En => format!(" {warnings} DAX warning(s)."),
            Lang::Fr => format!(" {warnings} avertissement(s) DAX."),
        });
    }
    s
}

/// Ctrl+Enter applies the tab shown, in the view, even while the editor has the focus.
pub fn keys(app: &mut App, ui: &Ui) -> bool {
    if app.tab != View::Tmdl || app.form.is_some() || app.palette.is_some() || app.confirm.is_some() {
        return false;
    }
    let hit = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
    if hit {
        apply(app);
    }
    hit
}

// --- drawing --------------------------------------------------------------------------------

/// What a click asked for (done after drawing).
enum Act {
    None,
    Apply,
    ScriptModel,
    Copy,
}

/// The view: the tool bar, the editor, the last Apply's answer, the tab strip.
pub fn show(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    app.cmd.tmdl.ensure(l);
    let rect = ui.max_rect();
    let bar = Rect::from_min_size(rect.min, vec2(rect.width(), BAR_H));
    let strip = Rect::from_min_max(pos2(rect.min.x, rect.max.y - TABS_H), rect.max);
    let note_h = if app.cmd.tmdl.active_tab().is_some_and(|q| q.note.is_some()) {
        64.0
    } else {
        0.0
    };
    let editor = Rect::from_min_max(
        pos2(rect.min.x + 12.0, bar.max.y + 8.0),
        pos2(
            (rect.max.x - 12.0).max(rect.min.x + 52.0),
            (strip.min.y - 8.0 - note_h).max(bar.max.y + 48.0),
        ),
    );
    let mut act = Act::None;
    ui.painter().rect_filled(bar, CornerRadius::ZERO, t.chrome);
    ui.painter()
        .hline(bar.x_range(), bar.max.y - 0.5, Stroke::new(1.0, t.separator));
    let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(12.0, 6.0))));
    tool_bar(app, &mut bui, &mut act);
    let mut eui = ui.new_child(egui::UiBuilder::new().max_rect(editor));
    editor_ui(app, &mut eui, editor);
    if note_h > 0.0 {
        let r = Rect::from_min_max(
            pos2(editor.min.x, editor.max.y + 6.0),
            pos2(editor.max.x, strip.min.y - 4.0),
        );
        let mut nui = ui.new_child(egui::UiBuilder::new().max_rect(r));
        note_ui(app, &mut nui);
    }
    ui.painter().rect_filled(strip, CornerRadius::ZERO, t.chrome);
    ui.painter()
        .hline(strip.x_range(), strip.min.y + 0.5, Stroke::new(1.0, t.separator));
    let mut sui = ui.new_child(egui::UiBuilder::new().max_rect(strip.shrink2(vec2(8.0, 2.0))));
    tab_strip(app, &mut sui);
    match act {
        Act::None => {}
        Act::Apply => apply(app),
        Act::ScriptModel => crate::ribbon::invoke(app, SCRIPT, json!({})),
        Act::Copy => {
            if let Some(q) = app.cmd.tmdl.active_tab() {
                ui.ctx().copy_text(q.text.clone());
                app.status = l.t("Script copied.", "Script copié.").to_string();
            }
        }
    }
}

/// Apply, Script model, Copy, and the applying mark.
fn tool_bar(app: &mut App, ui: &mut Ui, act: &mut Act) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let apply_ok = crate::ribbon::state(app, APPLY);
    let script_ok = crate::ribbon::state(app, SCRIPT);
    let empty = app.cmd.tmdl.active_tab().is_none_or(|q| q.text.trim().is_empty());
    let applying = app.cmd.tmdl.active_tab().is_some_and(|q| q.applying);
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let why = match &apply_ok {
            Err(e) => e.clone(),
            Ok(()) => l
                .t(
                    "Script objects or write a TMDL script first",
                    "Scriptez d'abord des objets ou écrivez un script TMDL",
                )
                .to_string(),
        };
        let r = widgets::primary(ui, l.t("Apply", "Appliquer"), apply_ok.is_ok() && !empty, &why);
        if r.on_hover_text(l.t(
            "Create or replace the script's objects in the model, in one step (Ctrl+Enter)",
            "Créer ou remplacer les objets du script dans le modèle, en une étape (Ctrl+Entrée)",
        ))
        .clicked()
        {
            *act = Act::Apply;
        }
        let r = widgets::small_button(
            ui,
            Icon::File,
            l.t("Script model", "Scripter le modèle"),
            script_ok.is_ok(),
            false,
        );
        let tip = match &script_ok {
            Err(e) => e.clone(),
            Ok(()) => l
                .t(
                    "The whole semantic model as a TMDL script",
                    "Tout le modèle sémantique en script TMDL",
                )
                .to_string(),
        };
        if r.on_hover_text(tip).clicked() && script_ok.is_ok() {
            *act = Act::ScriptModel;
        }
        let r = widgets::small_button(ui, Icon::Copy, l.t("Copy", "Copier"), !empty, false);
        if r.on_hover_text(l.t("Copy the script", "Copier le script"))
            .clicked()
            && !empty
        {
            *act = Act::Copy;
        }
        if applying {
            ui.spinner();
            ui.label(RichText::new(l.t("Applying…", "Application…")).color(t.text_dim));
        }
    });
}

/// The editor of the tab shown, in the code font.
fn editor_ui(app: &mut App, ui: &mut Ui, rect: Rect) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let ctx = ui.ctx().clone();
    let v = &mut app.cmd.tmdl;
    let Some(tab_id) = v.active_tab().map(|q| q.id) else {
        return;
    };
    let id = Id::unique(("bicraft-tmdl-editor", tab_id));
    if v.focus {
        ctx.memory_mut(|m| m.request_focus(id));
        v.focus = false;
    }
    let goto = v.goto.take();
    let focused = ctx.memory(|m| m.has_focus(id));
    let Some(tab) = v.tabs.get_mut(v.active) else {
        return;
    };
    egui::Frame::NONE
        .fill(t.field_bg)
        .stroke(Stroke::new(1.0, if focused { t.ring } else { t.border }))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.set_min_size(rect.size() - vec2(14.0, 14.0));
            egui::ScrollArea::both()
                .id_salt(("bicraft-tmdl-scroll", tab_id))
                .max_height(rect.height() - 14.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let o = egui::TextEdit::multiline(&mut tab.text)
                        .id(id)
                        .font(crate::formula::mono())
                        .code_editor()
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY)
                        .desired_rows(12)
                        .hint_text(l.t(
                            "Script TMDL from the Data pane or the Model explorer, or write createOrReplace and the objects under it",
                            "Scriptez du TMDL depuis le volet Données ou l'Explorateur de modèle, ou écrivez createOrReplace et les objets dessous",
                        ))
                        .show(ui);
                    o.response.response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::TextInput, true, l.t("TMDL script", "Script TMDL"))
                    });
                    if let Some(c) = goto {
                        let mut st = o.state.clone();
                        st.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(c))));
                        st.store(ui.ctx(), id);
                    }
                });
        });
}

/// The last Apply's answer: done, or the error with Go to its line and column.
fn note_ui(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let v = &mut app.cmd.tmdl;
    let Some(tab) = v.tabs.get(v.active) else {
        return;
    };
    let Some(note) = tab.note.clone() else {
        return;
    };
    let mut go = None;
    egui::Frame::NONE
        .fill(if note.ok { t.card } else { t.err_soft })
        .stroke(Stroke::new(1.0, if note.ok { t.card_border } else { t.err_soft }))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                let (icon, ink, word, color) = if note.ok {
                    (Icon::Check, t.ok, l.t("Done", "Terminé"), t.text)
                } else {
                    (Icon::Warning, t.err, l.t("Error", "Erreur"), t.err_text)
                };
                let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                icons::paint(ui.painter(), r, icon, ink, false);
                ui.label(RichText::new(word).font(theme::bold(13.0)).color(color));
                if let Some((line, col)) = note.at {
                    let label = match l {
                        Lang::En => format!("Go to line {line}, column {col}"),
                        Lang::Fr => format!("Aller à la ligne {line}, colonne {col}"),
                    };
                    if ui.link(RichText::new(label).color(color)).clicked() {
                        go = Some((line, col));
                    }
                }
                ui.add(egui::Label::new(RichText::new(&note.text).color(color)).wrap());
            });
        });
    if let Some((line, col)) = go {
        v.goto = Some(char_at(&tab.text, line, col));
        v.focus = true;
    }
}

/// What the tab strip asked for (done after drawing).
#[derive(Default)]
struct StripAct {
    select: Option<usize>,
    close: Option<u64>,
    duplicate: Option<u64>,
    /// Start renaming, or the name being typed.
    renaming: Option<(u64, String)>,
    commit: Option<(u64, String)>,
    cancel: bool,
    add: bool,
}

/// The script tabs: a click shows one, a double click or its menu renames it, its menu
/// duplicates or closes it, × closes it, + adds one.
fn tab_strip(app: &mut App, ui: &mut Ui) {
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let mut a = StripAct::default();
    let v = &app.cmd.tmdl;
    egui::ScrollArea::horizontal()
        .id_salt("bicraft-tmdl-tabs")
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (i, q) in v.tabs.iter().enumerate() {
                    if let Some((rid, name)) = &v.renaming
                        && *rid == q.id
                    {
                        let mut name = name.clone();
                        let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(140.0));
                        let r = r.on_hover_text(l.t(
                            "Enter to rename, Esc to cancel",
                            "Entrée pour renommer, Échap pour annuler",
                        ));
                        r.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::TextInput,
                                true,
                                l.t("Script name", "Nom du script"),
                            )
                        });
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            a.cancel = true;
                        } else if r.lost_focus() {
                            a.commit = Some((q.id, name));
                        } else {
                            if !r.has_focus() {
                                r.request_focus();
                            }
                            a.renaming = Some((q.id, name));
                        }
                        continue;
                    }
                    let r = widgets::tab(ui, &crate::app::clip(&q.name, 40), i == v.active, false);
                    if r.double_clicked() {
                        a.renaming = Some((q.id, q.name.clone()));
                    } else if r.clicked() {
                        a.select = Some(i);
                    }
                    crate::keyboard::context_menu(&r, |ui| {
                        if ui.button(l.t("Rename", "Renommer")).clicked() {
                            a.renaming = Some((q.id, q.name.clone()));
                            ui.close();
                        }
                        if ui.button(l.t("Duplicate", "Dupliquer")).clicked() {
                            a.duplicate = Some(q.id);
                            ui.close();
                        }
                        if ui.button(l.t("Close", "Fermer")).clicked() {
                            a.close = Some(q.id);
                            ui.close();
                        }
                    });
                    let (cr, cresp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                    if cresp.hovered() {
                        ui.painter().rect_filled(cr, CornerRadius::same(4), t.hover);
                    }
                    icons::paint(ui.painter(), cr.shrink(4.0), Icon::Close, t.text_dim, false);
                    let tip = match l {
                        Lang::En => format!("Close {}", q.name),
                        Lang::Fr => format!("Fermer {}", q.name),
                    };
                    cresp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &tip));
                    if cresp.on_hover_text(tip.clone()).clicked() {
                        a.close = Some(q.id);
                    }
                    ui.add_space(8.0);
                }
                let full = v.tabs.len() >= MAX_TABS;
                let sense = if full { Sense::hover() } else { Sense::click() };
                let (pr, presp) = ui.allocate_exact_size(vec2(24.0, 22.0), sense);
                if presp.hovered() && !full {
                    ui.painter().rect_filled(pr, CornerRadius::same(4), t.hover);
                }
                icons::paint(ui.painter(), pr.shrink(5.0), Icon::Plus, t.text, full);
                let tip = l.t("New script", "Nouveau script");
                presp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, !full, tip));
                if presp.on_hover_text(tip).clicked() && !full {
                    a.add = true;
                }
            });
        });
    let v = &mut app.cmd.tmdl;
    if let Some((id, name)) = a.commit {
        v.renaming = None;
        if !v.rename(id, &name) {
            app.status = l
                .t(
                    "A script tab needs a name that no other tab has.",
                    "Un onglet de script a besoin d'un nom qu'aucun autre onglet n'a.",
                )
                .to_string();
        }
    } else if a.cancel {
        v.renaming = None;
    } else if a.renaming.is_some() {
        v.renaming = a.renaming;
    }
    let v = &mut app.cmd.tmdl;
    if let Some(i) = a.select {
        v.active = i;
        v.focus = true;
    }
    if let Some(id) = a.duplicate {
        v.duplicate(id, l);
    }
    if let Some(id) = a.close {
        v.close(id, l);
    }
    if a.add {
        v.add(l, String::new());
    }
}
