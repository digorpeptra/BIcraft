//! The command palette (Ctrl+Shift+P, DeckCraft's `menus::palette`; ROADMAP M11.2): a searchable
//! list of every engine command of the UI's registry ([`crate::registry::COMMANDS`], the same
//! table in both shells), with its place in the app and its shortcut, the recent ones first.
//! Commands that cannot run now are greyed with the reason ([`crate::ribbon::state`], the same
//! answer the ribbon greys its buttons with). Enter runs the selected command as its button
//! does ([`crate::ribbon::invoke`]: the File view's flows, the parameters form when values are
//! missing, else the command); Shift+Enter always opens its form ([`crate::form`]).

use egui::{Align2, Color32, CornerRadius, Key, RichText, Sense, Vec2};

use crate::App;
use crate::i18n::Lang;
use crate::registry::{self, Cmd};
use crate::ribbon;
use crate::theme::{self, Tokens};

/// Most rows the palette shows: above the command count, so an empty search lists every command
/// (`the_palette_searches_and_runs_with_the_keyboard` checks it).
const MAX_ROWS: usize = 500;

/// The palette's state while it is open.
#[derive(Debug, Clone, Default)]
pub struct Palette {
    pub query: String,
    /// The selected row.
    pub sel: usize,
    focus: bool,
    scroll: bool,
}

/// One row of the palette.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: &'static str,
    /// The button's label in the window's language.
    pub label: &'static str,
    /// `Model › Relationships`.
    pub path: String,
    pub shortcut: Option<&'static str>,
    /// `Err`: why it cannot run now.
    pub state: Result<(), String>,
    pub recent: bool,
}

// --- search ---------------------------------------------------------------------------------

/// Lower case without accents (`Modèle` → `modele`), for matching.
pub fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'â' | 'ä' | 'á' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' | 'í' => 'i',
            'ô' | 'ö' | 'ó' => 'o',
            'ù' | 'û' | 'ü' | 'ú' => 'u',
            'ç' => 'c',
            'ÿ' => 'y',
            '_' | '.' | '›' => ' ',
            c => c,
        })
        .collect()
}

/// How well the folded word `q` matches the folded `text`: a substring (better at the start of a
/// word, better early), else the letters in order (better when they start words or follow each
/// other); `None`: no match.
pub fn score_word(q: &str, text: &str) -> Option<i32> {
    if q.is_empty() {
        return Some(0);
    }
    if let Some(pos) = text.find(q) {
        let word_start = pos == 0 || text.get(..pos).is_some_and(|s| s.ends_with(' '));
        let pos = i32::try_from(pos).unwrap_or(i32::MAX).min(200);
        return Some(1000 - pos + if word_start { 300 } else { 0 });
    }
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0usize;
    let mut score = 300i32;
    let mut last: Option<usize> = None;
    for qc in q.chars() {
        let found = (i..chars.len()).find(|&j| chars.get(j) == Some(&qc))?;
        let starts = found == 0 || chars.get(found - 1) == Some(&' ');
        if starts {
            score += 15;
        }
        match last {
            Some(l) if l + 1 == found => score += 10,
            Some(l) => score -= i32::try_from(found - l).unwrap_or(100).min(20),
            None => score -= i32::try_from(found).unwrap_or(100).min(30),
        }
        last = Some(found);
        i = found + 1;
    }
    Some(score.max(1))
}

/// How well `query` matches a command: every word must match its label (in either language),
/// its id, its engine label or its place; `None`: it does not.
pub fn score(query: &str, c: &Cmd, lang: Lang) -> Option<i32> {
    let other = if lang == Lang::En { Lang::Fr } else { Lang::En };
    let fields = [
        (fold(c.text(lang)), 4),
        (fold(c.text(other)), 3),
        (fold(c.id), 3),
        (fold(c.label), 2),
        (fold(&c.place_path(lang)), 1),
        (fold(&c.place_path(other)), 1),
    ];
    let mut total = 0i32;
    for w in fold(query).split_whitespace() {
        let best = fields
            .iter()
            .filter_map(|(text, weight)| score_word(w, text).map(|s| s * weight))
            .max()?;
        total = total.saturating_add(best);
    }
    Some(total)
}

/// The ids the palette lists with an empty search: every command of the registry (the coverage
/// test `every_command_has_a_button` reads it).
pub fn entries() -> Vec<&'static str> {
    registry::COMMANDS.iter().map(|c| c.id).collect()
}

/// The palette's rows for `query`: every match, best first; with no query, the recent commands
/// first, then every command in ribbon order (tabs, menus, palette-only).
pub fn rows(app: &App, query: &str) -> Vec<Row> {
    let l = app.lang;
    let recent = &app.prefs.commands;
    let mut scored: Vec<(i64, usize, &'static Cmd)> = registry::COMMANDS
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let r = recent.iter().position(|x| x == c.id);
            if query.trim().is_empty() {
                let rank = match r {
                    Some(p) => i64::try_from(p).unwrap_or(0) - 1000,
                    None => i64::try_from(c.place_rank()).unwrap_or(0),
                };
                return Some((rank, i, c));
            }
            let sc = score(query, c, l)?;
            let bonus = if r.is_some() { 150 } else { 0 };
            Some((-i64::from(sc) - bonus, i, c))
        })
        .collect();
    scored.sort_by_key(|(rank, i, _)| (*rank, *i));
    scored
        .into_iter()
        .take(MAX_ROWS)
        .map(|(_, _, c)| Row {
            id: c.id,
            label: c.text(l),
            path: c.place_path(l),
            shortcut: c.shortcut_for(app.caps().reserved_keys),
            state: ribbon::state(app, c.id),
            recent: recent.iter().any(|x| x == c.id),
        })
        .collect()
}

// --- opening and running --------------------------------------------------------------------

/// Opens the palette (or closes it when it is open).
pub fn toggle(app: &mut App) {
    if app.palette.is_some() {
        app.palette = None;
    } else {
        app.palette = Some(Palette {
            focus: true,
            ..Palette::default()
        });
        app.popup = None;
    }
}

/// Runs `id` from the palette as its button does, or through its form when `always_form`. A
/// command that cannot run keeps the palette open.
pub fn choose(app: &mut App, id: &str, always_form: bool) {
    let Some(c) = registry::find(id) else { return };
    if ribbon::state(app, id).is_err() {
        return;
    }
    app.palette = None;
    app.prefs.add_command(c.id);
    app.save_prefs();
    let pre = ribbon::prefill(app, c, None);
    if always_form {
        crate::form::open(app, c.id, pre);
    } else {
        ribbon::invoke(app, c.id, pre);
    }
}

/// The palette's key (Ctrl+Shift+P, in every view, the File view too). Returns true while the
/// palette or a form is open: the window's other keys wait.
pub fn keys(app: &mut App, ui: &egui::Ui) -> bool {
    let pal = ui.input(|i| i.modifiers.command && i.modifiers.shift && i.key_pressed(Key::P));
    if pal {
        toggle(app);
        return true;
    }
    app.palette.is_some() || app.form.is_some()
}

// --- drawing --------------------------------------------------------------------------------

/// Draws the palette when it is open (a 520 × 380 window 90 px from the top: search field, rows,
/// ↑ ↓ Enter Escape).
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut p) = app.palette.take() else {
        return;
    };
    let l = app.lang;
    let t = Tokens::of(app.prefs.theme);
    let rows = rows(app, &p.query);
    let (up, down, enter, shift, esc, page_up, page_down) = ctx.input(|i| {
        (
            i.key_pressed(Key::ArrowUp),
            i.key_pressed(Key::ArrowDown),
            i.key_pressed(Key::Enter),
            i.modifiers.shift,
            i.key_pressed(Key::Escape),
            i.key_pressed(Key::PageUp),
            i.key_pressed(Key::PageDown),
        )
    });
    if let Some(last) = rows.len().checked_sub(1) {
        let before = p.sel;
        if down {
            p.sel = p.sel.saturating_add(1);
        }
        if up {
            p.sel = p.sel.saturating_sub(1);
        }
        if page_down {
            p.sel = p.sel.saturating_add(8);
        }
        if page_up {
            p.sel = p.sel.saturating_sub(8);
        }
        p.sel = p.sel.min(last);
        p.scroll |= p.sel != before;
    } else {
        p.sel = 0;
    }
    let mut chosen: Option<(&'static str, bool)> = None;
    if enter && let Some(r) = rows.get(p.sel) {
        chosen = Some((r.id, shift));
    }
    egui::Window::new(l.t("Command palette", "Palette de commandes"))
        .id(egui::Id::new("bicraft-palette"))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .fixed_size(Vec2::new(520.0, 380.0))
        .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 90.0))
        .show(ctx, |ui| {
            let before = p.query.clone();
            let name = l.t("Search commands", "Rechercher une commande");
            let edit = ui.add(
                egui::TextEdit::singleline(&mut p.query)
                    .hint_text(name)
                    .desired_width(f32::INFINITY),
            );
            edit.widget_info(|| egui::WidgetInfo::text_edit(true, &before, &p.query, name));
            if p.focus {
                edit.request_focus();
                p.focus = false;
            }
            if p.query != before {
                p.sel = 0;
            }
            ui.add_space(4.0);
            let h = ui.available_height() - 22.0;
            egui::ScrollArea::vertical()
                .max_height(h.max(40.0))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if rows.is_empty() {
                        ui.label(
                            RichText::new(l.t("No command matches.", "Aucune commande ne correspond."))
                                .color(t.text_dim),
                        );
                    }
                    for (i, r) in rows.iter().enumerate() {
                        let resp = row(ui, r, i == p.sel, &t, l);
                        if i == p.sel && p.scroll {
                            resp.scroll_to_me(None);
                        }
                        if resp.clicked() {
                            p.sel = i;
                            chosen = Some((r.id, ui.input(|i| i.modifiers.shift)));
                        }
                    }
                });
            p.scroll = false;
            ui.label(
                RichText::new(l.t(
                    "↑ ↓ select · Enter run · Shift+Enter form · Esc close",
                    "↑ ↓ choisir · Entrée exécuter · Maj+Entrée formulaire · Échap fermer",
                ))
                .small()
                .color(t.text_faint),
            );
        });
    if esc {
        return;
    }
    if enter {
        // Enter took the focus off the search field
        p.focus = true;
    }
    app.palette = Some(p);
    if let Some((id, always)) = chosen {
        choose(app, id, always);
    }
}

/// One row: the label (greyed with the reason when it cannot run), its place and id, the
/// shortcut at the right.
fn row(ui: &mut egui::Ui, r: &Row, selected: bool, t: &Tokens, l: Lang) -> egui::Response {
    let w = ui.available_width();
    let enabled = r.state.is_ok();
    let h = if enabled { 36.0 } else { 50.0 };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
    let fill = if selected {
        t.accent_soft
    } else if resp.hovered() {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(8.0, 3.0)))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let main = if enabled { t.text } else { t.text_faint };
    // the label is the row's own (accessibility) name: painted, not a second label node
    let (top, _) = child.allocate_exact_size(Vec2::new(child.available_width(), 17.0), Sense::hover());
    let p = child.painter();
    let end = p.text(
        top.left_center(),
        Align2::LEFT_CENTER,
        r.label,
        theme::font(13.0),
        main,
    );
    if r.recent {
        p.text(
            egui::pos2(end.max.x + 8.0, top.center().y),
            Align2::LEFT_CENTER,
            l.t("recent", "récent"),
            theme::font(11.0),
            t.text_faint,
        );
    }
    if let Some(s) = r.shortcut {
        p.text(
            top.right_center(),
            Align2::RIGHT_CENTER,
            s,
            theme::font(11.0),
            t.text_faint,
        );
    }
    child.label(
        RichText::new(format!("{} · {}", r.path, r.id))
            .small()
            .color(t.text_dim),
    );
    if let Err(why) = &r.state {
        child.label(
            RichText::new(format!("{} {why}", l.t("Unavailable:", "Indisponible :")))
                .small()
                .color(t.text_dim),
        );
    }
    let label = r.label;
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, enabled, selected, label)
    });
    resp
}

#[cfg(test)]
#[path = "palette_tests.rs"]
mod tests;
